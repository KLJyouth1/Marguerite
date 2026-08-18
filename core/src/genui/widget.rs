//! `Widget` 枚举与各 widget 具名结构（Track C1）。
//!
//! 设计纪律：每个变体持有**具名结构体**（`id` + 类型化字段），不退化成裸
//! `serde_json::Value`。好处是字段缺失/类型错配在反序列化阶段即报错，
//! 而不是等到渲染层才炸；同时 [`Widget::text_fields`] 的穷尽 match 使得
//! 「新增变体忘记纳入沙箱校验」在编译期就无法通过。
//!
//! JSON 表示为内部标签式（internally tagged）：
//! `{"type":"layout","id":"root","blocks":[…]}`，标签字段为 `type`。

use serde::{Deserialize, Serialize};

/// 版面块（Layout widget 的最小内容单元）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    /// 标题块
    Heading {
        /// 标题文本
        text: String,
    },
    /// 正文块
    Text {
        /// 正文文本
        text: String,
    },
    /// 卡片块（标题 + 正文）
    Card {
        /// 卡片标题
        title: String,
        /// 卡片正文
        body: String,
    },
}

/// 版面 widget：由若干 [`Block`] 纵向组成。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutWidget {
    /// widget 唯一标识（action 事件按此 id 回指）
    pub id: String,
    /// 版面块序列
    pub blocks: Vec<Block>,
}

/// 一条数据系列（图表用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// 系列名（图例显示）
    pub name: String,
    /// 数据点 `(x, y)` 序列
    pub points: Vec<(f64, f64)>,
}

/// 图表 widget。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartWidget {
    /// widget 唯一标识
    pub id: String,
    /// 图表类型（如 "line" / "bar" / "pie"）
    pub chart_type: String,
    /// 数据系列
    pub series: Vec<Series>,
}

/// 表单字段描述。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    /// 展示标签
    pub label: String,
    /// 字段类型（如 "text" / "number" / "select"）
    pub field_type: String,
    /// 提交时的字段名
    pub name: String,
}

/// 表单 widget。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormWidget {
    /// widget 唯一标识
    pub id: String,
    /// 字段列表
    pub fields: Vec<FormField>,
}

/// 测验 widget（单题多选项）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuizWidget {
    /// widget 唯一标识
    pub id: String,
    /// 题干
    pub question: String,
    /// 选项文本列表
    pub options: Vec<String>,
}

/// Mermaid 图 widget（持 mermaid 源码文本）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MermaidWidget {
    /// widget 唯一标识
    pub id: String,
    /// mermaid 文本源码
    pub source: String,
}

/// 3D 场景 widget（持场景描述文本 / JSON 字符串）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene3DWidget {
    /// widget 唯一标识
    pub id: String,
    /// 场景描述（文本或 JSON 字符串）
    pub scene: String,
}

/// 生成式 UI 组件全集。
///
/// 未知 `type` 在反序列化时返回 [`serde_json::Error`]（经
/// [`GenUiError::Json`](crate::genui::GenUiError::Json) 上抛），**不 panic**。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    /// 版面
    Layout(LayoutWidget),
    /// 图表
    Chart(ChartWidget),
    /// 表单
    Form(FormWidget),
    /// 测验
    Quiz(QuizWidget),
    /// Mermaid 图
    Mermaid(MermaidWidget),
    /// 3D 场景
    #[serde(rename = "scene3d")]
    Scene3D(Scene3DWidget),
}

impl Widget {
    /// widget 唯一标识（用于 action 事件回指与校验报错定位）。
    pub fn id(&self) -> &str {
        match self {
            Widget::Layout(w) => &w.id,
            Widget::Chart(w) => &w.id,
            Widget::Form(w) => &w.id,
            Widget::Quiz(w) => &w.id,
            Widget::Mermaid(w) => &w.id,
            Widget::Scene3D(w) => &w.id,
        }
    }

    /// widget 类型名（与 JSON `type` 标签一致，可直接进决策日志）。
    pub fn kind(&self) -> &'static str {
        match self {
            Widget::Layout(_) => "layout",
            Widget::Chart(_) => "chart",
            Widget::Form(_) => "form",
            Widget::Quiz(_) => "quiz",
            Widget::Mermaid(_) => "mermaid",
            Widget::Scene3D(_) => "scene3d",
        }
    }

    /// 全部需过沙箱校验的文本字段，返回 `(字段名, 文本)` 序列。
    ///
    /// 覆盖一切会进入渲染层的字符串（含 `id` 与各类型名——它们同样可能被写入
    /// DOM 属性而成为注入面）。此处的 match 是穷尽的：新增 `Widget` 变体若不在
    /// 此列出字段，编译不通过，因此校验面不会被静默漏掉。
    pub fn text_fields(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = vec![("id", self.id())];
        match self {
            Widget::Layout(w) => {
                for b in &w.blocks {
                    match b {
                        Block::Heading { text } => out.push(("block.heading.text", text)),
                        Block::Text { text } => out.push(("block.text.text", text)),
                        Block::Card { title, body } => {
                            out.push(("block.card.title", title));
                            out.push(("block.card.body", body));
                        }
                    }
                }
            }
            Widget::Chart(w) => {
                out.push(("chart_type", &w.chart_type));
                for s in &w.series {
                    out.push(("series.name", &s.name));
                }
            }
            Widget::Form(w) => {
                for f in &w.fields {
                    out.push(("field.label", &f.label));
                    out.push(("field.field_type", &f.field_type));
                    out.push(("field.name", &f.name));
                }
            }
            Widget::Quiz(w) => {
                out.push(("question", &w.question));
                for o in &w.options {
                    out.push(("option", o));
                }
            }
            Widget::Mermaid(w) => out.push(("source", &w.source)),
            Widget::Scene3D(w) => out.push(("scene", &w.scene)),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 断言单个 widget 的 JSON 往返一致，且 `type` 标签符合契约。
    fn roundtrip(w: &Widget, expect_tag: &str) {
        let s = serde_json::to_string(w).expect("序列化");
        let v: serde_json::Value = serde_json::from_str(&s).expect("解析为 Value");
        assert_eq!(
            v.get("type").and_then(|t| t.as_str()),
            Some(expect_tag),
            "type 标签应为 {expect_tag}"
        );
        let back: Widget = serde_json::from_str(&s).expect("反序列化");
        assert_eq!(w, &back, "{expect_tag} JSON 往返应一致");
    }

    #[test]
    fn layout_widget_serde() {
        let w = Widget::Layout(LayoutWidget {
            id: "root".into(),
            blocks: vec![
                Block::Heading {
                    text: "季度回顾".into(),
                },
                Block::Text {
                    text: "营收同比增长".into(),
                },
                Block::Card {
                    title: "要点".into(),
                    body: "毛利率改善".into(),
                },
            ],
        });
        roundtrip(&w, "layout");
        assert_eq!(w.id(), "root");
        assert_eq!(w.kind(), "layout");
    }

    #[test]
    fn chart_widget_serde() {
        let w = Widget::Chart(ChartWidget {
            id: "c1".into(),
            chart_type: "line".into(),
            series: vec![Series {
                name: "营收".into(),
                points: vec![(0.0, 1.5), (1.0, 2.5)],
            }],
        });
        roundtrip(&w, "chart");
        assert_eq!(w.kind(), "chart");
    }

    #[test]
    fn form_widget_serde() {
        let w = Widget::Form(FormWidget {
            id: "f1".into(),
            fields: vec![FormField {
                label: "邮箱".into(),
                field_type: "text".into(),
                name: "email".into(),
            }],
        });
        roundtrip(&w, "form");
        assert_eq!(w.kind(), "form");
    }

    #[test]
    fn quiz_widget_serde() {
        let w = Widget::Quiz(QuizWidget {
            id: "q1".into(),
            question: "Rust 的所有权转移称为？".into(),
            options: vec!["move".into(), "copy".into()],
        });
        roundtrip(&w, "quiz");
        assert_eq!(w.kind(), "quiz");
    }

    #[test]
    fn mermaid_widget_serde() {
        let w = Widget::Mermaid(MermaidWidget {
            id: "m1".into(),
            source: "graph TD; A-->B;".into(),
        });
        roundtrip(&w, "mermaid");
        assert_eq!(w.kind(), "mermaid");
    }

    #[test]
    fn scene3d_widget_serde() {
        let w = Widget::Scene3D(Scene3DWidget {
            id: "s1".into(),
            scene: "{\"objects\":[\"cube\"]}".into(),
        });
        roundtrip(&w, "scene3d");
        assert_eq!(w.kind(), "scene3d");
    }

    #[test]
    fn unknown_widget_type_errors_not_panics() {
        let r: Result<Widget, _> = serde_json::from_str(r#"{"type":"hologram","id":"x"}"#);
        assert!(r.is_err(), "枚举外 widget 类型应返回错误而非 panic");
    }

    #[test]
    fn text_fields_covers_every_string_field() {
        let w = Widget::Layout(LayoutWidget {
            id: "root".into(),
            blocks: vec![Block::Card {
                title: "T".into(),
                body: "B".into(),
            }],
        });
        let fields = w.text_fields();
        assert!(fields.contains(&("id", "root")));
        assert!(fields.contains(&("block.card.title", "T")));
        assert!(fields.contains(&("block.card.body", "B")));
    }
}
