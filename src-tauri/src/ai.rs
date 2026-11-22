//! AI 服务（全局一处，字幕、漫画翻译、去广告、推荐、整理共用）
//! 兼容 OpenAI 接口格式：/chat/completions、/audio/transcriptions。也可以填 Ollama 等本地服务的地址。
//! 密钥保存在 Windows 凭据管理器，不写进数据库。
use crate::{db, library, AppState};
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager, State};

const KEYRING_SERVICE: &str = "music-player-ai";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AiConfig {
    /// 例如 https://api.openai.com/v1 或 http://localhost:11434/v1
    pub base_url: String,
    /// 语音识别模型（如 whisper-1）
    pub asr_model: String,
    /// 翻译 / 文本模型
    pub chat_model: String,
    /// 图像识别模型（漫画翻译用）
    pub vision_model: String,
    /// 每月 token 上限提醒（0 表示不限）
    pub monthly_token_limit: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            asr_model: "whisper-1".into(),
            chat_model: "gpt-4o-mini".into(),
            vision_model: "gpt-4o-mini".into(),
            monthly_token_limit: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Usage {
    /// 2026-11 这样的月份
    pub month: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub audio_seconds: f64,
    pub requests: u64,
}

pub fn load_config(conn: &rusqlite::Connection) -> AiConfig {
    library::get_setting(conn, "ai").ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

pub fn api_key() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, "default").ok()?.get_password().ok().filter(|s| !s.is_empty())
}

pub fn set_api_key(key: &str) -> Result<()> {
    let e = keyring::Entry::new(KEYRING_SERVICE, "default")?;
    if key.is_empty() {
        let _ = e.delete_credential();
    } else {
        e.set_password(key)?;
    }
    Ok(())
}

fn this_month() -> String {
    // 不引入日期库：按 UTC 粗略换算年月
    let days = db::now() / 86400;
    let (mut y, mut d) = (1970, days);
    loop {
        let len = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    while d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!("{y}-{:02}", m + 1)
}

pub fn usage(conn: &rusqlite::Connection) -> Usage {
    let u: Usage = library::get_setting(conn, "ai_usage").ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    if u.month != this_month() {
        Usage { month: this_month(), ..Default::default() }
    } else {
        u
    }
}

fn add_usage(app: &AppHandle, prompt: u64, completion: u64, audio: f64) {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    let mut u = usage(&conn);
    u.prompt_tokens += prompt;
    u.completion_tokens += completion;
    u.audio_seconds += audio;
    u.requests += 1;
    let _ = library::set_setting(&conn, "ai_usage", &serde_json::to_value(&u).unwrap_or_default());
}

/// 当前配置 + 密钥；没配置时给出明确的提示
pub fn ready(app: &AppHandle) -> Result<(AiConfig, String)> {
    let cfg = load_config(&app.state::<AppState>().db.lock());
    let key = api_key().unwrap_or_default();
    let local = cfg.base_url.contains("localhost") || cfg.base_url.contains("127.0.0.1");
    if key.is_empty() && !local {
        return Err(anyhow!("还没有配置 AI 服务：请在“设置 → AI 服务”里填写接口地址和密钥"));
    }
    Ok((cfg, key))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build().unwrap_or_default()
}

fn endpoint(cfg: &AiConfig, path: &str) -> String {
    format!("{}/{}", cfg.base_url.trim_end_matches('/'), path)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Msg {
    pub role: String,
    /// 字符串，或 OpenAI 的多段内容数组（图文混合）
    pub content: serde_json::Value,
}

impl Msg {
    pub fn system(s: &str) -> Self {
        Msg { role: "system".into(), content: json!(s) }
    }
    pub fn user(s: &str) -> Self {
        Msg { role: "user".into(), content: json!(s) }
    }
    /// 一段文字 + 一张图片（jpeg / png 字节）
    pub fn user_image(text: &str, image: &[u8], mime: &str) -> Self {
        let b64 = base64::engine::general_purpose::STANDARD.encode(image);
        Msg {
            role: "user".into(),
            content: json!([
                { "type": "text", "text": text },
                { "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{b64}") } }
            ]),
        }
    }
}

/// 调用 /chat/completions。json=true 时要求模型只返回 JSON 对象
pub async fn chat_with(app: &AppHandle, model: &str, messages: &[Msg], json_mode: bool) -> Result<String> {
    let (cfg, key) = ready(app)?;
    let mut body = json!({ "model": model, "messages": messages, "temperature": 0.2 });
    if json_mode {
        body["response_format"] = json!({ "type": "json_object" });
    }
    let mut req = client().post(endpoint(&cfg, "chat/completions")).json(&body);
    if !key.is_empty() {
        req = req.bearer_auth(&key);
    }
    let resp = req.send().await.context("连接 AI 服务失败")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("AI 服务返回的不是 JSON")?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("未知错误");
        return Err(anyhow!("AI 服务出错（{status}）：{msg}"));
    }
    let p = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let c = v["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    add_usage(app, p, c, 0.0);
    let text = v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
    Ok(text)
}

/// 用“翻译 / 文本模型”对话
pub async fn chat(app: &AppHandle, messages: &[Msg], json_mode: bool) -> Result<String> {
    let model = load_config(&app.state::<AppState>().db.lock()).chat_model;
    chat_with(app, &model, messages, json_mode).await
}

/// 最常用的形式：一段系统提示 + 一段用户输入
pub async fn chat_simple(app: &AppHandle, system: &str, user: &str, json_mode: bool) -> Result<String> {
    chat(app, &[Msg::system(system), Msg::user(user)], json_mode).await
}

/// 用“图像识别模型”看图
pub async fn vision(app: &AppHandle, system: &str, prompt: &str, image: &[u8], mime: &str, json_mode: bool) -> Result<String> {
    let model = load_config(&app.state::<AppState>().db.lock()).vision_model;
    chat_with(app, &model, &[Msg::system(system), Msg::user_image(prompt, image, mime)], json_mode).await
}

/// 从模型回复里取出 JSON（容忍 ```json 包裹和前后多余文字）
pub fn extract_json(s: &str) -> Result<serde_json::Value> {
    let t = s.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    if let Ok(v) = serde_json::from_str(t) {
        return Ok(v);
    }
    let (a, b) = (t.find(['{', '[']), t.rfind(['}', ']']));
    match (a, b) {
        (Some(a), Some(b)) if b > a => Ok(serde_json::from_str(&t[a..=b])?),
        _ => Err(anyhow!("AI 返回的内容无法解析为 JSON")),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// 语音识别：上传一段音频（wav / mp3 / flac 等），返回带时间戳的分段
pub async fn transcribe(app: &AppHandle, audio: Vec<u8>, filename: &str, language: Option<&str>, prompt: Option<&str>) -> Result<Vec<Segment>> {
    let (cfg, key) = ready(app)?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", cfg.asr_model.clone())
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "segment")
        .part("file", reqwest::multipart::Part::bytes(audio).file_name(filename.to_string()));
    if let Some(l) = language {
        form = form.text("language", l.to_string());
    }
    if let Some(p) = prompt {
        form = form.text("prompt", p.to_string());
    }
    let mut req = client().post(endpoint(&cfg, "audio/transcriptions")).multipart(form);
    if !key.is_empty() {
        req = req.bearer_auth(&key);
    }
    let resp = req.send().await.context("连接语音识别服务失败")?;
    let status = resp.status();
    let v: serde_json::Value = resp.json().await.context("语音识别服务返回的不是 JSON")?;
    if !status.is_success() {
        return Err(anyhow!("语音识别出错（{status}）：{}", v["error"]["message"].as_str().unwrap_or("未知错误")));
    }
    let dur = v["duration"].as_f64().unwrap_or(0.0);
    add_usage(app, 0, 0, dur);
    let segs = v["segments"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| Segment {
                    start: s["start"].as_f64().unwrap_or(0.0),
                    end: s["end"].as_f64().unwrap_or(0.0),
                    text: s["text"].as_str().unwrap_or("").trim().to_string(),
                })
                .filter(|s| !s.text.is_empty())
                .collect()
        })
        .unwrap_or_else(|| {
            // 不支持分段的服务：整段当一条
            let t = v["text"].as_str().unwrap_or("").trim().to_string();
            if t.is_empty() {
                vec![]
            } else {
                vec![Segment { start: 0.0, end: dur, text: t }]
            }
        });
    Ok(segs)
}

/// 粗略估算 token（中文约 1 字 1 token，英文约 4 字符 1 token），用于开始前提示用量
pub fn estimate_tokens(s: &str) -> u64 {
    let cjk = s.chars().filter(|c| (*c as u32) > 0x2E80).count() as u64;
    let other = s.chars().count() as u64 - cjk;
    cjk + other / 4 + 1
}

// ---------- 命令 ----------

#[derive(Serialize)]
pub struct AiState {
    pub config: AiConfig,
    pub has_key: bool,
    pub usage: Usage,
}

#[tauri::command]
pub fn ai_get_config(st: State<AppState>) -> AiState {
    let conn = st.db.lock();
    AiState { config: load_config(&conn), has_key: api_key().is_some(), usage: usage(&conn) }
}

/// key 为 None 时保留原密钥；空字符串表示删除
#[tauri::command]
pub fn ai_save_config(st: State<AppState>, config: AiConfig, key: Option<String>) -> Result<(), String> {
    library::set_setting(&st.db.lock(), "ai", &serde_json::to_value(&config).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if let Some(k) = key {
        set_api_key(k.trim()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 测试连接：让文本模型回一句话
#[tauri::command]
pub async fn ai_test(app: AppHandle) -> Result<String, String> {
    chat_simple(&app, "你是连接测试助手。", "只回复“连接正常”四个字。", false).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_extract() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```").unwrap()["a"], 1);
        assert_eq!(extract_json("好的：{\"a\":[1,2]} 以上").unwrap()["a"][1], 2);
        assert!(extract_json("没有").is_err());
    }

    #[test]
    fn month_format() {
        let m = this_month();
        assert_eq!(m.len(), 7);
        assert!(m.starts_with("20"));
    }

    #[test]
    fn tokens() {
        assert!(estimate_tokens("你好世界") >= 4);
    }
}
