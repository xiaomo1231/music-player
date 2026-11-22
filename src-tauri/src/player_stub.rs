//! 没有打开 `mpv` 特性时的占位：命令都返回“未安装 mpv”，前端据此退回网页播放
use tauri::{AppHandle, State};

const NO: &str = "没有安装 mpv 播放引擎";
type R<T> = Result<T, String>;

#[tauri::command]
pub fn mpv_load(_app: AppHandle, _path: String, _start: Option<f64>, _paused: Option<bool>) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_pause(_app: AppHandle, _paused: bool) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_stop(_app: AppHandle) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_seek(_app: AppHandle, _time: f64) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_volume(_app: AppHandle, _volume: f64) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_speed(_app: AppHandle, _speed: f64) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_eq(_app: AppHandle, _gains: Vec<f64>) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_set_option(_app: AppHandle, _name: String, _value: String) -> R<()> {
    Err(NO.into())
}
#[tauri::command]
pub fn mpv_play_video(_app: AppHandle, _st: State<crate::AppState>, _id: i64, _start: Option<f64>) -> R<()> {
    Err(NO.into())
}
