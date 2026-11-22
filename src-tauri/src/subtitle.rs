//! AI 字幕：提取音轨、切分人声、识别、断句对齐、翻译、导出
use anyhow::Result;

/// 启动时建表（CREATE TABLE IF NOT EXISTS）
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（可以启动后台任务）
pub fn setup(_app: &tauri::AppHandle) {}
