//! B 站：扫码登录（Cookie 只存在 Windows 凭据管理器里）、账号状态、解析链接列出分 P / 剧集、给 yt-dlp 的 Cookie 文件和参数。
//! 只用用户自己的登录状态访问其本来就能看的内容；大会员专享、付费且未购买、带 DRM 的内容由 yt-dlp 报错，不做绕过。
use super::{provider, source, FetchSettings};
use crate::tools;
use anyhow::{anyhow, bail, Context, Result};
use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

const KEYRING_SERVICE: &str = "music-player-bili";
/// B 站接口会拒绝没有浏览器 UA 的请求
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const QR_GENERATE: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/generate";
const QR_POLL: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";
const SEARCH: &str = "https://api.bilibili.com/x/web-interface/search/type";
const NAV: &str = "https://api.bilibili.com/x/web-interface/nav";
/// 登录后要保存的 Cookie
const KEEP: &[&str] = &["SESSDATA", "bili_jct", "DedeUserID", "DedeUserID__ckMd5", "sid"];

pub type Cookies = BTreeMap<String, String>;

/// 是不是 B 站的网址（含 b23.tv 短链接）
pub fn is_bili_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url.trim()) else { return false };
    let h = u.host_str().unwrap_or("").to_ascii_lowercase();
    h == "bilibili.com" || h.ends_with(".bilibili.com") || h == "b23.tv"
}

/// 番剧 / 影视（ep、ss、md 页面）
pub fn is_bangumi_url(url: &str) -> bool {
    is_bili_url(url) && url.contains("/bangumi/")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default()
}

// ---------- Cookie 保存 ----------

fn entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, "default")?)
}

pub fn load_cookies() -> Option<Cookies> {
    let s = entry().ok()?.get_password().ok()?;
    let c: Cookies = serde_json::from_str(&s).ok()?;
    c.get("SESSDATA").filter(|v| !v.is_empty())?;
    Some(c)
}

fn save_cookies(c: &Cookies) -> Result<()> {
    entry()?.set_password(&serde_json::to_string(c)?)?;
    Ok(())
}

pub fn clear_cookies() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
}

pub fn cookie_header(c: &Cookies) -> String {
    c.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
}

/// Netscape 格式的 cookies.txt（yt-dlp 的 --cookies 读这个）
pub fn netscape(c: &Cookies) -> String {
    let mut s = String::from("# Netscape HTTP Cookie File\n");
    for (k, v) in c {
        s.push_str(&format!(".bilibili.com\tTRUE\t/\tFALSE\t2147483647\t{k}\t{v}\n"));
    }
    s
}

/// 临时 Cookie 文件：给一次 yt-dlp 调用用，用完（离开作用域）立即删除
pub struct TempCookies(PathBuf);

impl TempCookies {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCookies {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 已登录且是 B 站网址时，写一份临时 Cookie 文件
pub fn temp_cookies(url: &str, dir: &Path, tag: &str) -> Option<TempCookies> {
    if !is_bili_url(url) {
        return None;
    }
    let c = load_cookies()?;
    std::fs::create_dir_all(dir).ok()?;
    let p = dir.join(format!(".bili-cookies-{tag}.txt"));
    std::fs::write(&p, netscape(&c)).ok()?;
    Some(TempCookies(p))
}

// ---------- 扫码登录 ----------

#[derive(Serialize, Clone, Debug)]
pub struct Qr {
    pub key: String,
    pub url: String,
    /// 二维码图片（SVG 文本）
    pub svg: String,
}

pub fn qr_svg(data: &str) -> Result<String> {
    let code = qrcode::QrCode::new(data.as_bytes())?;
    Ok(code.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).quiet_zone(true).build())
}

pub async fn qr_start() -> Result<Qr> {
    let v: Value = client().get(QR_GENERATE).send().await.context("连接 B 站失败")?.json().await?;
    if v["code"].as_i64() != Some(0) {
        bail!("获取登录二维码失败：{}", v["message"].as_str().unwrap_or("未知错误"));
    }
    let url = v["data"]["url"].as_str().ok_or_else(|| anyhow!("B 站没有返回二维码"))?.to_string();
    let key = v["data"]["qrcode_key"].as_str().ok_or_else(|| anyhow!("B 站没有返回二维码"))?.to_string();
    let svg = qr_svg(&url)?;
    Ok(Qr { key, url, svg })
}

/// 扫码状态码 → waiting / scanned / expired / ok
pub fn qr_status(code: i64) -> &'static str {
    match code {
        0 => "ok",
        86090 => "scanned",
        86038 => "expired",
        _ => "waiting",
    }
}

/// 从登录成功返回的跳转网址里取 Cookie（SESSDATA 等在查询参数里）
pub fn cookies_from_url(url: &str) -> Cookies {
    let mut c = Cookies::new();
    if let Ok(u) = url::Url::parse(url) {
        for (k, v) in u.query_pairs() {
            if KEEP.contains(&k.as_ref()) && !v.is_empty() {
                c.insert(k.into_owned(), v.into_owned());
            }
        }
    }
    c
}

/// Set-Cookie 头里的“名=值”
pub fn cookie_from_set_cookie(h: &str) -> Option<(String, String)> {
    let first = h.split(';').next()?;
    let (k, v) = first.split_once('=')?;
    let k = k.trim();
    KEEP.contains(&k).then(|| (k.to_string(), v.trim().to_string()))
}

/// 轮询一次扫码结果；成功时保存 Cookie
pub async fn qr_poll(key: &str) -> Result<&'static str> {
    let resp = client().get(QR_POLL).query(&[("qrcode_key", key)]).send().await.context("连接 B 站失败")?;
    let mut cookies = Cookies::new();
    for h in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        if let Some((k, v)) = h.to_str().ok().and_then(cookie_from_set_cookie) {
            cookies.insert(k, v);
        }
    }
    let v: Value = resp.json().await?;
    if v["code"].as_i64() != Some(0) {
        bail!("查询扫码状态失败：{}", v["message"].as_str().unwrap_or("未知错误"));
    }
    let status = qr_status(v["data"]["code"].as_i64().unwrap_or(-1));
    if status == "ok" {
        cookies.extend(cookies_from_url(v["data"]["url"].as_str().unwrap_or("")));
        if !cookies.contains_key("SESSDATA") {
            bail!("登录成功，但没有拿到登录凭据，请重试");
        }
        save_cookies(&cookies)?;
    }
    Ok(status)
}

// ---------- 账号 ----------

#[derive(Serialize, Clone, Debug, Default)]
pub struct Account {
    pub logged_in: bool,
    /// 保存过登录凭据但已经失效
    pub expired: bool,
    pub name: String,
    pub vip: bool,
    /// 例如“年度大会员”
    pub vip_label: String,
}

pub fn parse_nav(v: &Value) -> Account {
    let d = &v["data"];
    if v["code"].as_i64() != Some(0) || d["isLogin"].as_bool() != Some(true) {
        return Account::default();
    }
    Account {
        logged_in: true,
        expired: false,
        name: d["uname"].as_str().unwrap_or("").to_string(),
        vip: d["vipStatus"].as_i64() == Some(1),
        vip_label: d["vip_label"]["text"].as_str().unwrap_or("").to_string(),
    }
}

pub async fn account() -> Result<Account> {
    let Some(c) = load_cookies() else { return Ok(Account::default()) };
    let v: Value = client()
        .get(NAV)
        .header(reqwest::header::COOKIE, cookie_header(&c))
        .send()
        .await
        .context("连接 B 站失败")?
        .json()
        .await?;
    let mut a = parse_nav(&v);
    if !a.logged_in {
        a.expired = true;
    }
    Ok(a)
}

// ---------- 解析链接 ----------

#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    pub title: String,
    pub url: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Parsed {
    pub title: String,
    /// bangumi：番剧 / 影视；video：普通视频（可能多 P）；list：收藏夹、合集、UP 主投稿等
    pub kind: &'static str,
    /// 建议的入库类型：anime / video
    pub media_kind: &'static str,
    pub entries: Vec<Entry>,
}

/// 解析一个 B 站链接，列出里面的分 P / 剧集 / 视频（不下载）
pub async fn parse(url: &str, tmp: &Path) -> Result<Parsed> {
    let url = url.trim().to_string();
    if !is_bili_url(&url) {
        bail!("这不是 B 站的链接");
    }
    let exe = source::ytdlp_path().ok_or_else(|| anyhow!(source::YTDLP_MISSING))?;
    source::throttle(&url).await;
    let tmp = tmp.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Parsed> {
        let cookies = temp_cookies(&url, &tmp, "parse");
        let mut c = tools::command(&exe);
        c.args(["--flat-playlist", "-J", "--no-warnings", "--skip-download", "--socket-timeout", "20"]);
        if let Some(ck) = &cookies {
            c.arg("--cookies").arg(ck.path());
        }
        c.arg("--").arg(&url);
        let out = tools::run(c).map_err(|e| anyhow!("{}", friendly_error(&e.to_string())))?;
        let v: Value = serde_json::from_slice(&out).context("yt-dlp 的输出无法解析")?;
        Ok(parsed_from_json(&v, &url))
    })
    .await?
}

pub fn parsed_from_json(v: &Value, url: &str) -> Parsed {
    let entries = source::parse_ytdlp_json(v, url).into_iter().map(|(title, url)| Entry { title, url }).collect();
    let title = v["title"].as_str().unwrap_or("").to_string();
    let bangumi = is_bangumi_url(url)
        || v["entries"]
            .as_array()
            .is_some_and(|e| e.iter().any(|x| x["url"].as_str().is_some_and(is_bangumi_url)));
    let kind = if bangumi {
        "bangumi"
    } else if url.contains("/video/") || url.contains("b23.tv") {
        "video"
    } else {
        "list"
    };
    Parsed {
        title,
        kind,
        media_kind: if bangumi { "anime" } else { "video" },
        entries,
    }
}

/// 把 yt-dlp 的英文报错翻成用户看得懂的话
pub fn friendly_error(err: &str) -> String {
    let l = err.to_ascii_lowercase();
    if l.contains("premium member") {
        "这个内容需要大会员。请先用开通了大会员的账号扫码登录（软件不会绕过会员限制）。".into()
    } else if l.contains("purchase") {
        "这个内容需要购买，请先在 B 站购买后再下载。".into()
    } else if l.contains("drm") {
        "这个内容有 DRM 加密保护，无法下载。".into()
    } else if l.contains("login") || l.contains("log in") {
        "需要登录：请先在“B 站”面板里扫码登录。".into()
    } else if l.contains("412") || l.contains("429") || l.contains("too many requests") {
        "B 站暂时限制了访问（请求太频繁），请过几分钟再试。".into()
    } else {
        err.to_string()
    }
}

// ---------- 下载源（搜索页里搜 B 站） ----------

/// 接到“下载源接口”上的 B 站：在搜索页按关键字搜视频，下载交给 yt-dlp（清晰度、字幕、弹幕按设置）。
/// 番剧请在 B 站面板粘贴链接（搜索接口只返回普通视频）。
#[derive(Default)]
pub struct BiliProvider;

/// 搜索结果标题里的 <em class="keyword"> 高亮和 HTML 实体
pub fn clean_title(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// “3:45” / “1:02:03” → 秒
pub fn parse_duration(s: &str) -> Option<f64> {
    let mut t = 0.0;
    for part in s.split(':') {
        t = t * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    Some(t)
}

pub fn search_items(v: &Value, media_kind: &str) -> Vec<provider::Item> {
    let Some(list) = v["data"]["result"].as_array() else { return vec![] };
    list.iter()
        .filter_map(|r| {
            let bvid = r["bvid"].as_str().filter(|s| !s.is_empty())?;
            let url = r["arcurl"]
                .as_str()
                .filter(|u| u.starts_with("http"))
                .map(|u| u.replace("http://", "https://"))
                .unwrap_or_else(|| format!("https://www.bilibili.com/video/{bvid}"));
            let cover = r["pic"].as_str().unwrap_or("");
            let cover = if cover.starts_with("//") {
                format!("https:{cover}")
            } else {
                cover.to_string()
            };
            let author = r["author"].as_str().unwrap_or("").to_string();
            Some(provider::Item {
                id: bvid.to_string(),
                title: clean_title(r["title"].as_str().unwrap_or(bvid)),
                // 搜索页按 album 分组：同一个 UP 主的视频放在一起
                album: author.clone(),
                artist: author,
                duration: r["duration"].as_str().and_then(parse_duration),
                cover,
                media_kind: media_kind.to_string(),
                qualities: qualities(media_kind),
                extra: serde_json::json!({ "url": url }),
                ..Default::default()
            })
        })
        .collect()
}

/// 搜索页的档位下拉框。key 是清晰度上限（画面高度），“0”为账号能拿到的最高，空串按设置；只要音频时不分档位
pub fn qualities(media_kind: &str) -> Vec<provider::Quality> {
    if media_kind == "music" {
        return vec![provider::Quality {
            key: "0".into(),
            label: "音频".into(),
            format: "m4a".into(),
            ..Default::default()
        }];
    }
    [("", "按设置"), ("0", "最高"), ("1080", "1080P"), ("720", "720P"), ("480", "480P")]
        .into_iter()
        .map(|(k, l)| provider::Quality {
            key: k.into(),
            label: l.into(),
            format: "mp4".into(),
            ..Default::default()
        })
        .collect()
}

impl provider::Provider for BiliProvider {
    fn info(&self) -> provider::ProviderInfo {
        provider::ProviderInfo {
            id: "bilibili".into(),
            name: "B 站".into(),
            media_kinds: vec!["video".into(), "music".into()],
            note: "搜 B 站视频，下载交给 yt-dlp；清晰度、弹幕、字幕在设置里调。番剧请在 B 站面板粘贴链接。只下载你的账号能看的内容。".into(),
        }
    }

    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<provider::Item>, String>> {
        Box::pin(async move {
            source::throttle(SEARCH).await;
            // 搜索接口需要 buvid3；已登录时带上登录 Cookie
            let mut cookie = format!("buvid3={}infoc", uuid_like());
            if let Some(c) = load_cookies() {
                cookie = format!("{cookie}; {}", cookie_header(&c));
            }
            let page = page.max(1).to_string();
            let v: Value = client()
                .get(SEARCH)
                .header(reqwest::header::COOKIE, cookie)
                .header(reqwest::header::REFERER, "https://search.bilibili.com/")
                .query(&[("search_type", "video"), ("keyword", query), ("page", page.as_str())])
                .send()
                .await
                .map_err(|e| format!("连接 B 站失败：{e}"))?
                .json()
                .await
                .map_err(|e| format!("B 站返回的内容无法解析：{e}"))?;
            match v["code"].as_i64() {
                Some(0) => Ok(search_items(&v, media_kind)),
                Some(-412) => Err(friendly_error("412")),
                _ => Err(format!("B 站搜索失败：{}", v["message"].as_str().unwrap_or("未知错误"))),
            }
        })
    }

    fn resolve<'a>(&'a self, item: &'a provider::Item, _quality: &'a str) -> BoxFuture<'a, Result<provider::Resolved, String>> {
        Box::pin(async move {
            let url = item.extra["url"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("https://www.bilibili.com/video/{}", item.id));
            Ok(provider::Resolved {
                target: provider::Target::Ytdlp { url },
                lyric: None,
            })
        })
    }
}

/// 随机的 buvid3（只要格式像就行，不需要真实设备号）
fn uuid_like() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let h = format!("{:032X}", n.wrapping_mul(6364136223846793005) ^ (std::process::id() as u128));
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

// ---------- 给 yt-dlp 的下载参数 ----------

/// 一次 yt-dlp 下载的选项
#[derive(Debug, Clone, Default)]
pub struct YtOpts {
    pub bili: bool,
    pub bangumi: bool,
    pub audio_only: bool,
    /// 最高清晰度（高度），0 表示不限
    pub quality: u32,
    pub danmaku: bool,
    pub subs: bool,
    pub ffmpeg: Option<PathBuf>,
    pub cookies: Option<PathBuf>,
}

impl YtOpts {
    pub fn new(url: &str, media_kind: &str, s: &FetchSettings) -> Self {
        let bili = is_bili_url(url);
        YtOpts {
            bili,
            bangumi: bili && (is_bangumi_url(url) || media_kind == "anime"),
            audio_only: media_kind == "music",
            quality: s.bili_quality,
            danmaku: s.bili_danmaku,
            subs: s.bili_subs,
            ffmpeg: tools::ffmpeg(),
            cookies: None,
        }
    }
}

/// 组装 yt-dlp 参数（不含网址）。B 站：清晰度上限、优先 H.264（内置播放器兼容最好）、字幕与弹幕、请求间隔；
/// 番剧用“作品名 - S01E03”命名，方便整理成番剧库；音乐只下音频
pub fn ytdlp_args(o: &YtOpts, out_dir: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["--newline".into(), "--no-playlist".into(), "--no-warnings".into(), "-P".into(), out_dir.into()];
    if let Some(f) = &o.ffmpeg {
        a.push("--ffmpeg-location".into());
        a.push(f.into());
    }
    if let Some(c) = &o.cookies {
        a.push("--cookies".into());
        a.push(c.into());
    }
    let template = if o.bangumi {
        "%(series,playlist_title,title).100B - S%(season_number|1)02dE%(episode_number|0)02d [%(id)s].%(ext)s"
    } else {
        "%(title).150B [%(id)s].%(ext)s"
    };
    a.push("-o".into());
    a.push(template.into());
    if o.audio_only {
        a.push("-f".into());
        a.push("ba/b".into());
        if o.ffmpeg.is_some() {
            // 曲名、UP 主（作为艺人）和封面写进音频标签，整理时按标签归到音乐库
            a.extend(["--embed-metadata".into(), "--embed-thumbnail".into()]);
        }
    }
    if o.bili {
        a.extend(["--sleep-requests".into(), "1".into()]);
        if !o.audio_only {
            let sort = if o.quality > 0 {
                format!("res:{},codec:h264", o.quality)
            } else {
                "res,codec:h264".into()
            };
            a.extend(["-S".into(), sort.into(), "--merge-output-format".into(), "mp4".into()]);
            let langs = match (o.subs, o.danmaku) {
                (true, true) => Some("all"),
                (true, false) => Some("all,-danmaku"),
                (false, true) => Some("danmaku"),
                (false, false) => None,
            };
            if let Some(l) = langs {
                a.extend(["--write-subs".into(), "--sub-langs".into(), l.into()]);
            }
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn recognizes_bili_urls() {
        assert!(is_bili_url("https://www.bilibili.com/video/BV1xx411c7mD"));
        assert!(is_bili_url("https://b23.tv/abc"));
        assert!(is_bili_url("https://space.bilibili.com/123/favlist?fid=456"));
        assert!(!is_bili_url("https://notbilibili.com/video/x"));
        assert!(!is_bili_url("https://www.youtube.com/watch?v=1"));
        assert!(is_bangumi_url("https://www.bilibili.com/bangumi/play/ep123"));
        assert!(!is_bangumi_url("https://www.bilibili.com/video/BV1"));
    }

    #[test]
    fn netscape_file_and_header() {
        let mut c = Cookies::new();
        c.insert("SESSDATA".into(), "abc%2C1".into());
        c.insert("bili_jct".into(), "x".into());
        let s = netscape(&c);
        assert!(s.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(s.contains(".bilibili.com\tTRUE\t/\tFALSE\t2147483647\tSESSDATA\tabc%2C1\n"));
        assert_eq!(cookie_header(&c), "SESSDATA=abc%2C1; bili_jct=x");
    }

    #[test]
    fn login_cookies_from_redirect_and_headers() {
        let c = cookies_from_url(
            "https://passport.biligame.com/x/passport-login/web/crossDomain?DedeUserID=1&DedeUserID__ckMd5=ab&Expires=1&SESSDATA=s%2C1&bili_jct=j&gourl=x",
        );
        assert_eq!(c.get("SESSDATA").map(String::as_str), Some("s,1"));
        assert_eq!(c.get("bili_jct").map(String::as_str), Some("j"));
        assert!(!c.contains_key("gourl") && !c.contains_key("Expires"));
        assert_eq!(
            cookie_from_set_cookie("SESSDATA=abc; Path=/; Domain=bilibili.com; HttpOnly"),
            Some(("SESSDATA".into(), "abc".into()))
        );
        assert_eq!(cookie_from_set_cookie("other=1; Path=/"), None);
    }

    #[test]
    fn qr_codes() {
        assert_eq!(qr_status(0), "ok");
        assert_eq!(qr_status(86090), "scanned");
        assert_eq!(qr_status(86038), "expired");
        assert_eq!(qr_status(86101), "waiting");
        let svg = qr_svg("https://account.bilibili.com/h5/account-h5/auth/scan-web?qrcode_key=abc").unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn nav_response() {
        let v = serde_json::json!({"code":0,"data":{"isLogin":true,"uname":"潇","vipStatus":1,"vip_label":{"text":"年度大会员"}}});
        let a = parse_nav(&v);
        assert!(a.logged_in && a.vip);
        assert_eq!(a.name, "潇");
        assert_eq!(a.vip_label, "年度大会员");
        let v2 = serde_json::json!({"code":-101,"data":{"isLogin":false}});
        assert!(!parse_nav(&v2).logged_in);
    }

    #[test]
    fn parsed_lists() {
        let v = serde_json::json!({"title":"某番","entries":[
            {"title":"第1话","url":"https://www.bilibili.com/bangumi/play/ep1"},
            {"title":"第2话","url":"https://www.bilibili.com/bangumi/play/ep2"}]});
        let p = parsed_from_json(&v, "https://www.bilibili.com/bangumi/play/ss9");
        assert_eq!(p.kind, "bangumi");
        assert_eq!(p.media_kind, "anime");
        assert_eq!(p.entries.len(), 2);
        let v2 = serde_json::json!({"title":"单个视频","webpage_url":"https://www.bilibili.com/video/BV1"});
        let p2 = parsed_from_json(&v2, "https://www.bilibili.com/video/BV1");
        assert_eq!((p2.kind, p2.media_kind, p2.entries.len()), ("video", "video", 1));
        let p3 = parsed_from_json(
            &serde_json::json!({"title":"收藏夹","entries":[]}),
            "https://space.bilibili.com/1/favlist?fid=2",
        );
        assert_eq!(p3.kind, "list");
    }

    #[test]
    fn errors_are_translated() {
        assert!(friendly_error("ERROR: This video is for premium members only").contains("大会员"));
        assert!(friendly_error("HTTP Error 412: Precondition Failed").contains("频繁"));
        assert_eq!(friendly_error("something else"), "something else");
    }

    #[test]
    fn provider_search_results() {
        let v = serde_json::json!({"code":0,"data":{"result":[
            {"bvid":"BV1ab","title":"<em class=\"keyword\">星屑</em>邮局 &amp; 片头","author":"UP主","arcurl":"http://www.bilibili.com/video/av1","pic":"//i0.hdslb.com/a.jpg","duration":"3:45"},
            {"bvid":"","title":"没有 bvid 的跳过"}]}});
        let items = search_items(&v, "music");
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.title, "星屑邮局 & 片头");
        assert_eq!(it.artist, "UP主");
        assert_eq!(it.album, "UP主");
        assert_eq!(it.qualities.len(), 1, "只要音频时不分档位");
        assert_eq!(
            qualities("video").iter().map(|q| q.key.as_str()).collect::<Vec<_>>(),
            ["", "0", "1080", "720", "480"]
        );
        assert_eq!(it.cover, "https://i0.hdslb.com/a.jpg");
        assert_eq!(it.duration, Some(225.0));
        assert_eq!(it.media_kind, "music");
        assert_eq!(it.extra["url"], "https://www.bilibili.com/video/av1");
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration("x"), None);
        assert_eq!(uuid_like().len(), 36);
    }

    #[test]
    fn provider_resolves_to_ytdlp() {
        use provider::Provider;
        let p = BiliProvider;
        assert_eq!(p.info().id, "bilibili");
        let it = provider::Item {
            id: "BV1ab".into(),
            ..Default::default()
        };
        let r = futures_util::FutureExt::now_or_never(p.resolve(&it, "")).unwrap().unwrap();
        assert_eq!(
            r.target,
            provider::Target::Ytdlp {
                url: "https://www.bilibili.com/video/BV1ab".into()
            }
        );
    }

    fn args_text(o: &YtOpts) -> String {
        ytdlp_args(o, Path::new("out"))
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn ytdlp_args_for_bili() {
        let mut o = YtOpts {
            bili: true,
            quality: 1080,
            danmaku: true,
            subs: true,
            ..Default::default()
        };
        let s = args_text(&o);
        assert!(s.contains("-S res:1080,codec:h264"));
        assert!(s.contains("--write-subs --sub-langs all"));
        assert!(s.contains("--sleep-requests 1"));
        assert!(s.contains("%(title).150B [%(id)s].%(ext)s"));
        o.quality = 0;
        o.danmaku = false;
        let s = args_text(&o);
        assert!(s.contains("-S res,codec:h264"));
        assert!(s.contains("--sub-langs all,-danmaku"));
        o.subs = false;
        assert!(!args_text(&o).contains("--write-subs"));
        o.danmaku = true;
        assert!(args_text(&o).contains("--sub-langs danmaku"));
        o.cookies = Some(PathBuf::from("c.txt"));
        assert!(args_text(&o).contains("--cookies c.txt"));
    }

    #[test]
    fn ytdlp_args_bangumi_and_audio() {
        let o = YtOpts {
            bili: true,
            bangumi: true,
            ..Default::default()
        };
        assert!(args_text(&o).contains("S%(season_number|1)02dE%(episode_number|0)02d"));
        // 输出的文件名能被识别成番剧的季和集
        let info = parse::parse_episode(Path::new("某番 第二季 - S01E03 [ep123].mp4"));
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3.0));
        let a = YtOpts {
            bili: true,
            audio_only: true,
            ffmpeg: Some(PathBuf::from("ffmpeg.exe")),
            ..Default::default()
        };
        let s = args_text(&a);
        assert!(s.contains("-f ba/b") && s.contains("--embed-metadata"));
        assert!(!s.contains("-S "), "只下音频时不挑清晰度");
        // 非 B 站网址不加 B 站参数
        let g = YtOpts::default();
        let s = args_text(&g);
        assert!(!s.contains("--sleep-requests") && !s.contains("--write-subs"));
        assert!(s.starts_with("--newline --no-playlist"));
    }
}
