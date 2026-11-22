//! 窗口外壳：托盘、全局热键、壁纸模式、开机启动
use anyhow::Result;

/// 启动时建表（CREATE TABLE IF NOT EXISTS）
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（可以启动后台任务）
pub fn setup(_app: &tauri::AppHandle) {}
