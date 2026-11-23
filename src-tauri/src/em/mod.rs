//! .em 规则文件：用户自己添加的声明式规则（漫画排版 / 下载源 / 小说排版 / 整理文件）。
//! 规则是数据不是程序：引擎只按固定的几种动作执行，不运行脚本。
//!
//! - `format`：读文件、公共头部检查
//! - 本文件：规则存储（`<数据目录>/rules/<id>.em` + 表 `em_rules`）、内存注册表、导入 / 管理命令
//! - `comic` / `source` / `clean` / `organize`：各 kind 自己的检查、生效、测试
pub mod clean;
pub mod comic;
pub mod expr;
pub mod format;
pub mod net;
pub mod organize;
pub mod source;

use crate::AppState;
use anyhow::{anyhow, Result};
pub use format::{Issue, Kind, Rule};
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::RwLock;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;
fn es<T: ToString>(x: T) -> String {
    x.to_string()
}

/// 规则有变化（导入、启用、停用、删除、文件被改）时发给前端，payload 是 kind
pub const CHANGED: &str = "em://changed";

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS em_rules(
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            version TEXT NOT NULL DEFAULT '',
            author TEXT NOT NULL DEFAULT '',
            note TEXT NOT NULL DEFAULT '',
            hosts TEXT NOT NULL DEFAULT '[]',
            granted_hosts TEXT NOT NULL DEFAULT '[]',
            sha256 TEXT NOT NULL DEFAULT '',
            enabled INTEGER NOT NULL DEFAULT 1,
            installed_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    )?;
    comic::init(conn)?;
    Ok(())
}

static DIR: OnceCell<PathBuf> = OnceCell::new();
static APP: OnceCell<AppHandle> = OnceCell::new();

/// 规则文件目录
pub fn rules_dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(|| std::env::temp_dir().join("mp-rules"))
}

fn rule_path(id: &str) -> PathBuf {
    rules_dir().join(format!("{id}.em"))
}

/// 启动时：读入所有规则，已启用的交给各 kind 生效（例如下载源注册到 Provider 表）
pub fn setup(app: &AppHandle) {
    let st = app.state::<AppState>();
    let _ = DIR.set(st.dirs.data.join("rules"));
    let _ = APP.set(app.clone());
    let _ = std::fs::create_dir_all(rules_dir());
    let rows = {
        let conn = st.db.lock();
        load_rows(&conn).unwrap_or_default()
    };
    for r in rows {
        load_entry(&r);
    }
    for e in REG.read().values() {
        if e.active() {
            if let Some(rule) = &e.rule {
                apply(app, rule, true);
            }
        }
    }
}

// ---------------------------------------------------------------- 注册表

#[derive(Clone)]
struct Entry {
    rule: Option<Arc<Rule>>,
    /// 文件读不了、格式错、hosts 超出已授权范围时的说明
    problem: String,
    enabled: bool,
    granted: Vec<String>,
    mtime: Option<SystemTime>,
}

impl Entry {
    fn active(&self) -> bool {
        self.enabled && self.problem.is_empty() && self.rule.is_some()
    }
}

static REG: Lazy<RwLock<HashMap<String, Entry>>> = Lazy::new(|| RwLock::new(HashMap::new()));

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// 读一个规则文件，检查 kind 自己的格式和 hosts 授权
fn read_rule(path: &Path, granted: &[String]) -> (Option<Arc<Rule>>, String) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return (None, format!("读不到规则文件：{e}")),
    };
    let rule = match format::parse(&text).and_then(|r| check(&r).map(|_| r)) {
        Ok(r) => r,
        Err(is) => return (None, issues_text(&is)),
    };
    let extra: Vec<String> = rule.header.hosts.iter().filter(|h| !granted.contains(h)).cloned().collect();
    if !extra.is_empty() {
        return (
            Some(Arc::new(rule)),
            format!("规则文件新增了要访问的网站（{}），需要重新导入确认", extra.join("、")),
        );
    }
    (Some(Arc::new(rule)), String::new())
}

fn load_entry(row: &RuleRow) {
    let path = rule_path(&row.id);
    let granted: Vec<String> = row.granted_hosts.clone();
    let (rule, problem) = read_rule(&path, &granted);
    REG.write().insert(
        row.id.clone(),
        Entry {
            rule,
            problem,
            enabled: row.enabled,
            granted,
            mtime: mtime(&path),
        },
    );
}

/// 用户在编辑器里改了规则文件：下次用的时候重新读
fn refresh(id: &str) {
    let path = rule_path(id);
    let now = mtime(&path);
    let stale = REG.read().get(id).map(|e| e.mtime != now).unwrap_or(false);
    if !stale {
        return;
    }
    let (old, enabled, granted) = {
        let g = REG.read();
        let e = g.get(id).unwrap();
        (e.clone(), e.enabled, e.granted.clone())
    };
    let (rule, problem) = read_rule(&path, &granted);
    let entry = Entry {
        rule,
        problem,
        enabled,
        granted,
        mtime: now,
    };
    let (was, is) = (old.active(), entry.active());
    let kind = entry.rule.as_ref().or(old.rule.as_ref()).map(|r| r.kind());
    REG.write().insert(id.to_string(), entry.clone());
    if let Some(app) = APP.get() {
        if is {
            apply(app, entry.rule.as_ref().unwrap(), true);
        } else if was {
            apply(app, old.rule.as_ref().unwrap(), false);
        }
        if let Some(k) = kind {
            let _ = app.emit(CHANGED, k.as_str());
        }
    }
}

/// 某个 kind 所有生效中的规则（按 id 排序）。文件被改过会先重新读
pub fn enabled(kind: Kind) -> Vec<Arc<Rule>> {
    let ids: Vec<String> = REG.read().keys().cloned().collect();
    for id in &ids {
        refresh(id);
    }
    let g = REG.read();
    let mut v: Vec<Arc<Rule>> = g
        .values()
        .filter(|e| e.active())
        .filter_map(|e| e.rule.clone())
        .filter(|r| r.kind() == kind)
        .collect();
    v.sort_by(|a, b| a.id().cmp(b.id()));
    v
}

/// 按 id 取生效中的规则
pub fn get(id: &str) -> Option<Arc<Rule>> {
    refresh(id);
    REG.read().get(id).filter(|e| e.active()).and_then(|e| e.rule.clone())
}

// ---------------------------------------------------------------- 各 kind 的分发

/// kind 自己的检查；返回给导入确认框看的摘要
pub fn check(rule: &Rule) -> Result<Vec<String>, Vec<Issue>> {
    match rule.kind() {
        Kind::Comic => comic::check(rule),
        Kind::Source => source::check(rule),
        Kind::Clean => clean::check(rule),
        Kind::Organize => organize::check(rule),
    }
}

/// 生效 / 失效（导入、启用、停用、删除、启动时）
fn apply(app: &AppHandle, rule: &Rule, on: bool) {
    match rule.kind() {
        Kind::Comic => comic::apply(app, rule, on),
        Kind::Source => source::apply(app, rule, on),
        Kind::Clean => clean::apply(app, rule, on),
        Kind::Organize => organize::apply(app, rule, on),
    }
}

pub fn issues_text(is: &[Issue]) -> String {
    is.iter()
        .map(|i| {
            if i.line > 0 {
                format!("第 {} 行：{}", i.line, i.msg)
            } else {
                i.msg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------- 数据

#[derive(Serialize, Clone, Debug)]
pub struct RuleRow {
    pub id: String,
    pub kind: String,
    pub kind_label: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub note: String,
    pub hosts: Vec<String>,
    #[serde(skip)]
    pub granted_hosts: Vec<String>,
    pub enabled: bool,
    pub path: String,
    pub installed_at: i64,
    pub updated_at: i64,
    /// 有问题时的说明（规则不会生效）
    pub problem: String,
}

fn load_rows(conn: &Connection) -> Result<Vec<RuleRow>> {
    let mut st =
        conn.prepare("SELECT id,kind,name,version,author,note,hosts,granted_hosts,enabled,installed_at,updated_at FROM em_rules ORDER BY kind,name")?;
    let rows = st
        .query_map([], |r| {
            let kind: String = r.get(1)?;
            let id: String = r.get(0)?;
            Ok(RuleRow {
                path: rule_path(&id).to_string_lossy().into_owned(),
                id,
                kind_label: Kind::parse(&kind).map(|k| k.label()).unwrap_or("").to_string(),
                kind,
                name: r.get(2)?,
                version: r.get(3)?,
                author: r.get(4)?,
                note: r.get(5)?,
                hosts: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                granted_hosts: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
                enabled: r.get::<_, i64>(8)? != 0,
                installed_at: r.get(9)?,
                updated_at: r.get(10)?,
                problem: String::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn with_problem(mut r: RuleRow) -> RuleRow {
    refresh(&r.id);
    if let Some(e) = REG.read().get(&r.id) {
        r.problem = e.problem.clone();
        if let Some(rule) = &e.rule {
            // 用户改过文件：显示文件里的新名字、版本
            r.name = rule.header.name.clone();
            r.version = rule.header.version.clone();
            r.hosts = rule.header.hosts.clone();
        }
    }
    r
}

// ---------------------------------------------------------------- 导入

#[derive(Serialize, Debug)]
pub struct Preview {
    /// 能不能导入（有 issues 时为 false）
    pub ok: bool,
    pub issues: Vec<Issue>,
    pub id: String,
    pub kind: String,
    pub kind_label: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub note: String,
    pub hosts: Vec<String>,
    /// 其中的本机 / 内网地址
    pub lan_hosts: Vec<String>,
    pub login: String,
    /// kind 自己给的摘要（作用范围、会做什么）
    pub summary: Vec<String>,
    /// 已经装过同 id 的规则时：旧版本
    pub installed_version: Option<String>,
    /// 比已装版本多出来的网站
    pub new_hosts: Vec<String>,
}

/// 看一下规则文件（导入确认框用），不改任何东西
pub fn inspect_text(text: &str, installed: Option<(String, Vec<String>)>) -> Preview {
    let mut p = Preview {
        ok: false,
        issues: vec![],
        id: String::new(),
        kind: String::new(),
        kind_label: String::new(),
        name: String::new(),
        version: String::new(),
        author: String::new(),
        note: String::new(),
        hosts: vec![],
        lan_hosts: vec![],
        login: "none".into(),
        summary: vec![],
        installed_version: None,
        new_hosts: vec![],
    };
    let rule = match format::parse(text) {
        Ok(r) => r,
        Err(is) => {
            p.issues = is;
            return p;
        }
    };
    let h = &rule.header;
    p.id = h.id.clone();
    p.kind = h.kind.as_str().into();
    p.kind_label = h.kind.label().into();
    p.name = h.name.clone();
    p.version = h.version.clone();
    p.author = h.author.clone();
    p.note = h.note.clone();
    p.hosts = h.hosts.clone();
    p.lan_hosts = h.hosts.iter().filter(|x| format::is_lan_host(x)).cloned().collect();
    p.login = serde_json::to_value(h.login)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    match check(&rule) {
        Ok(s) => p.summary = s,
        Err(is) => p.issues = is,
    }
    if let Some((ver, granted)) = installed {
        p.new_hosts = h.hosts.iter().filter(|x| !granted.contains(x)).cloned().collect();
        p.installed_version = Some(ver);
    }
    p.ok = p.issues.is_empty();
    p
}

fn installed_of(conn: &Connection, id: &str) -> Result<Option<(String, Vec<String>)>> {
    Ok(conn
        .query_row("SELECT version, granted_hosts FROM em_rules WHERE id=?1", [id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .optional()?
        .map(|(v, g)| (v, serde_json::from_str(&g).unwrap_or_default())))
}

fn read_file(path: &str) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("读不到文件：{e}"))?;
    if bytes.len() > format::MAX_BYTES {
        return Err(anyhow!("规则文件太大（上限 {} KB）", format::MAX_BYTES / 1024));
    }
    String::from_utf8(bytes).map_err(|_| anyhow!("规则文件要用 UTF-8 编码保存"))
}

#[tauri::command]
pub fn em_inspect(st: State<AppState>, path: String) -> R<Preview> {
    let text = read_file(&path).map_err(es)?;
    let id = format::parse(&text).map(|r| r.header.id).unwrap_or_default();
    let installed = if id.is_empty() { None } else { installed_of(&st.db.lock(), &id).map_err(es)? };
    Ok(inspect_text(&text, installed))
}

/// 导入（或更新）规则：复制到规则目录、记录已授权的网站、生效
#[tauri::command]
pub fn em_install(app: AppHandle, st: State<AppState>, path: String) -> R<RuleRow> {
    let text = read_file(&path).map_err(es)?;
    let rule = format::parse(&text).map_err(|is| issues_text(&is))?;
    check(&rule).map_err(|is| issues_text(&is))?;
    let id = rule.id().to_string();
    // 先停掉旧版本
    let old = REG.read().get(&id).cloned();
    if let Some(e) = &old {
        if e.active() {
            apply(&app, e.rule.as_ref().unwrap(), false);
        }
    }
    std::fs::create_dir_all(rules_dir()).map_err(es)?;
    let dst = rule_path(&id);
    let same_file = std::fs::canonicalize(&path).ok() == std::fs::canonicalize(&dst).ok();
    if !same_file {
        std::fs::write(&dst, &rule.text).map_err(|e| format!("保存规则失败：{e}"))?;
    }
    let h = &rule.header;
    let hosts = serde_json::to_string(&h.hosts).unwrap_or_else(|_| "[]".into());
    let sha = hex::encode(Sha256::digest(rule.text.as_bytes()));
    let t = now();
    {
        let conn = st.db.lock();
        conn.execute(
            "INSERT INTO em_rules(id,kind,name,version,author,note,hosts,granted_hosts,sha256,enabled,installed_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8,1,?9,?9)
             ON CONFLICT(id) DO UPDATE SET kind=?2,name=?3,version=?4,author=?5,note=?6,hosts=?7,granted_hosts=?7,sha256=?8,enabled=1,updated_at=?9",
            params![id, h.kind.as_str(), h.name, h.version, h.author, h.note, hosts, sha, t],
        )
        .map_err(es)?;
    }
    let rule = Arc::new(rule);
    REG.write().insert(
        id.clone(),
        Entry {
            rule: Some(rule.clone()),
            problem: String::new(),
            enabled: true,
            granted: rule.header.hosts.clone(),
            mtime: mtime(&dst),
        },
    );
    apply(&app, &rule, true);
    if rule.kind() == Kind::Source {
        // 导入下载源规则后直接加进“自动获取 → 我的来源”，不用再手动添加
        if let Err(e) = source::add_to_sources(&app, &rule) {
            eprintln!("下载源规则加入来源失败：{e}");
        }
    }
    let _ = app.emit(CHANGED, rule.kind().as_str());
    let conn = st.db.lock();
    let row = load_rows(&conn).map_err(es)?.into_iter().find(|r| r.id == id).ok_or("导入失败")?;
    Ok(with_problem(row))
}

#[tauri::command]
pub fn em_list(st: State<AppState>) -> R<Vec<RuleRow>> {
    let rows = load_rows(&st.db.lock()).map_err(es)?;
    Ok(rows.into_iter().map(with_problem).collect())
}

#[tauri::command]
pub fn em_set_enabled(app: AppHandle, st: State<AppState>, id: String, enabled: bool) -> R<()> {
    st.db
        .lock()
        .execute("UPDATE em_rules SET enabled=?2 WHERE id=?1", params![id, enabled as i64])
        .map_err(es)?;
    refresh(&id);
    let entry = {
        let mut g = REG.write();
        let Some(e) = g.get_mut(&id) else {
            return Err("找不到这个规则".into());
        };
        let was = e.active();
        e.enabled = enabled;
        (was, e.clone())
    };
    let (was, e) = entry;
    if let Some(rule) = &e.rule {
        if e.active() && !was {
            apply(&app, rule, true);
        } else if was && !e.active() {
            apply(&app, rule, false);
        }
        let _ = app.emit(CHANGED, rule.kind().as_str());
    }
    Ok(())
}

#[tauri::command]
pub fn em_delete(app: AppHandle, st: State<AppState>, id: String) -> R<()> {
    let e = REG.write().remove(&id);
    if let Some(e) = &e {
        if let Some(rule) = &e.rule {
            if e.active() {
                apply(&app, rule, false);
            }
            match rule.kind() {
                Kind::Comic => comic::removed(&app, &id),
                Kind::Source => source::removed(&app, &id),
                Kind::Clean => clean::removed(&app, &id),
                Kind::Organize => organize::removed(&app, &id),
            }
            let _ = app.emit(CHANGED, rule.kind().as_str());
        }
    }
    st.db.lock().execute("DELETE FROM em_rules WHERE id=?1", [&id]).map_err(es)?;
    let _ = std::fs::remove_file(rule_path(&id));
    Ok(())
}

/// 导出规则文件（原样复制，包括用户在编辑器里改过的内容）
#[tauri::command]
pub fn em_export(id: String, path: String) -> R<()> {
    std::fs::copy(rule_path(&id), &path).map_err(|e| format!("导出失败：{e}"))?;
    Ok(())
}

/// 用系统默认程序（记事本等）打开规则文件，改完保存后下次使用时自动重新读取
#[tauri::command]
pub fn em_open(id: String) -> R<()> {
    let p = rule_path(&id);
    if !p.exists() {
        return Err("规则文件不存在".into());
    }
    tauri_plugin_opener::open_path(&p, None::<&str>).map_err(|e| format!("打不开：{e}"))
}

/// 测试规则：各 kind 自己定义参数和返回（source：按 test.query 跑一遍；comic：用一本漫画预览）
#[tauri::command]
pub async fn em_test(app: AppHandle, id: String, arg: Value) -> R<Value> {
    refresh(&id);
    let (rule, problem) = {
        let g = REG.read();
        let e = g.get(&id).ok_or("找不到这个规则")?;
        (e.rule.clone(), e.problem.clone())
    };
    let rule = rule.ok_or(problem)?;
    match rule.kind() {
        Kind::Comic => comic::test(app, rule, arg).await,
        Kind::Source => source::test(app, rule, arg).await,
        Kind::Clean => clean::test(app, rule, arg).await,
        Kind::Organize => organize::test(app, rule, arg).await,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn inspect_reports_new_hosts_and_lan() {
        let text = "em: 1\nkind: source\nid: nas\nname: 我的 NAS\nversion: 1.1.0\nhosts: [archive.org, nas.local]\nsearch: {}\nresolve: {}\n";
        let p = inspect_text(text, Some(("1.0.0".into(), vec!["archive.org".into()])));
        assert_eq!(p.lan_hosts, vec!["nas.local".to_string()]);
        assert_eq!(p.new_hosts, vec!["nas.local".to_string()]);
        assert_eq!(p.installed_version.as_deref(), Some("1.0.0"));
        assert_eq!(p.kind_label, "下载源");
    }

    #[test]
    fn inspect_bad_file() {
        let p = inspect_text("em: 1\nkind: comic\n", None);
        assert!(!p.ok);
        assert!(!p.issues.is_empty());
    }

    #[test]
    fn issues_text_has_lines() {
        let s = issues_text(&[Issue::at(3, "a"), Issue::new("b")]);
        assert_eq!(s, "第 3 行：a\nb");
    }
}
