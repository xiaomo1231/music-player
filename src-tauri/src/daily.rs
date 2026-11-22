//! 每日推荐：本地算法（模式一）+ 可选 AI 点评
//! 数据来源：play_history（播放记录）、tracks（播放次数、跳过次数）、feedback（反馈）；结果按天存在 daily 表，保留最近 7 天。
use crate::{ai, db, library, AppState};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Manager, State};

const DAY: i64 = 86_400;
/// 库里少于这么多首歌时不出推荐
const MIN_LIBRARY: usize = 10;
/// 每次推荐的目标首数
const TARGET: usize = 30;
/// 保留的天数（含今天）
const KEEP_DAYS: i64 = 7;
/// 候选类别：相似 / 旧爱 / 没听过 / 探索，顺序也是名额不够时的补位优先级
const KINDS: [&str; 4] = ["sim", "old", "never", "explore"];
const FEEDBACK_KINDS: [&str; 4] = ["like", "not_song", "not_artist", "not_today"];
const SHARE: [f64; 4] = [0.4, 0.3, 0.2, 0.1];

/// 启动时建表（daily 表 db.rs 已建，这里是幂等保险）
pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS daily(day TEXT PRIMARY KEY, data TEXT NOT NULL);")?;
    Ok(())
}

/// 启动时不预先生成：每天第一次请求（daily_get）时才生成
pub fn setup(_app: &AppHandle) {}

// ---------- 数据结构（前端也用这些字段） ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Item {
    /// 文件 id（同 Track.id）
    pub id: i64,
    pub reason: String,
    /// sim / old / never / explore
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TopTrack {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TopArtist {
    pub name: String,
    pub count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Stats {
    /// 昨天听过的歌（按次数，最多 10 首，不含跳过）
    pub yesterday: Vec<TopTrack>,
    pub yesterday_plays: i64,
    /// 昨天的总播放时长（秒）
    pub yesterday_seconds: f64,
    pub skipped_yesterday: i64,
    pub top_tracks_7d: Vec<TopTrack>,
    pub top_artists_7d: Vec<TopArtist>,
    pub top_tracks_30d: Vec<TopTrack>,
    pub top_artists_30d: Vec<TopArtist>,
    pub plays_7d: i64,
    pub skipped_7d: i64,
    /// 最近 7 天的总收听时长（秒）
    pub listen_seconds_7d: f64,
    /// 近 30 天每个小时的播放次数（本地时间，长度 24）
    pub hours: Vec<i64>,
    /// 最常听的时段，例如 "深夜 22 点左右"
    pub peak_label: Option<String>,
    pub library_tracks: i64,
}

/// 一天的推荐（存进 daily 表的 data，也是 daily_get 的返回值）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Doc {
    /// YYYY-MM-DD（本地日期）
    pub day: String,
    pub title: String,
    pub summary: String,
    pub tracks: Vec<Item>,
    pub stats: Stats,
    /// 是否经过 AI 点评
    pub ai: bool,
    pub generated_at: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct HistoryItem {
    pub day: String,
    pub title: String,
    pub summary: String,
    pub count: usize,
}

// ---------- 日期与时区（不引入日期库；Windows 上用 GetLocalTime 取本地时间） ----------

/// now 是 UTC 秒；offset 是本地时间比 UTC 快多少秒（中国为 28800）
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: i64,
    pub offset: i64,
}

impl Clock {
    pub fn current() -> Clock {
        let now = db::now();
        Clock {
            now,
            offset: local_offset(now),
        }
    }
    pub fn day_of(&self, ts: i64) -> String {
        civil_day((ts + self.offset).div_euclid(DAY))
    }
    pub fn hour_of(&self, ts: i64) -> usize {
        ((ts + self.offset).rem_euclid(DAY) / 3600) as usize
    }
    pub fn today(&self) -> String {
        self.day_of(self.now)
    }
}

#[cfg(windows)]
fn local_offset(now: i64) -> i64 {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(t: *mut SystemTime);
    }
    let mut t = SystemTime::default();
    unsafe { GetLocalTime(&mut t) };
    let local = days_from_civil(t.year as i64, t.month as i64, t.day as i64) * DAY + t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64;
    // 四舍五入到分钟，抵消两次读时间之间的误差
    (local - now + 30).div_euclid(60) * 60
}

#[cfg(not(windows))]
fn local_offset(_now: i64) -> i64 {
    0
}

/// 公历日期 → 从 1970-01-01 起的天数
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12; // 3 月为 0
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 从 1970-01-01 起的天数 → (年, 月, 日)
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn civil_day(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// 解析 YYYY-MM-DD，只接受规范写法
pub fn parse_day(s: &str) -> Option<i64> {
    let p: Vec<&str> = s.split('-').collect();
    if p.len() != 3 {
        return None;
    }
    let y = p[0].parse::<i64>().ok()?;
    let m = p[1].parse::<i64>().ok()?;
    let d = p[2].parse::<i64>().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    (civil_day(days) == s).then_some(days)
}

pub fn add_days(day: &str, n: i64) -> String {
    match parse_day(day) {
        Some(d) => civil_day(d + n),
        None => day.to_string(),
    }
}

fn period(h: usize) -> &'static str {
    match h {
        0..=4 => "凌晨",
        5..=8 => "早上",
        9..=11 => "上午",
        12..=13 => "中午",
        14..=17 => "下午",
        18..=21 => "晚上",
        _ => "深夜",
    }
}

// ---------- 随机数（不引入 rand；给一个种子即可复现） ----------

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// (0, 1) 之间的数
    pub fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn seed_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

// ---------- 候选与分组 ----------

/// 艺人的归类名：优先用 artist，没有就用专辑艺人，都没有返回空串（空串不参与“同一艺人”判断）
pub fn artist_key(artist: &str, album_artist: &str) -> String {
    let a = artist.trim();
    if !a.is_empty() {
        a.to_string()
    } else {
        album_artist.trim().to_string()
    }
}

#[derive(Clone, Debug)]
pub struct Cand {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub play_count: i64,
    pub last_played: Option<i64>,
    pub added_at: i64,
    /// 权重：常被跳过的歌是 0.3，其余 1.0
    pub weight: f64,
}

fn pool_of(c: &Cand, top_artists: &[String], top_genres: &[String], now: i64) -> usize {
    let same_artist = !c.artist.is_empty() && top_artists.contains(&c.artist);
    let same_genre = !c.genre.is_empty() && top_genres.contains(&c.genre);
    if same_artist || same_genre {
        return 0;
    }
    if c.play_count >= 3 && c.last_played.map_or(true, |t| now - t >= 90 * DAY) {
        return 1;
    }
    if c.play_count == 0 && c.last_played.is_none() {
        return 2;
    }
    3
}

/// 按比例分配名额（相似 40% / 旧爱 30% / 没听过 20% / 探索 10%）。
/// 某一类候选不够时，剩下的名额按优先级补给还有候选的类别，总数不超过 n 和候选总数。
pub fn allocate(n: usize, sizes: [usize; 4]) -> [usize; 4] {
    let n = n.min(sizes.iter().sum());
    let mut take = [0usize; 4];
    for i in 0..3 {
        take[i] = ((n as f64 * SHARE[i]).round() as usize).min(sizes[i]);
    }
    let first: usize = take[..3].iter().sum();
    take[3] = n.saturating_sub(first).min(sizes[3]);
    let mut used: usize = take.iter().sum();
    while used < n {
        let mut progressed = false;
        for i in 0..4 {
            if used >= n {
                break;
            }
            if take[i] < sizes[i] {
                take[i] += 1;
                used += 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    take
}

fn gap_text(sec: i64) -> String {
    let days = sec / DAY;
    if days >= 365 {
        "快一年".into()
    } else if days >= 60 {
        format!("{} 个多月", days / 30)
    } else {
        format!("{days} 天")
    }
}

fn reason(c: &Cand, kind: usize, top_artists: &[String], now: i64) -> String {
    match kind {
        0 => {
            if top_artists.contains(&c.artist) {
                format!("和你最近常听的{}是同一位艺人", c.artist)
            } else {
                format!("和你最近常听的{}风格相近", c.genre)
            }
        }
        1 => match c.last_played {
            Some(t) => format!("以前听过 {} 次，{}没听了", c.play_count, gap_text(now - t)),
            None => format!("以前听过 {} 次，很久没听了", c.play_count),
        },
        2 => {
            let days = (now - c.added_at) / DAY;
            if days <= 0 {
                "刚入库，还没听过".into()
            } else {
                format!("入库 {days} 天，还没听过")
            }
        }
        _ => {
            if c.genre.is_empty() {
                "换换口味：平时听得比较少的一首".into()
            } else {
                format!("换换口味：{}类型，平时听得比较少", c.genre)
            }
        }
    }
}

struct Pick {
    item: Item,
    artist: String,
}

/// 同一艺人尽量不连续：每次从“和上一首不同艺人”的歌里，取剩得最多的那个艺人的第一首。
/// 这样不会把某个艺人的歌全堆到最后；只有在数量实在不够时才会相邻。
fn arrange(mut rest: Vec<Pick>) -> Vec<Pick> {
    let mut left: HashMap<String, usize> = HashMap::new();
    for p in &rest {
        if !p.artist.is_empty() {
            *left.entry(p.artist.clone()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<Pick> = Vec::with_capacity(rest.len());
    while !rest.is_empty() {
        let last = out.last().map(|p| p.artist.clone()).unwrap_or_default();
        let idx = rest
            .iter()
            .enumerate()
            .filter(|(_, p)| p.artist.is_empty() || p.artist != last)
            .max_by(|(ia, a), (ib, b)| {
                let ca = left.get(&a.artist).copied().unwrap_or(0);
                let cb = left.get(&b.artist).copied().unwrap_or(0);
                ca.cmp(&cb).then(ib.cmp(ia))
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        let p = rest.remove(idx);
        if let Some(c) = left.get_mut(&p.artist) {
            *c -= 1;
        }
        out.push(p);
    }
    out
}

/// 纯函数：从候选里按比例挑出 n 首，附推荐理由，同一艺人不连续
pub fn pick(cands: &[Cand], top_artists: &[String], top_genres: &[String], now: i64, n: usize, rng: &mut Rng) -> Vec<Item> {
    let mut pools: [Vec<&Cand>; 4] = Default::default();
    for c in cands {
        pools[pool_of(c, top_artists, top_genres, now)].push(c);
    }
    let sizes = [pools[0].len(), pools[1].len(), pools[2].len(), pools[3].len()];
    let take = allocate(n, sizes);
    let mut out: Vec<Pick> = Vec::new();
    for (k, pool) in pools.iter().enumerate() {
        // 按权重随机排序：常被跳过的歌排在后面
        let mut keyed: Vec<(f64, &Cand)> = pool.iter().map(|c| (rng.unit() / c.weight.max(0.01), *c)).collect();
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.id.cmp(&b.1.id)));
        for (_, c) in keyed.into_iter().take(take[k]) {
            out.push(Pick {
                item: Item {
                    id: c.id,
                    reason: reason(c, k, top_artists, now),
                    kind: KINDS[k].to_string(),
                },
                artist: c.artist.clone(),
            });
        }
    }
    arrange(out).into_iter().map(|p| p.item).collect()
}

// ---------- 播放记录与统计 ----------

struct Play {
    id: i64,
    at: i64,
    listened: f64,
    skipped: bool,
    title: String,
    artist: String,
    genre: String,
}

fn load_plays(conn: &Connection, since: i64) -> Result<Vec<Play>> {
    let mut st = conn.prepare(
        "SELECT h.file_id, h.at, h.listened, h.skipped, t.title, t.artist, t.album_artist, t.genre
         FROM play_history h JOIN tracks t ON t.file_id=h.file_id JOIN files f ON f.id=h.file_id
         WHERE h.at>=?1 AND f.missing=0 ORDER BY h.at",
    )?;
    let rows = st.query_map([since], |r| {
        let artist: String = r.get(5)?;
        let album_artist: String = r.get(6)?;
        Ok(Play {
            id: r.get(0)?,
            at: r.get(1)?,
            listened: r.get(2)?,
            skipped: r.get::<_, i64>(3)? != 0,
            title: r.get(4)?,
            artist: artist_key(&artist, &album_artist),
            genre: r.get::<_, String>(7)?.trim().to_string(),
        })
    })?;
    let v: Vec<Play> = rows.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// 反馈：不感兴趣的歌和艺人要排除，喜欢的艺人在相似推荐里加分
#[derive(Default)]
struct Feedback {
    not_song: HashSet<i64>,
    not_today: HashSet<i64>,
    not_artist: HashSet<String>,
    liked_artists: HashMap<String, i64>,
}

fn load_feedback(conn: &Connection, now: i64) -> Result<Feedback> {
    let mut fb = Feedback::default();
    let mut st = conn.prepare("SELECT f.file_id, f.kind, f.at, t.artist, t.album_artist FROM feedback f LEFT JOIN tracks t ON t.file_id=f.file_id")?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        ))
    })?;
    for row in rows {
        let (id, kind, at, artist, album_artist) = row?;
        let key = artist_key(&artist, &album_artist);
        match kind.as_str() {
            "not_song" => {
                fb.not_song.insert(id);
            }
            // “今天不想听这种”：这首 3 天内不再推
            "not_today" if now - at < 3 * DAY => {
                fb.not_today.insert(id);
            }
            "not_artist" if !key.is_empty() => {
                fb.not_artist.insert(key);
            }
            "like" if !key.is_empty() && now - at < 90 * DAY => {
                *fb.liked_artists.entry(key).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    Ok(fb)
}

fn rank_tracks<'a>(plays: impl Iterator<Item = &'a Play>, limit: usize) -> Vec<TopTrack> {
    let mut m: HashMap<i64, TopTrack> = HashMap::new();
    for p in plays.filter(|p| !p.skipped) {
        let e = m.entry(p.id).or_insert_with(|| TopTrack {
            id: p.id,
            title: p.title.clone(),
            artist: p.artist.clone(),
            count: 0,
        });
        e.count += 1;
    }
    let mut v: Vec<TopTrack> = m.into_values().collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.title.cmp(&b.title)));
    v.truncate(limit);
    v
}

fn rank_artists<'a>(plays: impl Iterator<Item = &'a Play>, limit: usize) -> Vec<TopArtist> {
    let mut m: HashMap<String, i64> = HashMap::new();
    for p in plays.filter(|p| !p.skipped && !p.artist.is_empty()) {
        *m.entry(p.artist.clone()).or_insert(0) += 1;
    }
    let mut v: Vec<TopArtist> = m.into_iter().map(|(name, count)| TopArtist { name, count }).collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    v.truncate(limit);
    v
}

/// 统计：昨天、7 天、30 天、时段、总时长
fn stats(plays: &[Play], clock: Clock, yesterday: &str, library_tracks: usize) -> Stats {
    let now = clock.now;
    let mut s = Stats {
        library_tracks: library_tracks as i64,
        hours: vec![0; 24],
        ..Default::default()
    };
    for p in plays {
        let age = now - p.at;
        if clock.day_of(p.at) == yesterday {
            if p.skipped {
                s.skipped_yesterday += 1;
            } else {
                s.yesterday_plays += 1;
                s.yesterday_seconds += p.listened;
            }
        }
        if age < 7 * DAY {
            if p.skipped {
                s.skipped_7d += 1;
            } else {
                s.plays_7d += 1;
                s.listen_seconds_7d += p.listened;
            }
        }
        if age < 30 * DAY && !p.skipped {
            s.hours[clock.hour_of(p.at)] += 1;
        }
    }
    s.yesterday = rank_tracks(plays.iter().filter(|p| clock.day_of(p.at) == yesterday), 10);
    s.top_tracks_7d = rank_tracks(plays.iter().filter(|p| now - p.at < 7 * DAY), 5);
    s.top_artists_7d = rank_artists(plays.iter().filter(|p| now - p.at < 7 * DAY), 5);
    s.top_tracks_30d = rank_tracks(plays.iter().filter(|p| now - p.at < 30 * DAY), 15);
    s.top_artists_30d = rank_artists(plays.iter().filter(|p| now - p.at < 30 * DAY), 8);
    let best = s.hours.iter().enumerate().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(&a.0)));
    if let Some((h, &c)) = best {
        if c > 0 {
            s.peak_label = Some(format!("{} {h} 点左右", period(h)));
        }
    }
    s
}

/// 最近常听的艺人（没有最近记录时用 30 天；喜欢的艺人加分；不喜欢的艺人不参与）
fn top_artists(base: &[&Play], liked: &HashMap<String, i64>, excluded: &HashSet<String>) -> Vec<String> {
    let mut score: HashMap<String, f64> = HashMap::new();
    for p in base.iter().filter(|p| !p.skipped && !p.artist.is_empty()) {
        *score.entry(p.artist.clone()).or_insert(0.0) += 1.0;
    }
    for (a, n) in liked {
        *score.entry(a.clone()).or_insert(0.0) += 2.0 * *n as f64;
    }
    let mut v: Vec<(String, f64)> = score.into_iter().filter(|(a, _)| !excluded.contains(a)).collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    v.into_iter().take(5).map(|x| x.0).collect()
}

fn top_genres(base: &[&Play]) -> Vec<String> {
    let mut m: HashMap<&str, i64> = HashMap::new();
    for p in base.iter().filter(|p| !p.skipped && !p.genre.is_empty()) {
        *m.entry(p.genre.as_str()).or_insert(0) += 1;
    }
    let mut v: Vec<(&str, i64)> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    v.into_iter().take(3).map(|x| x.0.to_string()).collect()
}

fn summary(s: &Stats, items: &[Item]) -> String {
    let count = |k: &str| items.iter().filter(|i| i.kind == k).count();
    let head = if s.yesterday_plays > 0 {
        format!("昨天你听了 {} 首，约 {} 分钟。", s.yesterday_plays, (s.yesterday_seconds / 60.0).round() as i64)
    } else {
        "昨天没有播放记录。".to_string()
    };
    format!(
        "{head}今天从库里挑了 {} 首：{} 首和最近常听的相似，{} 首是旧爱，{} 首还没听过，{} 首换换口味。",
        items.len(),
        count("sim"),
        count("old"),
        count("never"),
        count("explore")
    )
}

/// 生成一天的推荐（纯计算，不写库）。exclude 是要避开的文件 id（昨天推过的、换一批时本批的）
fn build(conn: &Connection, clock: Clock, day: &str, seed: u64, exclude: &HashSet<i64>) -> Result<Doc> {
    let now = clock.now;
    let tracks = library::tracks(conn)?;
    let mut doc = Doc {
        day: day.to_string(),
        title: "今日推荐".into(),
        generated_at: now,
        ..Default::default()
    };
    if tracks.len() < MIN_LIBRARY {
        doc.stats.library_tracks = tracks.len() as i64;
        doc.summary = format!("库里只有 {} 首歌，至少要 {MIN_LIBRARY} 首才能生成推荐。先多添加一些音乐吧。", tracks.len());
        return Ok(doc);
    }
    let plays = load_plays(conn, now - 90 * DAY)?;
    let fb = load_feedback(conn, now)?;
    doc.stats = stats(&plays, clock, &add_days(day, -1), tracks.len());

    let p7: Vec<&Play> = plays.iter().filter(|p| now - p.at < 7 * DAY).collect();
    let p30: Vec<&Play> = plays.iter().filter(|p| now - p.at < 30 * DAY).collect();
    let base: Vec<&Play> = if p7.iter().any(|p| !p.skipped) { p7 } else { p30 };
    let top_a = top_artists(&base, &fb.liked_artists, &fb.not_artist);
    let top_g = top_genres(&base);

    // 90 天内每首歌的（跳过次数, 总次数）
    let mut skip90: HashMap<i64, (i64, i64)> = HashMap::new();
    for p in &plays {
        let e = skip90.entry(p.id).or_insert((0, 0));
        e.1 += 1;
        if p.skipped {
            e.0 += 1;
        }
    }

    let cands: Vec<Cand> = tracks
        .iter()
        .filter_map(|t| {
            let key = artist_key(&t.artist, &t.album_artist);
            if fb.not_song.contains(&t.id) || fb.not_today.contains(&t.id) || exclude.contains(&t.id) {
                return None;
            }
            if !key.is_empty() && fb.not_artist.contains(&key) {
                return None;
            }
            let (sk, tot) = skip90.get(&t.id).copied().unwrap_or((0, 0));
            // 经常被跳过的（累计跳过 ≥ 3 次且超过播放次数的两倍，或近 90 天一半以上被跳过）降权
            let heavy = (t.skip_count >= 3 && t.skip_count > t.play_count * 2) || (sk >= 3 && sk * 2 >= tot);
            Some(Cand {
                id: t.id,
                title: t.title.clone(),
                artist: key,
                genre: t.genre.trim().to_string(),
                play_count: t.play_count,
                last_played: t.last_played,
                added_at: t.added_at,
                weight: if heavy { 0.3 } else { 1.0 },
            })
        })
        .collect();

    let mut rng = Rng::new(seed ^ hash_str(day));
    doc.tracks = pick(&cands, &top_a, &top_g, now, TARGET, &mut rng);
    doc.summary = summary(&doc.stats, &doc.tracks);
    Ok(doc)
}

// ---------- 存取 ----------

fn load_doc(conn: &Connection, day: &str) -> Result<Option<Doc>> {
    let data: Option<String> = conn.query_row("SELECT data FROM daily WHERE day=?1", [day], |r| r.get(0)).optional()?;
    Ok(data.and_then(|s| serde_json::from_str(&s).ok()))
}

fn save_doc(conn: &Connection, doc: &Doc) -> Result<()> {
    conn.execute(
        "INSERT INTO daily(day,data) VALUES(?1,?2) ON CONFLICT(day) DO UPDATE SET data=excluded.data",
        params![doc.day, serde_json::to_string(doc)?],
    )?;
    Ok(())
}

/// 只保留最近 7 天（含今天）
fn prune(conn: &Connection, today: &str) -> Result<()> {
    conn.execute("DELETE FROM daily WHERE day < ?1", [add_days(today, -(KEEP_DAYS - 1))])?;
    Ok(())
}

/// 生成并保存今天的推荐。reroll 为 true 时（换一批）避开本批已有的歌
fn generate(conn: &Connection, clock: Clock, reroll: bool) -> Result<Doc> {
    let day = clock.today();
    let mut exclude: HashSet<i64> = HashSet::new();
    if let Some(prev) = load_doc(conn, &add_days(&day, -1))? {
        exclude.extend(prev.tracks.iter().map(|i| i.id));
    }
    if reroll {
        if let Some(cur) = load_doc(conn, &day)? {
            exclude.extend(cur.tracks.iter().map(|i| i.id));
        }
    }
    let mut doc = build(conn, clock, &day, seed_now(), &exclude)?;
    // 库太小、昨天的歌把候选全占了：放宽，允许重复，总比空着好
    if doc.tracks.is_empty() && !exclude.is_empty() && doc.stats.library_tracks >= MIN_LIBRARY as i64 {
        doc = build(conn, clock, &day, seed_now(), &HashSet::new())?;
    }
    if doc.stats.library_tracks >= MIN_LIBRARY as i64 {
        save_doc(conn, &doc)?;
        prune(conn, &day)?;
    }
    Ok(doc)
}

// ---------- AI 点评（可选） ----------

const AI_SYSTEM: &str = "你是音乐推荐编辑。只根据给出的播放统计和歌曲信息写作，不要编造不存在的歌曲。只返回 JSON。";

fn ai_prompt(doc: &Doc, rows: &[(String, String)]) -> String {
    let mut s = String::from("播放统计（最近 30 天，只有歌名、艺人和次数）：\n");
    for t in doc.stats.top_tracks_30d.iter().take(15) {
        s += &format!("- 《{}》 {} · {} 次\n", t.title, t.artist, t.count);
    }
    s += "常听的艺人：\n";
    for a in doc.stats.top_artists_30d.iter().take(8) {
        s += &format!("- {} · {} 次\n", a.name, a.count);
    }
    s += "\n今天的推荐（序号 / 歌名 / 艺人 / 来源）：\n";
    for (i, it) in doc.tracks.iter().enumerate() {
        let (title, artist) = rows.get(i).cloned().unwrap_or_default();
        s += &format!("{i}. 《{title}》 {artist} / {}\n", kind_label(&it.kind));
    }
    s += "\n请只返回 JSON：{\"name\": \"歌单名（不超过 8 个字）\", \"summary\": \"一句话总结（不超过 60 个字）\", \"reasons\": [{\"index\": 0, \"reason\": \"这首歌的推荐理由（不超过 30 个字）\"}]}";
    s
}

fn kind_label(k: &str) -> &'static str {
    match k {
        "sim" => "相似",
        "old" => "旧爱",
        "never" => "没听过",
        _ => "探索",
    }
}

fn apply_ai(doc: &mut Doc, v: &serde_json::Value) {
    if let Some(n) = v["name"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        doc.title = n.chars().take(20).collect();
    }
    if let Some(s) = v["summary"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        doc.summary = s.to_string();
    }
    if let Some(list) = v["reasons"].as_array() {
        for r in list {
            let (Some(i), Some(text)) = (r["index"].as_u64(), r["reason"].as_str()) else {
                continue;
            };
            let text = text.trim();
            if let Some(item) = doc.tracks.get_mut(i as usize) {
                if !text.is_empty() {
                    item.reason = text.to_string();
                }
            }
        }
    }
    doc.ai = true;
}

// ---------- 命令 ----------

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

fn default_playlist_name(day: &str) -> String {
    match parse_day(day) {
        Some(d) => {
            let (_, m, dd) = civil_from_days(d);
            format!("每日推荐 {m}-{dd}")
        }
        None => "每日推荐".into(),
    }
}

/// 某天的推荐。day 缺省为今天；今天第一次请求时生成
#[tauri::command]
pub fn daily_get(st: State<AppState>, day: Option<String>) -> Result<Doc, String> {
    let conn = st.db.lock();
    let clock = Clock::current();
    let today = clock.today();
    let day = day.unwrap_or_else(|| today.clone());
    if parse_day(&day).is_none() {
        return Err("日期格式应为 2024-05-01".into());
    }
    if let Some(doc) = load_doc(&conn, &day).map_err(e)? {
        return Ok(doc);
    }
    if day == today {
        return generate(&conn, clock, false).map_err(e);
    }
    Err(format!("{day} 没有每日推荐记录"))
}

/// 换一批：重新挑今天的歌，避开昨天和本批已有的
#[tauri::command]
pub fn daily_regenerate(st: State<AppState>) -> Result<Doc, String> {
    let conn = st.db.lock();
    generate(&conn, Clock::current(), true).map_err(e)
}

/// 反馈：like / not_song / not_artist / not_today
#[tauri::command]
pub fn daily_feedback(st: State<AppState>, file_id: i64, kind: String) -> Result<(), String> {
    if !FEEDBACK_KINDS.contains(&kind.as_str()) {
        return Err("未知的反馈类型".into());
    }
    st.db
        .lock()
        .execute("INSERT INTO feedback(file_id,kind,at) VALUES(?1,?2,?3)", params![file_id, kind, db::now()])
        .map_err(e)?;
    Ok(())
}

/// 最近 7 天的推荐记录（新到旧）
#[tauri::command]
pub fn daily_history(st: State<AppState>) -> Result<Vec<HistoryItem>, String> {
    let conn = st.db.lock();
    let mut s = conn.prepare("SELECT data FROM daily ORDER BY day DESC LIMIT 7").map_err(e)?;
    let rows: Vec<String> = s.query_map([], |r| r.get(0)).map_err(e)?.collect::<rusqlite::Result<_>>().map_err(e)?;
    Ok(rows
        .iter()
        .filter_map(|j| serde_json::from_str::<Doc>(j).ok())
        .map(|d| HistoryItem {
            count: d.tracks.len(),
            day: d.day,
            title: d.title,
            summary: d.summary,
        })
        .collect())
}

/// 把某天的推荐存成普通歌单，返回歌单 id。name 为空时用“每日推荐 M-D”
#[tauri::command]
pub fn daily_save_playlist(st: State<AppState>, day: String, name: String) -> Result<i64, String> {
    if parse_day(&day).is_none() {
        return Err("日期格式应为 2024-05-01".into());
    }
    let conn = st.db.lock();
    let doc = load_doc(&conn, &day).map_err(e)?.ok_or_else(|| format!("{day} 没有推荐记录"))?;
    if doc.tracks.is_empty() {
        return Err("这一天没有推荐的歌".into());
    }
    let name = match name.trim() {
        "" => default_playlist_name(&day),
        n => n.to_string(),
    };
    let id = library::create_playlist(&conn, &name).map_err(e)?;
    let ids: Vec<i64> = doc.tracks.iter().map(|i| i.id).collect();
    library::playlist_add(&conn, id, &ids).map_err(e)?;
    Ok(id)
}

/// AI 点评：给歌单起名、写总结和每首理由。只发送歌名、艺人、次数。没有配置 AI 时返回错误
#[tauri::command]
pub async fn daily_ai_enhance(app: AppHandle, day: String) -> Result<Doc, String> {
    if parse_day(&day).is_none() {
        return Err("日期格式应为 2024-05-01".into());
    }
    ai::ready(&app).map_err(e)?;
    let (doc, prompt) = {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        let doc = load_doc(&conn, &day).map_err(e)?.ok_or_else(|| format!("{day} 没有推荐记录"))?;
        let rows: Vec<(String, String)> = doc
            .tracks
            .iter()
            .map(|i| library::track(&conn, i.id).ok().flatten().map(|t| (t.title, t.artist)).unwrap_or_default())
            .collect();
        let prompt = ai_prompt(&doc, &rows);
        (doc, prompt)
    };
    let reply = ai::chat_simple(&app, AI_SYSTEM, &prompt, true).await.map_err(e)?;
    let v = ai::extract_json(&reply).map_err(e)?;
    let mut doc = doc;
    apply_ai(&mut doc, &v);
    let st = app.state::<AppState>();
    let conn = st.db.lock();
    save_doc(&conn, &doc).map_err(e)?;
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn clock() -> Clock {
        Clock { now: NOW, offset: 8 * 3600 }
    }

    /// 造 artists × per 首歌，艺人名 艺人0、艺人1……
    fn seed_lib(conn: &Connection, artists: usize, per: usize) -> Vec<i64> {
        let mut ids = vec![];
        for a in 0..artists {
            for k in 0..per {
                conn.execute(
                    "INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,'music',1,1,?2)",
                    params![format!("C:\\music\\a{a}_{k}.flac"), NOW - 200 * DAY],
                )
                .unwrap();
                let id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT INTO tracks(file_id,title,artist,album,genre,duration,format) VALUES(?1,?2,?3,'专辑','',200,'FLAC')",
                    params![id, format!("歌{a}-{k}"), format!("艺人{a}")],
                )
                .unwrap();
                ids.push(id);
            }
        }
        ids
    }

    fn artist_of(conn: &Connection, id: i64) -> String {
        conn.query_row("SELECT artist FROM tracks WHERE file_id=?1", [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn civil_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_day(19723), "2024-01-01");
        assert_eq!(parse_day("2024-02-30"), None);
        assert_eq!(parse_day("2024-2-3"), None);
        assert_eq!(add_days("2024-03-01", -1), "2024-02-29");
        assert_eq!(add_days("2024-01-01", -1), "2023-12-31");
        let c = Clock { now: 0, offset: 8 * 3600 };
        assert_eq!(c.hour_of(0), 8);
        assert_eq!(c.day_of(0), "1970-01-01");
        assert_eq!(c.day_of(16 * 3600), "1970-01-02");
    }

    #[test]
    fn allocate_follows_shares() {
        assert_eq!(allocate(30, [100, 100, 100, 100]), [12, 9, 6, 3]);
        // 没听过的只有 2 首：缺的名额补给探索（优先级靠后的）
        let t = allocate(30, [100, 100, 2, 100]);
        assert_eq!(t[2], 2);
        assert_eq!(t.iter().sum::<usize>(), 30);
        // 候选不够时总数不超过候选总数
        assert_eq!(allocate(30, [3, 0, 0, 0]), [3, 0, 0, 0]);
        assert_eq!(allocate(30, [0, 0, 0, 0]), [0, 0, 0, 0]);
    }

    #[test]
    fn same_artist_not_adjacent() {
        let cands: Vec<Cand> = (0..40)
            .map(|i| Cand {
                id: i,
                title: format!("t{i}"),
                artist: format!("艺人{}", i % 4),
                genre: String::new(),
                play_count: 0,
                last_played: None,
                added_at: NOW - 100 * DAY,
                weight: 1.0,
            })
            .collect();
        let mut rng = Rng::new(7);
        let items = pick(&cands, &[], &[], NOW, 30, &mut rng);
        assert_eq!(items.len(), 30);
        let artist = |id: i64| format!("艺人{}", id % 4);
        for w in items.windows(2) {
            assert_ne!(artist(w[0].id), artist(w[1].id));
        }
    }

    #[test]
    fn arrange_separates_neighbours() {
        let mk = |a: &str| Pick {
            item: Item {
                id: 0,
                reason: String::new(),
                kind: "explore".into(),
            },
            artist: a.into(),
        };
        let out = arrange(vec![mk("A"), mk("A"), mk("B"), mk("B")]);
        let arts: Vec<&str> = out.iter().map(|p| p.artist.as_str()).collect();
        assert_eq!(arts, vec!["A", "B", "A", "B"]);
    }

    #[test]
    fn excludes_yesterday_and_keeps_seven_days() {
        let conn = db::open_memory().unwrap();
        let ids = seed_lib(&conn, 12, 5);
        let c = clock();
        let today = c.today();
        let yesterday = add_days(&today, -1);
        // 昨天推过前 10 首
        let old = Doc {
            day: yesterday.clone(),
            tracks: ids[..10]
                .iter()
                .map(|&id| Item {
                    id,
                    reason: String::new(),
                    kind: "explore".into(),
                })
                .collect(),
            ..Default::default()
        };
        save_doc(&conn, &old).unwrap();
        // 8 天前的记录应被清理，3 天前的保留
        save_doc(
            &conn,
            &Doc {
                day: add_days(&today, -8),
                ..Default::default()
            },
        )
        .unwrap();
        save_doc(
            &conn,
            &Doc {
                day: add_days(&today, -3),
                ..Default::default()
            },
        )
        .unwrap();

        let doc = generate(&conn, c, false).unwrap();
        assert_eq!(doc.day, today);
        assert_eq!(doc.tracks.len(), TARGET);
        assert!(doc.tracks.iter().all(|i| !ids[..10].contains(&i.id)));
        assert!(load_doc(&conn, &add_days(&today, -8)).unwrap().is_none());
        assert!(load_doc(&conn, &add_days(&today, -3)).unwrap().is_some());
        // 同一天再次请求：读到的就是保存的那一份
        assert_eq!(load_doc(&conn, &today).unwrap().unwrap().tracks, doc.tracks);
    }

    #[test]
    fn feedback_excludes_song_and_artist() {
        let conn = db::open_memory().unwrap();
        let ids = seed_lib(&conn, 12, 5);
        conn.execute("INSERT INTO feedback(file_id,kind,at) VALUES(?1,'not_song',?2)", params![ids[0], NOW])
            .unwrap();
        conn.execute("INSERT INTO feedback(file_id,kind,at) VALUES(?1,'not_artist',?2)", params![ids[5], NOW])
            .unwrap();
        let doc = build(&conn, clock(), &clock().today(), 42, &HashSet::new()).unwrap();
        assert!(doc.tracks.iter().all(|i| i.id != ids[0]));
        assert!(doc.tracks.iter().all(|i| artist_of(&conn, i.id) != "艺人1"));
    }

    #[test]
    fn small_library_gets_empty_doc_not_stored() {
        let conn = db::open_memory().unwrap();
        seed_lib(&conn, 2, 3);
        let doc = generate(&conn, clock(), false).unwrap();
        assert!(doc.tracks.is_empty());
        assert!(doc.summary.contains("至少要"));
        assert!(load_doc(&conn, &clock().today()).unwrap().is_none());
    }

    #[test]
    fn stats_count_yesterday_and_hours() {
        let conn = db::open_memory().unwrap();
        let ids = seed_lib(&conn, 12, 5);
        let c = clock();
        let yesterday_noon = c.now - DAY; // 只要落在“昨天”这个本地日期里即可
        let y = c.day_of(yesterday_noon);
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO play_history(file_id,at,listened,duration,skipped,source) VALUES(?1,?2,200,200,0,NULL)",
                params![ids[0], yesterday_noon],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO play_history(file_id,at,listened,duration,skipped,source) VALUES(?1,?2,3,200,1,NULL)",
            params![ids[1], yesterday_noon],
        )
        .unwrap();
        let plays = load_plays(&conn, NOW - 90 * DAY).unwrap();
        let s = stats(&plays, c, &y, 60);
        assert_eq!(s.yesterday_plays, 3);
        assert_eq!(s.skipped_yesterday, 1);
        assert_eq!(s.yesterday[0].count, 3);
        assert_eq!(s.hours.len(), 24);
        assert_eq!(s.hours.iter().sum::<i64>(), 3);
        assert!(s.peak_label.is_some());
    }

    #[test]
    fn apply_ai_keeps_items_and_sets_flag() {
        let mut doc = Doc {
            tracks: vec![Item {
                id: 1,
                reason: "旧理由".into(),
                kind: "sim".into(),
            }],
            ..Default::default()
        };
        let v = serde_json::json!({"name": "雨后慢歌", "summary": "慢慢听", "reasons": [{"index": 0, "reason": "新理由"}, {"index": 9, "reason": "无效"}]});
        apply_ai(&mut doc, &v);
        assert_eq!(doc.title, "雨后慢歌");
        assert_eq!(doc.tracks[0].reason, "新理由");
        assert!(doc.ai);
    }
}
