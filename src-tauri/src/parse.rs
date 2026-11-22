//! 文件分类与文件名解析（番剧 / 剧集 / 漫画卷号）
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

pub const AUDIO: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "alac", "wav", "ape", "wv", "ogg", "oga", "opus", "wma", "aif", "aiff", "dsf", "dff", "tta", "tak", "ac3", "dts", "mka",
];
pub const ENCRYPTED: &[&str] = &["qmc0", "qmc3", "qmcflac", "qmcogg", "mgg", "mgg1", "mflac", "mflac0", "kgm", "vpr", "ncm"];
pub const VIDEO: &[&str] = &[
    "mp4", "m4v", "mkv", "avi", "mov", "wmv", "flv", "webm", "ts", "m2ts", "mts", "rmvb", "rm", "3gp", "mpg", "mpeg", "vob", "ogv",
];
pub const NOVEL: &[&str] = &["txt", "epub"];
pub const COMIC: &[&str] = &["cbz", "zip", "pdf"];
pub const IMAGE: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif"];
pub const SUBS: &[&str] = &["srt", "ass", "ssa", "vtt", "sup", "idx", "sub"];

pub fn ext(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// 只按扩展名和路径判断类别；None 表示不是媒体文件
pub fn classify(p: &Path) -> Option<&'static str> {
    let e = ext(p);
    let e = e.as_str();
    if AUDIO.contains(&e) || ENCRYPTED.contains(&e) {
        return Some("music");
    }
    if VIDEO.contains(&e) {
        return Some(if looks_like_anime(p) { "anime" } else { "video" });
    }
    if NOVEL.contains(&e) {
        // 小 txt 多半是说明文件，不算小说
        if e == "txt" {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            if size < 20 * 1024 {
                return None;
            }
        }
        return Some("novel");
    }
    if COMIC.contains(&e) {
        if e == "zip" || e == "pdf" {
            // zip / pdf 只有在“漫画”类路径下才算漫画，避免把普通压缩包和文档拉进来
            let s = p.to_string_lossy().to_lowercase();
            if !(s.contains("漫画") || s.contains("comic") || s.contains("manga") || s.contains("漫畫")) {
                return None;
            }
        }
        return Some("manga");
    }
    None
}

pub fn is_encrypted(p: &Path) -> bool {
    ENCRYPTED.contains(&ext(p).as_str())
}

pub fn is_lossless(ext: &str) -> bool {
    matches!(ext, "flac" | "wav" | "ape" | "wv" | "aif" | "aiff" | "dsf" | "dff" | "tta" | "tak" | "alac")
}

static RE_BRACKET_GROUP: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*[\[【]([^\]】]+)[\]】]").unwrap());
static RE_SXXEXX: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\bS(\d{1,2})\s*E(\d{1,3}(?:\.\d)?)\b").unwrap());
static RE_SEASON: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(?:\bS(\d{1,2})\b|Season\s*(\d{1,2})|第([一二三四五六七八九十\d]{1,3})季)").unwrap());
static RE_EP_CN: Lazy<Regex> = Lazy::new(|| Regex::new(r"第\s*(\d{1,4})\s*[话話集]").unwrap());
static RE_EP_BRACKET: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\[【(（]\s*(\d{1,4}(?:\.\d)?)\s*(?:v\d)?\s*(?:END|完)?\s*[\]】)）]").unwrap());
static RE_EP_DASH: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s[-–]\s*(\d{1,4}(?:\.\d)?)(?:v\d)?(?:\s|$|\[|\()").unwrap());
static RE_EP_E: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(?:\bEP?|\.E)(\d{1,4})\b").unwrap());
static RE_RES: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(2160p|4K|1080p|720p|480p|1920x1080|1280x720|3840x2160)\b").unwrap());
static RE_TAGS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)[\[【(（][^\]】)）]*[\]】)）]|\b(?:x26[45]|h\.?26[45]|hevc|avc|av1|aac|flac|10bit|8bit|web-?dl|webrip|bdrip|bluray|hdr|dv|ma10p|hi10p|chs|cht|gb|big5|简体|繁体|简繁|内封|外挂)\b").unwrap()
});
static RE_SPECIAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(OVA|OAD|SP|Special|Specials|剧场版|劇場版|Movie|NCOP|NCED|OP|ED|PV|CM|Menu)\b").unwrap());

#[derive(Debug, Clone, Default, serde::Serialize, PartialEq)]
pub struct EpisodeInfo {
    pub title: String,
    pub season: Option<i64>,
    pub episode: Option<f64>,
    pub group: Option<String>,
    pub resolution: Option<String>,
    pub special: bool,
}

fn cn_num(s: &str) -> Option<i64> {
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    let d = |c| "零一二三四五六七八九".chars().position(|x| x == c).map(|v| v as i64);
    let cs: Vec<char> = s.chars().collect();
    match cs.as_slice() {
        ['十'] => Some(10),
        ['十', b] => Some(10 + d(*b)?),
        [a, '十'] => Some(d(*a)? * 10),
        [a, '十', b] => Some(d(*a)? * 10 + d(*b)?),
        [a] => d(*a),
        _ => None,
    }
}

fn looks_like_anime(p: &Path) -> bool {
    let s = p.to_string_lossy();
    let lower = s.to_lowercase();
    if lower.contains("番剧") || lower.contains("动漫") || lower.contains("動畫") || lower.contains("anime") || lower.contains("bangumi") {
        return true;
    }
    let name = p.file_stem().and_then(|n| n.to_str()).unwrap_or("");
    // [字幕组] 名字 - 03 [1080p] 这种发布命名几乎都是番剧
    RE_BRACKET_GROUP.is_match(name) && (RE_EP_DASH.is_match(name) || RE_EP_BRACKET.is_match(name) || RE_EP_CN.is_match(name))
}

/// 解析发布名，例如 `[字幕组] 番剧名 S02 - 07 [1080p].mkv`
pub fn parse_episode(path: &Path) -> EpisodeInfo {
    let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("").replace('_', " ");
    let mut info = EpisodeInfo::default();
    let mut rest = stem.clone();
    if let Some(c) = RE_BRACKET_GROUP.captures(&stem) {
        let g = c[1].trim().to_string();
        // [1080p] 之类不是字幕组
        if !RE_RES.is_match(&g) && g.parse::<f64>().is_err() {
            info.group = Some(g);
            rest = stem[c.get(0).unwrap().end()..].to_string();
        }
    }
    info.resolution = RE_RES.captures(&stem).map(|c| c[1].to_string());
    info.special = RE_SPECIAL.is_match(&rest);
    let mut cut = rest.len();
    if let Some(c) = RE_SXXEXX.captures(&rest) {
        info.season = c[1].parse().ok();
        info.episode = c[2].parse().ok();
        cut = cut.min(c.get(0).unwrap().start());
    }
    if info.episode.is_none() {
        for re in [&*RE_EP_CN, &*RE_EP_DASH, &*RE_EP_BRACKET, &*RE_EP_E] {
            if let Some(c) = re.captures(&rest) {
                let v: Option<f64> = c[1].parse().ok();
                // 排除年份和分辨率被当成集数
                if let Some(n) = v {
                    if (1900.0..=2100.0).contains(&n) {
                        continue;
                    }
                }
                info.episode = v;
                cut = cut.min(c.get(0).unwrap().start());
                break;
            }
        }
    }
    if let Some(c) = RE_SEASON.captures(&rest) {
        let s = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
        if info.season.is_none() {
            info.season = cn_num(s);
        }
        cut = cut.min(c.get(0).unwrap().start());
    }
    let mut title = rest[..cut].to_string();
    title = RE_TAGS.replace_all(&title, " ").to_string();
    title = RE_SPECIAL.replace_all(&title, " ").to_string();
    let title = title
        .trim_matches(|c: char| c.is_whitespace() || "-–_.·|".contains(c))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    info.title = if title.is_empty() {
        // 文件名里没有名字时用所在文件夹名（去掉 Season 字样）
        let parent = path.parent().and_then(|p| {
            let n = p.file_name()?.to_str()?;
            if RE_SEASON.is_match(n) && n.len() < 16 {
                p.parent()?.file_name()?.to_str()
            } else {
                Some(n)
            }
        });
        let t = parent.unwrap_or(&stem);
        let t = RE_TAGS.replace_all(t, " ");
        let t = RE_SEASON.replace_all(&t, " ");
        t.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        title
    };
    if info.season.is_none() && info.episode.is_some() {
        // 文件夹叫 Season 2 / 第二季 的时候
        if let Some(dir) = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
            if let Some(c) = RE_SEASON.captures(dir) {
                let s = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
                info.season = cn_num(s);
            }
        }
    }
    if info.special {
        info.season = Some(0);
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn fansub_dash() {
        let i = parse_episode(&p(r"E:\Anime\[XX字幕组] Sousou no Frieren - 03 [1080p][HEVC].mkv"));
        assert_eq!(i.title, "Sousou no Frieren");
        assert_eq!(i.episode, Some(3.0));
        assert_eq!(i.group.as_deref(), Some("XX字幕组"));
        assert_eq!(i.resolution.as_deref(), Some("1080p"));
    }

    #[test]
    fn season_and_dash() {
        let i = parse_episode(&p(r"[字幕组] 番剧名 S02 - 07 [1080p].mkv"));
        assert_eq!(i.title, "番剧名");
        assert_eq!(i.season, Some(2));
        assert_eq!(i.episode, Some(7.0));
    }

    #[test]
    fn sxxexx() {
        let i = parse_episode(&p(r"D:\TV\Some.Show.S01E05.1080p.WEB-DL.mkv"));
        assert_eq!(i.season, Some(1));
        assert_eq!(i.episode, Some(5.0));
        assert_eq!(i.title, "Some.Show");
    }

    #[test]
    fn chinese_episode_and_folder_season() {
        let i = parse_episode(&p(r"F:\Anime\放学后的天台气象站\第二季\放学后的天台气象站 第07话.mp4"));
        assert_eq!(i.title, "放学后的天台气象站");
        assert_eq!(i.season, Some(2));
        assert_eq!(i.episode, Some(7.0));
    }

    #[test]
    fn bracket_episode() {
        let i = parse_episode(&p(r"[Group] 机巧少女与旧书店 [12][1080p].mkv"));
        assert_eq!(i.episode, Some(12.0));
        assert_eq!(i.title, "机巧少女与旧书店");
    }

    #[test]
    fn year_not_episode() {
        let i = parse_episode(&p(r"D:\Movies\潮汐之间 (2024) 2160p.mkv"));
        assert_eq!(i.episode, None);
    }

    #[test]
    fn classify_kinds() {
        assert_eq!(classify(&p("a.flac")), Some("music"));
        assert_eq!(classify(&p("a.mflac")), Some("music"));
        assert_eq!(classify(&p(r"E:\Downloads\Anime\x - 01.mkv")), Some("anime"));
        assert_eq!(classify(&p(r"D:\Movies\x.mkv")), Some("video"));
        assert_eq!(classify(&p(r"D:\漫画\x.cbz")), Some("manga"));
        assert_eq!(classify(&p(r"D:\docs\x.zip")), None);
    }
}
