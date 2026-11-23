//! 漫画 AI 翻译嵌字：识别气泡（云端图像识别模型一次完成检测 + OCR，或本地检测 + 本地 Xiaomi-OCR-0 逐块识别）、
//! 结合上下文翻译、导出 CBZ
//!
//! 数据：应用数据目录 `mangatr/<comic_id>.json`，结构 `{pages:{"<页号>":{blocks:[…]}}, glossary:[{src,tr}]}`。
//! 页号从 0 开始，与前端 `comicPage(id, page)` 一致。
//! 只保存在本机，只把页面图片和文字发给用户配置的 AI 服务。
use crate::{ai, comic, db, library, localocr, parse, textdet, AppState};
use anyhow::{anyhow, Result};
use base64::Engine as _;
use image::{imageops::FilterType, ImageEncoder, RgbImage};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

/// 识别时送给模型的图片长边上限
pub const MAX_SIDE: u32 = 1600;
const CANCEL_MSG: &str = "已取消";

const VISION_SYSTEM: &str = "你是漫画文字检测与识别助手。只输出 JSON，不输出其他文字。";
const VISION_PROMPT: &str = "找出这一页中所有对话气泡与旁白框里的文字块，并识别原文。\
只返回一个 JSON 对象：{\"blocks\":[{\"bbox\":[x,y,w,h],\"vertical\":true或false,\"order\":1,\"text\":\"原文\",\"kind\":\"bubble或caption或sfx或art\"}]}。\
bbox 为文字块相对整张图片的位置，x、y 为左上角，四个值都在 0 到 1 之间（相对宽高）。\
vertical 表示是否竖排。order 为阅读顺序（日漫从右到左、从上到下），从 1 开始。\
kind：bubble=对话气泡，caption=旁白框，sfx=拟声词，art=画面里的招牌或其他艺术字。\
text 保留原文，不要翻译。没有文字时返回 {\"blocks\":[]}。";

const TRANSLATE_SYSTEM: &str = "你是专业的漫画翻译，把日文对白译成简体中文。\
要求：1. 保持人称、称呼和语气前后一致，同一个角色的说话风格不变；\
2. 补出日语里省略的主语、宾语，让句子在中文里读得通顺；\
3. 严格使用术语表里的译法；\
4. 拿不准的地方，在 alt 里给一到两个备选译法，没有备选就给空数组；\
5. 按顺序逐句翻译，不合并、不拆分、不跳过句子；\
6. 只输出一个 JSON 数组，每个元素形如 {\"tr\":\"译文\",\"alt\":[\"备选\"]}，数组长度与待翻译句数相同，不输出任何其他文字。";

// ---------- 数据结构 ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Term {
    pub src: String,
    pub tr: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Block {
    pub id: String,
    /// [x, y, w, h]，0–1 之间，相对整页
    pub bbox: [f32; 4],
    pub vertical: bool,
    /// 阅读顺序，从 1 开始
    pub order: u32,
    pub src: String,
    #[serde(default)]
    pub tr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<Vec<String>>,
    /// bubble（对话）/ caption（旁白）/ sfx（拟声词）/ art（画面文字）
    pub kind: String,
    /// 拟声词、画面文字第一版不翻译
    pub skipped: bool,
    /// 取样得到的底色 #rrggbb，前端用它盖住原文
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct PageTr {
    pub blocks: Vec<Block>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct BookTr {
    pub pages: BTreeMap<u32, PageTr>,
    pub glossary: Vec<Term>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub comic_id: i64,
    pub page: u32,
    pub from: u32,
    pub to: u32,
    pub done: u32,
    pub total: u32,
    pub stage: String,
    pub message: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Estimate {
    pub pages: u32,
    pub per_page: u64,
    pub total: u64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrOut {
    pub tr: String,
    pub alt: Vec<String>,
}

/// 后台翻译任务的取消开关，按 comic_id 登记
static CANCEL: Lazy<Mutex<HashMap<i64, Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// 读写翻译数据文件时的串行锁（界面编辑与后台翻译可能同时写）
static IO: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// 启动时建表（CREATE TABLE IF NOT EXISTS）。翻译数据存在 JSON 文件里，不需要表。
pub fn init(_conn: &rusqlite::Connection) -> Result<()> {
    Ok(())
}

/// 启动时调用一次（可以启动后台任务）
pub fn setup(_app: &tauri::AppHandle) {}

// ---------- 纯函数：归一化、解析、术语、导出命名 ----------

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|x| x.is_finite())
}

fn str_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(|s| s.trim().to_string())
}

fn bool_field(v: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|k| match v.get(*k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => {
            let l = s.to_lowercase();
            l.contains("竖") || l.contains("vert") || l == "true"
        }
        _ => false,
    })
}

/// 归一化 bbox：裁剪到 0–1，拒绝太小或非法的框。有的模型返回 0–100 的百分比，这里自动换算。
pub fn normalize_bbox(raw: [f64; 4]) -> Option<[f32; 4]> {
    if raw.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let scale = if raw.iter().any(|v| *v > 1.0) && raw.iter().all(|v| *v <= 100.0) {
        0.01
    } else {
        1.0
    };
    let [x, y, w, h] = raw.map(|v| v * scale);
    let x0 = x.clamp(0.0, 1.0);
    let y0 = y.clamp(0.0, 1.0);
    let x1 = (x + w).clamp(0.0, 1.0);
    let y1 = (y + h).clamp(0.0, 1.0);
    let (w, h) = (x1 - x0, y1 - y0);
    if w < 0.005 || h < 0.005 {
        return None;
    }
    Some([x0 as f32, y0 as f32, w as f32, h as f32])
}

fn bbox_of(b: &Value) -> Option<[f64; 4]> {
    let v = &b["bbox"];
    if let Some(a) = v.as_array() {
        if a.len() < 4 {
            return None;
        }
        return Some([num(&a[0])?, num(&a[1])?, num(&a[2])?, num(&a[3])?]);
    }
    // 也接受 {x,y,w,h} 或 {x,y,width,height}，直接放在块上或放在 bbox 里
    let o = if v.is_object() { v } else { b };
    let w = o.get("w").or_else(|| o.get("width"))?;
    let h = o.get("h").or_else(|| o.get("height"))?;
    Some([num(&o["x"])?, num(&o["y"])?, num(w)?, num(h)?])
}

/// 模型给的类型名 → 内部类型
pub fn kind_of(s: &str) -> &'static str {
    let l = s.to_lowercase();
    if l.contains("sfx") || s.contains("拟声") || s.contains("音效") {
        "sfx"
    } else if l.contains("art") || s.contains("画面") || s.contains("招牌") {
        "art"
    } else if l.contains("caption") || s.contains("旁白") {
        "caption"
    } else {
        "bubble"
    }
}

/// 解析识别结果（容忍字段缺失、数字写成字符串、bbox 写成对象等）。
/// 返回的块按阅读顺序编号，拟声词和画面文字标记为 skipped。
pub fn parse_detect(page: u32, v: &Value) -> Vec<Block> {
    let Some(arr) = v.as_array().or_else(|| v.get("blocks").and_then(Value::as_array)) else {
        return vec![];
    };
    struct Raw {
        order: Option<f64>,
        bbox: [f32; 4],
        vertical: bool,
        src: String,
        kind: &'static str,
    }
    let mut rows: Vec<Raw> = arr
        .iter()
        .filter_map(|b| {
            let src = str_field(b, &["text", "src", "原文"]).filter(|s| !s.is_empty())?;
            let bbox = normalize_bbox(bbox_of(b)?)?;
            Some(Raw {
                order: num(b.get("order").unwrap_or(&Value::Null)),
                bbox,
                vertical: bool_field(b, &["vertical", "竖排"]),
                src,
                kind: kind_of(&str_field(b, &["kind", "type", "类型"]).unwrap_or_default()),
            })
        })
        .collect();
    // 模型给了顺序就按顺序；没给的排在后面，按行从上到下、同行从右到左（日漫习惯）
    rows.sort_by_key(|r| {
        let o = r.order.map(|o| (o * 1000.0) as i64).unwrap_or(i64::MAX);
        (o, (r.bbox[1] * 20.0) as i64, -((r.bbox[0] * 1000.0) as i64))
    });
    rows.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let skipped = r.kind == "sfx" || r.kind == "art";
            Block {
                id: format!("{page}-{}", i + 1),
                bbox: r.bbox,
                vertical: r.vertical,
                order: i as u32 + 1,
                src: r.src,
                tr: String::new(),
                alt: None,
                kind: r.kind.to_string(),
                skipped,
                bg: None,
            }
        })
        .collect()
}

/// 解析译文：必须是数组（或 {items:[…]}），条数必须与输入相同，每条译文不能为空。
pub fn parse_translations(v: &Value, expected: usize) -> Result<Vec<TrOut>, String> {
    let arr = v
        .as_array()
        .or_else(|| v.get("items").and_then(Value::as_array))
        .or_else(|| v.get("translations").and_then(Value::as_array))
        .ok_or_else(|| "返回的不是 JSON 数组".to_string())?;
    if arr.len() != expected {
        return Err(format!("返回了 {} 条译文，应为 {} 条", arr.len(), expected));
    }
    arr.iter()
        .enumerate()
        .map(|(i, it)| {
            let (tr, alt) = match it {
                Value::String(s) => (s.trim().to_string(), vec![]),
                _ => {
                    let alt = it
                        .get("alt")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    (str_field(it, &["tr", "translation", "译文"]).unwrap_or_default(), alt)
                }
            };
            if tr.is_empty() {
                return Err(format!("第 {} 条译文为空", i + 1));
            }
            Ok(TrOut { tr, alt })
        })
        .collect()
}

/// 替换文字里的术语，返回替换后的文字和替换次数
pub fn replace_term(text: &str, from: &str, to: &str) -> (String, usize) {
    if from.is_empty() || from == to {
        return (text.to_string(), 0);
    }
    (text.replace(from, to), text.matches(from).count())
}

/// 术语表改动后，把已有译文里的旧译法换成新译法（按原文 src 对应）。返回替换次数。
pub fn apply_glossary_change(book: &mut BookTr, old: &[Term], new: &[Term]) -> usize {
    let mut n = 0;
    for t in new {
        let Some(o) = old.iter().find(|o| o.src == t.src) else { continue };
        if o.tr.is_empty() || o.tr == t.tr {
            continue;
        }
        for pg in book.pages.values_mut() {
            for b in pg.blocks.iter_mut() {
                let (s, c) = replace_term(&b.tr, &o.tr, &t.tr);
                b.tr = s;
                n += c;
                if let Some(alt) = b.alt.as_mut() {
                    for a in alt.iter_mut() {
                        let (s, c) = replace_term(a, &o.tr, &t.tr);
                        *a = s;
                        n += c;
                    }
                }
            }
        }
    }
    n
}

/// 把 Unix 秒换成 "20261121-153012"，用于导出文件重名时的后缀（不引入日期库）
pub fn stamp_of(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil_from_days（Howard Hinnant）
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// 导出文件名：“原文件名 [中文].cbz”，放在原文件旁边。已存在时加时间后缀，不覆盖旧版。
pub fn export_target(src: &Path, exists: impl Fn(&Path) -> bool, stamp: &str) -> PathBuf {
    let stem = if src.is_dir() { src.file_name() } else { src.file_stem() }
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "漫画".into());
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let first = dir.join(format!("{stem} [中文].cbz"));
    if !exists(&first) {
        return first;
    }
    dir.join(format!("{stem} [中文] {stamp}.cbz"))
}

fn ext_of_mime(m: &str) -> &'static str {
    match m {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/avif" => "avif",
        _ => "jpg",
    }
}

/// 取一组像素的逐通道中位数（对异常点不敏感）
pub fn median_color(px: &[[u8; 3]]) -> Option<[u8; 3]> {
    if px.is_empty() {
        return None;
    }
    let mut out = [0u8; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let mut v: Vec<u8> = px.iter().map(|p| p[c]).collect();
        v.sort_unstable();
        *o = v[v.len() / 2];
    }
    Some(out)
}

/// 取文字框外圈的颜色作为气泡底色（文字在框内，框外一圈通常是气泡底）
pub fn sample_bg(img: &RgbImage, bbox: [f32; 4]) -> Option<String> {
    let (iw, ih) = (img.width(), img.height());
    if iw < 2 || ih < 2 {
        return None;
    }
    let (w, h) = (iw as f32, ih as f32);
    let pad = 3.0f32;
    let x0 = ((bbox[0] * w) - pad).floor().max(0.0) as u32;
    let y0 = ((bbox[1] * h) - pad).floor().max(0.0) as u32;
    let x1 = (((bbox[0] + bbox[2]) * w) + pad).ceil().min(w - 1.0) as u32;
    let y1 = (((bbox[1] + bbox[3]) * h) + pad).ceil().min(h - 1.0) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut px: Vec<[u8; 3]> = Vec::new();
    for x in x0..=x1 {
        px.push(img.get_pixel(x, y0).0);
        px.push(img.get_pixel(x, y1).0);
    }
    for y in y0..=y1 {
        px.push(img.get_pixel(x0, y).0);
        px.push(img.get_pixel(x1, y).0);
    }
    let [r, g, b] = median_color(&px)?;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// 拼翻译请求：术语表 + 前后文（仅供参考）+ 待翻译句子（带序号）
pub fn build_translate_user(items: &[(String, String)], prev: &[String], next: &[String], glossary: &[Term]) -> String {
    let mut s = String::new();
    let terms: Vec<&Term> = glossary.iter().filter(|t| !t.src.trim().is_empty() && !t.tr.trim().is_empty()).collect();
    if !terms.is_empty() {
        s.push_str("术语表（必须统一使用这些译法）：\n");
        for t in terms {
            s.push_str(&format!("- {} → {}\n", t.src, t.tr));
        }
        s.push('\n');
    }
    if !prev.is_empty() {
        s.push_str("前文原文（仅供理解上下文，不要翻译）：\n");
        for l in prev {
            s.push_str(&format!("- {l}\n"));
        }
        s.push('\n');
    }
    if !next.is_empty() {
        s.push_str("后文原文（仅供理解上下文，不要翻译）：\n");
        for l in next {
            s.push_str(&format!("- {l}\n"));
        }
        s.push('\n');
    }
    let n = items.len();
    s.push_str(&format!("待翻译（按阅读顺序，共 {n} 句，必须输出 {n} 个元素，顺序一致）：\n"));
    for (i, (_, src)) in items.iter().enumerate() {
        s.push_str(&format!("{}. {}\n", i + 1, src));
    }
    s
}

/// 把图片缩到长边 ≤ MAX_SIDE，编码成 JPEG；同时返回缩放后的像素（用于取底色）
pub fn prepare_image(bytes: &[u8]) -> Result<(Vec<u8>, RgbImage)> {
    let mut img = image::load_from_memory(bytes)?.to_rgb8();
    let (w, h) = img.dimensions();
    let m = w.max(h);
    if m > MAX_SIDE {
        let s = MAX_SIDE as f64 / m as f64;
        let nw = ((w as f64 * s).round() as u32).max(1);
        let nh = ((h as f64 * s).round() as u32).max(1);
        img = image::imageops::resize(&img, nw, nh, FilterType::Triangle);
    }
    let (w, h) = img.dimensions();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)?;
    Ok((out, img))
}

/// 每页大约用量（粗略估算，实际以服务商计费为准）。本地识别时只有翻译用云端。
fn per_page_tokens(local: bool) -> u64 {
    // 图片约 1600px 时视觉输入按 1500 token 左右估
    let vision = 1500 + ai::estimate_tokens(VISION_SYSTEM) + ai::estimate_tokens(VISION_PROMPT) + 600;
    (if local { 0 } else { vision }) + ai::estimate_tokens(TRANSLATE_SYSTEM) + 800 + 400
}

fn use_local_ocr(app: &AppHandle) -> bool {
    ai::load_config(&app.state::<AppState>().db.lock()).ocr_engine == "local"
}

/// 本地 OCR 的输出整理成一句：日文、中文的换行直接拼起来，英文等用空格连接
pub fn join_ocr_lines(s: &str) -> String {
    let mut out = String::new();
    for line in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let cjk = |c: char| matches!(c as u32, 0x3000..=0x9fff | 0xf900..=0xfaff | 0xff00..=0xffef);
        let glue = match (out.chars().last(), line.chars().next()) {
            (Some(a), Some(b)) => !(cjk(a) || cjk(b)),
            _ => false,
        };
        if glue {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
}

/// 识别结果里至少要有一个字母、数字或汉字假名，否则当作误检丢掉
pub fn meaningful(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric())
}

/// 本地识别：检测文字块 → 每块裁出来交给 Xiaomi-OCR-0
async fn detect_local(app: &AppHandle, c: Ctx, page: u32, original: Vec<u8>, cancel: Option<&AtomicBool>) -> Result<Vec<Block>> {
    let (img, found) = tauri::async_runtime::spawn_blocking(move || -> Result<_> {
        let img = image::load_from_memory(&original)?.to_rgb8();
        let found = textdet::detect(&img)?;
        Ok((img, found))
    })
    .await
    .map_err(|e| anyhow!("文字检测失败：{e}"))??;
    let total = found.len();
    let mut blocks = vec![];
    for (i, b) in found.into_iter().enumerate() {
        cancelled(cancel)?;
        emit(app, c, page, "detect", &format!("本地识别文字 {}/{}", i + 1, total));
        let png = textdet::crop_png(&img, b.bbox)?;
        let text = join_ocr_lines(&localocr::recognize(app, &png, localocr::PROMPT_TEXT, 512).await?);
        if !meaningful(&text) {
            continue;
        }
        blocks.push(serde_json::json!({"bbox": b.bbox, "vertical": b.vertical, "text": text, "kind": "bubble"}));
    }
    Ok(parse_detect(page, &Value::Array(blocks)))
}

// ---------- 文件与状态 ----------

fn book_file(data: &Path, comic_id: i64) -> PathBuf {
    data.join("mangatr").join(format!("{comic_id}.json"))
}

fn comic_path(st: &AppState, comic_id: i64) -> Result<PathBuf> {
    let p = library::file_path(&st.db.lock(), comic_id)?;
    Ok(PathBuf::from(p))
}

fn page_count(path: &Path) -> Result<u32> {
    if path.is_dir() {
        return Ok(comic::folder_pages(path)?.len() as u32);
    }
    if parse::ext(path) == "pdf" {
        return Err(anyhow!("PDF 暂不支持漫画翻译嵌字，请使用 CBZ / ZIP 或图片文件夹"));
    }
    Ok(comic::zip_pages(path)?.len() as u32)
}

/// 读翻译数据。文件损坏时把它改名备份，返回错误，避免覆盖掉用户的数据。
fn load_book(file: &Path) -> Result<BookTr> {
    match std::fs::read(file) {
        Ok(b) => match serde_json::from_slice(&b) {
            Ok(v) => Ok(v),
            Err(e) => {
                let bak = file.with_extension("json.bak");
                let _ = std::fs::rename(file, &bak);
                Err(anyhow!("翻译数据损坏，已备份为 {}：{e}", bak.display()))
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BookTr::default()),
        Err(e) => Err(e.into()),
    }
}

fn save_book(file: &Path, book: &BookTr) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(book)?)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

fn read_book(file: &Path) -> Result<BookTr> {
    let _g = IO.lock();
    load_book(file)
}

/// 读-改-写一本书的翻译数据（串行）。f 返回错误时不保存。
fn with_book<T>(file: &Path, f: impl FnOnce(&mut BookTr) -> Result<T>) -> Result<T> {
    let _g = IO.lock();
    let mut b = load_book(file)?;
    let r = f(&mut b)?;
    save_book(file, &b)?;
    Ok(r)
}

fn es(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn cancelled(flag: Option<&AtomicBool>) -> Result<()> {
    if flag.is_some_and(|f| f.load(Ordering::SeqCst)) {
        return Err(anyhow!(CANCEL_MSG));
    }
    Ok(())
}

fn emit(app: &AppHandle, c: Ctx, page: u32, stage: &str, message: &str) {
    let _ = app.emit(
        "mangatr://progress",
        Progress {
            comic_id: c.comic_id,
            page,
            from: c.from,
            to: c.to,
            done: c.done,
            total: c.total,
            stage: stage.into(),
            message: message.into(),
        },
    );
}

#[derive(Clone, Copy)]
struct Ctx {
    comic_id: i64,
    from: u32,
    to: u32,
    done: u32,
    total: u32,
}

/// 前后页的已识别原文，作为翻译上下文
fn neighbors(book: &BookTr, page: u32) -> (Vec<String>, Vec<String>) {
    let prev = page
        .checked_sub(1)
        .and_then(|p| book.pages.get(&p))
        .map(|pg| pg.blocks.iter().filter(|b| !b.skipped).map(|b| b.src.clone()).collect::<Vec<_>>())
        .unwrap_or_default();
    let prev = prev[prev.len().saturating_sub(10)..].to_vec();
    let next: Vec<String> = book
        .pages
        .get(&(page + 1))
        .map(|pg| pg.blocks.iter().filter(|b| !b.skipped).take(10).map(|b| b.src.clone()).collect())
        .unwrap_or_default();
    (prev, next)
}

// ---------- AI 调用 ----------

/// 识别：图像识别模型一次完成检测 + OCR。JSON 解析失败时重试一次。
async fn detect(app: &AppHandle, jpeg: &[u8], page: u32) -> Result<Vec<Block>> {
    let mut last = String::new();
    for _ in 0..2 {
        let text = ai::vision(app, VISION_SYSTEM, VISION_PROMPT, jpeg, "image/jpeg", false).await?;
        match ai::extract_json(&text) {
            Ok(v) => return Ok(parse_detect(page, &v)),
            Err(e) => last = e.to_string(),
        }
    }
    Err(anyhow!("识别结果不是有效的 JSON：{last}"))
}

/// 翻译一组句子。条数不对齐或格式不对时重试一次。
async fn translate_lines(app: &AppHandle, items: &[(String, String)], prev: &[String], next: &[String], glossary: &[Term]) -> Result<Vec<TrOut>> {
    let user = build_translate_user(items, prev, next, glossary);
    let mut last = String::new();
    for attempt in 0..2 {
        let u = if attempt == 0 {
            user.clone()
        } else {
            format!("{user}\n注意：上一次的返回条数或格式不对。必须输出恰好 {} 个元素的 JSON 数组。", items.len())
        };
        let text = ai::chat_simple(app, TRANSLATE_SYSTEM, &u, false).await?;
        match ai::extract_json(&text)
            .map_err(|e| e.to_string())
            .and_then(|v| parse_translations(&v, items.len()))
        {
            Ok(v) => return Ok(v),
            Err(e) => last = e,
        }
    }
    Err(anyhow!("译文与原文对不上（{last}），已重试一次仍失败"))
}

fn apply_outs(blocks: &mut [Block], idx: &[usize], outs: Vec<TrOut>) {
    for (k, o) in outs.into_iter().enumerate() {
        if let Some(&i) = idx.get(k) {
            blocks[i].tr = o.tr;
            blocks[i].alt = if o.alt.is_empty() { None } else { Some(o.alt) };
        }
    }
}

/// 识别 + 翻译一页并保存。成功才保存；中途失败不写入。
async fn run_page(app: &AppHandle, c: Ctx, page: u32, cancel: Option<&AtomicBool>) -> Result<PageTr> {
    let (path, file) = {
        let st = app.state::<AppState>();
        (comic_path(&st, c.comic_id)?, book_file(&st.dirs.data, c.comic_id))
    };
    let count = page_count(&path)?;
    if page >= count {
        return Err(anyhow!("没有第 {} 页", page + 1));
    }
    emit(app, c, page, "load", "读取页面");
    let (bytes, _) = comic::read_page(&path, page as usize)?;
    let local = use_local_ocr(app);
    let original = if local { bytes.clone() } else { Vec::new() };
    let (jpeg, rgb) = tauri::async_runtime::spawn_blocking(move || prepare_image(&bytes))
        .await
        .map_err(|e| anyhow!("图片处理失败：{e}"))??;
    cancelled(cancel)?;

    emit(app, c, page, "detect", "识别文字区域");
    let mut blocks = if local {
        detect_local(app, c, page, original, cancel).await?
    } else {
        detect(app, &jpeg, page).await?
    };
    for b in blocks.iter_mut().filter(|b| !b.skipped) {
        b.bg = sample_bg(&rgb, b.bbox);
    }
    cancelled(cancel)?;

    let todo: Vec<usize> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.skipped && !b.src.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    if !todo.is_empty() {
        let book = read_book(&file)?;
        let (prev, next) = neighbors(&book, page);
        let items: Vec<(String, String)> = todo.iter().map(|&i| (blocks[i].id.clone(), blocks[i].src.clone())).collect();
        emit(app, c, page, "translate", "结合上下文翻译");
        let outs = translate_lines(app, &items, &prev, &next, &book.glossary).await?;
        apply_outs(&mut blocks, &todo, outs);
    }
    cancelled(cancel)?;

    let pg = PageTr { blocks };
    with_book(&file, |b| {
        b.pages.insert(page, pg.clone());
        Ok(())
    })?;
    emit(app, c, page, "saved", "已保存");
    Ok(pg)
}

// ---------- 导出 ----------

fn export_cbz(path: &Path, pages_png: Vec<(u32, String)>) -> Result<PathBuf> {
    if parse::ext(path) == "pdf" {
        return Err(anyhow!("PDF 来源暂不支持导出为 CBZ，请先转换为 CBZ 或图片文件夹"));
    }
    let count = page_count(path)?;
    let mut pngs: HashMap<u32, Vec<u8>> = HashMap::new();
    for (p, b64) in pages_png {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| anyhow!("第 {} 页的图片数据无效：{e}", p + 1))?;
        pngs.insert(p, bytes);
    }
    let out = export_target(path, |p| p.exists(), &stamp_of(db::now()));
    let tmp = out.with_extension("cbz.part");
    let result = (|| -> Result<()> {
        let f = std::fs::File::create(&tmp)?;
        let mut zw = zip::ZipWriter::new(f);
        for i in 0..count {
            let (data, ext) = match pngs.remove(&i) {
                Some(d) => (d, "png"),
                None => {
                    let (b, m) = comic::read_page(path, i as usize)?;
                    (b, ext_of_mime(&m))
                }
            };
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zw.start_file(format!("{:04}.{ext}", i + 1), opts)?;
            zw.write_all(&data)?;
        }
        zw.finish()?;
        std::fs::rename(&tmp, &out)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(out)
}

// ---------- 命令 ----------

/// 全书翻译数据
#[tauri::command]
pub fn mangatr_get(st: State<AppState>, comic_id: i64) -> Result<BookTr, String> {
    read_book(&book_file(&st.dirs.data, comic_id)).map_err(es)
}

/// 识别 + 翻译一页（进度：mangatr://progress）
#[tauri::command]
pub async fn mangatr_translate_page(app: AppHandle, comic_id: i64, page: u32) -> Result<PageTr, String> {
    let c = Ctx {
        comic_id,
        from: page,
        to: page,
        done: 0,
        total: 1,
    };
    run_page(&app, c, page, None).await.map_err(es)
}

/// 后台逐页翻译 [from, to]（含两端）。用 mangatr_cancel 取消。
#[tauri::command]
pub async fn mangatr_translate_range(app: AppHandle, comic_id: i64, from: u32, to: u32) -> Result<(), String> {
    if from > to {
        return Err("起始页不能大于结束页".into());
    }
    let count = {
        let st = app.state::<AppState>();
        page_count(&comic_path(&st, comic_id).map_err(es)?).map_err(es)?
    };
    if to >= count {
        return Err(format!("这本书只有 {count} 页"));
    }
    let flag = {
        let mut m = CANCEL.lock();
        if m.contains_key(&comic_id) {
            return Err("这本书正在翻译中，请等它完成或先取消".into());
        }
        let f = Arc::new(AtomicBool::new(false));
        m.insert(comic_id, f.clone());
        f
    };
    let total = to - from + 1;
    tauri::async_runtime::spawn(async move {
        let mut outcome = ("done", String::from("翻译完成"));
        for (i, p) in (from..=to).enumerate() {
            let c = Ctx {
                comic_id,
                from,
                to,
                done: i as u32,
                total,
            };
            if flag.load(Ordering::SeqCst) {
                outcome = ("cancelled", "已取消".into());
                break;
            }
            if let Err(e) = run_page(&app, c, p, Some(flag.as_ref())).await {
                if flag.load(Ordering::SeqCst) {
                    outcome = ("cancelled", "已取消".into());
                } else {
                    outcome = ("error", format!("第 {} 页：{}", p + 1, es(e)));
                }
                break;
            }
        }
        CANCEL.lock().remove(&comic_id);
        let c = Ctx {
            comic_id,
            from,
            to,
            done: total,
            total,
        };
        emit(&app, c, to, outcome.0, &outcome.1);
    });
    Ok(())
}

#[tauri::command]
pub fn mangatr_cancel(comic_id: i64) -> Result<(), String> {
    if let Some(f) = CANCEL.lock().get(&comic_id) {
        f.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// 预计用量（粗略估算），开始前显示给用户确认
#[tauri::command]
pub fn mangatr_estimate(st: State<AppState>, comic_id: i64, from: u32, to: u32) -> Result<Estimate, String> {
    if from > to {
        return Err("起始页不能大于结束页".into());
    }
    let count = page_count(&comic_path(&st, comic_id).map_err(es)?).map_err(es)?;
    if to >= count {
        return Err(format!("这本书只有 {count} 页"));
    }
    let n = to - from + 1;
    let local = ai::load_config(&st.db.lock()).ocr_engine == "local";
    let per_page = per_page_tokens(local);
    Ok(Estimate {
        pages: n,
        per_page,
        total: per_page * n as u64,
        note: if local {
            "文字在本机识别，不上传图片；只有翻译用云端，按每页一次上下文翻译估算。本地识别每页约十几秒到一分钟（有显卡会快很多）".into()
        } else {
            "按每页约 1600 像素的图片和一次上下文翻译估算，实际以服务商计费为准".into()
        },
    })
}

/// 手动修改一句译文（立刻保存）
#[tauri::command]
pub fn mangatr_update_block(st: State<AppState>, comic_id: i64, page: u32, block_id: String, tr: String) -> Result<Block, String> {
    let file = book_file(&st.dirs.data, comic_id);
    with_book(&file, |b| {
        let blk = b
            .pages
            .get_mut(&page)
            .and_then(|p| p.blocks.iter_mut().find(|x| x.id == block_id))
            .ok_or_else(|| anyhow!("找不到这个气泡"))?;
        blk.tr = tr;
        Ok(blk.clone())
    })
    .map_err(es)
}

/// 带前后文重新翻译一句，给出新译文和备选
#[tauri::command]
pub async fn mangatr_retranslate_block(app: AppHandle, comic_id: i64, page: u32, block_id: String) -> Result<Block, String> {
    let file = {
        let st = app.state::<AppState>();
        book_file(&st.dirs.data, comic_id)
    };
    let book = read_book(&file).map_err(es)?;
    let pg = book.pages.get(&page).ok_or("这一页还没有翻译")?;
    let idx = pg.blocks.iter().position(|b| b.id == block_id).ok_or("找不到这个气泡")?;
    let target = pg.blocks[idx].clone();
    if target.skipped || target.src.trim().is_empty() {
        return Err("拟声词和画面文字不参与翻译".into());
    }
    // 上下文：同页前后的句子 + 前后页
    let (mut prev, mut next) = neighbors(&book, page);
    let same: Vec<String> = pg.blocks.iter().filter(|b| !b.skipped).map(|b| b.src.clone()).collect();
    // 目标句在“非跳过句”里的序号：它前面有几句非跳过句
    let my = pg.blocks[..idx].iter().filter(|b| !b.skipped).count();
    let mut before: Vec<String> = same[..my.min(same.len())].to_vec();
    before.append(&mut prev);
    let mut after: Vec<String> = same[(my + 1).min(same.len())..].to_vec();
    after.append(&mut next);
    let items = vec![(target.id.clone(), target.src.clone())];
    let outs = translate_lines(&app, &items, &before, &after, &book.glossary).await.map_err(es)?;
    let o = outs.into_iter().next().unwrap_or_default();
    with_book(&file, |b| {
        let blk = b
            .pages
            .get_mut(&page)
            .and_then(|p| p.blocks.iter_mut().find(|x| x.id == block_id))
            .ok_or_else(|| anyhow!("气泡已被删除"))?;
        blk.tr = o.tr;
        blk.alt = if o.alt.is_empty() { None } else { Some(o.alt) };
        Ok(blk.clone())
    })
    .map_err(es)
}

/// 改术语表；apply_all 时把已有译文里的旧译法替换成新译法。返回替换次数。
#[tauri::command]
pub fn mangatr_glossary_set(st: State<AppState>, comic_id: i64, glossary: Vec<Term>, apply_all: bool) -> Result<usize, String> {
    let file = book_file(&st.dirs.data, comic_id);
    // 去掉空项，同一原文只保留最后一条
    let mut clean: Vec<Term> = Vec::new();
    for t in glossary {
        let src = t.src.trim().to_string();
        let tr = t.tr.trim().to_string();
        if src.is_empty() {
            continue;
        }
        clean.retain(|c| c.src != src);
        clean.push(Term { src, tr });
    }
    with_book(&file, |b| {
        let old = std::mem::replace(&mut b.glossary, clean.clone());
        Ok(if apply_all { apply_glossary_change(b, &old, &clean) } else { 0 })
    })
    .map_err(es)
}

/// 导出 CBZ：pages_png 里的页用前端渲染好的译文图（base64 PNG），其余页拷原图。输出到原文件旁边。
#[tauri::command]
pub async fn mangatr_export(app: AppHandle, comic_id: i64, pages_png: Vec<(u32, String)>) -> Result<String, String> {
    let path = {
        let st = app.state::<AppState>();
        comic_path(&st, comic_id).map_err(es)?
    };
    tauri::async_runtime::spawn_blocking(move || export_cbz(&path, pages_png).map(|p| p.to_string_lossy().to_string()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(es)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bbox_clamps_and_rejects() {
        let b = normalize_bbox([-0.1, 0.5, 0.5, 0.1]).unwrap();
        assert_eq!(b[0], 0.0);
        assert!((b[2] - 0.4).abs() < 1e-6);
        assert!(normalize_bbox([1.2, 0.2, 0.1, 0.1]).is_none(), "完全在画面外");
        assert!(normalize_bbox([0.1, 0.1, 0.001, 0.2]).is_none(), "太窄");
        assert!(normalize_bbox([f64::NAN, 0.1, 0.2, 0.2]).is_none());
    }

    #[test]
    fn bbox_percent_scale() {
        let b = normalize_bbox([10.0, 20.0, 30.0, 40.0]).unwrap();
        assert!((b[0] - 0.1).abs() < 1e-6 && (b[3] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn detect_parse_is_tolerant() {
        let v = json!({"blocks":[
            {"bbox":["0.5","0.1","0.3","0.2"],"vertical":"竖排","order":"2","text":"  第二句 ","kind":"对话气泡"},
            {"bbox":{"x":0.1,"y":0.1,"width":0.2,"height":0.2},"order":1,"text":"第一句","kind":"旁白框"},
            {"bbox":[0.1,0.5,0.2,0.2],"text":"ドン","kind":"拟声词"},
            {"bbox":[0.1,0.7,0.2,0.2],"text":""},
            {"bbox":[0.1,0.8]},
            {"bbox":[0.2,0.3,0.2,0.1],"text":"无类型"}
        ]});
        let bl = parse_detect(3, &v);
        assert_eq!(bl.len(), 4, "空文字、bbox 不全的应丢弃");
        assert_eq!(bl[0].src, "第一句");
        assert_eq!(bl[0].kind, "caption");
        assert_eq!(bl[0].id, "3-1");
        assert_eq!(bl[0].order, 1);
        assert!(bl.iter().find(|b| b.src == "ドン").unwrap().skipped);
        let second = bl.iter().find(|b| b.src == "第二句").unwrap();
        assert!(second.vertical);
        assert!(!second.skipped);
        assert_eq!(second.kind, "bubble");
        assert_eq!(second.order, 2);
    }

    #[test]
    fn detect_without_order_reads_right_to_left() {
        let v = json!([
            {"bbox":[0.1,0.1,0.2,0.2],"text":"左"},
            {"bbox":[0.6,0.1,0.2,0.2],"text":"右"}
        ]);
        let bl = parse_detect(0, &v);
        assert_eq!(bl[0].src, "右");
        assert_eq!(bl[1].src, "左");
    }

    #[test]
    fn translations_must_align() {
        let ok = parse_translations(&json!([{"tr":"你好","alt":["嗨"]},"再见"]), 2).unwrap();
        assert_eq!(ok[0].tr, "你好");
        assert_eq!(ok[0].alt, vec!["嗨".to_string()]);
        assert_eq!(ok[1].tr, "再见");
        assert!(parse_translations(&json!([{"tr":"只有一条"}]), 2).is_err());
        assert!(parse_translations(&json!({"items":[{"tr":"a"},{"tr":""}]}), 2).is_err(), "空译文不接受");
        assert!(parse_translations(&json!("not array"), 1).is_err());
        assert_eq!(parse_translations(&json!({"items":[{"tr":"a"}]}), 1).unwrap()[0].tr, "a");
    }

    #[test]
    fn term_replacement_and_glossary_change() {
        assert_eq!(
            replace_term("港口邮局和港口邮局", "港口邮局", "港口邮政局"),
            ("港口邮政局和港口邮政局".into(), 2)
        );
        assert_eq!(replace_term("abc", "", "x"), ("abc".into(), 0));

        let blk = |tr: &str, alt: Option<Vec<String>>| Block {
            id: "0-1".into(),
            bbox: [0.0; 4],
            vertical: false,
            order: 1,
            src: "港の郵便局".into(),
            tr: tr.into(),
            alt,
            kind: "bubble".into(),
            skipped: false,
            bg: None,
        };
        let mut book = BookTr::default();
        book.pages.insert(
            0,
            PageTr {
                blocks: vec![blk("去港口邮局", Some(vec!["港口邮局？".into()]))],
            },
        );
        let old = vec![Term {
            src: "港の郵便局".into(),
            tr: "港口邮局".into(),
        }];
        let new = vec![Term {
            src: "港の郵便局".into(),
            tr: "港口邮政局".into(),
        }];
        assert_eq!(apply_glossary_change(&mut book, &old, &new), 2);
        let b = &book.pages[&0].blocks[0];
        assert_eq!(b.tr, "去港口邮政局");
        assert_eq!(b.alt.as_ref().unwrap()[0], "港口邮政局？");
        // 没有改译法时不替换
        assert_eq!(apply_glossary_change(&mut book, &new, &new), 0);
    }

    #[test]
    fn export_names_and_stamp() {
        assert_eq!(stamp_of(0), "19700101-000000");
        // 2026-11-21 01:01:01 UTC
        assert_eq!(stamp_of(1795219200 + 3661), "20261121-010101");
        let src = Path::new("D:/漫画/雾港邮局 第1卷.cbz");
        assert_eq!(export_target(src, |_| false, "x"), PathBuf::from("D:/漫画/雾港邮局 第1卷 [中文].cbz"));
        assert_eq!(
            export_target(src, |_| true, "20261121-010101"),
            PathBuf::from("D:/漫画/雾港邮局 第1卷 [中文] 20261121-010101.cbz")
        );
    }

    #[test]
    fn median_and_background() {
        assert_eq!(median_color(&[[1, 9, 9], [2, 8, 8], [3, 7, 7]]), Some([2, 8, 8]));
        assert_eq!(median_color(&[]), None);

        let mut img = RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255]));
        for y in 30..70 {
            for x in 30..70 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        // 文字在框内，框外一圈是白底
        assert_eq!(sample_bg(&img, [0.3, 0.3, 0.4, 0.4]).as_deref(), Some("#ffffff"));
        let dark = RgbImage::from_pixel(100, 100, image::Rgb([20, 20, 20]));
        assert_eq!(sample_bg(&dark, [0.3, 0.3, 0.4, 0.4]).as_deref(), Some("#141414"));
    }

    #[test]
    fn prompt_has_glossary_context_and_numbering() {
        let items = vec![("1-1".to_string(), "ねえ".to_string()), ("1-2".to_string(), "行こう".to_string())];
        let g = vec![
            Term {
                src: "霧港".into(),
                tr: "雾港".into(),
            },
            Term {
                src: "".into(),
                tr: "空".into(),
            },
        ];
        let s = build_translate_user(&items, &["前一句".into()], &[], &g);
        assert!(s.contains("霧港 → 雾港"));
        assert!(!s.contains("→ 空"), "空原文的术语要丢掉");
        assert!(s.contains("前文原文"));
        assert!(!s.contains("后文原文"));
        assert!(s.contains("共 2 句"));
        assert!(s.contains("1. ねえ") && s.contains("2. 行こう"));
    }

    #[test]
    fn file_keys_are_page_numbers() {
        let mut b = BookTr::default();
        b.pages.insert(12, PageTr::default());
        let v: Value = serde_json::to_value(&b).unwrap();
        assert!(v["pages"].get("12").is_some());
        let back: BookTr = serde_json::from_value(v).unwrap();
        assert!(back.pages.contains_key(&12));
    }

    #[test]
    fn per_page_estimate_is_positive() {
        assert!(per_page_tokens(false) > 1000);
        assert!(per_page_tokens(true) < per_page_tokens(false));
    }

    #[test]
    fn ocr_lines_joined() {
        assert_eq!(join_ocr_lines("立川で見た\n“穴”の下の\n巨大な眼は…"), "立川で見た“穴”の下の巨大な眼は…");
        assert_eq!(join_ocr_lines("HELLO\nWORLD"), "HELLO WORLD");
        assert_eq!(join_ocr_lines("LINK！\n私達"), "LINK！私達");
        assert!(meaningful("ギャッ"));
        assert!(!meaningful("…！？"));
    }
}
