//! 外部程序下载源：用户自己写的 exe（或 .bat / .cmd）按约定的命令行协议接进自动获取。
//!
//! 软件调用 `程序 <动作>`，把请求 JSON 写进标准输入，从标准输出读回 JSON（UTF-8）：
//! - `info`：返回 [`ProviderInfo`]（id、显示名、支持的类型、说明）
//! - `search`：输入 `{"query","media_kind","page"}`，返回 `Item` 数组（或 `{"items":[…]}`）
//! - `resolve`：输入 `{"item","quality"}`，返回 [`Resolved`]（下载地址和歌词）
//!
//! 出错时退出码非 0，标准错误的最后一行当作错误提示；也可以输出 `{"error":"…"}`。
//! 完整说明见 `docs/下载源接口.md` 第 9 节。
use super::provider::{self, Item, Provider, ProviderInfo, Resolved, Target};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// 外部下载源的 id 前缀，避免和内置下载源重名
pub const ID_PREFIX: &str = "ext-";
/// 协议版本，通过环境变量 `MP_PROVIDER_PROTOCOL` 传给程序
pub const PROTOCOL: &str = "1";

const INFO_TIMEOUT: Duration = Duration::from_secs(15);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
/// resolve 可能由程序自己把文件下好（`Target::File`），给足时间
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 保存在来源表 `rule_json` 里的内容：程序路径和它报告的信息（启动时直接注册，不用再运行一次）
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Stored {
    pub exe: String,
    pub info: ProviderInfo,
}

impl Stored {
    pub fn parse(rule_json: &str) -> Option<Stored> {
        let s: Stored = serde_json::from_str(rule_json).ok()?;
        (!s.exe.is_empty() && !s.info.id.is_empty()).then_some(s)
    }
}

pub struct ExternalProvider {
    pub exe: PathBuf,
    pub info: ProviderInfo,
}

impl ExternalProvider {
    pub fn from_stored(s: &Stored) -> Self {
        ExternalProvider {
            exe: PathBuf::from(&s.exe),
            info: s.info.clone(),
        }
    }
}

/// 看起来像可以当下载源的程序：存在的文件，扩展名是 exe / bat / cmd（非 Windows 上任何可执行文件都行）
pub fn looks_like_program(path: &str) -> bool {
    let p = Path::new(path.trim());
    if !p.is_file() {
        return false;
    }
    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let known = matches!(ext.as_str(), "exe" | "bat" | "cmd");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        known || std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        known
    }
}

/// id 只保留小写字母、数字和 `-`，再加上 `ext-` 前缀
pub fn normalize_id(raw: &str, exe: &Path) -> String {
    let base = if raw.trim().is_empty() {
        exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        raw.to_string()
    };
    let mut id = String::new();
    for ch in base.trim().to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            id.push(ch);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-');
    let id = id.strip_prefix(ID_PREFIX).unwrap_or(id);
    format!("{ID_PREFIX}{}", if id.is_empty() { "program" } else { id })
}

/// 运行 `程序 info`，整理成可以注册的信息
pub async fn probe(exe: &Path) -> Result<ProviderInfo, String> {
    let v = call(exe, "info", &Value::Null, INFO_TIMEOUT).await?;
    let mut info: ProviderInfo = serde_json::from_value(v).map_err(|err| format!("程序的 info 输出格式不对：{err}"))?;
    info.id = normalize_id(&info.id, exe);
    if info.name.trim().is_empty() {
        info.name = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| info.id.clone());
    }
    info.media_kinds
        .retain(|k| matches!(k.as_str(), "music" | "video" | "anime" | "manga" | "novel"));
    Ok(info)
}

/// 注册一个外部下载源（同 id 会替换旧的）
pub fn register(s: &Stored) {
    provider::register(Arc::new(ExternalProvider::from_stored(s)));
}

/// 从程序输出里取 JSON：允许前面有日志行，取第一个以 `{` 或 `[` 开头的行起的全部内容
pub fn parse_output(stdout: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8_lossy(stdout);
    let text = text.trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return check_error(v);
    }
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with('{') || t.starts_with('[') {
            if let Ok(v) = serde_json::from_str::<Value>(text[offset..].trim()) {
                return check_error(v);
            }
        }
        offset += line.len();
    }
    let preview: String = text.trim().chars().take(120).collect();
    Err(if preview.is_empty() {
        "程序没有输出任何内容".into()
    } else {
        format!("程序输出的不是 JSON：{preview}")
    })
}

fn check_error(v: Value) -> Result<Value, String> {
    match v.get("error").and_then(|e| e.as_str()) {
        Some(m) if !m.trim().is_empty() => Err(m.trim().to_string()),
        _ => Ok(v),
    }
}

/// `search` 的返回：`[…]` 或 `{"items":[…]}`
pub fn parse_items(v: Value, media_kind: &str) -> Result<Vec<Item>, String> {
    let list = match v {
        Value::Array(a) => a,
        Value::Object(mut o) => match o.remove("items") {
            Some(Value::Array(a)) => a,
            _ => return Err("search 应输出数组，或带 items 数组的对象".into()),
        },
        _ => return Err("search 应输出数组，或带 items 数组的对象".into()),
    };
    let mut out = Vec::new();
    for (i, x) in list.into_iter().enumerate() {
        let mut it: Item = serde_json::from_value(x).map_err(|err| format!("第 {} 条结果格式不对：{err}", i + 1))?;
        if it.title.trim().is_empty() {
            continue;
        }
        if it.media_kind.is_empty() {
            it.media_kind = media_kind.to_string();
        }
        out.push(it);
    }
    Ok(out)
}

pub fn parse_resolved(v: Value) -> Result<Resolved, String> {
    let r: Resolved = serde_json::from_value(v).map_err(|err| format!("resolve 输出格式不对：{err}"))?;
    if let Target::File { path } = &r.target {
        if !Path::new(path).is_file() {
            return Err(format!("程序给出的文件不存在：{path}"));
        }
    }
    Ok(r)
}

/// 运行一次程序：`exe <action>`，请求写进标准输入，读回标准输出里的 JSON
pub async fn call(exe: &Path, action: &str, request: &Value, timeout: Duration) -> Result<Value, String> {
    if !exe.is_file() {
        return Err(format!("找不到下载源程序：{}", exe.display()));
    }
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg(action)
        .env("MP_PROVIDER_PROTOCOL", PROTOCOL)
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = exe.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW：不弹黑框
    let mut child = cmd.spawn().map_err(|err| format!("启动下载源程序失败：{err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let body = serde_json::to_vec(request).unwrap_or_default();
        // 程序不读标准输入时会写失败，忽略即可
        let _ = stdin.write_all(&body).await;
        let _ = stdin.shutdown().await;
    }
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(r) => r.map_err(|err| format!("运行下载源程序失败：{err}"))?,
        Err(_) => return Err(format!("下载源程序 {} 超过 {} 秒没有结束", action, timeout.as_secs())),
    };
    if !out.status.success() {
        // 有 {"error":…} 时优先用它
        if let Err(m) = parse_output(&out.stdout) {
            if !m.starts_with("程序") {
                return Err(m);
            }
        }
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
        return Err(if last.is_empty() {
            format!("下载源程序出错（退出码 {code}）")
        } else {
            last.to_string()
        });
    }
    parse_output(&out.stdout)
}

impl Provider for ExternalProvider {
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }

    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async move {
            let req = serde_json::json!({ "query": query, "media_kind": media_kind, "page": page });
            let v = call(&self.exe, "search", &req, SEARCH_TIMEOUT).await?;
            parse_items(v, media_kind)
        })
    }

    fn resolve<'a>(&'a self, item: &'a Item, quality: &'a str) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(async move {
            let req = serde_json::json!({ "item": item, "quality": quality });
            let v = call(&self.exe, "resolve", &req, RESOLVE_TIMEOUT).await?;
            parse_resolved(v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_prefixed_and_clean() {
        let exe = Path::new("C:/tools/My Source.exe");
        assert_eq!(normalize_id("MySrc", exe), "ext-mysrc");
        assert_eq!(normalize_id("我的 源_1", exe), "ext-1");
        assert_eq!(normalize_id("", exe), "ext-my-source");
        assert_eq!(normalize_id("ext-abc", exe), "ext-abc");
        assert_eq!(normalize_id("！！", Path::new("中文.exe")), "ext-program");
    }

    #[test]
    fn output_with_log_lines() {
        let v = parse_output("正在登录…\n[{\"title\":\"a\"}]\n".as_bytes()).unwrap();
        assert!(v.is_array());
        let v = parse_output("\u{feff}{\"id\":\"x\"}".as_bytes()).unwrap();
        assert_eq!(v["id"], "x");
        assert_eq!(parse_output(b"{\"error\":\"key wrong\"}").unwrap_err(), "key wrong");
        assert!(parse_output(b"").unwrap_err().contains("没有输出"));
        assert!(parse_output(b"hello").unwrap_err().contains("不是 JSON"));
    }

    #[test]
    fn items_from_array_or_object() {
        let v = serde_json::json!([{ "id": "1", "title": "防波堤", "artist": "林间信号", "qualities": [{ "key": "flac", "label": "无损" }] }, { "title": "" }]);
        let items = parse_items(v, "music").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].media_kind, "music");
        assert_eq!(items[0].qualities[0].key, "flac");
        let v = serde_json::json!({ "items": [{ "title": "x", "media_kind": "video" }] });
        assert_eq!(parse_items(v, "music").unwrap()[0].media_kind, "video");
        assert!(parse_items(serde_json::json!(3), "music").is_err());
    }

    #[test]
    fn resolved_targets() {
        let r = parse_resolved(serde_json::json!({ "target": { "type": "http", "url": "https://a/b.flac", "ext": "flac" }, "lyric": null })).unwrap();
        assert!(matches!(r.target, Target::Http { .. }));
        let r = parse_resolved(serde_json::json!({ "target": { "type": "ytdlp", "url": "https://a" } })).unwrap();
        assert_eq!(r.lyric, None);
        assert!(parse_resolved(serde_json::json!({ "target": { "type": "file", "path": "/no/such/file.mp3" } })).is_err());
        assert!(parse_resolved(serde_json::json!({ "target": { "type": "ftp" } })).is_err());
    }

    #[test]
    fn stored_round_trip() {
        let s = Stored {
            exe: "C:/a.exe".into(),
            info: ProviderInfo {
                id: "ext-a".into(),
                name: "A".into(),
                ..Default::default()
            },
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(Stored::parse(&j), Some(s));
        assert_eq!(Stored::parse(""), None);
        assert_eq!(Stored::parse("{\"mode\":\"rules\"}"), None);
    }

    /// 写一个假的下载源程序（Windows 上是 .cmd，其他系统是 sh 脚本），走一遍 info / search / resolve
    #[tokio::test]
    async fn runs_a_real_program() {
        let dir = std::env::temp_dir().join(format!("mp-ext-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("song.mp3");
        std::fs::write(&song, b"ID3").unwrap();
        let song_json = song.to_string_lossy().replace('\\', "\\\\");
        #[cfg(windows)]
        let exe = {
            let p = dir.join("demo.cmd");
            let script = format!(
                "@echo off\r\nif \"%1\"==\"info\" echo {{\"id\":\"Demo\",\"name\":\"Demo Source\",\"media_kinds\":[\"music\",\"bad\"]}}\r\nif \"%1\"==\"search\" echo [{{\"id\":\"1\",\"title\":\"Breakwater\",\"artist\":\"Signal\"}}]\r\nif \"%1\"==\"resolve\" echo {{\"target\":{{\"type\":\"file\",\"path\":\"{song_json}\"}},\"lyric\":\"[00:00.00]x\"}}\r\nif \"%1\"==\"fail\" (echo bad key 1>&2 & exit /b 2)\r\n"
            );
            std::fs::write(&p, script).unwrap();
            p
        };
        #[cfg(unix)]
        let exe = {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("demo.sh");
            let script = format!(
                "#!/bin/sh\ncat > /dev/null\ncase \"$1\" in\n info) echo '{{\"id\":\"Demo\",\"name\":\"Demo Source\",\"media_kinds\":[\"music\",\"bad\"]}}';;\n search) echo 'log line'; echo '[{{\"id\":\"1\",\"title\":\"Breakwater\",\"artist\":\"Signal\"}}]';;\n resolve) echo '{{\"target\":{{\"type\":\"file\",\"path\":\"{song_json}\"}},\"lyric\":\"[00:00.00]x\"}}';;\n *) echo bad key >&2; exit 2;;\nesac\n"
            );
            std::fs::write(&p, script).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        assert!(looks_like_program(&exe.to_string_lossy()));
        assert!(!looks_like_program(&dir.to_string_lossy()));

        let info = probe(&exe).await.unwrap();
        assert_eq!(info.id, "ext-demo");
        assert_eq!(info.name, "Demo Source");
        assert_eq!(info.media_kinds, vec!["music".to_string()]);

        let p = ExternalProvider { exe: exe.clone(), info };
        let items = p.search("防波堤", "music", 1).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].artist, "Signal");
        let r = p.resolve(&items[0], "").await.unwrap();
        assert_eq!(
            r.target,
            Target::File {
                path: song.to_string_lossy().into_owned()
            }
        );
        assert_eq!(r.lyric.as_deref(), Some("[00:00.00]x"));

        let err = call(&exe, "fail", &Value::Null, INFO_TIMEOUT).await.unwrap_err();
        assert_eq!(err, "bad key");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
