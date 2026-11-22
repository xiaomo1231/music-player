//! 漫画：CBZ / ZIP、图片文件夹、PDF（PDF 交给 WebView2 自带的阅读器）
use crate::{metadata, parse};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct ComicInfo {
    pub title: String,
    pub series: String,
    pub format: String,
    pub pages: i64,
    pub cover: Option<String>,
}

/// 自然排序：page2 排在 page10 前面
pub fn natural_key(s: &str) -> Vec<(u8, u64, String)> {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d+|\D+").unwrap());
    RE.find_iter(&s.to_lowercase())
        .map(|m| {
            let t = m.as_str();
            match t.parse::<u64>() {
                Ok(n) => (0, n, String::new()),
                Err(_) => (1, 0, t.to_string()),
            }
        })
        .collect()
}

fn is_image(name: &str) -> bool {
    let e = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    parse::IMAGE.contains(&e.as_str())
}

pub fn zip_pages(path: &Path) -> Result<Vec<String>> {
    let f = std::fs::File::open(path)?;
    let z = zip::ZipArchive::new(f)?;
    let mut names: Vec<String> = z
        .file_names()
        .filter(|n| is_image(n) && !n.starts_with("__MACOSX"))
        .map(|s| s.to_string())
        .collect();
    names.sort_by_key(|n| natural_key(n));
    Ok(names)
}

pub fn folder_pages(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(&p.to_string_lossy()))
        .collect();
    v.sort_by_key(|p| natural_key(&p.file_name().unwrap().to_string_lossy()));
    Ok(v)
}

/// 图片文件夹是否算一本漫画
pub fn is_comic_folder(dir: &Path) -> bool {
    let s = dir.to_string_lossy().to_lowercase();
    if !(s.contains("漫画") || s.contains("comic") || s.contains("manga") || s.contains("漫畫")) {
        return false;
    }
    folder_pages(dir).map(|v| v.len() >= 3).unwrap_or(false)
}

pub fn pdf_page_count(path: &Path) -> i64 {
    // 粗略统计 /Type /Page，足够显示页数
    std::fs::read(path)
        .map(|b| {
            let s = String::from_utf8_lossy(&b);
            static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"/Type\s*/Page[^s]").unwrap());
            RE.find_iter(&s).count() as i64
        })
        .unwrap_or(0)
}

pub fn read_page(path: &Path, idx: usize) -> Result<(Vec<u8>, String)> {
    let mime = |n: &str| {
        match n.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "avif" => "image/avif",
            _ => "image/jpeg",
        }
        .to_string()
    };
    if path.is_dir() {
        let pages = folder_pages(path)?;
        let p = pages.get(idx).ok_or_else(|| anyhow!("没有第 {idx} 页"))?;
        return Ok((std::fs::read(p)?, mime(&p.to_string_lossy())));
    }
    let names = zip_pages(path)?;
    let name = names.get(idx).ok_or_else(|| anyhow!("没有第 {idx} 页"))?;
    let f = std::fs::File::open(path)?;
    let mut z = zip::ZipArchive::new(f)?;
    let mut buf = vec![];
    z.by_name(name)?.read_to_end(&mut buf)?;
    Ok((buf, mime(name)))
}

static RE_VOL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(?:第\s*\d+\s*[卷话話册]|vol\.?\s*\d+|v\d+|\s\d{1,3}$)").unwrap());

pub fn inspect(path: &Path, cover_dir: &Path) -> ComicInfo {
    let stem = if path.is_dir() { path.file_name() } else { path.file_stem() }
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let series = {
        let s = RE_VOL.replace_all(&stem, "").trim().to_string();
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // 卷号文件放在作品文件夹里时，系列名用文件夹名
        if s.is_empty() || RE_VOL.is_match(&stem) && !parent.is_empty() && !["漫画", "comic", "comics", "manga"].contains(&parent.to_lowercase().as_str()) {
            parent
        } else {
            s
        }
    };
    let format = if path.is_dir() { "folder".to_string() } else { parse::ext(path) };
    let (pages, cover) = match format.as_str() {
        "pdf" => (pdf_page_count(path), None),
        _ => {
            let n = if path.is_dir() {
                folder_pages(path).map(|v| v.len()).unwrap_or(0)
            } else {
                zip_pages(path).map(|v| v.len()).unwrap_or(0)
            };
            let cover = read_page(path, 0).ok().and_then(|(b, _)| metadata::save_cover(&b, cover_dir)).map(|c| c.0);
            (n as i64, cover)
        }
    };
    ComicInfo {
        title: stem,
        series,
        format,
        pages,
        cover,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn natural() {
        let mut v = vec!["p10.jpg", "p2.jpg", "p1.jpg"];
        v.sort_by_key(|s| super::natural_key(s));
        assert_eq!(v, vec!["p1.jpg", "p2.jpg", "p10.jpg"]);
    }
}
