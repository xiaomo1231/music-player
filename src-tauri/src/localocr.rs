//! 本地文字识别：Xiaomi-OCR-0（0.8B 视觉模型，Apache-2.0）在本机用 llama.cpp 运行，不上传图片。
//!
//! - 运行程序 llama-server.exe 随安装包放在程序目录 `bin\llama\`（见 tools/bundled-tools.json）。
//! - 模型（约 0.7 GB）第一次使用时下载到数据目录 `models\xiaomi-ocr-0\`，支持断点续传，下载后校验 SHA-256。
//!   先试 huggingface.co，连不上自动换 hf-mirror.com。
//! - 需要识别时才在后台启动 llama-server（只监听 127.0.0.1 的随机端口），空闲 5 分钟自动退出；
//!   Windows 上把它放进“随主程序关闭”的作业对象，播放器退出或崩溃时它也会退出。
use crate::{tools, AppState};
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use sha2::Digest;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

pub struct ModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// 社区 GGUF 转换（prithivMLmods/Xiaomi-OCR-0-GGUF，Apache-2.0）。Q4_K_M 体积最小，实测精度与原版基本一致。
pub const MODEL_REPO: &str = "prithivMLmods/Xiaomi-OCR-0-GGUF";
pub const MODEL: ModelFile = ModelFile {
    name: "Xiaomi-OCR-0.Q4_K_M.gguf",
    size: 529_297_664,
    sha256: "1e497ff06b91a3d822c543066df3e9ef69c62c5423be09d739dfa1db55630d3a",
};
pub const MMPROJ: ModelFile = ModelFile {
    name: "Xiaomi-OCR-0.mmproj-bf16.gguf",
    size: 207_346_624,
    sha256: "634d6bc95781ebf4a3f06570a59859fa5082d488db540fd474bc6b4061aec90e",
};
pub const FILES: [&ModelFile; 2] = [&MODEL, &MMPROJ];
/// 下载源，按顺序尝试
pub const MIRRORS: [&str; 2] = ["https://huggingface.co", "https://hf-mirror.com"];

/// 官方建议的整行文字识别提示词
pub const PROMPT_TEXT: &str = "Extract the text in the image.";

const IDLE_SECS: u64 = 300;
const CONTEXT: &str = "8192";

pub fn model_url(mirror: &str, file: &str) -> String {
    format!("{mirror}/{MODEL_REPO}/resolve/main/{file}")
}

pub fn model_dir(data: &Path) -> PathBuf {
    data.join("models").join("xiaomi-ocr-0")
}

/// llama-server.exe：环境变量 MP_LLAMA_SERVER、程序目录 bin\llama\、数据目录 bin\llama\
pub fn server_exe() -> Option<PathBuf> {
    const EXE: &str = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };
    if let Some(p) = std::env::var_os("MP_LLAMA_SERVER").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    let mut cands = vec![];
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        cands.push(dir.join("bin").join("llama").join(EXE));
    }
    if let Some(d) = tools::data_bin() {
        cands.push(d.join("llama").join(EXE));
    }
    cands.into_iter().find(|p| p.exists())
}

fn file_ok(dir: &Path, f: &ModelFile) -> bool {
    std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.len() == f.size)
}

pub fn installed(data: &Path) -> bool {
    let dir = model_dir(data);
    FILES.iter().all(|f| file_ok(&dir, f))
}

// ---------- 状态 ----------

#[derive(Serialize, Clone, Debug)]
pub struct Status {
    /// 模型文件齐全
    pub installed: bool,
    /// 找到了 llama-server.exe（安装包自带）
    pub runtime: bool,
    pub downloading: bool,
    pub running: bool,
    /// 模型总大小（字节）
    pub size: u64,
    /// 已下载（含未完成的部分）
    pub got: u64,
    pub dir: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub got: u64,
    pub total: u64,
    /// 字节/秒
    pub speed: u64,
    pub file: String,
    pub stage: String,
    pub message: String,
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);
static CANCEL_DL: AtomicBool = AtomicBool::new(false);

fn downloaded_bytes(dir: &Path) -> u64 {
    FILES
        .iter()
        .map(|f| {
            let full = std::fs::metadata(dir.join(f.name)).map(|m| m.len()).unwrap_or(0);
            let part = std::fs::metadata(dir.join(format!("{}.part", f.name))).map(|m| m.len()).unwrap_or(0);
            full.max(part).min(f.size)
        })
        .sum()
}

pub fn status(data: &Path) -> Status {
    let dir = model_dir(data);
    Status {
        installed: installed(data),
        runtime: server_exe().is_some(),
        downloading: DOWNLOADING.load(Ordering::SeqCst),
        running: SERVER.lock().as_ref().is_some(),
        size: FILES.iter().map(|f| f.size).sum(),
        got: downloaded_bytes(&dir),
        dir: dir.to_string_lossy().into_owned(),
    }
}

// ---------- 下载 ----------

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn emit(app: &AppHandle, p: Progress) {
    let _ = app.emit("localocr://progress", p);
}

/// 从一个下载源下载一个文件到 .part（断点续传），返回是否被取消
async fn fetch_one(app: &AppHandle, client: &reqwest::Client, url: &str, part: &Path, f: &ModelFile, before: u64, total: u64) -> Result<bool> {
    let offset = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    if offset >= f.size {
        return Ok(false);
    }
    let mut req = client.get(url);
    if offset > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let resp = req.send().await.with_context(|| format!("连接失败：{url}"))?;
    let st = resp.status();
    if !st.is_success() {
        return Err(anyhow!("HTTP {st}：{url}"));
    }
    let resumed = st == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = if resumed {
        tokio::fs::OpenOptions::new().append(true).open(part).await?
    } else {
        tokio::fs::File::create(part).await?
    };
    let mut got = if resumed { offset } else { 0 };
    let mut stream = resp.bytes_stream();
    let (mut win_t, mut win_b, mut speed) = (Instant::now(), 0u64, 0u64);
    let mut last = Instant::now() - Duration::from_secs(1);
    while let Some(chunk) = stream.next().await {
        if CANCEL_DL.load(Ordering::SeqCst) {
            file.flush().await?;
            return Ok(true);
        }
        let chunk = chunk.context("下载中断")?;
        file.write_all(&chunk).await?;
        got += chunk.len() as u64;
        win_b += chunk.len() as u64;
        if win_t.elapsed() >= Duration::from_secs(1) {
            speed = (win_b as f64 / win_t.elapsed().as_secs_f64()) as u64;
            win_t = Instant::now();
            win_b = 0;
        }
        if last.elapsed() >= Duration::from_millis(400) {
            last = Instant::now();
            emit(
                app,
                Progress {
                    got: before + got.min(f.size),
                    total,
                    speed,
                    file: f.name.into(),
                    stage: "download".into(),
                    message: "正在下载识别模型".into(),
                },
            );
        }
    }
    file.flush().await?;
    Ok(false)
}

async fn download_all(app: &AppHandle) -> Result<bool> {
    let data = app.state::<AppState>().dirs.data.clone();
    let dir = model_dir(&data);
    tokio::fs::create_dir_all(&dir).await?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let total: u64 = FILES.iter().map(|f| f.size).sum();
    let mut before = 0u64;
    for f in FILES {
        let dest = dir.join(f.name);
        if file_ok(&dir, f) {
            before += f.size;
            continue;
        }
        let part = dir.join(format!("{}.part", f.name));
        let mut errs = vec![];
        let mut done = false;
        // 每个下载源最多试 3 次（网络抖动时断点续传接着下）
        'mirrors: for m in MIRRORS {
            for _ in 0..3 {
                match fetch_one(app, &client, &model_url(m, f.name), &part, f, before, total).await {
                    Ok(true) => return Ok(false),
                    Ok(false) => {
                        if tokio::fs::metadata(&part).await.map(|x| x.len()).unwrap_or(0) >= f.size {
                            done = true;
                            break 'mirrors;
                        }
                    }
                    Err(e) => errs.push(format!("{e:#}")),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        if !done {
            let last = errs.pop().unwrap_or_default();
            return Err(anyhow!("模型下载失败，请检查网络后重试（{last}）"));
        }
        emit(
            app,
            Progress {
                got: before + f.size,
                total,
                speed: 0,
                file: f.name.into(),
                stage: "verify".into(),
                message: "正在校验文件".into(),
            },
        );
        let p = part.clone();
        let sum = tauri::async_runtime::spawn_blocking(move || sha256_file(&p))
            .await
            .map_err(|e| anyhow!("{e}"))??;
        if sum != f.sha256 {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(anyhow!("{} 校验不通过（文件损坏或被篡改），已删除，请重新下载", f.name));
        }
        tokio::fs::rename(&part, &dest).await?;
        before += f.size;
    }
    Ok(true)
}

// ---------- llama-server ----------

struct Server {
    child: Child,
    port: u16,
    last_used: Instant,
}

static SERVER: Lazy<Mutex<Option<Server>>> = Lazy::new(|| Mutex::new(None));
/// 防止并发启动多个 llama-server
static STARTING: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
static WATCHDOG: AtomicBool = AtomicBool::new(false);

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// 启动参数（不含路径以外的用户输入）
pub fn server_args(model: &Path, mmproj: &Path, port: u16) -> Vec<String> {
    vec![
        "-m".into(),
        model.to_string_lossy().into_owned(),
        "--mmproj".into(),
        mmproj.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        CONTEXT.into(),
        "-np".into(),
        "1".into(),
    ]
}

fn log_tail(p: &Path) -> String {
    let s = std::fs::read(p).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let lines: Vec<&str> = s.lines().rev().take(6).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// 进程还活着就返回端口
fn alive_port() -> Option<u16> {
    let mut g = SERVER.lock();
    let s = g.as_mut()?;
    match s.child.try_wait() {
        Ok(None) => {
            s.last_used = Instant::now();
            Some(s.port)
        }
        _ => {
            *g = None;
            None
        }
    }
}

pub fn stop() {
    if let Some(mut s) = SERVER.lock().take() {
        let _ = s.child.kill();
        let _ = s.child.wait();
    }
}

fn spawn_watchdog() {
    if WATCHDOG.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let idle = SERVER.lock().as_ref().is_some_and(|s| s.last_used.elapsed() > Duration::from_secs(IDLE_SECS));
            if idle {
                stop();
            }
        }
    });
}

async fn ensure_server(app: &AppHandle) -> Result<u16> {
    if let Some(p) = alive_port() {
        return Ok(p);
    }
    let _g = STARTING.lock().await;
    if let Some(p) = alive_port() {
        return Ok(p);
    }
    let data = app.state::<AppState>().dirs.data.clone();
    if !installed(&data) {
        return Err(anyhow!("本地识别模型还没下载：到“设置 → AI 服务 → 文字识别”里下载（约 0.7 GB）"));
    }
    let exe = server_exe().ok_or_else(|| anyhow!("没有找到 llama-server.exe（安装包自带，可能被删除了：重新安装播放器）"))?;
    let dir = model_dir(&data);
    let port = free_port()?;
    let log = data.join("logs").join("llama-server.log");
    if let Some(p) = log.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut cmd = tools::command(&exe);
    cmd.args(server_args(&dir.join(MODEL.name), &dir.join(MMPROJ.name), port))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log)?);
    if let Some(d) = exe.parent() {
        cmd.current_dir(d);
    }
    let child = cmd.spawn().context("启动 llama-server 失败")?;
    job::attach(&child);
    *SERVER.lock() = Some(Server {
        child,
        port,
        last_used: Instant::now(),
    });
    spawn_watchdog();

    // 等模型加载完：/health 返回 200
    let client = reqwest::Client::new();
    let started = Instant::now();
    loop {
        {
            let mut g = SERVER.lock();
            let exited = match g.as_mut() {
                Some(s) => !matches!(s.child.try_wait(), Ok(None)),
                None => true,
            };
            if exited {
                *g = None;
                return Err(anyhow!("本地识别程序启动失败：\n{}", log_tail(&log)));
            }
        }
        if let Ok(r) = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            if r.status().is_success() {
                warm_up(&client, port).await;
                return Ok(port);
            }
        }
        if started.elapsed() > Duration::from_secs(180) {
            stop();
            return Err(anyhow!("本地识别程序启动超时：\n{}", log_tail(&log)));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 预热：用显卡（Vulkan）时第一次识别要先编译着色器，实测十几秒，启动后先识别一张小图
async fn warm_up(client: &reqwest::Client, port: u16) {
    let Ok(png) = test_image() else { return };
    let _ = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .timeout(Duration::from_secs(120))
        .json(&request_body(&png, PROMPT_TEXT, 8))
        .send()
        .await;
}

/// 请求体：和官方 MCP 服务一致的解码参数（贪心解码、关闭思考模式）
pub fn request_body(png: &[u8], prompt: &str, max_tokens: u32) -> serde_json::Value {
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    json!({
        "model": "xiaomi-ocr-0",
        "messages": [{"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{b64}")}},
            {"type": "text", "text": prompt}
        ]}],
        "max_tokens": max_tokens,
        "temperature": 0.0,
        "top_p": 0.8,
        "chat_template_kwargs": {"enable_thinking": false}
    })
}

/// 去掉模型偶尔带上的代码块标记和首尾空白
pub fn clean_output(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix("```markdown").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim().to_string()
}

/// 识别一张图片（PNG）里的文字
pub async fn recognize(app: &AppHandle, png: &[u8], prompt: &str, max_tokens: u32) -> Result<String> {
    let port = ensure_server(app).await?;
    let body = request_body(png, prompt, max_tokens);
    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .timeout(Duration::from_secs(300))
        .json(&body)
        .send()
        .await
        .context("本地识别程序没有响应")?;
    let st = resp.status();
    let v: serde_json::Value = resp.json().await.context("本地识别返回格式不对")?;
    if !st.is_success() {
        return Err(anyhow!("本地识别出错（{st}）：{}", v["error"]["message"].as_str().unwrap_or("")));
    }
    if let Some(s) = SERVER.lock().as_mut() {
        s.last_used = Instant::now();
    }
    Ok(clean_output(v["choices"][0]["message"]["content"].as_str().unwrap_or("")))
}

// ---------- 作业对象：主程序退出时 llama-server 跟着退出 ----------

#[cfg(windows)]
mod job {
    use once_cell::sync::Lazy;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct Job(isize);
    // 句柄只在本进程里用，生命周期与进程相同
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: Lazy<Option<Job>> = Lazy::new(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return None;
        }
        Some(Job(h as isize))
    });

    pub fn attach(child: &std::process::Child) {
        if let Some(j) = JOB.as_ref() {
            unsafe {
                AssignProcessToJobObject(j.0 as _, child.as_raw_handle() as _);
            }
        }
    }
}

#[cfg(not(windows))]
mod job {
    pub fn attach(_child: &std::process::Child) {}
}

// ---------- 命令 ----------

#[tauri::command]
pub fn localocr_status(st: tauri::State<AppState>) -> Status {
    status(&st.dirs.data)
}

/// 下载模型（已下载的部分会接着下）。返回 true 表示完成，false 表示被取消。
#[tauri::command]
pub async fn localocr_download(app: AppHandle) -> Result<bool, String> {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("正在下载中".into());
    }
    CANCEL_DL.store(false, Ordering::SeqCst);
    let r = download_all(&app).await;
    DOWNLOADING.store(false, Ordering::SeqCst);
    let msg = match &r {
        Ok(true) => "下载完成",
        Ok(false) => "已暂停",
        Err(_) => "下载失败",
    };
    let data = app.state::<AppState>().dirs.data.clone();
    let s = status(&data);
    emit(
        &app,
        Progress {
            got: s.got,
            total: s.size,
            speed: 0,
            file: String::new(),
            stage: if r.as_ref().is_ok_and(|d| *d) { "done".into() } else { "stopped".into() },
            message: msg.into(),
        },
    );
    r.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn localocr_cancel() {
    CANCEL_DL.store(true, Ordering::SeqCst);
}

/// 删除已下载的模型（释放约 0.7 GB）
#[tauri::command]
pub fn localocr_remove(st: tauri::State<AppState>) -> Result<(), String> {
    if DOWNLOADING.load(Ordering::SeqCst) {
        return Err("正在下载，先暂停再删除".into());
    }
    stop();
    let dir = model_dir(&st.dirs.data);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除失败：{e}"))?;
    }
    Ok(())
}

/// 测试：启动本地识别并识别一张内置的小图，返回识别结果和用时
#[tauri::command]
pub async fn localocr_test(app: AppHandle) -> Result<String, String> {
    let png = test_image().map_err(|e| e.to_string())?;
    let t = Instant::now();
    let text = recognize(&app, &png, PROMPT_TEXT, 64).await.map_err(|e| format!("{e:#}"))?;
    Ok(format!("识别结果“{}”，用时 {:.1} 秒（第一次包含启动时间）", text, t.elapsed().as_secs_f64()))
}

/// 画一张写着粗体“OK 123”的白底黑字小图（不依赖字体文件）
fn test_image() -> Result<Vec<u8>> {
    const GLYPHS: [(char, [&str; 7]); 6] = [
        ('O', ["01110", "10001", "10001", "10001", "10001", "10001", "01110"]),
        ('K', ["10001", "10010", "10100", "11000", "10100", "10010", "10001"]),
        (' ', ["00000", "00000", "00000", "00000", "00000", "00000", "00000"]),
        ('1', ["00100", "01100", "00100", "00100", "00100", "00100", "01110"]),
        ('2', ["01110", "10001", "00001", "00010", "00100", "01000", "11111"]),
        ('3', ["11110", "00001", "00001", "01110", "00001", "00001", "11110"]),
    ];
    let scale = 8u32;
    let (w, h) = ((GLYPHS.len() as u32 * 6 + 4) * scale, 11 * scale);
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    for (i, (_, rows)) in GLYPHS.iter().enumerate() {
        for (r, row) in rows.iter().enumerate() {
            for (c, bit) in row.bytes().enumerate() {
                if bit == b'1' {
                    let x0 = (2 + i as u32 * 6 + c as u32) * scale;
                    let y0 = (2 + r as u32) * scale;
                    for y in y0..y0 + scale {
                        for x in x0..x0 + scale {
                            img.put_pixel(x, y, image::Rgb([0, 0, 0]));
                        }
                    }
                }
            }
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_paths() {
        assert_eq!(
            model_url(MIRRORS[1], MODEL.name),
            "https://hf-mirror.com/prithivMLmods/Xiaomi-OCR-0-GGUF/resolve/main/Xiaomi-OCR-0.Q4_K_M.gguf"
        );
        let d = model_dir(Path::new("/data"));
        assert!(d.ends_with("models/xiaomi-ocr-0"));
        let a = server_args(&d.join(MODEL.name), &d.join(MMPROJ.name), 4321);
        assert!(a.windows(2).any(|w| w[0] == "--host" && w[1] == "127.0.0.1"));
        assert!(a.windows(2).any(|w| w[0] == "--port" && w[1] == "4321"));
    }

    #[test]
    fn body_disables_thinking() {
        let b = request_body(b"png", PROMPT_TEXT, 100);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(b["temperature"], 0.0);
        assert!(b["messages"][0]["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn clean() {
        assert_eq!(clean_output("```markdown\nこんにちは\n```"), "こんにちは");
        assert_eq!(clean_output("  abc \n"), "abc");
    }

    #[test]
    fn install_check_uses_size() {
        let tmp = std::env::temp_dir().join(format!("mp-localocr-{}", std::process::id()));
        let dir = model_dir(&tmp);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!installed(&tmp));
        std::fs::write(dir.join(format!("{}.part", MMPROJ.name)), [0u8; 10]).unwrap();
        assert_eq!(downloaded_bytes(&dir), 10);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_image_is_png() {
        let png = test_image().unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert!(img.width() > img.height());
    }

    #[test]
    fn sha_of_file() {
        let p = std::env::temp_dir().join(format!("mp-sha-{}", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_file(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = std::fs::remove_file(&p);
    }
}
