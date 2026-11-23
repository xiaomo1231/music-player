//! 虚拟页表：按规则把一本漫画的原始页面变成“阅读时看到的页面”（跳过的去掉、跨页拆成两页），
//! 以及按虚拟页号读出处理后的图片
use super::img::{self, Half, OutFmt, Rect};
use super::spec::{Order, Skip, Spec};
use crate::comic;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------- 原始页面

/// 一本漫画的原始文件：压缩包或图片文件夹
pub enum Book {
    Zip(zip::ZipArchive<std::fs::File>),
    Dir(PathBuf),
}

impl Book {
    pub fn open(path: &Path) -> Result<Book> {
        if path.is_dir() {
            return Ok(Book::Dir(path.to_path_buf()));
        }
        Ok(Book::Zip(zip::ZipArchive::new(std::fs::File::open(path)?)?))
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        match self {
            Book::Dir(d) => Ok(std::fs::read(d.join(name))?),
            Book::Zip(z) => {
                let mut buf = vec![];
                z.by_name(name)?.read_to_end(&mut buf)?;
                Ok(buf)
            }
        }
    }

    /// 图片页面（压缩包里是完整路径，文件夹里是文件名），按 order 排好
    pub fn pages(&self, order: Order) -> Result<Vec<String>> {
        let mut v: Vec<String> = match self {
            Book::Dir(d) => comic::folder_pages(d)?
                .into_iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect(),
            Book::Zip(z) => (0..z.len())
                .filter_map(|i| z.name_for_index(i))
                .filter(|n| is_image(n) && !n.starts_with("__MACOSX"))
                .map(String::from)
                .collect(),
        };
        match order {
            Order::Natural => v.sort_by_key(|n| comic::natural_key(n)),
            Order::Name => v.sort(),
            // 文件夹没有“包内顺序”，按文件名
            Order::Keep => {
                if matches!(self, Book::Dir(_)) {
                    v.sort()
                }
            }
        }
        Ok(v)
    }

    /// ComicInfo.xml 的内容（有的话）
    pub fn comic_info(&mut self) -> Option<String> {
        let name = match self {
            Book::Dir(d) => std::fs::read_dir(&*d)
                .ok()?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.eq_ignore_ascii_case("ComicInfo.xml"))?,
            Book::Zip(z) => z.file_names().find(|n| n.eq_ignore_ascii_case("ComicInfo.xml"))?.to_string(),
        };
        let b = self.read(&name).ok()?;
        Some(String::from_utf8_lossy(&b).into_owned())
    }
}

fn is_image(name: &str) -> bool {
    let e = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    crate::parse::IMAGE.contains(&e.as_str())
}

pub fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        _ => "image/jpeg",
    }
}

/// 文件名（不含压缩包里的文件夹）
pub fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// 我们导出时在 ComicInfo.xml 的 Notes 里写 `em:<规则 id>@<版本>`
pub fn has_marker(xml: &str, rule_id: &str) -> bool {
    static NOTES: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<Notes>(.*?)</Notes>").unwrap());
    let pat = format!("em:{rule_id}");
    NOTES.captures_iter(xml).any(|c| {
        let s = &c[1];
        s.match_indices(&pat).any(|(i, _)| {
            let after = s[i + pat.len()..].chars().next();
            !matches!(after, Some(c) if c.is_ascii_alphanumeric() || c == '-')
        })
    })
}

// ---------------------------------------------------------------- 虚拟页表

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct VPage {
    /// 原始页序号（names 里的位置）
    pub src: usize,
    pub half: Half,
}

#[derive(Clone, Debug, Serialize)]
pub struct Skipped {
    pub src: usize,
    pub why: String,
}

pub struct View {
    pub path: PathBuf,
    pub names: Vec<String>,
    pub pages: Vec<VPage>,
    pub skipped: Vec<Skipped>,
    /// 规则是否改动页面（导出过的文件、规则只管阅读排版时为 false）
    pub processed: bool,
    /// 文件里有我们导出时写的标记
    pub marker: bool,
    pub spec: Arc<Spec>,
    pub ver: String,
    /// 每个虚拟页最后的裁剪框（算过的才有；None 表示不用裁）+ 是否裁了边
    crops: Mutex<HashMap<usize, (Option<Rect>, bool)>>,
}

/// 按规则算虚拟页表。要读尺寸（拆跨页）和感知哈希（like）时会读每一页，多线程
pub fn build(path: &Path, rule_id: &str, spec: Arc<Spec>, ver: String) -> Result<View> {
    let mut book = Book::open(path)?;
    let marker = book.comic_info().map(|x| has_marker(&x, rule_id)).unwrap_or(false);
    let processed = !marker && spec.touches_pages();
    let order = if processed { spec.order } else { Order::Natural };
    let names = book.pages(order)?;
    let n = names.len();
    let mut view = View {
        path: path.to_path_buf(),
        pages: (0..n).map(|src| VPage { src, half: Half::Full }).collect(),
        names,
        skipped: vec![],
        processed,
        marker,
        spec: spec.clone(),
        ver,
        crops: Mutex::new(HashMap::new()),
    };
    if !processed {
        return Ok(view);
    }

    // 1. 按文件名、前后几页跳过
    let mut why: Vec<Option<String>> = vec![None; n];
    for s in &spec.skip {
        match s {
            Skip::Name(re) => {
                for (i, name) in view.names.iter().enumerate() {
                    if why[i].is_none() && re.is_match(base_name(name)) {
                        why[i] = Some("文件名".into());
                    }
                }
            }
            Skip::First(k) => {
                for w in why.iter_mut().take(*k) {
                    w.get_or_insert_with(|| "开头".into());
                }
            }
            Skip::Last(k) => {
                for w in why.iter_mut().skip(n.saturating_sub(*k)) {
                    w.get_or_insert_with(|| "结尾".into());
                }
            }
            Skip::Like(..) => {}
        }
    }

    // 2. 需要读图的：相似页（解码算哈希）、跨页（只读尺寸）
    let samples: Vec<(u64, u32, String)> = spec
        .skip
        .iter()
        .filter_map(|s| match s {
            Skip::Like(f, d) => {
                let b = std::fs::read(Spec::sample_path(f)).ok()?;
                let (im, _) = img::decode(&b)?;
                Some((img::dhash(&im), *d, f.clone()))
            }
            _ => None,
        })
        .collect();
    let want_dims = spec.split.is_some();
    let todo: Vec<usize> = (0..n).filter(|&i| why[i].is_none()).collect();
    let mut dims: Vec<Option<(u32, u32)>> = vec![None; n];
    if want_dims || !samples.is_empty() {
        let info = scan(path, &view.names, &todo, want_dims, !samples.is_empty());
        for (i, d, h) in info {
            dims[i] = d;
            if let Some(h) = h {
                if let Some((_, _, f)) = samples.iter().find(|(s, dist, _)| img::hamming(*s, h) <= *dist) {
                    why[i] = Some(format!("像 {f}"));
                }
            }
        }
    }

    // 3. 组装：跳过的去掉，跨页拆两页
    let mut pages = vec![];
    let mut first = true;
    for i in 0..n {
        if let Some(w) = &why[i] {
            view.skipped.push(Skipped { src: i, why: w.clone() });
            continue;
        }
        let cover = std::mem::replace(&mut first, false);
        let wide = match (&spec.split, dims[i]) {
            (Some(s), Some((w, h))) if h > 0 => (w as f32 / h as f32) > s.ratio && !(cover && s.keep_cover),
            _ => false,
        };
        if wide {
            let rtl = spec.split.as_ref().map(|s| s.rtl).unwrap_or(false);
            let (a, b) = if rtl { (Half::Right, Half::Left) } else { (Half::Left, Half::Right) };
            pages.push(VPage { src: i, half: a });
            pages.push(VPage { src: i, half: b });
        } else {
            pages.push(VPage { src: i, half: Half::Full });
        }
    }
    view.pages = pages;
    Ok(view)
}

/// 页序号、尺寸、dHash
type Scanned = (usize, Option<(u32, u32)>, Option<u64>);

/// 并行读一批页面：尺寸、dHash
fn scan(path: &Path, names: &[String], todo: &[usize], want_dims: bool, want_hash: bool) -> Vec<Scanned> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
    let chunk = todo.len().div_ceil(threads).max(1);
    std::thread::scope(|sc| {
        let hs: Vec<_> = todo
            .chunks(chunk)
            .map(|part| {
                sc.spawn(move || {
                    let mut out = vec![];
                    let Ok(mut book) = Book::open(path) else { return out };
                    for &i in part {
                        let Ok(b) = book.read(&names[i]) else {
                            out.push((i, None, None));
                            continue;
                        };
                        let d = if want_dims { img::dims(&b) } else { None };
                        let h = if want_hash { img::decode(&b).map(|(im, _)| img::dhash(&im)) } else { None };
                        out.push((i, d, h));
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

impl View {
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// 已经算出来的裁剪框
    pub fn crop_known(&self, idx: usize) -> Option<Option<Rect>> {
        self.crops.lock().get(&idx).map(|c| c.0)
    }

    /// 这一页是否裁掉了边（算过才知道）
    pub fn trimmed(&self, idx: usize) -> bool {
        self.crops.lock().get(&idx).map(|c| c.1).unwrap_or(false)
    }

    /// 这一页要保留的区域（半页 + 裁边）；整页不用动时为 None
    pub fn crop_of(&self, idx: usize, im: &image::DynamicImage) -> Option<Rect> {
        if let Some(c) = self.crop_known(idx) {
            return c;
        }
        let vp = self.pages[idx];
        let (w, h) = (im.width(), im.height());
        let half = vp.half.rect(w, h);
        let mut r = half;
        if let Some(t) = &self.spec.trim {
            let g = im.to_luma8();
            r = img::trim_box(&g, r, t);
        }
        let c = if r == Rect::full(w, h) { None } else { Some(r) };
        self.crops.lock().insert(idx, (c, r != half));
        c
    }

    /// 读一页原图
    pub fn raw(&self, book: &mut Book, idx: usize) -> Result<(Vec<u8>, &str)> {
        let vp = self.pages.get(idx).ok_or_else(|| anyhow!("没有第 {} 页", idx + 1))?;
        let name = &self.names[vp.src];
        Ok((book.read(name)?, name))
    }

    /// 阅读用：处理后的一页（JPEG 92；原图是 PNG 时用 PNG）。不用改的页原样返回
    pub fn render(&self, idx: usize) -> Result<(Vec<u8>, String)> {
        let mut book = Book::open(&self.path)?;
        let (bytes, name) = self.raw(&mut book, idx)?;
        let mime = mime_of(name).to_string();
        if !self.processed || (self.pages[idx].half == Half::Full && self.spec.trim.is_none()) {
            return Ok((bytes, mime));
        }
        if self.crop_known(idx) == Some(None) {
            return Ok((bytes, mime));
        }
        // avif、坏图原样放行
        let Some((im, fmt)) = img::decode(&bytes) else {
            return Ok((bytes, mime));
        };
        let Some(r) = self.crop_of(idx, &im) else {
            return Ok((bytes, mime));
        };
        let out = im.crop_imm(r.x, r.y, r.w, r.h);
        let f = if fmt == image::ImageFormat::Png { OutFmt::Png } else { OutFmt::Jpeg };
        Ok((img::encode(&out, f, 92)?, f.mime().to_string()))
    }
}

// ---------------------------------------------------------------- 缓存

struct Slot {
    stamp: String,
    cell: once_cell::sync::OnceCell<Arc<View>>,
}

/// (漫画 id, 规则 id) → 虚拟页表；stamp = 文件修改时间 + 大小 + 规则内容
type ViewCache = HashMap<(i64, String), Arc<Slot>>;
static VIEWS: Lazy<Mutex<ViewCache>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn cached(key: (i64, String), stamp: String, make: impl FnOnce() -> Result<View>) -> Result<Arc<View>> {
    let slot = {
        let mut g = VIEWS.lock();
        if g.len() > 64 {
            g.clear();
        }
        match g.get(&key) {
            Some(s) if s.stamp == stamp => s.clone(),
            _ => {
                let s = Arc::new(Slot {
                    stamp,
                    cell: once_cell::sync::OnceCell::new(),
                });
                g.insert(key, s.clone());
                s
            }
        }
    };
    slot.cell.get_or_try_init(|| make().map(Arc::new)).cloned()
}

/// 清掉某本漫画（None 为全部）的缓存
pub fn forget(comic_id: Option<i64>) {
    let mut g = VIEWS.lock();
    match comic_id {
        Some(id) => g.retain(|k, _| k.0 != id),
        None => g.clear(),
    }
    let mut l = PAGES.lock();
    match comic_id {
        Some(id) => l.retain(|e| e.0 != id),
        None => l.clear(),
    }
}

type Encoded = (i64, String, usize, Arc<Vec<u8>>, String);
/// 处理好的页面（最近用过的几十页）
static PAGES: Lazy<Mutex<Vec<Encoded>>> = Lazy::new(|| Mutex::new(vec![]));
const LRU_BYTES: usize = 96 << 20;
const LRU_COUNT: usize = 40;

pub fn lru_get(id: i64, ver: &str, idx: usize) -> Option<(Arc<Vec<u8>>, String)> {
    let mut g = PAGES.lock();
    let pos = g.iter().position(|e| e.0 == id && e.1 == ver && e.2 == idx)?;
    let e = g.remove(pos);
    let out = (e.3.clone(), e.4.clone());
    g.push(e);
    Some(out)
}

pub fn lru_put(id: i64, ver: &str, idx: usize, bytes: Arc<Vec<u8>>, mime: String) {
    let mut g = PAGES.lock();
    g.retain(|e| !(e.0 == id && e.1 == ver && e.2 == idx));
    g.push((id, ver.to_string(), idx, bytes, mime));
    while g.len() > LRU_COUNT || (g.len() > 1 && g.iter().map(|e| e.3.len()).sum::<usize>() > LRU_BYTES) {
        g.remove(0);
    }
}
