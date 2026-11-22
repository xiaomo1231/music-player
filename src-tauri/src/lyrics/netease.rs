//! 网易云音乐：模拟 PC 客户端的 eapi（请求参数 AES 加密、响应是明文 JSON）。
//! 搜索 /eapi/search/song/list/page，歌词 /eapi/song/lyric/v1（lrc 逐行、yrc 逐字、tlyric 译文、romalrc 罗马音）。
//! eapi 失败时退回网页版的明文接口（/api/cloudsearch/pc、/api/song/lyric/v1）。参考 LDDC 的 core/api/lyrics/ne.py。
use super::{crypto, formats, top_cands, Cand, Found, Query, Source};
use crate::metadata::{parse_lrc, plain_lines, LyricLine};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

const APPVER: &str = "3.1.3.203419";
const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Safari/537.36 Chrome/91.0.4472.164 NeteaseMusicDesktop/3.1.3.203419";

/// 每次启动随机一个设备号
static DEVICE_ID: Lazy<String> = Lazy::new(|| {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    use md5::{Digest, Md5};
    hex::encode_upper(Md5::digest(n.to_le_bytes()))
});

const OSVER: &str = "Microsoft-Windows-10--build-22631-64bit";

fn cookies() -> String {
    format!(
        "os=pc; appver={APPVER}; osver={OSVER}; deviceId={}; channel=netease; mode=MS-iCraft B760M WIFI",
        *DEVICE_ID
    )
}

async fn eapi(http: &reqwest::Client, path: &str, mut params: Value) -> Result<Value> {
    params["header"] = Value::String(json!({"os":"pc","appver":APPVER,"deviceId":*DEVICE_ID,"requestId":0,"osver":OSVER}).to_string());
    let body = crypto::eapi_params(&path.replacen("/eapi/", "/api/", 1), &params.to_string());
    let v: Value = http
        .post(format!("https://interface.music.163.com{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("cookie", cookies())
        .header("origin", "orpheus://orpheus")
        .header("user-agent", UA)
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if v["code"].as_i64() != Some(200) {
        return Err(anyhow!("网易云接口错误 {}", v["code"]));
    }
    Ok(v)
}

async fn web_get(http: &reqwest::Client, path: &str, query: &[(&str, &str)]) -> Result<Value> {
    let v: Value = http
        .get(format!("https://music.163.com{path}"))
        .query(query)
        .header("referer", "https://music.163.com/")
        .header("cookie", "os=pc")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if v["code"].as_i64() != Some(200) {
        return Err(anyhow!("网易云接口错误 {}", v["code"]));
    }
    Ok(v)
}

/// simpleSongData / cloudsearch 的歌曲对象 → Cand（dt 是毫秒）
fn song(s: &Value) -> Option<Cand> {
    Some(Cand {
        id: s["id"].as_i64()?.to_string(),
        extra: String::new(),
        title: s["name"].as_str().unwrap_or("").to_string(),
        artist: s["ar"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join("/"))
            .unwrap_or_default(),
        album: s["al"]["name"].as_str().unwrap_or("").to_string(),
        duration: s["dt"].as_f64().map(|d| d / 1000.0),
    })
}

/// eapi 搜索（data.resources[].baseInfo.simpleSongData）或网页搜索（result.songs[]）
pub fn parse_songs(v: &Value) -> Vec<Cand> {
    if let Some(r) = v["data"]["resources"].as_array() {
        return r.iter().filter_map(|x| song(&x["baseInfo"]["simpleSongData"])).collect();
    }
    v["result"]["songs"].as_array().map(|a| a.iter().filter_map(song).collect()).unwrap_or_default()
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v[k]["lyric"].as_str().filter(|s| !s.trim().is_empty())
}

/// 歌词接口的返回：有 yrc 用逐字，没有用 lrc；再把 tlyric、romalrc 按时间对齐上去
pub fn parse_lyric(v: &Value) -> Vec<LyricLine> {
    let mut lines = match (field(v, "yrc"), field(v, "lrc")) {
        (Some(y), _) if !formats::parse_yrc(y).is_empty() => formats::parse_yrc(y),
        (_, Some(l)) => {
            let p = parse_lrc(l);
            if p.is_empty() {
                plain_lines(l)
            } else {
                p
            }
        }
        _ => return vec![],
    };
    if let Some(t) = field(v, "tlyric") {
        formats::attach(&mut lines, &parse_lrc(t), false);
    }
    if let Some(r) = field(v, "romalrc") {
        formats::attach(&mut lines, &parse_lrc(r), true);
    }
    lines
}

async fn search(http: &reqwest::Client, q: &Query) -> Result<Vec<Cand>> {
    let kw = q.keyword();
    let p = json!({"keyword": kw, "scene": "NORMAL", "needCorrect": "true", "limit": "20", "offset": "0"});
    if let Ok(v) = eapi(http, "/eapi/search/song/list/page", p).await {
        let c = parse_songs(&v);
        if !c.is_empty() {
            return Ok(c);
        }
    }
    let v = web_get(http, "/api/cloudsearch/pc", &[("s", kw.as_str()), ("type", "1"), ("limit", "20")]).await?;
    Ok(parse_songs(&v))
}

async fn fetch(http: &reqwest::Client, c: &Cand) -> Result<Found> {
    let id: i64 = c.id.parse()?;
    let v = match eapi(http, "/eapi/song/lyric/v1", json!({"id": id, "lv": "-1", "tv": "-1", "rv": "-1", "yv": "-1"})).await {
        Ok(v) => v,
        Err(_) => {
            let id = c.id.as_str();
            let q = [("id", id), ("lv", "-1"), ("tv", "-1"), ("rv", "-1"), ("yv", "-1")];
            web_get(http, "/api/song/lyric/v1", &q).await?
        }
    };
    Ok(Found {
        source: Source::Netease,
        title: c.title.clone(),
        artist: c.artist.clone(),
        duration: c.duration,
        lines: parse_lyric(&v),
    })
}

pub async fn find(http: &reqwest::Client, q: &Query) -> Result<Vec<Found>> {
    let mut out = vec![];
    for c in top_cands(q, search(http, q).await?, 2) {
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

    #[test]
    fn songs_both_shapes() {
        let e: Value = serde_json::from_str(
            r#"{"code":200,"data":{"resources":[{"baseInfo":{"simpleSongData":{"id":186016,"name":"晴天","ar":[{"name":"周杰伦"}],"al":{"name":"叶惠美"},"dt":269000}}}]}}"#,
        )
        .unwrap();
        let w: Value = serde_json::from_str(
            r#"{"code":200,"result":{"songs":[{"id":186016,"name":"晴天","ar":[{"name":"周杰伦"}],"al":{"name":"叶惠美"},"dt":269000}]}}"#,
        )
        .unwrap();
        for v in [e, w] {
            let c = parse_songs(&v);
            assert_eq!(c.len(), 1);
            assert_eq!((c[0].id.as_str(), c[0].title.as_str(), c[0].artist.as_str()), ("186016", "晴天", "周杰伦"));
            assert_eq!(c[0].duration, Some(269.0));
        }
    }

    #[test]
    fn lyric_prefers_yrc_and_attaches_translation() {
        let v = json!({
            "code": 200,
            "lrc": {"lyric": "[00:01.00]Hello world\n[00:04.00]Bye"},
            "tlyric": {"lyric": "[00:01.00]你好世界\n[00:04.00]再见"},
            "yrc": {"lyric": "[1000,2000](1000,500,0)Hello (1500,600,0)world\n[4000,1000](4000,1000,0)Bye"},
        });
        let l = parse_lyric(&v);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].words.len(), 2);
        assert_eq!(l[0].tr.as_deref(), Some("你好世界"));
        assert_eq!(l[1].tr.as_deref(), Some("再见"));

        let v = json!({"code": 200, "lrc": {"lyric": "[00:01.00]只有逐行"}, "yrc": {"lyric": ""}});
        let l = parse_lyric(&v);
        assert_eq!(l[0].text, "只有逐行");
        assert!(l[0].words.is_empty());
    }
}
