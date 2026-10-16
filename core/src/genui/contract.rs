//! GenUI 契约 v1：DSH GenUI 二创的 Rust 原生 Widget 枚举（spec FR-B2 / T-2.2）。
//!
//! 本文件为 DSH GenUI 二创定义的一组**轻量、面向卡片交互面板**的契约类型：
//! - [`Widget`]：枚举调度（Card / Button / ProgressBar / Form）
//! - [`Card`]：标题 + 正文行 + 操作按钮组
//! - [`Button`]：可点击按钮（含 enabled 开关）
//! - [`ProgressBar`]：进度条（0..=1 夹紧）
//! - [`Form`] / [`FormField`]：表单（文本 / 下拉）
//!
//! 所有类型都实现 `Serialize + Deserialize + PartialEq + Clone + Debug`，
//! 可直接作为 RPC / 事件总线载荷，满足 Steering 事件 roundtrip 要求。

use serde::{Deserialize, Serialize};

/// GenUI 顶层组件枚举。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    /// 卡片组件（标题 + 正文 + 底部按钮栏）。
    Card(Card),
    /// 独立按钮。
    Button(Button),
    /// 进度条。
    ProgressBar(ProgressBar),
    /// 表单。
    Form(Form),
}

impl Widget {
    /// 方便 match 调试时返回类型标签。
    pub fn kind(&self) -> &'static str {
        match self {
            Widget::Card(_) => "card",
            Widget::Button(_) => "button",
            Widget::ProgressBar(_) => "progress_bar",
            Widget::Form(_) => "form",
        }
    }
}

/// 卡片：标题 + 若干正文行 + 底部操作按钮。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Card {
    /// 卡片标题。
    pub title: String,
    /// 正文行（每一行字符串，渲染时左对齐截断）。
    pub body: Vec<String>,
    /// 底部动作按钮（横向排列）。
    pub actions: Vec<Button>,
}

/// 按钮组件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Button {
    /// 事件 id（click 派发时回传）。
    pub id: String,
    /// 显示文本。
    pub label: String,
    /// 是否启用；false 时渲染为灰色。
    #[serde(default)]
    pub enabled: bool,
}

impl Button {
    /// 便捷构造：默认 enabled=true。
    pub fn new<S: Into<String>>(id: S, label: S) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
        }
    }

    /// 禁用按钮（链式）。
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// 进度条组件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgressBar {
    /// 组件 id。
    pub id: String,
    /// 进度百分比，范围 0..=1；渲染端必须夹紧到 0..=1。
    pub percent: f32,
}

impl ProgressBar {
    /// 夹紧后的百分比（0..=1）。
    pub fn clamped_percent(&self) -> f32 {
        self.percent.clamp(0.0, 1.0)
    }
}

/// 表单组件：由若干字段纵向组成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Form {
    /// 表单 id。
    pub id: String,
    /// 字段列表。
    pub fields: Vec<FormField>,
}

/// 表单字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FormField {
    /// 文本输入框。
    Text {
        /// 字段 id。
        id: String,
        /// 左侧标签。
        label: String,
        /// 占位提示（默认空串）。
        #[serde(default)]
        placeholder: String,
    },
    /// 下拉选择框。
    Select {
        /// 字段 id。
        id: String,
        /// 左侧标签。
        label: String,
        /// 可选项（字符串数组）。
        options: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_serialize_roundtrip() {
        let w = Widget::Card(Card {
            title: "t".into(),
            body: vec!["b1".into(), "b2".into()],
            actions: vec![Button::new("ok", "OK")],
        });
        let json = serde_json::to_string(&w).expect("序列化");
        let back: Widget = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(w, back);
    }

    #[test]
    fn button_default_enabled_false_via_serde_default() {
        // 注：#[serde(default)] 对 bool 的默认值是 false，与 Button::new 手工 true 不同
        let json = r#"{"id":"a","label":"B"}"#;
        let b: Button = serde_json::from_str(json).unwrap();
        assert_eq!(b.id, "a");
        assert_eq!(b.label, "B");
        assert!(
            !b.enabled,
            "JSON 缺失 enabled 时 #[serde(default)] 应走 false"
        );
        // 而构造器默认 true
        let c = Button::new("x", "Y");
        assert!(c.enabled, "Button::new 默认应为 enabled=true");
        // disabled 链式
        let d = Button::new("x", "Y").disabled();
        assert!(!d.enabled, "disabled() 应把 enabled 转为 false");
    }

    #[test]
    fn progress_bar_clamps() {
        let p = ProgressBar {
            id: "p".into(),
            percent: 1.5,
        };
        assert!((p.clamped_percent() - 1.0).abs() < f32::EPSILON);
        let n = ProgressBar {
            id: "p".into(),
            percent: -0.5,
        };
        assert!((n.clamped_percent() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn form_field_text_default_placeholder() {
        let json = r#"{"kind":"text","id":"f1","label":"Name"}"#;
        let f: FormField = serde_json::from_str(json).unwrap();
        match f {
            FormField::Text {
                id,
                label,
                placeholder,
            } => {
                assert_eq!(id, "f1");
                assert_eq!(label, "Name");
                assert_eq!(placeholder, "");
            }
            _ => panic!("应为 Text 变体"),
        }
    }
}
