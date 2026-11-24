//! 插件提供的下载源（process 入口，协议 2）：注册到 `fetch::provider`，搜索、下载走自动获取现有的界面和队列
use super::manifest::{provider_id, SourceDecl};
use super::process::{self, Call, Event};
use crate::fetch::external;
use crate::fetch::provider::{self, DownloadedFile, Item, Progress, ProgressFn, Provider, ProviderInfo, Resolved, StopFn, Target};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// 有 episodes 能力时，搜索结果里前几部作品展开成各集
pub const MAX_WORKS: usize = 3;

pub struct PluginSource {
    pub plugin: String,
    pub decl: SourceDecl,
    pub info: ProviderInfo,
}

impl PluginSource {
    pub fn new(plugin: &str, plugin_name: &str, decl: &SourceDecl) -> PluginSource {
        let name = if decl.name.trim().is_empty() {
            plugin_name.to_string()
        } else {
            decl.name.clone()
        };
        PluginSource {
            plugin: plugin.to_string(),
            info: ProviderInfo {
                id: provider_id(plugin, &decl.id),
                name,
                media_kinds: decl.media_kinds.clone(),
                note: decl.note.clone(),
            },
            decl: decl.clone(),
        }
    }

    /// 调一次插件，记日志和状态
    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        idle: Option<Duration>,
        progress: Option<ProgressFn>,
        stop: Option<StopFn>,
    ) -> Result<Value, String> {
        let rt = super::runtime(&self.plugin)?;
        let mut params = params;
        params["source"] = json!(self.decl.id);
        let c = Call {
            argv: rt.argv,
            dir: rt.dir,
            env: rt.env,
            method: method.to_string(),
            params,
            settings: rt.settings,
            context: rt.context,
            timeout: Some(timeout),
            idle,
        };
        let plugin = self.plugin.clone();
        let m = method.to_string();
        let started = std::time::Instant::now();
        let r = process::run(
            &c,
            |e| match e {
                Event::Log { level, msg } => super::log(&plugin, &m, &level, &msg),
                Event::Progress(p) => {
                    if let Some(f) = &progress {
                        f(p)
                    }
                }
            },
            stop,
        )
        .await;
        let ms = started.elapsed().as_millis() as u64;
        match &r {
            Ok(_) => super::log(&self.plugin, method, "call", &format!("完成，用时 {ms} 毫秒")),
            Err(e) => super::log(&self.plugin, method, "error", e),
        }
        super::set_status(&self.info.id, method, r.as_ref().err().map(|s| s.as_str()));
        r
    }

    pub async fn episodes(&self, work: &Item) -> Result<Vec<Item>, String> {
        let v = self
            .call("episodes", json!({ "item": work }), Duration::from_secs(60), None, None, None)
            .await?;
        let mut eps = external::parse_items(v, &work.media_kind).map_err(|m| provider::coded("internal", &m))?;
        for e in eps.iter_mut() {
            if e.series.trim().is_empty() {
                e.series = if work.series.is_empty() { work.title.clone() } else { work.series.clone() };
            }
            if e.cover.is_empty() {
                e.cover = work.cover.clone();
            }
            if e.artist.is_empty() {
                e.artist = work.artist.clone();
            }
        }
        Ok(eps)
    }
}

/// 下载地址必须在插件声明的网站里（所有插件都检查）
pub fn check_target(plugin: &str, t: &Target) -> Result<(), String> {
    let urls: Vec<&String> = match t {
        Target::Http { url, .. } | Target::Ytdlp { url, .. } => vec![url],
        Target::Pages { urls, .. } => urls.iter().collect(),
        _ => vec![],
    };
    let allow = super::allow_of(plugin);
    for u in urls {
        let parsed = url::Url::parse(u).map_err(|_| provider::coded("internal", &format!("下载地址不正确：{u}")))?;
        allow
            .check(&parsed)
            .map_err(|_| provider::coded("forbidden", &format!("下载地址的网站 {} 不在插件声明的网站里", parsed.host_str().unwrap_or(""))))?;
    }
    if let Target::Http { headers, .. } | Target::Pages { headers, .. } = t {
        if let Some(k) = headers.keys().find(|k| !provider::allowed_header(k)) {
            return Err(provider::coded("forbidden", &format!("插件不能设置请求头 {k}")));
        }
    }
    if let Target::Ytdlp { format, .. } = t {
        if !format.is_empty() && !provider::valid_ytdlp_format(format) {
            return Err(provider::coded("forbidden", &format!("yt-dlp 格式选择不正确：{format}")));
        }
    }
    Ok(())
}

/// `search` 的返回：数组或 `{items, has_more}`
pub fn parse_search(v: Value, media_kind: &str) -> Result<Vec<Item>, String> {
    external::parse_items(v, media_kind).map_err(|m| provider::coded("internal", &m))
}

impl Provider for PluginSource {
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }

    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async move {
            let v = self
                .call(
                    "search",
                    json!({ "query": query, "media_kind": media_kind, "page": page }),
                    Duration::from_secs(60),
                    None,
                    None,
                    None,
                )
                .await?;
            let items = parse_search(v, media_kind)?;
            if !self.decl.can("episodes") {
                return Ok(items);
            }
            // 作品展开成各集：前几部没有集数的结果调 episodes，其他照原样
            let mut out = vec![];
            let mut works = 0;
            for it in items {
                if it.episode.is_none() && works < MAX_WORKS {
                    works += 1;
                    match self.episodes(&it).await {
                        Ok(eps) if !eps.is_empty() => out.extend(eps),
                        Ok(_) => out.push(it),
                        Err(e) => {
                            super::log(&self.plugin, "episodes", "warn", &e);
                            out.push(it);
                        }
                    }
                } else {
                    out.push(it);
                }
            }
            Ok(out)
        })
    }

    fn can_resolve(&self) -> bool {
        self.decl.can("resolve")
    }

    fn resolve<'a>(&'a self, item: &'a Item, quality: &'a str) -> BoxFuture<'a, Result<Resolved, String>> {
        Box::pin(async move {
            if !self.decl.can("resolve") {
                return Err(provider::coded("unsupported", "这个下载源只能自己下载"));
            }
            let v = self
                .call(
                    "resolve",
                    json!({ "item": item, "quality": quality }),
                    Duration::from_secs(120),
                    None,
                    None,
                    None,
                )
                .await?;
            let r = external::parse_resolved(v).map_err(|m| provider::coded("internal", &m))?;
            check_target(&self.plugin, &r.target)?;
            Ok(r)
        })
    }

    fn download<'a>(
        &'a self,
        item: &'a Item,
        quality: &'a str,
        dest: &'a Path,
        resume: bool,
        progress: ProgressFn,
        stop: StopFn,
    ) -> BoxFuture<'a, Result<Vec<DownloadedFile>, String>> {
        Box::pin(async move {
            if !self.decl.can("download") {
                return Err(provider::coded("unsupported", "这个下载源不支持自己下载"));
            }
            let v = self
                .call(
                    "download",
                    json!({ "item": item, "quality": quality, "dest_dir": dest.to_string_lossy(), "resume": resume }),
                    Duration::from_secs(24 * 3600),
                    Some(Duration::from_secs(300)),
                    Some(progress),
                    Some(stop),
                )
                .await?;
            parse_files(v)
        })
    }
}

pub fn parse_files(v: Value) -> Result<Vec<DownloadedFile>, String> {
    let files = v.get("files").cloned().unwrap_or(Value::Null);
    let files: Vec<DownloadedFile> = serde_json::from_value(files).map_err(|e| provider::coded("internal", &format!("download 的返回格式不对：{e}")))?;
    if files.is_empty() {
        return Err(provider::coded("internal", "插件没有返回下载好的文件"));
    }
    for f in &files {
        if !["main", "subtitle", "lyric", "cover"].contains(&f.role.as_str()) {
            return Err(provider::coded("internal", &format!("不认识的文件用途：{}", f.role)));
        }
    }
    Ok(files)
}

/// 测试面板也用：进度事件转成文字
pub fn progress_text(p: &Progress) -> String {
    match p.total {
        Some(t) => format!("{}/{} {}", p.done, t, p.unit),
        None => format!("{} {}", p.done, p.unit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files() {
        let f = parse_files(json!({ "files": [{ "path": "a.flac" }, { "path": "a.lrc", "role": "lyric" }] })).unwrap();
        assert_eq!(f[0].role, "main");
        assert!(parse_files(json!({ "files": [] })).is_err());
        assert!(parse_files(json!({ "files": [{ "path": "a", "role": "exe" }] })).is_err());
    }
}
