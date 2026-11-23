//! 插件仓库：用户自己添加的索引地址（`index.json`），列出其中的插件、安装、检查更新。
//! 软件不内置、不推荐任何仓库地址。格式见 `docs/插件开发.md` 第 6 节
use super::manifest;
use crate::fetch::source;
use crate::AppState;
use anyhow::Result;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;
fn es<T: ToString>(x: T) -> String {
    x.to_string()
}

const MAX_INDEX: usize = 2 << 20;
const MAX_PACKAGE: usize = 50 << 20;
/// 自动检查更新的间隔
const CHECK_EVERY: i64 = 24 * 3600;
/// 设置表里的键：自动更新（不需要新权限的更新直接装）
pub const AUTO_UPDATE_KEY: &str = "plugin.auto_update";

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS plugin_repos(
            url TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            index_json TEXT NOT NULL DEFAULT '',
            last_check INTEGER NOT NULL DEFAULT 0,
            last_error TEXT NOT NULL DEFAULT '',
            added_at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Index {
    pub index: u32,
    pub name: String,
    pub plugins: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// .mpx 地址，可以相对索引地址
    pub url: String,
    /// .mpx 文件的 SHA-256（十六进制）
    pub sha256: String,
    /// 最低软件版本
    pub app: String,
    pub runtime: String,
    pub media_kinds: Vec<String>,
    pub hosts: Vec<String>,
    pub login: String,
    pub tools: Vec<String>,
}

/// 已装插件在仓库里有新版本
#[derive(Serialize, Clone, Debug)]
pub struct Available {
    pub version: String,
    pub repo: String,
    /// 新版本要多出来的权限；空表示可以直接更新
    pub new_permissions: Vec<String>,
}

/// 内存里的各仓库索引（启动时从数据库读，刷新时更新）
static CACHE: Lazy<RwLock<HashMap<String, Index>>> = Lazy::new(|| RwLock::new(HashMap::new()));

pub fn parse_index(bytes: &[u8]) -> R<Index> {
    if bytes.len() > MAX_INDEX {
        return Err("仓库索引太大".into());
    }
    let ix: Index = serde_json::from_slice(bytes).map_err(|e| format!("仓库索引格式不对：{e}"))?;
    if ix.index != 1 {
        return Err(format!("不支持的仓库索引版本：{}", ix.index));
    }
    for (i, p) in ix.plugins.iter().enumerate() {
        if p.id.is_empty() || p.url.is_empty() || manifest::parse_version(&p.version).is_none() {
            return Err(format!("仓库索引第 {} 个插件缺少 id、url 或 version", i + 1));
        }
        if p.sha256.len() != 64 || !p.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("仓库索引里“{}”的 sha256 不正确", p.id));
        }
    }
    Ok(ix)
}

/// 相对地址按索引地址补全
pub fn join_url(base: &str, rel: &str) -> R<String> {
    let b = url::Url::parse(base).map_err(|_| "仓库地址不正确".to_string())?;
    let u = b.join(rel).map_err(|_| format!("插件地址不正确：{rel}"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("只支持 http / https 地址".into());
    }
    Ok(u.to_string())
}

async fn get_bytes(url: &str, max: usize) -> R<Vec<u8>> {
    source::throttle(url).await;
    let resp = source::client().get(url).send().await.map_err(|e| format!("连接失败：{e}"))?;
    source::check_status(resp.status())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    if resp.content_length().is_some_and(|n| n as usize > max) {
        return Err("文件太大".into());
    }
    let b = resp.bytes().await.map_err(|e| format!("下载失败：{e}"))?;
    if b.len() > max {
        return Err("文件太大".into());
    }
    Ok(b.to_vec())
}

/// 和已同意的权限比，新版本多出来的
pub fn new_permissions(e: &Entry, granted: &super::Granted, old_login: &str) -> Vec<String> {
    let mut out = vec![];
    for h in e.hosts.iter().filter(|h| !granted.hosts.contains(h)) {
        out.push(format!("访问网站 {h}"));
    }
    for t in e.tools.iter().filter(|t| !granted.tools.contains(t)) {
        out.push(format!("使用软件自带的 {t}"));
    }
    let login = if e.login.is_empty() { "none" } else { e.login.as_str() };
    if login != "none" && old_login == "none" {
        out.push("使用你保存的网站登录".into());
    }
    if e.runtime == "process" && granted.runtime != "process" {
        out.push("从受限的规则 / 脚本变成可以访问你电脑的程序".into());
    }
    out
}

/// 插件 `id` 在各仓库里比 `version` 新的最高版本
pub fn available(id: &str, version: &str) -> Option<Available> {
    let cur = manifest::parse_version(version)?;
    let app = super::app_version();
    let (granted, login) = super::REG
        .read()
        .get(id)
        .map(|l| (l.granted.clone(), l.manifest.permissions.login().to_string()))?;
    let cache = CACHE.read();
    let mut best: Option<(manifest::Ver, String, Entry)> = None;
    for (repo, ix) in cache.iter() {
        for e in ix.plugins.iter().filter(|e| e.id == id) {
            let Some(v) = manifest::parse_version(&e.version) else { continue };
            if v <= cur || !manifest::app_satisfies(&e.app, &app) {
                continue;
            }
            if best.as_ref().map_or(true, |(bv, _, _)| v > *bv) {
                best = Some((v, repo.clone(), e.clone()));
            }
        }
    }
    best.map(|(_, repo, e)| Available {
        new_permissions: new_permissions(&e, &granted, &login),
        version: e.version,
        repo,
    })
}

fn load_cache(conn: &Connection) {
    let mut st = match conn.prepare("SELECT url, index_json FROM plugin_repos") {
        Ok(s) => s,
        Err(_) => return,
    };
    let rows: Vec<(String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|it| it.flatten().collect())
        .unwrap_or_default();
    let mut c = CACHE.write();
    for (u, j) in rows {
        if let Ok(ix) = serde_json::from_str::<Index>(&j) {
            c.insert(u, ix);
        }
    }
}

/// 启动时：读缓存的索引；超过一天没检查就在后台刷新，并自动装不需要新权限的更新
pub fn setup(app: &AppHandle) {
    {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        load_cache(&conn);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let due = {
                let st = app.state::<AppState>();
                let conn = st.db.lock();
                conn.query_row(
                    "SELECT COUNT(*) FROM plugin_repos WHERE last_check < ?1",
                    [crate::db::now() - CHECK_EVERY],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                    > 0
            };
            if due {
                let _ = refresh_all(&app).await;
                auto_update(&app).await;
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

async fn refresh_one(app: &AppHandle, url: &str) -> R<Index> {
    let r = get_bytes(url, MAX_INDEX).await.and_then(|b| parse_index(&b));
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    match &r {
        Ok(ix) => {
            conn.execute(
                "UPDATE plugin_repos SET name=?2, index_json=?3, last_check=?4, last_error='' WHERE url=?1",
                params![url, ix.name, serde_json::to_string(ix).unwrap_or_default(), crate::db::now()],
            )
            .map_err(es)?;
            CACHE.write().insert(url.to_string(), ix.clone());
        }
        Err(e) => {
            conn.execute(
                "UPDATE plugin_repos SET last_check=?2, last_error=?3 WHERE url=?1",
                params![url, crate::db::now(), e],
            )
            .map_err(es)?;
        }
    }
    r
}

async fn refresh_all(app: &AppHandle) -> R<()> {
    let urls: Vec<String> = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let mut s = conn.prepare("SELECT url FROM plugin_repos").map_err(es)?;
        let v = s.query_map([], |r| r.get(0)).map_err(es)?.flatten().collect();
        v
    };
    for u in urls {
        let _ = refresh_one(app, &u).await;
    }
    let _ = app.emit(super::CHANGED, "repos");
    Ok(())
}

fn auto_update_on(app: &AppHandle) -> bool {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    crate::library::get_setting(&conn, AUTO_UPDATE_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

async fn auto_update(app: &AppHandle) {
    if !auto_update_on(app) {
        return;
    }
    let list: Vec<(String, String, bool)> = super::REG
        .read()
        .iter()
        .filter(|(_, l)| !l.dev)
        .map(|(id, l)| (id.clone(), l.manifest.version.clone(), l.granted.login))
        .collect();
    for (id, ver, login) in list {
        if let Some(a) = available(&id, &ver) {
            if a.new_permissions.is_empty() {
                match install_from(app, &a.repo, &id, login).await {
                    Ok(_) => super::log(&id, "update", "info", &format!("已自动更新到 {}", a.version)),
                    Err(e) => super::log(&id, "update", "error", &format!("自动更新失败：{e}")),
                }
            }
        }
    }
}

/// 从仓库下载并安装（新装或更新）
pub async fn install_from(app: &AppHandle, repo: &str, id: &str, allow_login: bool) -> R<String> {
    let entry = CACHE
        .read()
        .get(repo)
        .and_then(|ix| {
            ix.plugins
                .iter()
                .filter(|e| e.id == id)
                .max_by_key(|e| manifest::parse_version(&e.version))
                .cloned()
        })
        .ok_or("仓库里没有这个插件，请先刷新仓库")?;
    let url = join_url(repo, &entry.url)?;
    let bytes = get_bytes(&url, MAX_PACKAGE).await?;
    let sha = hex::encode(Sha256::digest(&bytes));
    if !sha.eq_ignore_ascii_case(&entry.sha256) {
        return Err("下载的插件包和仓库里记录的不一致（sha256 不同），已停止安装".into());
    }
    let dir = super::staging_dir();
    std::fs::create_dir_all(&dir).map_err(es)?;
    let file = dir.join(format!("{id}-{}.mpx", entry.version));
    std::fs::write(&file, &bytes).map_err(es)?;
    let (app2, file2, repo2) = (app.clone(), file.clone(), repo.to_string());
    let r = tauri::async_runtime::spawn_blocking(move || super::install(&app2, &file2, allow_login, &repo2))
        .await
        .map_err(es)?;
    let _ = std::fs::remove_file(&file);
    let got = r?;
    if got != id {
        return Err(format!("插件包里的 id 是“{got}”，和仓库里写的“{id}”不一致"));
    }
    Ok(got)
}

// ---------------------------------------------------------------- 命令

#[derive(Serialize)]
pub struct RepoEntryView {
    #[serde(flatten)]
    pub entry: Entry,
    pub installed_version: Option<String>,
    /// install / update / installed / newer_app（需要升级软件）
    pub state: String,
}

#[derive(Serialize)]
pub struct RepoView {
    pub url: String,
    pub name: String,
    pub last_check: i64,
    pub last_error: String,
    pub plugins: Vec<RepoEntryView>,
}

fn repo_views(conn: &Connection) -> R<Vec<RepoView>> {
    let mut st = conn
        .prepare("SELECT url,name,last_check,last_error FROM plugin_repos ORDER BY added_at")
        .map_err(es)?;
    let rows: Vec<(String, String, i64, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(es)?
        .flatten()
        .collect();
    let app = super::app_version();
    let cache = CACHE.read();
    let reg = super::REG.read();
    Ok(rows
        .into_iter()
        .map(|(url, name, last_check, last_error)| {
            let plugins = cache
                .get(&url)
                .map(|ix| {
                    ix.plugins
                        .iter()
                        .map(|e| {
                            let installed = reg.get(&e.id).map(|l| l.manifest.version.clone());
                            let state = if !manifest::app_satisfies(&e.app, &app) {
                                "newer_app"
                            } else {
                                match installed.as_deref().and_then(manifest::parse_version) {
                                    None if installed.is_none() => "install",
                                    Some(v) if manifest::parse_version(&e.version).is_some_and(|n| n > v) => "update",
                                    _ => "installed",
                                }
                            };
                            RepoEntryView {
                                entry: e.clone(),
                                installed_version: installed,
                                state: state.into(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            RepoView {
                url,
                name,
                last_check,
                last_error,
                plugins,
            }
        })
        .collect())
}

#[tauri::command]
pub fn plugin_repos(st: State<AppState>) -> R<Vec<RepoView>> {
    repo_views(&st.db.lock())
}

#[tauri::command]
pub async fn plugin_repo_add(app: AppHandle, url: String) -> R<Vec<RepoView>> {
    let url = url.trim().to_string();
    let u = url::Url::parse(&url).map_err(|_| "仓库地址不正确，应当是 index.json 的网址".to_string())?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("只支持 http / https 地址".into());
    }
    let ix = parse_index(&get_bytes(&url, MAX_INDEX).await?)?;
    {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        conn.execute(
            "INSERT INTO plugin_repos(url,name,index_json,last_check,last_error,added_at) VALUES(?1,?2,?3,?4,'',?4)
             ON CONFLICT(url) DO UPDATE SET name=?2,index_json=?3,last_check=?4,last_error=''",
            params![url, ix.name, serde_json::to_string(&ix).map_err(es)?, crate::db::now()],
        )
        .map_err(es)?;
    }
    CACHE.write().insert(url, ix);
    let _ = app.emit(super::CHANGED, "repos");
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    repo_views(&conn)
}

#[tauri::command]
pub fn plugin_repo_remove(app: AppHandle, st: State<AppState>, url: String) -> R<()> {
    st.db.lock().execute("DELETE FROM plugin_repos WHERE url=?1", [&url]).map_err(es)?;
    CACHE.write().remove(&url);
    let _ = app.emit(super::CHANGED, "repos");
    Ok(())
}

#[tauri::command]
pub async fn plugin_repo_refresh(app: AppHandle) -> R<Vec<RepoView>> {
    refresh_all(&app).await?;
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    repo_views(&conn)
}

/// 仓库里的插件安装前先看一眼：下载、校验、给出和本地安装一样的预览
#[tauri::command]
pub async fn plugin_repo_inspect(repo: String, id: String) -> R<super::Preview> {
    let entry = CACHE
        .read()
        .get(&repo)
        .and_then(|ix| {
            ix.plugins
                .iter()
                .filter(|e| e.id == id)
                .max_by_key(|e| manifest::parse_version(&e.version))
                .cloned()
        })
        .ok_or("仓库里没有这个插件，请先刷新仓库")?;
    let url = join_url(&repo, &entry.url)?;
    let bytes = get_bytes(&url, MAX_PACKAGE).await?;
    if !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(&entry.sha256) {
        return Err("下载的插件包和仓库里记录的不一致（sha256 不同）".into());
    }
    let dir = super::staging_dir();
    std::fs::create_dir_all(&dir).map_err(es)?;
    let file = dir.join(format!("inspect-{id}.mpx"));
    std::fs::write(&file, &bytes).map_err(es)?;
    let r = super::package::open(&file, &dir).map_err(es).map(|o| super::preview_of(&o.dir, &o.manifest));
    let _ = std::fs::remove_file(&file);
    let p = r?;
    if p.id != id {
        return Err(format!("插件包里的 id 是“{}”，和仓库里写的“{id}”不一致", p.id));
    }
    Ok(p)
}

#[tauri::command]
pub async fn plugin_repo_install(app: AppHandle, repo: String, id: String, allow_login: bool) -> R<super::PluginView> {
    let id = install_from(&app, &repo, &id, allow_login).await?;
    super::get_view(&id)
}

/// 用仓库里的新版本更新一个已装插件
#[tauri::command]
pub async fn plugin_update(app: AppHandle, id: String) -> R<super::PluginView> {
    let (ver, login) = super::REG
        .read()
        .get(&id)
        .map(|l| (l.manifest.version.clone(), l.granted.login))
        .ok_or("找不到这个插件")?;
    let a = available(&id, &ver).ok_or("没有可用的更新，可以先刷新仓库")?;
    install_from(&app, &a.repo, &id, login).await?;
    super::get_view(&id)
}

#[tauri::command]
pub fn plugin_get_auto_update(app: AppHandle) -> bool {
    auto_update_on(&app)
}

#[tauri::command]
pub fn plugin_set_auto_update(st: State<AppState>, on: bool) -> R<()> {
    crate::library::set_setting(&st.db.lock(), AUTO_UPDATE_KEY, &serde_json::Value::Bool(on)).map_err(es)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn index_parse_and_urls() {
        let j = format!(r#"{{"index":1,"name":"我的仓库","plugins":[{{"id":"a","version":"1.2.0","url":"a-1.2.0.mpx","sha256":"{SHA}"}}]}}"#);
        let ix = parse_index(j.as_bytes()).unwrap();
        assert_eq!(ix.plugins[0].id, "a");
        assert!(parse_index(br#"{"index":2,"plugins":[]}"#).is_err());
        assert!(parse_index(br#"{"index":1,"plugins":[{"id":"a","version":"1.0.0","url":"x","sha256":"zz"}]}"#).is_err());
        assert_eq!(join_url("https://x.org/repo/index.json", "a-1.mpx").unwrap(), "https://x.org/repo/a-1.mpx");
        assert_eq!(join_url("https://x.org/repo/index.json", "https://y.org/b.mpx").unwrap(), "https://y.org/b.mpx");
        assert!(join_url("https://x.org/index.json", "file:///etc/passwd").is_err());
    }

    #[test]
    fn permission_diff() {
        let g = super::super::Granted {
            hosts: vec!["a.com".into()],
            login: false,
            tools: vec![],
            runtime: "rule".into(),
        };
        let e = Entry {
            hosts: vec!["a.com".into(), "b.com".into()],
            tools: vec!["ytdlp".into()],
            login: "optional".into(),
            runtime: "process".into(),
            ..Default::default()
        };
        let d = new_permissions(&e, &g, "none");
        assert_eq!(d.len(), 4, "{d:?}");
        let same = Entry {
            hosts: vec!["a.com".into()],
            runtime: "rule".into(),
            ..Default::default()
        };
        assert!(new_permissions(&same, &g, "none").is_empty());
    }
}
