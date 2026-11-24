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
/// 转换参数（options）能用的类型
pub const OPTION_TYPES: [&str; 4] = ["string", "bool", "int", "select"];
/// 转换器 inputs 里的格式组
pub const INPUT_GROUPS: [&str; 3] = ["audio", "video", "*"];
/// exec 命令模板里能用的占位符（另外还有 `{option:<key>}`、`{setting:<key>}`）
pub const EXEC_VARS: [&str; 9] = [
    "input",
    "output",
    "output_dir",
    "input_ext",
    "output_ext",
    "work_dir",
    "plugin_dir",
    "ffmpeg",
    "ffprobe",
];

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
    pub converters: Vec<ConverterDecl>,
}

/// 格式转换器：声明能处理的输入格式、能输出的格式和每次转换的参数。说明见 `docs/插件开发.md` 第 15 节
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ConverterDecl {
    pub id: String,
    pub name: String,
    /// 扩展名（不带点，小写），或格式组 `audio` / `video` / `*`
    pub inputs: Vec<String>,
    /// `"mp3"`、`"same"`（和输入同格式），或 `{"ext": "mp3", "name": "MP3 320k"}`
    pub outputs: Vec<serde_json::Value>,
    /// 每次转换的参数，格式同 settings（类型只能是 string / bool / int / select）
    pub options: Vec<Setting>,
    /// 不写代码，直接运行的命令模板，如 `["bin/tool.exe", "{input}", "{output}"]`；空表示走协议 2 的 convert 方法
    pub exec: Vec<String>,
    /// 输出时长会和输入不同（剪掉静音、截取片段……）：替换原文件时不比较时长
    pub changes_duration: bool,
    pub note: String,
}

/// 转换器的一种输出格式
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OutputFmt {
    /// 扩展名，`same` 表示和输入相同
    pub ext: String,
    pub name: String,
}

/// 扩展名：小写字母和数字，1–16 位
pub fn ext_ok(e: &str) -> bool {
    (1..=16).contains(&e.len()) && e.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// 输入格式列表里有没有这个扩展名（不分大小写）。`*` 是任何文件，`audio` / `video` 是软件认识的音频 / 视频扩展名
pub fn accepts_ext(inputs: &[String], ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    inputs.iter().any(|i| match i.as_str() {
        "*" => true,
        "audio" => crate::parse::AUDIO.contains(&e.as_str()),
        "video" => crate::parse::VIDEO.contains(&e.as_str()),
        x => !e.is_empty() && x == e,
    })
}

impl ConverterDecl {
    /// 这个转换器能不能处理这个扩展名（不分大小写）
    pub fn accepts(&self, ext: &str) -> bool {
        accepts_ext(&self.inputs, ext)
    }

    /// 输出格式列表（写错的项跳过，检查时会报出来）
    pub fn output_list(&self) -> Vec<OutputFmt> {
        self.outputs
            .iter()
            .filter_map(|o| {
                let (ext, name) = match o {
                    serde_json::Value::String(e) => (e.clone(), String::new()),
                    serde_json::Value::Object(m) => (
                        m.get("ext")?.as_str()?.to_string(),
                        m.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                    ),
                    _ => return None,
                };
                let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
                let name = if !name.trim().is_empty() {
                    name
                } else if ext == "same" {
                    "同格式".to_string()
                } else {
                    ext.to_uppercase()
                };
                Some(OutputFmt { ext, name })
            })
            .collect()
    }

    /// 输出扩展名：`same` 换成输入的扩展名
    pub fn resolve_ext(spec: &str, input_ext: &str) -> String {
        if spec == "same" {
            input_ext.to_ascii_lowercase()
        } else {
            spec.to_string()
        }
    }

    fn check(&self, w: &str, m: &Manifest, dir: &Path, is: &mut Vec<String>) {
        if !id_ok(&self.id, 1) {
            is.push(format!("{w}.id 只能用小写字母、数字、-"));
        }
        if self.inputs.is_empty() {
            is.push(format!("{w}.inputs 至少写一种输入格式（扩展名，或 audio / video / *）"));
        }
        for i in &self.inputs {
            if !(INPUT_GROUPS.contains(&i.as_str()) || ext_ok(i)) {
                is.push(format!("{w}.inputs 只能写不带点的小写扩展名，或 audio / video / *：{i}"));
            }
        }
        if self.outputs.is_empty() {
            is.push(format!("{w}.outputs 至少写一种输出格式"));
        }
        let list = self.output_list();
        if list.len() != self.outputs.len() {
            is.push(format!("{w}.outputs 每一项是扩展名字符串，或 {{\"ext\": \"mp3\", \"name\": \"…\"}}"));
        }
        let mut seen = vec![];
        for o in &list {
            if !(o.ext == "same" || ext_ok(&o.ext)) {
                is.push(format!("{w}.outputs 的扩展名只能用小写字母和数字（或 same）：{}", o.ext));
            }
            if seen.contains(&o.ext) {
                is.push(format!("{w}.outputs 重复：{}", o.ext));
            }
            seen.push(o.ext.clone());
        }
        let mut okeys = vec![];
        for (j, o) in self.options.iter().enumerate() {
            if o.key.is_empty() || o.label.is_empty() {
                is.push(format!("{w}.options[{j}] 要有 key 和 label"));
            }
            if okeys.contains(&o.key) {
                is.push(format!("{w}.options[{j}].key 重复：{}", o.key));
            }
            okeys.push(o.key.clone());
            if !OPTION_TYPES.contains(&o.kind.as_str()) {
                is.push(format!("{w}.options[{j}].type 只能是 string / bool / int / select"));
            }
            if o.kind == "select" && o.options.is_empty() {
                is.push(format!("{w}.options[{j}] 是 select，要写 options"));
            }
        }
        if !self.exec.is_empty() {
            let first = &self.exec[0];
            if Path::new(first).is_absolute() || first.starts_with(['/', '\\']) {
                is.push(format!("{w}.exec 不能写绝对路径，用相对插件文件夹的路径"));
            } else if !first.starts_with('{') {
                let looks_local = first.contains(['/', '\\']) || Path::new(first).extension().is_some();
                if looks_local && (!safe_rel(first) || !dir.join(first).is_file()) {
                    is.push(format!("{w}.exec 里的程序不存在：{first}"));
                }
            }
            let mut has_out = false;
            for a in &self.exec {
                match exec_vars(a) {
                    Ok(vars) => {
                        for v in vars {
                            if v == "output" || v == "output_dir" {
                                has_out = true;
                            }
                            if (v == "ffmpeg" || v == "ffprobe") && !m.permissions.tools.iter().any(|t| t == v) {
                                is.push(format!("{w}.exec 用了 {{{v}}}，要在 permissions.tools 里声明 {v}"));
                            }
                            if let Some(k) = v.strip_prefix("option:") {
                                if !self.options.iter().any(|o| o.key == k) {
                                    is.push(format!("{w}.exec 引用的参数不存在：{{{v}}}"));
                                }
                            } else if let Some(k) = v.strip_prefix("setting:") {
                                if !m.settings.iter().any(|s| s.key == k) {
                                    is.push(format!("{w}.exec 引用的设置项不存在：{{{v}}}"));
                                }
                            } else if !EXEC_VARS.contains(&v) {
                                is.push(format!("{w}.exec 不认识的占位符：{{{v}}}"));
                            }
                        }
                    }
                    Err(e) => is.push(format!("{w}.exec：{e}")),
                }
            }
            if !has_out {
                is.push(format!("{w}.exec 要用 {{output}}（或 {{output_dir}}）告诉程序输出到哪里"));
            }
        }
    }
}

/// 取出命令模板一项里的占位符名。`{{` 和 `}}` 表示字面的花括号
pub fn exec_vars(arg: &str) -> Result<Vec<&str>, String> {
    let mut out = vec![];
    let b = arg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => i += 2,
            b'}' if b.get(i + 1) == Some(&b'}') => i += 2,
            b'{' => {
                let end = arg[i + 1..].find('}').ok_or_else(|| format!("花括号没有闭合：{arg}"))?;
                out.push(&arg[i + 1..i + 1 + end]);
                i += end + 2;
            }
            b'}' => return Err(format!("多了一个 }}：{arg}")),
            _ => i += 1,
        }
    }
    Ok(out)
}

/// 按占位符填好命令模板的一项；`get` 返回 None 时报错
pub fn fill_exec(arg: &str, get: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let b = arg.as_bytes();
    let mut i = 0;
    let mut start = 0;
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => {
                out.push_str(&arg[start..i]);
                out.push('{');
                i += 2;
                start = i;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                out.push_str(&arg[start..i]);
                out.push('}');
                i += 2;
                start = i;
            }
            b'{' => {
                out.push_str(&arg[start..i]);
                let end = arg[i + 1..].find('}').ok_or_else(|| format!("花括号没有闭合：{arg}"))?;
                let name = &arg[i + 1..i + 1 + end];
                out.push_str(&get(name).ok_or_else(|| format!("占位符 {{{name}}} 没有值"))?);
                i += end + 2;
                start = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&arg[start..]);
    Ok(out)
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

    pub fn converter(&self, id: &str) -> Option<&ConverterDecl> {
        self.contributes.converters.iter().find(|c| c.id == id)
    }

    /// 只有 exec 转换器、没有下载源时，process 入口可以不写 runtime.command
    fn command_optional(&self) -> bool {
        let c = &self.contributes;
        c.sources.is_empty() && !c.converters.is_empty() && c.converters.iter().all(|x| !x.exec.is_empty())
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
                    None if self.command_optional() && self.runtime.command.is_empty() => {}
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
        if (rt == "js" || (rt == "process" && !self.command_optional())) && !PROTOCOLS.contains(&self.engines.protocol) {
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
        if c.sources.is_empty() && c.rules.is_empty() && c.converters.is_empty() {
            is.push("contributes 里至少要有 sources、rules 或 converters".into());
        }
        if rt != "process" && !c.converters.is_empty() {
            is.push("converters 只能用在 process 入口（转换要读写文件）".into());
        }
        let mut cseen = vec![];
        for (i, cv) in c.converters.iter().enumerate() {
            let w = format!("contributes.converters[{i}]");
            if cseen.contains(&cv.id) {
                is.push(format!("{w}.id 重复：{}", cv.id));
            }
            cseen.push(cv.id.clone());
            cv.check(&w, self, dir, &mut is);
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
    // Windows 上 `/etc/passwd` 也算相对路径（没有盘符），所以以斜杠开头的另外排除
    !p.is_empty()
        && !p.starts_with(['/', '\\'])
        && path.is_relative()
        && !path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
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
    fn converters_are_checked() {
        let ok = Manifest::parse(
            r#"{"manifest":1,"id":"wrap-tool","name":"包装","version":"1.0.0","runtime":{"type":"process"},
               "contributes":{"converters":[{"id":"main","inputs":["xyz","audio"],"outputs":["mp3","same"],
                 "options":[{"key":"q","type":"int","label":"质量","default":2}],
                 "exec":["{setting:tool}","-q","{option:q}","{input}","{output}"]}]},
               "settings":[{"key":"tool","type":"string","label":"程序","required":true}]}"#,
        )
        .unwrap();
        // 只有 exec 转换器时可以不写 runtime.command 和 protocol
        assert_eq!(ok.check(&std::env::temp_dir(), "0.6.0"), Vec::<String>::new());
        assert!(ok.converter("main").unwrap().accepts("XYZ"));
        assert!(ok.converter("main").unwrap().accepts("flac"));
        assert!(!ok.converter("main").unwrap().accepts(""));

        let bad = Manifest::parse(
            r#"{"manifest":1,"id":"bad-conv","name":"坏","version":"1.0.0","runtime":{"type":"js","file":"x.js"},"engines":{"protocol":2},
               "contributes":{"converters":[
                 {"id":"a","inputs":[".MP3"],"outputs":[],"options":[{"key":"s","type":"secret","label":"x"}]},
                 {"id":"a","inputs":["wav"],"outputs":["flac","flac",3],"exec":["/usr/bin/x","{input}","{ffmpeg}","{option:nope}","{huh}"]}
               ]}}"#,
        )
        .unwrap();
        let is = bad.check(&std::env::temp_dir(), "0.6.0").join("\n");
        for want in [
            "只能用在 process",
            "id 重复",
            "inputs 只能写",
            "至少写一种输出",
            "options[0].type",
            "outputs 重复",
            "outputs 每一项",
            "绝对路径",
            "permissions.tools",
            "参数不存在",
            "不认识的占位符",
            "{output}",
        ] {
            assert!(is.contains(want), "缺少提示 {want}：\n{is}");
        }
    }

    #[test]
    fn exec_placeholders() {
        assert_eq!(exec_vars("-o={output}.{output_ext}").unwrap(), vec!["output", "output_ext"]);
        assert_eq!(exec_vars("{{x}}").unwrap(), Vec::<&str>::new());
        assert!(exec_vars("{oops").is_err());
        assert!(exec_vars("a}b").is_err());
        let get = |n: &str| (n == "input").then(|| "in.wav".to_string());
        assert_eq!(fill_exec("--in={input} {{raw}}", &get).unwrap(), "--in=in.wav {raw}");
        assert!(fill_exec("{output}", &get).is_err());
    }

    #[test]
    fn relative_paths() {
        assert!(safe_rel("rules/a.em"));
        assert!(!safe_rel("../a.em"));
        assert!(!safe_rel("/etc/passwd"));
        assert!(!safe_rel("\\x\\a.em"));
        assert!(!safe_rel("a/../../b"));
        assert!(!safe_rel(""));
    }
}
