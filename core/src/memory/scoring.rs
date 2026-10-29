//! 记忆检索的相关性打分与截断规则（**单一事实源**）
//!
//! # 为什么单独一个模块
//!
//! 「什么算相关」此前在两处各写一遍：`tiered`（分层召回，默认关闭）用
//! `0.60 余弦 + 0.30 flyhash + 0.10 访问次数` 再叠一道绝对门 `cosine ≥ 0.35`；
//! `long_term::search`（**默认开启**的 prompt 注入路）另有一份同值但独立命名的
//! `SIM_THRESHOLD = 0.35`。两份阈值互不相干，改一处忘一处是结构性的，不是偶然的。
//! 本模块只放**规则本身**，各调用方负责把它套到自己的结果类型上。
//!
//! 实测依据与决策记录见 `docs/prd/2026-10-29-memory-substrate-retrieval-fix.md`。
//!
//! # 规则一：两路证据，**词面优先**，语义只在真有语义时补位
//!
//! 幅度 `rank = fuse(...)`（见 [`fuse`]，供 [`keeps`] 与弱证据标注用），
//! 次序 `先比有无词面证据 → 再比 rank`（[`lexical_first`]）。
//!
//! - 不加权：两路标度不可比（一路是离散重合比例，一路是连续余弦），加权系数
//!   只会变成第二个没人敢调的魔数 —— 旧实现那三个系数正是从未被测量过的例子。
//! - 不用 RRF（秩倒数融合）：`1/(60+rank)` 把分数压进约 `[0, 0.033]`，规则二的
//!   「不比最佳分差 40%」在那种标度上**无法成立**。这是在评审阶段推翻规格原方案的原因。
//! - **次序也不取大**（被实测推翻的第二版）：只用 `max` 排序时，常驻回归
//!   `memory::recall_probe` 抓到 3 条「朴素关键词基线命中、分层通路漏掉」的查询，
//!   现场都是同一件事——目标条目共享 1 个实词（覆盖率 0.20），却排在 5 条
//!   **零重合**候选之后（其余弦 0.20–0.49）。词面那一路的可信度另有直接读数：
//!   `loose` 精确率 0.400 vs 纯余弦 0.147。
//! - **零重合的候选不得单独成立**（被实测推翻的第三版）。既然余弦压过了词面证据，
//!   下一步自然是「那把它关掉」。实测（[`recall_probe`] 的常驻测量）给出理由：默认后端
//!   在**零词面重合**的句子上根本区分不了「同族改述」和「彻底无关」——
//!   同族改述 p50 **0.137**，异族无关 p50 **0.104**、p90 **0.247**，且
//!   **207/2095（9.9%）的无关对余弦 ≥0.25**。也就是说噪声尾部已经越过信号中位，
//!   **不存在任何一个阈值能把两者分开**；`HARD_FLOOR = 0.15` 就落在这团噪声里面
//!   （回归 `memory::tiered::tests::a_wholly_unrelated_library_injects_nothing`
//!   当初红在那条 0.211，就是纯哈希桶碰撞）。
//!   所以 [`LOCAL_HASH_IS_SEMANTIC`] 为假时，`overlap == 0` 直接记 0 分：
//!   「词面上没关系、又拿不出真语义」的记忆**不该进被缓存的 System 前缀**。
//!   代价是 `hard·paraphrase`（与同族语料零实词重合的改述）从 50% 回到 0%——
//!   而那 50% 是碰运气：同档精确率只有 0.12，等于每投十行错八行。
//!   **这是诚实的空白，不是退步**：这条能力要靠真嵌入模型拿到，翻那个常量即可，
//!   届时常驻测量会先变红、逼改的人签字。
//!
//! 于是次序仍是「有词面证据的在前」；词面档内覆盖率对「重合实词数」单调，
//! 所以那一档就是实测最强的朴素关键词基线的排序。
//!
//! # 规则二：截断只看「这次查询到底有没有证据」
//!
//! `keeps(best, r) = best > 0 且 r ≥ RELATIVE_KEEP_RATIO × best`。
//!
//! 绝对门的失败模式是**饿死**而不是「排错序」：实测中文改写档最佳余弦中位数
//! **0.346**，恰好压在 0.35 门下 —— 于是那一整档什么都不注入，表现是「记忆像不存在」。
//! 相对判据只问「这条比本查询能找到的最好那条差多少」，所以库再小、措辞再偏，
//! 只要有证据就不会全空。
//!
//! ⚠️ 这里刻意**没有**数字地板。第一版留了一道 `HARD_FLOOR = 0.15` 用来挡
//! 「整库都不相关」，实测证明它挡错了对象：它按比例设线，于是惩罚**长查询**。
//! 中文查询经双字分词后实词动辄 15–25 个，「共享 1 个实词」这种真证据只有
//! 0.04–0.07 分，整档被它清空（`zh·paraphrase` 100% → 70%）。而「整库都不相关」
//! 本来就不是比例问题——它等于「最好的那条也共享不到任何实词」，也就是
//! `best == 0`，由 [`fuse`] 的零重合闸直接归零。两件事各归其位：**证据看有无，
//! 取舍看相对**。

/// 弱证据标注线：两路都低于它却仍被注入的命中，须带上「这是候选、不是事实」的信号。
///
/// ⚠️ 它**不决定丢弃**（丢弃只由 [`keeps`] 决定）。历史名字叫「余弦门控阈值」，
/// `memory.tiered.status` 沿用该键名以保持口径连续。
pub const WEAK_EVIDENCE: f64 = 0.35;
/// 相对截断比例：只注入「不比本查询可得的最好那条差 40% 以上」的命中。
pub const RELATIVE_KEEP_RATIO: f64 = 0.60;

/// 默认的本地后端（`LocalEmbedding` / `local-hash`）**是否承载真实语义**。
///
/// 实测为**否**：见模块头「规则一」第三版的三组数（同族改述 p50 0.137 vs
/// 异族无关 p50 0.104、p90 0.247）。它只是特征哈希的重合度，零重合的句子
/// 照样能靠桶碰撞拿到 0.2+。
///
/// ⚠️ 接入真正的嵌入模型（ONNX / 远端 embedding，见 `embedding_real.rs`）时，
/// 把这里翻成 `true` 才会重新允许零词面重合的候选单独进上下文。
/// **别先翻常量再测**：常驻测量
/// [`recall_probe::the_local_hash_channel_carries_no_semantic_signal`] 会先变红，
/// 那个红就是「请签字确认这条判据不再成立」的闸。
pub const LOCAL_HASH_IS_SEMANTIC: bool = false;

/// 路 A：实词覆盖率 = 与查询共享的实词数 / 查询实词数。
///
/// 分母只与**查询**有关，因此层内排序与「按共享实词数降序」完全一致
/// ——那正是实测 70.8%(en) / 100%(zh) 的朴素关键词基线；换成比例只是为了
/// 得到一个能跨查询比较、可交给 [`cutoff`] 的分数。
///
/// 查询没有实词（整句是停用词/标点）时返回 0：此时只能靠语义那一路，
/// 不该因为「查询没内容」而把所有候选都算成满分。
pub fn coverage(query_word_count: usize, shared: usize) -> f64 {
    if query_word_count == 0 {
        0.0
    } else {
        shared as f64 / query_word_count as f64
    }
}

/// 融合两路证据成一个**幅度**：默认后端下只认词面那一路。
///
/// 只用于 [`cutoff`] 与弱证据标注，**不用于排序**（排序见 [`lexical_first`]）。
/// 为什么三处不能都叫「取大」：把 `max` 同时当次序，等于允许一条零词面重合的余弦
/// 压掉一条真实存在的词面命中——实测就是这样漏掉 3 条查询的。
///
/// `overlap` 是必填项，不是冗余：拿不到词面证据时，语义那一路在默认后端下是噪声
/// （见 [`LOCAL_HASH_IS_SEMANTIC`]），于是这里退回 [`coverage`] —— 零重合自然记 0，
/// 由 [`cutoff`] 丢弃，而不是等调用方各自记得加这道闸。
/// 有词面证据时仍取大：那一路只在「已经共享了实词」的候选之间做细微修正，
/// 不再有无重合的候选靠碰撞混进来。
///
/// ⚠️ **这一处是已知未决的取舍，不是已证的结论**（2026-10-29 复测）。把档内的
/// `max(coverage, cosine)` 换成纯 `coverage`，常驻回归实测：
/// hit@5 与饿死**完全不变**（78%·饿5 / 0.65→**0.78** 精度 / 0.29→**0.23** 预算利用）——
/// 也就是「召回相同、投得更少也更准」。此前写在这里的「实测更差 0.338 → 0.200」
/// 是**旧精度口径**（分母固定为 K）的读数，那个口径把「少投」本身算成失分，
/// 已在 [`recall_probe`] 纠正；两个数都是 `f` 列，不是同一条判据。
/// 现在保持取大的理由只剩一条可辩护的：`WEAK_EVIDENCE` 是一条**绝对**标注线，
/// 纯覆盖率对长查询（中文 15–25 个实词）几乎恒低于它，于是注入行**全部**带
/// `(弱相关)` —— 标签饱和等于没有标签。
/// P2 给权重之前先按这两列表决，别靠口味。
pub fn fuse(overlap: usize, coverage: f64, cosine: f64) -> f64 {
    if overlap > 0 {
        coverage.max(cosine)
    } else if LOCAL_HASH_IS_SEMANTIC {
        cosine
    } else {
        0.0
    }
}

/// 两条命中之间的**证据优先级**（降序）：先比「有没有词面证据」，再比幅度。
///
/// 词面档内覆盖率对「重合实词数」单调，所以这一档就是实测最强的朴素关键词基线的顺序；
/// 零重合那档整体排在后面，且在默认后端下幅度为 0、过不了 [`cutoff`]——
/// 次序里保留它，是为了 [`LOCAL_HASH_IS_SEMANTIC`] 翻真之后这条兜底路自然生效，
/// 不必再改排序代码。
/// 调用方在此之后还应接一个确定性 tie-break（id / 时间戳），因为命中集合会进 System 前缀。
pub fn lexical_first(
    a_overlap: usize,
    a_rank: f64,
    b_overlap: usize,
    b_rank: f64,
) -> std::cmp::Ordering {
    let class = |o: usize| u8::from(o > 0);
    class(b_overlap)
        .cmp(&class(a_overlap))
        .then(b_rank.partial_cmp(&a_rank).unwrap_or(std::cmp::Ordering::Equal))
}

/// 本次查询的截断线（相对那部分）。见模块头「规则二」。
pub fn cutoff(best_rank: f64) -> f64 {
    best_rank * RELATIVE_KEEP_RATIO
}

/// 一条候选是否该注入。见模块头「规则二」。
///
/// 两条判据：① 本次查询**拿得出证据**（`best_rank > 0`，即至少有一条命中共享了实词）；
/// ② 不比可得的最好那条差 40% 以上。
///
/// ⚠️ 第 ① 条替代了旧的绝对数字地板 `HARD_FLOOR = 0.15`。数字地板按**比例**设线，
/// 于是惩罚长查询：中文查询经双字分词后实词数动辄 15–25 个，「共享 1 个实词」这种
/// 真证据只有 0.04–0.07 分，会被 0.15 直接挡掉——实测把 `zh·paraphrase` 从 100%
/// 打到 70%，而朴素关键词基线是 100%。「整库都不相关」这件事本来就不靠比例表达，
/// 它等于「最好的那条也共享不到实词」，也就是 `best_rank == 0`。
pub fn keeps(best_rank: f64, rank: f64) -> bool {
    best_rank > 0.0 && rank >= cutoff(best_rank)
}

/// 是否该标成弱证据。
pub fn is_weak(rank: f64) -> bool {
    rank < WEAK_EVIDENCE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_is_relative_to_the_query_only() {
        assert_eq!(coverage(4, 4), 1.0);
        assert_eq!(coverage(4, 2), 0.5);
        assert_eq!(coverage(0, 0), 0.0, "空查询不得给满分");
    }

    /// 融合幅度的两条分支，按「有无词面证据」拆开钉住。
    ///
    /// 零重合那格是本轮修复的核心：**它随 [`LOCAL_HASH_IS_SEMANTIC`] 换档**，
    /// 所以写成条件断言。判「语义通道到底有没有语义」的证据在
    /// [`recall_probe::the_local_hash_channel_carries_no_semantic_signal`]，不在这里。
    #[test]
    fn fuse_is_gated_on_lexical_evidence_not_a_weighted_sum() {
        // 有词面证据：两路取大（档内只做细微修正，不做准入判断）
        assert_eq!(fuse(2, 0.4, 0.9), 0.9);
        assert_eq!(fuse(2, 0.8, 0.3), 0.8);
        if LOCAL_HASH_IS_SEMANTIC {
            assert!(fuse(0, 0.0, 0.474) > 0.0, "真有语义时零重合也可以有分");
        } else {
            assert_eq!(fuse(0, 0.0, 0.474), 0.0, "哈希碰撞不得凭余弦单独进上下文");
        }
    }

    /// 整库拿不出词面证据 → 最佳分 0 → 一条都不注入（宁缺毋滥）。
    #[test]
    fn a_library_with_no_best_score_injects_nothing() {
        assert!(
            !keeps(0.0, 0.0),
            "最佳分为 0 时「至少给一条」就是把无关文本倒进 System 前缀"
        );
        assert!(keeps(0.2, 0.2), "最好的那条永远不会被自己的截断线挡掉");
    }

    /// 本次修复的正面断言：**一个词面命中不得被零重合的余弦压掉**。
    /// 实测漏掉的三条查询全是这个形状（目标覆盖 0.20，噪声余弦 0.20–0.49）。
    #[test]
    fn one_shared_content_word_outranks_a_zero_overlap_semantic_match() {
        let lexical = (1usize, 0.20f64);
        let semantic = (0usize, 0.49f64);
        assert_eq!(
            lexical_first(lexical.0, lexical.1, semantic.0, semantic.1),
            std::cmp::Ordering::Less,
            "降序比较里词面档应排在语义档之前"
        );
        // 档内仍按幅度：两条语义补位之间，余弦高的在前
        assert_eq!(
            lexical_first(0, 0.30, 0, 0.45),
            std::cmp::Ordering::Greater
        );
        // 两条词面之间按覆盖率（等价于按重合词数）
        assert_eq!(
            lexical_first(3, 0.60, 1, 0.20),
            std::cmp::Ordering::Less
        );
    }

    /// 饿死回归：最佳分只有 0.346（实测中文改写档中位数）时，绝对门会丢掉**全部**，
    /// 相对截断必须留下最好的那条。
    #[test]
    fn a_best_score_under_the_old_gate_is_not_starved() {
        let best = 0.346;
        assert!(
            best < WEAK_EVIDENCE,
            "夹具前提：这一档的最佳分本就低于旧绝对门"
        );
        assert!(
            cutoff(best) <= best,
            "只要够得上地板，最好的那条不会被自己的截断线挡掉"
        );
        assert!(best >= cutoff(best));
    }

    /// 已退休的数字地板，只留在这里作对照（决策记录见模块头「规则二」）。
    const RETIRED_HARD_FLOOR: f64 = 0.15;

    /// 地板已删除的**前提**：1/20 这种「共享一个实词的长查询」必须能注入。
    ///
    /// 这条断言就是当初 `HARD_FLOOR = 0.15` 被实测推翻的原因——它按比例设线，
    /// 于是中文双字分词后动辄 15–25 个实词的查询里，最弱的真证据（1 个共享实词）
    /// 恰好落在地板之下（`zh·paraphrase` 100% → 70%）。谁想把数字地板加回来，
    /// 先让这一条过。
    #[test]
    fn one_shared_word_in_a_long_query_is_still_kept() {
        let weak = coverage(20, 1);
        assert!(weak < RETIRED_HARD_FLOOR, "夹具前提：这条证据本来就低于旧地板");
        assert!(keeps(weak, weak), "最好的那条不得被自己的截断线挡掉");
    }
}
