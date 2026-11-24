//! 联网获取官方海报：番剧和动画电影查 Bangumi，电视剧和真人电影查 TMDB（需要用户自己填 API Key）。
//! 只把作品名发给外部服务，不发文件路径。匹配结果和海报缓存在本地，匹配过的不再重复查。
//!
//! 每部作品一条 art_match：
//! - state = auto：自动匹配上了，卡片用这张海报
//! - state = pending：有候选但拿不准，不换图，等用户在“更换封面”里确认
//! - state = locked：用户手动选的，重新扫描、重新匹配都不覆盖
//! - state = none：没搜到，或者用户选了“不要联网封面”
use crate::{artwork, library, AppState};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS art_match(
  kind TEXT NOT NULL,              -- series / video
  id INTEGER NOT NULL,
  source TEXT,                     -- bangumi / tmdb
  remote_id TEXT,
  title TEXT,                      -- 官方名字（中文名优先）
  year INTEGER,
  summary TEXT,
  poster TEXT,                     -- 本地缓存的海报（绝对路径）
  score REAL,
  state TEXT NOT NULL,             -- auto / pending / locked / none
  query TEXT,                      -- 当时用来搜的名字，作品改名后重新匹配
  candidates TEXT,                 -- 候选（JSON），给“更换封面”用
  checked_at INTEGER NOT NULL,
  PRIMARY KEY(kind, id)
);
"#,
    )?;
    // 按哪一版匹配规则配的：规则改进后，自动配的（不含手动选的）重新匹配一次
    crate::db::add_column(conn, "art_match", "rules", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(())
}

/// 匹配规则的版本。2：搜索带上文件名里的标题，打分看时长，AI 先说出官方名字再搜
const MATCH_RULES: i64 = 2;

pub fn setup(app: &AppHandle) {
    let app = app.clone();
    // 启动后稍等一会再在后台补全，不抢扫描和界面的资源
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let _ = run(&app).await;
    });
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---------------------------------------------------------------- 设置

/// 设置键 artwork_online
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ArtSettings {
    /// 联网获取封面和资料（默认开，只用 Bangumi 时不需要 key）
    pub enabled: bool,
    /// TMDB 的 API Key（v3）或 API 读访问令牌（v4，eyJ 开头）
    pub tmdb_key: String,
    /// 访问 TMDB 用的代理，例如 http://127.0.0.1:7890；空表示直连
    pub proxy: String,
}

impl Default for ArtSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            tmdb_key: String::new(),
            proxy: String::new(),
        }
    }
}

pub fn settings(conn: &Connection) -> ArtSettings {
    library::get_setting(conn, "artwork_online")
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn client(proxy: &str) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .user_agent(concat!(
            "xiaomo1231/music-player/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/xiaomo1231/music-player)"
        ))
        .timeout(Duration::from_secs(15));
    if !proxy.trim().is_empty() {
        b = b.proxy(reqwest::Proxy::all(proxy.trim())?);
    }
    Ok(b.build()?)
}

// ---------------------------------------------------------------- 候选

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Candidate {
    pub source: String,
    pub remote_id: String,
    /// 显示用的名字（中文名优先）
    pub title: String,
    /// 原名、别名，匹配用
    #[serde(default)]
    pub names: Vec<String>,
    pub year: Option<i64>,
    /// TV / 剧场版 / OVA / movie / tv …
    #[serde(default)]
    pub format: String,
    pub episodes: Option<i64>,
    /// 海报原图地址
    pub image: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub score: f64,
    /// 名字相似度（自动采用要求名字足够像）
    #[serde(default)]
    pub name_score: f64,
}

fn year_of(date: &str) -> Option<i64> {
    date.get(..4).and_then(|y| y.parse().ok())
}

/// Bangumi 条目类型：1 书籍，2 动画，6 三次元
pub async fn bangumi_search(http: &reqwest::Client, keyword: &str, kind: i64) -> Result<Vec<Candidate>> {
    let r = http
        .post("https://api.bgm.tv/v0/search/subjects?limit=10")
        .json(&json!({ "keyword": keyword, "filter": { "type": [kind] } }))
        .send()
        .await?;
    if !r.status().is_success() {
        return Err(anyhow!("Bangumi 返回 {}", r.status()));
    }
    let v: Value = r.json().await?;
    Ok(v["data"].as_array().map(|a| a.iter().filter_map(bangumi_item).collect()).unwrap_or_default())
}

fn bangumi_item(it: &Value) -> Option<Candidate> {
    let id = it["id"].as_i64()?;
    let name = it["name"].as_str().unwrap_or("").to_string();
    let cn = it["name_cn"].as_str().unwrap_or("").to_string();
    let mut names = vec![name.clone()];
    if !cn.is_empty() {
        names.push(cn.clone());
    }
    // infobox 里的“别名”
    if let Some(info) = it["infobox"].as_array() {
        for kv in info.iter().filter(|kv| kv["key"].as_str() == Some("别名")) {
            match &kv["value"] {
                Value::String(s) => names.push(s.clone()),
                Value::Array(a) => names.extend(a.iter().filter_map(|x| x["v"].as_str().map(String::from))),
                _ => {}
            }
        }
    }
    let image = ["large", "common", "medium"]
        .iter()
        .filter_map(|k| it["images"][k].as_str())
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    let episodes = it["eps"].as_i64().or_else(|| it["total_episodes"].as_i64()).filter(|n| *n > 0);
    Some(Candidate {
        source: "bangumi".into(),
        remote_id: id.to_string(),
        title: if cn.is_empty() { name } else { cn },
        names,
        year: it["date"].as_str().and_then(year_of),
        format: it["platform"].as_str().unwrap_or("").to_string(),
        episodes,
        image,
        summary: it["summary"].as_str().unwrap_or("").chars().take(600).collect(),
        score: 0.0,
        name_score: 0.0,
    })
}

/// TMDB 搜索；movie 为 true 时搜电影，否则搜剧集
pub async fn tmdb_search(http: &reqwest::Client, key: &str, query: &str, movie: bool) -> Result<Vec<Candidate>> {
    let key = key.trim();
    let url = format!("https://api.themoviedb.org/3/search/{}", if movie { "movie" } else { "tv" });
    let mut req = http.get(url).query(&[("query", query), ("language", "zh-CN"), ("include_adult", "false")]);
    // v4 读访问令牌走 Bearer，v3 key 走参数
    req = if key.starts_with("eyJ") {
        req.bearer_auth(key)
    } else {
        req.query(&[("api_key", key)])
    };
    let r = req.send().await?;
    if r.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(anyhow!("TMDB API Key 不对"));
    }
    if !r.status().is_success() {
        return Err(anyhow!("TMDB 返回 {}", r.status()));
    }
    let v: Value = r.json().await?;
    Ok(v["results"]
        .as_array()
        .map(|a| a.iter().take(10).filter_map(|it| tmdb_item(it, movie)).collect())
        .unwrap_or_default())
}

fn tmdb_item(it: &Value, movie: bool) -> Option<Candidate> {
    let id = it["id"].as_i64()?;
    let (t, o, d) = if movie {
        ("title", "original_title", "release_date")
    } else {
        ("name", "original_name", "first_air_date")
    };
    let title = it[t].as_str().unwrap_or("").to_string();
    let orig = it[o].as_str().unwrap_or("").to_string();
    let poster = it["poster_path"].as_str().unwrap_or("");
    Some(Candidate {
        source: "tmdb".into(),
        remote_id: format!("{}/{id}", if movie { "movie" } else { "tv" }),
        names: vec![title.clone(), orig],
        title,
        year: it[d].as_str().and_then(year_of),
        format: if movie { "movie" } else { "tv" }.into(),
        episodes: None,
        image: if poster.is_empty() {
            String::new()
        } else {
            format!("https://image.tmdb.org/t/p/w780{poster}")
        },
        summary: it["overview"].as_str().unwrap_or("").chars().take(600).collect(),
        score: 0.0,
        name_score: 0.0,
    })
}

/// AniList（GraphQL，不需要 key）；manga 为 true 时搜漫画
pub async fn anilist_search(http: &reqwest::Client, query: &str, manga: bool) -> Result<Vec<Candidate>> {
    const Q: &str = "query($s:String,$t:MediaType){Page(perPage:10){media(search:$s,type:$t){id format episodes chapters \
        title{romaji english native} synonyms startDate{year} coverImage{extraLarge large} description(asHtml:false)}}}";
    let r = http
        .post("https://graphql.anilist.co")
        .json(&json!({ "query": Q, "variables": { "s": query, "t": if manga { "MANGA" } else { "ANIME" } } }))
        .send()
        .await?;
    if !r.status().is_success() {
        return Err(anyhow!("AniList 返回 {}", r.status()));
    }
    let v: Value = r.json().await?;
    Ok(v["data"]["Page"]["media"]
        .as_array()
        .map(|a| a.iter().filter_map(anilist_item).collect())
        .unwrap_or_default())
}

fn anilist_item(it: &Value) -> Option<Candidate> {
    let id = it["id"].as_i64()?;
    let t = &it["title"];
    let mut names: Vec<String> = ["native", "romaji", "english"].iter().filter_map(|k| t[k].as_str().map(String::from)).collect();
    if let Some(a) = it["synonyms"].as_array() {
        names.extend(a.iter().filter_map(|x| x.as_str().map(String::from)));
    }
    let title = t["native"].as_str().or(t["romaji"].as_str()).unwrap_or("").to_string();
    let format = match it["format"].as_str().unwrap_or("") {
        "MOVIE" => "剧场版".to_string(),
        f => f.to_string(),
    };
    Some(Candidate {
        source: "anilist".into(),
        remote_id: id.to_string(),
        title,
        names,
        year: it["startDate"]["year"].as_i64(),
        format,
        episodes: it["episodes"].as_i64().or(it["chapters"].as_i64()).filter(|n| *n > 0),
        image: it["coverImage"]["extraLarge"]
            .as_str()
            .or(it["coverImage"]["large"].as_str())
            .unwrap_or("")
            .to_string(),
        summary: it["description"].as_str().unwrap_or("").chars().take(600).collect(),
        score: 0.0,
        name_score: 0.0,
    })
}

// ---------------------------------------------------------------- 匹配打分

/// 比较用的名字：小写、全角转半角、去掉空白和标点
pub fn norm(s: &str) -> String {
    s.chars()
        .map(|c| match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            0x3000 => ' ',
            _ => c,
        })
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn bigrams(s: &str) -> Vec<(char, char)> {
    let cs: Vec<char> = s.chars().collect();
    cs.windows(2).map(|w| (w[0], w[1])).collect()
}

/// 名字相似度 0～1
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (la, lb) = (a.chars().count(), b.chars().count());
    if la >= 2 && lb >= 2 && (a.contains(&b) || b.contains(&a)) {
        return 0.5 + 0.45 * la.min(lb) as f64 / la.max(lb) as f64;
    }
    let (x, y) = (bigrams(&a), bigrams(&b));
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }
    let mut y2 = y.clone();
    let mut hit = 0;
    for g in &x {
        if let Some(i) = y2.iter().position(|h| h == g) {
            y2.swap_remove(i);
            hit += 1;
        }
    }
    2.0 * hit as f64 / (x.len() + y.len()) as f64
}

/// 名字里写的季号：“第二季”“2nd Season”“Season 2”“第2期”“II”
pub fn season_in(name: &str) -> Option<i64> {
    static RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)第\s*([0-9一二三四五六七八九十]+)\s*[季期]|(\d+)(?:st|nd|rd|th)\s*season|season\s*(\d+)|\s(II|III|IV)$").unwrap());
    let c = RE.captures(name.trim())?;
    if let Some(m) = c.get(1) {
        let s = m.as_str();
        return s
            .parse()
            .ok()
            .or_else(|| "一二三四五六七八九十".chars().position(|x| s == x.to_string()).map(|i| i as i64 + 1));
    }
    if let Some(m) = c.get(2).or(c.get(3)) {
        return m.as_str().parse().ok();
    }
    c.get(4).map(|m| m.as_str().len() as i64)
}

/// 本地作品的情况，用来给候选打分
#[derive(Clone, Debug, Default)]
pub struct Local {
    pub names: Vec<String>,
    /// 正片（第 1 季）集数
    pub episodes: i64,
    /// 只有特别篇 / 剧场版（或者本身是电影）
    pub movie: bool,
    pub year: Option<i64>,
    /// 偏好的条目形式（漫画：Bangumi 书籍里优先“漫画”，不要小说）
    pub want: String,
    /// 正片平均时长（秒），0 表示不知道
    pub duration: f64,
}

pub fn name_score(c: &Candidate, local: &Local) -> f64 {
    local
        .names
        .iter()
        .flat_map(|q| c.names.iter().chain(std::iter::once(&c.title)).map(move |n| similarity(q, n)))
        .fold(0.0, f64::max)
}

pub fn score(c: &Candidate, local: &Local) -> f64 {
    let mut s = name_score(c, local) * 0.75;
    if !local.want.is_empty() && c.format.contains(local.want.as_str()) {
        s += 0.05;
    }
    // 类型对得上
    let f = c.format.to_lowercase();
    let tv = f == "tv" || f.contains("tv");
    let is_movie = f.contains("剧场") || f.contains("movie") || f.contains("电影") || (c.episodes == Some(1) && !tv);
    if is_movie == local.movie {
        s += 0.1;
    }
    // 一个多小时的单集是电影长度，TV 动画一集二十多分钟
    if local.duration >= 50.0 * 60.0 && tv && c.episodes.unwrap_or(0) > 1 {
        s -= 0.1;
    }
    // Bangumi 的“其他”多是 PV、广告、短片
    if f == "其他" {
        s -= 0.1;
    }
    // 集数接近
    if let (Some(e), true) = (c.episodes, local.episodes > 0) {
        let diff = (e - local.episodes).abs() as f64 / e.max(local.episodes) as f64;
        s += 0.1 * (1.0 - diff.min(1.0));
    } else {
        s += 0.04;
    }
    // 卡片代表整部作品，用第一季的海报：名字写着第二季以后的扣分（本地名字里本来就带季号的除外）
    let local_season = local.names.iter().filter_map(|n| season_in(n)).next();
    let cand_season = c.names.iter().chain(std::iter::once(&c.title)).filter_map(|n| season_in(n)).max();
    if cand_season.unwrap_or(1) > 1 && local_season.is_none() {
        s -= 0.15;
    }
    if let (Some(a), Some(b)) = (local.year, c.year) {
        if a == b {
            s += 0.05;
        } else if (a - b).abs() > 1 {
            s -= 0.05;
        }
    }
    s
}

/// 打分、排序；返回是否可以自动采用最高分的那个
pub fn rank(mut cands: Vec<Candidate>, local: &Local) -> (Vec<Candidate>, bool) {
    for c in &mut cands {
        c.score = score(c, local);
        c.name_score = name_score(c, local);
    }
    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
    cands.dedup_by(|a, b| a.source == b.source && a.remote_id == b.remote_id);
    let auto = match cands.as_slice() {
        [] => false,
        [a] => a.score >= 0.7 && a.name_score >= 0.85,
        [a, b, ..] => a.score >= 0.75 && a.name_score >= 0.85 && (a.score - b.score >= 0.06 || a.score >= 0.92),
    };
    (cands, auto)
}

trait Hopeless {
    fn is_none_or_low(&self) -> bool;
}
impl Hopeless for Option<&Candidate> {
    fn is_none_or_low(&self) -> bool {
        self.map_or(true, |c| c.name_score < 0.35)
    }
}

/// 搜索用的名字：原名、去掉括号的部分、括号里的部分（“girls band cry（哭泣少女乐队）”）
pub fn query_names(title: &str) -> Vec<String> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[（(\[【]([^）)\]】]+)[）)\]】]").unwrap());
    let mut out = vec![];
    let base = RE.replace_all(title, " ").trim().to_string();
    if !base.is_empty() {
        out.push(base);
    }
    for c in RE.captures_iter(title) {
        let s = c[1].trim().to_string();
        if norm(&s).chars().count() >= 2 && !out.contains(&s) {
            out.push(s);
        }
    }
    if out.is_empty() {
        out.push(title.trim().to_string());
    }
    out
}

// ---------------------------------------------------------------- 后台补全

/// 要匹配的一项
#[derive(Clone, Debug)]
struct Job {
    kind: &'static str,
    id: i64,
    title: String,
    /// anime / tv / movie / anime-movie
    what: &'static str,
    local: Local,
}

fn jobs(conn: &Connection, only: Option<(&str, i64)>, force: bool) -> Result<Vec<Job>> {
    let mut out = vec![];
    // 番剧 / 剧集
    let mut st = conn.prepare(
        "SELECT s.id,coalesce(s.custom_title,s.title),s.kind,s.year,
                sum(coalesce(v.season,1)=1),sum(coalesce(v.season,1)>=1),
                a.state,a.query,a.checked_at,coalesce(a.rules,0),
                avg(CASE WHEN coalesce(v.season,1)>=1 THEN v.duration END)
         FROM series s JOIN videos v ON v.series_id=s.id JOIN files f ON f.id=v.file_id
         LEFT JOIN art_match a ON a.kind='series' AND a.id=s.id
         WHERE f.missing=0 GROUP BY s.id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<i64>>(8)?,
            r.get::<_, i64>(9)?,
            r.get::<_, Option<f64>>(10)?,
        ))
    })?;
    let rows: Vec<_> = rows.flatten().collect();
    drop(st);
    for r in rows {
        let (id, title, kind, year, s1, main, state, query, at, rules, dur) = r;
        if !due(only, ("series", id), force, &title, state.as_deref(), query.as_deref(), at, rules) {
            continue;
        }
        let paths: Vec<String> = conn
            .prepare(
                "SELECT f.path FROM videos v JOIN files f ON f.id=v.file_id
                 WHERE v.series_id=?1 AND f.missing=0 ORDER BY coalesce(v.season,1)<1,v.season,v.episode LIMIT 4",
            )?
            .query_map([id], |r| r.get(0))?
            .flatten()
            .collect();
        let movie = main == 0;
        out.push(Job {
            kind: "series",
            id,
            title: title.clone(),
            what: match (kind.as_str(), movie) {
                ("anime", true) => "anime-movie",
                ("anime", false) => "anime",
                _ => "tv",
            },
            local: Local {
                names: with_file_titles(query_names(&title), &paths),
                episodes: s1,
                movie,
                year,
                want: String::new(),
                duration: dur.unwrap_or(0.0),
            },
        });
    }
    // 电影
    let mut st = conn.prepare(
        "SELECT f.id,coalesce(f.custom_title,v.title),f.kind,a.state,a.query,a.checked_at,coalesce(a.rules,0),f.path,v.duration
         FROM videos v JOIN files f ON f.id=v.file_id
         LEFT JOIN art_match a ON a.kind='video' AND a.id=f.id
         WHERE f.missing=0 AND v.movie=1 AND v.series_id IS NULL",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, f64>(8)?,
        ))
    })?;
    for r in rows.flatten() {
        let (id, title, kind, state, query, at, rules, path, dur) = r;
        if !due(only, ("video", id), force, &title, state.as_deref(), query.as_deref(), at, rules) {
            continue;
        }
        let year = movie_year(&title);
        out.push(Job {
            kind: "video",
            id,
            title: title.clone(),
            what: if kind == "anime" { "anime-movie" } else { "movie" },
            local: Local {
                names: with_file_titles(query_names(&strip_year(&title)), &[path]),
                episodes: 0,
                movie: true,
                year,
                want: String::new(),
                duration: dur,
            },
        });
    }
    // 漫画：按系列（一个系列一张海报）
    let series: Vec<String> = conn
        .prepare("SELECT DISTINCT c.series FROM comics c JOIN files f ON f.id=c.file_id WHERE f.missing=0 AND c.series<>''")?
        .query_map([], |r| r.get(0))?
        .flatten()
        .collect();
    for name in series {
        let id = comic_id(&name);
        let (state, query, at, rules): (Option<String>, Option<String>, Option<i64>, i64) = conn
            .query_row(
                "SELECT state,query,checked_at,coalesce(rules,0) FROM art_match WHERE kind='comic' AND id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .unwrap_or((None, None, None, MATCH_RULES));
        if !due(only, ("comic", id), force, &name, state.as_deref(), query.as_deref(), at, rules) {
            continue;
        }
        out.push(Job {
            kind: "comic",
            id,
            title: name.clone(),
            what: "comic",
            local: Local {
                names: query_names(&name),
                want: "漫画".into(),
                ..Default::default()
            },
        });
    }
    Ok(out)
}

/// 文件名里的标题也拿来搜：文件夹常是简称或译名，压制组的文件名里往往是罗马音 / 英文原名
/// （“辉夜姬”文件夹里的 `[MAI] Chou Kaguya-hime! [WebRip 2160p].mkv`）
fn with_file_titles(mut names: Vec<String>, paths: &[String]) -> Vec<String> {
    for p in paths {
        let t = crate::parse::parse_episode(std::path::Path::new(p)).title;
        let t = strip_year(t.trim());
        if norm(&t).chars().count() < 2 || names.iter().any(|n| similarity(n, &t) >= 0.85) {
            continue;
        }
        names.push(t);
    }
    names
}

/// 漫画系列没有自己的表，用系列名的哈希当编号（只取 52 位，前端 JS 数字也能精确表示）
pub fn comic_id(series: &str) -> i64 {
    let h = blake3::hash(series.as_bytes());
    i64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap_or([0; 8])) & ((1 << 52) - 1)
}

fn movie_year(title: &str) -> Option<i64> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:^|[\s.(（\[])((?:19|20)\d\d)(?:$|[\s.)）\]])").unwrap());
    RE.captures(title).and_then(|c| c[1].parse().ok())
}

fn strip_year(title: &str) -> String {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s.(（\[]*(?:19|20)\d\d[)）\]]?\s*$").unwrap());
    let s = RE.replace(title, "").trim().to_string();
    if s.is_empty() {
        title.to_string()
    } else {
        s
    }
}

/// 要不要（重新）匹配这一项
#[allow(clippy::too_many_arguments)]
fn due(only: Option<(&str, i64)>, me: (&str, i64), force: bool, title: &str, state: Option<&str>, query: Option<&str>, at: Option<i64>, rules: i64) -> bool {
    // 指定了某一项（手动更换、恢复自动）时不看状态
    if let Some(o) = only {
        return o == me;
    }
    match state {
        None => true,
        // 手动选的、手动关掉联网封面的都不再自动匹配
        Some("locked") | Some("off") => false,
        _ if force => true,
        // 作品改名（重新归类）了、匹配规则改进了就重新匹配
        _ if query != Some(title) || rules < MATCH_RULES => true,
        // 没搜到的一周后再试
        Some("none") => crate::db::now() - at.unwrap_or(0) > 7 * 86400,
        _ => false,
    }
}

static RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone)]
struct Updated {
    kind: &'static str,
    id: i64,
    poster: Option<String>,
    state: String,
}

/// 后台补全所有还没匹配过的作品。返回处理了多少个
pub async fn run(app: &AppHandle) -> Result<usize> {
    run_some(app, None, false).await
}

async fn run_some(app: &AppHandle, only: Option<(&str, i64)>, force: bool) -> Result<usize> {
    let (cfg, list) = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        (settings(&conn), jobs(&conn, only, force)?)
    };
    if !cfg.enabled || list.is_empty() || RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(0);
    }
    let r = run_jobs(app, &cfg, list).await;
    RUNNING.store(false, Ordering::SeqCst);
    r
}

async fn run_jobs(app: &AppHandle, cfg: &ArtSettings, list: Vec<Job>) -> Result<usize> {
    let bgm = client("")?;
    let tmdb = client(&cfg.proxy)?;
    let posters = app.state::<AppState>().dirs.covers.clone();
    let mut done = 0;
    for job in list {
        let mut job = job;
        let cands = search(&bgm, &tmdb, cfg, &job, &job.local.names).await;
        let Ok(mut cands) = cands else {
            // 网络不通：这次先不记，下次启动再试
            continue;
        };
        let (mut ranked, mut auto) = rank(cands.clone(), &job.local);
        // 拿不准时（配置了 AI 并打开了“AI 辅助识别”）：先让 AI 说出这部作品的官方名字，用这些名字再搜一轮
        if !auto && ai_on(app) {
            let names: Vec<String> = ai_names(app, &job)
                .await
                .into_iter()
                .filter(|n| !job.local.names.iter().any(|m| norm(m) == norm(n)))
                .collect();
            if !names.is_empty() {
                if let Ok(more) = search(&bgm, &tmdb, cfg, &job, &names).await {
                    cands.extend(more);
                }
                job.local.names.extend(names);
                (ranked, auto) = rank(cands, &job.local);
            }
        }
        let mut cands = ranked;
        // 还是拿不准：让 AI 从候选里挑；挑中的名字也得有几分像，AI 也拿不准就留着待确认
        if !auto && cands.len() > 1 && !cands.first().is_none_or_low() {
            if let Some(i) = ai_pick(app, &job, &cands).await {
                if cands[i].name_score >= 0.5 {
                    let c = cands.remove(i);
                    cands.insert(0, c);
                    auto = true;
                }
            }
        }
        // 最像的也差得远：当作没搜到（候选还留着，手动更换时可以看）
        let mut state = if cands.first().is_none_or_low() && !auto {
            "none"
        } else if auto {
            "auto"
        } else {
            "pending"
        };
        let mut poster = None;
        if auto {
            match download_poster(&bgm, &tmdb, &cands[0], &posters).await {
                Ok(p) => poster = Some(p),
                Err(_) => state = "pending",
            }
        }
        {
            let st = app.state::<AppState>();
            let conn = st.db.lock();
            save(&conn, &job, &cands, state, poster.as_deref())?;
        }
        let _ = app.emit(
            "art://updated",
            Updated {
                kind: job.kind,
                id: job.id,
                poster,
                state: state.into(),
            },
        );
        done += 1;
        // Bangumi 要求控制频率
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    Ok(done)
}

async fn search(bgm: &reqwest::Client, tmdb: &reqwest::Client, cfg: &ArtSettings, job: &Job, names: &[String]) -> Result<Vec<Candidate>> {
    let has_tmdb = !cfg.tmdb_key.trim().is_empty();
    let mut all = vec![];
    let mut last_err = None;
    for (n, q) in names.iter().take(3).enumerate() {
        if n > 0 {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        let r = match job.what {
            "anime" | "anime-movie" => match bangumi_search(bgm, q, 2).await {
                // Bangumi 没有结果（或连不上）时退到 AniList，再退到 TMDB
                Ok(v) if !v.is_empty() => Ok(v),
                _ => match anilist_search(bgm, q, false).await {
                    Ok(v) if v.is_empty() && has_tmdb => tmdb_search(tmdb, &cfg.tmdb_key, q, job.what == "anime-movie").await,
                    r => r,
                },
            },
            "comic" => match bangumi_search(bgm, q, 1).await {
                Ok(v) if !v.is_empty() => Ok(v),
                _ => anilist_search(bgm, q, true).await,
            },
            "tv" if has_tmdb => tmdb_search(tmdb, &cfg.tmdb_key, q, false).await,
            "movie" if has_tmdb => tmdb_search(tmdb, &cfg.tmdb_key, q, true).await,
            // 没填 TMDB key：电视剧和真人电影没有可用的来源
            _ => Ok(vec![]),
        };
        match r {
            Ok(v) => {
                all.extend(v);
                // 第一个名字就有很像的结果，不用再搜
                let (ranked, auto) = rank(all.clone(), &job.local);
                if auto && ranked[0].score >= 0.85 {
                    break;
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    match (all.is_empty(), last_err) {
        (true, Some(e)) => Err(e),
        _ => Ok(all),
    }
}

fn ai_on(app: &AppHandle) -> bool {
    let on = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        crate::ai::load_config(&conn).classify_media
    };
    on && crate::ai::ready(app).is_ok()
}

/// 给 AI 看的本地作品情况（只有名字和数字，不含路径）
fn ai_local(job: &Job) -> Value {
    json!({
        "folder": job.title,
        "names_in_files": job.local.names.iter().skip(1).take(3).collect::<Vec<_>>(),
        "type": job.what,
        "episodes": job.local.episodes,
        "minutes_per_episode": (job.local.duration > 0.0).then(|| (job.local.duration / 60.0).round() as i64),
        "year": job.local.year,
    })
}

const AI_NAMES: &str = r#"你帮本地媒体库识别作品。用户给出文件夹名、文件名里的标题、类型、集数和单集时长。
文件夹名常是简称或译名，文件名里常是罗马音或英文名。请判断这是哪部作品，给出它的官方名字：日文原名、中文名、英文名（知道几个给几个，最多 3 个）。
不知道是哪部作品就给空数组，不要猜。只输出 JSON：{"names":["…"]}"#;

/// AI 说出作品的官方名字，用来再搜一轮
async fn ai_names(app: &AppHandle, job: &Job) -> Vec<String> {
    let user = ai_local(job).to_string();
    let Ok(reply) = crate::ai::chat_simple(app, AI_NAMES, &user, true).await else {
        return vec![];
    };
    let Ok(v) = crate::ai::extract_json(&reply) else {
        return vec![];
    };
    let mut out: Vec<String> = vec![];
    for n in v["names"].as_array().into_iter().flatten().filter_map(|n| n.as_str()) {
        let n = n.trim().to_string();
        if norm(&n).chars().count() >= 2 && !out.iter().any(|m| norm(m) == norm(&n)) {
            out.push(n);
        }
    }
    out.truncate(3);
    out
}

const AI_SYSTEM: &str = r#"你帮本地媒体库给作品配官方海报。用户给出本地作品的情况（文件夹名、文件名里的标题、类型、集数、单集时长），以及资料库搜到的候选条目。
请选出和本地作品是同一部作品的那一个（整部作品优先选第一季 / 正篇；本地是一个多小时的单集时选剧场版 / 电影）。
名字只是部分相同的不同作品不要选。都不是或拿不准就回答 null。只输出 JSON：{"pick":候选编号或null}"#;

/// AI 从候选里挑一个，返回下标；AI 也拿不准时返回 None
async fn ai_pick(app: &AppHandle, job: &Job, cands: &[Candidate]) -> Option<usize> {
    if !ai_on(app) {
        return None;
    }
    let list: Vec<Value> = cands
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, c)| {
            json!({"n": i, "names": c.names.iter().chain(std::iter::once(&c.title)).take(5).collect::<Vec<_>>(),
                   "year": c.year, "format": c.format, "episodes": c.episodes,
                   "summary": c.summary.chars().take(80).collect::<String>()})
        })
        .collect();
    let user = json!({"local": ai_local(job), "candidates": list}).to_string();
    let reply = crate::ai::chat_simple(app, AI_SYSTEM, &user, true).await.ok()?;
    let v = crate::ai::extract_json(&reply).ok()?;
    let i = v["pick"].as_u64()? as usize;
    (i < cands.len().min(8)).then_some(i)
}

/// 下载海报，缩到长边 900 像素存成 jpg，返回本地路径
async fn download_poster(bgm: &reqwest::Client, tmdb: &reqwest::Client, c: &Candidate, dir: &std::path::Path) -> Result<String> {
    if c.image.is_empty() {
        return Err(anyhow!("没有海报"));
    }
    let out = dir.join(format!("art_{}_{}.jpg", c.source, c.remote_id.replace('/', "_")));
    if !out.exists() {
        let http = if c.source == "tmdb" { tmdb } else { bgm };
        let bytes = http.get(&c.image).send().await?.error_for_status()?.bytes().await?;
        let img = image::load_from_memory(&bytes)?;
        let out2 = out.clone();
        tauri::async_runtime::spawn_blocking(move || artwork::save_jpg(&img, 900, &out2)).await??;
    }
    Ok(out.to_string_lossy().into_owned())
}

fn save(conn: &Connection, job: &Job, cands: &[Candidate], state: &str, poster: Option<&str>) -> Result<()> {
    let best = cands.first().filter(|_| state == "auto" || state == "locked");
    conn.execute(
        "INSERT OR REPLACE INTO art_match(kind,id,source,remote_id,title,year,summary,poster,score,state,query,candidates,checked_at,rules)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            job.kind,
            job.id,
            best.map(|c| c.source.clone()),
            best.map(|c| c.remote_id.clone()),
            best.map(|c| c.title.clone()),
            best.and_then(|c| c.year),
            best.map(|c| c.summary.clone()),
            poster,
            cands.first().map(|c| c.score),
            state,
            job.title,
            serde_json::to_string(&cands.iter().take(12).collect::<Vec<_>>())?,
            crate::db::now(),
            MATCH_RULES
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- 命令

/// 一部作品的联网匹配结果（卡片、详情页、更换封面面板用）
#[derive(Serialize, Clone, Debug, Default)]
pub struct ArtInfo {
    pub kind: String,
    pub id: i64,
    pub source: Option<String>,
    pub remote_id: Option<String>,
    pub title: Option<String>,
    pub year: Option<i64>,
    pub summary: Option<String>,
    pub poster: Option<String>,
    pub state: String,
    /// 本地作品名（漫画按系列名对应）
    pub query: Option<String>,
}

fn row_info(r: &rusqlite::Row) -> rusqlite::Result<ArtInfo> {
    Ok(ArtInfo {
        kind: r.get(0)?,
        id: r.get(1)?,
        source: r.get(2)?,
        remote_id: r.get(3)?,
        title: r.get(4)?,
        year: r.get(5)?,
        summary: r.get(6)?,
        poster: r.get(7)?,
        state: r.get(8)?,
        query: r.get(9)?,
    })
}

const INFO_COLS: &str = "kind,id,source,remote_id,title,year,summary,poster,state,query";

/// 某一类（series / video）全部的匹配结果
#[tauri::command]
pub fn art_list(st: State<AppState>, kind: String) -> Result<Vec<ArtInfo>, String> {
    let conn = st.db.lock();
    let mut s = conn.prepare(&format!("SELECT {INFO_COLS} FROM art_match WHERE kind=?1")).map_err(err)?;
    let rows = s.query_map([kind], row_info).map_err(err)?.flatten().collect();
    Ok(rows)
}

/// 后台补全（打开番剧 / 电影页时调用；正在跑就直接返回）
#[tauri::command]
pub async fn art_auto(app: AppHandle) -> Result<usize, String> {
    run(&app).await.map_err(err)
}

/// 全部重新匹配（手动选过的保留）
#[tauri::command]
pub async fn art_refresh_all(app: AppHandle) -> Result<usize, String> {
    run_some(&app, None, true).await.map_err(err)
}

/// 测试连接：分别试 Bangumi 和 TMDB，返回每个来源的结果说明
#[tauri::command]
pub async fn art_test(st: State<'_, AppState>) -> Result<Vec<String>, String> {
    let cfg = settings(&st.db.lock());
    let mut out = vec![];
    match bangumi_search(&client("").map_err(err)?, "孤独摇滚", 2).await {
        Ok(v) if !v.is_empty() => out.push("Bangumi：连接正常".to_string()),
        Ok(_) => out.push("Bangumi：连上了，但没有搜到结果".to_string()),
        Err(e) => out.push(format!("Bangumi：连接失败（{e}）")),
    }
    if cfg.tmdb_key.trim().is_empty() {
        out.push("TMDB：没有填 API Key，电视剧和真人电影不会联网找海报".to_string());
    } else {
        match tmdb_search(&client(&cfg.proxy).map_err(err)?, &cfg.tmdb_key, "Inception", true).await {
            Ok(_) => out.push("TMDB：连接正常".to_string()),
            Err(e) => out.push(format!("TMDB：连接失败（{e}）。国内网络可能需要在上面填代理")),
        }
    }
    Ok(out)
}

/// 更换封面面板：当前结果和上次搜到的候选
#[derive(Serialize)]
pub struct Picker {
    pub info: Option<ArtInfo>,
    pub candidates: Vec<Candidate>,
    /// 搜索框里默认填的名字
    pub query: String,
    /// 这一项默认用哪个来源搜：bangumi / tmdb
    pub source: String,
}

fn job_for(conn: &Connection, kind: &str, id: i64) -> Result<Job> {
    jobs(conn, Some((kind, id)), true)?.into_iter().next().ok_or_else(|| anyhow!("找不到这部作品"))
}

fn default_source(what: &str) -> &'static str {
    match what {
        "tv" | "movie" => "tmdb",
        _ => "bangumi",
    }
}

#[tauri::command]
pub fn art_picker(st: State<AppState>, kind: String, id: i64) -> Result<Picker, String> {
    let conn = st.db.lock();
    let job = job_for(&conn, &kind, id).map_err(err)?;
    Ok(Picker {
        info: info(&conn, &kind, id).map_err(err)?,
        candidates: candidates(&conn, &kind, id).map_err(err)?,
        query: job.local.names.first().cloned().unwrap_or(job.title),
        source: default_source(job.what).into(),
    })
}

/// 手动搜索：source 为 bangumi / anilist / tmdb
#[tauri::command]
pub async fn art_search(st: State<'_, AppState>, kind: String, id: i64, query: String, source: String) -> Result<Vec<Candidate>, String> {
    let (cfg, job) = {
        let conn = st.db.lock();
        (settings(&conn), job_for(&conn, &kind, id).map_err(err)?)
    };
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let comic = job.what == "comic";
    let movie = matches!(job.what, "movie" | "anime-movie");
    let r = match source.as_str() {
        "anilist" => anilist_search(&client("").map_err(err)?, q, comic).await,
        "tmdb" => {
            if cfg.tmdb_key.trim().is_empty() {
                return Err("还没有填 TMDB API Key（设置 → 封面与资料）".into());
            }
            tmdb_search(&client(&cfg.proxy).map_err(err)?, &cfg.tmdb_key, q, movie).await
        }
        _ => bangumi_search(&client("").map_err(err)?, q, if comic { 1 } else { 2 }).await,
    };
    let local = Local {
        names: vec![q.to_string()],
        ..job.local
    };
    Ok(rank(r.map_err(err)?, &local).0)
}

/// 选定一个候选：下载海报并锁定，以后不再自动匹配
#[tauri::command]
pub async fn art_choose(app: AppHandle, kind: String, id: i64, candidate: Candidate) -> Result<ArtInfo, String> {
    let (cfg, job, dir) = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        (settings(&conn), job_for(&conn, &kind, id).map_err(err)?, posters_dir(&st))
    };
    let poster = download_poster(&client("").map_err(err)?, &client(&cfg.proxy).map_err(err)?, &candidate, &dir)
        .await
        .map_err(|e| format!("下载海报失败：{e}"))?;
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    let mut cands = candidates(&conn, &kind, id).map_err(err)?;
    cands.retain(|c| !(c.source == candidate.source && c.remote_id == candidate.remote_id));
    cands.insert(0, candidate);
    save(&conn, &job, &cands, "locked", Some(&poster)).map_err(err)?;
    let info = info(&conn, &kind, id).map_err(err)?.unwrap_or_default();
    let _ = app.emit(
        "art://updated",
        Updated {
            kind: job.kind,
            id,
            poster: Some(poster),
            state: "locked".into(),
        },
    );
    Ok(info)
}

/// 漫画系列的编号（漫画没有系列表，按名字算）
#[tauri::command]
pub fn art_comic_id(series: String) -> i64 {
    comic_id(&series)
}

/// 这部作品不要联网海报（用本地图或截帧）
#[tauri::command]
pub fn art_off(app: AppHandle, st: State<AppState>, kind: String, id: i64) -> Result<(), String> {
    let conn = st.db.lock();
    let job = job_for(&conn, &kind, id).map_err(err)?;
    let cands = candidates(&conn, &kind, id).map_err(err)?;
    save(&conn, &job, &cands, "off", None).map_err(err)?;
    let _ = app.emit(
        "art://updated",
        Updated {
            kind: job.kind,
            id,
            poster: None,
            state: "off".into(),
        },
    );
    Ok(())
}

/// 恢复自动匹配：清掉手动选择，马上重新匹配这一项
#[tauri::command]
pub async fn art_reset(app: AppHandle, kind: String, id: i64) -> Result<Option<ArtInfo>, String> {
    let kind_s: &'static str = match kind.as_str() {
        "series" => "series",
        "video" => "video",
        _ => "comic",
    };
    {
        let st = app.state::<AppState>();
        st.db
            .lock()
            .execute("DELETE FROM art_match WHERE kind=?1 AND id=?2", params![kind_s, id])
            .map_err(err)?;
    }
    let _ = app.emit(
        "art://updated",
        Updated {
            kind: kind_s,
            id,
            poster: None,
            state: "none".into(),
        },
    );
    // 后台批量补全正在跑时会顺带处理到；否则单独跑这一项
    run_some(&app, Some((kind_s, id)), true).await.map_err(err)?;
    let st = app.state::<AppState>();
    let r = info(&st.db.lock(), kind_s, id).map_err(err)?;
    Ok(r)
}

pub fn info(conn: &Connection, kind: &str, id: i64) -> Result<Option<ArtInfo>> {
    Ok(conn
        .query_row(
            &format!("SELECT {INFO_COLS} FROM art_match WHERE kind=?1 AND id=?2"),
            params![kind, id],
            row_info,
        )
        .optional()?)
}

pub fn candidates(conn: &Connection, kind: &str, id: i64) -> Result<Vec<Candidate>> {
    let s: Option<String> = conn
        .query_row("SELECT candidates FROM art_match WHERE kind=?1 AND id=?2", params![kind, id], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

pub fn posters_dir(st: &AppState) -> PathBuf {
    st.dirs.covers.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(title: &str, names: &[&str], format: &str, eps: Option<i64>) -> Candidate {
        Candidate {
            source: "bangumi".into(),
            remote_id: title.into(),
            title: title.into(),
            names: names.iter().map(|s| s.to_string()).collect(),
            format: format.into(),
            episodes: eps,
            ..Default::default()
        }
    }

    #[test]
    fn norm_and_similarity() {
        assert_eq!(norm("BanG Dream! It's MyGO!!!!!"), "bangdreamitsmygo");
        assert_eq!(norm("孤独摇滚！"), "孤独摇滚");
        assert_eq!(similarity("孤独摇滚！", "孤独摇滚!"), 1.0);
        assert!(similarity("Ave Mujica", "BanG Dream! Ave Mujica") > 0.6);
        assert!(similarity("孤独摇滚", "辉夜大小姐想让我告白") < 0.2);
    }

    #[test]
    fn season_numbers() {
        assert_eq!(season_in("孤独摇滚！第二季"), Some(2));
        assert_eq!(season_in("Bocchi the Rock! 2nd Season"), Some(2));
        assert_eq!(season_in("Season 3"), Some(3));
        assert_eq!(season_in("Overlord II"), Some(2));
        assert_eq!(season_in("BanG Dream! It's MyGO!!!!!"), None);
    }

    #[test]
    fn query_names_split_brackets() {
        assert_eq!(query_names("girls band cry（哭泣少女乐队）"), vec!["girls band cry", "哭泣少女乐队"]);
        assert_eq!(query_names("孤独摇滚！"), vec!["孤独摇滚！"]);
    }

    #[test]
    fn prefers_first_season_tv() {
        let local = Local {
            names: query_names("孤独摇滚！"),
            episodes: 12,
            movie: false,
            year: None,
            want: String::new(),
            duration: 0.0,
        };
        let cands = vec![
            cand("孤独摇滚！第二季", &["ぼっち・ざ・ろっく！ 2nd Season", "孤独摇滚！第二季"], "TV", Some(12)),
            cand(
                "剧场总集篇 孤独摇滚！前篇",
                &["劇場総集編ぼっち・ざ・ろっく！ Re:", "剧场总集篇 孤独摇滚！前篇"],
                "剧场版",
                None,
            ),
            cand("孤独摇滚！", &["ぼっち・ざ・ろっく！", "孤独摇滚！"], "TV", Some(12)),
        ];
        let (r, auto) = rank(cands, &local);
        assert_eq!(r[0].title, "孤独摇滚！");
        assert!(auto, "{:?}", r.iter().map(|c| (c.title.clone(), c.score)).collect::<Vec<_>>());
    }

    #[test]
    fn unsure_match_is_pending() {
        let local = Local {
            names: query_names("辉夜姬"),
            episodes: 1,
            movie: true,
            year: None,
            want: String::new(),
            duration: 0.0,
        };
        let cands = vec![
            cand("辉夜大小姐想让我告白", &["かぐや様は告らせたい"], "TV", Some(12)),
            cand("辉夜姬物语", &["かぐや姫の物語"], "剧场版", Some(1)),
        ];
        let (r, auto) = rank(cands, &local);
        assert!(!auto, "{:?}", r.iter().map(|c| (c.title.clone(), c.score)).collect::<Vec<_>>());
    }

    #[test]
    fn bangumi_json() {
        let v: Value = serde_json::from_str(
            r#"{"id":428735,"name":"BanG Dream! It's MyGO!!!!!","name_cn":"","date":"2023-06-29","platform":"TV","eps":13,
                "images":{"large":"https://lain.bgm.tv/pic/cover/l/x.jpg","common":"c"},
                "infobox":[{"key":"别名","value":[{"v":"迷途之子!!!!!"}]}]}"#,
        )
        .unwrap();
        let c = bangumi_item(&v).unwrap();
        assert_eq!(c.title, "BanG Dream! It's MyGO!!!!!");
        assert_eq!(c.year, Some(2023));
        assert_eq!(c.episodes, Some(13));
        assert!(c.names.contains(&"迷途之子!!!!!".to_string()));
        assert!(c.image.ends_with("x.jpg"));
    }

    /// 2026-11 从 Bangumi 实际搜到的结果（节选），用作品库里真实的名字测试
    fn bgm(id: &str, name: &str, cn: &str, platform: &str, eps: i64, alias: &[&str]) -> Candidate {
        let mut names = vec![name.to_string()];
        if !cn.is_empty() {
            names.push(cn.to_string());
        }
        names.extend(alias.iter().map(|s| s.to_string()));
        Candidate {
            source: "bangumi".into(),
            remote_id: id.into(),
            title: if cn.is_empty() { name.into() } else { cn.into() },
            names,
            format: platform.into(),
            episodes: (eps > 0).then_some(eps),
            ..Default::default()
        }
    }

    #[test]
    fn real_bangumi_results() {
        let pick = |title: &str, eps: i64, movie: bool, cands: Vec<Candidate>| {
            let (r, auto) = rank(
                cands,
                &Local {
                    names: query_names(title),
                    episodes: eps,
                    movie,
                    year: None,
                    want: String::new(),
                    duration: 0.0,
                },
            );
            (r[0].remote_id.clone(), auto && !Some(&r[0]).is_none_or_low())
        };
        let bocchi = vec![
            bgm("328609", "ぼっち・ざ・ろっく！", "孤独摇滚！", "TV", 12, &["BTR", "Bocchi the Rock!"]),
            bgm(
                "537409",
                "ぼっち・ざ・ろっく！ 第2期",
                "孤独摇滚！第二季",
                "TV",
                0,
                &["BTR 2", "Bocchi the Rock! Season 2"],
            ),
            bgm("718086", "映画ぼっち・ざ・ろっく！星のあしあと", "电影 孤独摇滚！星之足迹", "剧场版", 0, &[]),
            bgm("436738", "劇場総集編ぼっち・ざ・ろっく！ Re:", "剧场总集篇孤独摇滚！ Re:", "剧场版", 1, &[]),
        ];
        assert_eq!(pick("孤独摇滚！", 12, false, bocchi), ("328609".into(), true));
        let mygo = vec![
            bgm("428735", "BanG Dream! It's MyGO!!!!!", "", "TV", 13, &["迷途之子!!!!!"]),
            bgm(
                "473832",
                "劇場版 BanG Dream! It's MyGO!!!!! 前編 春の陽だまり、迷い猫",
                "剧场版 BanG Dream! It's MyGO!!!!! 前篇 春日向阳，迷途野猫",
                "剧场版",
                1,
                &[],
            ),
            bgm("186515", "BanG Dream!", "", "TV", 13, &["炸梦！"]),
        ];
        assert_eq!(pick("BanG Dream! It's MyGO!!!!!", 13, false, mygo), ("428735".into(), true));
        let ave = vec![
            bgm("454684", "BanG Dream! Ave Mujica", "", "TV", 13, &["颂乐人偶"]),
            bgm("578262", "BanG Dream! Ave Mujica prima aurora", "", "剧场版", 1, &[]),
            bgm(
                "548133",
                "BanG Dream! MyGO!!!!! // Ave Mujica",
                "BanG Dream! MyGO!!!!! // Ave Mujica",
                "TV",
                0,
                &[],
            ),
        ];
        assert_eq!(pick("BanG Dream! Ave Mujica", 12, false, ave), ("454684".into(), true));
        let gbc = vec![
            bgm(
                "431767",
                "ガールズバンドクライ",
                "少女乐队的呐喊",
                "TV",
                13,
                &["GIRLS BAND CRY", "少女樂團 吶喊吧"],
            ),
            bgm(
                "512792",
                "劇場版総集編 ガールズバンドクライ 青春狂走曲",
                "剧场版总集篇 少女乐队的呐喊 青春狂走曲",
                "剧场版",
                1,
                &["剧场版总集篇 GIRLS BAND CRY 青春狂走曲"],
            ),
            bgm(
                "587772",
                "ガールズバンドクライ THE MOVIE",
                "少女乐队的呐喊 THE MOVIE",
                "剧场版",
                0,
                &["GIRLS BAND CRY THE MOVIE"],
            ),
        ];
        assert_eq!(pick("girls band cry（哭泣少女乐队）", 13, false, gbc), ("431767".into(), true));
        // “辉夜姬”文件夹里是《超辉夜姬！》：名字不够像，不自动换，等手动确认
        let kaguya = vec![
            bgm("56847", "かぐや姫の物語", "辉夜姬物语", "剧场版", 1, &["辉耀姬物语"]),
            bgm("517768", "かぐやひめ", "辉夜姬", "其他", 0, &[]),
            bgm("604826", "超かぐや姫！", "超辉夜姬！", "WEB", 1, &["Cosmic Princess Kaguya!"]),
        ];
        assert!(!pick("辉夜姬", 1, true, kaguya.clone()).1);
        // 文件名里的标题也拿来搜
        let names = with_file_titles(
            query_names("辉夜姬"),
            &["/番剧/辉夜姬/[MAI] Chou Kaguya-hime! [WebRip 2160p HEVC-10bit EAC3 ASS].mkv".into()],
        );
        assert_eq!(names, vec!["辉夜姬".to_string(), "Chou Kaguya-hime!".to_string()]);
        // AI 说出官方名字后，《超辉夜姬！》名字对上了，2 小时 22 分的单集也是电影长度，可以自动采用
        let local = Local {
            names: [names, vec!["超かぐや姫！".into(), "Cosmic Princess Kaguya!".into()]].concat(),
            episodes: 1,
            movie: true,
            duration: 8523.0,
            ..Default::default()
        };
        let (r, auto) = rank(kaguya, &local);
        assert_eq!(r[0].remote_id, "604826");
        assert!(auto);
        assert!(r.iter().find(|c| c.remote_id == "517768").unwrap().score < 0.75);
        // 搜不到的：只有毫不相干的结果
        let none = vec![bgm("543189", "野原ひろし 昼メシの流儀", "野原广志午餐流派", "TV", 12, &[])];
        let (r, _) = rank(
            none,
            &Local {
                names: query_names("廣井菊里的深酒日記"),
                ..Default::default()
            },
        );
        assert!(Some(&r[0]).is_none_or_low());
    }

    #[test]
    fn anilist_json_and_comic_id() {
        let v: Value = serde_json::from_str(
            r#"{"id":130003,"format":"TV","episodes":12,"title":{"romaji":"Bocchi the Rock!","english":"BOCCHI THE ROCK!","native":"ぼっち・ざ・ろっく！"},
                "synonyms":["BTR"],"startDate":{"year":2022},"coverImage":{"extraLarge":"https://s4.anilist.co/x.png"}}"#,
        )
        .unwrap();
        let c = anilist_item(&v).unwrap();
        assert_eq!((c.title.as_str(), c.year, c.episodes), ("ぼっち・ざ・ろっく！", Some(2022), Some(12)));
        assert!(c.names.contains(&"BTR".to_string()));
        let id = comic_id("孤独摇滚");
        assert_eq!(id, comic_id("孤独摇滚"));
        assert!((0..(1 << 52)).contains(&id));
    }

    #[test]
    fn movie_title_year() {
        assert_eq!(movie_year("Inception 2010"), Some(2010));
        assert_eq!(strip_year("Inception (2010)"), "Inception");
        assert_eq!(strip_year("2012"), "2012");
    }

    #[test]
    fn jobs_and_due() {
        let conn = crate::db::open_memory().unwrap();
        init(&conn).unwrap();
        assert!(due(None, ("series", 1), false, "A", None, None, None, 0));
        assert!(!due(None, ("series", 1), true, "A", Some("locked"), Some("A"), Some(0), MATCH_RULES));
        assert!(!due(None, ("series", 1), false, "A", Some("auto"), Some("A"), Some(0), MATCH_RULES));
        assert!(due(None, ("series", 1), false, "B", Some("auto"), Some("A"), Some(0), MATCH_RULES));
        // 匹配规则改进后，自动配的重新匹配一次，手动选的、关掉的不动
        assert!(due(None, ("series", 1), false, "A", Some("auto"), Some("A"), Some(0), 0));
        assert!(!due(None, ("series", 1), false, "A", Some("locked"), Some("A"), Some(0), 0));
        assert!(!due(None, ("series", 1), false, "A", Some("off"), Some("A"), Some(0), 0));
        assert!(jobs(&conn, None, false).unwrap().is_empty());
    }
}
