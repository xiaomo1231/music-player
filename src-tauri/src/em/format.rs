//! .em 规则文件的读取和公共校验：YAML（兼容 JSON）→ 头部 + 正文
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 目前支持的格式版本
pub const EM_VERSION: u32 = 1;
/// .em 文件大小上限
pub const MAX_BYTES: usize = 256 * 1024;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Comic,
    Source,
    Clean,
    Organize,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Comic => "comic",
            Kind::Source => "source",
            Kind::Clean => "clean",
            Kind::Organize => "organize",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Comic => "漫画排版",
            Kind::Source => "下载源",
            Kind::Clean => "小说排版",
            Kind::Organize => "整理文件",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "comic" => Some(Kind::Comic),
            "source" => Some(Kind::Source),
            "clean" => Some(Kind::Clean),
            "organize" => Some(Kind::Organize),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Login {
    #[default]
    None,
    Optional,
    Required,
}

/// 所有 kind 共用的头部字段
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Header {
    pub em: u32,
    pub kind: Kind,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub note: String,
    /// 允许访问的域名（只有 source 用）
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub login: Login,
}

/// 一个问题：行号（从 1 开始，0 表示不知道）+ 说明
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Issue {
    pub line: usize,
    pub msg: String,
}

impl Issue {
    pub fn new(msg: impl Into<String>) -> Issue {
        Issue { line: 0, msg: msg.into() }
    }
    pub fn at(line: usize, msg: impl Into<String>) -> Issue {
        Issue { line, msg: msg.into() }
    }
}

/// 读好的规则：头部 + 整个文件的 JSON 形式（各 kind 自己再按需要反序列化）+ 原文
#[derive(Clone, Debug)]
pub struct Rule {
    pub header: Header,
    pub body: Value,
    pub text: String,
}

impl Rule {
    pub fn kind(&self) -> Kind {
        self.header.kind
    }
    pub fn id(&self) -> &str {
        &self.header.id
    }
    /// 正文里的某个字段，按 kind 自己的结构反序列化。字段不存在时用 Default
    pub fn section<T: serde::de::DeserializeOwned + Default>(&self, key: &str) -> Result<T, Issue> {
        match self.body.get(key) {
            None | Some(Value::Null) => Ok(T::default()),
            Some(v) => serde_json::from_value(v.clone()).map_err(|e| Issue::at(self.line_of(key), format!("{key}：{e}"))),
        }
    }
    /// 整个正文按某个结构反序列化（未知字段忽略）
    pub fn parse_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, Issue> {
        serde_json::from_value(self.body.clone()).map_err(|e| Issue::new(e.to_string()))
    }
    /// 顶层字段 `key:` 在第几行（报错用；找不到为 0）
    pub fn line_of(&self, key: &str) -> usize {
        line_of(&self.text, key)
    }
}

/// 顶层字段在第几行
pub fn line_of(text: &str, key: &str) -> usize {
    let pat = format!("{key}:");
    let quoted = format!("\"{key}\":");
    text.lines()
        .position(|l| l.starts_with(&pat) || l.trim_start().starts_with(&quoted))
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// 规则 id：小写字母、数字、`-`，2~40 个字符，不以 `-` 开头结尾
pub fn valid_id(id: &str) -> bool {
    (2..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !id.starts_with('-') && !id.ends_with('-')
}

/// 读 .em 文本：YAML 解析、头部检查。只做所有 kind 共用的检查，kind 自己的检查在各模块的 `check`
pub fn parse(text: &str) -> Result<Rule, Vec<Issue>> {
    if text.len() > MAX_BYTES {
        return Err(vec![Issue::new(format!(
            "规则文件太大（{} KB），上限 {} KB",
            text.len() / 1024,
            MAX_BYTES / 1024
        ))]);
    }
    let text = text.trim_start_matches('\u{feff}');
    let body: Value = serde_norway::from_str(text).map_err(|e| vec![yaml_issue(&e)])?;
    if !body.is_object() {
        return Err(vec![Issue::new("规则文件的最外层应该是“字段: 值”的形式")]);
    }
    let header: Header = serde_json::from_value(body.clone()).map_err(|e| vec![header_issue(text, &e.to_string())])?;
    let mut issues = vec![];
    if header.em == 0 || header.em > EM_VERSION {
        issues.push(Issue::at(
            line_of(text, "em"),
            if header.em > EM_VERSION {
                format!("这是第 {} 版规则格式，请先更新软件", header.em)
            } else {
                "em 版本号应该是 1".to_string()
            },
        ));
    }
    if !valid_id(&header.id) {
        issues.push(Issue::at(line_of(text, "id"), "id 只能用小写字母、数字和 -，2~40 个字符"));
    }
    if header.name.trim().is_empty() {
        issues.push(Issue::at(line_of(text, "name"), "name 不能为空"));
    }
    let hl = line_of(text, "hosts");
    if header.kind == Kind::Source {
        if header.hosts.is_empty() {
            issues.push(Issue::at(hl, "下载源规则必须在 hosts 里列出要访问的域名"));
        }
        for h in &header.hosts {
            if let Err(m) = check_host(h) {
                issues.push(Issue::at(hl, m));
            }
        }
    } else if !header.hosts.is_empty() {
        issues.push(Issue::at(hl, format!("{}规则不联网，不需要 hosts", header.kind.label())));
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    Ok(Rule {
        header,
        body,
        text: text.to_string(),
    })
}

fn yaml_issue(e: &serde_norway::Error) -> Issue {
    let line = e.location().map(|l| l.line()).unwrap_or(0);
    Issue::at(line, format!("格式错误：{e}"))
}

fn header_issue(text: &str, msg: &str) -> Issue {
    // serde 的报错形如 “missing field `kind`” / “unknown variant `xxx`”
    for key in ["em", "kind", "id", "name", "version", "hosts", "login"] {
        if msg.contains(&format!("`{key}`")) {
            let zh = if msg.starts_with("missing field") {
                format!("缺少 {key}")
            } else if key == "kind" {
                "kind 只能是 comic / source / clean / organize".to_string()
            } else if key == "login" {
                "login 只能是 none / optional / required".to_string()
            } else {
                format!("{key} 写得不对：{msg}")
            };
            return Issue::at(line_of(text, key), zh);
        }
    }
    if msg.contains("unknown variant") {
        if let Some(l) = [("kind", line_of(text, "kind")), ("login", line_of(text, "login"))].iter().find(|x| x.1 > 0) {
            return Issue::at(l.1, format!("{} 写得不对：{msg}", l.0));
        }
    }
    Issue::new(format!("头部字段写得不对：{msg}"))
}

/// 一个 hosts 条目：域名或 `*.域名`，可带端口；内网 / 本机地址允许，但会在导入时单独提示
pub fn check_host(h: &str) -> Result<(), String> {
    let h = h.trim();
    let bad = || {
        Err(format!(
            "hosts 里的“{h}”不是域名（只写域名，如 example.org 或 *.example.org，不要带 http:// 和路径）"
        ))
    };
    if h.is_empty() || h.contains("://") || h.contains('/') || h.contains(' ') {
        return bad();
    }
    let name = h.strip_prefix("*.").unwrap_or(h);
    let name = match name.rsplit_once(':') {
        Some((n, p)) if p.parse::<u16>().is_ok() => n,
        _ => name,
    };
    if name == "*" || name.is_empty() || name.contains('*') {
        return Err(format!("hosts 不能用“{h}”这样的通配，至少要写到一级域名，如 *.example.org"));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return bad();
    }
    if !name.contains('.') && name != "localhost" {
        return Err(format!("hosts 里的“{h}”不完整，要写完整域名"));
    }
    Ok(())
}

/// 是不是本机或内网地址（导入时高亮提示）
pub fn is_lan_host(h: &str) -> bool {
    let h = h.trim().trim_start_matches("*.");
    let name = match h.rsplit_once(':') {
        Some((n, p)) if p.parse::<u16>().is_ok() => n,
        _ => h,
    };
    if name == "localhost" || name.ends_with(".local") || name.ends_with(".lan") || name.ends_with(".home.arpa") {
        return true;
    }
    match name.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Err(_) => false,
    }
}

/// 比较版本号（1.2.10 > 1.2.9）；不是数字的部分按字符串比
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<&str> = a.split(['.', '-']).collect();
    let pb: Vec<&str> = b.split(['.', '-']).collect();
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or("0");
        let y = pb.get(i).copied().unwrap_or("0");
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(m), Ok(n)) => m.cmp(&n),
            _ => x.cmp(y),
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = "em: 1\nkind: comic\nid: jp-layout\nname: 日漫\nversion: 1.0.0\nread:\n  dir: rtl\n";

    #[test]
    fn parses_yaml_and_json() {
        let r = parse(OK).unwrap();
        assert_eq!(r.kind(), Kind::Comic);
        assert_eq!(r.id(), "jp-layout");
        assert_eq!(r.body["read"]["dir"], "rtl");
        let j = parse(r#"{"em":1,"kind":"clean","id":"x-1","name":"n"}"#).unwrap();
        assert_eq!(j.kind(), Kind::Clean);
    }

    #[test]
    fn reports_line_numbers() {
        let e = parse("em: 1\nkind: comic\nid: Bad_Id\nname: x\n").unwrap_err();
        assert_eq!(e[0].line, 3);
        let e = parse("em: 1\nkind: movie\nid: a1\nname: x\n").unwrap_err();
        assert_eq!(e[0].line, 2, "{e:?}");
        let e = parse("em: 1\nkind: comic\nname: x\n").unwrap_err();
        assert!(e[0].msg.contains("缺少 id"), "{e:?}");
        let e = parse("em: 1\nkind: comic\nid: ab\nname: [unclosed\n").unwrap_err();
        assert!(e[0].line >= 4, "{e:?}");
        let e = parse("em: 2\nkind: comic\nid: ab\nname: x\n").unwrap_err();
        assert!(e[0].msg.contains("更新软件"));
    }

    #[test]
    fn source_needs_hosts() {
        let e = parse("em: 1\nkind: source\nid: ab\nname: x\n").unwrap_err();
        assert!(e[0].msg.contains("hosts"));
        let e = parse("em: 1\nkind: source\nid: ab\nname: x\nhosts: [https://a.org/x]\n").unwrap_err();
        assert!(e[0].msg.contains("不是域名"));
        assert!(parse("em: 1\nkind: source\nid: ab\nname: x\nhosts: [archive.org, '*.example.org', 'nas.local:8080']\n").is_ok());
        let e = parse("em: 1\nkind: comic\nid: ab\nname: x\nhosts: [a.org]\n").unwrap_err();
        assert!(e[0].msg.contains("不联网"));
    }

    #[test]
    fn host_rules() {
        assert!(check_host("*").is_err());
        assert!(check_host("*.org").is_err());
        assert!(check_host("example").is_err());
        assert!(check_host("192.168.1.5:8080").is_ok());
        assert!(is_lan_host("192.168.1.5:8080"));
        assert!(is_lan_host("nas.local"));
        assert!(is_lan_host("localhost"));
        assert!(!is_lan_host("archive.org"));
    }

    #[test]
    fn versions() {
        use std::cmp::Ordering::*;
        assert_eq!(cmp_version("1.2.10", "1.2.9"), Greater);
        assert_eq!(cmp_version("1.0", "1.0.0"), Equal);
        assert_eq!(cmp_version("0.9", "1.0"), Less);
    }

    #[test]
    fn too_big() {
        let big = format!("{OK}#{}", "x".repeat(MAX_BYTES));
        assert!(parse(&big).unwrap_err()[0].msg.contains("太大"));
    }
}
