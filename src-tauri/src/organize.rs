//! AI 整理文件：识别、预览、执行（移动 / 复制 / 硬链接 / 仅库内）、撤销
use anyhow::Result;

/// 启动时建表（CREATE TABLE IF NOT EXISTS）
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（可以启动后台任务）
pub fn setup(_app: &tauri::AppHandle) {}
