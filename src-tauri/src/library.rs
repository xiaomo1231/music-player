//! 媒体库查询（给前端用）
use crate::{db, indexer};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Clone, Debug)]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<i64>,
    pub disc_no: Option<i64>,
    pub year: Option<i64>,
    pub genre: String,
    pub duration: f64,
    pub format: String,
    pub lossless: bool,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub bit_depth: Option<i64>,
    pub cover: Option<String>,
    pub hue: Option<i64>,
    pub play_count: i64,
    pub skip_count: i64,
    pub last_played: Option<i64>,
    pub favorite: bool,
    pub added_at: i64,
    pub encrypted: bool,
    pub size: i64,
}

const TRACK_COLS: &str = "f.id,f.path,coalesce(f.custom_title,t.title),t.artist,t.album,t.album_artist,t.track_no,t.disc_no,t.year,t.genre,t.duration,t.format,t.lossless,t.bitrate,t.sample_rate,t.bit_depth,coalesce(f.custom_cover,t.cover),t.hue,t.play_count,t.skip_count,t.last_played,t.favorite,f.added_at,f.encrypted,f.size";

fn track_row(r: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        album_artist: r.get(5)?,
        track_no: r.get(6)?,
        disc_no: r.get(7)?,
        year: r.get(8)?,
        genre: r.get(9)?,
        duration: r.get(10)?,
        format: r.get(11)?,
        lossless: r.get::<_, i64>(12)? != 0,
        bitrate: r.get(13)?,
        sample_rate: r.get(14)?,
        bit_depth: r.get(15)?,
        cover: r.get(16)?,
        hue: r.get(17)?,
        play_count: r.get(18)?,
        skip_count: r.get(19)?,
        last_played: r.get(20)?,
        favorite: r.get::<_, i64>(21)? != 0,
        added_at: r.get(22)?,
        encrypted: r.get::<_, i64>(23)? != 0,
        size: r.get(24)?,
    })
}

pub fn tracks(conn: &Connection) -> Result<Vec<Track>> {
    let sql = format!("SELECT {TRACK_COLS} FROM tracks t JOIN files f ON f.id=t.file_id WHERE f.missing=0 ORDER BY t.album_artist, t.artist, t.album, t.disc_no, t.track_no, t.title");
    let v = conn.prepare(&sql)?.query_map([], track_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

pub fn track(conn: &Connection, id: i64) -> Result<Option<Track>> {
    let sql = format!("SELECT {TRACK_COLS} FROM tracks t JOIN files f ON f.id=t.file_id WHERE f.id=?1");
    Ok(conn.query_row(&sql, [id], track_row).optional()?)
}

pub fn file_path(conn: &Connection, id: i64) -> Result<String> {
    Ok(conn.query_row("SELECT path FROM files WHERE id=?1", [id], |r| r.get(0))?)
}

#[derive(Serialize)]
pub struct Video {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub series_id: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<f64>,
    pub group: Option<String>,
    pub resolution: Option<String>,
    pub duration: f64,
    pub position: f64,
    pub watched: bool,
    pub last_played: Option<i64>,
    pub subs: String,
    pub thumb: Option<String>,
    pub size: i64,
    pub added_at: i64,
    /// 电影（放在“电影”分类里，不在独立视频里）
    pub movie: bool,
}

const VIDEO_COLS: &str = "f.id,f.path,coalesce(f.custom_title,v.title),v.series_id,v.season,v.episode,v.release_group,v.resolution,v.duration,v.position,v.watched,v.last_played,v.subs,coalesce(f.custom_cover,v.thumb),f.size,f.added_at,v.movie";

fn video_row(r: &Row) -> rusqlite::Result<Video> {
    Ok(Video {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        series_id: r.get(3)?,
        season: r.get(4)?,
        episode: r.get(5)?,
        group: r.get(6)?,
        resolution: r.get(7)?,
        duration: r.get(8)?,
        position: r.get(9)?,
        watched: r.get::<_, i64>(10)? != 0,
        last_played: r.get(11)?,
        subs: r.get(12)?,
        thumb: r.get(13)?,
        size: r.get(14)?,
        added_at: r.get(15)?,
        movie: r.get::<_, i64>(16)? != 0,
    })
}

/// 不属于任何剧集的独立视频
pub fn videos(conn: &Connection) -> Result<Vec<Video>> {
    let sql = format!("SELECT {VIDEO_COLS} FROM videos v JOIN files f ON f.id=v.file_id WHERE f.missing=0 AND v.series_id IS NULL ORDER BY f.added_at DESC");
    Ok(conn.prepare(&sql)?.query_map([], video_row)?.collect::<rusqlite::Result<_>>()?)
}

#[derive(Serialize)]
pub struct Series {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub poster: Option<String>,
    pub episodes: i64,
    pub watched: i64,
    pub seasons: i64,
    pub missing_subs: i64,
    pub last_played: Option<i64>,
    pub next_id: Option<i64>,
    pub next_label: Option<String>,
    pub thumb: Option<String>,
}

pub fn series(conn: &Connection, kind: &str) -> Result<Vec<Series>> {
    let mut st = conn.prepare(
        "SELECT s.id,coalesce(s.custom_title,s.title),s.kind,coalesce(s.custom_cover,s.poster),count(v.file_id),sum(v.watched),count(DISTINCT v.season),sum(v.subs='none'),max(v.last_played),
                (SELECT v2.thumb FROM videos v2 WHERE v2.series_id=s.id AND v2.thumb IS NOT NULL LIMIT 1)
         FROM series s JOIN videos v ON v.series_id=s.id JOIN files f ON f.id=v.file_id
         WHERE f.missing=0 AND s.kind=?1 GROUP BY s.id ORDER BY max(coalesce(v.last_played,0)) DESC, s.title",
    )?;
    let mut out: Vec<Series> = st
        .query_map([kind], |r| {
            Ok(Series {
                id: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                poster: r.get(3)?,
                episodes: r.get(4)?,
                watched: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                seasons: r.get(6)?,
                missing_subs: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                last_played: r.get(8)?,
                next_id: None,
                next_label: None,
                thumb: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for s in &mut out {
        if let Some((id, se, ep)) = next_episode(conn, s.id)? {
            s.next_id = Some(id);
            s.next_label = Some(ep_label(se, ep));
        }
    }
    Ok(out)
}

pub fn ep_label(season: Option<i64>, ep: Option<f64>) -> String {
    let e = ep
        .map(|e| if e.fract() == 0.0 { format!("{:02}", e as i64) } else { format!("{e}") })
        .unwrap_or_else(|| "?".into());
    match season {
        Some(0) => format!("特别篇 {e}"),
        Some(s) => format!("S{s:02}E{e}"),
        None => format!("第 {e} 集"),
    }
}

/// 继续观看：最近看过但没看完的那一集；否则是第一个没看过的
pub type EpRef = (i64, Option<i64>, Option<f64>);

pub fn next_episode(conn: &Connection, series_id: i64) -> Result<Option<EpRef>> {
    let partial = conn
        .query_row(
            "SELECT v.file_id,v.season,v.episode FROM videos v JOIN files f ON f.id=v.file_id WHERE f.missing=0 AND v.series_id=?1 AND v.watched=0 AND v.position>5
             ORDER BY v.last_played DESC LIMIT 1",
            [series_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if partial.is_some() {
        return Ok(partial);
    }
    Ok(conn
        .query_row(
            "SELECT v.file_id,v.season,v.episode FROM videos v JOIN files f ON f.id=v.file_id WHERE f.missing=0 AND v.series_id=?1 AND v.watched=0
             ORDER BY CASE WHEN v.season=0 THEN 999 ELSE coalesce(v.season,1) END, v.episode LIMIT 1",
            [series_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}

pub fn episodes(conn: &Connection, series_id: i64) -> Result<Vec<Video>> {
    let sql = format!(
        "SELECT {VIDEO_COLS} FROM videos v JOIN files f ON f.id=v.file_id WHERE f.missing=0 AND v.series_id=?1
         ORDER BY CASE WHEN v.season=0 THEN 999 ELSE coalesce(v.season,1) END, v.episode, v.title"
    );
    Ok(conn.prepare(&sql)?.query_map([series_id], video_row)?.collect::<rusqlite::Result<_>>()?)
}

pub fn video(conn: &Connection, id: i64) -> Result<Option<Video>> {
    let sql = format!("SELECT {VIDEO_COLS} FROM videos v JOIN files f ON f.id=v.file_id WHERE f.id=?1");
    Ok(conn.query_row(&sql, [id], video_row).optional()?)
}

pub fn save_video_progress(conn: &Connection, id: i64, position: f64, duration: f64) -> Result<()> {
    // 看到 92% 以上算看完
    let watched = duration > 0.0 && position / duration > 0.92;
    conn.execute(
        "UPDATE videos SET position=?1, duration=CASE WHEN ?2>0 THEN ?2 ELSE duration END, watched=max(watched,?3), last_played=?4 WHERE file_id=?5",
        params![if watched { 0.0 } else { position }, duration, watched as i64, db::now(), id],
    )?;
    Ok(())
}

pub fn set_watched(conn: &Connection, ids: &[i64], watched: bool) -> Result<()> {
    for id in ids {
        conn.execute("UPDATE videos SET watched=?1, position=0 WHERE file_id=?2", params![watched as i64, id])?;
    }
    Ok(())
}

#[derive(Serialize)]
pub struct BookItem {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub author: String,
    pub format: String,
    pub chapter: i64,
    pub chapters: i64,
    pub offset: f64,
    pub cover: Option<String>,
    pub last_read: Option<i64>,
    pub size: i64,
}

pub fn books(conn: &Connection) -> Result<Vec<BookItem>> {
    let mut st = conn.prepare(
        "SELECT f.id,f.path,coalesce(f.custom_title,b.title),b.author,b.format,b.chapter,b.chapters,b.offset,coalesce(f.custom_cover,b.cover),b.last_read,f.size FROM books b JOIN files f ON f.id=b.file_id
         WHERE f.missing=0 ORDER BY coalesce(b.last_read,0) DESC, b.title",
    )?;
    let v = st
        .query_map([], |r| {
            Ok(BookItem {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get(2)?,
                author: r.get(3)?,
                format: r.get(4)?,
                chapter: r.get(5)?,
                chapters: r.get(6)?,
                offset: r.get(7)?,
                cover: r.get(8)?,
                last_read: r.get(9)?,
                size: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

pub fn save_book_progress(conn: &Connection, id: i64, chapter: i64, offset: f64, chapters: i64) -> Result<()> {
    conn.execute(
        "UPDATE books SET chapter=?1, offset=?2, chapters=?3, last_read=?4 WHERE file_id=?5",
        params![chapter, offset, chapters, db::now(), id],
    )?;
    Ok(())
}

#[derive(Serialize)]
pub struct ComicItem {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub series: String,
    pub format: String,
    pub pages: i64,
    pub page: i64,
    pub cover: Option<String>,
    pub last_read: Option<i64>,
}

pub fn comics(conn: &Connection) -> Result<Vec<ComicItem>> {
    let mut st = conn.prepare(
        "SELECT f.id,f.path,coalesce(f.custom_title,c.title),c.series,c.format,c.pages,c.page,coalesce(f.custom_cover,c.cover),c.last_read FROM comics c JOIN files f ON f.id=c.file_id
         WHERE f.missing=0 ORDER BY c.series, c.title",
    )?;
    let mut v: Vec<ComicItem> = st
        .query_map([], |r| {
            Ok(ComicItem {
                id: r.get(0)?,
                path: r.get(1)?,
                title: r.get(2)?,
                series: r.get(3)?,
                format: r.get(4)?,
                pages: r.get(5)?,
                page: r.get(6)?,
                cover: r.get(7)?,
                last_read: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    v.sort_by(|a, b| {
        a.series
            .cmp(&b.series)
            .then_with(|| crate::comic::natural_key(&a.title).cmp(&crate::comic::natural_key(&b.title)))
    });
    Ok(v)
}

pub fn save_comic_progress(conn: &Connection, id: i64, page: i64) -> Result<()> {
    conn.execute("UPDATE comics SET page=?1, last_read=?2 WHERE file_id=?3", params![page, db::now(), id])?;
    Ok(())
}

// ---------- 播放统计 ----------

pub fn toggle_favorite(conn: &Connection, id: i64) -> Result<bool> {
    conn.execute("UPDATE tracks SET favorite=1-favorite WHERE file_id=?1", [id])?;
    Ok(conn.query_row("SELECT favorite FROM tracks WHERE file_id=?1", [id], |r| r.get::<_, i64>(0))? != 0)
}

/// 一次播放结束（切歌或播完）时调用。只有播放超过一半才计一次播放
pub fn record_play(conn: &Connection, id: i64, listened: f64, duration: f64, source: Option<String>) -> Result<()> {
    if listened < 1.0 {
        return Ok(());
    }
    let full = duration > 0.0 && listened >= duration / 2.0;
    let skipped = !full && listened < 30.0f64.min(duration * 0.3);
    conn.execute(
        "INSERT INTO play_history(file_id,at,listened,duration,skipped,source) VALUES(?1,?2,?3,?4,?5,?6)",
        params![id, db::now(), listened, duration, skipped as i64, source],
    )?;
    if full {
        conn.execute(
            "UPDATE tracks SET play_count=play_count+1, last_played=?1 WHERE file_id=?2",
            params![db::now(), id],
        )?;
    } else if skipped {
        conn.execute("UPDATE tracks SET skip_count=skip_count+1 WHERE file_id=?1", [id])?;
    }
    Ok(())
}

// ---------- 歌单 ----------

#[derive(Serialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub covers: Vec<String>,
    /// 用户设置的封面（绝对路径）
    pub cover: Option<String>,
}

pub fn playlists(conn: &Connection) -> Result<Vec<Playlist>> {
    let mut st =
        conn.prepare("SELECT p.id,p.name,(SELECT count(*) FROM playlist_items i WHERE i.playlist_id=p.id),p.cover FROM playlists p ORDER BY p.sort, p.id")?;
    let mut v: Vec<Playlist> = st
        .query_map([], |r| {
            Ok(Playlist {
                id: r.get(0)?,
                name: r.get(1)?,
                count: r.get(2)?,
                covers: vec![],
                cover: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    for p in &mut v {
        p.covers = conn
            .prepare("SELECT DISTINCT t.cover FROM playlist_items i JOIN tracks t ON t.file_id=i.file_id WHERE i.playlist_id=?1 AND t.cover IS NOT NULL ORDER BY i.pos LIMIT 4")?
            .query_map([p.id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(v)
}

pub fn create_playlist(conn: &Connection, name: &str) -> Result<i64> {
    let sort: i64 = conn.query_row("SELECT coalesce(max(sort),0)+1 FROM playlists", [], |r| r.get(0))?;
    conn.execute("INSERT INTO playlists(name,created_at,sort) VALUES(?1,?2,?3)", params![name, db::now(), sort])?;
    Ok(conn.last_insert_rowid())
}

pub fn rename_playlist(conn: &Connection, id: i64, name: &str) -> Result<()> {
    conn.execute("UPDATE playlists SET name=?1 WHERE id=?2", params![name, id])?;
    Ok(())
}

pub fn delete_playlist(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM playlists WHERE id=?1", [id])?;
    Ok(())
}

pub fn playlist_add(conn: &Connection, id: i64, files: &[i64]) -> Result<usize> {
    let mut pos: i64 = conn.query_row("SELECT coalesce(max(pos),0) FROM playlist_items WHERE playlist_id=?1", [id], |r| r.get(0))?;
    let mut n = 0;
    for f in files {
        pos += 1;
        n += conn.execute(
            "INSERT OR IGNORE INTO playlist_items(playlist_id,file_id,pos) VALUES(?1,?2,?3)",
            params![id, f, pos],
        )?;
    }
    Ok(n)
}

pub fn playlist_remove(conn: &Connection, id: i64, files: &[i64]) -> Result<()> {
    for f in files {
        conn.execute("DELETE FROM playlist_items WHERE playlist_id=?1 AND file_id=?2", params![id, f])?;
    }
    Ok(())
}

/// 按给定顺序重排
pub fn playlist_reorder(conn: &Connection, id: i64, files: &[i64]) -> Result<()> {
    for (i, f) in files.iter().enumerate() {
        conn.execute("UPDATE playlist_items SET pos=?1 WHERE playlist_id=?2 AND file_id=?3", params![i as i64, id, f])?;
    }
    Ok(())
}

pub fn playlist_items(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    Ok(conn
        .prepare("SELECT i.file_id FROM playlist_items i JOIN files f ON f.id=i.file_id WHERE i.playlist_id=?1 AND f.missing=0 ORDER BY i.pos")?
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn export_m3u8(conn: &Connection, id: i64, out: &Path) -> Result<()> {
    let mut s = String::from("#EXTM3U\n");
    for fid in playlist_items(conn, id)? {
        if let Some(t) = track(conn, fid)? {
            s.push_str(&format!("#EXTINF:{},{} - {}\n{}\n", t.duration as i64, t.artist, t.title, t.path));
        }
    }
    std::fs::write(out, s)?;
    Ok(())
}

/// 导入 m3u / m3u8：不在库里的文件先入库
pub fn import_m3u(conn: &Connection, file: &Path, dirs: &indexer::Dirs) -> Result<(i64, usize)> {
    let bytes = std::fs::read(file)?;
    let (text, _) = crate::reader::decode_text(&bytes);
    let base = file.parent().unwrap_or(Path::new("."));
    let name = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "导入的歌单".into());
    let id = create_playlist(conn, &name)?;
    let mut ids = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let p = Path::new(line);
        let p = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
        if !p.exists() {
            continue;
        }
        let ps = p.to_string_lossy().to_string();
        let mut fid: Option<i64> = conn.query_row("SELECT id FROM files WHERE path=?1", [&ps], |r| r.get(0)).optional()?;
        if fid.is_none() {
            if let Some(k) = crate::parse::classify(&p) {
                indexer::index_file(conn, &p, k, dirs)?;
                fid = conn.query_row("SELECT id FROM files WHERE path=?1", [&ps], |r| r.get(0)).optional()?;
            }
        }
        if let Some(f) = fid {
            ids.push(f);
        }
    }
    let n = playlist_add(conn, id, &ids)?;
    Ok((id, n))
}

// ---------- 搜索 ----------

#[derive(Serialize)]
pub struct Hit {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub sub: String,
    pub cover: Option<String>,
    pub series_id: Option<i64>,
}

pub fn search(conn: &Connection, q: &str, limit: i64) -> Result<Vec<Hit>> {
    let q = q.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let ids: Vec<(i64, String, String, String)> = if q.chars().count() >= 3 {
        let fts = format!("\"{}\"", q.replace('"', "\"\""));
        conn.prepare("SELECT file_id,kind,title,sub FROM search_idx WHERE search_idx MATCH ?1 ORDER BY rank LIMIT ?2")?
            .query_map(params![fts, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?
    } else {
        // trigram 至少要三个字，短关键词用 LIKE
        let like = format!("%{}%", q.replace(['%', '_'], ""));
        let ini = format!("{}%", q.to_lowercase());
        conn.prepare(
            "SELECT file_id,kind,title,sub FROM search_idx WHERE title LIKE ?1 OR sub LIKE ?1 OR initials LIKE ?2 OR initials LIKE '% ' || ?2 LIMIT ?3",
        )?
        .query_map(params![like, ini, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?
    };
    let mut out = vec![];
    let mut seen_series = std::collections::HashSet::new();
    for (id, kind, title, sub) in ids {
        let missing: i64 = conn.query_row("SELECT missing FROM files WHERE id=?1", [id], |r| r.get(0)).unwrap_or(1);
        if missing != 0 {
            continue;
        }
        let (cover, series_id) = match kind.as_str() {
            "music" => (
                conn.query_row("SELECT cover FROM tracks WHERE file_id=?1", [id], |r| r.get(0)).ok().flatten(),
                None,
            ),
            "novel" => (
                conn.query_row("SELECT cover FROM books WHERE file_id=?1", [id], |r| r.get(0)).ok().flatten(),
                None,
            ),
            "manga" => (
                conn.query_row("SELECT cover FROM comics WHERE file_id=?1", [id], |r| r.get(0)).ok().flatten(),
                None,
            ),
            _ => (
                None,
                conn.query_row("SELECT series_id FROM videos WHERE file_id=?1", [id], |r| r.get(0))
                    .ok()
                    .flatten(),
            ),
        };
        // 同一部番剧只出一条
        if let Some(s) = series_id {
            if !seen_series.insert(s) {
                continue;
            }
        }
        out.push(Hit {
            id,
            kind,
            title,
            sub,
            cover,
            series_id,
        });
    }
    Ok(out)
}

// ---------- 设置 ----------

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<serde_json::Value>> {
    let v: Option<String> = conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?;
    Ok(v.and_then(|s| serde_json::from_str(&s).ok()))
}

pub fn set_setting(conn: &Connection, key: &str, value: &serde_json::Value) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct Root {
    pub id: i64,
    pub path: String,
    pub watch: bool,
}

pub fn roots(conn: &Connection) -> Result<Vec<Root>> {
    Ok(conn
        .prepare("SELECT id,path,watch FROM scan_roots ORDER BY path")?
        .query_map([], |r| {
            Ok(Root {
                id: r.get(0)?,
                path: r.get(1)?,
                watch: r.get::<_, i64>(2)? != 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

pub fn add_root(conn: &Connection, path: &str, watch: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO scan_roots(path,watch) VALUES(?1,?2) ON CONFLICT(path) DO UPDATE SET watch=excluded.watch",
        params![path, watch as i64],
    )?;
    Ok(())
}

pub fn remove_root(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM scan_roots WHERE id=?1", [id])?;
    Ok(())
}

#[derive(Serialize)]
pub struct Stats {
    pub music: i64,
    pub video: i64,
    pub anime: i64,
    pub manga: i64,
    pub novel: i64,
}

pub fn stats(conn: &Connection) -> Result<Stats> {
    let c = |k: &str| {
        conn.query_row("SELECT count(*) FROM files WHERE kind=?1 AND missing=0", [k], |r| r.get::<_, i64>(0))
            .unwrap_or(0)
    };
    Ok(Stats {
        music: c("music"),
        video: c("video"),
        anime: c("anime"),
        manga: c("manga"),
        novel: c("novel"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(conn: &Connection) -> i64 {
        conn.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES('C:\\a.flac','music',1,1,1)", [])
            .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tracks(file_id,title,artist,album,duration,format,lossless) VALUES(?1,'雾港','林间信号','潮汐档案',200,'FLAC',1)",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO search_idx(file_id,kind,title,sub,initials) VALUES(?1,'music','雾港','林间信号 潮汐档案','wg ljxh cxda')",
            [id],
        )
        .unwrap();
        id
    }

    #[test]
    fn play_counting() {
        let c = db::open_memory().unwrap();
        let id = seed(&c);
        record_play(&c, id, 50.0, 200.0, None).unwrap();
        assert_eq!(track(&c, id).unwrap().unwrap().play_count, 0);
        record_play(&c, id, 120.0, 200.0, None).unwrap();
        assert_eq!(track(&c, id).unwrap().unwrap().play_count, 1);
        record_play(&c, id, 5.0, 200.0, None).unwrap();
        assert_eq!(track(&c, id).unwrap().unwrap().skip_count, 1);
    }

    #[test]
    fn playlists_crud() {
        let c = db::open_memory().unwrap();
        let id = seed(&c);
        let p = create_playlist(&c, "通勤").unwrap();
        assert_eq!(playlist_add(&c, p, &[id, id]).unwrap(), 1);
        assert_eq!(playlist_items(&c, p).unwrap(), vec![id]);
        assert_eq!(playlists(&c).unwrap()[0].count, 1);
        delete_playlist(&c, p).unwrap();
        assert!(playlists(&c).unwrap().is_empty());
    }

    #[test]
    fn search_short_and_initials() {
        let c = db::open_memory().unwrap();
        seed(&c);
        assert_eq!(search(&c, "雾港", 10).unwrap().len(), 1);
        assert_eq!(search(&c, "ljx", 10).unwrap().len(), 1);
        assert_eq!(search(&c, "潮汐档", 10).unwrap().len(), 1);
    }
}
