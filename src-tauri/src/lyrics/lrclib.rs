//! LRCLIB（https://lrclib.net）：开放歌词库，不需要密钥。
//! 先用 /api/get 按标题 + 艺术家 + 专辑 + 时长精确查，查不到再用 /api/search 搜。
use super::{Found, Query, Source};
use crate::metadata::{parse_lrc, plain_lines};
use anyhow::Result;
use serde::Deserialize;

const BASE: &str = "https://lrclib.net/api";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

impl Item {
    /// 有时间轴的优先；纯音乐和空歌词返回 None
    pub fn into_found(self) -> Option<Found> {
        if self.instrumental {
            return None;
        }
        let lines = match (self.synced_lyrics.as_deref(), self.plain_lyrics.as_deref()) {
            (Some(s), _) if !parse_lrc(s).is_empty() => parse_lrc(s),
            (_, Some(p)) => plain_lines(p),
            _ => vec![],
        };
        (!lines.is_empty()).then_some(Found {
            source: Source::Lrclib,
            title: self.track_name,
            artist: self.artist_name,
            duration: self.duration,
            lines,
        })
    }
}

async fn get(http: &reqwest::Client, q: &Query) -> Result<Option<Item>> {
    let dur = (q.duration.round() as i64).to_string();
    let r = http
        .get(format!("{BASE}/get"))
        .query(&[
            ("track_name", q.title.as_str()),
            ("artist_name", q.artist.as_str()),
            ("album_name", q.album.as_str()),
            ("duration", dur.as_str()),
        ])
        .send()
        .await?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(r.error_for_status()?.json().await?))
}

async fn search(http: &reqwest::Client, q: &Query) -> Result<Vec<Item>> {
    let mut params = vec![("track_name", q.title.as_str())];
    if !q.artist.is_empty() {
        params.push(("artist_name", q.artist.as_str()));
    }
    Ok(http
        .get(format!("{BASE}/search"))
        .query(&params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

pub async fn find(http: &reqwest::Client, q: &Query) -> Result<Vec<Found>> {
    // /api/get 要求四项都有，缺艺术家或时长时直接搜
    if !q.artist.is_empty() && q.duration > 0.0 {
        if let Ok(Some(item)) = get(http, q).await {
            if let Some(f) = item.into_found() {
                if f.sync() > super::Sync::Plain {
                    return Ok(vec![f]);
                }
            }
        }
    }
    Ok(search(http, q).await?.into_iter().filter_map(Item::into_found).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response() {
        let json = r#"[
          {"id":1,"trackName":"晴天","artistName":"周杰伦","albumName":"叶惠美","duration":269.0,
           "instrumental":false,"plainLyrics":"故事的小黄花","syncedLyrics":"[00:29.36]故事的小黄花\n[00:32.80]从出生那年就飘着"},
          {"id":2,"trackName":"晴天","artistName":"周杰伦","duration":269.0,"instrumental":false,
           "plainLyrics":"故事的小黄花\n从出生那年就飘着","syncedLyrics":null},
          {"id":3,"trackName":"晴天 (伴奏)","artistName":"周杰伦","duration":269.0,"instrumental":true,
           "plainLyrics":null,"syncedLyrics":null}
        ]"#;
        let items: Vec<Item> = serde_json::from_str(json).unwrap();
        let found: Vec<Found> = items.into_iter().filter_map(Item::into_found).collect();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].sync(), super::super::Sync::Line);
        assert_eq!(found[0].lines.len(), 2);
        assert!((found[0].lines[1].t - 32.8).abs() < 1e-6);
        assert_eq!(found[1].sync(), super::super::Sync::Plain);
        assert_eq!(found[1].lines[1].text, "从出生那年就飘着");
    }
}
