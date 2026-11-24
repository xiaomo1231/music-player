//! js 入口：插件是一个 ES 模块，在 QuickJS 沙箱里运行。
//!
//! 沙箱里没有文件、进程、定时器和模块加载，只有宿主给的 `mp`：
//! `mp.http`（只能访问声明的网站，经过和 .em 规则相同的联网检查）、`mp.html`（CSS 选择器）、
//! `mp.url`、`mp.log`、`mp.settings`、`mp.context`，以及全局的 `PluginError`。
//! 每次调用都新建运行环境，限制内存、栈和运行时间；插件只能搜索、列出分集和给出下载地址，不能自己下载。
use super::manifest::{provider_id, safe_rel, SourceDecl};
use super::source::{check_target, parse_search, MAX_WORKS};
use crate::em::format::Login;
use crate::em::net::{self, Allow, Body, Method, Net, Req};
use crate::fetch::external;
use crate::fetch::provider::{self, Item, Provider, ProviderInfo, Resolved};
use futures_util::future::BoxFuture;
use rquickjs::{CatchResultExt, CaughtError, Context, Function, Module, Promise, Runtime};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 内存上限
pub const MEMORY: usize = 64 << 20;
/// 入口文件最大
pub const MAX_SOURCE: u64 = 4 << 20;
/// 返回值最大（JSON 文本）
pub const MAX_RESULT: usize = 16 << 20;

pub struct JsSource {
    pub plugin: String,
    pub decl: SourceDecl,
    pub info: ProviderInfo,
}

/// 宿主函数在 JS 里的入口
const PRELUDE: &str = r#"
(() => {
  const host = globalThis.__mp_host;
  delete globalThis.__mp_host;
  const settings = JSON.parse(globalThis.__mp_settings);
  const context = JSON.parse(globalThis.__mp_context);
  delete globalThis.__mp_settings;
  delete globalThis.__mp_context;

  class PluginError extends Error {
    constructor(code, message) {
      super(message === undefined ? String(code) : String(message));
      this.name = "PluginError";
      this.code = message === undefined ? "internal" : String(code);
    }
  }
  globalThis.PluginError = PluginError;

  const call = (op, arg) => {
    const r = JSON.parse(host(op, JSON.stringify(arg === undefined ? null : arg)));
    if (r && Object.prototype.hasOwnProperty.call(r, "err")) {
      const m = /^\[([a-z_]+)\] ([\s\S]*)$/.exec(r.err);
      throw m ? new PluginError(m[1], m[2]) : new PluginError("network", r.err);
    }
    return r.ok;
  };

  const request = (o) => call("fetch", o);
  const http = Object.freeze({
    request,
    get: (url, o = {}) => request({ ...o, method: "GET", url }),
    post: (url, o = {}) => request({ ...o, method: "POST", url }),
    json: (url, o = {}) => {
      const r = request({ ...o, method: o.method || (o.form || o.json !== undefined ? "POST" : "GET"), url });
      try {
        return JSON.parse(r.text);
      } catch (e) {
        throw new PluginError("internal", "返回的不是 JSON：" + r.url);
      }
    },
  });
  const html = Object.freeze({
    select: (text, css) => call("select", { html: String(text), css: String(css) }),
    selectOne: (text, css) => call("select", { html: String(text), css: String(css), one: true })[0] || null,
  });
  const url = Object.freeze({
    resolve: (base, rel) => call("url", { base: String(base), rel: String(rel) }),
    encode: (s) => encodeURIComponent(String(s)),
    query: (o) =>
      Object.entries(o || {})
        .filter(([, v]) => v !== undefined && v !== null)
        .map(([k, v]) => encodeURIComponent(k) + "=" + encodeURIComponent(String(v)))
        .join("&"),
  });
  const say = (level) => (...a) => {
    call("log", { level, msg: a.map((x) => (typeof x === "string" ? x : JSON.stringify(x))).join(" ") });
  };
  const log = say("info");
  log.info = log;
  log.warn = say("warn");
  log.error = say("error");
  log.debug = say("debug");
  globalThis.console = Object.freeze({ log, info: log, warn: log.warn, error: log.error, debug: log.debug });
  globalThis.mp = Object.freeze({ http, html, url, log, settings: Object.freeze(settings), context: Object.freeze(context) });

  globalThis.__mp_run = async (f, params) => {
    const v = await f(JSON.parse(params), globalThis.mp);
    return JSON.stringify(v === undefined ? null : v);
  };
})();
"#;

/// 一次调用要的东西（测试也直接用）
pub struct Job {
    pub plugin: String,
    pub source: String,
    pub method: String,
    pub params: Value,
    pub settings: Value,
    pub context: Value,
    pub timeout: Duration,
}

struct Host {
    plugin: String,
    method: String,
    net: Arc<Net>,
    handle: tokio::runtime::Handle,
    deadline: Instant,
}

/// 联网失败的说明转成错误码
fn classify(msg: &str) -> String {
    if provider::split_code(msg).0.is_some() {
        return msg.to_string();
    }
    let code = if msg.contains("需要登录") || msg.contains("HTTP 401") {
        "login_required"
    } else if msg.contains("太频繁") {
        "rate_limited"
    } else if msg.contains("没有列出网站")
        || msg.contains("本机或内网")
        || msg.contains("转到了别处")
        || msg.contains("HTTP 403")
        || msg.contains("只能访问 http")
    {
        "forbidden"
    } else if msg.contains("HTTP 404") {
        "not_found"
    } else {
        "network"
    };
    provider::coded(code, msg)
}

impl Host {
    fn call(&self, op: &str, arg: &str) -> String {
        let arg: Value = serde_json::from_str(arg).unwrap_or(Value::Null);
        let r = match op {
            "fetch" => self.fetch(&arg),
            "select" => select(&arg),
            "url" => join(&arg),
            "log" => {
                let level = arg["level"].as_str().unwrap_or("info");
                let msg: String = arg["msg"].as_str().unwrap_or("").chars().take(4000).collect();
                super::log(&self.plugin, &self.method, level, &msg);
                Ok(Value::Null)
            }
            _ => Err(provider::coded("internal", &format!("不认识的宿主调用 {op}"))),
        };
        match r {
            Ok(v) => json!({ "ok": v }).to_string(),
            Err(e) => json!({ "err": e }).to_string(),
        }
    }

    fn fetch(&self, a: &Value) -> Result<Value, String> {
        if Instant::now() > self.deadline {
            return Err(provider::coded("network", "插件运行超时"));
        }
        let u = a["url"].as_str().unwrap_or("");
        let url = url::Url::parse(u).map_err(|_| provider::coded("internal", &format!("网址不正确：{u}")))?;
        let method = match a["method"].as_str().unwrap_or("GET").to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            m => return Err(provider::coded("unsupported", &format!("只支持 GET 和 POST，不支持 {m}"))),
        };
        let mut headers = vec![];
        if let Some(h) = a["headers"].as_object() {
            for (k, v) in h {
                // Cookie 等请求头由软件管，插件不能设置
                if !net::HEADERS.contains(&k.to_ascii_lowercase().as_str()) {
                    return Err(provider::coded(
                        "forbidden",
                        &format!("插件不能设置请求头 {k}（只能设置 Referer、Accept、Accept-Language）"),
                    ));
                }
                headers.push((k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string())));
            }
        }
        let body = if let Some(f) = a["form"].as_object() {
            Body::Form(
                f.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
                    .collect(),
            )
        } else if let Some(j) = a.get("json").filter(|j| !j.is_null()) {
            Body::Json(j.clone())
        } else {
            Body::None
        };
        let enc = match a["encoding"].as_str() {
            Some(l) if !l.is_empty() => net::encoding_of(l).map_err(|e| provider::coded("internal", &e))?,
            _ => None,
        };
        let req = Req { method, url, headers, body };
        let net = self.net.clone();
        let f = self.handle.block_on(async move { net.send(req, enc).await }).map_err(|e| classify(&e))?;
        Ok(json!({ "url": f.url, "status": f.status, "text": f.text }))
    }
}

fn select(a: &Value) -> Result<Value, String> {
    let css = a["css"].as_str().unwrap_or("");
    let sel = scraper::Selector::parse(css).map_err(|e| provider::coded("internal", &format!("CSS 选择器不正确：{css}（{e}）")))?;
    let doc = scraper::Html::parse_document(a["html"].as_str().unwrap_or(""));
    let one = a["one"].as_bool().unwrap_or(false);
    let mut out = vec![];
    for el in doc.select(&sel) {
        let attrs: serde_json::Map<String, Value> = el.value().attrs().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect();
        out.push(json!({
            "text": el.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "),
            "html": el.inner_html(),
            "attrs": attrs,
        }));
        if one || out.len() >= 5000 {
            break;
        }
    }
    Ok(Value::Array(out))
}

fn join(a: &Value) -> Result<Value, String> {
    let base = a["base"].as_str().unwrap_or("");
    let rel = a["rel"].as_str().unwrap_or("");
    let b = url::Url::parse(base).map_err(|_| provider::coded("internal", &format!("网址不正确：{base}")))?;
    let u = b.join(rel).map_err(|_| provider::coded("internal", &format!("网址不正确：{rel}")))?;
    Ok(Value::String(u.to_string()))
}

/// JS 抛出的错误转成带错误码的说明
fn caught(e: CaughtError<'_>) -> String {
    match e {
        CaughtError::Exception(ex) => {
            let o = ex.as_object();
            let code: Option<String> = o.get::<_, Option<String>>("code").ok().flatten();
            let msg = ex.message().unwrap_or_default();
            match code {
                Some(c) if provider::ERROR_CODES.contains(&c.as_str()) => provider::coded(&c, &msg),
                Some(_) | None => {
                    let stack = ex.stack().unwrap_or_default();
                    let first = stack
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .map(|l| format!("（{}）", l.trim()))
                        .unwrap_or_default();
                    if msg.contains("interrupted") {
                        provider::coded("network", "插件运行超时")
                    } else if msg.contains("out of memory") {
                        provider::coded("internal", "插件用的内存太多")
                    } else {
                        provider::coded("internal", &format!("插件出错：{msg}{first}"))
                    }
                }
            }
        }
        CaughtError::Value(v) => {
            let s = v.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| format!("{v:?}"));
            provider::coded("internal", &format!("插件出错：{s}"))
        }
        CaughtError::Error(rquickjs::Error::WouldBlock) => {
            provider::coded("internal", "插件的 Promise 一直没有完成（沙箱里没有定时器，不能用 setTimeout 等待）")
        }
        CaughtError::Error(e) => {
            let m = e.to_string();
            if m.contains("interrupted") {
                provider::coded("network", "插件运行超时")
            } else {
                provider::coded("internal", &format!("插件出错：{m}"))
            }
        }
    }
}

/// 在当前线程里跑一次（会阻塞，联网时用 `handle` 等待）
pub fn run_blocking(job: &Job, src: &str, net: Arc<Net>, handle: tokio::runtime::Handle) -> Result<Value, String> {
    let deadline = Instant::now() + job.timeout;
    let rt = Runtime::new().map_err(|e| provider::coded("internal", &e.to_string()))?;
    rt.set_memory_limit(MEMORY);
    rt.set_max_stack_size(1 << 20);
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let ctx = Context::full(&rt).map_err(|e| provider::coded("internal", &e.to_string()))?;
    let host = Arc::new(Host {
        plugin: job.plugin.clone(),
        method: job.method.clone(),
        net,
        handle,
        deadline,
    });
    let mut params = job.params.clone();
    params["source"] = json!(job.source);
    let params = params.to_string();
    let text: String = ctx.with(|ctx| {
        let r = (|| {
            let h = host.clone();
            let g = ctx.globals();
            g.set("__mp_host", Function::new(ctx.clone(), move |op: String, arg: String| h.call(&op, &arg))?)?;
            g.set("__mp_settings", job.settings.to_string())?;
            g.set("__mp_context", job.context.to_string())?;
            ctx.eval::<(), _>(PRELUDE)?;
            let (m, p) = Module::declare(ctx.clone(), "main.js", src)?.eval()?;
            p.finish::<()>()?;
            let f: rquickjs::Value = m.namespace()?.get(job.method.as_str())?;
            if !f.is_function() {
                return Ok(None);
            }
            let run: Function = g.get("__mp_run")?;
            let pr: Promise = run.call((f, params.as_str()))?;
            pr.finish::<String>().map(Some)
        })();
        match r.catch(&ctx) {
            Ok(Some(s)) => Ok(s),
            Ok(None) => Err(provider::coded("unsupported", &format!("插件没有导出 {} 函数", job.method))),
            Err(e) => Err(caught(e)),
        }
    })?;
    if text.len() > MAX_RESULT {
        return Err(provider::coded("internal", "插件返回的内容太大"));
    }
    serde_json::from_str(&text).map_err(|e| provider::coded("internal", &format!("插件返回的内容不对：{e}")))
}

fn no_cookie(_: &str) -> Option<String> {
    None
}

impl JsSource {
    pub fn new(plugin: &str, plugin_name: &str, decl: &SourceDecl) -> JsSource {
        let name = if decl.name.trim().is_empty() {
            plugin_name.to_string()
        } else {
            decl.name.clone()
        };
        JsSource {
            plugin: plugin.to_string(),
            info: ProviderInfo {
                id: provider_id(plugin, &decl.id),
                name,
                media_kinds: decl.media_kinds.clone(),
                note: decl.note.clone(),
            },
            decl: decl.clone(),
        }
    }

    async fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let started = Instant::now();
        let r = self.call_inner(method, params, timeout).await;
        let ms = started.elapsed().as_millis() as u64;
        match &r {
            Ok(_) => super::log(&self.plugin, method, "call", &format!("完成，用时 {ms} 毫秒")),
            Err(e) => super::log(&self.plugin, method, "error", e),
        }
        super::set_status(&self.info.id, method, r.as_ref().err().map(|s| s.as_str()));
        r
    }

    async fn call_inner(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let rt = super::runtime(&self.plugin)?;
        let file = if rt.manifest.runtime.file.trim().is_empty() {
            "main.js".to_string()
        } else {
            rt.manifest.runtime.file.clone()
        };
        if !safe_rel(&file) {
            return Err(provider::coded("bad_config", &format!("入口文件路径不正确：{file}")));
        }
        let path = rt.dir.join(&file);
        let meta = std::fs::metadata(&path).map_err(|_| provider::coded("bad_config", &format!("找不到入口文件 {file}")))?;
        if meta.len() > MAX_SOURCE {
            return Err(provider::coded("bad_config", "入口文件太大（上限 4 MB）"));
        }
        let src = std::fs::read_to_string(&path).map_err(|_| provider::coded("bad_config", &format!("入口文件 {file} 要用 UTF-8 编码")))?;
        let allow = Allow::new(&rt.granted.hosts);
        let login = if rt.granted.login { Login::Optional } else { Login::None };
        let cookies: fn(&str) -> Option<String> = if rt.granted.login { net::saved_cookies } else { no_cookie };
        let http = net::Http::new(&allow, true);
        let net = Arc::new(Net::new(allow, login, Arc::new(http), cookies));
        let job = Job {
            plugin: self.plugin.clone(),
            source: self.decl.id.clone(),
            method: method.to_string(),
            params,
            settings: rt.settings,
            context: json!({ "app_version": rt.context["app_version"], "lang": rt.context["lang"] }),
            timeout,
        };
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || run_blocking(&job, &src, net, handle))
            .await
            .map_err(|e| provider::coded("internal", &format!("插件运行失败：{e}")))?
    }

    pub async fn episodes(&self, work: &Item) -> Result<Vec<Item>, String> {
        let v = self.call("episodes", json!({ "item": work }), Duration::from_secs(60)).await?;
        let mut eps = external::parse_items(v, &work.media_kind).map_err(|m| provider::coded("internal", &m))?;
        for e in eps.iter_mut() {
            if e.series.trim().is_empty() {
                e.series = if work.series.is_empty() { work.title.clone() } else { work.series.clone() };
            }
            if e.cover.is_empty() {
                e.cover = work.cover.clone();
            }
            if e.artist.is_empty() {
                e.artist = work.artist.clone();
            }
        }
        Ok(eps)
    }
}

impl Provider for JsSource {
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }

    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async move {
            let v = self
                .call(
                    "search",
                    json!({ "query": query, "media_kind": media_kind, "page": page }),
                    Duration::from_secs(60),
                )
                .await?;
            let items = parse_search(v, media_kind)?;
            if !self.decl.can("episodes") {
                return Ok(items);
            }
            let mut out = vec![];
            let mut works = 0;
            for it in items {
                if it.episode.is_none() && works < MAX_WORKS {
                    works += 1;
                    match self.episodes(&it).await {
                        Ok(eps) if !eps.is_empty() => out.extend(eps),
                        Ok(_) => out.push(it),
                        Err(e) => {
                            super::log(&self.plugin, "episodes", "warn", &e);
                            out.push(it);
                        }
                    }
                } else {
                    out.push(it);
                }
            }
            Ok(out)
        })
    }

    fn can_episodes(&self) -> bool {
        self.decl.can("episodes")
    }

    fn search_works<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async move {
            let v = self
                .call(
                    "search",
                    json!({ "query": query, "media_kind": media_kind, "page": page }),
                    Duration::from_secs(60),
                )
                .await?;
            parse_search(v, media_kind)
        })
    }

    fn episodes<'a>(&'a self, work: &'a Item) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async move { JsSource::episodes(self, work).await })
    }

    fn resolve<'a>(&'a self, item: &'a Item, quality: &'a str) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(async move {
            let v = self
                .call("resolve", json!({ "item": item, "quality": quality }), Duration::from_secs(120))
                .await?;
            let r = external::parse_resolved(v).map_err(|m| provider::coded("internal", &m))?;
            // 沙箱里的插件不能指向本机文件
            if matches!(r.target, provider::Target::File { .. }) {
                return Err(provider::coded("forbidden", "脚本插件不能返回本机文件"));
            }
            check_target(&self.plugin, &r.target)?;
            Ok(r)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::em::net::tests::Canned;

    fn job(method: &str, params: Value, ms: u64) -> Job {
        Job {
            plugin: "demo".into(),
            source: "main".into(),
            method: method.into(),
            params,
            settings: json!({ "lang": "zh" }),
            context: json!({}),
            timeout: Duration::from_millis(ms),
        }
    }

    fn run(src: &str, method: &str, params: Value, c: Canned) -> Result<Value, String> {
        run_ms(src, method, params, c, 5000)
    }

    fn run_ms(src: &str, method: &str, params: Value, c: Canned, ms: u64) -> Result<Value, String> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let net = Arc::new(Net::new(Allow::new(&["a.org".to_string()]), Login::None, Arc::new(c), no_cookie));
        let j = job(method, params, ms);
        let src = src.to_string();
        let h = rt.handle().clone();
        rt.block_on(async move { tokio::task::spawn_blocking(move || run_blocking(&j, &src, net, h)).await.unwrap() })
    }

    const SRC: &str = r#"
export async function search({ query, page }, mp) {
  const r = mp.http.get("https://a.org/s?" + mp.url.query({ q: query, p: page }));
  const rows = mp.html.select(r.text, "li.item a");
  mp.log("found", rows.length);
  return rows.map((a, i) => ({ id: String(i), title: a.text, extra: { href: mp.url.resolve(r.url, a.attrs.href) }, lang: mp.settings.lang }));
}
export function resolve({ item }) {
  return { target: { type: "http", url: item.extra.href, ext: "mp3" } };
}
export function boom() { throw new PluginError("login_required", "先登录"); }
export function plain() { null.x; }
export function evil() { return mp.http.get("https://evil.org/"); }
export function cookie() { return mp.http.get("https://a.org/s", { headers: { Cookie: "a=1" } }); }
export function spin() { for (;;) {} }
export function hang() { return new Promise(() => {}); }
"#;

    fn canned() -> Canned {
        Canned::default().with(
            "https://a.org/s?q=hi&p=1",
            r#"<ul><li class="item"><a href="/f/1.mp3"> First  song </a></li><li class="item"><a href="2.mp3">Two</a></li></ul>"#,
        )
    }

    #[test]
    fn search_and_resolve() {
        let v = run(SRC, "search", json!({ "query": "hi", "page": 1 }), canned()).unwrap();
        assert_eq!(v[0]["title"], "First song");
        assert_eq!(v[0]["extra"]["href"], "https://a.org/f/1.mp3");
        assert_eq!(v[1]["extra"]["href"], "https://a.org/2.mp3");
        assert_eq!(v[0]["lang"], "zh");
        let items = parse_search(v, "music").unwrap();
        let r = run(SRC, "resolve", json!({ "item": items[0] }), Canned::default()).unwrap();
        assert_eq!(r["target"]["url"], "https://a.org/f/1.mp3");
    }

    #[test]
    fn errors_and_limits() {
        let e = run(SRC, "boom", json!({}), Canned::default()).unwrap_err();
        assert_eq!(e, "[login_required] 先登录");
        let e = run(SRC, "plain", json!({}), Canned::default()).unwrap_err();
        assert!(e.starts_with("[internal] 插件出错"), "{e}");
        let e = run(SRC, "evil", json!({}), Canned::default()).unwrap_err();
        assert!(e.starts_with("[forbidden]") && e.contains("evil.org"), "{e}");
        let e = run(SRC, "cookie", json!({}), Canned::default()).unwrap_err();
        assert!(e.starts_with("[forbidden]") && e.contains("Cookie"), "{e}");
        let e = run(SRC, "missing", json!({}), Canned::default()).unwrap_err();
        assert!(e.starts_with("[unsupported]"), "{e}");
        let e = run_ms(SRC, "spin", json!({}), Canned::default(), 300).unwrap_err();
        assert!(e.contains("超时"), "{e}");
        let e = run(SRC, "hang", json!({}), Canned::default()).unwrap_err();
        assert!(e.contains("Promise"), "{e}");
        let e = run("import fs from 'fs'; export function search(){}", "search", json!({}), Canned::default()).unwrap_err();
        assert!(e.starts_with("[internal]"), "不能加载其他模块：{e}");
        let e = run(
            "export function search(){ return typeof require + typeof process + typeof setTimeout; }",
            "search",
            json!({}),
            Canned::default(),
        )
        .unwrap();
        assert_eq!(e, json!("undefinedundefinedundefined"));
    }
}
