//! 在线歌词：各歌词源查候选 → 打分挑最合适的一份 → 存 SQLite 缓存。
//! 本地同名 .lrc 和内嵌歌词优先（metadata::lyrics_for），这里只在本地没有时才用。
//! 子模块：lrclib（LRCLIB 开放歌词库）。以后的网易云 / QQ / 酷狗 / AMLL 也按同样的形状加进 `Source`。
pub mod lrclib;

use crate::metadata::LyricLine;
use crate::{db, library, AppState};
use anyhow::Result;
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;

/// 查不到的歌过多久再查一次（秒）
const MISS_TTL: i64 = 3 * 24 * 3600;
/// 时长相差超过这么多秒就认为不是同一个版本
const MAX_DURATION_DIFF: f64 = 5.0;
/// 低于这个分数的候选直接丢掉
const MIN_SCORE: f64 = 0.6;

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS lyrics_cache(
            key TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            lines TEXT NOT NULL,
            fetched_at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

// ---------------------------------------------------------------- 数据结构

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Lrclib,
}

impl Source {
    pub fn parse(s: &str) -> Option<Source> {
        match s {
            "lrclib" => Some(Source::Lrclib),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Source::Lrclib => "lrclib",
        }
    }
}

/// 同步级别：越大越好
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sync {
    Plain,
    Line,
    Word,
}

/// 要找歌词的歌
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// 秒；不知道时为 0
    pub duration: f64,
}

impl Query {
    /// 标签里没有艺术家、标题却是“艺术家 - 标题”（文件名）时拆开
    pub fn new(title: &str, artist: &str, album: &str, duration: f64) -> Query {
        let (mut title, mut artist) = (title.trim().to_string(), artist.trim().to_string());
        if artist.is_empty() {
            if let Some((a, t)) = title.split_once(" - ") {
                (artist, title) = (a.trim().to_string(), t.trim().to_string());
            }
        }
        Query {
            title,
            artist,
            album: album.trim().to_string(),
            duration,
        }
    }

    pub fn cache_key(&self) -> String {
        format!("{}|{}|{}", norm(&self.title), norm(&self.artist), self.duration.round() as i64)
    }
}

/// 某个歌词源返回的一份歌词（已解析）
#[derive(Debug, Clone)]
pub struct Found {
    pub source: Source,
    pub title: String,
    pub artist: String,
    pub duration: Option<f64>,
    pub lines: Vec<LyricLine>,
}

impl Found {
    pub fn sync(&self) -> Sync {
        if self.lines.iter().any(|l| !l.words.is_empty()) {
            Sync::Word
        } else if self.lines.iter().any(|l| l.t >= 0.0) {
            Sync::Line
        } else {
            Sync::Plain
        }
    }
}

/// 发给前端的结果
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OnlineLyrics {
    pub source: Source,
    pub lines: Vec<LyricLine>,
}

// ---------------------------------------------------------------- 匹配打分

/// 归一化：全角转半角、小写、去掉括号里的版本说明（Live、伴奏…）、只留字母数字（含汉字）
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        let c = match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        };
        match c {
            '(' | '[' | '【' | '〔' | '{' => depth += 1,
            ')' | ']' | '】' | '〕' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => {}
        }
    }
    // 整个标题都在括号里时退回不去括号
    if out.is_empty() && depth == 0 && !s.is_empty() {
        return s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect();
    }
    out
}

fn lcs(a: &[char], b: &[char]) -> usize {
    let mut prev = vec![0usize; b.len() + 1];
    for &x in a {
        let mut cur = vec![0usize; b.len() + 1];
        for (j, &y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { prev[j + 1].max(cur[j]) };
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 0..=1 的相似度（已归一化的字符串）
fn sim(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.contains(b) || b.contains(a) {
        return 0.85;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    2.0 * lcs(&a, &b) as f64 / (a.len() + b.len()) as f64
}

fn split_artists(s: &str) -> Vec<String> {
    s.split(['/', ',', '&', ';', '、', '，', '；', '|', '＆'])
        .flat_map(|p| p.split(" feat. ").flat_map(|p| p.split(" ft. ")))
        .map(norm)
        .filter(|p| !p.is_empty())
        .collect()
}

/// 艺术家相似度：多位艺术家时取最像的一对；查询没有艺术家时给中性分
fn artist_sim(q: &str, c: &str) -> f64 {
    let (qs, cs) = (split_artists(q), split_artists(c));
    if qs.is_empty() {
        return 0.5;
    }
    let whole = sim(&norm(q), &norm(c));
    qs.iter().flat_map(|a| cs.iter().map(move |b| sim(a, b))).fold(whole, f64::max)
}

/// 候选和查询有多像；时长差太多或标题不像时返回 None
pub fn score(q: &Query, title: &str, artist: &str, duration: Option<f64>) -> Option<f64> {
    let t = sim(&norm(&q.title), &norm(title));
    if t < 0.6 {
        return None;
    }
    let d = match duration {
        Some(d) if d > 0.0 && q.duration > 0.0 => {
            let diff = (d - q.duration).abs();
            if diff > MAX_DURATION_DIFF {
                return None;
            }
            1.0 - diff / MAX_DURATION_DIFF
        }
        _ => 0.5,
    };
    let s = 0.55 * t + 0.3 * artist_sim(&q.artist, artist) + 0.15 * d;
    (s >= MIN_SCORE).then_some(s)
}

/// 从所有候选里挑一份：先比同步级别（逐字 > 逐行 > 纯文本），再比分数，再按歌词源顺序
pub fn pick(q: &Query, found: Vec<Found>, order: &[Source]) -> Option<Found> {
    let rank = |s: Source| order.iter().position(|x| *x == s).unwrap_or(usize::MAX);
    found
        .into_iter()
        .filter(|f| !f.lines.is_empty())
        .filter_map(|f| score(q, &f.title, &f.artist, f.duration).map(|s| (f, s)))
        .max_by(|(a, sa), (b, sb)| {
            a.sync()
                .cmp(&b.sync())
                .then(sa.partial_cmp(sb).unwrap_or(std::cmp::Ordering::Equal))
                .then(rank(b.source).cmp(&rank(a.source)))
        })
        .map(|(f, _)| f)
}

// ---------------------------------------------------------------- 设置

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlineCfg {
    pub enabled: bool,
    /// 按优先顺序
    pub sources: Vec<String>,
}

impl Default for OnlineCfg {
    fn default() -> Self {
        OnlineCfg {
            enabled: true,
            sources: vec!["lrclib".into()],
        }
    }
}

impl OnlineCfg {
    pub fn load(conn: &Connection) -> OnlineCfg {
        library::get_setting(conn, "lyrics_online")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
    pub fn source_list(&self) -> Vec<Source> {
        self.sources.iter().filter_map(|s| Source::parse(s)).collect()
    }
}

// ---------------------------------------------------------------- 缓存

/// Some(None) = 查过没有，还没过期；None = 没缓存
pub fn cache_get(conn: &Connection, key: &str, now: i64) -> Result<Option<Option<OnlineLyrics>>> {
    let row: Option<(String, String, i64)> = conn
        .query_row("SELECT source, lines, fetched_at FROM lyrics_cache WHERE key=?1", [key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((source, lines, at)) = row else { return Ok(None) };
    let Some(source) = Source::parse(&source) else {
        // 没查到的记录：过期就当没缓存
        return Ok((now - at < MISS_TTL).then_some(None));
    };
    let lines: Vec<LyricLine> = serde_json::from_str(&lines)?;
    Ok(Some(Some(OnlineLyrics { source, lines })))
}

pub fn cache_put(conn: &Connection, key: &str, v: Option<&OnlineLyrics>, now: i64) -> Result<()> {
    let (source, lines) = match v {
        Some(v) => (v.source.name(), serde_json::to_string(&v.lines)?),
        None => ("", "[]".to_string()),
    };
    conn.execute(
        "INSERT INTO lyrics_cache(key,source,lines,fetched_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(key) DO UPDATE SET source=excluded.source, lines=excluded.lines, fetched_at=excluded.fetched_at",
        params![key, source, lines, now],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- 查询

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("music-player/", env!("CARGO_PKG_VERSION"), " (local media player)"))
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default()
});

async fn search_source(src: Source, q: &Query) -> Result<Vec<Found>> {
    match src {
        Source::Lrclib => lrclib::find(&HTTP, q).await,
    }
}

/// 所有歌词源一起查，挑最合适的一份
pub async fn find(q: &Query, sources: &[Source]) -> Option<OnlineLyrics> {
    let results = futures_util::future::join_all(sources.iter().map(|&s| search_source(s, q))).await;
    let found: Vec<Found> = results.into_iter().filter_map(|r| r.ok()).flatten().collect();
    pick(q, found, sources).map(|f| OnlineLyrics {
        source: f.source,
        lines: f.lines,
    })
}

/// 本地没有歌词时由前端调用。在线歌词关掉时返回 None；`refresh` 跳过缓存重新查
#[tauri::command]
pub async fn lyrics_online(
    st: State<'_, AppState>,
    title: String,
    artist: String,
    album: String,
    duration: f64,
    refresh: Option<bool>,
) -> Result<Option<OnlineLyrics>, String> {
    let q = Query::new(&title, &artist, &album, duration);
    if q.title.is_empty() {
        return Ok(None);
    }
    let key = q.cache_key();
    let cfg = {
        let conn = st.db.lock();
        let cfg = OnlineCfg::load(&conn);
        if !cfg.enabled {
            return Ok(None);
        }
        if !refresh.unwrap_or(false) {
            if let Some(hit) = cache_get(&conn, &key, db::now()).map_err(|e| e.to_string())? {
                return Ok(hit);
            }
        }
        cfg
    };
    let got = find(&q, &cfg.source_list()).await;
    cache_put(&st.db.lock(), &key, got.as_ref(), db::now()).map_err(|e| e.to_string())?;
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: f64, s: &str) -> LyricLine {
        LyricLine {
            t,
            text: s.into(),
            ..Default::default()
        }
    }

    fn found(title: &str, artist: &str, dur: f64, lines: Vec<LyricLine>) -> Found {
        Found {
            source: Source::Lrclib,
            title: title.into(),
            artist: artist.into(),
            duration: Some(dur),
            lines,
        }
    }

    #[test]
    fn norm_strips_versions_and_width() {
        assert_eq!(norm("晴天 (Live)"), "晴天");
        assert_eq!(norm("Ｈｅｌｌｏ　Ｗｏｒｌｄ"), "helloworld");
        assert_eq!(norm("七里香【伴奏】"), "七里香");
        assert_eq!(norm("(无题)"), "无题");
    }

    #[test]
    fn query_splits_filename() {
        let q = Query::new("周杰伦 - 晴天", "", "", 269.0);
        assert_eq!((q.artist.as_str(), q.title.as_str()), ("周杰伦", "晴天"));
        let q = Query::new("A - B", "Someone", "", 0.0);
        assert_eq!(q.title, "A - B");
    }

    #[test]
    fn score_rules() {
        let q = Query::new("晴天", "周杰伦", "叶惠美", 269.0);
        assert!(score(&q, "晴天", "周杰伦", Some(270.0)).unwrap() > 0.9);
        assert!(score(&q, "晴天", "Jay Chou/周杰伦", Some(269.0)).is_some());
        // 时长差太多：多半是 Live 或别的版本
        assert!(score(&q, "晴天", "周杰伦", Some(300.0)).is_none());
        // 标题不像
        assert!(score(&q, "稻香", "周杰伦", Some(269.0)).is_none());
    }

    #[test]
    fn pick_prefers_better_sync_then_score() {
        let q = Query::new("晴天", "周杰伦", "", 269.0);
        let plain = found("晴天", "周杰伦", 269.0, vec![line(-1.0, "a")]);
        let synced = found("晴天", "某翻唱", 272.0, vec![line(1.0, "a")]);
        let wrong = found("稻香", "周杰伦", 269.0, vec![line(1.0, "a")]);
        let empty = found("晴天", "周杰伦", 269.0, vec![]);
        let got = pick(&q, vec![plain, wrong, empty, synced], &[Source::Lrclib]).unwrap();
        assert_eq!(got.artist, "某翻唱");
        assert!(pick(&q, vec![], &[Source::Lrclib]).is_none());
    }

    #[test]
    fn cache_roundtrip_and_miss_ttl() {
        let c = crate::db::open_memory().unwrap();
        init(&c).unwrap();
        assert!(cache_get(&c, "k", 100).unwrap().is_none());
        let v = OnlineLyrics {
            source: Source::Lrclib,
            lines: vec![line(1.0, "你好")],
        };
        cache_put(&c, "k", Some(&v), 100).unwrap();
        assert_eq!(cache_get(&c, "k", 100).unwrap(), Some(Some(v)));
        cache_put(&c, "miss", None, 100).unwrap();
        assert_eq!(cache_get(&c, "miss", 100 + 60).unwrap(), Some(None));
        assert_eq!(cache_get(&c, "miss", 100 + MISS_TTL + 1).unwrap(), None);
    }

    #[test]
    fn cfg_defaults() {
        let c = crate::db::open_memory().unwrap();
        let cfg = OnlineCfg::load(&c);
        assert!(cfg.enabled);
        assert_eq!(cfg.source_list(), vec![Source::Lrclib]);
        library::set_setting(&c, "lyrics_online", &serde_json::json!({"enabled": false})).unwrap();
        let cfg = OnlineCfg::load(&c);
        assert!(!cfg.enabled);
        assert_eq!(cfg.source_list(), vec![Source::Lrclib]);
    }
}
