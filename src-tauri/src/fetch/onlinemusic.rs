//! 在线音乐下载源：按关键字搜索，解析出直链后交给框架的 HTTP 下载队列。
//!
//! 调用约定（照上游前端实现）：
//!
//! ```text
//! POST {BASE}/ajax.php?act=search    platform=kuwo|wyy & keyword & page & size
//!     -> {"code":0,"data":{"list":[{id,name,artist,album_name,pic_url,duration,minfo[],time,sign}],"total":n}}
//! POST {BASE}/ajax.php?act=getUrl    platform & songid & format & bitrate & time & sign
//!     -> {"code":0,"data":{"url":"https://…/x.mp3","format":"mp3","bitrate":128}}
//! POST {BASE}/ajax.php?act=getLyric  platform & songid & time & sign
//!     -> {"code":0,"data":"[00:00.00]…"}
//! ```
//!
//! 两个要点：
//! 1. `act` 在 query 上，其余参数全在 form body 里（服务端只从表单体取参），所以必须 POST，不能拼 query。
//! 2. `time` / `sign` 由搜索接口按「会话 + 单曲」签发，必须原样回传；它们随搜索结果一起进
//!    `Item.extra`，`resolve` 直接取用，满足「每次开始 / 继续下载都重新解析」的约定。
//!
//! 音源选择：搜索页没有给下载源留设置项，所以用关键词前缀切换——
//! `wyy:关键词` / `网易:关键词` 走网易云，其余走酷我。

use super::{provider, source};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::time::Duration;

const BASE: &str = "https://flac.music.hi.cn";

/// 站点对 UA 不挑，但保持与浏览器一致，避免以后加 WAF 时踩坑
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                   (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Default)]
pub struct OnlineMusic;

// ---------- HTTP ----------

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default()
}

/// `application/x-www-form-urlencoded` 的 body（自己编码，不依赖 reqwest 的 form 特性开关）
fn encode(pairs: &[(&str, &str)]) -> String {
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs.iter() {
        ser.append_pair(*k, *v);
    }
    ser.finish()
}

/// 调一次 `ajax.php`，返回整个答复 JSON（`code != 0` 时把站点给的 `msg` 当错误抛出）
async fn post(act: &str, pairs: &[(&str, &str)]) -> Result<Value, String> {
    let url = format!("{BASE}/ajax.php?act={act}");
    source::throttle(&url).await;
    let body = encode(pairs);
    let resp = client()
        .post(&url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .header(reqwest::header::REFERER, format!("{BASE}/"))
        .header(reqwest::header::ORIGIN, BASE)
        .header("X-Requested-With", "XMLHttpRequest")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("连接音乐服务失败：{e}"))?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err("音乐服务拒绝了这次请求（403），请稍后再试。".into());
    }
    source::check_status(resp.status())?;
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("音乐服务返回的内容无法解析：{e}"))?;
    match v["code"].as_i64() {
        Some(0) => Ok(v),
        _ => Err(v["msg"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("音乐服务暂时不可用")
            .to_string()),
    }
}

// ---------- 字段提取 ----------

fn text(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn id_of(v: &Value) -> String {
    text(v, "id")
}

/// 站点的 duration 是「秒」的数字字符串
fn duration_of(v: &Value) -> Option<f64> {
    text(v, "duration").parse::<f64>().ok().filter(|d| *d > 0.0)
}

/// minfo[] → 档位。key 用 `格式:码率`，`resolve` 拆开后原样回传给站点
fn quality_of(m: &Value) -> Option<provider::Quality> {
    let format = text(m, "format").to_ascii_lowercase();
    if format.is_empty() {
        return None;
    }
    let bitrate = text(m, "bitrate").parse::<u32>().ok();
    let label = if format == "flac" {
        "无损".to_string()
    } else if bitrate == Some(320) {
        "320K".to_string()
    } else {
        match bitrate {
            Some(b) => format!("{b}K"),
            None => format.to_uppercase(),
        }
    };
    // "57.26Mb" / "3.18Mb" → 字节（parse_size 给的就是 i64，和 Quality::size 一致）
    let size = m["size"].as_str().and_then(source::parse_size);
    Some(provider::Quality {
        key: match bitrate {
            Some(b) => format!("{format}:{b}"),
            None => format.clone(),
        },
        label,
        format,
        size,
        // 码率已经写进 key / label；Quality.bitrate 的整数宽度在文档里没标注，留空更稳。
        // 想让下拉框带上码率，按字段实际类型加一行：bitrate: Some(b as _),
        ..Default::default()
    })
}

/// 档位排序权重：FLAC 优先，其次按码率。站点给的 minfo 是从低到高，要反过来
fn rank(q: &provider::Quality) -> (u8, u32) {
    let fmt = q.format.to_ascii_lowercase();
    // 码率从 key（`格式:码率`）里取，不依赖 Quality.bitrate 的整数类型
    let br = q
        .key
        .split_once(':')
        .and_then(|(_, n)| n.parse::<u32>().ok())
        .unwrap_or(0);
    (if fmt == "flac" { 1 } else { 0 }, br)
}

/// 关键词前缀切音源：`wyy:` / `网易:` 走网易云，其余走酷我
fn split_platform(query: &str) -> (&'static str, String) {
    const PREFIXES: [(&str, &str); 7] = [
        ("wyy:", "wyy"),
        ("wy:", "wyy"),
        ("网易云:", "wyy"),
        ("网易:", "wyy"),
        ("kuwo:", "kuwo"),
        ("kw:", "kuwo"),
        ("酷我:", "kuwo"),
    ];
    let q = query.trim();
    for (pfx, plat) in PREFIXES {
        if let Some(rest) = q.strip_prefix(pfx) {
            let rest = rest.trim();
            if !rest.is_empty() {
                return (plat, rest.to_string());
            }
        }
    }
    ("kuwo", q.to_string())
}

// ---------- 转换 ----------

/// 把一条搜索结果转成 `Item`（`extra` 里带上 resolve 需要的平台、曲号、时间戳、签名）
pub fn item_of(s: &Value, platform: &str) -> Option<provider::Item> {
    let id = id_of(s);
    if id.is_empty() {
        return None;
    }
    let mut qualities: Vec<provider::Quality> = s["minfo"]
        .as_array()
        .map(|a| a.iter().filter_map(quality_of).collect())
        .unwrap_or_default();
    // 框架约定「从好到差排，第一项是默认档位」；站点给的是 mp3→flac 从低到高
    qualities.sort_by(|a, b| rank(b).cmp(&rank(a)));
    // 站点偶尔不给 minfo，退化成默认 MP3 档位，至少能试一次
    if qualities.is_empty() {
        qualities.push(provider::Quality {
            key: "mp3".into(),
            label: "MP3".into(),
            format: "mp3".into(),
            ..Default::default()
        });
    }
    let time = s["time"].as_i64().unwrap_or(0);
    let sign = text(s, "sign");
    Some(provider::Item {
        id: id.clone(),
        title: text(s, "name"),
        artist: text(s, "artist"),
        album: text(s, "album_name"),
        year: None,
        track: None,
        duration: duration_of(s),
        cover: text(s, "pic_url"),
        media_kind: "music".into(),
        qualities,
        extra: json!({
            "platform": platform,
            "songid": id,
            "time": time,
            "sign": sign,
        }),
        ..Default::default()
    })
}

/// 解析档位串得到（format, bitrate）。空串表示用条目里的默认档位
fn pick_quality(item: &provider::Item, quality: &str) -> (String, String) {
    let key = if quality.trim().is_empty() {
        item.qualities
            .first()
            .map(|q| q.key.clone())
            .unwrap_or_else(|| "mp3".to_string())
    } else {
        quality.trim().to_string()
    };
    if let Some((f, b)) = key.split_once(':') {
        return (f.to_string(), b.to_string());
    }
    // 只有格式：补上条目里该格式的最高码率（同样从 key 里取）
    let br = item
        .qualities
        .iter()
        .filter(|q| q.format.eq_ignore_ascii_case(&key))
        .filter_map(|q| {
            q.key
                .split_once(':')
                .and_then(|(_, n)| n.parse::<u32>().ok())
        })
        .max()
        .map(|n| n.to_string())
        .unwrap_or_default();
    (key, br)
}

/// 签名过期时按标题重搜一次，拿回同一首的新 `time` / `sign`
async fn resign(platform: &str, item: &provider::Item) -> Option<(i64, String)> {
    let keyword = if item.title.trim().is_empty() {
        item.artist.trim().to_string()
    } else {
        item.title.trim().to_string()
    };
    if keyword.is_empty() {
        return None;
    }
    let pairs = [
        ("platform", platform),
        ("keyword", keyword.as_str()),
        ("page", "1"),
        ("size", "20"),
    ];
    let v = post("search", &pairs).await.ok()?;
    for s in v["data"]["list"].as_array()? {
        if id_of(s) != item.id {
            continue;
        }
        let time = s["time"].as_i64().unwrap_or(0);
        let sign = text(s, "sign");
        if time > 0 && !sign.is_empty() {
            return Some((time, sign));
        }
    }
    None
}

/// 取歌词；失败返回 None（歌词问题不该让下载失败）
async fn lyric(platform: &str, songid: &str, time: i64, sign: &str) -> Option<String> {
    if sign.is_empty() || time <= 0 {
        return None;
    }
    let time_s = time.to_string();
    let pairs = [
        ("platform", platform),
        ("songid", songid),
        ("time", time_s.as_str()),
        ("sign", sign),
    ];
    let v = post("getLyric", &pairs).await.ok()?;
    v["data"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

/// getUrl 的一次调用（参数都在栈上，避免跨 await 的临时借用）
async fn get_url(
    platform: &str,
    songid: &str,
    format: &str,
    bitrate: &str,
    time: i64,
    sign: &str,
) -> Result<Value, String> {
    let time_s = time.to_string();
    let pairs = [
        ("platform", platform),
        ("songid", songid),
        ("format", format),
        ("bitrate", bitrate),
        ("time", time_s.as_str()),
        ("sign", sign),
    ];
    post("getUrl", &pairs).await
}

// ---------- Provider ----------

impl provider::Provider for OnlineMusic {
    fn info(&self) -> provider::ProviderInfo {
        provider::ProviderInfo {
            id: "onlinemusic".into(),
            name: "在线音乐".into(),
            media_kinds: vec!["music".into()],
            note: "在线搜索并下载音乐，支持有损 MP3 与无损 FLAC，自动附带歌词。"
                .into(),
        }
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        media_kind: &'a str,
        page: u32,
    ) -> BoxFuture<'a, Result<Vec<provider::Item>, String>> {
        Box::pin(async move {
            if !media_kind.is_empty() && media_kind != "music" {
                return Ok(Vec::new());
            }
            let (platform, keyword) = split_platform(query);
            if keyword.is_empty() {
                return Err("请输入歌名".into());
            }
            // 实测：每页固定回 10~15 条，size 给 20 就够，翻页靠 page；关键词只用歌名最准
            let page = page.max(1).to_string();
            let pairs = [
                ("platform", platform),
                ("keyword", keyword.as_str()),
                ("page", page.as_str()),
                ("size", "20"),
            ];
            let v = post("search", &pairs).await?;
            let items: Vec<provider::Item> = v["data"]["list"]
                .as_array()
                .map(|a| a.iter().filter_map(|s| item_of(s, platform)).collect())
                .unwrap_or_default();
            if items.is_empty() {
                let total = v["data"]["total"].as_i64().unwrap_or(0);
                if total > 0 {
                    return Err(format!(
                        "接口说有 {total} 条，但没有返回条目；换个更准确的歌名试试"
                    ));
                }
            }
            Ok(items)
        })
    }

    fn resolve<'a>(
        &'a self,
        item: &'a provider::Item,
        quality: &'a str,
    ) -> BoxFuture<'a, Result<provider::Resolved, String>> {
        Box::pin(async move {
            let platform = item.extra["platform"]
                .as_str()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "kuwo".to_string());
            let songid = item.extra["songid"]
                .as_str()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| item.id.clone());
            if songid.is_empty() {
                return Err("这条结果没有曲目 id，请重新搜索".into());
            }
            let mut time = item.extra["time"].as_i64().unwrap_or(0);
            let mut sign = item.extra["sign"].as_str().unwrap_or("").to_string();

            let (format, bitrate) = pick_quality(item, quality);
            let ext = if format.is_empty() {
                "mp3".to_string()
            } else {
                format.clone()
            };

            let first = get_url(&platform, &songid, &format, &bitrate, time, &sign).await;
            let v = match first {
                Ok(v) => v,
                Err(first_err) => match resign(&platform, item).await {
                    Some((t, s)) => {
                        time = t;
                        sign = s;
                        get_url(&platform, &songid, &format, &bitrate, time, &sign)
                            .await
                            .map_err(|e2| format!("{first_err}；换新签名后仍失败：{e2}"))?
                    }
                    None => return Err(first_err),
                },
            };

            let mut url = text(&v["data"], "url");
            if url.is_empty() {
                if let Some((t, s)) = resign(&platform, item).await {
                    time = t;
                    sign = s;
                    if let Ok(v2) =
                        get_url(&platform, &songid, &format, &bitrate, time, &sign).await
                    {
                        url = text(&v2["data"], "url");
                    }
                }
            }
            if url.is_empty() {
                let shown = if bitrate.is_empty() { "?" } else { bitrate.as_str() };
                return Err(format!("没拿到下载地址（{ext} / {shown}k）"));
            }

            let lrc = lyric(&platform, &songid, time, &sign).await;
            Ok(provider::Resolved {
                target: provider::Target::Http { url, ext },
                lyric: lrc,
            })
        })
    }
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use provider::Provider;

    /// 真实响应节选（2026-11 抓包），只验证字段映射，不联网
    const SEARCH_JSON: &str = r#"{
      "code": 0, "msg": "ok",
      "data": {
        "total": 3514,
        "list": [{
          "id": "394586518",
          "name": "青花瓷",
          "artist": "周杰伦",
          "album_name": "我很忙",
          "pic_url": "https://img1.kuwo.cn/star/albumcover/120/x.jpg",
          "duration": "208",
          "minfo": [
            {"level":"h","bitrate":"128","format":"mp3","size":"3.18Mb"},
            {"level":"p","bitrate":"320","format":"mp3","size":"10.77Mb"},
            {"level":"ff","bitrate":"2000","format":"flac","size":"57.26Mb"}
          ],
          "time": 1795379858,
          "sign": "1b48061220b0747abd3891a4a8122014"
        }]
      }
    }"#;

    fn sample() -> provider::Item {
        let v: Value = serde_json::from_str(SEARCH_JSON).unwrap();
        let list = v["data"]["list"].as_array().unwrap();
        item_of(&list[0], "kuwo").unwrap()
    }

    #[test]
    fn maps_search_result() {
        let it = sample();
        assert_eq!(it.id, "394586518");
        assert_eq!(it.title, "青花瓷");
        assert_eq!(it.artist, "周杰伦");
        assert_eq!(it.album, "我很忙");
        assert_eq!(it.media_kind, "music");
        assert_eq!(it.duration, Some(208.0));
        assert_eq!(it.extra["platform"], "kuwo");
        assert_eq!(it.extra["sign"], "1b48061220b0747abd3891a4a8122014");
        assert_eq!(it.extra["time"], 1795379858i64);
    }

    #[test]
    fn qualities_are_best_first() {
        let it = sample();
        // 站点给的是低→高，转换后按「从好到差」重排，FLAC 成为默认档位
        assert_eq!(it.qualities.len(), 3);
        assert_eq!(it.qualities[0].key, "flac:2000");
        assert_eq!(it.qualities[0].label, "无损");
        assert_eq!(it.qualities[1].key, "mp3:320");
        assert_eq!(it.qualities[1].label, "320K");
        assert_eq!(it.qualities[2].key, "mp3:128");
        assert_eq!(it.qualities[2].label, "128K");
        // "57.26Mb" 解析成字节
        assert!(it.qualities[0].size.unwrap() > 50_000_000);
    }

    #[test]
    fn quality_key_round_trip() {
        let it = sample();
        assert_eq!(
            pick_quality(&it, "flac:2000"),
            ("flac".to_string(), "2000".to_string())
        );
        assert_eq!(
            pick_quality(&it, "mp3:320"),
            ("mp3".to_string(), "320".to_string())
        );
        // 只给格式：补上条目里该格式的最高码率
        assert_eq!(
            pick_quality(&it, "mp3"),
            ("mp3".to_string(), "320".to_string())
        );
        // 空串：用默认（第一档 = FLAC）
        assert_eq!(
            pick_quality(&it, ""),
            ("flac".to_string(), "2000".to_string())
        );
    }

    #[test]
    fn platform_prefix_switches_source() {
        assert_eq!(split_platform("青花瓷"), ("kuwo", "青花瓷".to_string()));
        assert_eq!(split_platform("wyy:青花瓷"), ("wyy", "青花瓷".to_string()));
        assert_eq!(
            split_platform("网易: 青花瓷 "),
            ("wyy", "青花瓷".to_string())
        );
        assert_eq!(
            split_platform("wyy:"),
            ("kuwo", "wyy:".to_string()),
            "只有前缀时不该把关键词吞掉"
        );
        assert_eq!(split_platform("  十年 "), ("kuwo", "十年".to_string()));
    }

    #[test]
    fn missing_minfo_falls_back_to_mp3() {
        let s: Value = serde_json::json!({"id": "1", "name": "x", "time": 1, "sign": "s"});
        let it = item_of(&s, "kuwo").unwrap();
        assert_eq!(it.qualities.len(), 1);
        assert_eq!(it.qualities[0].key, "mp3");
    }

    #[test]
    fn hit_and_file_name() {
        let it = sample();
        let h = provider::to_hit("onlinemusic", &it);
        assert_eq!(h.kind, "provider");
        assert_eq!(h.title, "青花瓷 - 周杰伦 [无损]");
        assert_eq!(h.work, "我很忙");
        assert_eq!(h.group.as_deref(), Some("周杰伦"));
        assert_eq!(provider::file_name(&it, "flac"), "青花瓷 - 周杰伦.flac");
        // 任务地址往返后 extra 仍在（重启继续下载靠这个）
        let (id, back, q) = provider::parse_task_url(&h.url).unwrap();
        assert_eq!(id, "onlinemusic");
        assert_eq!(q, "flac:2000");
        assert_eq!(back.extra["sign"], "1b48061220b0747abd3891a4a8122014");
    }

    #[test]
    fn info_is_well_formed() {
        let i = OnlineMusic.info();
        assert_eq!(i.id, "onlinemusic");
        assert!(i
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        assert_eq!(i.media_kinds, vec!["music".to_string()]);
        assert!(!i.note.trim().is_empty());
    }
}
