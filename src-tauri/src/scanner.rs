//! 扫描与实时监控
use crate::indexer::{self, Dirs, Outcome};
use crate::{comic, db, parse};
use anyhow::Result;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

const SKIP_DIRS: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "node_modules",
    ".git",
    "appdata",
    "steamapps",
    "epic games",
    "wpsystem",
    "windowsapps",
    "msocache",
    "$windows.~bt",
    "recovery",
];

pub fn skip_dir(name: &str) -> bool {
    let n = name.to_lowercase();
    SKIP_DIRS.contains(&n.as_str()) || n.starts_with('.') && n.len() > 1
}

#[derive(Serialize, Clone, Default)]
pub struct Progress {
    pub phase: String, // enum / read / done / canceled
    pub path: String,
    pub found: [u64; 5], // music video anime manga novel
    pub total: u64,
    pub done: u64,
    pub added: u64,
    pub updated: u64,
    pub missing: u64,
    pub recent: Vec<(String, String)>,
}

fn kind_idx(k: &str) -> usize {
    match k {
        "music" => 0,
        "video" => 1,
        "anime" => 2,
        "manga" => 3,
        _ => 4,
    }
}

#[derive(Default)]
pub struct ScanState {
    pub running: AtomicBool,
    pub cancel: AtomicBool,
    pub last: Mutex<Progress>,
}

/// 枚举一个根目录下所有媒体文件（只看扩展名，不打开文件）
pub fn enumerate(root: &Path, cancel: &AtomicBool, mut on: impl FnMut(&Path, &'static str)) {
    let walker = jwalk::WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(|_, _, _, children| {
            children.retain(|e| match e {
                Ok(e) if e.file_type().is_dir() => !skip_dir(&e.file_name().to_string_lossy()),
                Ok(_) => true,
                Err(_) => false,
            });
        });
    for e in walker.into_iter().flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let p = e.path();
        if e.file_type().is_dir() {
            if comic::is_comic_folder(&p) {
                on(&p, "manga");
            }
            continue;
        }
        if let Some(k) = parse::classify(&p) {
            // 漫画文件夹里的单张图片不算
            on(&p, k);
        }
    }
}

pub fn run_scan(app: AppHandle, db_path: PathBuf, dirs: Arc<Dirs>, roots: Vec<PathBuf>, state: Arc<ScanState>) -> Result<Progress> {
    let conn = db::open(&db_path)?;
    let mut pr = Progress {
        phase: "enum".into(),
        ..Default::default()
    };
    let mut items: Vec<(PathBuf, &'static str)> = vec![];
    let mut last_emit = std::time::Instant::now();
    for root in &roots {
        enumerate(root, &state.cancel, |p, k| {
            pr.found[kind_idx(k)] += 1;
            items.push((p.to_path_buf(), k));
            if last_emit.elapsed().as_millis() > 120 {
                pr.path = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
                let _ = app.emit("scan://progress", &pr);
                last_emit = std::time::Instant::now();
            }
        });
    }
    // 图片文件夹已算作漫画时，里面的 cbz 不重复（只去掉文件夹本身）
    pr.total = items.len() as u64;
    pr.phase = "read".into();
    let seen: HashSet<String> = items.iter().map(|(p, _)| p.to_string_lossy().to_string()).collect();
    for chunk in items.chunks(200) {
        if state.cancel.load(Ordering::Relaxed) {
            pr.phase = "canceled".into();
            let _ = app.emit("scan://progress", &pr);
            return Ok(pr);
        }
        conn.execute_batch("BEGIN")?;
        for (p, k) in chunk {
            match indexer::index_file(&conn, p, k, &dirs) {
                Ok(Outcome::Added) => {
                    pr.added += 1;
                    pr.recent
                        .insert(0, (k.to_string(), p.file_name().unwrap_or_default().to_string_lossy().to_string()));
                    pr.recent.truncate(8);
                }
                Ok(Outcome::Updated) => pr.updated += 1,
                Ok(Outcome::Unchanged) => {}
                Err(e) => log::warn(&format!("索引失败 {}: {e}", p.display())),
            }
            pr.done += 1;
            if last_emit.elapsed().as_millis() > 120 {
                pr.path = p.to_string_lossy().to_string();
                let _ = app.emit("scan://progress", &pr);
                last_emit = std::time::Instant::now();
            }
        }
        conn.execute_batch("COMMIT")?;
    }
    // 根目录下没再见到的文件标记为缺失
    for root in &roots {
        let prefix = root.to_string_lossy().to_string();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT id,path FROM files WHERE missing=0 AND path LIKE ?1 || '%'")?
            .query_map([&prefix], |r| Ok((r.get(0)?, r.get(1)?)))?
            .flatten()
            .collect();
        for (id, p) in rows {
            if !seen.contains(&p) && !Path::new(&p).exists() {
                conn.execute("UPDATE files SET missing=1 WHERE id=?1", [id])?;
                pr.missing += 1;
            }
        }
    }
    pr.phase = "done".into();
    let _ = app.emit("scan://progress", &pr);
    let _ = app.emit("library://changed", ());
    Ok(pr)
}

pub mod log {
    pub fn warn(s: &str) {
        eprintln!("[warn] {s}");
    }
}

/// 实时监控：新文件几秒内入库，删除的标记缺失
pub struct Watcher {
    _inner: Option<notify::RecommendedWatcher>,
}

pub fn start_watch(app: AppHandle, db_path: PathBuf, dirs: Arc<Dirs>, roots: Vec<PathBuf>) -> Result<Watcher> {
    use notify::{EventKind, RecursiveMode, Watcher as _};
    if roots.is_empty() {
        return Ok(Watcher { _inner: None });
    }
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut w = notify::recommended_watcher(tx)?;
    for r in &roots {
        if r.exists() {
            w.watch(r, RecursiveMode::Recursive)?;
        }
    }
    std::thread::spawn(move || {
        let Ok(conn) = db::open(&db_path) else { return };
        let mut pending: std::collections::HashMap<PathBuf, std::time::Instant> = Default::default();
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(800)) {
                Ok(Ok(ev)) => match ev.kind {
                    EventKind::Remove(_) => {
                        for p in ev.paths {
                            let _ = indexer::mark_missing(&conn, &p);
                        }
                        let _ = app.emit("library://changed", ());
                    }
                    EventKind::Create(_) | EventKind::Modify(_) => {
                        for p in ev.paths {
                            pending.insert(p, std::time::Instant::now());
                        }
                    }
                    _ => {}
                },
                Ok(Err(_)) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
            // 文件写完（2 秒没有新变化）再入库，避免读到下载到一半的文件
            let ready: Vec<PathBuf> = pending.iter().filter(|(_, t)| t.elapsed().as_secs() >= 2).map(|(p, _)| p.clone()).collect();
            let mut changed = false;
            for p in ready {
                pending.remove(&p);
                if p.is_file() {
                    if let Some(k) = parse::classify(&p) {
                        if indexer::index_file(&conn, &p, k, &dirs).is_ok() {
                            changed = true;
                        }
                    }
                } else if p.is_dir() && comic::is_comic_folder(&p) && indexer::index_file(&conn, &p, "manga", &dirs).is_ok() {
                    changed = true;
                }
            }
            if changed {
                let _ = app.emit("library://changed", ());
            }
        }
    });
    Ok(Watcher { _inner: Some(w) })
}

/// 列出本机磁盘（扫描向导用）
#[derive(Serialize)]
pub struct Drive {
    pub root: String,
    pub label: String,
    pub total: u64,
    pub free: u64,
}

pub fn drives() -> Vec<Drive> {
    let mut v = vec![];
    for c in b'C'..=b'Z' {
        let root = format!("{}:\\", c as char);
        if !Path::new(&root).exists() {
            continue;
        }
        let (total, free) = disk_space(&root).unwrap_or((0, 0));
        v.push(Drive {
            label: volume_label(&root).unwrap_or_default(),
            root,
            total,
            free,
        });
    }
    v
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn disk_space(root: &str) -> Option<(u64, u64)> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let w = wide(root);
    let (mut a, mut t, mut f) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut a, &mut t, &mut f) };
    (ok != 0).then_some((t, f))
}

#[cfg(windows)]
fn volume_label(root: &str) -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumeInformationW(root: *const u16, name: *mut u16, n: u32, serial: *mut u32, maxc: *mut u32, flags: *mut u32, fs: *mut u16, fsn: u32) -> i32;
    }
    let w = wide(root);
    let mut buf = [0u16; 261];
    let ok = unsafe {
        GetVolumeInformationW(
            w.as_ptr(),
            buf.as_mut_ptr(),
            261,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..n]))
}

#[cfg(not(windows))]
fn disk_space(_: &str) -> Option<(u64, u64)> {
    None
}
#[cfg(not(windows))]
fn volume_label(_: &str) -> Option<String> {
    None
}
