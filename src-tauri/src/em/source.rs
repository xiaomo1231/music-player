//! 下载源规则（kind: source）：按规则搜索、解析地址，注册成 Provider（id 前缀 em-）
use super::{Issue, Rule};
use serde_json::Value;
use std::sync::Arc;
use tauri::AppHandle;

/// 检查正文；返回导入确认框里显示的摘要（每行一句）
pub fn check(_rule: &Rule) -> Result<Vec<String>, Vec<Issue>> {
    Ok(vec![])
}

/// 生效（on = true）或失效
pub fn apply(_app: &AppHandle, _rule: &Rule, _on: bool) {}

/// 规则被删除后清理自己的数据
pub fn removed(_app: &AppHandle, _id: &str) {}

/// 规则管理页的“测试”
pub async fn test(_app: AppHandle, _rule: Arc<Rule>, _arg: Value) -> Result<Value, String> {
    Err("这类规则还不支持测试".into())
}
