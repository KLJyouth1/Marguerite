//! 顶层版本化契约 [`GenUiDocument`]（Track C2）。
//!
//! 上游 dsh-genui 的缺口之一是「30+ 组件无版本化 schema」——消费方无从判断
//! 自己是否看得懂这份界面描述。此处以显式 `version` 字段封顶：解析时不校验版本
//! （保留原样以便观测与报错），**渲染前必须过
//! [`validate`](crate::genui::validate)** 才能确认版本可承载。

use serde::{Deserialize, Serialize};

use super::{GenUiError, Widget, GENUI_SCHEMA_VERSION};

/// 一份完整的生成式 UI 文档（版本 + widget 列表）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenUiDocument {
    /// 契约版本；`> GENUI_SCHEMA_VERSION` 时 [`validate`](crate::genui::validate) 拒绝
    pub version: u32,
    /// widget 列表（按序渲染）
    pub widgets: Vec<Widget>,
}

impl GenUiDocument {
    /// 以当前引擎契约版本（[`GENUI_SCHEMA_VERSION`]）构造文档。
    pub fn new(widgets: Vec<Widget>) -> Self {
        Self {
            version: GENUI_SCHEMA_VERSION,
            widgets,
        }
    }

    /// 从 JSON 解析文档。
    ///
    /// 仅做结构解析：未知 widget 类型 / 字段错配 → [`GenUiError::Json`]。
    /// 版本与注入校验属于 [`validate`](crate::genui::validate) 的职责，
    /// 二者分离以便调用方对「解析失败」与「校验拒绝」分别观测。
    pub fn from_json(s: &str) -> Result<GenUiDocument, GenUiError> {
        Ok(serde_json::from_str(s)?)
    }

    /// 序列化为 JSON 字符串。
    ///
    /// 本类型图仅含字符串键、数值与数组，序列化不可能失败（非有限浮点由
    /// `serde_json` 记为 `null`），故此处以 `expect` 断言而非上抛 `Result`。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("GenUiDocument 序列化不应失败")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genui::widget::{
        Block, ChartWidget, LayoutWidget, MermaidWidget, Series,
    };

    #[test]
    fn valid_json_parses_and_roundtrips() {
        let src = r#"{
            "version": 1,
            "widgets": [
                {"type":"layout","id":"root","blocks":[
                    {"kind":"heading","text":"月报"},
                    {"kind":"card","title":"结论","body":"稳中向好"}
                ]},
                {"type":"chart","id":"c1","chart_type":"bar",
                 "series":[{"name":"GMV","points":[[0.0,1.0],[1.0,2.0]]}]},
                {"type":"mermaid","id":"m1","source":"graph LR; A-->B;"}
            ]
        }"#;
        let doc = GenUiDocument::from_json(src).expect("合法文档应解析成功");
        assert_eq!(doc.version, 1);
        assert_eq!(doc.widgets.len(), 3);

        // to_json → 再次解析应与首次解析结果完全相等（serde round-trip）
        let again = GenUiDocument::from_json(&doc.to_json()).expect("往返应可再解析");
        assert_eq!(doc, again, "JSON 往返应一致");
    }

    #[test]
    fn new_uses_current_schema_version() {
        let doc = GenUiDocument::new(vec![]);
        assert_eq!(doc.version, GENUI_SCHEMA_VERSION);
    }

    #[test]
    fn typed_fields_survive_roundtrip_not_degraded_to_value() {
        let doc = GenUiDocument::new(vec![
            Widget::Layout(LayoutWidget {
                id: "root".into(),
                blocks: vec![Block::Text { text: "正文".into() }],
            }),
            Widget::Chart(ChartWidget {
                id: "c1".into(),
                chart_type: "line".into(),
                series: vec![Series {
                    name: "s".into(),
                    points: vec![(1.0, 2.0)],
                }],
            }),
            Widget::Mermaid(MermaidWidget {
                id: "m1".into(),
                source: "graph TD; X-->Y;".into(),
            }),
        ]);
        let back = GenUiDocument::from_json(&doc.to_json()).expect("往返");
        assert_eq!(doc, back);
        // 强类型断言：解析结果仍是具名结构，而非退化的 Value
        match &back.widgets[1] {
            Widget::Chart(c) => {
                assert_eq!(c.chart_type, "line");
                assert_eq!(c.series[0].points[0], (1.0, 2.0));
            }
            other => panic!("应为 Chart 变体，实为 {}", other.kind()),
        }
    }

    #[test]
    fn malformed_json_returns_err() {
        assert!(GenUiDocument::from_json("{ not json").is_err());
        // 字段类型错配（version 应为数字）也须报错，而非静默为 0
        assert!(GenUiDocument::from_json(r#"{"version":"1","widgets":[]}"#).is_err());
    }
}
