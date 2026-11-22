//! 平台歌词格式解析：网易云 YRC、QQ 音乐 QRC、酷狗 KRC（都是逐字），以及把译文 / 罗马音按时间对齐到原文行。
//! 行头都是 `[开始毫秒,时长毫秒]`，区别在逐字时间戳的写法：
//! - YRC：`(开始,时长,0)字`，绝对时间，时间戳在字前面
//! - QRC：`字(开始,时长)`，绝对时间，时间戳在字后面
//! - KRC：`<偏移,时长,0>字`，相对行开始，时间戳在字前面；译文、罗马音在 `[language:base64 JSON]` 标签里
use crate::metadata::LyricLine;
use base64::Engine;
use serde::Deserialize;

/// 在 s[i..] 处匹配 `open 数字,数字[,数字] close`，返回数字和结束位置
fn stamp_at(s: &str, i: usize, open: char, close: char, n: usize) -> Option<(Vec<u64>, usize)> {
    let rest = s[i..].strip_prefix(open)?;
    let end = rest.find(close)?;
    let nums: Vec<u64> = rest[..end].split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    (nums.len() == n).then_some((nums, i + open.len_utf8() + end + close.len_utf8()))
}

/// 行头 `[开始,时长]`
fn line_head(line: &str) -> Option<(u64, &str)> {
    let (nums, end) = stamp_at(line, 0, '[', ']', 2)?;
    Some((nums[0], &line[end..]))
}

/// 时间戳在字前面（YRC / KRC）：返回 (时间戳数字, 字)
fn prefix_words(content: &str, open: char, close: char) -> Vec<(Vec<u64>, String)> {
    let mut out: Vec<(Vec<u64>, String)> = vec![];
    let mut i = 0;
    while i < content.len() {
        if let Some((nums, end)) = stamp_at(content, i, open, close, 3) {
            out.push((nums, String::new()));
            i = end;
            continue;
        }
        let c = content[i..].chars().next().unwrap();
        if let Some(last) = out.last_mut() {
            last.1.push(c);
        }
        i += c.len_utf8();
    }
    out
}

/// 时间戳在字后面（QRC）
fn suffix_words(content: &str) -> Vec<(u64, String)> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut i = 0;
    while i < content.len() {
        if let Some((nums, end)) = stamp_at(content, i, '(', ')', 2) {
            out.push((nums[0], std::mem::take(&mut cur)));
            i = end;
            continue;
        }
        let c = content[i..].chars().next().unwrap();
        cur.push(c);
        i += c.len_utf8();
    }
    out
}

/// `stamped`：这一行里有逐字时间戳（哪怕字是空的）；没有时整行内容就是歌词
fn make_line(start_ms: u64, words: Vec<(f64, String)>, raw: &str, stamped: bool) -> LyricLine {
    let text = if stamped {
        words.iter().map(|w| w.1.as_str()).collect()
    } else {
        raw.to_string()
    };
    let words = if words.len() > 1 { words } else { vec![] };
    LyricLine {
        t: start_ms as f64 / 1000.0,
        text: text.trim().to_string(),
        words,
        ..Default::default()
    }
}

/// 网易云 YRC；夹在中间的 JSON 行（作词作曲信息）跳过
pub fn parse_yrc(s: &str) -> Vec<LyricLine> {
    let mut out: Vec<LyricLine> = s
        .lines()
        .filter_map(|l| line_head(l.trim()))
        .map(|(start, content)| {
            let words: Vec<_> = prefix_words(content, '(', ')').into_iter().map(|(n, w)| (n[0] as f64 / 1000.0, w)).collect();
            let stamped = !words.is_empty();
            make_line(start, words, content, stamped)
        })
        .filter(|l| !l.text.is_empty())
        .collect();
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// QQ 音乐 QRC：明文是一段 XML，歌词在 LyricContent 属性里
pub fn parse_qrc(s: &str) -> Vec<LyricLine> {
    let body = match s.find("LyricContent=\"") {
        Some(i) => {
            let rest = &s[i + "LyricContent=\"".len()..];
            xml_unescape(&rest[..rest.rfind("\"/>").or_else(|| rest.rfind('"')).unwrap_or(rest.len())])
        }
        None => s.to_string(),
    };
    let mut out: Vec<LyricLine> = body
        .lines()
        .filter_map(|l| line_head(l.trim()))
        .map(|(start, content)| {
            let all = suffix_words(content);
            let stamped = !all.is_empty();
            let words = all
                .into_iter()
                .filter(|(_, w)| !w.is_empty() && w != "\r")
                .map(|(t, w)| (t as f64 / 1000.0, w))
                .collect::<Vec<_>>();
            make_line(start, words, content, stamped)
        })
        .filter(|l| !l.text.is_empty())
        .collect();
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

#[derive(Deserialize)]
struct KrcLanguage {
    #[serde(default)]
    content: Vec<KrcLangItem>,
}

#[derive(Deserialize)]
struct KrcLangItem {
    #[serde(rename = "type")]
    kind: i64,
    #[serde(rename = "lyricContent", default)]
    lyric_content: Vec<Vec<String>>,
}

/// 酷狗 KRC（已解密）。type 1 是逐行译文，按原文行号对齐；type 0 是逐字罗马音，跳过没有字的行
pub fn parse_krc(s: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = vec![];
    let mut language = None;
    for raw in s.lines() {
        let line = raw.trim();
        if let Some(v) = line.strip_prefix("[language:") {
            language = Some(v.trim_end_matches(']').trim().to_string());
            continue;
        }
        let Some((start, content)) = line_head(line) else { continue };
        let words: Vec<_> = prefix_words(content, '<', '>')
            .into_iter()
            .map(|(n, w)| ((start + n[0]) as f64 / 1000.0, w))
            .collect();
        let stamped = !words.is_empty();
        lines.push(make_line(start, words, content, stamped));
    }
    if let Some(lang) = language.filter(|l| !l.is_empty()) {
        let parsed = base64::engine::general_purpose::STANDARD
            .decode(lang)
            .ok()
            .and_then(|b| serde_json::from_slice::<KrcLanguage>(&b).ok());
        for item in parsed.map(|p| p.content).unwrap_or_default() {
            match item.kind {
                1 => {
                    for (l, tr) in lines.iter_mut().zip(&item.lyric_content) {
                        let t = tr.concat();
                        if !t.trim().is_empty() {
                            l.tr = Some(t.trim().to_string());
                        }
                    }
                }
                0 => {
                    let mut it = item.lyric_content.iter();
                    for l in lines.iter_mut().filter(|l| !l.text.is_empty()) {
                        let Some(r) = it.next() else { break };
                        let r = r.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
                        if !r.is_empty() {
                            l.roma = Some(r);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    lines.retain(|l| !l.text.is_empty());
    lines
}

/// 把另一份歌词（译文或罗马音，LRC 格式解析后的行）按时间对齐到原文：取 1 秒内最近的原文行
pub fn attach(lines: &mut [LyricLine], extra: &[LyricLine], roma: bool) {
    for e in extra {
        let text = e.text.trim();
        if text.is_empty() || text == "//" || e.t < 0.0 {
            continue;
        }
        let best = lines
            .iter()
            .enumerate()
            .map(|(i, l)| (i, (l.t - e.t).abs()))
            .filter(|(_, d)| *d <= 1.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = best {
            let slot = if roma { &mut lines[i].roma } else { &mut lines[i].tr };
            if slot.is_none() && text != lines[i].text {
                *slot = Some(text.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::parse_lrc;

    #[test]
    fn yrc() {
        let l = parse_yrc("{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[1000,2000](1000,500,0)Hello (1500,600,0)world\n[4000,1000](4000,1000,0)Bye");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "Hello world");
        assert_eq!(l[0].words, vec![(1.0, "Hello ".into()), (1.5, "world".into())]);
        assert!(l[1].words.is_empty());
        assert_eq!(l[1].text, "Bye");
    }

    #[test]
    fn qrc() {
        let x = r#"<?xml version="1.0" encoding="utf-8"?><QrcInfos><LyricInfo LyricCount="1"><Lyric_1 LyricType="1" LyricContent="[ti:晴天]
[29360,3440]故(29360,300)事(29660,300)的(29960,200)小(30160,400)黄(30560,300)花(30860,1940)
[33000,500](33000,500)
[34000,1000]I&apos;m(34000,500) (Live)(34500,500)
"/></LyricInfo></QrcInfos>"#;
        let l = parse_qrc(x);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "故事的小黄花");
        assert_eq!(l[0].words.len(), 6);
        assert!((l[0].words[1].0 - 29.66).abs() < 1e-9);
        assert_eq!(l[1].text, "I'm (Live)");
    }

    #[test]
    fn krc_with_language() {
        let lang = base64::engine::general_purpose::STANDARD
            .encode(r#"{"content":[{"type":1,"lyricContent":[["你好"],["世界"]]},{"type":0,"lyricContent":[["ha","lo"],["wa"]]}],"version":1}"#);
        let krc = format!("[ar:x]\n[language:{lang}]\n[1000,1000]<0,500,0>Hel<500,500,0>lo\n[3000,500]<0,500,0>World\n");
        let l = parse_krc(&krc);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "Hello");
        assert_eq!(l[0].words, vec![(1.0, "Hel".into()), (1.5, "lo".into())]);
        assert_eq!(l[0].tr.as_deref(), Some("你好"));
        assert_eq!(l[0].roma.as_deref(), Some("ha lo"));
        assert_eq!(l[1].tr.as_deref(), Some("世界"));
    }

    #[test]
    fn attach_by_time() {
        let mut l = parse_yrc("[1000,2000](1000,2000,0)Hello\n[5000,1000](5000,1000,0)Bye");
        attach(&mut l, &parse_lrc("[00:01.10]你好\n[00:05.00]//\n[00:09.00]远处"), false);
        assert_eq!(l[0].tr.as_deref(), Some("你好"));
        assert_eq!(l[1].tr, None);
    }
}
