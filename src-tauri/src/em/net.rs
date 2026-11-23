//! 下载源规则的联网沙箱：只能访问 hosts 里列出的网站（每一跳跳转都检查）、只用 GET / POST、
//! 请求头白名单、单个响应 8 MB、30 秒超时、同一网站至少间隔 1 秒、一次操作最多 2000 个请求。
//! 本机 / 内网地址只有在 hosts 里原样写出时才能访问。Cookie 只来自用户自己在“网站登录”里保存的登录。
use super::format::Login;
use crate::fetch::source;
use futures_util::future::BoxFuture;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use url::{Host, Url};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// 一次搜索 / 解析最多发这么多请求
pub const MAX_REQUESTS: usize = 2000;
/// 规则里能写的请求头（小写）
pub const HEADERS: &[&str] = &["referer", "accept", "accept-language"];
const MAX_REDIRECTS: usize = 10;
const USER_AGENT: &str = "MediaPlayerFetch/1.0 (em rule)";

// ---------------------------------------------------------------- 网站白名单

#[derive(Clone, Debug, PartialEq)]
struct HostPat {
    wild: bool,
    name: String,
    port: Option<u16>,
}

/// hosts 白名单
#[derive(Clone, Debug, Default)]
pub struct Allow {
    pats: Vec<HostPat>,
}

/// 是不是本机、内网、链路本地等不应该被随便访问的地址
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || (o[0] == 100 && (64..128).contains(&o[1])) // 100.64.0.0/10
                || o[0] == 0
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_private(IpAddr::V4(v4));
            }
            let s = v.segments();
            v.is_loopback() || v.is_unspecified() || v.is_multicast() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

impl Allow {
    pub fn new(hosts: &[String]) -> Allow {
        let pats = hosts
            .iter()
            .map(|h| {
                let h = h.trim().to_ascii_lowercase();
                let (wild, rest) = match h.strip_prefix("*.") {
                    Some(r) => (true, r.to_string()),
                    None => (false, h.clone()),
                };
                let (name, port) = match rest.rsplit_once(':') {
                    Some((n, p)) if p.parse::<u16>().is_ok() => (n.to_string(), p.parse().ok()),
                    _ => (rest, None),
                };
                HostPat { wild, name, port }
            })
            .collect();
        Allow { pats }
    }

    /// 这个主机名在 hosts 里原样写出了（不是通配）
    pub fn exact(&self, host: &str) -> bool {
        let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
        self.pats.iter().any(|p| !p.wild && p.name == h)
    }

    /// 能不能访问这个网址
    pub fn check(&self, u: &Url) -> Result<(), String> {
        if !matches!(u.scheme(), "http" | "https") {
            return Err(format!("只能访问 http / https 网址，不能访问“{}”", short(u.as_str())));
        }
        if !u.username().is_empty() || u.password().is_some() {
            return Err("网址里不能带用户名和密码".into());
        }
        let (name, literal) = match u.host() {
            Some(Host::Domain(d)) => (d.to_ascii_lowercase(), false),
            Some(Host::Ipv4(ip)) => (ip.to_string(), true),
            Some(Host::Ipv6(ip)) => (ip.to_string(), true),
            None => return Err("网址里没有网站名".into()),
        };
        let name = name.trim_end_matches('.').to_string();
        let port = u.port();
        let special = literal || name == "localhost" || name.ends_with(".localhost");
        let ok = self.pats.iter().any(|p| {
            let port_ok = p.port == port;
            let name_ok = if p.wild {
                !special && name.len() > p.name.len() && name.ends_with(&format!(".{}", p.name))
            } else {
                p.name == name
            };
            port_ok && name_ok
        });
        if ok {
            return Ok(());
        }
        let shown = match port {
            Some(p) => format!("{name}:{p}"),
            None => name,
        };
        Err(if special {
            format!("不能访问本机或内网地址 {shown}（要访问的话，在 hosts 里原样写出它）")
        } else {
            format!("规则没有列出网站 {shown}，不能访问（只能访问 hosts 里的网站）")
        })
    }
}

fn short(s: &str) -> String {
    if s.chars().count() > 80 {
        s.chars().take(80).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------- 请求与响应

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    None,
    Form(Vec<(String, String)>),
    Json(Value),
}

#[derive(Clone, Debug)]
pub struct Req {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Req {
    pub fn get(url: Url) -> Req {
        Req {
            method: Method::Get,
            url,
            headers: vec![],
            body: Body::None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Resp {
    pub status: u16,
    /// 跳转后的最终网址
    pub url: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// 真正发请求的那一层（测试时换成假的）
pub trait Fetch: Send + Sync {
    fn fetch<'a>(&'a self, req: &'a Req) -> BoxFuture<'a, Result<Resp, String>>;
}

/// 拒绝解析到内网的域名（hosts 里原样写出的除外）
struct GuardResolver {
    allow: Allow,
}

impl reqwest::dns::Resolve for GuardResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let lan_ok = self.allow.exact(&host);
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let ok: Vec<SocketAddr> = addrs.into_iter().filter(|a| lan_ok || !is_private(a.ip())).collect();
            if ok.is_empty() {
                return Err(format!("{host} 指向本机或内网地址，规则没有原样列出它，已拒绝").into());
            }
            Ok(Box::new(ok.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// 用 reqwest 联网：每一跳跳转都按白名单检查，先限频再请求，边读边数大小
pub struct Http {
    client: reqwest::Client,
}

impl Http {
    /// `proxy`：是否使用系统代理（测试里关掉）
    pub fn new(allow: &Allow, proxy: bool) -> Http {
        let a = allow.clone();
        let policy = reqwest::redirect::Policy::custom(move |att| {
            if att.previous().len() >= MAX_REDIRECTS {
                return att.error("跳转次数太多");
            }
            match a.check(att.url()) {
                Ok(()) => att.follow(),
                Err(m) => att.error(format!("网站把请求转到了别处：{m}")),
            }
        });
        let mut b = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .connect_timeout(Duration::from_secs(15))
            .redirect(policy)
            .dns_resolver(Arc::new(GuardResolver { allow: allow.clone() }));
        if !proxy {
            b = b.no_proxy();
        }
        Http {
            client: b.build().unwrap_or_default(),
        }
    }
}

/// reqwest 的错误连同原因一起显示（跳转被拒时原因在里层）
fn err_text(e: reqwest::Error) -> String {
    use std::error::Error;
    let e = e.without_url();
    let mut msg = if e.is_timeout() {
        "请求超时（30 秒）".to_string()
    } else {
        e.to_string()
    };
    let mut src = e.source();
    while let Some(s) = src {
        let t = s.to_string();
        if !msg.contains(&t) {
            msg = format!("{msg}：{t}");
        }
        src = s.source();
    }
    msg
}

impl Fetch for Http {
    fn fetch<'a>(&'a self, req: &'a Req) -> BoxFuture<'a, Result<Resp, String>> {
        Box::pin(async move {
            source::throttle(req.url.as_str()).await;
            let mut rb = match req.method {
                Method::Get => self.client.get(req.url.clone()),
                Method::Post => self.client.post(req.url.clone()),
            };
            for (k, v) in &req.headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            rb = match &req.body {
                Body::None => rb,
                Body::Form(f) => rb.form(f),
                Body::Json(v) => rb.json(v),
            };
            let mut resp = rb.send().await.map_err(|e| format!("连接失败：{}", err_text(e)))?;
            if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
                return Err(format!("返回的内容太大（超过 {} MB）", MAX_BYTES >> 20));
            }
            let status = resp.status().as_u16();
            let url = resp.url().to_string();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let mut bytes = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取失败：{}", err_text(e)))? {
                bytes.extend_from_slice(&chunk);
                if bytes.len() > MAX_BYTES {
                    return Err(format!("返回的内容太大（超过 {} MB）", MAX_BYTES >> 20));
                }
            }
            Ok(Resp {
                status,
                url,
                content_type,
                bytes,
            })
        })
    }
}

// ---------------------------------------------------------------- 文字编码

/// 规则里的 encoding：auto / 空 → None；其余要是认得的编码名
pub fn encoding_of(label: &str) -> Result<Option<&'static encoding_rs::Encoding>, String> {
    let l = label.trim();
    if l.is_empty() || l.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    encoding_rs::Encoding::for_label(l.as_bytes())
        .map(Some)
        .ok_or_else(|| format!("不认识的编码“{l}”（可用 auto、utf-8、gbk、big5、shift_jis 等）"))
}

fn charset_in(s: &str) -> Option<&'static encoding_rs::Encoding> {
    let l = s.to_ascii_lowercase();
    let i = l.find("charset=")?;
    let v: String = l[i + 8..]
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    encoding_rs::Encoding::for_label(v.as_bytes())
}

/// 按规则指定的编码解码；没指定时看 Content-Type、网页里的 meta charset，再自动识别
pub fn decode(bytes: &[u8], content_type: &str, forced: Option<&'static encoding_rs::Encoding>) -> String {
    if let Some(enc) = forced {
        return enc.decode(bytes).0.into_owned();
    }
    let has_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]) || bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]);
    if !has_bom {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
        if let Some(enc) = charset_in(content_type).or_else(|| charset_in(&head)) {
            return enc.decode(bytes).0.into_owned();
        }
    }
    crate::reader::decode_text(bytes).0
}

// ---------------------------------------------------------------- 沙箱

/// 取回的内容（已解码）
#[derive(Clone, Debug)]
pub struct Fetched {
    pub url: String,
    pub status: u16,
    pub text: String,
}

/// 一次操作（搜索、解析）用的联网入口：白名单、请求计数、Cookie、状态检查、解码，并记下每个请求给测试面板看
pub struct Net {
    allow: Allow,
    login: Login,
    fetch: Arc<dyn Fetch>,
    cookies: fn(&str) -> Option<String>,
    count: AtomicUsize,
    log: Mutex<Vec<Value>>,
}

/// 在软件里登录过的网站的 Cookie（用户自己保存的）
pub fn saved_cookies(url: &str) -> Option<String> {
    crate::fetch::login::cookie_header(url)
}

pub fn need_login_msg(host: &str) -> String {
    format!("这个下载源需要登录：请先在“网站登录”里登录 {host}，再回来搜索。软件不会替你登录。")
}

impl Net {
    pub fn new(allow: Allow, login: Login, fetch: Arc<dyn Fetch>, cookies: fn(&str) -> Option<String>) -> Net {
        Net {
            allow,
            login,
            fetch,
            cookies,
            count: AtomicUsize::new(0),
            log: Mutex::new(vec![]),
        }
    }

    pub fn allow(&self) -> &Allow {
        &self.allow
    }

    /// 已经发出的请求数
    pub fn count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// 取出并清空请求记录
    pub fn take_log(&self) -> Vec<Value> {
        std::mem::take(&mut *self.log.lock())
    }

    fn note(&self, req: &Req, status: Option<u16>, err: Option<&str>) {
        let mut l = self.log.lock();
        if l.len() < 200 {
            l.push(json!({
                "method": if req.method == Method::Post { "POST" } else { "GET" },
                "url": req.url.as_str(),
                "status": status,
                "error": err,
            }));
        }
    }

    pub async fn send(&self, mut req: Req, enc: Option<&'static encoding_rs::Encoding>) -> Result<Fetched, String> {
        let r = self.send_inner(&mut req, enc).await;
        match &r {
            Ok(f) => self.note(&req, Some(f.status), None),
            Err((st, e)) => self.note(&req, *st, Some(e)),
        }
        r.map_err(|(_, e)| e)
    }

    async fn send_inner(&self, req: &mut Req, enc: Option<&'static encoding_rs::Encoding>) -> Result<Fetched, (Option<u16>, String)> {
        self.allow.check(&req.url).map_err(|e| (None, e))?;
        if self.count.fetch_add(1, Ordering::Relaxed) >= MAX_REQUESTS {
            return Err((None, format!("请求太多（一次最多 {MAX_REQUESTS} 个），已停止")));
        }
        req.headers.retain(|(k, _)| HEADERS.contains(&k.to_ascii_lowercase().as_str()));
        if self.login != Login::None {
            match (self.cookies)(req.url.as_str()) {
                Some(c) => req.headers.push(("Cookie".into(), c)),
                None if self.login == Login::Required => {
                    return Err((None, need_login_msg(req.url.host_str().unwrap_or(""))));
                }
                None => {}
            }
        }
        let resp = self.fetch.fetch(req).await.map_err(|e| (None, e))?;
        let st = Some(resp.status);
        let final_url = Url::parse(&resp.url).map_err(|_| (st, "网站返回的地址不正确".to_string()))?;
        self.allow.check(&final_url).map_err(|m| (st, format!("网站把请求转到了别处：{m}")))?;
        let code = reqwest::StatusCode::from_u16(resp.status).unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
        if code == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err((st, "网站说请求太频繁（HTTP 429），过一会儿再试".into()));
        }
        source::check_status(code).map_err(|e| (st, e))?;
        if resp.bytes.len() > MAX_BYTES {
            return Err((st, format!("返回的内容太大（超过 {} MB）", MAX_BYTES >> 20)));
        }
        Ok(Fetched {
            url: resp.url,
            status: resp.status,
            text: decode(&resp.bytes, &resp.content_type, enc),
        })
    }
}

/// 页面是登录页或验证码页（结果为空时用来判断原因）
pub fn looks_blocked(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    let captcha = ["g-recaptcha", "h-captcha", "cf-challenge", "cf_chl_", "captcha", "验证码", "人机验证"]
        .iter()
        .any(|k| l.contains(k));
    captcha || crate::fetch::web::looks_like_login(text)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 测试用：按网址返回预先准备好的内容；`redirect` 模拟跳转后的最终地址
    #[derive(Default)]
    pub struct Canned {
        pub pages: HashMap<String, (u16, Vec<u8>)>,
        pub redirect: HashMap<String, String>,
        pub seen: Mutex<Vec<Req>>,
    }

    impl Canned {
        pub fn with(mut self, url: &str, body: &str) -> Self {
            self.pages.insert(url.to_string(), (200, body.as_bytes().to_vec()));
            self
        }
        pub fn bytes(mut self, url: &str, body: Vec<u8>) -> Self {
            self.pages.insert(url.to_string(), (200, body));
            self
        }
        pub fn status(mut self, url: &str, code: u16, body: &str) -> Self {
            self.pages.insert(url.to_string(), (code, body.as_bytes().to_vec()));
            self
        }
    }

    impl Fetch for Canned {
        fn fetch<'a>(&'a self, req: &'a Req) -> BoxFuture<'a, Result<Resp, String>> {
            Box::pin(async move {
                self.seen.lock().push(req.clone());
                let u = req.url.to_string();
                let url = self.redirect.get(&u).cloned().unwrap_or_else(|| u.clone());
                // 先找完全相同的网址，再找最长的前缀
                let hit = self.pages.get(&u).or_else(|| {
                    self.pages
                        .iter()
                        .filter(|(k, _)| u.starts_with(k.as_str()))
                        .max_by_key(|(k, _)| k.len())
                        .map(|(_, v)| v)
                });
                match hit {
                    Some((st, body)) => Ok(Resp {
                        status: *st,
                        url,
                        content_type: String::new(),
                        bytes: body.clone(),
                    }),
                    None => Ok(Resp {
                        status: 404,
                        url,
                        ..Default::default()
                    }),
                }
            })
        }
    }

    fn allow(h: &[&str]) -> Allow {
        Allow::new(&h.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn host_allowlist() {
        let a = allow(&["archive.org", "*.example.org", "nas.local:8080", "192.168.1.5"]);
        assert!(a.check(&u("https://archive.org/x")).is_ok());
        assert!(a.check(&u("https://ARCHIVE.org./x")).is_ok());
        assert!(a.check(&u("https://archive.org:443/x")).is_ok(), "默认端口");
        assert!(a.check(&u("https://archive.org:8443/x")).is_err(), "别的端口");
        assert!(a.check(&u("https://evil-archive.org/")).is_err());
        assert!(a.check(&u("https://archive.org.evil.com/")).is_err());
        assert!(a.check(&u("https://img.archive.org/")).is_err(), "没写通配");
        // 通配只匹配子域名
        assert!(a.check(&u("https://a.b.example.org/")).is_ok());
        assert!(a.check(&u("https://example.org/")).is_err());
        assert!(a.check(&u("https://badexample.org/")).is_err());
        // 端口
        assert!(a.check(&u("http://nas.local:8080/")).is_ok());
        assert!(a.check(&u("http://nas.local/")).is_err());
        // 协议、用户名密码
        assert!(a.check(&u("ftp://archive.org/")).is_err());
        assert!(a.check(&u("file:///etc/passwd")).is_err());
        assert!(a.check(&u("https://user:pw@archive.org/")).is_err());
    }

    #[test]
    fn private_addresses() {
        let a = allow(&["archive.org", "*.example.org", "192.168.1.5"]);
        for bad in [
            "http://127.0.0.1/",
            "http://localhost/",
            "http://10.0.0.1/",
            "http://192.168.1.6/",
            "http://[::1]/",
            "http://169.254.169.254/latest/meta-data",
            "http://0x7f.0.0.1/",
        ] {
            let e = a.check(&u(bad)).unwrap_err();
            assert!(e.contains("本机或内网"), "{bad}: {e}");
        }
        assert!(a.check(&u("http://192.168.1.5/")).is_ok(), "原样列出的内网地址可以");
        assert!(a.exact("192.168.1.5") && a.exact("archive.org") && !a.exact("x.example.org"));
        assert!(is_private("10.1.2.3".parse().unwrap()));
        assert!(is_private("100.100.0.1".parse().unwrap()));
        assert!(is_private("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_private("fd00::1".parse().unwrap()));
        assert!(!is_private("93.184.216.34".parse().unwrap()));
    }

    fn net(c: Canned, login: Login, cookies: fn(&str) -> Option<String>) -> Net {
        Net::new(allow(&["a.org"]), login, Arc::new(c), cookies)
    }

    fn no_cookie(_: &str) -> Option<String> {
        None
    }
    fn some_cookie(_: &str) -> Option<String> {
        Some("sid=1".into())
    }

    #[test]
    fn sandbox_checks() {
        tauri::async_runtime::block_on(async {
            let mut c = Canned::default().with("https://a.org/r", "ok");
            c.redirect.insert("https://a.org/r".into(), "https://evil.org/x".into());
            let n = net(c, Login::None, no_cookie);
            let e = n.send(Req::get(u("https://a.org/r")), None).await.unwrap_err();
            assert!(e.contains("转到了别处") && e.contains("evil.org"), "{e}");
            let e = n.send(Req::get(u("https://b.org/")), None).await.unwrap_err();
            assert!(e.contains("没有列出"), "{e}");
            assert_eq!(n.take_log().len(), 2);

            // 401 → 需要登录；403 → 拒绝访问
            let n = net(
                Canned::default().status("https://a.org/1", 401, "").status("https://a.org/2", 403, ""),
                Login::None,
                no_cookie,
            );
            assert!(n.send(Req::get(u("https://a.org/1")), None).await.unwrap_err().contains("需要登录"));
            assert!(n.send(Req::get(u("https://a.org/2")), None).await.unwrap_err().contains("403"));

            // 请求头白名单，Cookie 只来自保存的登录
            let c = Arc::new(Canned::default().with("https://a.org/", "x"));
            let n = Net::new(allow(&["a.org"]), Login::Optional, c.clone(), some_cookie);
            let mut r = Req::get(u("https://a.org/"));
            r.headers = vec![
                ("Referer".into(), "https://a.org/".into()),
                ("Cookie".into(), "evil=1".into()),
                ("Authorization".into(), "x".into()),
            ];
            n.send(r, None).await.unwrap();
            let sent = c.seen.lock()[0].headers.clone();
            assert_eq!(
                sent,
                vec![("Referer".to_string(), "https://a.org/".to_string()), ("Cookie".into(), "sid=1".into())]
            );
            let n = net(Canned::default().with("https://a.org/", "x"), Login::None, some_cookie);
            n.send(Req::get(u("https://a.org/")), None).await.unwrap();

            // 必须登录但没登录过
            let n = net(Canned::default().with("https://a.org/", "x"), Login::Required, no_cookie);
            let e = n.send(Req::get(u("https://a.org/")), None).await.unwrap_err();
            assert!(e.contains("网站登录") && e.contains("a.org"), "{e}");
        });
    }

    #[test]
    fn request_budget() {
        tauri::async_runtime::block_on(async {
            let n = net(Canned::default().with("https://a.org/", "x"), Login::None, no_cookie);
            n.count.store(MAX_REQUESTS, Ordering::Relaxed);
            assert!(n.send(Req::get(u("https://a.org/")), None).await.unwrap_err().contains("请求太多"));
        });
    }

    #[test]
    fn charsets() {
        let (gbk, _, _) = encoding_rs::GBK.encode("第一章 你好");
        assert_eq!(decode(&gbk, "text/html; charset=GBK", None), "第一章 你好");
        let mut page = b"<meta charset=\"gbk\"><p>".to_vec();
        page.extend_from_slice(&gbk);
        assert!(decode(&page, "text/html", None).contains("第一章 你好"));
        assert_eq!(decode(&gbk, "", encoding_of("gbk").unwrap()), "第一章 你好");
        assert_eq!(decode("中文".as_bytes(), "", None), "中文");
        assert!(encoding_of("auto").unwrap().is_none());
        assert!(encoding_of("klingon").is_err());
    }

    /// 真实的 reqwest 客户端：本机服务把请求跳转到别的网站，跳转被拒绝，不会去连那个网站
    #[test]
    fn real_redirect_refused() {
        tauri::async_runtime::block_on(async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = l.local_addr().unwrap().port();
            tokio::spawn(async move {
                for _ in 0..2 {
                    let (mut s, _) = l.accept().await.unwrap();
                    let mut buf = [0u8; 2048];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let resp = if req.starts_with("GET /ok") {
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string()
                    } else {
                        "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                    };
                    let _ = s.write_all(resp.as_bytes()).await;
                }
            });
            let a = allow(&[&format!("127.0.0.1:{port}")]);
            let http = Http::new(&a, false);
            let n = Net::new(a.clone(), Login::None, Arc::new(http), no_cookie);
            let ok = n.send(Req::get(u(&format!("http://127.0.0.1:{port}/ok"))), None).await.unwrap();
            assert_eq!(ok.text, "ok");
            let e = n.send(Req::get(u(&format!("http://127.0.0.1:{port}/go"))), None).await.unwrap_err();
            assert!(e.contains("本机或内网") || e.contains("转到了别处"), "{e}");
        });
    }

    #[test]
    fn blocked_pages() {
        assert!(looks_blocked("<div class=\"g-recaptcha\"></div>"));
        assert!(looks_blocked("<form><input type=password></form>"));
        assert!(!looks_blocked("<ul><li>结果</li></ul>"));
    }
}
