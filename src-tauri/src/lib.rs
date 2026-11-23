#![allow(clippy::needless_range_loop, clippy::too_many_arguments)]

pub mod adclean;
pub mod ai;
pub mod aiclassify;
pub mod comic;
pub mod commands;
pub mod convert;
pub mod daily;
pub mod danmaku;
pub mod db;
pub mod fetch;
pub mod indexer;
pub mod item;
pub mod library;
pub mod localocr;
pub mod lyrics;
pub mod mangatr;
pub mod media;
pub mod meta;
pub mod metadata;
pub mod organize;
pub mod parse;
#[cfg(feature = "mpv")]
pub mod player;
#[cfg(not(feature = "mpv"))]
#[path = "player_stub.rs"]
pub mod player;
pub mod reader;
pub mod scanner;
pub mod shell;
pub mod subtitle;
pub mod textdet;
pub mod tools;

/// 各功能模块自己的表
fn init_modules(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    adclean::init(conn)?;
    aiclassify::init(conn)?;
    // 视频归类规则更新后，已入库的视频按新规则重新归类（不用重新扫描）
    indexer::reclassify_videos(conn)?;
    convert::init(conn)?;
    daily::init(conn)?;
    fetch::init(conn)?;
    lyrics::init(conn)?;
    mangatr::init(conn)?;
    media::init(conn)?;
    organize::init(conn)?;
    shell::init(conn)?;
    subtitle::init(conn)?;
    Ok(())
}

fn setup_modules(app: &tauri::AppHandle) {
    {
        use tauri::Manager;
        tools::set_data_bin(app.state::<AppState>().dirs.data.join("bin"));
    }
    convert::setup(app);
    daily::setup(app);
    fetch::setup(app);
    mangatr::setup(app);
    media::setup(app);
    organize::setup(app);
    shell::setup(app);
    subtitle::setup(app);
    adclean::setup(app);
    aiclassify::setup(app);
}

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
    // 只有调试版认这个环境变量，正式版永远用系统的应用数据目录
    let test_dir = if cfg!(debug_assertions) { std::env::var_os("MP_DATA_DIR") } else { None };
    let (data, cache) = match test_dir {
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
    init_modules(&conn)?;
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
    // 调试版用 MP_DATA_DIR 跑测试实例时不做单实例限制（多个测试实例可以同时开）
    let test_instance = cfg!(debug_assertions) && std::env::var_os("MP_DATA_DIR").is_some();
    let mut builder = tauri::Builder::default();
    if !test_instance {
        // 单实例必须第一个注册
        builder = builder.plugin(tauri_plugin_single_instance::init(shell::on_second_instance));
    }
    builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().args(["--minimized"]).build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .register_uri_scheme_protocol("mpc", |ctx, req| comic_protocol(ctx.app_handle(), &req))
        .setup(|app| {
            let state = init_state(app.handle()).map_err(|e| format!("初始化失败：{e}"))?;
            app.manage(state);
            commands::restart_watcher(app.handle());
            setup_modules(app.handle());
            // 测试实例在标题上标明，避免和正式版混淆
            if cfg!(debug_assertions) && std::env::var_os("MP_DATA_DIR").is_some() {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_title("极光播放器 [测试实例 · 测试数据]");
                }
            }
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
            lyrics::lyrics_online,
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
            ai::ai_get_config,
            ai::ai_save_config,
            ai::ai_test,
            aiclassify::ai_classify_now,
            player::mpv_load,
            player::mpv_pause,
            player::mpv_stop,
            player::mpv_seek,
            player::mpv_volume,
            player::mpv_speed,
            player::mpv_eq,
            player::mpv_set_option,
            player::mpv_play_video,
            item::item_files,
            item::item_rename,
            item::item_set_cover,
            item::item_remove,
            item::item_delete,
            item::item_files_many,
            item::item_remove_many,
            item::item_delete_many,
            item::item_favorite_many,
            item::library_reset,
            item::item_save_image,
            item::playlist_set_cover,
            // 各功能模块
            media::media_probe,
            media::media_thumb,
            media::media_thumb_dir,
            media::media_subtitles,
            media::media_sub_vtt,
            media::media_save_screenshot,
            media::media_danmaku,
            adclean::adclean_scan,
            adclean::adclean_find_similar,
            adclean::adclean_estimate,
            adclean::adclean_ai_rules,
            adclean::adclean_apply,
            adclean::adclean_revert,
            adclean::adclean_list,
            adclean::adclean_rules,
            adclean::adclean_rule_set_enabled,
            adclean::adclean_rule_delete,
            adclean::adclean_rules_export,
            adclean::adclean_rules_import,
            adclean::adclean_export_clean,
            convert::convert_presets,
            convert::convert_hw,
            convert::convert_get_settings,
            convert::convert_set_settings,
            convert::convert_set_plugin,
            convert::convert_add,
            convert::convert_list,
            convert::convert_cancel,
            convert::convert_clear_done,
            convert::convert_retry,
            convert::convert_undo,
            convert::convert_expand,
            convert::convert_remove,
            subtitle::sub_list_videos,
            subtitle::sub_start,
            subtitle::sub_batch,
            subtitle::sub_cancel,
            subtitle::sub_status,
            subtitle::sub_result,
            subtitle::sub_save,
            subtitle::sub_estimate,
            subtitle::sub_realign,
            daily::daily_get,
            daily::daily_regenerate,
            daily::daily_feedback,
            daily::daily_history,
            daily::daily_save_playlist,
            daily::daily_ai_enhance,
            organize::org_scan,
            organize::org_estimate,
            organize::org_ai_identify,
            organize::org_execute,
            organize::org_undo,
            organize::org_batches,
            fetch::fetch_sources,
            fetch::fetch_add_source,
            fetch::fetch_remove_source,
            fetch::fetch_rebuild_rule,
            fetch::fetch_search,
            fetch::fetch_ai_pick,
            fetch::fetch_download,
            fetch::fetch_inspect_torrent,
            fetch::fetch_tasks,
            fetch::fetch_pause,
            fetch::fetch_resume,
            fetch::fetch_cancel,
            fetch::fetch_retry,
            fetch::fetch_get_settings,
            fetch::fetch_set_settings,
            fetch::fetch_subscribe,
            fetch::fetch_unsubscribe,
            fetch::fetch_subs,
            fetch::fetch_remove_task,
            fetch::fetch_providers,
            meta::meta_providers,
            meta::meta_search,
            meta::meta_episodes,
            fetch::fetch_login_open,
            fetch::fetch_login_finish,
            fetch::fetch_logins,
            fetch::fetch_login_clear,
            fetch::fetch_bili_account,
            fetch::fetch_bili_qr_start,
            fetch::fetch_bili_qr_poll,
            fetch::fetch_bili_logout,
            fetch::fetch_bili_parse,
            fetch::fetch_ytdlp_status,
            fetch::fetch_ytdlp_install,
            localocr::localocr_status,
            localocr::localocr_download,
            localocr::localocr_cancel,
            localocr::localocr_remove,
            localocr::localocr_test,
            mangatr::mangatr_get,
            mangatr::mangatr_translate_page,
            mangatr::mangatr_translate_range,
            mangatr::mangatr_cancel,
            mangatr::mangatr_estimate,
            mangatr::mangatr_update_block,
            mangatr::mangatr_retranslate_block,
            mangatr::mangatr_glossary_set,
            mangatr::mangatr_export,
            shell::shell_hotkeys,
            shell::shell_set_hotkeys,
            shell::shell_hotkeys_reset,
            shell::shell_show_main,
            shell::shell_tray_tooltip,
            shell::shell_startup_files,
            shell::shell_autostart_get,
            shell::shell_autostart_set,
            shell::shell_keep_awake,
            shell::shell_wallpaper,
            shell::shell_explorer_menu,
            shell::shell_desktop_lyrics,
            shell::shell_cache_size,
            shell::shell_cache_clear,
        ])
        .run(tauri::generate_context!())
        .expect("启动失败");
}
