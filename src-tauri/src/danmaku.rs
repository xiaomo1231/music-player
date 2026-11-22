//! 弹幕：B 站弹幕 XML 转 ASS（任何支持 ASS 的播放器都能显示），以及把我们生成的 ASS 读回来给内置视频播放器绘制。
//! 文件约定：视频旁边的 `<视频文件名>.danmaku.ass`。
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 画布大小（ASS 的 PlayRes）
const W: f64 = 1920.0;
const H: f64 = 1080.0;
/// 基准字号与行高
const FONT: f64 = 48.0;
const LINE: f64 = 54.0;
/// 滚动弹幕横穿屏幕的时间、顶部 / 底部弹幕停留时间（秒）
const SCROLL_SECS: f64 = 8.0;
const FIXED_SECS: f64 = 4.0;
/// 滚动弹幕只占屏幕上方 80%，避免挡住字幕
const SCROLL_AREA: f64 = 0.8;

static RE_D: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<d\s+p="([^"]*)"\s*>([^<]*)</d>"#).unwrap());
static RE_ENTITY: Lazy<Regex> = Lazy::new(|| Regex::new(r"&(#x[0-9a-fA-F]+|#\d+|amp|lt|gt|quot|apos);").unwrap());
static RE_DIALOGUE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^Dialogue:\s*\d+,(\d+):(\d\d):(\d\d)\.(\d\d),(\d+):(\d\d):(\d\d)\.(\d\d),(R2L|TOP|BTM),[^,]*,\d+,\d+,\d+,[^,]*,(.*)$").unwrap());
static RE_TAGS: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\{([^}]*)\}").unwrap());
static RE_MOVE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\move\(\s*-?[\d.]+\s*,\s*(-?[\d.]+)").unwrap());
static RE_POS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\pos\(\s*-?[\d.]+\s*,\s*(-?[\d.]+)").unwrap());
static RE_COLOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\c&H([0-9A-Fa-f]{6})&").unwrap());
static RE_FS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\fs([\d.]+)").unwrap());

/// 一条原始弹幕
#[derive(Debug, Clone, PartialEq)]
pub struct Dm {
    /// 出现时间（秒）
    pub t: f64,
    /// 1-3 滚动，4 底部，5 顶部，6 逆向滚动；7、8 是高级 / 代码弹幕，不转换
    pub mode: u8,
    pub size: u32,
    /// 0xRRGGBB
    pub color: u32,
    pub text: String,
}

fn unescape(s: &str) -> String {
    RE_ENTITY
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let ch = match e {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
                _ => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            };
            ch.map(|c| c.to_string()).unwrap_or_default()
        })
        .into_owned()
}

/// 解析 B 站弹幕 XML（`<d p="时间,模式,字号,颜色,...">内容</d>`），按时间排序
pub fn parse_xml(xml: &str) -> Vec<Dm> {
    let mut out: Vec<Dm> = RE_D
        .captures_iter(xml)
        .filter_map(|c| {
            let p: Vec<&str> = c[1].split(',').collect();
            let t: f64 = p.first()?.parse().ok()?;
            let mode: u8 = p.get(1)?.parse().ok()?;
            let size: u32 = p.get(2).and_then(|s| s.parse().ok()).unwrap_or(25);
            let color: u32 = p.get(3).and_then(|s| s.parse().ok()).unwrap_or(0xFF_FFFF);
            let text = unescape(&c[2]).replace(['\r', '\n'], " ").trim().to_string();
            if text.is_empty() || !t.is_finite() || t < 0.0 || !matches!(mode, 1..=6) {
                return None;
            }
            Some(Dm { t, mode, size, color, text })
        })
        .collect();
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

/// 估算文字宽度（像素）：中日韩等宽字按 1 个字号，其余按 0.55 个
fn text_width(text: &str, font: f64) -> f64 {
    text.chars().map(|c| if c.is_ascii() { 0.55 } else { 1.0 }).sum::<f64>() * font
}

fn ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as i64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

/// ASS 文本里不能出现花括号和反斜杠（会被当成样式代码）
fn ass_text(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '{' => '｛',
            '}' => '｝',
            '\\' => '＼',
            c => c,
        })
        .collect()
}

/// 0xRRGGBB → ASS 的 BBGGRR
fn ass_color(rgb: u32) -> String {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    format!("{b:02X}{g:02X}{r:02X}")
}

/// 转成 ASS。滚动弹幕按“轨道”排布：前一条的尾巴已经进屏、并且新的一条追不上它时才能用同一轨道；
/// 所有轨道都占满时丢弃这一条（太密时自动稀释）。
pub fn to_ass(list: &[Dm]) -> String {
    let lanes = ((H * SCROLL_AREA) / LINE).floor() as usize;
    let fixed_lanes = ((H * 0.5) / LINE).floor() as usize;
    // 每条滚动轨道记录上一条的 (出现时间, 宽度, 速度)
    let mut scroll: Vec<Option<(f64, f64, f64)>> = vec![None; lanes];
    let mut top: Vec<f64> = vec![f64::MIN; fixed_lanes];
    let mut btm: Vec<f64> = vec![f64::MIN; fixed_lanes];
    let mut events = String::new();
    for d in list {
        let scale = (d.size as f64 / 25.0).clamp(0.6, 1.6);
        let font = (FONT * scale).round();
        let w = text_width(&d.text, font);
        let mut tags = String::new();
        if d.color & 0xFF_FFFF != 0xFF_FFFF {
            tags.push_str(&format!("\\c&H{}&", ass_color(d.color)));
        }
        if (scale - 1.0).abs() > 0.01 {
            tags.push_str(&format!("\\fs{font}"));
        }
        let text = ass_text(&d.text);
        match d.mode {
            4 | 5 => {
                let slots = if d.mode == 5 { &mut top } else { &mut btm };
                let Some(i) = slots.iter().position(|end| *end <= d.t) else { continue };
                slots[i] = d.t + FIXED_SECS;
                let (style, y) = if d.mode == 5 {
                    ("TOP", i as f64 * LINE + 4.0)
                } else {
                    ("BTM", H - i as f64 * LINE - 8.0)
                };
                events.push_str(&format!(
                    "Dialogue: 1,{},{},{style},,0,0,0,,{{\\pos({},{y}){tags}}}{text}\n",
                    ass_time(d.t),
                    ass_time(d.t + FIXED_SECS),
                    W / 2.0
                ));
            }
            _ => {
                let v = (W + w) / SCROLL_SECS;
                let free = |slot: &Option<(f64, f64, f64)>| match slot {
                    None => true,
                    Some((pt, pw, pv)) => {
                        let tail_in = d.t >= pt + pw / pv;
                        // 新弹幕的头到达左边缘时，前一条必须已经完全离开
                        let no_catch = d.t + W / v >= pt + SCROLL_SECS;
                        tail_in && no_catch
                    }
                };
                let Some(i) = scroll.iter().position(free) else { continue };
                scroll[i] = Some((d.t, w, v));
                let y = i as f64 * LINE + 4.0;
                let (x1, x2) = if d.mode == 6 { (-w, W) } else { (W, -w) };
                events.push_str(&format!(
                    "Dialogue: 0,{},{},R2L,,0,0,0,,{{\\move({x1:.0},{y},{x2:.0},{y}){tags}}}{text}\n",
                    ass_time(d.t),
                    ass_time(d.t + SCROLL_SECS)
                ));
            }
        }
    }
    let style = |name: &str, align: u8| {
        format!("Style: {name},Microsoft YaHei,{FONT},&H33FFFFFF,&H33FFFFFF,&H33000000,&H00000000,1,0,0,0,100,100,0,0,1,1.5,0,{align},0,0,0,1\n")
    };
    format!(
        "[Script Info]\n; 由极光播放器从 B 站弹幕生成\nScriptType: v4.00+\nPlayResX: {W}\nPlayResY: {H}\nWrapStyle: 2\nScaledBorderAndShadow: yes\n\n\
         [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         {}{}{}\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n{events}",
        style("R2L", 7),
        style("TOP", 8),
        style("BTM", 2)
    )
}

/// 把目录里 yt-dlp 下载的 `*.danmaku.xml` 都转成同名的 `*.danmaku.ass`，转换成功后删除 XML。返回转换的个数
pub fn convert_dir(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(stem) = name.strip_suffix(".danmaku.xml") else { continue };
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let list = parse_xml(&String::from_utf8_lossy(&bytes));
        let out = p.with_file_name(format!("{stem}.danmaku.ass"));
        if std::fs::write(&out, to_ass(&list)).is_ok() {
            let _ = std::fs::remove_file(&p);
            n += 1;
        }
    }
    n
}

/// 给内置播放器用的一条弹幕
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Item {
    pub start: f64,
    pub end: f64,
    /// scroll / top / bottom
    pub mode: &'static str,
    /// 纵向位置，0-1（占画面高度的比例）
    pub y: f64,
    /// #rrggbb
    pub color: String,
    /// 相对基准字号的倍数
    pub scale: f64,
    pub text: String,
}

fn hms(h: &str, m: &str, s: &str, cs: &str) -> f64 {
    let p = |x: &str| x.parse::<f64>().unwrap_or(0.0);
    p(h) * 3600.0 + p(m) * 60.0 + p(s) + p(cs) / 100.0
}

/// 读回我们生成的 ASS（只认 R2L / TOP / BTM 三种样式）
pub fn parse_ass(src: &str) -> Vec<Item> {
    let mut out: Vec<Item> = src
        .lines()
        .filter_map(|line| {
            let c = RE_DIALOGUE.captures(line.trim_end())?;
            let start = hms(&c[1], &c[2], &c[3], &c[4]);
            let end = hms(&c[5], &c[6], &c[7], &c[8]);
            let mode = match &c[9] {
                "TOP" => "top",
                "BTM" => "bottom",
                _ => "scroll",
            };
            let body = &c[10];
            let (tags, text) = match RE_TAGS.captures(body) {
                Some(t) => (t[1].to_string(), body[t.get(0).unwrap().end()..].to_string()),
                None => (String::new(), body.to_string()),
            };
            let y = RE_MOVE
                .captures(&tags)
                .or_else(|| RE_POS.captures(&tags))
                .and_then(|m| m[1].parse::<f64>().ok())
                .map(|y| if mode == "bottom" { (y - LINE).max(0.0) } else { y })
                .unwrap_or(0.0);
            let color = RE_COLOR
                .captures(&tags)
                .map(|m| {
                    let bgr = &m[1];
                    format!("#{}{}{}", &bgr[4..6], &bgr[2..4], &bgr[0..2]).to_ascii_lowercase()
                })
                .unwrap_or_else(|| "#ffffff".into());
            let scale = RE_FS.captures(&tags).and_then(|m| m[1].parse::<f64>().ok()).map(|fs| fs / FONT).unwrap_or(1.0);
            let text = text.replace("\\N", " ").replace("\\n", " ");
            (!text.trim().is_empty() && end > start).then(|| Item {
                start,
                end,
                mode,
                y: (y / H).clamp(0.0, 0.95),
                color,
                scale,
                text,
            })
        })
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// 视频旁边的弹幕文件路径
pub fn sidecar(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let p = video.with_file_name(format!("{stem}.danmaku.ass"));
    p.is_file().then_some(p)
}

/// 文件名是不是弹幕（字幕列表里不显示它）
pub fn is_danmaku_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".danmaku.ass") || l.ends_with(".danmaku.xml")
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><i><chatserver>chat.bilibili.com</chatserver>
<d p="12.5,1,25,16777215,1700000000,0,abc,1,10">第二条 &amp; 测试</d>
<d p="1.0,1,25,16711680,1700000000,0,abc,2,10">红色弹幕</d>
<d p="3.0,5,25,16777215,1700000000,0,abc,3,10">顶部</d>
<d p="4.0,4,30,65280,1700000000,0,abc,4,10">底部大字</d>
<d p="5.0,7,25,16777215,1700000000,0,abc,5,10">[0,0,"高级弹幕"]</d>
<d p="6.0,1,25,16777215,1700000000,0,abc,6,10">{花括号}</d>
</i>"#;

    #[test]
    fn parses_and_sorts_xml() {
        let v = parse_xml(XML);
        assert_eq!(v.len(), 5, "高级弹幕不转换");
        assert_eq!(v[0].text, "红色弹幕");
        assert_eq!(v[0].color, 0xFF0000);
        assert_eq!(v.last().unwrap().text, "第二条 & 测试");
        assert_eq!(v[2].mode, 4);
        assert_eq!(v[2].size, 30);
    }

    #[test]
    fn times_and_colors() {
        assert_eq!(ass_time(0.0), "0:00:00.00");
        assert_eq!(ass_time(3723.456), "1:02:03.46");
        assert_eq!(ass_color(0xFF8000), "0080FF");
    }

    #[test]
    fn ass_round_trip() {
        let ass = to_ass(&parse_xml(XML));
        assert!(ass.contains("PlayResX: 1920"));
        assert!(ass.contains("｛花括号｝"), "花括号要换成全角");
        let items = parse_ass(&ass);
        assert_eq!(items.len(), 5);
        let red = items.iter().find(|i| i.text == "红色弹幕").unwrap();
        assert_eq!(red.mode, "scroll");
        assert_eq!(red.color, "#ff0000");
        assert!((red.start - 1.0).abs() < 1e-9 && (red.end - 9.0).abs() < 1e-9);
        let top = items.iter().find(|i| i.text == "顶部").unwrap();
        assert_eq!(top.mode, "top");
        let btm = items.iter().find(|i| i.text == "底部大字").unwrap();
        assert_eq!(btm.mode, "bottom");
        assert_eq!(btm.color, "#00ff00");
        assert!(btm.scale > 1.1);
        assert!(btm.y > 0.8);
        assert_eq!(items.iter().find(|i| i.text == "第二条 & 测试").unwrap().scale, 1.0);
    }

    #[test]
    fn dense_comments_are_thinned_and_lanes_do_not_overlap() {
        // 0.4 秒内 100 条：轨道数有限，多出来的被丢弃
        let list: Vec<Dm> = (0..100)
            .map(|i| Dm {
                t: 10.0 + i as f64 * 0.004,
                mode: 1,
                size: 25,
                color: 0xFFFFFF,
                text: format!("弹幕{i}"),
            })
            .collect();
        let items = parse_ass(&to_ass(&list));
        let lanes = ((H * SCROLL_AREA) / LINE).floor() as usize;
        assert_eq!(items.len(), lanes);
        let mut ys: Vec<i64> = items.iter().map(|i| (i.y * H).round() as i64).collect();
        ys.sort();
        ys.dedup();
        assert_eq!(ys.len(), lanes, "每条在不同轨道");
    }

    #[test]
    fn same_lane_is_reused_after_gap() {
        let list = vec![
            Dm {
                t: 0.0,
                mode: 1,
                size: 25,
                color: 0xFFFFFF,
                text: "短".into(),
            },
            Dm {
                t: 5.0,
                mode: 1,
                size: 25,
                color: 0xFFFFFF,
                text: "短".into(),
            },
        ];
        let items = parse_ass(&to_ass(&list));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].y, items[1].y);
    }

    #[test]
    fn converts_files_in_dir() {
        let dir = std::env::temp_dir().join(format!("mp-dm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("视频 [BV1].danmaku.xml"), XML).unwrap();
        std::fs::write(dir.join("视频 [BV1].mp4"), b"x").unwrap();
        assert_eq!(convert_dir(&dir), 1);
        assert!(!dir.join("视频 [BV1].danmaku.xml").exists());
        let video = dir.join("视频 [BV1].mp4");
        let side = sidecar(&video).unwrap();
        assert_eq!(parse_ass(&std::fs::read_to_string(side).unwrap()).len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn danmaku_names() {
        assert!(is_danmaku_name("a [BV1].danmaku.ass"));
        assert!(is_danmaku_name("a.DANMAKU.xml"));
        assert!(!is_danmaku_name("a.zh-Hans.srt"));
    }
}
