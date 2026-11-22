//! 格式转换队列（FFmpeg），完成后回写与撤销
use anyhow::Result;

/// 启动时建表（CREATE TABLE IF NOT EXISTS）
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（可以启动后台任务）
pub fn setup(_app: &tauri::AppHandle) {}
