//! 漫画页面的图片处理（纯函数）：读尺寸、拆半页、找白边、感知哈希、找空白行切长图、编码
use anyhow::{anyhow, Result};
use image::{DynamicImage, GrayImage, ImageFormat, ImageReader};
use serde::Serialize;
use std::io::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn full(w: u32, h: u32) -> Rect {
        Rect { x: 0, y: 0, w, h }
    }
}

/// 一页在虚拟页表里是整页、左半还是右半
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Half {
    Full,
    Left,
    Right,
}

impl Half {
    pub fn rect(self, w: u32, h: u32) -> Rect {
        let mid = w / 2;
        match self {
            Half::Full => Rect::full(w, h),
            Half::Left => Rect { x: 0, y: 0, w: mid.max(1), h },
            Half::Right => Rect {
                x: mid,
                y: 0,
                w: (w - mid).max(1),
                h,
            },
        }
    }
}

/// 只读文件头拿尺寸（不解码像素）
pub fn dims(bytes: &[u8]) -> Option<(u32, u32)> {
    ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// 解码；avif 等不支持的格式、损坏的图片返回 None（原样放行）
pub fn decode(bytes: &[u8]) -> Option<(DynamicImage, ImageFormat)> {
    let r = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let f = r.format()?;
    Some((r.decode().ok()?, f))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimColor {
    Auto,
    White,
    Black,
}

#[derive(Clone, Debug)]
pub struct TrimOpt {
    pub color: TrimColor,
    pub tolerance: u8,
    /// 每边最多裁掉的比例（0~0.45）
    pub max: f32,
    /// 上、下、左、右
    pub sides: [bool; 4],
}

/// 一行 / 一列里允许有多少“杂点”（扫描的灰尘）还算空白
fn noise_allowance(len: u32) -> u32 {
    len / 200
}

/// 在 area 里找要保留的区域：从四边往里数连续的“边框色”行 / 列，每边最多 max
pub fn trim_box(g: &GrayImage, area: Rect, o: &TrimOpt) -> Rect {
    if area.w < 8 || area.h < 8 {
        return area;
    }
    let px = |x: u32, y: u32| g.get_pixel(area.x + x, area.y + y).0[0];
    let reference: u8 = match o.color {
        TrimColor::White => 255,
        TrimColor::Black => 0,
        TrimColor::Auto => {
            // 四条边上像素的中位数
            let mut v: Vec<u8> = Vec::with_capacity(((area.w + area.h) * 2) as usize);
            for x in 0..area.w {
                v.push(px(x, 0));
                v.push(px(x, area.h - 1));
            }
            for y in 0..area.h {
                v.push(px(0, y));
                v.push(px(area.w - 1, y));
            }
            v.sort_unstable();
            v[v.len() / 2]
        }
    };
    let tol = o.tolerance as i32;
    let off = |p: u8| (p as i32 - reference as i32).abs() > tol;
    let row_blank = |y: u32, x0: u32, x1: u32| {
        let allow = noise_allowance(x1 - x0);
        let mut bad = 0;
        for x in x0..x1 {
            if off(px(x, y)) {
                bad += 1;
                if bad > allow {
                    return false;
                }
            }
        }
        true
    };
    let col_blank = |x: u32, y0: u32, y1: u32| {
        let allow = noise_allowance(y1 - y0);
        let mut bad = 0;
        for y in y0..y1 {
            if off(px(x, y)) {
                bad += 1;
                if bad > allow {
                    return false;
                }
            }
        }
        true
    };
    let max = o.max.clamp(0.0, 0.45);
    let cap_v = (area.h as f32 * max).floor() as u32;
    let cap_h = (area.w as f32 * max).floor() as u32;
    let (mut top, mut bottom, mut left, mut right) = (0u32, 0u32, 0u32, 0u32);
    if o.sides[0] {
        while top < cap_v && row_blank(top, 0, area.w) {
            top += 1;
        }
    }
    if o.sides[1] {
        while bottom < cap_v && row_blank(area.h - 1 - bottom, 0, area.w) {
            bottom += 1;
        }
    }
    // 左右只看上下裁剩下的部分
    let (y0, y1) = (top, area.h - bottom);
    if o.sides[2] {
        while left < cap_h && col_blank(left, y0, y1) {
            left += 1;
        }
    }
    if o.sides[3] {
        while right < cap_h && col_blank(area.w - 1 - right, y0, y1) {
            right += 1;
        }
    }
    Rect {
        x: area.x + left,
        y: area.y + top,
        w: area.w - left - right,
        h: area.h - top - bottom,
    }
}

/// 64 位差值哈希（dHash）：缩到 9×8 灰度，比较左右相邻像素
pub fn dhash(img: &DynamicImage) -> u64 {
    let g = img.thumbnail_exact(9, 8).to_luma8();
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h <<= 1;
            if g.get_pixel(x, y).0[0] > g.get_pixel(x + 1, y).0[0] {
                h |= 1;
            }
        }
    }
    h
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 整行是否空白（同一种颜色，允许少量杂点）
pub fn blank_row(g: &GrayImage, y: u32, tol: u8) -> bool {
    let w = g.width();
    if w == 0 {
        return true;
    }
    let row: Vec<u8> = (0..w).map(|x| g.get_pixel(x, y).0[0]).collect();
    let mut s = row.clone();
    s.sort_unstable();
    let med = s[s.len() / 2] as i32;
    let allow = noise_allowance(w) as usize;
    row.iter().filter(|&&p| (p as i32 - med).abs() > tol as i32).count() <= allow
}

/// 长图切成多段，每段不超过 max_h；at_blank 时只切在空白行（附近没有空白行就让这一段长一些，绝不切到画面）
pub fn slice_points(g: &GrayImage, max_h: u32, at_blank: bool, tol: u8) -> Vec<(u32, u32)> {
    let h = g.height();
    let max_h = max_h.max(1);
    let mut out = vec![];
    let mut y0 = 0u32;
    while h - y0 > max_h {
        let target = y0 + max_h;
        let cut = if !at_blank {
            target
        } else {
            let lo = y0 + (max_h / 3).max(1);
            (lo..=target)
                .rev()
                .find(|&y| blank_row(g, y, tol))
                .or_else(|| (target + 1..h).find(|&y| blank_row(g, y, tol)))
                .unwrap_or(h)
        };
        if cut >= h {
            break;
        }
        out.push((y0, cut));
        y0 = cut;
    }
    out.push((y0, h));
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutFmt {
    Jpeg,
    Png,
}

impl OutFmt {
    pub fn ext(self) -> &'static str {
        match self {
            OutFmt::Jpeg => "jpg",
            OutFmt::Png => "png",
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            OutFmt::Jpeg => "image/jpeg",
            OutFmt::Png => "image/png",
        }
    }
}

pub fn encode(img: &DynamicImage, fmt: OutFmt, quality: u8) -> Result<Vec<u8>> {
    let mut buf = vec![];
    match fmt {
        OutFmt::Jpeg => {
            let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
            rgb.write_with_encoder(enc).map_err(|e| anyhow!("JPEG 编码失败：{e}"))?;
        }
        OutFmt::Png => {
            img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
                .map_err(|e| anyhow!("PNG 编码失败：{e}"))?;
        }
    }
    Ok(buf)
}

/// 超过 max_h 就等比缩小
pub fn fit_height(img: DynamicImage, max_h: Option<u32>) -> DynamicImage {
    match max_h {
        Some(m) if img.height() > m && m > 0 => {
            let w = ((img.width() as f64) * m as f64 / img.height() as f64).round().max(1.0) as u32;
            img.resize_exact(w, m, image::imageops::FilterType::CatmullRom)
        }
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn opt(color: TrimColor, max: f32) -> TrimOpt {
        TrimOpt {
            color,
            tolerance: 12,
            max,
            sides: [true; 4],
        }
    }

    #[test]
    fn trims_white_border() {
        // 100×100 白底，中间 60×60 黑块
        let mut g = GrayImage::from_pixel(100, 100, Luma([255]));
        for y in 20..80 {
            for x in 20..80 {
                g.put_pixel(x, y, Luma([0]));
            }
        }
        let r = trim_box(&g, Rect::full(100, 100), &opt(TrimColor::Auto, 0.3));
        assert_eq!(r, Rect { x: 20, y: 20, w: 60, h: 60 });
        // 每边最多 8%
        let r = trim_box(&g, Rect::full(100, 100), &opt(TrimColor::White, 0.08));
        assert_eq!(r, Rect { x: 8, y: 8, w: 84, h: 84 });
    }

    #[test]
    fn dark_art_touching_edge_is_not_cropped() {
        // 白底，左边一条黑色画面贴边：左边不能裁
        let mut g = GrayImage::from_pixel(100, 100, Luma([255]));
        for y in 0..100 {
            for x in 0..30 {
                g.put_pixel(x, y, Luma([10]));
            }
        }
        let r = trim_box(&g, Rect::full(100, 100), &opt(TrimColor::Auto, 0.08));
        assert_eq!(r.x, 0, "贴边的画面不裁");
        assert_eq!(r.w, 100 - 8, "右边白边最多裁 8%");
        // 整页都是深色、按黑边裁：也只能裁到上限
        let g = GrayImage::from_pixel(100, 100, Luma([0]));
        let r = trim_box(&g, Rect::full(100, 100), &opt(TrimColor::Black, 0.08));
        assert_eq!(r, Rect { x: 8, y: 8, w: 84, h: 84 });
    }

    #[test]
    fn halves() {
        assert_eq!(Half::Left.rect(201, 100), Rect { x: 0, y: 0, w: 100, h: 100 });
        assert_eq!(Half::Right.rect(201, 100), Rect { x: 100, y: 0, w: 101, h: 100 });
    }

    #[test]
    fn slices_only_at_blank_rows() {
        // 宽 50，高 1000：画面块 [0,300) [320,700) [720,1000)，中间是白色空行
        let mut g = GrayImage::from_pixel(50, 1000, Luma([255]));
        let art = [(0u32, 300u32), (320, 700), (720, 1000)];
        for &(a, b) in &art {
            for y in a..b {
                for x in 0..50 {
                    g.put_pixel(x, y, Luma([((x * 7 + y * 3) % 200) as u8]));
                }
            }
        }
        let parts = slice_points(&g, 400, true, 8);
        assert_eq!(parts.first().unwrap().0, 0);
        assert_eq!(parts.last().unwrap().1, 1000);
        for w in parts.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        for &(_, end) in &parts[..parts.len() - 1] {
            assert!(art.iter().all(|&(a, b)| !(a < end && end < b)), "切在了画面中间：{end}");
            assert!(blank_row(&g, end, 8));
        }
        // 300 处没法切（第一段要不超过 400，空行在 300~320）：第一刀切在 300~320 之间
        assert!((300..320).contains(&parts[0].1));
        // 不要求空白行时按固定高度切
        let hard = slice_points(&g, 400, false, 8);
        assert_eq!(hard, vec![(0, 400), (400, 800), (800, 1000)]);
    }

    #[test]
    fn slice_without_blank_rows_never_cuts_art() {
        let mut g = GrayImage::new(20, 900);
        for y in 0..900 {
            for x in 0..20 {
                g.put_pixel(x, y, Luma([((x * 13 + y) % 250) as u8]));
            }
        }
        assert_eq!(slice_points(&g, 300, true, 8), vec![(0, 900)]);
    }

    #[test]
    fn dhash_similar() {
        let mut a = image::RgbImage::new(64, 64);
        for (x, y, p) in a.enumerate_pixels_mut() {
            *p = image::Rgb([(x * 4) as u8, (y * 4) as u8, 128]);
        }
        let b = image::imageops::resize(&a, 100, 100, image::imageops::FilterType::Triangle);
        let ha = dhash(&DynamicImage::ImageRgb8(a.clone()));
        let hb = dhash(&DynamicImage::ImageRgb8(b));
        assert!(hamming(ha, hb) <= 6);
        let mut c = a;
        image::imageops::flip_horizontal_in_place(&mut c);
        assert!(hamming(ha, dhash(&DynamicImage::ImageRgb8(c))) > 20);
    }
}
