//! comic 规则正文的解析和检查：match / read / pages / export → 编译好的 Spec + 导入确认框里的摘要
use super::img::{TrimColor, TrimOpt};
use crate::em::{Issue, Rule};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;

/// 一个匹配条件；同一条目里的几个条件要同时满足，条目之间任一满足即可
#[derive(Clone, Debug)]
pub enum Cond {
    Series(Regex),
    Path(Regex),
    /// 下载源：暂时只记录，下载下来的文件还没有来源标记
    Source(#[allow(dead_code)] String),
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct ReadPrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_alone: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<u32>,
}

impl ReadPrefs {
    fn is_empty(&self) -> bool {
        *self == ReadPrefs::default()
    }
}

#[derive(Clone, Debug)]
pub struct Split {
    pub ratio: f32,
    /// 先右半再左半
    pub rtl: bool,
    pub keep_cover: bool,
}

#[derive(Clone, Debug)]
pub enum Skip {
    Name(Regex),
    First(usize),
    Last(usize),
    /// 样张文件（相对规则目录）、允许的汉明距离
    Like(String, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Natural,
    Name,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Keep,
    Jpg,
    Png,
}

#[derive(Clone, Debug)]
pub struct Slice {
    pub max_height: u32,
    pub at_blank_rows: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    /// 序号，补零到几位
    Num(usize),
    Ext,
    /// 原文件名（不含扩展名）
    Orig,
}

#[derive(Clone, Debug)]
pub struct ExportOpt {
    pub format: Format,
    pub quality: u8,
    pub max_height: Option<u32>,
    pub rename: Vec<Part>,
    pub slice: Option<Slice>,
    /// ComicInfo.xml 的 Manga 字段
    pub manga: Option<String>,
}

impl Default for ExportOpt {
    fn default() -> Self {
        ExportOpt {
            format: Format::Keep,
            quality: 90,
            max_height: None,
            rename: vec![Part::Num(4), Part::Text(".".into()), Part::Ext],
            slice: None,
            manga: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub matchers: Vec<Vec<Cond>>,
    pub read: Option<ReadPrefs>,
    pub split: Option<Split>,
    pub trim: Option<TrimOpt>,
    pub skip: Vec<Skip>,
    pub order: Order,
    pub export: ExportOpt,
    pub summary: Vec<String>,
}

impl Spec {
    /// 阅读时要不要改动页面（拆、裁、跳、换顺序）
    pub fn touches_pages(&self) -> bool {
        self.split.is_some() || self.trim.is_some() || !self.skip.is_empty() || self.order != Order::Natural
    }

    /// 这本漫画是否适用。match 为空表示所有漫画
    pub fn matches(&self, series: &str, path: &str) -> bool {
        if self.matchers.is_empty() {
            return true;
        }
        let alt = if path.contains('\\') {
            path.replace('\\', "/")
        } else {
            path.replace('/', "\\")
        };
        self.matchers.iter().any(|conds| {
            !conds.is_empty()
                && conds.iter().all(|c| match c {
                    Cond::Series(re) => re.is_match(series),
                    Cond::Path(re) => re.is_match(path) || re.is_match(&alt),
                    Cond::Source(_) => false,
                })
        })
    }

    /// 样张文件（like）所在位置
    pub fn sample_path(file: &str) -> PathBuf {
        crate::em::rules_dir().join(file)
    }
}

// ---------------------------------------------------------------- 行号

/// 顶层 `section:` 下面（含同一行的 `{ … }` 写法）第一次出现 `key:` 的行；找不到时返回 section 的行
pub fn line_in(text: &str, section: &str, key: &str) -> usize {
    let start = crate::em::format::line_of(text, section);
    if start == 0 {
        return 0;
    }
    let lines: Vec<&str> = text.lines().collect();
    let pat = format!("{key}:");
    for (i, l) in lines.iter().enumerate().skip(start - 1) {
        if i > start - 1 {
            let top = !l.is_empty() && !l.starts_with(' ') && !l.starts_with('\t') && !l.starts_with('#') && !l.starts_with('-');
            if top {
                break;
            }
        }
        let body = if i == start - 1 { l.find(':').map(|p| &l[p + 1..]).unwrap_or(l) } else { l };
        let code = body.split(" #").next().unwrap_or("");
        if code
            .match_indices(&pat)
            .any(|(p, _)| p == 0 || !code.as_bytes()[p - 1].is_ascii_alphanumeric() && code.as_bytes()[p - 1] != b'_')
        {
            return i + 1;
        }
    }
    start
}

// ---------------------------------------------------------------- 编译

struct Ck<'a> {
    text: &'a str,
    issues: Vec<Issue>,
}

impl Ck<'_> {
    fn err(&mut self, sec: &str, key: &str, msg: impl Into<String>) {
        let line = if key.is_empty() {
            crate::em::format::line_of(self.text, sec)
        } else {
            line_in(self.text, sec, key)
        };
        self.issues.push(Issue::at(line, msg));
    }

    fn obj<'v>(&mut self, sec: &str, key: &str, v: &'v Value) -> Option<&'v Map<String, Value>> {
        match v {
            Value::Object(m) => Some(m),
            Value::Null => None,
            _ => {
                let name = if key.is_empty() { sec.to_string() } else { format!("{sec}.{key}") };
                self.err(sec, key, format!("{name} 应该是“字段: 值”的形式"));
                None
            }
        }
    }

    fn unknown(&mut self, sec: &str, m: &Map<String, Value>, allowed: &[&str]) {
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                self.err(sec, k, format!("{sec} 里不认识“{k}”（可以写：{}）", allowed.join("、")));
            }
        }
    }

    fn one_of(&mut self, sec: &str, key: &str, v: &Value, allowed: &[&str]) -> Option<String> {
        match v.as_str() {
            Some(s) if allowed.contains(&s) => Some(s.to_string()),
            _ => {
                self.err(sec, key, format!("{key} 只能是 {}", allowed.join(" / ")));
                None
            }
        }
    }

    fn boolean(&mut self, sec: &str, key: &str, v: &Value) -> Option<bool> {
        match v {
            Value::Bool(b) => Some(*b),
            _ => {
                self.err(sec, key, format!("{key} 只能是 true 或 false"));
                None
            }
        }
    }

    fn int(&mut self, sec: &str, key: &str, v: &Value, lo: i64, hi: i64) -> Option<i64> {
        let n = v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
        match n {
            Some(n) if (lo..=hi).contains(&n) => Some(n),
            _ => {
                self.err(sec, key, format!("{key} 应该是 {lo}~{hi} 之间的整数"));
                None
            }
        }
    }

    fn regex(&mut self, sec: &str, key: &str, v: &Value) -> Option<Regex> {
        let Some(s) = v.as_str() else {
            self.err(sec, key, format!("{key} 应该是一个正则表达式（文字）"));
            return None;
        };
        match regex::RegexBuilder::new(s).size_limit(1 << 20).build() {
            Ok(r) => Some(r),
            Err(e) => {
                let first = e.to_string().lines().last().unwrap_or("").trim().to_string();
                self.err(sec, key, format!("{key} 的正则“{s}”写得不对：{first}"));
                None
            }
        }
    }
}

const TOP: &[&str] = &[
    "em", "kind", "id", "name", "version", "author", "note", "hosts", "login", "match", "read", "pages", "export",
];

/// 检查并编译正文
pub fn compile(rule: &Rule) -> Result<Spec, Vec<Issue>> {
    let mut ck = Ck {
        text: &rule.text,
        issues: vec![],
    };
    let body = rule.body.as_object().cloned().unwrap_or_default();
    for k in body.keys() {
        if !TOP.contains(&k.as_str()) {
            let line = crate::em::format::line_of(&rule.text, k);
            ck.issues
                .push(Issue::at(line, format!("不认识的字段“{k}”（漫画排版规则可以写：match、read、pages、export）")));
        }
    }
    let mut summary = vec![];
    let mut warnings = vec![];

    // ---- match
    let mut matchers = vec![];
    let mut used = vec![];
    match body.get("match") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let items: Vec<Value> = match v {
                Value::Array(a) => a.clone(),
                Value::Object(_) => vec![v.clone()],
                _ => {
                    ck.err("match", "", "match 应该是一个列表，每项写 series / path / source");
                    vec![]
                }
            };
            for it in &items {
                let Some(m) = ck.obj("match", "", it) else { continue };
                ck.unknown("match", m, &["series", "path", "source"]);
                let mut conds = vec![];
                for (k, v) in m {
                    match k.as_str() {
                        "series" => {
                            if let Some(r) = ck.regex("match", "series", v) {
                                used.push(format!("作品名匹配 {}", r.as_str()));
                                conds.push(Cond::Series(r));
                            }
                        }
                        "path" => {
                            if let Some(r) = ck.regex("match", "path", v) {
                                used.push(format!("路径匹配 {}", r.as_str()));
                                conds.push(Cond::Path(r));
                            }
                        }
                        "source" => match v.as_str() {
                            Some(s) if !s.trim().is_empty() => {
                                used.push(format!("来自下载源 {s}（暂未生效）"));
                                conds.push(Cond::Source(s.to_string()));
                            }
                            _ => ck.err("match", "source", "source 应该写下载源规则的 id"),
                        },
                        _ => {}
                    }
                }
                if m.is_empty() {
                    ck.err("match", "", "match 里有空的条目");
                }
                matchers.push(conds);
            }
        }
    }
    summary.push(if used.is_empty() {
        "用于：所有漫画".to_string()
    } else {
        format!("用于：{}", used.join("、"))
    });

    // ---- read
    let mut read = ReadPrefs::default();
    if let Some(m) = body.get("read").and_then(|v| ck.obj("read", "", v)) {
        ck.unknown("read", m, &["flow", "dir", "spread", "cover_alone", "fit", "gap"]);
        for (k, v) in m {
            match k.as_str() {
                "flow" => read.flow = ck.one_of("read", k, v, &["h", "v", "vscroll", "hscroll"]),
                "dir" => read.dir = ck.one_of("read", k, v, &["ltr", "rtl"]),
                "spread" => read.spread = ck.one_of("read", k, v, &["single", "double"]),
                "cover_alone" => read.cover_alone = ck.boolean("read", k, v),
                "fit" => read.fit = ck.one_of("read", k, v, &["screen", "width", "height", "original"]),
                "gap" => read.gap = ck.int("read", k, v, 0, 80).map(|n| n as u32),
                _ => {}
            }
        }
    }
    if !read.is_empty() {
        let mut parts = vec![];
        if let Some(f) = &read.flow {
            parts.push(
                match f.as_str() {
                    "h" => "左右翻页",
                    "v" => "上下翻页",
                    "vscroll" => "上下连续滚动",
                    _ => "左右连续滚动",
                }
                .to_string(),
            );
        }
        if let Some(d) = &read.dir {
            parts.push(if d == "rtl" { "从右往左" } else { "从左往右" }.to_string());
        }
        if let Some(s) = &read.spread {
            parts.push(if s == "double" { "双页" } else { "单页" }.to_string());
        }
        if let Some(c) = read.cover_alone {
            parts.push(if c { "封面单独" } else { "封面不单独" }.to_string());
        }
        if let Some(f) = &read.fit {
            parts.push(
                match f.as_str() {
                    "screen" => "适应屏幕",
                    "width" => "适应宽度",
                    "height" => "适应高度",
                    _ => "原始大小",
                }
                .to_string(),
            );
        }
        if let Some(g) = read.gap {
            parts.push(format!("页间距 {g}"));
        }
        summary.push(format!("阅读：{}", parts.join("、")));
    }
    let rtl_default = read.dir.as_deref() == Some("rtl");

    // ---- pages
    let mut split = None;
    let mut trim = None;
    let mut skip = vec![];
    let mut order = Order::Natural;
    if let Some(p) = body.get("pages").and_then(|v| ck.obj("pages", "", v)) {
        ck.unknown("pages", p, &["split_wide", "trim", "skip", "order"]);
        if let Some(v) = p.get("split_wide") {
            let mut s = Split {
                ratio: 1.3,
                rtl: rtl_default,
                keep_cover: true,
            };
            match v {
                Value::Bool(false) | Value::Null => {}
                Value::Bool(true) => split = Some(s),
                _ => {
                    if let Some(m) = ck.obj("pages", "split_wide", v) {
                        ck.unknown("pages", m, &["when_ratio", "order", "keep_cover"]);
                        let mut ok = true;
                        if let Some(r) = m.get("when_ratio") {
                            match r.as_f64() {
                                Some(x) if (1.0..=5.0).contains(&x) && x > 1.0 => s.ratio = x as f32,
                                _ => {
                                    ck.err("pages", "when_ratio", "when_ratio 应该是大于 1、不超过 5 的数（宽 ÷ 高）");
                                    ok = false;
                                }
                            }
                        }
                        if let Some(o) = m.get("order") {
                            match ck.one_of("pages", "order", o, &["rtl", "ltr"]) {
                                Some(o) => s.rtl = o == "rtl",
                                None => ok = false,
                            }
                        }
                        if let Some(k) = m.get("keep_cover") {
                            match ck.boolean("pages", "keep_cover", k) {
                                Some(b) => s.keep_cover = b,
                                None => ok = false,
                            }
                        }
                        if ok {
                            split = Some(s);
                        }
                    }
                }
            }
        }
        if let Some(v) = p.get("trim") {
            let mut t = TrimOpt {
                color: TrimColor::Auto,
                tolerance: 12,
                max: 0.1,
                sides: [true; 4],
            };
            match v {
                Value::Bool(false) | Value::Null => {}
                Value::Bool(true) => trim = Some(t),
                _ => {
                    if let Some(m) = ck.obj("pages", "trim", v) {
                        ck.unknown("pages", m, &["color", "tolerance", "max", "sides"]);
                        let mut ok = true;
                        if let Some(c) = m.get("color") {
                            match ck.one_of("pages", "color", c, &["auto", "white", "black"]).as_deref() {
                                Some("white") => t.color = TrimColor::White,
                                Some("black") => t.color = TrimColor::Black,
                                Some(_) => {}
                                None => ok = false,
                            }
                        }
                        if let Some(x) = m.get("tolerance") {
                            match ck.int("pages", "tolerance", x, 0, 255) {
                                Some(n) => t.tolerance = n as u8,
                                None => ok = false,
                            }
                        }
                        if let Some(x) = m.get("max") {
                            match percent(x) {
                                Some(f) if f > 0.0 && f <= 0.45 => t.max = f,
                                _ => {
                                    ck.err("pages", "max", "max 应该是 1%~45% 之间的百分比，如 8%");
                                    ok = false;
                                }
                            }
                        }
                        if let Some(x) = m.get("sides") {
                            let names = ["top", "bottom", "left", "right"];
                            match x.as_array() {
                                Some(a) if !a.is_empty() => {
                                    let mut sides = [false; 4];
                                    for s in a {
                                        match s.as_str().and_then(|s| names.iter().position(|n| *n == s)) {
                                            Some(i) => sides[i] = true,
                                            None => {
                                                ck.err("pages", "sides", "sides 只能列 top / bottom / left / right");
                                                ok = false;
                                            }
                                        }
                                    }
                                    t.sides = sides;
                                }
                                _ => {
                                    ck.err("pages", "sides", "sides 应该是列表，如 [left, right]");
                                    ok = false;
                                }
                            }
                        }
                        if ok {
                            trim = Some(t);
                        }
                    }
                }
            }
        }
        if let Some(v) = p.get("skip") {
            match v {
                Value::Null => {}
                Value::Array(a) => {
                    for it in a {
                        let Some(m) = ck.obj("pages", "skip", it) else { continue };
                        ck.unknown("pages", m, &["name", "first", "last", "like", "distance"]);
                        if let Some(x) = m.get("name") {
                            if let Some(r) = ck.regex("pages", "name", x) {
                                skip.push(Skip::Name(r));
                            }
                        }
                        if let Some(x) = m.get("first") {
                            if let Some(n) = ck.int("pages", "first", x, 1, 50) {
                                skip.push(Skip::First(n as usize));
                            }
                        }
                        if let Some(x) = m.get("last") {
                            if let Some(n) = ck.int("pages", "last", x, 1, 50) {
                                skip.push(Skip::Last(n as usize));
                            }
                        }
                        if let Some(x) = m.get("like") {
                            let dist = match m.get("distance") {
                                Some(d) => ck.int("pages", "distance", d, 0, 32).unwrap_or(10) as u32,
                                None => 10,
                            };
                            match x.as_str() {
                                Some(f) if valid_sample(f) => {
                                    if !Spec::sample_path(f).is_file() {
                                        warnings.push(format!(
                                            "注意：样张 {f} 不在规则文件夹里，把它放到 {} 后才会按相似度跳过",
                                            crate::em::rules_dir().display()
                                        ));
                                    }
                                    skip.push(Skip::Like(f.to_string(), dist));
                                }
                                _ => ck.err("pages", "like", "like 应该写样张图片的文件名（放在规则文件夹里），不能带路径"),
                            }
                        } else if m.contains_key("distance") {
                            ck.err("pages", "distance", "distance 要和 like 写在同一项里");
                        }
                        if m.is_empty() {
                            ck.err("pages", "skip", "skip 里有空的条目");
                        }
                    }
                }
                _ => ck.err("pages", "skip", "skip 应该是一个列表，每项写 name / first / last / like"),
            }
        }
        if let Some(v) = p.get("order") {
            match ck.one_of("pages", "order", v, &["natural", "name", "keep"]).as_deref() {
                Some("name") => order = Order::Name,
                Some("keep") => order = Order::Keep,
                _ => {}
            }
        }
    }
    if let Some(s) = &split {
        let mut extra = vec![if s.rtl { "先右后左" } else { "先左后右" }];
        if s.keep_cover {
            extra.push("第 1 页不拆");
        }
        summary.push(format!("拆开宽高比超过 {} 的跨页（{}）", trim_float(s.ratio), extra.join("，")));
    }
    if let Some(t) = &trim {
        let what = match t.color {
            TrimColor::White => "白边",
            TrimColor::Black => "黑边",
            TrimColor::Auto => "白边 / 黑边",
        };
        let sides = if t.sides == [true; 4] {
            String::new()
        } else {
            let names = ["上", "下", "左", "右"];
            let s: String = (0..4).filter(|&i| t.sides[i]).map(|i| names[i]).collect();
            format!("只裁{s}，")
        };
        summary.push(format!("裁{what}（{sides}每边最多 {}%）", trim_float(t.max * 100.0)));
    }
    if !skip.is_empty() {
        let parts: Vec<String> = skip
            .iter()
            .map(|s| match s {
                Skip::Name(r) => format!("文件名含 {}", r.as_str()),
                Skip::First(n) => format!("每话前 {n} 页"),
                Skip::Last(n) => format!("每话最后 {n} 页"),
                Skip::Like(f, _) => format!("和 {f} 相似的页"),
            })
            .collect();
        summary.push(format!("跳过：{}", parts.join("、")));
    }
    match order {
        Order::Name => summary.push("页面顺序：按文件名字典序".into()),
        Order::Keep => summary.push("页面顺序：保持压缩包里的顺序".into()),
        Order::Natural => {}
    }

    // ---- export
    let mut export = ExportOpt::default();
    let mut ex_parts = vec![];
    if let Some(m) = body.get("export").and_then(|v| ck.obj("export", "", v)) {
        ck.unknown("export", m, &["format", "quality", "max_height", "rename", "slice", "comicinfo"]);
        if let Some(v) = m.get("format") {
            if v.as_str() == Some("webp") {
                ck.err("export", "format", "暂不支持导出 webp，请用 keep / jpg / png");
            } else {
                match ck.one_of("export", "format", v, &["keep", "jpg", "png"]).as_deref() {
                    Some("jpg") => export.format = Format::Jpg,
                    Some("png") => export.format = Format::Png,
                    _ => {}
                }
            }
        }
        if let Some(v) = m.get("quality") {
            if let Some(q) = ck.int("export", "quality", v, 1, 100) {
                export.quality = q as u8;
            }
        }
        if let Some(v) = m.get("max_height") {
            export.max_height = ck.int("export", "max_height", v, 200, 30000).map(|n| n as u32);
        }
        if let Some(v) = m.get("rename") {
            match v.as_str().map(parse_rename) {
                Some(Ok(p)) => export.rename = p,
                Some(Err(e)) => ck.err("export", "rename", e),
                None => ck.err("export", "rename", "rename 应该是文件名模板，如 '{序号:03}.{ext}'"),
            }
        }
        if let Some(v) = m.get("slice") {
            if let Some(s) = ck.obj("export", "slice", v) {
                ck.unknown("export", s, &["max_height", "at_blank_rows"]);
                let h = match s.get("max_height") {
                    Some(x) => ck.int("export", "max_height", x, 200, 30000),
                    None => {
                        ck.err("export", "slice", "slice 要写 max_height（每页最大高度，像素）");
                        None
                    }
                };
                let blank = match s.get("at_blank_rows") {
                    Some(x) => ck.boolean("export", "at_blank_rows", x).unwrap_or(true),
                    None => true,
                };
                if let Some(h) = h {
                    export.slice = Some(Slice {
                        max_height: h as u32,
                        at_blank_rows: blank,
                    });
                }
            }
        }
        if let Some(v) = m.get("comicinfo") {
            if let Some(c) = ck.obj("export", "comicinfo", v) {
                ck.unknown("export", c, &["manga"]);
                if let Some(x) = c.get("manga") {
                    export.manga = ck.one_of("export", "manga", x, &["YesAndRightToLeft", "Yes", "No"]);
                }
            }
        }
        match export.format {
            Format::Jpg => ex_parts.push(format!("JPG {}", export.quality)),
            Format::Png => ex_parts.push("PNG".into()),
            Format::Keep => {}
        }
        if let Some(h) = export.max_height {
            ex_parts.push(format!("最大高度 {h}"));
        }
        if let Some(s) = &export.slice {
            ex_parts.push(format!(
                "长图切成每页最多 {} 像素{}",
                s.max_height,
                if s.at_blank_rows { "（只切在空白处）" } else { "" }
            ));
        }
        if m.contains_key("rename") {
            if let Some(Value::String(s)) = m.get("rename") {
                ex_parts.push(format!("文件名 {s}"));
            }
        }
        match export.manga.as_deref() {
            Some("YesAndRightToLeft") => ex_parts.push("标记为日漫（右开）".into()),
            Some("Yes") => ex_parts.push("标记为漫画".into()),
            _ => {}
        }
    }
    if export.manga.is_none() && rtl_default {
        export.manga = Some("YesAndRightToLeft".into());
    }
    if !ex_parts.is_empty() {
        summary.push(format!("导出：{}", ex_parts.join("、")));
    }
    summary.extend(warnings);

    if !ck.issues.is_empty() {
        ck.issues.sort_by_key(|i| i.line);
        return Err(ck.issues);
    }
    Ok(Spec {
        matchers,
        read: if read.is_empty() { None } else { Some(read) },
        split,
        trim,
        skip,
        order,
        export,
        summary,
    })
}

fn trim_float(x: f32) -> String {
    let s = format!("{:.2}", x);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// “8%”、8（按百分数）、“0.08” → 0.08
fn percent(v: &Value) -> Option<f32> {
    let (n, pct) = match v {
        Value::Number(n) => {
            let x = n.as_f64()?;
            (x, x > 1.0)
        }
        Value::String(s) => {
            let s = s.trim();
            match s.strip_suffix('%') {
                Some(x) => (x.trim().parse().ok()?, true),
                None => {
                    let x: f64 = s.parse().ok()?;
                    (x, x > 1.0)
                }
            }
        }
        _ => return None,
    };
    let f = if pct { n / 100.0 } else { n };
    Some(f as f32)
}

fn valid_sample(f: &str) -> bool {
    !f.trim().is_empty() && !f.contains('/') && !f.contains('\\') && !f.contains("..")
}

/// 文件名模板：{序号} {序号:03} {ext} {原名}（也可以写 {n} {n:03} {name}）
pub fn parse_rename(s: &str) -> Result<Vec<Part>, String> {
    if s.contains('/') || s.contains('\\') {
        return Err("rename 只能是文件名，不能带文件夹".into());
    }
    let mut out = vec![];
    let mut rest = s;
    let mut has_num = false;
    while let Some(i) = rest.find('{') {
        if i > 0 {
            out.push(Part::Text(rest[..i].to_string()));
        }
        let Some(j) = rest[i..].find('}') else {
            return Err("rename 里的 { 没有对应的 }".into());
        };
        let tok = &rest[i + 1..i + j];
        let (name, fmt) = tok.split_once(':').unwrap_or((tok, ""));
        match name.trim() {
            "序号" | "n" => {
                let width = if fmt.is_empty() {
                    1
                } else {
                    match fmt.trim().trim_start_matches('0').parse::<usize>() {
                        Ok(w) if fmt.trim().starts_with('0') && (1..=8).contains(&w) => w,
                        _ => return Err(format!("rename 里的“{{{tok}}}”写得不对，补零要写成 {{序号:03}}")),
                    }
                };
                has_num = true;
                out.push(Part::Num(width));
            }
            "ext" | "扩展名" => out.push(Part::Ext),
            "原名" | "name" => out.push(Part::Orig),
            _ => return Err(format!("rename 里不认识“{{{tok}}}”（可以用 {{序号}}、{{序号:03}}、{{ext}}、{{原名}}）")),
        }
        rest = &rest[i + j + 1..];
    }
    if !rest.is_empty() {
        out.push(Part::Text(rest.to_string()));
    }
    if !has_num {
        return Err("rename 必须包含 {序号}，否则页面会重名".into());
    }
    if !out.contains(&Part::Ext) {
        return Err("rename 要以 .{ext} 结尾，保留图片扩展名".into());
    }
    Ok(out)
}

pub fn render_name(parts: &[Part], n: usize, ext: &str, orig: &str) -> String {
    let mut s = String::new();
    for p in parts {
        match p {
            Part::Text(t) => s.push_str(t),
            Part::Num(w) => s.push_str(&format!("{n:0w$}", w = *w)),
            Part::Ext => s.push_str(ext),
            Part::Orig => s.push_str(orig),
        }
    }
    s
}
