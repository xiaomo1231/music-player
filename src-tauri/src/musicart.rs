//! 歌曲 / 专辑封面补全。
//!
//! 1. 本地：内嵌封面、文件夹里的封面图（metadata.rs）；同一张专辑里有一首有封面，整张专辑都用它。
//! 2. 联网（设置里打开“联网补全元数据”时）：按“艺人 + 专辑名”查网易云、iTunes、MusicBrainz（Cover Art Archive），
//!    没有专辑名的单曲按“歌名 + 艺人”查它所在专辑的封面。艺人和专辑都对得上才自动换上，拿不准的留给“更换封面”确认。
//!    只把艺人、专辑名、歌名发给外部服务，不发文件路径。
//!
//! 补上的封面同时写进 tracks.cover（所有界面直接能用）和 tracks.net_cover（文件改动后重新读标签时不会被冲掉）。
//! 每张专辑一条 music_art：state = auto 自动匹配 / pending 拿不准 / none 没搜到 / locked 手动选的。
use crate::artmeta::{norm, similarity};
use crate::{db, library, metadata, AppState};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager, State};

/// 本地查找规则的版本：规则改进后，还没有封面的歌重新在本地找一次
const LOCAL_RULES: i64 = 1;
/// 联网匹配规则的版本：改进后自动匹配过的（不含手动选的）重新查一次
const NET_RULES: i64 = 1;
/// 没搜到 / 拿不准的专辑，隔多久再自动查一次（秒）
const RETRY_AFTER: i64 = 14 * 24 * 3600;

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS music_art(
  key TEXT PRIMARY KEY,            -- 艺人|专辑（单曲是 song|艺人|歌名），都是 norm 过的
  artist TEXT,
  album TEXT,
  source TEXT,                     -- netease / itunes / musicbrainz / local
  remote_id TEXT,
  title TEXT,                      -- 匹配上的专辑名
  cover TEXT,                      -- 封面缓存文件名
  score REAL,
  state TEXT NOT NULL,             -- auto / pending / none / locked
  candidates TEXT,
  checked_at INTEGER NOT NULL,
  rules INTEGER NOT NULL DEFAULT 0
);
"#,
    )?;
    Ok(())
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// 设置里的“联网补全元数据”
fn online(conn: &Connection) -> bool {
    library::get_setting(conn, "metadata_online")
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

// ---------------------------------------------------------------- 名字处理

/// 一个艺人字段拆成几个人：“AGA/陈奕迅”“A & B”“A feat. B”
pub fn split_artists(s: &str) -> Vec<String> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\s*(?:/|;|；|、|，|,|&|＆|\bfeat\.?|\bft\.|\bwith\b|\bx\b|×)\s*").unwrap());
    let mut v: Vec<String> = RE.split(s).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    v.dedup();
    v
}

/// 专辑名里不影响是哪一张专辑的部分：(feat. …)、[Remastered]、(Deluxe Edition)、[24bit-192kHz] 之类
pub fn clean_album(s: &str) -> String {
    static BR: Lazy<Regex> = Lazy::new(|| Regex::new(r"[（(\[【〔]([^）)\]】〕]*)[）)\]】〕]").unwrap());
    static NOISE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)feat|ft\.|remaster|deluxe|edition|version|explicit|hi-?res|\d+\s*bit|khz|flac|wav|ape|dsd|sacd|mqa|cd\s*\d|disc\s*\d|disk\s*\d|bonus|版|web|single|ep$").unwrap()
    });
    static TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\s+-\s+(single|ep)$").unwrap());
    let s = BR.replace_all(s, |c: &regex::Captures| if NOISE.is_match(&c[1]) { String::new() } else { c[0].to_string() });
    let s = TAIL.replace(&s, "");
    let s = s.trim().to_string();
    if s.is_empty() {
        s
    } else {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// 名字里去掉艺人名（“MOOV Live 2009 陈奕迅”→“MOOV Live 2009”）
fn without(name: &str, artists: &[String]) -> String {
    let mut n = norm(name);
    for a in artists {
        let a = norm(a);
        if a.chars().count() >= 2 && n != a {
            n = n.replace(&a, "");
        }
    }
    n
}

/// 两个专辑名（或歌名）有多像：原名、去掉版本后缀、去掉艺人名，取最高
pub fn title_sim(local: &str, remote: &str, artists: &[String]) -> f64 {
    let (lc, rc) = (clean_album(local), clean_album(remote));
    let mut best = similarity(local, remote).max(similarity(&lc, &rc));
    let (lw, rw) = (without(&lc, artists), without(&rc, artists));
    if !lw.is_empty() && !rw.is_empty() {
        best = best.max(similarity(&lw, &rw));
    }
    best
}

/// 艺人有多像：本地的每个艺人和候选的每个名字（含别名）比，取最高
pub fn artist_sim(local: &[String], remote: &[String]) -> f64 {
    local.iter().flat_map(|a| remote.iter().map(move |b| similarity(a, b))).fold(0.0, f64::max)
}

/// 不是真正专辑名的“专辑”：空、等于艺人名、或者是常见的杂物文件夹名（没有专辑标签时用了文件夹名）
pub fn generic_album(album: &str, artists: &[String]) -> bool {
    const GENERIC: &[&str] = &[
        "music",
        "musics",
        "songs",
        "audio",
        "mp3",
        "flac",
        "download",
        "downloads",
        "desktop",
        "新建文件夹",
        "音乐",
        "歌曲",
        "下载",
        "未知专辑",
        "unknown",
        "unknownalbum",
        "single",
        "singles",
        "单曲",
        "其他",
        "misc",
        "various",
        "cloudmusic",
        "kgmusic",
        "qqmusic",
        "vipsongsdownload",
    ];
    let n = norm(album);
    n.is_empty() || GENERIC.contains(&n.as_str()) || artists.iter().any(|a| norm(a) == n)
}

// ---------------------------------------------------------------- 曲目和分组

#[derive(Clone, Debug, Default)]
pub struct TrackRow {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub cover: Option<String>,
    pub hue: Option<i64>,
    pub net_cover: Option<String>,
    pub custom_cover: bool,
    pub lock: bool,
    pub art_rules: i64,
}

impl TrackRow {
    /// 这首歌自己带的封面（内嵌或文件夹里的），不是补上的
    fn own_cover(&self) -> bool {
        self.cover.is_some() && self.cover != self.net_cover
    }
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub key: String,
    /// 显示和搜索用的艺人（专辑艺人优先）
    pub artist: String,
    pub artists: Vec<String>,
    pub album: String,
    /// 没有真正的专辑名：按歌名找
    pub by_song: bool,
    pub tracks: Vec<TrackRow>,
}

fn load_tracks(conn: &Connection) -> Result<Vec<TrackRow>> {
    let mut st = conn.prepare(
        "SELECT f.id,f.path,t.title,coalesce(t.artist,''),coalesce(t.album,''),coalesce(t.album_artist,''),t.cover,t.hue,t.net_cover,f.custom_cover IS NOT NULL,t.net_lock,t.art_rules
         FROM tracks t JOIN files f ON f.id=t.file_id WHERE f.missing=0 AND f.encrypted=0",
    )?;
    let rows = st
        .query_map([], |r| {
            Ok(TrackRow {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                artist: r.get(3)?,
                album: r.get(4)?,
                album_artist: r.get(5)?,
                cover: r.get(6)?,
                hue: r.get(7)?,
                net_cover: r.get(8)?,
                custom_cover: r.get(9)?,
                lock: r.get::<_, i64>(10)? != 0,
                art_rules: r.get(11)?,
            })
        })?
        .flatten()
        .collect();
    Ok(rows)
}

/// 这首歌属于哪一组：有专辑名时按“专辑艺人（没有就用第一个艺人）+ 专辑”，否则按单曲
pub fn group_key(t: &TrackRow) -> (String, String, Vec<String>, bool) {
    let mut artists = split_artists(&t.album_artist);
    for a in split_artists(&t.artist) {
        if !artists.contains(&a) {
            artists.push(a);
        }
    }
    let main = artists.first().cloned().unwrap_or_default();
    if generic_album(&t.album, &artists) {
        (format!("song|{}|{}", norm(&main), norm(&clean_album(&t.title))), main, artists, true)
    } else {
        (format!("{}|{}", norm(&main), norm(&clean_album(&t.album))), main, artists, false)
    }
}

pub fn groups(tracks: Vec<TrackRow>) -> Vec<Group> {
    let mut map: HashMap<String, Group> = HashMap::new();
    let mut order = vec![];
    for t in tracks {
        let (key, artist, artists, by_song) = group_key(&t);
        let g = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Group {
                key,
                artist,
                artists: vec![],
                album: if by_song { String::new() } else { t.album.clone() },
                by_song,
                tracks: vec![],
            }
        });
        for a in artists {
            if !g.artists.contains(&a) {
                g.artists.push(a);
            }
        }
        g.tracks.push(t);
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

// ---------------------------------------------------------------- 本地补全

/// 还没封面的歌：按新规则再在本地找一次（内嵌封面查所有标签、更多文件夹图片）。
/// 再把同一专辑里已有的封面共用给没有的。返回补上了多少首
pub fn local_pass(conn: &Connection, covers: &Path) -> Result<usize> {
    let tracks = load_tracks(conn)?;
    let mut fixed = 0;
    // 同一个目录只找一次文件夹图
    let mut dir_cache: HashMap<PathBuf, Option<(String, i64)>> = HashMap::new();
    for t in tracks.iter().filter(|t| t.cover.is_none() && t.art_rules < LOCAL_RULES) {
        let path = Path::new(&t.path);
        let mut found = lofty::read_from_path(path)
            .ok()
            .and_then(|tf| metadata::embedded_picture(&tf))
            .and_then(|b| metadata::save_cover(&b, covers));
        if found.is_none() {
            if let Some(img) = metadata::folder_cover(path) {
                let dir = img.clone();
                found = dir_cache
                    .entry(dir)
                    .or_insert_with(|| std::fs::read(&img).ok().and_then(|b| metadata::save_cover(&b, covers)))
                    .clone();
            }
        }
        if let Some((name, hue)) = found {
            conn.execute("UPDATE tracks SET cover=?1, hue=?2 WHERE file_id=?3", params![name, hue, t.id])?;
            fixed += 1;
        }
        conn.execute("UPDATE tracks SET art_rules=?1 WHERE file_id=?2", params![LOCAL_RULES, t.id])?;
    }
    fixed += share_pass(conn)?;
    Ok(fixed)
}

/// 同一张专辑里有的歌有封面、有的没有：没有的用这张专辑最常见的那张
pub fn share_pass(conn: &Connection) -> Result<usize> {
    let mut fixed = 0;
    for g in groups(load_tracks(conn)?) {
        if g.by_song || g.tracks.iter().all(|t| t.cover.is_some()) {
            continue;
        }
        let mut count: HashMap<&str, (usize, Option<i64>)> = HashMap::new();
        for t in g.tracks.iter().filter(|t| t.own_cover()) {
            let e = count.entry(t.cover.as_deref().unwrap_or_default()).or_insert((0, t.hue));
            e.0 += 1;
        }
        let Some((cover, (_, hue))) = count.into_iter().max_by_key(|(c, (n, _))| (*n, std::cmp::Reverse(*c))) else {
            continue;
        };
        for t in g.tracks.iter().filter(|t| t.cover.is_none()) {
            conn.execute(
                "UPDATE tracks SET cover=?1, hue=?2, net_cover=?1, net_hue=?2 WHERE file_id=?3",
                params![cover, hue, t.id],
            )?;
            fixed += 1;
        }
    }
    Ok(fixed)
}

// ---------------------------------------------------------------- 联网来源

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Cand {
    pub source: String,
    pub remote_id: String,
    /// 专辑名
    pub title: String,
    /// 显示用的艺人
    pub artist: String,
    /// 艺人的所有名字（含别名、译名），匹配用
    #[serde(default)]
    pub artists: Vec<String>,
    /// 从歌曲搜索来的：歌名
    #[serde(default)]
    pub song: String,
    pub year: Option<i64>,
    pub tracks: Option<i64>,
    /// 小图（列表显示）
    pub thumb: String,
    /// 大图（下载）
    pub image: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub title_score: f64,
    #[serde(default)]
    pub artist_score: f64,
}

fn http() -> reqwest::Client {
    static C: Lazy<reqwest::Client> = Lazy::new(|| {
        reqwest::Client::builder()
            .user_agent(concat!(
                "xiaomo1231/music-player/",
                env!("CARGO_PKG_VERSION"),
                " (https://github.com/xiaomo1231/music-player)"
            ))
            .timeout(Duration::from_secs(12))
            .build()
            .unwrap_or_default()
    });
    C.clone()
}

fn https(u: &str) -> String {
    u.strip_prefix("http://").map(|r| format!("https://{r}")).unwrap_or_else(|| u.to_string())
}

fn year_ms(v: &Value) -> Option<i64> {
    // 毫秒时间戳 → 年份
    let ms = v.as_i64().filter(|m| *m > 0)?;
    let days = ms / 86_400_000;
    Some(1970 + (days as f64 / 365.2425) as i64)
}

fn netease_artists(a: &Value) -> (String, Vec<String>) {
    let mut shown = vec![];
    let mut names = vec![];
    for x in a.as_array().into_iter().flatten() {
        if let Some(n) = x["name"].as_str() {
            shown.push(n.to_string());
            names.push(n.to_string());
        }
        for k in ["alias", "alia", "tns", "trans"] {
            match &x[k] {
                Value::Array(v) => names.extend(v.iter().filter_map(|s| s.as_str()).map(String::from)),
                Value::String(s) if !s.is_empty() => names.push(s.clone()),
                _ => {}
            }
        }
    }
    (shown.join(" / "), names)
}

fn netease_pic(u: &str) -> (String, String) {
    let u = https(u);
    (format!("{u}?param=300y300"), format!("{u}?param=1000y1000"))
}

pub fn parse_netease_albums(v: &Value) -> Vec<Cand> {
    v["result"]["albums"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let pic = a["picUrl"].as_str().filter(|s| !s.is_empty())?;
            let (shown, mut names) = netease_artists(&a["artists"]);
            let (_, more) = netease_artists(&Value::Array(vec![a["artist"].clone()]));
            names.extend(more);
            let (thumb, image) = netease_pic(pic);
            Some(Cand {
                source: "netease".into(),
                remote_id: a["id"].as_i64()?.to_string(),
                title: a["name"].as_str().unwrap_or("").to_string(),
                artist: shown,
                artists: names,
                song: String::new(),
                year: year_ms(&a["publishTime"]),
                tracks: a["size"].as_i64(),
                thumb,
                image,
                ..Default::default()
            })
        })
        .collect()
}

pub fn parse_netease_songs(v: &Value) -> Vec<Cand> {
    v["result"]["songs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let al = &s["al"];
            let pic = al["picUrl"].as_str().filter(|s| !s.is_empty())?;
            let (shown, names) = netease_artists(&s["ar"]);
            let (thumb, image) = netease_pic(pic);
            Some(Cand {
                source: "netease".into(),
                remote_id: al["id"].as_i64()?.to_string(),
                title: al["name"].as_str().unwrap_or("").to_string(),
                artist: shown,
                artists: names,
                song: s["name"].as_str().unwrap_or("").to_string(),
                year: year_ms(&s["publishTime"]),
                tracks: None,
                thumb,
                image,
                ..Default::default()
            })
        })
        .collect()
}

async fn netease(q: &str, song: bool) -> Result<Vec<Cand>> {
    let v: Value = http()
        .get("https://music.163.com/api/cloudsearch/pc")
        .query(&[("s", q), ("type", if song { "1" } else { "10" }), ("limit", "10"), ("offset", "0")])
        .header("referer", "https://music.163.com/")
        .header("cookie", "os=pc")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if v["code"].as_i64() != Some(200) {
        return Err(anyhow!("网易云返回 {}", v["code"]));
    }
    Ok(if song { parse_netease_songs(&v) } else { parse_netease_albums(&v) })
}

pub fn parse_itunes(v: &Value) -> Vec<Cand> {
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let art = r["artworkUrl100"].as_str().filter(|s| !s.is_empty())?;
            let artist = r["artistName"].as_str().unwrap_or("").to_string();
            let mut names = split_artists(&artist);
            names.push(artist.clone());
            Some(Cand {
                source: "itunes".into(),
                remote_id: r["collectionId"].as_i64()?.to_string(),
                title: r["collectionName"].as_str().unwrap_or("").to_string(),
                artist,
                artists: names,
                song: r["trackName"].as_str().unwrap_or("").to_string(),
                year: r["releaseDate"].as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()),
                tracks: r["trackCount"].as_i64(),
                thumb: art.replace("100x100bb", "300x300bb"),
                image: art.replace("100x100bb", "1000x1000bb"),
                ..Default::default()
            })
        })
        .collect()
}

/// iTunes 搜索 API 每分钟大约 20 次，两次之间至少隔 3 秒
static ITUNES_LAST: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));
/// MusicBrainz 要求每秒最多 1 次
static MB_LAST: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

async fn throttle(last: &Mutex<Option<Instant>>, gap: Duration) {
    let wait = {
        let mut l = last.lock();
        let now = Instant::now();
        let next = l.map_or(now, |t| (t + gap).max(now));
        *l = Some(next);
        next - now
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

async fn itunes(q: &str, song: bool, country: &str) -> Result<Vec<Cand>> {
    throttle(&ITUNES_LAST, Duration::from_secs(3)).await;
    let v: Value = http()
        .get("https://itunes.apple.com/search")
        .query(&[
            ("term", q),
            ("media", "music"),
            ("entity", if song { "song" } else { "album" }),
            ("country", country),
            ("limit", "10"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(parse_itunes(&v))
}

pub fn parse_musicbrainz(v: &Value) -> Vec<Cand> {
    v["release-groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let id = g["id"].as_str()?.to_string();
            let mut shown = vec![];
            let mut names = vec![];
            for c in g["artist-credit"].as_array().into_iter().flatten() {
                if let Some(n) = c["name"].as_str() {
                    shown.push(n.to_string());
                    names.push(n.to_string());
                }
                if let Some(n) = c["artist"]["name"].as_str() {
                    names.push(n.to_string());
                }
                for a in c["artist"]["aliases"].as_array().into_iter().flatten() {
                    if let Some(n) = a["name"].as_str() {
                        names.push(n.to_string());
                    }
                }
            }
            Some(Cand {
                source: "musicbrainz".into(),
                thumb: format!("https://coverartarchive.org/release-group/{id}/front-250"),
                image: format!("https://coverartarchive.org/release-group/{id}/front-500"),
                remote_id: id,
                title: g["title"].as_str().unwrap_or("").to_string(),
                artist: shown.join(" / "),
                artists: names,
                song: String::new(),
                year: g["first-release-date"].as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()),
                tracks: None,
                ..Default::default()
            })
        })
        .collect()
}

fn lucene(s: &str) -> String {
    s.replace(['\\', '"'], " ")
}

async fn musicbrainz(artist: &str, album: &str, free: &str) -> Result<Vec<Cand>> {
    throttle(&MB_LAST, Duration::from_millis(1100)).await;
    let q = if album.is_empty() {
        lucene(free)
    } else if artist.is_empty() {
        format!("releasegroup:\"{}\"", lucene(album))
    } else {
        format!("releasegroup:\"{}\" AND artist:\"{}\"", lucene(album), lucene(artist))
    };
    let v: Value = http()
        .get("https://musicbrainz.org/ws/2/release-group/")
        .query(&[("query", q.as_str()), ("fmt", "json"), ("limit", "10")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(parse_musicbrainz(&v))
}

// ---------------------------------------------------------------- 打分

/// 要找的东西：专辑（或单曲的歌名）和艺人
#[derive(Clone, Debug, Default)]
pub struct Want {
    pub album: String,
    pub song: String,
    pub artists: Vec<String>,
    pub by_song: bool,
}

impl Want {
    pub fn of(g: &Group) -> Self {
        Want {
            album: g.album.clone(),
            song: g.tracks.first().map(|t| t.title.clone()).unwrap_or_default(),
            artists: g.artists.clone(),
            by_song: g.by_song,
        }
    }
}

pub fn score(c: &mut Cand, w: &Want) {
    let all: Vec<String> = w.artists.iter().chain(c.artists.iter()).cloned().collect();
    c.artist_score = artist_sim(&w.artists, &c.artists);
    c.title_score = if w.by_song || w.album.is_empty() {
        if c.song.is_empty() {
            // 单曲按专辑搜到的：单曲专辑名通常就是歌名
            title_sim(&w.song, &c.title, &all) * 0.9
        } else {
            title_sim(&w.song, &c.song, &all)
        }
    } else {
        let t = title_sim(&w.album, &c.title, &all);
        if c.song.is_empty() {
            t
        } else {
            // 按歌搜到的专辑：专辑名也得像
            t.min(title_sim(&w.song, &c.song, &all).max(0.0))
        }
    };
    c.score = c.title_score * 0.65 + c.artist_score * 0.35;
}

/// 打分排序；返回值第二项表示最好的那个够不够自动采用
pub fn rank(mut cands: Vec<Cand>, w: &Want) -> (Vec<Cand>, bool) {
    for c in &mut cands {
        score(c, w);
    }
    // 分数相同时保持来源顺序（网易云 > iTunes > MusicBrainz）
    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen = HashSet::new();
    cands.retain(|c| seen.insert((c.source.clone(), c.remote_id.clone())));
    let auto = cands.first().is_some_and(sure);
    (cands, auto)
}

fn sure(c: &Cand) -> bool {
    (c.title_score >= 0.85 && c.artist_score >= 0.6) || (c.title_score >= 0.97 && c.artist_score >= 0.45)
}

// ---------------------------------------------------------------- 后台补全

static RUNNING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
static KICKED: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone, Default, Debug)]
pub struct Progress {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub found: usize,
    /// 这一轮是用户点“重新获取”触发的（前端据此提示结果）
    pub manual: bool,
}
static PROGRESS: Lazy<Mutex<Progress>> = Lazy::new(|| Mutex::new(Progress::default()));

fn emit_progress(app: &AppHandle) {
    let p = PROGRESS.lock().clone();
    let _ = app.emit("musicart://progress", p);
}

/// 启动后、媒体库有变化（扫描完、实时监控加了新歌）后自动补
pub fn setup(app: &AppHandle) {
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(25)).await;
        let _ = run(&a, None, false).await;
    });
    let a = app.clone();
    app.listen("library://changed", move |_| kick(&a));
}

/// 合并短时间内的多次变化：8 秒后跑一轮；正在跑时等这一轮结束再跑
pub fn kick(app: &AppHandle) {
    if RUNNING.load(Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    if KICKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        KICKED.store(false, Ordering::SeqCst);
        let _ = run(&a, None, false).await;
    });
}

#[derive(Clone, Debug)]
struct Saved {
    state: String,
    checked_at: i64,
    rules: i64,
}

fn saved(conn: &Connection, key: &str) -> Result<Option<Saved>> {
    Ok(conn
        .query_row("SELECT state,checked_at,rules FROM music_art WHERE key=?1", [key], |r| {
            Ok(Saved {
                state: r.get(0)?,
                checked_at: r.get(1)?,
                rules: r.get(2)?,
            })
        })
        .optional()?)
}

/// 这一组要不要联网找。only：只处理这些组（用户点了重新获取），force：没搜到 / 自动配的也重新找
fn due(conn: &Connection, g: &Group, only: Option<&HashSet<String>>, force: bool) -> Result<bool> {
    if let Some(o) = only {
        if !o.contains(&g.key) {
            return Ok(false);
        }
    }
    // 有歌自带封面（已经共用给整张专辑了），或者手动换过的，不联网
    if g.tracks.iter().any(|t| t.own_cover() || t.lock || t.custom_cover) {
        return Ok(false);
    }
    let has_net = g.tracks.iter().all(|t| t.cover.is_some());
    let s = saved(conn, &g.key)?;
    Ok(match s {
        None => !has_net,
        Some(s) if s.state == "locked" => false,
        Some(_) if force => true,
        Some(s) if s.state == "auto" => s.rules < NET_RULES,
        Some(s) => !has_net && (s.rules < NET_RULES || db::now() - s.checked_at > RETRY_AFTER),
    })
}

/// 联网找一组的候选：先网易云，拿不准再 iTunes（中国区、港区），再 MusicBrainz；专辑都没配上时按歌名找。
/// 所有来源都连不上时返回错误（这次不记，下次再试）
async fn find(g: &Group) -> Result<(Vec<Cand>, bool)> {
    let w = Want::of(g);
    let artist = g.artist.clone();
    let album = clean_album(&g.album);
    let song = clean_album(&w.song);
    let mut all: Vec<Cand> = vec![];
    let mut ok = false;
    let mut last_err = None;
    let mut add = |r: Result<Vec<Cand>>, all: &mut Vec<Cand>| match r {
        Ok(v) => {
            ok = true;
            all.extend(v);
        }
        Err(e) => last_err = Some(e),
    };
    if !g.by_song {
        let q = format!("{artist} {album}").trim().to_string();
        add(netease(&q, false).await, &mut all);
        if !rank(all.clone(), &w).1 {
            add(itunes(&q, false, "cn").await, &mut all);
        }
        if !rank(all.clone(), &w).1 {
            add(itunes(&q, false, "hk").await, &mut all);
        }
        if !rank(all.clone(), &w).1 {
            add(musicbrainz(&artist, &album, "").await, &mut all);
        }
    }
    if !rank(all.clone(), &w).1 && !song.is_empty() {
        let q = format!("{song} {artist}").trim().to_string();
        add(netease(&q, true).await, &mut all);
        if !rank(all.clone(), &w).1 {
            add(itunes(&q, true, "cn").await, &mut all);
        }
    }
    if !ok {
        return Err(last_err.unwrap_or_else(|| anyhow!("连不上")));
    }
    let (mut list, auto) = rank(all, &w);
    list.truncate(24);
    Ok((list, auto))
}

/// 下载封面存进封面缓存，返回（文件名，主色色相）
async fn download(c: &Cand, covers: &Path) -> Result<(String, i64)> {
    let bytes = http().get(&c.image).send().await?.error_for_status()?.bytes().await?;
    let dir = covers.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || metadata::save_cover(&bytes, &dir))
        .await?
        .ok_or_else(|| anyhow!("图片无法识别"))
}

fn save_row(conn: &Connection, g: &Group, cands: &[Cand], state: &str, cover: Option<&str>) -> Result<()> {
    let best = cands.first().filter(|_| state == "auto" || state == "locked");
    conn.execute(
        "INSERT OR REPLACE INTO music_art(key,artist,album,source,remote_id,title,cover,score,state,candidates,checked_at,rules)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            g.key,
            g.artist,
            if g.by_song {
                g.tracks.first().map(|t| t.title.clone()).unwrap_or_default()
            } else {
                g.album.clone()
            },
            best.map(|c| c.source.clone()),
            best.map(|c| c.remote_id.clone()),
            best.map(|c| c.title.clone()),
            cover,
            cands.first().map(|c| c.score),
            state,
            serde_json::to_string(cands)?,
            db::now(),
            NET_RULES,
        ],
    )?;
    Ok(())
}

/// 把封面装到一组的歌上。lock：用户手动选的，连自带的封面也盖过
pub fn apply(conn: &Connection, g: &Group, cover: &str, hue: i64, lock: bool) -> Result<()> {
    for t in &g.tracks {
        if lock {
            conn.execute(
                "UPDATE tracks SET cover=?1, hue=?2, net_cover=?1, net_hue=?2, net_lock=1 WHERE file_id=?3",
                params![cover, hue, t.id],
            )?;
        } else if !t.own_cover() && !t.lock {
            conn.execute(
                "UPDATE tracks SET cover=?1, hue=?2, net_cover=?1, net_hue=?2 WHERE file_id=?3",
                params![cover, hue, t.id],
            )?;
        }
    }
    Ok(())
}

/// 跑一轮：本地补全（总是做），再联网（打开了“联网补全元数据”时）。返回（联网查了几组，补上了几组）
pub async fn run(app: &AppHandle, only: Option<HashSet<String>>, force: bool) -> Result<(usize, usize)> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return Ok((0, 0));
    }
    let manual = only.is_some() || force;
    let r = run_inner(app, only, force, manual).await;
    {
        let mut p = PROGRESS.lock();
        p.running = false;
    }
    emit_progress(app);
    RUNNING.store(false, Ordering::SeqCst);
    if AGAIN.swap(false, Ordering::SeqCst) {
        kick(app);
    }
    r
}

async fn run_inner(app: &AppHandle, only: Option<HashSet<String>>, force: bool, manual: bool) -> Result<(usize, usize)> {
    let (db_path, covers) = {
        let st = app.state::<AppState>();
        (st.db_path.clone(), st.dirs.covers.clone())
    };
    // 本地：读文件、找图片比较慢，用单独的连接在后台线程做，不占着界面用的连接
    let c2 = covers.clone();
    let local = tauri::async_runtime::spawn_blocking(move || -> Result<usize> {
        let conn = db::open(&db_path)?;
        local_pass(&conn, &c2)
    })
    .await??;
    if local > 0 {
        let _ = app.emit("library://changed", ());
    }
    let (on, list) = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let gs = groups(load_tracks(&conn)?);
        let mut list = vec![];
        for g in gs {
            if due(&conn, &g, only.as_ref(), force)? {
                list.push(g);
            }
        }
        (online(&conn), list)
    };
    if !on || list.is_empty() {
        return Ok((0, 0));
    }
    {
        let mut p = PROGRESS.lock();
        *p = Progress {
            running: true,
            done: 0,
            total: list.len(),
            found: 0,
            manual,
        };
    }
    emit_progress(app);
    let mut found = 0;
    let mut fails = 0;
    for (i, g) in list.iter().enumerate() {
        match find(g).await {
            Ok((cands, auto)) => {
                fails = 0;
                let mut state = if auto {
                    "auto"
                } else if cands.first().is_some_and(|c| c.title_score >= 0.5) {
                    "pending"
                } else {
                    "none"
                };
                let mut got = None;
                if auto {
                    match download(&cands[0], &covers).await {
                        Ok(c) => got = Some(c),
                        Err(_) => state = "pending",
                    }
                }
                let st = app.state::<AppState>();
                let conn = st.db.lock();
                save_row(&conn, g, &cands, state, got.as_ref().map(|c| c.0.as_str()))?;
                if let Some((cover, hue)) = &got {
                    apply(&conn, g, cover, *hue, false)?;
                    found += 1;
                }
            }
            Err(_) => {
                // 网络不通：连续失败几次就停下，下次再试
                fails += 1;
                if fails >= 5 {
                    break;
                }
            }
        }
        {
            let mut p = PROGRESS.lock();
            p.done = i + 1;
            p.found = found;
        }
        emit_progress(app);
        if found > 0 && (i + 1) % 15 == 0 {
            let _ = app.emit("library://changed", ());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if found > 0 {
        let _ = app.emit("library://changed", ());
    }
    Ok((list.len(), found))
}

// ---------------------------------------------------------------- 命令

fn group_of(conn: &Connection, track: i64) -> Result<Group> {
    let tracks = load_tracks(conn)?;
    let me = tracks.iter().find(|t| t.id == track).ok_or_else(|| anyhow!("找不到这首歌"))?;
    let key = group_key(me).0;
    groups(tracks).into_iter().find(|g| g.key == key).ok_or_else(|| anyhow!("找不到这首歌"))
}

#[derive(Serialize, Default)]
pub struct Picker {
    pub key: String,
    pub artist: String,
    pub album: String,
    pub song: String,
    pub by_song: bool,
    /// 这一组有几首
    pub count: usize,
    pub state: Option<String>,
    pub current: Option<String>,
    pub candidates: Vec<Cand>,
    pub query: String,
    pub online: bool,
}

#[tauri::command]
pub fn music_art_picker(st: State<AppState>, track: i64) -> Result<Picker, String> {
    let conn = st.db.lock();
    let g = group_of(&conn, track).map_err(err)?;
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row("SELECT state,source||':'||remote_id,candidates FROM music_art WHERE key=?1", [&g.key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .map_err(err)?;
    let me = g.tracks.iter().find(|t| t.id == track).cloned().unwrap_or_default();
    let song = clean_album(&me.title);
    let query = if g.by_song {
        format!("{song} {}", g.artist)
    } else {
        format!("{} {}", g.artist, clean_album(&g.album))
    };
    Ok(Picker {
        key: g.key.clone(),
        artist: g.artist.clone(),
        album: g.album.clone(),
        song: me.title.clone(),
        by_song: g.by_song,
        count: g.tracks.len(),
        state: row.as_ref().map(|r| r.0.clone()),
        current: row.as_ref().and_then(|r| r.1.clone()),
        candidates: row.and_then(|r| r.2).and_then(|c| serde_json::from_str(&c).ok()).unwrap_or_default(),
        query: query.trim().to_string(),
        online: online(&conn),
    })
}

/// 手动搜索：source = netease / itunes / musicbrainz；song = true 按歌曲搜
#[tauri::command]
pub async fn music_art_search(app: AppHandle, track: i64, query: String, source: String, song: bool) -> Result<Vec<Cand>, String> {
    let g = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        group_of(&conn, track).map_err(err)?
    };
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let list = match source.as_str() {
        "itunes" => {
            let mut v = itunes(q, song, "cn").await.map_err(err)?;
            if v.is_empty() {
                v = itunes(q, song, "hk").await.map_err(err)?;
            }
            v
        }
        "musicbrainz" => musicbrainz("", "", q).await.map_err(err)?,
        _ => netease(q, song).await.map_err(err)?,
    };
    let mut w = Want::of(&g);
    if let Some(t) = g.tracks.iter().find(|t| t.id == track) {
        w.song = t.title.clone();
    }
    Ok(rank(list, &w).0)
}

/// 选中一个候选：下载，装到整张专辑（单曲就是这一首），锁定
#[tauri::command]
pub async fn music_art_choose(app: AppHandle, track: i64, candidate: Cand) -> Result<(), String> {
    let (g, covers) = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        (group_of(&conn, track).map_err(err)?, st.dirs.covers.clone())
    };
    let (cover, hue) = download(&candidate, &covers).await.map_err(|e| format!("下载封面失败：{e}"))?;
    {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let mut cands: Vec<Cand> = conn
            .query_row("SELECT candidates FROM music_art WHERE key=?1", [&g.key], |r| r.get::<_, Option<String>>(0))
            .optional()
            .map_err(err)?
            .flatten()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();
        cands.retain(|c| !(c.source == candidate.source && c.remote_id == candidate.remote_id));
        cands.insert(0, candidate);
        save_row(&conn, &g, &cands, "locked", Some(&cover)).map_err(err)?;
        apply(&conn, &g, &cover, hue, true).map_err(err)?;
    }
    let _ = app.emit("library://changed", ());
    Ok(())
}

/// 用本地图片当整张专辑的封面
#[tauri::command]
pub fn music_art_local(app: AppHandle, st: State<AppState>, track: i64, image: String) -> Result<(), String> {
    let bytes = std::fs::read(&image).map_err(|e| format!("读取图片失败：{e}"))?;
    let (cover, hue) = metadata::save_cover(&bytes, &st.dirs.covers).ok_or("这个文件不是能识别的图片")?;
    let conn = st.db.lock();
    let g = group_of(&conn, track).map_err(err)?;
    let c = Cand {
        source: "local".into(),
        remote_id: image,
        title: g.album.clone(),
        ..Default::default()
    };
    save_row(&conn, &g, &[c], "locked", Some(&cover)).map_err(err)?;
    apply(&conn, &g, &cover, hue, true).map_err(err)?;
    let _ = app.emit("library://changed", ());
    Ok(())
}

/// 清掉一组补上的封面（含手动选的），重新从本地、联网找
fn reset_group(conn: &Connection, g: &Group) -> Result<()> {
    for t in &g.tracks {
        if t.lock || (t.cover.is_some() && t.cover == t.net_cover) {
            conn.execute(
                "UPDATE tracks SET cover=NULL, hue=NULL, net_cover=NULL, net_hue=NULL, net_lock=0, art_rules=0 WHERE file_id=?1",
                [t.id],
            )?;
        }
    }
    conn.execute("DELETE FROM music_art WHERE key=?1", [&g.key])?;
    Ok(())
}

/// 恢复自动：去掉手动选的封面，马上重新找这一组
#[tauri::command]
pub async fn music_art_reset(app: AppHandle, track: i64) -> Result<(usize, usize), String> {
    let key = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let g = group_of(&conn, track).map_err(err)?;
        reset_group(&conn, &g).map_err(err)?;
        g.key
    };
    let _ = app.emit("library://changed", ());
    run(&app, Some(HashSet::from([key])), true).await.map_err(err)
}

/// 批量重新获取。tracks 为空：所有还没封面的专辑（没搜到、拿不准的也再查一次）；
/// 否则只处理这些歌所在的专辑，自动配上的联网封面也重新找（手动选的不动）
#[tauri::command]
pub async fn music_art_refetch(app: AppHandle, tracks: Vec<i64>) -> Result<(usize, usize), String> {
    let only = if tracks.is_empty() {
        None
    } else {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let all = load_tracks(&conn).map_err(err)?;
        let want: HashSet<i64> = tracks.into_iter().collect();
        let keys: HashSet<String> = all.iter().filter(|t| want.contains(&t.id)).map(|t| group_key(t).0).collect();
        Some(keys)
    };
    if !online(&app.state::<AppState>().db.lock()) {
        return Err("先在设置里打开“联网补全元数据”".into());
    }
    if RUNNING.load(Ordering::SeqCst) {
        return Err("正在补全封面，等这一轮结束再试".into());
    }
    let n = run(&app, only, true).await.map_err(err)?;
    Ok(n)
}

/// 打开“联网补全元数据”后马上在后台补一轮
#[tauri::command]
pub fn music_art_auto(app: AppHandle) {
    kick(&app);
}

#[tauri::command]
pub fn music_art_status() -> Progress {
    PROGRESS.lock().clone()
}

/// 设置页显示：多少首歌还没有封面
#[tauri::command]
pub fn music_art_missing(st: State<AppState>) -> Result<i64, String> {
    st.db
        .lock()
        .query_row(
            "SELECT count(*) FROM tracks t JOIN files f ON f.id=t.file_id WHERE f.missing=0 AND f.encrypted=0 AND t.cover IS NULL AND f.custom_cover IS NULL",
            [],
            |r| r.get(0),
        )
        .map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cleans_album_names() {
        assert_eq!(clean_album("Special One (feat. Eason Chan)"), "Special One");
        assert_eq!(clean_album("CHIN UP! [24bit-192kHz]"), "CHIN UP!");
        assert_eq!(clean_album("U87 (Deluxe Edition)"), "U87");
        assert_eq!(clean_album("Live (Remastered 2019)"), "Live");
        assert_eq!(clean_album("Something - Single"), "Something");
        // 不是版本信息的括号保留
        assert_eq!(clean_album("黑白灰 (国)"), "黑白灰 (国)");
    }

    #[test]
    fn splits_artists() {
        assert_eq!(split_artists("AGA/陈奕迅"), vec!["AGA", "陈奕迅"]);
        assert_eq!(split_artists("AGA feat. Eason Chan"), vec!["AGA", "Eason Chan"]);
        assert_eq!(split_artists("A & B"), vec!["A", "B"]);
        assert_eq!(split_artists(""), Vec::<String>::new());
    }

    #[test]
    fn title_match_ignores_artist_and_suffix() {
        let a = vec!["陈奕迅".to_string(), "Eason Chan".to_string()];
        assert!(title_sim("MOOV Live 2009", "MOOV Live 2009 陈奕迅", &a) > 0.95);
        assert!(title_sim("Special One (feat. Eason Chan)", "Special One", &a) > 0.95);
        assert!(title_sim("CHIN UP!", "The Line-Up", &a) < 0.6);
    }

    fn netease_albums() -> Value {
        json!({"code":200,"result":{"albums":[
            {"name":"The Line-Up","id":6559,"size":10,"publishTime":1027440000000_i64,
             "picUrl":"http://p2.music.126.net/x/1.jpg","artist":{"name":"陈奕迅","alias":["Eason Chan"]},
             "artists":[{"name":"陈奕迅","alias":[]}]},
            {"name":"CHIN UP!","id":176456056,"size":8,"publishTime":1698336000000_i64,
             "picUrl":"https://p2.music.126.net/y/2.jpg","artist":{"name":"陈奕迅","alias":["Eason Chan"],"trans":""},
             "artists":[{"name":"陈奕迅","alias":[]}]}
        ]}})
    }

    fn group(artist: &str, album: &str, titles: &[&str]) -> Group {
        let tracks: Vec<TrackRow> = titles
            .iter()
            .enumerate()
            .map(|(i, t)| TrackRow {
                id: i as i64 + 1,
                title: t.to_string(),
                artist: artist.into(),
                album: album.into(),
                ..Default::default()
            })
            .collect();
        groups(tracks).remove(0)
    }

    #[test]
    fn netease_album_auto_match() {
        let c = parse_netease_albums(&netease_albums());
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].year, Some(2023));
        assert!(c[1].image.starts_with("https://") && c[1].image.ends_with("?param=1000y1000"));
        assert!(c[0].artists.contains(&"Eason Chan".to_string()));
        let g = group("陈奕迅", "CHIN UP!", &["尘大师 Lightly", "空城记 Something Missing"]);
        let (r, auto) = rank(c.clone(), &Want::of(&g));
        assert!(auto);
        assert_eq!(r[0].title, "CHIN UP!");
        // 标签里写的是英文名也能对上（网易云给了别名）
        let g = group("Eason Chan", "CHIN UP!", &["Lightly"]);
        assert!(rank(c.clone(), &Want::of(&g)).1);
        // 艺人对不上：不自动
        let g = group("周杰伦", "CHIN UP!", &["x"]);
        assert!(!rank(c, &Want::of(&g)).1);
    }

    #[test]
    fn single_by_song() {
        let v = json!({"code":200,"result":{"songs":[{"name":"Special One","id":1,"publishTime":1694102400000_i64,
            "al":{"id":174239027,"name":"Special One (feat. Eason Chan)","picUrl":"http://p1.music.126.net/z/3.jpg"},
            "ar":[{"name":"AGA","tns":["江海迦"],"alias":["Agatha Kong"]},{"name":"陈奕迅","alias":["Eason Chan"]}]}]}});
        let c = parse_netease_songs(&v);
        assert_eq!(c[0].song, "Special One");
        assert_eq!(c[0].remote_id, "174239027");
        // 专辑名等于歌名的单曲：按专辑分组，专辑名去掉 feat 后能对上
        let g = group("AGA", "Special One (feat. Eason Chan)", &["Special One (feat. Eason Chan)"]);
        assert!(!g.by_song);
        assert!(rank(c.clone(), &Want::of(&g)).1);
        // 没有专辑名（专辑就是艺人名或杂物文件夹）：按歌名找
        let g = group("AGA", "Music", &["Special One (feat. Eason Chan)"]);
        assert!(g.by_song);
        assert!(rank(c, &Want::of(&g)).1);
    }

    #[test]
    fn parses_itunes_and_musicbrainz() {
        let v = json!({"results":[{"collectionId":1,"collectionName":"MOOV Live 2009","artistName":"陳奕迅",
            "artworkUrl100":"https://is1.mzstatic.com/a/100x100bb.jpg","releaseDate":"2009-03-20T07:00:00Z","trackCount":20}]});
        let c = parse_itunes(&v);
        assert_eq!(c[0].image, "https://is1.mzstatic.com/a/1000x1000bb.jpg");
        assert_eq!(c[0].year, Some(2009));
        let v = json!({"release-groups":[{"id":"abc","title":"MOOV Live 2009","first-release-date":"2009",
            "artist-credit":[{"name":"陳奕迅","artist":{"name":"陳奕迅","aliases":[{"name":"Eason Chan"}]}}]}]});
        let c = parse_musicbrainz(&v);
        assert_eq!(c[0].image, "https://coverartarchive.org/release-group/abc/front-500");
        let g = group("Eason Chan", "MOOV Live 2009", &["于心有愧"]);
        assert!(rank(c, &Want::of(&g)).1);
    }

    fn seed(conn: &Connection, path: &str, album: &str, cover: Option<&str>) -> i64 {
        conn.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,'music',1,1,1)", [path])
            .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tracks(file_id,title,artist,album,duration,format,cover,hue,art_rules) VALUES(?1,'t','陈奕迅',?2,1,'FLAC',?3,10,?4)",
            params![id, album, cover, LOCAL_RULES],
        )
        .unwrap();
        id
    }

    fn cover_of(conn: &Connection, id: i64) -> Option<String> {
        conn.query_row("SELECT cover FROM tracks WHERE file_id=?1", [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn shares_cover_within_album_and_survives_rescan() {
        let conn = db::open_memory().unwrap();
        init(&conn).unwrap();
        let a = seed(&conn, "C:\\m\\a.flac", "CHIN UP!", Some("x.jpg"));
        let b = seed(&conn, "C:\\m\\b.flac", "CHIN UP!", None);
        let c = seed(&conn, "C:\\m\\c.flac", "MOOV Live 2009", None);
        assert_eq!(share_pass(&conn).unwrap(), 1);
        assert_eq!(cover_of(&conn, b), Some("x.jpg".into()));
        assert_eq!(cover_of(&conn, c), None);
        // 重新读标签（文件改了）时，没有自带封面就保留补上的
        conn.execute(
            "INSERT INTO tracks(file_id,title,duration,format,cover,hue) VALUES(?1,'t',1,'FLAC',NULL,NULL)
             ON CONFLICT(file_id) DO UPDATE SET cover=CASE WHEN tracks.net_lock=1 THEN tracks.net_cover ELSE coalesce(excluded.cover,tracks.net_cover) END",
            [b],
        )
        .unwrap();
        assert_eq!(cover_of(&conn, b), Some("x.jpg".into()));
        // 手动给整张专辑换封面，再恢复自动
        let g = group_of(&conn, a).unwrap();
        assert_eq!(g.tracks.len(), 2);
        apply(&conn, &g, "y.jpg", 20, true).unwrap();
        assert_eq!(cover_of(&conn, a), Some("y.jpg".into()));
        reset_group(&conn, &group_of(&conn, a).unwrap()).unwrap();
        assert_eq!(cover_of(&conn, a), None);
    }

    #[test]
    fn due_rules() {
        let conn = db::open_memory().unwrap();
        init(&conn).unwrap();
        let a = seed(&conn, "C:\\m\\a.flac", "MOOV Live 2009", None);
        let g = group_of(&conn, a).unwrap();
        assert!(due(&conn, &g, None, false).unwrap());
        save_row(&conn, &g, &[], "none", None).unwrap();
        assert!(!due(&conn, &g, None, false).unwrap());
        assert!(due(&conn, &g, None, true).unwrap());
        save_row(&conn, &g, &[], "locked", None).unwrap();
        assert!(!due(&conn, &g, None, true).unwrap());
        // 自带封面的专辑不联网
        let b = seed(&conn, "C:\\m\\b.flac", "U87", Some("z.jpg"));
        assert!(!due(&conn, &group_of(&conn, b).unwrap(), None, true).unwrap());
    }
}
