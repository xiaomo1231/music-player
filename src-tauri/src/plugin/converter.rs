//! 插件提供的格式转换器：列给“格式转换”页、检查参数。真正的转换在 `crate::convert` 的队列里跑
//! （协议 2 的 `convert` 方法，或清单里 `exec` 命令模板）。说明见 `docs/插件开发.md` 第 15 节
use super::manifest::{ConverterDecl, OutputFmt, Setting};
use super::{normalize_setting, R, REG};
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Serialize, Clone, Debug)]
pub struct ConverterView {
    pub plugin: String,
    pub plugin_name: String,
    pub id: String,
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<OutputFmt>,
    pub options: Vec<Setting>,
    pub note: String,
    /// 用 exec 命令模板（没有进度）
    pub exec: bool,
    /// 插件还有没填的必填设置
    pub needs_settings: Vec<String>,
}

pub fn view(plugin: &str, plugin_name: &str, c: &ConverterDecl, needs: Vec<String>) -> ConverterView {
    ConverterView {
        plugin: plugin.to_string(),
        plugin_name: plugin_name.to_string(),
        id: c.id.clone(),
        name: if c.name.trim().is_empty() { plugin_name.to_string() } else { c.name.clone() },
        inputs: c.inputs.clone(),
        outputs: c.output_list(),
        options: c.options.clone(),
        note: c.note.clone(),
        exec: !c.exec.is_empty(),
        needs_settings: needs,
    }
}

/// 所有启用、没有问题的插件提供的转换器
pub fn active() -> Vec<ConverterView> {
    let g = REG.read();
    let mut out = vec![];
    for (id, l) in g.iter().filter(|(_, l)| l.active()) {
        let settings = super::full_settings(id, &l.manifest, &l.settings);
        let needs = super::missing_settings(&l.manifest, &settings);
        for c in &l.manifest.contributes.converters {
            out.push(view(id, &l.manifest.name, c, needs.clone()));
        }
    }
    out.sort_by(|a, b| (&a.plugin_name, &a.name).cmp(&(&b.plugin_name, &b.name)));
    out
}

/// 找一个启用的转换器（插件停用、卸载、有问题时返回说明）
pub fn find(plugin: &str, conv: &str) -> R<(String, ConverterDecl)> {
    let g = REG.read();
    let l = g.get(plugin).ok_or_else(|| format!("转换插件“{plugin}”已卸载"))?;
    if !l.enabled {
        return Err(format!("插件“{}”已停用，启用后可以重试", l.manifest.name));
    }
    if !l.problem.is_empty() {
        return Err(format!("插件“{}”有问题：{}", l.manifest.name, l.problem));
    }
    let c = l
        .manifest
        .converter(conv)
        .ok_or_else(|| format!("插件“{}”里没有转换器 {conv}", l.manifest.name))?;
    Ok((l.manifest.name.clone(), c.clone()))
}

/// 显示名：“转换器名 → 输出格式名”；插件不在了返回 None
pub fn label(plugin: &str, conv: &str, ext: &str) -> Option<String> {
    let g = REG.read();
    let l = g.get(plugin)?;
    let c = l.manifest.converter(conv)?;
    let name = if c.name.trim().is_empty() { l.manifest.name.clone() } else { c.name.clone() };
    let out = c
        .output_list()
        .into_iter()
        .find(|o| o.ext == ext)
        .map(|o| o.name)
        .unwrap_or_else(|| ext.to_uppercase());
    Some(format!("{name} → {out}"))
}

/// 用户填的参数：按声明检查类型，没填的用默认值，多余的丢掉
pub fn normalize_options(c: &ConverterDecl, values: &Map<String, Value>) -> R<Map<String, Value>> {
    let mut out = Map::new();
    for o in &c.options {
        let v = match values.get(&o.key) {
            Some(v) if !v.is_null() => normalize_setting(o, v)?,
            _ => o.default.clone(),
        };
        if o.required && super::is_empty_value(Some(&v)) {
            return Err(format!("请填写“{}”", o.label));
        }
        out.insert(o.key.clone(), v);
    }
    Ok(out)
}

/// 设置 / 参数的值变成命令行里的文字
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::Manifest;
    use serde_json::json;

    fn decl() -> ConverterDecl {
        let m = Manifest::parse(
            r#"{"manifest":1,"id":"x-conv","name":"X","version":"1.0.0",
                "runtime":{"type":"process","command":{"default":["python","main.py"]}},
                "engines":{"protocol":2},
                "contributes":{"converters":[{"id":"main","inputs":["audio","xyz"],"outputs":["flac",{"ext":"mp3","name":"MP3 320k"},"same"],
                  "options":[{"key":"lufs","type":"int","label":"目标响度","default":-16},
                             {"key":"mode","type":"select","label":"模式","default":"a","options":["a","b"]}]}]}}"#,
        )
        .unwrap();
        m.contributes.converters[0].clone()
    }

    #[test]
    fn options_are_checked_and_defaulted() {
        let c = decl();
        let o = normalize_options(&c, &Map::new()).unwrap();
        assert_eq!(o["lufs"], json!(-16));
        assert_eq!(o["mode"], json!("a"));
        let mut v = Map::new();
        v.insert("lufs".into(), json!("-14"));
        v.insert("junk".into(), json!(1));
        let o = normalize_options(&c, &v).unwrap();
        assert_eq!(o["lufs"], json!(-14));
        assert!(!o.contains_key("junk"));
        v.insert("mode".into(), json!("zzz"));
        assert!(normalize_options(&c, &v).is_err());
        assert_eq!(value_text(&json!(true)), "true");
        assert_eq!(value_text(&json!(-16)), "-16");
        assert_eq!(value_text(&Value::Null), "");
    }

    #[test]
    fn accepts_and_outputs() {
        let c = decl();
        assert!(c.accepts("WAV"));
        assert!(c.accepts("xyz"));
        assert!(!c.accepts("mkv"));
        let outs = c.output_list();
        assert_eq!(outs[0].name, "FLAC");
        assert_eq!(outs[1].name, "MP3 320k");
        assert_eq!(outs[2].name, "同格式");
        assert_eq!(ConverterDecl::resolve_ext("same", "WAV"), "wav");
        assert_eq!(ConverterDecl::resolve_ext("mp3", "wav"), "mp3");
    }
}
