//! 小说排版规则（kind: clean）：去广告、替换、合并断行、缩进、分章
//!
//! - `remove`：写进去广告的规则表 novel_rules（source = `em:<规则 id>`），和用户自己加的规则一样在“扫描全书”时生效。
//!   作用范围是所有小说时写一行全局规则；按书名 / 路径限定时，给每本符合的书各写一行 `book:<id>` 规则：
//!   生效时把媒体库里已有的书都写上，之后每次打开或扫描一本书时再按当前书名、路径补上或删掉（新入库、改名的书也对）。
//!   规则文件停用或删除时这些行一起删掉。
//! - `replace`、`layout`：打开书时（阅读器、去广告扫描、导出干净副本）套用，原文件不改。
//!   去广告的清理记录按“套用排版之后”的文字记下标；排版规则变了以后，对不上的记录按原文片段重新定位
//!   （`adclean::relocate_cleanups`）。段首缩进、段间空行只影响显示和导出，在删完广告之后才加，不改变下标。
use super::{format, Issue, Kind, Rule};
use crate::{adclean, reader, AppState};
use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// 合并断行时一段最长多少字（防止整本没有标点的书被并成一段）
const MERGE_MAX: usize = 2000;
/// 自定义章节标题正则只认这么长以内的行
const TITLE_MAX: usize = 100;

// ---------------------------------------------------------------- 结构

/// 作用范围
#[derive(Debug, Clone)]
pub enum Scope {
    All,
    Title(Regex),
    Path(Regex),
}

impl Scope {
    /// 路径里的 `\` 统一换成 `/` 再匹配
    pub fn matches(&self, title: &str, path: &str) -> bool {
        match self {
            Scope::All => true,
            Scope::Title(re) => re.is_match(title),
            Scope::Path(re) => re.is_match(&path.replace('\\', "/")),
        }
    }
}

/// 一条去广告规则（和 novel_rules 一样的三种）
#[derive(Debug, Clone, PartialEq)]
pub struct Remove {
    /// line / regex / contains
    pub kind: String,
    pub pattern: String,
    pub note: String,
}

/// 检查过、编译好的规则
#[derive(Debug, Clone)]
pub struct Clean {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub remove: Vec<Remove>,
    pub replace: Vec<(Regex, String)>,
    /// None：不管（阅读器默认的断行处理）；true：合并断行；false：每行一段
    pub merge: Option<bool>,
    pub indent: Option<String>,
    pub blank_lines: Option<u8>,
    /// Some(true) 全角标点，Some(false) 保持原样
    pub fullwidth: Option<bool>,
    pub split: Option<Regex>,
}

// ---------------------------------------------------------------- 检查

const TOP_KEYS: &[&str] = &[
    "em", "kind", "id", "name", "version", "author", "note", "hosts", "login", "scope", "remove", "replace", "layout",
];
const LAYOUT_KEYS: &[&str] = &["merge_broken_lines", "indent", "blank_lines", "punctuation", "split_chapters"];

/// 列表 `key:` 的第 n 项（从 1 开始）从第几行开始：数 `- ` 开头的行，遇到下一个顶层字段为止。
/// 写成一行的 `[ ... ]` 找不到时返回 `from`
pub fn item_line(text: &str, from: usize, n: usize) -> usize {
    let mut k = 0;
    for (i, l) in text.lines().enumerate().skip(from) {
        if l.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            break;
        }
        if l.trim_start().starts_with("- ") || l.trim() == "-" {
            k += 1;
            if k == n {
                return i + 1;
            }
        }
    }
    from
}

/// 某段文字（正则等）在规则文件的第几行：从 `from` 行开始找，YAML 单引号里的 `'` 写成 `''`、双引号里的 `\` 写成 `\\`
pub fn find_line(text: &str, needle: &str, from: usize) -> usize {
    if needle.is_empty() {
        return from;
    }
    let cands = [
        needle.to_string(),
        needle.replace('\'', "''"),
        needle.replace('\\', "\\\\").replace('"', "\\\""),
    ];
    text.lines()
        .enumerate()
        .skip(from.saturating_sub(1))
        .find(|(_, l)| cands.iter().any(|c| l.contains(c.as_str())))
        .map(|(i, _)| i + 1)
        .unwrap_or(from)
}

fn regex_err(e: &regex::Error) -> String {
    // regex 的报错是多行的，取最后一行的说明
    let s = e.to_string();
    s.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(&s)
        .trim()
        .trim_start_matches("error: ")
        .to_string()
}

/// 去广告的 regex / line 规则匹配的是“去掉空白和标点、全角转半角”之后的文字，
/// 正则里（方括号外）直接写的全角标点、空格永远匹配不到
fn unreachable_char(pattern: &str) -> Option<char> {
    let mut in_class = false;
    let mut esc = false;
    for c in pattern.chars() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '[' => in_class = true,
            ']' => in_class = false,
            _ if in_class => {}
            ' ' | '\u{3000}' => return Some(c),
            _ if !c.is_ascii() && adclean::normalize(&c.to_string()).is_empty() => return Some(c),
            _ => {}
        }
    }
    None
}

static RE_GROUP_REF: Lazy<Regex> = Lazy::new(|| Regex::new(r"\$(?:\$|\{([^}]*)\}|([0-9A-Za-z_]+))").unwrap());

/// 替换文字里的 `$1` / `${name}` 要对得上正则里的分组
fn check_refs(re: &Regex, to: &str) -> Result<(), String> {
    for c in RE_GROUP_REF.captures_iter(to) {
        let Some(name) = c.get(1).or(c.get(2)).map(|m| m.as_str()) else { continue };
        let ok = match name.parse::<usize>() {
            Ok(n) => n < re.captures_len(),
            Err(_) => re.capture_names().flatten().any(|g| g == name),
        };
        if !ok {
            let digits: String = name.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            let hint = if c.get(2).is_some() && !digits.is_empty() {
                format!("（第 {digits} 组后面紧跟文字时写成 ${{{digits}}}）")
            } else {
                String::new()
            };
            return Err(format!("替换文字里的“{}”在正则里没有对应的分组{hint}", c.get(0).unwrap().as_str()));
        }
    }
    Ok(())
}

fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 检查并编译。所有问题一次报完
pub fn compile(rule: &Rule) -> Result<Clean, Vec<Issue>> {
    let text = &rule.text;
    let body = &rule.body;
    let mut is = vec![];
    if let Some(obj) = body.as_object() {
        for k in obj.keys() {
            if !TOP_KEYS.contains(&k.as_str()) {
                is.push(Issue::at(
                    rule.line_of(k),
                    format!("不认识的字段“{k}”（小说排版规则可以写 scope、remove、replace、layout）"),
                ));
            }
        }
    }

    // scope
    let sl = rule.line_of("scope");
    let scope = match body.get("scope") {
        None | Some(Value::Null) => Scope::All,
        Some(Value::String(s)) if s == "all" => Scope::All,
        Some(Value::Object(m)) if m.len() == 1 && (m.contains_key("title") || m.contains_key("path")) => {
            let (k, v) = m.iter().next().unwrap();
            match v.as_str() {
                Some(p) if !p.trim().is_empty() => match Regex::new(p) {
                    Ok(re) if k == "title" => Scope::Title(re),
                    Ok(re) => Scope::Path(re),
                    Err(e) => {
                        is.push(Issue::at(find_line(text, p, sl), format!("scope 的正则写错了：{}", regex_err(&e))));
                        Scope::All
                    }
                },
                _ => {
                    is.push(Issue::at(sl, format!("scope 的 {k} 要写成一个正则，例如 {{ {k}: '潮汐' }}")));
                    Scope::All
                }
            }
        }
        _ => {
            is.push(Issue::at(sl, "scope 只能是 all、{ title: '正则' } 或 { path: '正则' }"));
            Scope::All
        }
    };

    // remove
    let rl = rule.line_of("remove");
    let mut remove = vec![];
    match body.get("remove") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for (i, v) in list.iter().enumerate() {
                let n = i + 1;
                let il = item_line(text, rl, n);
                let Some(m) = v.as_object() else {
                    is.push(Issue::at(
                        il,
                        format!("remove 第 {n} 项要写成 {{line: '正则'}}、{{regex: '正则'}} 或 {{contains: '文字'}}"),
                    ));
                    continue;
                };
                let kinds: Vec<&str> = ["line", "regex", "contains"].into_iter().filter(|k| m.contains_key(*k)).collect();
                let extra: Vec<&String> = m.keys().filter(|k| !["line", "regex", "contains", "note"].contains(&k.as_str())).collect();
                let pat = kinds.first().and_then(|k| m.get(*k)).and_then(str_of).unwrap_or_default();
                let line = find_line(text, &pat, il);
                if kinds.len() != 1 || !extra.is_empty() {
                    is.push(Issue::at(
                        line,
                        format!("remove 第 {n} 项要写成 {{line: '正则'}}、{{regex: '正则'}} 或 {{contains: '文字'}}，三选一"),
                    ));
                    continue;
                }
                let kind = kinds[0];
                if let Err(e) = adclean::check_pattern(kind, &pat) {
                    is.push(Issue::at(line, format!("remove 第 {n} 项：{e}")));
                    continue;
                }
                if kind != "contains" {
                    if let Some(c) = unreachable_char(&pat) {
                        let what = if c.is_whitespace() { "空格".to_string() } else { format!("“{c}”") };
                        is.push(Issue::at(
                            line,
                            format!("remove 第 {n} 项：去广告规则匹配的是去掉空白和标点之后的文字，正则里的{what}永远匹配不到，删掉它（或放进 [] 里）"),
                        ));
                        continue;
                    }
                }
                remove.push(Remove {
                    kind: kind.into(),
                    pattern: pat,
                    note: m.get("note").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                });
            }
        }
        Some(_) => is.push(Issue::at(rl, "remove 要写成列表（每项以 - 开头）")),
    }

    // replace
    let pl = rule.line_of("replace");
    let mut replace = vec![];
    match body.get("replace") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            for (i, v) in list.iter().enumerate() {
                let n = i + 1;
                let il = item_line(text, pl, n);
                let m = v.as_object();
                let from = m.and_then(|m| m.get("from")).and_then(str_of);
                let to = m.and_then(|m| m.get("to")).map(|t| if t.is_null() { Some(String::new()) } else { str_of(t) });
                let bad_keys = m.is_some_and(|m| m.keys().any(|k| k != "from" && k != "to"));
                let line = find_line(text, from.as_deref().unwrap_or(""), il);
                let (Some(from), Some(Some(to)), false) = (from, to, bad_keys) else {
                    is.push(Issue::at(line, format!("replace 第 {n} 项要写成 {{ from: '正则', to: '替换成' }}")));
                    continue;
                };
                if from.is_empty() {
                    is.push(Issue::at(line, format!("replace 第 {n} 项的 from 不能为空")));
                    continue;
                }
                match Regex::new(&from) {
                    Ok(re) => match check_refs(&re, &to) {
                        Ok(()) => replace.push((re, to)),
                        Err(e) => is.push(Issue::at(line, format!("replace 第 {n} 项：{e}"))),
                    },
                    Err(e) => is.push(Issue::at(line, format!("replace 第 {n} 项的正则写错了：{}", regex_err(&e)))),
                }
            }
        }
        Some(_) => is.push(Issue::at(pl, "replace 要写成列表（每项以 - 开头）")),
    }

    // layout
    let ll = rule.line_of("layout");
    let (mut merge, mut indent, mut blank_lines, mut fullwidth, mut split) = (None, None, None, None, None);
    match body.get("layout") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            let at = |k: &str| find_line(text, k, ll);
            for k in m.keys() {
                if !LAYOUT_KEYS.contains(&k.as_str()) {
                    is.push(Issue::at(at(k), format!("layout 里不认识“{k}”（可以写 {}）", LAYOUT_KEYS.join("、"))));
                }
            }
            match m.get("merge_broken_lines") {
                None | Some(Value::Null) => {}
                Some(Value::Bool(b)) => merge = Some(*b),
                Some(_) => is.push(Issue::at(at("merge_broken_lines"), "merge_broken_lines 只能是 true 或 false")),
            }
            match m.get("indent") {
                None | Some(Value::Null) => {}
                Some(Value::String(s)) if s.chars().count() <= 8 && s.chars().all(|c| c == ' ' || c == '\u{3000}' || c == '\t') => indent = Some(s.clone()),
                Some(_) => is.push(Issue::at(at("indent"), "indent 只能是空格或全角空格（最多 8 个），不缩进写 ''")),
            }
            match m.get("blank_lines") {
                None | Some(Value::Null) => {}
                Some(v) => match v.as_u64() {
                    Some(n) if n <= 2 => blank_lines = Some(n as u8),
                    _ => is.push(Issue::at(at("blank_lines"), "blank_lines 只能是 0、1 或 2")),
                },
            }
            match m.get("punctuation").map(|v| v.as_str()) {
                None => {}
                Some(Some("fullwidth")) => fullwidth = Some(true),
                Some(Some("keep")) => fullwidth = Some(false),
                Some(_) => is.push(Issue::at(at("punctuation"), "punctuation 只能是 fullwidth（转成全角标点）或 keep（保持原样）")),
            }
            match m.get("split_chapters") {
                None | Some(Value::Null) => {}
                Some(Value::String(p)) if !p.trim().is_empty() => match Regex::new(p) {
                    Ok(re) => split = Some(re),
                    Err(e) => is.push(Issue::at(find_line(text, p, ll), format!("split_chapters 的正则写错了：{}", regex_err(&e)))),
                },
                Some(_) => is.push(Issue::at(at("split_chapters"), "split_chapters 要写成章节标题的正则")),
            }
        }
        Some(_) => is.push(Issue::at(ll, "layout 要写成“字段: 值”的形式")),
    }

    if is.is_empty()
        && remove.is_empty()
        && replace.is_empty()
        && merge.is_none()
        && indent.is_none()
        && blank_lines.is_none()
        && fullwidth.is_none()
        && split.is_none()
    {
        is.push(Issue::new("规则里没有要做的事：remove、replace、layout 至少写一项"));
    }
    if !is.is_empty() {
        is.sort_by_key(|i| i.line);
        return Err(is);
    }
    Ok(Clean {
        id: rule.id().to_string(),
        name: rule.header.name.clone(),
        scope,
        remove,
        replace,
        merge,
        indent,
        blank_lines,
        fullwidth,
        split,
    })
}

/// 检查正文；返回导入确认框里显示的摘要（每行一句）
pub fn check(rule: &Rule) -> Result<Vec<String>, Vec<Issue>> {
    let c = compile(rule)?;
    Ok(summary(&c))
}

fn summary(c: &Clean) -> Vec<String> {
    let mut out = vec![match &c.scope {
        Scope::All => "作用于：所有小说".to_string(),
        Scope::Title(re) => format!("作用于：书名符合 {} 的小说", re.as_str()),
        Scope::Path(re) => format!("作用于：路径符合 {} 的小说", re.as_str()),
    }];
    let mut what = vec![];
    if !c.remove.is_empty() {
        what.push(format!("删除 {} 类广告", c.remove.len()));
    }
    if !c.replace.is_empty() {
        what.push(format!("替换 {} 处", c.replace.len()));
    }
    if !what.is_empty() {
        out.push(what.join("、"));
    }
    let mut lay = vec![];
    match c.merge {
        Some(true) => lay.push("合并断行".to_string()),
        Some(false) => lay.push("每行一段".to_string()),
        None => {}
    }
    match c.indent.as_deref() {
        Some("") => lay.push("段首不缩进".to_string()),
        Some(_) => lay.push("段首缩进".to_string()),
        None => {}
    }
    if let Some(n) = c.blank_lines.filter(|n| *n > 0) {
        lay.push(format!("段间空 {n} 行"));
    }
    if c.fullwidth == Some(true) {
        lay.push("全角标点".to_string());
    }
    if c.split.is_some() {
        lay.push("按章节标题分章".to_string());
    }
    if !lay.is_empty() {
        out.push(format!("排版：{}", lay.join("、")));
    }
    if !c.remove.is_empty() {
        out.push("删除广告在阅读器的“扫描全书”时生效".to_string());
    }
    out
}

// ---------------------------------------------------------------- 文字处理

fn ends_sentence(s: &str) -> bool {
    s.chars().last().is_some_and(|c| "。！？!?….”」』\"'）)】：:；;—～~".contains(c))
}

/// 合并断行：一句话被硬换行拆开时接回去。不跨空行、不跨章节标题、上一行以句末标点结尾时不接；
/// 下一行有段首缩进或以引号、括号开头时也是新的一段
pub fn merge_lines(lines: Vec<String>, is_title: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut joinable = false;
    for l in lines {
        let t = l.trim();
        if t.is_empty() {
            joinable = false;
            continue;
        }
        let indented = l.starts_with([' ', '\u{3000}', '\t']);
        let opens = t.starts_with(['“', '「', '『', '"', '（', '(', '【']);
        let title = is_title(t);
        if joinable && !indented && !opens && !title {
            if let Some(prev) = out.last_mut().filter(|p| p.chars().count() < MERGE_MAX) {
                // 英文单词之间补一个空格
                if prev.ends_with(|c: char| c.is_ascii_alphanumeric()) && t.starts_with(|c: char| c.is_ascii_alphanumeric()) {
                    prev.push(' ');
                }
                prev.push_str(t);
                joinable = !ends_sentence(t);
                continue;
            }
        }
        joinable = !title && !ends_sentence(t);
        out.push(t.to_string());
    }
    out
}

/// 不合并：每个非空行是一段
pub fn keep_lines(lines: Vec<String>) -> Vec<String> {
    lines.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

/// 按规则分章：自定义的章节标题正则、断行处理
pub fn split_text(text: &str, split: Option<&Regex>, merge: Option<bool>) -> Vec<reader::Chapter> {
    let is_title = |l: &str| match split {
        Some(re) => l.trim().chars().count() <= TITLE_MAX && re.is_match(l),
        None => reader::is_chapter_title(l),
    };
    let merge_fn = |lines: Vec<String>| match merge {
        Some(true) => merge_lines(lines, &is_title),
        Some(false) => keep_lines(lines),
        None => reader::merge_broken_lines(lines),
    };
    reader::split_txt_with(text, &is_title, &merge_fn)
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3000..=0x303F | 0xFF00..=0xFFEF) || "“”‘’…—".contains(c)
}

/// 网址、域名、数字（3.14、1,000、12:30）原样保留
static RE_KEEP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:[a-z][a-z0-9+.-]*://|www\.)[^\s\p{Han}，。！？；：“”（）]+|[a-z0-9_-]+(?:\.[a-z0-9_-]+)+(?:/[^\s\p{Han}，。！？；：“”（）]*)?|\d+(?:[.,:]\d+)+").unwrap()
});

/// 中文语境里的半角标点转成全角（一个字换一个字，长度不变）。
/// 只转前后挨着中文的标点；网址、域名、3.14 这样的数字不动；连续的 . 不动（省略号交给 replace）
pub fn fullwidth(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut keep = vec![false; chars.len()];
    for m in RE_KEEP.find_iter(s) {
        // 网址末尾的标点多半是句子的（和常见的自动加链接一样处理）
        let t = m.as_str().trim_end_matches([',', '.', ';', ':', '!', '?', '(', ')', '"']);
        let a = s[..m.start()].chars().count();
        let b = a + t.chars().count();
        keep[a..b].iter_mut().for_each(|k| *k = true);
    }
    let mut open = false;
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if keep[i] || !c.is_ascii_punctuation() {
            out.push(c);
            continue;
        }
        // 前一个字看已经转换过的（“走吧”。里的句号前面是 ”）
        let prev = out.chars().rev().find(|c| !c.is_whitespace());
        let next = chars[i + 1..].iter().find(|c| !c.is_whitespace()).copied();
        let cjk = prev.is_some_and(is_cjk) || next.is_some_and(is_cjk);
        let r = match c {
            '"' if open => {
                open = false;
                '”'
            }
            '"' if cjk => {
                open = true;
                '“'
            }
            _ if !cjk => c,
            ',' => '，',
            '.' if prev != Some('.') && next != Some('.') => '。',
            '!' => '！',
            '?' => '？',
            ';' => '；',
            ':' => '：',
            '(' => '（',
            ')' => '）',
            _ => c,
        };
        out.push(r);
    }
    out
}

/// 按顺序做所有替换，count 累加命中次数
pub fn apply_replace(s: &str, reps: &[(Regex, String)], count: &mut usize) -> String {
    let mut t = s.to_string();
    for (re, to) in reps {
        let n = re.find_iter(&t).count();
        if n > 0 {
            *count += n;
            t = re.replace_all(&t, to.as_str()).into_owned();
        }
    }
    t
}

/// 多条规则同时作用于一本书时：按 id 顺序，后面的规则覆盖前面的设置；replace 依次全做
fn last<T>(rules: &[&Clean], f: impl Fn(&Clean) -> Option<T>) -> Option<T> {
    rules.iter().rev().find_map(|r| f(r))
}

/// 对章节标题和正文做替换、转全角标点；替换后变空的段落去掉。返回替换次数
pub fn transform(chapters: &mut [reader::Chapter], rules: &[&Clean]) -> usize {
    let reps: Vec<(Regex, String)> = rules.iter().flat_map(|r| r.replace.iter().cloned()).collect();
    let fw = last(rules, |r| r.fullwidth) == Some(true);
    let mut n = 0;
    if reps.is_empty() && !fw {
        return 0;
    }
    let fix = |s: &str, n: &mut usize| {
        let t = apply_replace(s, &reps, n);
        if fw {
            fullwidth(&t)
        } else {
            t
        }
    };
    for ch in chapters.iter_mut() {
        let mut dummy = 0;
        ch.title = fix(&ch.title, &mut dummy).trim().to_string();
        ch.paras = std::mem::take(&mut ch.paras)
            .into_iter()
            .map(|p| fix(&p, &mut n).trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
    }
    n
}

/// 按规则打开一本书（原文件不改）
pub fn build(path: &Path, rules: &[&Clean]) -> Result<reader::Book> {
    let is_txt = crate::parse::ext(path) != "epub";
    let split = last(rules, |r| r.split.clone());
    let merge = last(rules, |r| r.merge);
    let mut book = if is_txt && (split.is_some() || merge.is_some()) {
        reader::open_txt_with(path, &|t| split_text(t, split.as_ref(), merge))?
    } else {
        reader::open_book(path)?
    };
    transform(&mut book.chapters, rules);
    Ok(book)
}

// ---------------------------------------------------------------- 打开书

/// 阅读器显示用的排版（缩进、段间空行只在显示和导出时加）
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct View {
    pub indent: String,
    pub blank_lines: u8,
    /// 生效的规则名
    pub rules: Vec<String>,
}

/// open_book 命令的返回：书 + 套用的排版（没有规则时为 null）
#[derive(Serialize, Clone, Debug)]
pub struct Opened {
    #[serde(flatten)]
    pub book: reader::Book,
    pub layout: Option<View>,
}

/// 编译生效中的规则（启动 / 导入时已检查过，这里出错的跳过）
pub fn compiled(rules: &[Arc<Rule>]) -> Vec<Clean> {
    rules.iter().filter_map(|r| compile(r).ok()).collect()
}

fn book_title(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT title FROM books WHERE file_id=?1", [id], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

/// 打开一本书：套用生效中的小说排版规则（阅读器、去广告扫描、导出都走这里）。不要在持有主数据库锁时调用
pub fn open_book(conn: &Connection, id: i64, path: &str) -> Result<Opened> {
    let rules = compiled(&super::enabled(Kind::Clean));
    open_with(conn, id, path, &rules)
}

pub fn open_with(conn: &Connection, id: i64, path: &str, rules: &[Clean]) -> Result<Opened> {
    let title = book_title(conn, id).unwrap_or_else(|| Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    if let Err(e) = sync_book(conn, id, &title, path, rules) {
        eprintln!("小说排版规则：同步去广告规则失败：{e}");
    }
    let matching: Vec<&Clean> = rules.iter().filter(|r| r.scope.matches(&title, path)).collect();
    let book = build(Path::new(path), &matching)?;
    if let Err(e) = adclean::relocate_cleanups(conn, id, &book) {
        eprintln!("小说排版规则：重新定位清理记录失败：{e}");
    }
    let layout = (!matching.is_empty()).then(|| View {
        indent: last(&matching, |r| r.indent.clone()).unwrap_or_else(|| "　　".into()),
        blank_lines: last(&matching, |r| r.blank_lines).unwrap_or(0),
        rules: matching.iter().map(|r| r.name.clone()).collect(),
    });
    Ok(Opened { book, layout })
}

// ---------------------------------------------------------------- 同步去广告规则

fn source_of(id: &str) -> String {
    format!("{}{id}", adclean::EM_SOURCE)
}

fn note_of(c: &Clean, r: &Remove) -> String {
    if r.note.is_empty() {
        format!("规则文件「{}」", c.name)
    } else {
        format!("{}（规则文件「{}」）", r.note, c.name)
    }
}

/// 让 novel_rules 里 source = em:<id>（只看 scope 这一个范围时传 only_scope）的行正好是 wanted：多的删、少的加、说明更新
fn diff_rows(conn: &Connection, c: &Clean, only_scope: Option<&str>, wanted: &[(String, &Remove)]) -> Result<()> {
    let src = source_of(&c.id);
    let existing: Vec<(i64, String, String, String)> = {
        let sql = if only_scope.is_some() {
            "SELECT id,scope,kind,pattern FROM novel_rules WHERE source=?1 AND scope=?2"
        } else {
            "SELECT id,scope,kind,pattern FROM novel_rules WHERE source=?1 AND ?2 IS NULL"
        };
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map(params![src, only_scope], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut have = HashSet::new();
    for (rid, scope, kind, pattern) in existing {
        match wanted.iter().find(|(s, r)| *s == scope && r.kind == kind && r.pattern == pattern) {
            Some((_, r)) if have.insert((scope.clone(), kind.clone(), pattern.clone())) => {
                conn.execute("UPDATE novel_rules SET note=?1, enabled=1 WHERE id=?2", params![note_of(c, r), rid])?;
            }
            _ => {
                conn.execute("DELETE FROM novel_rules WHERE id=?1", [rid])?;
            }
        }
    }
    for (scope, r) in wanted {
        if have.insert((scope.clone(), r.kind.clone(), r.pattern.clone())) {
            conn.execute(
                "INSERT INTO novel_rules(scope,kind,pattern,note,enabled,source) VALUES(?1,?2,?3,?4,1,?5)",
                params![scope, r.kind, r.pattern, note_of(c, r), src],
            )?;
        }
    }
    Ok(())
}

/// 规则生效：全局规则写一行；按书名 / 路径限定的，给媒体库里每本符合的书各写一行
pub fn sync_all(conn: &Connection, c: &Clean) -> Result<()> {
    let wanted: Vec<(String, &Remove)> = match c.scope {
        Scope::All => c.remove.iter().map(|r| ("global".to_string(), r)).collect(),
        _ => {
            let mut st = conn.prepare("SELECT b.file_id, b.title, f.path FROM books b JOIN files f ON f.id=b.file_id ORDER BY b.file_id")?;
            let books: Vec<(i64, String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
            books
                .iter()
                .filter(|(_, t, p)| c.scope.matches(t, p))
                .flat_map(|(id, _, _)| c.remove.iter().map(move |r| (format!("book:{id}"), r)))
                .collect()
        }
    };
    diff_rows(conn, c, None, &wanted)
}

/// 打开 / 扫描一本书时：按这本书现在的书名、路径补上或删掉按范围限定的规则
pub fn sync_book(conn: &Connection, id: i64, title: &str, path: &str, rules: &[Clean]) -> Result<()> {
    let scope = format!("book:{id}");
    for c in rules {
        if matches!(c.scope, Scope::All) {
            continue;
        }
        let wanted: Vec<(String, &Remove)> = if c.scope.matches(title, path) {
            c.remove.iter().map(|r| (scope.clone(), r)).collect()
        } else {
            vec![]
        };
        diff_rows(conn, c, Some(&scope), &wanted)?;
    }
    Ok(())
}

/// 规则停用 / 删除：去掉它写进去的所有去广告规则（已经清理的记录保留，和删除普通规则一样）
pub fn unsync(conn: &Connection, id: &str) -> Result<usize> {
    Ok(conn.execute("DELETE FROM novel_rules WHERE source=?1", [source_of(id)])?)
}

fn open_conn(app: &AppHandle) -> Option<Connection> {
    let st = app.try_state::<AppState>()?;
    // 单独开连接：apply 可能在别人持有主连接锁时被调用（规则文件被改后的自动重读）
    let conn = Connection::open(&st.db_path).ok()?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).ok()?;
    Some(conn)
}

/// 生效（on = true）或失效
pub fn apply(app: &AppHandle, rule: &Rule, on: bool) {
    let Some(conn) = open_conn(app) else { return };
    let r = match (on, compile(rule)) {
        (true, Ok(c)) => sync_all(&conn, &c),
        _ => unsync(&conn, rule.id()).map(|_| ()),
    };
    if let Err(e) = r {
        eprintln!("小说排版规则 {}：同步去广告规则失败：{e}", rule.id());
    }
}

/// 规则被删除后清理自己的数据
pub fn removed(app: &AppHandle, id: &str) {
    if let Some(conn) = open_conn(app) {
        let _ = unsync(&conn, id);
    }
}

// ---------------------------------------------------------------- 测试

/// 用一段示例文字跑一遍：分章 → 替换、全角标点 → 删广告 → 缩进、空行
pub fn run_test(c: &Clean, text: &str) -> Value {
    let rules = [c];
    let text = text.replace("\r\n", "\n");
    let mut chapters = split_text(&text, c.split.as_ref(), c.merge);
    let replaced = transform(&mut chapters, &rules);
    let mut removed: Vec<String> = vec![];
    for ch in chapters.iter_mut() {
        let paras = std::mem::take(&mut ch.paras);
        for p in paras {
            let mut ranges: Vec<(usize, usize)> = vec![];
            for r in &c.remove {
                ranges.extend(adclean::spans_for(&r.kind, &r.pattern, &p).unwrap_or_default());
            }
            ranges.sort();
            ranges.dedup();
            let chars: Vec<char> = p.chars().collect();
            for &(s, e) in &ranges {
                let t: String = chars[s.min(chars.len())..e.min(chars.len())].iter().collect();
                if !t.trim().is_empty() {
                    removed.push(t);
                }
            }
            let t = if ranges.is_empty() {
                p
            } else {
                adclean::remove_ranges(&p, &ranges).trim().to_string()
            };
            if !t.is_empty() {
                ch.paras.push(t);
            }
        }
    }
    let titles: Vec<String> = chapters.iter().map(|c| c.title.clone()).collect();
    let book = reader::Book {
        title: String::new(),
        author: String::new(),
        encoding: "UTF-8".into(),
        chapters,
    };
    let indent = c.indent.clone().unwrap_or_else(|| "　　".into());
    let output = adclean::export_text_with(&book, &[], &indent, c.blank_lines.unwrap_or(0) as usize);
    json!({ "output": output.trim_end(), "removed": removed, "replaced": replaced, "chapters": titles })
}

/// 规则管理页的“测试”：arg = { text }
pub async fn test(_app: AppHandle, rule: Arc<Rule>, arg: Value) -> Result<Value, String> {
    let text = arg.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    if text.trim().is_empty() {
        return Err("先输入一段文字".into());
    }
    if text.len() > format::MAX_BYTES * 4 {
        return Err("文字太长，测试只需要一小段".into());
    }
    let c = compile(&rule).map_err(|is| super::issues_text(&is))?;
    Ok(run_test(&c, &text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::{Book, Chapter};

    fn rule(body: &str) -> Rule {
        format::parse(&format!("em: 1\nkind: clean\nid: t-rule\nname: 测试排版\n{body}")).unwrap()
    }

    fn clean(body: &str) -> Clean {
        compile(&rule(body)).unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn merge_joins_broken_sentence() {
        let v = merge_lines(lines(&["他推开门", "走了进来。", "外面在下雨"]), &reader::is_chapter_title);
        assert_eq!(v, vec!["他推开门走了进来。", "外面在下雨"]);
    }

    #[test]
    fn merge_stops_at_blank_lines() {
        let v = merge_lines(lines(&["他推开门", "", "走了进来"]), &reader::is_chapter_title);
        assert_eq!(v, vec!["他推开门", "走了进来"]);
    }

    #[test]
    fn merge_stops_at_titles_and_sentence_end() {
        let v = merge_lines(
            lines(&["上一章的最后一句", "第二章 雾港", "雾是从后半夜", "开始涨起来的。", "她说"]),
            &reader::is_chapter_title,
        );
        assert_eq!(v, vec!["上一章的最后一句", "第二章 雾港", "雾是从后半夜开始涨起来的。", "她说"]);
        // 以句末标点、引号结尾的不接；缩进或引号开头的下一行是新段
        let v = merge_lines(lines(&["“好。”", "他点头", "　　第二天", "“走吧”"]), &reader::is_chapter_title);
        assert_eq!(v, vec!["“好。”", "他点头", "第二天", "“走吧”"]);
        let v = merge_lines(lines(&["The fog came", "in slowly."]), &reader::is_chapter_title);
        assert_eq!(v, vec!["The fog came in slowly."]);
    }

    #[test]
    fn split_chapters_custom_regex() {
        let re = Regex::new(r"^◆\s*\d+").unwrap();
        let c = split_text("前言\n◆ 1 开始\n他走\n了。\n\n第二章 不是标题\n◆ 2 结束\n好。\n", Some(&re), Some(true));
        let titles: Vec<&str> = c.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["开始", "◆ 1 开始", "◆ 2 结束"]);
        assert_eq!(c[1].paras, vec!["他走了。", "第二章 不是标题"]);
        // 不合并：每行一段
        let c = split_text("第一章 a\n他走\n了。\n", None, Some(false));
        assert_eq!(c[0].paras, vec!["他走", "了。"]);
    }

    #[test]
    fn fullwidth_punctuation() {
        assert_eq!(fullwidth("他说,今天很好.真的!"), "他说，今天很好。真的！");
        assert_eq!(fullwidth("圆周率是3.14,不是3,14吗?"), "圆周率是3.14，不是3,14吗？");
        assert_eq!(fullwidth("网址https://example.com/a.b?x=1,请看"), "网址https://example.com/a.b?x=1，请看");
        assert_eq!(fullwidth("访问www.abc.cn(备用)"), "访问www.abc.cn（备用）");
        assert_eq!(fullwidth("他说:\"走吧\"."), "他说：“走吧”。");
        assert_eq!(fullwidth("Hello, world."), "Hello, world.");
        assert_eq!(fullwidth("等等...好"), "等等...好");
        assert_eq!(fullwidth("时间是12:30,对"), "时间是12:30，对");
        // 一个字换一个字
        let s = "他说,好.";
        assert_eq!(fullwidth(s).chars().count(), s.chars().count());
    }

    #[test]
    fn replace_with_groups() {
        let c = clean("replace:\n  - { from: '\\.{3,}', to: '……' }\n  - { from: '\\s+([，。！？；：])', to: '$1' }\n  - { from: '(\\d+)元', to: '${1} 元' }\n");
        let mut n = 0;
        assert_eq!(apply_replace("等等... 好 ，100元", &c.replace, &mut n), "等等…… 好，100 元");
        assert_eq!(n, 3);
    }

    #[test]
    fn scope_matching() {
        let c = clean("scope: { title: '潮汐' }\nlayout: { indent: '' }\n");
        assert!(c.scope.matches("潮汐档案", "D:\\x.txt"));
        assert!(!c.scope.matches("雾港纪事", "D:\\潮汐.txt"));
        let c = clean("scope: { path: '^D:/小说/连载/' }\nlayout: { indent: '' }\n");
        assert!(c.scope.matches("x", "D:\\小说\\连载\\a.txt"));
        assert!(!c.scope.matches("x", "D:\\小说\\完本\\a.txt"));
        assert!(matches!(clean("layout: { indent: '' }\n").scope, Scope::All));
    }

    #[test]
    fn validation_errors_have_lines() {
        let r = rule("scope: { title: '(' }\nremove:\n  - line: '^本章完$'\n  - regex: '[（(]本章未完，请翻页[)）]'\n  - foo: x\nreplace:\n  - { from: 'a(b', to: 'x' }\n  - { from: '(a)', to: '$2' }\nlayout:\n  blank_lines: 5\n  punctuation: half\n  bogus: 1\n");
        let is = compile(&r).unwrap_err();
        let at = |needle: &str| is.iter().find(|i| i.msg.contains(needle)).unwrap_or_else(|| panic!("{needle}: {is:?}")).line;
        assert_eq!(at("scope 的正则"), 5);
        assert_eq!(at("永远匹配不到"), 8);
        assert_eq!(at("三选一"), 9);
        assert_eq!(at("replace 第 1 项的正则"), 11);
        assert_eq!(at("没有对应的分组"), 12);
        assert_eq!(at("blank_lines"), 14);
        assert_eq!(at("punctuation"), 15);
        assert_eq!(at("bogus"), 16);
        let is = compile(&rule("layout:\n  split_chapters: '第(章'\n")).unwrap_err();
        assert_eq!(is[0].line, 6, "{is:?}");
        let is = compile(&rule("scope: all\n")).unwrap_err();
        assert!(is[0].msg.contains("没有要做的事"));
        let is = compile(&rule("remove: [ { line: '' } ]\n")).unwrap_err();
        assert!(is[0].msg.contains("不能为空"), "{is:?}");
    }

    #[test]
    fn example_file_checks() {
        let text = include_str!("../../../tools/em-examples/novel-layout.em");
        let r = format::parse(text).unwrap();
        let s = check(&r).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(s[0], "作用于：所有小说");
        assert!(s.iter().any(|l| l == "删除 2 类广告、替换 2 处"), "{s:?}");
        assert!(s.iter().any(|l| l == "排版：合并断行、段首缩进、全角标点、按章节标题分章"), "{s:?}");
    }

    #[test]
    fn test_run_on_sample() {
        let r = format::parse(include_str!("../../../tools/em-examples/novel-layout.em")).unwrap();
        let c = compile(&r).unwrap();
        let v = run_test(
            &c,
            "第一章 雾港\n雾是从后半夜\n开始涨起来的...\n本章完\n\n第二章 灯塔\n他说,圆周率是3.14 。（本章未完，请翻页）\n",
        );
        assert_eq!(v["chapters"], json!(["第一章 雾港", "第二章 灯塔"]));
        assert_eq!(v["replaced"], json!(2));
        let out = v["output"].as_str().unwrap();
        assert!(out.contains("　　雾是从后半夜开始涨起来的……"), "{out}");
        assert!(out.contains("　　他说，圆周率是3.14。"), "{out}");
        assert!(!out.contains("本章完") && !out.contains("翻页"), "{out}");
        assert_eq!(v["removed"].as_array().unwrap().len(), 2, "{v}");
    }

    fn mem() -> Connection {
        let conn = crate::db::open_memory().unwrap();
        adclean::init(&conn).unwrap();
        for (id, title, path) in [(1, "潮汐档案", "D:/小说/潮汐档案.txt"), (2, "雾港纪事", "D:/小说/雾港纪事.txt")] {
            conn.execute("INSERT INTO files(id,path,kind,added_at) VALUES(?1,?2,'novel',0)", params![id, path])
                .unwrap();
            conn.execute("INSERT INTO books(file_id,title,format) VALUES(?1,?2,'txt')", params![id, title])
                .unwrap();
        }
        conn
    }

    fn em_rows(conn: &Connection) -> Vec<(String, String, String)> {
        let mut st = conn
            .prepare("SELECT scope,pattern,source FROM novel_rules WHERE source LIKE 'em:%' ORDER BY scope,pattern")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn novel_rules_sync_global() {
        let conn = mem();
        let c = clean("remove:\n  - line: '^本章完$'\n  - contains: '求月票'\n");
        sync_all(&conn, &c).unwrap();
        sync_all(&conn, &c).unwrap();
        let rows = em_rows(&conn);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().all(|r| r.0 == "global" && r.2 == "em:t-rule"));
        // 改了规则：留下没变的那行（id 不变），删掉去掉的，加上新的
        let id_before: i64 = conn.query_row("SELECT id FROM novel_rules WHERE pattern='^本章完$'", [], |r| r.get(0)).unwrap();
        let c2 = clean("remove:\n  - line: '^本章完$'\n  - regex: '加群\\d+'\n");
        sync_all(&conn, &c2).unwrap();
        let rows = em_rows(&conn);
        assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), vec!["^本章完$", "加群\\d+"]);
        let id_after: i64 = conn.query_row("SELECT id FROM novel_rules WHERE pattern='^本章完$'", [], |r| r.get(0)).unwrap();
        assert_eq!(id_before, id_after);
        // 停用 / 删除
        assert_eq!(unsync(&conn, "t-rule").unwrap(), 2);
        assert!(em_rows(&conn).is_empty());
        // 内置规则不受影响
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM novel_rules WHERE source='builtin'", [], |r| r.get(0))
            .unwrap();
        assert!(n > 0);
    }

    #[test]
    fn novel_rules_sync_scoped() {
        let conn = mem();
        let c = clean("scope: { title: '潮汐' }\nremove:\n  - contains: '求月票'\n");
        sync_all(&conn, &c).unwrap();
        assert_eq!(em_rows(&conn), vec![("book:1".into(), "求月票".into(), "em:t-rule".into())]);
        // 书改名后打开：不再符合，删掉；另一本改名后符合，补上
        conn.execute("UPDATE books SET title='潮汐档案（下）' WHERE file_id=2", []).unwrap();
        sync_book(&conn, 1, "另一个名字", "D:/小说/a.txt", std::slice::from_ref(&c)).unwrap();
        sync_book(&conn, 2, "潮汐档案（下）", "D:/小说/雾港纪事.txt", std::slice::from_ref(&c)).unwrap();
        assert_eq!(em_rows(&conn), vec![("book:2".into(), "求月票".into(), "em:t-rule".into())]);
        unsync(&conn, "t-rule").unwrap();
        assert!(em_rows(&conn).is_empty());
    }

    #[test]
    fn open_with_applies_layout_and_relocates_cleanups() {
        let conn = mem();
        let dir = std::env::temp_dir().join(format!("mp-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("潮汐档案.txt");
        std::fs::write(&path, "第一章 雾港\n雾是从后半夜\n开始涨起来的。求月票\n　　第二段。\n").unwrap();
        let p = path.to_string_lossy().into_owned();
        conn.execute("UPDATE files SET path=?1 WHERE id=1", [&p]).unwrap();

        // 没有规则：阅读器默认分段
        let o = open_with(&conn, 1, &p, &[]).unwrap();
        assert!(o.layout.is_none());
        let paras = o.book.chapters[0].paras.clone();
        assert_eq!(paras, vec!["雾是从后半夜开始涨起来的。求月票", "第二段。"]);
        // 记一条清理：“求月票”
        conn.execute(
            "INSERT INTO novel_cleanups(file_id,chapter,text,rule_id,confidence,status) VALUES(1,0,?1,NULL,'hi','applied')",
            [r#"{"para":0,"start":13,"end":16,"text":"求月票","rule":"x"}"#],
        )
        .unwrap();

        // 启用“每行一段 + 替换”的规则：段落和下标都变了，清理记录跟着走
        let c = clean("replace:\n  - { from: '雾', to: '大雾' }\nlayout:\n  merge_broken_lines: false\n  indent: ''\n  blank_lines: 1\n");
        let o = open_with(&conn, 1, &p, std::slice::from_ref(&c)).unwrap();
        assert_eq!(o.book.chapters[0].paras, vec!["大雾是从后半夜", "开始涨起来的。求月票", "第二段。"]);
        assert_eq!(o.layout.as_ref().unwrap().indent, "");
        assert_eq!(o.layout.as_ref().unwrap().blank_lines, 1);
        let text: String = conn.query_row("SELECT text FROM novel_cleanups", [], |r| r.get(0)).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            (v["para"].as_u64(), v["start"].as_u64(), v["end"].as_u64()),
            (Some(1), Some(7), Some(10)),
            "{v}"
        );
        // 导出：先删广告再排版
        let hits = vec![adclean::Hit {
            chapter: 0,
            para: 1,
            start: 7,
            end: 10,
            text: "求月票".into(),
            status: "applied".into(),
            ..Default::default()
        }];
        let out = adclean::export_text_with(&o.book, &hits, "", 1);
        assert_eq!(out, "第一章 大雾港\n\n大雾是从后半夜\n\n开始涨起来的。\n\n第二段。\n\n\n");
        // 规则停用：回到默认分段，记录再挪回去
        open_with(&conn, 1, &p, &[]).unwrap();
        let text: String = conn.query_row("SELECT text FROM novel_cleanups", [], |r| r.get(0)).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((v["para"].as_u64(), v["start"].as_u64()), (Some(0), Some(13)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn book_struct_is_flattened() {
        let o = Opened {
            book: Book {
                title: "t".into(),
                author: String::new(),
                encoding: "UTF-8".into(),
                chapters: vec![Chapter {
                    title: "c".into(),
                    paras: vec![],
                }],
            },
            layout: None,
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["title"], "t");
        assert!(v["layout"].is_null());
    }
}
