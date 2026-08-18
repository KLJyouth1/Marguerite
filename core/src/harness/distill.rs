//! 上游社区仓库能力蒸馏表（融合映射单一事实源）
//!
//! 本表把 `third_party/{harness,computer_use}` 中 27 个上游社区仓库（10 computer-use + 17 harness），按统一勘探
//! （`third_party/exploration_report.md`）与供应链安全审查（`third_party/security_audit_report.md`）
//! 的结论，蒸馏为「仓库 → 可吸收能力标签」的映射。它是 Marguerite 原生 harness / computer_use
//! 模块做二次开发的**来源索引**，使融合有迹可循、可审计。
//!
//! 诚实性约束：每个标签必须对应仓库中真实可见的工程语义；`distilled_via` 指向本仓
//! 实际落地的实现模块（如 `agent::team`、`tools::computer_use`），不虚构映射。

/// 仓库分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoCategory {
    /// agent / subagent / 工具 / 路由 / 检查点类 harness
    Harness,
    /// 屏幕感知 / 键鼠控制 / GUI 定位 / 视觉 grounding 类
    ComputerUse,
}

impl RepoCategory {
    /// 稳定字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            RepoCategory::Harness => "harness",
            RepoCategory::ComputerUse => "computer_use",
        }
    }
}

/// 可吸收的能力标签
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityTag {
    SubagentIsolation,
    LifecycleHooks,
    DynamicRouting,
    ContextBudget,
    GitCheckpoint,
    SandboxGrading,
    ToolSystem,
    AgentTeams,
    CapabilityCenter,
    AutoEvolve,
    ScreenPerception,
    InputControl,
    GuiGrounding,
    VisionModel,
}

impl CapabilityTag {
    /// 稳定字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityTag::SubagentIsolation => "subagent_isolation",
            CapabilityTag::LifecycleHooks => "lifecycle_hooks",
            CapabilityTag::DynamicRouting => "dynamic_routing",
            CapabilityTag::ContextBudget => "context_budget",
            CapabilityTag::GitCheckpoint => "git_checkpoint",
            CapabilityTag::SandboxGrading => "sandbox_grading",
            CapabilityTag::ToolSystem => "tool_system",
            CapabilityTag::AgentTeams => "agent_teams",
            CapabilityTag::CapabilityCenter => "capability_center",
            CapabilityTag::AutoEvolve => "auto_evolve",
            CapabilityTag::ScreenPerception => "screen_perception",
            CapabilityTag::InputControl => "input_control",
            CapabilityTag::GuiGrounding => "gui_grounding",
            CapabilityTag::VisionModel => "vision_model",
        }
    }
}

/// 上游仓库蒸馏条目
pub struct UpstreamRepo {
    /// 仓库名（与 third_party 目录一致）
    pub name: &'static str,
    /// 分类
    pub category: RepoCategory,
    /// 主语言
    pub primary_language: &'static str,
    /// 许可证（简写）
    pub license: &'static str,
    /// 可吸收能力标签
    pub capabilities: &'static [CapabilityTag],
    /// 本仓实际落地的实现模块（融合去向）
    pub distilled_via: &'static str,
}

/// 27 个上游仓库的蒸馏映射（单一事实源）
pub const UPSTREAM_REPOS: &[UpstreamRepo] = &[
    // ---------------- harness 类 ----------------
    UpstreamRepo {
        name: "harness",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "MIT",
        capabilities: &[
            CapabilityTag::SubagentIsolation,
            CapabilityTag::LifecycleHooks,
            CapabilityTag::DynamicRouting,
            CapabilityTag::GitCheckpoint,
            CapabilityTag::ToolSystem,
            CapabilityTag::ContextBudget,
            CapabilityTag::VisionModel,
        ],
        distilled_via: "harness::{subagent,hooks,routing,checkpoint,registry} + mcp::registry",
    },
    UpstreamRepo {
        name: "iPolloWork",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "iPolloWork-Source-Available-1.0",
        capabilities: &[
            CapabilityTag::LifecycleHooks,
            CapabilityTag::DynamicRouting,
            CapabilityTag::GitCheckpoint,
            CapabilityTag::ToolSystem,
            CapabilityTag::AgentTeams,
            CapabilityTag::CapabilityCenter,
        ],
        distilled_via: "harness::{hooks,routing,checkpoint} + agent::team",
    },
    UpstreamRepo {
        name: "loushang",
        category: RepoCategory::Harness,
        primary_language: "Python",
        license: "Apache-2.0",
        capabilities: &[
            CapabilityTag::SubagentIsolation,
            CapabilityTag::LifecycleHooks,
            CapabilityTag::DynamicRouting,
            CapabilityTag::GitCheckpoint,
            CapabilityTag::ToolSystem,
            CapabilityTag::ContextBudget,
        ],
        distilled_via: "harness::{subagent,hooks,routing,checkpoint} + context::budget",
    },
    UpstreamRepo {
        name: "penguin-harness",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "Apache-2.0",
        capabilities: &[
            CapabilityTag::SubagentIsolation,
            CapabilityTag::LifecycleHooks,
            CapabilityTag::DynamicRouting,
            CapabilityTag::GitCheckpoint,
            CapabilityTag::ContextBudget,
        ],
        distilled_via: "harness::{subagent,hooks,routing,checkpoint} + context::budget",
    },
    UpstreamRepo {
        name: "harness-HenryZ838978",
        category: RepoCategory::Harness,
        primary_language: "Python",
        license: "MIT",
        capabilities: &[
            CapabilityTag::LifecycleHooks,
            CapabilityTag::ToolSystem,
            CapabilityTag::GitCheckpoint,
        ],
        distilled_via: "harness::{hooks,checkpoint} + mcp::registry",
    },
    UpstreamRepo {
        name: "deepseek-auto-evolving-harness",
        category: RepoCategory::Harness,
        primary_language: "Python",
        license: "(未声明)",
        capabilities: &[
            CapabilityTag::LifecycleHooks,
            CapabilityTag::DynamicRouting,
            CapabilityTag::GitCheckpoint,
            CapabilityTag::AutoEvolve,
        ],
        distilled_via: "harness::{hooks,routing,checkpoint} + harness::evolution",
    },
    UpstreamRepo {
        name: "dsh-agent-teams",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "(未声明)",
        capabilities: &[
            CapabilityTag::AgentTeams,
            CapabilityTag::ToolSystem,
            CapabilityTag::DynamicRouting,
            CapabilityTag::GitCheckpoint,
        ],
        distilled_via: "agent::team + mcp::registry",
    },
    UpstreamRepo {
        name: "Abu-Cowork",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "Apache-2.0",
        capabilities: &[
            CapabilityTag::AgentTeams,
            CapabilityTag::CapabilityCenter,
            CapabilityTag::LifecycleHooks,
            CapabilityTag::ContextBudget,
            CapabilityTag::SandboxGrading,
        ],
        distilled_via: "agent::team + harness::{hooks,subagent} + sandbox",
    },
    UpstreamRepo {
        name: "CodeWhale",
        category: RepoCategory::Harness,
        primary_language: "Rust",
        license: "MIT",
        capabilities: &[
            CapabilityTag::SubagentIsolation,
            CapabilityTag::ToolSystem,
            CapabilityTag::SandboxGrading,
        ],
        distilled_via: "harness::subagent + security::execpolicy",
    },
    UpstreamRepo {
        name: "CowAgent",
        category: RepoCategory::Harness,
        primary_language: "Python",
        license: "(未声明)",
        capabilities: &[CapabilityTag::ToolSystem, CapabilityTag::SubagentIsolation],
        distilled_via: "mcp::registry + harness::subagent",
    },
    UpstreamRepo {
        name: "LeAgent",
        category: RepoCategory::Harness,
        primary_language: "Python",
        license: "Apache-2.0",
        capabilities: &[CapabilityTag::AgentTeams, CapabilityTag::ToolSystem],
        distilled_via: "agent::team + mcp::registry",
    },
    UpstreamRepo {
        name: "dsh-desktop",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "MIT",
        capabilities: &[CapabilityTag::DynamicRouting, CapabilityTag::ToolSystem],
        distilled_via: "harness::routing + mcp::registry",
    },
    UpstreamRepo {
        name: "awesome-harness",
        category: RepoCategory::Harness,
        primary_language: "Python",
        license: "CC0",
        capabilities: &[CapabilityTag::ToolSystem],
        distilled_via: "mcp::registry (catalog/参考)",
    },
    UpstreamRepo {
        name: "awesome-harness-Dominic789654",
        category: RepoCategory::Harness,
        primary_language: "TS/JS",
        license: "CC0",
        capabilities: &[CapabilityTag::ToolSystem],
        distilled_via: "mcp::registry (catalog/参考)",
    },
    UpstreamRepo {
        name: "awesome-dsh-plugin",
        category: RepoCategory::Harness,
        primary_language: "(none)",
        license: "CC0",
        capabilities: &[CapabilityTag::ToolSystem],
        distilled_via: "mcp::registry (catalog/参考)",
    },
    UpstreamRepo {
        name: "command-code",
        category: RepoCategory::Harness,
        primary_language: "(none)",
        license: "(未声明)",
        capabilities: &[CapabilityTag::ToolSystem],
        distilled_via: "mcp::registry (参考)",
    },
    UpstreamRepo {
        name: "superpowers-reasonix",
        category: RepoCategory::Harness,
        primary_language: "(none)",
        license: "MIT",
        capabilities: &[CapabilityTag::LifecycleHooks],
        distilled_via: "harness::hooks (方法论参考)",
    },
    // ---------------- computer_use 类 ----------------
    UpstreamRepo {
        name: "ScreenSpot-Pro-GUI-Grounding",
        category: RepoCategory::ComputerUse,
        primary_language: "Python",
        license: "MIT",
        capabilities: &[
            CapabilityTag::ScreenPerception,
            CapabilityTag::GuiGrounding,
            CapabilityTag::VisionModel,
        ],
        distilled_via: "tools::computer_use::GuiGrounding (视觉 grounding 主参考)",
    },
    UpstreamRepo {
        name: "OpenCUA",
        category: RepoCategory::ComputerUse,
        primary_language: "Python",
        license: "MIT",
        capabilities: &[
            CapabilityTag::ScreenPerception,
            CapabilityTag::InputControl,
            CapabilityTag::GuiGrounding,
            CapabilityTag::VisionModel,
        ],
        distilled_via: "tools::computer_use (三层编排主参考)",
    },
    UpstreamRepo {
        name: "computer_use_ootb",
        category: RepoCategory::ComputerUse,
        primary_language: "Python",
        license: "Apache-2.0",
        capabilities: &[
            CapabilityTag::ScreenPerception,
            CapabilityTag::InputControl,
            CapabilityTag::GuiGrounding,
        ],
        distilled_via: "tools::computer_use (OOTB 编排参考)",
    },
    UpstreamRepo {
        name: "computer-agent",
        category: RepoCategory::ComputerUse,
        primary_language: "TS/JS",
        license: "Apache-2.0",
        capabilities: &[CapabilityTag::ScreenPerception, CapabilityTag::GuiGrounding],
        distilled_via: "tools::computer_use::{ScreenPerception,GuiGrounding}",
    },
    UpstreamRepo {
        name: "robotgo",
        category: RepoCategory::ComputerUse,
        primary_language: "Go",
        license: "Apache-2.0",
        capabilities: &[
            CapabilityTag::InputControl,
            CapabilityTag::GuiGrounding,
            CapabilityTag::VisionModel,
        ],
        distilled_via: "tools::computer_use::InputControl (跨平台键鼠主参考)",
    },
    UpstreamRepo {
        name: "computer-vision-guide",
        category: RepoCategory::ComputerUse,
        primary_language: "Python",
        license: "MIT",
        capabilities: &[CapabilityTag::VisionModel, CapabilityTag::GuiGrounding],
        distilled_via: "tools::computer_use::GuiGrounding (视觉方法参考)",
    },
    UpstreamRepo {
        name: "Ros_Qt5_Gui_App",
        category: RepoCategory::ComputerUse,
        primary_language: "C/C++",
        license: "GPL-2.0",
        capabilities: &[CapabilityTag::GuiGrounding],
        distilled_via: "tools::computer_use::GuiGrounding (Qt GUI 定位参考)",
    },
    UpstreamRepo {
        name: "Digital-Forensics-Guide",
        category: RepoCategory::ComputerUse,
        primary_language: "Python",
        license: "(未声明)",
        capabilities: &[CapabilityTag::ScreenPerception],
        distilled_via: "tools::computer_use::ScreenPerception (参考)",
    },
    UpstreamRepo {
        name: "awesome-computer-use",
        category: RepoCategory::ComputerUse,
        primary_language: "(none)",
        license: "(未声明)",
        capabilities: &[CapabilityTag::ScreenPerception],
        distilled_via: "tools::computer_use (catalog/参考)",
    },
    UpstreamRepo {
        name: "Guided-Attention-Inference-Network",
        category: RepoCategory::ComputerUse,
        primary_language: "Python",
        license: "(未声明)",
        capabilities: &[
            CapabilityTag::GuiGrounding,
            CapabilityTag::VisionModel,
            CapabilityTag::ScreenPerception,
        ],
        distilled_via: "tools::computer_use::GuiGrounding (注意力推断辅助 GUI grounding)",
    },
];

/// 按分类检索
pub fn by_category(cat: RepoCategory) -> impl Iterator<Item = &'static UpstreamRepo> {
    UPSTREAM_REPOS.iter().filter(move |r| r.category == cat)
}

/// 按能力标签检索
pub fn by_capability(tag: CapabilityTag) -> Vec<&'static UpstreamRepo> {
    UPSTREAM_REPOS
        .iter()
        .filter(|r| r.capabilities.contains(&tag))
        .collect()
}

// ───────────────────────────────────────────────────────────────────────────
// 自演化闭环（Self-Evolution Loop）
//
// 借鉴 deepseek-auto-evolving-harness：把「跑评测 → 诊断薄弱面 → 变异 prompt/
// 工具 schema → 按分数门控」做成可复用、可单测的纯函数闭环。蒸馏表负责「从哪来」，
// 本闭环负责「如何自我改进」，二者解耦、互不干扰。
// ───────────────────────────────────────────────────────────────────────────

/// 自演化提案：某次变异（改 prompt / 工具 schema）前后评分。
#[derive(Debug, Clone)]
pub struct EvolutionProposal {
    /// 演化目标（如 "system_prompt" / "tool_schema:search"）
    pub target: String,
    /// 本次变异内容（诊断驱动的改写）
    pub mutation: String,
    /// 变异前评分（0.0–1.0）
    pub score_before: f64,
    /// 变异后评分（0.0–1.0）
    pub score_after: f64,
}

/// 评分门控：达到阈值才允许落地。
pub fn gate(score: f64, threshold: f64) -> bool {
    score >= threshold
}

/// 诊断：把若干薄弱面聚合成一句可读诊断（供 [`mutate_prompt`] 驱动改写）。
pub fn diagnose(aspects: &[&str]) -> String {
    if aspects.is_empty() {
        return "无明显薄弱面".to_string();
    }
    format!("诊断薄弱面: {}", aspects.join("; "))
}

/// 变异：在基线 prompt 后追加诊断驱动的改写指令。
pub fn mutate_prompt(base: &str, diagnosis: &str) -> String {
    format!("{base}\n\n[自演化变异] {diagnosis}")
}

/// 自演化门禁：只有「评分不退化 且 达到阈值」的变异才被采纳。
#[derive(Debug, Clone)]
pub struct SelfEvolveGate {
    /// 采纳阈值（评分须 ≥ 此值）
    pub threshold: f64,
}

impl Default for SelfEvolveGate {
    fn default() -> Self {
        Self { threshold: 0.6 }
    }
}

impl SelfEvolveGate {
    pub fn new(threshold: f64) -> Self {
        Self { threshold }
    }

    /// 是否采纳该提案：评分必须非负向退化，且变异后达到阈值。
    pub fn accept(&self, proposal: &EvolutionProposal) -> bool {
        proposal.score_after >= proposal.score_before && gate(proposal.score_after, self.threshold)
    }
}

/// 跑一次「评测→诊断→变异→门控」闭环：
/// 给定基线 prompt、诊断、前后评分与阈值，门禁通过则返回变异后的 prompt，否则 `None`。
pub fn run_self_evolve(
    base_prompt: &str,
    diagnosis: &str,
    score_before: f64,
    score_after: f64,
    threshold: f64,
) -> Option<String> {
    let proposal = EvolutionProposal {
        target: "system_prompt".to_string(),
        mutation: diagnosis.to_string(),
        score_before,
        score_after,
    };
    if SelfEvolveGate::new(threshold).accept(&proposal) {
        Some(mutate_prompt(base_prompt, diagnosis))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_covers_all_26_repos() {
        assert_eq!(UPSTREAM_REPOS.len(), 27, "蒸馏表须覆盖全部 27 个仓库");
    }

    #[test]
    fn test_categories_partition() {
        let h = by_category(RepoCategory::Harness).count();
        let c = by_category(RepoCategory::ComputerUse).count();
        assert_eq!(h + c, 27);
        assert_eq!(h, 17, "harness 类应为 17");
        assert_eq!(c, 10, "computer_use 类应为 10");
    }

    #[test]
    fn test_gui_grounding_sources_are_present() {
        // ScreenSpot-Pro 是 GUI grounding 主参考，必须存在
        let grounding = by_capability(CapabilityTag::GuiGrounding);
        assert!(
            grounding
                .iter()
                .any(|r| r.name == "ScreenSpot-Pro-GUI-Grounding"),
            "GUI grounding 主参考缺失"
        );
        assert!(
            grounding.iter().any(|r| r.name == "robotgo"),
            "跨平台输入主参考缺失"
        );
    }

    #[test]
    fn self_evolve_gate_accepts_improved_score() {
        let g = SelfEvolveGate::default();
        let improved = EvolutionProposal {
            target: "p".into(),
            mutation: "x".into(),
            score_before: 0.5,
            score_after: 0.8,
        };
        assert!(g.accept(&improved), "评分提升且达阈值应采纳");
        let regress = EvolutionProposal {
            target: "p".into(),
            mutation: "x".into(),
            score_before: 0.8,
            score_after: 0.5,
        };
        assert!(!g.accept(&regress), "评分退化应拒绝");
        let below = EvolutionProposal {
            target: "p".into(),
            mutation: "x".into(),
            score_before: 0.5,
            score_after: 0.55,
        };
        assert!(!g.accept(&below), "未达默认阈值 0.6 应拒绝");
    }

    #[test]
    fn self_evolve_loop_returns_mutated_prompt_when_gated() {
        let out = run_self_evolve("你是助手", "减少幻觉", 0.5, 0.85, 0.6);
        assert!(out.is_some());
        assert!(
            out.unwrap().contains("减少幻觉"),
            "门禁通过应返回含诊断的变异 prompt"
        );
        let blocked = run_self_evolve("你是助手", "减少幻觉", 0.5, 0.4, 0.6);
        assert!(blocked.is_none(), "评分退化应被门禁拦截");
    }

    #[test]
    fn diagnose_joins_aspects() {
        assert_eq!(diagnose(&["幻觉", "冗长"]), "诊断薄弱面: 幻觉; 冗长");
        assert_eq!(diagnose(&[]), "无明显薄弱面");
    }

    #[test]
    fn catalog_tests_still_present() {
        // 自演化闭环为增量新增，原有蒸馏表测试不应受影响
        assert_eq!(UPSTREAM_REPOS.len(), 27);
    }
}
