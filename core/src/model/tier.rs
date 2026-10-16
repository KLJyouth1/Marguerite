//! 三档模型分层（5c，帕累托最优）+ 会话级成本预算熔断。
//!
//! 设计：`deliverables/gstack/model-tiering-flagship-design-2026-10-16.md`
//!
//! - [`ModelTier`]：`Flagship`（旗舰·全模态·长上下文）/ `Balanced`（均衡·高智能低成本，默认）/
//!   `Turbo`（极速·强实时）。档位是**路由层概念**，不改 `ModelProfile` 的接入语义。
//! - [`TierCaps`] / [`MultimodalCaps`]：档位准入线的可执行化（上下文/多模态/推理上限/TTFT/单价/流式）。
//! - [`SessionBudget`] + [`SessionState`]：会话级思考强度锁定 + token/美元预算，
//!   持久化到 `<config_dir>/session_state.json`（与 `config.yaml` 同目录惯例）。
//!   与任务 2（会话级思考强度持久化）同生命周期、共用同一套预算口径，避免两套数据。
//!
//! 失败语义（Zero Demo / fail-closed）：
//! - 损坏 / 缺字段的 session_state.json 一律回退到**默认安全值**（不 panic、不崩溃、不静默写坏）。
//! - 预算耗尽只触发**降档建议**与熔断标记，绝不静默改变已选中的模型凭证。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::model::thinking::ThinkingEffort;

// ---------------------------------------------------------------------------
// 三档枚举与能力向量
// ---------------------------------------------------------------------------

/// 模型档位（三档帕累托前沿）。serde 小写；缺省 = Balanced（保守、低成本、稳定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// 旗舰推理档：全模态 / 1M+ 上下文 / Max 思考 / 万亿参数级
    Flagship,
    /// 均衡档：高智能低成本，办公高频与规模化（默认）
    #[default]
    Balanced,
    /// 极速档：最高产出速度，强实时交互
    Turbo,
}

impl ModelTier {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelTier::Flagship => "flagship",
            ModelTier::Balanced => "balanced",
            ModelTier::Turbo => "turbo",
        }
    }

    /// 从字符串解析。未知值保守回落到 [`ModelTier::Balanced`]（不静默落到 Turbo 烧质量，
    /// 也不冒然升到 Flagship 烧钱）。
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "flagship" | "high" | "max" => ModelTier::Flagship,
            "turbo" | "fast" | "flash" => ModelTier::Turbo,
            "balanced" | "medium" | "auto" | "" => ModelTier::Balanced,
            _ => ModelTier::Balanced,
        }
    }

    /// 档位默认思考强度（与 `thinking.rs::profile_for` 对齐）。
    pub fn default_effort(self) -> ThinkingEffort {
        match self {
            ModelTier::Flagship => ThinkingEffort::Max,
            ModelTier::Balanced => ThinkingEffort::Normal,
            ModelTier::Turbo => ThinkingEffort::Off,
        }
    }

    /// 档位准入线（选型硬指标，见设计稿 §2）。缺字段即按此保守淘汰。
    pub fn caps(self) -> TierCaps {
        match self {
            ModelTier::Flagship => TierCaps {
                context_window: 1_000_000,
                multimodal: MultimodalCaps {
                    text: true,
                    image: true,
                    audio: false,
                    video: false,
                },
                max_reasoning: ThinkingEffort::Max,
                ttft_budget_ms: 15_000,
                cost_per_m_in: 30.0,
                cost_per_m_out: 60.0,
                streaming_stable: true,
            },
            ModelTier::Balanced => TierCaps {
                context_window: 65_536,
                multimodal: MultimodalCaps {
                    text: true,
                    image: true,
                    audio: false,
                    video: false,
                },
                max_reasoning: ThinkingEffort::Deep,
                ttft_budget_ms: 3_000,
                cost_per_m_in: 1.0,
                cost_per_m_out: 3.0,
                streaming_stable: true,
            },
            ModelTier::Turbo => TierCaps {
                context_window: 16_384,
                multimodal: MultimodalCaps {
                    text: true,
                    image: false,
                    audio: false,
                    video: false,
                },
                max_reasoning: ThinkingEffort::Off,
                ttft_budget_ms: 800,
                cost_per_m_in: 0.5,
                cost_per_m_out: 0.5,
                streaming_stable: true,
            },
        }
    }
}

/// 多模态能力向量（缺字段=不具备，保守淘汰）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MultimodalCaps {
    pub text: bool,
    pub image: bool,
    pub audio: bool,
    pub video: bool,
}

impl Default for MultimodalCaps {
    fn default() -> Self {
        Self {
            text: true,
            image: false,
            audio: false,
            video: false,
        }
    }
}

impl MultimodalCaps {
    /// 全模态需求：文本+图像+音频+视频全都要（用于「全模态理解」类硬约束）。
    pub const fn full() -> Self {
        Self {
            text: true,
            image: true,
            audio: true,
            video: true,
        }
    }

    /// 是否满足**调用方显式声明的模态需求**（比档位准入线更严）。
    ///
    /// 为什么必须与 `accepts()` 并存、而不是把音视频写进档位准入线：
    /// 设计稿 §2.1 把音频/视频定为旗舰档的「加分项」而非硬线，因此 `accepts()`
    /// 不强制它们（否则会把只差音视频的强模型全挡在旗舰外）。
    /// 但「全模态理解」类请求（视频理解、语音转写、多模态科研）必须把它们
    /// **升为本次请求的硬约束**——否则会把只能读图的模型派去做视频任务，
    /// 而这类错误在响应侧表现为「模型对视频完全没反应」，极难定位。
    /// 于是判定分两层：档位准入（宽松，覆盖多数场景）+ 本次请求模态需求（严格，按需）。
    pub const fn satisfies_modalities(self, need: Self) -> bool {
        (!need.text || self.text)
            && (!need.image || self.image)
            && (!need.audio || self.audio)
            && (!need.video || self.video)
    }
}

/// 档位能力向量（准入标准的可执行化）。serde default 保证旧 models.json 零漂移加载。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TierCaps {
    pub context_window: u32,
    pub multimodal: MultimodalCaps,
    pub max_reasoning: ThinkingEffort,
    pub ttft_budget_ms: u32,
    pub cost_per_m_in: f64,
    pub cost_per_m_out: f64,
    pub streaming_stable: bool,
}

impl Default for TierCaps {
    fn default() -> Self {
        ModelTier::Balanced.caps()
    }
}

impl TierCaps {
    /// 该档案是否「够格」归入此档：实测能力不得**低于**档位准入线。
    ///
    /// 逐条对齐设计稿 §2.1-2.3 的准入标准，六项全查：
    /// 上下文 / TTFT / 输入单价 / **输出单价** / **推理强度** / **流式稳定性** / 模态。
    /// 此前只查了前四项中的「上下文、TTFT、输入单价、图像」，
    /// 漏掉的三项各自都有明确后果：
    /// - **流式稳定性**：Turbo 档的存在意义就是「不断流的实时交互」（§2.3）。
    ///   漏检会把『不支持稳定流式』的模型派去承担实时场景 —— 直接抹掉该档的全部价值。
    /// - **推理强度**：Flagship 的意义是「Max 思考的最终答案档」（§2.1）。
    ///   漏检会让只支持 Off 的模型占据旗舰位，用户以为在做深推理、实际没有。
    /// - **输出单价**：预算预估与熔断按输出 token 结算为主（§6），
    ///   只查输入价会显著低估旗舰档成本。
    ///
    /// 关于「缺字段」：`TierCaps::default()` 取 Balanced 档能力（见 `Default` 实现），
    /// 因此未声明能力的档案只可能通过 Balanced 准入，无法冒充 Flagship（1M 上下文硬线）
    /// 或 Turbo（800ms TTFT 硬线）——这与「保守淘汰」的设计意图一致。
    pub fn accepts(self, tier: ModelTier) -> bool {
        let need = tier.caps();
        self.context_window >= need.context_window
            && self.ttft_budget_ms <= need.ttft_budget_ms
            && self.cost_per_m_in <= need.cost_per_m_in
            && self.cost_per_m_out <= need.cost_per_m_out
            && self.max_reasoning >= need.max_reasoning
            && self.streaming_stable >= need.streaming_stable
            && self.multimodal.satisfies_modalities(need.multimodal)
    }

    /// 本次请求的模态需求是否被满足（用于「全模态理解」等按需硬约束）。
    /// 与 `accepts()` 的关系见 [`MultimodalCaps::satisfies_modalities`]。
    pub fn accepts_modalities(self, need: MultimodalCaps) -> bool {
        self.multimodal.satisfies_modalities(need)
    }

    /// 逐项列出**不满足**该档准入线的原因；满足则返回空 Vec。
    /// 供路由/控制中心回显「为何没选到它」（对齐设计稿 §3.1 的 `RouteDecision.why`）。
    pub fn reject_reasons(self, tier: ModelTier) -> Vec<String> {
        let need = tier.caps();
        let mut why = Vec::new();
        if self.context_window < need.context_window {
            why.push(format!("上下文 {} < 准入 {}", self.context_window, need.context_window));
        }
        if self.ttft_budget_ms > need.ttft_budget_ms {
            why.push(format!("TTFT {}ms > 准入 {}ms", self.ttft_budget_ms, need.ttft_budget_ms));
        }
        if self.cost_per_m_in > need.cost_per_m_in {
            why.push(format!("输入单价 {} > 准入 {}", self.cost_per_m_in, need.cost_per_m_in));
        }
        if self.cost_per_m_out > need.cost_per_m_out {
            why.push(format!("输出单价 {} > 准入 {}", self.cost_per_m_out, need.cost_per_m_out));
        }
        if self.max_reasoning < need.max_reasoning {
            why.push(format!(
                "推理强度 {:?} < 准入 {:?}",
                self.max_reasoning, need.max_reasoning
            ));
        }
        if !self.streaming_stable && need.streaming_stable {
            why.push("不支持稳定流式（该档要求不断流）".to_string());
        }
        let need_mm = need.multimodal;
        if need_mm.text && !self.multimodal.text {
            why.push("缺少文本能力".to_string());
        }
        if need_mm.image && !self.multimodal.image {
            why.push("缺少图像能力".to_string());
        }
        if need_mm.audio && !self.multimodal.audio {
            why.push("缺少音频能力".to_string());
        }
        if need_mm.video && !self.multimodal.video {
            why.push("缺少视频能力".to_string());
        }
        why
    }
}

// ---------------------------------------------------------------------------
// M2：档位准入过滤（把 `accepts()` 真正接进选路）
// ---------------------------------------------------------------------------

/// 一个档案在某档位下的准入结论。
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    /// 能力已显式声明且达标 → 可归入该档
    Admitted,
    /// 能力已显式声明但不达标 → 拒绝，附原因
    Rejected(Vec<String>),
    /// **能力未声明**：caps 只是缺省填充，不能当作实测能力。
    ///
    /// 为什么必须单独一类（这是一个会出生产事故的坑）：
    /// `seed_models.json` 只标了 `tier`、没写 `tier_caps`，于是全部种子档案的 caps
    /// 都是 `TierCaps::default()`（＝均衡档能力）。若把这种"缺省填充"当成实测能力
    /// 去卡旗舰档的 1M 上下文硬线，**旗舰档候选链会瞬间变空** —— 路由直接无模型可用。
    /// 所以缺省填充必须与"实测不达标"区分开：前者信任其档位声明（并入 `unverified`
    /// 供上层提示），后者才真正拒绝。
    Unverified,
}

/// 档位候选链的解析结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TierChain {
    /// 通过准入、按链序排列的 id
    pub candidates: Vec<String>,
    /// 被准入线淘汰的 (id, 原因)
    pub rejected: Vec<(String, Vec<String>)>,
    /// 能力未声明、按档位声明放行但未经验证的 id
    pub unverified: Vec<String>,
    /// 链中出现、但档案表里不存在的 id（数据漂移，必须可见、不得当作可用）
    pub missing: Vec<String>,
}

/// 判断某档案在某档位下的准入结论。
///
/// `caps_declared` 必须由**显式标记**给出，不能靠比较 `tier_caps == TierCaps::default()`
/// 来推断 —— 一个恰好把 caps 填成均衡档数值的档案与"未填写"在数值上无法区分，
/// 用数值反推会在将来某次数据调整时静默误判。
pub fn judge(caps: TierCaps, caps_declared: bool, tier: ModelTier) -> Admission {
    if !caps_declared {
        return Admission::Unverified;
    }
    let reasons = caps.reject_reasons(tier);
    if reasons.is_empty() {
        Admission::Admitted
    } else {
        Admission::Rejected(reasons)
    }
}

/// 按档位准入过滤一条有序候选链（M2 核心）。
///
/// `lookup` 把 id 映射为 (caps, 是否已显式声明)；返回 None 表示该 id 不在档案表里。
pub fn filter_chain(chain: &[String], tier: ModelTier,
                    lookup: impl Fn(&str) -> Option<(TierCaps, bool)>) -> TierChain {
    let mut out = TierChain::default();
    for id in chain {
        match lookup(id) {
            None => out.missing.push(id.clone()),
            Some((caps, declared)) => match judge(caps, declared, tier) {
                Admission::Admitted => out.candidates.push(id.clone()),
                Admission::Unverified => {
                    out.unverified.push(id.clone());
                    out.candidates.push(id.clone());   // 信任档位声明，但标记未验证
                }
                Admission::Rejected(why) => out.rejected.push((id.clone(), why)),
            },
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 会话预算（成本熔断；任务 2 与任务 3 共用同一口径）
// ---------------------------------------------------------------------------

/// 预算剩余状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetState {
    /// 正常：剩余 ≥20%
    Healthy,
    /// 吃紧：剩余 <20% → 档内自动切便宜档（§3.1 ⑤）
    Degraded,
    /// 紧急：剩余 <8% → 非 high_value 请求降 Turbo
    Emergency,
    /// 耗尽：剩余 ≤0 → 拒绝新请求，提示换更低档/提预算/本地模型
    Depleted,
}

impl BudgetState {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetState::Healthy => "healthy",
            BudgetState::Degraded => "degraded",
            BudgetState::Emergency => "emergency",
            BudgetState::Depleted => "depleted",
        }
    }
}

/// 会话预算对象（设计稿 §6）。持久化到 session_state.json。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionBudget {
    /// token 上限（0 = 不按 token 熔断，仅记量）
    pub tokens_cap: u64,
    /// 美元上限（0 = 不按美元熔断，仅记量）
    pub usd_cap: f64,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub usd_used: f64,
}

impl Default for SessionBudget {
    /// 默认安全预算：200k token / $2.0——显式、温和、不会一上来就熔断。
    fn default() -> Self {
        Self {
            tokens_cap: 200_000,
            usd_cap: 2.0,
            tokens_used: 0,
            usd_used: 0.0,
        }
    }
}

impl SessionBudget {
    /// 预算剩余比例（0..1）。两个口径都设了上限时取更紧者；都没设时返回 1.0（不熔断）。
    pub fn remaining_ratio(&self) -> f64 {
        let token_ratio = if self.tokens_cap == 0 {
            1.0
        } else {
            1.0 - (self.tokens_used as f64 / self.tokens_cap as f64)
        };
        let usd_ratio = if self.usd_cap <= 0.0 {
            1.0
        } else {
            1.0 - (self.usd_used / self.usd_cap)
        };
        token_ratio.min(usd_ratio).clamp(0.0, 1.0)
    }

    pub fn state(&self) -> BudgetState {
        let r = self.remaining_ratio();
        if r <= 0.0 {
            BudgetState::Depleted
        } else if r < 0.08 {
            BudgetState::Emergency
        } else if r < 0.20 {
            BudgetState::Degraded
        } else {
            BudgetState::Healthy
        }
    }

    /// 记录一次真实用量（结算，差额滚动）。饱和累加，不允许负用量。
    pub fn record_usage(&mut self, tokens: u64, usd: f64) {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
        self.usd_used = (self.usd_used + usd).max(0.0);
    }

    /// 由「剩余比例」给出降档建议（供只有比例、拿不到预算对象的场景复用同一套阈值）。
    ///
    /// 为什么抽出来：档位判定（[`resolve_tier`]）只拿到一个比例，若在那里另抄一份
    /// 阈值，将来调阈值必然两边不一致——一边降档一边不降，且极难定位。
    pub fn downgrade_for_ratio(current: ModelTier, remaining: f64) -> ModelTier {
        let r = remaining.clamp(0.0, 1.0);
        if r <= 0.0 || r < 0.08 {
            ModelTier::Turbo
        } else if r < 0.20 {
            if current == ModelTier::Flagship {
                ModelTier::Balanced
            } else {
                current
            }
        } else {
            current
        }
    }

    /// 预算吃紧时建议降到哪一档：Emergency→Turbo，Degraded→Balanced，否则保持。
    /// `&self` 即可（只读 state()/remaining_ratio()），避免调用方被迫 move/copy budget。
    pub fn suggested_downgrade(&self, current: ModelTier) -> ModelTier {
        Self::downgrade_for_ratio(current, self.remaining_ratio())
    }
}

// ---------------------------------------------------------------------------
// M3：生效档自动判定（由模式 / 任务类型 / 延迟 / 预算算出，不再等调用方手填）
// ---------------------------------------------------------------------------

/// 档位判定输入（请求画像中与选档相关的子集）。
#[derive(Debug, Clone)]
pub struct TierRequest {
    /// 模式提示（`plan`/`agent`/`bloom`/`ensemble`/`omni_transcend`），None = 未指定
    pub mode_hint: Option<String>,
    /// 任务类型（见 `thinking.rs::TaskKind`）
    pub kind: Option<crate::model::thinking::TaskKind>,
    /// 复杂度信号（与 `kind` 组合使用，如 复杂代码任务 → 旗舰档）
    pub complex: bool,
    /// 端到端首字延迟要求（ms）。设了且严于极速档准入线 → 强实时优先。
    pub latency_slo_ms: Option<u32>,
    /// 会话剩余预算比例（0..1）
    pub budget_left: f64,
    /// 用户锁定档（会话级）
    pub locked: Option<ModelTier>,
}

impl Default for TierRequest {
    /// `budget_left = 1.0`（表达"无预算约束"）。
    ///
    /// 不能 `derive(Default)`：那会得到 0.0，语义是"预算耗尽"，
    /// 会把每一个请求都误降为极速档——一个只在生产才暴露的静默降质。
    fn default() -> Self {
        Self {
            mode_hint: None,
            kind: None,
            complex: false,
            latency_slo_ms: None,
            budget_left: 1.0,
            locked: None,
        }
    }
}

/// 档位判定结果。`why` 是理由链，供控制中心 / TUI / CLI 回显"为何选这一档"。
#[derive(Debug, Clone, PartialEq)]
pub struct TierDecision {
    pub tier: ModelTier,
    pub why: Vec<String>,
}

/// 由请求画像算出**生效档**（设计稿 §3.1 的 ①③④ 与 §6 预算降档）。
///
/// 优先级（高 → 低），每步都写入 `why` 以便回显：
/// 1. **用户锁定档**：尊重用户选择，但预算仍可降档（**只降不升**，绝不静默改写选择）。
/// 2. **强实时**：`latency_slo_ms` 严于极速档准入线 → 极速档（延迟是硬诉求，压过质量诉求）。
/// 3. **模式**：`omni_transcend`/`bloom`/`ensemble`/`plan` → 旗舰；`agent` → 均衡。
/// 4. **任务类型**：`Reasoning`/`Long` → 旗舰；`Code` → 复杂则旗舰否则均衡；
///    `Tool` → 均衡；`Chat` → 极速。
/// 5. **兜底**：均衡档（设计稿定义的默认安全档）。
///
/// 最后统一施加预算降档，保证任何路径都"不因预算而升级"。
pub fn resolve_tier(req: &TierRequest) -> TierDecision {
    let mut why: Vec<String> = Vec::new();
    let turbo_line = ModelTier::Turbo.caps().ttft_budget_ms;

    let mut tier = if let Some(lock) = req.locked {
        why.push(format!("用户锁定 {lock:?}"));
        lock
    } else if let Some(slo) = req.latency_slo_ms.filter(|s| *s <= turbo_line) {
        why.push(format!("延迟要求 {slo}ms ≤ 极速档准入 {turbo_line}ms"));
        ModelTier::Turbo
    } else {
        by_mode_and_kind(req, &mut why)
    };

    let after = SessionBudget::downgrade_for_ratio(tier, req.budget_left);
    if after != tier {
        why.push(format!(
            "预算剩余 {:.0}% → 由 {:?} 降为 {:?}",
            req.budget_left * 100.0,
            tier,
            after
        ));
        tier = after;
    }
    TierDecision { tier, why }
}

/// 模式优先、任务类型次之（不含锁定与延迟；优先级说明见 [`resolve_tier`]）。
fn by_mode_and_kind(req: &TierRequest, why: &mut Vec<String>) -> ModelTier {
    if let Some(mode) = req.mode_hint.as_deref() {
        let m = mode.trim().to_ascii_lowercase();
        let by_mode = match m.as_str() {
            "omni_transcend" | "omni" | "bloom" | "ensemble" | "plan" => Some(ModelTier::Flagship),
            "agent" => Some(ModelTier::Balanced),
            _ => None,
        };
        if let Some(t) = by_mode {
            why.push(format!("模式 {m} → {t:?}"));
            return t;
        }
    }
    if let Some(k) = req.kind {
        use crate::model::thinking::TaskKind;
        let t = match k {
            TaskKind::Reasoning | TaskKind::Long => ModelTier::Flagship,
            TaskKind::Code => {
                if req.complex {
                    ModelTier::Flagship
                } else {
                    ModelTier::Balanced
                }
            }
            TaskKind::Tool => ModelTier::Balanced,
            TaskKind::Chat => ModelTier::Turbo,
        };
        why.push(format!(
            "任务类型 {k:?}{} → {t:?}",
            if req.complex { "(复杂)" } else { "" }
        ));
        return t;
    }
    why.push("无模式/任务信号 → 默认均衡档".to_string());
    ModelTier::Balanced
}

// ---------------------------------------------------------------------------
// 会话态：思考强度锁定 + 档位锁定 + 预算（任务 2 主体）
// ---------------------------------------------------------------------------

/// 会话级、可持久化的工作台状态。重启后恢复「上次开到旗舰档 + 深想强度 + 预算」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    /// 用户锁定的档位（None=auto）。
    pub locked_tier: Option<ModelTier>,
    /// 会话级思考强度（Off/Normal/Deep/Max）。
    pub thinking_effort: ThinkingEffort,
    /// 会话预算。
    pub budget: SessionBudget,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            locked_tier: None,
            thinking_effort: ThinkingEffort::Normal,
            budget: SessionBudget::default(),
        }
    }
}

impl SessionState {
    /// 持久化路径：`<config_dir>/session_state.json`。
    pub fn path() -> PathBuf {
        crate::persistence::config::Config::config_dir().join("session_state.json")
    }

    /// 加载会话态。文件缺失或损坏 → **fail-closed 回退默认安全值**
    /// （不 panic、不把坏数据透传给路由）。
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// 路径注入式加载（测试用，避免触碰真实 ~/.marguerite）。
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<Self>(&raw) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("session_state.json 损坏，回退默认安全值: {e}");
                    Self::default()
                }
            },
            // 缺失 = 首启，用默认安全值即可
            Err(_) => Self::default(),
        }
    }

    /// 原子式落盘（先写 tmp 再 rename，避免半写坏文件）。
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_vec_pretty(self)?;
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, &raw)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 当前生效档位：用户锁定优先；否则按预算状态动态建议（不强行改写用户选择，
    /// 只给出建议档，供 UI/路由展示降级原因）。
    pub fn effective_tier(&self) -> ModelTier {
        match self.locked_tier {
            Some(t) => self.budget.suggested_downgrade(t),
            None => self.budget.suggested_downgrade(ModelTier::Balanced),
        }
    }

    /// 用**会话持久态**（锁定档 + 预算）补全**本次请求信号**，算出生效档（M3 接线）。
    ///
    /// 为什么必须有这个桥：`resolve_tier` 需要 `locked` 与 `budget_left`，这两者属于
    /// 会话态、跨请求持久；而 `mode_hint`/`kind`/`latency_slo_ms` 只存在于**本次请求**。
    /// 二者缺一不可：
    /// - 只读会话态 → 退化成 `effective_tier()`，即"永远均衡档"，
    ///   既不知道本次是长程科研（应旗舰）还是桌宠闲聊（应极速）；
    /// - 只读请求信号 → 丢掉用户的档位锁定与预算熔断。
    ///
    /// 语义：**会话持久态优先**——`locked` 与 `budget_left` 一律以 `self` 为准，
    /// 传入 `req` 里的同名字段被忽略。这样调用方无需知道用户锁了什么档、预算还剩多少，
    /// 也就不会因为漏传而静默绕过熔断。
    pub fn resolve_tier_for(&self, mut req: TierRequest) -> TierDecision {
        req.locked = self.locked_tier;
        req.budget_left = self.budget.remaining_ratio();
        resolve_tier(&req)
    }
}

// ---------------------------------------------------------------------------
// 档内择优规则（数据驱动；与 routing_rules.json 并存、兼容）
// ---------------------------------------------------------------------------

/// 档内 fallback 链 + 预算阈值（`tier_rules.json` 的内存形态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TierRules {
    pub default_tier: String,
    pub degrade_threshold: f64,
    pub emergency_threshold: f64,
    /// tier 名 → 档内有序 profile_id 链（跨云 fallback 数据源）。
    pub chains: std::collections::HashMap<String, Vec<String>>,
}

impl Default for TierRules {
    /// 空规则集（serde `#[serde(default)]` 反序列化缺字段时回退用）。
    /// **不可**在这里调 `from_embedded()`：那会让 serde 反序列化 → default →
    /// from_embedded → serde 反序列化 无限递归（栈溢出）。嵌入式规则走 `from_embedded()`。
    fn default() -> Self {
        Self {
            default_tier: "balanced".to_string(),
            degrade_threshold: 0.2,
            emergency_threshold: 0.08,
            chains: std::collections::HashMap::new(),
        }
    }
}

impl TierRules {
    /// 编译期内嵌 `tier_rules.json`（数据驱动，改数据文件即可调档内链）。
    pub fn from_embedded() -> Self {
        serde_json::from_str(include_str!("tier_rules.json"))
            .expect("内嵌 tier_rules.json 损坏")
    }

    /// 取某档的档内候选链（返回 profile_id 列表）。
    pub fn chain_for(&self, tier: ModelTier) -> &[String] {
        self.chains.get(tier.as_str()).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "mgt_tier_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn tier_parse_is_fail_closed_to_balanced() {
        assert_eq!(ModelTier::parse("flagship"), ModelTier::Flagship);
        assert_eq!(ModelTier::parse("TURBO"), ModelTier::Turbo);
        assert_eq!(ModelTier::parse("balanced"), ModelTier::Balanced);
        assert_eq!(ModelTier::parse(""), ModelTier::Balanced);
        assert_eq!(ModelTier::parse("something-weird"), ModelTier::Balanced);
    }

    #[test]
    fn tier_default_effort_matches_design() {
        assert_eq!(
            ModelTier::Flagship.default_effort(),
            ThinkingEffort::Max
        );
        assert_eq!(ModelTier::Balanced.default_effort(), ThinkingEffort::Normal);
        assert_eq!(ModelTier::Turbo.default_effort(), ThinkingEffort::Off);
    }

    #[test]
    fn flagship_caps_require_1m_context() {
        assert_eq!(ModelTier::Flagship.caps().context_window, 1_000_000);
        assert!(ModelTier::Flagship.caps().multimodal.image);
        // Turbo 准入线最严的是延迟（ttft 上限最小）
        assert!(
            ModelTier::Turbo.caps().ttft_budget_ms < ModelTier::Balanced.caps().ttft_budget_ms
        );
    }

    #[test]
    fn budget_state_thresholds() {
        let mut b = SessionBudget::default();
        assert_eq!(b.state(), BudgetState::Healthy);
        // 用掉 90% → remaining 10% → Degraded
        b.tokens_used = (b.tokens_cap as f64 * 0.90) as u64;
        assert_eq!(b.state(), BudgetState::Degraded);
        // 用掉 95% → remaining 5% → Emergency
        b.tokens_used = (b.tokens_cap as f64 * 0.95) as u64;
        assert_eq!(b.state(), BudgetState::Emergency);
        // 用满 → Depleted
        b.tokens_used = b.tokens_cap;
        assert_eq!(b.state(), BudgetState::Depleted);
        assert_eq!(b.remaining_ratio(), 0.0);
    }

    #[test]
    fn emergency_downgrades_flagship_to_turbo() {
        let mut b = SessionBudget::default();
        b.tokens_used = (b.tokens_cap as f64 * 0.97) as u64;
        assert_eq!(b.suggested_downgrade(ModelTier::Flagship), ModelTier::Turbo);
        // Degraded: Flagship→Balanced，但 Balanced/Turbo 保持
        let mut b2 = SessionBudget::default();
        b2.tokens_used = (b2.tokens_cap as f64 * 0.85) as u64;
        assert_eq!(b2.suggested_downgrade(ModelTier::Flagship), ModelTier::Balanced);
        assert_eq!(b2.suggested_downgrade(ModelTier::Turbo), ModelTier::Turbo);
    }

    #[test]
    fn session_state_roundtrip_persists_tier_effort_budget() {
        let dir = tmp("persist");
        let path = dir.join("session_state.json");
        let mut s = SessionState {
            locked_tier: Some(ModelTier::Flagship),
            thinking_effort: ThinkingEffort::Max,
            ..SessionState::default()
        };
        s.budget.record_usage(50_000, 0.5);
        s.save_to(&path).unwrap();

        let loaded = SessionState::load_from(&path);
        assert_eq!(loaded.locked_tier, Some(ModelTier::Flagship));
        assert_eq!(loaded.thinking_effort, ThinkingEffort::Max);
        assert_eq!(loaded.budget.tokens_used, 50_000);
        assert!((loaded.budget.usd_used - 0.5).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_returns_safe_defaults() {
        let dir = tmp("missing");
        let s = SessionState::load_from(&dir.join("nope.json"));
        assert_eq!(s.locked_tier, None);
        assert_eq!(s.thinking_effort, ThinkingEffort::Normal);
        assert_eq!(s.budget.state(), BudgetState::Healthy);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_file_fails_closed_to_defaults() {
        let dir = tmp("corrupt");
        let path = dir.join("session_state.json");
        std::fs::write(&path, "{ this is not valid json ").unwrap();
        let s = SessionState::load_from(&path);
        // 损坏不得 panic，必须回退默认安全值
        assert_eq!(s.locked_tier, None);
        assert_eq!(s.thinking_effort, ThinkingEffort::Normal);
        assert_eq!(s.budget.tokens_cap, SessionBudget::default().tokens_cap);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_file_defaults_missing_fields() {
        // 旧版本可能只写了部分字段：serde default 必须补齐，不丢、不炸。
        let dir = tmp("partial");
        let path = dir.join("session_state.json");
        // ThinkingEffort 无 serde rename，序列化为 PascalCase（"Deep"）。
        std::fs::write(&path, r#"{ "thinking_effort": "Deep" }"#).unwrap();
        let s = SessionState::load_from(&path);
        assert_eq!(s.thinking_effort, ThinkingEffort::Deep);
        assert_eq!(s.locked_tier, None);
        assert!(s.budget.tokens_cap > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_tier_rules_load() {
        let rules = TierRules::from_embedded();
        assert!(!rules.chain_for(ModelTier::Flagship).is_empty());
        assert!(!rules.chain_for(ModelTier::Balanced).is_empty());
        assert!(!rules.chain_for(ModelTier::Turbo).is_empty());
        // 未知档返回空链而非 panic
        assert!(!rules.chains.contains_key("nope"));
    }

    #[test]
    fn accepts_enforces_context_floor() {
        // 一个只配 16k 上下文、5s TTFT 的档案，远达不到 Flagship 的 1M 准入线。
        let small = TierCaps {
            context_window: 16_384,
            ttft_budget_ms: 5_000,
            cost_per_m_in: 0.5,
            multimodal: MultimodalCaps::default(),
            ..TierCaps::default()
        };
        assert!(!small.accepts(ModelTier::Flagship));
        // 未声明能力的档案取 Balanced 能力（Default 实现），因此通不过 Flagship 的
        // 1M 上下文硬线，也无法冒充 Turbo 的 800ms TTFT 硬线 —— 保守淘汰。
        let bare = TierCaps::default();
        assert!(!bare.accepts(ModelTier::Flagship));
        assert!(!bare.accepts(ModelTier::Turbo));
        assert!(bare.accepts(ModelTier::Balanced));
    }

    /// 构造一个「刚好满足某档」的档案，便于逐项打靶：把模态补满、流式设为稳定，
    /// 之后各测试再逐项削弱它，验证每一项都真的被校验。
    fn caps_like(tier: ModelTier) -> TierCaps {
        let mut c = tier.caps();
        c.multimodal = MultimodalCaps {
            text: true,
            image: tier.caps().multimodal.image,
            audio: true,
            video: true,
        };
        c.streaming_stable = true;
        c
    }

    #[test]
    fn baseline_caps_pass_their_own_tier() {
        // 反向守卫：收紧 accepts() 不能把「本来就够格」的档案误杀。
        for t in [ModelTier::Flagship, ModelTier::Balanced, ModelTier::Turbo] {
            assert!(caps_like(t).accepts(t), "{t:?} 的基准档案应通过自身准入");
        }
    }

    #[test]
    fn rejects_model_that_cannot_stream_stably() {
        // 覆盖本轮修掉的漏检：Turbo 存在意义就是「不断流的实时交互」（设计稿 §2.3）。
        // 漏检会把不支持稳定流式的模型派去做强实时场景，直接抹掉该档价值。
        let mut c = caps_like(ModelTier::Turbo);
        c.streaming_stable = false;
        assert!(!c.accepts(ModelTier::Turbo));
    }

    #[test]
    fn rejects_flagship_claim_without_max_reasoning() {
        // Flagship 的意义是「Max 思考的最终答案档」；漏检会让只支持中等思考的模型
        // 占据旗舰位 —— 用户以为在做深推理，实际没有。
        let mut c = caps_like(ModelTier::Flagship);
        c.max_reasoning = ThinkingEffort::Deep;
        assert!(!c.accepts(ModelTier::Flagship));
    }

    #[test]
    fn rejects_when_output_price_exceeds_tier_ceiling() {
        // 预算预扣/结算以输出 token 为主（设计稿 §6），只查输入价会低估成本。
        let mut c = caps_like(ModelTier::Balanced);
        c.cost_per_m_out = 99.0;
        assert!(!c.accepts(ModelTier::Balanced));
    }

    #[test]
    fn rejects_textless_profile_from_every_tier() {
        // 此前只查 image，导致 text=false / image=true 的畸形档案也能通过准入。
        for t in [ModelTier::Flagship, ModelTier::Balanced, ModelTier::Turbo] {
            let mut c = caps_like(t);
            c.multimodal.text = false;
            c.multimodal.image = true;
            assert!(!c.accepts(t), "{t:?} 不应接受无文本能力的档案");
        }
    }

    #[test]
    fn multimodal_needs_are_enforced_per_request() {
        // 用户要求「全模态理解」时，音/视频必须从加分项升为硬约束：
        // 否则会把只能读图的模型派去做视频任务，且响应侧表现为「对视频毫无反应」，极难定位。
        let mut c = caps_like(ModelTier::Flagship);
        c.multimodal.audio = false;
        c.multimodal.video = false;
        // 档位准入不强制音视频（设计稿 §2.1 定为加分项）→ 仍可归入旗舰档
        assert!(c.accepts(ModelTier::Flagship));
        // 但显式声明全模态需求的请求必须被拦下
        assert!(!c.accepts_modalities(MultimodalCaps::full()));
        // 只要求文本/图像的需求仍应通过
        assert!(c.accepts_modalities(MultimodalCaps::default()));
    }

    #[test]
    fn thinking_effort_ordering_is_semantic() {
        // accepts() 依赖 Off < Normal < Deep < Max 这一顺序。
        // 一旦有人重排 thinking.rs 的变体，准入判断会静默失真 —— 此测试负责立刻失败。
        assert!(ThinkingEffort::Off < ThinkingEffort::Normal);
        assert!(ThinkingEffort::Normal < ThinkingEffort::Deep);
        assert!(ThinkingEffort::Deep < ThinkingEffort::Max);
    }

    // ------------------------------------------------------------ M2 准入过滤
    #[test]
    fn unverified_caps_are_not_treated_as_verified_capability() {
        // 这是会出生产事故的坑：seed_models.json 只标 tier、未写 tier_caps，
        // 全部种子档案的 caps 都是 TierCaps::default()（均衡档能力）。
        // 若把这种"缺省填充"当成实测能力去卡旗舰档的 1M 上下文硬线，
        // 旗舰候选链会瞬间变空 → 路由无模型可用。
        let def = TierCaps::default();
        assert_eq!(judge(def, false, ModelTier::Flagship), Admission::Unverified);
        // 一旦作者**显式声明**了 caps，就必须按实测能力严格卡线
        assert!(matches!(
            judge(def, true, ModelTier::Flagship),
            Admission::Rejected(_)
        ));
        assert_eq!(judge(def, true, ModelTier::Balanced), Admission::Admitted);
    }

    #[test]
    fn filter_chain_splits_four_buckets() {
        let chain = vec![
            "ok".to_string(),
            "weak".to_string(),
            "undeclared".to_string(),
            "ghost".to_string(),
        ];
        let good = caps_like(ModelTier::Flagship);
        let weak = TierCaps {
            context_window: 1_000,
            ..caps_like(ModelTier::Flagship)
        };
        let out = filter_chain(&chain, ModelTier::Flagship, |id| match id {
            "ok" => Some((good, true)),
            "weak" => Some((weak, true)),
            "undeclared" => Some((TierCaps::default(), false)),
            _ => None, // 档案表里没有 → 数据漂移
        });
        // 通过准入 + 放行的未声明项，按链序保留
        assert_eq!(out.candidates, vec!["ok", "undeclared"]);
        assert_eq!(out.unverified, vec!["undeclared"]);
        assert_eq!(out.missing, vec!["ghost"]);
        assert_eq!(out.rejected.len(), 1);
        assert_eq!(out.rejected[0].0, "weak");
        assert!(
            out.rejected[0].1.iter().any(|r| r.contains("上下文")),
            "拒绝原因应逐项可读：{:?}",
            out.rejected[0].1
        );
    }

    #[test]
    fn reject_reasons_name_the_failing_criterion() {
        let mut c = caps_like(ModelTier::Turbo);
        c.streaming_stable = false;
        let why = c.reject_reasons(ModelTier::Turbo);
        assert!(why.iter().any(|w| w.contains("流式")), "应指出流式不达标：{why:?}");

        let mut c2 = caps_like(ModelTier::Flagship);
        c2.max_reasoning = ThinkingEffort::Off;
        let why2 = c2.reject_reasons(ModelTier::Flagship);
        assert!(why2.iter().any(|w| w.contains("推理强度")), "应指出推理强度不足：{why2:?}");

        // 达标时不得产出任何原因（否则会在 UI 上给出误导性的拒绝理由）
        assert!(caps_like(ModelTier::Balanced)
            .reject_reasons(ModelTier::Balanced)
            .is_empty());
    }

    // ------------------------------------------------------------ M3 生效档判定
    fn req() -> TierRequest {
        TierRequest::default()
    }

    #[test]
    fn tier_request_default_means_no_budget_constraint() {
        // 若 budget_left 默认成 0.0（derive 的行为），语义是"预算耗尽"，
        // 会把每个请求都静默降为极速档 —— 这个坑只在生产暴露，必须锁死。
        assert_eq!(req().budget_left, 1.0);
        assert_eq!(resolve_tier(&req()).tier, ModelTier::Balanced);
    }

    #[test]
    fn mode_hint_drives_tier_before_task_kind() {
        // 模式优先于任务类型：omni_transcend 即便配 Chat 也要上旗舰。
        let r = TierRequest {
            mode_hint: Some("omni_transcend".into()),
            kind: Some(crate::model::thinking::TaskKind::Chat),
            ..req()
        };
        assert_eq!(resolve_tier(&r).tier, ModelTier::Flagship);
        // agent 模式 → 均衡（即便任务是复杂代码，模式优先）
        let r2 = TierRequest {
            mode_hint: Some("agent".into()),
            kind: Some(crate::model::thinking::TaskKind::Reasoning),
            ..req()
        };
        assert_eq!(resolve_tier(&r2).tier, ModelTier::Balanced);
    }

    #[test]
    fn task_kind_maps_to_expected_tiers() {
        use crate::model::thinking::TaskKind;
        let t = |kind, complex| {
            resolve_tier(&TierRequest { kind: Some(kind), complex, ..req() }).tier
        };
        assert_eq!(t(TaskKind::Reasoning, false), ModelTier::Flagship);
        assert_eq!(t(TaskKind::Long, false), ModelTier::Flagship);
        assert_eq!(t(TaskKind::Code, true), ModelTier::Flagship);
        assert_eq!(t(TaskKind::Code, false), ModelTier::Balanced);
        assert_eq!(t(TaskKind::Tool, false), ModelTier::Balanced);
        assert_eq!(t(TaskKind::Chat, false), ModelTier::Turbo);
    }

    #[test]
    fn tight_latency_slo_wins_over_task_kind() {
        // 延迟是硬诉求：即便任务是长程推理，要求 500ms 首字也只能上极速档。
        let r = TierRequest {
            kind: Some(crate::model::thinking::TaskKind::Reasoning),
            latency_slo_ms: Some(500),
            ..req()
        };
        assert_eq!(resolve_tier(&r).tier, ModelTier::Turbo);
        // 而 5s 的 SLO 宽松于极速档准入线（800ms），不应触发强实时
        let r2 = TierRequest {
            kind: Some(crate::model::thinking::TaskKind::Reasoning),
            latency_slo_ms: Some(5_000),
            ..req()
        };
        assert_eq!(resolve_tier(&r2).tier, ModelTier::Flagship);
    }

    #[test]
    fn locked_tier_is_respected_but_never_upgraded_by_budget() {
        // 锁定优先：即便任务只是闲聊，锁了旗舰就上旗舰。
        let r = TierRequest {
            locked: Some(ModelTier::Flagship),
            kind: Some(crate::model::thinking::TaskKind::Chat),
            ..req()
        };
        assert_eq!(resolve_tier(&r).tier, ModelTier::Flagship);
        // 但预算吃紧仍可降档（只降不升）——绝不静默改写用户选择去升档
        let r2 = TierRequest {
            locked: Some(ModelTier::Flagship),
            budget_left: 0.05,
            ..req()
        };
        assert_eq!(resolve_tier(&r2).tier, ModelTier::Turbo);
    }

    #[test]
    fn budget_only_ever_downgrades() {
        // 预算充裕时不得把闲聊从极速档"升级"成旗舰
        let r = TierRequest {
            kind: Some(crate::model::thinking::TaskKind::Chat),
            budget_left: 1.0,
            ..req()
        };
        assert_eq!(resolve_tier(&r).tier, ModelTier::Turbo);
        // Degraded（<20%）：旗舰→均衡，但极速/均衡保持
        let degraded = |t: ModelTier| SessionBudget::downgrade_for_ratio(t, 0.10);
        assert_eq!(degraded(ModelTier::Flagship), ModelTier::Balanced);
        assert_eq!(degraded(ModelTier::Balanced), ModelTier::Balanced);
        assert_eq!(degraded(ModelTier::Turbo), ModelTier::Turbo);
    }

    #[test]
    fn downgrade_for_ratio_matches_budget_object_thresholds() {
        // 阈值单一来源守卫：比例版与预算对象版必须给出一致结论。
        for used_ratio in [0.0, 0.5, 0.79, 0.85, 0.93, 0.99, 1.0] {
            let mut b = SessionBudget::default();
            b.tokens_used = (b.tokens_cap as f64 * used_ratio) as u64;
            let remaining = b.remaining_ratio();
            for tier in [ModelTier::Flagship, ModelTier::Balanced, ModelTier::Turbo] {
                assert_eq!(
                    b.suggested_downgrade(tier),
                    SessionBudget::downgrade_for_ratio(tier, remaining),
                    "used={used_ratio} tier={tier:?} 两条路径结论必须一致"
                );
            }
        }
    }

    #[test]
    fn session_state_is_authoritative_for_lock_and_budget() {
        // 会话持久态优先：调用方即便传了 locked/budget_left，也必须被会话态覆盖，
        // 否则调用方漏传或乱传就会静默绕过用户的档位锁定与预算熔断。
        let s = SessionState {
            locked_tier: Some(ModelTier::Turbo),
            ..SessionState::default()
        };
        let r = TierRequest {
            locked: Some(ModelTier::Flagship),   // 应被忽略
            budget_left: 0.0,                    // 应被忽略
            mode_hint: Some("omni_transcend".into()),
            ..TierRequest::default()
        };
        let d = s.resolve_tier_for(r);
        // 会话锁定 Turbo 生效（而非被请求信号升成旗舰）
        assert_eq!(d.tier, ModelTier::Turbo);
    }

    #[test]
    fn request_signals_reach_the_decision_when_no_lock() {
        // 无锁定时，请求信号必须真正影响生效档；否则档位分层形同虚设
        //（旧实现只读会话态 → 每个未锁定请求都落到均衡档）。
        let s = SessionState::default();
        let omni = s.resolve_tier_for(TierRequest {
            mode_hint: Some("omni_transcend".into()),
            ..TierRequest::default()
        });
        assert_eq!(omni.tier, ModelTier::Flagship);
        let chat = s.resolve_tier_for(TierRequest {
            kind: Some(crate::model::thinking::TaskKind::Chat),
            ..TierRequest::default()
        });
        assert_eq!(chat.tier, ModelTier::Turbo);
        // 预算耗尽时仍可降档（会话态提供 budget_left）
        let mut tight = SessionState::default();
        tight.budget.tokens_used = tight.budget.tokens_cap;
        let d = tight.resolve_tier_for(TierRequest {
            mode_hint: Some("omni_transcend".into()),
            ..TierRequest::default()
        });
        assert_eq!(d.tier, ModelTier::Turbo);
    }

    #[test]
    fn decision_reasons_are_recorded_for_display() {
        // 理由链是"可解释选档"的载体（设计稿 §3.1 ⑥），不得为空。
        let r = TierRequest {
            mode_hint: Some("bloom".into()),
            budget_left: 0.05,
            ..req()
        };
        let d = resolve_tier(&r);
        assert_eq!(d.tier, ModelTier::Turbo);
        assert!(d.why.iter().any(|w| w.contains("bloom")), "{:?}", d.why);
        assert!(d.why.iter().any(|w| w.contains("预算")), "{:?}", d.why);
    }
}
