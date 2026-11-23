//! .em 取值表达式：`<取值方式> | 过滤器 | 过滤器`，以及文字模板 `{{变量 | 过滤器}}`。
//!
//! 取值方式：`css: 选择器 @属性`（默认 @text，还有 @html）、`json: JSONPath`、`regex: 正则`（取第 1 组）、
//! `tpl: 带 {{变量}} 的文字`、`const: 固定值`。没写取值方式的字符串当作 tpl。
//! 过滤器只做文字处理（见 [`FILTERS`]），没有加密、签名、脚本。
use once_cell::sync::Lazy;
use regex::{Regex, RegexBuilder};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use serde_json_path::JsonPath;
use std::cell::OnceCell;
use std::collections::BTreeMap;

/// 模板变量：`query`、`page`、`item.title`、`item.extra.xxx`、`chapter.url`、字段名、vars 里的名字
pub type Vars = BTreeMap<String, String>;

/// 可用的过滤器（报错提示用）
pub const FILTERS: &str =
    "trim、replace(a, b)、regex(正则, 组)、match(正则)、split(分隔符)、join(分隔符)、first、last、int、float、abs_url、url_encode、strip_html、size、date、default(值)";

/// 明确不支持的写法：给出原因而不是“不认识”
const FORBIDDEN: &[&str] = &[
    "md5",
    "sha1",
    "sha256",
    "hmac",
    "sign",
    "base64",
    "b64",
    "encrypt",
    "decrypt",
    "aes",
    "rsa",
    "eval",
    "js",
    "javascript",
    "exec",
    "script",
    "cookie",
];
pub const FORBIDDEN_MSG: &str = "规则不能做加密、签名、编码转换或运行脚本；需要这些的网站请改用外部程序下载源";

// ---------------------------------------------------------------- 错误

#[derive(Debug, Clone, PartialEq)]
pub enum EvalErr {
    /// 模板里用到的变量没有值（字段之间互相引用时会稍后再算一次）
    Missing(String),
    Msg(String),
}

impl std::fmt::Display for EvalErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalErr::Missing(n) => write!(f, "模板里的 {{{{{n}}}}} 没有值"),
            EvalErr::Msg(m) => f.write_str(m),
        }
    }
}

// ---------------------------------------------------------------- 页面与节点

/// 一个已经取回的页面：原文，按需解析成 HTML 或 JSON
pub struct Page {
    pub url: String,
    pub text: String,
    html: OnceCell<Html>,
    json: OnceCell<Option<Value>>,
}

impl Page {
    pub fn new(url: impl Into<String>, text: impl Into<String>) -> Page {
        Page {
            url: url.into(),
            text: text.into(),
            html: OnceCell::new(),
            json: OnceCell::new(),
        }
    }
    pub fn html(&self) -> &Html {
        self.html.get_or_init(|| Html::parse_document(&self.text))
    }
    pub fn json(&self) -> Option<&Value> {
        self.json
            .get_or_init(|| serde_json::from_str(self.text.trim_start_matches('\u{feff}').trim()).ok())
            .as_ref()
    }
}

/// 取值时的“当前位置”：整个页面、列表里的一个元素、一个 JSON 节点或一段文字
pub enum Node<'a> {
    Page(&'a Page),
    El(ElementRef<'a>),
    Json(&'a Value),
    Text(String),
}

// ---------------------------------------------------------------- 表达式

#[derive(Debug, Clone, PartialEq)]
pub enum Attr {
    Text,
    Html,
    Name(String),
}

pub enum Getter {
    /// 选择器为空表示当前元素本身
    Css {
        sel: Option<Selector>,
        attr: Attr,
    },
    Json(JsonPath),
    Regex(Regex),
    Tpl(Tpl),
    Const(String),
}

#[derive(Debug)]
pub enum Filter {
    Trim,
    Replace(String, String),
    Regex(Regex, usize),
    Match(Regex),
    Split(String),
    Join(String),
    First,
    Last,
    Int,
    Float,
    AbsUrl,
    UrlEncode,
    StripHtml,
    Size,
    Date,
    Default(String),
}

pub struct Expr {
    pub getter: Getter,
    pub filters: Vec<Filter>,
    pub src: String,
}

impl std::fmt::Debug for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Expr({})", self.src)
    }
}

/// 文字模板：`https://x.org/?q={{query | url_encode}}`
#[derive(Default)]
pub struct Tpl {
    parts: Vec<Part>,
}

enum Part {
    Lit(String),
    Var { name: String, filters: Vec<Filter> },
}

fn re(p: &str) -> Result<Regex, String> {
    RegexBuilder::new(p)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| format!("正则写得不对（{p}）：{}", first_line(&e.to_string())))
}

fn first_line(s: &str) -> String {
    s.lines().last().unwrap_or(s).trim().to_string()
}

/// 按 `|` 拆开：`|` 前面要有空格，括号、方括号、花括号里的不算，`\|` 不算
pub fn split_pipes(s: &str) -> Vec<String> {
    split_by_pipe(s, false)
}

/// `loose`：`|` 前面不要求空格（模板 `{{变量|过滤器}}` 里用）
fn split_by_pipe(s: &str, loose: bool) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut prev_ws = false;
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            cur.push(ch);
            esc = false;
            prev_ws = false;
            continue;
        }
        match ch {
            '\\' => esc = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            '|' if depth == 0 && (prev_ws || loose) => {
                out.push(cur.trim().to_string());
                cur.clear();
                prev_ws = true;
                continue;
            }
            _ => {}
        }
        prev_ws = ch.is_whitespace();
        cur.push(ch);
    }
    out.push(cur.trim().to_string());
    out
}

/// 去掉一层引号
fn unquote(s: &str) -> String {
    let t = s.trim();
    for q in ['\'', '"'] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

/// 过滤器参数按逗号拆开（引号、括号里的逗号不算），每个参数去掉引号
fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            cur.push(ch);
            esc = false;
            continue;
        }
        match (quote, ch) {
            (_, '\\') => esc = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(unquote(&cur));
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    out.push(unquote(&cur));
    out
}

fn parse_filter(s: &str) -> Result<Filter, String> {
    let s = s.trim();
    let (name, args) = match s.find('(') {
        Some(i) => {
            if !s.ends_with(')') {
                return Err(format!("过滤器“{s}”少了右括号"));
            }
            (s[..i].trim(), Some(&s[i + 1..s.len() - 1]))
        }
        None => (s, None),
    };
    let lname = name.to_ascii_lowercase();
    let one = || args.map(unquote).unwrap_or_default();
    let need = |what: &str| -> Result<String, String> {
        match args {
            Some(a) if !unquote(a).is_empty() || what == "default" || what == "join" => Ok(unquote(a)),
            _ => Err(format!("过滤器 {name} 要写参数，如 {name}(…)")),
        }
    };
    Ok(match lname.as_str() {
        "trim" => Filter::Trim,
        "first" => Filter::First,
        "last" => Filter::Last,
        "int" => Filter::Int,
        "float" => Filter::Float,
        "abs_url" => Filter::AbsUrl,
        "url_encode" => Filter::UrlEncode,
        "strip_html" => Filter::StripHtml,
        "size" => Filter::Size,
        "date" => Filter::Date,
        "default" => Filter::Default(need("default")?),
        "split" => Filter::Split(need("split")?),
        "join" => Filter::Join(if args.is_some() { one() } else { String::new() }),
        "match" => Filter::Match(re(&need("match")?)?),
        "replace" => {
            let a = split_args(args.ok_or("replace 要写成 replace(原文, 换成)")?);
            if a.is_empty() || a[0].is_empty() || a.len() > 2 {
                return Err("replace 要写成 replace(原文, 换成)，含逗号的参数用引号括起来".into());
            }
            Filter::Replace(a[0].clone(), a.get(1).cloned().unwrap_or_default())
        }
        "regex" => {
            let raw = args.ok_or("regex 要写成 regex(正则) 或 regex(正则, 组号)")?;
            let a = split_args(raw);
            let (p, g) = match a.as_slice() {
                [p, g] if g.trim().parse::<usize>().is_ok() => (p.clone(), g.trim().parse().unwrap()),
                _ => (unquote(raw), 1),
            };
            let r = re(&p)?;
            if g > r.captures_len() - 1 && !(g == 1 && r.captures_len() == 1) {
                return Err(format!("regex({p}) 没有第 {g} 组括号"));
            }
            Filter::Regex(r, g)
        }
        _ if FORBIDDEN.contains(&lname.as_str()) => return Err(format!("不支持过滤器 {name}：{FORBIDDEN_MSG}")),
        _ => return Err(format!("不认识的过滤器“{name}”（可用：{FILTERS}）")),
    })
}

fn parse_filters(parts: &[String]) -> Result<Vec<Filter>, String> {
    parts.iter().filter(|p| !p.is_empty()).map(|p| parse_filter(p)).collect()
}

/// 读模板。`{{` 没有闭合时报错
pub fn parse_tpl(s: &str) -> Result<Tpl, String> {
    let mut parts = vec![];
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        if i > 0 {
            parts.push(Part::Lit(rest[..i].to_string()));
        }
        let after = &rest[i + 2..];
        let j = after.find("}}").ok_or_else(|| format!("模板里的 {{{{ 没有对应的 }}}}：{s}"))?;
        let inner = &after[..j];
        let segs = split_by_pipe(inner, true);
        let name = segs[0].trim().to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-')) {
            return Err(format!("模板变量“{}”写得不对（只能是名字，如 {{{{query}}}}、{{{{item.id}}}}）", inner.trim()));
        }
        parts.push(Part::Var {
            name,
            filters: parse_filters(&segs[1..])?,
        });
        rest = &after[j + 2..];
    }
    if !rest.is_empty() {
        parts.push(Part::Lit(rest.to_string()));
    }
    Ok(Tpl { parts })
}

impl Tpl {
    /// 用到的变量名
    pub fn vars(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Var { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn render(&self, vars: &Vars, base: &str) -> Result<String, EvalErr> {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Lit(s) => out.push_str(s),
                Part::Var { name, filters } => {
                    let v: Vec<String> = vars.get(name).cloned().into_iter().collect();
                    let missing = v.is_empty();
                    let v = apply_filters(v, filters, base);
                    if v.is_empty() && missing {
                        return Err(EvalErr::Missing(name.clone()));
                    }
                    out.push_str(v.first().map(|s| s.as_str()).unwrap_or(""));
                }
            }
        }
        Ok(out)
    }
}

static PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(?i)(css|json|regex|tpl|const)\s*:").unwrap());
static ATTR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:^|\s)@([A-Za-z_][A-Za-z0-9_:.-]*)\s*$").unwrap());

/// 读一个取值表达式。没写取值方式时整串当作模板（不拆过滤器）
pub fn parse_expr(src: &str) -> Result<Expr, String> {
    let Some(m) = PREFIX.captures(src) else {
        return Ok(Expr {
            getter: Getter::Tpl(parse_tpl(src.trim())?),
            filters: vec![],
            src: src.to_string(),
        });
    };
    let kind = m[1].to_ascii_lowercase();
    let body = &src[m.get(0).unwrap().end()..];
    let parts = split_pipes(body);
    let g = parts[0].as_str();
    let getter = match kind.as_str() {
        "css" => {
            let (sel, attr) = match ATTR.captures(g) {
                Some(c) => {
                    let a = c[1].to_string();
                    let attr = match a.to_ascii_lowercase().as_str() {
                        "text" => Attr::Text,
                        "html" => Attr::Html,
                        _ => Attr::Name(a),
                    };
                    (g[..c.get(0).unwrap().start()].trim(), attr)
                }
                None => (g, Attr::Text),
            };
            let sel = if sel.is_empty() {
                None
            } else {
                Some(Selector::parse(sel).map_err(|_| format!("CSS 选择器写得不对：{sel}"))?)
            };
            Getter::Css { sel, attr }
        }
        "json" => {
            let p = if g.starts_with('$') {
                g.to_string()
            } else if g.starts_with('.') || g.starts_with('[') {
                format!("${g}")
            } else {
                format!("$.{g}")
            };
            Getter::Json(JsonPath::parse(&p).map_err(|e| format!("JSONPath 写得不对（{g}）：{e}"))?)
        }
        "regex" => {
            if g.is_empty() {
                return Err("regex: 后面要写正则".into());
            }
            Getter::Regex(re(g)?)
        }
        "tpl" => Getter::Tpl(parse_tpl(g)?),
        _ => Getter::Const(unquote(g)),
    };
    if kind == "css" && matches!(&getter, Getter::Css { sel: None, attr: Attr::Text }) && g.is_empty() {
        return Err("css: 后面要写选择器".into());
    }
    Ok(Expr {
        getter,
        filters: parse_filters(&parts[1..])?,
        src: src.to_string(),
    })
}

// ---------------------------------------------------------------- 取值

/// 元素的文字：块级元素和 <br> 换行，其余空白合并；跳过脚本和样式
pub fn el_text(el: ElementRef) -> String {
    fn walk(el: ElementRef, out: &mut String) {
        for c in el.children() {
            match c.value() {
                scraper::Node::Text(t) => out.push_str(t),
                scraper::Node::Element(e) => {
                    let n = e.name();
                    if matches!(n, "script" | "style" | "noscript" | "template") {
                        continue;
                    }
                    if n == "br" {
                        out.push('\n');
                        continue;
                    }
                    let block = matches!(
                        n,
                        "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "blockquote" | "pre" | "dd" | "dt"
                    );
                    if block {
                        out.push('\n');
                    }
                    if let Some(ce) = ElementRef::wrap(c) {
                        walk(ce, out);
                    }
                    if block {
                        out.push('\n');
                    }
                }
                _ => {}
            }
        }
    }
    let mut s = String::new();
    walk(el, &mut s);
    tidy_lines(&s)
}

/// 每行合并空白、去掉空行
pub fn tidy_lines(s: &str) -> String {
    s.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// JSON 值转文字：数组展开，null 跳过，对象原样输出 JSON
pub fn json_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Null => {}
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| json_strings(x, out)),
        Value::Object(_) => out.push(v.to_string()),
        _ => out.push(v.to_string()),
    }
}

fn el_value(e: ElementRef, attr: &Attr) -> Option<String> {
    match attr {
        Attr::Text => Some(el_text(e)),
        Attr::Html => Some(e.inner_html()),
        Attr::Name(a) => e.value().attr(a).map(|v| v.trim().to_string()),
    }
}

fn css_in_html(doc: &Html, sel: &Option<Selector>, attr: &Attr) -> Vec<String> {
    match sel {
        Some(s) => doc.select(s).filter_map(|e| el_value(e, attr)).collect(),
        None => el_value(doc.root_element(), attr).into_iter().collect(),
    }
}

/// 当前节点当作文字（regex 用）
fn node_text(node: &Node) -> String {
    match node {
        Node::Page(p) => p.text.clone(),
        Node::El(e) => e.html(),
        Node::Json(Value::String(s)) => s.clone(),
        Node::Json(v) => v.to_string(),
        Node::Text(t) => t.clone(),
    }
}

fn regex_all(r: &Regex, hay: &str) -> Vec<String> {
    let g = if r.captures_len() > 1 { 1 } else { 0 };
    r.captures_iter(hay).filter_map(|c| c.get(g).map(|m| m.as_str().to_string())).collect()
}

fn not_json() -> EvalErr {
    EvalErr::Msg("返回的内容不是 JSON，不能用 json: 取值（网页请用 css:）".into())
}

/// 取值方式本身（不含过滤器）
fn get(g: &Getter, node: &Node, vars: &Vars, base: &str) -> Result<Vec<String>, EvalErr> {
    Ok(match g {
        Getter::Css { sel, attr } => match node {
            Node::Page(p) => css_in_html(p.html(), sel, attr),
            Node::El(e) => match sel {
                Some(s) => e.select(s).filter_map(|x| el_value(x, attr)).collect(),
                None => el_value(*e, attr).into_iter().collect(),
            },
            Node::Json(Value::String(s)) => css_in_html(&Html::parse_fragment(s), sel, attr),
            Node::Text(t) => css_in_html(&Html::parse_fragment(t), sel, attr),
            Node::Json(_) => return Err(EvalErr::Msg("这里是 JSON，不能用 css: 取值（请用 json:）".into())),
        },
        Getter::Json(path) => {
            let owned;
            let root: &Value = match node {
                Node::Json(v) => v,
                Node::Page(p) => p.json().ok_or_else(not_json)?,
                Node::Text(t) => {
                    owned = serde_json::from_str::<Value>(t.trim()).map_err(|_| not_json())?;
                    &owned
                }
                Node::El(_) => return Err(EvalErr::Msg("这里是网页元素，不能用 json: 取值（请用 css:）".into())),
            };
            let mut out = vec![];
            for v in path.query(root).all() {
                json_strings(v, &mut out);
            }
            out
        }
        Getter::Regex(r) => regex_all(r, &node_text(node)),
        Getter::Tpl(t) => vec![t.render(vars, base)?],
        Getter::Const(s) => vec![s.clone()],
    })
}

/// 计算表达式，得到若干个值（字段一般取第一个）
pub fn eval(e: &Expr, node: &Node, vars: &Vars, base: &str) -> Result<Vec<String>, EvalErr> {
    let v = get(&e.getter, node, vars, base)?;
    Ok(apply_filters(v, &e.filters, base))
}

/// 列表：每一项是之后取字段的“当前位置”
pub fn eval_list<'a>(e: &Expr, page: &'a Page) -> Result<Vec<Node<'a>>, EvalErr> {
    Ok(match &e.getter {
        Getter::Css { sel: Some(s), .. } => page.html().select(s).map(Node::El).collect(),
        Getter::Json(path) => {
            let root = page.json().ok_or_else(not_json)?;
            let found = path.query(root).all();
            match found.as_slice() {
                [Value::Array(a)] => a.iter().map(Node::Json).collect(),
                _ => found.into_iter().map(Node::Json).collect(),
            }
        }
        Getter::Regex(r) => regex_all(r, &page.text).into_iter().map(Node::Text).collect(),
        _ => return Err(EvalErr::Msg("list 只能用 css: / json: / regex:".into())),
    })
}

// ---------------------------------------------------------------- 过滤器

static INT: Lazy<Regex> = Lazy::new(|| Regex::new(r"-?\d[\d,]*").unwrap());
static FLOAT: Lazy<Regex> = Lazy::new(|| Regex::new(r"-?\d+(?:\.\d+)?").unwrap());
static YMD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{4})\s*[-/.年]\s*(\d{1,2})\s*[-/.月]\s*(\d{1,2})").unwrap());

pub fn to_int(s: &str) -> Option<i64> {
    INT.find(s).and_then(|m| m.as_str().replace(',', "").parse().ok())
}

pub fn to_float(s: &str) -> Option<f64> {
    FLOAT.find(s).and_then(|m| m.as_str().parse().ok())
}

/// 网址组件的百分号编码（空格是 %20）
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn abs_url(base: &str, s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    match url::Url::parse(base) {
        Ok(b) => b.join(s).map(|u| u.to_string()).unwrap_or_else(|_| s.to_string()),
        Err(_) => s.to_string(),
    }
}

pub fn strip_html(s: &str) -> String {
    let frag = Html::parse_fragment(s);
    el_text(frag.root_element())
}

/// 天数 → 年月日（1970-01-01 起）
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 日期文字或时间戳 → YYYY-MM-DD
pub fn to_date(s: &str) -> Option<String> {
    if let Some(c) = YMD.captures(s) {
        let (m, d): (u32, u32) = (c[2].parse().ok()?, c[3].parse().ok()?);
        if (1..=12).contains(&m) && (1..=31).contains(&d) {
            return Some(format!("{}-{m:02}-{d:02}", &c[1]));
        }
    }
    let t = s.trim();
    if t.chars().all(|c| c.is_ascii_digit()) && (t.len() == 10 || t.len() == 13) {
        let secs: i64 = t.parse().ok()?;
        let secs = if t.len() == 13 { secs / 1000 } else { secs };
        let (y, m, d) = civil(secs.div_euclid(86_400));
        return Some(format!("{y}-{m:02}-{d:02}"));
    }
    None
}

pub fn apply_filters(mut v: Vec<String>, filters: &[Filter], base: &str) -> Vec<String> {
    for f in filters {
        v = match f {
            Filter::Trim => v.into_iter().map(|s| s.trim().to_string()).collect(),
            Filter::Replace(a, b) => v.into_iter().map(|s| s.replace(a.as_str(), b)).collect(),
            Filter::Regex(r, g) => v
                .iter()
                .filter_map(|s| r.captures(s).and_then(|c| c.get(*g).or_else(|| c.get(0))).map(|m| m.as_str().to_string()))
                .collect(),
            Filter::Match(r) => v.into_iter().filter(|s| r.is_match(s)).collect(),
            Filter::Split(sep) => v
                .iter()
                .flat_map(|s| s.split(sep.as_str()).map(|x| x.trim().to_string()).collect::<Vec<_>>())
                .filter(|s| !s.is_empty())
                .collect(),
            Filter::Join(sep) => vec![v.join(sep)],
            Filter::First => v.into_iter().take(1).collect(),
            Filter::Last => v.pop().into_iter().collect(),
            Filter::Int => v.iter().filter_map(|s| to_int(s)).map(|n| n.to_string()).collect(),
            Filter::Float => v.iter().filter_map(|s| to_float(s)).map(|n| n.to_string()).collect(),
            Filter::AbsUrl => v.iter().map(|s| abs_url(base, s)).filter(|s| !s.is_empty()).collect(),
            Filter::UrlEncode => v.iter().map(|s| url_encode(s)).collect(),
            Filter::StripHtml => v.iter().map(|s| strip_html(s)).collect(),
            Filter::Size => v
                .iter()
                .filter_map(|s| {
                    let t = s.trim();
                    t.parse::<i64>().ok().or_else(|| crate::fetch::source::parse_size(t))
                })
                .map(|n| n.to_string())
                .collect(),
            Filter::Date => v.iter().filter_map(|s| to_date(s)).collect(),
            Filter::Default(d) => {
                if v.iter().all(|s| s.trim().is_empty()) {
                    vec![d.clone()]
                } else {
                    v
                }
            }
        };
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(kv: &[(&str, &str)]) -> Vars {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn run(src: &str, page: &Page) -> Vec<String> {
        let e = parse_expr(src).unwrap();
        eval(&e, &Node::Page(page), &Vars::new(), &page.url).unwrap()
    }

    fn f(src: &str, input: &[&str]) -> Vec<String> {
        let parts = split_pipes(src);
        let fs = parse_filters(&parts).unwrap();
        apply_filters(input.iter().map(|s| s.to_string()).collect(), &fs, "https://a.org/dir/page.html")
    }

    #[test]
    fn pipes_and_args() {
        assert_eq!(split_pipes("json: $.a | first | default(x)"), vec!["json: $.a", "first", "default(x)"]);
        // 正则里的 | 在括号里、或者前面没有空格，不算分隔
        assert_eq!(split_pipes("regex: (a|b) | int"), vec!["regex: (a|b)", "int"]);
        assert_eq!(split_pipes("regex: a|b"), vec!["regex: a|b"]);
        assert_eq!(split_pipes(r"regex: a \| b"), vec![r"regex: a \| b"]);
        assert_eq!(split_args("' ', ', '"), vec![" ", ", "]);
        assert_eq!(split_args(r"\d{1,3}, 2"), vec![r"\d{1,3}", "2"]);
    }

    #[test]
    fn every_filter() {
        assert_eq!(f("trim", &["  a  "]), vec!["a"]);
        assert_eq!(f("replace(第, )", &["第3话"]), vec!["3话"]);
        assert_eq!(f("replace(', ', /)", &["a, b"]), vec!["a/b"]);
        assert_eq!(f(r"regex(第(\d+)话)", &["第12话", "无"]), vec!["12"]);
        assert_eq!(f(r"regex((\w+)-(\w+), 2)", &["ab-cd"]), vec!["cd"]);
        assert_eq!(f(r"match(\.epub$)", &["a.pdf", "b.epub", "c.epub"]), vec!["b.epub", "c.epub"]);
        assert_eq!(f("split(,)", &["a, b,,c"]), vec!["a", "b", "c"]);
        assert_eq!(f("join(、)", &["a", "b"]), vec!["a、b"]);
        assert_eq!(f("join", &["a", "b"]), vec!["ab"]);
        assert_eq!(f("first", &["a", "b"]), vec!["a"]);
        assert_eq!(f("last", &["a", "b"]), vec!["b"]);
        assert!(f("first", &[]).is_empty());
        assert_eq!(f("int", &["第 1,234 话", "x"]), vec!["1234"]);
        assert_eq!(f("float", &["约 3.5 集"]), vec!["3.5"]);
        assert_eq!(
            f("abs_url", &["../img/1.jpg", "https://b.org/x", "?p=2"]),
            vec!["https://a.org/img/1.jpg", "https://b.org/x", "https://a.org/dir/page.html?p=2"]
        );
        assert_eq!(f("url_encode", &["a b/中&"]), vec!["a%20b%2F%E4%B8%AD%26"]);
        assert_eq!(f("strip_html", &["<p>一<b>二</b></p><p>三&amp;</p>"]), vec!["一二\n三&"]);
        assert_eq!(f("size", &["1.5 GB", "2048", "无"]), vec!["1610612736", "2048"]);
        assert_eq!(
            f("date", &["2024年3月5日 更新", "2024/12/31", "1700000000", "1700000000000", "x"]),
            vec!["2024-03-05", "2024-12-31", "2023-11-14", "2023-11-14"]
        );
        assert_eq!(f("default(佚名)", &[]), vec!["佚名"]);
        assert_eq!(f("default(佚名)", &[" "]), vec!["佚名"]);
        assert_eq!(f("default(佚名)", &["甲"]), vec!["甲"]);
        assert_eq!(f("trim | replace(a, b) | int", &[" a12 "]), vec!["12"]);
    }

    #[test]
    fn filter_errors() {
        assert!(parse_filter("md5").unwrap_err().contains("加密"));
        assert!(parse_filter("eval(x)").unwrap_err().contains("脚本"));
        assert!(parse_filter("nope").unwrap_err().contains("不认识"));
        assert!(parse_filter("match(").unwrap_err().contains("右括号"));
        assert!(parse_filter("match((x)").unwrap_err().contains("正则"));
        assert!(parse_filter("regex(a, 2)").unwrap_err().contains("第 2 组"));
        assert!(parse_filter("replace").is_err());
        assert!(parse_filter("default").is_err());
    }

    #[test]
    fn css_getter() {
        let p = Page::new(
            "https://a.org/s?q=1",
            r#"<ul><li class="r"><a href="/b/1">书 <i>一</i></a><span>1.2 MB</span></li><li class="r"><a href="b/2">书二</a></li></ul><a class="next" href="?page=2">下一页</a>"#,
        );
        assert_eq!(run("css: li.r a", &p), vec!["书 一", "书二"]);
        assert_eq!(run("css: li.r a @href | abs_url", &p), vec!["https://a.org/b/1", "https://a.org/b/2"]);
        assert_eq!(run("css: a.next @href | abs_url", &p), vec!["https://a.org/s?page=2"]);
        assert_eq!(run("css: li span @html", &p), vec!["1.2 MB"]);
        assert!(run("css: .none", &p).is_empty());
        // 列表项里取字段；空选择器 = 元素本身
        let list = eval_list(&parse_expr("css: li.r a").unwrap(), &p).unwrap();
        assert_eq!(list.len(), 2);
        let e = parse_expr("css: @href").unwrap();
        assert_eq!(eval(&e, &list[1], &Vars::new(), "").unwrap(), vec!["b/2"]);
        assert!(parse_expr("css: ul >> li").unwrap_err().contains("选择器"));
        assert!(parse_expr("css: ").is_err());
    }

    #[test]
    fn json_getter() {
        let p = Page::new(
            "https://a.org/api",
            r#"{"docs":[{"id":"a","creator":["甲","乙"],"year":"1813"},{"id":"b","creator":"丙","n":3}]}"#,
        );
        assert_eq!(run("json: $.docs[*].id", &p), vec!["a", "b"]);
        assert_eq!(run("json: docs[1].n | int", &p), vec!["3"]);
        let list = eval_list(&parse_expr("json: $.docs").unwrap(), &p).unwrap();
        assert_eq!(list.len(), 2, "单个数组会展开");
        let c = parse_expr("json: $.creator | first").unwrap();
        assert_eq!(eval(&c, &list[0], &Vars::new(), "").unwrap(), vec!["甲"]);
        assert_eq!(eval(&c, &list[1], &Vars::new(), "").unwrap(), vec!["丙"]);
        assert!(parse_expr("json: $..[").is_err());
        let html = Page::new("u", "<p>x</p>");
        let e = parse_expr("json: $.a").unwrap();
        assert!(matches!(eval(&e, &Node::Page(&html), &Vars::new(), ""), Err(EvalErr::Msg(m)) if m.contains("不是 JSON")));
    }

    #[test]
    fn regex_getter() {
        let p = Page::new("u", "var pages = ['/i/1.jpg', '/i/2.jpg']; id=42;");
        assert_eq!(run(r"regex: '([^']+\.jpg)'", &p), vec!["/i/1.jpg", "/i/2.jpg"]);
        assert_eq!(run(r"regex: id=\d+ | int", &p), vec!["42"]);
        assert!(parse_expr("regex: (").is_err());
    }

    #[test]
    fn templates() {
        let v = vars(&[("query", "傲慢 与偏见"), ("page", "2"), ("item.id", "x/y")]);
        let t = parse_tpl("https://a.org/?q={{query | url_encode}}&p={{ page }}&id={{item.id|url_encode}}").unwrap();
        assert_eq!(
            t.render(&v, "").unwrap(),
            "https://a.org/?q=%E5%82%B2%E6%85%A2%20%E4%B8%8E%E5%81%8F%E8%A7%81&p=2&id=x%2Fy"
        );
        let t = parse_tpl("{{item.id|url_encode}}").unwrap();
        assert_eq!(t.render(&v, "").unwrap(), "x%2Fy");
        assert_eq!(parse_tpl("{{nope}}").unwrap().render(&v, ""), Err(EvalErr::Missing("nope".into())));
        assert_eq!(parse_tpl("{{nope | default(0)}}").unwrap().render(&v, "").unwrap(), "0");
        assert!(parse_tpl("a {{query").is_err());
        assert!(parse_tpl("{{a b}}").is_err());
        let e = parse_expr("tpl: https://a.org/img/{{id}} | replace(img, cover)").unwrap();
        assert_eq!(
            eval(&e, &Node::Text(String::new()), &vars(&[("id", "7")]), "").unwrap(),
            vec!["https://a.org/cover/7"]
        );
        let c = parse_expr("const: novel").unwrap();
        assert_eq!(eval(&c, &Node::Text(String::new()), &Vars::new(), "").unwrap(), vec!["novel"]);
        // 没写取值方式：整串是模板
        let plain = parse_expr("epub").unwrap();
        assert_eq!(eval(&plain, &Node::Text(String::new()), &Vars::new(), "").unwrap(), vec!["epub"]);
    }

    #[test]
    fn text_of_blocks() {
        let p = Page::new("u", "<div id=c>第一段<br>第二段<p>第三段</p><script>x()</script>  </div>");
        assert_eq!(run("css: #c", &p), vec!["第一段\n第二段\n第三段"]);
    }
}
