//! 小说去广告与文本净化。
//! - 第一层（本地、免费）：归一化 + 内置规则 / 用户规则 + 章首章尾频率统计
//! - 第三层（本地）：用户标记一段后，在全书找近似内容（归一化后的字符二元组连续覆盖）
//! - 第二层（AI）：把可疑段落（及前后各一段）发给 AI，归纳成规则；不发送整本书
//!
//! 默认不改原文件：清理记录写入 novel_cleanups，阅读时套用，导出时才生成新文件。
//! novel_cleanups.text 列存 JSON：{"para":段号,"start":段内起点,"end":终点,"text":原文片段,"rule":说明}
//! 下标都是 Unicode 字符（不是字节、不是 UTF-16），前端要按码点计算。
use crate::{ai, library, reader, AppState};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use tauri::{AppHandle, State};

type R<T> = std::result::Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

/// 内置规则：(kind, pattern, 说明)。pattern 作用于归一化后的文本（小写、无空白和标点）
const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "regex",
        r"(?:(?:www|m|wap)\.)?[a-z0-9\p{Han}]{1,30}\.(?:com|net|cn|org|cc|top|xyz|info|vip|me|la|co|io|tv|site|club)",
        "网址或域名（含变形）",
    ),
    (
        "regex",
        r"(?:请|别)?(?:记住|牢记|记得|别忘了)(?:本站|本书|我们|我们的|本)?(?:网址|域名|网站)",
        "请记住网址",
    ),
    ("line", r"本书首发|本站首发", "站点首发水印"),
    ("regex", r"首发[\p{Han}]{0,6}(?:小说网|书屋|书城|书院|文学网|阅读网)", "站点首发水印"),
    ("regex", r"(?:加入?|进入?|扣)?(?:群号|q{1,2}群|扣扣群|企鹅群)\d{0,12}", "QQ 群"),
    ("regex", r"公众号|微信公众|关注微信|加微信|添加微信|微信号|vx号?", "公众号或微信"),
    ("regex", r"求(?:月票|推荐票|收藏|推荐|订阅|打赏|票票)", "求票求订阅"),
    (
        "regex",
        r"手机(?:用户|端|版)?(?:请|可以|到|去|上)?(?:阅读|访问|看书|站)|手机阅读",
        "手机阅读提示",
    ),
    ("regex", r"更新最快|无弹窗|免费看全本|看最新章节|全文免费阅读|txt下载|最新章节请", "站点宣传"),
];

/// 章首 / 章尾各看几段
const EDGE: usize = 2;
/// 频率统计时句子至少这么长（归一化、去数字之后）
const MIN_KEY: usize = 8;
/// “整段都是广告”的 line 规则只对不超过这么长的段落生效
const LINE_MAX: usize = 120;
/// 交给 AI 的可疑段落上限
const AI_MAX: usize = 300;

static RE_URL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:(?:www|m|wap)\.)?[a-z0-9\p{Han}]{1,30}\.(?:com|net|cn|org|cc|top|xyz|info|vip|me|la|co|io|tv|site|club)").unwrap());
static RE_DIGITS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d{6,}").unwrap());
/// 只用来决定“把握高 / 需确认”，不参与识别
const SIGNAL_WORDS: &[&str] = &[
    "首发",
    "网址",
    "公众号",
    "微信",
    "求月票",
    "求推荐",
    "书屋",
    "小说网",
    "手机阅读",
    "关注",
    "加群",
    "免费看",
];

/// 启动时建表（表在 db.rs 里已建好）；写入内置规则。已存在则跳过，用户停用的状态保留
pub fn init(conn: &Connection) -> Result<()> {
    for (kind, pattern, note) in BUILTIN {
        conn.execute(
            "INSERT INTO novel_rules(scope,kind,pattern,note,enabled,source)
             SELECT 'global',?1,?2,?3,1,'builtin' WHERE NOT EXISTS(SELECT 1 FROM novel_rules WHERE scope='global' AND source='builtin' AND pattern=?2)",
            params![kind, pattern, note],
        )?;
    }
    Ok(())
}

/// 启动时调用一次（目前不需要后台任务）
pub fn setup(_app: &tauri::AppHandle) {}

// ---------- 归一化 ----------

/// 删掉的标点、符号（归一化时不保留）
const DROP: &str = "，,、；;：:！!？?“”\"'‘’「」『』（）()【】[]《》<>〈〉—–-―~～·•・*＊_＿|｜/\\#＃＆&@＠%％$＄^＾`＋+=＝…。";

fn fold_char(c: char) -> Option<char> {
    let c = match c {
        '\u{3000}' => ' ',
        // 全角 ASCII（含全角句点 ． 与全角 ！ 等）转半角
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        _ => c,
    };
    // 谐音 / 形近映射：变形网址靠它还原
    let c = match c {
        '点' | '點' => '.',
        '○' | '〇' | '◯' | 'ο' | 'о' => 'o',
        'а' => 'a',
        'е' => 'e',
        'с' => 'c',
        'р' => 'p',
        'х' => 'x',
        'і' => 'i',
        'у' => 'y',
        _ => c,
    };
    if c.is_whitespace() || DROP.contains(c) {
        return None;
    }
    Some(c.to_ascii_lowercase())
}

/// 归一化：返回（归一化字符，每个归一化字符对应的原文字符下标）
pub fn normalize_map(s: &str) -> (Vec<char>, Vec<usize>) {
    let mut out = Vec::new();
    let mut map = Vec::new();
    for (i, c) in s.chars().enumerate() {
        if let Some(f) = fold_char(c) {
            out.push(f);
            map.push(i);
        }
    }
    (out, map)
}

pub fn normalize(s: &str) -> String {
    normalize_map(s).0.into_iter().collect()
}

/// 一段归一化后的文本，带回原文下标的映射
struct Norm {
    chars: Vec<char>,
    map: Vec<usize>,
}

fn norm_of(p: &str) -> Norm {
    let (chars, map) = normalize_map(p);
    Norm { chars, map }
}

/// 句子用于频率统计的键：归一化并去掉数字（章节号、字数之类每章都不同）
fn freq_key(text: &str) -> String {
    normalize(text).chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// 命中是否“看起来像广告”：只影响把握度，不影响能否识别
fn signal(text: &str) -> bool {
    let n = normalize(text);
    RE_URL.is_match(&n) || RE_DIGITS.is_match(&n) || SIGNAL_WORDS.iter().any(|w| n.contains(w))
}

fn symbol_count(p: &str) -> usize {
    p.chars()
        .filter(|c| c.is_ascii_punctuation() || "＃＆＠％＄＾＊＋＝＿｜※☆★◆■▲◎●▼▽◇".contains(*c))
        .count()
}

// ---------- 规则编译与匹配 ----------

/// 把规则编译成正则。regex / line 直接用；contains 先归一化再转义。统一大小写不敏感
fn compile_pattern(kind: &str, pattern: &str) -> Result<Regex> {
    let p = pattern.trim();
    if p.is_empty() {
        return Err(anyhow!("规则内容不能为空"));
    }
    if p.chars().count() > 300 {
        return Err(anyhow!("规则太长（超过 300 字）"));
    }
    let src = match kind {
        "regex" | "line" => format!("(?i){p}"),
        "contains" => {
            let n = normalize(p);
            if n.is_empty() {
                return Err(anyhow!("规则归一化后为空"));
            }
            format!("(?i){}", regex::escape(&n))
        }
        _ => return Err(anyhow!("未知的规则类型：{kind}")),
    };
    Regex::new(&src).map_err(|err| anyhow!("规则无法编译：{err}"))
}

#[derive(Debug, Clone)]
struct Compiled {
    id: Option<i64>,
    kind: String,
    source: String,
    note: String,
    re: Regex,
}

/// 当前启用的规则：全局 + 本书。freq 来源的规则只用来记录，不参与匹配
fn load_rules(conn: &Connection, file_id: i64) -> Result<Vec<Compiled>> {
    let scope = format!("book:{file_id}");
    let mut st =
        conn.prepare("SELECT id,kind,pattern,note,source FROM novel_rules WHERE enabled=1 AND source!='freq' AND (scope='global' OR scope=?1) ORDER BY id")?;
    let rows: Vec<(i64, String, String, String, String)> = st
        .query_map([scope], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<String>>(3)?.unwrap_or_default(), r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = vec![];
    for (id, kind, pattern, note, source) in rows {
        // 个别规则编译失败不影响其他规则
        if let Ok(re) = compile_pattern(&kind, &pattern) {
            out.push(Compiled {
                id: Some(id),
                kind,
                source,
                note,
                re,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
fn builtin_compiled() -> Vec<Compiled> {
    BUILTIN
        .iter()
        .map(|(kind, pattern, note)| Compiled {
            id: None,
            kind: kind.to_string(),
            source: "builtin".into(),
            note: note.to_string(),
            re: compile_pattern(kind, pattern).unwrap(),
        })
        .collect()
}

/// 规则文件（.em）同步过来的规则，source 为 `em:<规则 id>`：只读，跟着规则文件启用 / 停用 / 删除
pub const EM_SOURCE: &str = "em:";

/// 检查一条规则能不能用（规则文件导入前校验）
pub fn check_pattern(kind: &str, pattern: &str) -> std::result::Result<(), String> {
    compile_pattern(kind, pattern).map(|_| ()).map_err(|err| err.to_string())
}

/// 一条规则在一段文字里命中的区间（字符下标），和扫描时一样：先归一化再匹配，line 规则整段删除
pub fn spans_for(kind: &str, pattern: &str, para: &str) -> Result<Vec<(usize, usize)>> {
    let re = compile_pattern(kind, pattern)?;
    let n = norm_of(para);
    let plen = para.chars().count();
    let mut v: Vec<(usize, usize)> = find_spans(&re, &n)
        .into_iter()
        .map(|(s, e)| if kind == "line" && n.chars.len() <= LINE_MAX { (0, plen) } else { (s, e) })
        .collect();
    v.dedup();
    Ok(v)
}

/// 在一段文字里找出规则命中的原文区间（字符下标，左闭右开）
fn find_spans(re: &Regex, n: &Norm) -> Vec<(usize, usize)> {
    if n.chars.is_empty() {
        return vec![];
    }
    let text: String = n.chars.iter().collect();
    let mut out = vec![];
    for m in re.find_iter(&text) {
        if m.end() <= m.start() {
            continue;
        }
        let ns = text[..m.start()].chars().count();
        let ne = ns + text[m.start()..m.end()].chars().count();
        out.push((n.map[ns], n.map[ne - 1] + 1));
    }
    out
}

/// 原文段落的一段文字（字符下标）
fn slice_chars(p: &str, s: usize, e: usize) -> String {
    p.chars().skip(s).take(e.saturating_sub(s)).collect()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Hit {
    /// novel_cleanups 的行号；没有写入数据库时为空
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub chapter: usize,
    #[serde(default)]
    pub para: usize,
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub end: usize,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub rule_id: Option<i64>,
    /// hi（自动处理）/ mid（待确认）
    #[serde(default)]
    pub confidence: String,
    /// applied / pending / reverted / new（找相似的结果还没写入）
    #[serde(default)]
    pub status: String,
}

impl Hit {
    fn key(&self) -> (usize, usize, usize, usize) {
        (self.chapter, self.para, self.start, self.end)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Packed {
    #[serde(default)]
    para: usize,
    #[serde(default)]
    start: usize,
    #[serde(default)]
    end: usize,
    #[serde(default)]
    text: String,
    #[serde(default)]
    rule: String,
}

fn pack(h: &Hit) -> String {
    serde_json::to_string(&Packed {
        para: h.para,
        start: h.start,
        end: h.end,
        text: h.text.clone(),
        rule: h.rule.clone(),
    })
    .unwrap_or_default()
}

fn conf_for(source: &str, text: &str) -> &'static str {
    if source == "builtin" || source == "user" || source.starts_with(EM_SOURCE) || signal(text) {
        "hi"
    } else {
        "mid"
    }
}

fn dedupe(v: Vec<Hit>) -> Vec<Hit> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|h| seen.insert(h.key())).collect()
}

/// 第一层：规则匹配
fn rule_hits(book: &reader::Book, rules: &[Compiled]) -> Vec<Hit> {
    let mut out = vec![];
    for (ci, ch) in book.chapters.iter().enumerate() {
        for (pi, p) in ch.paras.iter().enumerate() {
            let n = norm_of(p);
            if n.chars.is_empty() {
                continue;
            }
            let plen = p.chars().count();
            for r in rules {
                for (s, e) in find_spans(&r.re, &n) {
                    // line 规则：整段都是广告（段落不长时才整段删除）
                    let (s, e) = if r.kind == "line" && n.chars.len() <= LINE_MAX { (0, plen) } else { (s, e) };
                    let text = slice_chars(p, s, e);
                    out.push(Hit {
                        id: None,
                        chapter: ci,
                        para: pi,
                        start: s,
                        end: e,
                        confidence: conf_for(&r.source, &text).into(),
                        text,
                        rule: if r.note.is_empty() {
                            format!("规则 #{}", r.id.unwrap_or(0))
                        } else {
                            r.note.clone()
                        },
                        rule_id: r.id,
                        status: String::new(),
                    });
                }
            }
        }
    }
    dedupe(out)
}

// ---------- 频率统计 ----------

/// 句子切分（字符下标）。句末标点留在句子里
fn split_sentences(p: &str) -> Vec<(usize, usize)> {
    const END: &str = "。！？!?；;…";
    let mut out = vec![];
    let mut start = 0;
    let mut n = 0;
    for c in p.chars() {
        n += 1;
        if END.contains(c) {
            out.push((start, n));
            start = n;
        }
    }
    if start < n {
        out.push((start, n));
    }
    out.into_iter()
        .filter(|&(s, e)| p.chars().skip(s).take(e - s).any(|c| !c.is_whitespace()))
        .collect()
}

#[derive(Debug, Clone)]
pub struct FreqCluster {
    /// 代表句（归一化、去数字，截断到 200 字）
    pub pattern: String,
    /// 在多少章的章首章尾出现
    pub support: usize,
    /// 出现得足够多，且带有广告特征：自动处理
    pub strong: bool,
    /// (章, 段, 起点, 终点)
    pub spans: Vec<(usize, usize, usize, usize)>,
}

fn uf_find(p: &mut [usize], x: usize) -> usize {
    let mut x = x;
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}

fn bigrams(s: &str) -> HashSet<(char, char)> {
    let c: Vec<char> = s.chars().collect();
    c.windows(2).map(|w| (w[0], w[1])).collect()
}

/// 章首章尾的句子在大量章节里反复出现，判定为水印（不依赖关键词）。
/// 近似句（二元组 Dice 系数 ≥ 0.8）会被合并成一簇再统计
pub fn freq_clusters(book: &reader::Book) -> Vec<FreqCluster> {
    let total = book.chapters.len();
    if total < 3 {
        return vec![];
    }
    struct Sent {
        ci: usize,
        pi: usize,
        s: usize,
        e: usize,
        kid: usize,
    }
    let mut sents: Vec<Sent> = vec![];
    let mut key_ids: HashMap<String, usize> = HashMap::new();
    let mut keys: Vec<String> = vec![];
    for (ci, ch) in book.chapters.iter().enumerate() {
        let n = ch.paras.len();
        let idx: BTreeSet<usize> = (0..EDGE.min(n)).chain(n.saturating_sub(EDGE)..n).collect();
        for pi in idx {
            let p = &ch.paras[pi];
            for (s, e) in split_sentences(p) {
                let key = freq_key(&slice_chars(p, s, e));
                if key.chars().count() < MIN_KEY {
                    continue;
                }
                let kid = match key_ids.get(&key) {
                    Some(&k) => k,
                    None => {
                        keys.push(key.clone());
                        key_ids.insert(key, keys.len() - 1);
                        keys.len() - 1
                    }
                };
                sents.push(Sent { ci, pi, s, e, kid });
            }
        }
    }
    let nk = keys.len();
    // 每个键出现在哪些章
    let mut key_chapters: Vec<HashSet<usize>> = vec![HashSet::new(); nk];
    for s in &sents {
        key_chapters[s.kid].insert(s.ci);
    }
    // 近似合并：二元组倒排索引 + Dice 系数
    let grams: Vec<HashSet<(char, char)>> = keys.iter().map(|k| bigrams(k)).collect();
    let mut index: HashMap<(char, char), Vec<usize>> = HashMap::new();
    for k in 0..nk {
        if keys[k].chars().count() <= 80 {
            for b in &grams[k] {
                index.entry(*b).or_default().push(k);
            }
        }
    }
    let mut parent: Vec<usize> = (0..nk).collect();
    for k in 0..nk {
        if keys[k].chars().count() > 80 || grams[k].is_empty() {
            continue;
        }
        let mut cnt: HashMap<usize, usize> = HashMap::new();
        for b in &grams[k] {
            if let Some(post) = index.get(b) {
                if post.len() <= 300 {
                    for &j in post {
                        if j > k {
                            *cnt.entry(j).or_default() += 1;
                        }
                    }
                }
            }
        }
        for (j, c) in cnt {
            let dice = 2.0 * c as f64 / (grams[k].len() + grams[j].len()) as f64;
            if dice >= 0.8 {
                let (a, b) = (uf_find(&mut parent, k), uf_find(&mut parent, j));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    // 按簇统计
    let root_of: Vec<usize> = (0..nk).map(|k| uf_find(&mut parent, k)).collect();
    let mut group_chapters: HashMap<usize, HashSet<usize>> = HashMap::new();
    let mut group_keys: HashMap<usize, Vec<usize>> = HashMap::new();
    for k in 0..nk {
        let r = uf_find(&mut parent, k);
        group_chapters.entry(r).or_default().extend(key_chapters[k].iter().copied());
        group_keys.entry(r).or_default().push(k);
    }
    let need = ((total as f64) * 0.2).ceil().max(3.0) as usize;
    let mut out = vec![];
    for (root, chs) in group_chapters {
        let support = chs.len();
        if support < need {
            continue;
        }
        let members = &group_keys[&root];
        // 代表句：出现章数最多的键，并列时取较短的
        let rep = members
            .iter()
            .copied()
            .max_by_key(|&k| (key_chapters[k].len(), std::cmp::Reverse(keys[k].chars().count())))
            .unwrap_or(root);
        let pattern: String = keys[rep].chars().take(200).collect();
        let spans: Vec<(usize, usize, usize, usize)> = sents.iter().filter(|s| root_of[s.kid] == root).map(|s| (s.ci, s.pi, s.s, s.e)).collect();
        let strong = support * 2 >= total && signal(&pattern);
        out.push(FreqCluster {
            pattern,
            support,
            strong,
            spans,
        });
    }
    out.sort_by(|a, b| b.support.cmp(&a.support).then_with(|| a.pattern.cmp(&b.pattern)));
    out
}

// ---------- 找相似（用户标记之后） ----------

/// 在全书里找和 sample 近似的原文。返回 (章, 段, 起点, 终点, 原文, 是否高把握)
pub fn similar_spans(book: &reader::Book, sample: &str) -> Vec<(usize, usize, usize, usize, String, bool)> {
    let sn: Vec<char> = normalize_map(sample).0;
    let m = sn.len();
    if m < 4 {
        return vec![];
    }
    let sb: HashSet<(char, char)> = sn.windows(2).map(|w| (w[0], w[1])).collect();
    // 连续覆盖至少这么多字（短样本要求几乎全中）
    let need = std::cmp::max(m.min(6), (m as f64 * 0.6).ceil() as usize);
    let mut out = vec![];
    for (ci, ch) in book.chapters.iter().enumerate() {
        for (pi, p) in ch.paras.iter().enumerate() {
            let n = norm_of(p);
            let len = n.chars.len();
            if len < need {
                continue;
            }
            let mut covered = vec![false; len];
            for i in 0..len.saturating_sub(1) {
                if sb.contains(&(n.chars[i], n.chars[i + 1])) {
                    covered[i] = true;
                    covered[i + 1] = true;
                }
            }
            // 把被覆盖的字连成段；中间最多隔 1 个字（容忍一个多余或缺失的字）
            let mut runs: Vec<(usize, usize)> = vec![];
            let mut cur: Option<(usize, usize)> = None;
            for k in 0..len {
                if !covered[k] {
                    continue;
                }
                cur = match cur {
                    Some((rs, last)) if k - last <= 2 => Some((rs, k)),
                    Some(r) => {
                        runs.push(r);
                        Some((k, k))
                    }
                    None => Some((k, k)),
                };
            }
            if let Some(r) = cur {
                runs.push(r);
            }
            for (rs, last) in runs {
                let rl = last + 1 - rs;
                if rl < need {
                    continue;
                }
                // 这一段里要覆盖样本的大部分二元组，避免常见字碰巧连成一片
                let got: HashSet<(char, char)> = (rs..last).map(|i| (n.chars[i], n.chars[i + 1])).filter(|b| sb.contains(b)).collect();
                if (got.len() as f64) < 0.6 * sb.len() as f64 {
                    continue;
                }
                let (s, e) = (n.map[rs], n.map[last] + 1);
                out.push((ci, pi, s, e, slice_chars(p, s, e), rl * 10 >= m * 9));
            }
        }
    }
    out
}

// ---------- 文本处理 ----------

/// 删掉若干区间（字符下标），保留其余正文；删完留下的空括号一并去掉
pub fn remove_ranges(p: &str, ranges: &[(usize, usize)]) -> String {
    let chars: Vec<char> = p.chars().collect();
    let mut keep = vec![true; chars.len()];
    for &(s, e) in ranges {
        for k in s.min(chars.len())..e.min(chars.len()) {
            keep[k] = false;
        }
    }
    let s: String = chars.iter().enumerate().filter(|(k, _)| keep[*k]).map(|(_, c)| *c).collect();
    let mut t = s;
    for pat in ["（）", "()", "【】", "[]", "《》"] {
        t = t.replace(pat, "");
    }
    t
}

/// 生成干净的 TXT：章节标题 + 正文，已确认的广告不输出
pub fn export_text(book: &reader::Book, applied: &[Hit]) -> String {
    export_text_with(book, applied, "　　", 0)
}

/// 同上，段首缩进和段间空行可以指定（小说排版规则）。先删广告再排版，清理记录的下标不受影响
pub fn export_text_with(book: &reader::Book, applied: &[Hit], indent: &str, blank_lines: usize) -> String {
    let mut out = String::new();
    for (ci, ch) in book.chapters.iter().enumerate() {
        let title = ch.title.trim();
        if !title.is_empty() {
            out.push_str(title);
            out.push_str("\n\n");
        }
        for (pi, p) in ch.paras.iter().enumerate() {
            let ranges: Vec<(usize, usize)> = applied.iter().filter(|h| h.chapter == ci && h.para == pi).map(|h| (h.start, h.end)).collect();
            let t = if ranges.is_empty() {
                p.trim().to_string()
            } else {
                remove_ranges(p, &ranges).trim().to_string()
            };
            if t.is_empty() {
                continue;
            }
            out.push_str(indent);
            out.push_str(&t);
            out.push('\n');
            for _ in 0..blank_lines {
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

// ---------- AI 归纳规则 ----------

const AI_SYS: &str = "你是中文网络小说的文本净化助手。输入是从一本小说里挑出的段落，每条以 [编号] 开头；“前”“后”是相邻段落，供你判断上下文。\
请找出与正文无关的内容：网址或域名（包括用全角、空格、谐音字伪装的）、站点水印（如“本书首发”“请记住网址”）、拉群、关注公众号、求票求订阅、防盗乱码（无意义的符号串）等。\
正常的剧情、对白和叙述不是广告。\
只输出一个 JSON 对象，格式：{\"ads\":[编号,...],\"rules\":[{\"kind\":\"regex 或 contains 或 line\",\"pattern\":\"...\",\"note\":\"中文说明\"}]}。\
规则要能覆盖同类变体但不能匹配正文。规则会作用于“去掉空白和标点、全角转半角、转小写”之后的文本：regex 是 Rust 正则（可用 \\p{Han}、\\d、(?:...)，不要用前后查找和反向引用）；contains 是普通子串；line 是整段都是广告时用的正则。不要输出其他文字。";

fn cut(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// 可疑段落：含网址、长数字（QQ 号）、较多符号、或含广告常用词
fn suspicious(book: &reader::Book) -> Vec<(usize, usize)> {
    let mut v = vec![];
    for (ci, ch) in book.chapters.iter().enumerate() {
        for (pi, p) in ch.paras.iter().enumerate() {
            let n = normalize(p);
            if RE_URL.is_match(&n) || RE_DIGITS.is_match(&n) || symbol_count(p) >= 3 || SIGNAL_WORDS.iter().any(|w| n.contains(w)) {
                v.push((ci, pi));
                if v.len() >= AI_MAX {
                    return v;
                }
            }
        }
    }
    v
}

/// 发给 AI 的内容。samples 给了就只用样本；否则用本地挑出的可疑段落（带前后各一段）
fn build_payload(book: &reader::Book, samples: Option<&[String]>) -> (Vec<(usize, usize)>, String) {
    let mut lines = vec![];
    let mut flagged = vec![];
    match samples {
        Some(list) => {
            for (k, s) in list.iter().take(AI_MAX).enumerate() {
                lines.push(format!("[{k}] {}", cut(s, 200)));
            }
        }
        None => {
            for (ci, pi) in suspicious(book) {
                let paras = &book.chapters[ci].paras;
                let prev = if pi > 0 { cut(&paras[pi - 1], 120) } else { String::new() };
                let next = paras.get(pi + 1).map(|x| cut(x, 120)).unwrap_or_default();
                let k = flagged.len();
                lines.push(format!("[{k}] 前：{prev}\n    本段：{}\n    后：{next}", cut(&paras[pi], 200)));
                flagged.push((ci, pi));
            }
        }
    }
    (flagged, lines.join("\n"))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuleRow {
    #[serde(default)]
    pub id: i64,
    pub scope: String,
    pub kind: String,
    pub pattern: String,
    #[serde(default)]
    pub note: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub source: String,
    /// 命中的段落数（AI 识别时返回）
    #[serde(default)]
    pub hits: usize,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Clone, Debug)]
pub struct Estimate {
    /// 预计消耗的 token（系统提示 + 可疑段落）
    pub tokens: u64,
    /// 将发送的段落数
    pub samples: usize,
    /// 全书段落数
    pub paragraphs: usize,
}

// ---------- 数据库 ----------

fn open_conn(db: &Path) -> Result<Connection> {
    let conn = Connection::open(db)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(conn)
}

/// 耗时操作放到线程里跑，各自开连接，不占用主连接的锁
async fn blocking<T, F>(st: &State<'_, AppState>, f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce(&Path) -> Result<T> + Send + 'static,
{
    let db = st.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || f(&db)).await.map_err(e)?.map_err(e)
}

fn load_rows(conn: &Connection, file_id: i64) -> Result<Vec<Hit>> {
    let mut st = conn.prepare("SELECT id,chapter,rule_id,text,confidence,status FROM novel_cleanups WHERE file_id=?1 ORDER BY chapter,id")?;
    let rows: Vec<(i64, i64, Option<i64>, String, String, String)> = st
        .query_map([file_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, ch, rule_id, text, confidence, status)| {
            let p: Packed = serde_json::from_str(&text).unwrap_or_default();
            Hit {
                id: Some(id),
                chapter: ch as usize,
                para: p.para,
                start: p.start,
                end: p.end,
                text: p.text,
                rule: p.rule,
                rule_id,
                confidence,
                status,
            }
        })
        .collect())
}

fn insert_row(conn: &Connection, file_id: i64, h: &Hit, status: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO novel_cleanups(file_id,chapter,text,rule_id,confidence,status) VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            file_id,
            h.chapter as i64,
            pack(h),
            h.rule_id,
            if h.confidence.is_empty() { "hi" } else { &h.confidence },
            status
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 频率统计得到的规则：同一代表句只保留一条，返回 (规则 id, 是否启用)
fn upsert_freq_rule(conn: &Connection, file_id: i64, c: &FreqCluster) -> Result<(i64, bool)> {
    let scope = format!("book:{file_id}");
    let found = conn
        .query_row(
            "SELECT id,enabled FROM novel_rules WHERE scope=?1 AND source='freq' AND pattern=?2",
            params![scope, c.pattern],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    if let Some((id, enabled)) = found {
        return Ok((id, enabled != 0));
    }
    conn.execute(
        "INSERT INTO novel_rules(scope,kind,pattern,note,enabled,source) VALUES(?1,'contains',?2,?3,1,'freq')",
        params![scope, c.pattern, format!("{} 章的章首或章尾重复出现", c.support)],
    )?;
    Ok((conn.last_insert_rowid(), true))
}

/// 打开书：和阅读器看到的一样（套用了生效的小说排版规则），清理记录的下标都以它为准
fn book_of(conn: &Connection, id: i64) -> Result<(String, reader::Book)> {
    let (path, opened) = opened_of(conn, id)?;
    Ok((path, opened.book))
}

fn opened_of(conn: &Connection, id: i64) -> Result<(String, crate::em::clean::Opened)> {
    let path = library::file_path(conn, id)?;
    let opened = crate::em::clean::open_book(conn, id, &path)?;
    Ok((path, opened))
}

/// 清理记录对不上当前的文字时（排版规则改了、启用或停用了，原文件被改过），按记录的原文片段重新定位。
/// 同样的文字出现多处时，取离原位置最近的一处；找不到的保持原样（阅读器和导出都会忽略对不上的记录）。
/// 返回重新定位的条数
pub fn relocate_cleanups(conn: &Connection, file_id: i64, book: &reader::Book) -> Result<usize> {
    let rows = load_rows(conn, file_id)?;
    if rows.is_empty() {
        return Ok(0);
    }
    let paras: Vec<Vec<char>> = book.chapters.iter().flat_map(|c| c.paras.iter().map(|p| p.chars().collect())).collect();
    // 每章第一段在 paras 里的位置
    let mut first = vec![];
    let mut k = 0;
    for c in &book.chapters {
        first.push(k);
        k += c.paras.len();
    }
    let flat = |ch: usize, para: usize| -> Option<usize> {
        let c = book.chapters.get(ch)?;
        (para < c.paras.len()).then(|| first[ch] + para)
    };
    let fits = |h: &Hit| -> bool {
        flat(h.chapter, h.para).is_some_and(|f| {
            let p = &paras[f];
            h.start < h.end && h.end <= p.len() && p[h.start..h.end].iter().collect::<String>() == h.text
        })
    };
    let (ok, stale): (Vec<Hit>, Vec<Hit>) = rows.into_iter().partition(|h| fits(h));
    if stale.is_empty() || paras.is_empty() {
        return Ok(0);
    }
    let mut taken: HashSet<(usize, usize, usize)> = ok.iter().filter_map(|h| flat(h.chapter, h.para).map(|f| (f, h.start, h.end))).collect();
    // 原来大致在第几段（章节重新划分过时只是估计）
    let nch = book.chapters.len();
    let estimate = |h: &Hit| -> usize {
        let ch = h.chapter.min(nch - 1);
        first[ch] + h.para.min(book.chapters[ch].paras.len().saturating_sub(1))
    };
    let mut moved = 0;
    for h in stale {
        let needle: Vec<char> = h.text.chars().collect();
        if needle.is_empty() {
            continue;
        }
        let est = estimate(&h);
        let mut best: Option<(usize, usize, usize)> = None;
        for (f, p) in paras.iter().enumerate() {
            if p.len() < needle.len() {
                continue;
            }
            for s in 0..=p.len() - needle.len() {
                if p[s..s + needle.len()] == needle[..]
                    && !taken.contains(&(f, s, s + needle.len()))
                    && best.map_or(true, |(bf, _, _)| f.abs_diff(est) < bf.abs_diff(est))
                {
                    best = Some((f, s, s + needle.len()));
                }
            }
        }
        let Some((f, s, en)) = best else { continue };
        taken.insert((f, s, en));
        // 空章节会让 first 里有重复值，取最后一个不大于 f 的，正好是有这一段的章
        let ch = first.partition_point(|&x| x <= f) - 1;
        let nh = Hit {
            chapter: ch,
            para: f - first[ch],
            start: s,
            end: en,
            ..h.clone()
        };
        conn.execute(
            "UPDATE novel_cleanups SET chapter=?1, text=?2 WHERE id=?3",
            params![nh.chapter as i64, pack(&nh), h.id],
        )?;
        moved += 1;
    }
    Ok(moved)
}

// ---------- 命令 ----------

/// 对整本书跑第一层（规则 + 频率统计）。hi 直接记为已清理，mid 记为待确认。
/// 之前已确认或撤销过的同一处不会被改回去
#[tauri::command]
pub async fn adclean_scan(st: State<'_, AppState>, id: i64) -> R<Vec<Hit>> {
    blocking(&st, move |db| {
        let mut conn = open_conn(db)?;
        let (_, book) = book_of(&conn, id)?;
        let rules = load_rules(&conn, id)?;
        let mut hits = rule_hits(&book, &rules);
        for c in freq_clusters(&book) {
            let (rid, enabled) = upsert_freq_rule(&conn, id, &c)?;
            if !enabled {
                continue;
            }
            let rule = format!("频率统计：{} 章章首章尾重复", c.support);
            for (ci, pi, s, en) in c.spans {
                let text = slice_chars(&book.chapters[ci].paras[pi], s, en);
                hits.push(Hit {
                    id: None,
                    chapter: ci,
                    para: pi,
                    start: s,
                    end: en,
                    confidence: if c.strong && signal(&text) { "hi" } else { "mid" }.into(),
                    text,
                    rule: rule.clone(),
                    rule_id: Some(rid),
                    status: String::new(),
                });
            }
        }
        let hits = dedupe(hits);
        let tx = conn.transaction()?;
        // 待确认的旧结果作废，重新生成
        tx.execute("DELETE FROM novel_cleanups WHERE file_id=?1 AND status='pending'", [id])?;
        let existing: HashMap<(usize, usize, usize, usize), Hit> = load_rows(&tx, id)?.into_iter().map(|h| (h.key(), h)).collect();
        let mut out = vec![];
        for mut h in hits {
            if let Some(ex) = existing.get(&h.key()) {
                h.id = ex.id;
                h.status = ex.status.clone();
            } else {
                let status = if h.confidence == "hi" { "applied" } else { "pending" };
                h.id = Some(insert_row(&tx, id, &h, status)?);
                h.status = status.into();
            }
            out.push(h);
        }
        tx.commit()?;
        Ok(out)
    })
    .await
}

/// 用户标记了一段广告：在全书找近似内容。只返回结果，不写库，等用户确认后再 adclean_apply
#[tauri::command]
pub async fn adclean_find_similar(st: State<'_, AppState>, id: i64, sample: String) -> R<Vec<Hit>> {
    if normalize(&sample).chars().count() < 4 {
        return Err("标记的文字太短，至少需要 4 个字".into());
    }
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let (_, book) = book_of(&conn, id)?;
        let existing: HashMap<(usize, usize, usize, usize), Hit> = load_rows(&conn, id)?.into_iter().map(|h| (h.key(), h)).collect();
        let label = format!("相似：{}", cut(&sample, 12));
        let mut out = vec![];
        for (ci, pi, s, en, text, strong) in similar_spans(&book, &sample) {
            let mut h = Hit {
                id: None,
                chapter: ci,
                para: pi,
                start: s,
                end: en,
                text,
                rule: label.clone(),
                rule_id: None,
                confidence: if strong { "hi" } else { "mid" }.into(),
                status: "new".into(),
            };
            if let Some(ex) = existing.get(&h.key()) {
                h.id = ex.id;
                h.status = ex.status.clone();
            }
            out.push(h);
        }
        Ok(out)
    })
    .await
}

/// AI 识别前估算用量：发送多少段、大约多少 token
#[tauri::command]
pub async fn adclean_estimate(st: State<'_, AppState>, id: i64) -> R<Estimate> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let (_, book) = book_of(&conn, id)?;
        let paragraphs = book.chapters.iter().map(|c| c.paras.len()).sum();
        let (flagged, payload) = build_payload(&book, None);
        Ok(Estimate {
            tokens: ai::estimate_tokens(AI_SYS) + ai::estimate_tokens(&payload),
            samples: flagged.len(),
            paragraphs,
        })
    })
    .await
}

/// AI 识别：把可疑段落（或给定的样本）交给 AI，归纳出规则。
/// 规则必须能编译，并且真的命中了某个可疑段落，且没有铺得太广，才会保存为本书规则
#[tauri::command]
pub async fn adclean_ai_rules(app: AppHandle, st: State<'_, AppState>, id: i64, samples: Option<Vec<String>>) -> R<Vec<RuleRow>> {
    let (flagged, payload, book) = blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let (_, book) = book_of(&conn, id)?;
        let (flagged, payload) = build_payload(&book, samples.as_deref());
        Ok((flagged, payload, book))
    })
    .await?;
    if payload.is_empty() {
        return Err("没有需要 AI 判断的可疑片段".into());
    }
    let reply = ai::chat_simple(&app, AI_SYS, &payload, true).await.map_err(e)?;
    let v = ai::extract_json(&reply).map_err(e)?;
    let ads: HashSet<usize> = v["ads"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect())
        .unwrap_or_default();
    let rules: Vec<(String, String, String)> = v["rules"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| {
                    (
                        r["kind"].as_str().unwrap_or("").to_string(),
                        r["pattern"].as_str().unwrap_or("").to_string(),
                        r["note"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let scope = format!("book:{id}");
        let total: usize = book.chapters.iter().map(|c| c.paras.len()).sum();
        let limit = std::cmp::max(30, total / 50);
        let mut out = vec![];
        for (kind, pattern, note) in rules {
            let Ok(re) = compile_pattern(&kind, &pattern) else { continue };
            let mut matched = 0usize;
            let mut flagged_hit = false;
            for (ci, ch) in book.chapters.iter().enumerate() {
                for (pi, p) in ch.paras.iter().enumerate() {
                    if find_spans(&re, &norm_of(p)).is_empty() {
                        continue;
                    }
                    matched += 1;
                    if let Some(k) = flagged.iter().position(|&x| x == (ci, pi)) {
                        if ads.is_empty() || ads.contains(&k) {
                            flagged_hit = true;
                        }
                    }
                }
            }
            // 没命中任何可疑段落，或者铺得太广（像在删正文），都不要
            if matched == 0 || matched > limit || (!flagged.is_empty() && !flagged_hit) {
                continue;
            }
            conn.execute(
                "INSERT INTO novel_rules(scope,kind,pattern,note,enabled,source)
                 SELECT ?1,?2,?3,?4,1,'ai' WHERE NOT EXISTS(SELECT 1 FROM novel_rules WHERE scope=?1 AND source='ai' AND kind=?2 AND pattern=?3)",
                params![scope, kind, pattern, note],
            )?;
            let (rid, en): (i64, i64) = conn.query_row(
                "SELECT id,enabled FROM novel_rules WHERE scope=?1 AND source='ai' AND kind=?2 AND pattern=?3",
                params![scope, kind, pattern],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            out.push(RuleRow {
                id: rid,
                scope: scope.clone(),
                kind,
                pattern,
                note,
                enabled: en != 0,
                source: "ai".into(),
                hits: matched,
            });
        }
        Ok(out)
    })
    .await
}

/// 确认清理：把给定的命中记为已清理（没有记录的会新建）
#[tauri::command]
pub async fn adclean_apply(st: State<'_, AppState>, id: i64, items: Vec<Hit>) -> R<usize> {
    blocking(&st, move |db| {
        let mut conn = open_conn(db)?;
        let tx = conn.transaction()?;
        let existing: HashMap<(usize, usize, usize, usize), Hit> = load_rows(&tx, id)?.into_iter().map(|h| (h.key(), h)).collect();
        let mut n = 0;
        for mut h in items {
            if h.start >= h.end {
                continue;
            }
            if h.confidence.is_empty() {
                h.confidence = "hi".into();
            }
            match existing.get(&h.key()) {
                Some(ex) => {
                    tx.execute(
                        "UPDATE novel_cleanups SET status='applied', confidence=?1, rule_id=?2, text=?3 WHERE id=?4",
                        params![h.confidence, h.rule_id.or(ex.rule_id), pack(&h), ex.id],
                    )?;
                }
                None => {
                    insert_row(&tx, id, &h, "applied")?;
                }
            }
            n += 1;
        }
        tx.commit()?;
        Ok(n)
    })
    .await
}

/// 撤销：ids 为空表示撤销全部已清理的内容；否则撤销指定的行（待确认的也可以取消）
#[tauri::command]
pub async fn adclean_revert(st: State<'_, AppState>, id: i64, cleanup_ids: Vec<i64>) -> R<usize> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let n = if cleanup_ids.is_empty() {
            conn.execute("UPDATE novel_cleanups SET status='reverted' WHERE file_id=?1 AND status='applied'", [id])?
        } else {
            let mut n = 0;
            for cid in cleanup_ids {
                n += conn.execute("UPDATE novel_cleanups SET status='reverted' WHERE file_id=?1 AND id=?2", params![id, cid])?;
            }
            n
        };
        Ok(n)
    })
    .await
}

/// 列出本书的净化记录（包括待确认和已撤销的）
#[tauri::command]
pub async fn adclean_list(st: State<'_, AppState>, id: i64) -> R<Vec<Hit>> {
    blocking(&st, move |db| load_rows(&open_conn(db)?, id)).await
}

/// 规则列表。scope 为空返回全部；传 book:<id> 时包含全局规则
#[tauri::command]
pub async fn adclean_rules(st: State<'_, AppState>, scope: Option<String>) -> R<Vec<RuleRow>> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let map = |r: &rusqlite::Row| -> rusqlite::Result<RuleRow> {
            Ok(RuleRow {
                id: r.get(0)?,
                scope: r.get(1)?,
                kind: r.get(2)?,
                pattern: r.get(3)?,
                note: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                enabled: r.get::<_, i64>(5)? != 0,
                source: r.get(6)?,
                hits: 0,
            })
        };
        let cols = "SELECT id,scope,kind,pattern,note,enabled,source FROM novel_rules";
        let rows: Vec<RuleRow> = match scope {
            Some(s) => conn
                .prepare(&format!("{cols} WHERE scope=?1 OR scope='global' ORDER BY source='builtin' DESC, id"))?
                .query_map([s], map)?
                .collect::<rusqlite::Result<_>>()?,
            None => conn
                .prepare(&format!("{cols} ORDER BY id"))?
                .query_map([], map)?
                .collect::<rusqlite::Result<_>>()?,
        };
        Ok(rows)
    })
    .await
}

const EM_READONLY: &str = "这条规则来自规则文件，要停用请到“设置 › 规则文件”里停用或删除对应的规则文件";

fn rule_source(conn: &Connection, id: i64) -> Result<String> {
    Ok(conn.query_row("SELECT source FROM novel_rules WHERE id=?1", [id], |r| r.get(0))?)
}

/// 停用或启用一条规则
#[tauri::command]
pub async fn adclean_rule_set_enabled(st: State<'_, AppState>, id: i64, enabled: bool) -> R<()> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        if rule_source(&conn, id)?.starts_with(EM_SOURCE) {
            return Err(anyhow!(EM_READONLY));
        }
        conn.execute("UPDATE novel_rules SET enabled=?1 WHERE id=?2", params![enabled as i64, id])?;
        Ok(())
    })
    .await
}

/// 删除一条规则（内置规则只能停用，不能删除）
#[tauri::command]
pub async fn adclean_rule_delete(st: State<'_, AppState>, id: i64) -> R<()> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let src = rule_source(&conn, id)?;
        if src == "builtin" {
            return Err(anyhow!("内置规则只能停用，不能删除"));
        }
        if src.starts_with(EM_SOURCE) {
            return Err(anyhow!(EM_READONLY));
        }
        conn.execute("DELETE FROM novel_rules WHERE id=?1", [id])?;
        Ok(())
    })
    .await
}

/// 导出非内置规则为 JSON 文件
#[tauri::command]
pub async fn adclean_rules_export(st: State<'_, AppState>, path: String) -> R<usize> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let mut stmt =
            conn.prepare("SELECT id,scope,kind,pattern,note,enabled,source FROM novel_rules WHERE source!='builtin' AND source NOT LIKE 'em:%' ORDER BY id")?;
        let rows: Vec<RuleRow> = stmt
            .query_map([], |r| {
                Ok(RuleRow {
                    id: r.get(0)?,
                    scope: r.get(1)?,
                    kind: r.get(2)?,
                    pattern: r.get(3)?,
                    note: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    enabled: r.get::<_, i64>(5)? != 0,
                    source: r.get(6)?,
                    hits: 0,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        std::fs::write(&path, serde_json::to_string_pretty(&rows)?)?;
        Ok(rows.len())
    })
    .await
}

/// 导入规则 JSON。只接受能编译的规则；导入后统一作为全局的用户规则
#[tauri::command]
pub async fn adclean_rules_import(st: State<'_, AppState>, path: String) -> R<usize> {
    blocking(&st, move |db| {
        let text = std::fs::read_to_string(&path)?;
        let rows: Vec<RuleRow> = serde_json::from_str(&text).map_err(|err| anyhow!("规则文件格式不对：{err}"))?;
        let conn = open_conn(db)?;
        let mut n = 0;
        for r in rows {
            if compile_pattern(&r.kind, &r.pattern).is_err() {
                continue;
            }
            n += conn.execute(
                "INSERT INTO novel_rules(scope,kind,pattern,note,enabled,source)
                 SELECT 'global',?1,?2,?3,?4,'user' WHERE NOT EXISTS(SELECT 1 FROM novel_rules WHERE scope='global' AND kind=?1 AND pattern=?2 AND source='user')",
                params![r.kind, r.pattern, r.note, r.enabled as i64],
            )?;
        }
        Ok(n)
    })
    .await
}

/// 导出一份干净的 TXT 副本（已清理的内容不输出）。原文件不动
#[tauri::command]
pub async fn adclean_export_clean(st: State<'_, AppState>, id: i64, path: String) -> R<usize> {
    blocking(&st, move |db| {
        let conn = open_conn(db)?;
        let (src, opened) = opened_of(&conn, id)?;
        if path.eq_ignore_ascii_case(&src) {
            return Err(anyhow!("不能覆盖原文件，请换一个文件名"));
        }
        let book = &opened.book;
        // 对不上当前文字的记录（重新定位也没找到）不处理，免得删错
        let applied: Vec<Hit> = load_rows(&conn, id)?
            .into_iter()
            .filter(|h| h.status == "applied")
            .filter(|h| {
                book.chapters
                    .get(h.chapter)
                    .and_then(|c| c.paras.get(h.para))
                    .is_some_and(|p| slice_chars(p, h.start, h.end) == h.text)
            })
            .collect();
        let (indent, blank) = opened
            .layout
            .as_ref()
            .map(|l| (l.indent.as_str(), l.blank_lines as usize))
            .unwrap_or(("　　", 0));
        std::fs::write(&path, export_text_with(book, &applied, indent, blank))?;
        Ok(applied.len())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::{Book, Chapter};

    fn book(chapters: Vec<Vec<&str>>) -> Book {
        Book {
            title: "测试".into(),
            author: String::new(),
            encoding: "UTF-8".into(),
            chapters: chapters
                .into_iter()
                .enumerate()
                .map(|(i, paras)| Chapter {
                    title: format!("第{}章 测试", i + 1),
                    paras: paras.into_iter().map(String::from).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn normalize_variant_url() {
        assert_eq!(normalize("ｗｗｗ．ｘｘ○○．ｃｏｍ"), "www.xxoo.com");
        assert_eq!(normalize("w w w 点 xx 点 com"), "www.xx.com");
        assert_eq!(normalize("ｍ．某某书屋．ｅｘａｍｐｌｅ"), "m.某某书屋.example");
    }

    #[test]
    fn variant_url_recognized_and_only_middle_removed() {
        let rules = builtin_compiled();
        let p = "他站在岸边。ｗ ｗ ｗ 点 xx○○ 点 ｃｏｍ 雾很大。";
        let b = book(vec![vec![p]]);
        let hits = rule_hits(&b, &rules);
        assert!(hits.iter().any(|h| h.text.contains("ｃｏｍ")), "{hits:?}");
        let ranges: Vec<(usize, usize)> = hits.iter().map(|h| (h.start, h.end)).collect();
        let out = remove_ranges(p, &ranges);
        assert!(out.contains("他站在岸边"), "{out}");
        assert!(out.contains("雾很大"), "{out}");
        assert!(!out.contains("ｃｏｍ"), "{out}");
    }

    #[test]
    fn sentence_end_is_not_a_domain() {
        let rules = builtin_compiled();
        let p = "我很好。Mexico城的雾很大。";
        let b = book(vec![vec![p]]);
        assert!(rule_hits(&b, &rules).is_empty());
    }

    #[test]
    fn remove_ranges_keeps_surrounding_text() {
        assert_eq!(remove_ranges("前文甲乙丙丁后文", &[(2, 4)]), "前文丙丁后文");
    }

    #[test]
    fn whole_line_watermark_removed_as_paragraph() {
        let rules = builtin_compiled();
        let b = book(vec![vec!["本书首发某某书屋，更新最快，无弹窗。", "正文一句。"]]);
        let hits = rule_hits(&b, &rules);
        let h = hits.iter().find(|h| h.para == 0).expect("应命中首段");
        assert_eq!((h.start, h.end), (0, "本书首发某某书屋，更新最快，无弹窗。".chars().count()));
    }

    #[test]
    fn qq_group_inline_removed() {
        let rules = builtin_compiled();
        let p = "他说：“走吧。”（加QQ群123456）雾很大。";
        let b = book(vec![vec![p]]);
        let hits = rule_hits(&b, &rules);
        let ranges: Vec<(usize, usize)> = hits.iter().map(|h| (h.start, h.end)).collect();
        let out = remove_ranges(p, &ranges);
        assert!(out.contains("雾很大") && out.contains("走吧"), "{out}");
        assert!(!out.contains("QQ") && !out.contains("123456"), "{out}");
    }

    #[test]
    fn freq_detects_tail_watermark() {
        let words = ["灯塔", "渡船", "缆绳", "邮票", "钟表", "海雾", "旧信", "木箱", "铁门", "潮水"];
        let mut chapters = vec![];
        for (i, w) in words.iter().enumerate() {
            chapters.push(vec![
                format!("阿砚第{}次走到{}旁边。", i + 1, w),
                format!("那天晚上的{}格外安静。", w),
                "中间的正文不同".to_string(),
                "本书首发某某书屋，请记住网址：ｗｗｗ．ｘｘｘ．ｃｏｍ！".to_string(),
            ]);
        }
        let b = Book {
            title: "t".into(),
            author: String::new(),
            encoding: "UTF-8".into(),
            chapters: chapters
                .into_iter()
                .enumerate()
                .map(|(i, p)| Chapter {
                    title: format!("第{}章", i + 1),
                    paras: p,
                })
                .collect(),
        };
        let cs = freq_clusters(&b);
        let c = cs.iter().find(|c| c.pattern.contains("请记住网址")).expect("应识别出章尾水印");
        assert_eq!(c.support, 10);
        assert_eq!(c.spans.len(), 10);
        assert!(c.strong);
        assert!(c.spans.iter().all(|s| s.1 == 3));
    }

    #[test]
    fn similar_finds_variant_inline() {
        let b = book(vec![
            vec!["他站在岸边。本书首发 某某书屋，请记住网址！雾很大。", "没有广告的一段。"],
            vec!["另一章的正文。"],
        ]);
        let found = similar_spans(&b, "本书首发某某书屋请记住网址");
        assert_eq!(found.len(), 1, "{found:?}");
        let (ci, pi, s, e, _, _) = &found[0];
        assert_eq!((*ci, *pi), (0, 0));
        let out = remove_ranges(&b.chapters[0].paras[0], &[(*s, *e)]);
        assert!(out.contains("他站在岸边") && out.contains("雾很大"), "{out}");
        assert!(!out.contains("网址"), "{out}");
    }

    #[test]
    fn short_sample_rejected_by_similar() {
        let b = book(vec![vec!["一二三四五六。"]]);
        assert!(similar_spans(&b, "一二").is_empty());
    }
}
