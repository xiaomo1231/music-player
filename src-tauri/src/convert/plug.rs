//! 插件转换器的任务：协议 2 的 `convert` 方法，或清单里的 `exec` 命令模板。
//! 输出先写到输出目录下的 `.mpc-convert/<任务 id>.<扩展名>`，成功后和内置预设走同样的“完成后”处理
use super::{file_name, finalize_new, finalize_replace, report, run_child, target_dir, ConvertSettings, Guard, Job, JobCtl, CANCEL_MSG};
use crate::fetch::provider::{self, StopFn};
use crate::plugin::converter;
use crate::plugin::manifest::{self, ConverterDecl};
use crate::plugin::process::{self, Call, Event, Line};
use crate::{parse, tools, AppState};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

type R<T> = Result<T, String>;

/// 插件转换任务的 preset：`plg:<插件>/<转换器>:<输出扩展名或 same>`
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub plugin: String,
    pub conv: String,
    pub ext: String,
}

impl Spec {
    pub fn parse(preset: &str) -> Option<Spec> {
        let rest = preset.strip_prefix("plg:")?;
        let (pc, ext) = rest.rsplit_once(':')?;
        let (plugin, conv) = pc.split_once('/')?;
        if plugin.is_empty() || conv.is_empty() || ext.is_empty() {
            return None;
        }
        Some(Spec {
            plugin: plugin.into(),
            conv: conv.into(),
            ext: ext.into(),
        })
    }

    /// 队列里显示的名字
    pub fn label(&self) -> String {
        converter::label(&self.plugin, &self.conv, &self.ext).unwrap_or_else(|| format!("插件 {} → {}", self.plugin, self.ext.to_uppercase()))
    }
}

pub fn accepts(inputs: &[String], ext: &str) -> bool {
    manifest::accepts_ext(inputs, ext)
}

/// 去掉错误码前缀 `[xxx] `
fn plain(e: &str) -> String {
    provider::split_code(e).1.to_string()
}

fn library_kind(ext: &str) -> Option<&'static str> {
    parse::classify(Path::new(&format!("x.{ext}")))
}

/// 加入队列前检查：转换器可用、每个文件的格式都支持、输出能不能入库；返回检查好的参数
pub fn check_add(spec: &Spec, paths: &[String], after: &str, options: &Map<String, Value>) -> R<Map<String, Value>> {
    let (_, decl) = converter::find(&spec.plugin, &spec.conv)?;
    if let Some(v) = converter::active().into_iter().find(|v| v.plugin == spec.plugin && v.id == spec.conv) {
        if let Some(n) = v.needs_settings.first() {
            return Err(format!("先在“设置 → 插件 → {}”里填写“{n}”", v.plugin_name));
        }
    }
    if !decl.output_list().iter().any(|o| o.ext == spec.ext) {
        return Err(format!("这个转换器不能输出 {}", spec.ext));
    }
    let mut bad = vec![];
    let mut no_lib = vec![];
    for p in paths {
        let pb = PathBuf::from(p);
        let e = parse::ext(&pb);
        if !pb.is_file() || !decl.accepts(&e) {
            bad.push(p.clone());
            continue;
        }
        let out = ConverterDecl::resolve_ext(&spec.ext, &e);
        if after != "dir" && library_kind(&out).is_none() && !no_lib.contains(&out) {
            no_lib.push(out);
        }
    }
    if !bad.is_empty() {
        return Err(format!("这个转换器不支持以下文件：\n{}", bad.join("\n")));
    }
    if !no_lib.is_empty() {
        let list: Vec<String> = no_lib.iter().map(|e| e.to_uppercase()).collect();
        return Err(format!("{} 无法加入媒体库，请选“只输出到指定目录”", list.join(" / ")));
    }
    converter::normalize_options(&decl, options)
}

fn load_options(st: &AppState, id: i64) -> Map<String, Value> {
    let conn = st.db.lock();
    conn.query_row("SELECT options FROM convert_plugin_jobs WHERE job_id=?1", [id], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// 插件返回的进度：有 total 时 done / total，否则 done 本身是 0–1 的比例
pub fn ratio(p: &provider::Progress) -> f64 {
    match p.total {
        Some(t) if t > 0.0 => (p.done / t).clamp(0.0, 0.99),
        _ if (0.0..=1.0).contains(&p.done) => p.done.min(0.99),
        _ => 0.0,
    }
}

/// 移动文件；跨盘时复制后删除
fn move_file(from: &Path, to: &Path) -> R<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| format!("移动输出文件失败：{e}"))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// 文件夹里扩展名对得上的文件（只有一个文件时不看扩展名）
fn pick_output(dir: &Path, ext: &str) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    if files.len() == 1 {
        return files.into_iter().next();
    }
    let mut hits: Vec<PathBuf> = files.into_iter().filter(|p| parse::ext(p) == ext).collect();
    hits.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
    hits.into_iter().next()
}

struct Ctx<'a> {
    app: &'a AppHandle,
    ctl: &'a Arc<JobCtl>,
    id: i64,
    plugin: &'a str,
    decl: &'a ConverterDecl,
    rt: &'a crate::plugin::Runtime,
    input: &'a Path,
    in_ext: &'a str,
    out_ext: &'a str,
    tmp: &'a Path,
    out_dir: &'a Path,
    work: &'a Path,
    options: &'a Map<String, Value>,
    dur: f64,
}

/// 进度节流：最多 300 毫秒报一次
struct Throttle {
    last: Instant,
}

impl Throttle {
    fn new() -> Self {
        Throttle {
            last: Instant::now() - Duration::from_secs(1),
        }
    }
    fn on(&mut self, c: &Ctx, p: &provider::Progress) {
        if self.last.elapsed() >= Duration::from_millis(300) {
            self.last = Instant::now();
            report(c.app, c.id, ratio(p), p.speed.filter(|x| x.is_finite() && *x > 0.0), "running", None, None);
        }
    }
}

pub fn execute(app: &AppHandle, ctl: &Arc<JobCtl>, id: i64, job: &Job, settings: &ConvertSettings, spec: &Spec) -> R<String> {
    let st = app.state::<AppState>();
    let input = PathBuf::from(&job.input);
    let (pname, decl) = converter::find(&spec.plugin, &spec.conv)?;
    let in_ext = parse::ext(&input);
    if !decl.accepts(&in_ext) {
        return Err(format!("插件“{pname}”不支持 .{in_ext} 文件"));
    }
    let out_ext = ConverterDecl::resolve_ext(&spec.ext, &in_ext);
    let out_kind = library_kind(&out_ext);
    if job.after != "dir" && out_kind.is_none() {
        return Err(format!("{} 无法加入媒体库，请选“只输出到指定目录”", out_ext.to_uppercase()));
    }
    let options = converter::normalize_options(&decl, &load_options(&st, id))?;
    let rt = crate::plugin::runtime(&spec.plugin).map_err(|e| plain(&e))?;

    let work = Guard(st.dirs.data.join("convert-work").join(id.to_string()));
    let _ = std::fs::remove_dir_all(&work.0);
    let out_dir = work.0.join("out");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("无法创建临时目录：{e}"))?;
    let audio = !parse::VIDEO.contains(&out_ext.as_str()) && !parse::IMAGE.contains(&out_ext.as_str());
    let target = target_dir(job, settings, &input, audio, &st.dirs.data)?;
    std::fs::create_dir_all(&target).map_err(|e| format!("无法创建输出目录：{e}"))?;
    let tmp_dir = target.join(".mpc-convert");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("无法创建临时目录：{e}"))?;
    let tmp = Guard(tmp_dir.join(file_name(&id.to_string(), &out_ext)));
    let _ = std::fs::remove_file(&tmp.0);

    let media_in = matches!(parse::classify(&input), Some("music") | Some("video") | Some("anime"));
    let dur = if media_in {
        tools::probe(&input).map(|p| super::duration_of(&p)).unwrap_or(0.0)
    } else {
        0.0
    };
    report(app, id, 0.0, None, "running", None, None);
    let c = Ctx {
        app,
        ctl,
        id,
        plugin: &spec.plugin,
        decl: &decl,
        rt: &rt,
        input: &input,
        in_ext: &in_ext,
        out_ext: &out_ext,
        tmp: &tmp.0,
        out_dir: &out_dir,
        work: &work.0,
        options: &options,
        dur,
    };
    let started = Instant::now();
    let r = if decl.exec.is_empty() { run_protocol(&c) } else { run_exec(&c) };
    if ctl.canceled() {
        return Err(CANCEL_MSG.into());
    }
    match &r {
        Ok(()) => crate::plugin::log(&spec.plugin, "convert", "call", &format!("完成，用时 {} 秒", started.elapsed().as_secs())),
        Err(e) => crate::plugin::log(&spec.plugin, "convert", "error", e),
    }
    r?;
    // 只给了输出文件夹的程序：从里面挑出结果
    if !tmp.0.is_file() {
        if let Some(f) = pick_output(&out_dir, &out_ext) {
            move_file(&f, &tmp.0)?;
        }
    }
    let size = std::fs::metadata(&tmp.0).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Err(format!("插件“{pname}”没有生成输出文件"));
    }
    report(app, id, 0.99, None, "running", None, None);
    let media_out = matches!(out_kind, Some("music") | Some("video") | Some("anime"));
    if job.after == "replace" {
        let d = if decl.changes_duration { 0.0 } else { dur };
        finalize_replace(&st, id, &input, &tmp.0, &out_ext, d, media_out)
    } else {
        finalize_new(&st, &job.after, &input, &tmp.0, &out_ext, &target)
    }
}

/// 协议 2：`<命令> convert`
fn run_protocol(c: &Ctx) -> R<()> {
    let call = Call {
        argv: c.rt.argv.clone(),
        dir: c.rt.dir.clone(),
        env: c.rt.env.clone(),
        method: "convert".into(),
        params: json!({
            "converter": c.decl.id,
            "input": c.input,
            "output": c.tmp,
            "input_ext": c.in_ext,
            "output_ext": c.out_ext,
            "options": c.options,
            "work_dir": c.work,
            "duration": if c.dur > 0.0 { json!(c.dur) } else { Value::Null },
        }),
        settings: c.rt.settings.clone(),
        context: c.rt.context.clone(),
        timeout: Some(Duration::from_secs(24 * 3600)),
        idle: None,
    };
    let ctl = c.ctl.clone();
    let stop: StopFn = Arc::new(move || ctl.canceled());
    let mut th = Throttle::new();
    let r = tauri::async_runtime::block_on(process::run(
        &call,
        |e| match e {
            Event::Log { level, msg } => crate::plugin::log(c.plugin, "convert", &level, &msg),
            Event::Progress(p) => th.on(c, &p),
        },
        Some(stop),
    ));
    let v = r.map_err(|e| plain(&e))?;
    // 插件可以把结果写在别处（工作目录里），用 output 告诉软件
    if let Some(p) = v.get("output").and_then(|x| x.as_str()).map(PathBuf::from) {
        if p != c.tmp {
            let inside = p.starts_with(c.work) || p.parent() == c.tmp.parent();
            if !inside || !p.is_file() {
                return Err("插件返回的输出文件不在软件给的位置".into());
            }
            move_file(&p, c.tmp)?;
        }
    }
    Ok(())
}

/// 命令模板里一个占位符的值
fn var(c: &Ctx, name: &str) -> Option<String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let tool = |t: &str| -> Option<String> {
        if !c.rt.granted.tools.iter().any(|g| g == t) {
            return None;
        }
        let p = if t == "ffmpeg" { tools::ffmpeg() } else { tools::ffprobe() };
        p.map(|p| p.to_string_lossy().into_owned())
    };
    if let Some(k) = name.strip_prefix("option:") {
        return Some(converter::value_text(c.options.get(k).unwrap_or(&Value::Null)));
    }
    if let Some(k) = name.strip_prefix("setting:") {
        return Some(converter::value_text(c.rt.settings.get(k).unwrap_or(&Value::Null)));
    }
    Some(match name {
        "input" => s(c.input),
        "output" => s(c.tmp),
        "output_dir" => s(c.out_dir),
        "input_ext" => c.in_ext.to_string(),
        "output_ext" => c.out_ext.to_string(),
        "work_dir" => s(c.work),
        "plugin_dir" => s(&c.rt.dir),
        "ffmpeg" | "ffprobe" => return tool(name),
        _ => return None,
    })
}

/// 填好命令模板：第一项是程序（相对路径按插件文件夹找）
pub fn build_exec(template: &[String], plugin_dir: &Path, get: &dyn Fn(&str) -> Option<String>) -> R<(PathBuf, Vec<String>)> {
    let mut args = vec![];
    for a in template {
        args.push(manifest::fill_exec(a, get)?);
    }
    if args.is_empty() || args[0].trim().is_empty() {
        return Err("exec 没有写要运行的程序".into());
    }
    let first = args.remove(0);
    let raw = &template[0];
    let local = !raw.starts_with('{') && (raw.contains(['/', '\\']) || Path::new(raw).extension().is_some());
    let prog = if local { plugin_dir.join(&first) } else { PathBuf::from(&first) };
    Ok((prog, args))
}

/// exec：直接运行插件里的命令行程序。标准输出里的协议 2 事件行（progress / log）照样认
fn run_exec(c: &Ctx) -> R<()> {
    let (prog, args) = build_exec(&c.decl.exec, &c.rt.dir, &|n| var(c, n))?;
    let mut cmd = tools::command(&prog);
    cmd.args(&args).current_dir(c.work);
    for (k, v) in &c.rt.env {
        cmd.env(k, v);
    }
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    let mut th = Throttle::new();
    let mut lines = 0usize;
    let (ok, tail) = run_child(c.ctl, cmd, &mut |l: &str| match process::parse_line(l) {
        Line::Event(Event::Progress(p)) => th.on(c, &p),
        Line::Event(Event::Log { level, msg }) => {
            lines += 1;
            if lines <= 200 {
                crate::plugin::log(c.plugin, "convert", &level, &msg);
            }
        }
        _ => {}
    })
    .map_err(|e| format!("{e}（{}）", prog.display()))?;
    for l in tail.lines() {
        crate::plugin::log(c.plugin, "convert", "stderr", l);
    }
    if !ok {
        return Err(if tail.trim().is_empty() {
            "转换程序出错（退出码不是 0）".into()
        } else {
            format!("转换程序出错：{tail}")
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_round_trip() {
        let s = Spec::parse("plg:my-conv/main:flac").unwrap();
        assert_eq!(s.plugin, "my-conv");
        assert_eq!(s.conv, "main");
        assert_eq!(s.ext, "flac");
        assert_eq!(Spec::parse("plg:a/b:same").unwrap().ext, "same");
        assert!(Spec::parse("mp3").is_none());
        assert!(Spec::parse("plg:a:flac").is_none());
        assert!(Spec::parse("plg:a/b:").is_none());
        assert!(s.label().contains("FLAC"));
    }

    #[test]
    fn progress_ratio() {
        let p = |done: f64, total: Option<f64>| provider::Progress {
            done,
            total,
            ..Default::default()
        };
        assert_eq!(ratio(&p(5.0, Some(10.0))), 0.5);
        assert_eq!(ratio(&p(0.25, None)), 0.25);
        assert_eq!(ratio(&p(1.0, None)), 0.99);
        assert_eq!(ratio(&p(42.0, None)), 0.0);
    }

    #[test]
    fn exec_template() {
        let dir = Path::new("/plugins/x");
        let get = |n: &str| -> Option<String> {
            match n {
                "input" => Some("/music/a b.wav".into()),
                "output" => Some("/out/1.flac".into()),
                "option:level" => Some("8".into()),
                _ => None,
            }
        };
        let t: Vec<String> = ["bin/tool.exe", "-l", "{option:level}", "--in={input}", "{output}", "{{literal}}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (prog, args) = build_exec(&t, dir, &get).unwrap();
        assert_eq!(prog, dir.join("bin/tool.exe"));
        assert_eq!(args, vec!["-l", "8", "--in=/music/a b.wav", "/out/1.flac", "{literal}"]);
        let (prog, _) = build_exec(&["sox".to_string(), "{output}".to_string()], dir, &get).unwrap();
        assert_eq!(prog, PathBuf::from("sox"));
        assert!(build_exec(&["x".to_string(), "{nope}".to_string()], dir, &get).is_err());
    }

    #[test]
    fn picks_output_file() {
        let d = std::env::temp_dir().join(format!("mp-conv-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(pick_output(&d, "mp3").is_none());
        std::fs::write(d.join("a.log"), b"x").unwrap();
        assert_eq!(pick_output(&d, "mp3"), Some(d.join("a.log")));
        std::fs::write(d.join("b.mp3"), b"xyz").unwrap();
        assert_eq!(pick_output(&d, "mp3"), Some(d.join("b.mp3")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
