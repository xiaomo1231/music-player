//! QQ 音乐：musicu.fcg 统一接口（模拟 QQ 音乐简洁版安卓客户端）。
//! 先取会话（只取一次），再搜歌，再用 GetPlayLyricInfo 取 QRC（逐字）、译文（LRC）、罗马音（QRC），三者都是加密的十六进制。
//! 请求参数参考 LDDC 的 core/api/lyrics/qm.py。
use super::{crypto, formats, top_cands, Cand, Found, Query, Source};
use crate::metadata::{parse_lrc, plain_lines, LyricLine};
use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";

/// 会话（uid / sid / userip），第一次请求时取
static SESSION: Mutex<Option<Value>> = Mutex::const_new(None);

fn comm(session: Option<&Value>) -> Value {
    let mut c = json!({
        "ct": 11,
        "cv": "1003006",
        "v": "1003006",
        "os_ver": "15",
        "phonetype": "24122RKC7C",
        "rom": "Redmi/miro/miro:15/AE3A.240806.005/OS2.0.105.0.VOMCNXM:user/release-keys",
        "tmeAppID": "qqmusiclight",
        "nettype": "NETWORK_WIFI",
        "udid": "0",
    });
    if let Some(s) = session {
        for k in ["uid", "sid", "userip"] {
            c[k] = s[k].clone();
        }
    }
    c
}

async fn call(http: &reqwest::Client, session: Option<&Value>, method: &str, module: &str, param: Value) -> Result<Value> {
    let body = json!({ "comm": comm(session), "request": { "method": method, "module": module, "param": param } });
    let v: Value = http
        .post(URL)
        .header("cookie", "tmeLoginType=-1;")
        .header("user-agent", "okhttp/3.14.9")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let code = v["code"].as_i64().unwrap_or(-1);
    let rcode = v["request"]["code"].as_i64().unwrap_or(-1);
    if code != 0 || rcode != 0 {
        return Err(anyhow!("QQ 音乐接口错误 {code}/{rcode}"));
    }
    Ok(v["request"]["data"].clone())
}

/// 取会话；失败时不带会话继续请求（多数接口没有会话也能用）
async fn session(http: &reqwest::Client) -> Option<Value> {
    let mut g = SESSION.lock().await;
    if g.is_none() {
        let p = json!({"caller": 0, "uid": "0", "vkey": 0});
        if let Ok(d) = call(http, None, "GetSession", "music.getSession.session", p).await {
            *g = Some(d["session"].clone());
        }
    }
    g.clone()
}

fn search_id() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let r = ms.wrapping_mul(2654435761);
    ((r % 20 + 1) * 18014398509481984 + (r % 4194304) * 4294967296 + ms % 86400000).to_string()
}

pub fn parse_songs(data: &Value) -> Vec<Cand> {
    data["body"]["item_song"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|s| s["id"].as_i64().is_some())
        .map(|s| Cand {
            id: s["id"].as_i64().unwrap_or(0).to_string(),
            extra: String::new(),
            title: s["title"].as_str().or(s["name"].as_str()).unwrap_or("").to_string(),
            artist: s["singer"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join("/"))
                .unwrap_or_default(),
            album: s["album"]["name"].as_str().unwrap_or("").to_string(),
            duration: s["interval"].as_f64(),
        })
        .collect()
}

fn decrypt_field(data: &Value, key: &str) -> Option<String> {
    let s = data[key].as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    crypto::qrc_decrypt(s).ok()
}

/// 解密后的原文可能是 QRC，也可能是普通 LRC 或纯文本
fn parse_any(text: &str) -> Vec<LyricLine> {
    if text.contains("LyricContent=") {
        return formats::parse_qrc(text);
    }
    let l = parse_lrc(text);
    if l.is_empty() {
        plain_lines(text)
    } else {
        l
    }
}

/// GetPlayLyricInfo 的返回 → 原文 + 译文 + 罗马音
pub fn parse_lyric(data: &Value) -> Vec<LyricLine> {
    let Some(orig) = decrypt_field(data, "lyric") else { return vec![] };
    let mut lines = parse_any(&orig);
    if let Some(tr) = decrypt_field(data, "trans") {
        formats::attach(&mut lines, &parse_any(&tr), false);
    }
    if let Some(roma) = decrypt_field(data, "roma") {
        formats::attach(&mut lines, &parse_any(&roma), true);
    }
    lines
}

async fn fetch(http: &reqwest::Client, s: Option<&Value>, c: &Cand) -> Result<Found> {
    let b64 = |x: &str| base64::engine::general_purpose::STANDARD.encode(x);
    let param = json!({
        "albumName": b64(&c.album),
        "crypt": 1,
        "ct": 19,
        "cv": 2111,
        "interval": c.duration.unwrap_or(0.0).round() as i64,
        "lrc_t": 0,
        "qrc": 1,
        "qrc_t": 0,
        "roma": 1,
        "roma_t": 0,
        "singerName": b64(&c.artist),
        "songID": c.id.parse::<i64>()?,
        "songName": b64(&c.title),
        "trans": 1,
        "trans_t": 0,
        "type": 0,
    });
    let data = call(http, s, "GetPlayLyricInfo", "music.musichallSong.PlayLyricInfo", param).await?;
    Ok(Found {
        source: Source::Qq,
        title: c.title.clone(),
        artist: c.artist.clone(),
        duration: c.duration,
        lines: parse_lyric(&data),
    })
}

pub async fn find(http: &reqwest::Client, q: &Query) -> Result<Vec<Found>> {
    let s = session(http).await;
    let param = json!({
        "search_id": search_id(),
        "remoteplace": "search.android.keyboard",
        "query": q.keyword(),
        "search_type": 0,
        "num_per_page": 20,
        "page_num": 1,
        "highlight": 0,
        "nqc_flag": 0,
        "page_id": 1,
        "grp": 1,
    });
    let data = call(http, s.as_ref(), "DoSearchForQQMusicLite", "music.search.SearchCgiService", param).await?;
    let mut out = vec![];
    for c in top_cands(q, parse_songs(&data), 2) {
        if let Ok(f) = fetch(http, s.as_ref(), &c).await {
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
    use crate::lyrics::crypto::qrc_encrypt_for_test as enc;

    #[test]
    fn songs() {
        let d: Value = serde_json::from_str(
            r#"{"body":{"item_song":[{"id":97773,"mid":"0039MnYb0qxYhV","title":"晴天","singer":[{"name":"周杰伦"}],
                "album":{"name":"叶惠美"},"interval":269}]}}"#,
        )
        .unwrap();
        let c = parse_songs(&d);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].id.as_str(), c[0].artist.as_str(), c[0].album.as_str()), ("97773", "周杰伦", "叶惠美"));
        assert_eq!(c[0].duration, Some(269.0));
    }

    #[test]
    fn lyric_with_translation_and_roma() {
        let orig = r#"<Lyric_1 LyricType="1" LyricContent="[1000,1000]こん(1000,500)にちは(1500,500)
[3000,1000]さよ(3000,500)なら(3500,500)
"/>"#;
        let d = json!({
            "lyric": enc(orig),
            "trans": enc("[00:01.00]你好\n[00:03.00]再见"),
            "roma": enc(r#"<Lyric_1 LyricType="1" LyricContent="[1000,1000]kon (1000,500)nichiwa(1500,500)
        "/>"#),
        });
        let l = parse_lyric(&d);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "こんにちは");
        assert_eq!(l[0].words.len(), 2);
        assert_eq!(l[0].tr.as_deref(), Some("你好"));
        assert_eq!(l[0].roma.as_deref(), Some("kon nichiwa"));
        assert_eq!(l[1].tr.as_deref(), Some("再见"));
        assert_eq!(l[1].roma, None);
    }
}
