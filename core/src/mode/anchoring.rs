//! 三阶段锚定（策划 §三 定稿：dsh-forge 前缀门控 + PTC 单调扩展）。
//!
//! 三阶段（会话内工具面**单调扩展**，禁止中途替换或回退）：
//! 1. [`AnchorStage::Anchored`] 首轮锚定冷启动：仅暴露 `minimal_tools`
//!    （默认 bash_persistent + str_replace_editor）；
//! 2. [`AnchorStage::Ptc`] 晋升到 PTC：暴露 `ptc_tool`（单一 run_code）；
//! 3. [`AnchorStage::Full`] 扩展到 Bloom 全工具面：后续按需继续扩展。
//!
//! 前缀门控家族（策划 §三.1 dsh-forge 社区验证）：
//! - `deepseek`：首块包含 "we" 且不含 "let me" → 从 Anchored 晋升到 Ptc；
//! - 其他家族走通用门控；
//! - 未达成门控时，`fallback_steps`（默认 4）兜底强制晋升；
//! - 无工具首轮：响应后自动晋升。
//!
//! 工具面单调扩展是 Marguerite 插件系统的设计红线之一：`active_tools`
//! 随阶段只会加工具，不会回退、不会无规则重置。

use std::collections::BTreeSet;

use crate::mode::spec::BloomAnchoringSpec;

/// 锚定阶段（三阶段，单调前进）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnchorStage {
    /// 首轮锚定：`minimal_tools` 双工具（默认 bash_persistent + str_replace_editor）
    Anchored = 1,
    /// PTC 程序化工具调用（单一 run_code；已自动包含 minimal_tools）
    Ptc = 2,
    /// 扩展到 Bloom 全工具面（已包含 minimal_tools + ptc_tool）
    Full = 3,
}

/// 锚定状态机（纯逻辑、离线可测）
#[derive(Debug, Clone)]
pub struct AnchorState {
    stage: AnchorStage,
    /// 已观察的首块数量
    blocks: u32,
    /// 兜底晋升步数（未达门控时的 N 步兜底）
    steps: u32,
    /// 规格：最小工具集 / PTC 工具 / 兜底步数 / 前缀门控家族
    spec: BloomAnchoringSpec,
}

/// 四步兜底阈值（默认值；实际运行时优先读 spec.fallback_steps）
pub const FALLBACK_STEPS: u32 = 4;

impl Default for AnchorState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnchorState {
    /// 使用默认规格（deepseek 前缀门控 + 2 件最小工具 + 单一 run_code + 4 步兜底）
    pub fn new() -> Self {
        Self::with_spec(BloomAnchoringSpec::default())
    }

    /// 按 [`BloomAnchoringSpec`] 构造：规格化工具面，支持任意前缀门控家族。
    pub fn with_spec(spec: BloomAnchoringSpec) -> Self {
        // 锚定关闭时直接进入 Full 全工具面（向后兼容）
        let stage = if spec.enabled {
            AnchorStage::Anchored
        } else {
            AnchorStage::Full
        };
        Self {
            stage,
            blocks: 0,
            steps: 0,
            spec,
        }
    }

    pub fn stage(&self) -> AnchorStage {
        self.stage
    }

    pub fn is_promoted(&self) -> bool {
        self.stage != AnchorStage::Anchored
    }

    pub fn is_full(&self) -> bool {
        self.stage == AnchorStage::Full
    }

    /// 当前应暴露的工具集（单调扩展，禁止中途替换）：
    /// - Anchored：`minimal_tools`
    /// - Ptc：`minimal_tools` ∪ `{ptc_tool}`
    /// - Full：（调用方自行追加 Bloom 全工具面）；此处至少返回 Ptc 的超集
    pub fn active_tools(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = self.spec.minimal_tools.iter().cloned().collect();
        if self.stage >= AnchorStage::Ptc {
            out.insert(self.spec.ptc_tool.clone());
        }
        out.into_iter().collect()
    }

    /// 最小工具集快照（用于上层做差集，构造「单调追加」）
    pub fn minimal_tools(&self) -> Vec<String> {
        self.spec.minimal_tools.clone()
    }

    /// PTC 工具名
    pub fn ptc_tool(&self) -> &str {
        &self.spec.ptc_tool
    }

    /// 前缀门控家族（如 "deepseek"）
    pub fn prefix_gate(&self) -> &str {
        &self.spec.prefix_gate
    }

    /// 锚定期消息源白名单（对齐梁神：默认仅 user 与 goal 直通）
    pub fn message_sources(&self) -> &[String] {
        &self.spec.message_sources
    }

    /// 某消息源是否允许进入锚定期模型上下文。
    ///
    /// 锚定关闭或已晋升时全部放行；锚定期仅白名单放行（非白名单的
    /// 自动轮次被过滤后不进模型，因而也不会触发晋升分支——对齐梁神 issue #578）。
    pub fn source_allowed(&self, source: &str) -> bool {
        if !self.spec.enabled || self.is_promoted() {
            return true;
        }
        self.spec.message_sources.iter().any(|s| s == source)
    }

    /// 首轮请求输出预算（token；None = 不设限）
    pub fn bootstrap_max_tokens(&self) -> Option<u32> {
        self.spec.bootstrap_max_tokens
    }

    /// 晋升后延迟注入的来源（如 agent-instructions / skill-catalog）
    pub fn deferred_sources(&self) -> &[String] {
        &self.spec.deferred_sources
    }

    /// 晋升后延迟步数（对齐梁神 deferredGraceSteps）
    pub fn deferred_grace_steps(&self) -> u32 {
        self.spec.deferred_grace_steps
    }

    /// 观察一个首块文本；返回本次观察后 Anchored → Ptc 是否发生晋升。
    ///
    /// 门控规则按前缀家族分派（dsh-forge 社区验证）：
    /// - `deepseek`：首块包含 "we" 且不含 "let me" → 晋升
    /// - 其他家族：暂时与 deepseek 一致（通用门控占位，后续可扩展）
    /// - 未达门控时累计步数，达到 `spec.fallback_steps` 兜底强制晋升
    pub fn observe_block(&mut self, text: &str) -> bool {
        if self.is_promoted() {
            return true;
        }
        self.blocks += 1;
        if self.blocks == 1 {
            let (has_anchor, has_reject) = prefix_gate_eval(&self.spec.prefix_gate, text);
            if has_anchor && !has_reject {
                self.promote_to_ptc();
                return true;
            }
        }
        self.steps += 1;
        if self.steps >= self.spec.fallback_steps.max(1) {
            self.promote_to_ptc();
            return true;
        }
        false
    }

    /// 无工具首轮：响应后自动晋升（从 Anchored 到 Ptc）。
    ///
    /// 受规格 `promote_after_first_response` 控制（默认 true，向后兼容）；
    /// 置 false 时保持双工具会话等待显式晋升信号（对齐梁神
    /// `promoteAfterFirstResponse: false` 的严格锚定形态）。
    pub fn note_no_tool_round(&mut self) -> bool {
        if self.is_promoted() {
            return true;
        }
        if self.spec.promote_after_first_response {
            self.promote_to_ptc();
            return true;
        }
        false
    }

    /// 从 Ptc → Full（调用方在 PTC 运行 N 轮后自行决定扩展到 Bloom 全工具面）。
    /// 幂等：已处于 Full 时直接返回 true。
    pub fn expand_to_full(&mut self) -> bool {
        if self.stage == AnchorStage::Full {
            return true;
        }
        if self.stage == AnchorStage::Anchored {
            // 设计红线：禁止跳过 PTC 阶段直接进入 Full（保证单调扩展）
            self.promote_to_ptc();
        }
        self.stage = AnchorStage::Full;
        true
    }

    /// 强制晋升（直接进入 Full；保留给测试 / 紧急旁路）。
    /// 注意：正常路径应走 `observe_block` / `note_no_tool_round` / `expand_to_full`。
    pub fn promote(&mut self) {
        self.stage = AnchorStage::Full;
    }

    fn promote_to_ptc(&mut self) {
        if self.stage == AnchorStage::Anchored {
            self.stage = AnchorStage::Ptc;
        }
    }
}

/// 前缀门控评估：返回 (has_anchor_signal, has_reject_signal)
fn prefix_gate_eval(family: &str, text: &str) -> (bool, bool) {
    // 家族名忽略大小写、去空白
    let fam = family.trim().to_ascii_lowercase();
    // 文本按小写判定；包含边界：\b 语义（简单实现用前后非字母数字或字符串边界）
    let lower = text.to_ascii_lowercase();
    match fam.as_str() {
        "deepseek" => {
            let anchor = contains_word_boundary(&lower, "we");
            let reject = contains_sequence(&lower, "let me");
            (anchor, reject)
        }
        _ => {
            // 通用门控：与 deepseek 一致，留出后续扩展点
            let anchor = contains_word_boundary(&lower, "we");
            let reject = contains_sequence(&lower, "let me");
            (anchor, reject)
        }
    }
}

/// ASCII 字母数字词边界匹配（避免 "we" 在 "well" 中被误判为锚定信号）
fn contains_word_boundary(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if h.len() < n.len() {
        return false;
    }
    for i in 0..=(h.len() - n.len()) {
        if &h[i..i + n.len()] == n {
            let left_ok = i == 0 || !h[i - 1].is_ascii_alphanumeric();
            let right_ok = i + n.len() == h.len()
                || !h[i + n.len()].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
        }
    }
    false
}

fn contains_sequence(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn default_minimal() -> Vec<String> {
        vec!["bash_persistent".into(), "str_replace_editor".into()]
    }

    fn contains(tools: &[String], name: &str) -> bool {
        tools.iter().any(|t| t == name)
    }

    #[test]
    fn starts_anchored_with_minimal_tools() {
        let a = AnchorState::new();
        assert_eq!(a.stage(), AnchorStage::Anchored);
        assert!(!a.is_promoted());
        assert_eq!(a.active_tools(), default_minimal());
    }

    #[test]
    fn first_block_with_we_and_no_let_me_promotes_to_ptc() {
        let mut a = AnchorState::new();
        let promoted = a.observe_block("we should inspect the file");
        assert!(promoted);
        assert!(a.is_promoted());
        assert_eq!(a.stage(), AnchorStage::Ptc);
        // 工具面单调：minimal ∪ {run_code}
        let tools = a.active_tools();
        assert!(contains(&tools, "bash_persistent"));
        assert!(contains(&tools, "str_replace_editor"));
        assert!(contains(&tools, "run_code"));
    }

    #[test]
    fn first_block_with_let_me_holds() {
        let mut a = AnchorState::new();
        let promoted = a.observe_block("let me think about this");
        assert!(!promoted, "含 let me 不应晋升");
        assert!(!a.is_promoted());
        assert_eq!(a.stage(), AnchorStage::Anchored);
    }

    #[test]
    fn first_block_without_we_holds() {
        let mut a = AnchorState::new();
        assert!(!a.observe_block("analyzing the request"));
        assert!(!a.is_promoted());
    }

    #[test]
    fn we_is_matched_on_word_boundary() {
        // "well" 不应触发锚定信号（避免误判）
        let mut a = AnchorState::new();
        assert!(!a.observe_block("well this is a test"));
        assert!(!a.is_promoted());
        // "we, " "We." "We\n" 应命中（各自独立首块）
        let mut b1 = AnchorState::new();
        assert!(b1.observe_block("We should do it"));
        let mut b2 = AnchorState::new();
        assert!(b2.observe_block("yes, we will proceed now"));
        let mut b3 = AnchorState::new();
        assert!(b3.observe_block("We. Got it."));
    }

    #[test]
    fn fallback_steps_honor_spec() {
        let spec = BloomAnchoringSpec {
            fallback_steps: 2,
            ..BloomAnchoringSpec::default()
        };
        let mut a = AnchorState::with_spec(spec);
        assert!(!a.observe_block("let me"));
        // 第 2 步（未达第 1 块门控，但 2 步兜底）
        assert!(a.observe_block("whatever"));
        assert!(a.is_promoted());
    }

    #[test]
    fn four_step_fallback_promotes_to_ptc() {
        let mut a = AnchorState::new();
        // 首块未晋升；后续 3 步仍未晋升 → 第 4 步兜底晋升
        assert!(!a.observe_block("let me think"));
        assert!(!a.observe_block("still thinking"));
        assert!(!a.observe_block("more thinking"));
        assert!(a.observe_block("final step"));
        assert!(a.is_promoted());
        assert_eq!(a.stage(), AnchorStage::Ptc);
    }

    #[test]
    fn no_tool_round_auto_promotes_to_ptc() {
        let mut a = AnchorState::new();
        assert!(a.note_no_tool_round());
        assert!(a.is_promoted());
        assert_eq!(a.stage(), AnchorStage::Ptc);
    }

    #[test]
    fn expand_to_full_is_monotone_and_skipping_is_forbidden() {
        // Anchored → expand_to_full：先自动跳到 Ptc，再到 Full（禁止跳过 PTC）
        let mut a = AnchorState::new();
        assert_eq!(a.stage(), AnchorStage::Anchored);
        assert!(a.expand_to_full());
        assert_eq!(a.stage(), AnchorStage::Full);
        // Ptc → Full
        let mut b = AnchorState::new();
        b.observe_block("we move");
        assert_eq!(b.stage(), AnchorStage::Ptc);
        assert!(b.expand_to_full());
        assert_eq!(b.stage(), AnchorStage::Full);
        // Full 幂等
        assert!(b.expand_to_full());
        assert_eq!(b.stage(), AnchorStage::Full);
        // 单调：Full 工具集 ≥ Ptc ≥ Anchored
        let tools_full = b.active_tools();
        assert!(contains(&tools_full, "bash_persistent"));
        assert!(contains(&tools_full, "str_replace_editor"));
        assert!(contains(&tools_full, "run_code"));
    }

    #[test]
    fn promoted_is_idempotent() {
        let mut a = AnchorState::new();
        a.promote();
        assert!(a.observe_block("anything"));
        assert!(a.note_no_tool_round());
        assert!(a.is_full());
    }

    #[test]
    fn spec_disabled_starts_full() {
        let spec = BloomAnchoringSpec {
            enabled: false,
            ..BloomAnchoringSpec::default()
        };
        let a = AnchorState::with_spec(spec);
        assert_eq!(a.stage(), AnchorStage::Full);
        assert!(a.is_promoted() && a.is_full());
    }

    #[test]
    fn custom_tools_and_prefix_family() {
        let spec = BloomAnchoringSpec {
            minimal_tools: vec!["a".into(), "b".into()],
            ptc_tool: "c".into(),
            prefix_gate: "generic".into(),
            ..BloomAnchoringSpec::default()
        };
        let mut a = AnchorState::with_spec(spec);
        assert_eq!(a.active_tools(), vec!["a", "b"]);
        a.observe_block("We will go");
        let tools = a.active_tools();
        assert!(contains(&tools, "a"));
        assert!(contains(&tools, "b"));
        assert!(contains(&tools, "c"));
    }

    #[test]
    fn default_anchor_spec_fields_align_liangshen() {
        let spec = BloomAnchoringSpec::default();
        assert!(spec.message_sources.iter().any(|s| s == "user"));
        assert!(spec.message_sources.iter().any(|s| s == "goal"));
        assert_eq!(spec.bootstrap_max_tokens, None);
        assert_eq!(spec.deferred_grace_steps, 1);
        assert!(
            spec.deferred_sources
                .iter()
                .any(|s| s == "agent-instructions")
        );
        assert!(
            spec.deferred_sources
                .iter()
                .any(|s| s == "skill-catalog")
        );
        assert!(spec.promote_after_first_response);
    }

    #[test]
    fn source_allowed_filters_non_whitelisted_while_anchored() {
        let mut a = AnchorState::new();
        assert!(a.source_allowed("user"), "user 直通");
        assert!(a.source_allowed("goal"), "goal 自动轮直通");
        assert!(
            !a.source_allowed("auto-round"),
            "非白名单自动轮在锚定期应被过滤"
        );
        a.observe_block("we proceed");
        assert!(
            a.source_allowed("auto-round"),
            "晋升后所有来源放行"
        );
    }

    #[test]
    fn no_tool_round_respects_promote_flag() {
        let spec = BloomAnchoringSpec {
            promote_after_first_response: false,
            ..BloomAnchoringSpec::default()
        };
        let mut a = AnchorState::with_spec(spec);
        assert!(
            !a.note_no_tool_round(),
            "关闭自动晋升时无工具首轮应保持锚定"
        );
        assert_eq!(a.stage(), AnchorStage::Anchored);
        assert!(a.observe_block("we are ready"), "显式信号仍可晋升");
        assert_eq!(a.stage(), AnchorStage::Ptc);
    }

    #[test]
    fn bootstrap_and_deferred_getters_reflect_spec() {
        let spec = BloomAnchoringSpec {
            bootstrap_max_tokens: Some(1024),
            deferred_grace_steps: 2,
            ..BloomAnchoringSpec::default()
        };
        let a = AnchorState::with_spec(spec);
        assert_eq!(a.bootstrap_max_tokens(), Some(1024));
        assert_eq!(a.deferred_grace_steps(), 2);
    }
}
