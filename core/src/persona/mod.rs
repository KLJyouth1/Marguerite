//! DSH persona 组合渲染移植（上游 persona 包语义）
//!
//! - 组合式部署段落：有序 `PersonaSection` 拼接；带 `complete` 标记的段**阻断**
//!   其后所有段的追加（对齐上游「complete 阻断后续 prompt 追加」）；
//! - 模板解析：`{{model}}` / `{{cwd}}`（及任意变量名）在渲染时替换，
//!   未知变量替换为空串。
//!
//! 纯逻辑、离线可测；与红线 #1（仅追加、不重写历史）兼容——本层只渲染
//! 静态前缀文本，不触碰会话历史。

use std::collections::BTreeMap;

/// 人设组合的一个段落。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaSection {
    /// 段标识（诊断用）
    pub id: String,
    /// 段落文本
    pub content: String,
    /// complete 标记：true = 本段之后的所有段被阻断（不追加）
    pub complete: bool,
}

impl PersonaSection {
    pub fn new(id: &str, content: &str) -> Self {
        Self {
            id: id.to_string(),
            content: content.to_string(),
            complete: false,
        }
    }
}

/// 段内 complete 标记行（出现在任一段的独立行时：剥除标记、该段 complete=true，
/// 其后各段全部阻断——对齐上游 persona `complete` 语义）。
pub const COMPLETE_MARKER: &str = "<!--persona-complete-->";

/// 把原始段文本拆成 (content, complete)：含独立 `COMPLETE_MARKER` 行时剥除并置 complete。
pub fn strip_complete_marker(content: &str) -> (String, bool) {
    let lines: Vec<&str> = content.lines().collect();
    let complete = lines.iter().any(|l| l.trim() == COMPLETE_MARKER);
    if !complete {
        return (content.to_string(), false);
    }
    let stripped: Vec<&str> = lines
        .iter()
        .filter(|l| l.trim() != COMPLETE_MARKER)
        .copied()
        .collect();
    (stripped.join("\n"), true)
}

/// 渲染有序段：依序拼接；遇 `complete` 段后阻断后续段；
/// 每段文本先过 [`resolve_template`] 模板解析。
pub fn render_sections(sections: &[PersonaSection], vars: &BTreeMap<String, String>) -> String {
    let mut out: Vec<String> = Vec::new();
    for section in sections {
        let text = resolve_template(&section.content, vars);
        if !text.trim().is_empty() {
            out.push(text);
        }
        if section.complete {
            break;
        }
    }
    out.join("\n")
}

/// `{{name}}` 模板解析：命中变量表替换其值，未知变量替换为空串。
pub fn resolve_template(text: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                out.push_str(vars.get(name).map(String::as_str).unwrap_or(""));
                rest = &after[end + 2..];
            }
            None => {
                // 未闭合：原样保留，结束解析
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(model: &str, cwd: &str) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("model".to_string(), model.to_string());
        m.insert("cwd".to_string(), cwd.to_string());
        m
    }

    #[test]
    fn template_resolves_model_and_cwd() {
        let text = "模型：{{model}}；工作目录：{{cwd}}。";
        let out = resolve_template(text, &vars("deepseek-v4-pro", "D:/ws"));
        assert_eq!(out, "模型：deepseek-v4-pro；工作目录：D:/ws。");
    }

    #[test]
    fn template_unknown_variable_becomes_empty() {
        let out = resolve_template("值={{unknown}}。", &vars("m", "c"));
        assert_eq!(out, "值=。");
    }

    #[test]
    fn template_unclosed_brace_preserved_literally() {
        let out = resolve_template("前{{unclosed 后", &vars("m", "c"));
        assert_eq!(out, "前{{unclosed 后");
    }

    #[test]
    fn complete_section_blocks_later_sections() {
        let sections = vec![
            PersonaSection::new("a", "段A"),
            PersonaSection {
                id: "b".into(),
                content: "段B".into(),
                complete: true,
            },
            PersonaSection::new("c", "段C（应被阻断）"),
        ];
        let out = render_sections(&sections, &BTreeMap::new());
        assert_eq!(out, "段A\n段B");
        assert!(!out.contains("段C"));
    }

    #[test]
    fn strip_complete_marker_removes_line_and_flags() {
        let (text, complete) = strip_complete_marker("人设内容\n<!--persona-complete-->\n尾部");
        assert!(complete);
        assert_eq!(text, "人设内容\n尾部");
        let (text2, complete2) = strip_complete_marker("普通人设");
        assert!(!complete2);
        assert_eq!(text2, "普通人设");
    }

    #[test]
    fn render_empty_sections_gives_empty() {
        let sections = vec![PersonaSection::new("a", "   ")];
        assert_eq!(render_sections(&sections, &BTreeMap::new()), "");
    }

    #[test]
    fn render_resolves_templates_per_section() {
        let sections = vec![PersonaSection::new("core", "我是 {{model}} 助手")];
        let out = render_sections(&sections, &vars("deepseek-v4-pro", "/"));
        assert_eq!(out, "我是 deepseek-v4-pro 助手");
    }
}
