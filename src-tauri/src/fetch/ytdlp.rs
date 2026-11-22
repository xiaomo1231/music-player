//! yt-dlp 的安装与更新：从 yt-dlp 官方 GitHub 发布页下载 yt-dlp.exe，按同一发布里的 SHA2-256SUMS 校验后放到数据目录的 bin\。
//! B 站等网站经常改接口，yt-dlp 也随之频繁发版，所以在软件里一键更新，不用重新安装播放器。
use crate::tools;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Duration;

const BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";
const EXE: &str = "yt-dlp.exe";

#[derive(Serialize, Clone, Debug, Default)]
pub struct Status {
    pub installed: bool,
    pub version: String,
    pub path: String,
    /// 装在软件自己的数据目录里（可以一键更新）
    pub managed: bool,
}

fn version_of(exe: &std::path::Path) -> String {
    let mut c = tools::command(exe);
    c.arg("--version");
    tools::run(c).map(|o| String::from_utf8_lossy(&o).trim().to_string()).unwrap_or_default()
}

pub async fn status() -> Status {
    let Some(p) = tools::ytdlp() else { return Status::default() };
    let managed = tools::data_bin().is_some_and(|d| p.starts_with(d));
    let p2 = p.clone();
    let version = tokio::task::spawn_blocking(move || version_of(&p2)).await.unwrap_or_default();
    Status {
        installed: true,
        version,
        path: p.to_string_lossy().into_owned(),
        managed,
    }
}

/// SHA2-256SUMS 里某个文件的校验值
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == file && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn install() -> Result<Status> {
    let dir: PathBuf = tools::data_bin().ok_or_else(|| anyhow!("数据目录还没准备好"))?.to_path_buf();
    tokio::fs::create_dir_all(&dir).await?;
    let client = reqwest::Client::builder()
        .user_agent("MediaPlayer/1.0")
        .timeout(Duration::from_secs(300))
        .build()?;
    let sums = client
        .get(format!("{BASE}SHA2-256SUMS"))
        .send()
        .await
        .context("连接 GitHub 失败")?
        .error_for_status()?
        .text()
        .await?;
    let want = sum_for(&sums, EXE).ok_or_else(|| anyhow!("发布页里没有 {EXE} 的校验值"))?;
    let bytes = client
        .get(format!("{BASE}{EXE}"))
        .send()
        .await
        .context("下载 yt-dlp 失败")?
        .error_for_status()?
        .bytes()
        .await?;
    if sha256_hex(&bytes) != want {
        bail!("下载的 yt-dlp 校验不通过，已丢弃，请稍后重试");
    }
    let part = dir.join(format!("{EXE}.part"));
    tokio::fs::write(&part, &bytes).await?;
    tokio::fs::rename(&part, dir.join(EXE)).await?;
    Ok(status().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_checksum_line() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = format!("{a}  yt-dlp\n{b}  yt-dlp.exe\n{a}  yt-dlp_x86.exe\n");
        assert_eq!(sum_for(&sums, "yt-dlp.exe"), Some("b".repeat(64)));
        assert_eq!(sum_for(&sums, "yt-dlp"), Some(a.clone()));
        assert_eq!(sum_for(&sums, "nope.exe"), None);
        assert_eq!(sum_for("short  yt-dlp.exe", "yt-dlp.exe"), None);
    }

    #[test]
    fn hashes() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
