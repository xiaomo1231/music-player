//! AI 辅助归类：规则拿不准的视频（独立视频、没认出集号的剧集），在配置了 AI 服务时批量交给文本模型判断。
//! 只发路径里最后几级文件夹名和文件名，不发文件内容。结果存在 ai_class 表里，
//! 以后重新扫描、归类规则升级时都会沿用，同一个文件不会再发第二次。
use crate::{ai, indexer, parse, AppState};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ai_class(
           path TEXT PRIMARY KEY,
           kind TEXT NOT NULL,        -- anime / tv / movie / video
           title TEXT NOT NULL DEFAULT '',
           season INTEGER, episode REAL,
           at INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

pub fn setup(app: &AppHandle) {
    // 启动后在后台把还没判断过的条目跑一遍（没配置 AI 或关掉了就什么都不做）
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run(&app).await;
    });
}

/// AI 给出的归类
#[derive(Debug, Clone, PartialEq)]
pub struct AiClass {
    pub kind: String,
    pub title: String,
    pub season: Option<i64>,
    pub episode: Option<f64>,
}

pub fn get(conn: &Connection, path: &str) -> Option<AiClass> {
    conn.query_row("SELECT kind,title,season,episode FROM ai_class WHERE path=?1", [path], |r| {
        Ok(AiClass {
            kind: r.get(0)?,
            title: r.get(1)?,
            season: r.get(2)?,
            episode: r.get(3)?,
        })
    })
    .optional()
    .ok()
    .flatten()
}

/// 用 AI 的结果改写规则的归类；没有 AI 结果时原样返回
pub fn apply(conn: &Connection, path: &str, mut vc: parse::VideoClass) -> parse::VideoClass {
    let Some(a) = get(conn, path) else { return vc };
    let title = a.title.trim();
    match a.kind.as_str() {
        "anime" | "tv" => {
            vc.kind = if a.kind == "anime" { "anime" } else { "video" };
            vc.series = true;
            vc.movie = false;
            vc.movie_guess = false;
            if !title.is_empty() {
                vc.info.title = title.to_string();
            }
            if a.season.is_some() {
                vc.info.season = a.season;
            }
            if a.episode.is_some() {
                vc.info.episode = a.episode;
            }
            vc.info.special = vc.info.season == Some(0);
        }
        "movie" => {
            vc.kind = "video";
            vc.series = false;
            vc.movie = true;
            vc.movie_guess = false;
            if !title.is_empty() {
                vc.info.title = title.to_string();
            }
        }
        "video" => {
            vc.kind = "video";
            vc.series = false;
            vc.movie = false;
            vc.movie_guess = false;
        }
        _ => {}
    }
    vc
}

fn enabled(conn: &Connection) -> bool {
    ai::load_config(conn).classify_media
}

/// 规则拿不准的视频：独立视频（可能是没认出来的番剧 / 电影），以及在剧集里却没有集号的
fn candidates(conn: &Connection, limit: i64) -> Result<Vec<(i64, String, String, String)>> {
    Ok(conn
        .prepare(
            "SELECT f.id,f.path,
                    CASE WHEN v.movie=1 THEN 'movie' WHEN v.series_id IS NULL THEN 'video' WHEN f.kind='anime' THEN 'anime' ELSE 'tv' END,
                    coalesce(s.title,v.title)
             FROM files f JOIN videos v ON v.file_id=f.id LEFT JOIN series s ON s.id=v.series_id LEFT JOIN ai_class a ON a.path=f.path
             WHERE f.missing=0 AND f.kind IN ('video','anime') AND a.path IS NULL
               AND ((v.series_id IS NULL AND v.movie=0) OR (v.series_id IS NOT NULL AND v.episode IS NULL AND coalesce(v.season,1)<>0))
             ORDER BY f.path LIMIT ?1",
        )?
        .query_map([limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

/// 只留最后 4 级（3 级文件夹 + 文件名），不带盘符和用户名
pub fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.split(['/', '\\']).filter(|x| !x.is_empty()).collect();
    let n = parts.len();
    parts[n.saturating_sub(4)..].join("/")
}

const SYSTEM: &str = r#"你是本地媒体库的整理助手。用户给出一批视频文件的路径（只有最后几级文件夹名和文件名）以及程序按规则猜的归类。
请判断每个文件是什么，并给出整部作品的名字：
- type：anime（日本动画、国产动画等番剧，包括动画剧场版、OVA）、tv（真人电视剧 / 综艺 / 纪录片剧集）、movie（真人电影）、video（其他：录屏、短视频、MV、教程等）
- title：整部作品的名字。同一部作品的所有集（含剧场版、特典）必须给完全相同的名字；作品文件夹名是作品名时优先用文件夹名；程序猜的名字没错就照抄。type 为 video 时可留空
- season：季号，特典 / 剧场版 / OP / ED 用 0，不知道用 null
- episode：集号（数字，上 / 下部用 1 / 2），电影或不知道用 null
只输出 JSON：{"items":[{"id":数字,"type":"…","title":"…","season":数字或null,"episode":数字或null}]}"#;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// 把拿不准的条目分批发给 AI，结果写进媒体库。返回处理了多少个文件
pub async fn run(app: &AppHandle) -> Result<usize> {
    {
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        if !enabled(&conn) {
            return Ok(0);
        }
    }
    if ai::ready(app).is_err() || RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(0);
    }
    let r = run_batches(app).await;
    RUNNING.store(false, Ordering::SeqCst);
    if matches!(r, Ok(n) if n > 0) {
        let _ = app.emit("library://changed", ());
    }
    r
}

async fn run_batches(app: &AppHandle) -> Result<usize> {
    let mut done = 0;
    // 一次最多处理 400 个，剩下的下次扫描 / 启动时继续
    for _ in 0..10 {
        let batch = {
            let st = app.state::<AppState>();
            let conn = st.db.lock();
            candidates(&conn, 40)?
        };
        if batch.is_empty() {
            break;
        }
        let items: Vec<_> = batch
            .iter()
            .map(|(id, p, kind, title)| json!({"id": id, "path": short_path(p), "guess": {"type": kind, "title": title}}))
            .collect();
        let user = serde_json::to_string(&json!({ "files": items }))?;
        let reply = ai::chat_simple(app, SYSTEM, &user, true).await?;
        let answers = parse_reply(&reply);
        let st = app.state::<AppState>();
        let conn = st.db.lock();
        for (id, path, guess_kind, _) in &batch {
            // AI 没回答的按原样记下，免得反复发送
            let a = answers.iter().find(|(i, _)| i == id).map(|(_, a)| a.clone()).unwrap_or(AiClass {
                kind: guess_kind.clone(),
                title: String::new(),
                season: None,
                episode: None,
            });
            save(&conn, path, &a)?;
            indexer::reapply_video(&conn, *id, path)?;
        }
        done += batch.len();
    }
    Ok(done)
}

fn save(conn: &Connection, path: &str, a: &AiClass) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO ai_class(path,kind,title,season,episode,at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![path, a.kind, a.title, a.season, a.episode, crate::db::now()],
    )?;
    Ok(())
}

pub fn parse_reply(reply: &str) -> Vec<(i64, AiClass)> {
    let Ok(v) = ai::extract_json(reply) else { return vec![] };
    let arr = v["items"].as_array().or_else(|| v.as_array()).cloned().unwrap_or_default();
    arr.iter()
        .filter_map(|x| {
            let id = x["id"].as_i64()?;
            let kind = x["type"].as_str()?.to_lowercase();
            if !["anime", "tv", "movie", "video"].contains(&kind.as_str()) {
                return None;
            }
            Some((
                id,
                AiClass {
                    kind,
                    title: x["title"].as_str().unwrap_or("").trim().to_string(),
                    season: x["season"].as_i64(),
                    episode: x["episode"].as_f64(),
                },
            ))
        })
        .collect()
}

/// 设置里“用 AI 识别”按钮：立即跑一遍
#[tauri::command]
pub async fn ai_classify_now(app: AppHandle) -> Result<usize, String> {
    ai::ready(&app).map_err(|e| e.to_string())?;
    run(&app).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths() {
        assert_eq!(short_path(r"F:\番\孤独摇滚！\01.mp4"), "F:/番/孤独摇滚！/01.mp4");
        assert_eq!(short_path(r"C:\Users\name\Videos\a\b\c.mp4"), "Videos/a/b/c.mp4");
    }

    #[test]
    fn reply_parsing() {
        let r = parse_reply(
            r#"{"items":[{"id":3,"type":"anime","title":"孤独摇滚","season":1,"episode":2},{"id":4,"type":"x"},{"id":5,"type":"Movie","title":"阿甘正传","season":null,"episode":null}]}"#,
        );
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].1,
            AiClass {
                kind: "anime".into(),
                title: "孤独摇滚".into(),
                season: Some(1),
                episode: Some(2.0)
            }
        );
        assert_eq!(r[1].1.kind, "movie");
        assert!(parse_reply("不是 JSON").is_empty());
    }

    #[test]
    fn override_and_reapply() {
        let c = crate::db::open_memory().unwrap();
        init(&c).unwrap();
        let p = r"D:\Videos\杂\[XYZ] 某番 第3集.mp4";
        c.execute("INSERT INTO files(path,kind,size,mtime,added_at) VALUES(?1,'video',1,1,1)", [p])
            .unwrap();
        let id = c.last_insert_rowid();
        c.execute("INSERT INTO videos(file_id,title,position) VALUES(?1,'x',50)", [id]).unwrap();
        crate::indexer::reapply_video(&c, id, p).unwrap();
        save(
            &c,
            p,
            &AiClass {
                kind: "anime".into(),
                title: "某番".into(),
                season: Some(1),
                episode: Some(3.0),
            },
        )
        .unwrap();
        crate::indexer::reapply_video(&c, id, p).unwrap();
        let (kind, title, ep, pos): (String, String, f64, f64) = c
            .query_row(
                "SELECT f.kind,s.title,v.episode,v.position FROM files f JOIN videos v ON v.file_id=f.id JOIN series s ON s.id=v.series_id WHERE f.id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((kind.as_str(), title.as_str(), ep, pos), ("anime", "某番", 3.0, 50.0));
        // 已经问过的不再进候选
        assert!(candidates(&c, 10).unwrap().is_empty());
    }
}
