//! 下载源规则（kind: source）：按规则搜索、列分集、解析下载地址，注册成 Provider（id 前缀 em-）。
//!
//! 三个阶段：`search`（必填）→ `episodes`（可选：一部作品 → 各集 / 各章）→ `resolve`（必填，给出下载方式）。
//! 每个阶段是一个“步骤”：请求（request）→ 列表（list）→ 字段（fields）/ 中间值（vars）→ 翻页（next）。
//! 取值写法见 `expr.rs`，联网限制见 `net.rs`，用户文档见 `docs/rules/source.md`。
use super::expr::{self, abs_url, eval, eval_list, parse_expr, parse_tpl, EvalErr, Expr, Node, Page, Tpl, Vars};
use super::format::Login;
use super::net::{self, Allow, Body, Fetch, Fetched, Method, Net, Req};
use super::{Issue, Rule};
use crate::fetch::provider::{self, Item, Provider, ProviderInfo, Quality, Resolved, Target, TextChapter};
use crate::fetch::source::{collapse, LOGIN_MSG};
use futures_util::future::BoxFuture;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tauri::AppHandle;
use url::Url;

/// 注册成下载源时的 id 前缀
pub const ID_PREFIX: &str = "em-";
const DEFAULT_PAGES: u32 = 5;
const MAX_PAGES: u32 = 50;
const DEFAULT_WORKS: u32 = 3;
const MAX_WORKS: u32 = 10;
/// 搜索结果能填的字段（另外可以写 `extra.名字`）
const FIELDS: &[&str] = &[
    "id",
    "title",
    "series",
    "artist",
    "album",
    "year",
    "track",
    "episode",
    "season",
    "cover",
    "duration",
    "media_kind",
    "size",
    "quality",
];
const MEDIA: &[(&str, &str)] = &[("music", "音乐"), ("video", "视频"), ("anime", "番剧"), ("manga", "漫画"), ("novel", "小说")];

// ---------------------------------------------------------------- 规则文件里的写法

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    url: String,
    #[serde(default)]
    method: String,
    #[serde(default)]
    form: BTreeMap<String, Value>,
    #[serde(default)]
    json: Option<Value>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    encoding: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawStep {
    request: Option<RawRequest>,
    list: Option<String>,
    #[serde(default)]
    fields: BTreeMap<String, Value>,
    next: Option<String>,
    max_pages: Option<u32>,
    max_works: Option<u32>,
    #[serde(default)]
    vars: BTreeMap<String, Value>,
    target: Option<RawTarget>,
    on_empty: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
    ext: Option<String>,
    urls: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    toc: Option<RawToc>,
    content: Option<RawContent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawToc {
    request: Option<RawRequest>,
    list: String,
    title: String,
    url: String,
    next: Option<String>,
    max_pages: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContent {
    request: Option<RawRequest>,
    text: String,
    next: Option<String>,
    max_pages: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawTest {
    #[serde(default)]
    query: String,
    min_results: Option<u32>,
}

// ---------------------------------------------------------------- 读好的规则

pub struct Request {
    url: Tpl,
    method: Method,
    form: Vec<(String, Tpl)>,
    json: Option<Value>,
    headers: Vec<(String, Tpl)>,
    enc: Option<&'static encoding_rs::Encoding>,
}

pub struct Step {
    request: Option<Request>,
    list: Option<Expr>,
    fields: Vec<(String, Expr)>,
    next: Option<Expr>,
    max_pages: u32,
    max_works: u32,
    vars: Vec<(String, Expr)>,
    target: Option<TargetSpec>,
    on_empty: String,
}

pub enum TargetSpec {
    Http { url: Expr, ext: Option<Expr> },
    Ytdlp { url: Expr },
    Magnet { url: Expr },
    Pages { urls: Expr, headers: Vec<(String, Tpl)> },
    Chapters { toc: Box<Toc>, content: Box<Content> },
}

pub struct Toc {
    request: Option<Request>,
    list: Expr,
    title: Expr,
    url: Expr,
    next: Option<Expr>,
    max_pages: u32,
}

pub struct Content {
    request: Option<Request>,
    text: Expr,
    next: Option<Expr>,
    max_pages: u32,
}

/// 检查通过、可以执行的规则
pub struct Engine {
    pub id: String,
    pub name: String,
    pub note: String,
    pub hosts: Vec<String>,
    pub login: Login,
    pub media_kinds: Vec<String>,
    pub test_query: String,
    pub min_results: u32,
    search: Step,
    episodes: Option<Step>,
    resolve: Step,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Engine({})", self.id)
    }
}

// ---------------------------------------------------------------- 检查（带行号）

/// 嵌套字段在第几行：依次找每一层的 `key:`，下一层要比上一层缩进更深。找不到时返回已找到的最深一层
pub fn locate(text: &str, path: &[&str]) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    let (mut start, mut min_indent, mut found) = (0usize, -1isize, 0usize);
    for key in path {
        let pats = [format!("{key}:"), format!("\"{key}\":"), format!("'{key}':")];
        let mut hit = None;
        for (i, l) in lines.iter().enumerate().skip(start) {
            let t = l.trim_start();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let ind = indent(l) as isize;
            if found > 0 && ind <= min_indent {
                break;
            }
            let t = t.trim_start_matches("- ");
            if pats.iter().any(|p| t.starts_with(p.as_str())) {
                hit = Some((i, ind));
                break;
            }
        }
        match hit {
            Some((i, ind)) => {
                found = i + 1;
                start = i + 1;
                min_indent = ind;
            }
            None => return found,
        }
    }
    found
}

struct Cx<'a> {
    text: &'a str,
    allow: Allow,
    issues: Vec<Issue>,
}

impl Cx<'_> {
    fn err(&mut self, path: &[&str], msg: impl Into<String>) {
        let line = locate(self.text, path);
        self.issues.push(Issue::at(line, msg));
    }
    fn label(path: &[&str]) -> String {
        path.join(".")
    }
    fn expr(&mut self, path: &[&str], src: &str) -> Option<Expr> {
        match parse_expr(src) {
            Ok(e) => Some(e),
            Err(m) => {
                self.err(path, format!("{}：{m}", Self::label(path)));
                None
            }
        }
    }
    fn tpl(&mut self, path: &[&str], src: &str) -> Option<Tpl> {
        match parse_tpl(src) {
            Ok(t) => Some(t),
            Err(m) => {
                self.err(path, format!("{}：{m}", Self::label(path)));
                None
            }
        }
    }
    /// list / next 只能用 css: / json: / regex:
    fn selector_expr(&mut self, path: &[&str], src: &str) -> Option<Expr> {
        let e = self.expr(path, src)?;
        if !matches!(
            e.getter,
            expr::Getter::Css { sel: Some(_), .. } | expr::Getter::Json(_) | expr::Getter::Regex(_)
        ) {
            self.err(path, format!("{} 要用 css: / json: / regex: 取值，如 'css: .result li'", Self::label(path)));
            return None;
        }
        Some(e)
    }
    fn pages(&mut self, path: &[&str], n: Option<u32>) -> u32 {
        match n {
            None => DEFAULT_PAGES,
            Some(0) => {
                self.err(path, "max_pages 至少是 1");
                1
            }
            Some(n) if n > MAX_PAGES => {
                self.err(path, format!("max_pages 最多 {MAX_PAGES}"));
                MAX_PAGES
            }
            Some(n) => n,
        }
    }

    fn request(&mut self, path: &[&str], r: &RawRequest) -> Option<Request> {
        let p = |k: &'static str| -> Vec<&str> { path.iter().copied().chain([k]).collect() };
        let url_src = r.url.trim();
        let url_src = url_src.strip_prefix("tpl:").map(str::trim).unwrap_or(url_src);
        let url = self.tpl(&p("url"), url_src);
        if url_src.is_empty() {
            self.err(&p("url"), "request.url 不能为空");
        } else if !url_src.starts_with("{{") {
            let lower = url_src.to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                self.err(&p("url"), "request.url 要以 http:// 或 https:// 开头");
            } else {
                // 网址里写死的网站要在 hosts 里
                let fixed = url_src.split("{{").next().unwrap_or("");
                let after = &fixed[fixed.find("://").map(|i| i + 3).unwrap_or(0)..];
                if after.contains(['/', '?', '#']) || !url_src.contains("{{") {
                    if let Ok(u) = Url::parse(fixed) {
                        if let Err(m) = self.allow.check(&u) {
                            self.err(&p("url"), m);
                        }
                    }
                }
            }
        }
        let method = match r.method.trim().to_ascii_uppercase().as_str() {
            "" | "GET" => Method::Get,
            "POST" => Method::Post,
            m => {
                self.err(&p("method"), format!("method 只能是 GET 或 POST，不能是 {m}"));
                Method::Get
            }
        };
        if method == Method::Get && (!r.form.is_empty() || r.json.is_some()) {
            self.err(&p("method"), "要提交 form / json 时 method 写 POST");
        }
        if !r.form.is_empty() && r.json.is_some() {
            self.err(&p("json"), "form 和 json 只能写一个");
        }
        let mut form = vec![];
        for (k, v) in &r.form {
            if let Some(t) = self.tpl(&p("form"), &scalar(v)) {
                form.push((k.clone(), t));
            }
        }
        if let Some(j) = &r.json {
            let mut bad = None;
            walk_strings(j, &mut |s| {
                if let Err(m) = parse_tpl(s) {
                    bad.get_or_insert(m);
                }
            });
            if let Some(m) = bad {
                self.err(&p("json"), format!("json：{m}"));
            }
        }
        let headers = self.headers(&p("headers"), &r.headers, net::HEADERS);
        let enc = match net::encoding_of(&r.encoding) {
            Ok(e) => e,
            Err(m) => {
                self.err(&p("encoding"), m);
                None
            }
        };
        Some(Request {
            url: url?,
            method,
            form,
            json: r.json.clone(),
            headers,
            enc,
        })
    }

    fn headers(&mut self, path: &[&str], h: &BTreeMap<String, String>, allowed: &[&str]) -> Vec<(String, Tpl)> {
        let mut out = vec![];
        for (k, v) in h {
            if !allowed.contains(&k.to_ascii_lowercase().as_str()) {
                let names = if allowed.len() == 1 {
                    "Referer".to_string()
                } else {
                    "Referer、Accept、Accept-Language".to_string()
                };
                self.err(
                    path,
                    format!("请求头只能写 {names}，不能写 {k}（登录信息由软件按你在“网站登录”里保存的登录自动带上）"),
                );
                continue;
            }
            if let Some(t) = self.tpl(path, v) {
                out.push((k.clone(), t));
            }
        }
        out
    }

    fn step(&mut self, name: &'static str, raw: &RawStep) -> Step {
        let p = |k: &'static str| -> Vec<&str> { vec![name, k] };
        let request = match &raw.request {
            Some(r) => self.request(&p("request"), r),
            None => {
                if name != "resolve" {
                    self.err(&[name], format!("{name} 缺少 request（请求哪个网址）"));
                }
                None
            }
        };
        let list = match &raw.list {
            Some(l) => self.selector_expr(&p("list"), l),
            None => {
                if name != "resolve" {
                    self.err(
                        &[name],
                        format!("{name} 缺少 list（每条结果在哪里），如 'css: .result li' 或 'json: $.items[*]'"),
                    );
                }
                None
            }
        };
        let mut fields = vec![];
        if name == "resolve" {
            if !raw.fields.is_empty() {
                self.err(&p("fields"), "resolve 里不用 fields，中间值写在 vars 里");
            }
            if raw.list.is_some() {
                self.err(&p("list"), "resolve 里不用 list");
            }
        } else {
            if !raw.fields.contains_key("title") {
                self.err(&p("fields"), format!("{name}.fields 要有 title"));
            }
            for (k, v) in &raw.fields {
                let ok = FIELDS.contains(&k.as_str()) || k.strip_prefix("extra.").is_some_and(|x| !x.is_empty());
                if !ok {
                    self.err(&[name, "fields", k], format!("不认识的字段 {k}（可用：{}，或 extra.名字）", FIELDS.join("、")));
                    continue;
                }
                if let Some(e) = self.expr(&[name, "fields", k], &scalar_expr(v)) {
                    fields.push((k.clone(), e));
                }
            }
        }
        if name == "search" && !raw.vars.is_empty() {
            self.err(&p("vars"), "search 里不用 vars");
        }
        let mut vars = vec![];
        for (k, v) in &raw.vars {
            if k.is_empty() || !k.chars().all(|c| c.is_alphanumeric() || c == '_') || k == "item" || k == "chapter" {
                self.err(&[name, "vars", k], format!("vars 的名字“{k}”只能用字母、数字和下划线"));
                continue;
            }
            if let Some(e) = self.expr(&[name, "vars", k], &scalar_expr(v)) {
                vars.push((k.clone(), e));
            }
        }
        let next = raw.next.as_ref().and_then(|n| self.selector_expr(&p("next"), n));
        let max_pages = self.pages(&p("max_pages"), raw.max_pages);
        let max_works = match raw.max_works {
            Some(n) if name != "episodes" => {
                self.err(&p("max_works"), "max_works 只能写在 episodes 里");
                n
            }
            Some(0) => {
                self.err(&p("max_works"), "max_works 至少是 1");
                1
            }
            Some(n) if n > MAX_WORKS => {
                self.err(&p("max_works"), format!("max_works 最多 {MAX_WORKS}"));
                MAX_WORKS
            }
            Some(n) => n,
            None => DEFAULT_WORKS,
        };
        let target = match (&raw.target, name) {
            (Some(t), "resolve") => self.target(t, request.is_some() || raw.request.is_some()),
            (Some(_), _) => {
                self.err(&p("target"), "target 只能写在 resolve 里");
                None
            }
            (None, "resolve") => {
                self.err(&["resolve"], "resolve 缺少 target（下载方式）");
                None
            }
            _ => None,
        };
        Step {
            request,
            list,
            fields,
            next,
            max_pages,
            max_works,
            vars,
            target,
            on_empty: raw.on_empty.clone().unwrap_or_default().trim().to_string(),
        }
    }

    fn target(&mut self, t: &RawTarget, has_request: bool) -> Option<TargetSpec> {
        let p = |k: &'static str| -> Vec<&str> { vec!["resolve", "target", k] };
        let need = |cx: &mut Self, k: &'static str, v: &Option<String>| -> Option<Expr> {
            match v {
                Some(s) => cx.expr(&p(k), s),
                None => {
                    cx.err(&["resolve", "target"], format!("target 缺少 {k}"));
                    None
                }
            }
        };
        let kind = t.kind.trim().to_ascii_lowercase();
        if kind != "pages" && !t.headers.is_empty() {
            self.err(&p("headers"), "只有 pages 可以写 headers");
        }
        Some(match kind.as_str() {
            "http" => TargetSpec::Http {
                url: need(self, "url", &t.url)?,
                ext: match &t.ext {
                    Some(e) => Some(self.expr(&p("ext"), e)?),
                    None => None,
                },
            },
            "ytdlp" => TargetSpec::Ytdlp {
                url: need(self, "url", &t.url)?,
            },
            "magnet" => TargetSpec::Magnet {
                url: need(self, "url", &t.url)?,
            },
            "pages" => TargetSpec::Pages {
                headers: self.headers(&p("headers"), &t.headers, &["referer"]),
                urls: need(self, "urls", &t.urls)?,
            },
            "chapters" => {
                let (Some(toc), Some(content)) = (&t.toc, &t.content) else {
                    self.err(&["resolve", "target"], "chapters 要写 toc（目录）和 content（正文）");
                    return None;
                };
                if toc.request.is_none() && !has_request {
                    self.err(&p("toc"), "toc 要写 request，或者 resolve 先写 request（目录就在那一页）");
                }
                let tp = |k: &'static str| -> Vec<&str> { vec!["resolve", "target", "toc", k] };
                let cp = |k: &'static str| -> Vec<&str> { vec!["resolve", "target", "content", k] };
                let toc = Toc {
                    request: match &toc.request {
                        Some(r) => Some(self.request(&tp("request"), r)?),
                        None => None,
                    },
                    list: self.selector_expr(&tp("list"), &toc.list)?,
                    title: self.expr(&tp("title"), &toc.title)?,
                    url: self.expr(&tp("url"), &toc.url)?,
                    next: match &toc.next {
                        Some(n) => Some(self.selector_expr(&tp("next"), n)?),
                        None => None,
                    },
                    max_pages: self.pages(&tp("max_pages"), toc.max_pages),
                };
                let content = Content {
                    request: match &content.request {
                        Some(r) => Some(self.request(&cp("request"), r)?),
                        None => None,
                    },
                    text: self.expr(&cp("text"), &content.text)?,
                    next: match &content.next {
                        Some(n) => Some(self.selector_expr(&cp("next"), n)?),
                        None => None,
                    },
                    max_pages: self.pages(&cp("max_pages"), content.max_pages),
                };
                TargetSpec::Chapters {
                    toc: Box::new(toc),
                    content: Box::new(content),
                }
            }
            other => {
                self.err(
                    &p("type"),
                    format!("target.type 只能是 http / ytdlp / magnet / pages / chapters，不能是“{other}”"),
                );
                return None;
            }
        })
    }
}

/// YAML 里写成数字、布尔值的也当文字
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// 字段 / vars 的值：数字等当作 const
fn scalar_expr(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => format!("const: {}", scalar(other)),
    }
}

fn walk_strings(v: &Value, f: &mut impl FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter().for_each(|x| walk_strings(x, f)),
        Value::Object(o) => o.values().for_each(|x| walk_strings(x, f)),
        _ => {}
    }
}

/// serde 的报错换成中文，并找到行号
fn section_issue(text: &str, section: &str, msg: &str) -> Issue {
    if let Some(rest) = msg.strip_prefix("unknown field `") {
        let key = rest.split('`').next().unwrap_or("");
        let line = text
            .lines()
            .enumerate()
            .skip(locate(text, &[section]))
            .find(|(_, l)| l.trim_start().trim_start_matches("- ").starts_with(&format!("{key}:")))
            .map(|(i, _)| i + 1)
            .unwrap_or_else(|| locate(text, &[section]));
        return Issue::at(line, format!("{section} 里不认识的字段 {key}"));
    }
    if let Some(rest) = msg.strip_prefix("missing field `") {
        let key = rest.split('`').next().unwrap_or("");
        return Issue::at(locate(text, &[section]), format!("{section} 里缺少 {key}"));
    }
    Issue::at(locate(text, &[section]), format!("{section} 写得不对：{msg}"))
}

fn section<T: serde::de::DeserializeOwned>(rule: &Rule, key: &str, issues: &mut Vec<Issue>) -> Option<T> {
    match rule.body.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => match serde_json::from_value::<T>(v.clone()) {
            Ok(t) => Some(t),
            Err(e) => {
                issues.push(section_issue(&rule.text, key, &e.to_string()));
                None
            }
        },
    }
}

/// 检查并编译规则
pub fn compile(rule: &Rule) -> Result<Engine, Vec<Issue>> {
    let h = &rule.header;
    let mut cx = Cx {
        text: &rule.text,
        allow: Allow::new(&h.hosts),
        issues: vec![],
    };
    let mut pre = vec![];
    let raw_search: Option<RawStep> = section(rule, "search", &mut pre);
    let raw_eps: Option<RawStep> = section(rule, "episodes", &mut pre);
    let raw_resolve: Option<RawStep> = section(rule, "resolve", &mut pre);
    let raw_test: RawTest = section(rule, "test", &mut pre).unwrap_or_default();
    let media_kinds: Vec<String> = match rule.body.get("media_kinds") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => a.iter().map(scalar).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(_) => {
            pre.push(Issue::at(rule.line_of("media_kinds"), "media_kinds 要写成列表，如 [novel]"));
            vec![]
        }
    };
    cx.issues.extend(pre);
    for k in &media_kinds {
        if !MEDIA.iter().any(|(id, _)| id == k) {
            cx.err(
                &["media_kinds"],
                format!("media_kinds 里的“{k}”不认识（可用 music / video / anime / manga / novel）"),
            );
        }
    }
    if rule.body.get("search").is_none() {
        cx.issues.push(Issue::new("缺少 search（怎么搜索）"));
    }
    if rule.body.get("resolve").is_none() {
        cx.issues.push(Issue::new("缺少 resolve（怎么取下载地址）"));
    }
    let search = raw_search.map(|s| cx.step("search", &s));
    let episodes = raw_eps.map(|s| cx.step("episodes", &s));
    let resolve = raw_resolve.map(|s| cx.step("resolve", &s));
    if !cx.issues.is_empty() {
        cx.issues.sort_by_key(|i| i.line);
        cx.issues.dedup();
        return Err(cx.issues);
    }
    let (Some(search), Some(resolve)) = (search, resolve) else {
        return Err(vec![Issue::new("缺少 search 或 resolve")]);
    };
    Ok(Engine {
        id: h.id.clone(),
        name: h.name.clone(),
        note: h.note.clone(),
        hosts: h.hosts.clone(),
        login: h.login,
        media_kinds,
        test_query: raw_test.query.trim().to_string(),
        min_results: raw_test.min_results.unwrap_or(1),
        search,
        episodes,
        resolve,
    })
}

// ---------------------------------------------------------------- 执行

/// 列表里的一条：取到的字段 + 所在页面网址
#[derive(Clone, Debug)]
pub struct Rec {
    pub fields: Vars,
    pub page: String,
}

/// 第一页从哪来：按 request 去取，或者已经取好了
enum Start<'a> {
    Req(&'a Request),
    Got(Fetched),
}

fn first_value(v: Vec<String>) -> Option<String> {
    v.into_iter().map(|s| s.trim().to_string()).find(|s| !s.is_empty())
}

/// 依次计算字段：模板里引用了还没算出来的字段时，等别的算完再算。
/// 返回字段值和警告（始终没有值的模板字段留空）
fn eval_fields(fields: &[(String, Expr)], node: &Node, base_vars: &Vars, base: &str) -> Result<(Vars, Vec<String>), String> {
    let mut vars = base_vars.clone();
    let mut out = Vars::new();
    let mut pending: Vec<&(String, Expr)> = fields.iter().collect();
    let mut warns = vec![];
    while !pending.is_empty() {
        let mut still = vec![];
        let mut last = None;
        for f in &pending {
            match eval(&f.1, node, &vars, base) {
                Ok(v) => {
                    // 取不到值的不放进变量：引用它的模板会报“没有值”，而不是悄悄拼出空串
                    let s = first_value(v);
                    if let Some(x) = &s {
                        vars.insert(f.0.clone(), x.clone());
                    }
                    out.insert(f.0.clone(), s.unwrap_or_default());
                }
                Err(EvalErr::Missing(n)) => {
                    last = Some(n);
                    still.push(*f);
                }
                Err(EvalErr::Msg(m)) => return Err(format!("{}：{m}", f.0)),
            }
        }
        if still.len() == pending.len() {
            for f in &still {
                warns.push(format!("{}：{}", f.0, EvalErr::Missing(last.clone().unwrap_or_default())));
                out.insert(f.0.clone(), String::new());
            }
            break;
        }
        pending = still;
    }
    Ok((out, warns))
}

/// 搜索结果的变量：`item.title`、`item.extra.xxx` …
pub fn item_vars(it: &Item) -> Vars {
    let mut v = Vars::new();
    let mut put = |k: &str, s: String| {
        if !s.is_empty() {
            v.insert(format!("item.{k}"), s);
        }
    };
    put("id", it.id.clone());
    put("title", it.title.clone());
    put("series", it.series.clone());
    put("artist", it.artist.clone());
    put("album", it.album.clone());
    put("cover", it.cover.clone());
    put("media_kind", it.media_kind.clone());
    put("year", it.year.map(|n| n.to_string()).unwrap_or_default());
    put("track", it.track.map(|n| n.to_string()).unwrap_or_default());
    put("season", it.season.map(|n| n.to_string()).unwrap_or_default());
    put("episode", it.episode.map(num).unwrap_or_default());
    put("duration", it.duration.map(num).unwrap_or_default());
    if let Some(o) = it.extra.as_object() {
        for (k, x) in o {
            let s = scalar(x);
            put(&format!("extra.{k}"), s);
        }
    }
    v
}

fn num(f: f64) -> String {
    if f.fract() == 0.0 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    }
}

/// "3:25" / "1:02:03" / "205" → 秒
fn duration_secs(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.contains(':') {
        let mut total = 0.0;
        for p in t.split(':') {
            total = total * 60.0 + p.trim().parse::<f64>().ok()?;
        }
        return Some(total);
    }
    expr::to_float(t)
}

/// 字段 → Item。标题为空的跳过；没有 id 时用标题等算一个稳定的 id
pub fn to_item(f: &Vars, page: &str, media_kind: &str) -> Option<Item> {
    let g = |k: &str| collapse(f.get(k).map(|s| s.as_str()).unwrap_or(""));
    let title = g("title");
    if title.is_empty() {
        return None;
    }
    let mut extra = Map::new();
    for (k, v) in f {
        if let Some(x) = k.strip_prefix("extra.") {
            extra.insert(x.to_string(), Value::String(v.trim().to_string()));
        }
    }
    extra.insert("_page".into(), Value::String(page.to_string()));
    let episode = expr::to_float(&g("episode"));
    let mut id = g("id");
    if id.is_empty() {
        let h = Sha256::digest(format!("{}\n{}\n{}", title, g("series"), episode.map(num).unwrap_or_default()));
        id = hex::encode(&h[..8]);
    }
    let size = expr::to_int(&g("size")).or_else(|| crate::fetch::source::parse_size(&g("size")));
    let quality = g("quality");
    let mk = g("media_kind");
    let cover = g("cover");
    Some(Item {
        id,
        title,
        artist: g("artist"),
        album: g("album"),
        year: expr::to_int(&g("year")),
        track: expr::to_int(&g("track")),
        duration: duration_secs(&g("duration")),
        cover: if cover.is_empty() { cover } else { abs_url(page, &cover) },
        media_kind: if mk.is_empty() { media_kind.to_string() } else { mk },
        series: g("series"),
        season: expr::to_int(&g("season")),
        episode,
        qualities: if size.is_some() || !quality.is_empty() {
            vec![Quality {
                key: String::new(),
                label: quality,
                format: String::new(),
                bitrate: None,
                size,
            }]
        } else {
            vec![]
        },
        extra: Value::Object(extra),
    })
}

fn page_of(it: &Item) -> String {
    it.extra.get("_page").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// 一次搜索 / 解析的执行器
pub struct Runner {
    eng: Arc<Engine>,
    net: Net,
    events: Mutex<Vec<Value>>,
}

impl Runner {
    pub fn new(eng: Arc<Engine>, net: Net) -> Runner {
        Runner {
            eng,
            net,
            events: Mutex::new(vec![]),
        }
    }

    fn event(&self, v: Value) {
        let mut e = self.events.lock();
        if e.len() < 200 {
            e.push(v);
        }
    }

    fn warn(&self, msg: String) {
        self.event(json!({ "warn": msg }));
    }

    /// 取出这一阶段的记录（请求和每页的条数）
    pub fn take_log(&self) -> (Vec<Value>, Vec<Value>) {
        (self.net.take_log(), std::mem::take(&mut *self.events.lock()))
    }

    fn build_req(&self, r: &Request, vars: &Vars, base: &str) -> Result<Req, String> {
        let s = r.url.render(vars, base).map_err(|e| format!("request.url：{e}"))?;
        let s = abs_url(base, &s);
        let url = Url::parse(&s).map_err(|_| format!("请求地址不正确：{s}"))?;
        let mut headers = vec![];
        for (k, t) in &r.headers {
            headers.push((k.clone(), t.render(vars, base).map_err(|e| format!("headers.{k}：{e}"))?));
        }
        let body = if !r.form.is_empty() {
            let mut f = vec![];
            for (k, t) in &r.form {
                f.push((k.clone(), t.render(vars, base).map_err(|e| format!("form.{k}：{e}"))?));
            }
            Body::Form(f)
        } else if let Some(j) = &r.json {
            Body::Json(render_json(j, vars, base)?)
        } else {
            Body::None
        };
        Ok(Req {
            method: r.method,
            url,
            headers,
            body,
        })
    }

    /// 跟着 next 翻页的 GET 请求（带上同样的请求头）
    fn next_req(&self, r: Option<&Request>, url: &str, vars: &Vars) -> Result<Req, String> {
        let mut req = Req::get(Url::parse(url).map_err(|_| format!("翻页地址不正确：{url}"))?);
        if let Some(r) = r {
            for (k, t) in &r.headers {
                req.headers.push((k.clone(), t.render(vars, url).unwrap_or_default()));
            }
        }
        Ok(req)
    }

    /// 取一个列表：第一页 → 按 list 拆成若干条 → 每条取字段 → 按 next 翻页（最多 max_pages 页）
    #[allow(clippy::too_many_arguments)]
    async fn listing(
        &self,
        start: Start<'_>,
        list: &Expr,
        fields: &[(String, Expr)],
        next: Option<&Expr>,
        max_pages: u32,
        vars: &Vars,
        base: &str,
    ) -> Result<Vec<Rec>, String> {
        let (req_spec, mut pre) = match start {
            Start::Req(r) => (Some(r), None),
            Start::Got(f) => (None, Some(f)),
        };
        let enc = req_spec.and_then(|r| r.enc);
        let mut out = vec![];
        let mut seen = HashSet::new();
        let mut next_url: Option<String> = None;
        for i in 0..max_pages {
            let got = if let Some(f) = pre.take() {
                Ok(f)
            } else {
                let req = match (&next_url, req_spec) {
                    (Some(u), _) => self.next_req(req_spec, u, vars),
                    (None, Some(r)) => self.build_req(r, vars, base),
                    (None, None) => break,
                };
                match req {
                    Ok(req) => {
                        seen.insert(req.url.to_string());
                        self.net.send(req, enc).await
                    }
                    Err(e) => Err(e),
                }
            };
            let f = match got {
                Ok(f) => f,
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    self.warn(format!("第 {} 页：{e}", i + 1));
                    break;
                }
            };
            seen.insert(f.url.clone());
            let (recs, nxt, blocked, warns) = extract(&f, list, fields, next, vars)?;
            self.event(json!({ "page": f.url, "hits": recs.len() }));
            for w in warns.into_iter().take(3) {
                self.warn(w);
            }
            if recs.is_empty() {
                if i == 0 && blocked {
                    return Err(LOGIN_MSG.into());
                }
                break;
            }
            out.extend(recs);
            match nxt {
                Some(n) if !seen.contains(&n) => next_url = Some(n),
                _ => break,
            }
            if i + 1 == max_pages && next_url.is_some() {
                self.warn(format!("已到 max_pages（{max_pages} 页），后面的不再翻"));
            }
        }
        Ok(out)
    }

    fn default_kind(&self, mk: &str) -> String {
        let ks = &self.eng.media_kinds;
        if ks.is_empty() || ks.iter().any(|k| k == mk) {
            mk.to_string()
        } else {
            ks[0].clone()
        }
    }

    /// search 阶段：返回每条的字段和对应的 Item
    pub async fn search_works(&self, query: &str, media_kind: &str, page: u32) -> Result<Vec<(Vars, Item)>, String> {
        let s = &self.eng.search;
        let (Some(req), Some(list)) = (&s.request, &s.list) else {
            return Err("规则缺少 search.request 或 search.list".into());
        };
        if page > 1 && !req.url.vars().contains(&"page") {
            return Ok(vec![]);
        }
        let vars: Vars = [("query".to_string(), query.to_string()), ("page".to_string(), page.max(1).to_string())].into();
        let recs = self.listing(Start::Req(req), list, &s.fields, s.next.as_ref(), s.max_pages, &vars, "").await?;
        let mk = self.default_kind(media_kind);
        let mut seen = HashSet::new();
        Ok(recs
            .into_iter()
            .filter_map(|r| to_item(&r.fields, &r.page, &mk).map(|it| (r.fields, it)))
            .filter(|(_, it)| seen.insert(it.id.clone()))
            .collect())
    }

    /// episodes 阶段：一部作品 → 各集 / 各章
    pub async fn episodes(&self, work: &Item) -> Result<Vec<(Vars, Item)>, String> {
        let Some(st) = &self.eng.episodes else {
            return Ok(vec![]);
        };
        let (Some(req), Some(list)) = (&st.request, &st.list) else {
            return Err("规则缺少 episodes.request 或 episodes.list".into());
        };
        let vars = item_vars(work);
        let base = page_of(work);
        let recs = self
            .listing(Start::Req(req), list, &st.fields, st.next.as_ref(), st.max_pages, &vars, &base)
            .await?;
        let series = if work.series.trim().is_empty() {
            work.title.clone()
        } else {
            work.series.clone()
        };
        let mut out = vec![];
        let mut seen = HashSet::new();
        for r in recs {
            let Some(mut it) = to_item(&r.fields, &r.page, &work.media_kind) else {
                continue;
            };
            if !r.fields.contains_key("id") {
                // 同名的集在不同作品里也要分得开
                it.id = format!("{}#{}", work.id, it.id);
            }
            if it.series.is_empty() {
                it.series = series.clone();
            }
            for (dst, src) in [(&mut it.artist, &work.artist), (&mut it.album, &work.album), (&mut it.cover, &work.cover)] {
                if dst.is_empty() {
                    *dst = src.clone();
                }
            }
            it.year = it.year.or(work.year);
            it.season = it.season.or(work.season);
            // 作品的 extra 也带上（集的同名字段优先）
            let mut extra = work.extra.as_object().cloned().unwrap_or_default();
            if let Some(o) = it.extra.as_object() {
                for (k, v) in o {
                    extra.insert(k.clone(), v.clone());
                }
            }
            it.extra = Value::Object(extra);
            if seen.insert(it.id.clone()) {
                out.push((r.fields, it));
            }
        }
        Ok(out)
    }

    /// 搜索：作品列表；规则有 episodes 时把前几部作品展开成各集
    pub async fn search_all(&self, query: &str, media_kind: &str, page: u32) -> Result<Vec<Item>, String> {
        let works = self.search_works(query, media_kind, page).await?;
        let Some(ep) = &self.eng.episodes else {
            return Ok(works.into_iter().map(|w| w.1).collect());
        };
        let mut out = vec![];
        let mut first_err = None;
        for (_, w) in works.iter().take(ep.max_works as usize) {
            match self.episodes(w).await {
                Ok(eps) => out.extend(eps.into_iter().map(|e| e.1)),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        match first_err {
            Some(e) if out.is_empty() => Err(e),
            _ => Ok(out),
        }
    }

    /// 网址要在 hosts 里（下载地址也一样）
    fn checked_url(&self, base: &str, s: &str, what: &str) -> Result<String, String> {
        let u = abs_url(base, s);
        let parsed = Url::parse(&u).map_err(|_| format!("{what}不正确：{u}"))?;
        self.net.allow().check(&parsed).map_err(|m| format!("{what}：{m}"))?;
        Ok(u)
    }

    fn empty_msg(&self, default: &str) -> String {
        let m = &self.eng.resolve.on_empty;
        if m.is_empty() {
            default.to_string()
        } else {
            m.clone()
        }
    }

    /// resolve 阶段。`preview`：小说只取第一章正文（测试用），返回值第二项是给测试面板的摘要
    pub async fn resolve(&self, item: &Item, preview: bool) -> Result<(Resolved, Value), String> {
        let st = &self.eng.resolve;
        let spec = st.target.as_ref().ok_or("规则缺少 resolve.target")?;
        let mut vars = item_vars(item);
        let base0 = page_of(item);
        let fetched = match &st.request {
            Some(r) => Some(self.net.send(self.build_req(r, &vars, &base0)?, r.enc).await?),
            None => None,
        };
        let page_url = fetched.as_ref().map(|f| f.url.clone()).unwrap_or(base0);
        let text = fetched.as_ref().map(|f| f.text.clone()).unwrap_or_default();

        // 同步部分：在这一页上算 vars 和下载地址
        enum Next {
            Done(Target),
            Pages(Vec<String>, Option<String>),
            Chapters,
        }
        let (step2, blocked) = {
            let page = Page::new(page_url.clone(), text);
            let node = Node::Page(&page);
            let (vv, warns) = eval_fields(&st.vars, &node, &vars, &page_url)?;
            for w in warns {
                self.warn(w);
            }
            vars.extend(vv.into_iter().filter(|(_, v)| !v.is_empty()));
            // 模板里的变量没有值，当作“取不到”（给用户看 on_empty）
            let all = |e: &Expr| -> Result<Vec<String>, String> {
                match eval(e, &node, &vars, &page_url) {
                    Ok(v) => Ok(v),
                    Err(EvalErr::Missing(_)) => Ok(vec![]),
                    Err(m) => Err(m.to_string()),
                }
            };
            let one = |e: &Expr| all(e).map(first_value);
            let blocked = fetched.is_some() && net::looks_blocked(&page.text);
            let r = match spec {
                TargetSpec::Http { url, ext } => match one(url)? {
                    Some(u) => {
                        let u = self.checked_url(&page_url, &u, "下载地址")?;
                        let ext = match ext {
                            Some(e) => one(e)?.unwrap_or_default(),
                            None => String::new(),
                        };
                        Some(Next::Done(Target::Http {
                            ext: clean_ext(&ext, &u),
                            url: u,
                            headers: Default::default(),
                        }))
                    }
                    None => None,
                },
                TargetSpec::Ytdlp { url } => match one(url)? {
                    Some(u) => Some(Next::Done(Target::Ytdlp {
                        url: self.checked_url(&page_url, &u, "视频页地址")?,
                        format: String::new(),
                    })),
                    None => None,
                },
                TargetSpec::Magnet { url } => match one(url)? {
                    Some(u) if u.to_ascii_lowercase().starts_with("magnet:?") => Some(Next::Done(Target::Magnet { uri: u })),
                    Some(u) => Some(Next::Done(Target::Magnet {
                        uri: self.checked_url(&page_url, &u, "种子地址")?,
                    })),
                    None => None,
                },
                TargetSpec::Pages { urls, .. } => {
                    let list = all(urls)?;
                    let mut v = vec![];
                    for u in list.into_iter().filter(|s| !s.trim().is_empty()) {
                        v.push(self.checked_url(&page_url, &u, "图片地址")?);
                    }
                    let nxt = match &st.next {
                        Some(n) => one(n)?.map(|u| abs_url(&page_url, &u)),
                        None => None,
                    };
                    (!v.is_empty()).then_some(Next::Pages(v, nxt))
                }
                TargetSpec::Chapters { .. } => Some(Next::Chapters),
            };
            (r, blocked)
        };
        let step2 = match step2 {
            Some(s) => s,
            None if blocked => return Err(LOGIN_MSG.into()),
            None => return Err(self.empty_msg("按规则取不到下载地址")),
        };
        let target = match step2 {
            Next::Done(t) => t,
            Next::Pages(mut urls, mut nxt) => {
                let TargetSpec::Pages { urls: ue, headers } = spec else { unreachable!() };
                let mut seen: HashSet<String> = [page_url.clone()].into();
                let enc = st.request.as_ref().and_then(|r| r.enc);
                let mut n = 1;
                while let Some(u) = nxt.take().filter(|u| !seen.contains(u)) {
                    if n >= st.max_pages {
                        self.warn(format!("已到 max_pages（{} 页），后面的不再翻", st.max_pages));
                        break;
                    }
                    n += 1;
                    seen.insert(u.clone());
                    let f = match self.net.send(self.next_req(st.request.as_ref(), &u, &vars)?, enc).await {
                        Ok(f) => f,
                        Err(e) => {
                            self.warn(format!("第 {n} 页：{e}"));
                            break;
                        }
                    };
                    let page = Page::new(f.url.clone(), f.text);
                    let node = Node::Page(&page);
                    for x in eval(ue, &node, &vars, &f.url).map_err(|e| e.to_string())? {
                        if !x.trim().is_empty() {
                            urls.push(self.checked_url(&f.url, &x, "图片地址")?);
                        }
                    }
                    if let Some(e) = &st.next {
                        nxt = eval(e, &node, &vars, &f.url).ok().and_then(first_value).map(|x| abs_url(&f.url, &x));
                    }
                }
                let mut seen_u = HashSet::new();
                urls.retain(|u| seen_u.insert(u.clone()));
                let mut h = BTreeMap::new();
                for (k, t) in headers {
                    let v = t.render(&vars, &page_url).map_err(|e| format!("headers.{k}：{e}"))?;
                    if !v.is_empty() {
                        h.insert("Referer".to_string(), v);
                    }
                }
                Target::Pages { urls, headers: h }
            }
            Next::Chapters => {
                let TargetSpec::Chapters { toc, content } = spec else { unreachable!() };
                return self.chapters(toc, content, fetched, &vars, &page_url, preview).await;
            }
        };
        let summary = target_summary(&target, None);
        Ok((Resolved { target, lyric: None }, summary))
    }

    /// 小说：目录 → 每章正文
    async fn chapters(
        &self,
        toc: &Toc,
        content: &Content,
        fetched: Option<Fetched>,
        vars: &Vars,
        page_url: &str,
        preview: bool,
    ) -> Result<(Resolved, Value), String> {
        let fields = vec![
            ("title".to_string(), parse_expr_clone(&toc.title)),
            ("url".to_string(), parse_expr_clone(&toc.url)),
        ];
        let start = match (&toc.request, fetched) {
            (Some(r), _) => Start::Req(r),
            (None, Some(f)) => Start::Got(f),
            (None, None) => return Err("chapters 的 toc 缺少 request".into()),
        };
        let recs = self
            .listing(start, &toc.list, &fields, toc.next.as_ref(), toc.max_pages, vars, page_url)
            .await?;
        let mut list: Vec<(String, String)> = vec![];
        let mut seen = HashSet::new();
        for r in recs {
            let url = r.fields.get("url").cloned().unwrap_or_default();
            if url.trim().is_empty() {
                continue;
            }
            let url = self.checked_url(&r.page, &url, "章节地址")?;
            if seen.insert(url.clone()) {
                let title = collapse(r.fields.get("title").map(|s| s.as_str()).unwrap_or(""));
                list.push((title, url));
            }
        }
        if list.is_empty() {
            return Err(self.empty_msg("按规则取不到目录"));
        }
        let total = list.len();
        let mut chapters = vec![];
        for (i, (title, url)) in list.iter().enumerate() {
            if preview && i > 0 {
                break;
            }
            let mut v = vars.clone();
            v.insert("chapter.url".into(), url.clone());
            v.insert("chapter.title".into(), title.clone());
            v.insert("chapter.index".into(), (i + 1).to_string());
            let text = self.chapter_text(content, &v, url).await?;
            if text.trim().is_empty() {
                self.warn(format!("第 {} 章“{title}”没有取到正文", i + 1));
            }
            chapters.push(TextChapter {
                title: if title.is_empty() { format!("第 {} 章", i + 1) } else { title.clone() },
                text,
            });
        }
        if chapters.iter().all(|c| c.text.trim().is_empty()) {
            return Err(self.empty_msg("按规则取不到正文"));
        }
        let target = Target::Chapters { chapters };
        let summary = target_summary(&target, Some(total));
        Ok((Resolved { target, lyric: None }, summary))
    }

    async fn chapter_text(&self, c: &Content, vars: &Vars, url: &str) -> Result<String, String> {
        let (mut req, enc) = match &c.request {
            Some(r) => (self.build_req(r, vars, url)?, r.enc),
            None => (Req::get(Url::parse(url).map_err(|_| format!("章节地址不正确：{url}"))?), None),
        };
        let mut parts: Vec<String> = vec![];
        let mut seen = HashSet::new();
        for i in 0..c.max_pages {
            seen.insert(req.url.to_string());
            let f = match self.net.send(req, enc).await {
                Ok(f) => f,
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    self.warn(format!("正文第 {} 页：{e}", i + 1));
                    break;
                }
            };
            seen.insert(f.url.clone());
            let nxt = {
                let page = Page::new(f.url.clone(), f.text);
                let node = Node::Page(&page);
                let v = eval(&c.text, &node, vars, &f.url).map_err(|e| format!("content.text：{e}"))?;
                parts.extend(v.into_iter().map(|s| expr::tidy_lines(&s)).filter(|s| !s.is_empty()));
                match &c.next {
                    Some(e) => eval(e, &node, vars, &f.url).ok().and_then(first_value).map(|x| abs_url(&f.url, &x)),
                    None => None,
                }
            };
            match nxt {
                Some(n) if !seen.contains(&n) => {
                    let u = self.checked_url(url, &n, "正文翻页地址")?;
                    req = self.next_req(c.request.as_ref(), &u, vars)?;
                }
                _ => break,
            }
        }
        Ok(parts.join("\n"))
    }
}

/// Expr 不能 Clone：按原文重新读一次（编译时已经检查过）
fn parse_expr_clone(e: &Expr) -> Expr {
    parse_expr(&e.src).expect("已检查过的表达式")
}

fn render_json(v: &Value, vars: &Vars, base: &str) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) if s.contains("{{") => {
            let t = parse_tpl(s)?;
            Value::String(t.render(vars, base).map_err(|e| format!("json：{e}"))?)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| render_json(x, vars, base)).collect::<Result<_, _>>()?),
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, x) in o {
                m.insert(k.clone(), render_json(x, vars, base)?);
            }
            Value::Object(m)
        }
        other => other.clone(),
    })
}

/// 一页的结果：各条、下一页地址、是不是登录 / 验证码页、警告
type Extracted = (Vec<Rec>, Option<String>, bool, Vec<String>);

/// 一页上的列表和字段（同步：HTML 解析结果不跨 await）
fn extract(f: &Fetched, list: &Expr, fields: &[(String, Expr)], next: Option<&Expr>, vars: &Vars) -> Result<Extracted, String> {
    let page = Page::new(f.url.clone(), f.text.clone());
    let nodes = eval_list(list, &page).map_err(|e| format!("list：{e}"))?;
    let mut recs = vec![];
    let mut warns = vec![];
    for n in &nodes {
        let (vals, w) = eval_fields(fields, n, vars, &page.url)?;
        warns.extend(w);
        recs.push(Rec {
            fields: vals,
            page: page.url.clone(),
        });
    }
    let nxt = match next {
        Some(e) => eval(e, &Node::Page(&page), vars, &page.url)
            .ok()
            .and_then(first_value)
            .map(|s| abs_url(&page.url, &s)),
        None => None,
    };
    let blocked = nodes.is_empty() && net::looks_blocked(&page.text);
    Ok((recs, nxt, blocked, warns))
}

/// 扩展名：规则给的优先，否则看网址，都没有时用 bin
fn clean_ext(ext: &str, url: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()) {
        return e;
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    match name.rsplit_once('.') {
        Some((_, x)) if !x.is_empty() && x.len() <= 5 && x.chars().all(|c| c.is_ascii_alphanumeric()) => x.to_ascii_lowercase(),
        _ => "bin".into(),
    }
}

/// 给测试面板看的下载方式摘要（不下载）
pub fn target_summary(t: &Target, total_chapters: Option<usize>) -> Value {
    match t {
        Target::Http { url, ext, .. } => json!({ "type": "http", "label": "直链下载", "url": url, "ext": ext }),
        Target::Ytdlp { url, format } => json!({ "type": "ytdlp", "label": "交给 yt-dlp", "url": url, "format": format }),
        Target::Magnet { uri } => json!({ "type": "magnet", "label": "磁力 / 种子", "url": uri }),
        Target::Pages { urls, headers } => json!({
            "type": "pages", "label": "漫画图片", "count": urls.len(),
            "first": urls.first(), "last": urls.last(), "referer": headers.get("Referer"),
        }),
        Target::Chapters { chapters } => {
            let first = chapters.first();
            json!({
                "type": "chapters", "label": "小说章节", "count": total_chapters.unwrap_or(chapters.len()),
                "first_title": first.map(|c| c.title.clone()),
                "preview": first.map(|c| c.text.chars().take(200).collect::<String>()),
            })
        }
        Target::File { path } => json!({ "type": "file", "label": "本地文件", "url": path }),
    }
}

// ---------------------------------------------------------------- 下载源

pub struct EmProvider {
    eng: Arc<Engine>,
    fetch: Arc<dyn Fetch>,
    cookies: fn(&str) -> Option<String>,
}

impl EmProvider {
    pub fn new(eng: Engine, fetch: Arc<dyn Fetch>, cookies: fn(&str) -> Option<String>) -> EmProvider {
        EmProvider {
            eng: Arc::new(eng),
            fetch,
            cookies,
        }
    }

    pub fn runner(&self) -> Runner {
        let net = Net::new(Allow::new(&self.eng.hosts), self.eng.login, self.fetch.clone(), self.cookies);
        Runner::new(self.eng.clone(), net)
    }
}

impl Provider for EmProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: format!("{ID_PREFIX}{}", self.eng.id),
            name: self.eng.name.clone(),
            media_kinds: self.eng.media_kinds.clone(),
            note: if self.eng.note.is_empty() {
                format!("规则文件 {}.em", self.eng.id)
            } else {
                self.eng.note.clone()
            },
        }
    }

    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async move { self.runner().search_all(query, media_kind, page).await })
    }

    fn resolve<'a>(&'a self, item: &'a Item, _quality: &'a str) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(async move { self.runner().resolve(item, false).await.map(|r| r.0) })
    }
}

fn kinds_text(ks: &[String]) -> String {
    if ks.is_empty() {
        return "全部类型".into();
    }
    ks.iter()
        .map(|k| {
            let zh = MEDIA.iter().find(|(id, _)| id == k).map(|m| m.1).unwrap_or(k);
            format!("{zh}（{k}）")
        })
        .collect::<Vec<_>>()
        .join("、")
}

/// 检查正文；返回导入确认框里显示的摘要（每行一句）
pub fn check(rule: &Rule) -> Result<Vec<String>, Vec<Issue>> {
    let e = compile(rule)?;
    let target = match e.resolve.target.as_ref() {
        Some(TargetSpec::Http { .. }) => "直链下载",
        Some(TargetSpec::Ytdlp { .. }) => "交给 yt-dlp 下载",
        Some(TargetSpec::Magnet { .. }) => "磁力 / 种子",
        Some(TargetSpec::Pages { .. }) => "按话下载图片，打包成 CBZ",
        Some(TargetSpec::Chapters { .. }) => "按章取正文，生成 EPUB",
        None => "",
    };
    let mut out = vec![
        format!("下载源：搜索 {}", kinds_text(&e.media_kinds)),
        format!("访问：{}", e.hosts.join("、")),
        format!(
            "需要登录：{}",
            match e.login {
                Login::None => "否",
                Login::Optional => "可选（登录过的话带上你保存的登录）",
                Login::Required => "是（先在“网站登录”里登录）",
            }
        ),
        format!("下载方式：{target}"),
    ];
    if e.episodes.is_some() {
        out.push("搜索时展开分集 / 章节".into());
    }
    Ok(out)
}

fn no_saved_cookies(_: &str) -> Option<String> {
    None
}

/// 生效（on = true）或失效：注册 / 移除下载源 `em-<id>`
pub fn apply(_app: &AppHandle, rule: &Rule, on: bool) {
    let pid = format!("{ID_PREFIX}{}", rule.id());
    if !on {
        provider::unregister(&pid);
        return;
    }
    match compile(rule) {
        Ok(eng) => {
            let http = net::Http::new(&Allow::new(&eng.hosts), true);
            // 随插件安装的规则：用户在插件里同意交出登录时才带 Cookie
            let cookies: fn(&str) -> Option<String> = if crate::plugin::rule_may_use_login(rule.id()) {
                net::saved_cookies
            } else {
                no_saved_cookies
            };
            provider::register(Arc::new(EmProvider::new(eng, Arc::new(http), cookies)));
        }
        Err(_) => provider::unregister(&pid),
    }
}

/// 导入（或更新）规则后：加进“我的来源”。已经在里面时只更新显示名，用户停用过的保持停用
pub fn add_to_sources(app: &AppHandle, rule: &Rule) -> anyhow::Result<()> {
    let url = provider::source_url(&format!("{ID_PREFIX}{}", rule.id()));
    crate::fetch::with_db(app, |c| upsert_source(c, &url, &rule.header.name))
}

fn upsert_source(c: &rusqlite::Connection, url: &str, name: &str) -> anyhow::Result<()> {
    let n = c.execute("UPDATE fetch_sources SET name=?2 WHERE url=?1", rusqlite::params![url, name])?;
    if n == 0 {
        c.execute(
            "INSERT INTO fetch_sources(kind,name,url,rule_json,created_at,enabled) VALUES('provider',?1,?2,'',?3,1)",
            rusqlite::params![name, url, crate::db::now()],
        )?;
    }
    Ok(())
}

/// 规则被删除后：从“我的来源”里移除这个下载源（连同它的追更）
pub fn removed(app: &AppHandle, id: &str) {
    let url = provider::source_url(&format!("{ID_PREFIX}{id}"));
    let _ = crate::fetch::with_db(app, |c| {
        c.execute("DELETE FROM fetch_subs WHERE source_id IN (SELECT id FROM fetch_sources WHERE url=?1)", [&url])?;
        c.execute("DELETE FROM fetch_sources WHERE url=?1", [&url])?;
        Ok(())
    });
}

/// 一个阶段的测试记录
fn stage(name: &str, runner: &Runner, r: Result<Value, String>) -> (Value, bool) {
    let (requests, events) = runner.take_log();
    let pages: Vec<Value> = events.iter().filter(|e| e.get("hits").is_some()).cloned().collect();
    let warns: Vec<Value> = events.iter().filter_map(|e| e.get("warn").cloned()).collect();
    let ok = r.is_ok();
    let mut v = json!({ "name": name, "ok": ok, "requests": requests, "pages": pages, "warns": warns });
    match r {
        Ok(Value::Object(o)) => {
            for (k, x) in o {
                v[k] = x;
            }
        }
        Ok(_) => {}
        Err(e) => v["error"] = Value::String(e),
    }
    (v, ok)
}

fn preview_fields(list: &[(Vars, Item)]) -> Value {
    Value::Array(list.iter().take(3).map(|(f, _)| json!(f)).collect())
}

/// 按测试关键字跑一遍：搜索 → 第一条的分集 → 解析下载地址（不下载）
pub async fn run_test(runner: &Runner, query: &str) -> Value {
    let eng = runner.eng.clone();
    let mut steps = vec![];
    let r = runner
        .search_works(query, eng.media_kinds.first().map(|s| s.as_str()).unwrap_or("music"), 1)
        .await;
    let works = r.clone().unwrap_or_default();
    let (s, ok) = stage("搜索", runner, r.map(|w| json!({ "count": w.len(), "items": preview_fields(&w) })));
    steps.push(s);
    let mut result = json!({ "query": query, "ok": false, "count": works.len(), "min_results": eng.min_results });
    if !ok {
        result["steps"] = Value::Array(steps);
        return result;
    }
    if works.is_empty() {
        result["steps"] = Value::Array(steps);
        result["message"] = Value::String(if eng.search.on_empty.is_empty() {
            "没有搜索结果".into()
        } else {
            eng.search.on_empty.clone()
        });
        return result;
    }
    let mut item = works[0].1.clone();
    if eng.episodes.is_some() {
        let r = runner.episodes(&item).await;
        let eps = r.clone().unwrap_or_default();
        let (s, ok) = stage(
            "分集",
            runner,
            r.map(|e| json!({ "work": item.title, "count": e.len(), "items": preview_fields(&e) })),
        );
        steps.push(s);
        if !ok || eps.is_empty() {
            result["steps"] = Value::Array(steps);
            if ok {
                result["message"] = Value::String("这部作品没有取到分集".into());
            }
            return result;
        }
        item = eps[0].1.clone();
    }
    let r = runner.resolve(&item, true).await;
    let (s, ok) = stage("解析下载地址", runner, r.map(|(_, sum)| json!({ "item": item.title, "target": sum })));
    steps.push(s);
    result["ok"] = Value::Bool(ok);
    if ok && (works.len() as u32) < eng.min_results {
        result["message"] = Value::String(format!("只搜到 {} 条，少于 test.min_results（{}）", works.len(), eng.min_results));
    }
    result["steps"] = Value::Array(steps);
    result
}

/// 规则管理页的“测试”：`arg.query` 为空时用规则里的 test.query
pub async fn test(_app: AppHandle, rule: Arc<Rule>, arg: Value) -> Result<Value, String> {
    let eng = compile(&rule).map_err(|is| super::issues_text(&is))?;
    let query = arg
        .get("query")
        .and_then(|q| q.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| eng.test_query.clone());
    if query.is_empty() {
        return Err("请输入要搜索的关键字".into());
    }
    let http = net::Http::new(&Allow::new(&eng.hosts), true);
    let p = EmProvider::new(eng, Arc::new(http), net::saved_cookies);
    let runner = p.runner();
    Ok(run_test(&runner, &query).await)
}

#[cfg(test)]
mod tests {
    #[test]
    fn imported_rule_joins_my_sources_once() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        crate::fetch::init(&c).unwrap();
        let url = "provider://em-archive-books";
        super::upsert_source(&c, url, "公版书").unwrap();
        super::upsert_source(&c, url, "公版书 v2").unwrap();
        let rows: Vec<(String, String, i64)> = c
            .prepare("SELECT kind,name,enabled FROM fetch_sources WHERE url=?1")
            .unwrap()
            .query_map([url], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![("provider".to_string(), "公版书 v2".to_string(), 1)]);
        // 用户在来源里停用过：更新规则不会重新打开
        c.execute("UPDATE fetch_sources SET enabled=0 WHERE url=?1", [url]).unwrap();
        super::upsert_source(&c, url, "公版书 v3").unwrap();
        let en: i64 = c.query_row("SELECT enabled FROM fetch_sources WHERE url=?1", [url], |r| r.get(0)).unwrap();
        assert_eq!(en, 0);
    }

    use super::super::format;
    use super::super::net::tests::Canned;
    use super::*;

    const EXAMPLE: &str = include_str!("../../../tools/em-examples/archive-books.em");

    fn no_cookie(_: &str) -> Option<String> {
        None
    }

    fn provider(text: &str, c: Canned) -> (EmProvider, Arc<Canned>) {
        let rule = format::parse(text).unwrap_or_else(|e| panic!("{e:?}"));
        let eng = compile(&rule).unwrap_or_else(|e| panic!("{e:?}"));
        let c = Arc::new(c);
        (EmProvider::new(eng, c.clone(), no_cookie), c)
    }

    fn block<T>(f: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn archive_example_end_to_end() {
        let search = r#"{"response":{"numFound":2,"docs":[
            {"identifier":"prideandprejudi00aust","title":"Pride and Prejudice","creator":["Austen, Jane","Someone"],"year":"1813"},
            {"identifier":"pride2","title":"Pride & Prejudice (illustrated)","year":1894},
            {"identifier":"notitle"}
        ]}}"#;
        let meta = r#"{"files":[{"name":"x.pdf"},{"name":"Pride and Prejudice.epub"},{"name":"other.epub"}]}"#;
        let c = Canned::default()
            .with(
                "https://archive.org/advancedsearch.php?q=(pride%20and%20prejudice)%20AND%20mediatype:texts",
                search,
            )
            .with("https://archive.org/metadata/prideandprejudi00aust", meta)
            .with("https://archive.org/metadata/pride2", r#"{"files":[{"name":"a.pdf"}]}"#);
        let (p, seen) = provider(EXAMPLE, c);
        assert_eq!(p.info().id, "em-archive-books");
        assert_eq!(p.info().media_kinds, vec!["novel".to_string()]);
        let items = block(p.search("pride and prejudice", "novel", 1)).unwrap();
        assert_eq!(items.len(), 2, "没有标题的跳过");
        let it = &items[0];
        assert_eq!(it.id, "prideandprejudi00aust");
        assert_eq!(it.title, "Pride and Prejudice");
        assert_eq!(it.series, "Pride and Prejudice");
        assert_eq!(it.artist, "Austen, Jane");
        assert_eq!(it.year, Some(1813));
        assert_eq!(it.cover, "https://archive.org/services/img/prideandprejudi00aust");
        assert_eq!(it.media_kind, "novel");
        assert_eq!(items[1].artist, "佚名");
        assert_eq!(items[1].year, Some(1894));
        let url = seen.seen.lock()[0].url.to_string();
        assert!(url.contains("page=1&output=json") && url.contains("rows=30"), "{url}");
        // 搜索结果存进任务地址再读回来，也能解析
        let back = provider::parse_task_url(&provider::task_url("em-archive-books", it, "")).unwrap().1;
        let r = block(p.resolve(&back, "")).unwrap();
        assert_eq!(
            r.target,
            Target::Http {
                url: "https://archive.org/download/prideandprejudi00aust/Pride%20and%20Prejudice.epub".into(),
                ext: "epub".into(),
                headers: Default::default(),
            }
        );
        // 没有 EPUB：规则里写的 on_empty
        let e = block(p.resolve(&items[1], "")).unwrap_err();
        assert_eq!(e, "这个条目没有 EPUB 文件");
    }

    /// 联网跑一遍示例规则（默认不跑：cargo test --lib live_archive -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn live_archive() {
        let rule = format::parse(EXAMPLE).unwrap();
        let eng = compile(&rule).unwrap();
        let http = net::Http::new(&Allow::new(&eng.hosts), true);
        let p = EmProvider::new(eng, Arc::new(http), no_cookie);
        let r = block(run_test(&p.runner(), "pride and prejudice"));
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
        assert_eq!(r["ok"], true);
    }

    #[test]
    fn archive_example_check_summary() {
        let rule = format::parse(EXAMPLE).unwrap();
        let s = check(&rule).unwrap();
        assert_eq!(s[0], "下载源：搜索 小说（novel）");
        assert_eq!(s[1], "访问：archive.org");
        assert_eq!(s[2], "需要登录：否");
    }

    #[test]
    fn test_panel_report() {
        let c = Canned::default()
            .with(
                "https://archive.org/advancedsearch.php",
                r#"{"response":{"docs":[{"identifier":"a1","title":"甲"}]}}"#,
            )
            .with("https://archive.org/metadata/a1", r#"{"files":[{"name":"a.epub"}]}"#);
        let (p, _) = provider(EXAMPLE, c);
        let r = block(run_test(&p.runner(), "甲"));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["steps"][0]["count"], 1);
        assert_eq!(r["steps"][0]["items"][0]["title"], "甲");
        assert_eq!(r["steps"][0]["requests"][0]["status"], 200);
        assert_eq!(r["steps"][0]["pages"][0]["hits"], 1);
        assert_eq!(r["steps"][1]["target"]["type"], "http");
        assert!(r["message"].as_str().unwrap().contains("min_results"), "{r}");
        // 搜索出错时只有一步，带错误
        let (p, _) = provider(EXAMPLE, Canned::default().status("https://archive.org/", 401, ""));
        let r = block(run_test(&p.runner(), "甲"));
        assert_eq!(r["ok"], false);
        assert!(r["steps"][0]["error"].as_str().unwrap().contains("需要登录"));
    }

    const HTML_RULE: &str = r#"em: 1
kind: source
id: html-demo
name: 网页示例
hosts: [books.example.org, '*.cdn.example.org']
media_kinds: [manga]
search:
  request:
    url: 'https://books.example.org/s?q={{query | url_encode}}'
    headers: { Referer: 'https://books.example.org/' }
  list: 'css: .r'
  fields:
    title: 'css: a'
    id: 'css: a @href | regex(/w/(\d+)) '
    extra.link: 'css: a @href | abs_url'
    size: 'css: .size | size'
  next: 'css: a.next @href'
  max_pages: 3
episodes:
  request:
    url: '{{item.extra.link}}'
  list: 'css: li.ep'
  fields:
    title: 'css: a'
    episode: 'css: a | int'
    extra.url: 'css: a @href | abs_url'
resolve:
  request:
    url: '{{item.extra.url}}'
  target:
    type: pages
    urls: 'css: img.p @src'
    headers: { Referer: '{{item.extra.url}}' }
  next: 'css: a.np @href'
"#;

    fn search_page(n: u32, next: bool) -> String {
        let nx = if next {
            format!(r#"<a class="next" href="/s?q=x&p={}">下一页</a>"#, n + 1)
        } else {
            String::new()
        };
        format!(r#"<div class="r"><a href="/w/{n}">作品{n}</a><span class="size">1 MB</span></div>{nx}"#)
    }

    #[test]
    fn html_rule_with_pagination_episodes_and_pages() {
        let c = Canned::default()
            .with("https://books.example.org/s?q=%E7%8C%AB", &search_page(1, true))
            .with("https://books.example.org/s?q=x&p=2", &search_page(2, true))
            .with("https://books.example.org/s?q=x&p=3", &search_page(3, true))
            .with("https://books.example.org/s?q=x&p=4", &search_page(4, true))
            .with(
                "https://books.example.org/w/1",
                r#"<ul><li class="ep"><a href="/r/1">第1话</a></li><li class="ep"><a href="/r/2">第2话</a></li></ul>"#,
            )
            .with("https://books.example.org/w/2", r#"<ul><li class="ep"><a href="/r/9">第1话</a></li></ul>"#)
            .with("https://books.example.org/w/3", "<p>none</p>")
            .with(
                "https://books.example.org/r/1",
                r#"<img class="p" src="https://img.cdn.example.org/1.jpg"><img class="p" src="/2.jpg"><a class="np" href="/r/1?p=2">下</a>"#,
            )
            .with(
                "https://books.example.org/r/1?p=2",
                r#"<img class="p" src="https://img.cdn.example.org/3.jpg"><a class="np" href="/r/1">回到第一页</a>"#,
            );
        let (p, seen) = provider(HTML_RULE, c);
        let r = p.runner();
        let works = block(r.search_works("猫", "manga", 1)).unwrap();
        assert_eq!(works.len(), 3, "max_pages = 3，第 4 页不取");
        assert_eq!(works[0].1.id, "1");
        assert_eq!(works[0].1.qualities[0].size, Some(1024 * 1024));
        assert_eq!(works[0].1.extra["link"], "https://books.example.org/w/1");
        assert_eq!(works[0].1.extra["_page"], "https://books.example.org/s?q=%E7%8C%AB");
        assert_eq!(
            seen.seen.lock()[0].headers,
            vec![("Referer".to_string(), "https://books.example.org/".to_string())]
        );
        // 前 3 部作品展开分集（第 3 部没有分集）
        let items = block(p.search("猫", "manga", 1)).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].series, "作品1");
        assert_eq!(items[0].episode, Some(1.0));
        assert_eq!(items[1].episode, Some(2.0));
        assert_eq!(items[0].media_kind, "manga");
        assert_ne!(items[0].id, items[2].id, "不同作品的第 1 话 id 不同");
        assert_eq!(items[0].extra["link"], "https://books.example.org/w/1", "作品的 extra 带到分集");
        // 漫画图片：跟着 next 翻页，相对地址补全，回到看过的页面就停
        let res = block(p.resolve(&items[0], "")).unwrap();
        assert_eq!(
            res.target,
            Target::Pages {
                urls: vec![
                    "https://img.cdn.example.org/1.jpg".into(),
                    "https://books.example.org/2.jpg".into(),
                    "https://img.cdn.example.org/3.jpg".into()
                ],
                headers: BTreeMap::from([("Referer".to_string(), "https://books.example.org/r/1".to_string())]),
            }
        );
    }

    #[test]
    fn foreign_download_hosts_refused() {
        let c = Canned::default().with("https://books.example.org/r/1", r#"<img class="p" src="https://evil.org/1.jpg">"#);
        let (p, _) = provider(HTML_RULE, c);
        let it = Item {
            title: "x".into(),
            extra: json!({ "url": "https://books.example.org/r/1" }),
            ..Default::default()
        };
        let e = block(p.resolve(&it, "")).unwrap_err();
        assert!(e.contains("evil.org"), "{e}");
    }

    #[test]
    fn login_page_detected_when_empty() {
        let c = Canned::default().with(
            "https://books.example.org/s",
            "<html><body><form><input name=u><input type=password></form></body></html>",
        );
        let (p, _) = provider(HTML_RULE, c);
        let e = block(p.runner().search_works("猫", "manga", 1)).unwrap_err();
        assert!(e.contains("需要登录"), "{e}");
    }

    const NOVEL_RULE: &str = r#"em: 1
kind: source
id: novel-demo
name: 小说示例
hosts: [n.example.org]
login: optional
media_kinds: [novel]
search:
  request:
    url: https://n.example.org/api/search
    method: POST
    form: { kw: '{{query}}', page: '{{page}}' }
    encoding: gbk
  list: 'json: $.books[*]'
  fields:
    id: 'json: $.id'
    title: 'json: $.name'
    artist: 'json: $.author'
resolve:
  request:
    url: 'https://n.example.org/book/{{item.id}}/'
  target:
    type: chapters
    toc:
      list: 'css: #toc a'
      title: 'css: @text'
      url: 'css: @href'
    content:
      text: 'css: #content'
      next: 'css: a.more @href'
      max_pages: 3
  on_empty: 没有正文
"#;

    #[test]
    fn chapters_target() {
        let (gbk, _, _) = encoding_rs::GBK.encode(r#"{"books":[{"id":7,"name":"潮汐档案","author":"林间"}]}"#);
        let c = Canned::default()
            .bytes("https://n.example.org/api/search", gbk.to_vec())
            .with(
                "https://n.example.org/book/7/",
                r#"<div id="toc"><a href="1.html">第一章 起</a><a href="2.html">第二章 承</a><a href="1.html">重复</a></div>"#,
            )
            .with(
                "https://n.example.org/book/7/1.html",
                r#"<div id="content">第一段<br>第二段</div><a class="more" href="1_2.html">下一页</a>"#,
            )
            .with("https://n.example.org/book/7/1_2.html", r#"<div id="content"><p>第三段</p></div>"#)
            .with("https://n.example.org/book/7/2.html", r#"<div id="content">第二章正文</div>"#);
        let (p, seen) = provider(NOVEL_RULE, c);
        let items = block(p.search("潮汐", "novel", 1)).unwrap();
        assert_eq!(items[0].title, "潮汐档案");
        assert_eq!(items[0].id, "7");
        {
            let s = seen.seen.lock();
            assert_eq!(s[0].method, Method::Post);
            assert_eq!(s[0].body, Body::Form(vec![("kw".into(), "潮汐".into()), ("page".into(), "1".into())]));
        }
        let r = block(p.resolve(&items[0], "")).unwrap();
        let Target::Chapters { chapters } = &r.target else { panic!("{:?}", r.target) };
        assert_eq!(chapters.len(), 2, "重复的章节地址去掉");
        assert_eq!(chapters[0].title, "第一章 起");
        assert_eq!(chapters[0].text, "第一段\n第二段\n第三段");
        assert_eq!(chapters[1].text, "第二章正文");
        // 测试模式只取第一章
        let (_, sum) = block(p.runner().resolve(&items[0], true)).unwrap();
        assert_eq!(sum["count"], 2);
        assert_eq!(sum["first_title"], "第一章 起");
        assert!(sum["preview"].as_str().unwrap().starts_with("第一段"));
    }

    #[test]
    fn pagination_hard_cap() {
        let text = HTML_RULE.replace("max_pages: 3", "max_pages: 51");
        let rule = format::parse(&text).unwrap();
        let e = compile(&rule).unwrap_err();
        assert!(e.iter().any(|i| i.msg.contains("最多 50") && i.line == 18), "{e:?}");
        // 不写 max_pages 时默认 5 页
        let text = HTML_RULE.replace("  max_pages: 3\n", "");
        let mut c = Canned::default().with("https://books.example.org/s?q=x", &search_page(1, true));
        for n in 2..=8 {
            c = c.with(&format!("https://books.example.org/s?q=x&p={n}"), &search_page(n, true));
        }
        let (p, seen) = provider(&text, c);
        let works = block(p.runner().search_works("x", "manga", 1)).unwrap();
        assert_eq!(works.len(), 5);
        assert_eq!(seen.seen.lock().len(), 5);
    }

    fn issues(text: &str) -> Vec<Issue> {
        let rule = format::parse(text).unwrap_or_else(|e| panic!("{e:?}"));
        compile(&rule).err().unwrap_or_default()
    }

    #[test]
    fn validation_errors_have_lines() {
        let base = "em: 1\nkind: source\nid: v-1\nname: x\nhosts: [a.org]\n";
        // 缺少 search / resolve
        let e = issues(base);
        assert!(e.iter().any(|i| i.msg.contains("缺少 search")));
        assert!(e.iter().any(|i| i.msg.contains("缺少 resolve")));
        let ok_resolve = "resolve:\n  target:\n    type: http\n    url: 'tpl: {{item.id}}'\n";
        let t = format!("{base}search:\n  request:\n    url: https://a.org/?q={{{{query}}}}\n  list: 'css: li'\n  fields:\n    title: 'css: a'\n    titel: 'css: b'\n{ok_resolve}");
        let e = issues(&t);
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].line, 12);
        assert!(e[0].msg.contains("titel"));
        // 未知字段、过滤器、选择器、请求头、域名、方法、目标类型
        let cases: &[(&str, usize, &str)] = &[
            ("  lst: 'css: li'\n", 10, "不认识的字段 lst"),
            ("    year: 'css: .y | md5'\n", 12, "加密"),
            ("    year: 'css: ..y'\n", 12, "选择器"),
            ("  next: 'tpl: x'\n", 12, "css:"),
        ];
        for (extra, line, want) in cases {
            let t = format!(
                "{base}search:\n  request:\n    url: https://a.org/?q={{{{query}}}}\n  list: 'css: li'\n  fields:\n    title: 'css: a'\n{extra}{ok_resolve}"
            );
            let t = t.replacen("  fields:\n    title: 'css: a'\n  lst", "  lst", 1);
            let e = issues(&t);
            assert!(e.iter().any(|i| i.msg.contains(want) && i.line == *line), "{extra}: {e:?}");
        }
        let req = |r: &str| format!("{base}search:\n  request:\n{r}  list: 'css: li'\n  fields:\n    title: 'css: a'\n{ok_resolve}");
        let e = issues(&req("    url: https://b.org/s\n"));
        assert!(e[0].msg.contains("b.org") && e[0].line == 8, "{e:?}");
        let e = issues(&req("    url: https://a.org/s\n    headers: { Cookie: x }\n"));
        assert!(e[0].msg.contains("Cookie") && e[0].line == 9, "{e:?}");
        let e = issues(&req("    url: https://a.org/s\n    method: PUT\n"));
        assert!(e[0].msg.contains("PUT") && e[0].line == 9, "{e:?}");
        let e = issues(&req("    url: file:///etc/passwd\n"));
        assert!(e[0].msg.contains("http"), "{e:?}");
        let e = issues(&req("    url: https://a.org/s\n    encoding: x-klingon\n"));
        assert!(e[0].msg.contains("编码") && e[0].line == 9, "{e:?}");
        let t = format!("{base}search:\n  request:\n    url: https://a.org/\n  list: 'css: li'\n  fields:\n    title: 'css: a'\nresolve:\n  target:\n    type: ftp\n    url: x\n");
        let e = issues(&t);
        assert!(e[0].msg.contains("ftp") && e[0].line == 14, "{e:?}");
        let t = format!(
            "{base}media_kinds: [books]\nsearch:\n  request:\n    url: https://a.org/\n  list: 'css: li'\n  fields:\n    title: 'css: a'\n{ok_resolve}"
        );
        let e = issues(&t);
        assert!(e[0].msg.contains("books") && e[0].line == 6, "{e:?}");
        // 整个文件经过 em::check 的报错也带行号
        let rule = format::parse(&req("    url: https://b.org/s\n")).unwrap();
        assert!(super::super::check(&rule).unwrap_err()[0].line > 0);
    }

    #[test]
    fn locate_nested_keys() {
        let t = "search:\n  request:\n    url: x\n  fields:\n    title: a\nresolve:\n  fields:\n    title: b\n";
        assert_eq!(locate(t, &["search", "fields", "title"]), 5);
        assert_eq!(locate(t, &["resolve", "fields", "title"]), 8);
        assert_eq!(locate(t, &["resolve", "nope"]), 6);
        assert_eq!(locate(t, &["nope"]), 0);
    }

    #[test]
    fn items_from_fields() {
        let f: Vars = [
            ("title", " 第 3 集 "),
            ("episode", "第3集"),
            ("duration", "1:02:03"),
            ("cover", "/c.jpg"),
            ("extra.k", "v"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let it = to_item(&f, "https://a.org/x/", "anime").unwrap();
        assert_eq!(it.title, "第 3 集");
        assert_eq!(it.episode, Some(3.0));
        assert_eq!(it.duration, Some(3723.0));
        assert_eq!(it.cover, "https://a.org/c.jpg");
        assert_eq!(it.extra["k"], "v");
        assert_eq!(it.id.len(), 16);
        assert_eq!(to_item(&f, "https://a.org/x/", "anime").unwrap().id, it.id, "id 稳定");
        let v = item_vars(&it);
        assert_eq!(v["item.episode"], "3");
        assert_eq!(v["item.extra.k"], "v");
        assert_eq!(clean_ext("", "https://a.org/f.EPUB?x=1"), "epub");
        assert_eq!(clean_ext(".M4A", "x"), "m4a");
        assert_eq!(clean_ext("", "https://a.org/dl"), "bin");
    }
}
