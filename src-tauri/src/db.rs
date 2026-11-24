//! SQLite 媒体库：建表、迁移、连接
use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

const SCHEMA_VERSION: i64 = 7;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub(crate) fn add_column(conn: &Connection, table: &str, col: &str, ty: &str) -> Result<()> {
    let exists: bool = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .flatten()
        .any(|c| c == col);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty};"))?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v < 1 {
        conn.execute_batch(SCHEMA_V1)?;
    }
    if v < 3 {
        // v2 / v3：加列时先看列在不在，重复执行也安全
        for (t, c) in [
            ("files", "custom_title"),
            ("files", "custom_cover"),
            ("series", "custom_title"),
            ("series", "custom_cover"),
            ("playlists", "cover"),
        ] {
            add_column(conn, t, c, "TEXT")?;
        }
        conn.execute_batch(SCHEMA_V2)?;
    }
    if v < 4 {
        // v4：电影标记；rules 记录这条视频按哪一版归类规则处理过，规则更新后启动时重新归类
        add_column(conn, "videos", "movie", "INTEGER NOT NULL DEFAULT 0")?;
        add_column(conn, "videos", "rules", "INTEGER NOT NULL DEFAULT 0")?;
    }
    if v < 5 {
        // v5：番剧 / 剧集卡片的竖版截帧（没有海报时用）
        add_column(conn, "series", "frame", "TEXT")?;
    }
    if v < 6 {
        // v6：歌曲的补全封面（同专辑共用或联网找到的），重新读标签时不会被冲掉；
        // net_lock = 1 表示用户手动选的，连内嵌封面也盖过；art_rules 记录按哪一版本地查找规则补过
        add_column(conn, "tracks", "net_cover", "TEXT")?;
        add_column(conn, "tracks", "net_hue", "INTEGER")?;
        add_column(conn, "tracks", "net_lock", "INTEGER NOT NULL DEFAULT 0")?;
        add_column(conn, "tracks", "art_rules", "INTEGER NOT NULL DEFAULT 0")?;
    }
    if v < 7 {
        // v7：0.6.3 之前“替换原文件”转换加密格式后，记录换成了 flac/mp3 但 encrypted 没清，歌一直留在“加密格式”里
        fix_encrypted_flags(conn)?;
    }
    conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION}"))?;
    Ok(())
}

/// 按路径扩展名重新算 encrypted，返回改了几条
pub(crate) fn fix_encrypted_flags(conn: &Connection) -> Result<usize> {
    let wrong: Vec<i64> = conn
        .prepare("SELECT id,path FROM files WHERE encrypted=1")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .flatten()
        .filter(|(_, p)| !crate::parse::is_encrypted(Path::new(p)))
        .map(|(id, _)| id)
        .collect();
    for id in &wrong {
        conn.execute("UPDATE files SET encrypted=0 WHERE id=?1", [id])?;
    }
    Ok(wrong.len())
}

/// v2：从媒体库移除的路径（自定义名字、封面的列由 add_column 加）
const SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS excluded(path TEXT PRIMARY KEY, at INTEGER NOT NULL);
"#;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS files(
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,              -- music / video / anime / manga / novel
  size INTEGER NOT NULL DEFAULT 0,
  mtime INTEGER NOT NULL DEFAULT 0,
  added_at INTEGER NOT NULL,
  fingerprint TEXT,
  missing INTEGER NOT NULL DEFAULT 0,
  encrypted INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS files_kind ON files(kind, missing);

CREATE TABLE IF NOT EXISTS tracks(
  file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  artist TEXT NOT NULL DEFAULT '',
  album TEXT NOT NULL DEFAULT '',
  album_artist TEXT NOT NULL DEFAULT '',
  track_no INTEGER, disc_no INTEGER, year INTEGER,
  genre TEXT NOT NULL DEFAULT '',
  duration REAL NOT NULL DEFAULT 0,
  bitrate INTEGER, sample_rate INTEGER, bit_depth INTEGER, channels INTEGER,
  format TEXT NOT NULL DEFAULT '',
  lossless INTEGER NOT NULL DEFAULT 0,
  cover TEXT,                       -- 缓存里的封面文件名
  hue INTEGER,                      -- 封面主色色相，用于背景光晕
  play_count INTEGER NOT NULL DEFAULT 0,
  skip_count INTEGER NOT NULL DEFAULT 0,
  last_played INTEGER,
  favorite INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS tracks_artist ON tracks(artist);
CREATE INDEX IF NOT EXISTS tracks_album ON tracks(album);

CREATE TABLE IF NOT EXISTS series(
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  kind TEXT NOT NULL,               -- anime / tv
  year INTEGER,
  poster TEXT,
  UNIQUE(kind, title)
);

CREATE TABLE IF NOT EXISTS videos(
  file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  series_id INTEGER REFERENCES series(id) ON DELETE SET NULL,
  season INTEGER, episode REAL,
  release_group TEXT, resolution TEXT,
  duration REAL NOT NULL DEFAULT 0,
  width INTEGER, height INTEGER, codec TEXT,
  thumb TEXT,
  position REAL NOT NULL DEFAULT 0,
  watched INTEGER NOT NULL DEFAULT 0,
  last_played INTEGER,
  subs TEXT NOT NULL DEFAULT 'none'  -- embedded / external / ai / none
);
CREATE INDEX IF NOT EXISTS videos_series ON videos(series_id, season, episode);

CREATE TABLE IF NOT EXISTS books(
  file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  author TEXT NOT NULL DEFAULT '',
  format TEXT NOT NULL,             -- txt / epub
  encoding TEXT,
  chapter INTEGER NOT NULL DEFAULT 0,
  offset REAL NOT NULL DEFAULT 0,
  chapters INTEGER NOT NULL DEFAULT 0,
  cover TEXT,
  last_read INTEGER
);

CREATE TABLE IF NOT EXISTS comics(
  file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  series TEXT NOT NULL DEFAULT '',
  format TEXT NOT NULL,             -- cbz / folder / pdf
  pages INTEGER NOT NULL DEFAULT 0,
  page INTEGER NOT NULL DEFAULT 0,
  cover TEXT,
  last_read INTEGER
);

CREATE TABLE IF NOT EXISTS playlists(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  sort INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS playlist_items(
  playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  pos INTEGER NOT NULL,
  PRIMARY KEY(playlist_id, file_id)
);

CREATE TABLE IF NOT EXISTS play_history(
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  at INTEGER NOT NULL,
  listened REAL NOT NULL,
  duration REAL NOT NULL,
  skipped INTEGER NOT NULL DEFAULT 0,
  source TEXT
);
CREATE INDEX IF NOT EXISTS history_at ON play_history(at);

CREATE TABLE IF NOT EXISTS scan_roots(
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  watch INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS settings(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs(
  id INTEGER PRIMARY KEY,
  input TEXT NOT NULL,
  output TEXT,
  preset TEXT NOT NULL,
  after TEXT NOT NULL,
  status TEXT NOT NULL,             -- queued / running / done / failed / canceled / undone
  progress REAL NOT NULL DEFAULT 0,
  error TEXT,
  backup TEXT,                      -- 替换时原文件去向（回收站），用于撤销
  created_at INTEGER NOT NULL,
  finished_at INTEGER
);

CREATE TABLE IF NOT EXISTS novel_rules(
  id INTEGER PRIMARY KEY,
  scope TEXT NOT NULL,              -- global / book:<file_id>
  kind TEXT NOT NULL,               -- regex / line / contains
  pattern TEXT NOT NULL,
  note TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  source TEXT NOT NULL DEFAULT 'user'  -- builtin / user / ai / freq
);
CREATE TABLE IF NOT EXISTS novel_cleanups(
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  chapter INTEGER NOT NULL,
  text TEXT NOT NULL,
  rule_id INTEGER,
  confidence TEXT NOT NULL,         -- hi / mid
  status TEXT NOT NULL              -- applied / pending / reverted
);

CREATE TABLE IF NOT EXISTS org_batches(
  id INTEGER PRIMARY KEY,
  created_at INTEGER NOT NULL,
  mode TEXT NOT NULL,
  undone INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS org_moves(
  batch_id INTEGER NOT NULL REFERENCES org_batches(id) ON DELETE CASCADE,
  src TEXT NOT NULL,
  dst TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS feedback(
  file_id INTEGER NOT NULL,
  kind TEXT NOT NULL,               -- like / not_song / not_artist / not_today
  at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS daily(
  day TEXT PRIMARY KEY,
  data TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS search_idx USING fts5(
  file_id UNINDEXED, kind UNINDEXED, title, sub, initials,
  tokenize = 'trigram'
);
"#;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn migration_is_repeatable() {
        let c = super::open_memory().unwrap();
        c.execute_batch("PRAGMA user_version=2").unwrap();
        super::migrate(&c).unwrap();
        super::migrate(&c).unwrap();
    }

    #[test]
    fn fixes_encrypted_flag_of_replaced_files() {
        let c = super::open_memory().unwrap();
        c.execute_batch(
            "INSERT INTO files(path,kind,size,mtime,added_at,encrypted) VALUES
             ('D:/a/1.flac','music',1,1,1,1),('D:/a/2.ncm','music',1,1,1,1),('D:/a/3.mp3','music',1,1,1,0);
             PRAGMA user_version=6;",
        )
        .unwrap();
        super::migrate(&c).unwrap();
        let enc: Vec<(String, i64)> = c
            .prepare("SELECT path,encrypted FROM files ORDER BY path")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(enc, vec![("D:/a/1.flac".into(), 0), ("D:/a/2.ncm".into(), 1), ("D:/a/3.mp3".into(), 0)]);
    }

    #[test]
    fn schema_creates() {
        let c = super::open_memory().unwrap();
        let n: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |r| r.get(0)).unwrap();
        assert!(n > 10);
    }
}
