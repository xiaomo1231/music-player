//! 前端可调用的 Tauri 命令
use crate::{indexer, library, metadata, parse, scanner, AppState};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub engine: &'static str,
    pub data_dir: String,
    pub cover_dir: String,
}

#[tauri::command]
pub fn app_info(app: AppHandle, st: State<AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        engine: if cfg!(feature = "mpv") { "mpv" } else { "web" },
        data_dir: st.dirs.data.to_string_lossy().into(),
        cover_dir: st.dirs.covers.to_string_lossy().into(),
    }
}

#[tauri::command]
pub fn cover_dir(st: State<AppState>) -> String {
    st.dirs.covers.to_string_lossy().into()
}

#[tauri::command]
pub fn library_stats(st: State<AppState>) -> R<library::Stats> {
    library::stats(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub async fn list_tracks(st: State<'_, AppState>) -> R<Vec<library::Track>> {
    library::tracks(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub fn get_track(st: State<AppState>, id: i64) -> R<Option<library::Track>> {
    library::track(&st.db.lock(), id).map_err(e)
}

#[tauri::command]
pub async fn list_videos(st: State<'_, AppState>) -> R<Vec<library::Video>> {
    library::videos(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub async fn list_series(st: State<'_, AppState>, kind: String) -> R<Vec<library::Series>> {
    library::series(&st.db.lock(), &kind).map_err(e)
}

#[tauri::command]
pub fn series_episodes(st: State<AppState>, id: i64) -> R<Vec<library::Video>> {
    library::episodes(&st.db.lock(), id).map_err(e)
}

#[tauri::command]
pub fn get_video(st: State<AppState>, id: i64) -> R<Option<library::Video>> {
    library::video(&st.db.lock(), id).map_err(e)
}

#[tauri::command]
pub fn save_video_progress(st: State<AppState>, id: i64, position: f64, duration: f64) -> R<()> {
    library::save_video_progress(&st.db.lock(), id, position, duration).map_err(e)
}

#[tauri::command]
pub fn set_watched(st: State<AppState>, ids: Vec<i64>, watched: bool) -> R<()> {
    library::set_watched(&st.db.lock(), &ids, watched).map_err(e)
}

#[tauri::command]
pub async fn list_books(st: State<'_, AppState>) -> R<Vec<library::BookItem>> {
    library::books(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub async fn open_book(st: State<'_, AppState>, id: i64) -> R<crate::em::clean::Opened> {
    let path = library::file_path(&st.db.lock(), id).map_err(e)?;
    let db = st.db_path.clone();
    // 套用小说排版规则（.em）；单独开连接，不占主连接的锁
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<crate::em::clean::Opened> {
        let conn = rusqlite::Connection::open(&db)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        crate::em::clean::open_book(&conn, id, &path)
    })
    .await
    .map_err(e)?
    .map_err(e)
}

#[tauri::command]
pub fn save_book_progress(st: State<AppState>, id: i64, chapter: i64, offset: f64, chapters: i64) -> R<()> {
    library::save_book_progress(&st.db.lock(), id, chapter, offset, chapters).map_err(e)
}

#[tauri::command]
pub async fn list_comics(st: State<'_, AppState>) -> R<Vec<library::ComicItem>> {
    library::comics(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub fn save_comic_progress(st: State<AppState>, id: i64, page: i64) -> R<()> {
    library::save_comic_progress(&st.db.lock(), id, page).map_err(e)
}

#[tauri::command]
pub async fn lyrics(path: String) -> R<Vec<metadata::LyricLine>> {
    tauri::async_runtime::spawn_blocking(move || metadata::lyrics_for(Path::new(&path)))
        .await
        .map_err(e)
}

#[tauri::command]
pub fn toggle_favorite(st: State<AppState>, id: i64) -> R<bool> {
    library::toggle_favorite(&st.db.lock(), id).map_err(e)
}

#[tauri::command]
pub fn record_play(st: State<AppState>, id: i64, listened: f64, duration: f64, source: Option<String>) -> R<()> {
    library::record_play(&st.db.lock(), id, listened, duration, source).map_err(e)
}

#[tauri::command]
pub fn list_playlists(st: State<AppState>) -> R<Vec<library::Playlist>> {
    library::playlists(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub fn create_playlist(st: State<AppState>, name: String) -> R<i64> {
    library::create_playlist(&st.db.lock(), &name).map_err(e)
}

#[tauri::command]
pub fn rename_playlist(st: State<AppState>, id: i64, name: String) -> R<()> {
    library::rename_playlist(&st.db.lock(), id, &name).map_err(e)
}

#[tauri::command]
pub fn delete_playlist(st: State<AppState>, id: i64) -> R<()> {
    library::delete_playlist(&st.db.lock(), id).map_err(e)
}

#[tauri::command]
pub fn playlist_add(st: State<AppState>, id: i64, files: Vec<i64>) -> R<usize> {
    library::playlist_add(&st.db.lock(), id, &files).map_err(e)
}

#[tauri::command]
pub fn playlist_remove(st: State<AppState>, id: i64, files: Vec<i64>) -> R<()> {
    library::playlist_remove(&st.db.lock(), id, &files).map_err(e)
}

#[tauri::command]
pub fn playlist_reorder(st: State<AppState>, id: i64, files: Vec<i64>) -> R<()> {
    library::playlist_reorder(&st.db.lock(), id, &files).map_err(e)
}

#[tauri::command]
pub fn playlist_items(st: State<AppState>, id: i64) -> R<Vec<i64>> {
    library::playlist_items(&st.db.lock(), id).map_err(e)
}

#[tauri::command]
pub fn export_playlist(st: State<AppState>, id: i64, path: String) -> R<()> {
    library::export_m3u8(&st.db.lock(), id, Path::new(&path)).map_err(e)
}

#[tauri::command]
pub fn import_playlist(st: State<AppState>, path: String) -> R<(i64, usize)> {
    library::import_m3u(&st.db.lock(), Path::new(&path), &st.dirs).map_err(e)
}

#[tauri::command]
pub fn search(st: State<AppState>, q: String) -> R<Vec<library::Hit>> {
    library::search(&st.db.lock(), &q, 60).map_err(e)
}

#[tauri::command]
pub fn get_setting(st: State<AppState>, key: String) -> R<Option<serde_json::Value>> {
    library::get_setting(&st.db.lock(), &key).map_err(e)
}

#[tauri::command]
pub fn set_setting(app: AppHandle, st: State<AppState>, key: String, value: serde_json::Value) -> R<()> {
    library::set_setting(&st.db.lock(), &key, &value).map_err(e)?;
    if key == "watch" {
        restart_watcher(&app);
    }
    Ok(())
}

#[tauri::command]
pub fn list_roots(st: State<AppState>) -> R<Vec<library::Root>> {
    library::roots(&st.db.lock()).map_err(e)
}

#[tauri::command]
pub fn add_root(app: AppHandle, st: State<AppState>, path: String, watch: bool) -> R<()> {
    library::add_root(&st.db.lock(), &path, watch).map_err(e)?;
    restart_watcher(&app);
    Ok(())
}

#[tauri::command]
pub fn remove_root(app: AppHandle, st: State<AppState>, id: i64) -> R<()> {
    library::remove_root(&st.db.lock(), id).map_err(e)?;
    restart_watcher(&app);
    Ok(())
}

#[tauri::command]
pub fn list_drives() -> Vec<scanner::Drive> {
    scanner::drives()
}

pub fn restart_watcher(app: &AppHandle) {
    let st = app.state::<AppState>();
    let enabled = library::get_setting(&st.db.lock(), "watch")
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let roots: Vec<PathBuf> = library::roots(&st.db.lock())
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.watch)
        .map(|r| PathBuf::from(r.path))
        .collect();
    let w = if enabled {
        scanner::start_watch(app.clone(), st.db_path.clone(), st.dirs.clone(), roots).ok()
    } else {
        None
    };
    *st.watcher.lock() = w;
}

/// 扫描指定目录；不传就扫描所有已添加的目录
#[tauri::command]
pub fn start_scan(app: AppHandle, st: State<AppState>, roots: Option<Vec<String>>) -> R<()> {
    if st.scan.running.swap(true, Ordering::SeqCst) {
        return Err("扫描正在进行".into());
    }
    st.scan.cancel.store(false, Ordering::SeqCst);
    let roots: Vec<PathBuf> = match roots {
        Some(r) => r.into_iter().map(PathBuf::from).collect(),
        None => library::roots(&st.db.lock()).map_err(e)?.into_iter().map(|r| PathBuf::from(r.path)).collect(),
    };
    let (db_path, dirs, scan) = (st.db_path.clone(), st.dirs.clone(), st.scan.clone());
    std::thread::spawn(move || {
        let r = scanner::run_scan(app.clone(), db_path, dirs, roots, scan.clone());
        match r {
            Ok(p) => *scan.last.lock() = p,
            Err(err) => {
                let _ = app.emit("scan://error", err.to_string());
            }
        }
        scan.running.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_scan(st: State<AppState>) {
    st.scan.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn scan_status(st: State<AppState>) -> (bool, scanner::Progress) {
    (st.scan.running.load(Ordering::SeqCst), st.scan.last.lock().clone())
}

/// 拖进来或“打开文件”：入库并返回 (file_id, kind)
#[tauri::command]
pub async fn add_files(app: AppHandle, st: State<'_, AppState>, paths: Vec<String>) -> R<Vec<(i64, String)>> {
    let out = add_paths(&st.db.lock(), &st.dirs, paths);
    let _ = app.emit("library://changed", ());
    Ok(out)
}

/// 把文件 / 文件夹加入媒体库（add_files 和其他模块共用；不发 library://changed）
pub fn add_paths(conn: &rusqlite::Connection, dirs: &indexer::Dirs, paths: Vec<String>) -> Vec<(i64, String)> {
    let mut out = vec![];
    for p in paths {
        // 用户主动添加的文件，即使以前“从媒体库移除”过也重新入库
        let _ = conn.execute("DELETE FROM excluded WHERE path=?1 OR path LIKE ?1 || '%'", [&p]);
        let pb = PathBuf::from(&p);
        let mut files = vec![];
        if pb.is_dir() {
            scanner::enumerate(&pb, &Default::default(), |f, k| files.push((f.to_path_buf(), k)));
        } else if let Some(k) = parse::classify(&pb).or_else(|| {
            // 小 txt、普通 zip 拖进来也允许
            match parse::ext(&pb).as_str() {
                "txt" => Some("novel"),
                "zip" | "cbz" | "pdf" => Some("manga"),
                _ => None,
            }
        }) {
            files.push((pb, k));
        }
        for (f, k) in files {
            if indexer::index_file(conn, &f, k, dirs).is_ok() {
                if let Ok(id) = conn.query_row("SELECT id FROM files WHERE path=?1", [f.to_string_lossy()], |r| r.get::<_, i64>(0)) {
                    out.push((id, k.to_string()));
                }
            }
        }
    }
    out
}

#[tauri::command]
pub fn show_in_folder(path: String) -> R<()> {
    std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn().map_err(e)?;
    Ok(())
}
