//! 外部工具：ffmpeg / ffprobe / yt-dlp 的查找与调用（不弹黑框）
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 软件自己下载 / 更新的工具放在数据目录的 bin\（启动时设置）
static DATA_BIN: once_cell::sync::OnceCell<PathBuf> = once_cell::sync::OnceCell::new();

pub fn set_data_bin(dir: PathBuf) {
    let _ = DATA_BIN.set(dir);
}

pub fn data_bin() -> Option<&'static Path> {
    DATA_BIN.get().map(|p| p.as_path())
}

/// 查找顺序：环境变量（MP_FFMPEG 等）、数据目录的 bin\、程序目录下的 bin\、PATH
fn find(exe: &str, env_key: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(env_key) {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(p) = data_bin().map(|d| d.join(exe)).filter(|p| p.exists()) {
        return Some(p);
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            for cand in [dir.join("bin").join(exe), dir.join(exe)] {
                if cand.exists() {
                    return Some(cand);
                }
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.exists())
}

pub fn ffmpeg() -> Option<PathBuf> {
    find("ffmpeg.exe", "MP_FFMPEG")
}
pub fn ffprobe() -> Option<PathBuf> {
    find("ffprobe.exe", "MP_FFPROBE").or_else(|| ffmpeg().map(|f| f.with_file_name("ffprobe.exe")).filter(|p| p.exists()))
}
pub fn ytdlp() -> Option<PathBuf> {
    find("yt-dlp.exe", "MP_YTDLP")
}

/// 新建一个不弹窗口的命令
pub fn command(exe: &Path) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c.stdin(Stdio::null());
    c
}

pub fn ffmpeg_cmd() -> Result<Command> {
    let p = ffmpeg().ok_or_else(|| anyhow!("没有找到 ffmpeg.exe（放到程序目录的 bin 文件夹，或加入 PATH）"))?;
    let mut c = command(&p);
    c.args(["-hide_banner", "-nostdin", "-y"]);
    Ok(c)
}

/// 运行到结束，失败时带上 stderr 最后几行
pub fn run(mut c: Command) -> Result<Vec<u8>> {
    let out = c.stdout(Stdio::piped()).stderr(Stdio::piped()).output().context("启动外部程序失败")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(6).collect();
        return Err(anyhow!("{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    Ok(out.stdout)
}

/// ffprobe 的 JSON（format + streams）
pub fn probe(path: &Path) -> Result<serde_json::Value> {
    let exe = ffprobe().ok_or_else(|| anyhow!("没有找到 ffprobe.exe"))?;
    let mut c = command(&exe);
    c.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(path);
    let out = run(c)?;
    Ok(serde_json::from_slice(&out)?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_ffmpeg_if_installed() {
        // 只检查不 panic；CI 上可能没有 ffmpeg
        let _ = super::ffmpeg();
    }
}
