//! 标签、封面、歌词
use crate::parse;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: String,
    pub duration: f64,
    pub bitrate: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub format: String,
    pub lossless: bool,
    pub cover: Option<String>,
    pub hue: Option<i64>,
}

/// 读音频标签；封面写进 cover_dir，文件名是内容哈希，避免重复
pub fn read_track(path: &Path, cover_dir: &Path) -> TrackMeta {
    let e = parse::ext(path);
    let mut m = TrackMeta {
        title: path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string(),
        format: e.to_uppercase(),
        lossless: parse::is_lossless(&e),
        ..Default::default()
    };
    if parse::is_encrypted(path) {
        m.format = "加密格式".into();
        return m;
    }
    if let Ok(tf) = lofty::read_from_path(path) {
        let pr = tf.properties();
        m.duration = pr.duration().as_secs_f64();
        m.bitrate = pr.audio_bitrate();
        m.sample_rate = pr.sample_rate();
        m.bit_depth = pr.bit_depth();
        m.channels = pr.channels();
        if e == "m4a" && m.bit_depth.is_some() {
            // m4a 带位深的就是 ALAC
            m.lossless = true;
            m.format = "ALAC".into();
        }
        if let Some(tag) = tf.primary_tag().or_else(|| tf.first_tag()) {
            if let Some(t) = tag.title() {
                if !t.trim().is_empty() {
                    m.title = t.trim().to_string();
                }
            }
            m.artist = tag.artist().map(|s| s.trim().to_string()).unwrap_or_default();
            m.album = tag.album().map(|s| s.trim().to_string()).unwrap_or_default();
            m.album_artist = tag.get_string(ItemKey::AlbumArtist).unwrap_or("").trim().to_string();
            m.genre = tag.genre().map(|s| s.to_string()).unwrap_or_default();
            m.track_no = tag.track();
            m.disc_no = tag.disk();
            m.year = tag.date().map(|d| d.year as u32);
        }
        // 封面可能不在主标签里（例如 FLAC 前面多了一个 ID3v2，图放在那里），所有标签都找一遍
        if let Some(bytes) = embedded_picture(&tf) {
            if let Some((name, hue)) = save_cover(&bytes, cover_dir) {
                m.cover = Some(name);
                m.hue = Some(hue);
            }
        }
    }
    if m.cover.is_none() {
        if let Some(img) = folder_cover(path) {
            if let Ok(bytes) = std::fs::read(&img) {
                if let Some((name, hue)) = save_cover(&bytes, cover_dir) {
                    m.cover = Some(name);
                    m.hue = Some(hue);
                }
            }
        }
    }
    if m.artist.is_empty() {
        // “艺人 - 歌名.mp3” 这种文件名
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if let Some((a, t)) = m.title.clone().split_once(" - ") {
            m.artist = a.trim().to_string();
            m.title = t.trim().to_string();
        } else if let Some((a, _)) = stem.split_once(" - ") {
            m.artist = a.trim().to_string();
        }
    }
    if m.album.is_empty() {
        m.album = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("").to_string();
    }
    m
}

/// 文件里内嵌的封面：所有标签里优先“封面（正面）”，没有就用第一张图
pub fn embedded_picture(tf: &lofty::file::TaggedFile) -> Option<Vec<u8>> {
    use lofty::picture::PictureType;
    let pics: Vec<&lofty::picture::Picture> = tf.tags().iter().flat_map(|t| t.pictures()).collect();
    pics.iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| {
            pics.iter()
                .find(|p| p.pic_type() != PictureType::CoverBack && p.pic_type() != PictureType::Artist)
        })
        .or(pics.first())
        .map(|p| p.data().to_vec())
}

const IMG_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];
/// 专辑封面常用的文件名（不含扩展名，忽略大小写），越靠前越优先
const COVER_NAMES: &[&str] = &[
    "cover",
    "folder",
    "front",
    "album",
    "albumart",
    "jacket",
    "封面",
    "poster",
    "front cover",
    "albumartlarge",
];
/// 文件名里出现这些字的图片也当封面（back / 背面 / 歌词本内页除外）
const COVER_WORDS: &[&str] = &["cover", "front", "folder", "jacket", "封面", "正面"];
const NOT_COVER_WORDS: &[&str] = &["back", "背面", "inlay", "inside", "booklet", "disc", "cd ", "obi", "tray", "spine"];
/// 扫图目录：里面的 front / cover 才是封面
const SCAN_DIRS: &[&str] = &[
    "scans", "scan", "artwork", "artworks", "covers", "cover", "images", "image", "pics", "扫图", "封面",
];

fn images_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| IMG_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn stem_lower(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase()
}

/// 一个目录里最像专辑封面的图：固定名字 > 和目录同名 > 名字带 cover / front 的
fn cover_in(dir: &Path, imgs: &[PathBuf]) -> Option<PathBuf> {
    for n in COVER_NAMES {
        if let Some(p) = imgs.iter().find(|p| stem_lower(p) == *n) {
            return Some(p.clone());
        }
    }
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if let Some(p) = imgs.iter().find(|p| !dir_name.is_empty() && stem_lower(p) == dir_name) {
        return Some(p.clone());
    }
    imgs.iter()
        .find(|p| {
            let s = stem_lower(p);
            COVER_WORDS.iter().any(|w| s.contains(w)) && !NOT_COVER_WORDS.iter().any(|w| s.contains(w))
        })
        .cloned()
}

/// 碟片子目录：CD1、Disc 2、DISK1、碟1
pub fn is_disc_dir(name: &str) -> bool {
    let n: String = name
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_' && *c != '.')
        .collect();
    ["cd", "disc", "disk", "碟", "dvd"].iter().any(|p| {
        n.strip_prefix(p)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

/// 看起来像专辑封面的单张图片：接近正方形、不太小（只读文件头，不解码）
fn squareish(p: &Path) -> bool {
    image::image_dimensions(p).is_ok_and(|(w, h)| w >= 200 && h >= 200 && (w as f64 / h as f64) > 0.8 && (w as f64 / h as f64) < 1.25)
}

/// 本地封面图：和歌曲同名的图 > 歌曲目录里的封面 > 扫图目录里的封面 > 碟片目录上一层的封面 > 目录里唯一一张方图
pub fn folder_cover(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let imgs = images_in(dir);
    let stem = stem_lower(path);
    if let Some(p) = imgs.iter().find(|p| stem_lower(p) == stem) {
        return Some(p.clone());
    }
    let mut dirs = vec![(dir.to_path_buf(), imgs)];
    if dir.file_name().and_then(|n| n.to_str()).is_some_and(is_disc_dir) {
        if let Some(up) = dir.parent() {
            dirs.push((up.to_path_buf(), images_in(up)));
        }
    }
    for (d, imgs) in &dirs {
        if let Some(p) = cover_in(d, imgs) {
            return Some(p);
        }
        let subs: Vec<PathBuf> = std::fs::read_dir(d)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        for s in subs {
            let name = s.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            if SCAN_DIRS.contains(&name.as_str()) {
                let simgs = images_in(&s);
                if let Some(p) = cover_in(&s, &simgs) {
                    return Some(p);
                }
            }
        }
    }
    for (_, imgs) in &dirs {
        if imgs.len() == 1 && squareish(&imgs[0]) {
            return Some(imgs[0].clone());
        }
    }
    None
}

/// 缩放到 600px 存成 jpg，返回（文件名，主色色相）
pub fn save_cover(bytes: &[u8], dir: &Path) -> Option<(String, i64)> {
    let hash = blake3::hash(bytes).to_hex()[..20].to_string();
    let name = format!("{hash}.jpg");
    let out = dir.join(&name);
    let img = image::load_from_memory(bytes).ok()?;
    let hue = dominant_hue(&img);
    if !out.exists() {
        let small = img.thumbnail(600, 600).to_rgb8();
        small.save_with_format(&out, image::ImageFormat::Jpeg).ok()?;
    }
    Some((name, hue))
}

/// 取饱和度较高像素的平均色相，作为背景光晕的颜色
pub fn dominant_hue(img: &image::DynamicImage) -> i64 {
    let t = img.thumbnail(48, 48).to_rgb8();
    let (mut sx, mut sy, mut w) = (0f64, 0f64, 0f64);
    for p in t.pixels() {
        let [r, g, b] = p.0.map(|v| v as f64 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        if d < 0.08 || max < 0.15 {
            continue;
        }
        let h = if max == r {
            60.0 * (((g - b) / d) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        let s = d / max;
        let a = h.to_radians();
        sx += a.cos() * s;
        sy += a.sin() * s;
        w += s;
    }
    if w == 0.0 {
        return 220;
    }
    let h = sy.atan2(sx).to_degrees();
    ((h + 360.0) % 360.0).round() as i64
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
pub struct LyricLine {
    pub t: f64,
    pub text: String,
    /// 逐字（增强 LRC）：(开始时间, 字)
    pub words: Vec<(f64, String)>,
    /// 译文（在线歌词源提供时才有）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tr: Option<String>,
    /// 罗马音 / 音译
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roma: Option<String>,
}

fn parse_ts(s: &str) -> Option<f64> {
    let (m, rest) = s.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let sec: f64 = rest.trim().replace(':', ".").parse().ok()?;
    Some(m * 60.0 + sec)
}

pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut out = vec![];
    let mut offset = 0.0;
    for raw in text.lines() {
        let mut line = raw.trim();
        if let Some(v) = line.strip_prefix("[offset:") {
            offset = v.trim_end_matches(']').trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
            continue;
        }
        let mut stamps = vec![];
        while line.starts_with('[') {
            let Some(end) = line.find(']') else { break };
            match parse_ts(&line[1..end]) {
                Some(t) => stamps.push(t),
                None => break,
            }
            line = &line[end + 1..];
        }
        if stamps.is_empty() {
            continue;
        }
        // 增强 LRC：<00:12.34>字
        let mut words = vec![];
        let mut plain = String::new();
        if line.contains('<') {
            let mut rest = line;
            while let Some(s) = rest.find('<') {
                plain.push_str(&rest[..s]);
                if let Some(last) = words.last_mut() {
                    let (_, w): &mut (f64, String) = last;
                    w.push_str(&rest[..s]);
                }
                let Some(e) = rest[s..].find('>') else { break };
                if let Some(t) = parse_ts(&rest[s + 1..s + e]) {
                    words.push((t - offset, String::new()));
                }
                rest = &rest[s + e + 1..];
            }
            plain.push_str(rest);
            if let Some(last) = words.last_mut() {
                last.1.push_str(rest);
            }
            words.retain(|w| !w.1.is_empty());
        } else {
            plain = line.to_string();
        }
        for t in stamps {
            out.push(LyricLine {
                t: (t - offset).max(0.0),
                text: plain.trim().to_string(),
                words: words.clone(),
                ..Default::default()
            });
        }
    }
    // sort_by 是稳定排序：同一时间戳的行保持文件里的先后顺序
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    out.retain(|l| !l.text.is_empty());
    merge_translations(out)
}

/// 双语 LRC（网易云 / ESLyric / 163MusicLyrics 等导出的格式）：译文紧跟原文、时间戳相同。
/// 同一时间戳的第二行并入原文的 `tr`，第三行并入 `roma`，再多的丢掉。
/// 网易云用 “//” 占位表示这一行没有译文。
fn merge_translations(lines: Vec<LyricLine>) -> Vec<LyricLine> {
    let mut out: Vec<LyricLine> = Vec::with_capacity(lines.len());
    let mut extra = 0;
    for l in lines {
        if let Some(prev) = out.last_mut() {
            if prev.t >= 0.0 && (prev.t - l.t).abs() < 0.001 {
                extra += 1;
                let v = (l.text != "//").then_some(l.text);
                match extra {
                    1 => prev.tr = v,
                    2 => prev.roma = v,
                    _ => {}
                }
                continue;
            }
        }
        extra = 0;
        out.push(l);
    }
    out
}

fn read_text_any(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    Some(crate::reader::decode_text(&bytes).0)
}

/// 没有时间轴的纯文本歌词：t = -1
pub fn plain_lines(text: &str) -> Vec<LyricLine> {
    text.lines()
        .filter(|s| !s.trim().is_empty())
        .map(|s| LyricLine {
            t: -1.0,
            text: s.trim().into(),
            ..Default::default()
        })
        .collect()
}

/// 同名 .lrc 优先，其次文件内嵌歌词
pub fn lyrics_for(path: &Path) -> Vec<LyricLine> {
    let lrc = path.with_extension("lrc");
    if let Some(t) = read_text_any(&lrc) {
        let l = parse_lrc(&t);
        if !l.is_empty() {
            return l;
        }
    }
    if let Ok(tf) = lofty::read_from_path(path) {
        if let Some(tag) = tf.primary_tag().or_else(|| tf.first_tag()) {
            if let Some(t) = tag.get_string(ItemKey::Lyrics) {
                let l = parse_lrc(t);
                if !l.is_empty() {
                    return l;
                }
                // 没有时间轴的纯文本歌词
                return plain_lines(t);
            }
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(p: &Path, w: u32, h: u32) {
        image::RgbImage::new(w, h).save(p).unwrap();
    }

    #[test]
    fn disc_dirs() {
        assert!(is_disc_dir("CD1"));
        assert!(is_disc_dir("Disc 2"));
        assert!(is_disc_dir("disk-01"));
        assert!(!is_disc_dir("CDs"));
        assert!(!is_disc_dir("Discovery"));
    }

    #[test]
    fn finds_folder_covers() {
        let base = std::env::temp_dir().join(format!("mp_fc_{}", std::process::id()));
        let album = base.join("陈奕迅 - CHIN UP!");
        let cd1 = album.join("CD1");
        std::fs::create_dir_all(&cd1).unwrap();
        let song = cd1.join("01.flac");
        assert_eq!(folder_cover(&song), None);
        // 碟片目录上一层的扫图目录
        std::fs::create_dir_all(album.join("Scans")).unwrap();
        png(&album.join("Scans").join("Back.png"), 10, 10);
        assert_eq!(folder_cover(&song), None);
        png(&album.join("Scans").join("Front.png"), 10, 10);
        assert_eq!(folder_cover(&song), Some(album.join("Scans").join("Front.png")));
        // 名字带 cover 的图
        png(&album.join("Eason Chan - CHIN UP! (Cover).png"), 10, 10);
        assert_eq!(folder_cover(&song), Some(album.join("Eason Chan - CHIN UP! (Cover).png")));
        // 歌曲目录里的优先
        png(&cd1.join("folder.png"), 10, 10);
        assert_eq!(folder_cover(&song), Some(cd1.join("folder.png")));
        // 和歌曲同名的图最优先
        png(&cd1.join("01.png"), 10, 10);
        assert_eq!(folder_cover(&song), Some(cd1.join("01.png")));
        // 目录里只有一张不知名的图：方图才用
        let loose = base.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        png(&loose.join("IMG_1234.png"), 400, 200);
        assert_eq!(folder_cover(&loose.join("a.mp3")), None);
        png(&loose.join("IMG_1234.png"), 300, 300);
        assert_eq!(folder_cover(&loose.join("a.mp3")), Some(loose.join("IMG_1234.png")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn lrc_basic() {
        let l = parse_lrc("[ti:x]\n[00:01.50]第一句\n[00:03.00][00:10.00]重复\n");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].text, "第一句");
        assert!((l[0].t - 1.5).abs() < 1e-6);
        assert!((l[2].t - 10.0).abs() < 1e-6);
    }

    #[test]
    fn lrc_enhanced() {
        let l = parse_lrc("[00:01.00]<00:01.00>你<00:01.40>好<00:01.80>");
        assert_eq!(l[0].text, "你好");
        assert_eq!(l[0].words.len(), 2);
        assert_eq!(l[0].words[1].1, "好");
    }

    #[test]
    fn lrc_bilingual() {
        let l = parse_lrc(
            "[00:01.00]Hello\n[00:01.00]你好\n[00:03.00]こんにちは\n[00:03.00]你好\n[00:03.00]konnichiwa\n[00:05.00]Bye\n[00:05.00]//\n[00:07.00]End",
        );
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].tr.as_deref(), Some("你好"));
        assert_eq!(l[1].tr.as_deref(), Some("你好"));
        assert_eq!(l[1].roma.as_deref(), Some("konnichiwa"));
        assert_eq!(l[2].tr, None);
        assert_eq!(l[3].text, "End");
        assert_eq!(l[3].tr, None);
    }

    #[test]
    fn lrc_offset() {
        let l = parse_lrc("[offset:500]\n[00:02.00]a");
        assert!((l[0].t - 1.5).abs() < 1e-6);
    }
}
