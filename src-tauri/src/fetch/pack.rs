//! 下载源的“按章节”结果打包：漫画图片 → CBZ，小说正文 → EPUB。
//! 打包好的文件交给整理入库，阅读器直接打开（漫画走 comic.rs，小说走 reader.rs）。
use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

/// 小说的一章：标题 + 正文（纯文本，一行一段）
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TextChapter {
    pub title: String,
    pub text: String,
}

/// 图片扩展名：先看网址，再看文件头，都认不出时用 jpg
pub fn image_ext(url: &str, bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        return "png";
    }
    if bytes.starts_with(b"GIF8") {
        return "gif";
    }
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "webp";
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return "jpg";
    }
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    for e in ["png", "gif", "webp", "avif", "bmp", "jpeg", "jpg"] {
        if path.ends_with(&format!(".{e}")) {
            return if e == "jpeg" { "jpg" } else { e };
        }
    }
    "jpg"
}

/// 第 i 页（从 0 开始）在临时文件夹里的文件名前缀：0001、0002……
pub fn page_stem(i: usize) -> String {
    format!("{:04}", i + 1)
}

/// 临时文件夹里已经下好的第 i 页（断点续传用）
pub fn existing_page(dir: &Path, i: usize) -> Option<PathBuf> {
    let stem = page_stem(i);
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file() && p.file_stem().map(|s| s.to_string_lossy() == stem).unwrap_or(false) && !p.to_string_lossy().ends_with(".part"))
}

/// 把文件夹里的图片按文件名顺序打包成 CBZ（图片本身已压缩，用存储模式）
pub fn write_cbz(pages_dir: &Path, out: &Path) -> Result<usize> {
    let mut pages: Vec<PathBuf> = std::fs::read_dir(pages_dir)
        .with_context(|| format!("读不到 {}", pages_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.to_string_lossy().ends_with(".part"))
        .collect();
    pages.sort();
    if pages.is_empty() {
        anyhow::bail!("没有下载到任何图片");
    }
    let tmp = out.with_extension("cbz.part");
    let f = std::fs::File::create(&tmp)?;
    let mut z = zip::ZipWriter::new(f);
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for p in &pages {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        z.start_file(name, opt)?;
        z.write_all(&std::fs::read(p)?)?;
    }
    z.finish()?;
    std::fs::rename(&tmp, out)?;
    Ok(pages.len())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 写一个最小的 EPUB 3（带 EPUB 2 的 toc.ncx，老阅读器也能打开）
pub fn write_epub(out: &Path, title: &str, author: &str, chapters: &[TextChapter]) -> Result<()> {
    if chapters.is_empty() {
        anyhow::bail!("没有任何章节");
    }
    let title = if title.trim().is_empty() { "未命名" } else { title.trim() };
    let tmp = out.with_extension("epub.part");
    let f = std::fs::File::create(&tmp)?;
    let mut z = zip::ZipWriter::new(f);
    // mimetype 必须是第一个文件且不压缩
    z.start_file("mimetype", SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored))?;
    z.write_all(b"application/epub+zip")?;
    let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("META-INF/container.xml", opt)?;
    z.write_all(
        br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
    )?;

    let mut manifest = String::new();
    let mut spine = String::new();
    let mut nav = String::new();
    let mut ncx = String::new();
    for (i, ch) in chapters.iter().enumerate() {
        let n = i + 1;
        let ct = if ch.title.trim().is_empty() {
            format!("第 {n} 章")
        } else {
            ch.title.trim().to_string()
        };
        let body: String = ch
            .text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| format!("<p>{}</p>\n", xml_escape(l)))
            .collect();
        let html = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><meta charset=\"UTF-8\"/><title>{t}</title></head>\n<body><h2>{t}</h2>\n{body}</body></html>",
            t = xml_escape(&ct)
        );
        z.start_file(format!("OEBPS/c{n:04}.xhtml"), opt)?;
        z.write_all(html.as_bytes())?;
        manifest.push_str(&format!("<item id=\"c{n}\" href=\"c{n:04}.xhtml\" media-type=\"application/xhtml+xml\"/>\n"));
        spine.push_str(&format!("<itemref idref=\"c{n}\"/>\n"));
        nav.push_str(&format!("<li><a href=\"c{n:04}.xhtml\">{}</a></li>\n", xml_escape(&ct)));
        ncx.push_str(&format!(
            "<navPoint id=\"p{n}\" playOrder=\"{n}\"><navLabel><text>{}</text></navLabel><content src=\"c{n:04}.xhtml\"/></navPoint>\n",
            xml_escape(&ct)
        ));
    }
    let uid = format!("urn:mp:{}", blake3::hash(format!("{title}\n{author}").as_bytes()).to_hex());
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="uid">{uid}</dc:identifier>
<dc:title>{t}</dc:title>
<dc:creator>{a}</dc:creator>
<dc:language>zh</dc:language>
</metadata>
<manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{manifest}</manifest>
<spine toc="ncx">
{spine}</spine>
</package>"#,
        t = xml_escape(title),
        a = xml_escape(author.trim()),
    );
    z.start_file("OEBPS/content.opf", opt)?;
    z.write_all(opf.as_bytes())?;
    let nav_doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><meta charset=\"UTF-8\"/><title>目录</title></head>\n<body><nav epub:type=\"toc\"><ol>\n{nav}</ol></nav></body></html>"
    );
    z.start_file("OEBPS/nav.xhtml", opt)?;
    z.write_all(nav_doc.as_bytes())?;
    let ncx_doc = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head><meta name=\"dtb:uid\" content=\"{uid}\"/></head>\n<docTitle><text>{}</text></docTitle><navMap>\n{ncx}</navMap></ncx>",
        xml_escape(title)
    );
    z.start_file("OEBPS/toc.ncx", opt)?;
    z.write_all(ncx_doc.as_bytes())?;
    z.finish()?;
    std::fs::rename(&tmp, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mp-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn image_ext_by_magic_then_url() {
        assert_eq!(image_ext("https://a/x", b"\x89PNG\r\n"), "png");
        assert_eq!(image_ext("https://a/x", b"RIFF\0\0\0\0WEBPVP8 "), "webp");
        assert_eq!(image_ext("https://a/x", &[0xFF, 0xD8, 0xFF]), "jpg");
        assert_eq!(image_ext("https://a/p.AVIF?t=1", b"...."), "avif");
        assert_eq!(image_ext("https://a/p", b"...."), "jpg");
    }

    #[test]
    fn cbz_keeps_page_order_and_opens() {
        let d = tmp("cbz");
        let pages = d.join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        // 故意乱序写入，打包后应按 0001、0002…… 排
        for i in [2usize, 0, 1] {
            std::fs::write(pages.join(format!("{}.png", page_stem(i))), b"\x89PNG fake").unwrap();
        }
        std::fs::write(pages.join("0004.jpg.part"), b"half").unwrap();
        assert_eq!(existing_page(&pages, 1), Some(pages.join("0002.png")));
        assert_eq!(existing_page(&pages, 3), None);
        let out = d.join("星屑 第3话.cbz");
        assert_eq!(write_cbz(&pages, &out).unwrap(), 3);
        assert_eq!(crate::comic::zip_pages(&out).unwrap(), vec!["0001.png", "0002.png", "0003.png"]);
        assert!(write_cbz(&d.join("none"), &d.join("x.cbz")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn epub_round_trips_through_reader() {
        let d = tmp("epub");
        let out = d.join("潮汐档案 - 林间.epub");
        let chapters = vec![
            TextChapter {
                title: "第一章 起点".into(),
                text: "第一段 <b>\n\n第二段 & 结束".into(),
            },
            TextChapter {
                title: String::new(),
                text: "只有正文".into(),
            },
        ];
        write_epub(&out, "潮汐档案", "林间", &chapters).unwrap();
        let book = crate::reader::open_epub(&out).unwrap();
        assert_eq!(book.title, "潮汐档案");
        assert_eq!(book.author, "林间");
        assert_eq!(book.chapters.len(), 2);
        assert_eq!(book.chapters[0].title, "第一章 起点");
        assert!(book.chapters[0].paras.iter().any(|p| p.contains("第二段 & 结束")));
        assert_eq!(book.chapters[1].title, "第 2 章");
        assert!(write_epub(&d.join("空.epub"), "空", "", &[]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
