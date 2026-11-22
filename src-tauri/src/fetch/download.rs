//! 下载引擎：统一队列（同时下载数、暂停 / 继续 / 取消 / 重试），三种后端：
//! HTTP（断点续传 + 失败重试 3 次 + 限速）、BT / 磁力（librqbit，下完即停或做种到分享率）、yt-dlp（子进程，解析进度行）。
//! 下载完成后交给 organize 删除可执行文件、识别、重命名、移入整理目录并入库。
use super::{emit_task, load_settings, load_task, organize, remove_task_files, provider, set_task_progress, set_task_status, source, with_db, FetchSettings, TaskView};
use crate::{db, tools};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use librqbit::api::TorrentIdOrHash;
use librqbit::{AddTorrent, AddTorrentOptions, Session, SessionOptions};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use rusqlite::params;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

/// 任务控制标记
pub const RUN: u8 = 0;
pub const PAUSE: u8 = 1;
pub const CANCEL: u8 = 2;

#[derive(Default)]
pub struct Ctl {
    flag: AtomicU8,
    delete: AtomicBool,
}

impl Ctl {
    fn flag(&self) -> u8 {
        self.flag.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct Engine {
    /// 正在下载的任务
    running: HashMap<i64, Arc<Ctl>>,
}

static ENGINE: Lazy<Mutex<Engine>> = Lazy::new(|| Mutex::new(Engine::default()));
/// librqbit 会话，第一次用到 BT 时创建
static BT: tokio::sync::OnceCell<Arc<Session>> = tokio::sync::OnceCell::const_new();
static RE_YTDLP_PROGRESS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[download\]\s+([\d.]+)%\s+of\s+~?\s*([\d.]+\s*[KMGT]?i?B)(?:\s+at\s+([\d.]+\s*[KMGT]?i?B)/s)?").unwrap());

/// 按并发数从排队的任务里取出来开始下载
pub fn pump(app: &AppHandle) {
    let Ok(settings) = with_db(app, |c| Ok(load_settings(c))) else { return };
    let mut eng = ENGINE.lock();
    let free = settings.concurrency.saturating_sub(eng.running.len());
    if free == 0 {
        return;
    }
    let ids: Vec<i64> = with_db(app, |c| {
        let mut st = c.prepare("SELECT id FROM fetch_tasks WHERE status='queued' ORDER BY id LIMIT ?1")?;
        let v = st.query_map([free as i64], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(v)
    })
    .unwrap_or_default();
    for id in ids {
        let ctl = Arc::new(Ctl::default());
        eng.running.insert(id, ctl.clone());
        set_task_status(app, id, "downloading", "");
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move { worker(app2, id, ctl).await });
    }
}

/// 暂停：正在下载的会在下一个检查点停下（保留已下部分）；排队中的直接改为暂停
pub fn pause(app: &AppHandle, id: i64) {
    let running = ENGINE.lock().running.get(&id).cloned();
    match running {
        Some(ctl) => ctl.flag.store(PAUSE, Ordering::SeqCst),
        None => {
            let _ = with_db(app, |c| {
                c.execute("UPDATE fetch_tasks SET status='paused' WHERE id=?1 AND status='queued'", [id])?;
                Ok(())
            });
            emit_task(app, id);
        }
    }
}

/// 继续 / 重试：暂停、失败、取消的任务重新排队（HTTP 从已下部分续传）
pub fn resume(app: &AppHandle, id: i64) {
    let running = ENGINE.lock().running.get(&id).cloned();
    if let Some(ctl) = running {
        ctl.flag.store(RUN, Ordering::SeqCst);
        return;
    }
    let _ = with_db(app, |c| {
        c.execute(
            "UPDATE fetch_tasks SET status='queued', error='', finished_at=NULL WHERE id=?1 AND status IN ('paused','failed','canceled','manual')",
            [id],
        )?;
        Ok(())
    });
    emit_task(app, id);
    pump(app);
}

/// 取消：正在下载的会停止；delete_files 为真时删掉已下载的文件（只删下载目录里的）
pub fn cancel(app: &AppHandle, id: i64, delete_files: bool) {
    let running = ENGINE.lock().running.get(&id).cloned();
    if let Some(ctl) = running {
        ctl.delete.store(delete_files, Ordering::SeqCst);
        ctl.flag.store(CANCEL, Ordering::SeqCst);
        return;
    }
    let info = with_db(app, |c| {
        let t = load_task(c, id)?;
        Ok(t.map(|t| (t, load_settings(c))))
    })
    .ok()
    .flatten();
    let _ = with_db(app, |c| {
        c.execute(
            "UPDATE fetch_tasks SET status='canceled', finished_at=?2 WHERE id=?1 AND status<>'done'",
            params![id, db::now()],
        )?;
        Ok(())
    });
    if delete_files {
        if let Some((t, s)) = info {
            cleanup_task(id, &t.save_path, Path::new(&s.download_dir));
        }
    }
    emit_task(app, id);
}

/// 删掉某个任务留下的文件和文件夹（都在下载目录里）
fn cleanup_task(id: i64, save_path: &str, dl: &Path) {
    remove_task_files(save_path, dl);
    for sub in [format!("bt-{id}"), format!("yt-{id}"), format!("file-{id}")] {
        remove_task_files(&dl.join(sub).to_string_lossy(), dl);
    }
}

async fn worker(app: AppHandle, id: i64, ctl: Arc<Ctl>) {
    let res = run(&app, id, &ctl).await;
    let flag = ctl.flag();
    match (res, flag) {
        (Ok(Some(out)), _) => {
            let status = if out.manual { "manual" } else { "done" };
            let error = if out.manual { out.note.clone() } else { String::new() };
            finish_task(&app, id, status, &out.save_path, &error);
        }
        (_, PAUSE) => set_task_status(&app, id, "paused", ""),
        (_, CANCEL) => {
            if ctl.delete.load(Ordering::SeqCst) {
                if let Ok(Some(t)) = with_db(&app, |c| load_task(c, id)) {
                    let dl = with_db(&app, |c| Ok(load_settings(c).download_dir)).unwrap_or_default();
                    cleanup_task(id, &t.save_path, Path::new(&dl));
                }
            }
            set_task_status(&app, id, "canceled", "");
        }
        (Err(err), _) => set_task_status(&app, id, "failed", &format!("{err:#}")),
        (Ok(None), _) => set_task_status(&app, id, "paused", ""),
    }
    ENGINE.lock().running.remove(&id);
    pump(&app);
}

fn finish_task(app: &AppHandle, id: i64, status: &str, save_path: &str, error: &str) {
    let _ = with_db(app, |c| {
        c.execute(
            "UPDATE fetch_tasks SET status=?2, save_path=?3, error=?4, progress=CASE WHEN ?2 IN ('done','manual') THEN 1 ELSE progress END, finished_at=?5 WHERE id=?1",
            params![id, status, save_path, error, db::now()],
        )?;
        Ok(())
    });
    emit_task(app, id);
}

/// 一次任务：下载 → 整理入库。返回 None 表示被暂停或取消
async fn run(app: &AppHandle, id: i64, ctl: &Arc<Ctl>) -> Result<Option<organize::Outcome>> {
    let (task, settings) = with_db(app, |c| {
        let t = load_task(c, id)?.ok_or_else(|| anyhow!("任务不存在"))?;
        Ok((t, load_settings(c)))
    })?;
    let dl_dir = PathBuf::from(&settings.download_dir);
    tokio::fs::create_dir_all(&dl_dir).await?;
    let downloaded: Option<PathBuf> = match task.kind.as_str() {
        "http" => http_download(app, &task, ctl, &dl_dir, &settings).await?,
        "file" => local_copy(app, &task, &dl_dir).await?,
        "magnet" | "torrent" => bt_download(app, &task, ctl, &dl_dir, &settings).await?,
        "ytdlp" => ytdlp_download(app, &task, ctl, &dl_dir).await?,
        "provider" => provider_download(app, &task, ctl, &dl_dir, &settings).await?,
        other => bail!("不支持的下载类型：{other}"),
    };
    let Some(path) = downloaded else { return Ok(None) };
    if ctl.flag() == CANCEL {
        return Ok(None);
    }
    set_task_status(app, id, "organizing", "");
    let media_kind = task.media_kind.clone();
    let org_root = PathBuf::from(&settings.organize_dir);
    let app2 = app.clone();
    let out = tokio::task::spawn_blocking(move || organize::finish(&app2, &path, &media_kind, &org_root, false)).await??;
    Ok(Some(out))
}

/// 下载目录里的文件名：取网址最后一段，没有就用标题
fn file_name_for(url: &str, title: &str) -> String {
    let from_url = url::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(|x| x.to_string())))
        .map(|s| source::pct_decode(&s))
        .filter(|s| s.contains('.'));
    let raw = from_url.unwrap_or_else(|| title.to_string());
    let n = organize::sanitize(&raw);
    if n == "未命名" {
        "download.bin".into()
    } else {
        n
    }
}

fn save_path_of(app: &AppHandle, id: i64, p: &Path) {
    let _ = with_db(app, |c| {
        c.execute("UPDATE fetch_tasks SET save_path=?2 WHERE id=?1", params![id, p.to_string_lossy()])?;
        Ok(())
    });
}

/// HTTP / HTTPS 直链：流式写入 *.part，中断后用 Range 续传，失败重试 3 次，超时和限速由设置决定
async fn http_download(app: &AppHandle, task: &TaskView, ctl: &Ctl, dl_dir: &Path, settings: &FetchSettings) -> Result<Option<PathBuf>> {
    url::Url::parse(&task.url).context("下载地址不正确")?;
    let final_path = if task.save_path.is_empty() {
        organize::unique(dl_dir.join(file_name_for(&task.url, &task.title)))
    } else {
        PathBuf::from(&task.save_path)
    };
    save_path_of(app, task.id, &final_path);
    let part = PathBuf::from(format!("{}.part", final_path.display()));
    if let Some(dir) = final_path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let client = source::client();
    let limit = settings.speed_kbps * 1024;
    let mut last_err = String::new();
    for _attempt in 1..=3 {
        if ctl.flag() != RUN {
            return Ok(None);
        }
        source::throttle(&task.url).await;
        let offset = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
        let mut req = client.get(task.url.as_str());
        if offset > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let resp = match source::with_auth(req, &task.url).send().await {
            Ok(r) => r,
            Err(err) => {
                last_err = format!("连接失败：{err}");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let status = resp.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = tokio::fs::remove_file(&part).await;
            last_err = "断点续传失败，改为重新下载".into();
            continue;
        }
        // 需要登录或验证码：直接报错，不重试
        source::check_status(status).map_err(|m| anyhow!(m))?;
        let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT && offset > 0;
        let base = if resumed { offset } else { 0 };
        let total: Option<u64> = resp.content_length().map(|n| n + base);
        let mut file = if resumed {
            tokio::fs::OpenOptions::new().append(true).open(&part).await?
        } else {
            tokio::fs::File::create(&part).await?
        };
        let mut got = base;
        let mut stream = resp.bytes_stream();
        let started = Instant::now();
        let mut sent: u64 = 0;
        let mut win_t = Instant::now();
        let mut win_b: u64 = 0;
        let mut speed: i64 = 0;
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut stopped = false;
        let mut broken: Option<String> = None;
        while let Some(chunk) = stream.next().await {
            if ctl.flag() != RUN {
                stopped = true;
                break;
            }
            let chunk = match chunk {
                Ok(c) => c,
                Err(err) => {
                    broken = Some(format!("下载中断：{err}"));
                    break;
                }
            };
            file.write_all(&chunk).await?;
            let n = chunk.len() as u64;
            got += n;
            sent += n;
            win_b += n;
            if limit > 0 {
                let want = Duration::from_secs_f64(sent as f64 / limit as f64);
                let el = started.elapsed();
                if want > el {
                    tokio::time::sleep(want - el).await;
                }
            }
            if win_t.elapsed() >= Duration::from_secs(1) {
                speed = (win_b as f64 / win_t.elapsed().as_secs_f64()) as i64;
                win_t = Instant::now();
                win_b = 0;
            }
            if last_emit.elapsed() >= Duration::from_millis(500) {
                last_emit = Instant::now();
                let t = total.unwrap_or(0);
                let prog = if t > 0 { (got as f64 / t as f64).min(1.0) } else { 0.0 };
                set_task_progress(app, task.id, prog, speed, t as i64);
            }
        }
        file.flush().await?;
        drop(file);
        if stopped {
            return Ok(None);
        }
        if let Some(msg) = broken {
            last_err = msg;
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        if let Some(t) = total {
            if got < t {
                last_err = "下载不完整，准备续传".into();
                continue;
            }
        }
        tokio::fs::rename(&part, &final_path).await?;
        set_task_progress(app, task.id, 1.0, 0, got as i64);
        return Ok(Some(final_path));
    }
    bail!("{last_err}（已重试 3 次）")
}

/// 本地或映射盘文件：复制到下载目录（不动原文件）
async fn local_copy(app: &AppHandle, task: &TaskView, dl_dir: &Path) -> Result<Option<PathBuf>> {
    let src = PathBuf::from(&task.url);
    if !src.is_file() {
        bail!("找不到源文件（网盘或 NAS 是否已连接？）：{}", task.url);
    }
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file.bin".into());
    let dir = dl_dir.join(format!("file-{}", task.id));
    tokio::fs::create_dir_all(&dir).await?;
    let dst = dir.join(organize::sanitize(&name));
    save_path_of(app, task.id, &dst);
    let (s, d) = (src.clone(), dst.clone());
    tokio::task::spawn_blocking(move || std::fs::copy(&s, &d)).await??;
    set_task_progress(app, task.id, 1.0, 0, std::fs::metadata(&dst).map(|m| m.len() as i64).unwrap_or(0));
    Ok(Some(dst))
}

/// 初始化 librqbit 会话（整个程序只一个）
async fn session(dl_dir: &Path) -> Result<Arc<Session>> {
    let dir = dl_dir.to_path_buf();
    let s = BT
        .get_or_try_init(|| async move { Session::new_with_opts(dir, SessionOptions::default()).await })
        .await?;
    Ok(s.clone())
}

/// BT / 磁力：每个任务一个文件夹；下完后按设置停止或做种到分享率；暂停时从会话里移除（文件保留，继续时校验后续传）
async fn bt_download(app: &AppHandle, task: &TaskView, ctl: &Ctl, dl_dir: &Path, settings: &FetchSettings) -> Result<Option<PathBuf>> {
    let session = session(dl_dir).await?;
    let out_dir = dl_dir.join(format!("bt-{}", task.id));
    tokio::fs::create_dir_all(&out_dir).await?;
    let add = if task.url.starts_with("magnet:") || source::is_http(&task.url) {
        AddTorrent::from_url(task.url.clone())
    } else {
        AddTorrent::from_local_filename(&task.url)?
    };
    let opts = AddTorrentOptions {
        output_folder: Some(out_dir.to_string_lossy().into_owned()),
        overwrite: true,
        ..Default::default()
    };
    let resp = session.add_torrent(add, Some(opts)).await.context("添加种子失败")?;
    let handle = resp.into_handle().ok_or_else(|| anyhow!("种子没有返回下载句柄"))?;
    let id = handle.id();
    let mut last_emit = Instant::now() - Duration::from_secs(5);
    loop {
        if ctl.flag() != RUN {
            let del = ctl.flag() == CANCEL && ctl.delete.load(Ordering::SeqCst);
            let _ = session.delete(TorrentIdOrHash::Id(id), del).await;
            return Ok(None);
        }
        let st = handle.stats();
        if let Some(err) = &st.error {
            let _ = session.delete(TorrentIdOrHash::Id(id), false).await;
            bail!("BT 下载出错：{err}");
        }
        if last_emit.elapsed() >= Duration::from_secs(1) {
            last_emit = Instant::now();
            let total = st.total_bytes.max(1);
            let prog = st.progress_bytes as f64 / total as f64;
            let speed = st.live.as_ref().map(|l| l.download_speed.as_bytes() as i64).unwrap_or(0);
            set_task_progress(app, task.id, prog.min(1.0), speed, st.total_bytes as i64);
        }
        if st.finished {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if settings.seed_policy == "ratio" {
        set_task_status(app, task.id, "seeding", "");
        loop {
            if ctl.flag() != RUN {
                let _ = session.delete(TorrentIdOrHash::Id(id), false).await;
                return Ok(None);
            }
            let st = handle.stats();
            let ratio = st.uploaded_bytes as f64 / st.total_bytes.max(1) as f64;
            if ratio >= settings.seed_ratio {
                break;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    // 下完即停：从会话里移除（不删文件），之后才能移动文件
    let _ = session.delete(TorrentIdOrHash::Id(id), false).await;
    Ok(Some(out_dir))
}

/// yt-dlp 子进程：解析 [download] 进度行，结束后找到输出文件
async fn ytdlp_download(app: &AppHandle, task: &TaskView, ctl: &Arc<Ctl>, dl_dir: &Path) -> Result<Option<PathBuf>> {
    let exe = source::ytdlp_path().ok_or_else(|| anyhow!(source::YTDLP_MISSING))?;
    let out_dir = dl_dir.join(format!("yt-{}", task.id));
    tokio::fs::create_dir_all(&out_dir).await?;
    source::throttle(&task.url).await;
    let (app2, ctl2) = (app.clone(), ctl.clone());
    let (id, url) = (task.id, task.url.clone());
    tokio::task::spawn_blocking(move || ytdlp_blocking(&app2, id, &ctl2, &exe, &url, &out_dir)).await?
}

fn ytdlp_blocking(app: &AppHandle, id: i64, ctl: &Ctl, exe: &Path, url: &str, out_dir: &Path) -> Result<Option<PathBuf>> {
    let mut cmd = tools::command(exe);
    cmd.args(["--newline", "--no-playlist", "--no-warnings", "-P"])
        .arg(out_dir)
        .args(["-o", "%(title).150B [%(id)s].%(ext)s", "--"])
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().context("启动 yt-dlp 失败")?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("无法读取 yt-dlp 的输出"))?;
    let mut last_dest: Option<PathBuf> = None;
    let mut last_emit = Instant::now() - Duration::from_secs(5);
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if ctl.flag() != RUN {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        if let Some(d) = parse_destination(&line) {
            last_dest = Some(d);
        }
        if let Some((frac, speed, total)) = parse_ytdlp_progress(&line) {
            if last_emit.elapsed() >= Duration::from_millis(500) {
                last_emit = Instant::now();
                set_task_progress(app, id, frac, speed.unwrap_or(0), total.unwrap_or(0));
            }
        }
    }
    let status = child.wait()?;
    if ctl.flag() != RUN {
        return Ok(None);
    }
    if !status.success() {
        bail!("yt-dlp 下载失败（退出码 {}）", status.code().unwrap_or(-1));
    }
    let path = last_dest
        .filter(|p| p.is_file())
        .or_else(|| newest_file(out_dir))
        .ok_or_else(|| anyhow!("下载完成，但找不到输出文件"))?;
    Ok(Some(path))
}

/// yt-dlp 输出的目标文件行
pub fn parse_destination(line: &str) -> Option<PathBuf> {
    let l = line.trim();
    for pre in ["[download] Destination: ", "[ExtractAudio] Destination: "] {
        if let Some(rest) = l.strip_prefix(pre) {
            return Some(PathBuf::from(rest.trim()));
        }
    }
    if let Some(i) = l.find("Merging formats into \"") {
        let rest = &l[i + "Merging formats into \"".len()..];
        let end = rest.rfind('"')?;
        return Some(PathBuf::from(&rest[..end]));
    }
    None
}

/// yt-dlp 进度行：[download]  45.3% of ~ 120.50MiB at 2.31MiB/s ETA 00:31
/// 返回 (进度 0-1, 速度 字节/秒, 总大小 字节)
pub fn parse_ytdlp_progress(line: &str) -> Option<(f64, Option<i64>, Option<i64>)> {
    let c = RE_YTDLP_PROGRESS.captures(line)?;
    let frac = c[1].parse::<f64>().ok()? / 100.0;
    let total = source::parse_size(&c[2]);
    let speed = c.get(3).and_then(|s| source::parse_size(s.as_str()));
    Some((frac.clamp(0.0, 1.0), speed, total))
}

fn newest_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.to_string_lossy().ends_with(".part") && !p.to_string_lossy().ends_with(".ytdl"))
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// 下载源：每次开始 / 继续时重新取下载地址（直链可能过期），再交给 HTTP 或 yt-dlp；
/// 下完后音乐补标签、存歌词，最后照常整理入库
async fn provider_download(app: &AppHandle, task: &TaskView, ctl: &Arc<Ctl>, dl_dir: &Path, settings: &FetchSettings) -> Result<Option<PathBuf>> {
    let (pid, item, quality) = provider::parse_task_url(&task.url).ok_or_else(|| anyhow!("下载源任务地址不正确"))?;
    let p = provider::get(&pid).ok_or_else(|| anyhow!("这个版本的软件里没有下载源“{pid}”"))?;
    let resolved = p.resolve(&item, &quality).await.map_err(|m| anyhow!(m))?;
    let mut t = task.clone();
    let got = match &resolved.target {
        provider::Target::Http { url, ext } => {
            if t.save_path.is_empty() {
                t.save_path = organize::unique(dl_dir.join(provider::file_name(&item, ext))).to_string_lossy().into_owned();
            }
            t.url = url.clone();
            http_download(app, &t, ctl, dl_dir, settings).await?
        }
        provider::Target::Ytdlp { url } => {
            t.url = url.clone();
            ytdlp_download(app, &t, ctl, dl_dir).await?
        }
    };
    let Some(path) = got else { return Ok(None) };
    if path.is_file() {
        if let Err(err) = provider::fill_tags(&path, &item) {
            eprintln!("补标签失败 {}：{err}", path.display());
        }
        if let Some(l) = &resolved.lyric {
            let _ = provider::save_lyric(&path, l);
        }
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ytdlp_progress_lines() {
        let (f, s, t) = parse_ytdlp_progress("[download]  45.3% of ~ 120.50MiB at    2.31MiB/s ETA 00:31").unwrap();
        assert!((f - 0.453).abs() < 1e-9);
        assert_eq!(s, source::parse_size("2.31MiB"));
        assert_eq!(t, source::parse_size("120.50MiB"));
        let (f2, s2, _) = parse_ytdlp_progress("[download] 100% of 1.20GiB in 00:05").unwrap();
        assert_eq!(f2, 1.0);
        assert_eq!(s2, None);
        assert!(parse_ytdlp_progress("[info] something").is_none());
    }

    #[test]
    fn ytdlp_destination_lines() {
        assert_eq!(
            parse_destination("[download] Destination: D:\\dl\\a b.mp4"),
            Some(PathBuf::from("D:\\dl\\a b.mp4"))
        );
        assert_eq!(
            parse_destination("[Merger] Merging formats into \"D:\\dl\\x.mkv\""),
            Some(PathBuf::from("D:\\dl\\x.mkv"))
        );
        assert_eq!(parse_destination("[download] 10% of 1MiB"), None);
    }

    #[test]
    fn file_name_from_url() {
        assert_eq!(file_name_for("https://a.b/dir/Show%20E01.mkv?x=1", "t"), "Show E01.mkv");
        assert_eq!(file_name_for("https://a.b/", "星屑 - 01"), "星屑 - 01");
    }

    #[test]
    fn pause_flags_are_distinct() {
        let c = Ctl::default();
        assert_eq!(c.flag(), RUN);
        c.flag.store(PAUSE, Ordering::SeqCst);
        assert_eq!(c.flag(), PAUSE);
        assert_ne!(PAUSE, CANCEL);
    }
}
