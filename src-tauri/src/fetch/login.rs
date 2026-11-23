//! 网站登录：在软件里打开一个浏览器窗口，用户自己登录；登录后把这个网站的 Cookie 存进 Windows 凭据管理器，
//! 之后识别链接、搜索、HTTP 下载、yt-dlp（--cookies）访问这个网站时自动带上。
//!
//! 只保存用户自己登录得到的 Cookie，不自动填写账号密码，也不处理验证码；访问限制由网站自己决定，软件不绕过。
//! Cookie 按网站分开保存（压缩后分段写入凭据管理器，每段不超过系统上限），可以在“自动获取 → 设置”里查看和清除。
//! 调试版用 MP_DATA_DIR 跑测试实例时用单独的凭据名，不碰正式登录。
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const WINDOW: &str = "site-login";
/// 凭据管理器单条上限 2560 字节，留余量
const CHUNK: usize = 2000;
const MAX_CHUNKS: usize = 32;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    /// 空表示只对登录时的主机有效
    pub domain: String,
    pub path: String,
    pub secure: bool,
    /// Unix 秒；0 表示会话 Cookie
    pub expires: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SiteLogin {
    /// 网站（登录页的主机名，去掉 www.）
    pub site: String,
    /// 登录页地址
    pub url: String,
    pub saved_at: i64,
    pub cookies: Vec<StoredCookie>,
}

/// 给界面看的（不含 Cookie 内容）
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LoginView {
    pub site: String,
    pub url: String,
    pub saved_at: i64,
    pub count: usize,
}

impl SiteLogin {
    fn view(&self) -> LoginView {
        LoginView {
            site: self.site.clone(),
            url: self.url.clone(),
            saved_at: self.saved_at,
            count: self.cookies.len(),
        }
    }
}

// ---------- 网址与 Cookie 匹配 ----------

/// 网址的主机名（小写，去掉 www.）
pub fn site_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let h = u.host_str()?.to_ascii_lowercase();
    Some(h.strip_prefix("www.").unwrap_or(&h).to_string())
}

/// Cookie 的 domain 是否适用于这个主机（RFC 6265 的域匹配）
fn domain_match(host: &str, cookie_domain: &str, login_site: &str) -> bool {
    let d = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    if d.is_empty() {
        // 只对登录主机有效
        return host == login_site || host == format!("www.{login_site}");
    }
    host == d || host.ends_with(&format!(".{d}"))
}

fn path_match(req_path: &str, cookie_path: &str) -> bool {
    let cp = if cookie_path.is_empty() { "/" } else { cookie_path };
    req_path == cp || (req_path.starts_with(cp) && (cp.ends_with('/') || req_path[cp.len()..].starts_with('/')))
}

/// 从所有登录里挑出适用于这个网址的 Cookie（同名时后保存的优先）
pub fn cookies_for(logins: &[SiteLogin], url: &str, now: i64) -> Vec<StoredCookie> {
    let Ok(u) = url::Url::parse(url) else { return vec![] };
    let Some(host) = u.host_str().map(|h| h.to_ascii_lowercase()) else {
        return vec![];
    };
    let https = u.scheme() == "https";
    let mut sorted: Vec<&SiteLogin> = logins.iter().collect();
    sorted.sort_by_key(|l| l.saved_at);
    let mut out: Vec<StoredCookie> = vec![];
    for l in sorted {
        for c in &l.cookies {
            if c.expires > 0 && c.expires < now {
                continue;
            }
            if c.secure && !https {
                continue;
            }
            if !domain_match(&host, &c.domain, &l.site) || !path_match(u.path(), &c.path) {
                continue;
            }
            out.retain(|x| x.name != c.name);
            out.push(c.clone());
        }
    }
    out
}

pub fn header_of(cs: &[StoredCookie]) -> String {
    cs.iter().map(|c| format!("{}={}", c.name, c.value)).collect::<Vec<_>>().join("; ")
}

/// Netscape 格式的 cookies.txt（yt-dlp 的 --cookies 读这个）
pub fn netscape(cs: &[StoredCookie], host: &str) -> String {
    let mut s = String::from("# Netscape HTTP Cookie File\n");
    for c in cs {
        let (domain, sub) = if c.domain.is_empty() {
            (host.to_string(), "FALSE")
        } else {
            (format!(".{}", c.domain.trim_start_matches('.')), "TRUE")
        };
        let path = if c.path.is_empty() { "/" } else { &c.path };
        // 会话 Cookie 写成远期过期，yt-dlp 才会读进去
        let exp = if c.expires > 0 { c.expires } else { 2_147_483_647 };
        s.push_str(&format!(
            "{domain}\t{sub}\t{path}\t{}\t{exp}\t{}\t{}\n",
            if c.secure { "TRUE" } else { "FALSE" },
            c.name,
            c.value
        ));
    }
    s
}

// ---------- 保存（Windows 凭据管理器，压缩后分段） ----------

fn service() -> &'static str {
    if cfg!(test) || (cfg!(debug_assertions) && std::env::var_os("MP_DATA_DIR").is_some()) {
        "music-player-site-login-test"
    } else {
        "music-player-site-login"
    }
}

fn entry(name: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(service(), name)?)
}

pub fn encode(logins: &[SiteLogin]) -> Result<Vec<String>> {
    let json = serde_json::to_vec(logins)?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&json)?;
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(z.finish()?);
    let parts: Vec<String> = b64.as_bytes().chunks(CHUNK).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    if parts.len() > MAX_CHUNKS {
        return Err(anyhow!("保存的登录信息太多了，请先清除不用的网站"));
    }
    Ok(parts)
}

pub fn decode(parts: &[String]) -> Result<Vec<SiteLogin>> {
    let raw = base64::engine::general_purpose::STANDARD_NO_PAD.decode(parts.concat())?;
    let mut s = Vec::new();
    flate2::read::DeflateDecoder::new(&raw[..]).read_to_end(&mut s)?;
    Ok(serde_json::from_slice(&s)?)
}

fn read_store() -> Vec<SiteLogin> {
    let n: usize = entry("count")
        .and_then(|e| Ok(e.get_password()?))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if n == 0 {
        return vec![];
    }
    let parts: Option<Vec<String>> = (0..n.min(MAX_CHUNKS)).map(|i| entry(&format!("part{i}")).ok()?.get_password().ok()).collect();
    parts.and_then(|p| decode(&p).ok()).unwrap_or_default()
}

fn write_store(logins: &[SiteLogin]) -> Result<()> {
    let old: usize = entry("count")
        .and_then(|e| Ok(e.get_password()?))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let parts = if logins.is_empty() { vec![] } else { encode(logins)? };
    for (i, p) in parts.iter().enumerate() {
        entry(&format!("part{i}"))?.set_password(p).context("写入 Windows 凭据管理器失败")?;
    }
    for i in parts.len()..old {
        if let Ok(e) = entry(&format!("part{i}")) {
            let _ = e.delete_credential();
        }
    }
    if parts.is_empty() {
        if let Ok(e) = entry("count") {
            let _ = e.delete_credential();
        }
    } else {
        entry("count")?.set_password(&parts.len().to_string())?;
    }
    Ok(())
}

/// 内存里的副本：每次请求都会查，不能每次都读凭据管理器
static STORE: Lazy<RwLock<Option<Vec<SiteLogin>>>> = Lazy::new(|| RwLock::new(None));

fn with_store<T>(f: impl FnOnce(&[SiteLogin]) -> T) -> T {
    if STORE.read().is_none() {
        let loaded = read_store();
        let mut w = STORE.write();
        if w.is_none() {
            *w = Some(loaded);
        }
    }
    f(STORE.read().as_deref().unwrap_or(&[]))
}

fn save_site(login: SiteLogin) -> Result<()> {
    let mut all = with_store(|s| s.to_vec());
    all.retain(|l| l.site != login.site);
    all.push(login);
    write_store(&all)?;
    *STORE.write() = Some(all);
    Ok(())
}

pub fn list() -> Vec<LoginView> {
    let mut v: Vec<LoginView> = with_store(|s| s.iter().map(|l| l.view()).collect());
    v.sort_by(|a, b| a.site.cmp(&b.site));
    v
}

pub fn clear(site: &str) -> Result<()> {
    let mut all = with_store(|s| s.to_vec());
    all.retain(|l| l.site != site);
    write_store(&all)?;
    *STORE.write() = Some(all);
    Ok(())
}

/// 请求这个网址时要带的 Cookie 头（没有登录过时为 None）
pub fn cookie_header(url: &str) -> Option<String> {
    let cs = with_store(|s| cookies_for(s, url, crate::db::now()));
    (!cs.is_empty()).then(|| header_of(&cs))
}

/// 临时 Cookie 文件：给一次 yt-dlp 调用用，离开作用域时删除
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

/// 登录过这个网站时，写一份给 yt-dlp 的临时 Cookie 文件
pub fn temp_cookies(url: &str, dir: &Path, tag: &str) -> Option<TempCookies> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    let cs = with_store(|s| cookies_for(s, url, crate::db::now()));
    if cs.is_empty() {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    let p = dir.join(format!(".site-cookies-{tag}.txt"));
    std::fs::write(&p, netscape(&cs, &host)).ok()?;
    Some(TempCookies(p))
}

// ---------- 登录窗口 ----------

/// 正在登录的网址
static PENDING: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[derive(Serialize, Clone, Debug)]
pub struct LoginEvent {
    /// saved / canceled / error
    pub status: String,
    pub site: String,
    pub message: String,
}

fn emit(app: &AppHandle, status: &str, site: &str, message: &str) {
    let _ = app.emit(
        "fetch://login",
        LoginEvent {
            status: status.into(),
            site: site.into(),
            message: message.into(),
        },
    );
}

/// 打开登录窗口（已经开着就换到这个网址）
pub fn open(app: &AppHandle, url: &str) -> Result<String> {
    let site = site_of(url).ok_or_else(|| anyhow!("请填写以 http:// 或 https:// 开头的网址"))?;
    let target: url::Url = url.trim().parse()?;
    *PENDING.lock() = Some(target.to_string());
    if let Some(w) = app.get_webview_window(WINDOW) {
        w.navigate(target)?;
        let _ = w.set_title(&format!("登录 {site}（登录完成后关闭这个窗口，或回到播放器点“完成登录”）"));
        let _ = w.set_focus();
        return Ok(site);
    }
    let win = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(target))
        .title(format!("登录 {site}（登录完成后关闭这个窗口，或回到播放器点“完成登录”）"))
        .inner_size(1000.0, 760.0)
        .center()
        .build()
        .context("打开登录窗口失败")?;
    let app2 = app.clone();
    let win2 = win.clone();
    win.on_window_event(move |ev| {
        if let WindowEvent::CloseRequested { api, .. } = ev {
            // 关窗口 = 完成登录：先读 Cookie 再关（Windows 上不能在事件回调里同步读）
            if PENDING.lock().is_some() {
                api.prevent_close();
                let (app3, w) = (app2.clone(), win2.clone());
                tauri::async_runtime::spawn(async move {
                    let _ = finish_inner(&app3).await;
                    let _ = w.destroy();
                });
            }
        }
    });
    Ok(site)
}

/// 读取登录窗口里的 Cookie 并保存，然后关窗口
pub async fn finish(app: &AppHandle) -> Result<LoginView> {
    let r = finish_inner(app).await;
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.destroy();
    }
    r
}

async fn finish_inner(app: &AppHandle) -> Result<LoginView> {
    let Some(start) = PENDING.lock().take() else {
        return Err(anyhow!("没有正在进行的登录"));
    };
    let site = site_of(&start).unwrap_or_default();
    let res = collect(app, &start, &site);
    match &res {
        Ok(v) if v.count > 0 => emit(app, "saved", &site, &format!("已保存 {site} 的登录状态")),
        Ok(_) => emit(app, "canceled", &site, "没有读到这个网站的 Cookie，可能还没有登录成功"),
        Err(e) => emit(app, "error", &site, &e.to_string()),
    }
    res
}

fn collect(app: &AppHandle, start: &str, site: &str) -> Result<LoginView> {
    let w = app.get_webview_window(WINDOW).ok_or_else(|| anyhow!("登录窗口已经关闭"))?;
    let mut urls: Vec<url::Url> = vec![start.parse()?];
    if let Ok(cur) = w.url() {
        if matches!(cur.scheme(), "http" | "https") && !urls.contains(&cur) {
            urls.push(cur);
        }
    }
    let mut cookies: Vec<StoredCookie> = vec![];
    for u in urls {
        let host = u.host_str().unwrap_or("").to_ascii_lowercase();
        for c in w.cookies_for_url(u).context("读取登录窗口的 Cookie 失败")? {
            let sc = StoredCookie {
                name: c.name().to_string(),
                value: c.value().to_string(),
                // 没写 domain 的是“只对当前主机”，记成那个主机
                domain: c.domain().map(|d| d.to_string()).unwrap_or_else(|| host.clone()),
                path: c.path().unwrap_or("/").to_string(),
                secure: c.secure().unwrap_or(false),
                expires: c.expires().and_then(|e| e.datetime()).map(|t| t.unix_timestamp()).unwrap_or(0),
            };
            cookies.retain(|x| !(x.name == sc.name && x.domain == sc.domain && x.path == sc.path));
            cookies.push(sc);
        }
    }
    let login = SiteLogin {
        site: site.to_string(),
        url: start.to_string(),
        saved_at: crate::db::now(),
        cookies,
    };
    let view = login.view();
    if view.count > 0 {
        save_site(login)?;
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ck(name: &str, domain: &str, path: &str, secure: bool, expires: i64) -> StoredCookie {
        StoredCookie {
            name: name.into(),
            value: format!("v-{name}"),
            domain: domain.into(),
            path: path.into(),
            secure,
            expires,
        }
    }

    fn login(site: &str, saved_at: i64, cookies: Vec<StoredCookie>) -> SiteLogin {
        SiteLogin {
            site: site.into(),
            url: format!("https://{site}/login"),
            saved_at,
            cookies,
        }
    }

    #[test]
    fn site_names() {
        assert_eq!(site_of("https://www.Example.com/a?b=1").as_deref(), Some("example.com"));
        assert_eq!(site_of("http://pan.example.org").as_deref(), Some("pan.example.org"));
        assert_eq!(site_of("magnet:?xt=1"), None);
        assert_eq!(site_of("D:\\x"), None);
    }

    #[test]
    fn domain_path_secure_and_expiry() {
        let ls = vec![login(
            "example.com",
            1,
            vec![
                ck("all", ".example.com", "/", false, 0),
                ck("host", "", "/", false, 0),
                ck("api", "api.example.com", "/", false, 0),
                ck("sub", ".example.com", "/user", false, 0),
                ck("sec", ".example.com", "/", true, 0),
                ck("old", ".example.com", "/", false, 100),
            ],
        )];
        let names = |u: &str| cookies_for(&ls, u, 1000).into_iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(names("https://example.com/search?q=1"), ["all", "host", "sec"]);
        assert_eq!(names("http://www.example.com/user/1"), ["all", "host", "sub"]);
        assert_eq!(names("https://api.example.com/x"), ["all", "api", "sec"]);
        assert_eq!(names("https://cdn.example.com/userx"), ["all", "sec"]);
        assert!(names("https://example.org/").is_empty());
        assert!(names("https://notexample.com/").is_empty());
    }

    #[test]
    fn newer_login_wins_same_name() {
        let mut new = ck("sid", ".example.com", "/", false, 0);
        new.value = "new".into();
        let mut old = new.clone();
        old.value = "old".into();
        let ls = vec![login("b.example.com", 5, vec![new]), login("example.com", 1, vec![old])];
        assert_eq!(header_of(&cookies_for(&ls, "https://example.com/", 10)), "sid=new");
    }

    #[test]
    fn store_round_trip_in_chunks() {
        // 足够多、足够长的 Cookie，压缩后也要分成多段
        let many: Vec<StoredCookie> = (0..400)
            .map(|i| ck(&format!("c{i}-{}", "x".repeat(i % 37)), ".example.com", "/", i % 2 == 0, i as i64 * 7919))
            .collect();
        let ls = vec![login("example.com", 3, many), login("other.org", 4, vec![ck("a", "", "/", false, 0)])];
        let parts = encode(&ls).unwrap();
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.len() <= CHUNK));
        assert_eq!(decode(&parts).unwrap(), ls);
    }

    #[test]
    fn netscape_lines() {
        let s = netscape(&[ck("a", ".example.com", "/", true, 1900000000), ck("b", "", "", false, 0)], "example.com");
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], "# Netscape HTTP Cookie File");
        assert_eq!(lines[1], ".example.com\tTRUE\t/\tTRUE\t1900000000\ta\tv-a");
        assert_eq!(lines[2], "example.com\tFALSE\t/\tFALSE\t2147483647\tb\tv-b");
    }
}
