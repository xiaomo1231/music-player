//! 标签、封面、歌词
use crate::parse;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use serde::Serialize;
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
            if let Some(pic) = tag
                .pictures()
                .iter()
                .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
                .or(tag.pictures().first())
            {
                if let Some((name, hue)) = save_cover(pic.data(), cover_dir) {
                    m.cover = Some(name);
                    m.hue = Some(hue);
                }
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

/// 同目录的 cover.jpg / folder.jpg 等
pub fn folder_cover(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    for n in ["cover", "folder", "front", "album", "poster", "Cover", "Folder", "Front"] {
        for e in ["jpg", "jpeg", "png", "webp"] {
            let p = dir.join(format!("{n}.{e}"));
            if p.exists() {
                return Some(p);
            }
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

#[derive(Debug, Serialize, Clone)]
pub struct LyricLine {
    pub t: f64,
    pub text: String,
    /// 逐字（增强 LRC）：(开始时间, 字)
    pub words: Vec<(f64, String)>,
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
            });
        }
    }
    out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    out.retain(|l| !l.text.is_empty());
    out
}

fn read_text_any(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    Some(crate::reader::decode_text(&bytes).0)
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
                return t
                    .lines()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| LyricLine {
                        t: -1.0,
                        text: s.trim().into(),
                        words: vec![],
                    })
                    .collect();
            }
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn lrc_offset() {
        let l = parse_lrc("[offset:500]\n[00:02.00]a");
        assert!((l[0].t - 1.5).abs() < 1e-6);
    }
}
