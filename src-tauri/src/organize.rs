//! AI 整理文件：识别、预览、执行（移动 / 复制 / 硬链接 / 仅库内）、撤销
//!
//! 安全规则（这里直接改用户硬盘上的文件，所以严格）：
//! - 只处理用户选定的文件夹；系统目录（Windows、Program Files 等）不碰。
//! - 不删除任何用户文件。跨盘移动先复制、校验大小一致，再删源；撤销复制 / 硬链接时，把我们建的副本放进回收站。
//! - 每一步先写 org_moves（done=0），成功后改成 done=1；只有 done=1 的步骤才会被撤销。
//! - 打不开的文件（多半被占用）直接跳过并记录。
use crate::{ai, db, indexer, library, metadata, parse, reader, scanner, tools, AppState};
use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

/// 认不准、需要用户确认的文件放进这个文件夹（只移动，不删除）
const DUP_DIR: &str = "重复待处理";
/// 路径长度上限（Windows 默认 260，留余量）
const MAX_PATH: usize = 240;
/// 可以跟着视频一起移动的配套文件
const COMPANION_EXTS: &[&str] = &["ttf", "otf", "ttc", "nfo", "lrc", "jpg", "jpeg", "png", "webp"];
/// 系统和程序目录：不在这些目录里做任何移动
const SYSTEM_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "recovery",
    "windowsapps",
    "$windows.~bt",
    "msocache",
];

/// init 时建表（org_batches / org_moves 已在 db.rs 建好，这里是幂等保险）
pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS org_batches(id INTEGER PRIMARY KEY, created_at INTEGER NOT NULL, mode TEXT NOT NULL, undone INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE IF NOT EXISTS org_moves(batch_id INTEGER NOT NULL REFERENCES org_batches(id) ON DELETE CASCADE, src TEXT NOT NULL, dst TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0);",
    )?;
    Ok(())
}

/// 启动时不做事：整理只在用户点击后进行
pub fn setup(_app: &AppHandle) {}

// ---------- 数据结构（前端也用这些字段） ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Extra {
    pub src: String,
    pub dst: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Item {
    pub src: String,
    /// 空字符串 = 原地不动
    pub dst: String,
    /// anime / tv / movie / music / novel / manga / other
    pub kind: String,
    /// hi / mid / low
    pub confidence: String,
    pub reason: String,
    /// 配套文件（字幕、字体、封面、nfo），跟着视频一起移动
    pub extras: Vec<Extra>,
    pub checked: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Failed {
    pub path: String,
    pub error: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ExecReport {
    /// 没有成功移动任何文件时为 null（不生成空批次）
    pub batch_id: Option<i64>,
    pub done: usize,
    pub failed: Vec<Failed>,
}

#[derive(Serialize, Clone, Debug)]
pub struct UndoReport {
    pub reverted: usize,
    pub failed: Vec<Failed>,
    /// 这一批是否已经全部撤销
    pub finished: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct BatchInfo {
    pub id: i64,
    pub created_at: i64,
    pub mode: String,
    pub undone: bool,
    pub count: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Estimate {
    /// 需要交给 AI 的文件数
    pub files: usize,
    /// 预计消耗的 token（只算提示词，不含回复）
    pub tokens: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: String,
    /// move / copy / hardlink / library / undo / done
    pub phase: String,
}

/// 整理设置（存在 settings 表的 "org" 里）：模板可以改，排除名单
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct OrgSettings {
    pub templates: HashMap<String, String>,
    pub exclude: Vec<String>,
}

pub(crate) fn load_settings(conn: &Connection) -> OrgSettings {
    library::get_setting(conn, "org")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn default_template(key: &str) -> &'static str {
    match key {
        "anime" => "番剧/{作品名} ({年份})/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext",
        "anime_special" => "番剧/{作品名} ({年份})/Specials/{作品名} - {特典}.ext",
        "tv" => "电视剧/{作品名} ({年份})/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext",
        "tv_special" => "电视剧/{作品名} ({年份})/Specials/{作品名} - {特典}.ext",
        "movie" => "电影/{片名} ({年份})/{片名} ({年份}).ext",
        "music" => "音乐/{艺人}/{专辑} ({年份})/{音轨:02} - {歌名}.ext",
        "manga" => "漫画/{作品名}/{原文件名}.ext",
        _ => "小说/{作者}/{书名}.ext",
    }
}

fn template_for(st: &OrgSettings, key: &str) -> String {
    st.templates
        .get(key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_template(key).to_string())
}

// ---------- 文件名：替换非法字符、保留字、长度 ----------

/// 替换 Windows 不允许的字符；空串原样返回（表示“没有这个信息”）
pub fn sanitize(s: &str) -> String {
    if s.trim().is_empty() {
        return String::new();
    }
    let t: String = s
        .chars()
        .map(|c| match c {
            ':' => '：',
            '?' => '？',
            '*' => '＊',
            '"' => '＂',
            '<' => '＜',
            '>' => '＞',
            '|' => '｜',
            '/' => '／',
            '\\' => '＼',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let t = t.trim().trim_end_matches(['.', ' ']).trim().to_string();
    if t.is_empty() {
        return "_".into();
    }
    if is_reserved(&t) {
        format!("_{t}")
    } else {
        t
    }
}

fn is_reserved(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    let base = u.split('.').next().unwrap_or("");
    if matches!(base, "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let b = base.as_bytes();
    base.len() == 4 && (base.starts_with("COM") || base.starts_with("LPT")) && b[3].is_ascii_digit() && b[3] != b'0'
}

/// 拆出扩展名（只认很短的扩展名，避免把 “Dr. Stone” 之类当成扩展名）
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 8 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// 文件名超长时截断主名，保留扩展名
fn shorten_last(p: PathBuf) -> PathBuf {
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return p;
    };
    let Some(parent) = p.parent().map(Path::to_path_buf) else { return p };
    let plen = parent.to_string_lossy().chars().count() + 1;
    if plen + name.chars().count() <= MAX_PATH {
        return p;
    }
    let (stem, ext) = split_ext(&name);
    let room = MAX_PATH.saturating_sub(plen + ext.chars().count()).max(8);
    let cut: String = stem.chars().take(room).collect();
    parent.join(format!("{}{}", cut.trim_end(), ext))
}

type Vars = HashMap<String, String>;

fn vars(pairs: &[(&str, String)]) -> Vars {
    pairs.iter().map(|(k, v)| (k.to_string(), sanitize(v))).collect()
}

static RE_TOKEN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{([^{}:]+)(?::(\d+))?\}").unwrap());
/// “(… {年份} …)” 整个括号：年份未知时一起去掉
static RE_YEAR_GROUP: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*[(（][^()（）]*\{年份\}[^()（）]*[)）]").unwrap());

fn render_component(comp: &str, vars: &Vars) -> String {
    let year_empty = vars.get("年份").map_or(true, |y| y.is_empty());
    let mut comp = if year_empty {
        RE_YEAR_GROUP.replace_all(comp, "").into_owned()
    } else {
        comp.to_string()
    };
    // 空变量后面紧跟的“ - ”一起去掉，例如没有音轨号时不留“00 - ”
    for (k, v) in vars.iter() {
        if v.is_empty() {
            if let Ok(re) = Regex::new(&format!(r"\{{{}(?::\d+)?\}}\s*[-–]\s*", regex::escape(k))) {
                comp = re.replace_all(&comp, "").into_owned();
            }
        }
    }
    RE_TOKEN
        .replace_all(&comp, |c: &regex::Captures| {
            let name = c.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            let width = c.get(2).and_then(|m| m.as_str().parse::<usize>().ok()).unwrap_or(0);
            let v = vars.get(name).cloned().unwrap_or_default();
            if width > 0 && !v.is_empty() && v.chars().all(|ch| ch.is_ascii_digit()) {
                format!("{:0>width$}", v, width = width)
            } else {
                v
            }
        })
        .into_owned()
}

/// 按模板生成目标路径。模板用 / 分隔目录；末尾的 .ext 换成源文件的扩展名
pub fn render_dst(target: &Path, tpl: &str, vars: &Vars, ext: &str) -> PathBuf {
    let (base, with_ext) = match tpl.strip_suffix(".ext") {
        Some(b) => (b, true),
        None => (tpl, false),
    };
    let parts: Vec<&str> = base.split('/').collect();
    let last = parts.len().saturating_sub(1);
    let mut out = target.to_path_buf();
    for (i, p) in parts.iter().enumerate() {
        // 单个目录名 / 文件名不超过 120 个字，整体长度再由 shorten_last 收一下
        let mut s: String = render_component(p, vars).trim().chars().take(120).collect();
        s = s.trim_end().to_string();
        if i == last && with_ext && !ext.is_empty() {
            s = format!("{s}.{ext}");
        }
        if !s.is_empty() {
            out.push(s);
        }
    }
    shorten_last(out)
}

fn key_of(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    key_of(a) == key_of(b)
}

/// 重名时加序号：“x.mkv” → “x (2).mkv”。taken 是这一批已经占用的目标（小写）
fn unique(dst: &Path, src: &Path, taken: &mut HashSet<String>) -> PathBuf {
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = split_ext(&name);
    let dir = dst.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut n = 1;
    loop {
        let cand = if n == 1 { dst.to_path_buf() } else { dir.join(format!("{stem} ({n}){ext}")) };
        let k = key_of(&cand);
        if !same_path(&cand, src) && !taken.contains(&k) && !cand.exists() {
            taken.insert(k);
            return cand;
        }
        n += 1;
    }
}

/// 系统目录检查：路径里任何一级是系统目录就拒绝
fn check_path_allowed(p: &Path) -> Result<()> {
    for c in p.components() {
        if let Component::Normal(n) = c {
            let name = n.to_string_lossy().to_lowercase();
            if SYSTEM_DIRS.contains(&name.as_str()) {
                bail!("{} 位于系统目录，不能整理", p.display());
            }
        }
    }
    Ok(())
}

// ---------- 识别 ----------

/// 一个文件的识别结果
#[derive(Clone, Debug)]
struct Guess {
    kind: String,
    /// hi / mid / low
    conf: &'static str,
    reason: String,
    /// 模板名 + 变量；None 表示认不出，目标位置留空
    tpl: Option<(&'static str, Vars)>,
    /// 同一集的标识，用来找重复版本
    ep_key: Option<String>,
    /// 分辨率得分，用来挑画质高的版本
    res: i64,
}

impl Guess {
    fn none(kind: &str, reason: &str) -> Guess {
        Guess {
            kind: kind.into(),
            conf: "low",
            reason: reason.into(),
            tpl: None,
            ep_key: None,
            res: 0,
        }
    }
}

static RE_YEAR: Lazy<Regex> = Lazy::new(|| Regex::new(r"[(（\[]((?:19|20)\d{2})[)）\]]").unwrap());
static RE_SPECIAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(NCOP|NCED|OVA|OAD|SP|OP|ED|PV|CM|Menu)\b|特典|花絮").unwrap());

/// 特典标签（OP / ED / SP / OVA / NCOP / NCED / PV）
pub fn special_label(stem: &str) -> Option<String> {
    RE_SPECIAL.find(stem).map(|m| m.as_str().to_uppercase())
}

/// 父目录名，从近到远，最多 n 层
fn folder_names(path: &Path, n: usize) -> Vec<String> {
    path.ancestors()
        .skip(1)
        .take(n)
        .filter_map(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

fn find_year(stem: &str, folders: &[String]) -> String {
    std::iter::once(stem.to_string())
        .chain(folders.iter().cloned())
        .find_map(|s| RE_YEAR.captures(&s).map(|c| c[1].to_string()))
        .unwrap_or_default()
}

fn resolution_score(s: &str) -> i64 {
    let s = s.to_lowercase();
    if s.contains("2160p") || s.contains("4k") || s.contains("3840") {
        4
    } else if s.contains("1080p") || s.contains("1920") {
        3
    } else if s.contains("720p") || s.contains("1280") {
        2
    } else if s.contains("480p") {
        1
    } else {
        0
    }
}

fn ep_text(e: f64) -> String {
    if e.fract() == 0.0 {
        format!("{}", e as i64)
    } else {
        format!("{e}")
    }
}

/// 视频时长（秒）。没有 ffprobe 或打不开时返回 None
fn probe_duration(path: &Path) -> Option<f64> {
    let v = tools::probe(path).ok()?;
    let d = &v["format"]["duration"];
    d.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| d.as_f64())
}

fn guess_video(path: &Path) -> Guess {
    let info = parse::parse_episode(path);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let folders = folder_names(path, 2);
    let hay = format!("{} {}", folders.join(" "), stem).to_lowercase();
    let mins = probe_duration(path).map(|s| s / 60.0);
    let res = resolution_score(info.resolution.as_deref().unwrap_or(&stem));
    let title = info.title.trim().to_string();
    if title.is_empty() {
        return Guess {
            res,
            ..Guess::none("other", "文件名里没有作品名")
        };
    }
    let year = find_year(&stem, &folders);
    let special = info.special || special_label(&stem).is_some();
    let anime_hint = ["番剧", "动漫", "动画", "anime", "bangumi"].iter().any(|k| hay.contains(k)) || info.group.is_some();
    let tv_hint = ["电视剧", "剧集", "美剧", "日剧", "韩剧", "国产剧", "tv"].iter().any(|k| hay.contains(k));
    let movie_hint = ["电影", "movie", "剧场版"].iter().any(|k| hay.contains(k));

    if info.episode.is_some() || special {
        // 24 分钟左右多半是番剧，45 分钟以上多半是电视剧
        let by_len = match mins {
            Some(m) if (12.0..=32.0).contains(&m) => Some("anime"),
            Some(m) if (33.0..=75.0).contains(&m) => Some("tv"),
            _ => None,
        };
        let is_anime = if anime_hint {
            true
        } else if tv_hint {
            false
        } else {
            by_len == Some("anime")
        };
        let kind = if is_anime { "anime" } else { "tv" };
        let conf = if info.group.is_some() && info.episode.is_some() && !special {
            "hi"
        } else if anime_hint || tv_hint || by_len.is_some() {
            "mid"
        } else {
            "low"
        };
        let why = if info.group.is_some() {
            "字幕组命名，作品名和集数都能拆出"
        } else if anime_hint {
            "文件名或文件夹提示是番剧"
        } else if tv_hint {
            "文件名或文件夹提示是电视剧"
        } else if by_len.is_some() {
            "时长符合番剧或电视剧"
        } else {
            "看不出是番剧还是电视剧，先按电视剧处理"
        };
        if special {
            let label = special_label(&stem).unwrap_or_else(|| "特典".into());
            let key = if is_anime { "anime_special" } else { "tv_special" };
            return Guess {
                kind: "other".into(),
                conf: if conf == "low" { "low" } else { "mid" },
                reason: format!("特典（{label}），单独放进 Specials 文件夹"),
                tpl: Some((key, vars(&[("作品名", title), ("年份", year), ("特典", label)]))),
                ep_key: None,
                res,
            }
            .with_kind(kind);
        }
        let season = info.season.unwrap_or(1);
        let ep = info.episode.unwrap_or(0.0);
        let key = if is_anime { "anime" } else { "tv" };
        let ep_key = format!("{kind}|{}|{season}|{}", title.to_lowercase(), (ep * 10.0).round() as i64);
        return Guess {
            kind: kind.into(),
            conf,
            reason: why.into(),
            tpl: Some((key, vars(&[("作品名", title), ("年份", year), ("季", season.to_string()), ("集", ep_text(ep))]))),
            ep_key: Some(ep_key),
            res,
        };
    }
    if movie_hint || mins.is_some_and(|m| m >= 70.0) {
        let why = if movie_hint {
            "文件名或文件夹提示是电影".to_string()
        } else {
            format!("时长约 {} 分钟，多半是电影", mins.unwrap_or(0.0).round() as i64)
        };
        return Guess {
            kind: "movie".into(),
            conf: "mid",
            reason: why,
            tpl: Some(("movie", vars(&[("片名", title.clone()), ("作品名", title), ("年份", year)]))),
            ep_key: None,
            res,
        };
    }
    Guess {
        res,
        ..Guess::none("other", "没有集数，时长也不典型，认不出是什么")
    }
}

impl Guess {
    fn with_kind(mut self, kind: &str) -> Guess {
        self.kind = kind.into();
        self
    }
}

fn guess_music(path: &Path, covers: &Path) -> Guess {
    if parse::is_encrypted(path) {
        return Guess::none("music", "加密格式，需要先解密才能识别");
    }
    let m = metadata::read_track(path, covers);
    let artist = if m.artist.trim().is_empty() {
        m.album_artist.trim().to_string()
    } else {
        m.artist.trim().to_string()
    };
    if artist.is_empty() {
        return Guess::none("music", "没有艺人标签，无法确定放在哪个文件夹");
    }
    let (album, conf, why) = if m.album.trim().is_empty() {
        ("未知专辑".to_string(), "mid", "没有专辑标签，放进“未知专辑”")
    } else {
        (m.album.trim().to_string(), "hi", "读取了标签：艺人、专辑、音轨号")
    };
    let year = m.year.map(|y| y.to_string()).unwrap_or_default();
    let track = m.track_no.map(|n| n.to_string()).unwrap_or_default();
    Guess {
        kind: "music".into(),
        conf,
        reason: why.into(),
        tpl: Some((
            "music",
            vars(&[
                ("艺人", artist),
                ("专辑", album),
                ("年份", year),
                ("音轨", track),
                ("歌名", m.title.trim().to_string()),
            ]),
        )),
        ep_key: None,
        res: 0,
    }
}

fn guess_novel(path: &Path) -> Guess {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut title, mut author) = reader::split_title_author(&stem);
    let mut conf = "mid";
    if parse::ext(path) == "epub" {
        if let Ok(b) = reader::open_epub(path) {
            if !b.title.trim().is_empty() {
                title = b.title;
            }
            if !b.author.trim().is_empty() {
                author = b.author;
                conf = "hi";
            }
        }
    }
    if title.trim().is_empty() {
        return Guess::none("novel", "认不出书名");
    }
    if author.trim().is_empty() {
        return Guess::none("novel", "文件名里没有作者，无法确定放在哪个文件夹");
    }
    Guess {
        kind: "novel".into(),
        conf,
        reason: "读取了书名和作者".into(),
        tpl: Some(("novel", vars(&[("作者", author), ("书名", title)]))),
        ep_key: None,
        res: 0,
    }
}

fn guess_manga(path: &Path) -> Guess {
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let generic = ["漫画", "comic", "manga", "漫畫", "下载", "download", "downloads"].contains(&parent.to_lowercase().as_str());
    if parent.trim().is_empty() || generic {
        return Guess::none("manga", "认不出作品名（所在文件夹不是作品名）");
    }
    Guess {
        kind: "manga".into(),
        conf: "mid",
        reason: "按所在文件夹名识别作品".into(),
        tpl: Some(("manga", vars(&[("作品名", parent), ("原文件名", stem)]))),
        ep_key: None,
        res: 0,
    }
}

// ---------- .em 整理规则 ----------

/// 季号：阿拉伯数字或“一”～“九十九”这样的中文数字
fn num_of(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let d = |c: char| "零一二三四五六七八九".chars().position(|x| x == c).map(|i| i as u32);
    let cs: Vec<char> = s.chars().collect();
    match cs.as_slice() {
        ['十'] => Some(10),
        [a] => d(*a),
        ['十', b] => Some(10 + d(*b)?),
        [a, '十'] => Some(d(*a)? * 10),
        [a, '十', b] => Some(d(*a)? * 10 + d(*b)?),
        _ => None,
    }
}

/// 规则匹配结果 → 识别结果（把握高）。模板名按 media 取，有特典时用 Specials 的模板
fn rule_guess(m: &crate::em::organize::Matched, path: &Path) -> Guess {
    let f = |k: &str| m.fields.get(k).cloned().unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (series, title) = (f("series"), f("title"));
    let work = if series.is_empty() { title.clone() } else { series.clone() };
    let named = if title.is_empty() { series.clone() } else { title.clone() };
    let year = match f("year") {
        y if !y.is_empty() => y,
        _ => find_year(&stem, &folder_names(path, 2)),
    };
    let episodic = matches!(m.media.as_str(), "anime" | "tv");
    let season = if episodic {
        num_of(&f("season")).unwrap_or(1).to_string()
    } else {
        f("season")
    };
    let ep_num = f("episode").parse::<f64>().ok();
    let ep = ep_num.map(ep_text).unwrap_or_else(|| f("episode"));
    let track = f("track").parse::<u32>().map(|n| n.to_string()).unwrap_or_else(|_| f("track"));
    let special = f("special");
    let key: &'static str = match (m.media.as_str(), special.is_empty()) {
        ("anime", false) => "anime_special",
        ("tv", false) => "tv_special",
        ("anime", true) => "anime",
        ("tv", true) => "tv",
        ("movie", _) => "movie",
        ("music", _) => "music",
        ("manga", _) => "manga",
        _ => "novel",
    };
    let ep_key = match (episodic && special.is_empty(), ep_num) {
        (true, Some(e)) => Some(format!("{}|{}|{season}|{}", m.media, work.to_lowercase(), (e * 10.0).round() as i64)),
        _ => None,
    };
    let v = vars(&[
        ("作品名", work),
        ("片名", named.clone()),
        ("书名", named),
        ("标题", title.clone()),
        ("歌名", title),
        ("年份", year),
        ("季", season),
        ("集", ep),
        ("特典", special),
        ("艺人", f("artist")),
        ("作者", f("artist")),
        ("专辑", f("album")),
        ("音轨", track),
        ("字幕组", f("group")),
        ("原文件名", stem.clone()),
    ]);
    Guess {
        kind: m.media.clone(),
        conf: "hi",
        reason: format!("规则「{}」", m.rule_name),
        tpl: Some((key, v)),
        ep_key,
        res: resolution_score(&stem),
    }
}

/// 规则给出的目标位置：规则自己的模板，没有就用整理页里这类文件的模板
pub(crate) fn rule_dst(m: &crate::em::organize::Matched, path: &Path, target: &Path, st: &OrgSettings) -> PathBuf {
    let g = rule_guess(m, path);
    let (key, v) = g.tpl.expect("rule_guess 总有模板");
    let tpl = m.template.clone().unwrap_or_else(|| template_for(st, key));
    render_dst(target, &tpl, &v, &parse::ext(path))
}

// ---------- 配套文件 ----------

fn is_companion_ext(e: &str) -> bool {
    parse::SUBS.contains(&e) || COMPANION_EXTS.contains(&e)
}

/// 把同一目录下的配套文件归到对应的视频 / 音频。名字以媒体文件的主名开头（后面紧跟“.”）时归它；
/// 多个媒体文件都匹配时，归主名最长的那个（例如 a.v2.mkv 的字幕不归 a.mkv）
fn assign_companions(list: &[PathBuf]) -> HashMap<PathBuf, Vec<PathBuf>> {
    let media: Vec<(PathBuf, String)> = list
        .iter()
        .filter(|p| parse::classify(p).is_some())
        .map(|p| (p.clone(), p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))
        .collect();
    let mut out: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for c in list {
        if !is_companion_ext(&parse::ext(c)) {
            continue;
        }
        let Some(cname) = c.file_name().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let owner = media
            .iter()
            .filter(|(_, s)| cname.len() > s.len() && cname.starts_with(s.as_str()) && cname.as_bytes()[s.len()] == b'.')
            .max_by_key(|(_, s)| s.len());
        if let Some((m, _)) = owner {
            out.entry(m.clone()).or_default().push(c.clone());
        }
    }
    out
}

/// 配套文件的目标名：主文件的新主名 + 原来的后缀（例如 “.zh.ass”）
fn extras_with(map: &HashMap<PathBuf, Vec<PathBuf>>, src: &Path, dst: &Path, taken: &mut HashSet<String>) -> Vec<Extra> {
    let Some(comps) = map.get(src) else { return vec![] };
    let src_stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dst_stem = dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(dst_dir) = dst.parent().map(Path::to_path_buf) else { return vec![] };
    let mut out = vec![];
    for c in comps {
        let Some(cname) = c.file_name().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        let Some(tail) = cname.get(src_stem.len()..) else { continue };
        let want = dst_dir.join(format!("{dst_stem}{tail}"));
        // 目标和原位置一样，就不动它
        if same_path(&want, c) {
            continue;
        }
        let fin = unique(&want, c, taken);
        out.push(Extra {
            src: c.to_string_lossy().into_owned(),
            dst: fin.to_string_lossy().into_owned(),
        });
    }
    out
}

fn list_dir_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default()
}

// ---------- 扫描（预览） ----------

fn walk_files(root: &Path, excludes: &[PathBuf]) -> Vec<PathBuf> {
    let ex = excludes.to_vec();
    let walker = jwalk::WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(move |_, _, _, children| {
            children.retain(|c| match c {
                Ok(e) if e.file_type().is_dir() => {
                    let p = e.path();
                    !scanner::skip_dir(&e.file_name().to_string_lossy()) && !ex.iter().any(|x| p.starts_with(x))
                }
                Ok(_) => true,
                Err(_) => false,
            });
        });
    walker.into_iter().flatten().filter(|e| e.file_type().is_file()).map(|e| e.path()).collect()
}

fn type_ok(types: &str, ext: &str) -> bool {
    match types {
        "video" => parse::VIDEO.contains(&ext),
        "music" => parse::AUDIO.contains(&ext) || parse::ENCRYPTED.contains(&ext),
        _ => true,
    }
}

struct Pending {
    src: PathBuf,
    g: Guess,
    size: u64,
    dst: Option<PathBuf>,
}

/// 扫描并生成预览列表。不改动任何文件
fn scan_core(roots: &[PathBuf], types: &str, target: &Path, st: &OrgSettings, covers: &Path, em: &[crate::em::organize::OrgRule]) -> Result<Vec<Item>> {
    if !matches!(types, "all" | "video" | "music") {
        bail!("未知的整理范围：{types}");
    }
    if roots.is_empty() {
        bail!("请先添加要整理的文件夹");
    }
    if !target.is_absolute() {
        bail!("请选择整理到的文件夹");
    }
    check_path_allowed(target)?;
    let excludes: Vec<PathBuf> = st.exclude.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    let mut files: Vec<PathBuf> = vec![];
    for r in roots {
        if !r.is_dir() {
            bail!("{} 不是文件夹", r.display());
        }
        check_path_allowed(r)?;
        files.extend(walk_files(r, &excludes));
    }
    files.sort();
    files.dedup();

    // 按目录分组，找配套文件
    let mut by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for f in &files {
        if let Some(d) = f.parent() {
            by_dir.entry(d.to_path_buf()).or_default().push(f.clone());
        }
    }
    let mut companions: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for list in by_dir.values() {
        companions.extend(assign_companions(list));
    }

    let mut pend: Vec<Pending> = vec![];
    for f in &files {
        let Some(kind) = parse::classify(f) else { continue };
        if !type_ok(types, &parse::ext(f)) {
            continue;
        }
        // 先试 .em 整理规则（第一条匹配的说了算），都不匹配再用自带的识别
        let (g, own_tpl) = match crate::em::organize::match_path(em, f) {
            Some(m) => (rule_guess(&m, f), m.template),
            None => (
                match kind {
                    "music" => guess_music(f, covers),
                    "novel" => guess_novel(f),
                    "manga" => guess_manga(f),
                    _ => guess_video(f),
                },
                None,
            ),
        };
        let size = fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        let dst = g
            .tpl
            .as_ref()
            .map(|(key, v)| render_dst(target, &own_tpl.clone().unwrap_or_else(|| template_for(st, key)), v, &parse::ext(f)));
        pend.push(Pending { src: f.clone(), g, size, dst });
    }

    // 同一集的多个版本：保留画质高的（同画质取文件大的），其余移到“重复待处理”
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in pend.iter().enumerate() {
        if let Some(k) = &p.g.ep_key {
            groups.entry(k.clone()).or_default().push(i);
        }
    }
    let mut dup_groups: Vec<Vec<usize>> = groups.into_values().filter(|v| v.len() > 1).collect();
    dup_groups.sort();
    for idxs in dup_groups {
        let mut order = idxs.clone();
        order.sort_by(|&a, &b| {
            (pend[b].g.res, pend[b].size)
                .cmp(&(pend[a].g.res, pend[a].size))
                .then(pend[a].src.cmp(&pend[b].src))
        });
        let keeper = order[0];
        let keeper_name = pend[keeper].src.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let n = idxs.len();
        pend[keeper].g.reason = format!("{}；同一集有 {n} 个版本，建议保留这个", pend[keeper].g.reason);
        for &i in &order[1..] {
            let name = pend[i].dst.as_ref().and_then(|d| d.file_name().map(|s| s.to_os_string()));
            if let Some(name) = name {
                pend[i].dst = Some(target.join(DUP_DIR).join(name));
            }
            pend[i].g.conf = "mid";
            pend[i].g.reason = format!("同一集有 {n} 个版本，建议保留画质更高的（{keeper_name}），这个移到“{DUP_DIR}”");
        }
    }

    pend.sort_by(|a, b| a.src.cmp(&b.src));
    let mut taken: HashSet<String> = HashSet::new();
    for p in pend.iter_mut() {
        if let Some(d) = p.dst.take() {
            if same_path(&d, &p.src) {
                // 已经在目标位置了
                p.g.reason = format!("{}；已经在整理好的位置", p.g.reason);
                p.g.conf = "low";
            } else {
                p.dst = Some(unique(&d, &p.src, &mut taken));
            }
        }
    }
    let mut out: Vec<Item> = vec![];
    for p in pend.iter_mut() {
        let extras = match &p.dst {
            Some(d) => extras_with(&companions, &p.src, d, &mut taken),
            None => vec![],
        };
        let dst = p.dst.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(Item {
            src: p.src.to_string_lossy().into_owned(),
            checked: !dst.is_empty() && p.g.conf != "low",
            dst,
            kind: p.g.kind.clone(),
            confidence: p.g.conf.into(),
            reason: p.g.reason.clone(),
            extras,
        });
    }
    Ok(out)
}

// ---------- AI 识别 ----------

const AI_SYSTEM: &str = "你是影视和番剧文件的整理助手。只根据文件名和文件夹名判断，不要编造作品。只返回 JSON。";

/// 认不出的视频文件（需要交给 AI 的）
fn ai_candidates(items: &[Item]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, it)| it.dst.is_empty() && it.kind == "other" && parse::VIDEO.contains(&parse::ext(Path::new(&it.src)).as_str()))
        .map(|(i, _)| i)
        .collect()
}

/// 提示词：只有文件名和文件夹名，不含文件内容
fn ai_prompt(items: &[Item], idxs: &[usize]) -> String {
    let mut s = String::from("下面是一些无法自动识别的视频文件（只提供文件名和所在文件夹名，不含文件内容）：\n");
    for &i in idxs {
        let p = Path::new(&items[i].src);
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folders = folder_names(p, 2).into_iter().rev().collect::<Vec<_>>().join("/");
        s += &format!("#{i} 文件名：{name}；所在文件夹：{folders}\n");
    }
    s += "\n请判断每一项：kind 取 anime（番剧）、tv（电视剧）、movie（电影）或 other（无法判断）；title 作品名（优先中文名，没有就用原名或罗马音）；year 年份（不确定填 null）；season 季（默认 1）；episode 集（不是单集填 null）；confidence 把握，mid 或 low；reason 一句话理由。\n";
    s += "只返回 JSON：{\"items\": [{\"index\": 0, \"kind\": \"anime\", \"title\": \"作品名\", \"year\": 2024, \"season\": 1, \"episode\": 3, \"confidence\": \"mid\", \"reason\": \"理由\"}]}";
    s
}

/// 把 AI 的结果写回条目。AI 的把握最高只给 mid，不会直接勾选为高把握
fn apply_ai(items: &mut [Item], v: &serde_json::Value, st: &OrgSettings, target: &Path) -> usize {
    let mut taken: HashSet<String> = items
        .iter()
        .filter(|i| !i.dst.is_empty())
        .flat_map(|i| std::iter::once(i.dst.clone()).chain(i.extras.iter().map(|x| x.dst.clone())))
        .map(|d| d.to_lowercase())
        .collect();
    let Some(list) = v["items"].as_array() else { return 0 };
    let mut n = 0;
    for r in list {
        let Some(idx) = r["index"].as_u64().map(|x| x as usize) else { continue };
        let Some(it) = items.get_mut(idx) else { continue };
        if it.kind != "other" || !it.dst.is_empty() {
            continue;
        }
        let kind = r["kind"].as_str().unwrap_or("other");
        let title = r["title"].as_str().unwrap_or("").trim().to_string();
        let episode = r["episode"].as_f64();
        if title.is_empty() || !matches!(kind, "anime" | "tv" | "movie") {
            continue;
        }
        if kind != "movie" && episode.is_none() {
            continue;
        }
        let year = r["year"].as_i64().map(|y| y.to_string()).unwrap_or_default();
        let season = r["season"].as_i64().unwrap_or(1);
        let conf: &'static str = if r["confidence"].as_str() == Some("mid") { "mid" } else { "low" };
        let src = PathBuf::from(&it.src);
        let ext = parse::ext(&src);
        let (key, v) = if kind == "movie" {
            ("movie", vars(&[("片名", title.clone()), ("作品名", title.clone()), ("年份", year)]))
        } else {
            let key = if kind == "anime" { "anime" } else { "tv" };
            (
                key,
                vars(&[
                    ("作品名", title.clone()),
                    ("年份", year),
                    ("季", season.to_string()),
                    ("集", ep_text(episode.unwrap_or(0.0))),
                ]),
            )
        };
        let planned = render_dst(target, &template_for(st, key), &v, &ext);
        let dst = unique(&planned, &src, &mut taken);
        let parent = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let map = assign_companions(&list_dir_files(&parent));
        let extras = extras_with(&map, &src, &dst, &mut taken);
        it.kind = kind.to_string();
        it.confidence = conf.into();
        it.reason = format!("AI 判断：{}", r["reason"].as_str().unwrap_or("根据文件名和文件夹名推测"));
        it.dst = dst.to_string_lossy().into_owned();
        it.extras = extras;
        it.checked = conf != "low";
        n += 1;
    }
    n
}

// ---------- 执行 ----------

/// 同一个盘（盘符相同）
fn same_drive(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| {
        p.components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    root(a) == root(b)
}

fn is_cross_device(e: &io::Error) -> bool {
    // ERROR_NOT_SAME_DEVICE(17) / EXDEV(18)
    matches!(e.raw_os_error(), Some(17) | Some(18))
}

/// 复制到一个新文件（不覆盖已有文件），校验大小一致。失败时删掉不完整的副本
fn copy_new(src: &Path, dst: &Path) -> Result<()> {
    let mut i = File::open(src).context("文件被占用或无法读取，已跳过")?;
    let mut o = OpenOptions::new().write(true).create_new(true).open(dst).context("目标位置已有同名文件")?;
    let res = io::copy(&mut i, &mut o).and_then(|_| o.sync_all());
    drop(o);
    if let Err(e) = res {
        let _ = fs::remove_file(dst);
        return Err(e.into());
    }
    let (a, b) = (fs::metadata(src)?.len(), fs::metadata(dst)?.len());
    if a != b {
        let _ = fs::remove_file(dst);
        bail!("复制后大小不一致，原文件保留");
    }
    Ok(())
}

/// 移动：同盘直接改名；跨盘先复制并校验，再删源
fn move_file(src: &Path, dst: &Path) -> Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            copy_new(src, dst)?;
            if let Err(e) = fs::remove_file(src) {
                // 删源失败：撤掉副本，两边保持原样
                let _ = fs::remove_file(dst);
                return Err(e).context("复制已完成，但删除原文件失败，已撤回副本");
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn run_one(
    db: &Mutex<Connection>,
    dirs: &indexer::Dirs,
    batch_id: i64,
    src: &Path,
    planned: &Path,
    mode: &str,
    taken: &mut HashSet<String>,
) -> Result<PathBuf> {
    check_path_allowed(src)?;
    check_path_allowed(planned)?;
    if !src.is_file() {
        bail!("找不到源文件");
    }
    // 打不开（多半被占用）就跳过
    File::open(src).context("文件被占用或无法读取，已跳过")?;
    let dst = unique(planned, src, taken);
    if dst.to_string_lossy().chars().count() > 255 {
        bail!("目标路径过长");
    }
    let src_s = src.to_string_lossy().into_owned();
    let dst_s = dst.to_string_lossy().into_owned();
    let lib_kind: Option<String> = {
        let conn = db.lock();
        conn.query_row("SELECT kind FROM files WHERE path=?1", [&src_s], |r| r.get(0)).optional()?
    };
    if mode != "library" {
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p).context("无法创建目标文件夹")?;
        }
    }
    let row = {
        let conn = db.lock();
        conn.execute(
            "INSERT INTO org_moves(batch_id,src,dst,done) VALUES(?1,?2,?3,0)",
            params![batch_id, src_s, dst_s],
        )?;
        conn.last_insert_rowid()
    };
    let res: Result<()> = match mode {
        "move" => move_file(src, &dst),
        "copy" => copy_new(src, &dst),
        "hardlink" => fs::hard_link(src, &dst).context("硬链接失败（要求源文件和目标在同一个盘）"),
        _ => Ok(()),
    };
    if let Err(e) = res {
        let conn = db.lock();
        conn.execute("DELETE FROM org_moves WHERE rowid=?1", [row])?;
        return Err(e);
    }
    let conn = db.lock();
    conn.execute("UPDATE org_moves SET done=1 WHERE rowid=?1", [row])?;
    match mode {
        "move" => {
            conn.execute("UPDATE files SET path=?1 WHERE path=?2", params![dst_s, src_s])?;
        }
        "copy" | "hardlink" => {
            // 新副本加入媒体库（原文件仍在原位置，媒体库里的记录不动）
            if let Some(k) = lib_kind {
                let _ = indexer::index_file(&conn, &dst, &k, dirs);
            }
        }
        _ => {}
    }
    Ok(dst)
}

/// 执行整理。只执行勾选的、有目标位置的条目；配套文件跟着主文件走
pub fn execute_core(
    db: &Mutex<Connection>,
    dirs: &indexer::Dirs,
    items: &[Item],
    mode: &str,
    cleanup: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<ExecReport> {
    if !matches!(mode, "move" | "copy" | "hardlink" | "library") {
        bail!("未知的整理方式：{mode}");
    }
    // (源, 目标, 所属条目, 是否主文件)
    let mut ops: Vec<(PathBuf, PathBuf, usize, bool)> = vec![];
    for (gi, it) in items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.checked && !i.dst.is_empty() && !same_path(Path::new(&i.src), Path::new(&i.dst)))
    {
        ops.push((PathBuf::from(&it.src), PathBuf::from(&it.dst), gi, true));
        for x in &it.extras {
            if !x.dst.is_empty() && !same_path(Path::new(&x.src), Path::new(&x.dst)) {
                ops.push((PathBuf::from(&x.src), PathBuf::from(&x.dst), gi, false));
            }
        }
    }
    if mode == "hardlink" {
        for (s, d, _, _) in &ops {
            if !same_drive(s, d) {
                bail!("硬链接要求源文件和目标在同一个盘，这些不在同一个盘：{}", s.display());
            }
        }
    }
    let total = ops.len();
    let batch_id = {
        let conn = db.lock();
        conn.execute("INSERT INTO org_batches(created_at,mode) VALUES(?1,?2)", params![db::now(), mode])?;
        conn.last_insert_rowid()
    };
    let mut done = 0usize;
    let mut failed: Vec<Failed> = vec![];
    let mut bad_items: HashSet<usize> = HashSet::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut dst_list: Vec<PathBuf> = vec![];
    let mut parents: Vec<PathBuf> = vec![];
    for (n, (src, planned, gi, main)) in ops.iter().enumerate() {
        progress(Progress {
            done: n,
            total,
            current: src.to_string_lossy().into_owned(),
            phase: mode.into(),
        });
        if bad_items.contains(gi) {
            failed.push(Failed {
                path: src.to_string_lossy().into_owned(),
                error: "主文件没有整理成功，配套文件一起跳过".into(),
            });
            continue;
        }
        match run_one(db, dirs, batch_id, src, planned, mode, &mut taken) {
            Ok(dst) => {
                done += 1;
                dst_list.push(dst);
                if mode == "move" {
                    if let Some(p) = src.parent() {
                        parents.push(p.to_path_buf());
                    }
                }
            }
            Err(e) => {
                if *main {
                    bad_items.insert(*gi);
                }
                failed.push(Failed {
                    path: src.to_string_lossy().into_owned(),
                    error: format!("{e:#}"),
                });
            }
        }
    }
    progress(Progress {
        done: total,
        total,
        current: String::new(),
        phase: "done".into(),
    });

    let batch = if done == 0 {
        let conn = db.lock();
        conn.execute("DELETE FROM org_batches WHERE id=?1", [batch_id])?;
        None
    } else {
        Some(batch_id)
    };
    if cleanup && mode == "move" {
        remove_empty_dirs(&parents, &dst_list);
    }
    Ok(ExecReport { batch_id: batch, done, failed })
}

/// 只删除完全为空的文件夹；不删目标文件夹本身或它的上级
fn remove_empty_dirs(parents: &[PathBuf], dsts: &[PathBuf]) {
    let mut ps: Vec<PathBuf> = parents.to_vec();
    ps.sort();
    ps.dedup();
    ps.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in ps {
        if d.parent().is_none() || check_path_allowed(&d).is_err() {
            continue;
        }
        if dsts.iter().any(|x| x.starts_with(&d)) {
            continue;
        }
        let empty = fs::read_dir(&d).map(|mut rd| rd.next().is_none()).unwrap_or(false);
        if empty {
            let _ = fs::remove_dir(&d);
        }
    }
}

/// 撤销一批：倒序把文件移回原位，并改回媒体库里的路径
pub fn undo_core(db: &Mutex<Connection>, batch_id: i64, progress: &mut dyn FnMut(Progress)) -> Result<UndoReport> {
    let (mode, undone) = {
        let conn = db.lock();
        conn.query_row("SELECT mode, undone FROM org_batches WHERE id=?1", [batch_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .context("找不到这一批整理记录")?
    };
    if undone != 0 {
        bail!("这一批已经撤销过了");
    }
    let rows: Vec<(i64, String, String)> = {
        let conn = db.lock();
        let mut st = conn.prepare("SELECT rowid,src,dst FROM org_moves WHERE batch_id=?1 AND done=1 ORDER BY rowid DESC")?;
        let v = st
            .query_map([batch_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        v
    };
    let total = rows.len();
    let mut reverted = 0usize;
    let mut failed: Vec<Failed> = vec![];
    for (n, (row, src, dst)) in rows.iter().enumerate() {
        progress(Progress {
            done: n,
            total,
            current: dst.clone(),
            phase: "undo".into(),
        });
        match undo_one(db, &mode, src, dst) {
            Ok(()) => {
                let conn = db.lock();
                conn.execute("UPDATE org_moves SET done=0 WHERE rowid=?1", [row])?;
                reverted += 1;
            }
            Err(e) => failed.push(Failed {
                path: dst.clone(),
                error: format!("{e:#}"),
            }),
        }
    }
    progress(Progress {
        done: total,
        total,
        current: String::new(),
        phase: "done".into(),
    });
    let finished = {
        let conn = db.lock();
        let left: i64 = conn.query_row("SELECT count(*) FROM org_moves WHERE batch_id=?1 AND done=1", [batch_id], |r| r.get(0))?;
        if left == 0 {
            conn.execute("UPDATE org_batches SET undone=1 WHERE id=?1", [batch_id])?;
            true
        } else {
            false
        }
    };
    Ok(UndoReport { reverted, failed, finished })
}

fn undo_one(db: &Mutex<Connection>, mode: &str, src: &str, dst: &str) -> Result<()> {
    let (sp, dp) = (Path::new(src), Path::new(dst));
    match mode {
        "move" => {
            if !dp.exists() {
                bail!("找不到 {dst}，可能已经被移走");
            }
            if sp.exists() {
                bail!("原位置已经有同名文件，没有覆盖");
            }
            if let Some(p) = sp.parent() {
                fs::create_dir_all(p)?;
            }
            move_file(dp, sp)?;
            let conn = db.lock();
            conn.execute("UPDATE files SET path=?1 WHERE path=?2", params![src, dst])?;
        }
        "copy" | "hardlink" => {
            // 副本是我们建的：放进回收站，不直接删除；原文件不动
            if dp.exists() {
                trash::delete(dp).context("移到回收站失败")?;
            }
            let conn = db.lock();
            conn.execute("DELETE FROM files WHERE path=?1", [dst])?;
        }
        _ => {}
    }
    Ok(())
}

pub fn batches_core(conn: &Connection) -> Result<Vec<BatchInfo>> {
    let mut st = conn.prepare(
        "SELECT b.id,b.created_at,b.mode,b.undone,(SELECT count(*) FROM org_moves m WHERE m.batch_id=b.id AND m.done=1)
         FROM org_batches b ORDER BY b.id DESC LIMIT 50",
    )?;
    let v = st
        .query_map([], |r| {
            Ok(BatchInfo {
                id: r.get(0)?,
                created_at: r.get(1)?,
                mode: r.get(2)?,
                undone: r.get::<_, i64>(3)? != 0,
                count: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

// ---------- 命令 ----------

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

/// 扫描：返回预览列表（不改动文件）
#[tauri::command]
pub async fn org_scan(app: AppHandle, roots: Vec<String>, types: String, target_root: String) -> Result<Vec<Item>, String> {
    let (settings, covers) = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        (load_settings(&conn), st.dirs.covers.clone())
    };
    let roots: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let em = crate::em::organize::active();
        scan_core(&roots, &types, Path::new(&target_root), &settings, &covers, &em)
    })
    .await
    .map_err(e)?
    .map_err(e)
}

/// 交给 AI 之前：算出需要识别的文件数和预计 token
#[tauri::command]
pub fn org_estimate(items: Vec<Item>) -> Estimate {
    let idx = ai_candidates(&items);
    let tokens = if idx.is_empty() { 0 } else { ai::estimate_tokens(&ai_prompt(&items, &idx)) };
    Estimate { files: idx.len(), tokens }
}

/// 把认不出的视频交给 AI（只发文件名和文件夹名），返回更新后的条目。没有配置 AI 时返回错误
#[tauri::command]
pub async fn org_ai_identify(app: AppHandle, items: Vec<Item>, target_root: String) -> Result<Vec<Item>, String> {
    ai::ready(&app).map_err(e)?;
    let target = PathBuf::from(&target_root);
    check_path_allowed(&target).map_err(e)?;
    let settings = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        load_settings(&conn)
    };
    let mut items = items;
    let idx = ai_candidates(&items);
    for chunk in idx.chunks(40) {
        let prompt = ai_prompt(&items, chunk);
        let reply = ai::chat_simple(&app, AI_SYSTEM, &prompt, true).await.map_err(e)?;
        let v = ai::extract_json(&reply).map_err(e)?;
        apply_ai(&mut items, &v, &settings, &target);
    }
    Ok(items)
}

/// 执行整理。mode：move / copy / hardlink / library。cleanup_empty 为 true 时清理变空的旧文件夹
#[tauri::command]
pub async fn org_execute(app: AppHandle, items: Vec<Item>, mode: String, cleanup_empty: Option<bool>) -> Result<ExecReport, String> {
    let app2 = app.clone();
    let cleanup = cleanup_empty.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let mut emit = |p: Progress| {
            let _ = app2.emit("org://progress", p);
        };
        let r = execute_core(&st.db, &st.dirs, &items, &mode, cleanup, &mut emit);
        let _ = app2.emit("library://changed", ());
        r
    })
    .await
    .map_err(e)?
    .map_err(e)
}

/// 撤销整批
#[tauri::command]
pub async fn org_undo(app: AppHandle, batch_id: i64) -> Result<UndoReport, String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let mut emit = |p: Progress| {
            let _ = app2.emit("org://progress", p);
        };
        let r = undo_core(&st.db, batch_id, &mut emit);
        let _ = app2.emit("library://changed", ());
        r
    })
    .await
    .map_err(e)?
    .map_err(e)
}

/// 历史批次（新到旧）
#[tauri::command]
pub fn org_batches(st: State<AppState>) -> Result<Vec<BatchInfo>, String> {
    let conn = st.db.lock();
    batches_core(&conn).map_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let d = std::env::temp_dir().join(format!("org-test-{tag}-{nanos}"));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn dirs_for(d: &Path) -> indexer::Dirs {
        indexer::Dirs {
            data: d.to_path_buf(),
            covers: d.to_path_buf(),
            thumbs: d.to_path_buf(),
        }
    }

    fn org_rule(text: &str) -> crate::em::organize::OrgRule {
        crate::em::organize::compile(&crate::em::format::parse(text).unwrap()).unwrap()
    }

    #[test]
    fn em_rule_matches_first_and_falls_through() {
        let root = temp_dir("em");
        let src = root.join("下载");
        fs::create_dir_all(&src).unwrap();
        for n in [
            "[Nekomoe kissaten] 葬送的芙莉莲 - 05 [1080p].mkv",
            "[Nekomoe kissaten] 葬送的芙莉莲 - 05 [1080p].sc.ass",
            "[LoliHouse] 葬送的芙莉莲 第二季 - 03 [1080p].mkv",
            "随便一个视频.mp4",
        ] {
            fs::write(src.join(n), b"x").unwrap();
        }
        let target = root.join("媒体");
        let rule = org_rule(include_str!("../../tools/em-examples/anime-release.em"));
        let items = scan_core(std::slice::from_ref(&src), "all", &target, &OrgSettings::default(), &root, &[rule]).unwrap();
        let by = |part: &str| items.iter().find(|i| i.src.contains(part)).unwrap_or_else(|| panic!("{part}: {items:?}"));
        let a = by("- 05");
        assert_eq!(a.confidence, "hi");
        assert_eq!(a.reason, "规则「字幕组发布名」");
        assert_eq!(a.kind, "anime");
        assert!(a.checked);
        assert_eq!(
            PathBuf::from(&a.dst),
            target.join("番剧").join("葬送的芙莉莲").join("Season 01").join("葬送的芙莉莲 - S01E05.mkv")
        );
        // 配套字幕跟着走
        assert_eq!(a.extras.len(), 1);
        assert!(a.extras[0].dst.ends_with("葬送的芙莉莲 - S01E05.sc.ass"), "{:?}", a.extras);
        // 第二种文件名没有自己的模板：用默认的番剧模板，季号认中文数字
        let b = by("第二季");
        assert_eq!(
            PathBuf::from(&b.dst),
            target.join("番剧").join("葬送的芙莉莲").join("Season 02").join("葬送的芙莉莲 - S02E03.mkv")
        );
        // 不匹配的走原来的识别
        let c = by("随便");
        assert_ne!(c.reason, "规则「字幕组发布名」");
        assert_ne!(c.confidence, "hi");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rule_dst_uses_settings_template_and_fields() {
        let rule = org_rule(
            "em: 1\nkind: organize\nid: music-x\nname: 音乐\nrules:\n  - match: '^(?P<artist>.+?) - (?P<album>.+?) - (?P<track>\\d+) (?P<title>.+)\\.'\n    media: music\n",
        );
        let p = Path::new("/dl/林间信号 - 潮汐档案 - 3 雾港.flac");
        let m = crate::em::organize::match_path(std::slice::from_ref(&rule), p).unwrap();
        let mut st = OrgSettings::default();
        st.templates.insert("music".into(), "歌/{艺人}/{专辑}/{音轨:02} {歌名}.ext".into());
        let d = rule_dst(&m, p, Path::new("/m"), &st);
        assert_eq!(d, PathBuf::from("/m/歌/林间信号/潮汐档案/03 雾港.flac"));
        assert_eq!(num_of("十二"), Some(12));
        assert_eq!(num_of("二十"), Some(20));
        assert_eq!(num_of("03"), Some(3));
    }

    #[test]
    fn anime_template_and_missing_year() {
        let v = vars(&[("作品名", "星屑邮局".into()), ("年份", String::new()), ("季", "1".into()), ("集", "3".into())]);
        let p = render_dst(Path::new("D:\\Media"), default_template("anime"), &v, "mkv");
        assert_eq!(
            p,
            PathBuf::from("D:\\Media")
                .join("番剧")
                .join("星屑邮局")
                .join("Season 01")
                .join("星屑邮局 - S01E03.mkv")
        );
        let v = vars(&[
            ("作品名", "星屑邮局".into()),
            ("年份", "2024".into()),
            ("季", "2".into()),
            ("集", "12.5".into()),
        ]);
        let p = render_dst(Path::new("D:\\Media"), default_template("anime"), &v, "mp4");
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "星屑邮局 - S02E12.5.mp4");
        assert!(p.to_string_lossy().contains("星屑邮局 (2024)"));
    }

    #[test]
    fn music_template() {
        let v = vars(&[
            ("艺人", "林间信号".into()),
            ("专辑", "潮汐档案".into()),
            ("年份", "2024".into()),
            ("音轨", "1".into()),
            ("歌名", "潮汐档案 · 序".into()),
        ]);
        let p = render_dst(Path::new("D:\\Media"), default_template("music"), &v, "flac");
        assert_eq!(
            p,
            PathBuf::from("D:\\Media")
                .join("音乐")
                .join("林间信号")
                .join("潮汐档案 (2024)")
                .join("01 - 潮汐档案 · 序.flac")
        );
        let v = vars(&[
            ("艺人", "A".into()),
            ("专辑", "B".into()),
            ("年份", String::new()),
            ("音轨", String::new()),
            ("歌名", "C".into()),
        ]);
        let p = render_dst(Path::new("D:\\Media"), default_template("music"), &v, "mp3");
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "C.mp3");
    }

    #[test]
    fn sanitize_illegal_and_reserved() {
        assert_eq!(sanitize("Re:Zero?"), "Re：Zero？");
        assert_eq!(sanitize("a/b\\c"), "a／b＼c");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("COM1.txt"), "_COM1.txt");
        assert_eq!(sanitize("name. "), "name");
        assert_eq!(sanitize("..."), "_");
        assert_eq!(sanitize("  "), "");
    }

    #[test]
    fn long_names_are_shortened() {
        let long = "a".repeat(300);
        let v = vars(&[("作品名", long.clone()), ("年份", String::new()), ("季", "1".into()), ("集", "1".into())]);
        let p = render_dst(Path::new("D:\\Media"), default_template("anime"), &v, "mkv");
        assert!(p.to_string_lossy().chars().count() <= MAX_PATH);
        assert!(p.to_string_lossy().ends_with(".mkv"));
    }

    #[test]
    fn special_detection() {
        assert_eq!(special_label("雾港异闻录 - NCOP").as_deref(), Some("NCOP"));
        assert_eq!(special_label("Show OVA 01").as_deref(), Some("OVA"));
        assert_eq!(special_label("雾港异闻录 - 07"), None);
        assert_eq!(special_label("Operation Stone"), None);
    }

    #[test]
    fn duplicate_names_get_numbers() {
        let d = temp_dir("unique");
        fs::write(d.join("x.mkv"), b"1").unwrap();
        let src = d.join("other.mkv");
        let mut taken = HashSet::new();
        let a = unique(&d.join("x.mkv"), &src, &mut taken);
        assert_eq!(a.file_name().unwrap().to_string_lossy(), "x (2).mkv");
        // 同一批里已经占用的名字也要跳过
        let b = unique(&d.join("x (2).mkv"), &src, &mut taken);
        assert_eq!(b.file_name().unwrap().to_string_lossy(), "x (2) (2).mkv");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn companions_follow_longest_stem() {
        let list: Vec<PathBuf> = ["a.mkv", "a.zh.ass", "a.ttf", "a.v2.mkv", "a.v2.zh.ass", "a.lrc"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let map = assign_companions(&list);
        let mine: Vec<String> = map.get(Path::new("a.mkv")).unwrap().iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert!(mine.contains(&"a.zh.ass".to_string()));
        assert!(mine.contains(&"a.ttf".to_string()));
        assert!(!mine.contains(&"a.v2.zh.ass".to_string()));
        let v2: Vec<String> = map
            .get(Path::new("a.v2.mkv"))
            .unwrap()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(v2, vec!["a.v2.zh.ass".to_string()]);
    }

    #[test]
    fn ai_reply_is_applied_with_mid_cap() {
        let mut items = vec![Item {
            src: "D:\\下载\\xxyj_s1_04_final.mp4".into(),
            kind: "other".into(),
            confidence: "low".into(),
            ..Default::default()
        }];
        let v = serde_json::json!({"items": [{"index": 0, "kind": "anime", "title": "星屑邮局", "year": 2024, "season": 1, "episode": 4, "confidence": "hi", "reason": "拼音缩写"}]});
        let n = apply_ai(&mut items, &v, &OrgSettings::default(), Path::new("D:\\Media"));
        assert_eq!(n, 1);
        assert_eq!(items[0].confidence, "low");
        assert!(!items[0].checked);
        assert!(items[0].dst.ends_with("星屑邮局 - S01E04.mp4"));
        assert!(items[0].reason.starts_with("AI 判断"));
    }

    #[test]
    fn undo_reads_newest_first() {
        let db = Mutex::new(db::open_memory().unwrap());
        {
            let c = db.lock();
            c.execute("INSERT INTO org_batches(created_at,mode) VALUES(1,'move')", []).unwrap();
            for i in 0..3 {
                c.execute(
                    "INSERT INTO org_moves(batch_id,src,dst,done) VALUES(1,?1,?2,1)",
                    params![format!("s{i}"), format!("d{i}")],
                )
                .unwrap();
            }
        }
        let c = db.lock();
        let mut st = c.prepare("SELECT src FROM org_moves WHERE batch_id=1 AND done=1 ORDER BY rowid DESC").unwrap();
        let order: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        assert_eq!(order, vec!["s2", "s1", "s0"]);
    }

    #[test]
    fn real_move_then_undo() {
        let d = temp_dir("move");
        let src_dir = d.join("下载");
        let dst_dir = d.join("Media");
        fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("[Hoshi] Show - 03.mkv");
        let sub = src_dir.join("[Hoshi] Show - 03.zh.ass");
        fs::write(&src, b"video-bytes").unwrap();
        fs::write(&sub, b"sub").unwrap();
        let dst = dst_dir.join("番剧").join("Show").join("Season 01").join("Show - S01E03.mkv");
        let sub_dst = dst.with_extension("zh.ass");

        let db = Mutex::new(db::open_memory().unwrap());
        db.lock()
            .execute(
                "INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,'anime',11,0,0)",
                [src.to_string_lossy().into_owned()],
            )
            .unwrap();
        let file_id: i64 = db.lock().query_row("SELECT id FROM files", [], |r| r.get(0)).unwrap();

        let item = Item {
            src: src.to_string_lossy().into_owned(),
            dst: dst.to_string_lossy().into_owned(),
            kind: "anime".into(),
            confidence: "hi".into(),
            checked: true,
            extras: vec![Extra {
                src: sub.to_string_lossy().into_owned(),
                dst: sub_dst.to_string_lossy().into_owned(),
            }],
            ..Default::default()
        };
        let dirs = dirs_for(&d);
        let mut seen = 0;
        let rep = execute_core(&db, &dirs, &[item], "move", true, &mut |_| seen += 1).unwrap();
        assert_eq!(rep.done, 2);
        assert!(rep.failed.is_empty());
        let batch = rep.batch_id.unwrap();
        assert!(dst.exists() && sub_dst.exists());
        assert!(!src.exists() && !sub.exists());
        // 媒体库里的 id 不变，路径改成新位置
        let path: String = db.lock().query_row("SELECT path FROM files WHERE id=?1", [file_id], |r| r.get(0)).unwrap();
        assert_eq!(path, dst.to_string_lossy());
        // 空的旧文件夹被清理
        assert!(!src_dir.exists());

        let u = undo_core(&db, batch, &mut |_| {}).unwrap();
        assert_eq!(u.reverted, 2);
        assert!(u.finished);
        assert!(src.exists() && sub.exists());
        assert!(!dst.exists() && !sub_dst.exists());
        let path: String = db.lock().query_row("SELECT path FROM files WHERE id=?1", [file_id], |r| r.get(0)).unwrap();
        assert_eq!(path, src.to_string_lossy());
        // 撤销过的批次不能再撤销
        assert!(undo_core(&db, batch, &mut |_| {}).is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}
