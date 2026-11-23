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

/// 程序目录下的 bin\（安装包自带的 yt-dlp、ffmpeg、ffprobe 装在这里）
fn bundled_bin() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|d| d.join("bin"))
}

/// 查找顺序：环境变量（MP_FFMPEG 等）、数据目录的 bin\、程序目录下的 bin\（安装包自带）、PATH
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
/// yt-dlp：安装包自带一份（程序目录 bin\），用户在软件里点“更新”会下载到数据目录 bin\。
/// 两份都在时用版本新的那份：软件升级后自带的可能比用户以前更新的还新。
pub fn ytdlp() -> Option<PathBuf> {
    const EXE: &str = "yt-dlp.exe";
    if std::env::var_os("MP_YTDLP").is_some() {
        return find(EXE, "MP_YTDLP");
    }
    let data = data_bin().map(|d| d.join(EXE)).filter(|p| p.exists());
    let bundled = bundled_bin().map(|d| d.join(EXE)).filter(|p| p.exists());
    match (data, bundled) {
        (Some(d), Some(b)) => {
            let bundled_newer = match (bundled_version(b.parent()?, "yt-dlp"), ytdlp_data_version(&d)) {
                (Some(bv), Some(dv)) => version_newer(&bv, &dv),
                _ => false,
            };
            Some(if bundled_newer { b } else { d })
        }
        (Some(p), None) | (None, Some(p)) => Some(p),
        (None, None) => find(EXE, "MP_YTDLP"),
    }
}

/// 安装包自带程序的版本（打包时写在 bin\versions.json）
fn bundled_version(bin: &Path, name: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(bin.join("versions.json")).ok()?).ok()?;
    v[name]["version"].as_str().map(str::to_string)
}

/// 数据目录里 yt-dlp 的版本：更新时记在旁边的 yt-dlp.version；
/// 老版本软件装的没有这个文件，就用文件修改日期（当时下载的是最新版，版本号 ≈ 下载日期）
fn ytdlp_data_version(exe: &Path) -> Option<String> {
    if let Ok(v) = std::fs::read_to_string(exe.with_extension("version")) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let secs = std::fs::metadata(exe)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    Some(format!("{y}.{m:02}.{d:02}"))
}

/// 记下数据目录里 yt-dlp 的版本（安装 / 更新后调用）
pub fn write_ytdlp_data_version(exe: &Path, version: &str) {
    let _ = std::fs::write(exe.with_extension("version"), version.trim());
}

/// yt-dlp 的版本号形如 2026.10.26 或 2026.10.26.1，逐段按数字比较
fn version_newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| s.split('.').map(|x| x.trim().parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parse(a) > parse(b)
}

/// 1970-01-01 起的天数 → (年, 月, 日)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
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
    let p = ffmpeg().ok_or_else(|| anyhow!("没有找到 ffmpeg.exe（安装包自带，可能被删除了：重新安装播放器，或放到程序目录的 bin 文件夹）"))?;
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

    #[test]
    fn compares_ytdlp_versions() {
        use super::version_newer;
        assert!(version_newer("2026.10.26", "2026.09.30"));
        assert!(version_newer("2026.10.26.1", "2026.10.26"));
        assert!(version_newer("2027.01.02", "2026.12.31"));
        assert!(!version_newer("2026.10.26", "2026.10.26"));
        assert!(!version_newer("2026.09.30", "2026.10.26"));
    }

    #[test]
    fn days_to_date() {
        use super::civil_from_days;
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_415), (2025, 11, 23));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
