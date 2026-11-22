//! 资料源接口（预留）：按名字查番剧 / 漫画 / 小说 / 影视的资料（封面、简介、集数、每集标题、放送日期）。
//!
//! 和下载源（`fetch/provider.rs`）分开：资料源只提供“这是什么作品”，不提供下载地址。
//! 软件本身不带任何资料源，[`builtin`] 为空。以后接 Bangumi、TMDB 这类有公开接口的服务时，
//! 写一个实现了 [`MetaProvider`] 的结构体，在 [`builtin`] 里加一行即可：
//! 自动获取的搜索页会在结果上方显示作品资料卡（封面、简介、共几集、每集标题）。
//! 完整说明见 `docs/漫画番剧来源接口.md`。
use futures_util::future::BoxFuture;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 资料源的基本信息
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MetaInfo {
    /// 唯一标识，只用小写字母、数字和 `-`，例如 `bangumi`
    pub id: String,
    pub name: String,
    /// 支持的类型：`anime` / `manga` / `novel` / `video` / `music`；留空表示都支持
    pub media_kinds: Vec<String>,
    /// 一句话说明
    pub note: String,
    /// 作品页网址模板，`{id}` 会换成作品 id，例如 `https://bgm.tv/subject/{id}`；可留空
    pub link_template: String,
}

/// 一部作品
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Subject {
    /// 资料源内部 id
    pub id: String,
    /// 原名
    pub title: String,
    /// 中文名（没有就留空，界面优先显示它）
    pub title_cn: String,
    /// 别名，搜索结果按作品分组时也会用来匹配
    pub aliases: Vec<String>,
    /// `anime` / `manga` / `novel` / `video` / `music`
    pub media_kind: String,
    pub year: Option<i64>,
    /// 开播 / 发售日期，`YYYY-MM-DD`
    pub date: String,
    pub cover: String,
    pub summary: String,
    /// 总集数 / 总话数 / 总卷数，未知为 None
    pub total: Option<i64>,
    /// 评分（0–10）
    pub rating: Option<f64>,
    pub tags: Vec<String>,
    /// 作者 / 制作公司等
    pub staff: Vec<String>,
    /// 资料源自用数据
    pub extra: serde_json::Value,
}

/// 一集 / 一话
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct EpisodeMeta {
    /// 第几集（和文件名里认出的集数对应）
    pub sort: f64,
    pub title: String,
    pub title_cn: String,
    /// 放送日期 `YYYY-MM-DD`
    pub airdate: String,
    /// 时长（秒）
    pub duration: Option<f64>,
    /// 是否为特别篇 / SP
    pub special: bool,
}

/// 资料源要实现的接口
pub trait MetaProvider: Send + Sync {
    fn info(&self) -> MetaInfo;

    /// 按名字搜作品，`media_kind` 为搜索页当前选的类型。按匹配程度排序，第一项会作为资料卡显示
    fn search<'a>(&'a self, query: &'a str, media_kind: &'a str) -> BoxFuture<'a, Result<Vec<Subject>, String>>;

    /// 作品的分集列表；没有分集概念（如单本小说）时返回空
    fn episodes<'a>(&'a self, subject_id: &'a str) -> BoxFuture<'a, Result<Vec<EpisodeMeta>, String>>;
}

static PROVIDERS: Lazy<RwLock<Vec<Arc<dyn MetaProvider>>>> = Lazy::new(|| RwLock::new(builtin()));

/// 随软件一起编译进来的资料源。目前为空，接入时在这里加一行：
/// `Arc::new(bangumi::Bangumi::default()),`
fn builtin() -> Vec<Arc<dyn MetaProvider>> {
    vec![]
}

/// 运行时注册（同 id 会替换旧的）
pub fn register(p: Arc<dyn MetaProvider>) {
    let id = p.info().id;
    let mut list = PROVIDERS.write();
    list.retain(|x| x.info().id != id);
    list.push(p);
}

pub fn list() -> Vec<MetaInfo> {
    PROVIDERS.read().iter().map(|p| p.info()).collect()
}

pub fn get(id: &str) -> Option<Arc<dyn MetaProvider>> {
    PROVIDERS.read().iter().find(|p| p.info().id == id).cloned()
}

fn supports(info: &MetaInfo, media_kind: &str) -> bool {
    info.media_kinds.is_empty() || media_kind.is_empty() || info.media_kinds.iter().any(|k| k == media_kind)
}

/// 资料卡：某个资料源查到的一部作品
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Card {
    pub provider: String,
    pub provider_name: String,
    /// 作品页网址（资料源没给模板时为空）
    pub link: String,
    pub subject: Subject,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MetaOut {
    pub cards: Vec<Card>,
    /// 出错的资料源
    pub notes: Vec<String>,
}

/// 依次问所有支持该类型的资料源，每个最多取 `per` 条
pub async fn search(query: &str, media_kind: &str, per: usize) -> MetaOut {
    let q = query.trim();
    let mut out = MetaOut::default();
    if q.is_empty() {
        return out;
    }
    let ps: Vec<Arc<dyn MetaProvider>> = PROVIDERS.read().iter().filter(|p| supports(&p.info(), media_kind)).cloned().collect();
    for p in ps {
        let info = p.info();
        match p.search(q, media_kind).await {
            Ok(list) => out.cards.extend(list.into_iter().take(per.max(1)).map(|s| Card {
                link: if info.link_template.is_empty() {
                    String::new()
                } else {
                    info.link_template.replace("{id}", &s.id)
                },
                provider: info.id.clone(),
                provider_name: info.name.clone(),
                subject: s,
            })),
            Err(msg) => out.notes.push(format!("{}：{msg}", info.name)),
        }
    }
    out
}

// ---------- 前端命令 ----------

/// 已接入的资料源（软件默认没有）
#[tauri::command]
pub fn meta_providers() -> Vec<MetaInfo> {
    list()
}

/// 按名字查作品资料；没有资料源时返回空结果
#[tauri::command]
pub async fn meta_search(query: String, media_kind: String) -> Result<MetaOut, String> {
    Ok(search(&query, &media_kind, 3).await)
}

/// 作品的分集列表
#[tauri::command]
pub async fn meta_episodes(provider: String, subject_id: String) -> Result<Vec<EpisodeMeta>, String> {
    let p = get(&provider).ok_or_else(|| format!("这个版本的软件里没有资料源“{provider}”"))?;
    p.episodes(&subject_id).await
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// 测试用的假资料源（不联网）
    pub struct MockMeta {
        pub id: &'static str,
        pub kinds: Vec<String>,
        pub fail: bool,
    }

    impl MetaProvider for MockMeta {
        fn info(&self) -> MetaInfo {
            MetaInfo {
                id: self.id.into(),
                name: format!("测试资料 {}", self.id),
                media_kinds: self.kinds.clone(),
                link_template: "https://meta.example/subject/{id}".into(),
                ..Default::default()
            }
        }
        fn search<'a>(&'a self, query: &'a str, mk: &'a str) -> BoxFuture<'a, Result<Vec<Subject>, String>> {
            Box::pin(async move {
                if self.fail {
                    return Err("服务暂时不可用".into());
                }
                Ok((1..=5)
                    .map(|i| Subject {
                        id: format!("{i}"),
                        title: format!("{query} {i}"),
                        title_cn: format!("{query}（中文）{i}"),
                        media_kind: mk.into(),
                        total: Some(12),
                        ..Default::default()
                    })
                    .collect())
            })
        }
        fn episodes<'a>(&'a self, subject_id: &'a str) -> BoxFuture<'a, Result<Vec<EpisodeMeta>, String>> {
            Box::pin(async move {
                Ok((1..=3)
                    .map(|i| EpisodeMeta {
                        sort: i as f64,
                        title: format!("{subject_id}-{i}"),
                        ..Default::default()
                    })
                    .collect())
            })
        }
    }

    #[test]
    fn search_filters_by_kind_and_collects_errors() {
        register(Arc::new(MockMeta {
            id: "t-anime",
            kinds: vec!["anime".into()],
            fail: false,
        }));
        register(Arc::new(MockMeta {
            id: "t-anime",
            kinds: vec!["anime".into()],
            fail: false,
        })); // 同 id 替换
        register(Arc::new(MockMeta {
            id: "t-novel",
            kinds: vec!["novel".into()],
            fail: false,
        }));
        register(Arc::new(MockMeta {
            id: "t-broken",
            kinds: vec!["anime".into()],
            fail: true,
        }));
        assert_eq!(list().iter().filter(|m| m.id == "t-anime").count(), 1);

        let out = tauri::async_runtime::block_on(search("星屑", "anime", 2));
        let mine: Vec<&Card> = out.cards.iter().filter(|c| c.provider.starts_with("t-")).collect();
        assert_eq!(mine.len(), 2, "每个资料源最多取 per 条");
        assert!(mine.iter().all(|c| c.provider == "t-anime"));
        assert_eq!(mine[0].link, "https://meta.example/subject/1");
        assert_eq!(mine[0].subject.title_cn, "星屑（中文）1");
        assert!(out.notes.iter().any(|n| n.contains("t-broken") && n.contains("暂时不可用")));

        let empty = tauri::async_runtime::block_on(search("  ", "anime", 3));
        assert!(empty.cards.is_empty());

        let eps = tauri::async_runtime::block_on(meta_episodes("t-novel".into(), "9".into())).unwrap();
        assert_eq!(eps.len(), 3);
        assert_eq!(eps[2].title, "9-3");
        assert!(tauri::async_runtime::block_on(meta_episodes("nope".into(), "1".into())).is_err());
    }
}
