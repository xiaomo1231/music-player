//! 任意网页来源（AI 解析）：精简页面结构 → AI 写提取规则 → 按规则试搜验证 → 保存复用；失效时自动重新生成；
//! 兜底模式 ai_direct：每次都把精简后的页面交给 AI 直接返回结果。
use super::source::{self, collapse, fill_query, kind_of_url, match_tokens, parse_size, tokens, LOGIN_MSG};
use super::{e, with_db, Hit, SourceRow};
use crate::ai;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};

/// 学习规则时用的示例关键字
const SAMPLE_QUERY: &str = "test";
/// 生成规则失败（可以回退到 ai_direct）的前缀
pub const GEN_FAIL: &str = "生成提取规则失败";
/// 精简摘要的最大字数
const SUMMARY_LIMIT: usize = 12000;
const SKIP: &[&str] = &[
    "script", "style", "noscript", "svg", "iframe", "link", "meta", "head", "path", "template", "img", "br",
];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Field {
    /// 相对于结果容器的选择器；空字符串表示容器本身
    #[serde(default)]
    pub sel: String,
    /// text 表示取文字，其他为属性名
    #[serde(default)]
    pub attr: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Rules {
    #[serde(default)]
    pub item: String,
    #[serde(default)]
    pub title: Field,
    #[serde(default)]
    pub link: Field,
    #[serde(default)]
    pub size: Option<Field>,
    #[serde(default)]
    pub date: Option<Field>,
    #[serde(default)]
    pub next: Option<String>,
}

/// 存在 fetch_sources.rule_json 里的内容
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct StoredRule {
    /// rules 或 ai_direct
    pub mode: String,
    pub template: String,
    pub rules: Option<Rules>,
}

impl StoredRule {
    pub fn parse(json: &str) -> StoredRule {
        let mut s: StoredRule = if json.trim().is_empty() {
            StoredRule::default()
        } else {
            serde_json::from_str(json).unwrap_or_default()
        };
        if s.mode.is_empty() {
            s.mode = "rules".into();
        }
        s
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub title: String,
    pub link: String,
    pub size: Option<i64>,
    pub date: Option<String>,
}

fn step(app: &AppHandle, url: &str, name: &str, status: &str, msg: &str) {
    let _ = app.emit("fetch://source", json!({ "step": name, "status": status, "message": msg, "url": url }));
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

fn push_line(out: &mut String, depth: usize, text: &str) {
    out.push_str(&"  ".repeat(depth));
    out.push_str(text);
    out.push('\n');
}

fn open_tag(e: ElementRef) -> String {
    let v = e.value();
    let mut s = format!("<{}", v.name());
    for attr in ["id", "class", "href", "title", "type", "name"] {
        if let Some(val) = v.attr(attr) {
            let val = if attr == "href" { cut(val.trim(), 120) } else { cut(&collapse(val), 80) };
            s.push_str(&format!(" {attr}=\"{val}\""));
        }
    }
    s.push('>');
    s
}

fn walk(el: ElementRef, depth: usize, out: &mut String) {
    if depth > 14 || out.len() > SUMMARY_LIMIT * 4 {
        return;
    }
    let mut last: Option<String> = None;
    let mut repeat = 0usize;
    for child in el.children() {
        if let Some(t) = child.value().as_text() {
            let s = collapse(t);
            if !s.is_empty() {
                push_line(out, depth, &format!("\"{}\"", cut(&s, 60)));
            }
            continue;
        }
        let Some(ce) = ElementRef::wrap(child) else { continue };
        let name = ce.value().name();
        if SKIP.contains(&name) {
            continue;
        }
        // 同一结构连续出现很多次时只保留前两个，其余用省略号代替
        let sig = format!("{name}.{}", ce.value().attr("class").unwrap_or(""));
        if last.as_deref() == Some(sig.as_str()) {
            repeat += 1;
            if repeat == 3 {
                push_line(out, depth, "…（同样结构还有更多，省略）");
            }
            if repeat >= 3 {
                continue;
            }
        } else {
            last = Some(sig);
            repeat = 1;
        }
        push_line(out, depth, &open_tag(ce));
        walk(ce, depth + 1, out);
    }
}

/// 去掉脚本和样式，保留标签层级、class、id、链接等，重复结构只留前两个
pub fn simplify(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut out = String::new();
    let body = Selector::parse("body").unwrap();
    match doc.select(&body).next() {
        Some(b) => walk(b, 0, &mut out),
        None => walk(doc.root_element(), 0, &mut out),
    }
    if out.chars().count() > SUMMARY_LIMIT {
        out = out.chars().take(SUMMARY_LIMIT).collect::<String>() + "\n…（已截断）";
    }
    out
}

/// 页面是登录页（有密码输入框）
pub fn looks_like_login(html: &str) -> bool {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("input[type=password]").unwrap();
    doc.select(&sel).next().is_some()
}

/// 页面明确写着没有结果（这时不应该当作“规则失效”）
pub fn looks_empty(html: &str) -> bool {
    let l = html.to_lowercase();
    [
        "没有找到",
        "没有相关",
        "没有结果",
        "无结果",
        "未找到",
        "没有匹配",
        "no results",
        "nothing found",
        "0 results",
        "no matches",
    ]
    .iter()
    .any(|k| l.contains(k))
}

fn resolve(base: &str, link: &str) -> String {
    url::Url::parse(base)
        .and_then(|b| b.join(link))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| link.to_string())
}

/// 取一个字段：先在容器里找选择器，再取文字或属性
fn pick(el: ElementRef, f: &Field) -> Result<Option<String>, String> {
    let target = if f.sel.trim().is_empty() {
        Some(el)
    } else {
        let s = Selector::parse(f.sel.trim()).map_err(|_| format!("选择器无效：{}", f.sel))?;
        el.select(&s).next()
    };
    let Some(t) = target else { return Ok(None) };
    let attr = f.attr.trim();
    let v = if attr.is_empty() || attr == "text" {
        collapse(&t.text().collect::<String>())
    } else {
        t.value().attr(attr).unwrap_or("").trim().to_string()
    };
    Ok(Some(v))
}

/// 按规则从页面里提取结果
pub fn apply_rules(html: &str, base: &str, rules: &Rules) -> Result<Vec<Raw>, String> {
    let doc = Html::parse_document(html);
    let item = Selector::parse(rules.item.trim()).map_err(|_| format!("结果容器选择器无效：{}", rules.item))?;
    let mut out = vec![];
    for el in doc.select(&item) {
        let title = pick(el, &rules.title)?.unwrap_or_default();
        let link = pick(el, &rules.link)?.unwrap_or_default();
        if title.is_empty() && link.is_empty() {
            continue;
        }
        let size = match &rules.size {
            Some(f) => pick(el, f)?.and_then(|s| parse_size(&s)),
            None => None,
        };
        let date = match &rules.date {
            Some(f) => pick(el, f)?.filter(|s| !s.is_empty()),
            None => None,
        };
        let link = if link.is_empty() { String::new() } else { resolve(base, &link) };
        out.push(Raw { title, link, size, date });
    }
    Ok(out)
}

/// 规则提取的结果是否合理：有结果，且 80% 以上有标题和链接
pub fn check_raws(raws: &[Raw]) -> Result<(), String> {
    if raws.is_empty() {
        return Err("按规则没有提取到任何结果（容器选择器可能不对）".into());
    }
    let n = raws.len();
    let with_title = raws.iter().filter(|r| !r.title.is_empty()).count();
    let with_link = raws.iter().filter(|r| !r.link.is_empty()).count();
    if with_title * 10 < n * 8 {
        return Err(format!("{n} 条结果里只有 {with_title} 条有标题，标题字段对不上"));
    }
    if with_link * 10 < n * 8 {
        return Err(format!("{n} 条结果里只有 {with_link} 条有链接，链接字段对不上"));
    }
    Ok(())
}

const RULE_SYSTEM: &str = "你是网页结构分析助手。根据给出的搜索结果页结构摘要，写出一份提取搜索结果的规则，只输出一个 JSON 对象，字段如下：\n\
{\"item\": 每条搜索结果的容器元素的 CSS 选择器（要能匹配到全部结果，但不要太宽泛）,\n\
 \"title\": {\"sel\": 相对于容器的选择器，空字符串表示容器本身, \"attr\": \"text\" 表示取文字，否则写属性名如 title、href、alt},\n\
 \"link\": {\"sel\": 相对于容器的选择器, \"attr\": \"href\"},\n\
 \"size\": {\"sel\": ..., \"attr\": ...} 或 null（页面没有大小信息时）,\n\
 \"date\": {\"sel\": ..., \"attr\": ...} 或 null,\n\
 \"next\": 下一页链接的选择器或 null}\n\
选择器只能用标签名、class、id、属性选择器和后代、子代组合，不要用 :contains、:has 等扩展语法。不要输出解释文字。";

/// 让 AI 写规则并用这一页试一次，不合理就把错误反馈回去重试（最多 3 次）
pub async fn learn(app: &AppHandle, template: &str, sample: &str) -> Result<(Rules, Vec<Raw>), String> {
    let url = fill_query(template, sample);
    step(app, template, "打开页面", "run", "正在打开网站…");
    let html = match source::http_text(&url).await {
        Ok(h) => h,
        Err(m) => {
            step(app, template, "打开页面", "error", &m);
            return Err(m);
        }
    };
    if looks_like_login(&html) {
        step(app, template, "打开页面", "error", LOGIN_MSG);
        return Err(LOGIN_MSG.into());
    }
    step(app, template, "打开页面", "ok", "已打开");
    step(app, template, "分析页面结构", "run", "正在精简页面结构…");
    let summary = simplify(&html);
    step(app, template, "分析页面结构", "ok", &format!("已精简为 {} 字", summary.chars().count()));

    let mut feedback = String::new();
    let mut last = String::new();
    for attempt in 1..=3 {
        step(app, template, "生成提取规则", "run", &format!("第 {attempt} 次生成规则…"));
        let user = format!("搜索网址模板：{template}\n示例关键字：{sample}\n页面结构摘要：\n{summary}\n{feedback}");
        let reply = ai::chat_simple(app, RULE_SYSTEM, &user, true).await.map_err(e)?;
        let v = match ai::extract_json(&reply) {
            Ok(v) => v,
            Err(_) => {
                feedback = "上一次的回复不是合法 JSON，请只输出 JSON 对象。".into();
                last = "AI 回复不是合法 JSON".into();
                step(app, template, "生成提取规则", "warn", &last);
                continue;
            }
        };
        let rules: Rules = match serde_json::from_value(v) {
            Ok(r) => r,
            Err(err) => {
                feedback = format!("JSON 字段不对：{err}。请严格按规定的字段输出。");
                last = format!("规则字段不对：{err}");
                step(app, template, "生成提取规则", "warn", &last);
                continue;
            }
        };
        match apply_rules(&html, &url, &rules).and_then(|raws| check_raws(&raws).map(|_| raws)) {
            Ok(raws) => {
                step(app, template, "生成提取规则", "ok", "规则已生成");
                step(app, template, "试搜并验证", "ok", &format!("按规则提取到 {} 条结果", raws.len()));
                return Ok((rules, raws));
            }
            Err(msg) => {
                feedback = format!("上一版规则的结果不合理：{msg}。请修正选择器后重新输出完整 JSON。");
                last = msg;
                step(app, template, "生成提取规则", "warn", &format!("需要修正：{last}"));
            }
        }
    }
    Err(format!("{GEN_FAIL}（已重试 3 次）：{last}"))
}

/// 添加或重建规则：成功返回要保存的 rule_json；生成不了稳定规则则切到 ai_direct 模式
pub async fn add_rule(app: &AppHandle, template: &str) -> Result<String, String> {
    match learn(app, template, SAMPLE_QUERY).await {
        Ok((rules, _)) => Ok(StoredRule {
            mode: "rules".into(),
            template: template.into(),
            rules: Some(rules),
        }
        .to_json()),
        Err(msg) if msg.starts_with(GEN_FAIL) => {
            step(
                app,
                template,
                "生成提取规则",
                "warn",
                "未能生成稳定的规则，已切换为“AI 直接读取”模式（慢一些，费一点额度）",
            );
            Ok(StoredRule {
                mode: "ai_direct".into(),
                template: template.into(),
                rules: None,
            }
            .to_json())
        }
        Err(msg) => Err(msg),
    }
}

const DIRECT_SYSTEM: &str = "你是搜索结果提取助手。根据给出的网页结构摘要，找出页面上的搜索结果（每一条是一个作品或一个文件的条目），只输出 JSON：{\"results\":[{\"title\":\"标题\",\"url\":\"该条目的原始链接\",\"size\":\"大小文字，没有写空字符串\",\"date\":\"日期，没有写空字符串\"}]}，最多 50 条。不要输出解释文字。";

async fn ai_direct(app: &AppHandle, url: &str, query: &str, html: &str) -> Result<Vec<Raw>, String> {
    if looks_like_login(html) {
        return Err(LOGIN_MSG.into());
    }
    let summary = simplify(html);
    let user = format!("页面网址：{url}\n搜索关键字：{query}\n页面结构摘要（缩进表示层级）：\n{summary}");
    let reply = ai::chat_simple(app, DIRECT_SYSTEM, &user, true).await.map_err(e)?;
    let v = ai::extract_json(&reply).map_err(e)?;
    let arr = v["results"].as_array().cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|x| {
            let title = collapse(x["title"].as_str()?);
            let link = resolve(url, x["url"].as_str()?);
            let size = x["size"].as_str().and_then(parse_size);
            let date = x["date"].as_str().map(collapse).filter(|s| !s.is_empty());
            (!link.is_empty()).then_some(Raw { title, link, size, date })
        })
        .take(50)
        .collect())
}

fn save_rule(app: &AppHandle, id: i64, stored: &StoredRule) -> Result<(), String> {
    with_db(app, |c| {
        c.execute("UPDATE fetch_sources SET rule_json=?2 WHERE id=?1", rusqlite::params![id, stored.to_json()])?;
        Ok(())
    })
    .map_err(e)
}

/// 搜索网页来源：优先用保存的规则；0 结果或字段对不上时自动重新生成（rule_updated=true）
pub async fn search(app: &AppHandle, row: &SourceRow, query: &str, _media_kind: &str) -> Result<Vec<Hit>, String> {
    let stored = StoredRule::parse(&row.rule_json);
    let template = if stored.template.is_empty() {
        row.url.clone()
    } else {
        stored.template.clone()
    };
    let has_q = template.contains("{q}");
    let url = fill_query(&template, query);
    let html = source::http_text(&url).await?;
    let mut updated = false;
    let raws: Vec<Raw> = if stored.mode == "ai_direct" {
        ai_direct(app, &url, query, &html).await?
    } else {
        let attempt = match &stored.rules {
            Some(r) => apply_rules(&html, &url, r).and_then(|v| check_raws(&v).map(|_| v)),
            None => Err("还没有提取规则".to_string()),
        };
        match attempt {
            Ok(v) => v,
            Err(msg) => {
                if looks_like_login(&html) {
                    return Err(LOGIN_MSG.into());
                }
                if looks_empty(&html) {
                    return Ok(vec![]);
                }
                let (rules, v) = learn(app, &template, query)
                    .await
                    .map_err(|e2| format!("网站规则失效（{msg}），自动重新生成也失败：{e2}"))?;
                save_rule(
                    app,
                    row.id,
                    &StoredRule {
                        mode: "rules".into(),
                        template: template.clone(),
                        rules: Some(rules),
                    },
                )?;
                updated = true;
                v
            }
        }
    };
    let toks = tokens(query);
    let mut hits = vec![];
    for r in raws {
        if r.link.is_empty() {
            continue;
        }
        if !has_q && !match_tokens(&r.title, &toks) {
            continue;
        }
        hits.push(Hit {
            kind: kind_of_url(&r.link).to_string(),
            title: r.title,
            url: r.link,
            size: r.size,
            date: r.date,
            rule_updated: updated,
            ..Default::default()
        });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html><html><head><style>.x{}</style><script>var a="<a class='item'>";</script></head>
    <body><div class="results">
      <div class="item"><a class="t" href="/detail/1">星屑邮局 第 1 季 01</a><span class="sz">1.2 GB</span><span class="d">2024-01-02</span></div>
      <div class="item"><a class="t" href="/detail/2">星屑邮局 第 1 季 02</a><span class="sz">900 MB</span><span class="d">2024-01-09</span></div>
      <div class="item"><a class="t" href="/detail/3">星屑邮局 第 1 季 03</a><span class="sz">950 MB</span></div>
      <div class="item"><a class="t" href="/detail/4">星屑邮局 第 1 季 04</a></div>
    </div></body></html>"#;

    fn rules() -> Rules {
        Rules {
            item: "div.item".into(),
            title: Field {
                sel: "a.t".into(),
                attr: "text".into(),
            },
            link: Field {
                sel: "a.t".into(),
                attr: "href".into(),
            },
            size: Some(Field {
                sel: ".sz".into(),
                attr: "text".into(),
            }),
            date: Some(Field {
                sel: ".d".into(),
                attr: "text".into(),
            }),
            next: None,
        }
    }

    #[test]
    fn simplify_drops_scripts_and_collapses_repeats() {
        let s = simplify(PAGE);
        assert!(!s.contains("var a"));
        assert!(!s.contains(".x{}"));
        assert!(s.contains("class=\"results\""));
        assert!(s.contains("href=\"/detail/1\""));
        assert!(s.contains("省略"));
        assert!(!s.contains("detail/4"));
    }

    #[test]
    fn rules_extract_and_resolve_links() {
        let raws = apply_rules(PAGE, "https://site.example/search?q=x", &rules()).unwrap();
        assert_eq!(raws.len(), 4);
        assert_eq!(raws[0].title, "星屑邮局 第 1 季 01");
        assert_eq!(raws[0].link, "https://site.example/detail/1");
        assert_eq!(raws[0].size, Some((1.2 * 1024.0 * 1024.0 * 1024.0) as i64));
        assert_eq!(raws[0].date.as_deref(), Some("2024-01-02"));
        assert_eq!(raws[3].size, None);
        assert!(check_raws(&raws).is_ok());
    }

    #[test]
    fn bad_selector_is_reported() {
        let mut r = rules();
        r.item = "div.item:contains(x)".into();
        assert!(apply_rules(PAGE, "https://a.b/", &r).is_err());
    }

    #[test]
    fn check_rejects_empty_and_mismatched_fields() {
        assert!(check_raws(&[]).is_err());
        let bad = vec![
            Raw {
                title: String::new(),
                link: "https://a/1".into(),
                size: None,
                date: None
            };
            5
        ];
        assert!(check_raws(&bad).is_err());
    }

    #[test]
    fn login_and_empty_pages() {
        assert!(looks_like_login(r#"<form><input type="password" name="p"></form>"#));
        assert!(!looks_like_login(PAGE));
        assert!(looks_empty("<p>没有找到相关结果</p>"));
        assert!(!looks_empty(PAGE));
    }

    #[test]
    fn stored_rule_round_trip() {
        let s = StoredRule {
            mode: "rules".into(),
            template: "https://a/{q}".into(),
            rules: Some(rules()),
        };
        let back = StoredRule::parse(&s.to_json());
        assert_eq!(back.template, "https://a/{q}");
        assert_eq!(back.rules.unwrap().item, "div.item");
        assert_eq!(StoredRule::parse("").mode, "rules");
    }
}
