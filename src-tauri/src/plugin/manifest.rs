//! 插件清单 `plugin.json` 的格式和检查。字段说明见 `docs/插件开发.md` 第 2 节
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// 软件支持的清单格式版本
pub const MANIFEST_VERSION: u32 = 1;
/// 软件支持的调用协议版本（当前和上一个）
pub const PROTOCOLS: [u32; 2] = [1, 2];
pub const KINDS: [&str; 5] = ["music", "video", "anime", "manga", "novel"];
pub const CAPS: [&str; 4] = ["search", "episodes", "resolve", "download"];
pub const TOOLS: [&str; 3] = ["ytdlp", "ffmpeg", "ffprobe"];
pub const SETTING_TYPES: [&str; 6] = ["string", "secret", "bool", "int", "select", "folder"];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub manifest: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub homepage: String,
    pub license: String,
    pub icon: String,
    pub engines: Engines,
    pub runtime: Runtime,
    pub contributes: Contributes,
    pub permissions: Permissions,
    pub settings: Vec<Setting>,
    pub update: Update,
    pub legacy_ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Engines {
    /// 最低软件版本，如 `>=0.6.0`
    pub app: String,
    pub protocol: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Runtime {
    /// rule / process / js
    #[serde(rename = "type")]
    pub kind: String,
    /// process：按系统写命令数组，键为 windows / macos / linux / default
    pub command: BTreeMap<String, Vec<String>>,
    /// rule / js：入口文件
    pub file: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Contributes {
    pub sources: Vec<SourceDecl>,
    pub rules: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SourceDecl {
    pub id: String,
    pub name: String,
    pub media_kinds: Vec<String>,
    pub capabilities: Vec<String>,
    pub note: String,
}

impl SourceDecl {
    pub fn can(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Permissions {
    pub hosts: Vec<String>,
    /// none / optional / required
    pub login: String,
    pub tools: Vec<String>,
    /// 只能写 `setting:<key>`
    pub paths: Vec<String>,
}

impl Permissions {
    pub fn login(&self) -> &str {
        if self.login.is_empty() {
            "none"
        } else {
            &self.login
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Setting {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    pub help: String,
    pub default: serde_json::Value,
    pub required: bool,
    /// select 的选项：`[{"value":"a","label":"A"}]` 或 `["a","b"]`
    pub options: Vec<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Update {
    /// 仓库索引地址（检查更新用）
    pub index: String,
}

/// 插件的下载源注册成的下载源 id：`plg-<插件>`，源 id 不是 main 时 `plg-<插件>.<源>`
pub fn provider_id(plugin: &str, source: &str) -> String {
    if source == "main" || source.is_empty() {
        format!("plg-{plugin}")
    } else {
        format!("plg-{plugin}.{source}")
    }
}

/// 反过来：从下载源 id 取出（插件 id，源 id）
pub fn split_provider_id(pid: &str) -> Option<(&str, &str)> {
    let rest = pid.strip_prefix("plg-")?;
    Some(match rest.split_once('.') {
        Some((p, s)) => (p, s),
        None => (rest, "main"),
    })
}

fn id_ok(id: &str, min: usize) -> bool {
    let n = id.chars().count();
    (min..=64).contains(&n) && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !id.starts_with('-') && !id.ends_with('-')
}

pub type Ver = (u64, u64, u64);

/// 语义化版本 `主.次.修`（后面可带 -beta 之类），返回可比较的三元组
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().split(['-', '+']).next()?;
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    it.next().is_none().then_some((a, b, c))
}

/// 检查 `>=0.6.0` 这样的最低版本要求；空表示不限
pub fn app_satisfies(req: &str, app: &str) -> bool {
    let req = req.trim();
    if req.is_empty() || req == "*" {
        return true;
    }
    let need = req.trim_start_matches(">=").trim_start_matches('^').trim();
    match (parse_version(need), parse_version(app)) {
        (Some(n), Some(a)) => a >= n,
        _ => false,
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let text = text.trim_start_matches('\u{feff}');
        serde_json::from_str(text).map_err(|e| format!("plugin.json 第 {} 行格式不对：{e}", e.line()))
    }

    /// 信任级别：rule / js 由软件代为联网（受限），process 是普通程序（完全信任）
    pub fn trust(&self) -> &'static str {
        if self.runtime.kind == "process" {
            "full"
        } else {
            "limited"
        }
    }

    /// 该系统上要运行的命令
    pub fn command(&self) -> Option<Vec<String>> {
        let key = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        self.runtime
            .command
            .get(key)
            .or_else(|| self.runtime.command.get("default"))
            .filter(|v| !v.is_empty())
            .cloned()
    }

    pub fn source(&self, id: &str) -> Option<&SourceDecl> {
        self.contributes.sources.iter().find(|s| s.id == id)
    }

    /// 检查清单。`dir` 是插件文件夹（检查入口文件是否存在），`app_version` 是当前软件版本
    pub fn check(&self, dir: &Path, app_version: &str) -> Vec<String> {
        let mut is = vec![];
        if self.manifest == 0 {
            is.push("缺少 manifest（写 1）".into());
        } else if self.manifest > MANIFEST_VERSION {
            is.push(format!("这个插件需要更新版本的软件（清单格式 {}）", self.manifest));
        }
        if !id_ok(&self.id, 3) {
            is.push("id 只能用小写字母、数字、-，3–64 位，不能以 - 开头或结尾".into());
        }
        if self.name.trim().is_empty() {
            is.push("缺少 name".into());
        }
        if parse_version(&self.version).is_none() {
            is.push("version 要写成 主.次.修，如 1.0.0".into());
        }
        if !self.engines.app.is_empty() && !app_satisfies(&self.engines.app, app_version) {
            is.push(format!("这个插件需要 {} 版本的软件，当前是 {app_version}，请先升级软件", self.engines.app));
        }
        let rt = self.runtime.kind.as_str();
        match rt {
            "process" => {
                match self.command() {
                    None => is.push("runtime.command 要按系统写命令数组，如 {\"default\": [\"python\", \"main.py\"]}".into()),
                    Some(argv) => {
                        let first = Path::new(&argv[0]);
                        let looks_local = argv[0].contains(['/', '\\']) || first.extension().is_some();
                        if looks_local && first.is_relative() && !dir.join(first).is_file() && !is_interpreter(&argv[0]) {
                            is.push(format!("runtime.command 里的程序不存在：{}", argv[0]));
                        }
                        if first.is_absolute() {
                            is.push("runtime.command 不能写绝对路径，用相对插件文件夹的路径".into());
                        }
                    }
                }
                for k in self.runtime.command.keys() {
                    if !["windows", "macos", "linux", "default"].contains(&k.as_str()) {
                        is.push(format!("runtime.command 不认识的系统：{k}"));
                    }
                }
            }
            "rule" | "js" => {
                let f = &self.runtime.file;
                if f.is_empty() || !safe_rel(f) || !dir.join(f).is_file() {
                    is.push(format!("runtime.file 指向的文件不存在：{f}"));
                }
            }
            _ => is.push("runtime.type 只能是 rule / process / js".into()),
        }
        if matches!(rt, "process" | "js") && !PROTOCOLS.contains(&self.engines.protocol) {
            if self.engines.protocol > *PROTOCOLS.last().unwrap() {
                is.push(format!("这个插件用的是更新的协议 {}，请先升级软件", self.engines.protocol));
            } else {
                is.push("process / js 入口要写 engines.protocol（2）".into());
            }
        }
        if rt == "js" && self.engines.protocol != 2 {
            is.push("js 入口只支持协议 2".into());
        }
        let c = &self.contributes;
        if c.sources.is_empty() && c.rules.is_empty() {
            is.push("contributes 里至少要有 sources 或 rules".into());
        }
        if rt == "rule" && c.sources.len() > 1 {
            is.push("rule 入口只能有一个下载源".into());
        }
        let mut seen = vec![];
        for (i, s) in c.sources.iter().enumerate() {
            let w = format!("contributes.sources[{i}]");
            if !id_ok(&s.id, 1) {
                is.push(format!("{w}.id 只能用小写字母、数字、-"));
            }
            if seen.contains(&s.id) {
                is.push(format!("{w}.id 重复：{}", s.id));
            }
            seen.push(s.id.clone());
            let bad: Vec<_> = s.media_kinds.iter().filter(|k| !KINDS.contains(&k.as_str())).collect();
            if !bad.is_empty() {
                is.push(format!("{w}.media_kinds 不认识：{bad:?}"));
            }
            let bad: Vec<_> = s.capabilities.iter().filter(|k| !CAPS.contains(&k.as_str())).collect();
            if !bad.is_empty() {
                is.push(format!("{w}.capabilities 不认识：{bad:?}"));
            }
            if !s.can("search") {
                is.push(format!("{w}.capabilities 必须有 search"));
            }
            if !s.can("resolve") && !s.can("download") {
                is.push(format!("{w}.capabilities 至少要有 resolve 或 download"));
            }
            if rt != "process" && s.can("download") {
                is.push(format!("{w}：rule / js 入口不能 download（没有文件权限），请用 resolve"));
            }
            if self.engines.protocol == 1 && (s.can("episodes") || s.can("download")) {
                is.push(format!("{w}：协议 1 只支持 search / resolve"));
            }
        }
        for r in &c.rules {
            if !safe_rel(r) || !dir.join(r).is_file() {
                is.push(format!("contributes.rules 里的文件不存在：{r}"));
            }
        }
        let p = &self.permissions;
        if !["none", "optional", "required"].contains(&p.login()) {
            is.push("permissions.login 只能是 none / optional / required".into());
        }
        if p.login() != "none" && p.hosts.is_empty() {
            is.push("用登录（login）时必须写 permissions.hosts".into());
        }
        let bad: Vec<_> = p.tools.iter().filter(|t| !TOOLS.contains(&t.as_str())).collect();
        if !bad.is_empty() {
            is.push(format!("permissions.tools 只能是 ytdlp / ffmpeg / ffprobe：{bad:?}"));
        }
        let keys: Vec<&str> = self.settings.iter().map(|s| s.key.as_str()).collect();
        for x in &p.paths {
            match x.strip_prefix("setting:") {
                Some(k) if keys.contains(&k) => {}
                _ => is.push(format!("permissions.paths 只能引用设置项（setting:<key>）：{x}")),
            }
        }
        for h in &p.hosts {
            if h.contains("://") || h.contains('/') || h.trim().is_empty() {
                is.push(format!("permissions.hosts 只写域名（可带端口和 *.），不写 http:// 和路径：{h}"));
            }
        }
        let mut skeys = vec![];
        for (i, s) in self.settings.iter().enumerate() {
            if s.key.is_empty() || s.label.is_empty() {
                is.push(format!("settings[{i}] 要有 key 和 label"));
            }
            if skeys.contains(&s.key) {
                is.push(format!("settings[{i}].key 重复：{}", s.key));
            }
            skeys.push(s.key.clone());
            if !SETTING_TYPES.contains(&s.kind.as_str()) {
                is.push(format!("settings[{i}].type 只能是 string / secret / bool / int / select / folder"));
            }
            if s.kind == "select" && s.options.is_empty() {
                is.push(format!("settings[{i}] 是 select，要写 options"));
            }
        }
        if !self.icon.is_empty() && (!safe_rel(&self.icon) || !dir.join(&self.icon).is_file()) {
            is.push(format!("icon 文件不存在：{}", self.icon));
        }
        for l in &self.legacy_ids {
            if !(l.starts_with("ext-") || l.starts_with("em-")) {
                is.push(format!("legacy_ids 只能写 ext- 或 em- 开头的旧下载源 id：{l}"));
            }
        }
        is
    }
}

/// python / node 这类解释器名（不检查文件是否存在）
fn is_interpreter(s: &str) -> bool {
    let n = Path::new(s).file_stem().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    ["python", "python3", "py", "node", "deno", "bun", "pwsh", "powershell", "cmd", "sh", "bash"].contains(&n.as_str())
}

/// 相对路径，且不含 `..`
pub fn safe_rel(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && path.is_relative()
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::Prefix(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest::parse(
            r#"{
              "manifest": 1, "id": "local-library", "name": "本地曲库", "version": "1.0.0",
              "engines": { "app": ">=0.1.0", "protocol": 2 },
              "runtime": { "type": "process", "command": { "default": ["python", "main.py"] } },
              "contributes": { "sources": [ { "id": "main", "media_kinds": ["music"], "capabilities": ["search", "download"] } ] },
              "permissions": { "paths": ["setting:dir"] },
              "settings": [ { "key": "dir", "type": "folder", "label": "曲库", "required": true } ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn good_manifest_passes() {
        let m = sample();
        let dir = std::env::temp_dir();
        assert_eq!(m.check(&dir, "0.6.0"), Vec::<String>::new());
        assert_eq!(m.trust(), "full");
        assert_eq!(m.command().unwrap(), vec!["python", "main.py"]);
        assert_eq!(provider_id(&m.id, "main"), "plg-local-library");
        assert_eq!(provider_id("a", "video"), "plg-a.video");
        assert_eq!(split_provider_id("plg-a.video"), Some(("a", "video")));
        assert_eq!(split_provider_id("plg-abc"), Some(("abc", "main")));
        assert_eq!(split_provider_id("em-abc"), None);
    }

    #[test]
    fn bad_manifest_lists_problems() {
        let m = Manifest::parse(
            r#"{"manifest":1,"id":"Bad_ID","version":"1","runtime":{"type":"js","file":"../x.js"},
               "contributes":{"sources":[{"id":"m","capabilities":["download"],"media_kinds":["comic"]}]},
               "permissions":{"login":"required","hosts":["https://a.com/"]},
               "settings":[{"key":"k","type":"color","label":"x"}]}"#,
        )
        .unwrap();
        let is = m.check(&std::env::temp_dir(), "0.6.0").join("\n");
        for want in [
            "id 只能",
            "缺少 name",
            "version",
            "runtime.file",
            "comic",
            "必须有 search",
            "不能 download",
            "只写域名",
            "settings[0].type",
            "协议",
        ] {
            assert!(is.contains(want), "缺少提示 {want}：\n{is}");
        }
    }

    #[test]
    fn versions_and_requirements() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert!(app_satisfies(">=0.5.0", "0.6.0"));
        assert!(!app_satisfies(">=0.7.0", "0.6.9"));
        assert!(app_satisfies("", "0.1.0"));
        let mut m = sample();
        m.engines.app = ">=9.0.0".into();
        assert!(m.check(&std::env::temp_dir(), "0.6.0").iter().any(|s| s.contains("升级软件")));
        m.engines.app.clear();
        m.engines.protocol = 3;
        assert!(m.check(&std::env::temp_dir(), "0.6.0").iter().any(|s| s.contains("更新的协议")));
    }

    #[test]
    fn relative_paths() {
        assert!(safe_rel("rules/a.em"));
        assert!(!safe_rel("../a.em"));
        assert!(!safe_rel("/etc/passwd"));
        assert!(!safe_rel(""));
    }
}
