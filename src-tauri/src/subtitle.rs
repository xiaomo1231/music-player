//! AI 字幕：提取音轨、检测人声、切块识别、断句对齐、可选翻译、导出 srt / ass / 封装 mkv
//! 命令前缀 sub_。任务进内存队列，同一时间只跑一个，进度事件 sub://progress。
//! 纯函数（静音解析、切块、断句、对齐、渲染）都有单元测试。
use crate::{ai, ai::Segment, library, tools, AppState};
use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;

const PROGRESS_EVENT: &str = "sub://progress";
/// 每块最长秒数（语音识别接口对单次时长的常见限制）
const MAX_CHUNK_SECS: f64 = 60.0;
/// 单块上传最大字节数
const MAX_UPLOAD_BYTES: f64 = 20.0 * 1024.0 * 1024.0;
/// mp3 64 kbps 约 8000 字节 / 秒
const MP3_BYTES_PER_SEC: f64 = 8000.0;
const SILENCE_FILTER: &str = "silencedetect=noise=-35dB:d=0.4";
/// 静音间隔小于这个值的人声区间合并
const VOICE_MERGE_GAP: f64 = 0.8;
/// 一次翻译送给模型的句数
const TRANS_BATCH: usize = 40;
/// 只用于 sub_estimate：每分钟音频的翻译用量估算（输入 + 输出，含提示词）
const TOKENS_PER_MINUTE: f64 = 600.0;

// ---------- 数据类型 ----------

/// 人声区间 / 块（秒）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Iv {
    pub start: f64,
    pub end: f64,
}

/// 一条字幕：原文 src，译文 zh（未翻译时为空）
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Line {
    pub start: f64,
    pub end: f64,
    pub src: String,
    #[serde(default)]
    pub zh: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SubOptions {
    /// auto / ja / en / zh（空或 auto 表示自动检测）
    pub lang: Option<String>,
    /// both（双语，中文在上）/ zh / src
    pub target: Option<String>,
    /// srt / ass / mkv
    pub output: Option<String>,
    /// 术语表，每行 原文=译文；存进 settings，整季共用
    pub glossary: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct JobView {
    pub id: u64,
    pub video_id: i64,
    pub title: String,
    /// queued / running / done / error / cancelled
    pub status: String,
    /// extract / vad / asr / align / translate / done / error
    pub step: String,
    pub progress: f64,
    pub message: String,
    pub outputs: Vec<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct SubVideo {
    pub id: i64,
    pub title: String,
    pub series_id: Option<i64>,
    pub series_title: String,
    pub season: Option<i64>,
    pub episode: Option<f64>,
    pub subs: String,
    pub duration: f64,
    pub path: String,
}

#[derive(Serialize)]
pub struct Estimate {
    pub minutes: f64,
    pub est_tokens: u64,
    pub monthly_limit: u64,
    pub used_tokens: u64,
    pub over_limit: bool,
}

// ---------- 纯函数：静音 / 人声 / 切块 ----------

fn num_after(s: &str) -> Option<f64> {
    let t = s.trim_start();
    let end = t.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(t.len());
    t[..end].parse().ok()
}

/// 解析 ffmpeg silencedetect 的日志，返回静音区间（结尾没有 end 的静音 end 记为 0，由 voice_from_silence 兜底）
pub fn parse_silences(log: &str) -> Vec<Iv> {
    let mut out = Vec::new();
    let mut open: Option<f64> = None;
    for line in log.lines() {
        if let Some(p) = line.find("silence_start:") {
            open = num_after(&line[p + "silence_start:".len()..]);
        } else if let Some(p) = line.find("silence_end:") {
            if let Some(e) = num_after(&line[p + "silence_end:".len()..]) {
                let s = open.take().unwrap_or(0.0);
                out.push(Iv { start: s.max(0.0), end: e });
            }
        }
    }
    if let Some(s) = open {
        out.push(Iv {
            start: s.max(0.0),
            end: f64::MAX,
        });
    }
    out
}

/// 静音的补集就是人声；过短的（<0.3 秒）丢掉
pub fn voice_from_silence(silences: &[Iv], total: f64) -> Vec<Iv> {
    let mut sil = silences.to_vec();
    sil.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out = Vec::new();
    let mut cur = 0.0_f64;
    for s in sil {
        if s.start > cur {
            out.push(Iv {
                start: cur,
                end: s.start.min(total),
            });
        }
        cur = cur.max(s.end.min(total.max(cur)));
    }
    if total > cur {
        out.push(Iv { start: cur, end: total });
    }
    out.retain(|v| v.end - v.start >= 0.3);
    out
}

/// 间隔不超过 gap 秒的区间合并
pub fn merge_intervals(v: &[Iv], gap: f64) -> Vec<Iv> {
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<Iv> = Vec::new();
    for x in v {
        if let Some(last) = out.last_mut() {
            if x.start - last.end <= gap {
                last.end = last.end.max(x.end);
                continue;
            }
        }
        out.push(x);
    }
    out
}

/// 把人声区间打包成识别块：每块不超过 max_len 秒且估算字节不超过 max_bytes；
/// 超长的连续人声先按上限切开，再尽量往后合并（中间的静音一并送去）。
pub fn plan_chunks(voice: &[Iv], max_len: f64, bytes_per_sec: f64, max_bytes: f64) -> Vec<Iv> {
    let limit = max_len.min(max_bytes / bytes_per_sec).max(1.0);
    let mut pieces = Vec::new();
    for v in voice {
        let mut s = v.start;
        while v.end - s > limit {
            pieces.push(Iv { start: s, end: s + limit });
            s += limit;
        }
        if v.end > s {
            pieces.push(Iv { start: s, end: v.end });
        }
    }
    let mut out: Vec<Iv> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(c) if p.end - c.start <= limit => c.end = p.end,
            _ => out.push(p),
        }
    }
    out
}

/// 找最近的人声边界：起点用区间开始，终点用区间结束
fn nearest(t: f64, voice: &[Iv], is_start: bool) -> Option<f64> {
    let mut best: Option<f64> = None;
    for v in voice {
        let c = if is_start { v.start } else { v.end };
        if best.map_or(true, |b| (c - t).abs() < (b - t).abs()) {
            best = Some(c);
        }
    }
    best
}

/// 在 tol 秒以内贴到最近的人声边界，否则保持原值
pub fn snap(t: f64, voice: &[Iv], is_start: bool, tol: f64) -> f64 {
    nearest(t, voice, is_start).filter(|c| (c - t).abs() <= tol).unwrap_or(t)
}

// ---------- 纯函数：断句与对齐 ----------

pub fn is_cjk_text(s: &str) -> bool {
    let (mut cjk, mut other) = (0usize, 0usize);
    for c in s.chars().filter(|c| !c.is_whitespace()) {
        if (c as u32) >= 0x2E80 {
            cjk += 1;
        } else if c.is_alphanumeric() {
            other += 1;
        }
    }
    cjk > 0 && cjk >= other
}

#[derive(Clone, Copy)]
struct Limits {
    max_chars: usize,
    max_secs: f64,
    min_secs: f64,
}

/// 中日文每条最多 2 行 × 20 字，英文 2 行 × 42 字符；时长 1–7 秒
fn limits(cjk: bool) -> Limits {
    if cjk {
        Limits {
            max_chars: 40,
            max_secs: 7.0,
            min_secs: 1.0,
        }
    } else {
        Limits {
            max_chars: 84,
            max_secs: 7.0,
            min_secs: 1.0,
        }
    }
}

fn is_term(c: char) -> bool {
    matches!(c, '。' | '！' | '？' | '!' | '?' | '；' | ';' | '…')
}

fn is_close(c: char) -> bool {
    matches!(c, '」' | '』' | '”' | '’' | '"' | '\'' | '）' | ')')
}

fn is_soft(c: char) -> bool {
    matches!(c, '，' | ',' | '、' | ' ' | '：' | ':')
}

/// 按句末标点切句（省略号、连续感叹号、引号收尾都算一句）
pub fn split_sentences(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.trim().chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let prev = if i > 0 { chars[i - 1] } else { ' ' };
        let next = chars.get(i + 1).copied();
        let boundary = is_term(c) || (c == '.' && next.map_or(true, |n| n.is_whitespace())) || (is_close(c) && (is_term(prev) || prev == '.'));
        if !boundary {
            continue;
        }
        if let Some(n) = next {
            if is_term(n) || is_close(n) {
                continue;
            }
        }
        let t = cur.trim().to_string();
        if !t.is_empty() {
            out.push(t);
        }
        cur.clear();
    }
    let t = cur.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
    out
}

/// 把一句话尽量均匀地切成 n 段，切点优先放在逗号、空格后面
pub fn split_even(text: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = text.trim().chars().collect();
    let len = chars.len();
    let n = n.min(len);
    if n <= 1 {
        return vec![text.trim().to_string()];
    }
    let step = len as f64 / n as f64;
    let win = ((step / 4.0) as usize).max(1);
    let mut cuts = vec![0usize];
    for k in 1..n {
        let last = *cuts.last().unwrap_or(&0);
        let max_cut = len - (n - k);
        let target = ((step * k as f64).round() as usize).clamp(last + 1, max_cut);
        let lo = target.saturating_sub(win).max(last + 1);
        let hi = (target + win).min(max_cut);
        let cut = (lo..=hi)
            .filter(|&i| is_soft(chars[i - 1]))
            .min_by_key(|&i| i.abs_diff(target))
            .unwrap_or(target);
        cuts.push(cut);
    }
    cuts.push(len);
    cuts.windows(2)
        .map(|w| chars[w[0]..w[1]].iter().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn weight(s: &str) -> f64 {
    s.chars().filter(|c| !c.is_whitespace()).count().max(1) as f64
}

fn piece_count(text: &str, secs: f64, lim: Limits) -> usize {
    let len = text.chars().count();
    let by_len = if len <= lim.max_chars {
        1
    } else {
        ((len as f64) / (lim.max_chars as f64 * 0.8)).ceil() as usize
    };
    let by_time = if secs > lim.max_secs { (secs / lim.max_secs).ceil() as usize } else { 1 };
    by_len.max(by_time).max(1)
}

/// 两段之间是否有停顿：时间差超过 0.7 秒，或中间夹着一段静音（人声区间断开）
fn has_pause(a: f64, b: f64, voice: &[Iv]) -> bool {
    b - a > 0.7 || voice.iter().any(|v| v.end > a + 0.05 && v.end < b - 0.05)
}

struct Piece {
    start: f64,
    end: f64,
    text: String,
}

/// 断句与对齐：先按标点切句并按字数分配时间，再按停顿合并过短的句子，
/// 控制长度与时长，最后把起止时间贴到人声边界上
pub fn align_segments(segs: &[Segment], voice: &[Iv], cjk: bool) -> Vec<Line> {
    let lim = limits(cjk);
    let mut pieces: Vec<Piece> = Vec::new();
    for s in segs {
        let text = s.text.trim();
        if text.is_empty() {
            continue;
        }
        let s_start = s.start;
        let s_end = if s.end > s.start + 0.05 { s.end } else { s.start + 0.5 };
        let sents = split_sentences(text);
        let total_w: f64 = sents.iter().map(|x| weight(x)).sum::<f64>().max(1.0);
        let dur = s_end - s_start;
        let mut t = s_start;
        for (k, sent) in sents.iter().enumerate() {
            let st_sent = t;
            let en_sent = if k + 1 == sents.len() { s_end } else { t + dur * weight(sent) / total_w };
            t = en_sent;
            let n = piece_count(sent, en_sent - st_sent, lim);
            let subs = split_even(sent, n);
            let sub_total: f64 = subs.iter().map(|x| weight(x)).sum::<f64>().max(1.0);
            let mut u = st_sent;
            let count = subs.len();
            for (j, sub_t) in subs.into_iter().enumerate() {
                let ue = if j + 1 == count {
                    en_sent
                } else {
                    u + (en_sent - st_sent) * weight(&sub_t) / sub_total
                };
                pieces.push(Piece {
                    start: u,
                    end: ue,
                    text: sub_t,
                });
                u = ue;
            }
        }
    }

    // 合并：与前一条相连、没有停顿、且前一条过短（时长或字数），并且合并后不超过上限
    let mut cues: Vec<(f64, f64, String)> = Vec::new();
    let mut prev_end: Option<f64> = None;
    for p in pieces {
        let pause = prev_end.is_some_and(|pe| has_pause(pe, p.start, voice));
        prev_end = Some(p.end);
        if let Some(last) = cues.last_mut() {
            let short = (last.1 - last.0) < lim.min_secs || last.2.chars().count() < 6;
            let len = last.2.chars().count() + p.text.chars().count() + 1;
            let dur = p.end - last.0;
            if !pause && short && len <= lim.max_chars && dur <= lim.max_secs {
                last.1 = p.end;
                if cjk {
                    last.2.push_str(&p.text);
                } else {
                    last.2.push(' ');
                    last.2.push_str(&p.text);
                }
                continue;
            }
        }
        cues.push((p.start, p.end, p.text));
    }

    // 贴到人声边界，保证不重叠
    let mut out: Vec<Line> = Vec::new();
    for (s, e, t) in cues {
        let mut start = snap(s, voice, true, 0.5);
        let mut end = snap(e, voice, false, 0.5);
        if let Some(p) = out.last() {
            if start < p.end {
                start = p.end;
            }
        }
        if end <= start {
            end = start + lim.min_secs;
        }
        out.push(Line {
            start,
            end,
            src: t,
            zh: String::new(),
        });
    }
    // 时长下限 / 上限，且不越过下一条的开始
    for i in 0..out.len() {
        let next_start = out.get(i + 1).map(|n| n.start);
        let l = &mut out[i];
        if l.end - l.start < lim.min_secs {
            l.end = l.start + lim.min_secs;
        }
        if l.end - l.start > lim.max_secs {
            l.end = l.start + lim.max_secs;
        }
        if let Some(ns) = next_start {
            if l.end > ns && ns > l.start {
                l.end = ns;
            }
        }
    }
    out
}

/// 已有字幕：整体偏移（各条开始到最近人声开始的差，取中位数，只看 3 秒以内的），再逐条贴到人声边界
pub fn realign(cues: &[Line], voice: &[Iv]) -> Vec<Line> {
    let mut diffs: Vec<f64> = cues
        .iter()
        .filter_map(|c| {
            let v = nearest(c.start, voice, true)?;
            let d = v - c.start;
            (d.abs() <= 3.0).then_some(d)
        })
        .collect();
    diffs.sort_by(|a, b| a.total_cmp(b));
    let shift = if diffs.is_empty() { 0.0 } else { diffs[diffs.len() / 2] };
    let mut out: Vec<Line> = Vec::new();
    for c in cues {
        let (s0, e0) = (c.start + shift, c.end + shift);
        let mut s = snap(s0, voice, true, 0.5);
        let mut e = snap(e0, voice, false, 0.5);
        if let Some(p) = out.last() {
            if s < p.end {
                s = p.end;
            }
        }
        if e <= s {
            e = s + (e0 - s0).max(0.5);
        }
        out.push(Line {
            start: s,
            end: e,
            src: c.src.clone(),
            zh: c.zh.clone(),
        });
    }
    out
}

// ---------- 纯函数：字幕文件 ----------

pub fn fmt_srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000)
}

pub fn fmt_ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

/// target: both（中文在上、原文在下）/ zh / src
pub fn render_srt(lines: &[Line], target: &str) -> String {
    let mut s = String::new();
    for (i, l) in lines.iter().enumerate() {
        let text = match target {
            "src" => l.src.clone(),
            "zh" => {
                if l.zh.is_empty() {
                    l.src.clone()
                } else {
                    l.zh.clone()
                }
            }
            _ => {
                if l.zh.is_empty() {
                    l.src.clone()
                } else {
                    format!("{}\n{}", l.zh, l.src)
                }
            }
        };
        s.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, fmt_srt_time(l.start), fmt_srt_time(l.end), text));
    }
    s
}

fn ass_escape(t: &str) -> String {
    t.replace('\n', "\\N").replace('{', "(").replace('}', ")")
}

fn dialogue(out: &mut String, start: &str, end: &str, style: &str, text: &str) {
    out.push_str(&format!("Dialogue: 0,{start},{end},{style},,0,0,0,,{}\n", ass_escape(text)));
}

/// ass：中文用 Zh 样式（靠上），原文用 Src 样式（靠下）；同一时间显示
pub fn render_ass(lines: &[Line], target: &str) -> String {
    let mut s = String::from(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\nWrapStyle: 0\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Zh,Microsoft YaHei,64,&H00FFFFFF,&H000000FF,&H00000000,&H64000000,0,0,0,0,100,100,0,0,1,3,1,2,40,40,90,1\n\
         Style: Src,Arial,46,&H00D9D9D9,&H000000FF,&H00000000,&H64000000,0,0,0,0,100,100,0,0,1,2.5,1,2,40,40,30,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for l in lines {
        let (st, en) = (fmt_ass_time(l.start), fmt_ass_time(l.end));
        let has_zh = !l.zh.is_empty();
        if target != "src" && has_zh {
            dialogue(&mut s, &st, &en, "Zh", &l.zh);
        }
        if target != "zh" || !has_zh {
            dialogue(&mut s, &st, &en, "Src", &l.src);
        }
    }
    s
}

pub fn parse_srt_time(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// 读 srt：每块取时间行和之后的文字（作为原文 src）
pub fn parse_srt(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let lines: Vec<&str> = block.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect();
        let Some(idx) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let times: Vec<&str> = lines[idx].split("-->").collect();
        if times.len() != 2 {
            continue;
        }
        let (Some(s), Some(e)) = (parse_srt_time(times[0]), parse_srt_time(times[1])) else {
            continue;
        };
        out.push(Line {
            start: s,
            end: e,
            src: lines[idx + 1..].join("\n"),
            zh: String::new(),
        });
    }
    out
}

/// 解析翻译结果：应当是 {"lines": [...]}（或直接数组），条数必须与输入一致
pub fn parse_translations(reply: &str, expected: usize) -> Result<Vec<String>> {
    let v = ai::extract_json(reply)?;
    let arr: Vec<serde_json::Value> = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(o) => ["lines", "translations", "zh"]
            .iter()
            .find_map(|k| o.get(*k))
            .and_then(|x| x.as_array())
            .cloned()
            .ok_or_else(|| anyhow!("翻译结果缺少 lines 数组"))?,
        _ => return Err(anyhow!("翻译结果格式不对")),
    };
    let out: Vec<String> = arr.iter().map(|x| x.as_str().unwrap_or("").trim().to_string()).collect();
    if out.len() != expected {
        return Err(anyhow!("翻译条数不对：应为 {expected} 条，实际 {} 条", out.len()));
    }
    Ok(out)
}

// ---------- 队列与任务状态 ----------

struct Job {
    view: JobView,
    input: PathBuf,
    series_id: Option<i64>,
    options: SubOptions,
    lines: Vec<Line>,
    cancel: bool,
}

struct Queue {
    next: u64,
    jobs: Vec<Job>,
    queue: VecDeque<u64>,
    running: bool,
}

static STATE: Lazy<Mutex<Queue>> = Lazy::new(|| {
    Mutex::new(Queue {
        next: 1,
        jobs: Vec::new(),
        queue: VecDeque::new(),
        running: false,
    })
});

fn with_job<T>(id: u64, f: impl FnOnce(&mut Job) -> T) -> Option<T> {
    let mut s = STATE.lock();
    s.jobs.iter_mut().find(|j| j.view.id == id).map(f)
}

fn is_cancelled(id: u64) -> bool {
    with_job(id, |j| j.cancel).unwrap_or(true)
}

fn check_cancel(id: u64) -> Result<()> {
    if is_cancelled(id) {
        Err(anyhow!("已取消"))
    } else {
        Ok(())
    }
}

fn progress(app: &AppHandle, id: u64, step: &str, progress: f64, message: &str) {
    let _ = with_job(id, |j| {
        j.view.step = step.to_string();
        j.view.progress = progress;
        j.view.message = message.to_string();
    });
    let _ = app.emit(PROGRESS_EVENT, json!({ "job": id, "step": step, "progress": progress, "message": message }));
}

fn new_job(video: &library::Video, options: &SubOptions) -> u64 {
    let mut s = STATE.lock();
    let id = s.next;
    s.next += 1;
    s.jobs.push(Job {
        view: JobView {
            id,
            video_id: video.id,
            title: video.title.clone(),
            status: "queued".into(),
            step: String::new(),
            progress: 0.0,
            message: "排队中".into(),
            outputs: Vec::new(),
            error: None,
        },
        input: PathBuf::from(&video.path),
        series_id: video.series_id,
        options: options.clone(),
        lines: Vec::new(),
        cancel: false,
    });
    s.queue.push_back(id);
    id
}

/// 没有任务在跑时启动一个后台工作循环，依次处理队列
fn kick(app: &AppHandle) {
    let start = {
        let mut s = STATE.lock();
        if s.running || s.queue.is_empty() {
            false
        } else {
            s.running = true;
            true
        }
    };
    if !start {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            // 取任务和结束标记在同一把锁里做，避免漏掉新排进来的任务
            let next = {
                let mut s = STATE.lock();
                match s.queue.pop_front() {
                    Some(id) => Some(id),
                    None => {
                        s.running = false;
                        None
                    }
                }
            };
            let Some(id) = next else { break };
            run_job(&app, id).await;
        }
    });
}

async fn run_job(app: &AppHandle, id: u64) {
    let skip = with_job(id, |j| {
        if j.cancel {
            true
        } else {
            j.view.status = "running".into();
            false
        }
    })
    .unwrap_or(true);
    if skip {
        return;
    }
    let result = pipeline(app, id).await;
    let cancelled = is_cancelled(id);
    let video_id = with_job(id, |j| j.view.video_id).unwrap_or(0);
    if let Ok(w) = work_dir(app, video_id) {
        let _ = std::fs::remove_file(w.join("audio.flac"));
    }
    match result {
        Ok((outs, lines)) => {
            let _ = with_job(id, |j| {
                j.view.status = "done".into();
                j.view.progress = 1.0;
                j.view.message = "字幕已生成，可以逐句修改".into();
                j.view.outputs = outs;
                j.lines = lines;
            });
            progress(app, id, "done", 1.0, "字幕已生成，可以逐句修改");
        }
        Err(e) => {
            let (status, msg) = if cancelled {
                ("cancelled", "已取消".to_string())
            } else {
                ("error", format!("{e:#}"))
            };
            let _ = with_job(id, |j| {
                j.view.status = status.into();
                j.view.error = if cancelled { None } else { Some(msg.clone()) };
                j.view.message = msg.clone();
            });
            progress(app, id, "error", 0.0, &msg);
        }
    }
}

// ---------- 流水线 ----------

fn es<E: Display>(e: E) -> String {
    format!("{e:#}")
}

fn path_s(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn ff(args: &[&str]) -> Result<()> {
    let mut c = tools::ffmpeg_cmd()?;
    c.args(args);
    tools::run(c)?;
    Ok(())
}

/// 跑 ffmpeg 并返回 stderr（silencedetect 的结果在 stderr 里）
fn ff_stderr(args: &[&str]) -> Result<String> {
    let mut c = tools::ffmpeg_cmd()?;
    c.args(args);
    let out = c.stdout(Stdio::piped()).stderr(Stdio::piped()).output().context("启动 ffmpeg 失败")?;
    let log = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        let tail: Vec<&str> = log.lines().rev().take(6).collect();
        return Err(anyhow!("{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    Ok(log)
}

/// 在后台线程里跑阻塞操作（ffmpeg、写文件），不占住异步运行时
async fn blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| anyhow!("{e}"))?
}

fn work_dir(app: &AppHandle, video_id: i64) -> Result<PathBuf> {
    let st = app.state::<AppState>();
    let base = st.dirs.thumbs.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| st.dirs.data.clone());
    let dir = base.join("subs").join(video_id.to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn extract_audio(input: &Path, audio: &Path) -> Result<()> {
    let (a, o) = (path_s(input), path_s(audio));
    ff(&["-i", &a, "-vn", "-ac", "1", "-ar", "16000", "-c:a", "flac", &o])
}

/// 音频上的人声区间。返回 (总时长, 人声区间)
fn detect_voice(audio: &Path) -> Result<(f64, Vec<Iv>)> {
    let a = path_s(audio);
    let log = ff_stderr(&["-i", &a, "-af", SILENCE_FILTER, "-f", "null", "-"])?;
    let mut silences = parse_silences(&log);
    let probed = tools::probe(audio)?["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let total = if probed > 0.0 {
        probed
    } else {
        silences.iter().filter(|s| s.end < f64::MAX).map(|s| s.end).fold(0.0, f64::max)
    };
    for s in silences.iter_mut() {
        if s.end == f64::MAX {
            s.end = total;
        }
    }
    let voice = merge_intervals(&voice_from_silence(&silences, total), VOICE_MERGE_GAP);
    Ok((total, voice))
}

fn make_chunk(audio: &Path, start: f64, len: f64, out: &Path) -> Result<()> {
    let (a, o) = (path_s(audio), path_s(out));
    let (ss, tt) = (format!("{start:.3}"), format!("{len:.3}"));
    ff(&["-ss", &ss, "-t", &tt, "-i", &a, "-c:a", "libmp3lame", "-b:a", "64k", "-ac", "1", &o])
}

fn norm_lang(l: Option<&str>) -> Option<String> {
    match l {
        None | Some("") | Some("auto") => None,
        Some(x) => Some(x.to_string()),
    }
}

fn tag_of(target: &str) -> &'static str {
    if target == "src" {
        "src"
    } else {
        "zh"
    }
}

/// 术语表在 settings 里的键：整季（同一个番剧）共用一份，独立视频各自一份
fn glossary_key(series_id: Option<i64>, video_id: i64) -> String {
    match series_id {
        Some(s) => format!("glossary:s{s}"),
        None => format!("glossary:v{video_id}"),
    }
}

fn load_glossary(app: &AppHandle, key: &str) -> String {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    library::get_setting(&conn, key)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn set_subs_ai(app: &AppHandle, video_id: i64) -> Result<()> {
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    conn.execute("UPDATE videos SET subs='ai' WHERE file_id=?1", [video_id])?;
    Ok(())
}

const SYSTEM_TRANSLATE: &str = "你是专业的字幕译者。把 user 消息里 lines 的每一句翻译成自然的简体中文口语。\
规则：1. 只输出 JSON 对象 {\"lines\": [...]}，数组长度必须与输入的 lines 完全相同，一一对应，不合并、不拆分、不增删；\
2. 人名、称呼、术语严格按 glossary 的写法（每行 原文=译文）；\
3. context_before / context_after 只用来理解语境，不要翻译它们；4. 语气自然，不加解释。";

async fn translate_all(app: &AppHandle, id: u64, lines: &mut [Line], glossary: &str) -> Result<()> {
    let n = lines.len();
    let batches = n.div_ceil(TRANS_BATCH);
    for b in 0..batches {
        check_cancel(id)?;
        let s = b * TRANS_BATCH;
        let e = (s + TRANS_BATCH).min(n);
        progress(app, id, "translate", b as f64 / batches as f64, &format!("翻译第 {}/{} 批", b + 1, batches));
        let src: Vec<String> = lines[s..e].iter().map(|l| l.src.clone()).collect();
        let before: Vec<String> = lines[s.saturating_sub(3)..s].iter().map(|l| l.src.clone()).collect();
        let after: Vec<String> = lines[e..(e + 2).min(n)].iter().map(|l| l.src.clone()).collect();
        let user = json!({ "glossary": glossary, "context_before": before, "context_after": after, "lines": src }).to_string();
        let mut result: Option<Vec<String>> = None;
        let mut last_err: Option<anyhow::Error> = None;
        for _ in 0..2 {
            let reply = ai::chat_simple(app, SYSTEM_TRANSLATE, &user, true).await?;
            match parse_translations(&reply, src.len()) {
                Ok(v) => {
                    result = Some(v);
                    break;
                }
                Err(err) => last_err = Some(err),
            }
        }
        let v = match result {
            Some(v) => v,
            None => return Err(last_err.unwrap_or_else(|| anyhow!("翻译失败"))),
        };
        for (l, z) in lines[s..e].iter_mut().zip(v) {
            l.zh = z;
        }
    }
    Ok(())
}

/// 写出字幕文件，返回路径。mkv 先写临时 srt 再封装（不重新编码）
fn write_outputs(input: &Path, work: &Path, target: &str, output: &str, lines: &[Line]) -> Result<Vec<String>> {
    let dir = input.parent().unwrap_or(Path::new(".")).to_path_buf();
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "subtitle".into());
    match output {
        "ass" => {
            let p = dir.join(format!("{stem}.{}.ass", tag_of(target)));
            std::fs::write(&p, render_ass(lines, target))?;
            Ok(vec![path_s(&p)])
        }
        "mkv" => {
            let tmp = work.join("subs.srt");
            std::fs::write(&tmp, render_srt(lines, target))?;
            let out = dir.join(format!("{stem}.subbed.mkv"));
            let (v, s, o) = (path_s(input), path_s(&tmp), path_s(&out));
            ff(&["-i", &v, "-i", &s, "-map", "0", "-map", "1", "-c", "copy", "-c:s", "srt", &o])?;
            Ok(vec![o])
        }
        _ => {
            let p = dir.join(format!("{stem}.{}.srt", tag_of(target)));
            std::fs::write(&p, render_srt(lines, target))?;
            Ok(vec![path_s(&p)])
        }
    }
}

async fn pipeline(app: &AppHandle, id: u64) -> Result<(Vec<String>, Vec<Line>)> {
    let (video_id, input, series_id, opts) =
        with_job(id, |j| (j.view.video_id, j.input.clone(), j.series_id, j.options.clone())).ok_or_else(|| anyhow!("任务不存在"))?;
    let target = opts.target.clone().unwrap_or_else(|| "both".into());
    let output = opts.output.clone().unwrap_or_else(|| "srt".into());
    let lang = norm_lang(opts.lang.as_deref());
    let work = work_dir(app, video_id)?;

    // 1. 提取音轨（16 kHz 单声道 flac）
    progress(app, id, "extract", 0.0, "正在提取音轨");
    let audio = work.join("audio.flac");
    {
        let (i, a) = (input.clone(), audio.clone());
        blocking(move || extract_audio(&i, &a)).await?;
    }
    check_cancel(id)?;

    // 2. 检测人声，切块
    progress(app, id, "vad", 0.0, "正在检测人声");
    let (total, voice) = {
        let a = audio.clone();
        blocking(move || detect_voice(&a)).await?
    };
    if voice.is_empty() {
        return Err(anyhow!("没有检测到人声（音轨可能是静音或只有音乐）"));
    }
    let chunks = plan_chunks(&voice, MAX_CHUNK_SECS, MP3_BYTES_PER_SEC, MAX_UPLOAD_BYTES);
    progress(app, id, "vad", 1.0, &format!("人声 {} 段，切成 {} 块", voice.len(), chunks.len()));

    // 3. 语音识别，按块上传；时间戳加上块的起点
    let n = chunks.len();
    let mut segs: Vec<Segment> = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        check_cancel(id)?;
        progress(app, id, "asr", i as f64 / n as f64, &format!("识别第 {}/{} 块", i + 1, n));
        let start = (c.start - 0.3).max(0.0);
        let len = (c.end + 0.3).min(total.max(c.end)) - start;
        let chunk_path = work.join(format!("chunk_{i}.mp3"));
        {
            let (a, cp) = (audio.clone(), chunk_path.clone());
            blocking(move || make_chunk(&a, start, len, &cp)).await?;
        }
        let bytes = std::fs::read(&chunk_path)?;
        let _ = std::fs::remove_file(&chunk_path);
        let mut got = ai::transcribe(app, bytes, "chunk.mp3", lang.as_deref(), None).await?;
        for s in got.iter_mut() {
            s.start += start;
            s.end += start;
        }
        segs.extend(got);
    }
    if segs.is_empty() {
        return Err(anyhow!("没有识别出语音"));
    }

    // 4. 断句与对齐
    progress(app, id, "align", 0.0, "断句与对齐");
    let cjk = match lang.as_deref() {
        Some("ja") | Some("zh") => true,
        Some(_) => false,
        None => is_cjk_text(&segs.iter().map(|s| s.text.as_str()).collect::<String>()),
    };
    let mut lines = align_segments(&segs, &voice, cjk);
    if lines.is_empty() {
        return Err(anyhow!("断句后没有字幕"));
    }
    progress(app, id, "align", 1.0, &format!("共 {} 条字幕", lines.len()));

    // 5. 翻译（可选）
    if target != "src" {
        let glossary = load_glossary(app, &glossary_key(series_id, video_id));
        translate_all(app, id, &mut lines, &glossary).await?;
    }
    check_cancel(id)?;

    // 6. 输出，并把视频的字幕状态记为 ai
    progress(app, id, "done", 0.9, "正在写出字幕");
    let (i, t, o, w, ls) = (input.clone(), target.clone(), output.clone(), work.clone(), lines.clone());
    let outs = blocking(move || write_outputs(&i, &w, &t, &o, &ls)).await?;
    set_subs_ai(app, video_id)?;
    Ok((outs, lines))
}

// ---------- 命令 ----------

fn ensure_ready(app: &AppHandle) -> R<()> {
    tools::ffmpeg().ok_or_else(|| "没有找到 ffmpeg.exe（放到程序目录的 bin 文件夹，或加入 PATH）".to_string())?;
    ai::ready(app).map_err(es)?;
    Ok(())
}

/// 校验选项；术语表有内容时存进 settings（按 key：整季共用）
fn prepare(st: &AppState, key: &str, mut options: SubOptions) -> R<SubOptions> {
    if let Some(t) = options.target.as_deref() {
        if !matches!(t, "both" | "zh" | "src") {
            return Err(format!("不认识的字幕语言：{t}"));
        }
    }
    if let Some(o) = options.output.as_deref() {
        if !matches!(o, "srt" | "ass" | "mkv") {
            return Err(format!("不认识的输出格式：{o}"));
        }
    }
    options.lang = norm_lang(options.lang.as_deref());
    if let Some(g) = options.glossary.clone().filter(|g| !g.trim().is_empty()) {
        let conn = st.db.lock();
        library::set_setting(&conn, key, &json!(g)).map_err(es)?;
    }
    Ok(options)
}

fn to_sub(v: library::Video, series_title: String) -> SubVideo {
    SubVideo {
        id: v.id,
        title: v.title,
        series_id: v.series_id,
        series_title,
        season: v.season,
        episode: v.episode,
        subs: v.subs,
        duration: v.duration,
        path: v.path,
    }
}

/// 列出所有视频（独立视频 + 番剧 / 剧集的每一集），供任务设置里选择
#[tauri::command]
pub fn sub_list_videos(st: State<AppState>) -> R<Vec<SubVideo>> {
    let conn = st.db.lock();
    let mut out = Vec::new();
    for v in library::videos(&conn).map_err(es)? {
        out.push(to_sub(v, String::new()));
    }
    for kind in ["anime", "tv"] {
        for s in library::series(&conn, kind).map_err(es)? {
            for v in library::episodes(&conn, s.id).map_err(es)? {
                out.push(to_sub(v, s.title.clone()));
            }
        }
    }
    Ok(out)
}

/// 开始一个视频的字幕任务，返回任务 id
#[tauri::command]
pub fn sub_start(app: AppHandle, st: State<AppState>, video_id: i64, options: SubOptions) -> R<u64> {
    ensure_ready(&app)?;
    let video = {
        let conn = st.db.lock();
        library::video(&conn, video_id).map_err(es)?.ok_or_else(|| "找不到这个视频".to_string())?
    };
    let key = glossary_key(video.series_id, video.id);
    let opts = prepare(&st, &key, options)?;
    let id = new_job(&video, &opts);
    kick(&app);
    Ok(id)
}

/// 整季（或整部）缺字幕的集批量处理，返回每一集的任务 id
#[tauri::command]
pub fn sub_batch(app: AppHandle, st: State<AppState>, series_id: i64, season: Option<i64>, options: SubOptions) -> R<Vec<u64>> {
    ensure_ready(&app)?;
    let todo: Vec<library::Video> = {
        let conn = st.db.lock();
        library::episodes(&conn, series_id)
            .map_err(es)?
            .into_iter()
            .filter(|v| v.subs == "none" && season.map_or(true, |s| v.season == Some(s)))
            .collect()
    };
    if todo.is_empty() {
        return Err("这里没有缺字幕的集".into());
    }
    let key = glossary_key(Some(series_id), 0);
    let opts = prepare(&st, &key, options)?;
    let ids: Vec<u64> = todo.iter().map(|v| new_job(v, &opts)).collect();
    kick(&app);
    Ok(ids)
}

/// 取消任务：排队中的直接移除；正在跑的在下一步开始前停止（识别请求中途不能打断）
#[tauri::command]
pub fn sub_cancel(job: u64) -> R<()> {
    let mut s = STATE.lock();
    if let Some(p) = s.queue.iter().position(|&x| x == job) {
        s.queue.remove(p);
    }
    let j = s.jobs.iter_mut().find(|j| j.view.id == job).ok_or_else(|| "任务不存在".to_string())?;
    j.cancel = true;
    if j.view.status == "queued" {
        j.view.status = "cancelled".into();
        j.view.message = "已取消".into();
    }
    Ok(())
}

/// 所有任务的状态（最近 50 个）
#[tauri::command]
pub fn sub_status() -> R<Vec<JobView>> {
    let s = STATE.lock();
    let mut v: Vec<JobView> = s.jobs.iter().rev().take(50).map(|j| j.view.clone()).collect();
    v.reverse();
    Ok(v)
}

/// 任务的分段结果（可直接编辑译文后交给 sub_save）
#[tauri::command]
pub fn sub_result(job: u64) -> R<Vec<Line>> {
    with_job(job, |j| j.lines.clone()).ok_or_else(|| "任务不存在".to_string())
}

/// 用户在界面改过后，按原来的选项重新写出字幕文件，返回路径
#[tauri::command]
pub async fn sub_save(app: AppHandle, job: u64, segments: Vec<Line>) -> R<Vec<String>> {
    let (input, video_id, target, output) = with_job(job, |j| {
        (
            j.input.clone(),
            j.view.video_id,
            j.options.target.clone().unwrap_or_else(|| "both".into()),
            j.options.output.clone().unwrap_or_else(|| "srt".into()),
        )
    })
    .ok_or_else(|| "任务不存在".to_string())?;
    let work = work_dir(&app, video_id).map_err(es)?;
    let ls = segments.clone();
    let outs = blocking(move || write_outputs(&input, &work, &target, &output, &ls)).await.map_err(es)?;
    set_subs_ai(&app, video_id).map_err(es)?;
    let _ = with_job(job, |j| {
        j.lines = segments;
        j.view.outputs = outs.clone();
    });
    Ok(outs)
}

/// 开始前估算：按视频时长估算翻译用量，并和本月上限比较
#[tauri::command]
pub fn sub_estimate(st: State<AppState>, video_id: i64) -> R<Estimate> {
    let conn = st.db.lock();
    let video = library::video(&conn, video_id).map_err(es)?.ok_or_else(|| "找不到这个视频".to_string())?;
    let minutes = (video.duration / 60.0 * 10.0).round() / 10.0;
    let est_tokens = (minutes * TOKENS_PER_MINUTE).ceil() as u64;
    let limit = ai::load_config(&conn).monthly_token_limit;
    let u = ai::usage(&conn);
    let used = u.prompt_tokens + u.completion_tokens;
    Ok(Estimate {
        minutes,
        est_tokens,
        monthly_limit: limit,
        used_tokens: used,
        over_limit: limit > 0 && used + est_tokens > limit,
    })
}

/// 只做对齐：用视频的人声区间重新对齐已有的 srt，写成 同名.aligned.srt，返回路径
#[tauri::command]
pub async fn sub_realign(app: AppHandle, video_id: i64, sub_path: String) -> R<String> {
    let input = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        library::video(&conn, video_id).map_err(es)?.ok_or_else(|| "找不到这个视频".to_string())?.path
    };
    let text = std::fs::read_to_string(&sub_path).map_err(es)?;
    let cues = parse_srt(&text);
    if cues.is_empty() {
        return Err("没有读到字幕内容（需要 srt 格式）".into());
    }
    let work = work_dir(&app, video_id).map_err(es)?;
    let voice = blocking(move || {
        let audio = work.join("realign.flac");
        extract_audio(Path::new(&input), &audio)?;
        let (_, voice) = detect_voice(&audio)?;
        let _ = std::fs::remove_file(&audio);
        Ok(voice)
    })
    .await
    .map_err(es)?;
    if voice.is_empty() {
        return Err("没有检测到人声，无法对齐".into());
    }
    let aligned = realign(&cues, &voice);
    let src = PathBuf::from(&sub_path);
    let dir = src.parent().unwrap_or(Path::new(".")).to_path_buf();
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "subtitle".into());
    let out = dir.join(format!("{stem}.aligned.srt"));
    std::fs::write(&out, render_srt(&aligned, "src")).map_err(es)?;
    Ok(path_s(&out))
}

// ---------- 启动 ----------

/// 没有需要建的表（字幕状态直接写在 videos.subs，任务在内存里）
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（无后台任务，任务由命令触发）
pub fn setup(_app: &AppHandle) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment { start, end, text: text.into() }
    }

    #[test]
    fn silences_parse() {
        let log = "[silencedetect @ 0x1] silence_start: 2.5\n\
                   [silencedetect @ 0x1] silence_end: 4.1 | silence_duration: 1.6\n\
                   [silencedetect @ 0x1] silence_start: 9.0\n";
        let s = parse_silences(log);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], Iv { start: 2.5, end: 4.1 });
        assert_eq!(s[1].start, 9.0);
    }

    #[test]
    fn voice_is_complement_of_silence() {
        let sil = vec![Iv { start: 2.0, end: 3.0 }, Iv { start: 8.0, end: 10.0 }];
        let v = voice_from_silence(&sil, 12.0);
        assert_eq!(v, vec![Iv { start: 0.0, end: 2.0 }, Iv { start: 3.0, end: 8.0 }, Iv { start: 10.0, end: 12.0 }]);
    }

    #[test]
    fn short_voice_dropped() {
        let v = voice_from_silence(&[Iv { start: 0.2, end: 0.4 }], 1.0);
        assert!(v.iter().all(|x| x.end - x.start >= 0.3));
    }

    #[test]
    fn merge_close_intervals() {
        let v = vec![Iv { start: 0.0, end: 1.0 }, Iv { start: 1.5, end: 2.0 }, Iv { start: 5.0, end: 6.0 }];
        let m = merge_intervals(&v, 0.8);
        assert_eq!(m, vec![Iv { start: 0.0, end: 2.0 }, Iv { start: 5.0, end: 6.0 }]);
    }

    #[test]
    fn chunks_respect_length() {
        let voice = vec![Iv { start: 0.0, end: 10.0 }, Iv { start: 12.0, end: 20.0 }, Iv { start: 100.0, end: 250.0 }];
        let c = plan_chunks(&voice, 60.0, 8000.0, 20.0 * 1024.0 * 1024.0);
        assert!(c.iter().all(|x| x.end - x.start <= 60.0 + 1e-9));
        assert_eq!(c.first().unwrap().start, 0.0);
        assert_eq!(c.last().unwrap().end, 250.0);
        // 0-20 合成一块，100-250 切成三块
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn chunks_respect_bytes() {
        // 每秒 100 KB、上限 1 MB：块不能超过 10 秒
        let c = plan_chunks(&[Iv { start: 0.0, end: 35.0 }], 60.0, 100_000.0, 1_000_000.0);
        assert!(c.iter().all(|x| x.end - x.start <= 10.0 + 1e-9));
    }

    #[test]
    fn sentences_split() {
        assert_eq!(split_sentences("ねえ、聞いた？港の。夜！"), vec!["ねえ、聞いた？", "港の。", "夜！"]);
        assert_eq!(split_sentences("Hello world. This is 3.5 cats."), vec!["Hello world.", "This is 3.5 cats."]);
        assert_eq!(split_sentences("「やめて！」と言った。"), vec!["「やめて！」", "と言った。"]);
    }

    #[test]
    fn even_split_keeps_text() {
        let t = "abcdefghij，klmnopqrst";
        let parts = split_even(t, 2);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.concat().replace(' ', ""), t.replace(' ', ""));
    }

    #[test]
    fn align_limits_and_order() {
        let segs = vec![
            seg(0.0, 10.0, "ねえ、聞いた？港の郵便局のこと。夜しか開かないんだって。"),
            seg(10.5, 12.0, "まさか。"),
            seg(
                12.0,
                40.0,
                "本当だよ。霧の夜だけ、灯りがつくの。それを見た人は、誰もいないと言うけれど、私は一度だけ見たことがあるの。",
            ),
        ];
        let voice = vec![Iv { start: 0.0, end: 12.0 }, Iv { start: 12.0, end: 40.0 }];
        let lines = align_segments(&segs, &voice, true);
        assert!(lines.len() >= 3);
        for (i, l) in lines.iter().enumerate() {
            assert!(l.end > l.start, "时长为正");
            assert!(l.end - l.start <= 7.0 + 1e-9, "时长不超过 7 秒: {:?}", l);
            assert!(l.src.chars().count() <= 40, "每条不超过 40 字: {}", l.src);
            if i > 0 {
                assert!(l.start >= lines[i - 1].end - 1e-9, "不重叠");
            }
        }
    }

    #[test]
    fn align_snaps_to_voice() {
        let segs = vec![seg(1.0, 4.0, "こんにちは。")];
        let voice = vec![Iv { start: 1.2, end: 3.8 }];
        let lines = align_segments(&segs, &voice, true);
        assert!((lines[0].start - 1.2).abs() < 1e-9);
        assert!((lines[0].end - 3.8).abs() < 1e-9);
    }

    #[test]
    fn srt_round_trip() {
        let lines = vec![Line {
            start: 1.5,
            end: 3.25,
            src: "ねえ".into(),
            zh: "喂".into(),
        }];
        let srt = render_srt(&lines, "both");
        assert!(srt.contains("00:00:01,500 --> 00:00:03,250"));
        assert!(srt.contains("喂\nねえ"));
        let back = parse_srt(&srt);
        assert_eq!(back.len(), 1);
        assert!((back[0].start - 1.5).abs() < 1e-9);
        assert_eq!(back[0].src, "喂\nねえ");
    }

    #[test]
    fn ass_has_two_styles() {
        let lines = vec![Line {
            start: 0.0,
            end: 2.0,
            src: "ねえ".into(),
            zh: "喂".into(),
        }];
        let ass = render_ass(&lines, "both");
        assert!(ass.contains("Dialogue: 0,0:00:00.00,0:00:02.00,Zh,,0,0,0,,喂"));
        assert!(ass.contains("Dialogue: 0,0:00:00.00,0:00:02.00,Src,,0,0,0,,ねえ"));
        assert_eq!(fmt_ass_time(61.5), "0:01:01.50");
    }

    #[test]
    fn realign_shifts_and_snaps() {
        let cues = vec![Line {
            start: 5.0,
            end: 7.0,
            src: "a".into(),
            zh: String::new(),
        }];
        let voice = vec![Iv { start: 5.4, end: 7.3 }];
        let r = realign(&cues, &voice);
        assert!((r[0].start - 5.4).abs() < 1e-9);
        assert!((r[0].end - 7.3).abs() < 1e-9);
    }

    #[test]
    fn translations_must_match_count() {
        let ok = parse_translations("{\"lines\":[\"a\",\"b\"]}", 2).unwrap();
        assert_eq!(ok, vec!["a", "b"]);
        assert!(parse_translations("```json\n[\"a\"]\n```", 2).is_err());
        assert_eq!(parse_translations("[\"甲\"]", 1).unwrap(), vec!["甲"]);
    }

    #[test]
    fn time_format() {
        assert_eq!(fmt_srt_time(3723.004), "01:02:03,004");
    }
}
