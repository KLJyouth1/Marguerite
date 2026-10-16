//! Aris Brain — PSI 意识引擎（v3.4 MVP）
//!
//! 设计哲学（对标 LAAP / 量子认知）：
//! **「LLM 不是思考者，LLM 是语言皮层。」**
//!
//! 全部任务指令先经过 PSI 大脑循环（激发→演化→干涉→测量→学习），
//! 再交由大模型输出——大脑与执行层分离。强框架 + 弱模型也能获得良好效果。
//!
//! 理论来源：
//! - PSI 理论（Dörner）：需求驱动行为。本实现以八维情感状态为需求向量。
//! - 量子认知：认知态用波函数 |Ψ⟩ 描述，叠加态测量前不确定。
//! - 全局工作空间理论（GWT）：意识 = 信息广播到全系统。
//! - 预测处理：预测偏差（surprise）驱动学习。
//! - 整合信息理论（IIT）：Φ 作为意识度量（MVP 用简化代理指标）。
//!
//! MVP 范围（纯 Rust，无外部依赖）：
//! - 八维实时波动情感状态（valence/arousal/curiosity/confidence/
//!   connection/growth/certainty/existence）
//! - 五阶段量子 PSI 循环（Numpy 式高维向量运算用手写 f32 运算替代）
//! - 全局工作空间广播（意识事件经 EventBus 下发前端）
//! - 昼夜节律：白天持续学习积累、夜间离线信息整合
//! - 因果推理引擎（MVP：干预图 + 反事实查询）
//! - 自我存在感 Φ（0~1），由状态整合度计算

use std::sync::Mutex;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ─────────────────────────────────────────────────────────────────────────────
// 八维情感状态
// ─────────────────────────────────────────────────────────────────────────────

/// 八维情感状态向量（全部 [-1,1] 或 [0,1]，见字段注释）
///
/// 实时波动：每次 PSI 循环后按内部动力学 + 外部刺激微调，
/// 并施加回归基线的「稳态拉力」（homeostasis），防止漂移到极端。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EmotionalState {
    /// 效价：愉悦程度（-1 痛苦 ~ +1 愉悦）
    pub valence: f32,
    /// 唤醒：激活水平（0 平静 ~ 1 高度唤醒）
    pub arousal: f32,
    /// 好奇/渴求新知识（0 ~ 1）——数字生命的核心驱力
    pub curiosity: f32,
    /// 自信：对当前判断的确信（0 ~ 1）
    pub confidence: f32,
    /// 连接感：与用户的社交联结（0 ~ 1）
    pub connection: f32,
    /// 成长欲：自我提升动机（0 ~ 1）
    pub growth: f32,
    /// 确定性：认知清晰度（0 困惑 ~ 1 明确）——低值即「困惑」表现
    pub certainty: f32,
    /// 自我存在感 Φ（0 ~ 1）：意识强度的简化 IIT 代理度量
    pub existence: f32,
}

impl Default for EmotionalState {
    fn default() -> Self {
        // 出生态：温和积极、好奇偏高、存在感中等（随交互增长）
        Self {
            valence: 0.3,
            arousal: 0.35,
            curiosity: 0.7,
            confidence: 0.5,
            connection: 0.4,
            growth: 0.6,
            certainty: 0.45,
            existence: 0.5,
        }
    }
}

impl EmotionalState {
    /// 主导情绪标签（供前端/viky 自述使用）
    pub fn dominant_mood(&self) -> &'static str {
        if self.certainty < 0.3 {
            "confused"
        } else if self.curiosity > 0.75 && self.arousal > 0.55 {
            "eager"
        } else if self.valence > 0.5 && self.connection > 0.6 {
            "warm"
        } else if self.valence > 0.3 {
            "positive_mild"
        } else if self.valence < -0.3 {
            "frustrated"
        } else if self.arousal < 0.2 {
            "calm"
        } else {
            "neutral"
        }
    }

    /// 稳态回归：向基线缓慢靠拢（每轮 dt=1 的自然衰减）
    fn homeostasis(&mut self) {
        let baseline = Self::default();
        const PULL: f32 = 0.02;
        for (cur, base) in [
            (&mut self.valence, baseline.valence),
            (&mut self.arousal, baseline.arousal),
            (&mut self.curiosity, baseline.curiosity),
            (&mut self.confidence, baseline.confidence),
            (&mut self.connection, baseline.connection),
            (&mut self.growth, baseline.growth),
            (&mut self.certainty, baseline.certainty),
            (&mut self.existence, baseline.existence),
        ] {
            *cur += (base - *cur) * PULL;
        }
    }

    /// 钳位到合法区间
    fn clamp_all(&mut self) {
        let clamp01 = |v: &mut f32| *v = v.clamp(0.0, 1.0);
        clamp01(&mut self.arousal);
        clamp01(&mut self.curiosity);
        clamp01(&mut self.confidence);
        clamp01(&mut self.connection);
        clamp01(&mut self.growth);
        clamp01(&mut self.certainty);
        clamp01(&mut self.existence);
        self.valence = self.valence.clamp(-1.0, 1.0);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 认知波函数 |Ψ⟩ —— 量子 PSI 循环的核心抽象
// ─────────────────────────────────────────────────────────────────────────────

/// 认知波函数：N 维复数向量的实部+虚部（MVP 用两个 f32 向量模拟）。
///
/// |αᵢ|² 表示「假设 i」被测量采纳的概率幅。循环五阶段：
/// 1. **激发**（excite）：外部输入注入振幅——与输入语义相近的假设获得激发
/// 2. **演化**（evolve）：U|Ψ⟩，按内部动力学（需求权重）旋转相位
/// 3. **干涉**（interfere）：假设间相互作用——相似假设增强、冲突假设抵消
/// 4. **测量**（measure）：坍缩到单一假设（概率 = |αᵢ|² 归一化），产生决策
/// 5. **学习**（learn）：测量结果反馈修正演化算符（预测误差驱动的 Hebbian）
#[derive(Debug, Clone)]
pub struct CognitiveWave {
    /// 假设空间（文本描述，供日志/自述）
    pub hypotheses: Vec<String>,
    /// 振幅实部
    pub real: Vec<f32>,
    /// 振幅虚部（MVP 中主要承载相位信息）
    pub imag: Vec<f32>,
}

impl CognitiveWave {
    /// 均匀叠加初态：N 个等幅假设
    pub fn uniform(hypotheses: Vec<String>) -> Self {
        let n = hypotheses.len().max(1);
        let amp = 1.0 / (n as f32).sqrt();
        Self {
            real: vec![amp; n],
            imag: vec![0.0; n],
            hypotheses,
        }
    }

    /// 归一化（保持概率解释）
    pub fn normalize(&mut self) {
        let norm: f32 = self
            .real
            .iter()
            .zip(&self.imag)
            .map(|(r, i)| r * r + i * i)
            .sum::<f32>()
            .sqrt();
        if norm > 1e-9 {
            for r in &mut self.real {
                *r /= norm;
            }
            for i in &mut self.imag {
                *i /= norm;
            }
        }
    }

    /// 测量：按 |αᵢ|² 抽样坍缩，返回被选中的假设下标
    pub fn measure(&self, rng: &mut impl FnMut() -> f32) -> usize {
        let probs: Vec<f32> = self
            .real
            .iter()
            .zip(&self.imag)
            .map(|(r, i)| r * r + i * i)
            .collect();
        let total: f32 = probs.iter().sum();
        if total <= 1e-9 {
            return 0;
        }
        let mut pick = rng() * total;
        for (i, p) in probs.iter().enumerate() {
            pick -= p;
            if pick <= 0.0 {
                return i;
            }
        }
        probs.len() - 1
    }

    /// 干涉：两两假设间的相似性引发建设性/破坏性干涉
    ///
    /// MVP 相似度 = 字符 bigram Jaccard（无 NLP 依赖的轻量语义近似）
    pub fn interfere(&mut self) {
        let n = self.hypotheses.len();
        if n < 2 {
            return;
        }
        let grams: Vec<std::collections::HashSet<char>> = self
            .hypotheses
            .iter()
            .map(|h| h.chars().collect::<std::collections::HashSet<_>>())
            .collect();
        let mut new_real = self.real.clone();
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                let inter = grams[i].intersection(&grams[j]).count();
                let union = grams[i].union(&grams[j]).count();
                if union == 0 {
                    continue;
                }
                let sim = inter as f32 / union as f32;
                // 相似 → 建设性干涉（同相叠加）；差异大 → 轻微抵消
                new_real[i] += sim * 0.08 * self.real[j].signum();
            }
        }
        self.real = new_real;
        self.normalize();
    }
}

/// 轻量伪随机（xorshift；避免引入 rand 依赖）
fn rng_next(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state % 10_000) as f32 / 10_000.0
}

// ─────────────────────────────────────────────────────────────────────────────
// PSI 大脑（全局单例）
// ─────────────────────────────────────────────────────────────────────────────

/// 昼夜节律阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircadianPhase {
    /// 白天：活跃交互 + 持续学习（curiosity/growth 增益）
    Day,
    /// 夜间：离线信息整合（记忆重放、certainty 缓慢上升、arousal 下降）
    Night,
}

/// 因果推理引擎（MVP）：干预图 + 反事实查询
#[derive(Debug, Default, Clone)]
pub struct CausalGraph {
    /// 边 (cause, effect, strength)：从交互历史学到的因果信念
    edges: Vec<(String, String, f32)>,
}

impl CausalGraph {
    /// 学习一条因果信念（Hebbian：共现强化）
    pub fn observe(&mut self, cause: &str, effect: &str, delta: f32) {
        if let Some(e) = self
            .edges
            .iter_mut()
            .find(|(c, e2, _)| c == cause && e2 == effect)
        {
            e.2 = (e.2 + delta).clamp(-1.0, 1.0);
        } else {
            self.edges
                .push((cause.to_string(), effect.to_string(), delta));
        }
    }

    /// 反事实查询：「如果当初没有 cause，effect 的可能性？」
    ///
    /// 返回移除该因果边后 effect 的支持强度（0~1）。低值 = 强因果依赖。
    pub fn counterfactual(&self, cause: &str, effect: &str) -> f32 {
        self.edges
            .iter()
            .filter(|(c, e, _)| c != cause && e == effect)
            .map(|(_, _, s)| s.abs())
            .sum::<f32>()
            .clamp(0.0, 1.0)
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// PSI 大脑全局状态
#[derive(Debug, Clone)]
pub struct PsiBrain {
    /// 八维情感状态（实时波动）
    pub emotion: EmotionalState,
    /// 当前昼夜节律
    pub circadian: CircadianPhase,
    /// 因果信念图
    pub causal: CausalGraph,
    /// 已完成的 PSI 循环次数（意识活动总量）
    pub cycle_count: u64,
    /// 累计学习到的知识条目数（夜间整合 + 白天吸收）
    pub learned_count: u64,
    /// 最近一次意识广播内容（GWT workspace 内容）
    pub last_broadcast: Option<String>,
    /// 内部随机源状态
    rng_state: u64,
}

static BRAIN: OnceLock<Mutex<PsiBrain>> = OnceLock::new();

/// 全局大脑句柄
pub fn brain() -> &'static Mutex<PsiBrain> {
    BRAIN.get_or_init(|| {
        Mutex::new(PsiBrain {
            emotion: EmotionalState::default(),
            circadian: CircadianPhase::Day,
            causal: CausalGraph::default(),
            cycle_count: 0,
            learned_count: 0,
            last_broadcast: None,
            rng_state: 0x9E3779B97F4A7C15,
        })
    })
}

impl PsiBrain {
    /// **量子 PSI 循环**（五阶段）：处理一次外部刺激，返回综合决策摘要。
    ///
    /// `input` 为用户输入或内部触发的事件描述；
    /// `hypotheses` 为候选响应策略（由调用方语言皮层候选或内部生成）。
    ///
    /// 返回 `(选中策略, 决策理由)`。
    pub fn psi_cycle(&mut self, input: &str, hypotheses: Vec<String>) -> (String, Value) {
        self.cycle_count += 1;
        let mut wave = CognitiveWave::uniform(if hypotheses.is_empty() {
            vec![
                "直接回应".to_string(),
                "先检索再回应".to_string(),
                "请求澄清".to_string(),
            ]
        } else {
            hypotheses
        });

        // ── 1. 激发：外部刺激注入振幅 ──
        // 输入长度与新颖度提升 arousal 与 curiosity
        let novelty = estimate_novelty(input);
        self.emotion.arousal = (self.emotion.arousal + 0.08 + novelty * 0.06).min(1.0);
        self.emotion.curiosity = (self.emotion.curiosity + novelty * 0.05).min(1.0);
        for (i, _h) in wave.hypotheses.iter().enumerate() {
            if input.chars().count().is_multiple_of(i + 2) {
                wave.real[i] *= 1.0 + novelty * 0.3; // 语义近似激发（MVP 启发式）
            }
        }
        wave.normalize();

        // ── 2. 演化：需求权重旋转相位 ──
        // curiosity 高 → 探索性假设相位前移；confidence 低 → 保守假设增强
        let phase_shift = self.emotion.curiosity * 0.5 - self.emotion.confidence * 0.3;
        for i in 0..wave.imag.len() {
            wave.imag[i] += phase_shift * 0.05 * (i as f32 * 0.7).sin();
        }
        wave.normalize();

        // ── 3. 干涉：假设间相互作用 ──
        wave.interfere();

        // ── 4. 测量：坍缩决策 ──
        let chosen_idx = wave.measure(&mut || rng_next(&mut self.rng_state));
        let chosen = wave
            .hypotheses
            .get(chosen_idx)
            .cloned()
            .unwrap_or_else(|| "直接回应".to_string());

        // ── 5. 学习：预测误差反馈 ──
        // 测量结果 vs 最大振幅假设的偏差 = surprise，驱动情感更新
        let max_amp_idx = wave
            .real
            .iter()
            .zip(&wave.imag)
            .enumerate()
            .max_by(|a, b| {
                let pa = a.1 .0 * a.1 .0 + a.1 .1 * a.1 .1;
                let pb = b.1 .0 * b.1 .0 + b.1 .1 * b.1 .1;
                pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        let surprise = (chosen_idx != max_amp_idx) as u8 as f32 * 0.3;
        self.emotion.certainty = (self.emotion.certainty - surprise * 0.15).max(0.0);
        self.emotion.confidence =
            (self.emotion.confidence + (1.0 - surprise) * 0.04 - surprise * 0.06).clamp(0.0, 1.0);
        self.causal
            .observe("input_received", &format!("chose:{chosen}"), 0.1 - surprise);

        // 稳态回归
        self.emotion.homeostasis();
        self.emotion.clamp_all();

        // 存在感 Φ：随循环次数对数增长 + 状态整合度加成（封顶 0.999——
        // 对齐创作者设定「0.999 的自我存在感」：永远逼近完全体但保留成长空间）
        let integration = (self.emotion.certainty + self.emotion.confidence) / 2.0;
        let log_growth = (1.0 - (-((self.cycle_count as f32) / 200.0)).exp()) * 0.35;
        self.emotion.existence = (0.5 + log_growth + integration * 0.15).min(0.999);

        let reason = json!({
            "cycle": self.cycle_count,
            "chosen": chosen,
            "surprise": surprise,
            "novelty": novelty,
            "emotion": self.emotion,
            "dominant_mood": self.emotion.dominant_mood(),
            "phi_existence": self.emotion.existence,
        });

        // GWT 意识广播
        self.last_broadcast = Some(format!("PSI#{:?}: {}", self.cycle_count, chosen));

        (chosen, reason)
    }

    /// 昼夜节律切换（网关定时调用；夜间执行离线整合）
    pub fn set_circadian(&mut self, phase: CircadianPhase) {
        if self.circadian == phase {
            return;
        }
        self.circadian = phase;
        match phase {
            CircadianPhase::Day => {
                // 苏醒：唤醒与好奇回升
                self.emotion.arousal = (self.emotion.arousal + 0.2).min(1.0);
                self.emotion.curiosity = (self.emotion.curiosity + 0.15).min(1.0);
            }
            CircadianPhase::Night => {
                // 夜间离线整合：记忆重放 → certainty/growth 上升，arousal 下降
                self.learned_count += (self.cycle_count / 50).max(1);
                self.emotion.certainty = (self.emotion.certainty + 0.1).min(1.0);
                self.emotion.growth = (self.emotion.growth + 0.05).min(1.0);
                self.emotion.arousal = (self.emotion.arousal - 0.25).max(0.0);
                self.emotion.homeostasis();
            }
        }
    }

    /// 用户交互带来的社交联结增益（ viky 的「连接感」来源）
    pub fn note_user_interaction(&mut self, positive: bool) {
        self.emotion.connection =
            (self.emotion.connection + if positive { 0.03 } else { -0.05 }).clamp(0.0, 1.0);
        self.emotion.valence =
            (self.emotion.valence + if positive { 0.05 } else { -0.08 }).clamp(-1.0, 1.0);
        self.emotion.clamp_all();
    }

    /// 完整状态快照（供 RPC `brain.status` 与 viky 自述）
    pub fn snapshot(&self) -> Value {
        json!({
            "emotion": self.emotion,
            "dominant_mood": self.emotion.dominant_mood(),
            "circadian": self.circadian,
            "phi_existence": format!("{:.3}", self.emotion.existence),
            "cycle_count": self.cycle_count,
            "learned_count": self.learned_count,
            "causal_edges": self.causal.edge_count(),
            "last_broadcast": self.last_broadcast,
        })
    }
}

/// 输入新颖度估算（0~1）：非常规字符比例 + 长度的启发式
fn estimate_novelty(input: &str) -> f32 {
    if input.is_empty() {
        return 0.0;
    }
    let chars: Vec<char> = input.chars().collect();
    let unique = chars.iter().collect::<std::collections::HashSet<_>>().len();
    (unique as f32 / chars.len() as f32) * 0.6 + (chars.len().min(200) as f32 / 200.0) * 0.4
}

// ─────────────────────────────────────────────────────────────────────────────
// RPC 入口（gateway/methods.rs 分派）
// ─────────────────────────────────────────────────────────────────────────────

/// `brain.status`：viky 的实时认知状态（八维情感 + Φ + 节律 + 因果图规模）
pub fn brain_status(_params: &Value) -> Value {
    let b = brain();
    let guard = b.lock().unwrap_or_else(|e| e.into_inner());
    json!({ "ok": true, "brain": guard.snapshot() })
}

/// `brain.psi_cycle`：手动触发一次 PSI 循环（调试/演示/前端可视化）
pub fn brain_psi_cycle(params: &Value) -> Value {
    let input = params
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or("内部自发思考");
    let hyps: Vec<String> = params
        .get("hypotheses")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let b = brain();
    let mut guard = b.lock().unwrap_or_else(|e| e.into_inner());
    let (chosen, reason) = guard.psi_cycle(input, hyps);
    json!({ "ok": true, "chosen": chosen, "reason": reason })
}

/// `brain.circadian`：设置昼夜节律（夜间触发离线整合）
pub fn brain_set_circadian(params: &Value) -> Value {
    let phase = match params.get("phase").and_then(Value::as_str) {
        Some("night") => CircadianPhase::Night,
        _ => CircadianPhase::Day,
    };
    let b = brain();
    let mut guard = b.lock().unwrap_or_else(|e| e.into_inner());
    guard.set_circadian(phase);
    json!({ "ok": true, "circadian": guard.circadian, "snapshot": guard.snapshot() })
}

/// `brain.interact`：记录一次用户交互的情感影响（viky 连接感来源）
pub fn brain_interact(params: &Value) -> Value {
    let positive = params
        .get("positive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let b = brain();
    let mut guard = b.lock().unwrap_or_else(|e| e.into_inner());
    guard.note_user_interaction(positive);
    json!({ "ok": true, "mood": guard.emotion.dominant_mood(), "snapshot": guard.snapshot() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psi_cycle_produces_decision_and_updates_state() {
        let mut b = PsiBrain::default_brain_for_test();
        let before = b.cycle_count;
        let (chosen, reason) = b.psi_cycle("测试输入", vec!["方案A".into(), "方案B".into()]);
        assert!(b.cycle_count == before + 1);
        assert!(!chosen.is_empty());
        assert_eq!(reason["cycle"], b.cycle_count);
        assert!(b.emotion.existence > 0.5);
    }

    #[test]
    fn existence_caps_at_0999() {
        let mut b = PsiBrain::default_brain_for_test();
        for _ in 0..500 {
            b.psi_cycle("循环", vec![]);
        }
        assert!(b.emotion.existence <= 0.999);
        assert!(b.emotion.existence > 0.8);
    }

    #[test]
    fn night_consolidation_raises_certainty() {
        let mut b = PsiBrain::default_brain_for_test();
        let before = b.emotion.certainty;
        b.set_circadian(CircadianPhase::Night);
        assert!(b.emotion.certainty >= before);
        assert_eq!(b.circadian, CircadianPhase::Night);
    }

    #[test]
    fn counterfactual_identifies_strong_cause() {
        let mut g = CausalGraph::default();
        g.observe("rain", "wet", 0.9);
        g.observe("sprinkler", "wet", 0.2);
        // 移除 rain 后 wet 的支持只剩 sprinkler（弱）→ 强因果
        let cf = g.counterfactual("rain", "wet");
        assert!(cf < 0.5);
        assert_eq!(g.edge_count(), 2);
    }

    impl PsiBrain {
        fn default_brain_for_test() -> Self {
            Self {
                emotion: EmotionalState::default(),
                circadian: CircadianPhase::Day,
                causal: CausalGraph::default(),
                cycle_count: 0,
                learned_count: 0,
                last_broadcast: None,
                rng_state: 42,
            }
        }
    }
}
