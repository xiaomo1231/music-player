//! 种子 / 磁力下载前的确认：先拿到种子元数据（磁力要先从 DHT / 节点拿），列出里面的文件、体积和识别出的类型，
//! 让用户勾选要下载的文件。元数据存成本地 .torrent，之后的下载任务直接用它，不用再等一次磁力解析。
use super::{download, load_settings, source, with_db};
use crate::{parse, AppState};
use anyhow::{anyhow, bail, Context, Result};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// 磁力链接拿元数据最多等多久
const META_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    /// 在种子里的文件序号（下载时按它选文件）
    pub index: usize,
    /// 种子里的相对路径，用 / 分隔
    pub path: String,
    pub size: u64,
    /// video / audio / sub / image / doc / archive / other
    pub kind: String,
    pub season: Option<i64>,
    pub episode: Option<f64>,
    /// 特典、OP / ED、PV、菜单之类
    pub special: bool,
    /// 给界面看的识别结果，例如“第 3 集”“S02E05”“特典”“字幕”
    pub label: String,
    /// 建议默认勾选（视频、音频、字幕）
    pub suggested: bool,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TorrentInfo {
    pub name: String,
    pub info_hash: String,
    pub total_size: u64,
    /// 保存下来的 .torrent 文件，下载任务用它
    pub torrent_path: String,
    pub files: Vec<TorrentFile>,
    /// 识别出的作品名（取视频文件里最常见的一个）
    pub work: String,
    /// 正片集数
    pub episodes: usize,
    pub specials: usize,
}

const DOC: &[&str] = &["txt", "nfo", "md", "pdf", "url", "htm", "html", "doc", "docx", "log", "cue"];
const ARCHIVE: &[&str] = &["zip", "rar", "7z", "tar", "gz"];

fn file_kind(p: &Path) -> &'static str {
    let e = parse::ext(p);
    let e = e.as_str();
    if parse::VIDEO.contains(&e) {
        "video"
    } else if parse::AUDIO.contains(&e) {
        "audio"
    } else if parse::SUBS.contains(&e) {
        "sub"
    } else if parse::IMAGE.contains(&e) {
        "image"
    } else if DOC.contains(&e) {
        "doc"
    } else if ARCHIVE.contains(&e) {
        "archive"
    } else {
        "other"
    }
}

fn fmt_ep(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// S02E05 里的集号补两位
fn fmt_ep2(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{:02}", n as i64)
    } else {
        format!("{n}")
    }
}

/// 把种子里的文件列表（相对路径、大小）整理成界面要的样子；补齐文件（padding）去掉
pub fn describe_files(raw: &[(String, u64, bool)]) -> Vec<TorrentFile> {
    raw.iter()
        .enumerate()
        .filter(|(_, (path, _, padding))| !*padding && !is_padding_name(path))
        .map(|(index, (path, size, _))| {
            let p = Path::new(path);
            let kind = file_kind(p);
            let (mut season, mut episode, mut special) = (None, None, false);
            if kind == "video" || kind == "audio" || kind == "sub" {
                let ep = parse::parse_episode(p);
                special = ep.special || in_special_dir(path);
                episode = ep.episode;
                season = if special { None } else { ep.season };
            }
            let label = match kind {
                "video" | "audio" => {
                    let what = if kind == "video" { "视频" } else { "音频" };
                    match (special, season, episode) {
                        (true, _, Some(e)) => format!("特典 {}", fmt_ep(e)),
                        (true, _, None) => "特典".into(),
                        (false, Some(s), Some(e)) => format!("S{s:02}E{}", fmt_ep2(e)),
                        (false, None, Some(e)) => format!("第 {} 集", fmt_ep(e)),
                        _ => what.into(),
                    }
                }
                "sub" => "字幕".into(),
                "image" => "图片".into(),
                "doc" => "说明文件".into(),
                "archive" => "压缩包".into(),
                _ => "其他".into(),
            };
            TorrentFile {
                index,
                path: path.clone(),
                size: *size,
                kind: kind.into(),
                season,
                episode,
                special,
                label,
                suggested: matches!(kind, "video" | "audio" | "sub"),
            }
        })
        .collect()
}

fn is_padding_name(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("_____padding_file") || name.starts_with(".pad")
}

/// SPs / Specials / 特典 / Extras 这类文件夹里的视频算特典
fn in_special_dir(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.iter().any(|d| {
        let l = d.to_lowercase();
        matches!(
            l.as_str(),
            "sp" | "sps" | "special" | "specials" | "extra" | "extras" | "bonus" | "menu" | "menus" | "cds" | "scans"
        ) || l.contains("特典")
            || l.contains("映像特典")
            || l.contains("花絮")
    })
}

/// 识别出的作品名：视频文件解析出的名字里出现最多的那个，没有就用种子名
fn guess_work(files: &[TorrentFile], name: &str) -> String {
    let mut count: Vec<(String, usize)> = vec![];
    for f in files.iter().filter(|f| f.kind == "video" && !f.special) {
        let t = parse::parse_episode(Path::new(&f.path)).title;
        if t.is_empty() {
            continue;
        }
        match count.iter_mut().find(|(k, _)| *k == t) {
            Some((_, n)) => *n += 1,
            None => count.push((t, 1)),
        }
    }
    count.sort_by_key(|c| std::cmp::Reverse(c.1));
    count.into_iter().next().map(|(t, _)| t).unwrap_or_else(|| {
        let t = parse::parse_episode(Path::new(&format!("{name}.mkv"))).title;
        if t.is_empty() {
            name.to_string()
        } else {
            t
        }
    })
}

fn build_info(name: String, info_hash: String, torrent_path: String, raw: &[(String, u64, bool)]) -> TorrentInfo {
    let files = describe_files(raw);
    let total_size = files.iter().map(|f| f.size).sum();
    let episodes = files.iter().filter(|f| f.kind == "video" && !f.special && f.episode.is_some()).count();
    let specials = files.iter().filter(|f| f.kind == "video" && f.special).count();
    let work = guess_work(&files, &name);
    TorrentInfo {
        name,
        info_hash,
        total_size,
        torrent_path,
        files,
        work,
        episodes,
        specials,
    }
}

/// 解析磁力链接 / 种子网址 / 本地种子文件，返回文件列表。不会开始下载
pub async fn inspect(app: &AppHandle, url: &str) -> Result<TorrentInfo> {
    let url = url.trim();
    let kind = source::kind_of_url(url);
    if kind != "magnet" && kind != "torrent" {
        bail!("不是磁力链接或种子");
    }
    let settings = with_db(app, |c| Ok(load_settings(c)))?;
    let dl_dir = PathBuf::from(&settings.download_dir);
    tokio::fs::create_dir_all(&dl_dir).await?;
    let session = download::session(&dl_dir).await?;
    let lo = list_only(&session, url).await?;
    // 存一份 .torrent，磁力链接下次不用再等元数据
    let dir = app.state::<AppState>().dirs.data.join("torrents");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{}.torrent", lo.info_hash));
    tokio::fs::write(&path, &lo.torrent_bytes).await.context("保存种子文件失败")?;
    Ok(build_info(lo.name, lo.info_hash, path.to_string_lossy().into_owned(), &lo.raw))
}

/// 只读元数据得到的东西：名字、info hash、文件列表（相对路径、大小、是否补齐文件）、.torrent 内容
pub struct Listed {
    pub name: String,
    pub info_hash: String,
    pub raw: Vec<(String, u64, bool)>,
    pub torrent_bytes: Vec<u8>,
}

/// 用 librqbit 的 list_only 读种子信息（磁力链接会先去拿元数据），不开始下载
pub async fn list_only(session: &Arc<Session>, url: &str) -> Result<Listed> {
    let add = if url.starts_with("magnet:") || source::is_http(url) {
        AddTorrent::from_url(url.to_string())
    } else {
        AddTorrent::from_local_filename(url)?
    };
    let opts = AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };
    let resp = tokio::time::timeout(META_TIMEOUT, session.add_torrent(add, Some(opts)))
        .await
        .map_err(|_| anyhow!("等了 {} 秒还没拿到种子信息，可能没有人在分享，稍后再试", META_TIMEOUT.as_secs()))?
        .context("读取种子信息失败")?;
    let AddTorrentResponse::ListOnly(lo) = resp else {
        bail!("读取种子信息失败")
    };
    let info_hash = lo.info_hash.as_string();
    let name = lo.info.name().map(|n| n.to_string()).unwrap_or_else(|| info_hash.clone());
    let raw = lo
        .info
        .iter_file_details()
        .map(|d| (d.filename.to_vec().join("/"), d.len, d.attrs().padding))
        .collect();
    Ok(Listed {
        name,
        info_hash,
        raw,
        torrent_bytes: lo.torrent_bytes.to_vec(),
    })
}

/// 选中文件之外的文件：librqbit 会建出空文件（边界分片还会写进一点数据），下完后删掉，免得被整理入库
pub fn remove_unselected(out_dir: &Path, all: &[PathBuf], selected: &[usize]) {
    for (i, rel) in all.iter().enumerate() {
        if selected.contains(&i) {
            continue;
        }
        let p = out_dir.join(rel);
        if p.starts_with(out_dir) && p.is_file() {
            let _ = std::fs::remove_file(&p);
        }
        // 顺手删掉因此变空的文件夹
        let mut dir = p.parent();
        while let Some(d) = dir {
            if d == out_dir || !d.starts_with(out_dir) || std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
}

/// 数据库里存的选中文件（JSON 数组）；空表示全部
pub fn parse_selection(s: &str) -> Option<Vec<usize>> {
    let v: Vec<usize> = serde_json::from_str(s.trim()).ok()?;
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(list: &[(&str, u64)]) -> Vec<(String, u64, bool)> {
        list.iter().map(|(p, s)| (p.to_string(), *s, false)).collect()
    }

    #[test]
    fn recognizes_episodes_specials_and_extras() {
        let r = raw(&[
            ("[Group] Show - 01 [1080p].mkv", 1_000),
            ("[Group] Show - 02 [1080p].mkv", 1_000),
            ("SPs/[Group] Show NCOP [1080p].mkv", 100),
            ("[Group] Show - 01 [1080p].ass", 10),
            ("readme.txt", 1),
            ("_____padding_file_0_xxx", 5),
        ]);
        let info = build_info("[Group] Show".into(), "h".into(), "p".into(), &r);
        assert_eq!(info.files.len(), 5, "补齐文件不显示");
        assert_eq!(info.total_size, 2_111);
        assert_eq!(info.files[0].label, "第 1 集");
        assert_eq!(info.files[0].episode, Some(1.0));
        assert!(info.files[2].special);
        assert_eq!(info.files[2].label, "特典");
        assert_eq!(info.files[3].label, "字幕");
        assert!(info.files[3].suggested);
        assert_eq!(info.files[4].kind, "doc");
        assert!(!info.files[4].suggested);
        assert_eq!(info.episodes, 2);
        assert_eq!(info.specials, 1);
        assert_eq!(info.work, "Show");
    }

    #[test]
    fn keeps_original_index_after_padding() {
        let mut r = raw(&[("a/E01.mkv", 1), ("a/E02.mkv", 1)]);
        r.insert(1, ("a/pad".into(), 1, true));
        let files = describe_files(&r);
        assert_eq!(files.iter().map(|f| f.index).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn season_episode_label() {
        let files = describe_files(&raw(&[("Show.S02E05.1080p.mkv", 1)]));
        assert_eq!(files[0].label, "S02E05");
    }

    #[test]
    fn selection_round_trip() {
        assert_eq!(parse_selection("[0,3]"), Some(vec![0, 3]));
        assert_eq!(parse_selection(""), None);
        assert_eq!(parse_selection("[]"), None);
    }

    #[test]
    fn removes_only_unselected() {
        let dir = std::env::temp_dir().join(format!("mp-torrent-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("SPs")).unwrap();
        std::fs::write(dir.join("E01.mkv"), b"x").unwrap();
        std::fs::write(dir.join("SPs/NCOP.mkv"), b"x").unwrap();
        let all = vec![PathBuf::from("E01.mkv"), PathBuf::from("SPs/NCOP.mkv")];
        remove_unselected(&dir, &all, &[0]);
        assert!(dir.join("E01.mkv").exists());
        assert!(!dir.join("SPs").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
