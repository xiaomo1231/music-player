//! 各类来源：订阅 RSS / Atom、yt-dlp 列表、本地目录与 WebDAV；以及共用的 HTTP 小工具（限频、登录检测）
use super::{Hit, SourceRow};
use crate::{parse, tools};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const FORBIDDEN_MSG: &str = "网站拒绝了这次访问（HTTP 403）。可能需要登录，也可能是网站不允许程序访问（防火墙、地区或防盗链限制）。可以先在浏览器里打开这个地址确认；软件不会绕过登录或访问限制。";
pub const LOGIN_MSG: &str = "需要登录或验证码：请在浏览器里打开这个网站，自己完成登录或验证后再来搜索。软件不会绕过登录或验证码。";
pub const YTDLP_MISSING: &str = "没有找到 yt-dlp.exe。请在“自动获取 → B 站”里点“安装 yt-dlp”，或把 yt-dlp.exe 放到程序目录的 bin 文件夹 / 系统 PATH 后重试。";
const USER_AGENT: &str = "MediaPlayerFetch/1.0";
const ONE_SEC: Duration = Duration::from_secs(1);
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><propfind xmlns="DAV:"><prop><getcontentlength/><resourcetype/></prop></propfind>"#;

/// 同一站点两次请求之间至少间隔 1 秒
static NEXT_ALLOWED: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(40))
        .build()
        .unwrap_or_default()
}

/// 按站点限频：排队等到距离上一次请求满 1 秒
pub async fn throttle(url: &str) {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_default();
    let wait = {
        let mut m = NEXT_ALLOWED.lock();
        let now = Instant::now();
        let start = m.get(&host).copied().filter(|t| *t > now).unwrap_or(now);
        m.insert(host, start + ONE_SEC);
        start.saturating_duration_since(now)
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

/// 用户名和密码写在网址里时（如 WebDAV）用基本认证
pub fn with_auth(req: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
    // 在软件里登录过这个网站时带上它的 Cookie
    let req = match super::login::cookie_header(url) {
        Some(h) => req.header(reqwest::header::COOKIE, h),
        None => req,
    };
    match url::Url::parse(url) {
        Ok(u) if !u.username().is_empty() => req.basic_auth(pct_decode(u.username()), u.password().map(pct_decode)),
        _ => req,
    }
}

pub fn check_status(s: reqwest::StatusCode) -> Result<(), String> {
    if s == reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!("{}（HTTP {}）", LOGIN_MSG, s.as_u16()));
    }
    // 403 不一定是要登录：很多网站会直接拒绝程序访问（防火墙、地区或防盗链限制）
    if s == reqwest::StatusCode::FORBIDDEN {
        return Err(FORBIDDEN_MSG.into());
    }
    if !s.is_success() {
        return Err(format!("网站返回 HTTP {}", s.as_u16()));
    }
    Ok(())
}

pub async fn http_bytes(url: &str) -> Result<Vec<u8>, String> {
    throttle(url).await;
    let resp = with_auth(client().get(url), url).send().await.map_err(|e| format!("连接失败：{e}"))?;
    check_status(resp.status())?;
    resp.bytes().await.map(|b| b.to_vec()).map_err(|e| format!("读取失败：{e}"))
}

pub async fn http_text(url: &str) -> Result<String, String> {
    let b = http_bytes(url).await?;
    Ok(crate::reader::decode_text(&b).0)
}

/// 搜索网址模板里的 {q} 换成关键字（百分号编码）；没有 {q} 原样返回
pub fn fill_query(template: &str, q: &str) -> String {
    if !template.contains("{q}") {
        return template.to_string();
    }
    let enc: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
    template.replace("{q}", &enc)
}

pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn is_http(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// Windows 盘符路径或 \\server\share、//server/share
pub fn is_local_path(s: &str) -> bool {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([A-Za-z]:[\\/]|\\\\|//)").unwrap());
    RE.is_match(s.trim())
}

/// 文件大小文字（1.2 GB、800MB、12 KiB）转字节数
pub fn parse_size(s: &str) -> Option<i64> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)([\d]+(?:[.,]\d+)?)\s*([KMGT]?i?B)\b").unwrap());
    let c = RE.captures(s)?;
    let n: f64 = c[1].replace(',', "").parse().ok()?;
    let mul = match c[2].chars().next()?.to_ascii_uppercase() {
        'K' => 1024.0,
        'M' => 1024.0 * 1024.0,
        'G' => 1024.0 * 1024.0 * 1024.0,
        'T' => 1024f64.powi(4),
        _ => 1.0,
    };
    Some((n * mul) as i64)
}

pub fn tokens(q: &str) -> Vec<String> {
    q.to_lowercase().split_whitespace().map(|s| s.to_string()).collect()
}

pub fn match_tokens(name: &str, toks: &[String]) -> bool {
    let n = name.to_lowercase();
    toks.iter().all(|t| n.contains(t.as_str()))
}

/// 是否是这种媒体：music 只要音乐，其他只要视频
pub fn media_ok(name: &str, media_kind: &str) -> bool {
    match parse::classify(Path::new(name)) {
        Some("music") => media_kind == "music",
        Some("video" | "anime") => media_kind != "music",
        _ => false,
    }
}

/// 下载地址的类型：磁力、种子文件、本地文件、普通 HTTP
pub fn kind_of_url(url: &str) -> &'static str {
    let l = url.to_ascii_lowercase();
    if l.starts_with("magnet:") {
        return "magnet";
    }
    let path = url::Url::parse(url).map(|u| u.path().to_ascii_lowercase()).unwrap_or_else(|_| l.clone());
    if path.ends_with(".torrent") {
        return "torrent";
    }
    if is_local_path(url) {
        return "file";
    }
    // B 站页面不是直链，交给 yt-dlp
    if super::bili::is_bili_url(url) {
        return "ytdlp";
    }
    "http"
}

/// 只看地址的样子判断类型（不联网）；None 表示要进一步判断
pub fn guess_kind(url: &str) -> Option<&'static str> {
    if is_local_path(url) {
        return Some("dir");
    }
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let path = u.path().to_ascii_lowercase();
    if path.contains("/dav") || path.contains("webdav") {
        return Some("dir");
    }
    if [".xml", ".rss", ".atom"].iter().any(|x| path.ends_with(x)) || path.contains("/rss") || path.contains("/feed") || path.contains("atom") {
        return Some("rss");
    }
    None
}

// ---------- 订阅 RSS / Atom ----------

#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub title: String,
    pub url: String,
    pub size: Option<i64>,
    pub date: Option<String>,
}

/// 能当作下载地址的：磁力、种子、媒体文件
fn is_downloadable(u: &str) -> bool {
    if u.to_ascii_lowercase().starts_with("magnet:") {
        return true;
    }
    match url::Url::parse(u) {
        Ok(x) => {
            let p = x.path().to_ascii_lowercase();
            p.ends_with(".torrent") || parse::classify(Path::new(&p)).is_some()
        }
        Err(_) => false,
    }
}

/// 解析 RSS 2.0（优先）或 Atom。条目里的 enclosure、磁力链接、种子链接作为下载地址
pub fn parse_feed(bytes: &[u8]) -> Result<Vec<FeedItem>, String> {
    if let Ok(ch) = rss::Channel::read_from(bytes) {
        let items = ch
            .items()
            .iter()
            .filter_map(|it| {
                let enc = it.enclosure();
                let url = enc
                    .map(|e| e.url().to_string())
                    .filter(|u| !u.is_empty())
                    .or_else(|| it.link().map(|s| s.trim().to_string()).filter(|l| is_downloadable(l)))?;
                let title = it.title().map(collapse).filter(|t| !t.is_empty()).unwrap_or_else(|| url.clone());
                let size = enc.and_then(|e| e.length().parse::<i64>().ok()).filter(|n| *n > 0);
                Some(FeedItem {
                    title,
                    url,
                    size,
                    date: it.pub_date().map(|s| s.to_string()),
                })
            })
            .collect();
        return Ok(items);
    }
    let text = String::from_utf8_lossy(bytes);
    let doc = scraper::Html::parse_document(&text);
    let sel = |s: &str| scraper::Selector::parse(s).unwrap();
    let entries: Vec<_> = doc.select(&sel("entry")).collect();
    if entries.is_empty() && doc.select(&sel("feed")).next().is_none() {
        return Err("不是 RSS 或 Atom 订阅链接".into());
    }
    let mut items = vec![];
    for en in entries {
        let title = en
            .select(&sel("title"))
            .next()
            .map(|t| collapse(&t.text().collect::<String>()))
            .unwrap_or_default();
        let mut url: Option<String> = None;
        let mut size = None;
        for l in en.select(&sel("link")) {
            let href = l.value().attr("href").unwrap_or("").trim();
            if href.is_empty() {
                continue;
            }
            if l.value().attr("rel") == Some("enclosure") {
                url = Some(href.to_string());
                size = l.value().attr("length").and_then(|s| s.parse::<i64>().ok()).filter(|n| *n > 0);
                break;
            }
            if url.is_none() && is_downloadable(href) {
                url = Some(href.to_string());
            }
        }
        let Some(url) = url else { continue };
        let date = en
            .select(&sel("published"))
            .next()
            .or_else(|| en.select(&sel("updated")).next())
            .map(|d| collapse(&d.text().collect::<String>()));
        let title = if title.is_empty() { url.clone() } else { title };
        items.push(FeedItem { title, url, size, date });
    }
    Ok(items)
}

fn feed_hits(items: Vec<FeedItem>) -> Vec<Hit> {
    items
        .into_iter()
        .map(|it| Hit {
            kind: kind_of_url(&it.url).to_string(),
            title: it.title,
            url: it.url,
            size: it.size,
            date: it.date,
            ..Default::default()
        })
        .collect()
}

pub async fn search_rss(row: &SourceRow, query: &str) -> Result<Vec<Hit>, String> {
    let url = fill_query(&row.url, query);
    let bytes = http_bytes(&url).await?;
    let items = parse_feed(&bytes)?;
    let has_q = row.url.contains("{q}");
    let toks = tokens(query);
    let items = items.into_iter().filter(|it| has_q || match_tokens(&it.title, &toks)).collect();
    Ok(feed_hits(items))
}

// ---------- 目录（本地 / 映射盘 / WebDAV） ----------

pub async fn search_dir(row: &SourceRow, query: &str, media_kind: &str) -> Result<Vec<Hit>, String> {
    let toks = tokens(query);
    if is_http(&row.url) {
        return webdav_search(&row.url, &toks, media_kind).await;
    }
    let root = PathBuf::from(&row.url);
    if !root.is_dir() {
        return Err("文件夹不存在或无法访问（网盘或 NAS 是否已连接？）".into());
    }
    let mut files = vec![];
    walk_local(&root, 0, &mut files);
    let mut hits = vec![];
    for p in files {
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !match_tokens(&name, &toks) || !media_ok(&name, media_kind) {
            continue;
        }
        let size = std::fs::metadata(&p).ok().map(|m| m.len() as i64);
        let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
        hits.push(Hit {
            title,
            url: p.to_string_lossy().into_owned(),
            size,
            kind: "file".into(),
            ..Default::default()
        });
    }
    Ok(hits)
}

/// 最多 6 层、两万个文件，防止误选了整个磁盘
fn walk_local(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 6 || out.len() > 20000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_local(&p, depth + 1, out);
        } else {
            out.push(p);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DavEntry {
    pub href: String,
    pub size: Option<i64>,
    pub is_dir: bool,
}

/// 解析 WebDAV 的 multistatus 响应（忽略命名空间前缀）
pub fn parse_multistatus(xml: &str) -> Vec<DavEntry> {
    static RE_RESP: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(?:[a-z0-9]+:)?response\b[^>]*>(.*?)</(?:[a-z0-9]+:)?response>").unwrap());
    static RE_HREF: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(?:[a-z0-9]+:)?href\b[^>]*>\s*([^<]*?)\s*</(?:[a-z0-9]+:)?href>").unwrap());
    static RE_LEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(?:[a-z0-9]+:)?getcontentlength\b[^>]*>\s*(\d+)\s*<").unwrap());
    static RE_COLL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(?:[a-z0-9]+:)?collection\b").unwrap());
    let mut out = vec![];
    for c in RE_RESP.captures_iter(xml) {
        let body = &c[1];
        let Some(h) = RE_HREF.captures(body) else { continue };
        out.push(DavEntry {
            href: h[1].to_string(),
            size: RE_LEN.captures(body).and_then(|l| l[1].parse::<i64>().ok()),
            is_dir: RE_COLL.is_match(body) || h[1].ends_with('/'),
        });
    }
    out
}

async fn propfind(url: &str, depth: &str) -> Result<String, String> {
    throttle(url).await;
    let m = reqwest::Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?;
    let req = client()
        .request(m, url)
        .header("Depth", depth)
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(PROPFIND_BODY);
    let resp = with_auth(req, url).send().await.map_err(|e| format!("连接失败：{e}"))?;
    check_status(resp.status())?;
    resp.text().await.map_err(|e| e.to_string())
}

/// 添加 WebDAV 来源时确认地址可以列目录
pub async fn check_webdav(url: &str) -> Result<(), String> {
    propfind(url, "0").await.map(|_| ()).map_err(|e| format!("不是可访问的 WebDAV 目录：{e}"))
}

async fn webdav_search(base: &str, toks: &[String], media_kind: &str) -> Result<Vec<Hit>, String> {
    let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
    let mut stack = vec![(base.clone(), 0usize)];
    let mut visited = HashSet::new();
    let mut hits = vec![];
    let mut first = true;
    while let Some((dir, depth)) = stack.pop() {
        if !visited.insert(dir.clone()) || visited.len() > 300 {
            continue;
        }
        let xml = match propfind(&dir, "1").await {
            Ok(x) => x,
            Err(e) if first => return Err(e),
            Err(_) => continue,
        };
        first = false;
        for ent in parse_multistatus(&xml) {
            let Ok(abs) = url::Url::parse(&dir).and_then(|b| b.join(&ent.href)) else {
                continue;
            };
            let abs = abs.to_string();
            if abs.trim_end_matches('/') == dir.trim_end_matches('/') {
                continue;
            }
            if ent.is_dir {
                if depth < 3 {
                    stack.push((abs, depth + 1));
                }
                continue;
            }
            let last = abs.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            let name = pct_decode(last);
            if !match_tokens(&name, toks) || !media_ok(&name, media_kind) {
                continue;
            }
            let title = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
            hits.push(Hit {
                title,
                url: abs,
                size: ent.size,
                kind: "http".into(),
                ..Default::default()
            });
            if hits.len() >= 500 {
                return Ok(hits);
            }
        }
    }
    Ok(hits)
}

// ---------- yt-dlp ----------

pub fn ytdlp_path() -> Option<PathBuf> {
    tools::ytdlp()
}

/// 解析 yt-dlp -J 的输出：播放列表返回各条目，单个视频返回它自己
pub fn parse_ytdlp_json(v: &serde_json::Value, fallback_url: &str) -> Vec<(String, String)> {
    if let Some(entries) = v["entries"].as_array() {
        return entries
            .iter()
            .filter_map(|e| {
                let url = e["webpage_url"].as_str().or(e["url"].as_str())?;
                let title = e["title"].as_str().map(collapse).filter(|t| !t.is_empty()).unwrap_or_else(|| url.to_string());
                Some((title, url.to_string()))
            })
            .collect();
    }
    let url = v["webpage_url"].as_str().unwrap_or(fallback_url).to_string();
    let title = v["title"].as_str().map(collapse).filter(|t| !t.is_empty()).unwrap_or_else(|| url.clone());
    vec![(title, url)]
}

/// 列出 yt-dlp 能识别的网址下的条目（不下载）
pub async fn list_ytdlp(url: &str) -> Result<Vec<(String, String)>, String> {
    let exe = ytdlp_path().ok_or_else(|| YTDLP_MISSING.to_string())?;
    throttle(url).await;
    let u = url.to_string();
    tokio::task::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join("music-player-ytdlp");
        let cookies = super::login::temp_cookies(&u, &tmp, &format!("list-{}", std::process::id()));
        let mut c = tools::command(&exe);
        c.args(["--flat-playlist", "-J", "--no-warnings", "--skip-download", "--socket-timeout", "20"]);
        if let Some(ck) = &cookies {
            c.arg("--cookies").arg(ck.path());
        }
        c.arg("--").arg(&u);
        let out = tools::run(c).map_err(|e| format!("yt-dlp 失败：{e}"))?;
        let v: serde_json::Value = serde_json::from_slice(&out).map_err(|e| format!("yt-dlp 的输出无法解析：{e}"))?;
        Ok(parse_ytdlp_json(&v, &u))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 添加来源时判断 yt-dlp 能不能识别这个网址（最多等 60 秒）
pub async fn probe_ytdlp(url: &str) -> bool {
    matches!(tokio::time::timeout(Duration::from_secs(60), list_ytdlp(url)).await, Ok(Ok(v)) if !v.is_empty())
}

pub async fn search_ytdlp(row: &SourceRow, query: &str) -> Result<Vec<Hit>, String> {
    let toks = tokens(query);
    let list = list_ytdlp(&row.url).await?;
    Ok(list
        .into_iter()
        .filter(|(t, _)| match_tokens(t, &toks))
        .map(|(title, url)| Hit {
            title,
            url,
            kind: "ytdlp".into(),
            ..Default::default()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fansub_release_counts_as_video() {
        assert!(media_ok("[XX字幕组] 星屑邮局 - 01 [1080p].mp4", "anime"));
        assert!(!media_ok("[XX字幕组] 星屑邮局 - 01 [1080p].mp4", "music"));
        assert!(media_ok("雾港.flac", "music"));
    }

    #[test]
    fn guess_kind_by_shape() {
        assert_eq!(guess_kind("D:\\NAS\\media"), Some("dir"));
        assert_eq!(guess_kind("\\\\nas\\media"), Some("dir"));
        assert_eq!(guess_kind("https://example.com/feed.xml"), Some("rss"));
        assert_eq!(guess_kind("https://example.com/rss/anime"), Some("rss"));
        assert_eq!(guess_kind("https://example.com/webdav/music"), Some("dir"));
        assert_eq!(guess_kind("https://example.com/search?q={q}"), None);
        assert_eq!(guess_kind("magnet:?xt=urn:btih:abc"), None);
    }

    #[test]
    fn local_path_detection() {
        assert!(is_local_path("E:\\Videos"));
        assert!(is_local_path("\\\\server\\share"));
        assert!(!is_local_path("https://a.b/c"));
    }

    #[test]
    fn query_is_encoded_into_template() {
        assert_eq!(
            fill_query("https://a.b/s?q={q}&x=1", "星屑 邮局"),
            "https://a.b/s?q=%E6%98%9F%E5%B1%91+%E9%82%AE%E5%B1%80&x=1"
        );
        assert_eq!(fill_query("https://a.b/list", "x"), "https://a.b/list");
    }

    #[test]
    fn size_text_to_bytes() {
        assert_eq!(parse_size("1.5 GB"), Some((1.5 * 1024.0 * 1024.0 * 1024.0) as i64));
        assert_eq!(parse_size("800MB"), Some(800 * 1024 * 1024));
        assert_eq!(parse_size("12 KiB"), Some(12 * 1024));
        assert_eq!(parse_size("unknown"), None);
    }

    #[test]
    fn rss_items_with_enclosure_and_magnet() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
            <item><title>[组] 星屑邮局 - 01 [1080p]</title><link>https://a.b/1</link>
              <enclosure url="https://a.b/1.mkv" length="123456" type="video/x-matroska"/>
              <pubDate>Mon, 01 Jan 2024 00:00:00 GMT</pubDate></item>
            <item><title>磁力版</title><link>magnet:?xt=urn:btih:ABC</link></item>
            <item><title>只有网页</title><link>https://a.b/page.html</link></item>
            </channel></rss>"#;
        let items = parse_feed(xml.as_bytes()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].url, "https://a.b/1.mkv");
        assert_eq!(items[0].size, Some(123456));
        assert_eq!(items[0].date.as_deref(), Some("Mon, 01 Jan 2024 00:00:00 GMT"));
        assert_eq!(kind_of_url(&items[1].url), "magnet");
    }

    #[test]
    fn atom_entries() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>f</title>
            <entry><title>Album 01</title><link rel="alternate" href="https://a.b/p"/><link rel="enclosure" href="https://a.b/t.torrent" length="99"/><updated>2024-01-02T00:00:00Z</updated></entry>
            </feed>"#;
        let items = parse_feed(xml.as_bytes()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, "https://a.b/t.torrent");
        assert_eq!(kind_of_url(&items[0].url), "torrent");
        assert_eq!(items[0].size, Some(99));
    }

    #[test]
    fn not_a_feed_is_rejected() {
        assert!(parse_feed(b"<html><body><p>hi</p></body></html>").is_err());
    }

    #[test]
    fn multistatus_entries() {
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/dav/music/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
          <D:response><D:href>/dav/a%20b.flac</D:href><D:propstat><D:prop><D:getcontentlength>4096</D:getcontentlength><D:resourcetype/></D:prop></D:propstat></D:response>
          </D:multistatus>"#;
        let v = parse_multistatus(xml);
        assert_eq!(v.len(), 2);
        assert!(v[0].is_dir);
        assert_eq!(v[1].size, Some(4096));
        assert!(!v[1].is_dir);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(pct_decode("a%20b%E6%98%9F.mkv"), "a b星.mkv");
        assert_eq!(pct_decode("100%"), "100%");
    }

    #[test]
    fn token_and_media_filters() {
        let t = tokens("星屑  邮局");
        assert!(match_tokens("[组] 星屑 邮局 - 01.mkv", &t));
        assert!(!match_tokens("星屑 - 01.mkv", &t));
        assert!(media_ok("a.flac", "music"));
        assert!(!media_ok("a.flac", "anime"));
        assert!(media_ok("a.mkv", "anime"));
        assert!(!media_ok("setup.exe", "anime"));
    }

    #[test]
    fn ytdlp_json_playlist_and_single() {
        let pl: serde_json::Value = serde_json::json!({"entries":[{"title":"A  B","url":"https://x/1"},{"title":"C","webpage_url":"https://x/2"}]});
        let v = parse_ytdlp_json(&pl, "https://x/list");
        assert_eq!(
            v,
            vec![("A B".to_string(), "https://x/1".to_string()), ("C".to_string(), "https://x/2".to_string())]
        );
        let one: serde_json::Value = serde_json::json!({"title":"Song","webpage_url":"https://x/s"});
        assert_eq!(parse_ytdlp_json(&one, "https://x/s"), vec![("Song".to_string(), "https://x/s".to_string())]);
    }

    #[test]
    fn collapse_spaces() {
        assert_eq!(collapse("  a \n b\t c "), "a b c");
    }
}
