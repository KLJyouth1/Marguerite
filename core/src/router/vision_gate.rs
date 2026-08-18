//! 视觉取证门（Track B2）：对译 opensquilla `vision_followup_gate.py` 的纯决策逻辑。
//!
//! 端口自 `opensquilla/engine/steps/vision_followup_gate.py`（376 行，含测试）：
//! - L30-41 中英 opt-out 正则（`_IMAGE_OPTOUT_RE` + `_ZH_IMAGE_REFS` 守卫）
//! - L42-63 显式「引用上一张图」判定（`_PREVIOUS_IMAGE_REF_RE` + `_ZH_PREVIOUS_IMAGE_REFS`）
//! - L310-331 unknown 默认策略（`image_if_recent` + `fallback_recent_turns`）
//! - L343-353 上游调用超时（此处仅作文档化常量，见 [`VISION_GATE_TIMEOUT_SECS`]）
//!
//! 本模块是**纯决策函数**：无 LLM、无 IO、零网络。它把「当前文本是否需要对图片取证」
//! 收敛为一个 [`GateDecision`]，完全可单测。真实上游取图（含超时）交由后续切片在 IO 层实现。

use std::sync::LazyLock;

use regex::Regex;

/// 视觉取证门决策（对译 opensquilla `router_vision_followup_gate_decision` 取值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// 需要（重新）获取 / 上传一张图片。
    NeedImage,
    /// 复用最近一轮的图片（携带该轮 turn id）。
    UseRecentImage(String),
    /// 仅用文本，不使用任何图片。
    TextOnly,
    /// 信号不足，交由 unknown 默认策略裁决（见 [`resolve_unknown`]）。
    Unknown,
}

impl GateDecision {
    /// 小写字符串表示（与 JSONL 决策日志 / 事件负载对齐）。
    pub fn as_str(&self) -> &'static str {
        match self {
            GateDecision::NeedImage => "need_image",
            GateDecision::UseRecentImage(_) => "use_recent_image",
            GateDecision::TextOnly => "text_only",
            GateDecision::Unknown => "unknown",
        }
    }
}

/// 视觉取证门的输入上下文（纯数据，无 IO）。
///
/// 对译 opensquilla `TurnContext.metadata` 中 gate 关心的字段：
/// `has_recent_image` ← `router_history_has_recent_image`，
/// `recent_turn_id` ← 最近图片轮次 id，`recent_turns` ← `router_turns_since_last_image`。
#[derive(Debug, Clone)]
pub struct VisionGateContext {
    pub message: String,
    pub has_recent_image: bool,
    pub recent_turn_id: Option<String>,
    pub recent_turns: usize,
}

/// 上游视觉模型调用的硬性超时（秒）。
///
/// 端口 python L343-353 的 `vision_followup_gate_timeout_seconds`（默认 3.0）。
/// 本常量仅作文档化契约：当前 [`evaluate_vision_need`] 是纯决策函数（无网络、无模型调用），
/// 因此不在此产生实际超时；真实上游取图超时交由后续切片在 IO 层实现。
pub const VISION_GATE_TIMEOUT_SECS: f64 = 3.0;

/// unknown 默认策略的「近期窗口」上限（端口 python `vision_followup_gate_fallback_recent_turns` 默认 2）。
const FALLBACK_RECENT_TURNS: usize = 2;

/// 端口 python L30-41 `_IMAGE_OPTOUT_RE`（中英 opt-out 正则，大小写不敏感）。
///
/// 英文：do not / don't / dont / without / no need to + (use|inspect|look at|view|analyze|analyse|consider)
///       + (image|picture|photo|screenshot|screen|diagram)；或 ignore + 图词。
/// 中文：不要/不用/无需/不需要/别 (+ 看/使用/参考/分析/检查) (+ 图/图片/截图/照片)；或 忽略/无视 (+ 图词)。
static OPT_OUT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:do\s+not|don't|dont|without|no\s+need\s+to)\b.{0,80}\b(?:use|inspect|look\s+at|view|analy[sz]e|consider)\b.{0,80}\b(?:image|picture|photo|screenshot|screen|diagram)\b|\bignore\b.{0,80}\b(?:image|picture|photo|screenshot|screen|diagram)\b|(?:不要|不用|无需|不需要|别).{0,40}(?:看|使用|参考|分析|检查).{0,40}(?:图|图片|截图|照片)|(?:忽略|无视).{0,40}(?:图|图片|截图|照片)",
    )
    .expect("opt-out 正则必须合法")
});

/// 端口 python L42-49 `_PREVIOUS_IMAGE_REF_RE`（英文「引用上一张图」正则，大小写不敏感）。
static PREV_IMG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:previous|last|earlier|above|that|the)\b.{0,50}\b(?:image|picture|photo|screenshot|screen|diagram)\b|\b(?:image|picture|photo|screenshot|screen|diagram)\b.{0,50}\b(?:above|before|earlier)\b",
    )
    .expect("previous-image 正则必须合法")
});

/// 端口 python L28 `_IMAGE_REF_RE`（英文图词守卫，opt-out 前置判定用）。
static IMG_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:image|picture|photo|screenshot|screen|diagram)\b")
        .expect("image-ref 正则必须合法")
});

/// 端口 python L29 `_ZH_IMAGE_REFS`：中文图词守卫（opt-out 前置判定用）。
const ZH_IMAGE_REFS: &[&str] = &["图", "图片", "截图", "照片"];

/// 端口 python L50-63 `_ZH_PREVIOUS_IMAGE_REFS`：中文「上一张图」等短语。
const ZH_PREV_IMG_REFS: &[&str] = &[
    "上一张图",
    "上一张图片",
    "上张图",
    "上张图片",
    "刚才那张图",
    "刚才那张图片",
    "前面那张图",
    "前面那张图片",
    "之前那张图",
    "之前那张图片",
    "那张图",
    "那张图片",
];

/// 端口 python L28/L29：`_IMAGE_REF_RE` + `_ZH_IMAGE_REFS` —— 消息是否提及「图」。
fn mentions_image(text: &str) -> bool {
    IMG_REF_RE.is_match(text) || ZH_IMAGE_REFS.iter().any(|r| text.contains(r))
}

/// 端口 python L281-288 `_current_text_explicitly_opts_out_image`。
fn text_opts_out(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    if !mentions_image(text) {
        return false;
    }
    OPT_OUT_RE.is_match(text)
}

/// 端口 python L291-297 `_current_text_explicitly_requests_previous_image`。
fn text_requests_previous_image(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    if ZH_PREV_IMG_REFS.iter().any(|r| text.contains(r)) {
        return true;
    }
    PREV_IMG_RE.is_match(text)
}

/// 解析 indeterminate（Unknown）门状态 —— 端口 python `_apply_unknown_fallback`（L310-331）。
///
/// 策略 `image_if_recent`：当存在近期图片且距上次图片不超过 [`FALLBACK_RECENT_TURNS`] 轮时，
/// 复用最近图片；否则需要一张新图。
pub fn resolve_unknown(ctx: &VisionGateContext) -> GateDecision {
    if ctx.has_recent_image && ctx.recent_turns <= FALLBACK_RECENT_TURNS {
        GateDecision::UseRecentImage(ctx.recent_turn_id.clone().unwrap_or_default())
    } else {
        GateDecision::NeedImage
    }
}

/// 评估当前文本是否需要对图片取证（纯决策，无 LLM / 无 IO）。
///
/// 端口 opensquilla `apply_vision_followup_gate` 的纯逻辑部分（L356-376 的 opt-out /
/// explicit-previous-image 分支），并把「无显式信号」归为 [`GateDecision::Unknown`]
/// 交 [`resolve_unknown`] 裁决（对齐 python 对 unknown 走 `image_if_recent` 的策略）。
pub fn evaluate_vision_need(ctx: &VisionGateContext) -> GateDecision {
    if text_opts_out(&ctx.message) {
        return GateDecision::TextOnly;
    }
    if text_requests_previous_image(&ctx.message) {
        if ctx.has_recent_image {
            return GateDecision::UseRecentImage(ctx.recent_turn_id.clone().unwrap_or_default());
        }
        return GateDecision::NeedImage;
    }
    GateDecision::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(
        message: &str,
        has_recent: bool,
        turn_id: Option<&str>,
        recent_turns: usize,
    ) -> VisionGateContext {
        VisionGateContext {
            message: message.to_string(),
            has_recent_image: has_recent,
            recent_turn_id: turn_id.map(|s| s.to_string()),
            recent_turns,
        }
    }

    // (a) opt-out → TextOnly（端口 python test_gate_respects_explicit_*_image_opt_out）
    #[test]
    fn opt_out_english() {
        let c = ctx(
            "Do not use or inspect the previous image. Reply exactly: TEXT-ONLY",
            true,
            Some("t1"),
            1,
        );
        assert_eq!(evaluate_vision_need(&c), GateDecision::TextOnly);
    }

    #[test]
    fn opt_out_chinese() {
        let c = ctx("不要看上一张图片，直接回答：TEXT-ONLY", true, Some("t1"), 1);
        assert_eq!(evaluate_vision_need(&c), GateDecision::TextOnly);
    }

    // (b) 显式引用上一张图
    #[test]
    fn explicit_ref_recent_use_recent_chinese() {
        let c = ctx("上一张图片是什么颜色？", true, Some("t-7"), 1);
        assert_eq!(
            evaluate_vision_need(&c),
            GateDecision::UseRecentImage("t-7".to_string())
        );
    }

    #[test]
    fn explicit_ref_recent_use_recent_english() {
        let c = ctx("What color was the previous image?", true, Some("t-8"), 1);
        assert_eq!(
            evaluate_vision_need(&c),
            GateDecision::UseRecentImage("t-8".to_string())
        );
    }

    #[test]
    fn explicit_ref_no_recent_need_image() {
        let c = ctx("What was in the previous image?", false, None, 0);
        assert_eq!(evaluate_vision_need(&c), GateDecision::NeedImage);
    }

    // (c) unknown 默认策略（image_if_recent + fallback_recent_turns）
    #[test]
    fn unknown_recent_falls_to_image() {
        let c = ctx("随便聊聊", true, Some("t-9"), 1);
        assert_eq!(
            resolve_unknown(&c),
            GateDecision::UseRecentImage("t-9".to_string())
        );
    }

    #[test]
    fn unknown_no_recent_needs_image() {
        let c = ctx("随便聊聊", false, None, 0);
        assert_eq!(resolve_unknown(&c), GateDecision::NeedImage);
    }

    // (c) 边界：超出近期窗口 → NeedImage（端口 python test_gate_unknown_old_falls_back_to_text）
    #[test]
    fn unknown_old_turn_falls_to_text() {
        let c = ctx("What about this?", true, Some("t-old"), 3);
        assert_eq!(resolve_unknown(&c), GateDecision::NeedImage);
    }

    // (6) 无显式信号 → Unknown；经默认策略落到 image_if_recent
    #[test]
    fn no_signal_is_unknown_then_falls_to_image_if_recent() {
        let c = ctx("今天天气怎么样", true, Some("t-x"), 1);
        assert_eq!(evaluate_vision_need(&c), GateDecision::Unknown);
        // 该 Unknown 经默认策略落到 image_if_recent
        assert_eq!(
            resolve_unknown(&c),
            GateDecision::UseRecentImage("t-x".to_string())
        );
    }

    #[test]
    fn no_signal_without_recent_image_stays_need_image() {
        let c = ctx("今天天气怎么样", false, None, 0);
        assert_eq!(evaluate_vision_need(&c), GateDecision::Unknown);
        assert_eq!(resolve_unknown(&c), GateDecision::NeedImage);
    }

    #[test]
    fn as_str_values() {
        assert_eq!(GateDecision::NeedImage.as_str(), "need_image");
        assert_eq!(
            GateDecision::UseRecentImage("x".into()).as_str(),
            "use_recent_image"
        );
        assert_eq!(GateDecision::TextOnly.as_str(), "text_only");
        assert_eq!(GateDecision::Unknown.as_str(), "unknown");
    }

    #[test]
    fn timeout_constant_is_three() {
        assert_eq!(VISION_GATE_TIMEOUT_SECS, 3.0);
    }

    #[test]
    fn fallback_recent_turns_boundary_at_limit() {
        // recent_turns == FALLBACK_RECENT_TURNS(2) 应仍视为近期
        let c = ctx("hi", true, Some("t-2"), 2);
        assert_eq!(
            resolve_unknown(&c),
            GateDecision::UseRecentImage("t-2".to_string())
        );
    }
}
