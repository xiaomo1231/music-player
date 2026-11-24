//! 插件：带 `plugin.json` 的文件夹或 `.mpx` 包。入口可以是 .em 规则（rule）、外部程序（process，协议 2）
//! 或沙箱脚本（js）。插件提供的下载源装上就出现在“自动获取 → 我的来源”里。
//!
//! - `manifest`：清单格式和检查
//! - `package`：打开文件夹 / .mpx、安全解压、整包摘要
//! - `process`：process 入口的协议 2 运行器
//! - `source`：插件下载源（实现 `fetch::provider::Provider`）
//! - `repo`：插件仓库和更新
//! - `js`：js 入口（QuickJS 沙箱）
//!
//! 完整说明见 `docs/插件开发.md`。软件不内置任何第三方网站插件，也不内置插件仓库地址。
pub mod js;
pub mod manifest;
pub mod package;
pub mod process;
pub mod repo;
pub mod source;

use crate::em;
use crate::fetch::{self, provider};
use crate::AppState;
use anyhow::{anyhow, Result};
use manifest::{provider_id, Manifest};
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::{Mutex, RwLock};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;
fn es<T: ToString>(x: T) -> String {
    x.to_string()
}

/// 插件有变化（安装、启停、卸载、设置、状态）时发给前端
pub const CHANGED: &str = "plugin://changed";
const KEYRING_SERVICE: &str = "music-player-plugins";
const MAX_LOG: usize = 300;

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS plugins(
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            version TEXT NOT NULL DEFAULT '',
            runtime TEXT NOT NULL DEFAULT '',
            manifest TEXT NOT NULL DEFAULT '{}',
            enabled INTEGER NOT NULL DEFAULT 1,
            granted TEXT NOT NULL DEFAULT '{}',
            settings TEXT NOT NULL DEFAULT '{}',
            sha256 TEXT NOT NULL DEFAULT '',
            origin TEXT NOT NULL DEFAULT '',
            dev_path TEXT NOT NULL DEFAULT '',
            rules TEXT NOT NULL DEFAULT '[]',
            installed_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    )?;
    repo::init(conn)?;
    Ok(())
}

static DIR: OnceCell<PathBuf> = OnceCell::new();
static APP: OnceCell<AppHandle> = OnceCell::new();

/// 插件目录 `<数据目录>\plugins`
pub fn plugins_dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(|| std::env::temp_dir().join("mp-plugins"))
}

fn installed_dir(id: &str) -> PathBuf {
    plugins_dir().join(id)
}

fn prev_dir(id: &str) -> PathBuf {
    plugins_dir().join(".prev").join(id)
}

fn staging_dir() -> PathBuf {
    plugins_dir().join(".staging")
}

/// 插件自己的可写缓存目录
fn data_dir(id: &str) -> PathBuf {
    plugins_dir().join(".data").join(id)
}

fn app_version() -> String {
    APP.get().map(|a| a.package_info().version.to_string()).unwrap_or_else(|| "0.0.0".into())
}

// ---------------------------------------------------------------- 注册表

/// 用户同意的权限
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Granted {
    pub hosts: Vec<String>,
    /// 允许把“网站登录”里保存的登录交给插件
    pub login: bool,
    pub tools: Vec<String>,
    pub runtime: String,
}

impl Granted {
    fn of(m: &Manifest, login: bool) -> Granted {
        Granted {
            hosts: m.permissions.hosts.clone(),
            login: login && m.permissions.login() != "none",
            tools: m.permissions.tools.clone(),
            runtime: m.runtime.kind.clone(),
        }
    }
}

#[derive(Clone)]
struct Loaded {
    manifest: Manifest,
    dir: PathBuf,
    enabled: bool,
    dev: bool,
    problem: String,
    granted: Granted,
    settings: Map<String, Value>,
    sha256: String,
    origin: String,
    rules: Vec<String>,
    installed_at: i64,
    updated_at: i64,
}

impl Loaded {
    fn active(&self) -> bool {
        self.enabled && self.problem.is_empty()
    }
}

static REG: Lazy<RwLock<HashMap<String, Loaded>>> = Lazy::new(|| RwLock::new(HashMap::new()));

/// 每个插件下载源最近一次调用的结果
#[derive(Serialize, Clone, Debug, Default)]
pub struct Status {
    pub method: String,
    /// 空表示成功
    pub code: String,
    pub message: String,
    pub at: i64,
}

static STATUS: Lazy<RwLock<HashMap<String, Status>>> = Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Serialize, Clone, Debug)]
pub struct LogLine {
    pub at: i64,
    pub method: String,
    /// info / warn / debug / stderr / error / call
    pub level: String,
    pub msg: String,
}

static LOGS: Lazy<Mutex<HashMap<String, VecDeque<LogLine>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn log(plugin: &str, method: &str, level: &str, msg: &str) {
    let mut g = LOGS.lock();
    let q = g.entry(plugin.to_string()).or_default();
    q.push_back(LogLine {
        at: crate::db::now(),
        method: method.to_string(),
        level: level.to_string(),
        msg: msg.chars().take(2000).collect(),
    });
    while q.len() > MAX_LOG {
        q.pop_front();
    }
}

pub fn set_status(provider_id: &str, method: &str, err: Option<&str>) {
    let (code, message) = match err {
        None => (String::new(), String::new()),
        Some(e) => {
            let (c, m) = provider::split_code(e);
            (c.unwrap_or("internal").to_string(), m.to_string())
        }
    };
    STATUS.write().insert(
        provider_id.to_string(),
        Status {
            method: method.to_string(),
            code,
            message,
            at: crate::db::now(),
        },
    );
    if let Some(app) = APP.get() {
        let _ = app.emit(CHANGED, "status");
    }
}

/// 插件声明的网站（下载地址检查用）
pub fn allow_of(plugin: &str) -> em::net::Allow {
    let hosts = REG.read().get(plugin).map(|l| l.granted.hosts.clone()).unwrap_or_default();
    em::net::Allow::new(&hosts)
}

/// 下载源属于哪个插件：`plg-<插件>[.<源>]`，或插件带的 .em 规则 `em-<规则>`
pub fn owner_of(pid: &str) -> Option<String> {
    if let Some((plugin, _)) = manifest::split_provider_id(pid) {
        return Some(plugin.to_string());
    }
    let rule = pid.strip_prefix("em-")?;
    REG.read().iter().find(|(_, l)| l.rules.iter().any(|r| r == rule)).map(|(id, _)| id.clone())
}

/// 下载任务找不到下载源时的说明（插件停用 / 卸载）
pub fn missing_reason(pid: &str) -> Option<String> {
    let plugin = owner_of(pid)?;
    let plugin = plugin.as_str();
    let g = REG.read();
    Some(match g.get(plugin) {
        Some(l) if !l.enabled => provider::coded("plugin_disabled", &format!("插件“{}”已停用，启用后可以继续", l.manifest.name)),
        Some(l) if !l.problem.is_empty() => provider::coded("bad_config", &format!("插件“{}”有问题：{}", l.manifest.name, l.problem)),
        Some(_) => return None,
        None => format!("插件“{plugin}”已卸载"),
    })
}

/// 运行一次插件需要的东西
pub struct Runtime {
    pub argv: Vec<String>,
    pub dir: PathBuf,
    pub env: Vec<(String, String)>,
    pub settings: Value,
    pub context: Value,
    pub manifest: Manifest,
    pub granted: Granted,
}

/// 取运行环境：设置（含密钥）、Cookie、工具路径。插件停用或缺设置时直接返回带错误码的说明
pub fn runtime(plugin: &str) -> R<Runtime> {
    refresh_dev(plugin);
    let l = REG.read().get(plugin).cloned().ok_or_else(|| format!("插件“{plugin}”已卸载"))?;
    if !l.enabled {
        return Err(provider::coded("plugin_disabled", &format!("插件“{}”已停用", l.manifest.name)));
    }
    if !l.problem.is_empty() {
        return Err(provider::coded("bad_config", &l.problem));
    }
    let settings = full_settings(plugin, &l.manifest, &l.settings);
    if let Some(s) = missing_settings(&l.manifest, &settings).first() {
        return Err(provider::coded("bad_config", &format!("先在插件设置里填写“{s}”")));
    }
    let argv = l.manifest.command().unwrap_or_default();
    let data = data_dir(plugin);
    let _ = std::fs::create_dir_all(&data);
    let mut env = vec![
        ("MP_PLUGIN_ID".to_string(), plugin.to_string()),
        ("MP_PLUGIN_DIR".to_string(), l.dir.to_string_lossy().into_owned()),
        ("MP_PLUGIN_DATA".to_string(), data.to_string_lossy().into_owned()),
        ("MP_TEMP".to_string(), std::env::temp_dir().to_string_lossy().into_owned()),
        ("MP_LANG".to_string(), "zh-CN".to_string()),
        ("MP_APP_VERSION".to_string(), app_version()),
    ];
    for t in &l.granted.tools {
        let p = match t.as_str() {
            "ytdlp" => crate::tools::ytdlp(),
            "ffmpeg" => crate::tools::ffmpeg(),
            "ffprobe" => crate::tools::ffprobe(),
            _ => None,
        };
        if let Some(p) = p {
            env.push((format!("MP_{}", t.to_ascii_uppercase()), p.to_string_lossy().into_owned()));
        }
    }
    let mut cookies = Map::new();
    if l.granted.login {
        for h in l.granted.hosts.iter().filter(|h| !h.starts_with("*.")) {
            if let Some(c) = fetch::login::cookie_header(&format!("https://{h}/")) {
                cookies.insert(h.clone(), Value::String(c));
            }
        }
    }
    Ok(Runtime {
        argv,
        dir: l.dir.clone(),
        env,
        settings: Value::Object(settings),
        context: json!({ "app_version": app_version(), "lang": "zh-CN", "cookies": cookies }),
        manifest: l.manifest.clone(),
        granted: l.granted.clone(),
    })
}

// ---------------------------------------------------------------- 设置

fn secret_entry(plugin: &str, key: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("{plugin}/{key}")).ok()
}

fn get_secret(plugin: &str, key: &str) -> Option<String> {
    secret_entry(plugin, key)?.get_password().ok().filter(|s| !s.is_empty())
}

/// 设置值：保存的值 + 默认值 + 密钥（从系统凭据管理器取）
fn full_settings(plugin: &str, m: &Manifest, saved: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for s in &m.settings {
        let v = if s.kind == "secret" {
            get_secret(plugin, &s.key).map(Value::String).unwrap_or(Value::Null)
        } else {
            saved.get(&s.key).cloned().filter(|v| !v.is_null()).unwrap_or_else(|| s.default.clone())
        };
        out.insert(s.key.clone(), v);
    }
    out
}

fn is_empty_value(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        _ => false,
    }
}

/// 没填的必填设置（返回显示名）
fn missing_settings(m: &Manifest, values: &Map<String, Value>) -> Vec<String> {
    m.settings
        .iter()
        .filter(|s| s.required && is_empty_value(values.get(&s.key)))
        .map(|s| s.label.clone())
        .collect()
}

#[derive(Serialize)]
pub struct SettingsView {
    pub schema: Vec<manifest::Setting>,
    /// 密钥不回传，只给是否已填
    pub values: Map<String, Value>,
    pub secrets_set: Vec<String>,
}

#[tauri::command]
pub fn plugin_get_settings(id: String) -> R<SettingsView> {
    let l = REG.read().get(&id).cloned().ok_or("找不到这个插件")?;
    let mut values = Map::new();
    let mut secrets_set = vec![];
    for s in &l.manifest.settings {
        if s.kind == "secret" {
            if get_secret(&id, &s.key).is_some() {
                secrets_set.push(s.key.clone());
            }
        } else {
            values.insert(s.key.clone(), l.settings.get(&s.key).cloned().unwrap_or_else(|| s.default.clone()));
        }
    }
    Ok(SettingsView {
        schema: l.manifest.settings.clone(),
        values,
        secrets_set,
    })
}

/// 保存设置。密钥：非空字符串表示设置新值，null 表示清除，不传表示不变
#[tauri::command]
pub fn plugin_save_settings(app: AppHandle, st: State<AppState>, id: String, values: Map<String, Value>) -> R<()> {
    let l = REG.read().get(&id).cloned().ok_or("找不到这个插件")?;
    let mut saved = l.settings.clone();
    for s in &l.manifest.settings {
        let Some(v) = values.get(&s.key) else { continue };
        if s.kind == "secret" {
            let e = secret_entry(&id, &s.key).ok_or("系统凭据管理器不可用")?;
            match v {
                Value::String(x) if !x.is_empty() => e.set_password(x).map_err(|e| format!("保存密钥失败：{e}"))?,
                Value::Null => {
                    let _ = e.delete_credential();
                }
                _ => {}
            }
            continue;
        }
        let v = normalize_setting(s, v)?;
        saved.insert(s.key.clone(), v);
    }
    let text = serde_json::to_string(&saved).map_err(es)?;
    st.db
        .lock()
        .execute("UPDATE plugins SET settings=?2 WHERE id=?1", params![id, text])
        .map_err(es)?;
    if let Some(x) = REG.write().get_mut(&id) {
        x.settings = saved;
    }
    let _ = app.emit(CHANGED, "settings");
    Ok(())
}

fn normalize_setting(s: &manifest::Setting, v: &Value) -> R<Value> {
    Ok(match s.kind.as_str() {
        "bool" => Value::Bool(v.as_bool().unwrap_or(false)),
        "int" => match v {
            Value::Number(_) => v.clone(),
            Value::String(x) if x.trim().is_empty() => Value::Null,
            Value::String(x) => json!(x.trim().parse::<i64>().map_err(|_| format!("“{}”要填整数", s.label))?),
            _ => Value::Null,
        },
        "select" => {
            let x = v.as_str().unwrap_or("");
            let ok = s.options.iter().any(|o| o.as_str() == Some(x) || o["value"].as_str() == Some(x));
            if !ok && !x.is_empty() {
                return Err(format!("“{}”的选项不正确", s.label));
            }
            Value::String(x.to_string())
        }
        "folder" => {
            let x = v.as_str().unwrap_or("").trim().to_string();
            if !x.is_empty() && !Path::new(&x).is_dir() {
                return Err(format!("找不到文件夹：{x}"));
            }
            Value::String(x)
        }
        _ => Value::String(v.as_str().unwrap_or("").to_string()),
    })
}

// ---------------------------------------------------------------- 加载、生效

#[derive(Clone)]
struct Row {
    id: String,
    manifest: String,
    enabled: bool,
    granted: String,
    settings: String,
    sha256: String,
    origin: String,
    dev_path: String,
    rules: String,
    installed_at: i64,
    updated_at: i64,
}

fn load_rows(conn: &Connection) -> Result<Vec<Row>> {
    let mut st = conn.prepare("SELECT id,manifest,enabled,granted,settings,sha256,origin,dev_path,rules,installed_at,updated_at FROM plugins ORDER BY name")?;
    let rows = st
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                manifest: r.get(1)?,
                enabled: r.get::<_, i64>(2)? != 0,
                granted: r.get(3)?,
                settings: r.get(4)?,
                sha256: r.get(5)?,
                origin: r.get(6)?,
                dev_path: r.get(7)?,
                rules: r.get(8)?,
                installed_at: r.get(9)?,
                updated_at: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 从数据库的一行读出插件：读插件文件夹里的清单，检查摘要（安装后文件被改过就停用并提示）
fn load_row(r: &Row) -> Loaded {
    let dev = !r.dev_path.is_empty();
    let dir = if dev { PathBuf::from(&r.dev_path) } else { installed_dir(&r.id) };
    let saved: Manifest = serde_json::from_str(&r.manifest).unwrap_or_default();
    let mut problem = String::new();
    let manifest = match std::fs::read_to_string(dir.join("plugin.json")).map_err(es).and_then(|t| Manifest::parse(&t)) {
        Ok(m) => m,
        Err(e) => {
            problem = format!("读不到插件：{e}");
            saved.clone()
        }
    };
    let granted: Granted = serde_json::from_str(&r.granted).unwrap_or_default();
    if problem.is_empty() {
        let is = manifest.check(&dir, &app_version());
        if !is.is_empty() {
            problem = is.join("；");
        } else if !dev && package::digest(&dir).ok().as_deref() != Some(r.sha256.as_str()) {
            problem = "插件文件在安装后被改过，为了安全已停用。请重新安装，或在开发者模式里加载文件夹".into();
        } else if needs_reconsent(&granted, &manifest) {
            problem = "插件要求的权限变了（网站、登录、工具或入口类型），请重新安装确认".into();
        }
    }
    Loaded {
        manifest,
        dir,
        enabled: r.enabled,
        dev,
        problem,
        granted,
        settings: serde_json::from_str(&r.settings).unwrap_or_default(),
        sha256: r.sha256.clone(),
        origin: r.origin.clone(),
        rules: serde_json::from_str(&r.rules).unwrap_or_default(),
        installed_at: r.installed_at,
        updated_at: r.updated_at,
    }
}

/// 清单要求的权限超出用户同意过的
fn needs_reconsent(g: &Granted, m: &Manifest) -> bool {
    m.permissions.hosts.iter().any(|h| !g.hosts.contains(h))
        || m.permissions.tools.iter().any(|t| !g.tools.contains(t))
        || (m.runtime.kind != g.runtime && m.runtime.kind == "process")
}

/// 开发者模式加载的插件：清单改了就重新读
fn refresh_dev(id: &str) {
    let cur = REG.read().get(id).cloned();
    let Some(cur) = cur.filter(|l| l.dev) else { return };
    let Ok(text) = std::fs::read_to_string(cur.dir.join("plugin.json")) else {
        return;
    };
    let Ok(m) = Manifest::parse(&text) else { return };
    if m == cur.manifest {
        return;
    }
    let mut next = cur.clone();
    next.problem = m.check(&cur.dir, &app_version()).join("；");
    // 开发时声明的权限随改随用（只在开发者模式）
    next.granted = Granted::of(&m, cur.granted.login || m.permissions.login() != "none");
    next.manifest = m;
    REG.write().insert(id.to_string(), next.clone());
    if let Some(app) = APP.get() {
        apply(app, id, false, Some(&cur));
        if next.active() {
            apply(app, id, true, None);
        }
        let _ = app.emit(CHANGED, "reload");
    }
}

/// 启动时：读入所有插件，启用的注册下载源
pub fn setup(app: &AppHandle) {
    let st = app.state::<AppState>();
    let _ = DIR.set(st.dirs.data.join("plugins"));
    let _ = APP.set(app.clone());
    let _ = std::fs::create_dir_all(plugins_dir());
    let _ = std::fs::remove_dir_all(staging_dir());
    let rows = {
        let conn = st.db.lock();
        load_rows(&conn).unwrap_or_default()
    };
    for r in &rows {
        let l = load_row(r);
        REG.write().insert(r.id.clone(), l);
    }
    for r in &rows {
        let active = REG.read().get(&r.id).is_some_and(|l| l.active());
        if active {
            apply(app, &r.id, true, None);
        }
    }
    repo::setup(app);
}

/// 生效 / 失效：注册或移除下载源，启停随插件安装的规则
fn apply(app: &AppHandle, id: &str, on: bool, old: Option<&Loaded>) {
    let l = match old {
        Some(o) => o.clone(),
        None => match REG.read().get(id).cloned() {
            Some(l) => l,
            None => return,
        },
    };
    match l.manifest.runtime.kind.as_str() {
        "process" | "js" => {
            for s in &l.manifest.contributes.sources {
                let pid = provider_id(id, &s.id);
                if on {
                    if l.manifest.runtime.kind == "js" {
                        provider::register(Arc::new(js::JsSource::new(id, &l.manifest.name, s)));
                    } else {
                        provider::register(Arc::new(source::PluginSource::new(id, &l.manifest.name, s)));
                    }
                } else {
                    provider::unregister(&pid);
                }
            }
        }
        _ => {}
    }
    for r in &l.rules {
        let _ = em::set_enabled(app, r, on);
    }
}

/// 插件下载源在来源表里的地址（rule 入口用规则的 em- id）
fn source_urls(l: &Loaded) -> Vec<(String, String)> {
    let mut out = vec![];
    if l.manifest.runtime.kind == "rule" {
        if let Some(rule) = l.rules.first() {
            let name = l.manifest.contributes.sources.first().map(|s| s.name.clone()).filter(|n| !n.is_empty());
            out.push((provider::source_url(&format!("em-{rule}")), name.unwrap_or_else(|| l.manifest.name.clone())));
        }
    } else {
        for s in &l.manifest.contributes.sources {
            let name = if s.name.trim().is_empty() { l.manifest.name.clone() } else { s.name.clone() };
            out.push((provider::source_url(&provider_id(&l.manifest.id, &s.id)), name));
        }
    }
    out
}

/// 装上的下载源直接加进“我的来源”（已经在的只更新名字，用户关掉过的保持关闭）
fn upsert_sources(app: &AppHandle, l: &Loaded) {
    let urls = source_urls(l);
    let _ = fetch::with_db(app, |c| {
        for (url, name) in &urls {
            let n = c.execute("UPDATE fetch_sources SET name=?2 WHERE url=?1", params![url, name])?;
            if n == 0 {
                c.execute(
                    "INSERT INTO fetch_sources(kind,name,url,rule_json,created_at,enabled) VALUES('provider',?1,?2,'',?3,1)",
                    params![name, url, crate::db::now()],
                )?;
            }
        }
        Ok(())
    });
}

/// 旧方式（ext- 外部程序、em- 规则）升级成插件：来源、追更、没下完的任务转到新下载源
fn migrate_legacy(app: &AppHandle, l: &Loaded) {
    let Some(new_pid) = l.manifest.contributes.sources.first().map(|s| provider_id(&l.manifest.id, &s.id)) else {
        return;
    };
    if l.manifest.runtime.kind == "rule" {
        return;
    }
    let new_url = provider::source_url(&new_pid);
    for old in &l.manifest.legacy_ids {
        let old_url = provider::source_url(old);
        let _ = fetch::with_db(app, |c| {
            let exists: bool = c.query_row("SELECT COUNT(*) FROM fetch_sources WHERE url=?1", [&new_url], |r| r.get::<_, i64>(0))? > 0;
            if exists {
                c.execute(
                    "UPDATE fetch_subs SET source_id=(SELECT id FROM fetch_sources WHERE url=?2) WHERE source_id IN (SELECT id FROM fetch_sources WHERE url=?1)",
                    params![old_url, new_url],
                )?;
                c.execute("DELETE FROM fetch_sources WHERE url=?1", [&old_url])?;
            } else {
                c.execute("UPDATE fetch_sources SET url=?2, rule_json='' WHERE url=?1", params![old_url, new_url])?;
            }
            c.execute(
                "UPDATE fetch_tasks SET url=?2 || substr(url, length(?1) + 1) WHERE kind='provider' AND (url LIKE ?1 || '/%' OR url LIKE ?1 || '?%')",
                params![old_url, new_url],
            )?;
            Ok(())
        });
        provider::unregister(old);
    }
}

fn remove_sources(app: &AppHandle, l: &Loaded) {
    if l.manifest.runtime.kind == "rule" {
        return; // 规则删除时自己会移除
    }
    for (url, _) in source_urls(l) {
        let _ = fetch::with_db(app, |c| {
            c.execute("DELETE FROM fetch_subs WHERE source_id IN (SELECT id FROM fetch_sources WHERE url=?1)", [&url])?;
            c.execute("DELETE FROM fetch_sources WHERE url=?1", [&url])?;
            Ok(())
        });
    }
}

// ---------------------------------------------------------------- 给界面看的数据

#[derive(Serialize, Clone, Debug)]
pub struct SourceView {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub media_kinds: Vec<String>,
    pub capabilities: Vec<String>,
    pub status: Option<Status>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub homepage: String,
    pub license: String,
    pub runtime: String,
    /// limited / full
    pub trust: String,
    pub enabled: bool,
    pub dev: bool,
    pub dir: String,
    pub problem: String,
    pub hosts: Vec<String>,
    pub login: String,
    pub login_granted: bool,
    pub tools: Vec<String>,
    /// 图标文件的完整路径（前端用 fileUrl 显示）
    pub icon: String,
    pub readme: String,
    pub sources: Vec<SourceView>,
    /// 没填的必填设置
    pub needs_settings: Vec<String>,
    pub has_settings: bool,
    pub rules: Vec<String>,
    pub origin: String,
    pub installed_at: i64,
    pub updated_at: i64,
    pub has_prev: bool,
    /// 安装时整包的 sha256（开发者模式为空）
    pub sha256: String,
    pub update: Option<repo::Available>,
}

fn readme(dir: &Path) -> String {
    for n in ["README.md", "readme.md", "README.txt"] {
        if let Ok(t) = std::fs::read_to_string(dir.join(n)) {
            return t.chars().take(8000).collect();
        }
    }
    String::new()
}

fn view(id: &str, l: &Loaded) -> PluginView {
    let m = &l.manifest;
    let settings = full_settings(id, m, &l.settings);
    let status = STATUS.read();
    let sources = if m.runtime.kind == "rule" {
        let pid = l.rules.first().map(|r| format!("em-{r}")).unwrap_or_default();
        m.contributes
            .sources
            .iter()
            .take(1)
            .map(|s| SourceView {
                id: s.id.clone(),
                provider_id: pid.clone(),
                name: if s.name.is_empty() { m.name.clone() } else { s.name.clone() },
                media_kinds: s.media_kinds.clone(),
                capabilities: s.capabilities.clone(),
                status: None,
            })
            .collect()
    } else {
        m.contributes
            .sources
            .iter()
            .map(|s| {
                let pid = provider_id(id, &s.id);
                SourceView {
                    status: status.get(&pid).cloned(),
                    provider_id: pid,
                    id: s.id.clone(),
                    name: if s.name.is_empty() { m.name.clone() } else { s.name.clone() },
                    media_kinds: s.media_kinds.clone(),
                    capabilities: s.capabilities.clone(),
                }
            })
            .collect()
    };
    PluginView {
        id: id.to_string(),
        name: m.name.clone(),
        version: m.version.clone(),
        author: m.author.clone(),
        description: m.description.clone(),
        homepage: m.homepage.clone(),
        license: m.license.clone(),
        runtime: m.runtime.kind.clone(),
        trust: m.trust().into(),
        enabled: l.enabled,
        dev: l.dev,
        dir: l.dir.to_string_lossy().into_owned(),
        problem: l.problem.clone(),
        hosts: m.permissions.hosts.clone(),
        login: m.permissions.login().into(),
        login_granted: l.granted.login,
        tools: m.permissions.tools.clone(),
        icon: if m.icon.is_empty() {
            String::new()
        } else {
            l.dir.join(&m.icon).to_string_lossy().into_owned()
        },
        readme: readme(&l.dir),
        sources,
        needs_settings: missing_settings(m, &settings),
        has_settings: !m.settings.is_empty(),
        rules: l.rules.clone(),
        origin: l.origin.clone(),
        installed_at: l.installed_at,
        updated_at: l.updated_at,
        has_prev: !l.dev && prev_dir(id).join("plugin.json").is_file(),
        sha256: l.sha256.clone(),
        update: repo::available(id, &m.version),
    }
}

#[tauri::command]
pub fn plugin_list() -> Vec<PluginView> {
    let ids: Vec<String> = REG.read().keys().cloned().collect();
    for id in &ids {
        refresh_dev(id);
    }
    let g = REG.read();
    let mut v: Vec<PluginView> = g.iter().map(|(id, l)| view(id, l)).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn get_view(id: &str) -> R<PluginView> {
    let g = REG.read();
    let l = g.get(id).ok_or("找不到这个插件")?;
    Ok(view(id, l))
}

// ---------------------------------------------------------------- 安装前查看

#[derive(Serialize, Debug, Default)]
pub struct Preview {
    pub ok: bool,
    pub issues: Vec<String>,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub runtime: String,
    pub trust: String,
    pub media_kinds: Vec<String>,
    pub sources: Vec<String>,
    pub hosts: Vec<String>,
    pub lan_hosts: Vec<String>,
    pub login: String,
    pub tools: Vec<String>,
    pub rules: Vec<String>,
    pub settings: Vec<manifest::Setting>,
    pub readme: String,
    pub installed_version: Option<String>,
    /// 比已装版本多出来的权限（新网站、开始要登录、新工具、变成程序）
    pub new_permissions: Vec<String>,
    pub downgrade: bool,
    pub dev_installed: bool,
}

#[derive(Serialize, Debug)]
pub struct Inspect {
    /// plugin / em / exe
    pub kind: String,
    pub preview: Option<Preview>,
}

pub fn preview_of(dir: &Path, m: &Manifest) -> Preview {
    let mut p = Preview {
        issues: m.check(dir, &app_version()),
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        author: m.author.clone(),
        description: m.description.clone(),
        runtime: m.runtime.kind.clone(),
        trust: m.trust().into(),
        hosts: m.permissions.hosts.clone(),
        lan_hosts: m.permissions.hosts.iter().filter(|h| em::format::is_lan_host(h)).cloned().collect(),
        login: m.permissions.login().into(),
        tools: m.permissions.tools.clone(),
        rules: m.contributes.rules.clone(),
        settings: m.settings.clone(),
        readme: readme(dir),
        ..Default::default()
    };
    let mut kinds = vec![];
    for s in &m.contributes.sources {
        p.sources.push(if s.name.is_empty() { m.name.clone() } else { s.name.clone() });
        for k in &s.media_kinds {
            if !kinds.contains(k) {
                kinds.push(k.clone());
            }
        }
    }
    p.media_kinds = kinds;
    // rule 入口：.em 里的 hosts 必须和清单一致
    if m.runtime.kind == "rule" && p.issues.is_empty() {
        match std::fs::read_to_string(dir.join(&m.runtime.file))
            .map_err(es)
            .and_then(|t| em::format::parse(&t).map_err(|is| em::issues_text(&is)))
        {
            Ok(rule) => {
                if rule.kind() != em::Kind::Source {
                    p.issues.push("rule 入口要指向 kind: source 的规则".into());
                }
                let mut a = rule.header.hosts.clone();
                let mut b = m.permissions.hosts.clone();
                a.sort();
                b.sort();
                if a != b {
                    p.issues.push("plugin.json 的 permissions.hosts 要和 .em 里的 hosts 一致".into());
                }
                if let Err(is) = em::check(&rule) {
                    p.issues.push(em::issues_text(&is));
                }
            }
            Err(e) => p.issues.push(format!("{}：{e}", m.runtime.file)),
        }
    }
    for r in &m.contributes.rules {
        if let Err(e) = std::fs::read_to_string(dir.join(r))
            .map_err(es)
            .and_then(|t| em::format::parse(&t).map_err(|is| em::issues_text(&is)))
        {
            p.issues.push(format!("{r}：{e}"));
        }
    }
    if let Some(l) = REG.read().get(&m.id) {
        p.installed_version = Some(l.manifest.version.clone());
        p.dev_installed = l.dev;
        let g = &l.granted;
        for h in m.permissions.hosts.iter().filter(|h| !g.hosts.contains(h)) {
            p.new_permissions.push(format!("访问网站 {h}"));
        }
        if m.permissions.login() != "none" && l.manifest.permissions.login() == "none" {
            p.new_permissions.push("使用你保存的网站登录".into());
        }
        for t in m.permissions.tools.iter().filter(|t| !g.tools.contains(t)) {
            p.new_permissions.push(format!("使用软件自带的 {t}"));
        }
        if m.runtime.kind == "process" && l.manifest.runtime.kind != "process" {
            p.new_permissions.push("从受限的规则 / 脚本变成可以访问你电脑的程序".into());
        }
        if let (Some(a), Some(b)) = (manifest::parse_version(&m.version), manifest::parse_version(&l.manifest.version)) {
            p.downgrade = a < b;
        }
    }
    p.ok = p.issues.is_empty();
    p
}

/// 看一下要安装的东西：插件包 / 文件夹给出预览；.em 和 exe 交给原来的导入流程
#[tauri::command]
pub fn plugin_inspect(path: String) -> R<Inspect> {
    let p = PathBuf::from(path.trim().trim_matches('"'));
    let lower = p.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".em") {
        return Ok(Inspect {
            kind: "em".into(),
            preview: None,
        });
    }
    if p.is_file() && !package::is_zip(&p) && fetch::external::looks_like_program(&p.to_string_lossy()) {
        return Ok(Inspect {
            kind: "exe".into(),
            preview: None,
        });
    }
    let o = package::open(&p, &staging_dir()).map_err(es)?;
    Ok(Inspect {
        kind: "plugin".into(),
        preview: Some(preview_of(&o.dir, &o.manifest)),
    })
}

// ---------------------------------------------------------------- 安装、卸载、启停

/// 安装（或更新）插件包。`allow_login`：用户同意把保存的网站登录交给插件
#[tauri::command]
pub async fn plugin_install(app: AppHandle, path: String, allow_login: bool) -> R<PluginView> {
    let p = PathBuf::from(path.trim().trim_matches('"'));
    let app2 = app.clone();
    let id = tauri::async_runtime::spawn_blocking(move || install(&app2, &p, allow_login, "local"))
        .await
        .map_err(es)??;
    self_check(&id).await;
    get_view(&id)
}

/// 安装的实际步骤（仓库更新也用）
pub fn install(app: &AppHandle, path: &Path, allow_login: bool, origin: &str) -> R<String> {
    let o = package::open(path, &staging_dir()).map_err(es)?;
    let pv = preview_of(&o.dir, &o.manifest);
    if !pv.ok {
        return Err(pv.issues.join("\n"));
    }
    let m = o.manifest.clone();
    let id = m.id.clone();
    // 停掉旧版本
    let old = REG.read().get(&id).cloned();
    if let Some(l) = &old {
        if l.active() {
            apply(app, &id, false, Some(l));
        }
    }
    let result = (|| -> Result<(String, Vec<String>)> {
        std::fs::create_dir_all(plugins_dir())?;
        let fresh = plugins_dir().join(format!(".new-{id}"));
        let _ = std::fs::remove_dir_all(&fresh);
        package::copy_dir(&o.dir, &fresh)?;
        let sha = package::digest(&fresh)?;
        let target = installed_dir(&id);
        if target.exists() {
            let prev = prev_dir(&id);
            let _ = std::fs::remove_dir_all(&prev);
            std::fs::create_dir_all(prev.parent().unwrap())?;
            if old.as_ref().is_some_and(|l| !l.dev) {
                std::fs::rename(&target, &prev).or_else(|_| std::fs::remove_dir_all(&target))?;
            } else {
                std::fs::remove_dir_all(&target)?;
            }
        }
        std::fs::rename(&fresh, &target)?;
        // 规则：rule 入口的 .em 和 contributes.rules
        let mut files: Vec<String> = vec![];
        if m.runtime.kind == "rule" {
            files.push(m.runtime.file.clone());
        }
        files.extend(m.contributes.rules.iter().cloned());
        let mut rules = vec![];
        for f in files {
            let text = std::fs::read_to_string(target.join(&f))?;
            let row = em::install_text(app, &text, &id).map_err(|e| anyhow!("{f}：{e}"))?;
            if !rules.contains(&row.id) {
                rules.push(row.id);
            }
        }
        // 旧版本有、新版本没有的规则删掉
        if let Some(l) = &old {
            for r in l.rules.iter().filter(|r| !rules.contains(r)) {
                let _ = em::delete(app, r);
            }
        }
        Ok((sha, rules))
    })();
    let (sha, rules) = match result {
        Ok(x) => x,
        Err(e) => {
            if let Some(l) = &old {
                if l.active() {
                    apply(app, &id, true, Some(l));
                }
            }
            return Err(format!("安装失败：{e}"));
        }
    };
    let granted = Granted::of(&m, allow_login);
    let now = crate::db::now();
    let st = app.state::<AppState>();
    let settings = old.as_ref().map(|l| l.settings.clone()).unwrap_or_default();
    {
        let conn = st.db.lock();
        conn.execute(
            "INSERT INTO plugins(id,name,version,runtime,manifest,enabled,granted,settings,sha256,origin,dev_path,rules,installed_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9,'',?10,?11,?11)
             ON CONFLICT(id) DO UPDATE SET name=?2,version=?3,runtime=?4,manifest=?5,enabled=1,granted=?6,sha256=?8,origin=?9,dev_path='',rules=?10,updated_at=?11",
            params![
                id,
                m.name,
                m.version,
                m.runtime.kind,
                serde_json::to_string(&m).map_err(es)?,
                serde_json::to_string(&granted).map_err(es)?,
                serde_json::to_string(&settings).map_err(es)?,
                sha,
                origin,
                serde_json::to_string(&rules).map_err(es)?,
                now
            ],
        )
        .map_err(es)?;
    }
    let loaded = Loaded {
        manifest: m,
        dir: installed_dir(&id),
        enabled: true,
        dev: false,
        problem: String::new(),
        granted,
        settings,
        sha256: sha,
        origin: origin.to_string(),
        rules,
        installed_at: old.as_ref().map(|l| l.installed_at).unwrap_or(now),
        updated_at: now,
    };
    REG.write().insert(id.clone(), loaded.clone());
    apply(app, &id, true, None);
    upsert_sources(app, &loaded);
    migrate_legacy(app, &loaded);
    log(&id, "install", "info", &format!("已安装 {}", loaded.manifest.version));
    let _ = app.emit(CHANGED, "install");
    let _ = app.emit(em::CHANGED, "source");
    Ok(id)
}

/// 装好后调一次 info 自检（只记日志和状态）
async fn self_check(id: &str) {
    let Some(l) = REG.read().get(id).cloned() else { return };
    let Some(s) = l.manifest.contributes.sources.first() else { return };
    if l.manifest.runtime.kind == "process" && l.manifest.engines.protocol == 2 {
        let src = source::PluginSource::new(id, &l.manifest.name, s);
        let _ = info_call(&src).await;
    }
}

async fn info_call(src: &source::PluginSource) -> R<Value> {
    let rt = runtime(&src.plugin)?;
    let c = process::Call {
        argv: rt.argv,
        dir: rt.dir,
        env: rt.env,
        method: "info".into(),
        params: json!({ "source": src.decl.id }),
        settings: rt.settings,
        context: rt.context,
        timeout: Some(std::time::Duration::from_secs(15)),
        idle: None,
    };
    let plugin = src.plugin.clone();
    let r = process::run(
        &c,
        |e| {
            if let process::Event::Log { level, msg } = e {
                log(&plugin, "info", &level, &msg)
            }
        },
        None,
    )
    .await;
    set_status(&src.info.id, "info", r.as_ref().err().map(|s| s.as_str()));
    r
}

/// 开发者模式：直接用这个文件夹，不复制、不检查摘要，改了清单自动重新读
#[tauri::command]
pub fn plugin_load_dev(app: AppHandle, path: String) -> R<PluginView> {
    let dir = std::path::absolute(PathBuf::from(path.trim().trim_matches('"'))).map_err(es)?;
    if !dir.join("plugin.json").is_file() {
        return Err("这个文件夹里没有 plugin.json".into());
    }
    let o = package::open(&dir, &staging_dir()).map_err(es)?;
    let pv = preview_of(&o.dir, &o.manifest);
    if !pv.ok {
        return Err(pv.issues.join("\n"));
    }
    if o.manifest.runtime.kind == "rule" || !o.manifest.contributes.rules.is_empty() {
        return Err("开发者模式只支持 process / js 入口；.em 规则请直接导入，改完保存会自动生效".into());
    }
    let m = o.manifest.clone();
    let id = m.id.clone();
    if let Some(l) = REG.read().get(&id).cloned() {
        if !l.dev {
            return Err(format!("已经安装了“{}”，请先卸载再用开发者模式加载", l.manifest.name));
        }
        apply(&app, &id, false, Some(&l));
    }
    let granted = Granted::of(&m, true);
    let now = crate::db::now();
    let st = app.state::<AppState>();
    st.db
        .lock()
        .execute(
            "INSERT INTO plugins(id,name,version,runtime,manifest,enabled,granted,settings,sha256,origin,dev_path,rules,installed_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,1,?6,'{}','','dev',?7,'[]',?8,?8)
             ON CONFLICT(id) DO UPDATE SET name=?2,version=?3,runtime=?4,manifest=?5,enabled=1,granted=?6,dev_path=?7,updated_at=?8",
            params![
                id,
                m.name,
                m.version,
                m.runtime.kind,
                serde_json::to_string(&m).map_err(es)?,
                serde_json::to_string(&granted).map_err(es)?,
                dir.to_string_lossy(),
                now
            ],
        )
        .map_err(es)?;
    let settings = REG.read().get(&id).map(|l| l.settings.clone()).unwrap_or_default();
    let loaded = Loaded {
        manifest: m,
        dir,
        enabled: true,
        dev: true,
        problem: String::new(),
        granted,
        settings,
        sha256: String::new(),
        origin: "dev".into(),
        rules: vec![],
        installed_at: now,
        updated_at: now,
    };
    REG.write().insert(id.clone(), loaded.clone());
    apply(&app, &id, true, None);
    upsert_sources(&app, &loaded);
    let _ = app.emit(CHANGED, "install");
    get_view(&id)
}

#[tauri::command]
pub fn plugin_set_enabled(app: AppHandle, st: State<AppState>, id: String, enabled: bool) -> R<PluginView> {
    let l = REG.read().get(&id).cloned().ok_or("找不到这个插件")?;
    if l.active() {
        apply(&app, &id, false, Some(&l));
    }
    st.db
        .lock()
        .execute("UPDATE plugins SET enabled=?2 WHERE id=?1", params![id, enabled as i64])
        .map_err(es)?;
    if let Some(x) = REG.write().get_mut(&id) {
        x.enabled = enabled;
    }
    if REG.read().get(&id).is_some_and(|l| l.active()) {
        apply(&app, &id, true, None);
        // 之前因为插件停用而暂停的任务可以继续了
    }
    let _ = app.emit(CHANGED, "enabled");
    let _ = app.emit(em::CHANGED, "source");
    get_view(&id)
}

/// 交出或收回“网站登录”里保存的登录（只影响之后的调用）
#[tauri::command]
pub fn plugin_set_login(app: AppHandle, st: State<AppState>, id: String, allow: bool) -> R<PluginView> {
    let granted = {
        let mut g = REG.write();
        let l = g.get_mut(&id).ok_or("找不到这个插件")?;
        if allow && l.manifest.permissions.login() == "none" {
            return Err("这个插件没有声明要用登录".into());
        }
        l.granted.login = allow;
        l.granted.clone()
    };
    st.db
        .lock()
        .execute(
            "UPDATE plugins SET granted=?2 WHERE id=?1",
            params![id, serde_json::to_string(&granted).map_err(es)?],
        )
        .map_err(es)?;
    let _ = app.emit(CHANGED, "login");
    get_view(&id)
}

/// 卸载：移除下载源和随插件安装的规则，删插件文件夹。`purge` 时连设置、密钥、缓存一起删
#[tauri::command]
pub fn plugin_uninstall(app: AppHandle, st: State<AppState>, id: String, purge: bool) -> R<()> {
    let l = REG.write().remove(&id).ok_or("找不到这个插件")?;
    apply(&app, &id, false, Some(&l));
    remove_sources(&app, &l);
    for r in &l.rules {
        let _ = em::delete(&app, r);
    }
    st.db.lock().execute("DELETE FROM plugins WHERE id=?1", [&id]).map_err(es)?;
    if !l.dev {
        let _ = std::fs::remove_dir_all(installed_dir(&id));
        let _ = std::fs::remove_dir_all(prev_dir(&id));
    }
    if purge {
        let _ = std::fs::remove_dir_all(data_dir(&id));
        for s in l.manifest.settings.iter().filter(|s| s.kind == "secret") {
            if let Some(e) = secret_entry(&id, &s.key) {
                let _ = e.delete_credential();
            }
        }
    }
    LOGS.lock().remove(&id);
    let _ = app.emit(CHANGED, "uninstall");
    let _ = app.emit(em::CHANGED, "source");
    Ok(())
}

/// 回退到上一个版本（更新后发现有问题时）
#[tauri::command]
pub async fn plugin_rollback(app: AppHandle, id: String) -> R<PluginView> {
    let prev = prev_dir(&id);
    if !prev.join("plugin.json").is_file() {
        return Err("没有可以回退的上一个版本".into());
    }
    let keep = plugins_dir().join(".staging").join(format!("rollback-{id}"));
    let _ = std::fs::remove_dir_all(&keep);
    std::fs::create_dir_all(keep.parent().unwrap()).map_err(es)?;
    std::fs::rename(&prev, &keep).map_err(es)?;
    let login = REG.read().get(&id).is_some_and(|l| l.granted.login);
    let (app2, keep2) = (app.clone(), keep.clone());
    let r = tauri::async_runtime::spawn_blocking(move || install(&app2, &keep2, login, "rollback"))
        .await
        .map_err(es)?;
    let _ = std::fs::remove_dir_all(&keep);
    // 回退后不再保留“上一个版本”（就是刚才的新版本）
    let _ = std::fs::remove_dir_all(prev_dir(&id));
    r?;
    get_view(&id)
}

#[tauri::command]
pub fn plugin_logs(id: String) -> Vec<LogLine> {
    LOGS.lock().get(&id).map(|q| q.iter().cloned().collect()).unwrap_or_default()
}

#[tauri::command]
pub fn plugin_clear_logs(id: String) {
    LOGS.lock().remove(&id);
}

#[tauri::command]
pub fn plugin_open_dir(id: String) -> R<()> {
    let dir = REG.read().get(&id).map(|l| l.dir.clone()).ok_or("找不到这个插件")?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(|e| format!("打不开：{e}"))
}

/// 把插件文件夹打成 .mpx（开发者模式用）
#[tauri::command]
pub fn plugin_pack(id: String, path: String) -> R<()> {
    let dir = REG.read().get(&id).map(|l| l.dir.clone()).ok_or("找不到这个插件")?;
    package::pack(&dir, Path::new(&path)).map_err(|e| format!("打包失败：{e}"))
}

/// 打包任意插件文件夹（还没加载的也行）
#[tauri::command]
pub fn plugin_pack_dir(dir: String, path: String) -> R<()> {
    let d = PathBuf::from(&dir);
    let o = package::open(&d, &staging_dir()).map_err(es)?;
    let pv = preview_of(&o.dir, &o.manifest);
    if !pv.ok {
        return Err(pv.issues.join("\n"));
    }
    package::pack(&o.dir, Path::new(&path)).map_err(|e| format!("打包失败：{e}"))
}

// ---------------------------------------------------------------- 测试

#[derive(Serialize, Debug, Default)]
pub struct Stage {
    pub name: String,
    pub ok: bool,
    pub ms: u64,
    pub error: String,
    pub code: String,
    pub summary: String,
    pub preview: Value,
}

fn stage<T>(name: &str, started: std::time::Instant, r: &R<T>) -> Stage {
    let (code, error) = match r {
        Ok(_) => (String::new(), String::new()),
        Err(e) => {
            let (c, m) = provider::split_code(e);
            (c.unwrap_or("").to_string(), m.to_string())
        }
    };
    Stage {
        name: name.into(),
        ok: r.is_ok(),
        ms: started.elapsed().as_millis() as u64,
        error,
        code,
        ..Default::default()
    }
}

/// 测试一个下载源：自检、搜索（含分集）、解析第一条的下载地址（不下载）
#[tauri::command]
pub async fn plugin_test(id: String, source: String, query: String, media_kind: String) -> R<Vec<Stage>> {
    let l = REG.read().get(&id).cloned().ok_or("找不到这个插件")?;
    let decl = l
        .manifest
        .source(&source)
        .cloned()
        .or_else(|| l.manifest.contributes.sources.first().cloned())
        .ok_or("这个插件没有下载源")?;
    let mk = if media_kind.is_empty() {
        decl.media_kinds.first().cloned().unwrap_or_else(|| "music".into())
    } else {
        media_kind
    };
    let pid = if l.manifest.runtime.kind == "rule" {
        format!("em-{}", l.rules.first().cloned().unwrap_or_default())
    } else {
        provider_id(&id, &decl.id)
    };
    let p = provider::get(&pid).ok_or_else(|| missing_reason(&pid).unwrap_or_else(|| "插件没有生效，请先启用".into()))?;
    let mut out = vec![];
    if l.manifest.runtime.kind == "process" && l.manifest.engines.protocol == 2 {
        let src = source::PluginSource::new(&id, &l.manifest.name, &decl);
        let t = std::time::Instant::now();
        let r = info_call(&src).await;
        let mut s = stage("自检（info）", t, &r);
        if let Ok(v) = &r {
            s.summary = v["message"].as_str().unwrap_or("").to_string();
            s.preview = v.clone();
        }
        out.push(s);
    }
    if query.trim().is_empty() {
        return Ok(out);
    }
    let t = std::time::Instant::now();
    let r = p.search(query.trim(), &mk, 1).await;
    let mut s = stage("搜索", t, &r);
    let items = r.unwrap_or_default();
    s.summary = format!("{} 条结果", items.len());
    s.preview = json!(items.iter().take(5).collect::<Vec<_>>());
    out.push(s);
    let Some(first) = items.first() else { return Ok(out) };
    if p.can_resolve() {
        let q = first.qualities.first().map(|q| q.key.clone()).unwrap_or_default();
        let t = std::time::Instant::now();
        let r = p.resolve(first, &q).await;
        let mut s = stage("解析下载地址（不下载）", t, &r);
        if let Ok(res) = &r {
            s.summary = em::source::target_summary(&res.target, None)["label"].as_str().unwrap_or("").to_string();
            s.preview = em::source::target_summary(&res.target, None);
        }
        out.push(s);
    } else {
        out.push(Stage {
            name: "下载".into(),
            ok: true,
            summary: "这个下载源自己下载，测试时不实际下载；在搜索页下载一条看看".into(),
            ..Default::default()
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconsent_rules() {
        let m = Manifest::parse(r#"{"runtime":{"type":"process"},"permissions":{"hosts":["a.com"],"tools":["ytdlp"]}}"#).unwrap();
        let g = Granted::of(&m, false);
        assert!(!needs_reconsent(&g, &m));
        let mut m2 = m.clone();
        m2.permissions.hosts.push("b.com".into());
        assert!(needs_reconsent(&g, &m2));
        let mut m3 = m.clone();
        m3.permissions.tools.push("ffmpeg".into());
        assert!(needs_reconsent(&g, &m3));
        let r = Manifest::parse(r#"{"runtime":{"type":"rule"}}"#).unwrap();
        let gr = Granted::of(&r, false);
        assert!(needs_reconsent(&gr, &m), "规则变成程序要重新确认");
        assert!(!needs_reconsent(&g, &r));
    }

    #[test]
    fn settings_values() {
        let m = Manifest::parse(
            r#"{"settings":[{"key":"dir","type":"folder","label":"曲库","required":true},{"key":"n","type":"int","label":"条数","default":50},{"key":"q","type":"select","label":"音质","options":["flac","mp3"]}]}"#,
        )
        .unwrap();
        let mut saved = Map::new();
        let v = full_settings("x", &m, &saved);
        assert_eq!(v["n"], json!(50));
        assert_eq!(missing_settings(&m, &v), vec!["曲库".to_string()]);
        saved.insert("dir".into(), json!(std::env::temp_dir().to_string_lossy()));
        assert!(missing_settings(&m, &full_settings("x", &m, &saved)).is_empty());
        assert_eq!(normalize_setting(&m.settings[1], &json!("12")).unwrap(), json!(12));
        assert!(normalize_setting(&m.settings[1], &json!("abc")).is_err());
        assert!(normalize_setting(&m.settings[2], &json!("ogg")).is_err());
        assert!(normalize_setting(&m.settings[0], &json!("/no/such/dir/x")).is_err());
    }
}
