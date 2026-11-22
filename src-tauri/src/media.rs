//! 视频信息、缩略图、字幕提取（ffprobe / ffmpeg）
use crate::{danmaku, library, reader, tools, AppState};
use anyhow::{anyhow, Result};
use base64::Engine;
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::State;

/// 启动时建表（CREATE TABLE IF NOT EXISTS）。本模块只用 videos 表，不需要自己的表。
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（可以启动后台任务）
pub fn setup(_app: &tauri::AppHandle) {}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// 在阻塞线程里跑外部程序，避免卡住界面
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?.map_err(err)
}

/// 按视频 id 取文件路径（取不到说明已从媒体库移除）
fn path_of(st: &AppState, id: i64) -> Result<String, String> {
    library::file_path(&st.db.lock(), id).map_err(|_| "找不到这个视频（可能已从媒体库移除）".to_string())
}

// ---------- ffprobe ----------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub index: i64,
    pub lang: String,
    pub title: String,
    pub codec: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ProbeInfo {
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub codec: String,
    pub audio: Vec<Track>,
    pub subs: Vec<Track>,
}

fn tag(s: &Value, key: &str) -> String {
    s["tags"][key].as_str().unwrap_or("").to_string()
}

/// 解析 ffprobe -show_format -show_streams 的 JSON
pub fn parse_probe(v: &Value) -> ProbeInfo {
    let mut info = ProbeInfo {
        duration: 0.0,
        width: 0,
        height: 0,
        codec: String::new(),
        audio: vec![],
        subs: vec![],
    };
    info.duration = v["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| v["format"]["duration"].as_f64())
        .unwrap_or(0.0);
    let Some(streams) = v["streams"].as_array() else { return info };
    for s in streams {
        let track = Track {
            index: s["index"].as_i64().unwrap_or(-1),
            lang: tag(s, "language"),
            title: tag(s, "title"),
            codec: s["codec_name"].as_str().unwrap_or("").to_string(),
        };
        match s["codec_type"].as_str().unwrap_or("") {
            "video" => {
                // 封面图（attached_pic）不算视频流
                if s["disposition"]["attached_pic"].as_i64() == Some(1) || !info.codec.is_empty() {
                    continue;
                }
                info.codec = track.codec.clone();
                info.width = s["width"].as_i64().unwrap_or(0);
                info.height = s["height"].as_i64().unwrap_or(0);
                if info.duration <= 0.0 {
                    info.duration = s["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
                }
            }
            "audio" => info.audio.push(track),
            "subtitle" => info.subs.push(track),
            _ => {}
        }
    }
    info
}

#[tauri::command]
pub async fn media_probe(st: State<'_, AppState>, id: i64) -> Result<ProbeInfo, String> {
    let path = path_of(&st, id)?;
    let info = blocking(move || {
        let v = tools::probe(Path::new(&path))?;
        Ok(parse_probe(&v))
    })
    .await?;
    // 写回 videos 表；内封字幕有流而数据库标 none 的，改成 embedded
    let has_subs = (!info.subs.is_empty()) as i64;
    st.db
        .lock()
        .execute(
            "UPDATE videos SET duration=CASE WHEN ?1>0 THEN ?1 ELSE duration END, width=?2, height=?3, codec=?4,
               subs=CASE WHEN ?5=1 AND subs IN ('embedded?','none') THEN 'embedded'
                         WHEN ?5=0 AND subs='embedded?' THEN 'none' ELSE subs END
             WHERE file_id=?6",
            params![info.duration, info.width, info.height, info.codec, has_subs, id],
        )
        .map_err(err)?;
    Ok(info)
}

// ---------- 缩略图 ----------

/// 缩略图文件名：路径的 blake3 哈希前 24 位 + .jpg
pub fn thumb_name(path: &str) -> String {
    format!("{}.jpg", &blake3::hash(path.as_bytes()).to_hex()[..24])
}

/// 在 10% 和 25% 处各截一帧，取 JPEG 体积较大的那张（纯黑、纯白的画面压缩后很小）
fn make_thumb(thumbs: &Path, src: &Path, name: &str, duration: f64) -> Result<String> {
    let out = thumbs.join(name);
    let duration = if duration > 0.0 {
        duration
    } else {
        tools::probe(src).map(|v| parse_probe(&v).duration).unwrap_or(0.0)
    };
    let stamps = if duration > 0.0 { vec![duration * 0.10, duration * 0.25] } else { vec![10.0] };
    let stem = name.trim_end_matches(".jpg");
    let mut best: Option<(usize, Vec<u8>)> = None;
    let mut last_err: Option<anyhow::Error> = None;
    for (i, ts) in stamps.iter().enumerate() {
        let tmp = thumbs.join(format!("{stem}.{i}.tmp.jpg"));
        let mut c = tools::ffmpeg_cmd()?;
        c.args(["-ss", &format!("{ts:.2}"), "-i"])
            .arg(src)
            .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4"])
            .arg(&tmp);
        match tools::run(c) {
            Ok(_) => {
                if let Ok(bytes) = std::fs::read(&tmp) {
                    if best.as_ref().map_or(true, |b| bytes.len() > b.0) {
                        best = Some((bytes.len(), bytes));
                    }
                }
            }
            Err(e) => last_err = Some(e),
        }
        let _ = std::fs::remove_file(&tmp);
    }
    let (_, bytes) = best.ok_or_else(|| last_err.unwrap_or_else(|| anyhow!("截取缩略图失败")))?;
    std::fs::write(&out, bytes)?;
    Ok(name.to_string())
}

fn save_thumb_name(st: &AppState, id: i64, name: &str) -> Result<(), String> {
    st.db
        .lock()
        .execute("UPDATE videos SET thumb=?1 WHERE file_id=?2", params![name, id])
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn media_thumb(st: State<'_, AppState>, id: i64) -> Result<String, String> {
    let path = path_of(&st, id)?;
    let name = thumb_name(&path);
    let thumbs = st.dirs.thumbs.clone();
    if thumbs.join(&name).exists() {
        save_thumb_name(&st, id, &name)?;
        return Ok(name);
    }
    let duration = {
        let c = st.db.lock();
        library::video(&c, id).map_err(err)?.map(|v| v.duration).unwrap_or(0.0)
    };
    let src = PathBuf::from(&path);
    let made = {
        let name = name.clone();
        blocking(move || make_thumb(&thumbs, &src, &name, duration)).await?
    };
    save_thumb_name(&st, id, &made)?;
    Ok(made)
}

#[tauri::command]
pub fn media_thumb_dir(st: State<AppState>) -> String {
    st.dirs.thumbs.to_string_lossy().into_owned()
}

// ---------- 字幕 ----------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SubItem {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<i64>,
    pub label: String,
}

/// 外挂字幕：文件名以视频文件名开头，紧跟的字符不是字母数字（避免 01 匹配到 010）。
/// 返回（语言 / 标签，扩展名大写）。
pub fn external_sub_tag(video_stem: &str, file_name: &str) -> Option<(String, String)> {
    let p = Path::new(file_name);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "srt" | "ass" | "ssa" | "vtt") || crate::danmaku::is_danmaku_name(file_name) {
        return None;
    }
    let stem = p.file_stem()?.to_str()?;
    let rest = stem.strip_prefix(video_stem)?;
    if !rest.is_empty() && rest.starts_with(|c: char| c.is_alphanumeric()) {
        return None;
    }
    let tag = rest.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let tag = if tag.is_empty() { "默认".to_string() } else { tag };
    Some((tag, ext.to_ascii_uppercase()))
}

/// 内封字幕的显示名
pub fn stream_label(lang: &str, title: &str, codec: &str) -> String {
    let mut parts = vec![if lang.is_empty() { "未知语言".to_string() } else { lang.to_string() }];
    if !title.is_empty() {
        parts.push(title.to_string());
    }
    if !codec.is_empty() {
        parts.push(codec.to_string());
    }
    format!("内封 · {}", parts.join(" · "))
}

fn external_subs(video: &Path) -> Vec<SubItem> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str())) else {
        return vec![];
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut found: Vec<(PathBuf, String)> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let (tag, ext) = external_sub_tag(stem, &name)?;
            Some((e.path(), format!("外挂 · {tag} · {ext}")))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
        .into_iter()
        .map(|(p, label)| SubItem {
            kind: "external",
            path: Some(p.to_string_lossy().into_owned()),
            stream: None,
            label,
        })
        .collect()
}

#[tauri::command]
pub async fn media_subtitles(st: State<'_, AppState>, id: i64) -> Result<Vec<SubItem>, String> {
    let path = path_of(&st, id)?;
    blocking(move || {
        let p = PathBuf::from(&path);
        let mut out = external_subs(&p);
        // ffprobe 不可用时只列外挂字幕
        if let Ok(v) = tools::probe(&p) {
            for t in parse_probe(&v).subs {
                out.push(SubItem {
                    kind: "embedded",
                    path: None,
                    stream: Some(t.index),
                    label: stream_label(&t.lang, &t.title, &t.codec),
                });
            }
        }
        Ok(out)
    })
    .await
}

/// 视频旁边的弹幕（`<视频名>.danmaku.ass`），没有就返回空列表
#[tauri::command]
pub async fn media_danmaku(st: State<'_, AppState>, id: i64) -> Result<Vec<danmaku::Item>, String> {
    let path = path_of(&st, id)?;
    blocking(move || {
        let Some(p) = danmaku::sidecar(Path::new(&path)) else { return Ok(vec![]) };
        Ok(danmaku::parse_ass(&read_text(&p)?))
    })
    .await
}

/// 去掉 BOM，统一换行，按编码识别读成文本
fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(reader::decode_text(&bytes).0.replace('\r', ""))
}

/// SRT 转 WebVTT：加 WEBVTT 头，时间码的逗号改成点
pub fn srt_to_vtt(src: &str) -> String {
    let body: Vec<String> = src
        .trim_start_matches('\u{feff}')
        .replace('\r', "")
        .lines()
        .map(|l| if l.contains("-->") { l.replace(',', ".") } else { l.to_string() })
        .collect();
    format!("WEBVTT\n\n{}\n", body.join("\n").trim())
}

fn ffmpeg_vtt(input: &Path, stream: Option<i64>) -> Result<String> {
    let mut c = tools::ffmpeg_cmd()?;
    c.arg("-i").arg(input);
    if let Some(s) = stream {
        c.args(["-map", &format!("0:{s}")]);
    }
    c.args(["-f", "webvtt", "-"]);
    let out = tools::run(c)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[tauri::command]
pub async fn media_sub_vtt(path: Option<String>, video: Option<String>, stream: Option<i64>) -> Result<String, String> {
    blocking(move || {
        if let Some(p) = path {
            let pb = PathBuf::from(&p);
            let ext = pb.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            return match ext.as_str() {
                "vtt" => read_text(&pb),
                "srt" => Ok(srt_to_vtt(&read_text(&pb)?)),
                _ => ffmpeg_vtt(&pb, None),
            };
        }
        if let (Some(v), Some(s)) = (video, stream) {
            return ffmpeg_vtt(Path::new(&v), Some(s));
        }
        Err(anyhow!("没有指定字幕"))
    })
    .await
}

// ---------- 截图 ----------

/// 秒 → "01-23-45"（用于截图文件名）
pub fn fmt_hms(t: f64) -> String {
    let s = t.max(0.0) as i64;
    format!("{:02}-{:02}-{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// 保存前端截的 PNG，文件名 "视频名_时间.png"，放在视频旁边。`at` 是播放位置（秒），缺省时用当前时间戳。
#[tauri::command]
pub async fn media_save_screenshot(video_path: String, png_base64: String, at: Option<f64>) -> Result<String, String> {
    blocking(move || {
        // 兼容 data:image/png;base64,xxx 形式
        let data = png_base64.rsplit(',').next().unwrap_or("").trim();
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
        let vp = PathBuf::from(&video_path);
        let dir = vp.parent().ok_or_else(|| anyhow!("视频路径无效"))?;
        let stem = vp.file_stem().and_then(|s| s.to_str()).unwrap_or("截图");
        let when = match at {
            Some(t) => fmt_hms(t),
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
        };
        let base = format!("{stem}_{when}");
        let mut out = dir.join(format!("{base}.png"));
        let mut n = 2;
        while out.exists() {
            out = dir.join(format!("{base} ({n}).png"));
            n += 1;
        }
        std::fs::write(&out, bytes)?;
        Ok(out.to_string_lossy().into_owned())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn srt_becomes_vtt() {
        let srt = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello, world\r\n\r\n2\r\n00:01:02,003 --> 00:01:04,000\r\n第二句\r\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500\nHello, world"));
        assert!(vtt.contains("00:01:02.003 --> 00:01:04.000\n第二句"));
        assert!(!vtt.contains(",000"));
    }

    #[test]
    fn thumb_name_is_stable_and_jpg() {
        let a = thumb_name("D:\\video\\a.mkv");
        assert_eq!(a, thumb_name("D:\\video\\a.mkv"));
        assert_ne!(a, thumb_name("D:\\video\\b.mkv"));
        assert!(a.ends_with(".jpg"));
        assert_eq!(a.len(), 24 + 4);
    }

    #[test]
    fn external_sub_matching() {
        assert_eq!(external_sub_tag("Show - 01", "Show - 01.srt"), Some(("默认".into(), "SRT".into())));
        assert_eq!(external_sub_tag("Show - 01", "Show - 01.zh.ass"), Some(("zh".into(), "ASS".into())));
        assert_eq!(external_sub_tag("Show - 01", "Show - 01.chs.ssa"), Some(("chs".into(), "SSA".into())));
        // 01 不能匹配到 010
        assert_eq!(external_sub_tag("Show - 01", "Show - 010.srt"), None);
        // 视频本身或其他扩展名不算字幕
        assert_eq!(external_sub_tag("Show - 01", "Show - 01.mkv"), None);
        assert_eq!(external_sub_tag("Show - 01", "Other.srt"), None);
        // 弹幕文件不出现在字幕列表里（播放器单独绘制）
        assert_eq!(external_sub_tag("Show - 01", "Show - 01.danmaku.ass"), None);
    }

    #[test]
    fn parses_probe_json() {
        let v = json!({
            "format": {"duration": "1420.5"},
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080},
                {"index": 1, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}},
                {"index": 2, "codec_type": "audio", "codec_name": "flac", "tags": {"language": "jpn", "title": "5.1"}},
                {"index": 3, "codec_type": "subtitle", "codec_name": "ass", "tags": {"language": "chi"}},
                {"index": 4, "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle"}
            ]
        });
        let p = parse_probe(&v);
        assert_eq!(p.duration, 1420.5);
        assert_eq!((p.width, p.height), (1920, 1080));
        assert_eq!(p.codec, "hevc");
        assert_eq!(
            p.audio,
            vec![Track {
                index: 2,
                lang: "jpn".into(),
                title: "5.1".into(),
                codec: "flac".into()
            }]
        );
        assert_eq!(p.subs.len(), 2);
        assert_eq!(p.subs[1].lang, "");
    }

    #[test]
    fn probe_duration_falls_back_to_stream() {
        let v = json!({"format": {}, "streams": [{"index": 0, "codec_type": "video", "codec_name": "h264", "width": 640, "height": 360, "duration": "12.5"}]});
        assert_eq!(parse_probe(&v).duration, 12.5);
    }

    #[test]
    fn labels_and_times() {
        assert_eq!(stream_label("chi", "", "ass"), "内封 · chi · ass");
        assert_eq!(stream_label("", "", ""), "内封 · 未知语言");
        assert_eq!(fmt_hms(0.0), "00-00-00");
        assert_eq!(fmt_hms(3725.9), "01-02-05");
    }
}
