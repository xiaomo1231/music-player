//! 下载源接口（预留）：把“能搜索、能给出下载地址”的服务接进自动获取。
//! 音乐、视频、番剧、漫画、小说共用这一套：番剧可以给直链或磁力，漫画可以按章给图片地址（下完打包成 CBZ），
//! 小说可以按章给正文（下完生成 EPUB）。
//!
//! 目前内置 B 站（`bili::BiliProvider`）。以后有正规渠道时，写一个实现了 [`Provider`] 的结构体，
//! 在 [`builtin`] 里加一行即可：它会出现在“我的来源 → 添加下载源”里，搜索结果进入同一个搜索页，
//! 下载走同一个队列（HTTP 断点续传或 yt-dlp），下完自动补标签、存歌词、整理入库。
//! 完整说明见 `docs/下载源接口.md`。
//!
//! 只接你有权获取内容的服务：官方开放接口、自己的网盘或 NAS、已购买可下载的曲目、CC 授权曲库等。
//! 不接绕过付费、会员或其他访问限制的来源。
pub use super::pack::TextChapter;
use super::Hit;
use crate::parse;
use futures_util::future::BoxFuture;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// 下载源的基本信息（显示在“添加下载源”里）
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ProviderInfo {
    /// 唯一标识，只用小写字母、数字和 `-`，例如 `jamendo`
    pub id: String,
    /// 显示名
    pub name: String,
    /// 支持的媒体类型：`music` / `video` / `anime` / `manga`（漫画）/ `novel`（小说）
    pub media_kinds: Vec<String>,
    /// 一句话说明（授权范围、需要的账号等）
    pub note: String,
}

/// 一个可选的音质 / 清晰度档位
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Quality {
    /// 下载源自己认得的档位标识，原样传回 `resolve`
    pub key: String,
    /// 显示文字，例如“无损”“320K”“1080P”
    pub label: String,
    /// 文件格式：flac / mp3 / m4a / mp4 …
    pub format: String,
    /// 码率（kbps），没有可不填
    pub bitrate: Option<u32>,
    /// 大小（字节），没有可不填
    pub size: Option<i64>,
}

/// 搜索结果里的一项
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Item {
    /// 下载源内部的 id
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: Option<i64>,
    pub track: Option<i64>,
    /// 时长（秒）
    pub duration: Option<f64>,
    /// 封面网址
    pub cover: String,
    /// `music` / `video` / `anime` / `manga` / `novel`
    pub media_kind: String,
    /// 作品名（番剧 / 漫画 / 小说用）：搜索页按它分组，整理入库按它建文件夹。音乐不用填，用 `album`
    pub series: String,
    /// 第几季（番剧用，可选）
    pub season: Option<i64>,
    /// 第几集 / 第几话 / 第几章（可选）。填了以后搜索页能“追更”，新出的集数会自动下载
    pub episode: Option<f64>,
    /// 从好到差排列；第一项是默认档位
    pub qualities: Vec<Quality>,
    /// 下载源自用的附加数据（签名、会话令牌等），原样带回 `resolve`
    pub extra: serde_json::Value,
}

/// 下载方式
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Target {
    /// HTTP / HTTPS 直链，`ext` 为文件扩展名（不带点）。整本的 CBZ / EPUB / PDF 也用这个。
    /// `headers` 只能放 Referer、Accept、Accept-Language、Authorization（见 [`allowed_header`]）
    Http {
        url: String,
        ext: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
    /// 交给 yt-dlp 下载的页面地址（含 m3u8 等流媒体）。`format` 是 yt-dlp 的格式选择（`-f`），只用来选清晰度，空表示按设置
    Ytdlp {
        url: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        format: String,
    },
    /// 磁力链接或 .torrent 网址，走 BT 下载（下完即停或做种，按设置）
    Magnet { uri: String },
    /// 漫画一话 / 一卷的全部图片地址，按顺序下载后打包成 CBZ。`headers` 是取图时要带的请求头（如 Referer、授权令牌）
    Pages {
        urls: Vec<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// 小说若干章的正文，生成一本 EPUB
    Chapters { chapters: Vec<TextChapter> },
    /// 已经下好的本地文件（外部程序下载源自己下载时用），直接拿去补标签、整理入库
    File { path: String },
}

/// `resolve` 的结果：真正的下载地址，以及可选的歌词
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Resolved {
    pub target: Target,
    /// LRC 歌词全文，下载完成后存成同名 .lrc
    pub lyric: Option<String>,
}

/// 插件给出的直链请求头里允许的名字（不能带 Cookie：登录只来自用户自己在“网站登录”里保存的）
pub fn allowed_header(name: &str) -> bool {
    ["referer", "accept", "accept-language", "authorization"].contains(&name.to_ascii_lowercase().as_str())
}

/// yt-dlp 格式选择只允许这些字符（防止借 `-f` 夹带别的参数）
pub fn valid_ytdlp_format(f: &str) -> bool {
    f.len() <= 120 && !f.starts_with('-') && f.chars().all(|c| c.is_ascii_alphanumeric() || "*+/[]<>=!:._,-?^$|()".contains(c))
}

// ---------- 错误码 ----------

/// 插件可以返回的错误码（见 docs/插件开发.md）。错误文字写成 `[code] 给用户看的话`，前端据此给出“去登录”“打开设置”等按钮
pub const ERROR_CODES: [&str; 10] = [
    "login_required",
    "captcha",
    "forbidden",
    "not_found",
    "rate_limited",
    "network",
    "bad_config",
    "unsupported",
    "internal",
    "plugin_disabled",
];

/// 带错误码的错误文字
pub fn coded(code: &str, msg: &str) -> String {
    let code = if ERROR_CODES.contains(&code) { code } else { "internal" };
    format!("[{code}] {}", msg.trim())
}

/// 拆出错误码：`[login_required] 请先登录` → (Some("login_required"), "请先登录")
pub fn split_code(s: &str) -> (Option<&str>, &str) {
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((code, msg)) = rest.split_once("] ") {
            if ERROR_CODES.contains(&code) {
                return (Some(code), msg);
            }
        }
    }
    (None, s)
}

// ---------- 插件自己下载 ----------

/// 下载进度（插件自己下载时回报）
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Progress {
    pub done: f64,
    pub total: Option<f64>,
    /// bytes / pages / items
    pub unit: String,
    /// 字节 / 秒
    pub speed: Option<f64>,
    /// 状态文字，如“正在合并分段 3/12”
    pub text: String,
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;
/// 返回 true 表示用户暂停或取消了，插件应当尽快停下
pub type StopFn = Arc<dyn Fn() -> bool + Send + Sync>;

/// 插件下好的一个文件
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DownloadedFile {
    pub path: String,
    /// main / subtitle / lyric / cover
    #[serde(default = "main_role")]
    pub role: String,
}

fn main_role() -> String {
    "main".into()
}

/// 下载源要实现的接口
pub trait Provider: Send + Sync {
    fn info(&self) -> ProviderInfo;

    /// 按关键字搜索。`media_kind` 是搜索页当前选的类型，`page` 从 1 开始
    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>>;

    /// 下载开始前调用，把搜索结果换成下载地址。`quality` 是 `Quality::key`，空串表示默认档位。
    /// 直链会过期时也不用担心：每次开始 / 继续下载都会重新调用。
    fn resolve<'a>(&'a self, item: &'a Item, quality: &'a str) -> BoxFuture<'a, Result<Resolved, String>>;

    /// 能不能用 `resolve` 给出下载地址（由软件下载）。插件只声明了 `download` 时为 false
    fn can_resolve(&self) -> bool {
        true
    }

    /// 能不能把一部作品展开成各集 / 话 / 章（插件的 `episodes` 能力、.em 规则的 `episodes` 阶段）
    fn can_episodes(&self) -> bool {
        false
    }

    /// 搜索页用：只取作品列表，不展开分集（点开作品卡时再调 [`Provider::episodes`]）。
    /// 默认和 `search` 一样；有分集能力的下载源要覆盖它
    fn search_works<'a>(&'a self, query: &'a str, media_kind: &'a str, page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        self.search(query, media_kind, page)
    }

    /// 一部作品（`search_works` 给出的一条）→ 各集 / 话 / 章
    fn episodes<'a>(&'a self, _work: &'a Item) -> BoxFuture<'a, Result<Vec<Item>, String>> {
        Box::pin(async { Err(coded("unsupported", "这个下载源不能列出分集")) })
    }

    /// 由下载源自己把文件下到 `dest` 文件夹里（插件的 `download` 能力）。`resume` 表示上次没下完，`dest` 里留着上次的文件
    fn download<'a>(
        &'a self,
        _item: &'a Item,
        _quality: &'a str,
        _dest: &'a Path,
        _resume: bool,
        _progress: ProgressFn,
        _stop: StopFn,
    ) -> BoxFuture<'a, Result<Vec<DownloadedFile>, String>> {
        Box::pin(async { Err(coded("unsupported", "这个下载源不支持自己下载")) })
    }
}

/// 已注册的下载源
static PROVIDERS: Lazy<RwLock<Vec<Arc<dyn Provider>>>> = Lazy::new(|| RwLock::new(builtin()));

/// 随软件一起编译进来的下载源。接入新的下载源时在这里加一行，例如
/// `Arc::new(jamendo::Jamendo::default()),`
fn builtin() -> Vec<Arc<dyn Provider>> {
    vec![Arc::new(super::bili::BiliProvider)]
}

/// 运行时注册（同 id 会替换旧的）
pub fn register(p: Arc<dyn Provider>) {
    let id = p.info().id;
    let mut list = PROVIDERS.write();
    list.retain(|x| x.info().id != id);
    list.push(p);
}

/// 移除一个运行时注册的下载源（删除外部程序来源时用）
pub fn unregister(id: &str) {
    PROVIDERS.write().retain(|x| x.info().id != id);
}

pub fn list() -> Vec<ProviderInfo> {
    PROVIDERS.read().iter().map(|p| p.info()).collect()
}

pub fn get(id: &str) -> Option<Arc<dyn Provider>> {
    PROVIDERS.read().iter().find(|p| p.info().id == id).cloned()
}

// ---------- 来源地址与任务地址 ----------

pub const SCHEME: &str = "provider://";

/// 来源表里保存的地址：`provider://<id>`
pub fn source_url(id: &str) -> String {
    format!("{SCHEME}{id}")
}

/// 从来源地址或任务地址里取出下载源 id
pub fn provider_id(url: &str) -> Option<&str> {
    let rest = url.strip_prefix(SCHEME)?;
    let id = rest.split(['/', '?']).next().unwrap_or("");
    (!id.is_empty()).then_some(id)
}

/// 下载任务里保存的地址：`provider://<id>/item?q=<档位>&d=<搜索结果 JSON>`。
/// 整个搜索结果都带上，重启后继续下载时也能重新取下载地址
pub fn task_url(provider: &str, item: &Item, quality: &str) -> String {
    let json = serde_json::to_string(item).unwrap_or_default();
    let qs: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", quality)
        .append_pair("d", &json)
        .finish();
    format!("{SCHEME}{provider}/item?{qs}")
}

/// 解析任务地址，返回（下载源 id，搜索结果，档位）
pub fn parse_task_url(u: &str) -> Option<(String, Item, String)> {
    let id = provider_id(u)?.to_string();
    let (_, qs) = u.split_once('?')?;
    let mut item = None;
    let mut quality = String::new();
    for (k, v) in url::form_urlencoded::parse(qs.as_bytes()) {
        match k.as_ref() {
            "d" => item = serde_json::from_str::<Item>(&v).ok(),
            "q" => quality = v.into_owned(),
            _ => {}
        }
    }
    Some((id, item?, quality))
}

/// 集数显示：3 → “03”，3.5 → “3.5”
pub fn ep_num(ep: f64) -> String {
    if ep.fract() == 0.0 {
        format!("{:02}", ep as i64)
    } else {
        ep.to_string()
    }
}

/// 下载目录里的文件名，整理入库时会从它认出作品和集数：
/// - 音乐 / 视频：`歌名 - 艺人.ext`（没有艺人时只用歌名）
/// - 番剧：`作品名 - 03.ext`（有季时 `作品名 S02E03.ext`）
/// - 漫画：`作品名 第03话.cbz`
/// - 小说：`书名 - 作者.epub`
pub fn file_name(item: &Item, ext: &str) -> String {
    let series = item.series.trim();
    let base = match (item.media_kind.as_str(), item.episode) {
        ("anime", Some(ep)) if !series.is_empty() => match item.season {
            Some(s) if s > 1 => format!("{series} S{s:02}E{}", ep_num(ep)),
            _ => format!("{series} - {}", ep_num(ep)),
        },
        ("manga", Some(ep)) if !series.is_empty() => format!("{series} 第{}话", ep_num(ep)),
        ("manga", _) if !series.is_empty() && series != item.title.trim() => format!("{series} {}", item.title.trim()),
        ("novel", _) => {
            let t = if series.is_empty() { item.title.trim() } else { series };
            if item.artist.trim().is_empty() {
                t.to_string()
            } else {
                format!("{t} - {}", item.artist.trim())
            }
        }
        _ if item.artist.trim().is_empty() => item.title.trim().to_string(),
        _ => format!("{} - {}", item.title.trim(), item.artist.trim()),
    };
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    let ext = if ext.is_empty() { "bin".to_string() } else { ext };
    format!("{}.{ext}", super::organize::sanitize(&base))
}

// ---------- 搜索 ----------

fn get_or_missing(id: &str) -> Result<Arc<dyn Provider>, String> {
    get(id).ok_or_else(|| crate::plugin::missing_reason(id).unwrap_or_else(|| format!("这个版本的软件里没有下载源“{id}”")))
}

/// 搜索一个下载源，把结果转换成搜索页通用的 `Hit`。
/// `works` 为 true（搜索页）时，有分集能力的下载源只返回作品，作品条目标上 `expand`，点开时再用 [`episodes`] 取各集；
/// 为 false（追更）时照旧展开成各集
pub async fn search(row: &super::SourceRow, query: &str, media_kind: &str, works: bool) -> Result<Vec<Hit>, String> {
    let id = provider_id(&row.url).ok_or("来源地址不正确")?;
    let p = get_or_missing(id)?;
    let info = p.info();
    if !info.media_kinds.is_empty() && !info.media_kinds.iter().any(|k| k == media_kind) {
        return Ok(vec![]);
    }
    if works && p.can_episodes() {
        let items = p.search_works(query, media_kind, 1).await?;
        return Ok(items
            .iter()
            .map(|it| {
                let mut h = to_hit(id, it);
                h.expand = it.episode.is_none();
                h
            })
            .collect());
    }
    let items = p.search(query, media_kind, 1).await?;
    Ok(items.iter().map(|it| to_hit(id, it)).collect())
}

/// 搜索页点开作品卡：把一条作品结果（`Hit.url`）展开成各集，按集数排好
pub async fn episodes(work_url: &str) -> Result<Vec<Hit>, String> {
    let (id, work, _) = parse_task_url(work_url).ok_or("作品地址不正确")?;
    let p = get_or_missing(&id)?;
    if !p.can_episodes() {
        return Err(coded("unsupported", "这个下载源不能列出分集"));
    }
    let eps = p.episodes(&work).await?;
    Ok(episode_hits(&id, &work, eps))
}

/// 各集转成 `Hit`：没写作品名、封面的用作品的；有集数的按集数排，没集数的保持原顺序排在后面
pub fn episode_hits(provider: &str, work: &Item, eps: Vec<Item>) -> Vec<Hit> {
    let series = if work.series.trim().is_empty() { &work.title } else { &work.series };
    let mut eps: Vec<Item> = eps
        .into_iter()
        .map(|mut e| {
            if e.series.trim().is_empty() {
                e.series = series.clone();
            }
            if e.cover.is_empty() {
                e.cover = work.cover.clone();
            }
            e
        })
        .collect();
    eps.sort_by(|a, b| match (a.episode, b.episode) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    eps.iter().map(|e| to_hit(provider, e)).collect()
}

pub fn to_hit(provider: &str, it: &Item) -> Hit {
    let q = it.qualities.first();
    let mut title = if it.artist.trim().is_empty() {
        it.title.clone()
    } else {
        format!("{} - {}", it.title, it.artist)
    };
    if let Some(q) = q.filter(|q| !q.label.is_empty()) {
        title.push_str(&format!(" [{}]", q.label));
    }
    // 作品名：番剧 / 漫画 / 小说用 series，音乐用专辑，都没有就用标题
    let work = [&it.series, &it.album, &it.title]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_default();
    Hit {
        title,
        url: task_url(provider, it, q.map(|q| q.key.as_str()).unwrap_or("")),
        size: q.and_then(|q| q.size),
        kind: "provider".into(),
        work,
        season: it.season,
        episode: it.episode,
        resolution: q.map(|q| q.label.clone()).filter(|s| !s.is_empty()),
        group: (!it.artist.is_empty()).then(|| it.artist.clone()),
        ..Default::default()
    }
}

// ---------- 下载后处理 ----------

/// 音乐文件缺标签时用搜索结果补上（整理入库按“艺人 / 专辑”归档，缺艺人会被留给手动整理）
pub fn fill_tags(path: &Path, item: &Item) -> anyhow::Result<()> {
    use lofty::config::WriteOptions;
    use lofty::file::TaggedFileExt;
    use lofty::prelude::{Accessor, TagExt};
    use lofty::tag::Tag;
    if parse::classify(path) != Some("music") {
        return Ok(());
    }
    let mut tf = lofty::read_from_path(path)?;
    if tf.primary_tag().is_none() {
        let t = tf.primary_tag_type();
        tf.insert_tag(Tag::new(t));
    }
    let tag = tf.primary_tag_mut().ok_or_else(|| anyhow::anyhow!("无法写入标签"))?;
    let empty = |s: Option<std::borrow::Cow<'_, str>>| s.map_or(true, |v| v.trim().is_empty());
    if empty(tag.title()) && !item.title.is_empty() {
        tag.set_title(item.title.clone());
    }
    if empty(tag.artist()) && !item.artist.is_empty() {
        tag.set_artist(item.artist.clone());
    }
    if empty(tag.album()) && !item.album.is_empty() {
        tag.set_album(item.album.clone());
    }
    if tag.track().is_none() {
        if let Some(n) = item.track.filter(|n| *n > 0) {
            tag.set_track(n as u32);
        }
    }
    tag.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

/// 歌词存成同名 .lrc（整理入库时跟着音乐文件一起移动）
pub fn save_lyric(path: &Path, lyric: &str) -> std::io::Result<()> {
    if lyric.trim().is_empty() {
        return Ok(());
    }
    std::fs::write(path.with_extension("lrc"), lyric)
}

// ---------- 测试 ----------

#[cfg(test)]
pub mod tests {
    use super::*;

    /// 测试用的假下载源（不联网）
    pub struct Mock;

    impl Provider for Mock {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                id: "mock".into(),
                name: "测试源".into(),
                media_kinds: vec!["music".into()],
                note: String::new(),
            }
        }
        fn search<'a>(&'a self, query: &'a str, _mk: &'a str, _page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
            Box::pin(async move { Ok(vec![sample(query)]) })
        }
        fn resolve<'a>(&'a self, item: &'a Item, quality: &'a str) -> BoxFuture<'a, Result<Resolved, String>> {
            Box::pin(async move {
                Ok(Resolved {
                    target: Target::Http {
                        url: format!("https://example.invalid/{}.{quality}", item.id),
                        ext: quality.into(),
                        headers: BTreeMap::new(),
                    },
                    lyric: Some("[00:00.00]测试".into()),
                })
            })
        }
    }

    fn sample(title: &str) -> Item {
        Item {
            id: "42".into(),
            title: title.into(),
            artist: "林间信号".into(),
            album: "潮汐档案".into(),
            track: Some(3),
            media_kind: "music".into(),
            qualities: vec![
                Quality {
                    key: "flac".into(),
                    label: "无损".into(),
                    format: "flac".into(),
                    bitrate: None,
                    size: Some(30_000_000),
                },
                Quality {
                    key: "mp3".into(),
                    label: "320K".into(),
                    format: "mp3".into(),
                    bitrate: Some(320),
                    size: None,
                },
            ],
            extra: serde_json::json!({ "sign": "a&b=c 中文" }),
            ..Default::default()
        }
    }

    /// 有分集能力的假下载源：搜索给两部作品，episodes 给乱序的各话
    struct Works;

    impl Provider for Works {
        fn info(&self) -> ProviderInfo {
            ProviderInfo {
                id: "works".into(),
                name: "分集测试源".into(),
                media_kinds: vec!["manga".into()],
                note: String::new(),
            }
        }
        fn can_episodes(&self) -> bool {
            true
        }
        fn search<'a>(&'a self, _q: &'a str, _mk: &'a str, _page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
            Box::pin(async { Err("追更才会走这里".to_string()) })
        }
        fn search_works<'a>(&'a self, _q: &'a str, _mk: &'a str, _page: u32) -> BoxFuture<'a, Result<Vec<Item>, String>> {
            Box::pin(async {
                Ok(vec![
                    Item {
                        id: "w1".into(),
                        title: "星屑邮局".into(),
                        cover: "https://example.invalid/c.jpg".into(),
                        media_kind: "manga".into(),
                        ..Default::default()
                    },
                    Item {
                        id: "w2#1".into(),
                        title: "单话".into(),
                        series: "潮汐档案".into(),
                        episode: Some(1.0),
                        media_kind: "manga".into(),
                        ..Default::default()
                    },
                ])
            })
        }
        fn episodes<'a>(&'a self, work: &'a Item) -> BoxFuture<'a, Result<Vec<Item>, String>> {
            Box::pin(async move {
                Ok([Some(2.0), None, Some(1.0), Some(10.0)]
                    .iter()
                    .enumerate()
                    .map(|(i, ep)| Item {
                        id: format!("{}#{i}", work.id),
                        title: format!("第{i}话"),
                        episode: *ep,
                        media_kind: "manga".into(),
                        ..Default::default()
                    })
                    .collect())
            })
        }
        fn resolve<'a>(&'a self, _item: &'a Item, _q: &'a str) -> BoxFuture<'a, Result<Resolved, String>> {
            Box::pin(async { Err("不下载".to_string()) })
        }
    }

    #[tokio::test]
    async fn search_works_then_episodes() {
        register(Arc::new(Works));
        let row = super::super::SourceRow {
            id: 1,
            kind: "provider".into(),
            name: "分集测试源".into(),
            url: source_url("works"),
            rule_json: String::new(),
            enabled: true,
            created_at: 0,
        };
        // 搜索页：只拿作品，没集数的作品标上 expand
        let hits = search(&row, "星屑", "manga", true).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].expand);
        assert_eq!(hits[0].work, "星屑邮局");
        assert!(!hits[1].expand);
        // 追更：走 search（这里故意报错，说明没有用 search_works）
        assert!(search(&row, "星屑", "manga", false).await.is_err());
        // 点开作品：按集数排序，作品名和封面从作品补上
        let eps = episodes(&hits[0].url).await.unwrap();
        let nums: Vec<Option<f64>> = eps.iter().map(|h| h.episode).collect();
        assert_eq!(nums, vec![Some(1.0), Some(2.0), Some(10.0), None]);
        assert!(eps.iter().all(|h| h.work == "星屑邮局" && !h.expand));
        let (_, first, _) = parse_task_url(&eps[0].url).unwrap();
        assert_eq!(first.cover, "https://example.invalid/c.jpg");
        unregister("works");
    }

    #[tokio::test]
    async fn episodes_needs_capability() {
        let u = task_url("mock-none", &Item::default(), "");
        assert!(episodes(&u).await.is_err());
    }

    #[test]
    fn task_url_round_trip() {
        let it = sample("防波堤");
        let u = task_url("mock", &it, "flac");
        assert!(u.starts_with("provider://mock/item?"));
        let (id, back, q) = parse_task_url(&u).unwrap();
        assert_eq!(id, "mock");
        assert_eq!(q, "flac");
        assert_eq!(back, it);
    }

    #[test]
    fn provider_id_from_urls() {
        assert_eq!(provider_id("provider://jamendo"), Some("jamendo"));
        assert_eq!(provider_id("provider://jamendo/item?q=1"), Some("jamendo"));
        assert_eq!(provider_id("provider://"), None);
        assert_eq!(provider_id("https://a.b"), None);
        assert_eq!(source_url("x-1"), "provider://x-1");
    }

    #[test]
    fn file_names() {
        let it = sample("防波堤?");
        assert_eq!(file_name(&it, ".FLAC"), "防波堤 - 林间信号.flac");
        let mut solo = it.clone();
        solo.artist.clear();
        assert_eq!(file_name(&solo, ""), "防波堤.bin");
    }

    #[test]
    fn hits_use_first_quality() {
        let h = to_hit("mock", &sample("防波堤"));
        assert_eq!(h.kind, "provider");
        assert_eq!(h.title, "防波堤 - 林间信号 [无损]");
        assert_eq!(h.size, Some(30_000_000));
        assert_eq!(h.work, "潮汐档案");
        let (_, _, q) = parse_task_url(&h.url).unwrap();
        assert_eq!(q, "flac");
    }

    #[test]
    fn registry_and_mock_flow() {
        register(Arc::new(Mock));
        register(Arc::new(Mock)); // 同 id 替换，不重复
        assert_eq!(list().iter().filter(|p| p.id == "mock").count(), 1);
        let p = get("mock").unwrap();
        let items = tauri::async_runtime::block_on(p.search("防波堤", "music", 1)).unwrap();
        let r = tauri::async_runtime::block_on(p.resolve(&items[0], "flac")).unwrap();
        assert_eq!(
            r.target,
            Target::Http {
                url: "https://example.invalid/42.flac".into(),
                ext: "flac".into(),
                headers: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn file_names_for_series() {
        let anime = Item {
            title: "邮差与星".into(),
            series: "星屑邮局".into(),
            episode: Some(3.0),
            media_kind: "anime".into(),
            ..Default::default()
        };
        assert_eq!(file_name(&anime, "mkv"), "星屑邮局 - 03.mkv");
        let s2 = Item {
            season: Some(2),
            episode: Some(12.5),
            ..anime.clone()
        };
        assert_eq!(file_name(&s2, "mp4"), "星屑邮局 S02E12.5.mp4");
        let manga = Item {
            media_kind: "manga".into(),
            ..anime.clone()
        };
        assert_eq!(file_name(&manga, "cbz"), "星屑邮局 第03话.cbz");
        let vol = Item {
            title: "第2卷".into(),
            episode: None,
            ..manga.clone()
        };
        assert_eq!(file_name(&vol, "cbz"), "星屑邮局 第2卷.cbz");
        let novel = Item {
            media_kind: "novel".into(),
            series: "潮汐档案".into(),
            artist: "林间".into(),
            ..Default::default()
        };
        assert_eq!(file_name(&novel, "epub"), "潮汐档案 - 林间.epub");
        // 整理入库能从文件名认回作品和集数
        let info = parse::parse_episode(Path::new("星屑邮局 - 03.mkv"));
        assert_eq!((info.title.as_str(), info.episode), ("星屑邮局", Some(3.0)));
    }

    #[test]
    fn hits_carry_series_and_episode() {
        let it = Item {
            title: "邮差与星".into(),
            series: "星屑邮局".into(),
            season: Some(1),
            episode: Some(4.0),
            media_kind: "anime".into(),
            ..Default::default()
        };
        let h = to_hit("mock", &it);
        assert_eq!(h.work, "星屑邮局");
        assert_eq!((h.season, h.episode), (Some(1), Some(4.0)));
        // 旧格式（没有新字段）的任务地址仍能读回
        let old = r#"{"id":"1","title":"x","media_kind":"music","qualities":[]}"#;
        let back: Item = serde_json::from_str(old).unwrap();
        assert_eq!((back.series.as_str(), back.episode), ("", None));
    }

    #[test]
    fn targets_serialize_with_type_tag() {
        let t = Target::Pages {
            urls: vec!["https://img.example/1.jpg".into()],
            headers: BTreeMap::from([("Referer".to_string(), "https://example".to_string())]),
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["type"], "pages");
        assert_eq!(
            serde_json::from_value::<Target>(serde_json::json!({"type":"pages","urls":["a"]})).unwrap(),
            Target::Pages {
                urls: vec!["a".into()],
                headers: BTreeMap::new()
            }
        );
        let m: Target = serde_json::from_str(r#"{"type":"magnet","uri":"magnet:?xt=urn:btih:abc"}"#).unwrap();
        assert_eq!(
            m,
            Target::Magnet {
                uri: "magnet:?xt=urn:btih:abc".into()
            }
        );
        let c: Target = serde_json::from_str(r#"{"type":"chapters","chapters":[{"title":"一","text":"正文"}]}"#).unwrap();
        assert_eq!(
            c,
            Target::Chapters {
                chapters: vec![TextChapter {
                    title: "一".into(),
                    text: "正文".into()
                }]
            }
        );
    }

    #[test]
    fn lyric_saved_next_to_file() {
        let dir = std::env::temp_dir().join(format!("mp-provider-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("曲 - 艺人.flac");
        save_lyric(&f, "[00:01.00]一").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("曲 - 艺人.lrc")).unwrap(), "[00:01.00]一");
        save_lyric(&dir.join("空.mp3"), "  ").unwrap();
        assert!(!dir.join("空.lrc").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 生成 0.1 秒静音 WAV
    fn silent_wav(p: &Path) {
        let n: u32 = 4410 * 2;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + n).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(44100u32.to_le_bytes());
        b.extend((44100u32 * 2).to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(n.to_le_bytes());
        b.extend(vec![0u8; n as usize]);
        std::fs::write(p, b).unwrap();
    }

    #[test]
    fn tags_filled_when_missing() {
        use lofty::file::TaggedFileExt;
        use lofty::prelude::Accessor;
        let dir = std::env::temp_dir().join(format!("mp-provider-tag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.wav");
        silent_wav(&f);
        fill_tags(&f, &sample("防波堤")).unwrap();
        let tf = lofty::read_from_path(&f).unwrap();
        let tag = tf.primary_tag().or_else(|| tf.first_tag()).unwrap();
        assert_eq!(tag.artist().as_deref(), Some("林间信号"));
        assert_eq!(tag.album().as_deref(), Some("潮汐档案"));
        assert_eq!(tag.title().as_deref(), Some("防波堤"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 前端换档位后用 URLSearchParams 重新编码的地址，后端要能读回
    #[test]
    fn parses_url_reencoded_by_frontend() {
        let u = "provider://mock/item?q=mp3&d=%7B%22id%22%3A%2242%22%2C%22title%22%3A%22%E9%98%B2%E6%B3%A2%E5%A0%A4%22%2C%22artist%22%3A%22%E6%9E%97%E9%97%B4%E4%BF%A1%E5%8F%B7%22%2C%22album%22%3A%22%E6%BD%AE%E6%B1%90%E6%A1%A3%E6%A1%88%22%2C%22year%22%3Anull%2C%22track%22%3A3%2C%22duration%22%3A205%2C%22cover%22%3A%22%22%2C%22media_kind%22%3A%22music%22%2C%22qualities%22%3A%5B%7B%22key%22%3A%22flac%22%2C%22label%22%3A%22%E6%97%A0%E6%8D%9F%22%2C%22format%22%3A%22flac%22%2C%22bitrate%22%3Anull%2C%22size%22%3A30000000%7D%2C%7B%22key%22%3A%22mp3%22%2C%22label%22%3A%22320K%22%2C%22format%22%3A%22mp3%22%2C%22bitrate%22%3A320%2C%22size%22%3A8000000%7D%5D%2C%22extra%22%3A%7B%22sign%22%3A%22a%26b%3Dc+%E4%B8%AD%E6%96%87%2B%22%7D%7D";
        let (id, it, q) = parse_task_url(u).unwrap();
        assert_eq!((id.as_str(), q.as_str()), ("mock", "mp3"));
        assert_eq!(it.title, "防波堤");
        assert_eq!(it.qualities[1].bitrate, Some(320));
        assert_eq!(it.extra["sign"], "a&b=c 中文+");
    }
}
