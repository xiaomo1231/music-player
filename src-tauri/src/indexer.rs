//! 把单个文件读出信息写进媒体库（扫描和实时监控共用）
use crate::{comic, db, metadata, parse, reader};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Dirs {
    pub data: std::path::PathBuf,
    pub covers: std::path::PathBuf,
    pub thumbs: std::path::PathBuf,
}

/// 拼音首字母，用于“zjl”搜到“周杰伦”
pub fn initials(s: &str) -> String {
    use pinyin::ToPinyin;
    let mut out = String::new();
    for c in s.chars() {
        if let Some(p) = c.to_pinyin() {
            out.push_str(p.first_letter());
        } else if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

pub fn set_search(conn: &Connection, file_id: i64, kind: &str, title: &str, sub: &str) -> Result<()> {
    // 用户改过名字时，搜索用改过的名字（番剧用整部的名字）
    let custom: Option<String> = conn
        .query_row(
            "SELECT coalesce(f.custom_title, s.custom_title) FROM files f LEFT JOIN videos v ON v.file_id=f.id LEFT JOIN series s ON s.id=v.series_id WHERE f.id=?1",
            [file_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let title = custom.as_deref().unwrap_or(title);
    conn.execute("DELETE FROM search_idx WHERE file_id=?1", [file_id])?;
    let ini = format!("{} {}", initials(title), initials(sub));
    conn.execute(
        "INSERT INTO search_idx(file_id,kind,title,sub,initials) VALUES(?1,?2,?3,?4,?5)",
        params![file_id, kind, title, sub, ini],
    )?;
    Ok(())
}

pub enum Outcome {
    Added,
    Updated,
    Unchanged,
}

/// 入库或更新；文件没变（大小 + 修改时间一致）时直接跳过
pub fn index_file(conn: &Connection, path: &Path, kind: &str, dirs: &Dirs) -> Result<Outcome> {
    // 用户“从媒体库移除”过的文件，扫描时跳过
    let excluded: bool = conn
        .query_row("SELECT 1 FROM excluded WHERE path=?1", [path.to_string_lossy()], |_| Ok(true))
        .optional()?
        .unwrap_or(false);
    if excluded {
        return Ok(Outcome::Unchanged);
    }
    let meta = std::fs::metadata(path)?;
    let size = meta.len() as i64;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let p = path.to_string_lossy().to_string();
    let existing: Option<(i64, i64, i64, i64)> = conn
        .query_row("SELECT id,size,mtime,missing FROM files WHERE path=?1", [&p], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    if let Some((id, s, m, missing)) = existing {
        if s == size && m == mtime {
            if missing != 0 {
                conn.execute("UPDATE files SET missing=0 WHERE id=?1", [id])?;
            }
            return Ok(Outcome::Unchanged);
        }
    }
    // 文件被移动：同样大小 + 修改时间、原路径已不存在的记录，直接改路径，保留播放次数和进度
    let moved: Option<i64> = if existing.is_none() {
        let cands: Vec<(i64, String)> = conn
            .prepare("SELECT id,path FROM files WHERE size=?1 AND mtime=?2 AND kind=?3")?
            .query_map(params![size, mtime, kind], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        cands.into_iter().find(|(_, old)| !Path::new(old).exists()).map(|(id, _)| id)
    } else {
        None
    };
    let encrypted = parse::is_encrypted(path) as i64;
    let file_id = if let Some(id) = moved {
        conn.execute("UPDATE files SET path=?1, missing=0 WHERE id=?2", params![p, id])?;
        return Ok(Outcome::Updated);
    } else if let Some((id, ..)) = existing {
        conn.execute(
            "UPDATE files SET size=?1, mtime=?2, missing=0, kind=?3 WHERE id=?4",
            params![size, mtime, kind, id],
        )?;
        id
    } else {
        conn.execute(
            "INSERT INTO files(path,kind,size,mtime,added_at,encrypted) VALUES(?1,?2,?3,?4,?5,?6)",
            params![p, kind, size, mtime, db::now(), encrypted],
        )?;
        conn.last_insert_rowid()
    };
    match kind {
        "music" => {
            let m = metadata::read_track(path, &dirs.covers);
            conn.execute(
                "INSERT INTO tracks(file_id,title,artist,album,album_artist,track_no,disc_no,year,genre,duration,bitrate,sample_rate,bit_depth,channels,format,lossless,cover,hue)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
                 ON CONFLICT(file_id) DO UPDATE SET title=excluded.title,artist=excluded.artist,album=excluded.album,album_artist=excluded.album_artist,
                 track_no=excluded.track_no,disc_no=excluded.disc_no,year=excluded.year,genre=excluded.genre,duration=excluded.duration,bitrate=excluded.bitrate,
                 sample_rate=excluded.sample_rate,bit_depth=excluded.bit_depth,channels=excluded.channels,format=excluded.format,lossless=excluded.lossless,cover=excluded.cover,hue=excluded.hue",
                params![file_id, m.title, m.artist, m.album, m.album_artist, m.track_no, m.disc_no, m.year, m.genre, m.duration, m.bitrate, m.sample_rate, m.bit_depth, m.channels, m.format, m.lossless as i64, m.cover, m.hue],
            )?;
            set_search(conn, file_id, "music", &m.title, &format!("{} {}", m.artist, m.album))?;
        }
        "video" | "anime" => {
            let info = parse::parse_episode(path);
            let series_id: Option<i64> = if kind == "anime" || info.episode.is_some() {
                let skind = if kind == "anime" { "anime" } else { "tv" };
                conn.execute("INSERT OR IGNORE INTO series(title,kind) VALUES(?1,?2)", params![info.title, skind])?;
                let sid: i64 = conn.query_row("SELECT id FROM series WHERE title=?1 AND kind=?2", params![info.title, skind], |r| r.get(0))?;
                if let Some(img) = metadata::folder_cover(path) {
                    conn.execute(
                        "UPDATE series SET poster=?1 WHERE id=?2 AND poster IS NULL",
                        params![img.to_string_lossy(), sid],
                    )?;
                }
                Some(sid)
            } else {
                None
            };
            let subs = if has_external_subs(path) {
                "external"
            } else if parse::ext(path) == "mkv" {
                "embedded?"
            } else {
                "none"
            };
            let title = if info.episode.is_some() {
                path.file_stem().unwrap().to_string_lossy().to_string()
            } else {
                info.title.clone()
            };
            conn.execute(
                "INSERT INTO videos(file_id,title,series_id,season,episode,release_group,resolution,subs) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(file_id) DO UPDATE SET title=excluded.title,series_id=excluded.series_id,season=excluded.season,episode=excluded.episode,
                 release_group=excluded.release_group,resolution=excluded.resolution,subs=excluded.subs",
                params![
                    file_id,
                    title,
                    series_id,
                    info.season.or(series_id.map(|_| 1)),
                    info.episode,
                    info.group,
                    info.resolution,
                    subs
                ],
            )?;
            // 时长、分辨率、编码（ffprobe 不在时跳过，播放器打开时还会再读）
            if let Ok(v) = crate::tools::probe(path) {
                let dur: f64 = v["format"]["duration"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let vs = v["streams"]
                    .as_array()
                    .and_then(|a| a.iter().find(|s| s["codec_type"] == "video").cloned())
                    .unwrap_or_default();
                conn.execute(
                    "UPDATE videos SET duration=?1, width=?2, height=?3, codec=?4 WHERE file_id=?5",
                    params![dur, vs["width"].as_i64(), vs["height"].as_i64(), vs["codec_name"].as_str(), file_id],
                )?;
            }
            set_search(conn, file_id, kind, &info.title, &title)?;
        }
        "novel" => {
            let stem = path.file_stem().unwrap().to_string_lossy().to_string();
            let (mut title, mut author) = reader::split_title_author(&stem);
            let fmt = parse::ext(path);
            let mut cover = None;
            if fmt == "epub" {
                if let Ok(b) = reader::open_epub(path) {
                    title = b.title;
                    if !b.author.is_empty() {
                        author = b.author;
                    }
                }
                if let Some(bytes) = reader::epub_cover(path) {
                    cover = metadata::save_cover(&bytes, &dirs.covers).map(|c| c.0);
                }
            }
            conn.execute(
                "INSERT INTO books(file_id,title,author,format,cover) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(file_id) DO UPDATE SET title=excluded.title,author=excluded.author,format=excluded.format,cover=excluded.cover",
                params![file_id, title, author, fmt, cover],
            )?;
            set_search(conn, file_id, "novel", &title, &author)?;
        }
        "manga" => {
            let info = comic::inspect(path, &dirs.covers);
            conn.execute(
                "INSERT INTO comics(file_id,title,series,format,pages,cover) VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(file_id) DO UPDATE SET title=excluded.title,series=excluded.series,format=excluded.format,pages=excluded.pages,cover=excluded.cover",
                params![file_id, info.title, info.series, info.format, info.pages, info.cover],
            )?;
            set_search(conn, file_id, "manga", &info.title, &info.series)?;
        }
        _ => {}
    }
    Ok(if existing.is_some() { Outcome::Updated } else { Outcome::Added })
}

pub fn has_external_subs(path: &Path) -> bool {
    let Some(dir) = path.parent() else { return false };
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                let p = e.path();
                parse::SUBS.contains(&parse::ext(&p).as_str()) && p.file_stem().map(|s| s.to_string_lossy().starts_with(&stem)).unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// 文件已删除：标记为缺失（保留统计，文件回来时恢复）
pub fn mark_missing(conn: &Connection, path: &Path) -> Result<()> {
    conn.execute("UPDATE files SET missing=1 WHERE path=?1", [path.to_string_lossy()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn pinyin_initials() {
        assert_eq!(super::initials("周杰伦"), "zjl");
        assert_eq!(super::initials("Dawn 协议"), "dawnxy");
    }
}
