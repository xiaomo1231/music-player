//! 整理文件规则（kind: organize）：按文件名正则识别作品 / 集数，套目录模板
//!
//! 整理页扫描时（`organize::scan_core`），先按 id 顺序试生效中的规则，第一条匹配的规则说了算：
//! 把握高、理由“规则「名字」”，目标位置用整理页同样的模板生成；都不匹配的文件走原来的识别。
use super::clean::{find_line, item_line};
use super::{Issue, Kind, Rule};
use crate::AppState;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// 正则里可以用的命名分组
pub const GROUPS: &[&str] = &["series", "season", "episode", "title", "year", "group", "artist", "album", "track", "special"];
/// 模板里可以用的变量（和整理页一样，另加 {字幕组} {标题}）
pub const PLACEHOLDERS: &[&str] = &[
    "作品名",
    "片名",
    "年份",
    "季",
    "集",
    "特典",
    "艺人",
    "专辑",
    "音轨",
    "歌名",
    "作者",
    "书名",
    "原文件名",
    "字幕组",
    "标题",
];
const MEDIA: &[(&str, &str)] = &[
    ("anime", "番剧"),
    ("tv", "电视剧"),
    ("movie", "电影"),
    ("music", "音乐"),
    ("manga", "漫画"),
    ("novel", "小说"),
];
const ENTRY_KEYS: &[&str] = &["match", "media", "template", "on"];
const TOP_KEYS: &[&str] = &["em", "kind", "id", "name", "version", "author", "note", "hosts", "login", "rules"];

pub fn media_label(m: &str) -> &'static str {
    MEDIA.iter().find(|x| x.0 == m).map(|x| x.1).unwrap_or("其他")
}

/// 一种文件名
#[derive(Debug, Clone)]
pub struct Entry {
    pub re: Regex,
    /// anime / tv / movie / music / manga / novel
    pub media: String,
    /// 自己的目录模板；None 用整理页里这类文件的模板
    pub template: Option<String>,
    /// true：匹配完整路径（`\` 换成 `/`）；false：只匹配文件名（含扩展名）
    pub on_path: bool,
}

/// 检查过、编译好的规则
#[derive(Debug, Clone)]
pub struct OrgRule {
    pub id: String,
    pub name: String,
    pub entries: Vec<Entry>,
}

/// 匹配结果
#[derive(Debug, Clone, PartialEq)]
pub struct Matched {
    pub rule_id: String,
    pub rule_name: String,
    /// 第几种文件名（从 0 开始）
    pub entry: usize,
    pub media: String,
    pub template: Option<String>,
    /// 命名分组拿到的值（空的不放）
    pub fields: BTreeMap<String, String>,
}

static RE_TOKEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{([^{}:]+)(?::(\d+))?\}").unwrap());

fn regex_err(e: &regex::Error) -> String {
    let s = e.to_string();
    s.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(&s)
        .trim()
        .trim_start_matches("error: ")
        .to_string()
}

/// 检查模板：变量都认识、以 .ext 结尾、没有 .. 和绝对路径
pub fn check_template(tpl: &str) -> Result<(), String> {
    let t = tpl.trim();
    if t.is_empty() {
        return Err("template 不能为空（不写就用整理页里的模板）".into());
    }
    let outside_braces_colon = t.split(['{', '}']).step_by(2).any(|s| s.contains(':'));
    if t.starts_with('/') || t.starts_with('\\') || outside_braces_colon {
        return Err("template 是相对“整理到”文件夹的路径，不能写盘符或以 / 开头".into());
    }
    if t.split('/').any(|seg| seg.trim() == "..") {
        return Err("template 里不能有 ..".into());
    }
    if t.contains('\\') {
        return Err("template 用 / 分隔文件夹，不要用 \\".into());
    }
    for c in RE_TOKEN.captures_iter(t) {
        let name = c[1].trim();
        if !PLACEHOLDERS.contains(&name) {
            return Err(format!(
                "template 里的 {{{name}}} 不认识（可以用 {}）",
                PLACEHOLDERS.iter().map(|p| format!("{{{p}}}")).collect::<Vec<_>>().join(" ")
            ));
        }
    }
    if !t.ends_with(".ext") {
        return Err("template 要以 .ext 结尾（换成原文件的扩展名）".into());
    }
    Ok(())
}

/// 检查并编译
pub fn compile(rule: &Rule) -> Result<OrgRule, Vec<Issue>> {
    let text = &rule.text;
    let mut is = vec![];
    if let Some(obj) = rule.body.as_object() {
        for k in obj.keys() {
            if !TOP_KEYS.contains(&k.as_str()) {
                is.push(Issue::at(rule.line_of(k), format!("不认识的字段“{k}”（整理文件规则只需要 rules）")));
            }
        }
    }
    let rl = rule.line_of("rules");
    let mut entries = vec![];
    match rule.body.get("rules") {
        Some(Value::Array(list)) if !list.is_empty() => {
            for (i, v) in list.iter().enumerate() {
                let n = i + 1;
                let il = item_line(text, rl, n);
                let Some(m) = v.as_object() else {
                    is.push(Issue::at(il, format!("rules 第 {n} 项要写成 match、media 等字段")));
                    continue;
                };
                let pat = m.get("match").and_then(Value::as_str).unwrap_or("");
                let line = find_line(text, pat, il);
                let at = |k: &str| find_line(text, &format!("{k}:"), il);
                let before = is.len();
                for k in m.keys() {
                    if !ENTRY_KEYS.contains(&k.as_str()) {
                        is.push(Issue::at(at(k), format!("rules 第 {n} 项里不认识“{k}”（可以写 match、media、template、on）")));
                    }
                }
                let re = if pat.trim().is_empty() {
                    is.push(Issue::at(line, format!("rules 第 {n} 项缺少 match（文件名的正则）")));
                    None
                } else {
                    match Regex::new(pat) {
                        Ok(re) => {
                            let names: Vec<&str> = re.capture_names().flatten().collect();
                            if let Some(bad) = names.iter().find(|g| !GROUPS.contains(g)) {
                                is.push(Issue::at(line, format!("rules 第 {n} 项：分组名 {bad} 不认识（可以用 {}）", GROUPS.join("、"))));
                            } else if !names.contains(&"series") && !names.contains(&"title") {
                                is.push(Issue::at(
                                    line,
                                    format!("rules 第 {n} 项：正则里至少要有 (?P<series>…) 或 (?P<title>…) 分组，用来认出作品名"),
                                ));
                            }
                            Some(re)
                        }
                        Err(e) => {
                            is.push(Issue::at(line, format!("rules 第 {n} 项的正则写错了：{}", regex_err(&e))));
                            None
                        }
                    }
                };
                let media = m.get("media").and_then(Value::as_str).unwrap_or("");
                if !MEDIA.iter().any(|x| x.0 == media) {
                    let msg = if media.is_empty() { "缺少 media" } else { "media 写得不对" };
                    is.push(Issue::at(
                        at("media"),
                        format!("rules 第 {n} 项{msg}：只能是 anime、tv、movie、music、manga、novel"),
                    ));
                }
                let template = match m.get("template") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(t)) => {
                        if let Err(e) = check_template(t) {
                            is.push(Issue::at(find_line(text, t, il), format!("rules 第 {n} 项：{e}")));
                        }
                        Some(t.trim().to_string())
                    }
                    Some(_) => {
                        is.push(Issue::at(at("template"), format!("rules 第 {n} 项的 template 要写成文字")));
                        None
                    }
                };
                let on_path = match m.get("on").map(|v| v.as_str()) {
                    None | Some(Some("name")) => false,
                    Some(Some("path")) => true,
                    Some(_) => {
                        is.push(Issue::at(at("on"), format!("rules 第 {n} 项的 on 只能是 name（文件名）或 path（完整路径）")));
                        false
                    }
                };
                if let (Some(re), true) = (re, is.len() == before) {
                    entries.push(Entry {
                        re,
                        media: media.to_string(),
                        template,
                        on_path,
                    });
                }
            }
        }
        _ => is.push(Issue::at(rl, "要有 rules：至少写一种文件名（每项以 - 开头）")),
    }
    if !is.is_empty() {
        is.sort_by_key(|i| i.line);
        return Err(is);
    }
    Ok(OrgRule {
        id: rule.id().to_string(),
        name: rule.header.name.clone(),
        entries,
    })
}

/// 检查正文；返回导入确认框里显示的摘要（每行一句）
pub fn check(rule: &Rule) -> Result<Vec<String>, Vec<Issue>> {
    let r = compile(rule)?;
    let mut kinds: Vec<&str> = vec![];
    for e in &r.entries {
        let l = media_label(&e.media);
        if !kinds.contains(&l) {
            kinds.push(l);
        }
    }
    let mut out = vec![format!("识别 {} 种文件名 → {}", r.entries.len(), kinds.join("、"))];
    let own = r.entries.iter().filter(|e| e.template.is_some()).count();
    if own > 0 {
        out.push(format!("{own} 种用自己的目录模板，其余用整理页里的模板"));
    } else {
        out.push("目录按整理页里的模板".into());
    }
    if r.entries.iter().any(|e| e.on_path) {
        out.push("部分按完整路径匹配".into());
    }
    out.push("整理文件时先按这个规则识别，认不出的再用自带的识别".into());
    Ok(out)
}

/// 生效中的规则（按 id 排序）
pub fn active() -> Vec<OrgRule> {
    super::enabled(Kind::Organize).iter().filter_map(|r| compile(r).ok()).collect()
}

/// 按规则认一个文件：第一个匹配的说了算
pub fn match_path(rules: &[OrgRule], path: &Path) -> Option<Matched> {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let full = path.to_string_lossy().replace('\\', "/");
    for r in rules {
        for (i, e) in r.entries.iter().enumerate() {
            let hay = if e.on_path { &full } else { &name };
            let Some(c) = e.re.captures(hay) else { continue };
            let fields: BTreeMap<String, String> =
                e.re.capture_names()
                    .flatten()
                    .filter_map(|g| c.name(g).map(|m| (g.to_string(), m.as_str().trim().to_string())))
                    .filter(|(_, v)| !v.is_empty())
                    .collect();
            // 作品名分组是空的（可选分组没匹配上）就当没认出来，交给下一条
            if !fields.contains_key("series") && !fields.contains_key("title") {
                continue;
            }
            return Some(Matched {
                rule_id: r.id.clone(),
                rule_name: r.name.clone(),
                entry: i,
                media: e.media.clone(),
                template: e.template.clone(),
                fields,
            });
        }
    }
    None
}

/// 生效（on = true）或失效。整理规则在扫描时才读，这里不用做什么
pub fn apply(_app: &AppHandle, _rule: &Rule, _on: bool) {}

/// 规则被删除后清理自己的数据（没有）
pub fn removed(_app: &AppHandle, _id: &str) {}

/// 测试一行：匹配结果、字段、目标位置
pub fn test_line(rules: &[OrgRule], line: &str, target: &Path, st: &crate::organize::OrgSettings) -> Value {
    let p = Path::new(line);
    match match_path(rules, p) {
        Some(m) => {
            let dst: PathBuf = crate::organize::rule_dst(&m, p, target, st);
            json!({
                "line": line,
                "matched": true,
                "entry": m.entry + 1,
                "media": m.media,
                "media_label": media_label(&m.media),
                "fields": m.fields,
                "dst": dst.to_string_lossy(),
            })
        }
        None => json!({ "line": line, "matched": false }),
    }
}

/// 规则管理页的“测试”：arg = { lines: string[] | string, root?: string }
pub async fn test(app: AppHandle, rule: Arc<Rule>, arg: Value) -> Result<Value, String> {
    let r = compile(&rule).map_err(|is| super::issues_text(&is))?;
    let lines: Vec<String> = match arg.get("lines") {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        Some(Value::String(s)) => s.lines().map(String::from).collect(),
        _ => vec![],
    };
    let lines: Vec<String> = lines
        .into_iter()
        .map(|l| l.trim().trim_matches('"').to_string())
        .filter(|l| !l.is_empty())
        .take(500)
        .collect();
    if lines.is_empty() {
        return Err("先输入文件名，一行一个".into());
    }
    let root = arg
        .get("root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(r"D:\媒体")
        .to_string();
    let settings = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        crate::organize::load_settings(&conn)
    };
    let target = PathBuf::from(&root);
    let rules = [r];
    let items: Vec<Value> = lines.iter().map(|l| test_line(&rules, l, &target, &settings)).collect();
    Ok(json!({ "root": root, "items": items }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::em::format;

    fn rule(body: &str) -> Rule {
        format::parse(&format!("em: 1\nkind: organize\nid: t-org\nname: 测试整理\n{body}")).unwrap()
    }

    fn example() -> OrgRule {
        let r = format::parse(include_str!("../../../tools/em-examples/anime-release.em")).unwrap();
        compile(&r).unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn example_checks() {
        let r = format::parse(include_str!("../../../tools/em-examples/anime-release.em")).unwrap();
        let s = check(&r).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(s[0].starts_with("识别 ") && s[0].contains("番剧"), "{s:?}");
    }

    #[test]
    fn matches_release_name() {
        let rules = [example()];
        let m = match_path(&rules, Path::new("/dl/[Nekomoe kissaten] 葬送的芙莉莲 - 05 [1080p].mkv")).unwrap();
        assert_eq!(m.entry, 1);
        assert_eq!(m.media, "anime");
        assert_eq!(m.fields["series"], "葬送的芙莉莲");
        assert_eq!(m.fields["episode"], "05");
        assert_eq!(m.fields["group"], "Nekomoe kissaten");
        assert!(match_path(&rules, Path::new("/dl/随便一个视频.mp4")).is_none());
        let m = match_path(&rules, Path::new("/dl/[LoliHouse] 葬送的芙莉莲 第二季 - 03 [1080p].mkv")).unwrap();
        assert_eq!(m.entry, 0);
        assert_eq!((m.fields["series"].as_str(), m.fields["season"].as_str()), ("葬送的芙莉莲", "二"));
    }

    #[test]
    fn on_path_uses_forward_slashes() {
        let r = compile(&rule(
            "rules:\n  - match: '/剧集/(?P<series>[^/]+)/S(?P<season>\\d+)E(?P<episode>\\d+)'\n    media: tv\n    on: path\n",
        ))
        .unwrap();
        let m = match_path(&[r], Path::new(r"D:\剧集\漫长的季节\S01E03.mkv")).unwrap();
        assert_eq!(m.fields["series"], "漫长的季节");
        assert_eq!(m.fields["season"], "01");
    }

    #[test]
    fn validation_errors_have_lines() {
        let r = rule(
            "rules:\n  - match: '(?P<series>.+'\n    media: anime\n  - match: '(?P<name>.+) - (?P<episode>\\d+)'\n    media: anime\n  - match: '(?P<episode>\\d+)'\n    media: anime\n  - match: '(?P<series>.+)'\n    media: cartoon\n    on: folder\n  - match: '(?P<series>.+)'\n    media: tv\n    template: '剧/{作品}/{集:02}.ext'\n  - match: '(?P<series>.+)'\n    media: tv\n    template: '剧/{作品名}/{集:02}.mkv'\n",
        );
        let is = compile(&r).unwrap_err();
        let at = |needle: &str| is.iter().find(|i| i.msg.contains(needle)).unwrap_or_else(|| panic!("{needle}: {is:?}")).line;
        assert_eq!(at("正则写错了"), 6);
        assert_eq!(at("分组名 name"), 8);
        assert_eq!(at("至少要有"), 10);
        assert_eq!(at("media 写得不对"), 13);
        assert_eq!(at("on 只能是"), 14);
        assert_eq!(at("{作品} 不认识"), 17);
        assert_eq!(at(".ext 结尾"), 20);
        let is = compile(&rule("rules: []\n")).unwrap_err();
        assert_eq!(is[0].line, 5);
        assert!(check_template("../x/{作品名}.ext").is_err());
        assert!(check_template("D:/x/{作品名}.ext").is_err());
        assert!(check_template("番剧/{作品名}/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext").is_ok());
    }
}
