//! 记忆**写入闸门**：一处决定「要不要写、写到哪层、写坏了怎么退场」。
//!
//! # 为什么单独一个模块
//!
//! 写入路径此前是三个入口各写各的：`long_term::store` 调用方写下就落库、
//! `tiered::insert` 只做 FIFO 淘汰、`consolidate` 只做「相同内容跳过」。
//! 于是「什么算值得长期记住」「旧条目怎么退场」「同一个 key 写两次听谁的」
//! 这三个问题在仓库里没有答案，只有各入口的偶发行为。
//! 本模块只放**策略本身**，调用方负责把决策落到自己的存储上 —— 与 [`scoring`]
//! 同一定位：规则一处、执行各处。检索（`long_term::search` / `tiered::recall`）
//! 一行不动。
//!
//! [`scoring`]: crate::memory::scoring
//!
//! # 三条规则
//!
//! 1. **准入**（[`decide`]）：分数 → `Promote` / `Keep` / `Drop`，阈值全部集中在
//!    文件头部常量（[`PROMOTE_THRESHOLD`] / [`KEEP_THRESHOLD`]）。
//!    分数 = 重要度（显式要求记住的再加 [`EXPLICIT_BOOST`]），**不含时间**：
//!    时间只属于「退场」，两件事分开才能在测试里各自钉住，也才不会被
//!    「刚写进来的高分条目因为时间戳差点意思被判 Drop」这种口径混在一起。
//! 2. **退场**（[`forget`]）：`保留度 = 重要度 × 0.5^(距最后命中秒数 / 半衰期)`，
//!    低于 [`FORGET_THRESHOLD`] 时**只降一层**（L3 → L2 → L1），**绝不删除**，
//!    并把保留度与原因留在记录里。删除不可逆，降级还能被下一次命中救回来；
//!    「超阈值就删」在上一个版本里正是「记忆莫名消失」的形态。
//!    L1 是地板层，没有更低层可退 —— 那时返回 [`ForgetOutcome::Keep`]，
//!    真正的 TTL 回收归调用方，本闸门不越界。
//! 3. **同 key 冲突**（[`WriteGate::write`]）：保留新的、旧的标 `superseded_by`
//!    留痕，召回（[`WriteGate::recall`] / [`WriteGate::recall_by_key`]）只回最新一条。
//!    两条同 key 的值同时进上下文，等于把「该信哪个」丢给模型自己猜；
//!    而直接删掉旧值又会让「它什么时候被改掉的」无从追溯。
//!
//! 计数（[`WriteGate::promoted`] / [`WriteGate::dropped`] / [`WriteGate::superseded`]）
//! 只做只读观测，**不注册 RPC**：这里是库内策略层，对外口径由上层决定。

use crate::memory::tiered::MemoryTierVariant;

/// 准入阈值：分数到此即 [`Verdict::Promote`]（升到长时层）。
pub const PROMOTE_THRESHOLD: f64 = 0.70;
/// 准入阈值：分数到此**才**写（[`Verdict::Keep`]），低于它就是 [`Verdict::Drop`]。
pub const KEEP_THRESHOLD: f64 = 0.25;
/// 退场阈值：保留度低于它时降一层（不删除）。
pub const FORGET_THRESHOLD: f64 = 0.15;
/// 衰减半衰期：7 天。保留度每过 7 天减半。
pub const DECAY_HALF_LIFE_SECS: i64 = 7 * 24 * 3600;
/// 用户显式要求记住（「记住这个」）的加分。
pub const EXPLICIT_BOOST: f64 = 0.20;

/// 待写入的条目（闸门的输入）。
///
/// `at` 是**决策时刻**而不是读墙钟：写入路径一旦读实时时钟，测试就只能靠
/// `sleep` 或放宽断言，「过期降级」这条规则就再也钉不住（同 `scoring` 里
/// 「阈值必须可被并排测量」的取舍）。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 冲突判定的键（如 `ops.db_url`）。`None` 表示这条不参与去重。
    pub key: Option<String>,
    /// 内容
    pub text: String,
    /// 主观重要度，0..=1
    pub importance: f64,
    /// 最后命中时间（unix 秒）
    pub last_hit: i64,
    /// 用户显式要求记住
    pub explicit: bool,
    /// 决策时刻（unix 秒）
    pub at: i64,
    /// 写入时所在层级（`Promote` 会把它抬到长时层）
    pub tier: MemoryTierVariant,
}

/// 准入结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 值得长期记住：写入并升到长时层
    Promote,
    /// 值得写，但保持原层级
    Keep,
    /// 不值得写
    Drop,
}

/// 准入分数（不含时间，见模块头「规则 1」）。
pub fn gate_score(entry: &Entry) -> f64 {
    let base = entry.importance.clamp(0.0, 1.0);
    if entry.explicit {
        (base + EXPLICIT_BOOST).min(1.0)
    } else {
        base
    }
}

/// 准入决策：阈值只在文件头部定义一次，这里只做比较。
pub fn decide(entry: &Entry) -> Verdict {
    let score = gate_score(entry);
    if score >= PROMOTE_THRESHOLD {
        Verdict::Promote
    } else if score >= KEEP_THRESHOLD {
        Verdict::Keep
    } else {
        Verdict::Drop
    }
}

/// 保留度：重要度 × 指数衰减（半衰期 [`DECAY_HALF_LIFE_SECS`]）。
///
/// 时间是**距最后命中**的秒数，不是创建时间：一条老记忆只要还在被命中，
/// 就不该退场 —— 这是「最后命中时间 × 重要度」的本意。
pub fn retention(importance: f64, last_hit: i64, at: i64) -> f64 {
    let age = at.saturating_sub(last_hit).max(0) as f64;
    importance.clamp(0.0, 1.0) * 0.5f64.powf(age / DECAY_HALF_LIFE_SECS as f64)
}

/// 退场结论。**没有「删除」分支**：过期只降级。
#[derive(Debug, Clone, PartialEq)]
pub enum ForgetOutcome {
    /// 保留度仍够，原位不动
    Keep,
    /// 降一层，连同保留度与原因一起留痕
    Demote {
        /// 原层级
        from: MemoryTierVariant,
        /// 降到的层级
        to: MemoryTierVariant,
        /// 决策时的保留度
        retention: f64,
        /// 人话原因（进日志 / 观测面）
        reason: String,
    },
}

/// 遗忘决策：低于 [`FORGET_THRESHOLD`] 只降一层，返回原因；地板层返回 `Keep`。
pub fn forget(tier: MemoryTierVariant, importance: f64, last_hit: i64, at: i64) -> ForgetOutcome {
    let kept = retention(importance, last_hit, at);
    if kept >= FORGET_THRESHOLD {
        return ForgetOutcome::Keep;
    }
    let to = match tier {
        MemoryTierVariant::LongTerm => MemoryTierVariant::ShortTerm,
        MemoryTierVariant::ShortTerm => MemoryTierVariant::Sensory,
        // 地板层没有更低层级：删除归调用方的 TTL，不归本闸门。
        MemoryTierVariant::Sensory => return ForgetOutcome::Keep,
    };
    let age = at.saturating_sub(last_hit).max(0);
    ForgetOutcome::Demote {
        from: tier,
        to,
        retention: kept,
        reason: format!(
            "保留度 {kept:.3} 低于 {FORGET_THRESHOLD:.3}（最后命中距今 {age} 秒，半衰期 {DECAY_HALF_LIFE_SECS}s），{} → {}",
            tier.label(),
            to.label()
        ),
    }
}

/// 库里的一条记录（与 [`Entry`] 的差别：它已有 id、层级可能被降过、可能被取代）。
#[derive(Debug, Clone)]
pub struct Record {
    /// 自增 id（写入顺序）
    pub id: u64,
    /// 冲突键
    pub key: Option<String>,
    /// 内容
    pub text: String,
    /// 主观重要度，0..=1
    pub importance: f64,
    /// 最后命中时间（unix 秒）
    pub last_hit: i64,
    /// 写入时刻（unix 秒）
    pub created_at: i64,
    /// **当前**层级（退场会就地降一层）
    pub tier: MemoryTierVariant,
    /// 同 key 冲突时被哪条新记录取代；有值即「留痕但不参与召回」
    pub superseded_by: Option<u64>,
    /// 最近一次降级落到的层级
    pub demoted_to: Option<MemoryTierVariant>,
    /// 最近一次降级的原因
    pub demote_reason: Option<String>,
}

/// 写入结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// 被闸门挡下（不落库）
    Dropped,
    /// 已写入，`verdict` 说明是 Promote 还是 Keep
    Written { id: u64, verdict: Verdict },
}

/// 写入闸门：内存态的策略台账，调用方据此决定真正往哪层落库。
#[derive(Debug)]
pub struct WriteGate {
    next_id: u64,
    records: Vec<Record>,
    promoted: u64,
    kept: u64,
    dropped: u64,
    superseded: u64,
}

impl Default for WriteGate {
    fn default() -> Self {
        Self::new()
    }
}

impl WriteGate {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            records: Vec::new(),
            promoted: 0,
            kept: 0,
            dropped: 0,
            superseded: 0,
        }
    }

    /// 写入一条：先过 [`decide`]，Promote 抬到长时层，同 key 则让旧记录被取代。
    ///
    /// 被挡下的条目不落库（[`WriteOutcome::Dropped`]），旧记录也不会被它取代 ——
    /// 「垃圾写入不得挤掉已有的好记忆」。
    pub fn write(&mut self, entry: &Entry) -> WriteOutcome {
        let verdict = decide(entry);
        match verdict {
            Verdict::Drop => {
                self.dropped += 1;
                return WriteOutcome::Dropped;
            }
            Verdict::Promote => self.promoted += 1,
            Verdict::Keep => self.kept += 1,
        }
        let id = self.next_id;
        self.next_id += 1;
        let tier = match verdict {
            Verdict::Promote => MemoryTierVariant::LongTerm,
            _ => entry.tier,
        };
        let mut replaced_old = false;
        if let Some(key) = entry.key.as_deref() {
            if let Some(old) = self
                .records
                .iter_mut()
                .rev()
                .find(|r| r.superseded_by.is_none() && r.key.as_deref() == Some(key))
            {
                old.superseded_by = Some(id);
                replaced_old = true;
            }
        }
        if replaced_old {
            self.superseded += 1;
        }
        self.records.push(Record {
            id,
            key: entry.key.clone(),
            text: entry.text.clone(),
            importance: entry.importance.clamp(0.0, 1.0),
            last_hit: entry.last_hit,
            created_at: entry.at,
            tier,
            superseded_by: None,
            demoted_to: None,
            demote_reason: None,
        });
        WriteOutcome::Written { id, verdict }
    }

    /// 过期扫描：按 `at` 判定所有**仍在召回中**的记录，返回本次降级条数。
    ///
    /// 由调用方按需周期调用（同 `long_term::consolidate` 的定位），不在每次
    /// [`write`](Self::write) 里顺带扫全表 —— 写入路径不该是 O(库大小)。
    /// 已被取代的记录不参与（它们只是历史留痕）。
    pub fn sweep(&mut self, at: i64) -> usize {
        let mut demoted = 0;
        for record in &mut self.records {
            if record.superseded_by.is_some() {
                continue;
            }
            if let ForgetOutcome::Demote { to, reason, .. } =
                forget(record.tier, record.importance, record.last_hit, at)
            {
                record.tier = to;
                record.demoted_to = Some(to);
                record.demote_reason = Some(reason);
                demoted += 1;
            }
        }
        demoted
    }

    /// 召回视图：只回当前有效（未被取代）的记录。检索实现不在这里。
    pub fn recall(&self) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|r| r.superseded_by.is_none())
            .collect()
    }

    /// 按 key 召回最新一条（同 key 冲突后只回新值）。
    pub fn recall_by_key(&self, key: &str) -> Option<&Record> {
        self.records
            .iter()
            .rev()
            .find(|r| r.superseded_by.is_none() && r.key.as_deref() == Some(key))
    }

    /// 全量记录（含被取代的留痕）——观测/排查用。
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// 被晋升的条数。
    pub fn promoted(&self) -> u64 {
        self.promoted
    }

    /// 原层级保留的条数。
    pub fn kept(&self) -> u64 {
        self.kept
    }

    /// 被闸门挡下的条数。
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 因同 key 冲突被取代的条数。
    pub fn superseded(&self) -> u64 {
        self.superseded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 准入的下界：低分条目连库都进不去。
    #[test]
    fn a_low_score_candidate_is_dropped() {
        let low = Entry {
            key: Some("billing.total".into()),
            text: "随口一句无关紧要的话".into(),
            importance: 0.1,
            last_hit: 0,
            explicit: false,
            at: 0,
            tier: MemoryTierVariant::ShortTerm,
        };
        assert_eq!(decide(&low), Verdict::Drop, "0.1 分属于噪声档");

        let mut gate = WriteGate::new();
        assert_eq!(gate.write(&low), WriteOutcome::Dropped);
        assert_eq!(
            (
                gate.promoted(),
                gate.kept(),
                gate.dropped(),
                gate.superseded()
            ),
            (0, 0, 1, 0)
        );
        assert!(gate.recall().is_empty(), "被挡下的条目不落库");
        assert!(gate.records().is_empty());
    }

    /// 准入的上界：高分条目写入并抬到长时层；中间档只写不抬。
    #[test]
    fn a_high_score_candidate_is_promoted() {
        let high = Entry {
            key: Some("ops.deploy_convention".into()),
            text: "生产发布只用 canary，不许直接全量".into(),
            importance: 0.9,
            last_hit: 0,
            explicit: false,
            at: 0,
            tier: MemoryTierVariant::ShortTerm,
        };
        assert_eq!(decide(&high), Verdict::Promote);

        let mid = Entry {
            importance: 0.5,
            ..high.clone()
        };
        assert_eq!(decide(&mid), Verdict::Keep, "中间档写但保持原层级");

        let mut gate = WriteGate::new();
        match gate.write(&high) {
            WriteOutcome::Written { id, verdict } => {
                assert_eq!(id, 1);
                assert_eq!(verdict, Verdict::Promote);
            }
            WriteOutcome::Dropped => panic!("高分条目不该被挡下"),
        }
        assert_eq!(gate.promoted(), 1);
        let active = gate.recall();
        assert_eq!(active.len(), 1);
        assert_eq!(
            active[0].tier,
            MemoryTierVariant::LongTerm,
            "Promote 必须真把层级抬上去"
        );
    }

    /// 退场：超过衰减线只降一层、留原因，记录**不删除**。
    #[test]
    fn an_expired_entry_is_demoted_not_deleted() {
        let day = 24 * 3600;
        let born = 1_000_000;
        let mut gate = WriteGate::new();
        let stale = Entry {
            key: None,
            text: "已过时的构建约定".into(),
            importance: 0.5,
            last_hit: born,
            explicit: false,
            at: born,
            tier: MemoryTierVariant::LongTerm,
        };
        let id = match gate.write(&stale) {
            WriteOutcome::Written { id, .. } => id,
            WriteOutcome::Dropped => panic!("0.5 分条目应当写进去"),
        };

        let later = born + 30 * day;
        assert!(
            retention(stale.importance, stale.last_hit, later) < FORGET_THRESHOLD,
            "夹具前提：30 个半衰期后保留度必须低于退场线"
        );
        assert_eq!(gate.sweep(later), 1, "过期记录应当被降级一次");

        let record = gate.records().iter().find(|r| r.id == id).unwrap();
        assert_eq!(record.demoted_to, Some(MemoryTierVariant::ShortTerm));
        assert_eq!(record.tier, MemoryTierVariant::ShortTerm);
        assert!(
            record
                .demote_reason
                .as_deref()
                .unwrap_or("")
                .contains("保留度"),
            "降级必须留下原因：{:?}",
            record.demote_reason
        );
        assert_eq!(gate.records().len(), 1, "降级不是删除：记录仍在库里");
        assert_eq!(gate.recall().len(), 1, "降级后的记录仍在召回视图里");
    }

    /// 同 key 冲突：召回只回新值，旧值不删、只标 `superseded_by`。
    #[test]
    fn a_same_key_conflict_keeps_the_new_value_and_hides_the_old() {
        let old = Entry {
            key: Some("ops.db_url".into()),
            text: "postgres://old-cluster/marguerite".into(),
            importance: 0.8,
            last_hit: 0,
            explicit: false,
            at: 0,
            tier: MemoryTierVariant::LongTerm,
        };
        let new = Entry {
            key: Some("ops.db_url".into()),
            text: "postgres://new-cluster/marguerite".into(),
            importance: 0.8,
            last_hit: 0,
            explicit: false,
            at: 0,
            tier: MemoryTierVariant::LongTerm,
        };

        let mut gate = WriteGate::new();
        let WriteOutcome::Written { id: id_old, .. } = gate.write(&old) else {
            panic!("旧值应当先写进去");
        };
        let WriteOutcome::Written { id: id_new, .. } = gate.write(&new) else {
            panic!("新值应当写进去");
        };
        assert_eq!(gate.superseded(), 1);

        let active = gate.recall();
        assert_eq!(active.len(), 1, "同 key 只回一条");
        assert_eq!(active[0].id, id_new);
        assert_eq!(active[0].text, "postgres://new-cluster/marguerite");
        assert_eq!(
            gate.recall_by_key("ops.db_url")
                .expect("新值必须按 key 召回")
                .text,
            "postgres://new-cluster/marguerite"
        );

        let stored_old = gate.records().iter().find(|r| r.id == id_old).unwrap();
        assert_eq!(
            stored_old.superseded_by,
            Some(id_new),
            "旧值不删，只留 superseded_by 供追溯"
        );
        assert_eq!(gate.records().len(), 2, "旧记录必须仍然在库里");
    }
}
