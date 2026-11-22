//! 酷狗音乐：lyrics.kugou.com 按“艺术家 - 标题”和时长直接搜歌词候选，再下载 KRC（逐字 + 译文 + 罗马音）。
//! 这是酷狗 PC 客户端多年未变的歌词接口，ESLyric、LDDC 等都在用。
use super::{crypto, formats, top_cands, Cand, Found, Query, Source};
use crate::metadata::{parse_lrc, plain_lines};
use anyhow::{anyhow, Result};
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;

const BASE: &str = "http://lyrics.kugou.com";

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    candidates: Vec<Value>,
}

fn s(v: &Value, k: &str) -> String {
    match &v[k] {
        Value::String(x) => x.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// 歌词候选 → Cand（duration 是毫秒）
pub fn parse_candidates(v: Vec<Value>) -> Vec<Cand> {
    v.iter()
        .filter(|c| !s(c, "id").is_empty() && !s(c, "accesskey").is_empty())
        .map(|c| Cand {
            id: s(c, "id"),
            extra: s(c, "accesskey"),
            title: s(c, "song"),
            artist: s(c, "singer"),
            album: String::new(),
            duration: c["duration"].as_f64().map(|d| d / 1000.0),
        })
        .collect()
}

/// download 接口的返回：contenttype 2 是 base64 的纯文本 / LRC，其他是 KRC
pub fn parse_download(v: &Value) -> Result<Vec<crate::metadata::LyricLine>> {
    let content = v["content"].as_str().ok_or_else(|| anyhow!("酷狗没有返回歌词"))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(content)?;
    if v["contenttype"].as_i64() == Some(2) {
        let text = String::from_utf8_lossy(&bytes);
        let l = parse_lrc(&text);
        return Ok(if l.is_empty() { plain_lines(&text) } else { l });
    }
    Ok(formats::parse_krc(&crypto::krc_decrypt(&bytes)?))
}

async fn fetch(http: &reqwest::Client, c: &Cand) -> Result<Found> {
    let v: Value = http
        .get(format!("{BASE}/download"))
        .query(&[
            ("ver", "1"),
            ("client", "pc"),
            ("id", c.id.as_str()),
            ("accesskey", c.extra.as_str()),
            ("fmt", "krc"),
            ("charset", "utf8"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(Found {
        source: Source::Kugou,
        title: c.title.clone(),
        artist: c.artist.clone(),
        duration: c.duration,
        lines: parse_download(&v)?,
    })
}

pub async fn find(http: &reqwest::Client, q: &Query) -> Result<Vec<Found>> {
    let keyword = if q.artist.is_empty() {
        q.title.clone()
    } else {
        format!("{} - {}", q.artist, q.title)
    };
    let dur = ((q.duration * 1000.0).round() as i64).to_string();
    let r: Search = http
        .get(format!("{BASE}/search"))
        .query(&[
            ("ver", "1"),
            ("man", "yes"),
            ("client", "pc"),
            ("keyword", keyword.as_str()),
            ("duration", dur.as_str()),
            ("hash", ""),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut out = vec![];
    for c in top_cands(q, parse_candidates(r.candidates), 2) {
        if let Ok(f) = fetch(http, &c).await {
            if !f.lines.is_empty() {
                out.push(f);
                break;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn candidates_and_download() {
        let v: Value = serde_json::from_str(
            r#"{"status":200,"candidates":[{"id":"123","accesskey":"ABC","song":"晴天","singer":"周杰伦","duration":269000,"score":60},
                {"id":"","accesskey":"x","song":"坏的"}]}"#,
        )
        .unwrap();
        let c = parse_candidates(v["candidates"].as_array().unwrap().clone());
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].id.as_str(), c[0].extra.as_str()), ("123", "ABC"));
        assert_eq!(c[0].duration, Some(269.0));

        // 构造一份 KRC：krc1 头 + 异或 + zlib
        let plain = "[1000,1000]<0,500,0>你<500,500,0>好";
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(plain.as_bytes()).unwrap();
        let key = b"@Gaw^2tGQ61-\xce\xd2ni";
        let mut data = b"krc1".to_vec();
        data.extend(z.finish().unwrap().iter().enumerate().map(|(i, b)| b ^ key[i % key.len()]));
        let b64 = base64::engine::general_purpose::STANDARD.encode(data);
        let d: Value = serde_json::json!({"status":200,"contenttype":0,"content":b64});
        let l = parse_download(&d).unwrap();
        assert_eq!(l[0].text, "你好");
        assert_eq!(l[0].words.len(), 2);

        let d: Value = serde_json::json!({"contenttype":2,"content":base64::engine::general_purpose::STANDARD.encode("[00:01.00]一行")});
        assert_eq!(parse_download(&d).unwrap()[0].text, "一行");
    }
}
