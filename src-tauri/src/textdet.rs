//! 本地文字检测：找出漫画页里每一块文字（气泡、旁白）的位置，交给本地 OCR 逐块识别。
//!
//! 模型：PaddleOCR 的 PP-OCRv4 检测模型（DBNet，约 4.7 MB，Apache-2.0），随安装包放在 `bin\ocr\`。
//! 用纯 Rust 的 tract 运行，不需要额外的运行库。横排、竖排都能检出“行 / 列”，
//! 再把挨得近、字号相近的行 / 列合并成一个文字块。
use anyhow::{anyhow, Context, Result};
use image::{imageops::FilterType, RgbImage};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tract_onnx::prelude::*;

pub const MODEL_FILE: &str = "ch_PP-OCRv4_det_infer.onnx";
/// 检测时把页面缩放到长边约这么多像素（太小会漏掉竖排小字）
const DET_SIDE: f32 = 1600.0;
/// 小图最多放大这么多倍
const MAX_UPSCALE: f32 = 2.0;
/// 像素是文字的概率阈值
const THRESH: f32 = 0.3;
/// 一个连通区域的平均概率低于这个值就丢掉
const BOX_THRESH: f32 = 0.5;
/// 往外扩的比例（DBNet 预测的是缩进去的文字核心）
const UNCLIP: f32 = 1.6;

type Plan = Arc<tract_onnx::tract_core::plan::SimplePlan<TypedFact, Box<dyn TypedOp>>>;

/// 按输入尺寸缓存编译好的模型（同一本漫画的页面一般一样大）
static PLANS: Lazy<Mutex<HashMap<(usize, usize), Plan>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// 检测模型的位置：环境变量 MP_TEXTDET_MODEL、程序目录 bin\ocr\、数据目录 bin\ocr\
pub fn model_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MP_TEXTDET_MODEL").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    let mut cands = vec![];
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        cands.push(dir.join("bin").join("ocr").join(MODEL_FILE));
    }
    if let Some(d) = crate::tools::data_bin() {
        cands.push(d.join("ocr").join(MODEL_FILE));
    }
    cands.into_iter().find(|p| p.exists())
}

fn plan(h: usize, w: usize) -> Result<Plan> {
    if let Some(p) = PLANS.lock().get(&(h, w)) {
        return Ok(p.clone());
    }
    let path = model_path().ok_or_else(|| anyhow!("没有找到文字检测模型（{MODEL_FILE}，安装包自带），请重新安装播放器"))?;
    let p = tract_onnx::onnx()
        .model_for_path(&path)
        .with_context(|| format!("读取检测模型失败：{}", path.display()))?
        .with_input_fact(0, f32::fact([1, 3, h, w]).into())?
        .into_optimized()?
        .into_runnable()?;
    let mut g = PLANS.lock();
    if g.len() >= 4 {
        g.clear();
    }
    g.insert((h, w), p.clone());
    Ok(p)
}

/// 检测输入尺寸：长边缩放到 DET_SIDE，边长取 32 的倍数
pub fn det_size(w: u32, h: u32) -> (u32, u32) {
    let s = (DET_SIDE / w.max(h).max(1) as f32).min(MAX_UPSCALE);
    let r = |v: u32| (((v as f32 * s) / 32.0).round() as u32).max(1) * 32;
    (r(w), r(h))
}

/// 一行 / 一列文字，或合并后的文字块。坐标是像素。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    /// 字号的估计：一行 / 一列的粗细
    pub fn thickness(&self) -> f32 {
        self.w().min(self.h())
    }
    pub fn vertical(&self) -> bool {
        self.h() > self.w() * 1.3
    }
    fn union(&self, o: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    fn gap(&self, o: &Rect) -> (f32, f32) {
        let gx = (o.x0 - self.x1).max(self.x0 - o.x1).max(0.0);
        let gy = (o.y0 - self.y1).max(self.y0 - o.y1).max(0.0);
        (gx, gy)
    }
}

/// 概率图 → 文字行 / 列的外框（概率图坐标）。prob 按行存放，大小 w×h。
pub fn boxes_from_prob(prob: &[f32], w: usize, h: usize) -> Vec<Rect> {
    let mut seen = vec![false; w * h];
    let mut out = vec![];
    let mut stack = vec![];
    for start in 0..w * h {
        if seen[start] || prob[start] <= THRESH {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        let (mut n, mut sum) = (0usize, 0f32);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            n += 1;
            sum += prob[i];
            let mut push = |j: usize| {
                if !seen[j] && prob[j] > THRESH {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        if bw.min(bh) < 3.0 || sum / (n as f32) < BOX_THRESH {
            continue;
        }
        // 按 DBNet 的做法往外扩：距离 = 面积 × 比例 / 周长
        let d = bw * bh * UNCLIP / (2.0 * (bw + bh));
        out.push(Rect {
            x0: (x0 as f32 - d).max(0.0),
            y0: (y0 as f32 - d).max(0.0),
            x1: (x1 as f32 + 1.0 + d).min(w as f32),
            y1: (y1 as f32 + 1.0 + d).min(h as f32),
        });
    }
    out
}

/// 把挨得近、字号相近的行 / 列合并成文字块（一个气泡通常是几列竖排字）
pub fn group(lines: &[Rect]) -> Vec<Rect> {
    let n = lines.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut i = i;
        while p[i] != r {
            let next = p[i];
            p[i] = r;
            i = next;
        }
        r
    }
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&lines[i], &lines[j]);
            let (ta, tb) = (a.thickness(), b.thickness());
            let t = ta.max(tb);
            // 注音假名比正文小一半左右，也算同一块
            if ta.min(tb) * 3.0 < t {
                continue;
            }
            let (gx, gy) = a.gap(b);
            let close = if a.vertical() || b.vertical() {
                // 竖排：左右相邻的列，上下要有重叠
                gx <= t * 0.9 && gy <= t * 0.5
            } else {
                gy <= t * 0.9 && gx <= t * 0.5
            };
            if close {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut groups: HashMap<usize, Rect> = HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).and_modify(|g| *g = g.union(&lines[i])).or_insert(lines[i]);
    }
    let mut v: Vec<Rect> = groups.into_values().collect();
    v.sort_by(|a, b| a.y0.total_cmp(&b.y0).then(b.x0.total_cmp(&a.x0)));
    v
}

/// 文字块里的行 / 列是否以竖排为主
pub fn block_vertical(block: &Rect, lines: &[Rect]) -> bool {
    let (mut v, mut h) = (0f32, 0f32);
    for l in lines
        .iter()
        .filter(|l| l.x0 >= block.x0 - 0.5 && l.x1 <= block.x1 + 0.5 && l.y0 >= block.y0 - 0.5 && l.y1 <= block.y1 + 0.5)
    {
        let area = l.w() * l.h();
        if l.vertical() {
            v += area;
        } else if l.w() > l.h() * 1.3 {
            h += area;
        }
    }
    if v == 0.0 && h == 0.0 {
        return block.vertical();
    }
    v > h
}

/// 检测结果：x、y、w、h 都是相对整页的 0–1
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    pub bbox: [f32; 4],
    pub vertical: bool,
}

/// 检测一页。img 是原图（任意大小）。
pub fn detect(img: &RgbImage) -> Result<Vec<TextBlock>> {
    let (w, h) = img.dimensions();
    if w < 16 || h < 16 {
        return Ok(vec![]);
    }
    let (nw, nh) = det_size(w, h);
    let r = image::imageops::resize(img, nw, nh, FilterType::Triangle);
    let (nw, nh) = (nw as usize, nh as usize);
    let input = tract_ndarray::Array4::from_shape_fn((1, 3, nh, nw), |(_, c, y, x)| (r.get_pixel(x as u32, y as u32)[c] as f32 / 255.0 - 0.5) / 0.5);
    let out = plan(nh, nw)?.run(tvec!(Tensor::from(input).into()))?;
    let o = out[0].to_plain_array_view::<f32>()?;
    let shape = o.shape();
    if shape.len() != 4 {
        return Err(anyhow!("检测模型输出格式不对：{shape:?}"));
    }
    let (ph, pw) = (shape[2], shape[3]);
    let prob: Vec<f32> = o.iter().copied().take(ph * pw).collect();
    let lines = boxes_from_prob(&prob, pw, ph);
    let blocks = group(&lines);
    let (sx, sy) = (1.0 / pw as f32, 1.0 / ph as f32);
    Ok(blocks
        .iter()
        .filter(|b| b.w() * b.h() * sx * sy > 0.0004)
        .map(|b| TextBlock {
            bbox: [b.x0 * sx, b.y0 * sy, b.w() * sx, b.h() * sy],
            vertical: block_vertical(b, &lines),
        })
        .collect())
}

/// 从原图裁出一块（四周留一点白边），太小的放大一点，编码成 PNG 给 OCR
pub fn crop_png(img: &RgbImage, bbox: [f32; 4]) -> Result<Vec<u8>> {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let pad = 0.15 * (bbox[2] * w).min(bbox[3] * h);
    let x0 = (bbox[0] * w - pad).max(0.0).floor() as u32;
    let y0 = (bbox[1] * h - pad).max(0.0).floor() as u32;
    let x1 = ((bbox[0] + bbox[2]) * w + pad).min(w).ceil() as u32;
    let y1 = ((bbox[1] + bbox[3]) * h + pad).min(h).ceil() as u32;
    let (cw, ch) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let mut c = image::imageops::crop_imm(img, x0, y0, cw, ch).to_image();
    // 视觉模型对太小的字认不准（实测 199×131 的五列小字会漏列），短边放大到约 320 像素，最多放大 4 倍，长边不超过 1600
    let short = cw.min(ch) as f32;
    let long = cw.max(ch) as f32;
    let s = (320.0 / short).min(4.0).min(1600.0 / long);
    if s > 1.05 {
        c = image::imageops::resize(&c, (cw as f32 * s).round() as u32, (ch as f32 * s).round() as u32, FilterType::Lanczos3);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    c.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn size_is_multiple_of_32() {
        let (w, h) = det_size(1100, 1600);
        assert_eq!((w % 32, h % 32), (0, 0));
        assert_eq!(h, 1600);
        // 小图最多放大两倍
        assert_eq!(det_size(100, 200), (192, 416));
    }

    #[test]
    fn components_found_and_unclipped() {
        let (w, h) = (40, 30);
        let mut p = vec![0f32; w * h];
        for y in 5..25 {
            for x in 10..14 {
                p[y * w + x] = 0.9;
            }
        }
        // 噪点：一个像素，太小
        p[2 * w + 35] = 0.9;
        let b = boxes_from_prob(&p, w, h);
        assert_eq!(b.len(), 1);
        let r = b[0];
        assert!(r.x0 < 10.0 && r.x1 > 14.0 && r.y0 < 5.0 && r.y1 > 25.0);
        assert!(r.vertical());
    }

    #[test]
    fn low_score_dropped() {
        let (w, h) = (20, 20);
        let mut p = vec![0f32; w * h];
        for y in 2..12 {
            for x in 2..12 {
                p[y * w + x] = 0.35;
            }
        }
        assert!(boxes_from_prob(&p, w, h).is_empty());
    }

    #[test]
    fn vertical_columns_merge_into_one_bubble() {
        // 三列竖排字，列宽 20，间隔 8；另一个气泡离得很远
        let cols = [
            rect(100.0, 50.0, 120.0, 250.0),
            rect(128.0, 50.0, 148.0, 200.0),
            rect(156.0, 60.0, 176.0, 240.0),
            rect(600.0, 50.0, 620.0, 200.0),
        ];
        let g = group(&cols);
        assert_eq!(g.len(), 2);
        let big = g.iter().find(|r| r.x0 < 200.0).unwrap();
        assert_eq!(*big, rect(100.0, 50.0, 176.0, 250.0));
        assert!(block_vertical(big, &cols));
    }

    #[test]
    fn furigana_joins_but_huge_title_does_not() {
        let lines = [
            rect(100.0, 50.0, 124.0, 250.0),
            // 注音：细一半，紧挨着
            rect(125.0, 60.0, 135.0, 100.0),
            // 大字标题：粗 4 倍以上，即使挨着也不合并
            rect(140.0, 0.0, 240.0, 400.0),
        ];
        assert_eq!(group(&lines).len(), 2);
    }

    #[test]
    fn horizontal_lines_merge() {
        let lines = [rect(10.0, 10.0, 300.0, 30.0), rect(20.0, 36.0, 280.0, 56.0)];
        let g = group(&lines);
        assert_eq!(g.len(), 1);
        assert!(!block_vertical(&g[0], &lines));
    }

    #[test]
    fn crop_upscales_small_text() {
        let img = RgbImage::from_pixel(400, 400, image::Rgb([255, 255, 255]));
        let png = crop_png(&img, [0.1, 0.1, 0.05, 0.2]).unwrap();
        let c = image::load_from_memory(&png).unwrap();
        // 原来约 26×86，放大 4 倍封顶
        assert!(c.width() >= 100 && c.width() <= 4 * 30);
        // 大块不放大
        let png = crop_png(&img, [0.0, 0.0, 1.0, 1.0]).unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 400);
    }

    /// 有模型文件时（MP_TEXTDET_MODEL）跑一次真实检测：白底上画三列黑块，应合并成一个竖排块
    #[test]
    fn real_model_if_available() {
        if model_path().is_none() {
            // 没有模型时说明测试被跳过
            eprintln!("没有检测模型，跳过");
            return;
        }
        let img = RgbImage::from_pixel(600, 800, image::Rgb([255, 255, 255]));
        let r = detect(&img).unwrap();
        assert!(r.is_empty(), "白纸上不应检出文字：{r:?}");
        // 本地调试：MP_TEXTDET_PAGE 指向一张漫画页时打印检测结果
        if let Some(p) = std::env::var_os("MP_TEXTDET_PAGE") {
            let img = image::open(p).unwrap().to_rgb8();
            let t = std::time::Instant::now();
            let r = detect(&img).unwrap();
            println!("{} 块，用时 {:?}", r.len(), t.elapsed());
            for b in &r {
                let px = [
                    b.bbox[0] * img.width() as f32,
                    b.bbox[1] * img.height() as f32,
                    b.bbox[2] * img.width() as f32,
                    b.bbox[3] * img.height() as f32,
                ];
                println!("{:?} vertical={}", px.map(|v| v.round()), b.vertical);
            }
        }
    }
}
