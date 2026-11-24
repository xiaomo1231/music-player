//! 自动获取：来源（订阅 RSS / yt-dlp / 目录 / 网页 AI 解析）、下载引擎（HTTP / BT / yt-dlp）、
//! 自动归类入库、追更。子模块：source（各类来源）、web（网页 AI 规则）、download（下载队列）、organize（整理入库）。
pub mod bili;
pub mod download;
pub mod external;
pub mod login;
pub mod organize;
pub mod pack;
pub mod provider;
pub mod source;
pub mod torrent;
pub mod web;
pub mod ytdlp;

use crate::{ai, db, library, AppState};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

pub(crate) fn e<T: ToString>(x: T) -> String {
    x.to_string()
}

/// 启动时建表（CREATE TABLE IF NOT EXISTS）
pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fetch_sources(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            name TEXT NOT NULL DEFAULT '',
            url TEXT NOT NULL,
            rule_json TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS fetch_tasks(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id INTEGER,
            title TEXT NOT NULL DEFAULT '',
            kind TEXT NOT NULL,
            url TEXT NOT NULL,
            status TEXT NOT NULL,
            progress REAL NOT NULL DEFAULT 0,
            speed INTEGER NOT NULL DEFAULT 0,
            size INTEGER NOT NULL DEFAULT 0,
            save_path TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            finished_at INTEGER,
            media_kind TEXT NOT NULL DEFAULT 'video'
        );
        CREATE TABLE IF NOT EXISTS fetch_subs(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id INTEGER NOT NULL,
            query TEXT NOT NULL,
            media_kind TEXT NOT NULL,
            last_check INTEGER,
            seen_json TEXT NOT NULL DEFAULT '[]'
        );",
    )?;
    // 种子任务只下载选中的文件：JSON 数组（文件序号），空表示全部
    db::add_column(conn, "fetch_tasks", "files", "TEXT NOT NULL DEFAULT ''")?;
    Ok(())
}

/// 启动后台任务：上次没结束的任务改为暂停，恢复排队，每 30 分钟检查一次追更
pub fn setup(app: &AppHandle) {
    let _ = with_db(app, |c| {
        c.execute(
            "UPDATE fetch_tasks SET status='paused' WHERE status IN ('downloading','organizing','seeding')",
            [],
        )?;
        Ok(())
    });
    register_external(app);
    download::pump(app);
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            check_subs(&h).await;
            tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        }
    });
}

/// 注册用户添加过的外部程序下载源（信息存在来源表的 rule_json 里，启动时不用运行程序）
fn register_external(app: &AppHandle) {
    let rows = with_db(app, |c| list_sources(c, false)).unwrap_or_default();
    for r in rows.iter().filter(|r| r.kind == "provider") {
        if let Some(s) = external::Stored::parse(&r.rule_json) {
            external::register(&s);
        }
    }
}

// ---------- 数据库小工具 ----------

/// 在数据库锁里执行一段同步代码（里面不要 await）
pub(crate) fn with_db<T>(app: &AppHandle, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    f(&conn)
}

// ---------- 数据结构 ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Hit {
    pub source_id: i64,
    pub source_name: String,
    pub title: String,
    pub url: String,
    pub size: Option<i64>,
    pub date: Option<String>,
    /// magnet / torrent / http / ytdlp / file
    pub kind: String,
    /// 识别出的作品名（没识别出时同标题）
    pub work: String,
    pub season: Option<i64>,
    pub episode: Option<f64>,
    pub group: Option<String>,
    pub resolution: Option<String>,
    pub special: bool,
    /// 这次搜索是否重新生成了网页提取规则
    pub rule_updated: bool,
}

#[derive(Serialize, Debug)]
pub struct Group {
    pub work: String,
    pub indices: Vec<usize>,
}

#[derive(Serialize, Debug, Default)]
pub struct SearchOut {
    pub hits: Vec<Hit>,
    pub groups: Vec<Group>,
    /// 某个来源失败时的说明（其他来源照常返回结果）
    pub notes: Vec<String>,
    /// 每个来源的结果数、用时和错误（错误码见 `provider::ERROR_CODES`）
    pub stats: Vec<SourceStat>,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct SourceStat {
    pub source_id: i64,
    pub name: String,
    pub count: usize,
    pub ms: u64,
    pub error: String,
    pub code: String,
    /// 这个来源不支持当前类型，没有搜
    pub skipped: bool,
}

#[derive(Serialize, Debug)]
pub struct Pick {
    pub index: usize,
    pub reason: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct SourceView {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub created_at: i64,
    /// 网页来源：rules（已生成规则）或 ai_direct（每次让 AI 读页面）；其他类型为空
    pub mode: String,
    pub has_rule: bool,
    /// 插件提供的下载源：插件 id（内置下载源和其他来源为空）
    pub plugin: String,
    /// 下载源能搜的类型（空表示不限）
    pub media_kinds: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SourceRow {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub rule_json: String,
    pub enabled: bool,
    pub created_at: i64,
}

impl SourceRow {
    pub fn view(&self) -> SourceView {
        let stored = web::StoredRule::parse(&self.rule_json);
        SourceView {
            id: self.id,
            kind: self.kind.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            enabled: self.enabled,
            created_at: self.created_at,
            mode: if self.kind == "web" { stored.mode } else { String::new() },
            has_rule: !self.rule_json.is_empty(),
            plugin: self.plugin_id(),
            media_kinds: self.media_kinds(),
        }
    }

    fn pid(&self) -> Option<&str> {
        (self.kind == "provider").then(|| provider::provider_id(&self.url)).flatten()
    }

    /// 插件下载源所属的插件（.em 规则装进插件时也算）
    pub fn plugin_id(&self) -> String {
        self.pid().and_then(crate::plugin::owner_of).unwrap_or_default()
    }

    pub fn media_kinds(&self) -> Vec<String> {
        self.pid().and_then(provider::get).map(|p| p.info().media_kinds).unwrap_or_default()
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct TaskView {
    pub id: i64,
    pub source_id: Option<i64>,
    pub title: String,
    pub kind: String,
    pub url: String,
    /// queued / downloading / paused / seeding / organizing / done / manual / failed / canceled
    pub status: String,
    pub progress: f64,
    pub speed: i64,
    pub size: i64,
    pub save_path: String,
    pub error: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub media_kind: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DlItem {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub media_kind: String,
    #[serde(default)]
    pub source_id: Option<i64>,
    #[serde(default)]
    pub size: Option<i64>,
    /// 种子里要下载的文件序号；None 表示全部
    #[serde(default)]
    pub files: Option<Vec<usize>>,
}

#[derive(Serialize, Clone, Debug)]
pub struct SubView {
    pub id: i64,
    pub source_id: i64,
    pub source_name: String,
    pub query: String,
    pub media_kind: String,
    pub last_check: Option<i64>,
    /// 已记为“见过”的条目数
    pub seen: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct FetchSettings {
    /// 下载目录（默认 “下载\极光播放器”）
    pub download_dir: String,
    /// 整理目标目录（番剧、音乐、视频都放在这下面）
    pub organize_dir: String,
    /// 同时下载数，1 到 8
    pub concurrency: usize,
    /// HTTP 限速 KB/s，0 表示不限
    pub speed_kbps: u64,
    /// stop：下完即停；ratio：做种到分享率 seed_ratio 后停止
    pub seed_policy: String,
    pub seed_ratio: f64,
    /// B 站清晰度上限（画面高度，如 1080），0 表示账号能拿到的最高
    pub bili_quality: u32,
    /// B 站：下载弹幕（转成 ASS，放在视频旁边）
    pub bili_danmaku: bool,
    /// B 站：下载 CC / AI 字幕（需要登录）
    pub bili_subs: bool,
}

impl Default for FetchSettings {
    fn default() -> Self {
        let mut s = FetchSettings {
            download_dir: String::new(),
            organize_dir: String::new(),
            concurrency: 3,
            speed_kbps: 0,
            seed_policy: "stop".into(),
            seed_ratio: 1.0,
            bili_quality: 0,
            bili_danmaku: true,
            bili_subs: true,
        };
        s.normalize();
        s
    }
}

impl FetchSettings {
    /// 补全空路径、限制数值范围
    pub fn normalize(&mut self) {
        if self.download_dir.trim().is_empty() {
            self.download_dir = default_download_dir().to_string_lossy().into_owned();
        }
        if self.organize_dir.trim().is_empty() {
            self.organize_dir = default_organize_dir().to_string_lossy().into_owned();
        }
        self.concurrency = self.concurrency.clamp(1, 8);
        if self.seed_policy != "ratio" {
            self.seed_policy = "stop".into();
        }
        if !(self.seed_ratio.is_finite() && self.seed_ratio > 0.0) {
            self.seed_ratio = 1.0;
        }
        self.seed_ratio = self.seed_ratio.clamp(0.1, 20.0);
        if !matches!(self.bili_quality, 0 | 360 | 480 | 720 | 1080 | 1440 | 2160) {
            self.bili_quality = 0;
        }
    }
}

fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("极光播放器")
}

fn default_organize_dir() -> PathBuf {
    dirs::video_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("极光媒体库")
}

pub fn load_settings(c: &Connection) -> FetchSettings {
    let mut s: FetchSettings = library::get_setting(c, "fetch")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    s.normalize();
    s
}

// ---------- 来源表 ----------

pub fn list_sources(c: &Connection, only_enabled: bool) -> Result<Vec<SourceRow>> {
    let sql = if only_enabled {
        "SELECT id,kind,name,url,rule_json,enabled,created_at FROM fetch_sources WHERE enabled=1 ORDER BY id"
    } else {
        "SELECT id,kind,name,url,rule_json,enabled,created_at FROM fetch_sources ORDER BY id"
    };
    let mut st = c.prepare(sql)?;
    let rows = st
        .query_map([], |r| {
            Ok(SourceRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                name: r.get(2)?,
                url: r.get(3)?,
                rule_json: r.get(4)?,
                enabled: r.get::<_, i64>(5)? != 0,
                created_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn load_source(c: &Connection, id: i64) -> Result<Option<SourceRow>> {
    Ok(list_sources(c, false)?.into_iter().find(|r| r.id == id))
}

// ---------- 搜索 ----------

/// 从发布名里识别作品、季、集、字幕组、清晰度
pub fn enrich(h: &mut Hit) {
    let clean = h.title.replace(['/', '\\'], " ");
    let info = crate::parse::parse_episode(Path::new(&clean));
    h.work = if info.title.trim().is_empty() { clean.trim().to_string() } else { info.title };
    h.season = info.season;
    h.episode = info.episode;
    h.group = info.group;
    h.resolution = info.resolution;
    h.special = info.special;
}

/// 追更记“已见”用的键：下载源结果用“下载源 id + 条目 id”（地址里带着整条结果，封面、档位一变地址就变），
/// 其他来源用下载地址
fn seen_key(h: &Hit) -> String {
    if h.kind == "provider" {
        if let Some((pid, item, _)) = provider::parse_task_url(&h.url) {
            if !item.id.is_empty() {
                return format!("{}{pid}/{}", provider::SCHEME, item.id);
            }
        }
    }
    h.url.clone()
}

fn group_key(work: &str) -> String {
    work.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect()
}

/// 同时搜索所有启用的来源（only_source 指定时只搜那一个）
pub async fn search(app: &AppHandle, query: &str, media_kind: &str, only_source: Option<i64>) -> Result<SearchOut, String> {
    search_in(app, query, media_kind, only_source.map(|s| vec![s]).as_deref()).await
}

/// 只搜一个来源
async fn search_one(app: &AppHandle, row: &SourceRow, q: &str, media_kind: &str) -> (Result<Vec<Hit>, String>, bool) {
    if !row.media_kinds().is_empty() && !row.media_kinds().iter().any(|k| k == media_kind) {
        return (Ok(vec![]), true);
    }
    let r = match row.kind.as_str() {
        "rss" => source::search_rss(row, q).await,
        "dir" => source::search_dir(row, q, media_kind).await,
        "ytdlp" => source::search_ytdlp(row, q).await,
        "web" => web::search(app, row, q, media_kind).await,
        "provider" => provider::search(row, q, media_kind).await,
        other => Err(format!("未知来源类型：{other}")),
    };
    (r, false)
}

/// 在指定的来源里搜（`None` 为全部启用的来源），各来源同时搜
pub async fn search_in(app: &AppHandle, query: &str, media_kind: &str, only: Option<&[i64]>) -> Result<SearchOut, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Err("请输入要搜索的名字".into());
    }
    let rows = with_db(app, |c| list_sources(c, true)).map_err(e)?;
    let mut out = SearchOut::default();
    if rows.is_empty() {
        out.notes.push("还没有添加任何来源".into());
    }
    let rows: Vec<SourceRow> = rows.into_iter().filter(|r| only.map_or(true, |s| s.contains(&r.id))).collect();
    let runs = rows.iter().map(|row| {
        let q = q.clone();
        async move {
            let t = std::time::Instant::now();
            let (r, skipped) = search_one(app, row, &q, media_kind).await;
            (r, skipped, t.elapsed().as_millis() as u64)
        }
    });
    let results = futures_util::future::join_all(runs).await;
    for (row, (res, skipped, ms)) in rows.iter().zip(results) {
        let mut stat = SourceStat {
            source_id: row.id,
            name: row.name.clone(),
            ms,
            skipped,
            ..Default::default()
        };
        match res {
            Ok(mut hits) => {
                for h in hits.iter_mut() {
                    h.source_id = row.id;
                    h.source_name = row.name.clone();
                    // 下载源自己给出了作品名（专辑），不用再从标题里猜集数
                    if h.kind != "provider" {
                        enrich(h);
                    }
                }
                stat.count = hits.len();
                out.hits.append(&mut hits);
            }
            Err(msg) => {
                let (code, text) = provider::split_code(&msg);
                stat.code = code.unwrap_or("").to_string();
                stat.error = text.to_string();
                out.notes.push(format!("{}：{}", row.name, text));
            }
        }
        out.stats.push(stat);
    }
    let mut seen = HashSet::new();
    out.hits.retain(|h| seen.insert(h.url.clone()));
    let mut order: Vec<String> = vec![];
    for h in &out.hits {
        let k = group_key(&h.work);
        if !order.contains(&k) {
            order.push(k);
        }
    }
    for k in order {
        let indices: Vec<usize> = out.hits.iter().enumerate().filter(|(_, h)| group_key(&h.work) == k).map(|(i, _)| i).collect();
        let work = out.hits[indices[0]].work.clone();
        out.groups.push(Group { work, indices });
    }
    Ok(out)
}

/// AI 按偏好挑推荐版本（只返回序号和理由，不改动结果）
pub async fn ai_pick(app: &AppHandle, hits: &[Hit], prefs: &str) -> Result<Vec<Pick>, String> {
    if hits.is_empty() {
        return Ok(vec![]);
    }
    let list: String = hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "{i}. {} | 作品:{} | 集:{} | 字幕组:{} | 清晰度:{} | 大小:{} | 来源:{} | 类型:{}\n",
                h.title,
                h.work,
                h.episode.map(|e| e.to_string()).unwrap_or_else(|| "未知".into()),
                h.group.clone().unwrap_or_else(|| "无".into()),
                h.resolution.clone().unwrap_or_else(|| "未知".into()),
                h.size.map(|s| format!("{s} 字节")).unwrap_or_else(|| "未知".into()),
                h.source_name,
                h.kind
            )
        })
        .collect();
    let system = "你是番剧和音乐资源的挑选助手。根据用户偏好，从候选版本中挑出最合适的几个：同一作品只推荐一个主推版本，集数有缺口时可以多挑几个补齐，避免重复。只输出 JSON：{\"picks\":[{\"index\":序号,\"reason\":\"一句话理由\"}]}";
    let user = format!(
        "用户偏好：{}\n候选版本：\n{}",
        if prefs.trim().is_empty() { "无特别偏好" } else { prefs.trim() },
        list
    );
    let reply = ai::chat_simple(app, system, &user, true).await.map_err(e)?;
    let v = ai::extract_json(&reply).map_err(e)?;
    let picks = v["picks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let i = p["index"].as_u64()? as usize;
                    (i < hits.len()).then(|| Pick {
                        index: i,
                        reason: p["reason"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(picks)
}

// ---------- 任务表 ----------

const TASK_COLS: &str = "id,source_id,title,kind,url,status,progress,speed,size,save_path,error,created_at,finished_at,media_kind";

fn task_row(r: &rusqlite::Row) -> rusqlite::Result<TaskView> {
    Ok(TaskView {
        id: r.get(0)?,
        source_id: r.get(1)?,
        title: r.get(2)?,
        kind: r.get(3)?,
        url: r.get(4)?,
        status: r.get(5)?,
        progress: r.get(6)?,
        speed: r.get(7)?,
        size: r.get(8)?,
        save_path: r.get(9)?,
        error: r.get(10)?,
        created_at: r.get(11)?,
        finished_at: r.get(12)?,
        media_kind: r.get(13)?,
    })
}

pub fn load_task(c: &Connection, id: i64) -> Result<Option<TaskView>> {
    Ok(c.query_row(&format!("SELECT {TASK_COLS} FROM fetch_tasks WHERE id=?1"), [id], task_row)
        .optional()?)
}

/// 推送单个任务的最新状态给界面
pub(crate) fn emit_task(app: &AppHandle, id: i64) {
    if let Ok(Some(t)) = with_db(app, |c| load_task(c, id)) {
        let _ = app.emit("fetch://progress", t);
    }
}

pub(crate) fn set_task_status(app: &AppHandle, id: i64, status: &str, error: &str) {
    let _ = with_db(app, |c| {
        c.execute(
            "UPDATE fetch_tasks SET status=?2, error=?3, finished_at=CASE WHEN ?2 IN ('done','manual','failed','canceled') THEN ?4 ELSE NULL END WHERE id=?1",
            params![id, status, error, db::now()],
        )?;
        Ok(())
    });
    emit_task(app, id);
}

pub(crate) fn set_task_progress(app: &AppHandle, id: i64, progress: f64, speed: i64, size: i64) {
    let _ = with_db(app, |c| {
        c.execute(
            "UPDATE fetch_tasks SET progress=?2, speed=?3, size=CASE WHEN ?4>0 THEN ?4 ELSE size END WHERE id=?1",
            params![id, progress, speed, size],
        )?;
        Ok(())
    });
    emit_task(app, id);
}

pub(crate) fn insert_task(c: &Connection, source_id: Option<i64>, title: &str, kind: &str, url: &str, media_kind: &str, size: i64) -> Result<i64> {
    c.execute(
        "INSERT INTO fetch_tasks(source_id,title,kind,url,status,size,created_at,media_kind) VALUES(?1,?2,?3,?4,'queued',?5,?6,?7)",
        params![source_id, title, kind, url, size, db::now(), media_kind],
    )?;
    Ok(c.last_insert_rowid())
}

/// 只删下载目录里面的文件，目录本身和下载目录不动
pub(crate) fn remove_task_files(path: &str, download_dir: &Path) {
    let p = PathBuf::from(path);
    if p.as_os_str().is_empty() || !p.starts_with(download_dir) || p == download_dir {
        return;
    }
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(&p);
    } else {
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{}.part", p.display()));
    }
}

// ---------- 来源管理命令 ----------

#[tauri::command]
pub fn fetch_sources(st: State<AppState>) -> Result<Vec<SourceView>, String> {
    let c = st.db.lock();
    Ok(list_sources(&c, false).map_err(e)?.iter().map(|r| r.view()).collect())
}

fn default_name(url: &str) -> String {
    if let Ok(u) = url::Url::parse(url) {
        if let Some(h) = u.host_str() {
            return h.to_string();
        }
    }
    Path::new(url)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| url.to_string())
}

/// 自动判断来源类型：本地路径 / WebDAV → dir；能解析的订阅 → rss；yt-dlp 能识别 → ytdlp；其他 → web（AI 解析）
async fn detect_kind(url: &str) -> String {
    if let Some(k) = source::guess_kind(url) {
        if k != "rss" {
            return k.to_string();
        }
        if let Ok(b) = source::http_bytes(url).await {
            if source::parse_feed(&b).is_ok() {
                return "rss".into();
            }
        }
    }
    if source::ytdlp_path().is_some() && source::probe_ytdlp(url).await {
        return "ytdlp".into();
    }
    "web".into()
}

#[tauri::command]
pub async fn fetch_add_source(app: AppHandle, kind: String, url: String, name: Option<String>, search_template: Option<String>) -> Result<SourceView, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("请先粘贴网站地址、订阅链接或文件夹路径".into());
    }
    let mut kind = kind.trim().to_lowercase();
    if (kind.is_empty() || kind == "auto") && external::looks_like_program(&url) {
        kind = "exe".into();
    }
    if kind == "exe" {
        return add_external(&app, &url, name).await;
    }
    if kind.is_empty() || kind == "auto" {
        kind = detect_kind(&url).await;
    }
    // 网页来源：url 列保存搜索网址模板（含 {q} 的地址）；没填模板就用原地址
    let template = search_template.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let stored_url = if kind == "web" {
        template.unwrap_or_else(|| url.clone())
    } else {
        url.clone()
    };
    let display = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_name(&url));

    let rule_json = match kind.as_str() {
        "dir" => {
            if source::is_local_path(&url) {
                if !Path::new(&url).is_dir() {
                    return Err(format!("找不到这个文件夹：{url}"));
                }
            } else {
                source::check_webdav(&url).await?;
            }
            String::new()
        }
        "rss" => {
            let bytes = source::http_bytes(&url).await?;
            source::parse_feed(&bytes)?;
            String::new()
        }
        "ytdlp" => {
            if source::ytdlp_path().is_none() {
                return Err(source::YTDLP_MISSING.into());
            }
            let v = source::list_ytdlp(&url).await?;
            if v.is_empty() {
                return Err("yt-dlp 没有在这个地址下找到可下载的内容".into());
            }
            String::new()
        }
        "web" => web::add_rule(&app, &stored_url).await?,
        "provider" => {
            let id = provider::provider_id(&url).ok_or("下载源地址应为 provider://<id>")?;
            if provider::get(id).is_none() {
                return Err(format!("这个版本的软件里没有下载源“{id}”"));
            }
            String::new()
        }
        other => return Err(format!("未知来源类型：{other}")),
    };

    let id = with_db(&app, |c| {
        c.execute(
            "INSERT INTO fetch_sources(kind,name,url,rule_json,created_at,enabled) VALUES(?1,?2,?3,?4,?5,1)",
            params![kind, display, stored_url, rule_json, db::now()],
        )?;
        Ok(c.last_insert_rowid())
    })
    .map_err(e)?;
    let row = with_db(&app, |c| load_source(c, id)).map_err(e)?.ok_or("保存失败")?;
    Ok(row.view())
}

/// 添加外部程序下载源：运行 `程序 info` 取得信息，注册，存进来源表（kind = provider）
async fn add_external(app: &AppHandle, path: &str, name: Option<String>) -> Result<SourceView, String> {
    let exe = PathBuf::from(path.trim().trim_matches('"'));
    if !exe.is_file() {
        return Err(format!("找不到这个程序：{}", exe.display()));
    }
    let exe = std::path::absolute(&exe).unwrap_or(exe);
    let mut info = external::probe(&exe).await?;
    if let Some(n) = name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        info.name = n;
    }
    let url = provider::source_url(&info.id);
    let exists = with_db(app, |c| list_sources(c, false)).map_err(e)?.into_iter().any(|r| r.url == url);
    if exists {
        return Err(format!("已经添加过下载源“{}”（{}），要换程序请先删掉旧的", info.name, info.id));
    }
    if provider::get(&info.id).is_some() {
        return Err(format!("下载源 id“{}”和已有的下载源重名，请在程序的 info 里换一个 id", info.id));
    }
    let stored = external::Stored {
        exe: exe.to_string_lossy().into_owned(),
        info: info.clone(),
    };
    let rule_json = serde_json::to_string(&stored).map_err(e)?;
    external::register(&stored);
    let id = with_db(app, |c| {
        c.execute(
            "INSERT INTO fetch_sources(kind,name,url,rule_json,created_at,enabled) VALUES('provider',?1,?2,?3,?4,1)",
            params![info.name, url, rule_json, db::now()],
        )?;
        Ok(c.last_insert_rowid())
    })
    .map_err(e)?;
    let row = with_db(app, |c| load_source(c, id)).map_err(e)?.ok_or("保存失败")?;
    Ok(row.view())
}

#[tauri::command]
pub async fn fetch_rebuild_rule(app: AppHandle, id: i64) -> Result<SourceView, String> {
    let row = with_db(&app, |c| load_source(c, id)).map_err(e)?.ok_or("找不到这个来源")?;
    if row.kind != "web" {
        return Err("只有网页来源需要生成提取规则".into());
    }
    let stored = web::StoredRule::parse(&row.rule_json);
    let template = if stored.template.is_empty() {
        row.url.clone()
    } else {
        stored.template.clone()
    };
    let rule_json = web::add_rule(&app, &template).await?;
    with_db(&app, |c| {
        c.execute("UPDATE fetch_sources SET rule_json=?2 WHERE id=?1", params![id, rule_json])?;
        Ok(())
    })
    .map_err(e)?;
    let row = with_db(&app, |c| load_source(c, id)).map_err(e)?.ok_or("找不到这个来源")?;
    Ok(row.view())
}

#[tauri::command]
pub fn fetch_remove_source(st: State<AppState>, id: i64) -> Result<(), String> {
    let c = st.db.lock();
    if let Ok(Some(row)) = load_source(&c, id) {
        if external::Stored::parse(&row.rule_json).is_some() {
            if let Some(pid) = provider::provider_id(&row.url) {
                provider::unregister(pid);
            }
        }
    }
    c.execute("DELETE FROM fetch_subs WHERE source_id=?1", [id]).map_err(e)?;
    c.execute("DELETE FROM fetch_sources WHERE id=?1", [id]).map_err(e)?;
    Ok(())
}

// ---------- 搜索 / 挑选命令 ----------

#[tauri::command]
pub async fn fetch_search(app: AppHandle, query: String, media_kind: String, sources: Option<Vec<i64>>) -> Result<SearchOut, String> {
    let mk = if media_kind.trim().is_empty() { "anime".to_string() } else { media_kind };
    let only = sources.filter(|s| !s.is_empty());
    search_in(&app, &query, &mk, only.as_deref()).await
}

/// 启用 / 停用一个来源（停用的来源不参与搜索和追更）
#[tauri::command]
pub fn fetch_set_source_enabled(app: AppHandle, id: i64, enabled: bool) -> Result<(), String> {
    with_db(&app, |c| {
        c.execute("UPDATE fetch_sources SET enabled=?2 WHERE id=?1", params![id, enabled as i64])?;
        Ok(())
    })
    .map_err(e)
}

#[tauri::command]
pub async fn fetch_ai_pick(app: AppHandle, results: Vec<Hit>, prefs: String) -> Result<Vec<Pick>, String> {
    ai_pick(&app, &results, &prefs).await
}

// ---------- 下载命令 ----------

#[tauri::command]
pub fn fetch_download(app: AppHandle, items: Vec<DlItem>) -> Result<Vec<i64>, String> {
    if items.is_empty() {
        return Err("没有选中要下载的条目".into());
    }
    let mut ids = vec![];
    with_db(&app, |c| {
        for it in &items {
            let url = it.url.trim();
            if url.is_empty() {
                continue;
            }
            let kind = if it.kind.trim().is_empty() {
                source::kind_of_url(url).to_string()
            } else {
                it.kind.clone()
            };
            let mk = if it.media_kind.trim().is_empty() {
                "video".to_string()
            } else {
                it.media_kind.clone()
            };
            let title = if it.title.trim().is_empty() { url.to_string() } else { it.title.clone() };
            let id = insert_task(c, it.source_id, &title, &kind, url, &mk, it.size.unwrap_or(0))?;
            if let Some(files) = it.files.as_ref().filter(|f| !f.is_empty()) {
                c.execute("UPDATE fetch_tasks SET files=?2 WHERE id=?1", params![id, serde_json::to_string(files)?])?;
            }
            ids.push(id);
        }
        Ok(())
    })
    .map_err(e)?;
    if ids.is_empty() {
        return Err("没有有效的下载地址".into());
    }
    download::pump(&app);
    Ok(ids)
}

/// 解析磁力链接 / 种子，列出里面的文件给用户勾选（不开始下载）
#[tauri::command]
pub async fn fetch_inspect_torrent(app: AppHandle, url: String) -> Result<torrent::TorrentInfo, String> {
    torrent::inspect(&app, &url).await.map_err(|err| format!("{err:#}"))
}

#[tauri::command]
pub fn fetch_tasks(st: State<AppState>) -> Result<Vec<TaskView>, String> {
    let c = st.db.lock();
    let mut s = c
        .prepare(&format!("SELECT {TASK_COLS} FROM fetch_tasks ORDER BY id DESC LIMIT 500"))
        .map_err(e)?;
    let rows = s.query_map([], task_row).map_err(e)?.collect::<rusqlite::Result<Vec<_>>>().map_err(e)?;
    Ok(rows)
}

#[tauri::command]
pub fn fetch_pause(app: AppHandle, id: i64) -> Result<(), String> {
    download::pause(&app, id);
    Ok(())
}

#[tauri::command]
pub fn fetch_resume(app: AppHandle, id: i64) -> Result<(), String> {
    download::resume(&app, id);
    Ok(())
}

#[tauri::command]
pub fn fetch_cancel(app: AppHandle, id: i64, delete_files: bool) -> Result<(), String> {
    download::cancel(&app, id, delete_files);
    Ok(())
}

#[tauri::command]
pub fn fetch_retry(app: AppHandle, id: i64) -> Result<(), String> {
    download::resume(&app, id);
    Ok(())
}

#[tauri::command]
pub fn fetch_get_settings(st: State<AppState>) -> Result<FetchSettings, String> {
    let c = st.db.lock();
    Ok(load_settings(&c))
}

#[tauri::command]
pub fn fetch_set_settings(app: AppHandle, value: FetchSettings) -> Result<FetchSettings, String> {
    let mut s = value;
    s.normalize();
    with_db(&app, |c| {
        library::set_setting(c, "fetch", &serde_json::to_value(&s)?)?;
        Ok(())
    })
    .map_err(e)?;
    download::pump(&app);
    Ok(s)
}

// ---------- 追更 ----------

#[tauri::command]
pub async fn fetch_subscribe(app: AppHandle, source_id: i64, query: String, media_kind: String) -> Result<i64, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Err("请输入要追更的作品名".into());
    }
    let mk = if media_kind.trim().is_empty() { "anime".to_string() } else { media_kind };
    // 订阅时把当前已有的结果记为“已见”，之后只下载新出现的集
    let out = search(&app, &q, &mk, Some(source_id)).await?;
    let seen: Vec<String> = out.hits.iter().map(seen_key).collect();
    let id = with_db(&app, |c| {
        c.execute(
            "INSERT INTO fetch_subs(source_id,query,media_kind,last_check,seen_json) VALUES(?1,?2,?3,?4,?5)",
            params![source_id, q, mk, db::now(), serde_json::to_string(&seen)?],
        )?;
        Ok(c.last_insert_rowid())
    })
    .map_err(e)?;
    Ok(id)
}

#[tauri::command]
pub fn fetch_unsubscribe(st: State<AppState>, id: i64) -> Result<(), String> {
    st.db.lock().execute("DELETE FROM fetch_subs WHERE id=?1", [id]).map_err(e)?;
    Ok(())
}

#[tauri::command]
pub fn fetch_subs(st: State<AppState>) -> Result<Vec<SubView>, String> {
    let c = st.db.lock();
    let mut s = c
        .prepare(
            "SELECT s.id, s.source_id, COALESCE(src.name,''), s.query, s.media_kind, s.last_check, s.seen_json
             FROM fetch_subs s LEFT JOIN fetch_sources src ON src.id = s.source_id ORDER BY s.id",
        )
        .map_err(e)?;
    let rows = s
        .query_map([], |r| {
            let seen: String = r.get(6)?;
            Ok(SubView {
                id: r.get(0)?,
                source_id: r.get(1)?,
                source_name: r.get(2)?,
                query: r.get(3)?,
                media_kind: r.get(4)?,
                last_check: r.get(5)?,
                seen: serde_json::from_str::<Vec<String>>(&seen).map(|v| v.len()).unwrap_or(0),
            })
        })
        .map_err(e)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(e)?;
    Ok(rows)
}

/// 检查所有订阅：新出现的集自动下载，并推送 fetch://new
pub async fn check_subs(app: &AppHandle) {
    type SubRow = (i64, i64, String, String, Vec<String>);
    let subs: Vec<SubRow> = with_db(app, |c| {
        let mut st = c.prepare("SELECT id,source_id,query,media_kind,seen_json FROM fetch_subs")?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(a, b, q, m, s)| (a, b, q, m, serde_json::from_str(&s).unwrap_or_default()))
            .collect())
    })
    .unwrap_or_default();
    for (sub_id, source_id, query, media_kind, mut seen) in subs {
        let out = match search(app, &query, &media_kind, Some(source_id)).await {
            Ok(o) => o,
            Err(_) => continue,
        };
        let mut fresh = vec![];
        for h in out.hits {
            let key = seen_key(&h);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            if h.episode.is_some() {
                fresh.push(h);
            }
        }
        if seen.len() > 3000 {
            let drop = seen.len() - 3000;
            seen.drain(0..drop);
        }
        let titles: Vec<String> = fresh.iter().map(|h| h.title.clone()).collect();
        let _ = with_db(app, |c| {
            for h in &fresh {
                insert_task(c, Some(source_id), &h.title, &h.kind, &h.url, &media_kind, h.size.unwrap_or(0))?;
            }
            c.execute(
                "UPDATE fetch_subs SET last_check=?2, seen_json=?3 WHERE id=?1",
                params![sub_id, db::now(), serde_json::to_string(&seen)?],
            )?;
            Ok(())
        });
        if !fresh.is_empty() {
            download::pump(app);
            let _ = app.emit("fetch://new", json!({ "subId": sub_id, "query": query, "titles": titles }));
        }
    }
}

// ---------- B 站与 yt-dlp ----------

#[tauri::command]
pub async fn fetch_bili_account() -> Result<bili::Account, String> {
    bili::account().await.map_err(e)
}

#[tauri::command]
pub async fn fetch_bili_qr_start() -> Result<bili::Qr, String> {
    bili::qr_start().await.map_err(e)
}

/// 返回 waiting / scanned / expired / ok
#[tauri::command]
pub async fn fetch_bili_qr_poll(key: String) -> Result<String, String> {
    bili::qr_poll(&key).await.map(|s| s.to_string()).map_err(e)
}

#[tauri::command]
pub fn fetch_bili_logout() {
    bili::clear_cookies();
}

#[tauri::command]
pub async fn fetch_bili_parse(app: AppHandle, url: String) -> Result<bili::Parsed, String> {
    let dl = with_db(&app, |c| Ok(load_settings(c).download_dir)).map_err(e)?;
    bili::parse(&url, Path::new(&dl)).await.map_err(|err| bili::friendly_error(&format!("{err:#}")))
}

#[tauri::command]
pub async fn fetch_ytdlp_status() -> ytdlp::Status {
    ytdlp::status().await
}

/// 下载或更新 yt-dlp（官方发布页，校验 SHA-256）
#[tauri::command]
pub async fn fetch_ytdlp_install() -> Result<ytdlp::Status, String> {
    ytdlp::install().await.map_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrich_fills_work_and_episode() {
        let mut h = Hit {
            title: "[字幕组] 星屑邮局 - 03 [1080p]".into(),
            ..Default::default()
        };
        enrich(&mut h);
        assert_eq!(h.work, "星屑邮局");
        assert_eq!(h.episode, Some(3.0));
        assert_eq!(h.group.as_deref(), Some("字幕组"));
        assert_eq!(h.resolution.as_deref(), Some("1080p"));
    }

    #[test]
    fn settings_normalize() {
        let mut s = FetchSettings {
            concurrency: 99,
            seed_policy: "xx".into(),
            seed_ratio: f64::NAN,
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.concurrency, 8);
        assert_eq!(s.seed_policy, "stop");
        assert_eq!(s.seed_ratio, 1.0);
        assert!(!s.download_dir.is_empty());
    }

    #[test]
    fn default_concurrency_is_three() {
        assert_eq!(FetchSettings::default().concurrency, 3);
    }

    #[test]
    fn remove_only_inside_download_dir() {
        let root = Path::new("D:/dl");
        assert!(Path::new("D:/dl/a/b.mkv").starts_with(root));
        assert!(!Path::new("D:/media/b.mkv").starts_with(root));
    }

    #[test]
    fn seen_key_uses_provider_item_id() {
        let it = provider::Item {
            id: "ep-7".into(),
            title: "x".into(),
            cover: "https://a/1.jpg?t=1".into(),
            ..Default::default()
        };
        let mut h = provider::to_hit("demo", &it);
        assert_eq!(seen_key(&h), "provider://demo/ep-7");
        // 封面地址变了，仍算同一集
        let it2 = provider::Item {
            cover: "https://a/1.jpg?t=2".into(),
            ..it.clone()
        };
        assert_eq!(seen_key(&provider::to_hit("demo", &it2)), "provider://demo/ep-7");
        h.kind = "magnet".into();
        h.url = "magnet:?xt=1".into();
        assert_eq!(seen_key(&h), "magnet:?xt=1");
    }

    #[test]
    fn group_key_ignores_spaces_and_case() {
        assert_eq!(group_key("Star Post"), group_key("starpost"));
    }

    #[test]
    fn init_is_idempotent() {
        let c = db::open_memory().unwrap();
        init(&c).unwrap();
        init(&c).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM fetch_tasks", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}

/// 从下载列表里删掉一条已结束的记录（已整理入库的文件不受影响）
#[tauri::command]
pub fn fetch_remove_task(app: AppHandle, id: i64) -> Result<usize, String> {
    with_db(&app, |c| {
        Ok(c.execute("DELETE FROM fetch_tasks WHERE id=?1 AND status NOT IN ('queued','downloading')", [id])?)
    })
    .map_err(e)
}

/// 已接入的下载源（软件默认没有，见 `fetch/provider.rs`）
#[tauri::command]
pub fn fetch_providers() -> Vec<provider::ProviderInfo> {
    provider::list()
}

// ---------- 网站登录（软件内浏览器窗口） ----------

/// 打开登录窗口，返回网站名（Windows 上同步命令里建窗口会卡死，所以是 async）
#[tauri::command]
pub async fn fetch_login_open(app: AppHandle, url: String) -> Result<String, String> {
    login::open(&app, &url).map_err(e)
}

/// 读取登录窗口的 Cookie 并保存，然后关闭窗口
#[tauri::command]
pub async fn fetch_login_finish(app: AppHandle) -> Result<login::LoginView, String> {
    login::finish(&app).await.map_err(e)
}

#[tauri::command]
pub fn fetch_logins() -> Vec<login::LoginView> {
    login::list()
}

#[tauri::command]
pub fn fetch_login_clear(site: String) -> Result<(), String> {
    login::clear(&site).map_err(e)
}
