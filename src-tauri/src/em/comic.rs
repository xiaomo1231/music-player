//! 漫画排版规则（kind: comic）：阅读排版、页面处理（拆跨页 / 裁边 / 跳页）、导出新 CBZ
//!
//! - `spec`：正文检查、编译
//! - `view`：虚拟页表（阅读器、进度条缩略图、AI 翻译都按它的页号）
//! - `img`：图片处理
//! - `export`：导出新 CBZ
mod export;
mod img;
mod spec;
mod view;

use super::{Issue, Kind, Rule};
use crate::AppState;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use spec::{ReadPrefs, Spec};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use view::View;

/// 导出进度事件
pub const EXPORT_EVENT: &str = "em://comic-export";

/// 记录每本漫画是从哪个下载源下载的（match: source 用）
pub fn init(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS em_comic_sources(
            path TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn source_of(conn: &rusqlite::Connection, path: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    conn.query_row("SELECT source FROM em_comic_sources WHERE path=?1", [path], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

fn set_source(conn: &rusqlite::Connection, path: &str, source: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO em_comic_sources(path,source,at) VALUES(?1,?2,?3) ON CONFLICT(path) DO UPDATE SET source=?2, at=?3",
        rusqlite::params![path, source, crate::db::now()],
    )?;
    Ok(())
}

/// 下载源下载完成、整理入库后调用：记下来源；匹配的规则写了 `export.auto: true` 时自动导出排版好的 CBZ，
/// 替换刚下载的文件（刚下载的原文件可以重新下载，直接删除，不进回收站）
pub fn after_download(app: &AppHandle, path: &str, provider: &str) {
    if !matches!(crate::parse::ext(Path::new(path)).as_str(), "cbz" | "zip") {
        return;
    }
    let st = app.state::<AppState>();
    let id: Option<i64> = {
        let conn = st.db.lock();
        if let Err(e) = set_source(&conn, path, provider) {
            eprintln!("记录漫画来源失败：{e}");
        }
        conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).ok()
    };
    let Some(id) = id else { return };
    view::forget(Some(id));
    let Ok(t) = target(&st, id) else { return };
    let Some((_, spec)) = pick(&t, None) else { return };
    if !spec.export.auto {
        return;
    }
    match export_one(app, id, Mode::Delete, 0, 1) {
        Ok(_) => {
            let _ = app.emit("library://changed", ());
        }
        Err(e) => eprintln!("自动排版导出失败：{e}"),
    }
}

/// 导出后原文件怎么处理
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// 另存为新文件
    Side,
    /// 原文件进回收站，新文件放在原位置
    Trash,
    /// 原文件直接删除（刚下载的文件），新文件放在原位置
    Delete,
}

/// 检查正文；返回导入确认框里显示的摘要（每行一句）
pub fn check(rule: &Rule) -> Result<Vec<String>, Vec<Issue>> {
    spec::compile(rule).map(|s| s.summary)
}

/// 生效（on = true）或失效：清掉缓存，下次打开时按新规则排
pub fn apply(_app: &AppHandle, rule: &Rule, _on: bool) {
    SPECS.lock().remove(rule.id());
    view::forget(None);
}

/// 规则被删除后清理自己的数据
pub fn removed(_app: &AppHandle, id: &str) {
    SPECS.lock().remove(id);
    view::forget(None);
}

// ---------------------------------------------------------------- 规则 → Spec

/// 规则 id → (正文哈希, 编译好的 Spec)
type SpecCache = HashMap<String, (String, Arc<Spec>)>;
static SPECS: Lazy<Mutex<SpecCache>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn hash_of(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex()[..16].to_string()
}

fn spec_of(rule: &Rule) -> Option<Arc<Spec>> {
    let h = hash_of(&rule.text);
    if let Some((hh, s)) = SPECS.lock().get(rule.id()) {
        if *hh == h {
            return Some(s.clone());
        }
    }
    let s = Arc::new(spec::compile(rule).ok()?);
    SPECS.lock().insert(rule.id().to_string(), (h, s.clone()));
    Some(s)
}

// ---------------------------------------------------------------- 一本漫画

struct Target {
    path: PathBuf,
    title: String,
    series: String,
    format: String,
    /// 从哪个下载源下载的
    source: Option<String>,
}

fn target(st: &AppState, id: i64) -> Result<Target> {
    let conn = st.db.lock();
    let path = crate::library::file_path(&conn, id)?;
    let (title, series, format) = conn
        .query_row(
            "SELECT coalesce(f.custom_title,c.title),c.series,c.format FROM comics c JOIN files f ON f.id=c.file_id WHERE c.file_id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        )
        .unwrap_or_default();
    let source = source_of(&conn, &path);
    let path = PathBuf::from(path);
    let format = if format.is_empty() { crate::parse::ext(&path) } else { format };
    Ok(Target {
        path,
        title,
        series,
        format,
        source,
    })
}

/// 适用的规则：指定了 id 就用它（测试），否则按 id 顺序第一个匹配的
fn pick(t: &Target, only: Option<&str>) -> Option<(Arc<Rule>, Arc<Spec>)> {
    if t.format == "pdf" {
        return None;
    }
    let path = t.path.to_string_lossy();
    match only {
        Some(id) => {
            let r = crate::em::get(id).filter(|r| r.kind() == Kind::Comic)?;
            let s = spec_of(&r)?;
            Some((r, s))
        }
        None => crate::em::enabled(Kind::Comic)
            .into_iter()
            .filter_map(|r| spec_of(&r).map(|s| (r, s)))
            .find(|(_, s)| s.matches(&t.series, &path, t.source.as_deref())),
    }
}

/// 文件修改时间 + 大小
fn stamp(path: &Path) -> String {
    let m = std::fs::metadata(path).ok();
    let t = m
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t}-{}", m.map(|m| m.len()).unwrap_or(0))
}

/// 虚拟页表的版本：规则内容、文件、样张有没有
fn ver_of(rule: &Rule, spec: &Spec, path: &Path) -> String {
    let samples: String = spec
        .skip
        .iter()
        .filter_map(|s| match s {
            spec::Skip::Like(f, _) => Some(format!("{f}:{}", stamp(&Spec::sample_path(f)))),
            _ => None,
        })
        .collect();
    hash_of(&format!("{}\n{}\n{}\n{samples}", rule.id(), rule.text, stamp(path)))[..12].to_string()
}

fn view_for(t: &Target, id: i64, only: Option<&str>) -> Result<Option<(Arc<Rule>, Arc<View>)>> {
    let Some((rule, spec)) = pick(t, only) else { return Ok(None) };
    let ver = ver_of(&rule, &spec, &t.path);
    let v = view::cached((id, rule.id().to_string()), ver.clone(), || view::build(&t.path, rule.id(), spec, ver))?;
    Ok(Some((rule, v)))
}

/// 页数（按规则排过之后）。没有规则时和原来一样
pub fn page_count(st: &AppState, comic_id: i64) -> Result<usize> {
    let t = target(st, comic_id)?;
    if t.format == "pdf" {
        return Ok(crate::comic::pdf_page_count(&t.path).max(0) as usize);
    }
    if let Some((_, v)) = view_for(&t, comic_id, None)? {
        return Ok(v.len());
    }
    Ok(if t.path.is_dir() {
        crate::comic::folder_pages(&t.path)?.len()
    } else {
        crate::comic::zip_pages(&t.path)?.len()
    })
}

/// 读第 idx 页（按规则处理过）。没有规则时和 comic::read_page 一样
pub fn read_page(st: &AppState, comic_id: i64, idx: usize) -> Result<(Vec<u8>, String)> {
    read_page_with(st, comic_id, idx, None)
}

/// 同 read_page；only 指定只用某个规则（测试面板的缩略图）
pub fn read_page_with(st: &AppState, comic_id: i64, idx: usize, only: Option<&str>) -> Result<(Vec<u8>, String)> {
    let t = target(st, comic_id)?;
    let Some((_, v)) = view_for(&t, comic_id, only)? else {
        return crate::comic::read_page(&t.path, idx);
    };
    if !v.processed {
        return v.render(idx);
    }
    if let Some((b, m)) = view::lru_get(comic_id, &v.ver, idx) {
        return Ok(((*b).clone(), m));
    }
    let (b, m) = v.render(idx)?;
    let b = Arc::new(b);
    view::lru_put(comic_id, &v.ver, idx, b.clone(), m.clone());
    Ok((Arc::try_unwrap(b).unwrap_or_else(|a| (*a).clone()), m))
}

// ---------------------------------------------------------------- 命令

type R<T> = Result<T, String>;
fn es<T: ToString>(x: T) -> String {
    x.to_string()
}

#[derive(Serialize)]
pub struct RuleRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct ComicView {
    pub rule: Option<RuleRef>,
    pub read: Option<ReadPrefs>,
    pub pages: usize,
    pub processed: bool,
    /// 页面版本（变了就要重新加载图片）
    pub ver: String,
    pub skipped: usize,
    /// 这本是按这个规则导出的，页面不再处理
    pub marker: bool,
}

fn comic_view(st: &AppState, id: i64) -> Result<ComicView> {
    let t = target(st, id)?;
    match view_for(&t, id, None)? {
        Some((rule, v)) => Ok(ComicView {
            rule: Some(RuleRef {
                id: rule.id().to_string(),
                name: rule.header.name.clone(),
            }),
            read: v.spec.read.clone(),
            pages: v.len(),
            processed: v.processed,
            ver: v.ver.clone(),
            skipped: v.skipped.len(),
            marker: v.marker,
        }),
        None => Ok(ComicView {
            rule: None,
            read: None,
            pages: page_count(st, id)?,
            processed: false,
            ver: String::new(),
            skipped: 0,
            marker: false,
        }),
    }
}

/// 阅读器打开漫画时：适用的规则、阅读排版、排过之后的页数
#[tauri::command]
pub async fn em_comic_view(app: AppHandle, id: i64) -> R<ComicView> {
    tauri::async_runtime::spawn_blocking(move || comic_view(&app.state::<AppState>(), id))
        .await
        .map_err(es)?
        .map_err(es)
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
    current: String,
    page: usize,
    pages: usize,
}

#[derive(Serialize, Default)]
pub struct ExportReport {
    /// 写好的文件
    pub outputs: Vec<String>,
    /// 没导出的：说明
    pub failed: Vec<String>,
}

fn is_archive(p: &Path) -> bool {
    p.is_file() && matches!(crate::parse::ext(p).as_str(), "cbz" | "zip")
}

fn export_one(app: &AppHandle, id: i64, mode: Mode, done: usize, total: usize) -> Result<String> {
    let st = app.state::<AppState>();
    let t = target(&st, id)?;
    let name = if t.title.is_empty() {
        t.path.to_string_lossy().into_owned()
    } else {
        t.title.clone()
    };
    if t.format == "pdf" {
        return Err(anyhow!("「{name}」是 PDF，不能按规则导出"));
    }
    let Some((rule, v)) = view_for(&t, id, None)? else {
        return Err(anyhow!("「{name}」没有适用的排版规则"));
    };
    if v.marker {
        return Err(anyhow!("「{name}」已经按「{}」排过版了", rule.header.name));
    }
    let meta = export::Meta {
        rule_id: rule.id(),
        rule_version: &rule.header.version,
        title: &t.title,
        series: &t.series,
    };
    let side = export::target_for(&t.path);
    export::write(&v, &v.spec, &meta, &side, |page, pages| {
        let _ = app.emit(
            EXPORT_EVENT,
            Progress {
                done,
                total,
                current: name.clone(),
                page,
                pages,
            },
        );
    })?;
    if mode != Mode::Side && is_archive(&t.path) {
        if mode == Mode::Delete {
            std::fs::remove_file(&t.path).map_err(|e| anyhow!("「{name}」已另存为 {}，但原文件没能删除：{e}", side.display()))?;
        } else {
            trash::delete(&t.path).map_err(|e| anyhow!("「{name}」已另存为 {}，但原文件没能移到回收站：{e}", side.display()))?;
        }
        std::fs::rename(&side, &t.path)?;
        let p = t.path.to_string_lossy().into_owned();
        {
            let conn = st.db.lock();
            crate::commands::add_paths(&conn, &st.dirs, vec![p.clone()]);
            let _ = conn.execute("UPDATE comics SET page=0 WHERE file_id=?1", [id]);
        }
        view::forget(Some(id));
        return Ok(p);
    }
    let p = side.to_string_lossy().into_owned();
    {
        let conn = st.db.lock();
        if let Some(s) = &t.source {
            let _ = set_source(&conn, &p, s);
        }
        crate::commands::add_paths(&conn, &st.dirs, vec![p.clone()]);
    }
    Ok(p)
}

/// 按各自适用的规则导出新 CBZ。replace：原文件是 CBZ / ZIP 时移到回收站，新文件放在原位置
#[tauri::command]
pub async fn em_comic_export(app: AppHandle, ids: Vec<i64>, replace: bool) -> R<ExportReport> {
    let a = app.clone();
    let rep = tauri::async_runtime::spawn_blocking(move || {
        let mut rep = ExportReport::default();
        let total = ids.len();
        for (i, id) in ids.into_iter().enumerate() {
            let mode = if replace { Mode::Trash } else { Mode::Side };
            match export_one(&a, id, mode, i, total) {
                Ok(p) => rep.outputs.push(p),
                Err(e) => rep.failed.push(e.to_string()),
            }
        }
        let _ = a.emit(
            EXPORT_EVENT,
            Progress {
                done: total,
                total,
                current: String::new(),
                page: 0,
                pages: 0,
            },
        );
        rep
    })
    .await
    .map_err(es)?;
    if !rep.outputs.is_empty() {
        let _ = app.emit("library://changed", ());
    }
    if rep.outputs.is_empty() && !rep.failed.is_empty() {
        return Err(rep.failed.join("\n"));
    }
    Ok(rep)
}

// ---------------------------------------------------------------- 测试

#[derive(Serialize)]
struct TestPage {
    i: usize,
    src: usize,
    name: String,
    half: img::Half,
    trim: bool,
}

fn test_view(v: &View) -> Value {
    // 每页算一次裁边（多线程），结果留在缓存里，阅读时不用再算
    let n = v.len();
    if v.processed && v.spec.trim.is_some() {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
        let idx: Vec<usize> = (0..n).collect();
        let chunk = n.div_ceil(threads).max(1);
        std::thread::scope(|sc| {
            for part in idx.chunks(chunk) {
                sc.spawn(move || {
                    let Ok(mut book) = view::Book::open(&v.path) else { return };
                    for &i in part {
                        if let Ok((b, _)) = v.raw(&mut book, i) {
                            if let Some((im, _)) = img::decode(&b) {
                                v.crop_of(i, &im);
                            }
                        }
                    }
                });
            }
        });
    }
    let pages: Vec<TestPage> = v
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let trim = v.trimmed(i);
            TestPage {
                i,
                src: p.src,
                name: view::base_name(&v.names[p.src]).to_string(),
                half: p.half,
                trim,
            }
        })
        .collect();
    let split = pages.iter().filter(|p| p.half == img::Half::Left).count();
    let trimmed = pages.iter().filter(|p| p.trim).count();
    let skipped: Vec<Value> = v
        .skipped
        .iter()
        .map(|s| json!({"src": s.src, "name": view::base_name(&v.names[s.src]), "why": s.why}))
        .collect();
    json!({
        "before": v.names.len(),
        "after": n,
        "split": split,
        "trimmed": trimmed,
        "skipped": skipped.len(),
        "processed": v.processed,
        "marker": v.marker,
        "ver": v.ver,
        "pages": pages,
        "skipped_pages": skipped,
    })
}

/// 规则管理页的“测试”：arg = {comic_id} 返回排版后的页表摘要；不带 comic_id 时列出漫画（是否匹配这个规则）
pub async fn test(app: AppHandle, rule: Arc<Rule>, arg: Value) -> Result<Value, String> {
    let Some(id) = arg.get("comic_id").and_then(|v| v.as_i64()) else {
        let spec = spec_of(&rule).ok_or("规则有错误")?;
        let st = app.state::<AppState>();
        let (list, sources) = {
            let conn = st.db.lock();
            let list = crate::library::comics(&conn).map_err(es)?;
            let sources: HashMap<String, String> = conn
                .prepare("SELECT path, source FROM em_comic_sources")
                .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
                .unwrap_or_default();
            (list, sources)
        };
        let v: Vec<Value> = list
            .into_iter()
            .filter(|c| c.format != "pdf")
            .map(|c| {
                let matched = spec.matches(&c.series, &c.path, sources.get(&c.path).map(|s| s.as_str()));
                json!({"id": c.id, "title": c.title, "series": c.series, "matched": matched})
            })
            .collect();
        return Ok(json!({ "comics": v }));
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<Value> {
        let st = app.state::<AppState>();
        let t = target(&st, id)?;
        if t.format == "pdf" {
            return Err(anyhow!("PDF 不支持排版规则"));
        }
        let spec = spec_of(&rule).ok_or_else(|| anyhow!("规则有错误"))?;
        let matched = spec.matches(&t.series, &t.path.to_string_lossy(), t.source.as_deref());
        let (_, v) = view_for(&t, id, Some(rule.id()))?.ok_or_else(|| anyhow!("规则没有生效"))?;
        let mut out = test_view(&v);
        out["matched"] = json!(matched);
        out["title"] = json!(t.title);
        Ok(out)
    })
    .await
    .map_err(es)?
    .map_err(es)
}

#[cfg(test)]
mod tests;
