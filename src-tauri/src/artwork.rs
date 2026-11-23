//! 番剧 / 剧集 / 电影的封面：本地海报查找、截帧挑选、竖版裁切。
//! 纯图像计算都在这里（有单元测试），调用 ffmpeg 的部分在 media.rs。
use image::{imageops, DynamicImage, GenericImageView, RgbImage};
use std::path::{Path, PathBuf};

/// 本地海报的文件名（不含扩展名，比较时忽略大小写）
const POSTER_NAMES: &[&str] = &["poster", "folder", "cover", "show", "keyart"];
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];
/// 往上找海报时跳过的目录：这些目录里的图（CD 封面、BD 扫图）不是作品海报
const SKIP_DIRS: &[&str] = &["cds", "cd", "scans", "scan", "fonts", "sps", "extras", "bonus", "menu", "特典"];

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// 一个目录里的海报：poster / folder / cover / show / keyart，或者和目录同名的图片
fn poster_in(dir: &Path) -> Option<PathBuf> {
    let dir_name = dir.file_name().and_then(|n| n.to_str()).map(|s| s.to_lowercase());
    let mut found: Vec<(usize, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_lowercase();
            let rank = POSTER_NAMES
                .iter()
                .position(|n| *n == stem)
                .or_else(|| (dir_name.as_deref() == Some(stem.as_str())).then_some(POSTER_NAMES.len()))?;
            Some((rank, p))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, p)| p)
}

/// 视频对应的本地海报：先看视频所在目录，再往上最多找 3 层（季目录、清晰度目录上面的作品目录）。
/// 不进入媒体库根目录（根目录里的图属于所有作品），也不在 CDs / Scans 这类目录里找。
pub fn local_poster(video: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let mut dir = video.parent()?;
    for _ in 0..4 {
        if roots.iter().any(|r| r.as_path() == dir) || dir.parent().is_none() {
            return None;
        }
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
        if !SKIP_DIRS.contains(&name.as_str()) {
            if let Some(p) = poster_in(dir) {
                return Some(p);
            }
        }
        // 不知道媒体库根目录时只看视频所在目录和上一层
        if roots.is_empty() && video.parent() != Some(dir) {
            return None;
        }
        dir = dir.parent()?;
    }
    None
}

/// 和视频同名的截图（`01.jpg`、`01-thumb.jpg`），一些下载工具和媒体服务器会生成
pub fn sidecar_thumb(video: &Path) -> Option<PathBuf> {
    let dir = video.parent()?;
    let stem = video.file_stem()?.to_str()?;
    for suffix in ["", "-thumb", ".thumb"] {
        for e in IMAGE_EXTS {
            let p = dir.join(format!("{stem}{suffix}.{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

// ---------- 截帧打分 ----------

fn luma(p: &image::Rgb<u8>) -> f64 {
    (0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64) / 255.0
}

fn saturation(p: &image::Rgb<u8>) -> f64 {
    let mx = p.0.iter().copied().max().unwrap_or(0) as f64;
    let mn = p.0.iter().copied().min().unwrap_or(0) as f64;
    if mx <= 0.0 {
        0.0
    } else {
        (mx - mn) / mx
    }
}

/// 一帧画面适不适合当封面，分数越高越好；纯黑、纯白、大片单色（片头黑屏、字幕卡、白底菜单）为 0
pub fn frame_score(img: &DynamicImage) -> f64 {
    let small = img.thumbnail(160, 160).to_rgb8();
    let (w, h) = small.dimensions();
    if w < 4 || h < 4 {
        return 0.0;
    }
    let n = (w * h) as f64;
    let mut sum = 0.0;
    let mut sum2 = 0.0;
    let mut sat = 0.0;
    let mut dark = 0.0;
    let mut bright = 0.0;
    let mut edge = 0.0;
    for y in 0..h {
        for x in 0..w {
            let p = small.get_pixel(x, y);
            let l = luma(p);
            sum += l;
            sum2 += l * l;
            sat += saturation(p);
            if l < 0.07 {
                dark += 1.0;
            }
            if l > 0.93 {
                bright += 1.0;
            }
            if x + 1 < w {
                edge += (l - luma(small.get_pixel(x + 1, y))).abs();
            }
            if y + 1 < h {
                edge += (l - luma(small.get_pixel(x, y + 1))).abs();
            }
        }
    }
    let mean = sum / n;
    let std = (sum2 / n - mean * mean).max(0.0).sqrt();
    let (sat, edge, dark, bright) = (sat / n, edge / n, dark / n, bright / n);
    if !(0.08..=0.9).contains(&mean) || std < 0.05 || dark > 0.55 || bright > 0.45 {
        return 0.0;
    }
    // 细节多、颜色鲜艳、亮度适中的画面优先
    edge.min(0.4) * (0.4 + sat) * (1.0 - (mean - 0.45).abs()) * (1.0 - dark - bright).max(0.1)
}

/// 去掉上下左右的黑边（遮幅），返回裁切后的画面；每边最多裁 25%
pub fn crop_bars(img: &DynamicImage) -> DynamicImage {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w < 16 || h < 16 {
        return img.clone();
    }
    let row_dark = |y: u32| (0..w).step_by(4).all(|x| luma(rgb.get_pixel(x, y)) < 0.07);
    let col_dark = |x: u32| (0..h).step_by(4).all(|y| luma(rgb.get_pixel(x, y)) < 0.07);
    let mut top = 0;
    while top < h / 4 && row_dark(top) {
        top += 1;
    }
    let mut bottom = 0;
    while bottom < h / 4 && row_dark(h - 1 - bottom) {
        bottom += 1;
    }
    let mut left = 0;
    while left < w / 4 && col_dark(left) {
        left += 1;
    }
    let mut right = 0;
    while right < w / 4 && col_dark(w - 1 - right) {
        right += 1;
    }
    if top + bottom + left + right == 0 {
        return img.clone();
    }
    img.crop_imm(left, top, w - left - right, h - top - bottom)
}

/// 每一列的“内容量”（亮度变化 + 饱和度），用来找人物所在的位置
fn column_weights(img: &RgbImage) -> Vec<f64> {
    let (w, h) = img.dimensions();
    (0..w)
        .map(|x| {
            let mut s = 0.0;
            for y in (0..h).step_by(2) {
                let p = img.get_pixel(x, y);
                let l = luma(p);
                if x + 1 < w {
                    s += (l - luma(img.get_pixel(x + 1, y))).abs();
                }
                if y + 2 < h {
                    s += (l - luma(img.get_pixel(x, y + 2))).abs();
                }
                s += saturation(p) * 0.05;
            }
            s
        })
        .collect()
}

/// 在宽 w 的画面里放一个宽 win 的窗口，内容量最大的位置（略偏向中间）
pub fn best_window(weights: &[f64], win: usize) -> usize {
    let w = weights.len();
    if win >= w {
        return 0;
    }
    let mut prefix = vec![0.0; w + 1];
    for (i, v) in weights.iter().enumerate() {
        prefix[i + 1] = prefix[i] + v;
    }
    let total = prefix[w].max(1e-9);
    let span = (w - win) as f64;
    (0..=w - win)
        .max_by(|&a, &b| {
            let score = |x: usize| {
                let off = (x as f64 / span - 0.5).abs();
                (prefix[x + win] - prefix[x]) / total * (1.0 - 0.3 * off)
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap_or(0)
}

/// 横版画面裁成 2:3 竖版：先去黑边，再把窗口放在内容最多的地方
pub fn portrait_crop(img: &DynamicImage) -> DynamicImage {
    let img = crop_bars(img);
    let (w, h) = img.dimensions();
    let win = (h as f64 * 2.0 / 3.0).round() as u32;
    if win >= w {
        // 本来就比 2:3 还窄：上下裁
        let hh = ((w as f64) * 1.5).round() as u32;
        if hh >= h {
            return img;
        }
        return img.crop_imm(0, (h - hh) / 2, w, hh);
    }
    // 在缩小图上算位置再按比例放大，快很多
    let small = img.thumbnail(320, 320).to_rgb8();
    let scale = w as f64 / small.width() as f64;
    let swin = ((win as f64 / scale).round() as usize).max(1);
    let x = (best_window(&column_weights(&small), swin) as f64 * scale).round() as u32;
    img.crop_imm(x.min(w - win), 0, win, h)
}

/// 截帧的时间点：避开片头片尾，在 20%～70% 之间均匀取 n 个
pub fn sample_times(duration: f64, n: usize) -> Vec<f64> {
    if duration <= 0.0 {
        return vec![10.0, 30.0, 60.0];
    }
    let mut start = duration * 0.2;
    let mut end = duration * 0.7;
    // 正常长度的一集（>10 分钟）：至少跳过 OP（约 3 分钟）和 ED
    if duration > 600.0 {
        start = start.max(180.0);
        end = end.min(duration - 240.0);
    }
    if end <= start || n < 2 {
        return vec![duration * 0.3];
    }
    (0..n).map(|i| start + (end - start) * i as f64 / (n - 1) as f64).collect()
}

/// 从一组截帧里挑最好的那张；全都不合格时退回第一张
pub fn pick_best(frames: Vec<DynamicImage>) -> Option<DynamicImage> {
    let mut best: Option<(f64, DynamicImage)> = None;
    for f in frames {
        let s = frame_score(&f);
        if best.as_ref().map_or(true, |b| s > b.0) {
            best = Some((s, f));
        }
    }
    best.map(|(_, f)| f)
}

/// 存成 jpg（缩到最长边 max 像素以内）
pub fn save_jpg(img: &DynamicImage, max: u32, out: &Path) -> anyhow::Result<()> {
    let img = if img.width() > max || img.height() > max {
        img.resize(max, max, imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let tmp = out.with_extension("tmp.jpg");
    img.to_rgb8().save_with_format(&tmp, image::ImageFormat::Jpeg)?;
    std::fs::rename(&tmp, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn solid(w: u32, h: u32, c: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb(c)))
    }

    /// 彩色方格：细节多、颜色鲜艳
    fn busy(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                Rgb([230, 120, 160])
            } else {
                Rgb([40, 90, 200])
            }
        }))
    }

    /// 手动验证真实截帧：ART_DIR 下每个子目录是一部作品的若干帧，输出挑中的帧和竖版裁切
    #[test]
    #[ignore]
    fn real_frames() {
        let Ok(dir) = std::env::var("ART_DIR") else { return };
        for d in std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().is_dir()) {
            let mut files: Vec<_> = std::fs::read_dir(d.path()).unwrap().flatten().map(|e| e.path()).collect();
            files.sort();
            let frames: Vec<_> = files.iter().filter_map(|p| image::open(p).ok()).collect();
            for (p, f) in files.iter().zip(&frames) {
                println!("{} {:.4}", p.display(), frame_score(f));
            }
            let best = crop_bars(&pick_best(frames).unwrap());
            let out = Path::new(&dir).join(format!("{}-portrait.jpg", d.file_name().to_string_lossy()));
            save_jpg(&portrait_crop(&best), 720, &out).unwrap();
        }
    }

    #[test]
    fn plain_frames_score_zero() {
        assert_eq!(frame_score(&solid(320, 180, [0, 0, 0])), 0.0);
        assert_eq!(frame_score(&solid(320, 180, [255, 255, 255])), 0.0);
        assert_eq!(frame_score(&solid(320, 180, [120, 120, 120])), 0.0);
        assert!(frame_score(&busy(320, 180)) > 0.0);
    }

    #[test]
    fn white_menu_with_small_picture_loses() {
        // 白底中间嵌一张小图（光盘菜单那种）
        let mut menu = RgbImage::from_pixel(320, 180, Rgb([250, 250, 252]));
        let pic = busy(80, 60).to_rgb8();
        imageops::replace(&mut menu, &pic, 120, 60);
        let menu = DynamicImage::ImageRgb8(menu);
        assert_eq!(frame_score(&menu), 0.0);
        let best = pick_best(vec![menu, busy(320, 180)]).unwrap();
        assert!(frame_score(&best) > 0.0);
    }

    #[test]
    fn crops_letterbox() {
        let mut img = RgbImage::from_pixel(320, 240, Rgb([0, 0, 0]));
        imageops::replace(&mut img, &busy(320, 180).to_rgb8(), 0, 30);
        let c = crop_bars(&DynamicImage::ImageRgb8(img));
        assert_eq!((c.width(), c.height()), (320, 180));
    }

    #[test]
    fn portrait_follows_content() {
        // 左边纯色，右边有内容：竖版窗口应该落在右边
        let mut img = RgbImage::from_pixel(480, 270, Rgb([90, 140, 90]));
        imageops::replace(&mut img, &busy(160, 270).to_rgb8(), 300, 0);
        let c = portrait_crop(&DynamicImage::ImageRgb8(img.clone()));
        assert_eq!((c.width(), c.height()), (180, 270));
        let w = column_weights(&img);
        let x = best_window(&w, 180);
        assert!(x >= 250, "窗口在 {x}");
    }

    #[test]
    fn portrait_of_tall_image_trims_height() {
        let c = portrait_crop(&busy(200, 400));
        assert_eq!((c.width(), c.height()), (200, 300));
    }

    #[test]
    fn sample_times_skip_op_ed() {
        let t = sample_times(24.0 * 60.0, 6);
        assert_eq!(t.len(), 6);
        assert!(t[0] >= 180.0 && *t.last().unwrap() <= 24.0 * 60.0 - 240.0);
        assert_eq!(sample_times(0.0, 6), vec![10.0, 30.0, 60.0]);
        assert_eq!(sample_times(100.0, 6).len(), 6);
    }

    #[test]
    fn finds_posters_in_series_folder() {
        let base = std::env::temp_dir().join(format!("mp_art_{}", std::process::id()));
        let root = base.join("番");
        let series = root.join("MyGO");
        let season = series.join("Season 1");
        let cds = series.join("CDs").join("01");
        std::fs::create_dir_all(&season).unwrap();
        std::fs::create_dir_all(&cds).unwrap();
        std::fs::write(cds.join("Cover.jpg"), b"x").unwrap();
        std::fs::write(root.join("folder.jpg"), b"x").unwrap();
        let ep = season.join("01.mkv");
        let roots = vec![root.clone()];
        // 根目录里的图不算，CD 封面也不算
        assert_eq!(local_poster(&ep, &roots), None);
        std::fs::write(series.join("Poster.PNG"), b"x").unwrap();
        assert_eq!(local_poster(&ep, &roots), Some(series.join("Poster.PNG")));
        // 季目录里的优先
        std::fs::write(season.join("folder.jpg"), b"x").unwrap();
        assert_eq!(local_poster(&ep, &roots), Some(season.join("folder.jpg")));
        std::fs::write(season.join("01-thumb.jpg"), b"x").unwrap();
        assert_eq!(sidecar_thumb(&ep), Some(season.join("01-thumb.jpg")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
