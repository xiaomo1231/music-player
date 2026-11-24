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
        conn.execute("UPDATE files SET path=?1, missing=0, encrypted=?2 WHERE id=?3", params![p, encrypted, id])?;
        return Ok(Outcome::Updated);
    } else if let Some((id, ..)) = existing {
        conn.execute(
            // 加密格式转换后“替换原文件”会把记录改成新路径：encrypted 要跟着新扩展名走，否则歌还留在“加密格式”里
            "UPDATE files SET size=?1, mtime=?2, missing=0, kind=?3, encrypted=?4 WHERE id=?5",
            params![size, mtime, kind, encrypted, id],
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
                 sample_rate=excluded.sample_rate,bit_depth=excluded.bit_depth,channels=excluded.channels,format=excluded.format,lossless=excluded.lossless,
                 cover=CASE WHEN tracks.net_lock=1 THEN tracks.net_cover ELSE coalesce(excluded.cover,tracks.net_cover) END,
                 hue=CASE WHEN tracks.net_lock=1 THEN tracks.net_hue WHEN excluded.cover IS NULL THEN tracks.net_hue ELSE excluded.hue END",
                params![file_id, m.title, m.artist, m.album, m.album_artist, m.track_no, m.disc_no, m.year, m.genre, m.duration, m.bitrate, m.sample_rate, m.bit_depth, m.channels, m.format, m.lossless as i64, m.cover, m.hue],
            )?;
            set_search(conn, file_id, "music", &m.title, &format!("{} {}", m.artist, m.album))?;
        }
        "video" | "anime" => {
            let vc = crate::aiclassify::apply(conn, &p, parse::video_class_as(path, kind));
            if vc.kind != kind {
                conn.execute("UPDATE files SET kind=?1 WHERE id=?2", params![vc.kind, file_id])?;
            }
            let subs = if has_external_subs(path) {
                "external"
            } else if parse::ext(path) == "mkv" {
                "embedded?"
            } else {
                "none"
            };
            conn.execute(
                "INSERT INTO videos(file_id,title,subs) VALUES(?1,'',?2) ON CONFLICT(file_id) DO UPDATE SET subs=excluded.subs",
                params![file_id, subs],
            )?;
            let title = write_video_class(conn, file_id, path, &vc)?;
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
                if vc.movie_guess && dur > 0.0 && dur < MOVIE_MIN_SECS {
                    conn.execute("UPDATE videos SET movie=0 WHERE file_id=?1", [file_id])?;
                }
            }
            set_search(conn, file_id, vc.kind, &vc.info.title, &title)?;
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

/// 按文件名猜出来的电影，时长不到 40 分钟就不算
const MOVIE_MIN_SECS: f64 = 40.0 * 60.0;

/// 找同名（忽略标点、大小写）的番剧 / 剧集，没有就新建
fn series_for(conn: &Connection, title: &str, kind: &str) -> Result<i64> {
    let key = parse::title_key(title);
    if !key.is_empty() {
        let found = conn
            .prepare("SELECT id,title FROM series WHERE kind=?1 ORDER BY id")?
            .query_map([kind], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .find(|(_, t)| parse::title_key(t) == key);
        if let Some((id, _)) = found {
            return Ok(id);
        }
    }
    conn.execute("INSERT OR IGNORE INTO series(title,kind) VALUES(?1,?2)", params![title, kind])?;
    Ok(conn.query_row("SELECT id FROM series WHERE title=?1 AND kind=?2", params![title, kind], |r| r.get(0))?)
}

/// 把归类结果写进 videos（标题、所属番剧 / 剧集、季、集、电影标记），返回这一条的标题
fn write_video_class(conn: &Connection, file_id: i64, path: &Path, vc: &parse::VideoClass) -> Result<String> {
    let info = &vc.info;
    let series_id = if vc.series {
        let skind = if vc.kind == "anime" { "anime" } else { "tv" };
        let sid = series_for(conn, &info.title, skind)?;
        let roots: Vec<std::path::PathBuf> = crate::library::roots(conn)?.into_iter().map(|r| std::path::PathBuf::from(r.path)).collect();
        if let Some(img) = crate::artwork::local_poster(path, &roots) {
            conn.execute(
                "UPDATE series SET poster=?1 WHERE id=?2 AND poster IS NULL",
                params![img.to_string_lossy(), sid],
            )?;
        }
        Some(sid)
    } else {
        None
    };
    // 番剧里的每一集用文件名当标题，独立视频和电影用解析出的名字
    let title = if series_id.is_some() {
        path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| info.title.clone())
    } else {
        info.title.clone()
    };
    conn.execute(
        "UPDATE videos SET title=?1,series_id=?2,season=?3,episode=?4,release_group=?5,resolution=?6,movie=?7,rules=?8 WHERE file_id=?9",
        params![
            title,
            series_id,
            info.season.or(series_id.map(|_| 1)),
            info.episode,
            info.group,
            info.resolution,
            vc.movie as i64,
            parse::VIDEO_RULES,
            file_id
        ],
    )?;
    Ok(title)
}

/// 按规则（再叠加 AI 的结果）重新归类一个视频并写库；碟片菜单这类不该入库的直接移除
fn reclassify_one(conn: &Connection, id: i64, p: &str, old_kind: &str, dur: f64) -> Result<()> {
    let path = Path::new(p);
    let Some(vc) = parse::classify_video(path) else {
        conn.execute("DELETE FROM search_idx WHERE file_id=?1", [id])?;
        conn.execute("DELETE FROM files WHERE id=?1", [id])?;
        return Ok(());
    };
    // 以前就在番剧里的不往回挪（可能是下载时手动选的番剧库）
    let kind = if old_kind == "anime" { "anime" } else { vc.kind };
    let vc = if kind == vc.kind { vc } else { parse::video_class_as(path, kind) };
    let vc = crate::aiclassify::apply(conn, p, vc);
    conn.execute("UPDATE files SET kind=?1 WHERE id=?2", params![vc.kind, id])?;
    let title = write_video_class(conn, id, path, &vc)?;
    if vc.movie_guess && dur > 0.0 && dur < MOVIE_MIN_SECS {
        conn.execute("UPDATE videos SET movie=0 WHERE file_id=?1", [id])?;
    }
    set_search(conn, id, vc.kind, &vc.info.title, &title)
}

/// AI 给出结果后重新归类这一条，并清掉没有集了的番剧 / 剧集
pub fn reapply_video(conn: &Connection, id: i64, p: &str) -> Result<()> {
    let Some((kind, dur)) = conn
        .query_row(
            "SELECT f.kind,v.duration FROM files f JOIN videos v ON v.file_id=f.id WHERE f.id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    reclassify_one(conn, id, p, &kind, dur)?;
    conn.execute(
        "DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM videos WHERE series_id IS NOT NULL)",
        [],
    )?;
    Ok(())
}

/// 归类规则升级后，按新规则重新归类已经入库的视频（只看路径，不读文件，不丢观看进度）。
/// 碟片菜单这类现在不入库的文件直接移出媒体库
pub fn reclassify_videos(conn: &Connection) -> Result<usize> {
    let rows: Vec<(i64, String, String, f64)> = conn
        .prepare(
            "SELECT f.id,f.path,f.kind,v.duration FROM files f JOIN videos v ON v.file_id=f.id
             WHERE f.kind IN ('video','anime') AND v.rules < ?1",
        )?
        .query_map([parse::VIDEO_RULES], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if rows.is_empty() {
        return Ok(0);
    }
    conn.execute_batch("BEGIN")?;
    let res = (|| -> Result<()> {
        for (id, p, old_kind, dur) in &rows {
            reclassify_one(conn, *id, p, old_kind, *dur)?;
        }
        conn.execute(
            "DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM videos WHERE series_id IS NOT NULL)",
            [],
        )?;
        Ok(())
    })();
    match res {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(rows.len())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
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
    /// 加密格式转换后替换原文件：同一条记录换成新路径再入库，encrypted 要清掉
    #[test]
    fn replaced_encrypted_file_is_no_longer_encrypted() {
        use rusqlite::params;
        let c = crate::db::open_memory().unwrap();
        let tmp = std::env::temp_dir().join(format!("mp-enc-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let old = tmp.join("song.ncm");
        let new = tmp.join("song.flac");
        std::fs::write(&new, b"not really audio").unwrap();
        c.execute(
            "INSERT INTO files(path,kind,size,mtime,added_at,encrypted) VALUES(?1,'music',1,1,1,1)",
            params![old.to_string_lossy().to_string()],
        )
        .unwrap();
        let id = c.last_insert_rowid();
        // convert::relink 的做法：先改路径，再重新入库
        c.execute("UPDATE files SET path=?1 WHERE id=?2", params![new.to_string_lossy().to_string(), id])
            .unwrap();
        let dirs = super::Dirs {
            data: tmp.clone(),
            covers: tmp.join("covers"),
            thumbs: tmp.join("thumbs"),
        };
        super::index_file(&c, &new, "music", &dirs).unwrap();
        let enc: i64 = c.query_row("SELECT encrypted FROM files WHERE id=?1", [id], |r| r.get(0)).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(enc, 0);
    }

    #[test]
    fn pinyin_initials() {
        assert_eq!(super::initials("周杰伦"), "zjl");
        assert_eq!(super::initials("Dawn 协议"), "dawnxy");
    }

    /// 旧版本扫进来的数据：番剧全在“视频”里、碟片菜单成了一部叫 menu 的番剧。
    /// 启动时按新规则重新归类，观看进度保留
    #[test]
    fn reclassify_old_library() {
        use rusqlite::params;
        let c = crate::db::open_memory().unwrap();
        let mygo = r"F:\番\[DBD-Raws][BanG Dream! It's MyGO!!!!!][01-13TV全集][1080P][BDRip][HEVC-10bit][简繁日双语外挂][FLAC][MKV](1)";
        let rows: Vec<(String, &str)> = vec![
            (r"F:\番\孤独摇滚！\01.mp4".into(), "video"),
            (r"F:\番\孤独摇滚！\02.mp4".into(), "video"),
            (r"F:\番\0.孤独摇滚（上+下）2024【剧场总集篇】中字\孤独摇滚（上）劇場総集編.2024.HD1080P（中字）.mp4".into(), "video"),
            (format!(r"{mygo}\Season 1\S01E01.mkv"), "video"),
            (format!(r"{mygo}\menu\[DBD-Raws][BanG Dream! It's MyGO!!!!!][menu][01][1080P][BDRip][HEVC-10bit][FLAC].mkv"), "anime"),
            (r"F:\电影\[阿甘正传][1994][中英双字][4K HDR][29.6G]\[阿甘正传][1994][中英双字][4K HDR][29.6G]\Forrest.Gump.1994.Bluray.2160p.x265.10bit.HDR.2Audio-MiniHD.mkv".into(), "video"),
        ];
        c.execute("INSERT INTO series(title,kind) VALUES('menu','anime')", []).unwrap();
        for (i, (p, k)) in rows.iter().enumerate() {
            c.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,?2,1,1,1)", params![p, k])
                .unwrap();
            let id = c.last_insert_rowid();
            let sid: Option<i64> = if *k == "anime" { Some(1) } else { None };
            c.execute(
                "INSERT INTO videos(file_id,title,series_id,position) VALUES(?1,?2,?3,?4)",
                params![id, format!("{i:02}"), sid, 100.0],
            )
            .unwrap();
        }
        assert_eq!(super::reclassify_videos(&c).unwrap(), 6);
        // 再跑一次什么都不做
        assert_eq!(super::reclassify_videos(&c).unwrap(), 0);
        let series: Vec<(String, String, i64)> = c
            .prepare("SELECT s.title,s.kind,count(*) FROM series s JOIN videos v ON v.series_id=s.id GROUP BY s.id ORDER BY s.title")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            series,
            vec![
                ("BanG Dream! It's MyGO!!!!!".into(), "anime".into(), 1),
                ("孤独摇滚！".into(), "anime".into(), 3)
            ]
        );
        // 菜单移出媒体库，电影单独标出来，进度还在
        let n: i64 = c.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 5);
        let (t, m, pos): (String, i64, f64) = c
            .query_row("SELECT title,movie,position FROM videos WHERE series_id IS NULL", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((t.as_str(), m, pos), ("阿甘正传", 1, 100.0));
        let kinds: i64 = c.query_row("SELECT count(*) FROM files WHERE kind='anime'", [], |r| r.get(0)).unwrap();
        assert_eq!(kinds, 4);
    }
}
