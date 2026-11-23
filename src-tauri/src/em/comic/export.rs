//! 按规则导出新 CBZ：拆、裁、跳过之后再按 export 转格式、缩小、切长图、改名，写 ComicInfo.xml
use super::img::{self, OutFmt};
use super::spec::{render_name, Format, Spec};
use super::view::{base_name, Book, View};
use anyhow::{anyhow, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Meta<'a> {
    pub rule_id: &'a str,
    pub rule_version: &'a str,
    pub title: &'a str,
    pub series: &'a str,
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn comic_info(m: &Meta, manga: Option<&str>, pages: usize) -> String {
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ComicInfo xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n");
    if !m.title.is_empty() {
        x += &format!("  <Title>{}</Title>\n", xml_escape(m.title));
    }
    if !m.series.is_empty() {
        x += &format!("  <Series>{}</Series>\n", xml_escape(m.series));
    }
    x += &format!("  <PageCount>{pages}</PageCount>\n");
    if let Some(mg) = manga {
        x += &format!("  <Manga>{mg}</Manga>\n");
    }
    let ver = if m.rule_version.is_empty() {
        String::new()
    } else {
        format!("@{}", m.rule_version)
    };
    x += &format!("  <Notes>em:{}{}</Notes>\n", xml_escape(m.rule_id), xml_escape(&ver));
    x += "</ComicInfo>\n";
    x
}

/// 输出文件名：`<原名> [排版].cbz`，重名时加 (2)、(3)…
pub fn target_for(path: &Path) -> PathBuf {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = if path.is_dir() { path.file_name() } else { path.file_stem() }
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "漫画".into());
    let mut out = dir.join(format!("{stem} [排版].cbz"));
    let mut k = 2;
    while out.exists() {
        out = dir.join(format!("{stem} [排版] ({k}).cbz"));
        k += 1;
    }
    out
}

/// 把 view 按 spec 写成 CBZ（先写 .part 再改名）。on_page(done, total)
pub fn write(view: &View, spec: &Spec, meta: &Meta, out: &Path, mut on_page: impl FnMut(usize, usize)) -> Result<usize> {
    let tmp = out.with_extension("cbz.part");
    let r = (|| -> Result<usize> {
        let f = std::fs::File::create(&tmp)?;
        let mut zw = zip::ZipWriter::new(f);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut book = Book::open(&view.path)?;
        let ex = &spec.export;
        let total = view.len();
        let mut n = 0usize;
        let mut names = std::collections::HashSet::new();
        for idx in 0..total {
            let (bytes, name) = view.raw(&mut book, idx)?;
            let orig_ext = name.rsplit('.').next().unwrap_or("jpg").to_ascii_lowercase();
            let orig_stem = {
                let b = base_name(name);
                b.rsplit_once('.').map(|x| x.0).unwrap_or(b).to_string()
            };
            let decoded = img::decode(&bytes);
            // 不用动的页：原样拷贝
            let untouched = |im: &image::DynamicImage| {
                ex.format == Format::Keep
                    && ex.max_height.map(|m| im.height() <= m).unwrap_or(true)
                    && ex.slice.as_ref().map(|s| im.height() <= s.max_height).unwrap_or(true)
            };
            let mut outs: Vec<(Vec<u8>, String)> = vec![];
            match decoded {
                None => outs.push((bytes, orig_ext.clone())),
                Some((im, fmt)) => {
                    let crop = if view.processed { view.crop_of(idx, &im) } else { None };
                    if crop.is_none() && untouched(&im) {
                        outs.push((bytes, orig_ext.clone()));
                    } else {
                        let im = match crop {
                            Some(r) => im.crop_imm(r.x, r.y, r.w, r.h),
                            None => im,
                        };
                        let of = match ex.format {
                            Format::Jpg => OutFmt::Jpeg,
                            Format::Png => OutFmt::Png,
                            Format::Keep => {
                                if fmt == image::ImageFormat::Jpeg {
                                    OutFmt::Jpeg
                                } else {
                                    OutFmt::Png
                                }
                            }
                        };
                        let pieces = match &ex.slice {
                            Some(s) if im.height() > s.max_height => {
                                let g = im.to_luma8();
                                img::slice_points(&g, s.max_height, s.at_blank_rows, 10)
                                    .into_iter()
                                    .map(|(a, b)| im.crop_imm(0, a, im.width(), b - a))
                                    .collect()
                            }
                            _ => vec![im],
                        };
                        for p in pieces {
                            let p = img::fit_height(p, ex.max_height);
                            outs.push((img::encode(&p, of, ex.quality)?, of.ext().to_string()));
                        }
                    }
                }
            }
            for (data, ext) in outs {
                n += 1;
                let mut file = render_name(&ex.rename, n, &ext, &orig_stem);
                if !names.insert(file.clone()) {
                    file = format!("{n:05}_{file}");
                    names.insert(file.clone());
                }
                zw.start_file(file, stored)?;
                zw.write_all(&data)?;
            }
            on_page(idx + 1, total);
        }
        let manga = spec.export.manga.as_deref();
        let deflate = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zw.start_file("ComicInfo.xml", deflate)?;
        zw.write_all(comic_info(meta, manga, n).as_bytes())?;
        zw.finish()?;
        Ok(n)
    })();
    match r {
        Ok(n) => {
            std::fs::rename(&tmp, out).map_err(|e| anyhow!("保存失败：{e}"))?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
