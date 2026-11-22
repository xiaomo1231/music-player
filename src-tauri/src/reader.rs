//! 小说：编码识别、TXT 分章、EPUB 解包
use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::io::Read;
use std::path::Path;

/// 自动识别 GBK / UTF-8 / Big5 等编码，返回（文本，编码名）
pub fn decode_text(bytes: &[u8]) -> (String, &'static str) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(rest).into_owned(), "UTF-8");
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (s, _, _) = encoding_rs::UTF_16LE.decode(bytes);
        return (s.into_owned(), "UTF-16LE");
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (s, _, _) = encoding_rs::UTF_16BE.decode(bytes);
        return (s.into_owned(), "UTF-16BE");
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_string(), "UTF-8");
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(&bytes[..bytes.len().min(256 * 1024)], bytes.len() <= 256 * 1024);
    let enc = det.guess(Some(b"cn"), true);
    let (s, enc, _) = enc.decode(bytes);
    (s.into_owned(), enc.name())
}

#[derive(Debug, Serialize, Clone)]
pub struct Chapter {
    pub title: String,
    /// 段落
    pub paras: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Book {
    pub title: String,
    pub author: String,
    pub encoding: String,
    pub chapters: Vec<Chapter>,
}

static RE_CHAPTER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?:(?:第\s*[0-9零〇一二两三四五六七八九十百千万]+\s*[章节回卷集部篇])|(?:卷\s*[0-9零〇一二三四五六七八九十百千]+)|(?:Chapter\s+\d+)|(?:序章|楔子|序言|引子|尾声|后记|番外))[^\n]{0,40}$").unwrap()
});

/// 断行修复：一句话被拆成多行时合并回去；去掉多余空行
pub fn merge_broken_lines(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for l in lines {
        let t = l.trim().trim_start_matches('\u{3000}').trim().to_string();
        if t.is_empty() {
            continue;
        }
        if let Some(prev) = out.last_mut() {
            let end = prev.chars().last().unwrap_or('。');
            // 上一行没有句末标点且很短的续行，合并
            let ends_sentence = "。！？!?…”」』\"'）)】:：".contains(end);
            if !ends_sentence && !RE_CHAPTER.is_match(prev) && !RE_CHAPTER.is_match(&t) && prev.chars().count() < 200 {
                prev.push_str(&t);
                continue;
            }
        }
        out.push(t);
    }
    out
}

pub fn split_txt(text: &str) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = vec![];
    let mut cur = Chapter { title: "开始".into(), paras: vec![] };
    let mut buf: Vec<String> = vec![];
    for line in text.lines() {
        if RE_CHAPTER.is_match(line) && line.trim().chars().count() <= 40 {
            cur.paras = merge_broken_lines(std::mem::take(&mut buf));
            if !cur.paras.is_empty() || !chapters.is_empty() {
                chapters.push(cur);
            }
            cur = Chapter { title: line.trim().to_string(), paras: vec![] };
        } else {
            buf.push(line.to_string());
        }
    }
    cur.paras = merge_broken_lines(buf);
    chapters.push(cur);
    // 去掉重复章节（同标题同开头）
    let mut seen = std::collections::HashSet::new();
    chapters.retain(|c| seen.insert((c.title.clone(), c.paras.first().cloned())));
    // 没有章节标记的长文本，按 3000 字切
    if chapters.len() == 1 && chapters[0].paras.iter().map(|p| p.len()).sum::<usize>() > 30000 {
        let all = std::mem::take(&mut chapters[0].paras);
        chapters.clear();
        let mut c = Chapter { title: "第 1 部分".into(), paras: vec![] };
        let mut n = 0usize;
        for p in all {
            n += p.chars().count();
            c.paras.push(p);
            if n > 3000 {
                let k = chapters.len() + 2;
                chapters.push(std::mem::replace(&mut c, Chapter { title: format!("第 {k} 部分"), paras: vec![] }));
                n = 0;
            }
        }
        if !c.paras.is_empty() {
            chapters.push(c);
        }
    }
    chapters
}

pub fn open_txt(path: &Path) -> Result<Book> {
    let bytes = std::fs::read(path).with_context(|| format!("读取 {}", path.display()))?;
    let (text, enc) = decode_text(&bytes);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let (title, author) = split_title_author(&stem);
    Ok(Book { title, author, encoding: enc.into(), chapters: split_txt(&text) })
}

/// 《书名》作者：xxx / 书名 - 作者
pub fn split_title_author(stem: &str) -> (String, String) {
    static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"《(.+?)》(?:.*?作者[:：]\s*(.+))?").unwrap());
    if let Some(c) = RE.captures(stem) {
        return (c[1].trim().into(), c.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default());
    }
    if let Some((a, b)) = stem.split_once(" - ") {
        return (a.trim().into(), b.trim().into());
    }
    (stem.trim().into(), String::new())
}

static RE_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static RE_BLOCK: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)</?(p|div|br|h[1-6]|li|tr|section|blockquote)[^>]*>").unwrap());
static RE_HEAD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(script|style|head)[^>]*>.*?</(script|style|head)>").unwrap());
static RE_H: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<h[1-3][^>]*>(.*?)</h[1-3]>").unwrap());

pub fn unescape(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

fn html_to_paras(html: &str) -> (Option<String>, Vec<String>) {
    let h = RE_H.captures(html).map(|c| unescape(RE_TAG.replace_all(&c[1], "").trim()));
    let body = RE_HEAD.replace_all(html, "");
    let body = RE_BLOCK.replace_all(&body, "\n");
    let text = RE_TAG.replace_all(&body, "");
    let paras = unescape(&text).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    (h, paras)
}

fn zip_read(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<String> {
    let mut f = z.by_name(name).map_err(|e| anyhow!("{name}: {e}"))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')? + i;
    Some(&tag[i..j])
}

fn join_href(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let mut parts: Vec<&str> = base.rsplit_once('/').map(|(d, _)| d.split('/').collect()).unwrap_or_default();
    for seg in href.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    let joined = parts.join("/");
    // EPUB 里的路径常带 %20
    joined.replace("%20", " ")
}

pub fn open_epub(path: &Path) -> Result<Book> {
    let f = std::fs::File::open(path)?;
    let mut z = zip::ZipArchive::new(f)?;
    let container = zip_read(&mut z, "META-INF/container.xml")?;
    let opf_path = container.find("full-path=\"").and_then(|i| {
        let s = &container[i + 11..];
        s.find('"').map(|j| s[..j].to_string())
    }).ok_or_else(|| anyhow!("EPUB 缺少 OPF"))?;
    let opf = zip_read(&mut z, &opf_path)?;
    static RE_ITEM: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<(?:opf:)?item\s[^>]*>").unwrap());
    static RE_REF: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<(?:opf:)?itemref\s[^>]*>").unwrap());
    static RE_TITLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<dc:title[^>]*>(.*?)</dc:title>").unwrap());
    static RE_CREATOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<dc:creator[^>]*>(.*?)</dc:creator>").unwrap());
    let mut items = std::collections::HashMap::new();
    for m in RE_ITEM.find_iter(&opf) {
        if let (Some(id), Some(href)) = (attr(m.as_str(), "id"), attr(m.as_str(), "href")) {
            items.insert(id.to_string(), join_href(&opf_path, href));
        }
    }
    let mut chapters = vec![];
    for m in RE_REF.find_iter(&opf) {
        let Some(id) = attr(m.as_str(), "idref") else { continue };
        let Some(href) = items.get(id) else { continue };
        let Ok(html) = zip_read(&mut z, href) else { continue };
        let (h, paras) = html_to_paras(&html);
        if paras.is_empty() {
            continue;
        }
        let title = h.unwrap_or_else(|| paras[0].chars().take(30).collect());
        chapters.push(Chapter { title, paras });
    }
    let title = RE_TITLE.captures(&opf).map(|c| unescape(c[1].trim())).unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().into());
    let author = RE_CREATOR.captures(&opf).map(|c| unescape(c[1].trim())).unwrap_or_default();
    Ok(Book { title, author, encoding: "UTF-8".into(), chapters })
}

/// EPUB 封面图片字节
pub fn epub_cover(path: &Path) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let names: Vec<String> = z.file_names().map(|s| s.to_string()).collect();
    let pick = names.iter().find(|n| {
        let l = n.to_lowercase();
        l.contains("cover") && (l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png"))
    })?;
    let mut buf = vec![];
    z.by_name(pick).ok()?.read_to_end(&mut buf).ok()?;
    Some(buf)
}

pub fn open_book(path: &Path) -> Result<Book> {
    match crate::parse::ext(path).as_str() {
        "epub" => open_epub(path),
        _ => open_txt(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gbk_detect() {
        let (bytes, _, _) = encoding_rs::GBK.encode("第一章 雾港\n雾是从后半夜开始涨起来的。先是漫过防波堤，再漫过码头上那一排旧缆桩。");
        let (s, enc) = decode_text(&bytes);
        assert!(s.contains("雾港"), "{s}");
        assert_eq!(enc, "GBK");
    }

    #[test]
    fn chapters_split() {
        let t = "简介\n\n第一章 开始\n他走了\n进来。\n第二章 结束\n她说：“好。”\n";
        let c = split_txt(t);
        assert_eq!(c.len(), 3);
        assert_eq!(c[1].title, "第一章 开始");
        assert_eq!(c[1].paras, vec!["他走了进来。"]);
    }

    #[test]
    fn title_author() {
        assert_eq!(split_title_author("《雾港纪事》作者：阿砚"), ("雾港纪事".into(), "阿砚".into()));
    }

    #[test]
    fn href_join() {
        assert_eq!(join_href("OEBPS/content.opf", "Text/ch1.xhtml"), "OEBPS/Text/ch1.xhtml");
        assert_eq!(join_href("OEBPS/Text/a.xhtml", "../Images/c.jpg"), "OEBPS/Images/c.jpg");
    }
}
