pub mod comic;
pub mod commands;
pub mod db;
pub mod indexer;
pub mod library;
pub mod metadata;
pub mod parse;
pub mod reader;
pub mod scanner;

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub db_path: PathBuf,
    pub dirs: Arc<indexer::Dirs>,
    pub scan: Arc<scanner::ScanState>,
    pub watcher: Mutex<Option<scanner::Watcher>>,
}

fn init_state(app: &tauri::AppHandle) -> anyhow::Result<AppState> {
    // MP_DATA_DIR：测试时把数据库和缓存放到指定目录，不碰正式数据
    let (data, cache) = match std::env::var_os("MP_DATA_DIR") {
        Some(d) => (PathBuf::from(&d), PathBuf::from(&d).join("cache")),
        None => (app.path().app_data_dir()?, app.path().app_local_data_dir()?.join("cache")),
    };
    let covers = cache.join("covers");
    let thumbs = cache.join("thumbs");
    for d in [&data, &covers, &thumbs] {
        std::fs::create_dir_all(d)?;
    }
    let db_path = data.join("library.db");
    let conn = db::open(&db_path)?;
    Ok(AppState {
        db: Mutex::new(conn),
        db_path,
        dirs: Arc::new(indexer::Dirs { data, covers, thumbs }),
        scan: Arc::new(scanner::ScanState::default()),
        watcher: Mutex::new(None),
    })
}

/// `mpc://localhost/<file_id>/<page>`：漫画页面（压缩包里的图片不解压到磁盘）
fn comic_protocol(app: &tauri::AppHandle, req: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let resp = |code: u16, mime: &str, body: Vec<u8>| {
        tauri::http::Response::builder()
            .status(code)
            .header("Content-Type", mime)
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "max-age=3600")
            .body(body)
            .unwrap()
    };
    let path = req.uri().path().trim_matches('/').to_string();
    let mut it = path.split('/');
    let (Some(id), Some(page)) = (it.next().and_then(|s| s.parse::<i64>().ok()), it.next().and_then(|s| s.parse::<usize>().ok())) else {
        return resp(400, "text/plain", b"bad request".to_vec());
    };
    let state = app.state::<AppState>();
    let Ok(file) = library::file_path(&state.db.lock(), id) else {
        return resp(404, "text/plain", b"not found".to_vec());
    };
    match comic::read_page(std::path::Path::new(&file), page) {
        Ok((bytes, mime)) => resp(200, &mime, bytes),
        Err(e) => resp(404, "text/plain", e.to_string().into_bytes()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_uri_scheme_protocol("mpc", |ctx, req| comic_protocol(ctx.app_handle(), &req))
        .setup(|app| {
            let state = init_state(app.handle()).map_err(|e| format!("初始化失败：{e}"))?;
            app.manage(state);
            commands::restart_watcher(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::library_stats,
            commands::list_tracks,
            commands::get_track,
            commands::list_videos,
            commands::list_series,
            commands::series_episodes,
            commands::get_video,
            commands::save_video_progress,
            commands::set_watched,
            commands::list_books,
            commands::open_book,
            commands::save_book_progress,
            commands::list_comics,
            commands::save_comic_progress,
            commands::lyrics,
            commands::toggle_favorite,
            commands::record_play,
            commands::list_playlists,
            commands::create_playlist,
            commands::rename_playlist,
            commands::delete_playlist,
            commands::playlist_add,
            commands::playlist_remove,
            commands::playlist_reorder,
            commands::playlist_items,
            commands::export_playlist,
            commands::import_playlist,
            commands::search,
            commands::get_setting,
            commands::set_setting,
            commands::list_roots,
            commands::add_root,
            commands::remove_root,
            commands::list_drives,
            commands::start_scan,
            commands::cancel_scan,
            commands::scan_status,
            commands::add_files,
            commands::show_in_folder,
            commands::cover_dir,
        ])
        .run(tauri::generate_context!())
        .expect("启动失败");
}
