//! 自动归类入库：先删除可执行文件，再认出媒体（视频 / 音乐），按模板重命名，移动（或复制）到整理目录，
//! 入库并通知界面。认不出的文件留在下载目录，由任务标记为“需要手动整理”。
//! 模板（和 organize 模块各自独立实现）：
//!   番剧\作品名\Season 01\作品名 - S01E03.ext
//!   音乐\艺人\专辑 (年份)\03 - 曲名.ext
//!   视频\名字\原文件名
use crate::{indexer, parse, AppState};
use anyhow::Result;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, ItemKey};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

/// 这些扩展名一律删除（不会移入媒体库）
pub const EXE_EXT: &[&str] = &[
    "exe", "bat", "cmd", "msi", "msp", "scr", "ps1", "vbs", "vbe", "js", "jse", "wsf", "hta", "lnk", "com", "pif", "cpl", "jar", "dll", "reg", "sh", "appx",
];

pub fn is_executable(p: &Path) -> bool {
    EXE_EXT.contains(&parse::ext(p).as_str())
}

/// 整理结果
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// 第一个整理到的文件路径（没有整理到就是源路径）
    pub save_path: String,
    /// 有文件认不出或没整理成功，需要手动处理
    pub manual: bool,
    pub note: String,
}

/// 去掉 Windows 文件名里不能用的字符，压缩空白
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let s = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        "未命名".into()
    } else {
        s
    }
}

/// 集数写法：整数 E03，带小数 E03.5
pub fn ep_text(ep: f64) -> String {
    if ep.fract() == 0.0 {
        format!("E{:02}", ep as i64)
    } else {
        format!("E{:04.1}", ep)
    }
}

/// 番剧：作品名\Season 01\作品名 - S01E03.ext（没有集数时保留原文件名）；season 为 0 时放到 Specials
pub fn anime_rel(title: &str, season: i64, episode: Option<f64>, ext: &str, orig: &str) -> PathBuf {
    let t = sanitize(title);
    let sdir = if season == 0 { "Specials".to_string() } else { format!("Season {season:02}") };
    let file = match episode {
        Some(ep) => format!("{t} - S{season:02}{}.{ext}", ep_text(ep)),
        None => sanitize(orig),
    };
    PathBuf::from("番剧").join(&t).join(sdir).join(file)
}

/// 音乐：艺人\专辑 (年份)\03 - 曲名.ext
pub fn music_rel(artist: &str, album: &str, year: Option<i64>, track: Option<i64>, title: &str, ext: &str) -> PathBuf {
    let album_dir = match year {
        Some(y) => format!("{} ({y})", sanitize(album)),
        None => sanitize(album),
    };
    let file = match track {
        Some(n) => format!("{n:02} - {}.{ext}", sanitize(title)),
        None => format!("{}.{ext}", sanitize(title)),
    };
    PathBuf::from("音乐").join(sanitize(artist)).join(album_dir).join(file)
}

/// 普通视频：视频\名字\原文件名
pub fn video_rel(title: &str, orig: &str) -> PathBuf {
    PathBuf::from("视频").join(sanitize(title)).join(sanitize(orig))
}

/// 目标已存在时加 (2)、(3)……
pub fn unique(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|s| s.to_string_lossy().into_owned());
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    for n in 2..1000 {
        let name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let c = dir.join(name);
        if !c.exists() {
            return c;
        }
    }
    p
}

/// 读音乐标签：艺人、专辑、年份、曲号、曲名（缺艺人标签返回 None，交给手动整理）
fn music_plan(path: &Path) -> Option<PathBuf> {
    let tf = lofty::read_from_path(path).ok()?;
    let tag = tf.primary_tag().or_else(|| tf.first_tag())?;
    let nonempty = |s: String| {
        let t = s.trim().to_string();
        (!t.is_empty()).then_some(t)
    };
    let artist = tag
        .artist()
        .map(|s| s.to_string())
        .and_then(nonempty)
        .or_else(|| tag.get_string(ItemKey::AlbumArtist).map(|s| s.to_string()).and_then(nonempty))?;
    let album = tag.album().map(|s| s.to_string()).and_then(nonempty).unwrap_or_else(|| "未知专辑".into());
    let title = tag
        .title()
        .map(|s| s.to_string())
        .and_then(nonempty)
        .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let year = tag.date().map(|d| d.year as i64);
    let track = tag.track().map(|n| n as i64);
    let ext = parse::ext(path);
    Some(music_rel(&artist, &album, year, track, &title, &ext))
}

/// 决定一个媒体文件的目标相对路径；None 表示认不出
fn plan(path: &Path, media_kind: &str) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if parse::classify(path) == Some("music") {
        return music_plan(path);
    }
    let info = parse::parse_episode(path);
    let ext = parse::ext(path);
    if info.episode.is_some() || media_kind == "anime" {
        let title = info.title.trim().to_string();
        if title.is_empty() {
            return None;
        }
        let season = if info.special { 0 } else { info.season.unwrap_or(1) };
        return Some(anime_rel(&title, season, info.episode, &ext, &name));
    }
    let title = if info.title.trim().is_empty() {
        path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        info.title.clone()
    };
    Some(video_rel(&title, &name))
}

fn is_main_media(p: &Path) -> bool {
    matches!(parse::classify(p), Some("music") | Some("video"))
}

fn is_companion(p: &Path) -> bool {
    let e = parse::ext(p);
    parse::SUBS.contains(&e.as_str()) || parse::IMAGE.contains(&e.as_str()) || e == "lrc"
}

/// 递归收集文件；可执行文件直接删除
fn collect(p: &Path, depth: usize, out: &mut Vec<PathBuf>, removed: &mut usize) {
    if p.is_file() {
        if is_executable(p) {
            if std::fs::remove_file(p).is_ok() {
                *removed += 1;
            }
        } else {
            out.push(p.to_path_buf());
        }
        return;
    }
    if depth > 6 {
        return;
    }
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            collect(&e.path(), depth + 1, out, removed);
        }
    }
}

/// 复制或移动（跨盘时先复制再删除源文件）
fn place(src: &Path, dst: &Path, copy: bool) -> Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    if copy {
        std::fs::copy(src, dst)?;
        return Ok(());
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst)?;
    std::fs::remove_file(src)?;
    Ok(())
}

/// 删除下载文件夹里变空的子文件夹（不删非空的，也不删 src 本身）
fn remove_empty_dirs(dir: &Path, depth: usize) {
    if depth > 6 {
        return;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                remove_empty_dirs(&p, depth + 1);
                let _ = std::fs::remove_dir(&p);
            }
        }
    }
}

/// 整理一个下载完成的文件或文件夹。copy=true 时保留源文件（做种期间用）
pub fn finish(app: &AppHandle, src: &Path, media_kind: &str, org_root: &Path, copy: bool) -> Result<Outcome> {
    let mut files = vec![];
    let mut removed = 0usize;
    collect(src, 0, &mut files, &mut removed);
    let removed_note = if removed > 0 {
        format!("，已删除 {removed} 个可执行文件")
    } else {
        String::new()
    };
    let media: Vec<PathBuf> = files.iter().filter(|p| is_main_media(p)).cloned().collect();
    let companions: Vec<PathBuf> = files.iter().filter(|p| is_companion(p)).cloned().collect();
    if media.is_empty() {
        return Ok(Outcome {
            save_path: src.to_string_lossy().into_owned(),
            manual: true,
            note: format!("没有认出可以整理的媒体文件{removed_note}"),
        });
    }

    let mut placed: Vec<(PathBuf, &'static str)> = vec![];
    let mut manual: Vec<String> = vec![];
    let mut moved_companions: HashSet<PathBuf> = HashSet::new();
    for m in &media {
        let kind: &'static str = if parse::classify(m) == Some("music") { "music" } else { "video" };
        let Some(rel) = plan(m, media_kind) else {
            manual.push(m.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            continue;
        };
        let dst = unique(org_root.join(rel));
        if let Err(err) = place(m, &dst, copy) {
            manual.push(format!("{}（{err}）", m.display()));
            continue;
        }
        placed.push((dst.clone(), kind));

        // 同名的字幕、封面跟着一起移动，文件名后缀保留（如 .zh.srt）
        let ms = m.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let dstem = dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        for c in &companions {
            if moved_companions.contains(c) || c.parent() != m.parent() {
                continue;
            }
            let cs = c.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if !cs.starts_with(&ms) {
                continue;
            }
            let suffix = &cs[ms.len()..];
            let cext = parse::ext(c);
            let dst_c = unique(dst.with_file_name(format!("{dstem}{suffix}.{cext}")));
            if place(c, &dst_c, copy).is_ok() {
                moved_companions.insert(c.clone());
            }
        }
    }

    if !copy {
        remove_empty_dirs(src, 0);
    }
    if !placed.is_empty() {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        for (p, kind) in &placed {
            let _ = indexer::index_file(&conn, p, kind, &st.dirs);
        }
        drop(conn);
        let _ = app.emit("library://changed", ());
    }

    let save_path = placed
        .first()
        .map(|(p, _)| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| src.to_string_lossy().into_owned());
    let manual_flag = placed.is_empty() || !manual.is_empty();
    let note = if manual.is_empty() {
        format!("已整理 {} 个文件{removed_note}", placed.len())
    } else {
        format!(
            "已整理 {} 个，{} 个需要手动整理{removed_note}：{}",
            placed.len(),
            manual.len(),
            manual.join("、")
        )
    };
    Ok(Outcome {
        save_path,
        manual: manual_flag,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_bad_chars() {
        assert_eq!(sanitize("a/b:c*d?"), "a b c d");
        assert_eq!(sanitize("  name.  "), "name");
        assert_eq!(sanitize("..."), "未命名");
    }

    #[test]
    fn episode_text() {
        assert_eq!(ep_text(3.0), "E03");
        assert_eq!(ep_text(12.0), "E12");
        assert_eq!(ep_text(3.5), "E03.5");
    }

    #[test]
    fn anime_template() {
        assert_eq!(
            anime_rel("星屑邮局", 1, Some(3.0), "mkv", "x.mkv"),
            PathBuf::from("番剧").join("星屑邮局").join("Season 01").join("星屑邮局 - S01E03.mkv")
        );
        assert_eq!(
            anime_rel("星屑邮局", 0, Some(1.0), "mp4", "sp.mp4"),
            PathBuf::from("番剧").join("星屑邮局").join("Specials").join("星屑邮局 - S00E01.mp4")
        );
        // 没有集数：保留原文件名
        assert_eq!(
            anime_rel("星屑邮局", 2, None, "mkv", "extra.mkv"),
            PathBuf::from("番剧").join("星屑邮局").join("Season 02").join("extra.mkv")
        );
    }

    #[test]
    fn music_template() {
        assert_eq!(
            music_rel("林间信号", "潮汐档案", Some(2024), Some(3), "防波堤", "flac"),
            PathBuf::from("音乐").join("林间信号").join("潮汐档案 (2024)").join("03 - 防波堤.flac")
        );
        assert_eq!(
            music_rel("艺人", "专辑", None, None, "曲", "mp3"),
            PathBuf::from("音乐").join("艺人").join("专辑").join("曲.mp3")
        );
    }

    #[test]
    fn executables_are_recognized() {
        assert!(is_executable(Path::new("setup.EXE")));
        assert!(is_executable(Path::new("run.ps1")));
        assert!(!is_executable(Path::new("movie.mkv")));
        assert!(!is_executable(Path::new("readme.txt")));
    }

    #[test]
    fn media_and_companions() {
        assert!(is_main_media(Path::new("a.mkv")));
        assert!(is_main_media(Path::new("a.flac")));
        assert!(!is_main_media(Path::new("a.srt")));
        assert!(is_companion(Path::new("a.zh.srt")));
        assert!(!is_companion(Path::new("a.zip")));
    }

    #[test]
    fn video_template() {
        assert_eq!(video_rel("电影", "电影.mp4"), PathBuf::from("视频").join("电影").join("电影.mp4"));
    }
}
