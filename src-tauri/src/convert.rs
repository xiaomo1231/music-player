//! 格式转换队列（FFmpeg）：预设、硬件编码检测、任务队列、完成后回写与撤销
//!
//! 数据约定（jobs 表，不改表结构）：
//! - 排队 / 运行 / 失败 / 取消时 `output` 保存“请求的输出目录”；完成后保存实际输出文件路径。
//! - 替换原文件时 `backup` 保存原文件路径（原文件已移入回收站），撤销时按这个路径还原。
//!
//! 事件：`convert://progress`（进度）、`convert://hw`（硬件检测完成）、`library://changed`（媒体库变化）。
use crate::{db, indexer, library, parse, tools, AppState};
use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;

const CANCEL_MSG: &str = "已取消";
const PLUGIN_OFF: &str = "加密格式需要先启用解锁插件：请在格式转换页面勾选“仅用于本人已购买的内容”";
const PLUGIN_MISSING: &str = "没有找到解锁插件：请把插件 .exe 放到应用数据目录下的 plugins 文件夹";
const LOUDNORM: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";

/// 启动时建表（jobs 表已在 db.rs 建好，这里不需要额外建表）
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

// ---------- 预设 ----------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PresetInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// audio：输出只有音频（视频也可以提取音轨）；video：输入必须有视频画面；both：音视频都可以
    pub kind: &'static str,
    /// 输出扩展名；auto 表示由输入的音频编码决定
    pub ext: &'static str,
    pub note: &'static str,
}

pub const PRESETS: &[PresetInfo] = &[
    PresetInfo {
        id: "mp3",
        name: "转 MP3 320k",
        kind: "audio",
        ext: "mp3",
        note: "保留标签和封面",
    },
    PresetInfo {
        id: "flac",
        name: "转 FLAC",
        kind: "audio",
        ext: "flac",
        note: "无损，保留标签和封面",
    },
    PresetInfo {
        id: "aac",
        name: "转 AAC (m4a)",
        kind: "audio",
        ext: "m4a",
        note: "256k，保留标签和封面",
    },
    PresetInfo {
        id: "opus",
        name: "转 Opus",
        kind: "audio",
        ext: "opus",
        note: "160k；Opus 不保留封面",
    },
    PresetInfo {
        id: "mp4_h264",
        name: "转 MP4 H.264",
        kind: "video",
        ext: "mp4",
        note: "有硬件编码器时自动使用，音频转 AAC",
    },
    PresetInfo {
        id: "mp4_h265",
        name: "转 MP4 H.265",
        kind: "video",
        ext: "mp4",
        note: "体积更小，编码更慢；有硬件编码器时自动使用",
    },
    PresetInfo {
        id: "remux",
        name: "仅换封装（MKV→MP4）",
        kind: "video",
        ext: "mp4",
        note: "不重新编码，秒级完成；字幕流会被丢弃",
    },
    PresetInfo {
        id: "extract_flac",
        name: "提取音轨 → FLAC",
        kind: "video",
        ext: "flac",
        note: "从视频里取出音轨，无损",
    },
    PresetInfo {
        id: "extract_copy",
        name: "提取音轨（原编码）",
        kind: "video",
        ext: "auto",
        note: "不重新编码，按原音频编码选扩展名",
    },
    PresetInfo {
        id: "gif",
        name: "视频转 GIF",
        kind: "video",
        ext: "gif",
        note: "12 fps，宽 480；GIF 无法加入媒体库",
    },
    PresetInfo {
        id: "burn_sub",
        name: "压制字幕",
        kind: "video",
        ext: "mp4",
        note: "烧录同目录同名的 .srt / .ass / .ssa",
    },
    PresetInfo {
        id: "loudnorm",
        name: "响度均衡（loudnorm）",
        kind: "both",
        ext: "auto",
        note: "视频输出 MP4（画面不变），音频输出 FLAC",
    },
];

pub fn find_preset(id: &str) -> R<&'static PresetInfo> {
    PRESETS.iter().find(|p| p.id == id).ok_or_else(|| format!("未知的预设：{id}"))
}

// ---------- 硬件编码 ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HwProbe {
    pub name: String,
    pub ok: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HwStatus {
    /// 是否已经检测完成
    pub checked: bool,
    /// 实际使用的 H.264 编码器
    pub h264: String,
    /// 实际使用的 H.265 编码器
    pub hevc: String,
    pub probes: Vec<HwProbe>,
}

impl HwStatus {
    fn pending() -> Self {
        HwStatus {
            checked: false,
            h264: "libx264".into(),
            hevc: "libx265".into(),
            probes: vec![],
        }
    }
}

#[derive(Clone, Debug)]
pub struct HwEnc {
    pub h264: String,
    pub hevc: String,
}

impl HwEnc {
    pub fn soft() -> Self {
        HwEnc {
            h264: "libx264".into(),
            hevc: "libx265".into(),
        }
    }
}

static HW: Mutex<Option<HwStatus>> = Mutex::new(None);

fn hw_enc() -> HwEnc {
    match HW.lock().as_ref() {
        Some(s) => HwEnc {
            h264: s.h264.clone(),
            hevc: s.hevc.clone(),
        },
        None => HwEnc::soft(),
    }
}

/// 从 `ffmpeg -encoders` 的输出里取出编码器名
pub fn parse_encoder_list(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.split_whitespace().nth(1)).map(|s| s.to_string()).collect()
}

/// 按优先级选编码器：NVENC > QSV > AMF > 软件
pub fn pick_encoder(probes: &[HwProbe], hevc: bool) -> String {
    let (names, fallback) = if hevc {
        (["hevc_nvenc", "hevc_qsv", "hevc_amf"], "libx265")
    } else {
        (["h264_nvenc", "h264_qsv", "h264_amf"], "libx264")
    };
    for n in names {
        if probes.iter().any(|p| p.name == n && p.ok) {
            return n.to_string();
        }
    }
    fallback.to_string()
}

fn list_encoders() -> Vec<String> {
    let Ok(c) = tools::ffmpeg_cmd() else { return vec![] };
    let mut c = c;
    c.arg("-encoders");
    match tools::run(c) {
        Ok(out) => parse_encoder_list(&String::from_utf8_lossy(&out)),
        Err(_) => vec![],
    }
}

/// 用 1 秒测试源真正编一次，确认驱动和显卡可用
fn test_encoder(name: &str) -> bool {
    let Ok(mut c) = tools::ffmpeg_cmd() else { return false };
    c.args([
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=1:size=320x240:rate=25",
        "-c:v",
        name,
        "-t",
        "1",
        "-f",
        "null",
        "-",
    ]);
    c.stdout(Stdio::null()).stderr(Stdio::null());
    // 个别驱动下（例如核显的 AMF）试编码会卡住不退出，最多等 8 秒，超时就当不可用并结束进程
    let Ok(mut child) = c.spawn() else { return false };
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.success(),
            Ok(None) if start.elapsed() < std::time::Duration::from_secs(8) => std::thread::sleep(std::time::Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub fn detect_hw() -> HwStatus {
    let listed = list_encoders();
    let names = ["h264_nvenc", "h264_qsv", "h264_amf", "hevc_nvenc", "hevc_qsv", "hevc_amf"];
    let probes: Vec<HwProbe> = names
        .iter()
        .map(|n| HwProbe {
            name: n.to_string(),
            ok: listed.iter().any(|x| x == n) && test_encoder(n),
        })
        .collect();
    HwStatus {
        checked: true,
        h264: pick_encoder(&probes, false),
        hevc: pick_encoder(&probes, true),
        probes,
    }
}

// ---------- 探测结果辅助 ----------

fn streams(p: &Value) -> &[Value] {
    p["streams"].as_array().map(|v| v.as_slice()).unwrap_or(&[])
}

fn is_attached(s: &Value) -> bool {
    s["disposition"]["attached_pic"].as_i64() == Some(1)
}

/// 第一个真正的视频流（不是封面图）的索引
pub fn real_video_index(p: &Value) -> Option<i64> {
    streams(p)
        .iter()
        .find(|s| s["codec_type"].as_str() == Some("video") && !is_attached(s))
        .and_then(|s| s["index"].as_i64())
}

fn audio_codec(p: &Value) -> Option<String> {
    streams(p)
        .iter()
        .find(|s| s["codec_type"].as_str() == Some("audio"))
        .and_then(|s| s["codec_name"].as_str().map(|x| x.to_string()))
}

/// 媒体时长（秒）：优先 format.duration，没有就取流里最长的
pub fn duration_of(p: &Value) -> f64 {
    if let Some(d) = p["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()) {
        if d > 0.0 {
            return d;
        }
    }
    streams(p)
        .iter()
        .filter_map(|s| s["duration"].as_str().and_then(|x| x.parse::<f64>().ok()))
        .fold(0.0, f64::max)
}

/// 封面图的映射参数：只有 mp3 / flac / m4a 才带封面（mjpeg 或 png 的 attached pic）
fn cover_args(p: &Value, ext: &str) -> Vec<String> {
    if !matches!(ext, "mp3" | "flac" | "m4a") {
        return vec![];
    }
    let Some(s) = streams(p).iter().find(|s| s["codec_type"].as_str() == Some("video") && is_attached(s)) else {
        return vec![];
    };
    if !matches!(s["codec_name"].as_str(), Some("mjpeg") | Some("png")) {
        return vec![];
    }
    let Some(idx) = s["index"].as_i64() else { return vec![] };
    vec![
        "-map".into(),
        format!("0:{idx}"),
        "-c:v".into(),
        "copy".into(),
        "-disposition:v:0".into(),
        "attached_pic".into(),
    ]
}

/// 无损 / 原编码提取时，按音频编码选扩展名
pub fn copy_ext(codec: Option<&str>) -> &'static str {
    match codec.unwrap_or("") {
        "aac" | "alac" => "m4a",
        "mp3" => "mp3",
        "opus" => "opus",
        "vorbis" => "ogg",
        "flac" => "flac",
        "ac3" => "ac3",
        "eac3" => "eac3",
        "dts" => "dts",
        "wmav1" | "wmav2" => "wma",
        c if c.starts_with("pcm_") => "wav",
        _ => "mka",
    }
}

// ---------- 参数生成 ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub ext: String,
    /// 不含 -i 和输出路径
    pub args: Vec<String>,
}

fn sv(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

fn b(ext: &str, mut parts: Vec<String>) -> Built {
    let mut args = sv(&["-map_metadata", "0"]);
    args.append(&mut parts);
    Built { ext: ext.to_string(), args }
}

fn video_enc(hevc: bool, enc: &str) -> Vec<String> {
    let q = if hevc { "26" } else { "23" };
    let a: Vec<&str> = match enc {
        "h264_nvenc" | "hevc_nvenc" => vec!["-c:v", enc, "-preset", "p5", "-rc", "vbr", "-cq", q, "-b:v", "0"],
        "h264_qsv" | "hevc_qsv" => vec!["-c:v", enc, "-global_quality", q],
        "h264_amf" | "hevc_amf" => vec!["-c:v", enc, "-quality", "balanced", "-rc", "cqp", "-qp_i", q, "-qp_p", q],
        "libx265" => vec!["-c:v", "libx265", "-preset", "medium", "-crf", "24"],
        "libx264" => vec!["-c:v", "libx264", "-preset", "medium", "-crf", "20"],
        _ if hevc => vec!["-c:v", "libx265", "-preset", "medium", "-crf", "24"],
        _ => vec!["-c:v", "libx264", "-preset", "medium", "-crf", "20"],
    };
    let mut out = sv(&a);
    if hevc {
        out.extend(sv(&["-tag:v", "hvc1"]));
    }
    out
}

/// 生成某个预设的 ffmpeg 参数。p 是输入的 ffprobe 结果，sub 是放在工作目录里的字幕文件名
pub fn build_args(preset: &str, p: &Value, hw: &HwEnc, sub: Option<&str>) -> R<Built> {
    let need_video = || real_video_index(p).ok_or_else(|| "这个预设需要视频画面，但输入文件里没有视频".to_string());
    match preset {
        "mp3" => Ok(b(
            "mp3",
            [
                sv(&[
                    "-map",
                    "0:a:0",
                    "-c:a",
                    "libmp3lame",
                    "-b:a",
                    "320k",
                    "-id3v2_version",
                    "3",
                    "-write_id3v1",
                    "1",
                ]),
                cover_args(p, "mp3"),
            ]
            .concat(),
        )),
        "flac" => Ok(b(
            "flac",
            [sv(&["-map", "0:a:0", "-c:a", "flac", "-compression_level", "8"]), cover_args(p, "flac")].concat(),
        )),
        "aac" => Ok(b(
            "m4a",
            [
                sv(&["-map", "0:a:0", "-c:a", "aac", "-b:a", "256k", "-movflags", "+faststart"]),
                cover_args(p, "m4a"),
            ]
            .concat(),
        )),
        "opus" => Ok(b("opus", sv(&["-map", "0:a:0", "-c:a", "libopus", "-b:a", "160k", "-vbr", "on"]))),
        "mp4_h264" | "mp4_h265" => {
            let v = need_video()?;
            let hevc = preset == "mp4_h265";
            let enc = if hevc { &hw.hevc } else { &hw.h264 };
            let mut a: Vec<String> = vec!["-map".into(), format!("0:{v}"), "-map".into(), "0:a:0?".into()];
            a.extend(video_enc(hevc, enc));
            a.extend(sv(&["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]));
            Ok(b("mp4", a))
        }
        "remux" => {
            let v = need_video()?;
            let a: Vec<String> = vec![
                "-map".into(),
                format!("0:{v}"),
                "-map".into(),
                "0:a?".into(),
                "-c".into(),
                "copy".into(),
                "-movflags".into(),
                "+faststart".into(),
            ];
            Ok(b("mp4", a))
        }
        "extract_flac" => Ok(b("flac", sv(&["-map", "0:a:0", "-c:a", "flac"]))),
        "extract_copy" => {
            let ext = copy_ext(audio_codec(p).as_deref());
            Ok(b(ext, [sv(&["-map", "0:a:0", "-c:a", "copy"]), cover_args(p, ext)].concat()))
        }
        "gif" => {
            let v = need_video()?;
            let fc = format!("[0:{v}]fps=12,scale=480:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse");
            Ok(b("gif", vec!["-filter_complex".into(), fc, "-an".into(), "-loop".into(), "0".into()]))
        }
        "burn_sub" => {
            let v = need_video()?;
            let name = sub.ok_or_else(|| "没有找到同名字幕文件（同目录的 .srt / .ass / .ssa）".to_string())?;
            let mut a: Vec<String> = vec![
                "-map".into(),
                format!("0:{v}"),
                "-map".into(),
                "0:a:0?".into(),
                "-vf".into(),
                format!("subtitles={name}"),
            ];
            a.extend(video_enc(false, &hw.h264));
            a.extend(sv(&["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]));
            Ok(b("mp4", a))
        }
        "loudnorm" => match real_video_index(p) {
            Some(v) => Ok(b(
                "mp4",
                vec![
                    "-map".into(),
                    format!("0:{v}"),
                    "-map".into(),
                    "0:a:0?".into(),
                    "-c:v".into(),
                    "copy".into(),
                    "-af".into(),
                    LOUDNORM.into(),
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    "192k".into(),
                    "-movflags".into(),
                    "+faststart".into(),
                ],
            )),
            None => Ok(b(
                "flac",
                vec!["-map".into(), "0:a:0".into(), "-af".into(), LOUDNORM.into(), "-c:a".into(), "flac".into()],
            )),
        },
        other => Err(format!("未知的预设：{other}")),
    }
}

// ---------- 输出路径 ----------

/// 目录里 `stem.ext` 不存在就用它，重名就加 " (1)"、" (2)"……
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(file_name(stem, ext));
    if !first.exists() {
        return first;
    }
    for n in 1..10000 {
        let p = dir.join(file_name(&format!("{stem} ({n})"), ext));
        if !p.exists() {
            return p;
        }
    }
    dir.join(file_name(&format!("{stem} ({})", db::now()), ext))
}

fn file_name(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

fn default_out_dir(audio: bool, data: &Path) -> PathBuf {
    let base = if audio { dirs::audio_dir() } else { dirs::video_dir() }.unwrap_or_else(|| data.to_path_buf());
    base.join("Converted")
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---------- 设置 ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ConvertSettings {
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default)]
    pub concurrency: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PluginSetting {
    #[serde(default)]
    pub enabled: bool,
    /// 用户已确认“仅用于本人已购买的内容”
    #[serde(default)]
    pub ack: bool,
}

fn load_settings(conn: &Connection) -> ConvertSettings {
    library::get_setting(conn, "convert")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<ConvertSettings>(v).ok())
        .unwrap_or_default()
}

fn load_plugin(conn: &Connection) -> PluginSetting {
    library::get_setting(conn, "convert_plugin")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<PluginSetting>(v).ok())
        .unwrap_or_default()
}

fn concurrency(conn: &Connection) -> usize {
    load_settings(conn).concurrency.unwrap_or(2).clamp(1, 4) as usize
}

/// 插件目录里第一个 .exe
fn plugin_exe(data: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(data.join("plugins"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && parse::ext(p) == "exe")
        .collect();
    v.sort();
    v.into_iter().next()
}

/// 同目录同名字幕
fn find_subtitle(input: &Path) -> Option<PathBuf> {
    let dir = input.parent()?;
    let stem = input.file_stem()?.to_string_lossy().to_string();
    ["srt", "ass", "ssa"].iter().map(|e| dir.join(format!("{stem}.{e}"))).find(|p| p.is_file())
}

// ---------- 进度 ----------

/// 解析 `-progress pipe:1` 的一行 key=value
pub fn parse_progress_line(line: &str) -> Option<(&str, &str)> {
    line.trim().split_once('=')
}

/// speed=1.23x 解析为 1.23；N/A 返回 None
pub fn parse_speed(v: &str) -> Option<f64> {
    v.trim().trim_end_matches('x').parse::<f64>().ok().filter(|x| x.is_finite() && *x > 0.0)
}

/// 已输出时长 / 总时长，最多到 0.99（真正完成由进程结束决定）
pub fn progress_ratio(out_us: i64, dur: f64) -> f64 {
    if dur <= 0.0 {
        return 0.0;
    }
    (out_us as f64 / 1_000_000.0 / dur).clamp(0.0, 0.99)
}

#[derive(Serialize, Clone, Debug)]
pub struct ProgressEvent {
    pub id: i64,
    pub progress: f64,
    pub speed: Option<f64>,
    pub status: String,
    pub error: Option<String>,
    pub output: Option<String>,
}

fn report(app: &AppHandle, id: i64, progress: f64, speed: Option<f64>, status: &str, error: Option<String>, output: Option<String>) {
    if status == "running" {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let _ = conn.execute("UPDATE jobs SET progress=?1 WHERE id=?2", params![progress, id]);
    }
    let _ = app.emit(
        "convert://progress",
        ProgressEvent {
            id,
            progress,
            speed,
            status: status.to_string(),
            error,
            output,
        },
    );
}

// ---------- 运行中的任务 ----------

pub struct JobCtl {
    canceled: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl JobCtl {
    fn new() -> Self {
        JobCtl {
            canceled: AtomicBool::new(false),
            child: Mutex::new(None),
        }
    }
    fn canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }
}

/// 正在运行的任务（id, 控制块）
static RUNNING: Mutex<Vec<(i64, Arc<JobCtl>)>> = Mutex::new(Vec::new());

/// 离开作用域时删除文件或目录（用于临时文件和工作目录）
struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let n = lines.len();
    lines[n.saturating_sub(6)..].join("\n")
}

/// 启动外部程序并逐行读 stdout；返回（是否成功，stderr 末尾几行）。取消时会被 kill
fn run_child(ctl: &JobCtl, mut cmd: Command, on_line: &mut dyn FnMut(&str)) -> R<(bool, String)> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("启动外部程序失败：{e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *ctl.child.lock() = Some(child);
    if ctl.canceled() {
        if let Some(c) = ctl.child.lock().as_mut() {
            let _ = c.kill();
        }
    }
    let err_handle = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut text);
        }
        text
    });
    if let Some(out) = stdout {
        for line in BufReader::new(out).lines() {
            match line {
                Ok(l) => on_line(&l),
                Err(_) => break,
            }
        }
    }
    let status = {
        let mut g = ctl.child.lock();
        g.take().and_then(|mut c| c.wait().ok())
    };
    let err = err_handle.join().unwrap_or_default();
    Ok((status.map(|s| s.success()).unwrap_or(false), tail(&err)))
}

// ---------- 队列 ----------

/// 把一个 queued 任务改成 running，返回 id（没有排队任务时 None）
fn claim(conn: &Connection) -> Option<i64> {
    let id: i64 = conn
        .query_row("SELECT id FROM jobs WHERE status='queued' ORDER BY id LIMIT 1", [], |r| r.get(0))
        .optional()
        .ok()
        .flatten()?;
    let n = conn
        .execute("UPDATE jobs SET status='running', progress=0, error=NULL WHERE id=?1 AND status='queued'", [id])
        .ok()?;
    if n == 1 {
        Some(id)
    } else {
        None
    }
}

/// 后台调度：每 300ms 检查一次，按并发上限启动排队任务
fn dispatcher(app: AppHandle) {
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let conc = {
            let st = app.state::<AppState>();
            let conn = st.db.lock();
            concurrency(&conn)
        };
        loop {
            let running = RUNNING.lock().len();
            if running >= conc {
                break;
            }
            let next = {
                let st = app.state::<AppState>();
                let conn = st.db.lock();
                claim(&conn)
            };
            let Some(id) = next else { break };
            let ctl = Arc::new(JobCtl::new());
            RUNNING.lock().push((id, ctl.clone()));
            let a = app.clone();
            std::thread::spawn(move || run_job(a, id, ctl));
        }
    }
}

struct Job {
    input: String,
    preset: String,
    after: String,
    output: Option<String>,
}

fn load_job(conn: &Connection, id: i64) -> R<Job> {
    conn.query_row("SELECT input,preset,after,output FROM jobs WHERE id=?1", [id], |r| {
        Ok(Job {
            input: r.get(0)?,
            preset: r.get(1)?,
            after: r.get(2)?,
            output: r.get(3)?,
        })
    })
    .map_err(|e| format!("读取任务失败：{e}"))
}

fn run_job(app: AppHandle, id: i64, ctl: Arc<JobCtl>) {
    let res = execute(&app, &ctl, id);
    let canceled = ctl.canceled();
    let after = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        conn.query_row("SELECT after FROM jobs WHERE id=?1", [id], |r| r.get::<_, String>(0))
            .unwrap_or_default()
    };
    let (status, output, error) = match res {
        Ok(out) => ("done", Some(out), None),
        Err(_) if canceled => ("canceled", None, None),
        Err(e) => ("failed", None, Some(e)),
    };
    {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let _ = conn.execute(
            "UPDATE jobs SET status=?1, progress=CASE WHEN ?1='done' THEN 1 ELSE progress END, error=?2, output=COALESCE(?3, output), finished_at=?4 WHERE id=?5",
            params![status, error, output, db::now(), id],
        );
    }
    let progress = if status == "done" { 1.0 } else { 0.0 };
    report(&app, id, progress, None, status, error, output);
    if after != "dir" {
        let _ = app.emit("library://changed", ());
    }
    RUNNING.lock().retain(|(j, _)| *j != id);
}

fn execute(app: &AppHandle, ctl: &JobCtl, id: i64) -> R<String> {
    let st = app.state::<AppState>();
    let (job, settings, plugin) = {
        let conn = st.db.lock();
        (load_job(&conn, id)?, load_settings(&conn), load_plugin(&conn))
    };
    let input = PathBuf::from(&job.input);
    if !input.is_file() {
        return Err("原文件不存在，可能已被移动或删除".into());
    }
    let preset = find_preset(&job.preset)?;
    let work = Guard(st.dirs.data.join("convert-work").join(id.to_string()));
    std::fs::create_dir_all(&work.0).map_err(|e| format!("无法创建临时目录：{e}"))?;

    // 加密格式：交给用户自己安装的插件解出标准音频；未启用时直接失败
    let src = if parse::is_encrypted(&input) {
        if !(plugin.enabled && plugin.ack) {
            return Err(PLUGIN_OFF.into());
        }
        let exe = plugin_exe(&st.dirs.data).ok_or_else(|| PLUGIN_MISSING.to_string())?;
        let out = work.0.join("decoded.bin");
        let mut c = tools::command(&exe);
        c.arg(&input).arg(&out);
        let (ok, text) = run_child(ctl, c, &mut |_: &str| {})?;
        if ctl.canceled() {
            return Err(CANCEL_MSG.into());
        }
        if !ok || !out.is_file() {
            return Err(format!("插件解密失败：{}", if text.is_empty() { "插件没有输出文件".to_string() } else { text }));
        }
        out
    } else {
        input.clone()
    };

    let probe_src = tools::probe(&src).map_err(|e| format!("读取源文件失败：{e}"))?;
    let dur = duration_of(&probe_src);

    let sub_name = if preset.id == "burn_sub" {
        let sp = find_subtitle(&input).ok_or_else(|| "没有找到同名字幕文件（同目录的 .srt / .ass / .ssa）".to_string())?;
        let name = format!("sub.{}", parse::ext(&sp));
        std::fs::copy(&sp, work.0.join(&name)).map_err(|e| format!("复制字幕失败：{e}"))?;
        Some(name)
    } else {
        None
    };
    let built = build_args(preset.id, &probe_src, &hw_enc(), sub_name.as_deref())?;

    if job.after != "dir" && parse::classify(Path::new(&format!("x.{}", built.ext))).is_none() {
        return Err(format!("{} 无法加入媒体库，请选“只输出到指定目录”", built.ext.to_uppercase()));
    }
    if ctl.canceled() {
        return Err(CANCEL_MSG.into());
    }

    // 输出目录：替换 / 旁边 → 原文件所在目录；加入媒体库 / 只输出 → 任务里的目录、设置里的目录、默认目录
    let target: PathBuf = match job.after.as_str() {
        "replace" | "side" => input.parent().map(|p| p.to_path_buf()).ok_or("原文件路径无效")?,
        _ => {
            let custom = job
                .output
                .clone()
                .filter(|x| !x.trim().is_empty())
                .or_else(|| settings.out_dir.clone().filter(|x| !x.trim().is_empty()));
            match custom {
                Some(d) => PathBuf::from(d),
                None => default_out_dir(parse::AUDIO.contains(&built.ext.as_str()), &st.dirs.data),
            }
        }
    };
    std::fs::create_dir_all(&target).map_err(|e| format!("无法创建输出目录：{e}"))?;
    // 临时文件放在隐藏子目录里：扫描器会跳过以 . 开头的目录，不会把半成品当成新文件入库
    let tmp_dir = target.join(".mpc-convert");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("无法创建临时目录：{e}"))?;
    let tmp = Guard(tmp_dir.join(format!("{id}.{}", built.ext)));

    let mut cmd = tools::ffmpeg_cmd().map_err(s)?;
    cmd.current_dir(&work.0);
    cmd.args(["-progress", "pipe:1", "-nostats", "-i"]).arg(&src).args(&built.args).arg(&tmp.0);

    let mut last = Instant::now() - Duration::from_secs(1);
    let mut speed: Option<f64> = None;
    let mut ratio = 0.0f64;
    let mut on_line = |line: &str| {
        let Some((k, v)) = parse_progress_line(line) else { return };
        match k {
            "out_time_us" => {
                if let Ok(us) = v.trim().parse::<i64>() {
                    ratio = progress_ratio(us, dur);
                }
            }
            "speed" => speed = parse_speed(v),
            _ => {}
        }
        if last.elapsed() >= Duration::from_millis(300) {
            last = Instant::now();
            report(app, id, ratio, speed, "running", None, None);
        }
    };
    let (ok, text) = run_child(ctl, cmd, &mut on_line)?;
    if ctl.canceled() {
        return Err(CANCEL_MSG.into());
    }
    let size = std::fs::metadata(&tmp.0).map(|m| m.len()).unwrap_or(0);
    if !ok || size == 0 {
        return Err(if text.is_empty() { "转换失败".to_string() } else { text });
    }
    report(app, id, 0.99, None, "running", None, None);

    if job.after == "replace" {
        finalize_replace(&st, id, &input, &tmp.0, &built.ext, dur)
    } else {
        finalize_new(&st, &job.after, &input, &tmp.0, &built.ext, &target)
    }
}

// ---------- 完成后处理 ----------

/// 把新文件放进目录并入库（不改原文件）。dir 模式不入库
fn finalize_new(st: &AppState, after: &str, input: &Path, tmp: &Path, ext: &str, target: &Path) -> R<String> {
    let stem = input.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let fin = unique_path(target, &stem, ext);
    std::fs::rename(tmp, &fin).map_err(|e| format!("保存文件失败：{e}"))?;
    if let Some(parent) = tmp.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    if after != "dir" {
        let conn = st.db.lock();
        index_path(&conn, &fin, &st.dirs)?;
    }
    Ok(fin.to_string_lossy().to_string())
}

/// 替换原文件：校验新文件 → 原文件进回收站 → 新文件改名到原位置 → 库里换路径（保留 id）
fn finalize_replace(st: &AppState, id: i64, input: &Path, tmp: &Path, ext: &str, src_dur: f64) -> R<String> {
    validate_output(tmp, src_dur)?;
    let fid: Option<i64> = {
        let conn = st.db.lock();
        conn.query_row("SELECT id FROM files WHERE path=?1", [input.to_string_lossy().to_string()], |r| r.get(0))
            .optional()
            .map_err(s)?
    };
    trash::delete(input).map_err(|e| format!("原文件移入回收站失败（它可能正在被播放）：{e}"))?;
    let parent = input.parent().ok_or("原文件路径无效")?;
    let stem = input.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let fin = unique_path(parent, &stem, ext);
    if let Err(e) = std::fs::rename(tmp, &fin) {
        return Err(match restore_from_trash(input) {
            Ok(()) => format!("替换失败，原文件已还原：{e}"),
            Err(b) => format!("替换失败：{e}；原文件还原失败，请到回收站手动还原（{b}）"),
        });
    }
    // 先记下原文件去向，之后的库更新失败也能撤销
    {
        let conn = st.db.lock();
        conn.execute(
            "UPDATE jobs SET backup=?1, output=?2 WHERE id=?3",
            params![input.to_string_lossy().to_string(), fin.to_string_lossy().to_string(), id],
        )
        .map_err(s)?;
        match fid {
            Some(fid) => relink(&conn, fid, &fin, &st.dirs)?,
            None => index_path(&conn, &fin, &st.dirs)?,
        }
    }
    Ok(fin.to_string_lossy().to_string())
}

/// 新文件的大小、时长和流都要正常，时长和原文件相差不能太多
fn validate_output(path: &Path, src_dur: f64) -> R<()> {
    let p = tools::probe(path).map_err(|e| format!("新文件校验失败：{e}"))?;
    if streams(&p).is_empty() {
        return Err("新文件没有音频或视频流，已取消替换".into());
    }
    let d = duration_of(&p);
    if src_dur > 0.0 && d > 0.0 {
        let tol = (src_dur * 0.03).max(2.0);
        if (d - src_dur).abs() > tol {
            return Err(format!("新文件时长不对（{d:.1} 秒，原文件 {src_dur:.1} 秒），已取消替换"));
        }
    }
    // 抽样解码前 60 秒，确认不是坏文件
    let mut c = tools::ffmpeg_cmd().map_err(s)?;
    c.args(["-v", "error", "-xerror", "-i"]).arg(path).args(["-t", "60", "-f", "null", "-"]);
    tools::run(c).map_err(|e| format!("新文件解码检查失败，已取消替换：{e}"))?;
    Ok(())
}

/// 同 group 的类别（video 与 anime 都是视频）
fn kind_group(k: &str) -> &str {
    if k == "anime" || k == "video" {
        "video"
    } else {
        k
    }
}

/// 文件类别变了（如视频转成音频）时清掉旧类别的行；同组保持原类别（番剧仍是番剧）。返回生效的类别
fn switch_kind(conn: &Connection, fid: i64, old: &str, new: &str) -> R<String> {
    if kind_group(old) == kind_group(new) {
        return Ok(old.to_string());
    }
    for t in ["tracks", "videos", "books", "comics"] {
        conn.execute(&format!("DELETE FROM {t} WHERE file_id=?1"), [fid]).map_err(s)?;
    }
    conn.execute("DELETE FROM search_idx WHERE file_id=?1", [fid]).map_err(s)?;
    conn.execute("UPDATE files SET kind=?1 WHERE id=?2", params![new, fid]).map_err(s)?;
    Ok(new.to_string())
}

/// 把库里的一行改到新路径（保持 id：播放次数、收藏、歌单、进度都保留），再重新读标签
fn relink(conn: &Connection, fid: i64, new_path: &Path, dirs: &indexer::Dirs) -> R<()> {
    let p = new_path.to_string_lossy().to_string();
    // 旧的“缺失”记录占着这个路径时先删掉
    conn.execute("DELETE FROM files WHERE path=?1 AND id!=?2 AND missing=1", params![p, fid])
        .map_err(s)?;
    conn.execute("UPDATE files SET path=?1, missing=0 WHERE id=?2", params![p, fid]).map_err(s)?;
    let old: String = conn.query_row("SELECT kind FROM files WHERE id=?1", [fid], |r| r.get(0)).map_err(s)?;
    let Some(new_kind) = parse::classify(new_path) else { return Ok(()) };
    let kind = switch_kind(conn, fid, &old, new_kind)?;
    indexer::index_file(conn, new_path, &kind, dirs).map_err(|e| format!("新文件已替换，但入库失败：{e}"))?;
    Ok(())
}

/// 新文件入库（不在库里时）
fn index_path(conn: &Connection, path: &Path, dirs: &indexer::Dirs) -> R<()> {
    let Some(kind) = parse::classify(path) else { return Ok(()) };
    indexer::index_file(conn, path, kind, dirs).map_err(|e| format!("文件已保存，但入库失败：{e}"))?;
    Ok(())
}

/// 回收站里按原路径找最近的一项
fn trash_item_for(path: &Path) -> R<trash::TrashItem> {
    let items = trash::os_limited::list().map_err(|e| format!("读取回收站失败：{e}"))?;
    let key = norm(path);
    items
        .into_iter()
        .rfind(|it| norm(&it.original_path()) == key)
        .ok_or_else(|| format!("回收站里找不到原文件（可能已被清空或已手动还原）：{}", path.display()))
}

fn restore_from_trash(path: &Path) -> R<()> {
    let item = trash_item_for(path)?;
    trash::os_limited::restore_all([item]).map_err(|e| format!("还原失败：{e}"))
}

// ---------- 命令 ----------

#[derive(Serialize)]
pub struct SettingsView {
    pub out_dir: Option<String>,
    pub out_dir_audio: String,
    pub out_dir_video: String,
    pub concurrency: u32,
    pub plugin_enabled: bool,
    pub plugin_ack: bool,
    pub plugin_dir: String,
    pub plugin_file: Option<String>,
}

#[tauri::command]
pub fn convert_presets() -> Vec<PresetInfo> {
    PRESETS.to_vec()
}

#[tauri::command]
pub fn convert_hw() -> HwStatus {
    HW.lock().clone().unwrap_or_else(HwStatus::pending)
}

#[tauri::command]
pub fn convert_get_settings(st: State<AppState>) -> R<SettingsView> {
    let conn = st.db.lock();
    let s = load_settings(&conn);
    let p = load_plugin(&conn);
    let custom = s.out_dir.clone().filter(|x| !x.trim().is_empty());
    let audio = custom.clone().map(PathBuf::from).unwrap_or_else(|| default_out_dir(true, &st.dirs.data));
    let video = custom.clone().map(PathBuf::from).unwrap_or_else(|| default_out_dir(false, &st.dirs.data));
    Ok(SettingsView {
        out_dir: custom,
        out_dir_audio: audio.to_string_lossy().to_string(),
        out_dir_video: video.to_string_lossy().to_string(),
        concurrency: s.concurrency.unwrap_or(2).clamp(1, 4),
        plugin_enabled: p.enabled && p.ack,
        plugin_ack: p.ack,
        plugin_dir: st.dirs.data.join("plugins").to_string_lossy().to_string(),
        plugin_file: plugin_exe(&st.dirs.data).map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string()),
    })
}

/// out_dir：Some("") 清除自定义目录（恢复默认）；None 表示不改。concurrency 限制在 1–4
#[tauri::command]
pub fn convert_set_settings(st: State<AppState>, out_dir: Option<String>, concurrency: Option<u32>) -> R<()> {
    let conn = st.db.lock();
    let mut cfg = load_settings(&conn);
    if let Some(d) = out_dir {
        cfg.out_dir = if d.trim().is_empty() { None } else { Some(d) };
    }
    if let Some(c) = concurrency {
        cfg.concurrency = Some(c.clamp(1, 4));
    }
    library::set_setting(&conn, "convert", &serde_json::to_value(&cfg).map_err(s)?).map_err(s)?;
    Ok(())
}

/// 打开 / 关闭加密格式插件。开启必须同时确认 ack（“仅用于本人已购买的内容”）
#[tauri::command]
pub fn convert_set_plugin(st: State<AppState>, enabled: bool, ack: bool) -> R<()> {
    if enabled && !ack {
        return Err("请先勾选“仅用于本人已购买的内容”".into());
    }
    let conn = st.db.lock();
    library::set_setting(&conn, "convert_plugin", &serde_json::json!({ "enabled": enabled, "ack": ack })).map_err(s)?;
    Ok(())
}

#[tauri::command]
pub fn convert_add(st: State<AppState>, paths: Vec<String>, preset: String, after: String, out_dir: Option<String>) -> R<Vec<i64>> {
    let p = find_preset(&preset)?;
    if !matches!(after.as_str(), "lib" | "replace" | "side" | "dir") {
        return Err("未知的完成后处理方式".into());
    }
    if paths.is_empty() {
        return Err("没有选择文件".into());
    }
    let mut bad: Vec<String> = vec![];
    for path in &paths {
        let pb = PathBuf::from(path);
        let ok = pb.is_file()
            && match parse::classify(&pb) {
                Some("novel") | Some("manga") | None => false,
                Some("music") => p.kind != "video",
                Some(_) => true,
            };
        if !ok {
            bad.push(path.clone());
        }
    }
    if !bad.is_empty() {
        return Err(format!("以下文件不能用“{}”转换：\n{}", p.name, bad.join("\n")));
    }
    let output = if after == "lib" || after == "dir" {
        out_dir.filter(|d| !d.trim().is_empty())
    } else {
        None
    };
    let conn = st.db.lock();
    let mut ids = vec![];
    for path in &paths {
        conn.execute(
            "INSERT INTO jobs(input,output,preset,after,status,progress,created_at) VALUES(?1,?2,?3,?4,'queued',0,?5)",
            params![path, output, preset, after, db::now()],
        )
        .map_err(s)?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

#[derive(Serialize)]
pub struct JobView {
    pub id: i64,
    pub name: String,
    pub input: String,
    /// 完成后：实际输出文件
    pub output: Option<String>,
    /// 排队 / 失败时：请求的输出目录
    pub out_dir: Option<String>,
    pub preset: String,
    pub preset_name: String,
    pub after: String,
    pub status: String,
    pub progress: f64,
    pub error: Option<String>,
    pub backup: Option<String>,
    pub can_undo: bool,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[tauri::command]
pub fn convert_list(st: State<AppState>) -> R<Vec<JobView>> {
    let conn = st.db.lock();
    let mut stmt = conn
        .prepare("SELECT id,input,output,preset,after,status,progress,error,backup,created_at,finished_at FROM jobs ORDER BY id DESC LIMIT 200")
        .map_err(s)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, Option<i64>>(10)?,
            ))
        })
        .map_err(s)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(s)?;
    let mut out = vec![];
    for (id, input, output, preset, after, status, progress, error, backup, created_at, finished_at) in rows {
        let name = Path::new(&input).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let preset_name = PRESETS
            .iter()
            .find(|p| p.id == preset)
            .map(|p| p.name.to_string())
            .unwrap_or_else(|| preset.clone());
        let done = status == "done" || status == "undone";
        let is_dir_mode = after == "lib" || after == "dir";
        let can_undo = after == "replace" && backup.is_some() && (status == "done" || status == "failed") && output.is_some();
        out.push(JobView {
            id,
            name,
            input,
            output: if done { output.clone() } else { None },
            out_dir: if is_dir_mode && !done { output } else { None },
            preset,
            preset_name,
            after,
            status,
            progress,
            error,
            backup,
            can_undo,
            created_at,
            finished_at,
        });
    }
    Ok(out)
}

/// 取消：排队中的直接标记；运行中的杀掉进程并删除半成品
#[tauri::command]
pub fn convert_cancel(st: State<AppState>, app: AppHandle, id: i64) -> R<()> {
    let running = RUNNING.lock().iter().find(|(j, _)| *j == id).map(|(_, c)| c.clone());
    if let Some(ctl) = running {
        ctl.canceled.store(true, Ordering::SeqCst);
        let mut g = ctl.child.lock();
        if let Some(c) = g.as_mut() {
            let _ = c.kill();
        }
        return Ok(());
    }
    let conn = st.db.lock();
    let n = conn
        .execute(
            "UPDATE jobs SET status='canceled', finished_at=?1 WHERE id=?2 AND status='queued'",
            params![db::now(), id],
        )
        .map_err(s)?;
    if n == 0 {
        return Err("这个任务已经不在排队中".into());
    }
    report(&app, id, 0.0, None, "canceled", None, None);
    Ok(())
}

/// 清掉已结束的任务记录（不删文件）
#[tauri::command]
pub fn convert_clear_done(st: State<AppState>) -> R<usize> {
    let conn = st.db.lock();
    conn.execute("DELETE FROM jobs WHERE status IN ('done','failed','canceled','undone')", [])
        .map_err(s)
}

/// 失败或取消的任务重新排队。替换过原文件的任务不能直接重试（要先撤销）
#[tauri::command]
pub fn convert_retry(st: State<AppState>, app: AppHandle, id: i64) -> R<()> {
    let conn = st.db.lock();
    let n = conn
        .execute(
            "UPDATE jobs SET status='queued', progress=0, error=NULL, finished_at=NULL WHERE id=?1 AND status IN ('failed','canceled') AND backup IS NULL",
            [id],
        )
        .map_err(s)?;
    if n == 0 {
        return Err("只能重试失败或已取消的任务（替换过原文件的任务请先撤销）".into());
    }
    report(&app, id, 0.0, None, "queued", None, None);
    Ok(())
}

/// 撤销替换：从回收站还原原文件，库里的路径改回去；转换出来的新文件保留并作为新条目入库
#[tauri::command]
pub fn convert_undo(st: State<AppState>, app: AppHandle, id: i64) -> R<()> {
    let conn = st.db.lock();
    let (status, after, output, backup): (String, String, Option<String>, Option<String>) = conn
        .query_row("SELECT status,after,output,backup FROM jobs WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|_| "找不到这个任务".to_string())?;
    if after != "replace" || !(status == "done" || status == "failed") {
        return Err("只有“替换原文件”的任务可以撤销".into());
    }
    let (Some(output), Some(backup)) = (output, backup) else {
        return Err("没有记录原文件位置，无法撤销".into());
    };
    let output = PathBuf::from(output);
    let backup = PathBuf::from(backup);
    if !output.is_file() {
        return Err("替换后的新文件已不在原位置，无法撤销".into());
    }
    let same = norm(&output) == norm(&backup);
    if !same && backup.exists() {
        return Err("原位置已经有同名文件，无法还原，请先移走它".into());
    }
    // 先确认回收站里有原文件，再动任何东西
    trash_item_for(&backup)?;
    let fid: Option<i64> = conn
        .query_row("SELECT id FROM files WHERE path=?1", [output.to_string_lossy().to_string()], |r| r.get(0))
        .optional()
        .map_err(s)?;

    // 同名时先把转换出来的文件改个名，给原文件让位
    let mut now_out = output.clone();
    if same {
        let parent = output.parent().ok_or("路径无效")?;
        let stem = output.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        now_out = unique_path(parent, &format!("{stem} (转换版)"), &parse::ext(&output));
        std::fs::rename(&output, &now_out).map_err(|e| format!("无法改名新文件：{e}"))?;
    }
    if let Err(e) = restore_from_trash(&backup) {
        if same {
            let _ = std::fs::rename(&now_out, &output);
        }
        return Err(e);
    }

    let kind_orig = parse::classify(&backup).unwrap_or("video");
    if let Some(fid) = fid {
        conn.execute(
            "UPDATE files SET path=?1, missing=0 WHERE id=?2",
            params![backup.to_string_lossy().to_string(), fid],
        )
        .map_err(s)?;
        let old: String = conn.query_row("SELECT kind FROM files WHERE id=?1", [fid], |r| r.get(0)).map_err(s)?;
        let kind = switch_kind(&conn, fid, &old, kind_orig)?;
        indexer::index_file(&conn, &backup, &kind, &st.dirs).map_err(|e| format!("原文件已还原，但入库失败：{e}"))?;
    }
    if let Some(k) = parse::classify(&now_out) {
        indexer::index_file(&conn, &now_out, k, &st.dirs).map_err(|e| format!("入库失败：{e}"))?;
    }
    conn.execute(
        "UPDATE jobs SET status='undone', output=?1, finished_at=?2 WHERE id=?3",
        params![now_out.to_string_lossy().to_string(), db::now(), id],
    )
    .map_err(s)?;
    drop(conn);
    let _ = app.emit("library://changed", ());
    Ok(())
}

/// 把拖进来的文件夹展开成媒体文件（只保留音乐和视频），文件则原样保留媒体文件
#[tauri::command]
pub fn convert_expand(paths: Vec<String>) -> Vec<String> {
    let mut out = vec![];
    let cancel = AtomicBool::new(false);
    for p in paths {
        let pb = PathBuf::from(&p);
        if pb.is_dir() {
            crate::scanner::enumerate(&pb, &cancel, |f, k| {
                if matches!(k, "music" | "video" | "anime") {
                    out.push(f.to_string_lossy().to_string());
                }
            });
        } else if pb.is_file() && matches!(parse::classify(&pb), Some("music") | Some("video") | Some("anime")) {
            out.push(p);
        }
    }
    out
}

/// 启动时调用一次：清理上次意外退出的任务，启动调度线程，后台检测硬件编码器
pub fn setup(app: &AppHandle) {
    {
        let st = app.state::<AppState>();
        let _ = std::fs::create_dir_all(st.dirs.data.join("plugins"));
        let conn = st.db.lock();
        let _ = conn.execute(
            "UPDATE jobs SET status='failed', error='程序退出时中断，可以重试', finished_at=?1 WHERE status='running'",
            [db::now()],
        );
    }
    let hw_app = app.clone();
    std::thread::spawn(move || {
        *HW.lock() = Some(detect_hw());
        let _ = hw_app.emit("convert://hw", ());
    });
    let dispatch_app = app.clone();
    std::thread::spawn(move || dispatcher(dispatch_app));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn probe_audio_with_cover(codec: &str) -> Value {
        json!({
            "format": {"duration": "200.5"},
            "streams": [
                {"index": 0, "codec_type": "audio", "codec_name": codec},
                {"index": 1, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}}
            ]
        })
    }

    fn probe_video(codec_a: &str) -> Value {
        json!({
            "format": {"duration": "60.0"},
            "streams": [
                {"index": 0, "codec_type": "video", "codec_name": "h264"},
                {"index": 1, "codec_type": "audio", "codec_name": codec_a}
            ]
        })
    }

    #[test]
    fn progress_line_parsing() {
        assert_eq!(parse_progress_line("out_time_us=12345678"), Some(("out_time_us", "12345678")));
        assert_eq!(parse_progress_line("progress=end"), Some(("progress", "end")));
        assert_eq!(parse_progress_line("garbage"), None);
        assert_eq!(parse_speed("1.25x"), Some(1.25));
        assert_eq!(parse_speed(" 0.5x "), Some(0.5));
        assert_eq!(parse_speed("N/A"), None);
        assert!((progress_ratio(30_000_000, 60.0) - 0.5).abs() < 1e-9);
        assert_eq!(progress_ratio(100_000_000, 0.0), 0.0);
        assert!(progress_ratio(999_000_000, 1.0) <= 0.99);
    }

    #[test]
    fn encoder_list_and_pick() {
        let list = parse_encoder_list(" V....D h264_nvenc   NVIDIA\n V..... libx264  x\n A....D aac  AAC\n");
        assert!(list.contains(&"h264_nvenc".to_string()));
        assert!(list.contains(&"aac".to_string()));
        let probes = vec![
            HwProbe {
                name: "h264_nvenc".into(),
                ok: false,
            },
            HwProbe {
                name: "h264_qsv".into(),
                ok: true,
            },
            HwProbe {
                name: "hevc_nvenc".into(),
                ok: true,
            },
        ];
        assert_eq!(pick_encoder(&probes, false), "h264_qsv");
        assert_eq!(pick_encoder(&probes, true), "hevc_nvenc");
        assert_eq!(pick_encoder(&[], true), "libx265");
    }

    #[test]
    fn mp3_keeps_cover_and_metadata() {
        let p = probe_audio_with_cover("flac");
        let b = build_args("mp3", &p, &HwEnc::soft(), None).unwrap();
        assert_eq!(b.ext, "mp3");
        let joined = b.args.join(" ");
        assert!(joined.contains("-map_metadata 0"));
        assert!(joined.contains("libmp3lame -b:a 320k"));
        assert!(joined.contains("-map 0:1 -c:v copy -disposition:v:0 attached_pic"));
    }

    #[test]
    fn no_cover_when_none_or_opus() {
        let p = json!({"streams": [{"index": 0, "codec_type": "audio", "codec_name": "mp3"}]});
        let b = build_args("flac", &p, &HwEnc::soft(), None).unwrap();
        assert!(!b.args.iter().any(|a| a == "attached_pic"));
        let c = build_args("opus", &probe_audio_with_cover("flac"), &HwEnc::soft(), None).unwrap();
        assert!(!c.args.iter().any(|a| a == "attached_pic"));
        assert_eq!(c.ext, "opus");
    }

    #[test]
    fn video_encoder_follows_hw() {
        let p = probe_video("aac");
        let sw = build_args("mp4_h264", &p, &HwEnc::soft(), None).unwrap();
        assert!(sw.args.contains(&"libx264".to_string()));
        let hw = HwEnc {
            h264: "h264_nvenc".into(),
            hevc: "hevc_nvenc".into(),
        };
        let g = build_args("mp4_h264", &p, &hw, None).unwrap();
        assert!(g.args.contains(&"h264_nvenc".to_string()));
        let h = build_args("mp4_h265", &p, &hw, None).unwrap();
        assert!(h.args.contains(&"hevc_nvenc".to_string()));
        assert!(h.args.contains(&"hvc1".to_string()));
    }

    #[test]
    fn video_presets_need_video() {
        let audio = json!({"streams": [{"index": 0, "codec_type": "audio", "codec_name": "mp3"}]});
        assert!(build_args("gif", &audio, &HwEnc::soft(), None).is_err());
        assert!(build_args("mp4_h264", &audio, &HwEnc::soft(), None).is_err());
        // 没有视频的音频走响度均衡的音频分支
        let b = build_args("loudnorm", &audio, &HwEnc::soft(), None).unwrap();
        assert_eq!(b.ext, "flac");
    }

    #[test]
    fn burn_sub_needs_subtitle() {
        let p = probe_video("aac");
        assert!(build_args("burn_sub", &p, &HwEnc::soft(), None).is_err());
        let b = build_args("burn_sub", &p, &HwEnc::soft(), Some("sub.srt")).unwrap();
        assert!(b.args.contains(&"subtitles=sub.srt".to_string()));
    }

    #[test]
    fn extract_copy_extension() {
        assert_eq!(copy_ext(Some("aac")), "m4a");
        assert_eq!(copy_ext(Some("opus")), "opus");
        assert_eq!(copy_ext(Some("pcm_s24le")), "wav");
        assert_eq!(copy_ext(Some("weird")), "mka");
        assert_eq!(copy_ext(None), "mka");
        let b = build_args("extract_copy", &probe_video("aac"), &HwEnc::soft(), None).unwrap();
        assert_eq!(b.ext, "m4a");
        assert!(b.args.join(" ").contains("-c:a copy"));
    }

    #[test]
    fn remux_drops_subtitles() {
        let b = build_args("remux", &probe_video("aac"), &HwEnc::soft(), None).unwrap();
        assert_eq!(b.ext, "mp4");
        assert!(b.args.join(" ").contains("-c copy"));
        assert!(!b.args.iter().any(|a| a.contains(":s")));
    }

    #[test]
    fn duration_from_format_or_streams() {
        assert!((duration_of(&probe_video("aac")) - 60.0).abs() < 1e-9);
        let only_streams = json!({"streams": [{"duration": "12.5"}, {"duration": "30"}]});
        assert!((duration_of(&only_streams) - 30.0).abs() < 1e-9);
        assert_eq!(duration_of(&json!({})), 0.0);
    }

    #[test]
    fn unique_path_adds_number() {
        let dir = std::env::temp_dir().join(format!("mpc-convert-test-{}", db::now()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "雾港", "mp3");
        assert_eq!(first.file_name().unwrap(), "雾港.mp3");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "雾港", "mp3");
        assert_eq!(second.file_name().unwrap(), "雾港 (1).mp3");
        std::fs::write(&second, b"x").unwrap();
        // 换扩展名后不冲突
        assert_eq!(unique_path(&dir, "雾港", "flac").file_name().unwrap(), "雾港.flac");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claim_moves_queued_to_running() {
        let c = db::open_memory().unwrap();
        c.execute(
            "INSERT INTO jobs(input,preset,after,status,created_at) VALUES('a.flac','mp3','lib','queued',1),('b.flac','mp3','lib','queued',1)",
            [],
        )
        .unwrap();
        assert_eq!(claim(&c), Some(1));
        assert_eq!(claim(&c), Some(2));
        assert_eq!(claim(&c), None);
        let st: String = c.query_row("SELECT status FROM jobs WHERE id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(st, "running");
    }

    #[test]
    fn switch_kind_keeps_group() {
        let c = db::open_memory().unwrap();
        c.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES('C:\\x.mp4','anime',1,1,1)", [])
            .unwrap();
        let fid = c.last_insert_rowid();
        // 番剧 → 视频：同组，保持番剧
        assert_eq!(switch_kind(&c, fid, "anime", "video").unwrap(), "anime");
        // 视频 → 音乐：不同组，改成音乐
        assert_eq!(switch_kind(&c, fid, "anime", "music").unwrap(), "music");
        let k: String = c.query_row("SELECT kind FROM files WHERE id=?1", [fid], |r| r.get(0)).unwrap();
        assert_eq!(k, "music");
    }

    #[test]
    fn parse_preset_ids_unique() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        assert!(find_preset("mp3").is_ok());
        assert!(find_preset("nope").is_err());
    }
}

/// 删除一条已结束的记录（不动任何文件）
#[tauri::command]
pub fn convert_remove(st: State<AppState>, id: i64) -> R<usize> {
    st.db
        .lock()
        .execute("DELETE FROM jobs WHERE id=?1 AND status IN ('done','failed','canceled','undone')", [id])
        .map_err(s)
}
