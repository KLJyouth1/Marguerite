//! 安全闸门（Epic C4）：全局开关 + 敏感动作二次确认 + 命令字段注入扫描。
//!
//! 完全开放模式下仍保留 kill-switch 与二次确认；前端在 `desktop.action` 调用前
//! 弹确认，后端也保留拒绝逻辑（未带 approved=true 则拒绝敏感动作）。
//!
//! 2026-10-26（W3.5）语义化：旧实现对 `action.params.to_string()` 全文匹配
//! `; | cmd powershell ..\`，于是「比较 cmd 与 powershell 的区别」这种普通文本被拦、
//! JSON 转义也会误判，而「点击名为『删除』的按钮」反而完全不拦（动作 kind 是 click，
//! 词表里只有 delete/send/...）。现在：
//! - 注入特征**只**扫会进入终端/地址栏/命令行的字段（见 [`Policy::scan_command_text`]）；
//! - 危险目标（删除/支付/发送…）按**控件名称**判定后要求二次确认（见 [`Policy::sensitive_target`]）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::desktop_automation::driver::UiAction;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// 动作 kind 本身即敏感（需要 approved=true）。
const SENSITIVE_KINDS: &[&str] = &[
    "delete",
    "send",
    "pay",
    "quit",
    "close",
    "shutdown",
    "system_command",
    "inject",
];

/// 会进入命令面的动作 kind（做注入特征扫描）。
const COMMAND_KINDS: &[&str] = &["run_command", "command", "exec"];

/// 文本类动作：只有目标是终端/地址栏时才做命令面扫描。
const TEXT_KINDS: &[&str] = &["type", "type_text", "set_value"];

/// 命令面注入特征（含路径逃逸；只在命令字段内匹配）。
const COMMAND_INJECTION_PATTERNS: &[&str] = &[
    ";", "&&", "||", "|", "$(", "`", "..\\", "../", "cmd", "powershell", "regsvr32",
];

/// 危险操作的目标文案（点击/调用这些控件前必须二次确认）。
///
/// CJK 词按「整词 / 首尾 + ≤2 字余量」匹配（见 [`label_matches`]），ASCII 词按词边界匹配，
/// 因此「发送者信息」不会误伤，而「删除文件」「确认删除」会命中。
const SENSITIVE_TARGETS: &[&str] = &[
    "删除",
    "移除",
    "卸载",
    "格式化",
    "清空",
    "支付",
    "转账",
    "发送",
    "提交订单",
    "注销",
    "delete",
    "remove",
    "uninstall",
    "format",
    "erase",
    "pay",
    "transfer",
    "send",
    "submit",
    "sign out",
];

/// 按分隔符与 camelCase 驼峰切词的小写词表。
///
/// `AutomationId` 常是 `DeleteButton` / `sendMessageBtn`：只按非字母数字切分时它们
/// 是一个整词，永远匹配不到 `delete` / `send` —— 而这正是最需要确认的控件。
fn split_words(label: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev_was_lower = false;
    for ch in label.chars() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_was_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_was_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        prev_was_lower = ch.is_lowercase() || ch.is_numeric();
        current.extend(ch.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// 目标标签是否命中危险词。
///
/// - ASCII 词：按分隔符与 camelCase 切词后的整词匹配（`Delete file`、`DeleteButton`
///   命中，`sender` 不命中）；含空格的短语（`sign out`）按子串匹配。
/// - CJK 词：整词相等，或位于标签首/尾且余量 ≤ 2 个字。这样 `删除文件`（余 2）命中，
///   `发送者信息`（余 3 且是名词短语）不命中。
fn label_matches(term: &str, label: &str) -> bool {
    let label = label.trim();
    let term = term.to_lowercase();
    if term.is_empty() || label.is_empty() {
        return false;
    }
    if term.is_ascii() {
        if term.contains(' ') {
            return label.to_lowercase().contains(&term);
        }
        return split_words(label).iter().any(|word| word == &term);
    }
    let label = label.to_lowercase();
    if label == term {
        return true;
    }
    let short_tail = |rest: &str| rest.chars().count() <= 2;
    if let Some(rest) = label.strip_prefix(&term) {
        if short_tail(rest) {
            return true;
        }
    }
    if let Some(rest) = label.strip_suffix(&term) {
        if short_tail(rest) {
            return true;
        }
    }
    false
}

pub struct Policy;

impl Policy {
    /// 全局开关注入点（kill-switch）。关闭后所有桌面自动化动作被拒。
    pub fn set_enabled(on: bool) {
        ENABLED.store(on, Ordering::SeqCst);
    }

    pub fn is_enabled() -> bool {
        ENABLED.load(Ordering::SeqCst)
    }

    /// 是否需要二次确认。默认对敏感动作返回 true；前端据此弹确认，
    /// 后端在 `desktop.action` 中若未带 `approved=true` 则拒绝。
    ///
    /// 除 kind 白名单外，`click`/`invoke` 的目标名称命中危险词也算敏感
    /// （「点一下删除」不比「delete 动作」更安全）；**按坐标点击**同样算敏感，
    /// 因为落点是什么在这一层无从判断（见 [`Policy::confirmation_reason`]）。
    pub fn require_confirmation(action: &UiAction) -> bool {
        Self::confirmation_reason(action).is_some()
    }

    /// 需要确认的**理由**（不需要确认则为 `None`）。
    ///
    /// 2026-10-31 修：旧实现只按 `kind` 与目标名称判定，于是「点击坐标 (x, y)」
    /// 这一整类动作**永远**命中不了敏感词 —— 模型只要不写控件名、改给坐标，
    /// 删除/支付/发送这类高危控件就静默免确认地被执行。裸坐标无法在纯策略层
    /// 解析成控件语义，所以按「未知目标按不可信处理」要求确认。
    pub fn confirmation_reason(action: &UiAction) -> Option<String> {
        let kind = action.kind.as_str();
        if SENSITIVE_KINDS.contains(&kind) {
            return Some(format!("动作 `{kind}` 本身属于敏感类别"));
        }
        if !matches!(kind, "click" | "invoke" | "double_click") {
            return None;
        }
        if let Some(term) = Self::sensitive_target(action) {
            return Some(format!("目标命中危险操作词「{term}」"));
        }
        if let Some((x, y)) = Self::point(action) {
            return Some(format!(
                "按坐标 ({x:.1}, {y:.1}) 点击时无法确定落点控件，请携带 approved=true 或改用目标名称"
            ));
        }
        None
    }

    /// 动作的落点坐标：`point:{x,y}` 优先，其次顶层 `x`/`y`（数字）。
    /// 只接受有限数（`NaN`/`inf` 不是坐标）。
    pub fn point(action: &UiAction) -> Option<(f64, f64)> {
        fn finite(v: &serde_json::Value) -> Option<f64> {
            v.as_f64().filter(|n| n.is_finite())
        }
        let params = &action.params;
        if let Some(serde_json::Value::Object(p)) = params.get("point") {
            if let (Some(x), Some(y)) = (p.get("x").and_then(finite), p.get("y").and_then(finite)) {
                return Some((x, y));
            }
        }
        match (
            params.get("x").and_then(finite),
            params.get("y").and_then(finite),
        ) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        }
    }

    /// 点击/调用目标是否命中危险文案；命中返回命中的词，便于回执说明原因。
    pub fn sensitive_target(action: &UiAction) -> Option<String> {
        for key in [
            "name",
            "target",
            "label",
            "text",
            "automation_id",
            "automationId",
            "help_text",
        ] {
            let Some(raw) = action.param_str(key) else {
                continue;
            };
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            for term in SENSITIVE_TARGETS {
                if label_matches(term, raw) {
                    return Some((*term).to_string());
                }
            }
        }
        None
    }

    /// 注入扫描：**只**检查会进入命令/终端的字段。
    ///
    /// 普通文本输入（搜索框里写 `a|b`、把「比较 cmd 与 powershell」写进笔记）不扫描 ——
    /// 旧实现把整个 params 序列化后匹配，把这些正当文本一律拦下。命中则返回告警理由。
    pub fn scan_command_text(action: &UiAction) -> Option<String> {
        let kind = action.kind.as_str();
        let enters_command_surface = if COMMAND_KINDS.contains(&kind) {
            true
        } else if TEXT_KINDS.contains(&kind) {
            let target = ["target", "name", "label"]
                .iter()
                .filter_map(|key| action.param_str(key))
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            ["terminal", "console", "shell", "地址栏", "address", "omnibox", "url"]
                .iter()
                .any(|needle| target.contains(needle))
        } else {
            false
        };
        if !enters_command_surface {
            return None;
        }
        for key in ["command", "text", "value", "argv"] {
            let Some(value) = action.params.get(key) else {
                continue;
            };
            let haystack = match value {
                serde_json::Value::String(text) => text.clone(),
                serde_json::Value::Array(items) => items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => continue,
            };
            for pattern in COMMAND_INJECTION_PATTERNS {
                if haystack.contains(pattern) {
                    return Some(format!("检测到潜在注入特征: {pattern}"));
                }
            }
        }
        None
    }
}

/// 启动时在网关侧注册一个全局 Policy 实例引用（供未来扩展持久化开关）。
pub fn global_policy() -> &'static Policy {
    static P: OnceLock<Policy> = OnceLock::new();
    P.get_or_init(|| Policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn action(kind: &str, params: serde_json::Value) -> UiAction {
        UiAction {
            kind: kind.to_string(),
            params,
        }
    }

    #[test]
    fn plain_text_containing_command_words_is_not_blocked() {
        let a = action("type_text", json!({ "text": "比较 cmd 与 powershell 的区别" }));
        assert_eq!(Policy::scan_command_text(&a), None);
        let piped_note = action("type", json!({ "name": "笔记", "text": "a|b" }));
        assert_eq!(Policy::scan_command_text(&piped_note), None);
    }

    #[test]
    fn terminal_input_with_shell_metacharacters_is_blocked() {
        let a = action(
            "type",
            json!({ "target": "Terminal", "text": "rm -rf build && echo ok" }),
        );
        assert!(Policy::scan_command_text(&a).is_some());
        let cmd = action("run_command", json!({ "command": "cat ../secrets" }));
        assert!(Policy::scan_command_text(&cmd).is_some());
    }

    #[test]
    fn clicking_a_destructive_label_requires_confirmation() {
        let a = action("click", json!({ "name": "删除" }));
        assert!(Policy::require_confirmation(&a));
        assert_eq!(Policy::sensitive_target(&a).as_deref(), Some("删除"));
        let wide = action("click", json!({ "name": "删除文件" }));
        assert!(Policy::require_confirmation(&wide));
    }

    #[test]
    fn noun_phrases_containing_a_danger_word_are_not_sensitive() {
        // 「发送者信息」是名词短语，不是发送动作 —— 不该要求确认。
        let a = action("click", json!({ "name": "发送者信息" }));
        assert!(!Policy::require_confirmation(&a), "不应误伤名词短语");
        assert_eq!(Policy::sensitive_target(&a), None);
        let harmless = action("click", json!({ "name": "Settings" }));
        assert!(!Policy::require_confirmation(&harmless));
    }

    #[test]
    fn ascii_destructive_labels_match_on_word_boundaries() {
        assert!(Policy::require_confirmation(&action("click", json!({ "name": "Delete file" }))));
        assert!(Policy::require_confirmation(&action("click", json!({ "name": "Sign out" }))));
        assert!(!Policy::require_confirmation(&action("click", json!({ "name": "sender" }))));
    }

    #[test]
    fn sensitive_kinds_still_require_confirmation() {
        assert!(Policy::require_confirmation(&action("delete", json!({}))));
        assert!(Policy::require_confirmation(&action("pay", json!({}))));
    }

    #[test]
    fn coordinate_click_requires_confirmation() {
        // 回归：旧实现只按名称判定，坐标点击永远免确认 —— 于是一个只给坐标的
        // 「点删除按钮」可以静默越过二次确认。落点未知就必须问。
        let blind = action("click", json!({ "x": 812.0, "y": 604.0 }));
        assert!(Policy::require_confirmation(&blind));
        assert!(Policy::confirmation_reason(&blind)
            .unwrap_or_default()
            .contains("无法确定落点控件"));
        let via_point = action("click", json!({ "point": { "x": 10, "y": 20 } }));
        assert!(Policy::require_confirmation(&via_point));
        let double = action("double_click", json!({ "x": 1, "y": 2 }));
        assert!(Policy::require_confirmation(&double));
        // 只给一半坐标不是坐标点击（不猜），交给既有的目标判定。
        let half = action("click", json!({ "x": 10.0, "name": "Settings" }));
        assert!(!Policy::require_confirmation(&half));
        // 非点击类动作不受这条规则影响（type 有文本、scroll 只有方向）。
        assert!(!Policy::require_confirmation(&action(
            "type",
            json!({ "x": 1.0, "y": 2.0, "text": "hi" })
        )));
    }

    #[test]
    fn coordinate_click_still_named_sensitive_wins_the_reason() {
        let both = action("click", json!({ "x": 5, "y": 6, "name": "删除" }));
        let reason = Policy::confirmation_reason(&both).unwrap_or_default();
        assert!(reason.contains("删除"), "命名命中应优先解释：{reason}");
    }

    #[test]
    fn point_rejects_non_finite_and_partial_coordinates() {
        assert_eq!(Policy::point(&action("click", json!({ "x": 1.0 }))), None);
        assert_eq!(
            Policy::point(&action("click", json!({ "x": null, "y": 3 }))),
            None
        );
        assert_eq!(
            Policy::point(&action("click", json!({ "point": { "x": 7, "y": 8 } }))),
            Some((7.0, 8.0))
        );
        // 顶层 x/y 也可以直接是整数。
        assert_eq!(
            Policy::point(&action("click", json!({ "x": 7, "y": 8 }))),
            Some((7.0, 8.0))
        );
    }

    #[test]
    fn automation_id_participates_in_sensitivity() {
        // 有些框架（Chromium/UIA）把可读名称藏在 AutomationId 里，Name 为空。
        let a = action("click", json!({ "automation_id": "DeleteButton" }));
        assert!(Policy::require_confirmation(&a));
    }

    #[test]
    fn camel_case_automation_ids_split_into_words() {
        for id in ["DeleteButton", "btnDelete", "delete_file", "sendMessageBtn"] {
            assert!(
                Policy::require_confirmation(&action("click", json!({ "automation_id": id }))),
                "{id} 应命中危险词"
            );
        }
        // 含词素但不是整词的不算：`SenderList` 只是发件人列表。
        assert!(!Policy::require_confirmation(&action(
            "click",
            json!({ "automation_id": "SenderList" })
        )));
    }
}
