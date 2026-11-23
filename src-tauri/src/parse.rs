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
        // 碟片菜单之类的垃圾视频返回 None，不入库
        return classify_video(p).map(|c| c.kind);
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
            // zip / pdf 只有在“漫画”类路径下、或文件名像漫画章节 / 卷时才算漫画，避免把普通压缩包和文档拉进来
            let s = p.to_string_lossy().to_lowercase();
            let (dirs, stem) = split_path(p);
            let comic_path = s.contains("漫画") || s.contains("comic") || s.contains("manga") || s.contains("漫畫");
            // 放在“番”这类动漫文件夹里的 pdf / zip 基本是漫画（如 F:\番\某作品\1 第01話.pdf）
            let anime_dir = dirs.iter().any(|d| ANIME_DIRS.contains(&d.trim().to_lowercase().as_str()));
            if !(comic_path || anime_dir || RE_COMIC_CHAPTER.is_match(&stem)) {
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

/// 漫画章节 / 卷的文件名：第01話、第3卷、Vol.2、Ch.15、番外
static RE_COMIC_CHAPTER: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)第\s*\d+(?:\.\d+)?\s*[话話回卷册冊]|番外|\bvol\.?\s*\d|\bch(?:apter)?\.?\s*\d").unwrap());
static RE_SPECIAL_CJK: Lazy<Regex> = Lazy::new(|| Regex::new(r"剧场版|劇場版|剧场|劇場|总集篇|總集篇|総集編|剧版|劇版|特典|特别篇|特別篇|番外").unwrap());
static RE_EP_ONLY: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(\d{1,3}(?:\.\d)?)\s*$").unwrap());
static RE_EP_LEAD: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*[\[【](\d{1,3})(?:[^\d\]】.p][^\]】]*)?[\]】]").unwrap());
static RE_PART: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[（(【\[]\s*([上中下])\s*[)）】\]]|([上中下])(?:篇|集|部|卷|編|编)|(?:剧版|劇版|剧场版|劇場版)\s*([上中下])|([前後后])[編编篇]").unwrap()
});
static RE_YEAR_SEP: Lazy<Regex> = Lazy::new(|| Regex::new(r"[.\s(（\[【](?:19|20)\d{2}(?:[.\s)）\]】]|$)").unwrap());
static RE_YEAR_ANY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(^|\D)(?:19|20)\d{2}(\D|$)").unwrap());
static RE_FANSUB: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)-raws\b|\braws\b|字幕[组組社]|喵萌|lolihouse|nekomoe|vcb-?studio|sakurato|kitaujisub|dbd-raws|dmhy").unwrap());
static RE_QUALITY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)blu-?ray|bdrip|remux|web-?dl|webrip|hdtv|2160p|1080p|720p|\b4k\b|\buhd\b|x26[45]|h\.?26[45]|hevc").unwrap());
static RE_SEG: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\[【]([^\]】]*)[\]】]").unwrap());
static RE_PAREN: Lazy<Regex> = Lazy::new(|| Regex::new(r"[(（]([^)）]*)[)）]").unwrap());
static RE_ORDER_PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\d{1,2}[.、]\s*").unwrap());
static RE_COPY_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*[(（]\d{1,2}[)）]\s*$").unwrap());
static RE_TAG_WORDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:x26[45]|h\.?26[45]|hevc|avc|av1|aac|e?ac-?3|dts|flac|\d{1,2}-?bit|web-?dl|webrip|web|bdrip|bd|bluray|blu-ray|remux|hdr10?|hdr|sdr|dv|ma10p|hi10p|chs|cht|jpn?|gb|big5|ass|srt|mkv|mp4|\d{3,4}[pi]|4k|8k|uhd|fhd|hd|tv|ova|oad|tvrip|dvdrip|v\d|\d+(?:\.\d+)?\s*[gmt]i?b?|\d{1,4}(?:\s*[-~]\s*\d{1,4})?)\b|中英双字|中英字幕|中日双字|中日双语|简繁日|简繁|简日|繁日|简体|繁体|简中|繁中|双语|雙語|中字|字幕|内封|內封|外挂|外掛|内嵌|全集|合集|全\d+[集话話]|高清|超清|蓝光|藍光|修复|修復|国语|國語|粤语|日语|中配|无删减|完结|完結|版本|收藏版|上\s*[+＋&]\s*下",
    )
    .unwrap()
});
static RE_GROUP_WORD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)raws?\b|sub(?:s|team)?\b|字幕|[组組社]$|production|house|studio|team|&|＆|喵萌|nekomoe|vcb").unwrap());

/// 文件夹名直接等于这些时，说明是“番剧库”的根目录
const ANIME_DIRS: &[&str] = &[
    "番",
    "新番",
    "旧番",
    "老番",
    "补番",
    "番剧",
    "番劇",
    "番组",
    "番組",
    "动漫",
    "動漫",
    "动画",
    "動畫",
    "アニメ",
    "anime",
    "animes",
    "animation",
    "bangumi",
    "二次元",
];
const MOVIE_DIRS: &[&str] = &["电影", "電影", "影片", "movie", "movies", "film", "films"];
const LIB_DIRS: &[&str] = &[
    "电视剧",
    "電視劇",
    "剧集",
    "劇集",
    "连续剧",
    "美剧",
    "日剧",
    "韩剧",
    "英剧",
    "tv",
    "tv shows",
    "tvshows",
    "shows",
    "series",
    "downloads",
    "download",
    "下载",
    "下載",
    "desktop",
    "桌面",
    "videos",
    "video",
    "视频",
    "視頻",
    "影音",
    "影视",
    "影視",
    "media",
    "媒体",
    "temp",
    "tmp",
    "新建文件夹",
    "我的视频",
    "users",
    "home",
    "public",
    "onedrive",
];
/// 名字里带这些字的文件夹是分类目录（如“2024新番”“欧美电影”），不是作品名
const LIB_DIR_WORDS: &[&str] = &[
    "新番",
    "番剧",
    "番劇",
    "动漫",
    "動漫",
    "动画",
    "動畫",
    "anime",
    "bangumi",
    "电影",
    "電影",
    "movies",
    "电视剧",
    "電視劇",
    "剧集",
    "劇集",
    "美剧",
    "日剧",
    "韩剧",
];
/// 番剧文件夹里放特典 / 附带内容的子文件夹
const EXTRA_DIRS: &[&str] = &[
    "sp",
    "sps",
    "special",
    "specials",
    "extra",
    "extras",
    "bonus",
    "特典",
    "映像特典",
    "花絮",
    "番外",
    "ncop",
    "nced",
    "ncop&nced",
    "nc",
    "cm",
    "pv",
    "cds",
    "cd",
    "scans",
    "fonts",
    "字幕",
    "字幕文件",
    "subs",
    "subtitles",
    "featurettes",
    "剧场版",
    "劇場版",
    "ova",
    "oad",
    "ova&oad",
];

/// 路径拆成“从近到远的各级文件夹名”和“不带扩展名的文件名”。
/// 自己按 / 和 \ 拆，测试在非 Windows 上也能用 Windows 路径
fn split_path(p: &Path) -> (Vec<String>, String) {
    let s = p.to_string_lossy();
    let mut parts: Vec<String> = s.split(['/', '\\']).filter(|x| !x.is_empty()).map(String::from).collect();
    let name = parts.pop().unwrap_or_default();
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name,
    };
    parts.reverse();
    (parts, stem)
}

/// 分类目录或盘符：往上找作品名时到这里停
fn is_lib_dir(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    n.ends_with(':')
        || ANIME_DIRS.contains(&n.as_str())
        || MOVIE_DIRS.contains(&n.as_str())
        || LIB_DIRS.contains(&n.as_str())
        || LIB_DIR_WORDS.iter().any(|w| n.contains(w))
}

fn is_movie_dir(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    MOVIE_DIRS.contains(&n.as_str()) || n.contains("电影") || n.contains("電影")
}

/// 只剩标签（分辨率、编码、字幕说明、集数范围、体积……）
fn is_tag(seg: &str) -> bool {
    let sep = sep_tags(seg);
    let t = RE_TAG_WORDS.replace_all(&sep, " ");
    let t = RE_YEAR_ANY.replace_all(&t, "$1 $2");
    !t.chars().any(|c| c.is_alphabetic() && !"上中下+＋".contains(c))
}

/// “4K修复”“1080P中字”：英文标签和汉字连在一起时 \b 不生效，先隔开
fn sep_tags(s: &str) -> String {
    static A: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(\d{3,4}[pi]|[48]k|hdr|uhd|bd|web)([^\x00-\x7f])").unwrap());
    static B: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)([^\x00-\x7f])(\d{3,4}[pi]|[48]k|hdr|uhd)\b").unwrap());
    let s = A.replace_all(s, "$1 $2");
    B.replace_all(&s, "$1 $2").to_string()
}

fn is_group(seg: &str) -> bool {
    RE_GROUP_WORD.is_match(seg.trim())
}

fn tidy(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c.is_whitespace() || "-–_.·|,，、+".contains(c))
        .to_string()
}

/// 从文件夹名 / 发布名里取出作品名，取不到时返回空串。
/// `0.孤独摇滚（上+下）2024【剧场总集篇】中字` → `孤独摇滚`；
/// `[DBD-Raws][BanG Dream! It's MyGO!!!!!][01-13TV全集][1080P]` → `BanG Dream! It's MyGO!!!!!`
pub fn clean_title(name: &str) -> String {
    let s = RE_ORDER_PREFIX.replace(name, "");
    let s = RE_COPY_SUFFIX.replace(&s, "");
    // 方括号外面的文字；圆括号只去掉标签（上+下、年份），保留“（哭泣少女乐队）”这种别名
    let outside = RE_SEG.replace_all(&s, " ");
    let outside = RE_PAREN.replace_all(&outside, |c: &regex::Captures| if is_tag(&c[1]) { " ".to_string() } else { c[0].to_string() });
    let outside = sep_tags(&outside);
    let outside = RE_TAG_WORDS.replace_all(&outside, " ");
    let outside = RE_YEAR_ANY.replace_all(&outside, "$1 $2");
    let outside = tidy(&outside);
    if outside.chars().any(|c| c.is_alphabetic()) {
        return outside;
    }
    // 名字全在方括号里：跳过开头的字幕组和各种标签
    let segs: Vec<String> = RE_SEG
        .captures_iter(&s)
        .map(|c| c[1].trim().to_string())
        .filter(|x| !x.is_empty() && !is_tag(x))
        .collect();
    let pick = segs.iter().find(|x| !is_group(x));
    match pick {
        Some(x) => tidy(&RE_TAG_WORDS.replace_all(&sep_tags(x), " ")),
        None => String::new(),
    }
}

#[derive(Debug, Default)]
struct WorkDir {
    title: Option<String>,
    /// 在 SPs / NCOP&NCED / 特典 这类子文件夹里
    special: bool,
    season: Option<i64>,
}

/// 从近到远找第一个像作品名的文件夹：跳过 Season 1、4K修复、[喵萌Production][1080p] 这类，
/// 碰到“番”“电影”这类分类目录或盘符就停
fn work_title(dirs: &[String]) -> WorkDir {
    let mut w = WorkDir::default();
    for d in dirs.iter().take(6) {
        if is_lib_dir(d) {
            break;
        }
        let low = d.trim().to_lowercase();
        if EXTRA_DIRS.contains(&low.as_str()) {
            w.special = true;
            continue;
        }
        let mut name = d.clone();
        if let Some(c) = RE_SEASON.captures(d) {
            let s = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
            if w.season.is_none() {
                w.season = cn_num(s);
            }
            name = RE_SEASON.replace_all(d, " ").to_string();
        }
        let t = clean_title(&name);
        if !t.is_empty() {
            w.title = Some(t);
            break;
        }
    }
    w
}

/// 碟片菜单（BD menu）：几百 KB 的循环画面，不当成一集
fn is_menu(dirs: &[String], stem: &str) -> bool {
    static RE_MENU: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(?:^|[\[\s_.\-])(?:bd)?menus?(?:[\]\s_.\-]|$)").unwrap());
    dirs.first()
        .map(|d| matches!(d.trim().to_lowercase().as_str(), "menu" | "menus" | "bdmenu"))
        .unwrap_or(false)
        || RE_MENU.is_match(stem)
}

/// 用来判断两个作品名是不是同一部：只比字母和数字（“孤独摇滚！”和“孤独摇滚”算同一部）
pub fn title_key(t: &str) -> String {
    t.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// 视频的归类结果
#[derive(Debug, Clone, PartialEq)]
pub struct VideoClass {
    /// anime / video
    pub kind: &'static str,
    /// 归到番剧 / 电视剧里（info.title 是整部的名字）
    pub series: bool,
    pub movie: bool,
    /// 电影是按文件名猜的（没放在“电影”文件夹里），时长太短时要撤销
    pub movie_guess: bool,
    pub info: EpisodeInfo,
}

/// 归类规则改了就加一，已经入库的视频启动时会按新规则重新归类
pub const VIDEO_RULES: i64 = 1;

/// 视频归类；None 表示不该入库（碟片菜单）
pub fn classify_video(p: &Path) -> Option<VideoClass> {
    let (dirs, stem) = split_path(p);
    if is_menu(&dirs, &stem) {
        return None;
    }
    let kind = if looks_like_anime(p, &dirs, &stem) { "anime" } else { "video" };
    Some(video_class_as(p, kind))
}

/// 按指定的库（anime / video）归类，用于下载时用户自己选了存到哪个库
pub fn video_class_as(p: &Path, kind: &str) -> VideoClass {
    let (dirs, stem) = split_path(p);
    let mut info = parse_episode(p);
    let wd = work_title(&dirs);
    if wd.special && !info.special {
        info.special = true;
        info.season = Some(0);
    }
    if info.season.is_none() && info.episode.is_some() {
        info.season = wd.season;
    }
    if kind == "anime" {
        // 番剧以作品文件夹为准：同一文件夹里的正片、剧场版、特典归到一部
        if let Some(t) = wd.title {
            info.title = t;
        }
        return VideoClass {
            kind: "anime",
            series: true,
            movie: false,
            movie_guess: false,
            info,
        };
    }
    let movie_dir = dirs.iter().any(|d| is_movie_dir(d));
    let movie_name = info.episode.is_none() && !info.special && RE_YEAR_SEP.is_match(&stem) && RE_QUALITY.is_match(&stem);
    if movie_dir || movie_name {
        if info.episode.is_some() {
            // 电影的上 / 下部：保留文件名
            info.title = tidy(&RE_TAGS.replace_all(&stem, " "));
        } else if let Some(t) = wd.title {
            info.title = t;
        } else if !info.title.contains(' ') && info.title.contains('.') {
            info.title = info.title.replace('.', " ");
        }
        return VideoClass {
            kind: "video",
            series: false,
            movie: true,
            movie_guess: !movie_dir,
            info,
        };
    }
    let series = info.episode.is_some();
    if series {
        if let Some(t) = wd.title {
            info.title = t;
        }
    }
    VideoClass {
        kind: "video",
        series,
        movie: false,
        movie_guess: false,
        info,
    }
}

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

fn looks_like_anime(p: &Path, dirs: &[String], stem: &str) -> bool {
    let lower = p.to_string_lossy().to_lowercase();
    if ["番剧", "番劇", "动漫", "動漫", "动画", "動畫", "anime", "bangumi", "アニメ"]
        .iter()
        .any(|k| lower.contains(k))
    {
        return true;
    }
    // 文件夹叫“番”“新番”之类
    if dirs.iter().any(|d| ANIME_DIRS.contains(&d.trim().to_lowercase().as_str())) {
        return true;
    }
    // 动漫压制组 / 字幕组的发布名（文件名或所在文件夹）
    if RE_FANSUB.is_match(stem) || dirs.iter().take(4).any(|d| RE_FANSUB.is_match(d)) {
        return true;
    }
    // [字幕组] 名字 - 03 [1080p] 这种发布命名几乎都是番剧
    RE_BRACKET_GROUP.is_match(stem) && (RE_EP_DASH.is_match(stem) || RE_EP_BRACKET.is_match(stem) || RE_EP_CN.is_match(stem))
}

/// 解析发布名，例如 `[字幕组] 番剧名 S02 - 07 [1080p].mkv`
pub fn parse_episode(path: &Path) -> EpisodeInfo {
    let (dirs, stem) = split_path(path);
    let stem = stem.replace('_', " ");
    let mut info = EpisodeInfo::default();
    let mut rest = stem.clone();
    // 纯数字文件名：01.mp4、1.mp4
    if let Some(c) = RE_EP_ONLY.captures(&stem) {
        info.episode = c[1].parse().ok();
    }
    // [01東京ワッショイ][...]：开头方括号里的集号 + 集名
    if let Some(c) = RE_EP_LEAD.captures(&stem) {
        info.episode = c[1].parse().ok();
        rest = stem[c.get(0).unwrap().end()..].to_string();
    } else if let Some(c) = RE_BRACKET_GROUP.captures(&stem) {
        let g = c[1].trim().to_string();
        // [1080p] 之类不是字幕组
        if !RE_RES.is_match(&g) && g.parse::<f64>().is_err() {
            info.group = Some(g);
            rest = stem[c.get(0).unwrap().end()..].to_string();
        }
    }
    info.resolution = RE_RES.captures(&stem).map(|c| c[1].to_string());
    info.special = RE_SPECIAL.is_match(&rest) || RE_SPECIAL_CJK.is_match(&rest);
    let mut cut = rest.len();
    if info.episode.is_some() {
        cut = 0;
    }
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
    // 上 / 下、前編 / 後編：剧场版常见的分部
    if info.episode.is_none() {
        if let Some(c) = RE_PART.captures(&rest) {
            let m = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
            info.episode = match m {
                "上" | "前" => Some(1.0),
                "中" => Some(1.5),
                _ => Some(2.0),
            };
        }
    }
    if let Some(c) = RE_SEASON.captures(&rest) {
        let s = c.get(1).or(c.get(2)).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
        if info.season.is_none() {
            info.season = cn_num(s);
        }
        cut = cut.min(c.get(0).unwrap().start());
    }
    // Forrest.Gump.1994.Bluray / 潮汐之间 (2024)：年份之后都是版本信息
    if let Some(m) = RE_YEAR_SEP.find(&rest) {
        if m.start() > 0 {
            cut = cut.min(m.start());
        }
    }
    let mut title = rest[..cut].to_string();
    title = RE_TAGS.replace_all(&title, " ").to_string();
    title = RE_SPECIAL.replace_all(&title, " ").to_string();
    title = RE_SPECIAL_CJK.replace_all(&title, " ").to_string();
    let title = title
        .trim_matches(|c: char| c.is_whitespace() || "-–_.·|".contains(c))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = if title.chars().any(|c| c.is_alphabetic()) { title } else { String::new() };
    info.title = if !title.is_empty() {
        title
    } else if !clean_title(&rest).is_empty() {
        // [BanG Dream! It's MyGO!!!!!][NCOP][1080P] 这种全是方括号的名字
        clean_title(&rest)
    } else {
        // 文件名里没有名字时用所在文件夹名（跳过 Season、4K 这类文件夹）
        work_title(&dirs).title.unwrap_or_else(|| {
            let t = RE_TAGS.replace_all(&stem, " ");
            t.split_whitespace().collect::<Vec<_>>().join(" ")
        })
    };
    if info.season.is_none() && info.episode.is_some() {
        // 文件夹叫 Season 2 / 第二季 的时候
        if let Some(dir) = dirs.first() {
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
        assert_eq!(classify(&p(r"D:\docs\报告.pdf")), None);
        // 用户的 F:\番\廣井菊里的深酒日記：章节 pdf
        assert_eq!(classify(&p(r"F:\番\廣井菊里的深酒日記\1 第01話.pdf")), Some("manga"));
        assert_eq!(classify(&p(r"F:\番\廣井菊里的深酒日記\32.5 2025新年賀圖.pdf")), Some("manga"));
        assert_eq!(classify(&p(r"D:\下载\某漫画\第12話.pdf")), Some("manga"));
        assert_eq!(classify(&p(r"D:\下载\Some Manga Vol.3.zip")), Some("manga"));
    }

    fn vc(s: &str) -> VideoClass {
        classify_video(&p(s)).unwrap()
    }

    // 下面的路径照搬用户真实的 F:\番 和 F:\电影 目录结构
    #[test]
    fn real_anime_folders() {
        // 纯数字文件名，作品名在上一级文件夹
        let v = vc(r"F:\番\孤独摇滚！\08.mp4");
        assert_eq!(
            (v.kind, v.series, v.info.title.as_str(), v.info.episode, v.info.season),
            ("anime", true, "孤独摇滚！", Some(8.0), None)
        );
        // 剧场总集篇：作品名和 TV 版对得上，算特别篇，上 / 下是第 1 / 2 部
        let v = vc(r"F:\番\0.孤独摇滚（上+下）2024【剧场总集篇】中字\孤独摇滚（下）劇場総集編.2024.HD1080P（中字）.mp4");
        assert_eq!(v.kind, "anime");
        assert_eq!(v.info.title, "孤独摇滚");
        assert_eq!(title_key(&v.info.title), title_key("孤独摇滚！"));
        assert!(v.info.special);
        assert_eq!((v.info.season, v.info.episode), (Some(0), Some(2.0)));
        // 跳过 [喵萌Production][1080p][简日双语] 这层
        let v = vc(r"F:\番\BanG Dream! Ave Mujica\[喵萌Production][1080p][简日双语]\10.mp4");
        assert_eq!((v.info.title.as_str(), v.info.episode), ("BanG Dream! Ave Mujica", Some(10.0)));
        // Season 1 + 全是方括号的文件夹名
        let mygo = r"F:\番\[DBD-Raws][BanG Dream! It's MyGO!!!!!][01-13TV全集][1080P][BDRip][HEVC-10bit][简繁日双语外挂][FLAC][MKV](1)";
        let v = vc(&format!(r"{mygo}\Season 1\S01E03.mkv"));
        assert_eq!(
            (v.kind, v.info.title.as_str(), v.info.season, v.info.episode),
            ("anime", "BanG Dream! It's MyGO!!!!!", Some(1), Some(3.0))
        );
        // 无字幕 OP / ED 归到同一部的特别篇
        let v = vc(&format!(
            r"{mygo}\NCOP&NCED\[DBD-Raws][BanG Dream! It's MyGO!!!!!][NCOP][1080P][BDRip][HEVC-10bit][FLAC].mkv"
        ));
        assert_eq!((v.info.title.as_str(), v.info.season), ("BanG Dream! It's MyGO!!!!!", Some(0)));
        // 碟片菜单不入库
        assert!(classify_video(&p(&format!(
            r"{mygo}\menu\[DBD-Raws][BanG Dream! It's MyGO!!!!!][menu][01][1080P][BDRip][HEVC-10bit][FLAC].mkv"
        )))
        .is_none());
        assert_eq!(
            classify(&p(&format!(
                r"{mygo}\menu\[DBD-Raws][BanG Dream! It's MyGO!!!!!][menu][02][1080P][BDRip][HEVC-10bit][FLAC].mkv"
            ))),
            None
        );
        // [01集名][...] 开头的集号；4K修复 这层跳过；剧版上 / 下 算特别篇
        let gbc = r"F:\番\girls band cry（哭泣少女乐队）\4K修复";
        let v = vc(&format!(
            r"{gbc}\[01東京ワッショイ][B站沸羊羊都得叫我师傅][ガールズバンドクライ][2160P][喵萌Production&LoliHouse][4K修复].mkv"
        ));
        assert_eq!(
            (v.info.title.as_str(), v.info.episode, v.info.special),
            ("girls band cry（哭泣少女乐队）", Some(1.0), false)
        );
        let v = vc(&format!(r"{gbc}\剧版下.mp4"));
        assert_eq!(
            (v.info.title.as_str(), v.info.season, v.info.episode),
            ("girls band cry（哭泣少女乐队）", Some(0), Some(2.0))
        );
        // 动画电影：单独一部，用文件夹名
        let v = vc(r"F:\番\辉夜姬\4K\[MAI] Chou Kaguya-hime! [WebRip 2160p HEVC-10bit EAC3 ASS].mkv");
        assert_eq!((v.kind, v.series, v.info.title.as_str()), ("anime", true, "辉夜姬"));
    }

    #[test]
    fn real_movie_folders() {
        let v = vc(
            r"F:\电影\[阿甘正传][1994][中英双字][4K HDR][29.6G]\[阿甘正传][1994][中英双字][4K HDR][29.6G]\Forrest.Gump.1994.Bluray.2160p.x265.10bit.HDR.2Audio-MiniHD.mkv",
        );
        assert_eq!(
            (v.kind, v.series, v.movie, v.movie_guess, v.info.title.as_str()),
            ("video", false, true, false, "阿甘正传")
        );
        // 不在“电影”文件夹里，按发布名猜
        let v = vc(r"D:\Downloads\Forrest.Gump.1994.Bluray.2160p.x265.mkv");
        assert_eq!((v.movie, v.movie_guess, v.info.title.as_str()), (true, true, "Forrest Gump"));
        let v = vc(r"D:\Movies\潮汐之间 (2024) 2160p.mkv");
        assert_eq!((v.movie, v.info.title.as_str()), (true, "潮汐之间"));
        // 普通视频不算电影
        let v = vc(r"D:\Videos\录屏 2024-05-01.mp4");
        assert!(!v.movie && !v.series);
    }

    #[test]
    fn clean_titles() {
        assert_eq!(clean_title("[阿甘正传][1994][中英双字][4K HDR][29.6G]"), "阿甘正传");
        assert_eq!(clean_title("[喵萌Production][1080p][简日双语]"), "");
        assert_eq!(clean_title("4K修复"), "");
        assert_eq!(clean_title("0.孤独摇滚（上+下）2024【剧场总集篇】中字"), "孤独摇滚");
        assert_eq!(clean_title("girls band cry（哭泣少女乐队）"), "girls band cry（哭泣少女乐队）");
    }

    #[test]
    fn folder_title_beats_misc_dirs() {
        // 分类目录不当作品名
        let v = vc(r"D:\Downloads\[XX字幕组] Sousou no Frieren - 03 [1080p].mkv");
        assert_eq!((v.kind, v.info.title.as_str()), ("anime", "Sousou no Frieren"));
        let v = vc(r"E:\2024新番\[XX字幕组] Sousou no Frieren - 03 [1080p].mkv");
        assert_eq!(v.info.title, "Sousou no Frieren");
        let v = vc(r"D:\TV\Some.Show.S01E05.1080p.WEB-DL.mkv");
        assert_eq!((v.kind, v.series, v.info.title.as_str()), ("video", true, "Some.Show"));
    }

    fn vc(s: &str) -> VideoClass {
        classify_video(&p(s)).unwrap()
    }

    // 下面的路径照搬用户真实的 F:\番 和 F:\电影 目录结构
    #[test]
    fn real_anime_folders() {
        // 纯数字文件名，作品名在上一级文件夹
        let v = vc(r"F:\番\孤独摇滚！\08.mp4");
        assert_eq!(
            (v.kind, v.series, v.info.title.as_str(), v.info.episode, v.info.season),
            ("anime", true, "孤独摇滚！", Some(8.0), None)
        );
        // 剧场总集篇：作品名和 TV 版对得上，算特别篇，上 / 下是第 1 / 2 部
        let v = vc(r"F:\番\0.孤独摇滚（上+下）2024【剧场总集篇】中字\孤独摇滚（下）劇場総集編.2024.HD1080P（中字）.mp4");
        assert_eq!(v.kind, "anime");
        assert_eq!(v.info.title, "孤独摇滚");
        assert_eq!(title_key(&v.info.title), title_key("孤独摇滚！"));
        assert!(v.info.special);
        assert_eq!((v.info.season, v.info.episode), (Some(0), Some(2.0)));
        // 跳过 [喵萌Production][1080p][简日双语] 这层
        let v = vc(r"F:\番\BanG Dream! Ave Mujica\[喵萌Production][1080p][简日双语]\10.mp4");
        assert_eq!((v.info.title.as_str(), v.info.episode), ("BanG Dream! Ave Mujica", Some(10.0)));
        // Season 1 + 全是方括号的文件夹名
        let mygo = r"F:\番\[DBD-Raws][BanG Dream! It's MyGO!!!!!][01-13TV全集][1080P][BDRip][HEVC-10bit][简繁日双语外挂][FLAC][MKV](1)";
        let v = vc(&format!(r"{mygo}\Season 1\S01E03.mkv"));
        assert_eq!(
            (v.kind, v.info.title.as_str(), v.info.season, v.info.episode),
            ("anime", "BanG Dream! It's MyGO!!!!!", Some(1), Some(3.0))
        );
        // 无字幕 OP / ED 归到同一部的特别篇
        let v = vc(&format!(
            r"{mygo}\NCOP&NCED\[DBD-Raws][BanG Dream! It's MyGO!!!!!][NCOP][1080P][BDRip][HEVC-10bit][FLAC].mkv"
        ));
        assert_eq!((v.info.title.as_str(), v.info.season), ("BanG Dream! It's MyGO!!!!!", Some(0)));
        // 碟片菜单不入库
        assert!(classify_video(&p(&format!(
            r"{mygo}\menu\[DBD-Raws][BanG Dream! It's MyGO!!!!!][menu][01][1080P][BDRip][HEVC-10bit][FLAC].mkv"
        )))
        .is_none());
        assert_eq!(
            classify(&p(&format!(
                r"{mygo}\menu\[DBD-Raws][BanG Dream! It's MyGO!!!!!][menu][02][1080P][BDRip][HEVC-10bit][FLAC].mkv"
            ))),
            None
        );
        // [01集名][...] 开头的集号；4K修复 这层跳过；剧版上 / 下 算特别篇
        let gbc = r"F:\番\girls band cry（哭泣少女乐队）\4K修复";
        let v = vc(&format!(
            r"{gbc}\[01東京ワッショイ][B站沸羊羊都得叫我师傅][ガールズバンドクライ][2160P][喵萌Production&LoliHouse][4K修复].mkv"
        ));
        assert_eq!(
            (v.info.title.as_str(), v.info.episode, v.info.special),
            ("girls band cry（哭泣少女乐队）", Some(1.0), false)
        );
        let v = vc(&format!(r"{gbc}\剧版下.mp4"));
        assert_eq!(
            (v.info.title.as_str(), v.info.season, v.info.episode),
            ("girls band cry（哭泣少女乐队）", Some(0), Some(2.0))
        );
        // 动画电影：单独一部，用文件夹名
        let v = vc(r"F:\番\辉夜姬\4K\[MAI] Chou Kaguya-hime! [WebRip 2160p HEVC-10bit EAC3 ASS].mkv");
        assert_eq!((v.kind, v.series, v.info.title.as_str()), ("anime", true, "辉夜姬"));
    }

    #[test]
    fn real_movie_folders() {
        let v = vc(
            r"F:\电影\[阿甘正传][1994][中英双字][4K HDR][29.6G]\[阿甘正传][1994][中英双字][4K HDR][29.6G]\Forrest.Gump.1994.Bluray.2160p.x265.10bit.HDR.2Audio-MiniHD.mkv",
        );
        assert_eq!(
            (v.kind, v.series, v.movie, v.movie_guess, v.info.title.as_str()),
            ("video", false, true, false, "阿甘正传")
        );
        // 不在“电影”文件夹里，按发布名猜
        let v = vc(r"D:\Downloads\Forrest.Gump.1994.Bluray.2160p.x265.mkv");
        assert_eq!((v.movie, v.movie_guess, v.info.title.as_str()), (true, true, "Forrest Gump"));
        let v = vc(r"D:\Movies\潮汐之间 (2024) 2160p.mkv");
        assert_eq!((v.movie, v.info.title.as_str()), (true, "潮汐之间"));
        // 普通视频不算电影
        let v = vc(r"D:\Videos\录屏 2024-05-01.mp4");
        assert!(!v.movie && !v.series);
    }

    #[test]
    fn clean_titles() {
        assert_eq!(clean_title("[阿甘正传][1994][中英双字][4K HDR][29.6G]"), "阿甘正传");
        assert_eq!(clean_title("[喵萌Production][1080p][简日双语]"), "");
        assert_eq!(clean_title("4K修复"), "");
        assert_eq!(clean_title("0.孤独摇滚（上+下）2024【剧场总集篇】中字"), "孤独摇滚");
        assert_eq!(clean_title("girls band cry（哭泣少女乐队）"), "girls band cry（哭泣少女乐队）");
    }

    #[test]
    fn folder_title_beats_misc_dirs() {
        // 分类目录不当作品名
        let v = vc(r"D:\Downloads\[XX字幕组] Sousou no Frieren - 03 [1080p].mkv");
        assert_eq!((v.kind, v.info.title.as_str()), ("anime", "Sousou no Frieren"));
        let v = vc(r"E:\2024新番\[XX字幕组] Sousou no Frieren - 03 [1080p].mkv");
        assert_eq!(v.info.title, "Sousou no Frieren");
        let v = vc(r"D:\TV\Some.Show.S01E05.1080p.WEB-DL.mkv");
        assert_eq!((v.kind, v.series, v.info.title.as_str()), ("video", true, "Some.Show"));
    }
}
