//! mpv 播放引擎（打开 `mpv` 特性时编译）
//! - 音乐：一个无画面的 mpv 实例，状态以 `mpv://state` 事件推给前端
//! - 视频：网页播放器解码不了的格式（HEVC、RMVB 等），交给独立的 mpv 窗口播放（自带控制条和快捷键），关闭时回报播放位置
use crate::{library, AppState};
use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

type R<T> = Result<T, String>;
fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

static AUDIO: OnceCell<Arc<Mpv>> = OnceCell::new();

#[derive(Serialize, Clone, Default)]
pub struct MpvState {
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    pub idle: bool,
}

/// 10 段均衡器的 FFmpeg 滤镜
pub fn eq_filter(gains: &[f64]) -> String {
    const F: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
    if gains.iter().all(|g| g.abs() < 0.01) {
        return String::new();
    }
    let parts: Vec<String> = F.iter().zip(gains).map(|(f, g)| format!("equalizer=f={f}:t=o:w=1:g={g}")).collect();
    format!("lavfi=[{}]", parts.join(","))
}

fn audio(app: &AppHandle) -> R<Arc<Mpv>> {
    AUDIO
        .get_or_try_init(|| {
            let mpv = Mpv::with_initializer(|i| {
                i.set_option("vo", "null")?;
                i.set_option("video", "no")?;
                i.set_option("idle", "yes")?;
                i.set_option("gapless-audio", "weak")?;
                i.set_option("audio-display", "no")?;
                Ok(())
            })
            .map_err(e)?;
            let mpv = Arc::new(mpv);
            {
                let st = app.state::<AppState>();
                let s = library::get_setting(&st.db.lock(), "audio").ok().flatten().unwrap_or_default();
                if s["exclusive"].as_bool() == Some(true) {
                    let _ = mpv.set_property("audio-exclusive", "yes");
                }
                if s["replaygain"].as_bool() == Some(true) {
                    let _ = mpv.set_property("replaygain", "track");
                }
            }
            mpv.observe_property("time-pos", Format::Double, 1).map_err(e)?;
            mpv.observe_property("duration", Format::Double, 2).map_err(e)?;
            mpv.observe_property("pause", Format::Flag, 3).map_err(e)?;
            let ev = mpv.clone();
            let app2 = app.clone();
            std::thread::spawn(move || {
                let mut s = MpvState { paused: true, idle: true, ..Default::default() };
                let mut last = std::time::Instant::now();
                loop {
                    match ev.wait_event(0.25) {
                        Some(Ok(Event::PropertyChange { name, change, .. })) => {
                            match (name, change) {
                                ("time-pos", PropertyData::Double(t)) => s.time = t,
                                ("duration", PropertyData::Double(d)) => s.duration = d,
                                ("pause", PropertyData::Flag(p)) => s.paused = p,
                                _ => {}
                            }
                            if last.elapsed().as_millis() >= 100 {
                                let _ = app2.emit("mpv://state", &s);
                                last = std::time::Instant::now();
                            }
                        }
                        Some(Ok(Event::StartFile)) => s.idle = false,
                        Some(Ok(Event::EndFile(reason))) => {
                            // 0 = 播完；其他（停止、出错、换歌）不算播完
                            let _ = app2.emit("mpv://end", reason as i64 == 0);
                        }
                        Some(Ok(Event::Shutdown)) => return,
                        Some(Err(err)) => {
                            let _ = app2.emit("mpv://error", err.to_string());
                        }
                        _ => {}
                    }
                }
            });
            Ok::<_, String>(mpv)
        })
        .cloned()
}

#[tauri::command]
pub fn mpv_load(app: AppHandle, path: String, start: Option<f64>, paused: Option<bool>) -> R<()> {
    let m = audio(&app)?;
    m.set_property("pause", paused.unwrap_or(false)).map_err(e)?;
    let opts = start.filter(|s| *s > 0.0).map(|s| format!("start={s}")).unwrap_or_default();
    if opts.is_empty() {
        m.command("loadfile", &[&path, "replace"]).map_err(e)
    } else {
        m.command("loadfile", &[&path, "replace", "-1", &opts]).map_err(e)
    }
}

#[tauri::command]
pub fn mpv_pause(app: AppHandle, paused: bool) -> R<()> {
    audio(&app)?.set_property("pause", paused).map_err(e)
}

#[tauri::command]
pub fn mpv_stop(app: AppHandle) -> R<()> {
    audio(&app)?.command("stop", &[]).map_err(e)
}

#[tauri::command]
pub fn mpv_seek(app: AppHandle, time: f64) -> R<()> {
    audio(&app)?.command("seek", &[&format!("{time}"), "absolute"]).map_err(e)
}

#[tauri::command]
pub fn mpv_volume(app: AppHandle, volume: f64) -> R<()> {
    audio(&app)?.set_property("volume", (volume * 100.0).clamp(0.0, 100.0)).map_err(e)
}

#[tauri::command]
pub fn mpv_speed(app: AppHandle, speed: f64) -> R<()> {
    audio(&app)?.set_property("speed", speed).map_err(e)
}

#[tauri::command]
pub fn mpv_eq(app: AppHandle, gains: Vec<f64>) -> R<()> {
    audio(&app)?.set_property("af", eq_filter(&gains)).map_err(e)
}

/// 音频选项：audio-exclusive、replaygain 等
#[tauri::command]
pub fn mpv_set_option(app: AppHandle, name: String, value: String) -> R<()> {
    audio(&app)?.set_property(&name, value).map_err(e)
}

static VIDEO_OPEN: Mutex<bool> = Mutex::new(false);

/// 用独立的 mpv 窗口播放视频；关闭窗口后回报位置并保存进度
#[tauri::command]
pub fn mpv_play_video(app: AppHandle, st: State<AppState>, id: i64, start: Option<f64>) -> R<()> {
    let path = library::file_path(&st.db.lock(), id).map_err(e)?;
    {
        let mut open = VIDEO_OPEN.lock();
        if *open {
            return Err("已经有一个 mpv 视频窗口在播放".into());
        }
        *open = true;
    }
    // 暂停音乐
    if let Some(a) = AUDIO.get() {
        let _ = a.set_property("pause", true);
    }
    let title = std::path::Path::new(&path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    std::thread::spawn(move || {
        let run = || -> Result<(f64, f64), String> {
            let mpv = Mpv::with_initializer(|i| {
                i.set_option("vo", "gpu-next")?;
                i.set_option("hwdec", "auto-safe")?;
                i.set_option("osc", "yes")?;
                i.set_option("input-default-bindings", "yes")?;
                i.set_option("input-vo-keyboard", "yes")?;
                i.set_option("keep-open", "yes")?;
                i.set_option("force-window", "yes")?;
                i.set_option("autofit", "80%x80%")?;
                i.set_option("sub-auto", "fuzzy")?;
                i.set_option("title", format!("{title} - 极光播放器").as_str())?;
                Ok(())
            })
            .map_err(e)?;
            mpv.observe_property("time-pos", Format::Double, 1).map_err(e)?;
            mpv.observe_property("duration", Format::Double, 2).map_err(e)?;
            let opts = start.filter(|s| *s > 5.0).map(|s| format!("start={s}")).unwrap_or_default();
            let quoted = path.clone();
            if opts.is_empty() {
                mpv.command("loadfile", &[&quoted]).map_err(e)?;
            } else {
                mpv.command("loadfile", &[&quoted, "replace", "-1", &opts]).map_err(e)?;
            }
            let (mut t, mut d) = (0.0, 0.0);
            loop {
                match mpv.wait_event(1.0) {
                    Some(Ok(Event::PropertyChange { name: "time-pos", change: PropertyData::Double(v), .. })) => t = v,
                    Some(Ok(Event::PropertyChange { name: "duration", change: PropertyData::Double(v), .. })) => d = v,
                    Some(Ok(Event::Shutdown)) => break,
                    _ => {}
                }
            }
            Ok((t, d))
        };
        let r = run();
        *VIDEO_OPEN.lock() = false;
        match r {
            Ok((t, d)) => {
                let st = app.state::<AppState>();
                let _ = library::save_video_progress(&st.db.lock(), id, t, d);
                let _ = app.emit("mpv://video-closed", (id, t, d));
            }
            Err(err) => {
                let _ = app.emit("mpv://error", err);
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn eq_flat_is_empty() {
        assert_eq!(super::eq_filter(&[0.0; 10]), "");
        assert!(super::eq_filter(&[3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).contains("f=31:t=o:w=1:g=3"));
    }
}
