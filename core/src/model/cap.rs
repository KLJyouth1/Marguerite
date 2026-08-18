//! 角色→模型 能力路由（C4 蒸馏自 CodeWhale `config/src/route`）
//!
//! 从 CodeWhale 的 `RouteResolver` / `RouteCapabilities` / `CapabilityState` 蒸馏出
//! 「按角色所需能力选择模型」的最小内核，作为 Marguerite `fleet` 角色绑定的能力增强层：
//!
//! 1. **三态能力事实 `CapabilityState`**（Supported/Unsupported/Unknown）—— 直接移植
//!    CodeWhale 硬约束：缺失即 `Unknown`，**绝不**因协议/模型家族启发式把 `Unknown`
//!    提升为 `Supported`；`Required` 能力仅 `Supported` 通过，可选能力只要非
//!    `Unsupported` 即可（诚实承认「不知道」）。
//! 2. **角色需求 `RoleRequirements`**——声明该角色「必须」具备的能力。
//! 3. **`RoleCapRouter::resolve`**——筛掉显式不满足某必需能力的 offering，其余按能力
//!    覆盖率排序、同覆盖取低成本，得到最优绑定。
//!
//! 与 CodeWhale 的差异：CodeWhale 是 provider/model 路由（无 prompt 字段、显式 provider
//! 作用域、防前缀嗅探）；本模块只负责「角色能力匹配」，不接管 provider 解析，专注可单测、
//! 可组合的纯选择逻辑。

use serde::{Deserialize, Serialize};

use crate::model::fleet::RoleKind;

/// 三态能力事实（蒸馏自 CodeWhale `capabilities::CapabilityState`）。
///
/// 一个模型 offering 对某项能力「是否已确认支持」的诚实陈述。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    /// 该 offering 明确声明支持。
    Supported,
    /// 该 offering 明确声明不支持。
    Unsupported,
    /// 该 offering 未声明该事实（默认的「不知道」，绝不推断为支持）。
    #[serde(other)]
    Unknown,
}

impl Default for CapabilityState {
    fn default() -> Self {
        CapabilityState::Unknown
    }
}

impl CapabilityState {
    /// 把可选的布尔事实保留为三态：Some(true)=Supported，Some(false)=Unsupported，None=Unknown。
    #[must_use]
    pub const fn from_optional_bool(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Supported,
            Some(false) => Self::Unsupported,
            None => Self::Unknown,
        }
    }

    /// 该 offering 是否明确支持此能力。
    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    /// 该能力是否满足「角色要求」：
    /// - 当 `required == true`（角色必需）：仅 `Supported` 通过；
    /// - 当 `required == false`（可选）：`Supported`/`Unknown` 均通过，`Unsupported` 才排除。
    #[must_use]
    pub const fn satisfies_requirement(self, required: bool) -> bool {
        if required {
            self.is_supported()
        } else {
            !matches!(self, Self::Unsupported)
        }
    }
}

/// 一个模型 offering 暴露的能力事实（蒸馏自 CodeWhale `RouteCapabilities` 的子集）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferingCapabilities {
    #[serde(default)]
    pub image_input: CapabilityState,
    #[serde(default)]
    pub reasoning: CapabilityState,
    #[serde(default)]
    pub native_tool_calls: CapabilityState,
    #[serde(default)]
    pub structured_output: CapabilityState,
    #[serde(default)]
    pub streaming: CapabilityState,
    #[serde(default)]
    pub prompt_caching: CapabilityState,
    #[serde(default)]
    pub parallel_tool_calls: CapabilityState,
}

/// 候选模型（轻量视图，不依赖具体 provider 注册表，便于纯单测）。
#[derive(Debug, Clone)]
pub struct ModelOffering {
    /// 稳定标识（如 profile id）。
    pub id: String,
    /// provider 名（仅作展示/审计，不参与选择，避免 CodeWhale 反对的「前缀嗅探」）。
    pub provider: String,
    /// 线上模型 id（原样透传，不在此推断 provider）。
    pub wire_model_id: String,
    /// 能力事实。
    pub capabilities: OfferingCapabilities,
    /// 上下文窗口 token 数（0 = 未知）。
    pub context_tokens: u32,
    /// 每百万输出 token 成本（0 = 未知/免费）。
    pub cost_per_1m_out: f64,
}

/// 角色对能力的硬性要求。
///
/// `true` = 该能力 **必须** 为 `Supported`（否则此 offering 被排除）；
/// `false` = 可选（`Unsupported` 才排除，`Unknown` 视为可接受）。
#[derive(Debug, Clone, Copy, Default)]
pub struct RoleRequirements {
    pub image_input: bool,
    pub reasoning: bool,
    pub native_tool_calls: bool,
    pub structured_output: bool,
    pub streaming: bool,
    pub prompt_caching: bool,
    pub parallel_tool_calls: bool,
}

/// 把 Marguerite 的 `RoleKind` 映射为能力需求（C4 角色路由核心：不同角色要求不同能力）。
///
/// - `Architect`：强推理 + 结构化输出（方案 JSON）+ 工具调用。
/// - `Builder`：原生工具调用 + 结构化输出 + 流式（编码 UX）。
/// - `Reviewer`：强推理 + 结构化输出（判定 JSON）。
/// - `CheapCommander`：流式（便宜模型指挥贵模型）。
/// - `Custom`：按名字嗅探语义——含 vision/computer/image/gui 的角色 **必须** image_input
///   （GUI_Agent 的 computer-use 视觉闭环即此类），含 reason 的角色须推理。
#[must_use]
pub fn role_requirements_for(kind: &RoleKind) -> RoleRequirements {
    match kind {
        RoleKind::Architect => RoleRequirements {
            reasoning: true,
            structured_output: true,
            native_tool_calls: true,
            ..Default::default()
        },
        RoleKind::Builder => RoleRequirements {
            native_tool_calls: true,
            structured_output: true,
            streaming: true,
            ..Default::default()
        },
        RoleKind::Reviewer => RoleRequirements {
            reasoning: true,
            structured_output: true,
            ..Default::default()
        },
        RoleKind::CheapCommander => RoleRequirements {
            streaming: true,
            ..Default::default()
        },
        RoleKind::Custom(name) => {
            let n = name.to_ascii_lowercase();
            if n.contains("vision")
                || n.contains("computer")
                || n.contains("image")
                || n.contains("gui")
            {
                // GUI_Agent 视觉/操控角色：必须能接收截图图像输入。
                RoleRequirements {
                    image_input: true,
                    streaming: true,
                    ..Default::default()
                }
            } else if n.contains("reason") {
                RoleRequirements {
                    reasoning: true,
                    structured_output: true,
                    ..Default::default()
                }
            } else {
                RoleRequirements::default()
            }
        }
    }
}

/// 单 offering 对角色需求的匹配度。
///
/// 返回 `None` 表示该 offering **显式不满足**某必需能力（被排除）；
/// 否则返回 0..1 的能力覆盖率（非 `Unsupported` 的能力项数 / 总项数），用作排序键。
#[must_use]
pub fn score_role_match(req: &RoleRequirements, off: &ModelOffering) -> Option<f64> {
    let caps = &off.capabilities;
    if !caps.image_input.satisfies_requirement(req.image_input) {
        return None;
    }
    if !caps.reasoning.satisfies_requirement(req.reasoning) {
        return None;
    }
    if !caps.native_tool_calls.satisfies_requirement(req.native_tool_calls) {
        return None;
    }
    if !caps.structured_output.satisfies_requirement(req.structured_output) {
        return None;
    }
    if !caps.streaming.satisfies_requirement(req.streaming) {
        return None;
    }
    if !caps.prompt_caching.satisfies_requirement(req.prompt_caching) {
        return None;
    }
    if !caps
        .parallel_tool_calls
        .satisfies_requirement(req.parallel_tool_calls)
    {
        return None;
    }
    let fields = [
        caps.image_input,
        caps.reasoning,
        caps.native_tool_calls,
        caps.structured_output,
        caps.streaming,
        caps.prompt_caching,
        caps.parallel_tool_calls,
    ];
    let satisfied = fields
        .iter()
        .filter(|c| !matches!(c, CapabilityState::Unsupported))
        .count();
    Some(satisfied as f64 / fields.len() as f64)
}

/// 角色能力路由器：在候选 offering 中选最优绑定。
#[derive(Debug, Clone, Default)]
pub struct RoleCapRouter {
    pub offerings: Vec<ModelOffering>,
}

/// 一次角色解析的结果（不可变快照；若后续有覆盖写，须带 provenance，参见 CodeWhale 设计）。
#[derive(Debug, Clone)]
pub struct ResolvedRoleRoute {
    pub offering_id: String,
    pub provider: String,
    pub wire_model_id: String,
    /// 能力覆盖率 0..1。
    pub coverage: f64,
    pub context_tokens: u32,
    pub cost_per_1m_out: f64,
}

impl RoleCapRouter {
    #[must_use]
    pub fn new(offerings: Vec<ModelOffering>) -> Self {
        Self { offerings }
    }

    /// 为某角色解析最优 offering：覆盖率最高者胜，覆盖率相同取低成本。
    /// 无任何 offering 满足必需能力时返回 `None`。
    #[must_use]
    pub fn resolve(&self, role: &RoleKind) -> Option<ResolvedRoleRoute> {
        self.acceptable(role).into_iter().next()
    }

    /// 返回所有「满足必需能力」的候选，按覆盖率降序、成本升序排列。
    #[must_use]
    pub fn acceptable(&self, role: &RoleKind) -> Vec<ResolvedRoleRoute> {
        let req = role_requirements_for(role);
        let mut routes: Vec<ResolvedRoleRoute> = self
            .offerings
            .iter()
            .filter_map(|off| {
                let cov = score_role_match(&req, off)?;
                Some(ResolvedRoleRoute {
                    offering_id: off.id.clone(),
                    provider: off.provider.clone(),
                    wire_model_id: off.wire_model_id.clone(),
                    coverage: cov,
                    context_tokens: off.context_tokens,
                    cost_per_1m_out: off.cost_per_1m_out,
                })
            })
            .collect();
        routes.sort_by(|a, b| {
            b.coverage
                .partial_cmp(&a.coverage)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.cost_per_1m_out
                        .partial_cmp(&b.cost_per_1m_out)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        routes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offering(
        id: &str,
        caps: OfferingCapabilities,
        context_tokens: u32,
        cost: f64,
    ) -> ModelOffering {
        ModelOffering {
            id: id.to_string(),
            provider: "deepseek".to_string(),
            wire_model_id: id.to_string(),
            capabilities: caps,
            context_tokens,
            cost_per_1m_out: cost,
        }
    }

    fn supported() -> OfferingCapabilities {
        OfferingCapabilities {
            image_input: CapabilityState::Supported,
            reasoning: CapabilityState::Supported,
            native_tool_calls: CapabilityState::Supported,
            structured_output: CapabilityState::Supported,
            streaming: CapabilityState::Supported,
            prompt_caching: CapabilityState::Supported,
            parallel_tool_calls: CapabilityState::Supported,
        }
    }

    #[test]
    fn capability_state_three_valued() {
        assert_eq!(
            CapabilityState::from_optional_bool(None),
            CapabilityState::Unknown
        );
        assert_eq!(
            CapabilityState::from_optional_bool(Some(false)),
            CapabilityState::Unsupported
        );
        assert_eq!(
            CapabilityState::from_optional_bool(Some(true)),
            CapabilityState::Supported
        );
        // 三态默认即 Unknown（缺失绝不推断为支持）
        assert_eq!(CapabilityState::default(), CapabilityState::Unknown);
        // Required 仅 Supported 通过；可选则 Unknown 也可接受
        assert!(CapabilityState::Supported.satisfies_requirement(true));
        assert!(!CapabilityState::Unknown.satisfies_requirement(true));
        assert!(!CapabilityState::Unsupported.satisfies_requirement(true));
        assert!(CapabilityState::Unknown.satisfies_requirement(false));
        assert!(CapabilityState::Supported.satisfies_requirement(false));
        assert!(!CapabilityState::Unsupported.satisfies_requirement(false));
    }

    #[test]
    fn architect_requires_reasoning_and_structured() {
        let req = role_requirements_for(&RoleKind::Architect);
        assert!(req.reasoning);
        assert!(req.structured_output);
        // 全部 Supported 的 offering 通过
        let good = offering("v4-pro", supported(), 64_000, 5.0);
        assert!(score_role_match(&req, &good).is_some());
        // reasoning=Unknown 的 offering 被排除（Required 不满足）
        let mut weak = supported();
        weak.reasoning = CapabilityState::Unknown;
        let no_reason = offering("weak", weak, 64_000, 1.0);
        assert!(score_role_match(&req, &no_reason).is_none());
        // reasoning=Unsupported 同样排除
        let mut bad = supported();
        bad.reasoning = CapabilityState::Unsupported;
        let bad_off = offering("bad", bad, 64_000, 1.0);
        assert!(score_role_match(&req, &bad_off).is_none());
    }

    #[test]
    fn vision_custom_role_requires_image_input() {
        let role = RoleKind::Custom("computer_use".to_string());
        let req = role_requirements_for(&role);
        assert!(req.image_input);
        // 带 image_input=Supported 的 offering 通过
        let vis = offering("vlmm", supported(), 128_000, 8.0);
        assert!(score_role_match(&req, &vis).is_some());
        // image_input=Unknown（未声明）的 offering 被排除
        let mut no_img = supported();
        no_img.image_input = CapabilityState::Unknown;
        let blind = offering("blind", no_img, 128_000, 1.0);
        assert!(score_role_match(&req, &blind).is_none());
    }

    #[test]
    fn resolve_picks_highest_coverage_then_cheapest() {
        // 仅 `Unsupported`（而非 `Unknown`）才真正降低覆盖率。Architect 不要求 streaming，
        // 故 streaming=Unsupported 只降覆盖率，不被排除。
        let mut partial = supported();
        partial.streaming = CapabilityState::Unsupported; // 6/7
        let offerings = vec![
            offering("cheap-partial", partial, 32_000, 1.0),
            offering("pricey-full", supported(), 64_000, 9.0), // 7/7
            offering("mid-full", supported(), 64_000, 5.0),    // 7/7
        ];
        let router = RoleCapRouter::new(offerings);
        let best = router.resolve(&RoleKind::Architect).expect("应有候选");
        assert_eq!(best.offering_id, "mid-full"); // 7/7 胜 6/7
        // 同覆盖率取低成本：两个 full 候选，便宜者胜
        let offerings2 = vec![
            offering("pricey", supported(), 64_000, 9.0),
            offering("cheap", supported(), 32_000, 1.0),
        ];
        let router2 = RoleCapRouter::new(offerings2);
        let best2 = router2.resolve(&RoleKind::Architect).expect("应有候选");
        assert_eq!(best2.offering_id, "cheap");
    }

    #[test]
    fn acceptable_returns_sorted_acceptable_only() {
        // Builder 需要 native_tool_calls + structured_output + streaming
        let mut no_tools = supported();
        no_tools.native_tool_calls = CapabilityState::Unsupported; // 被排除
        let offerings = vec![
            offering("no-tools", no_tools, 64_000, 1.0),
            offering("full-a", supported(), 64_000, 3.0),
            offering("full-b", supported(), 32_000, 2.0),
        ];
        let router = RoleCapRouter::new(offerings);
        let list = router.acceptable(&RoleKind::Builder);
        // no-tools 被排除，剩下两个 full，按成本升序：full-b(2.0) 在前
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].offering_id, "full-b");
        assert_eq!(list[1].offering_id, "full-a");
    }
}
