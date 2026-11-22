//! 条目管理：重命名、更换封面、从媒体库移除、删除文件（进回收站）
//! kind：track / video / book / comic 用文件 id；series 用番剧 / 剧集 id（作用于整部的所有集）
//! 自定义的名字和封面存在 files / series 的 custom_* 列，重新扫描不会覆盖
use crate::{db, indexer, metadata, parse, AppState};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

type R<T> = Result<T, String>;
fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

const SIDECARS: &[&str] = &["lrc", "srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "nfo"];

/// 这个条目对应的 (file_id, 路径)
pub fn files_of(conn: &Connection, kind: &str, id: i64) -> Result<Vec<(i64, String)>> {
    let v = match kind {
        "series" => conn
            .prepare("SELECT f.id,f.path FROM videos v JOIN files f ON f.id=v.file_id WHERE v.series_id=?1 ORDER BY v.season, v.episode")?
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        "track" | "video" | "book" | "comic" => conn
            .query_row("SELECT id,path FROM files WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .into_iter()
            .collect(),
        _ => return Err(anyhow!("未知的条目类型：{kind}")),
    };
    Ok(v)
}

/// 同名的歌词、字幕等配套文件（删除时一起进回收站）
pub fn sidecars(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().map(|s| s.to_string_lossy().to_string())) else {
        return vec![];
    };
    if path.is_dir() {
        return vec![];
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != path && p.is_file())
        .filter(|p| SIDECARS.contains(&parse::ext(p).as_str()))
        .filter(|p| {
            let s = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            // 视频名.srt、视频名.zh.ass 都算
            s == stem || s.starts_with(&format!("{stem}."))
        })
        .collect();
    v.sort();
    v
}

/// 未改名时的原始名字
fn base_title(conn: &Connection, kind: &str, id: i64) -> Result<String> {
    let sql = match kind {
        "series" => "SELECT title FROM series WHERE id=?1",
        "track" => "SELECT title FROM tracks WHERE file_id=?1",
        "video" => "SELECT title FROM videos WHERE file_id=?1",
        "book" => "SELECT title FROM books WHERE file_id=?1",
        "comic" => "SELECT title FROM comics WHERE file_id=?1",
        _ => return Err(anyhow!("未知的条目类型：{kind}")),
    };
    Ok(conn.query_row(sql, [id], |r| r.get(0))?)
}

/// 改名；name 为 None 或空白时恢复原名
pub fn rename(conn: &Connection, kind: &str, id: i64, name: Option<&str>) -> Result<String> {
    let name = name.map(str::trim).filter(|s| !s.is_empty());
    if kind == "series" {
        conn.execute("UPDATE series SET custom_title=?1 WHERE id=?2", params![name, id])?;
    } else {
        conn.execute("UPDATE files SET custom_title=?1 WHERE id=?2", params![name, id])?;
    }
    let shown = match name {
        Some(n) => n.to_string(),
        None => base_title(conn, kind, id)?,
    };
    // 同步搜索索引
    for (fid, _) in files_of(conn, kind, id)? {
        let t = if kind == "series" {
            shown.clone()
        } else {
            conn.query_row(
                "SELECT coalesce(f.custom_title, s.custom_title) FROM files f LEFT JOIN videos v ON v.file_id=f.id LEFT JOIN series s ON s.id=v.series_id WHERE f.id=?1",
                [fid],
                |r| r.get::<_, Option<String>>(0),
            )?
            .unwrap_or_else(|| shown.clone())
        };
        let sub: String = conn
            .query_row("SELECT sub FROM search_idx WHERE file_id=?1", [fid], |r| r.get(0))
            .optional()?
            .unwrap_or_default();
        conn.execute(
            "UPDATE search_idx SET title=?1, initials=?2 WHERE file_id=?3",
            params![t, format!("{} {}", indexer::initials(&t), indexer::initials(&sub)), fid],
        )?;
    }
    Ok(shown)
}

/// 设置封面（已保存好的图片绝对路径）；None 恢复默认
pub fn set_cover(conn: &Connection, kind: &str, id: i64, cover: Option<&str>) -> Result<()> {
    if kind == "series" {
        conn.execute("UPDATE series SET custom_cover=?1 WHERE id=?2", params![cover, id])?;
    } else {
        conn.execute("UPDATE files SET custom_cover=?1 WHERE id=?2", params![cover, id])?;
    }
    Ok(())
}

/// 从库里删掉这些文件的记录（播放次数、进度、歌单里的位置一起删）
fn drop_rows(conn: &Connection, files: &[(i64, String)], exclude: bool) -> Result<()> {
    for (fid, path) in files {
        if exclude {
            conn.execute("INSERT OR REPLACE INTO excluded(path,at) VALUES(?1,?2)", params![path, db::now()])?;
        }
        conn.execute("DELETE FROM search_idx WHERE file_id=?1", [fid])?;
        conn.execute("DELETE FROM files WHERE id=?1", [fid])?;
    }
    // 没有集了的番剧 / 剧集
    conn.execute(
        "DELETE FROM series WHERE id NOT IN (SELECT DISTINCT series_id FROM videos WHERE series_id IS NOT NULL)",
        [],
    )?;
    Ok(())
}

/// 从媒体库移除（不动硬盘文件，以后扫描也跳过）
pub fn remove(conn: &Connection, kind: &str, id: i64) -> Result<usize> {
    let files = files_of(conn, kind, id)?;
    drop_rows(conn, &files, true)?;
    Ok(files.len())
}

#[derive(Serialize)]
pub struct FileList {
    /// 媒体文件
    pub files: Vec<String>,
    /// 同名的歌词、字幕等配套文件
    pub extras: Vec<String>,
    pub size: u64,
}

pub fn list_files(conn: &Connection, kind: &str, id: i64) -> Result<FileList> {
    let files = files_of(conn, kind, id)?;
    let mut extras = vec![];
    let mut size = 0u64;
    for (_, p) in &files {
        let pb = Path::new(p);
        size += std::fs::metadata(pb).map(|m| m.len()).unwrap_or(0);
        extras.extend(sidecars(pb).into_iter().map(|x| x.to_string_lossy().to_string()));
    }
    Ok(FileList {
        files: files.into_iter().map(|f| f.1).collect(),
        extras,
        size,
    })
}

// ---------- 命令 ----------

#[tauri::command]
pub fn item_files(st: State<AppState>, kind: String, id: i64) -> R<FileList> {
    list_files(&st.db.lock(), &kind, id).map_err(e)
}

#[tauri::command]
pub fn item_rename(app: AppHandle, st: State<AppState>, kind: String, id: i64, name: Option<String>) -> R<String> {
    let r = rename(&st.db.lock(), &kind, id, name.as_deref()).map_err(e)?;
    let _ = app.emit("library://changed", ());
    Ok(r)
}

/// image 为本地图片路径；None 恢复默认封面
#[tauri::command]
pub fn item_set_cover(app: AppHandle, st: State<AppState>, kind: String, id: i64, image: Option<String>) -> R<()> {
    let saved = match image {
        Some(p) => Some(save_image(&p, &st.dirs.covers)?),
        None => None,
    };
    set_cover(&st.db.lock(), &kind, id, saved.as_deref()).map_err(e)?;
    let _ = app.emit("library://changed", ());
    Ok(())
}

#[tauri::command]
pub fn item_remove(app: AppHandle, st: State<AppState>, kind: String, id: i64) -> R<usize> {
    let n = remove(&st.db.lock(), &kind, id).map_err(e)?;
    let _ = app.emit("library://changed", ());
    Ok(n)
}

/// 删除文件：媒体文件和同名配套文件一起移入回收站，然后从库里删掉记录
#[tauri::command]
pub fn item_delete(app: AppHandle, st: State<AppState>, kind: String, id: i64) -> R<usize> {
    let files = files_of(&st.db.lock(), &kind, id).map_err(e)?;
    let mut paths: Vec<PathBuf> = vec![];
    for (_, p) in &files {
        let pb = PathBuf::from(p);
        paths.extend(sidecars(&pb));
        if pb.exists() {
            paths.push(pb);
        }
    }
    if !paths.is_empty() {
        trash::delete_all(&paths).map_err(|err| format!("移入回收站失败（文件可能正在被使用）：{err}"))?;
    }
    drop_rows(&st.db.lock(), &files, false).map_err(e)?;
    let _ = app.emit("library://changed", ());
    Ok(files.len())
}

/// 歌单封面：image 为本地图片路径，None 恢复为歌曲封面拼图
#[tauri::command]
pub fn playlist_set_cover(app: AppHandle, st: State<AppState>, id: i64, image: Option<String>) -> R<()> {
    let saved = match image {
        Some(p) => Some(save_image(&p, &st.dirs.covers)?),
        None => None,
    };
    st.db
        .lock()
        .execute("UPDATE playlists SET cover=?1 WHERE id=?2", params![saved, id])
        .map_err(e)?;
    let _ = app.emit("library://changed", ());
    Ok(())
}

/// 把用户选的图片缩放后存进封面缓存，返回绝对路径（智能列表的封面也用这个）
#[tauri::command]
pub fn item_save_image(st: State<AppState>, path: String) -> R<String> {
    save_image(&path, &st.dirs.covers)
}

fn save_image(path: &str, covers: &Path) -> R<String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读取图片失败：{err}"))?;
    let (name, _) = metadata::save_cover(&bytes, covers).ok_or("这个文件不是能识别的图片")?;
    Ok(covers.join(name).to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(c: &Connection) -> (i64, i64, i64) {
        c.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES('C:\\a.flac','music',1,1,1)", [])
            .unwrap();
        let t = c.last_insert_rowid();
        c.execute("INSERT INTO tracks(file_id,title,duration,format) VALUES(?1,'原名',1,'FLAC')", [t])
            .unwrap();
        c.execute(
            "INSERT INTO search_idx(file_id,kind,title,sub,initials) VALUES(?1,'music','原名','歌手','ym gs')",
            [t],
        )
        .unwrap();
        c.execute("INSERT INTO series(title,kind) VALUES('番剧','anime')", []).unwrap();
        let s = c.last_insert_rowid();
        for i in 1..=3 {
            c.execute(
                "INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,'anime',1,1,1)",
                [format!("C:\\ep{i}.mkv")],
            )
            .unwrap();
            let f = c.last_insert_rowid();
            c.execute(
                "INSERT INTO videos(file_id,title,series_id,season,episode) VALUES(?1,'ep',?2,1,?3)",
                params![f, s, i],
            )
            .unwrap();
        }
        (t, s, 0)
    }

    #[test]
    fn rename_and_restore() {
        let c = db::open_memory().unwrap();
        let (t, s, _) = seed(&c);
        assert_eq!(rename(&c, "track", t, Some(" 新名字 ")).unwrap(), "新名字");
        let title: String = c.query_row("SELECT title FROM search_idx WHERE file_id=?1", [t], |r| r.get(0)).unwrap();
        assert_eq!(title, "新名字");
        assert_eq!(crate::library::track(&c, t).unwrap().unwrap().title, "新名字");
        assert_eq!(rename(&c, "track", t, None).unwrap(), "原名");
        assert_eq!(crate::library::track(&c, t).unwrap().unwrap().title, "原名");
        rename(&c, "series", s, Some("新番名")).unwrap();
        assert_eq!(crate::library::series(&c, "anime").unwrap()[0].title, "新番名");
    }

    #[test]
    fn custom_survives_rescan_update() {
        let c = db::open_memory().unwrap();
        let (t, _, _) = seed(&c);
        rename(&c, "track", t, Some("自定义")).unwrap();
        set_cover(&c, "track", t, Some("D:\\covers\\x.jpg")).unwrap();
        // 扫描更新标签时只改 tracks 表
        c.execute("UPDATE tracks SET title='标签里的名字', cover='y.jpg' WHERE file_id=?1", [t])
            .unwrap();
        let tr = crate::library::track(&c, t).unwrap().unwrap();
        assert_eq!(tr.title, "自定义");
        assert_eq!(tr.cover.as_deref(), Some("D:\\covers\\x.jpg"));
        set_cover(&c, "track", t, None).unwrap();
        assert_eq!(crate::library::track(&c, t).unwrap().unwrap().cover.as_deref(), Some("y.jpg"));
    }

    #[test]
    fn remove_series_excludes_all_episodes() {
        let c = db::open_memory().unwrap();
        let (_, s, _) = seed(&c);
        assert_eq!(remove(&c, "series", s).unwrap(), 3);
        let n: i64 = c.query_row("SELECT count(*) FROM excluded", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        let left: i64 = c.query_row("SELECT count(*) FROM series", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn excluded_file_not_reindexed() {
        let c = db::open_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("mp-item-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.txt");
        std::fs::write(&f, "第一章 开始\n内容".repeat(3000)).unwrap();
        c.execute("INSERT INTO excluded(path,at) VALUES(?1,1)", [f.to_string_lossy()]).unwrap();
        let dirs = indexer::Dirs {
            data: dir.clone(),
            covers: dir.clone(),
            thumbs: dir.clone(),
        };
        indexer::index_file(&c, &f, "novel", &dirs).unwrap();
        let n: i64 = c.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sidecar_detection() {
        let dir = std::env::temp_dir().join(format!("mp-side-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["a.mkv", "a.srt", "a.zh.ass", "ab.srt", "a.jpg", "b.lrc"] {
            std::fs::write(dir.join(n), "x").unwrap();
        }
        let v: Vec<String> = sidecars(&dir.join("a.mkv"))
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(v, vec!["a.srt", "a.zh.ass"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
