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
    Ok(file_list(files_of(conn, kind, id)?))
}

// ---------- 批量 ----------

/// 多个条目对应的 (file_id, 路径)，去重并保持顺序
pub fn files_of_many(conn: &Connection, kind: &str, ids: &[i64]) -> Result<Vec<(i64, String)>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for &id in ids {
        for f in files_of(conn, kind, id)? {
            if seen.insert(f.0) {
                out.push(f);
            }
        }
    }
    Ok(out)
}

pub fn list_files_many(conn: &Connection, kind: &str, ids: &[i64]) -> Result<FileList> {
    Ok(file_list(files_of_many(conn, kind, ids)?))
}

/// 在一个事务里执行（几千条时快很多，出错时整体回滚）
fn in_tx<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let tx = conn.unchecked_transaction()?;
    let r = f()?;
    tx.commit()?;
    Ok(r)
}

/// 批量从媒体库移除，返回被移除的文件路径（撤销时用它们重新入库）
pub fn remove_many(conn: &Connection, kind: &str, ids: &[i64]) -> Result<Vec<String>> {
    in_tx(conn, || {
        let files = files_of_many(conn, kind, ids)?;
        drop_rows(conn, &files, true)?;
        Ok(files.into_iter().map(|f| f.1).collect())
    })
}

/// 要移入回收站的路径：存在的媒体文件 + 同名配套文件（去重）
pub fn trash_paths(files: &[(i64, String)]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = vec![];
    for (_, p) in files {
        let pb = PathBuf::from(p);
        for x in sidecars(&pb).into_iter().chain(pb.exists().then_some(pb.clone())) {
            if seen.insert(x.clone()) {
                paths.push(x);
            }
        }
    }
    paths
}

/// 批量收藏 / 取消收藏歌曲，返回改动的条数
pub fn set_favorite_many(conn: &Connection, ids: &[i64], favorite: bool) -> Result<usize> {
    in_tx(conn, || {
        let mut n = 0;
        let mut st = conn.prepare("UPDATE tracks SET favorite=?1 WHERE file_id=?2 AND favorite<>?1")?;
        for id in ids {
            n += st.execute(params![favorite as i64, id])?;
        }
        Ok(n)
    })
}

fn file_list(files: Vec<(i64, String)>) -> FileList {
    let mut extras: Vec<String> = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut size = 0u64;
    let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    for (_, p) in &files {
        let pb = Path::new(p);
        size += len(pb);
        for x in sidecars(pb) {
            if seen.insert(x.clone()) {
                size += len(&x);
                extras.push(x.to_string_lossy().to_string());
            }
        }
    }
    FileList {
        files: files.into_iter().map(|f| f.1).collect(),
        extras,
        size,
    }
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
    item_delete_many(app, st, kind, vec![id])
}

/// 批量列出要删除的文件（给确认框用）
#[tauri::command]
pub fn item_files_many(st: State<AppState>, kind: String, ids: Vec<i64>) -> R<FileList> {
    list_files_many(&st.db.lock(), &kind, &ids).map_err(e)
}

/// 批量从媒体库移除，返回被移除的文件路径（前端撤销时交给 add_files 重新入库）
#[tauri::command]
pub fn item_remove_many(app: AppHandle, st: State<AppState>, kind: String, ids: Vec<i64>) -> R<Vec<String>> {
    let v = remove_many(&st.db.lock(), &kind, &ids).map_err(e)?;
    let _ = app.emit("library://changed", ());
    Ok(v)
}

/// 批量删除：所有文件和同名字幕 / 歌词一次性移入回收站，然后删掉库里的记录
#[tauri::command]
pub fn item_delete_many(app: AppHandle, st: State<AppState>, kind: String, ids: Vec<i64>) -> R<usize> {
    let files = files_of_many(&st.db.lock(), &kind, &ids).map_err(e)?;
    let paths = trash_paths(&files);
    if !paths.is_empty() {
        trash::delete_all(&paths).map_err(|err| format!("移入回收站失败（文件可能正在被使用）：{err}"))?;
    }
    let conn = st.db.lock();
    in_tx(&conn, || drop_rows(&conn, &files, false)).map_err(e)?;
    drop(conn);
    let _ = app.emit("library://changed", ());
    Ok(files.len())
}

/// 批量收藏 / 取消收藏歌曲
#[tauri::command]
pub fn item_favorite_many(st: State<AppState>, ids: Vec<i64>, favorite: bool) -> R<usize> {
    set_favorite_many(&st.db.lock(), &ids, favorite).map_err(e)
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

    fn add_track(c: &Connection, path: &str) -> i64 {
        c.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,'music',1,1,1)", [path])
            .unwrap();
        let id = c.last_insert_rowid();
        c.execute("INSERT INTO tracks(file_id,title,duration,format) VALUES(?1,'t',1,'MP3')", [id])
            .unwrap();
        id
    }

    #[test]
    fn remove_many_returns_paths_and_excludes() {
        let c = db::open_memory().unwrap();
        let (t, s, _) = seed(&c);
        let t2 = add_track(&c, "C:\\b.mp3");
        let keep = add_track(&c, "C:\\keep.mp3");
        // 重复的 id 只算一次
        let v = remove_many(&c, "track", &[t, t2, t]).unwrap();
        assert_eq!(v, vec!["C:\\a.flac".to_string(), "C:\\b.mp3".to_string()]);
        let left: Vec<i64> = c
            .prepare("SELECT id FROM files WHERE kind='music'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, vec![keep]);
        let n: i64 = c.query_row("SELECT count(*) FROM excluded", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        // 整部番剧
        assert_eq!(remove_many(&c, "series", &[s]).unwrap().len(), 3);
        let n: i64 = c.query_row("SELECT count(*) FROM series", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        // 未知类型整体失败，不会删掉任何东西
        assert!(remove_many(&c, "nope", &[keep]).is_err());
        assert!(remove_many(&c, "track", &[]).unwrap().is_empty());
    }

    #[test]
    fn remove_many_rolls_back_on_error() {
        let c = db::open_memory().unwrap();
        let a = add_track(&c, "C:\\a.mp3");
        // 第二个 id 不存在不算错；类型错才回滚
        assert_eq!(remove_many(&c, "track", &[a, 99999]).unwrap().len(), 1);
        let b = add_track(&c, "C:\\b.mp3");
        assert!(remove_many(&c, "bad", &[b]).is_err());
        let n: i64 = c.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn files_many_lists_sidecars_once() {
        let c = db::open_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("mp-many-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (n, body) in [("a.mp3", "12345"), ("a.lrc", "12"), ("b.mp3", "123"), ("b.zh.lrc", "1"), ("c.mp3", "1")] {
            std::fs::write(dir.join(n), body).unwrap();
        }
        let p = |n: &str| dir.join(n).to_string_lossy().to_string();
        let a = add_track(&c, &p("a.mp3"));
        let b = add_track(&c, &p("b.mp3"));
        let gone = add_track(&c, &p("missing.mp3"));
        let fl = list_files_many(&c, "track", &[a, b, a, gone]).unwrap();
        assert_eq!(fl.files, vec![p("a.mp3"), p("b.mp3"), p("missing.mp3")]);
        assert_eq!(fl.extras, vec![p("a.lrc"), p("b.zh.lrc")]);
        assert_eq!(fl.size, 5 + 2 + 3 + 1);
        // 回收站列表：不存在的文件跳过，配套文件带上
        let files = files_of_many(&c, "track", &[a, b, gone]).unwrap();
        let tp: Vec<String> = trash_paths(&files)
            .iter()
            .map(|x| x.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(tp, vec!["a.lrc", "a.mp3", "b.zh.lrc", "b.mp3"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn favorite_many_sets_value() {
        let c = db::open_memory().unwrap();
        let a = add_track(&c, "C:\\a.mp3");
        let b = add_track(&c, "C:\\b.mp3");
        crate::library::toggle_favorite(&c, a).unwrap();
        assert_eq!(set_favorite_many(&c, &[a, b], true).unwrap(), 1);
        assert!(crate::library::track(&c, b).unwrap().unwrap().favorite);
        assert_eq!(set_favorite_many(&c, &[a, b], false).unwrap(), 2);
        assert!(!crate::library::track(&c, a).unwrap().unwrap().favorite);
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

/// 清空媒体库（不动硬盘上的任何媒体文件）。full=true 时连设置也恢复默认（AI 密钥在凭据管理器里，另行保留）
pub fn reset_library(conn: &Connection, full: bool) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0))?;
    let mut tables = vec![
        "playlist_items",
        "playlists",
        "play_history",
        "files",
        "series",
        "scan_roots",
        "jobs",
        "excluded",
        "novel_cleanups",
        "org_moves",
        "org_batches",
        "feedback",
        "daily",
        "search_idx",
    ];
    // 自动获取模块的表（可能不存在）
    for t in ["fetch_tasks", "fetch_subs", "fetch_sources"] {
        let exists: bool = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1", [t], |r| r.get::<_, i64>(0))? > 0;
        if exists {
            tables.push(t);
        }
    }
    conn.execute_batch("BEGIN")?;
    let r = (|| -> Result<()> {
        for t in &tables {
            conn.execute(&format!("DELETE FROM {t}"), [])?;
        }
        // 用户和 AI 生成的去广告规则一起清掉，内置规则保留
        conn.execute("DELETE FROM novel_rules WHERE source <> 'builtin'", [])?;
        if full {
            conn.execute("DELETE FROM settings", [])?;
        }
        Ok(())
    })();
    match r {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(err) => {
            conn.execute_batch("ROLLBACK")?;
            return Err(err);
        }
    }
    Ok(n as usize)
}

#[tauri::command]
pub fn library_reset(app: AppHandle, st: State<AppState>, full: bool) -> R<usize> {
    let n = reset_library(&st.db.lock(), full).map_err(e)?;
    // 封面和缩略图缓存一起清掉
    for d in [&st.dirs.covers, &st.dirs.thumbs] {
        if let Ok(rd) = std::fs::read_dir(d) {
            for f in rd.flatten() {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
    crate::commands::restart_watcher(&app);
    let _ = app.emit("library://changed", ());
    Ok(n)
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    #[test]
    fn reset_clears_everything_but_builtin_rules() {
        let c = db::open_memory().unwrap();
        c.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES('D:/t/a.flac','music',1,1,1)", [])
            .unwrap();
        c.execute("INSERT INTO scan_roots(path) VALUES('D:/t')", []).unwrap();
        c.execute("INSERT INTO playlists(name,created_at) VALUES('p',1)", []).unwrap();
        c.execute("INSERT INTO settings(key,value) VALUES('watch','true')", []).unwrap();
        c.execute("INSERT INTO novel_rules(scope,kind,pattern,source) VALUES('global','regex','x','builtin')", [])
            .unwrap();
        c.execute("INSERT INTO novel_rules(scope,kind,pattern,source) VALUES('global','regex','y','user')", [])
            .unwrap();
        assert_eq!(reset_library(&c, false).unwrap(), 1);
        for t in ["files", "scan_roots", "playlists"] {
            let n: i64 = c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0, "{t}");
        }
        let rules: i64 = c.query_row("SELECT count(*) FROM novel_rules", [], |r| r.get(0)).unwrap();
        assert_eq!(rules, 1);
        let s: i64 = c.query_row("SELECT count(*) FROM settings", [], |r| r.get(0)).unwrap();
        assert_eq!(s, 1);
        reset_library(&c, true).unwrap();
        let s: i64 = c.query_row("SELECT count(*) FROM settings", [], |r| r.get(0)).unwrap();
        assert_eq!(s, 0);
    }
}
