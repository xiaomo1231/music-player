//! 窗口外壳：托盘、全局热键、单实例、开机启动、防休眠、壁纸模式、资源管理器菜单、桌面歌词、缓存清理
//!
//! 集成说明见 docs/integration/shell.md。
use crate::{library, AppState};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 启动时建表（CREATE TABLE IF NOT EXISTS）；本模块不需要表
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次：建托盘、注册热键、接管主窗口关闭、处理启动参数
pub fn setup(app: &AppHandle) {
    if let Err(e) = build_tray(app) {
        eprintln!("托盘创建失败：{e}");
    }
    apply_hotkeys(app, &merge_hotkeys(get_setting(app, HOTKEYS_KEY).as_ref()));

    if let Some(w) = app.get_webview_window(MAIN) {
        let h = app.clone();
        w.on_window_event(move |ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                on_close_requested(&h);
            }
        });
    }

    // 启动参数：开机自启带 --minimized；资源管理器右键带文件路径
    let args: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let (paths, add_only) = parse_open_args(&args, &cwd);
    if !paths.is_empty() {
        *PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(OpenPayload { paths, add_only });
    }
    if args.iter().any(|a| a == "--minimized") && get_bool(app, "autostart_minimized", false) {
        if let Some(w) = app.get_webview_window(MAIN) {
            let _ = w.hide();
        }
    }
}

/// 第二次启动时（tauri-plugin-single-instance 回调）：拉起已有窗口，并把文件交给前端
pub fn on_second_instance(app: &AppHandle, args: Vec<String>, cwd: String) {
    show_main(app);
    let (paths, add_only) = parse_open_args(&args, &cwd);
    if !paths.is_empty() {
        let _ = app.emit(OPEN_EVENT, OpenPayload { paths, add_only });
    }
}

// ---------------------------------------------------------------- 常量与类型

pub const CMD_EVENT: &str = "shell://cmd";
pub const OPEN_EVENT: &str = "shell://open-files";
/// 关闭 / 退出需要前端弹窗确认时发出，内容见 CloseAsk
pub const CLOSE_EVENT: &str = "shell://close-ask";
/// 设置项：关闭按钮行为 "ask" / "tray" / "quit"（旧版本用布尔值 close_to_tray）
const CLOSE_ACTION_KEY: &str = "close_action";
/// 桌面歌词窗口的 label
pub const LYRICS: &str = "lyrics";
const TRAY_ID: &str = "main-tray";
const MAIN: &str = "main";
const HOTKEYS_KEY: &str = "hotkeys";

/// 热键表：(动作, 默认按键, 命令)。命令发给前端 shell://cmd；"window" 由 Rust 直接处理（显示 / 隐藏主窗口）
pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("play_pause", "MediaPlayPause", "toggle"),
    ("media_next", "MediaTrackNext", "next"),
    ("media_prev", "MediaTrackPrevious", "prev"),
    ("next", "Ctrl+Alt+ArrowRight", "next"),
    ("prev", "Ctrl+Alt+ArrowLeft", "prev"),
    ("vol_up", "Ctrl+Alt+ArrowUp", "vol_up"),
    ("vol_down", "Ctrl+Alt+ArrowDown", "vol_down"),
    ("window", "Ctrl+Alt+P", "window"),
    ("fav", "Ctrl+Alt+D", "fav"),
    ("lyrics", "Ctrl+Alt+L", "lyrics"),
];

/// 资源管理器右键菜单项：(注册表子键名, 菜单文字, 追加的参数)
const EXPLORER_ITEMS: [(&str, &str, &str); 2] = [("AuroraPlay", "用极光播放器播放", ""), ("AuroraLib", "加入极光播放器媒体库", " --add")];

const ES_CONTINUOUS: u32 = 0x8000_0000;
const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HotkeyCfg {
    pub action: String,
    pub accel: String,
    pub enabled: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HotkeyStatus {
    pub action: String,
    pub accel: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenPayload {
    pub paths: Vec<String>,
    pub add_only: bool,
}

/// 发给前端的关闭询问：ask=true 时让用户选最小化还是退出；ask=false 时只是退出前确认下载
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CloseAsk {
    pub ask: bool,
    pub downloads: i64,
}

/// 前端是否已经在监听关闭询问；没就绪（比如界面还没加载完）时不弹窗，按原来的方式处理
static CLOSE_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Serialize)]
pub struct AutostartInfo {
    pub enabled: bool,
    pub minimized: bool,
}

#[derive(Serialize)]
pub struct CacheInfo {
    pub thumbs: u64,
    pub covers: u64,
}

struct SavedWin {
    pos: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    decorated: bool,
}

/// 热键注册结果（最近一次 apply 的结果，shell_hotkeys() 读取）
static HK_STATUS: Mutex<Vec<HotkeyStatus>> = Mutex::new(Vec::new());
/// 启动时或第二实例带来的待打开文件（前端就绪后用 shell_startup_files() 取走）
static PENDING_OPEN: Mutex<Option<OpenPayload>> = Mutex::new(None);
/// 防休眠线程的停止开关：drop 掉 Sender 线程就会恢复系统默认
static KEEP_AWAKE: Mutex<Option<mpsc::Sender<()>>> = Mutex::new(None);
/// 壁纸模式开启前的窗口状态
static WALLPAPER: Mutex<Option<SavedWin>> = Mutex::new(None);

// ---------------------------------------------------------------- 纯函数（有单元测试）

pub fn default_hotkeys() -> Vec<HotkeyCfg> {
    ACTIONS
        .iter()
        .map(|(a, k, _)| HotkeyCfg {
            action: a.to_string(),
            accel: k.to_string(),
            enabled: true,
        })
        .collect()
}

/// 合并已保存的热键设置和默认值：未知动作丢弃；缺失的动作用默认值补上；空按键保留默认值
pub fn merge_hotkeys(saved: Option<&serde_json::Value>) -> Vec<HotkeyCfg> {
    let mut out = default_hotkeys();
    let Some(arr) = saved.and_then(|v| v.as_array()) else {
        return out;
    };
    for item in arr {
        let Ok(cfg) = serde_json::from_value::<HotkeyCfg>(item.clone()) else {
            continue;
        };
        if let Some(d) = out.iter_mut().find(|d| d.action == cfg.action) {
            let accel = cfg.accel.trim();
            if !accel.is_empty() {
                d.accel = accel.to_string();
            }
            d.enabled = cfg.enabled;
        }
    }
    out
}

const MODIFIER_NAMES: [&str; 9] = ["ctrl", "control", "alt", "option", "shift", "win", "super", "cmd", "meta"];

/// 把单个按键名规范成 Code 名（单个字母 -> KeyX，数字 -> DigitN）；修饰键名返回 None
fn canonical_key(k: &str) -> Option<String> {
    if MODIFIER_NAMES.contains(&k.to_ascii_lowercase().as_str()) {
        return None;
    }
    if k.len() == 1 {
        let c = k.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(format!("Key{}", c.to_ascii_uppercase()));
        }
        if c.is_ascii_digit() {
            return Some(format!("Digit{c}"));
        }
    }
    Some(k.to_string())
}

fn is_media_key(k: &str) -> bool {
    k.to_ascii_lowercase().starts_with("media")
}

fn is_function_key(k: &str) -> bool {
    k.to_ascii_lowercase()
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
}

/// 解析并校验按键字符串，例如 "Ctrl+Alt+ArrowLeft"、"ctrl+alt+p"、"MediaPlayPause"。
/// 返回 (规范写法, 插件用的 Shortcut)。规则：单键必须带 Ctrl / Alt / Win，或是多媒体键 / F 键；
/// 只带 Shift 会影响打字，不允许。
pub fn parse_accel(accel: &str) -> std::result::Result<(String, Shortcut), String> {
    let parts: Vec<&str> = accel.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(format!("按键格式不对：{accel}"));
    }
    let Some((key_raw, mods)) = parts.split_last() else {
        return Err("按键不能为空".into());
    };
    let (mut ctrl, mut alt, mut shift, mut win) = (false, false, false, false);
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "win" | "super" | "cmd" | "meta" => win = true,
            _ => return Err(format!("不认识的修饰键：{m}")),
        }
    }
    let key = canonical_key(key_raw).ok_or_else(|| format!("缺少按键：{accel}"))?;
    let any_mod = ctrl || alt || win;
    if !any_mod && !is_media_key(&key) && !is_function_key(&key) {
        return Err(if shift {
            "只按 Shift 会影响打字，请再加上 Ctrl / Alt / Win".into()
        } else {
            "单键需要配合 Ctrl / Alt / Win，或使用多媒体键、F1–F24".into()
        });
    }

    let mut canon: Vec<String> = vec![];
    let mut plugin: Vec<&str> = vec![];
    for (on, name, plug) in [(ctrl, "Ctrl", "Ctrl"), (alt, "Alt", "Alt"), (shift, "Shift", "Shift"), (win, "Win", "Super")] {
        if on {
            canon.push(name.to_string());
            plugin.push(plug);
        }
    }
    canon.push(key.clone());
    plugin.push(key.as_str());
    let sc = Shortcut::from_str(&plugin.join("+")).map_err(|_| format!("不支持的按键：{key_raw}"))?;
    Ok((canon.join("+"), sc))
}

/// 从命令行参数里取出要打开的文件路径；--add 表示只加入媒体库不播放。argv[0] 是程序本身
pub fn parse_open_args(args: &[String], cwd: &str) -> (Vec<String>, bool) {
    let add_only = args.iter().skip(1).any(|a| a == "--add");
    let paths = args
        .iter()
        .skip(1)
        .filter(|a| !a.is_empty() && !a.starts_with("--"))
        .map(|a| {
            if Path::new(a).is_absolute() || cwd.is_empty() {
                a.clone()
            } else {
                PathBuf::from(cwd).join(a).to_string_lossy().into_owned()
            }
        })
        .collect();
    (paths, add_only)
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    rd.flatten()
        .map(|e| {
            let path = e.path();
            if path.is_dir() {
                dir_size(&path)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// 清空目录里的内容（保留目录本身），返回释放的字节数
fn clear_dir(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else {
        return 0;
    };
    let mut freed = 0;
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            let size = dir_size(&path);
            if std::fs::remove_dir_all(&path).is_ok() {
                freed += size;
            }
        } else {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            if std::fs::remove_file(&path).is_ok() {
                freed += len;
            }
        }
    }
    freed
}

// ---------------------------------------------------------------- 设置读写

fn get_setting(app: &AppHandle, key: &str) -> Option<serde_json::Value> {
    let st = app.state::<AppState>();
    let conn = st.db.lock();

    library::get_setting(&conn, key).ok().flatten()
}

fn get_bool(app: &AppHandle, key: &str, default: bool) -> bool {
    get_setting(app, key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn save_setting(app: &AppHandle, key: &str, value: &serde_json::Value) -> Result<(), String> {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    library::set_setting(&conn, key, value).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 窗口与托盘

/// 显示主窗口并拉到前台
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 显示 / 隐藏主窗口（托盘单击、Ctrl+Alt+P）
pub fn toggle_main(app: &AppHandle) {
    let Some(w) = app.get_webview_window(MAIN) else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
    if visible {
        let _ = w.hide();
    } else {
        show_main(app);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let play = MenuItem::with_id(app, "toggle", "播放 / 暂停", true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", "上一首", true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", "下一首", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let mini = MenuItem::with_id(app, "mini", "迷你播放器", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&play, &prev, &next, &sep1, &show, &mini, &sep2, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("极光播放器")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app: &AppHandle, ev: MenuEvent| on_tray_menu(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// 读取关闭按钮行为：优先 close_action；没有时沿用旧开关 close_to_tray（开着就是 "tray"），否则每次询问
fn resolve_close_action(action: Option<&serde_json::Value>, legacy_tray: Option<&serde_json::Value>) -> &'static str {
    match action.and_then(|v| v.as_str()) {
        Some("tray") => "tray",
        Some("quit") => "quit",
        Some("ask") => "ask",
        _ if legacy_tray.and_then(|v| v.as_bool()) == Some(true) => "tray",
        _ => "ask",
    }
}

fn close_action(app: &AppHandle) -> &'static str {
    resolve_close_action(get_setting(app, CLOSE_ACTION_KEY).as_ref(), get_setting(app, "close_to_tray").as_ref())
}

/// 正在进行的下载数（排队、下载中、整理中；做种不算）
fn active_downloads(app: &AppHandle) -> i64 {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    conn.query_row(
        "SELECT COUNT(*) FROM fetch_tasks WHERE status IN ('queued','downloading','organizing')",
        [],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// 点了主窗口的关闭按钮
fn on_close_requested(app: &AppHandle) {
    match close_action(app) {
        "tray" => {
            if let Some(w) = app.get_webview_window(MAIN) {
                let _ = w.hide();
            }
        }
        "quit" => request_quit(app),
        _ => {
            if !CLOSE_READY.load(std::sync::atomic::Ordering::SeqCst) {
                app.exit(0);
                return;
            }
            show_main(app);
            let _ = app.emit(
                CLOSE_EVENT,
                CloseAsk {
                    ask: true,
                    downloads: active_downloads(app),
                },
            );
        }
    }
}

/// 退出程序；有下载在进行时先让前端确认
fn request_quit(app: &AppHandle) {
    let downloads = active_downloads(app);
    if downloads == 0 || !CLOSE_READY.load(std::sync::atomic::Ordering::SeqCst) {
        app.exit(0);
        return;
    }
    show_main(app);
    let _ = app.emit(CLOSE_EVENT, CloseAsk { ask: false, downloads });
}

fn on_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "quit" => request_quit(app),
        "show" => {
            show_main(app);
            let _ = app.emit(CMD_EVENT, "show");
        }
        "mini" => {
            show_main(app);
            let _ = app.emit(CMD_EVENT, "mini");
        }
        "toggle" | "next" | "prev" => {
            let _ = app.emit(CMD_EVENT, id);
        }
        _ => {}
    }
}

/// 热键触发：多数发给前端；"window" 在 Rust 里切换主窗口
fn hotkey_fired(app: &AppHandle, cmd: &str) {
    if cmd == "window" {
        toggle_main(app);
    } else {
        let _ = app.emit(CMD_EVENT, cmd);
    }
}

// ---------------------------------------------------------------- 全局热键

/// 按配置注册全局热键（先全部注销），返回每一项的结果并记下来
fn apply_hotkeys(app: &AppHandle, list: &[HotkeyCfg]) -> Vec<HotkeyStatus> {
    let _ = app.global_shortcut().unregister_all();
    let mut seen: Vec<String> = vec![];
    let mut out = Vec::with_capacity(list.len());
    for cfg in list {
        let mut st = HotkeyStatus {
            action: cfg.action.clone(),
            accel: cfg.accel.clone(),
            enabled: cfg.enabled,
            error: None,
        };
        if cfg.enabled {
            match register_one(app, &cfg.action, &cfg.accel, &mut seen) {
                Ok(canon) => st.accel = canon,
                Err(e) => st.error = Some(e),
            }
        }
        out.push(st);
    }
    *HK_STATUS.lock().unwrap_or_else(|e| e.into_inner()) = out.clone();
    out
}

fn register_one(app: &AppHandle, action: &str, accel: &str, seen: &mut Vec<String>) -> std::result::Result<String, String> {
    let (canon, sc) = parse_accel(accel)?;
    if seen.contains(&canon) {
        return Err("与其他热键重复".into());
    }
    let cmd = ACTIONS
        .iter()
        .find(|a| a.0 == action)
        .map(|a| a.2)
        .ok_or_else(|| format!("未知动作：{action}"))?;
    app.global_shortcut()
        .on_shortcut(sc, move |app, _sc, ev| {
            if ev.state == ShortcutState::Pressed {
                hotkey_fired(app, cmd);
            }
        })
        .map_err(|e| hotkey_error_text(&e.to_string()))?;
    seen.push(canon.clone());
    Ok(canon)
}

/// 把插件返回的英文调试信息换成给用户看的中文提示
fn hotkey_error_text(raw: &str) -> String {
    let l = raw.to_ascii_lowercase();
    if l.contains("already registered") || l.contains("already in use") {
        "已被其他软件占用，请换一个组合".into()
    } else if l.contains("couldn't recognize") || l.contains("unexpected") || l.contains("unsupported") {
        "系统不支持这个组合，请换一个".into()
    } else {
        "注册失败，请换一个组合或稍后再试".into()
    }
}

#[tauri::command]
pub fn shell_hotkeys() -> Vec<HotkeyStatus> {
    HK_STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 设置页录制新组合期间先注销全部热键，避免按下的组合被系统拦截（界面收不到按键、反而触发了动作）；
/// on=false 时按已保存的配置重新注册。异步命令不占主线程，注册期间界面照常刷新。
#[tauri::command]
pub async fn shell_hotkeys_suspend(app: AppHandle, on: bool) -> Result<Vec<HotkeyStatus>, String> {
    if on {
        let _ = app.global_shortcut().unregister_all();
        Ok(shell_hotkeys())
    } else {
        Ok(apply_hotkeys(&app, &merge_hotkeys(get_setting(&app, HOTKEYS_KEY).as_ref())))
    }
}

#[tauri::command]
pub async fn shell_set_hotkeys(app: AppHandle, list: Vec<HotkeyCfg>) -> Result<Vec<HotkeyStatus>, String> {
    let merged = merge_hotkeys(Some(&serde_json::to_value(&list).map_err(|e| e.to_string())?));
    save_setting(&app, HOTKEYS_KEY, &serde_json::to_value(&merged).map_err(|e| e.to_string())?)?;
    Ok(apply_hotkeys(&app, &merged))
}

#[tauri::command]
pub async fn shell_hotkeys_reset(app: AppHandle) -> Result<Vec<HotkeyStatus>, String> {
    let list = default_hotkeys();
    save_setting(&app, HOTKEYS_KEY, &serde_json::to_value(&list).map_err(|e| e.to_string())?)?;
    Ok(apply_hotkeys(&app, &list))
}

// ---------------------------------------------------------------- 命令：窗口 / 托盘 / 启动文件

#[tauri::command]
pub fn shell_show_main(app: AppHandle) {
    show_main(&app);
}

#[tauri::command]
pub fn shell_tray_tooltip(app: AppHandle, text: String) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let short: String = text.chars().take(120).collect();
        tray.set_tooltip(Some(short)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 前端已开始监听关闭询问
#[tauri::command]
pub fn shell_close_ready() {
    CLOSE_READY.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// 关闭询问的结果：action 为 "tray" 或 "quit"；remember 为真时记住，下次不再询问
#[tauri::command]
pub fn shell_close_choose(app: AppHandle, action: String, remember: bool) -> Result<(), String> {
    if action != "tray" && action != "quit" {
        return Err(format!("未知操作：{action}"));
    }
    if remember {
        save_setting(&app, CLOSE_ACTION_KEY, &serde_json::json!(action))?;
    }
    if action == "tray" {
        if let Some(w) = app.get_webview_window(MAIN) {
            let _ = w.hide();
        }
    } else {
        app.exit(0);
    }
    Ok(())
}

/// 取走启动时带来的文件（前端就绪后调用一次）
#[tauri::command]
pub fn shell_startup_files() -> Option<OpenPayload> {
    PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).take()
}

// ---------------------------------------------------------------- 开机启动

#[tauri::command]
pub fn shell_autostart_get(app: AppHandle) -> Result<AutostartInfo, String> {
    let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    Ok(AutostartInfo {
        enabled,
        minimized: get_bool(&app, "autostart_minimized", false),
    })
}

/// 开关开机启动。插件的启动参数固定带 --minimized，"启动后最小化"由设置 autostart_minimized 决定（见 setup）
#[tauri::command]
pub fn shell_autostart_set(app: AppHandle, enabled: bool, minimized: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch().enable().map_err(|e| e.to_string())?;
    } else {
        app.autolaunch().disable().map_err(|e| e.to_string())?;
    }
    save_setting(&app, "autostart_minimized", &serde_json::json!(minimized))
}

// ---------------------------------------------------------------- 防休眠

#[cfg(windows)]
fn exec_state(flags: u32) {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    unsafe {
        SetThreadExecutionState(flags);
    }
}

#[cfg(not(windows))]
fn exec_state(_flags: u32) {}

/// 执行状态只对调用线程有效，所以单独开一个线程，每 30 秒续一次；停止时恢复 ES_CONTINUOUS
fn keep_awake_loop(rx: mpsc::Receiver<()>) {
    loop {
        exec_state(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
        match rx.recv_timeout(Duration::from_secs(30)) {
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            _ => break,
        }
    }
    exec_state(ES_CONTINUOUS);
}

#[tauri::command]
pub fn shell_keep_awake(on: bool) -> Result<(), String> {
    let mut slot = KEEP_AWAKE.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        if slot.is_none() {
            let (tx, rx) = mpsc::channel::<()>();
            std::thread::Builder::new()
                .name("keep-awake".into())
                .spawn(move || keep_awake_loop(rx))
                .map_err(|e| e.to_string())?;
            *slot = Some(tx);
        }
    } else {
        *slot = None; // drop Sender，线程退出并恢复系统默认
    }
    Ok(())
}

// ---------------------------------------------------------------- 壁纸模式（非官方做法）

#[cfg(windows)]
mod win32 {
    //! 直接声明用到的 Win32 函数，HWND 用 isize 表示（与指针同宽）
    pub type Hwnd = isize;
    pub const SMTO_NORMAL: u32 = 0;
    pub const SM_CXSCREEN: i32 = 0;
    pub const SM_CYSCREEN: i32 = 1;
    pub const HWND_BOTTOM: Hwnd = 1;
    pub const SWP_NOZORDER: u32 = 0x0004;
    pub const SWP_NOACTIVATE: u32 = 0x0010;
    pub const SWP_SHOWWINDOW: u32 = 0x0040;
    pub const WM_SPAWN_WORKER: u32 = 0x052C;

    #[link(name = "user32")]
    extern "system" {
        pub fn FindWindowW(class: *const u16, name: *const u16) -> Hwnd;
        pub fn FindWindowExW(parent: Hwnd, after: Hwnd, class: *const u16, name: *const u16) -> Hwnd;
        pub fn SendMessageTimeoutW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize, flags: u32, timeout: u32, result: *mut usize) -> isize;
        pub fn EnumWindows(f: unsafe extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        pub fn SetParent(child: Hwnd, parent: Hwnd) -> Hwnd;
        pub fn GetSystemMetrics(index: i32) -> i32;
        pub fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }

    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
}

#[cfg(windows)]
unsafe extern "system" fn enum_workerw(top: win32::Hwnd, lparam: isize) -> i32 {
    use win32::*;
    let def = FindWindowExW(top, 0, wide("SHELLDLL_DefView").as_ptr(), std::ptr::null());
    if def != 0 {
        // 含 SHELLDLL_DefView（桌面图标）的窗口之后的那个 WorkerW 就是壁纸层
        let worker = FindWindowExW(0, top, wide("WorkerW").as_ptr(), std::ptr::null());
        if worker != 0 {
            *(lparam as *mut Hwnd) = worker;
        }
        return 0;
    }
    1
}

/// 找到桌面的 WorkerW：向 Progman 发 0x052C 让它生成 WorkerW，再枚举顶层窗口
#[cfg(windows)]
fn find_worker() -> std::result::Result<win32::Hwnd, String> {
    use win32::*;
    unsafe {
        let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
        if progman == 0 {
            return Err("找不到 Progman 窗口（系统桌面进程），壁纸模式不可用".into());
        }
        let mut res: usize = 0;
        SendMessageTimeoutW(progman, WM_SPAWN_WORKER, 0xD, 1, SMTO_NORMAL, 1000, &mut res);
        for _ in 0..10 {
            let mut worker: Hwnd = 0;
            EnumWindows(enum_workerw, &mut worker as *mut Hwnd as isize);
            if worker != 0 {
                return Ok(worker);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Err("没有找到桌面的 WorkerW 窗口，壁纸模式不可用（可能是系统桌面设置不同）".into())
}

#[cfg(windows)]
fn wallpaper_apply(app: &AppHandle, on: bool) -> std::result::Result<(), String> {
    use win32::*;
    let w = app.get_webview_window(MAIN).ok_or("找不到主窗口")?;
    let hwnd = w.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let mut slot = WALLPAPER.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        if slot.is_some() {
            return Ok(());
        }
        let worker = find_worker()?;
        let saved = SavedWin {
            pos: w.outer_position().map_err(|e| e.to_string())?,
            size: w.outer_size().map_err(|e| e.to_string())?,
            decorated: w.is_decorated().unwrap_or(true),
        };
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_fullscreen(false);
        let _ = w.set_decorations(false);
        let parented = unsafe {
            if SetParent(hwnd, worker) == 0 {
                false
            } else {
                let (cx, cy) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
                SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, cx, cy, SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER);
                true
            }
        };
        if !parented {
            let _ = w.set_decorations(saved.decorated);
            return Err("把窗口挂到桌面失败".into());
        }
        *slot = Some(saved);
    } else {
        let Some(saved) = slot.take() else {
            return Ok(());
        };
        unsafe {
            SetParent(hwnd, 0);
        }
        let _ = w.set_decorations(saved.decorated);
        let _ = w.set_size(saved.size);
        let _ = w.set_position(saved.pos);
    }
    Ok(())
}

#[cfg(not(windows))]
fn wallpaper_apply(_app: &AppHandle, _on: bool) -> std::result::Result<(), String> {
    Err("壁纸模式只支持 Windows".into())
}

/// 壁纸模式：把主窗口挂到桌面 WorkerW 下面（非官方做法，失败会返回原因）
#[tauri::command]
pub fn shell_wallpaper(app: AppHandle, on: bool) -> Result<(), String> {
    wallpaper_apply(&app, on)
}

// ---------------------------------------------------------------- 资源管理器右键菜单（只写 HKCU）

#[cfg(windows)]
fn reg(args: &[&str]) -> std::result::Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("注册表写入失败（退出码 {:?}）", out.status.code()))
    }
}

#[cfg(windows)]
fn explorer_apply(on: bool) -> std::result::Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    for kind in ["audio", "video"] {
        for (name, label, extra) in EXPLORER_ITEMS {
            let key = format!(r"HKCU\Software\Classes\SystemFileAssociations\{kind}\shell\{name}");
            let cmd_key = format!(r"{key}\command");
            if on {
                let cmd = format!("\"{exe}\"{extra} \"%1\"");
                reg(&["add", key.as_str(), "/ve", "/d", label, "/f"])?;
                reg(&["add", cmd_key.as_str(), "/ve", "/d", cmd.as_str(), "/f"])?;
            } else {
                let _ = reg(&["delete", key.as_str(), "/f"]);
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn explorer_apply(_on: bool) -> std::result::Result<(), String> {
    Err("资源管理器集成只支持 Windows".into())
}

/// 资源管理器右键菜单：音频 / 视频文件加“用极光播放器播放”“加入极光播放器媒体库”
#[tauri::command]
pub fn shell_explorer_menu(on: bool) -> Result<(), String> {
    explorer_apply(on)
}

// ---------------------------------------------------------------- 桌面歌词

/// 桌面歌词窗口：透明、无边框、置顶、不在任务栏、鼠标穿透。窗口加载 index.html#lyrics
#[tauri::command]
pub fn shell_desktop_lyrics(app: AppHandle, on: bool) -> Result<(), String> {
    const W: f64 = 1000.0;
    const H: f64 = 140.0;
    if !on {
        if let Some(w) = app.get_webview_window(LYRICS) {
            w.close().map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(w) = app.get_webview_window(LYRICS) {
        w.show().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(&app, LYRICS, WebviewUrl::App("index.html#lyrics".into()))
        .title("桌面歌词")
        .inner_size(W, H)
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())?;
    // 放在主显示器底部居中
    if let Ok(Some(m)) = app.primary_monitor() {
        let sc = m.scale_factor();
        let (pw, ph) = ((W * sc) as i32, (H * sc) as i32);
        let x = m.position().x + (m.size().width as i32 - pw) / 2;
        let y = m.position().y + m.size().height as i32 - ph - (60.0 * sc) as i32;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
    win.set_ignore_cursor_events(true).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------- 缓存

#[tauri::command]
pub fn shell_cache_size(app: AppHandle) -> CacheInfo {
    let st = app.state::<AppState>();
    CacheInfo {
        thumbs: dir_size(&st.dirs.thumbs),
        covers: dir_size(&st.dirs.covers),
    }
}

/// 清理缓存：默认只清缩略图；covers=true 时连封面缓存一起清（封面会重新生成）。返回释放的字节数。不碰数据库
#[tauri::command]
pub fn shell_cache_clear(app: AppHandle, covers: bool) -> u64 {
    let st = app.state::<AppState>();
    let mut freed = clear_dir(&st.dirs.thumbs);
    if covers {
        freed += clear_dir(&st.dirs.covers);
    }
    freed
}

// ---------------------------------------------------------------- 单元测试

#[cfg(test)]
mod tests {
    #[test]
    fn hotkey_error_text_is_friendly() {
        use super::hotkey_error_text;
        let raw = "HotKey already registered: HotKey { mods: Modifiers(ALT | CONTROL), key: KeyP, id: 589858 }";
        assert_eq!(hotkey_error_text(raw), "已被其他软件占用，请换一个组合");
        assert_eq!(hotkey_error_text("Unable to register hotkey: 1409"), "注册失败，请换一个组合或稍后再试");
        assert_eq!(
            hotkey_error_text("Couldn't recognize \"Foo\" as a valid HotKey Code"),
            "系统不支持这个组合，请换一个"
        );
    }

    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn accel_valid_forms() {
        assert_eq!(parse_accel("Ctrl+Alt+ArrowLeft").unwrap().0, "Ctrl+Alt+ArrowLeft");
        assert_eq!(parse_accel("ctrl + alt + p").unwrap().0, "Ctrl+Alt+KeyP");
        assert_eq!(parse_accel("Ctrl+Alt+D").unwrap().0, "Ctrl+Alt+KeyD");
        assert_eq!(parse_accel("Ctrl+Alt+1").unwrap().0, "Ctrl+Alt+Digit1");
        assert_eq!(parse_accel("Win+Ctrl+KeyL").unwrap().0, "Ctrl+Win+KeyL");
        assert_eq!(parse_accel("MediaPlayPause").unwrap().0, "MediaPlayPause");
        assert_eq!(parse_accel("MediaTrackNext").unwrap().0, "MediaTrackNext");
        assert_eq!(parse_accel("F5").unwrap().0, "F5");
        assert_eq!(parse_accel("Ctrl+Space").unwrap().0, "Ctrl+Space");
    }

    #[test]
    fn accel_invalid_forms() {
        assert!(parse_accel("").is_err());
        assert!(parse_accel("Ctrl+").is_err());
        assert!(parse_accel("Ctrl+Alt").is_err(), "只有修饰键没有按键");
        assert!(parse_accel("A").is_err(), "单字母没有修饰键");
        assert!(parse_accel("Shift+A").is_err(), "只带 Shift 会影响打字");
        assert!(parse_accel("Foo+Alt+A").is_err(), "不认识的修饰键");
        assert!(parse_accel("Ctrl+Alt+NotAKey").is_err(), "不认识的按键");
        assert!(parse_accel("Ctrl+Alt+A+B").is_err(), "多个按键");
    }

    #[test]
    fn function_and_media_rules() {
        assert!(is_function_key("F1"));
        assert!(is_function_key("f24"));
        assert!(!is_function_key("F25"));
        assert!(!is_function_key("Fx"));
        assert!(!is_function_key("F"));
        assert!(is_media_key("MediaStop"));
    }

    #[test]
    fn hotkeys_defaults_and_merge() {
        let d = default_hotkeys();
        assert_eq!(d.len(), ACTIONS.len());
        assert!(d.iter().all(|h| h.enabled));
        // 没有保存过：用默认值
        assert_eq!(merge_hotkeys(None), d);
        // 部分保存：只覆盖被保存的项，其余保持默认，未知动作丢弃
        let saved = serde_json::json!([
            {"action": "fav", "accel": "Ctrl+Alt+F", "enabled": false},
            {"action": "bogus", "accel": "Ctrl+Alt+X", "enabled": true},
            {"action": "window", "accel": "  ", "enabled": true}
        ]);
        let m = merge_hotkeys(Some(&saved));
        assert_eq!(m.len(), ACTIONS.len());
        let fav = m.iter().find(|h| h.action == "fav").unwrap();
        assert_eq!(fav.accel, "Ctrl+Alt+F");
        assert!(!fav.enabled);
        let win = m.iter().find(|h| h.action == "window").unwrap();
        assert_eq!(win.accel, "Ctrl+Alt+P", "空按键保留默认值");
        assert!(m.iter().all(|h| h.action != "bogus"));
        // 格式错误的值：忽略，得到默认值
        assert_eq!(merge_hotkeys(Some(&serde_json::json!("oops"))), d);
    }

    #[test]
    fn close_action_resolves_with_legacy_fallback() {
        let j = |v: serde_json::Value| Some(v);
        assert_eq!(resolve_close_action(None, None), "ask");
        assert_eq!(resolve_close_action(None, j(serde_json::json!(false)).as_ref()), "ask");
        assert_eq!(resolve_close_action(None, j(serde_json::json!(true)).as_ref()), "tray");
        assert_eq!(
            resolve_close_action(j(serde_json::json!("quit")).as_ref(), j(serde_json::json!(true)).as_ref()),
            "quit"
        );
        assert_eq!(
            resolve_close_action(j(serde_json::json!("ask")).as_ref(), j(serde_json::json!(true)).as_ref()),
            "ask"
        );
        assert_eq!(resolve_close_action(j(serde_json::json!("tray")).as_ref(), None), "tray");
        assert_eq!(resolve_close_action(j(serde_json::json!("weird")).as_ref(), None), "ask");
    }

    #[test]
    fn open_args() {
        let (p, add) = parse_open_args(&s(&["app.exe", "--add", "D:\\music\\a.mp3", "--minimized"]), "");
        assert_eq!(p, vec!["D:\\music\\a.mp3".to_string()]);
        assert!(add);
        let (p, add) = parse_open_args(&s(&["app.exe", "D:\\x.flac"]), "");
        assert_eq!(p, vec!["D:\\x.flac".to_string()]);
        assert!(!add);
        let (p, _) = parse_open_args(&s(&["app.exe"]), "D:\\cwd");
        assert!(p.is_empty());
        let (p, _) = parse_open_args(&s(&["app.exe", "rel.mp3"]), "D:\\cwd");
        assert_eq!(p.len(), 1);
        assert!(p[0].ends_with("rel.mp3"));
    }

    #[test]
    fn cache_clear_only_touches_dir_contents() {
        let base = std::env::temp_dir().join(format!("aurora-shell-test-{}", std::process::id()));
        let thumbs = base.join("thumbs");
        std::fs::create_dir_all(thumbs.join("sub")).unwrap();
        std::fs::write(thumbs.join("a.jpg"), vec![0u8; 100]).unwrap();
        std::fs::write(thumbs.join("sub").join("b.jpg"), vec![0u8; 50]).unwrap();
        assert_eq!(dir_size(&thumbs), 150);
        assert_eq!(clear_dir(&thumbs), 150);
        assert!(thumbs.is_dir(), "目录本身保留");
        assert_eq!(dir_size(&thumbs), 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
