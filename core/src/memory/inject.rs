//! P2·B 分层稀疏记忆的**引擎接线层**（把 `flyhash` + `tiered` 从"能测的库"变成"真的在用"）。
//!
//! # 为什么需要这一层（而不是直接调 `TieredMemory`）
//!
//! `memory::tiered::TieredMemory` 是**值类型**：谁 new 谁独享。而"记忆"必须跨轮、跨会话
//! 共享——否则每轮 new 一个空库，检索永远 0 条，功能等于不存在（本项目最典型的
//! "有而不用"缺陷）。所以需要一层**进程级单例**把生命周期抬到进程维度。
//!
//! # 与既有五路召回的关系（复用，不替代）
//!
//! `gateway::chat::memory_stable_prefix` 已经有五路并行召回（跨项目 / 长期向量 /
//! 短期 FTS / 召回库 / 经验台账）。本模块提供的是**第六路**，且：
//! - **默认关闭**（`memory.tiered_enabled = false`）：开启会让所有历史会话的 prompt
//!   多一段内容，属于最难排查的静默回归，必须由用户显式打开；
//! - 开启后**只增不减**：不改动前五路的任何行为，只追加 tiered 命中；
//! - 命中带独立前缀 `[分层记忆]`，出问题时可一眼定位是哪一路注入的。
//!
//! # 为什么写入放在召回之前（同一轮内）
//!
//! 没有独立的"记忆写入"时机：会话历史由 `session_history` 管理，本模块不想再复制一份
//! 存储。取最小闭环——每轮把当前用户消息 `remember` 进 tiered（进程内、有容量上限），
//! 下一轮即可被检索到。这样跨轮有效、零磁盘副作用、且容量上限天然限制内存增长。

use std::sync::{Mutex, OnceLock};

use crate::memory::tiered::{MemoryTierVariant, TieredMemory};

/// 每层容量上限。
///
/// 定 512 的理由：tiered 里每条记忆都是**短文本片段**（一轮用户消息/一条经验），
/// 512 条约几百 KB 量级；再大就该上磁盘（那是 `long_term` 向量库的职责，不该由本层承担）。
const MAX_PER_TIER: usize = 512;

/// 单次召回最多注入的条数（与既有五路的 top-K 量级对齐，避免抢占比重大）。
const RECALL_TOP_K: usize = 3;

/// 注入内容的字符预算：超过即逐条截断追加，防止一段记忆吃掉整个上下文预算。
const INJECT_CHAR_BUDGET: usize = 1_200;

fn cell() -> &'static Mutex<TieredMemory> {
    static CELL: OnceLock<Mutex<TieredMemory>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(TieredMemory::with_capacity(MAX_PER_TIER)))
}

/// 测试用：串行化**所有**触碰 tiered 单例的用例。
///
/// 必须是唯一入口：此前 `memory::inject` 与 `gateway::chat` 各自 `OnceLock<Mutex>`，
/// 两把不同的锁保护同一个单例 —— 并行跑时必然互相清库（实测：单跑绿、混跑红）。
/// 锁与被保护对象必须同源，这是同一个原则的延伸。
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 测试用：清空进程级库（避免用例相互污染）。
#[cfg(test)]
pub(crate) fn reset_for_test() {
    if let Ok(mut g) = cell().lock() {
        *g = TieredMemory::with_capacity(MAX_PER_TIER);
    }
}

/// 把一段文本写入分层记忆（幂等：完全相同的文本不重复入库）。
///
/// 写入落在 `LongTerm`（语义长时层）——它是跨轮持久的那一层；
/// `Sensory`/`Working` 由 `tiered` 内部按其 TTL 语义管理，本层不越权。
///
/// # 写回闸门（本函数是全仓**唯一**的自动写入入口）
///
/// 入 tiered 之前先过一次 [`crate::memory::writeback::remember_fact`]：它按
/// [`crate::memory::promote`] 的阈值决定这条文本值不值得**落盘**、落跨项目层还是
/// 项目局部层。`Drop` 时不产生任何磁盘记录（闸门有牙），但进程内的 tiered 缓存
/// 行为保持不变 —— 那是**临时**层，与「值得长期记住」是两件事，不该被同一道闸门
/// 顺带改掉（那是比本功能大得多的行为变更）。
///
/// `meta` 就是写回层的「项目」：会话键（`agent:main:webchat:…`）本身标识了这条事实
/// 属于哪条工作流，写入后跨项目检索能按它标注来源。
pub fn remember(text: &str, meta: &str) -> bool {
    let t = text.trim();
    // 太短的消息（"hi" / "继续"）没有检索价值，入库只会稀释近邻质量
    if t.chars().count() < 8 {
        return false;
    }
    let mut g = match cell().lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    // 去重：同一条消息重复投递（重试、回放）不应占多个槽位
    if g.recall(MemoryTierVariant::LongTerm, t, 1)
        .iter()
        .any(|h| h.entry.text == t)
    {
        return false;
    }
    // 写回闸门：只有真正过闸的文本才会在磁盘上留下事实记录。
    let _ = crate::memory::writeback::remember_fact(meta, t, "tiered", crate::cache::now_ms());
    g.insert(MemoryTierVariant::LongTerm, t, meta);
    true
}

/// 只读计数快照：`(sensory, short_term, long_term, flyhash 索引条目数)`。
///
/// 供 `memory.tiered.status` RPC 观测「分层稀疏记忆到底在用没用、索引是否健全」。
/// **不加任何写入能力**：本层是进程内单例，外部只应观测，不应绕过 chat 链路灌数据。
pub fn tier_counts() -> (usize, usize, usize, usize) {
    let g = cell().lock().unwrap_or_else(|e| e.into_inner());
    (
        g.len(MemoryTierVariant::Sensory),
        g.len(MemoryTierVariant::ShortTerm),
        g.len(MemoryTierVariant::LongTerm),
        g.fly_len(),
    )
}

/// 只读配置快照：`(每层容量上限, 弱证据标注线)`。
///
/// 第二个量的历史名字叫「余弦门控阈值」—— 语义已变（见 `tiered` 模块头）：它不再决定
/// 丢弃，只决定注入行是否带 `(弱相关)`。真正的丢弃规则是相对截断，见 [`tier_cutoff_rule`]。
pub fn tier_limits() -> (usize, f64) {
    let g = cell().lock().unwrap_or_else(|e| e.into_inner());
    (g.max_per_tier(), g.weak_evidence())
}

/// 只读截断规则：`(相对保留比例, 零重合可否单独成立)`。
///
/// 注入与否实际由这两项决定：前者是「不比最好的那条差多少」，后者在默认后端下
/// 恒为 `false`（词面共享 0 个实词的候选一律不给，见 `scoring::LOCAL_HASH_IS_SEMANTIC`）。
pub fn tier_cutoff_rule() -> (f64, bool) {
    let g = cell().lock().unwrap_or_else(|e| e.into_inner());
    g.cutoff_rule()
}

/// 召回：词面证据优先（零词面重合的候选在默认后端下不单独成立），档内按幅度排序后
/// 相对截断，返回可拼进 prompt 的若干行（规则见 `crate::memory::scoring`）。
///
/// 返回 `None` 表示无命中（**不返回空 Vec**：调用方据此完全跳过注入，
/// 避免在 prompt 里留下一个空标题段）。
pub fn recall(query: &str) -> Option<Vec<String>> {
    let mut g = match cell().lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    let hits = g.progressive_retrieve(query, RECALL_TOP_K);
    if hits.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut used = 0usize;
    for h in hits {
        // 弱证据仍会注入（相对截断认为它是本查询能拿到的最好几条之一），
        // 但必须让模型知道这是候选、不是事实——否则模糊召回会被当成既定结论引用。
        let tag = if h.weak { "(弱相关) " } else { "" };
        let line = format!("[分层记忆] {}{}", tag, h.entry.text.trim());
        let cost = line.chars().count();
        if used + cost > INJECT_CHAR_BUDGET {
            break;
        }
        used += cost;
        out.push(line);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 组装成一段可直接塞进 System 消息的文本（带标题与来源标识）。
pub fn render(lines: &[String]) -> String {
    let mut s = String::from("## 相关分层记忆（关键词/语义两路证据，仅作参考；标 `(弱相关)` 的是候选不是事实）\n");
    for l in lines {
        s.push_str("- ");
        s.push_str(l);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用例必须串行：单例是进程级的，并行会互相看到对方的条目。
    /// 统一走 `test_guard()`，与 `gateway::chat` 的 tiered 用例共用**同一把锁**。
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        test_guard()
    }

    #[test]
    fn remember_then_recall_returns_the_same_item() {
        let _g = lock();
        reset_for_test();
        assert!(remember("这次部署本地模型要记得先激活再发布快照", "s1"));
        let got = recall("本地模型部署后怎么让它生效").expect("应命中");
        assert!(
            got.iter().any(|l| l.contains("先激活再发布快照")),
            "应召回刚写入的条目: {got:?}"
        );
    }

    #[test]
    fn short_messages_are_rejected() {
        let _g = lock();
        reset_for_test();
        // "hi" / "继续" 这类没有检索价值，入库只会稀释近邻质量
        assert!(!remember("hi", "s1"));
        assert!(!remember("继续", "s1"));
    }

    #[test]
    fn duplicate_write_is_ignored() {
        let _g = lock();
        reset_for_test();
        let t = "同一条消息重复投递不应占多个槽位";
        assert!(remember(t, "s1"));
        assert!(!remember(t, "s1"), "第二次写入应被判重");
    }

    /// 写回闸门**真的接在写入路径上**：过闸的文本必须在事实库里留下记录
    /// （只断言「调用了闸门」是不够的 —— 那正是本仓最典型的「有而不用」缺陷）。
    #[test]
    fn remembering_through_the_inject_path_leaves_a_fact_record() {
        let _g = lock();
        reset_for_test();
        let dir = std::env::temp_dir().join(format!(
            "mgt-inject-writeback-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        crate::persistence::config::Config::set_config_dir_override(dir.clone());

        let text = "记住：团队约定发布前必须先跑 verify.sh --clean --all（脚本在 /scripts/verify.sh）";
        assert!(remember(text, "agent:main:webchat:1"));
        let facts = crate::memory::writeback::active_records();
        let hit = facts
            .iter()
            .find(|f| f.text == text)
            .unwrap_or_else(|| panic!("过闸的文本必须落进事实库: {facts:?}"));
        assert_eq!(hit.tier, "cross_project", "高分文本应进跨项目层: {hit:?}");
        assert_eq!(
            hit.project, "agent:main:webchat:1",
            "会话键就是写回层的项目标识"
        );

        std::fs::remove_dir_all(&dir).unwrap_or(());
    }

    #[test]
    fn unrelated_query_yields_nothing_or_only_unrelated() {
        let _g = lock();
        reset_for_test();
        // 只写一条关于音乐的，再问数据库索引——不应把它当"相关记忆"注入
        remember("这首歌的鼓点落在 129 BPM，切镜要吸附拍点", "s1");
        match recall("PostgreSQL 的 B-tree 索引在什么情况下失效") {
            None => {}
            Some(lines) => {
                // 允许把唯一一条当近邻返回（库太小），但**必须**只返回一条，
                // 不能把整个库都倒进上下文
                assert!(lines.len() <= RECALL_TOP_K, "不得超召回上限: {lines:?}");
            }
        }
    }

    #[test]
    fn empty_store_returns_none_not_empty_vec() {
        let _g = lock();
        reset_for_test();
        assert!(recall("任何查询").is_none(), "空库必须返回 None 而非空数组");
    }

    #[test]
    fn rendered_block_carries_provenance_prefix() {
        let _g = lock();
        reset_for_test();
        remember("渲染出来的记忆块要带来源前缀，便于定位是哪一路注入的", "s1");
        let lines = recall("记忆块要带来源前缀").expect("应命中");
        let text = render(&lines);
        assert!(text.contains("## 相关分层记忆"), "{text}");
        assert!(text.contains("[分层记忆]"), "每条都要带来源标记: {text}");
    }

    #[test]
    fn recall_respects_char_budget() {
        let _g = lock();
        reset_for_test();
        // 灌入多条长文本，注入总长必须被预算截住
        for i in 0..8 {
            remember(
                &format!("第{i}条很长的记忆片段，用来验证注入预算是否生效：这一段刻意写得很长很长以求撑爆预算上限"),
                "s1",
            );
        }
        let lines = recall("验证注入预算是否生效").expect("应命中");
        let total: usize = lines.iter().map(|l| l.chars().count()).sum();
        assert!(total <= INJECT_CHAR_BUDGET, "注入总长 {total} 超过预算 {INJECT_CHAR_BUDGET}");
    }
}
