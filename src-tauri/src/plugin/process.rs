//! process 入口的调用协议 2：运行 `<命令> <方法>`，标准输入写一个请求 JSON，标准输出一行一个事件
//! （log / progress / result / error）。协议 1 的程序走 `fetch/external.rs`。说明见 `docs/插件开发.md` 第 3 节
use crate::fetch::provider::{self, Progress};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// 标准输出最多读这么多（防止插件刷屏把内存吃光）
const MAX_STDOUT: usize = 64 << 20;
/// 一行事件最长（一次 search 的结果在一行里，给足）
const MAX_LINE: usize = 32 << 20;

/// 一次调用要用的东西
#[derive(Clone, Debug, Default)]
pub struct Call {
    pub argv: Vec<String>,
    pub dir: PathBuf,
    pub env: Vec<(String, String)>,
    pub method: String,
    pub params: Value,
    pub settings: Value,
    pub context: Value,
    /// 总时长上限
    pub timeout: Option<Duration>,
    /// 多久没有任何输出就停掉（download 用）
    pub idle: Option<Duration>,
}

/// 调用中产生的事件（日志、进度）
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Log { level: String, msg: String },
    Progress(Progress),
}

/// 解析一行输出。不是 JSON 的行当作日志
pub fn parse_line(line: &str) -> Line {
    let t = line.trim().trim_start_matches('\u{feff}');
    if t.is_empty() {
        return Line::Skip;
    }
    if !t.starts_with('{') {
        return Line::Event(Event::Log {
            level: "info".into(),
            msg: t.to_string(),
        });
    }
    let Ok(v) = serde_json::from_str::<Value>(t) else {
        return Line::Event(Event::Log {
            level: "info".into(),
            msg: t.chars().take(500).collect(),
        });
    };
    match v.get("event").and_then(|e| e.as_str()) {
        Some("log") => Line::Event(Event::Log {
            level: v["level"].as_str().unwrap_or("info").to_string(),
            msg: v["msg"].as_str().unwrap_or("").to_string(),
        }),
        Some("progress") => Line::Event(Event::Progress(serde_json::from_value(v).unwrap_or_default())),
        Some("result") => Line::Result(v.get("data").cloned().unwrap_or(Value::Null)),
        Some("error") => {
            let code = v["code"].as_str().unwrap_or("internal");
            let msg = v["message"].as_str().unwrap_or("插件出错");
            Line::Error(provider::coded(code, msg))
        }
        _ => Line::Event(Event::Log {
            level: "warn".into(),
            msg: format!("不认识的输出：{}", t.chars().take(200).collect::<String>()),
        }),
    }
}

#[derive(Debug, PartialEq)]
pub enum Line {
    Skip,
    Event(Event),
    Result(Value),
    Error(String),
}

/// 找到要运行的程序：相对路径按插件文件夹找，其他按 PATH
fn program(argv0: &str, dir: &Path) -> PathBuf {
    let local = dir.join(argv0);
    if (argv0.contains(['/', '\\']) || Path::new(argv0).extension().is_some()) && local.is_file() {
        return local;
    }
    PathBuf::from(argv0)
}

/// 运行一次。`on_event` 收到日志和进度；`stop` 返回 true 时结束插件进程（整棵进程树）
pub async fn run(c: &Call, mut on_event: impl FnMut(Event), stop: Option<provider::StopFn>) -> Result<Value, String> {
    let Some(argv0) = c.argv.first() else {
        return Err(provider::coded("internal", "插件没有写要运行的命令"));
    };
    let mut cmd = tokio::process::Command::new(program(argv0, &c.dir));
    cmd.args(&c.argv[1..])
        .arg(&c.method)
        .current_dir(&c.dir)
        .env("MP_PROVIDER_PROTOCOL", "2")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in &c.env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000 | 0x0000_0200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| provider::coded("internal", &format!("启动插件失败（{argv0}）：{e}")))?;
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        let req = json!({
            "protocol": 2,
            "method": c.method,
            "params": c.params,
            "settings": c.settings,
            "context": c.context,
        });
        let body = serde_json::to_vec(&req).unwrap_or_default();
        let _ = stdin.write_all(&body).await;
        let _ = stdin.shutdown().await;
    }
    // 标准错误另开任务收集，留最后 20 行
    let stderr = child.stderr.take();
    let err_task = tokio::spawn(async move {
        let mut tail: Vec<String> = vec![];
        if let Some(e) = stderr {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if !l.trim().is_empty() {
                    tail.push(l.chars().take(500).collect());
                    if tail.len() > 20 {
                        tail.remove(0);
                    }
                }
            }
        }
        tail
    });
    let stdout = child.stdout.take().ok_or_else(|| provider::coded("internal", "读不到插件的输出"))?;
    let mut reader = BufReader::new(stdout);
    let started = Instant::now();
    let mut last_out = Instant::now();
    let mut outcome: Option<Result<Value, String>> = None;
    let mut total = 0usize;
    let mut buf = Vec::new();
    let mut killed: Option<String> = None;
    loop {
        // 超时只是为了定时检查停止标记：没读完的半行留在 buf 里，下次接着读
        let mut limited = (&mut reader).take(MAX_LINE as u64);
        let read = limited.read_until(b'\n', &mut buf);
        let r = tokio::time::timeout(Duration::from_millis(300), read).await;
        match r {
            Ok(Ok(0)) => {
                if !buf.is_empty() {
                    match parse_line(&String::from_utf8_lossy(&buf)) {
                        Line::Result(v) => outcome = Some(Ok(v)),
                        Line::Error(m) => outcome = Some(Err(m)),
                        Line::Event(e) => on_event(e),
                        Line::Skip => {}
                    }
                }
                break;
            }
            Ok(Ok(n)) => {
                total += n;
                last_out = Instant::now();
                if total > MAX_STDOUT {
                    killed = Some(provider::coded("internal", "插件输出太多，已停止"));
                    break;
                }
                if !buf.ends_with(b"\n") && buf.len() < MAX_LINE {
                    // 还没到行尾（最后一行没有换行时下一次会读到 0）
                    continue;
                }
                let line = String::from_utf8_lossy(&buf).into_owned();
                buf.clear();
                match parse_line(&line) {
                    Line::Skip => {}
                    Line::Event(e) => on_event(e),
                    Line::Result(v) => outcome = Some(Ok(v)),
                    Line::Error(m) => outcome = Some(Err(m)),
                }
            }
            Ok(Err(e)) => {
                killed = Some(provider::coded("internal", &format!("读取插件输出失败：{e}")));
                break;
            }
            Err(_) => {}
        }
        if stop.as_ref().is_some_and(|s| s()) {
            killed = Some(provider::coded("internal", "已停止"));
            break;
        }
        if let Some(t) = c.timeout {
            if started.elapsed() > t {
                killed = Some(provider::coded("internal", &format!("插件的 {} 超过 {} 秒没有结束", c.method, t.as_secs())));
                break;
            }
        }
        if let Some(t) = c.idle {
            if last_out.elapsed() > t {
                killed = Some(provider::coded("internal", &format!("插件 {} 秒没有任何进度，已停止", t.as_secs())));
                break;
            }
        }
    }
    if let Some(m) = killed {
        kill_tree(pid);
        let _ = child.kill().await;
        return Err(m);
    }
    let status = match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Ok(Ok(s)) => Some(s),
        _ => {
            kill_tree(pid);
            let _ = child.kill().await;
            None
        }
    };
    let tail = err_task.await.unwrap_or_default();
    for l in &tail {
        on_event(Event::Log {
            level: "stderr".into(),
            msg: l.clone(),
        });
    }
    match outcome {
        Some(r) => r,
        None => {
            let code = status.and_then(|s| s.code());
            Err(match tail.last() {
                Some(l) if code != Some(0) => provider::coded("internal", l),
                _ => match code {
                    Some(0) => provider::coded("internal", "插件没有输出 result 或 error 事件"),
                    Some(c) => provider::coded("internal", &format!("插件出错（退出码 {c}）")),
                    None => provider::coded("internal", "插件被结束"),
                },
            })
        }
    }
}

/// 结束整棵进程树（插件可能又启动了 yt-dlp 之类的子进程）
pub fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(parse_line("  "), Line::Skip);
        assert_eq!(
            parse_line("hello"),
            Line::Event(Event::Log {
                level: "info".into(),
                msg: "hello".into()
            })
        );
        assert_eq!(parse_line(r#"{"event":"result","data":[1]}"#), Line::Result(json!([1])));
        assert_eq!(
            parse_line(r#"{"event":"error","code":"login_required","message":"先登录"}"#),
            Line::Error("[login_required] 先登录".into())
        );
        assert_eq!(
            parse_line(r#"{"event":"error","code":"weird","message":"x"}"#),
            Line::Error("[internal] x".into())
        );
        match parse_line(r#"{"event":"progress","done":5,"total":10,"unit":"pages","text":"第 5 页"}"#) {
            Line::Event(Event::Progress(p)) => {
                assert_eq!(p.done, 5.0);
                assert_eq!(p.total, Some(10.0));
                assert_eq!(p.text, "第 5 页");
            }
            x => panic!("{x:?}"),
        }
    }

    #[cfg(unix)]
    fn script(name: &str, body: &str) -> (PathBuf, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("mp-plg-proc-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("p.sh"), body).unwrap();
        (dir, vec!["sh".into(), "p.sh".into()])
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_protocol_2() {
        let (dir, argv) = script(
            "ok",
            "read -r req\necho \"log line\"\necho '{\"event\":\"progress\",\"done\":1,\"total\":2,\"unit\":\"items\"}'\necho 'oops' >&2\necho \"{\\\"event\\\":\\\"result\\\",\\\"data\\\":{\\\"m\\\":\\\"$1\\\",\\\"id\\\":\\\"$MP_PLUGIN_ID\\\"}}\"\n",
        );
        let c = Call {
            argv,
            dir: dir.clone(),
            env: vec![("MP_PLUGIN_ID".into(), "demo".into())],
            method: "search".into(),
            timeout: Some(Duration::from_secs(10)),
            ..Default::default()
        };
        let mut evs = vec![];
        let v = run(&c, |e| evs.push(e), None).await.unwrap();
        assert_eq!(v, json!({ "m": "search", "id": "demo" }));
        assert!(evs.iter().any(|e| matches!(e, Event::Progress(p) if p.done == 1.0)));
        assert!(evs.iter().any(|e| matches!(e, Event::Log { level, msg } if level == "stderr" && msg == "oops")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn errors_timeouts_and_stop() {
        let (dir, argv) = script("err", "echo '{\"event\":\"error\",\"code\":\"bad_config\",\"message\":\"先选文件夹\"}'\n");
        let c = Call {
            argv: argv.clone(),
            dir: dir.clone(),
            method: "info".into(),
            ..Default::default()
        };
        assert_eq!(run(&c, |_| {}, None).await.unwrap_err(), "[bad_config] 先选文件夹");

        std::fs::write(dir.join("p.sh"), "echo bad key >&2\nexit 3\n").unwrap();
        assert_eq!(run(&c, |_| {}, None).await.unwrap_err(), "[internal] bad key");

        std::fs::write(dir.join("p.sh"), "sleep 30\n").unwrap();
        let mut slow = c.clone();
        slow.timeout = Some(Duration::from_secs(1));
        let t = Instant::now();
        assert!(run(&slow, |_| {}, None).await.unwrap_err().contains("超过"));
        assert!(t.elapsed() < Duration::from_secs(5));

        let t = Instant::now();
        let stop: provider::StopFn = std::sync::Arc::new(move || t.elapsed() > Duration::from_millis(500));
        assert!(run(&c, |_| {}, Some(stop)).await.unwrap_err().contains("已停止"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
