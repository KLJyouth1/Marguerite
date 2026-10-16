//! 联想记忆网络（突触图）
//!
//! ## 诚实边界（务必先读）
//!
//! 本模块**不是**深度神经网络，也不做任何「智能涌现」宣称。它是一个
//! **Hebbian 式联想记忆图**：节点 = 记忆条目 id，边 = 同批被召回的共现权重
//! （"一起被想起的东西，下次更容易一起被想起"）。打分是确定性的**扩散激活**
//! （spreading activation，一跳/两跳求和 + 归一），仅用于召回结果重排。
//!
//! ## 设计约束
//!
//! - **纯确定性**：无随机数、**不读系统时钟**（`now_ms` 一律由调用方传入；既作节点时间记录，
//!   也作时间驱动衰减的判据 —— 见 [`SynapseGraph::maybe_decay_by_elapsed`]）。
//! - **无 panic**：全部用 `saturating` / `Result`，坏数据回退空图而非崩溃。
//! - **隐私**：节点 id 由调用方提供（调用方用哈希 id，绝不落记忆正文）。
//! - **原子持久化**：临时文件 + rename，UTF-8 无 BOM；解析失败回退空图。

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// 单条边权重上限（饱和加法：反复强化也不会溢出/发散）。
pub const MAX_WEIGHT: f64 = 8.0;
/// 每次共召回强化的权重增量。
pub const REINFORCE_STEP: f64 = 1.0;
/// 衰减低于该权重的边即剪枝。
pub const MIN_WEIGHT: f64 = 0.05;
/// 二跳激活的衰减系数（比一跳弱，符合"越想越远越模糊"的直觉）。
pub const ACTIVATION_HOP2_FACTOR: f64 = 0.5;
/// 时间驱动衰减的最小间隔（毫秒）：间隔不足此值不衰减。
pub const DECAY_INTERVAL_MS: u64 = 6 * 60 * 60 * 1000;
/// 每经过一个 [`DECAY_INTERVAL_MS`]，权重乘以该系数（幂次施加）。
pub const DECAY_FACTOR_PER_INTERVAL: f64 = 0.9;
/// `stats()` 返回的 top 边条数上限。
pub const TOP_EDGES: usize = 10;

/// 一条边（供 `stats()` / RPC 暴露；仅 id + 权重，**不含任何记忆正文**）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynapseEdge {
    /// 端点 a（字典序较小者）
    pub a: String,
    /// 端点 b（字典序较大者）
    pub b: String,
    /// 边权重
    pub weight: f64,
}

/// 突触图可观测统计。
#[derive(Debug, Clone, PartialEq)]
pub struct SynapseStats {
    /// 节点数（已知记忆条目 id 数）
    pub nodes: usize,
    /// 无向边数
    pub edges: usize,
    /// 累计强化的共现对数
    pub reinforced: u64,
    /// 累计被剪枝的边数
    pub decayed: u64,
    /// 累计扩散激活调用次数
    pub activations: u64,
    /// 平均边权重（无边时为 0.0）
    pub average_weight: f64,
    /// 权重最高的前 [`TOP_EDGES`] 条边（降序，含 id 与权重）
    pub top_edges: Vec<SynapseEdge>,
}

/// 联想记忆图（Hebbian 共现图，非神经网络）。
///
/// 内部用原子计数支持 `activate(&self, ..)` 这类只读方法也能累计调用次数；
/// 结构本身无随机、无内部时钟。
#[derive(Debug, Default)]
pub struct SynapseGraph {
    /// 邻接表：节点 id → { 邻居 id → 权重 }（无向边两侧对称登记）
    adj: HashMap<String, HashMap<String, f64>>,
    /// 节点最近一次被强化时的时间戳（由调用方传入，纯记录，不参与打分）
    last_seen_ms: HashMap<String, u64>,
    reinforced: AtomicU64,
    decayed: AtomicU64,
    activations: AtomicU64,
    /// 上次**时间驱动**衰减的时间戳（毫秒；0 = 尚未建立基准）。
    last_decay_ms: AtomicU64,
}

/// 落盘载荷（可序列化视图）。原子写 + rename，UTF-8 无 BOM。
#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedSynapse {
    /// 无向边（`a < b` 字典序），权重
    #[serde(default)]
    edges: Vec<(String, String, f64)>,
    #[serde(default)]
    reinforced: u64,
    #[serde(default)]
    decayed: u64,
    #[serde(default)]
    activations: u64,
    /// 上次时间驱动衰减的时间戳（毫秒）。旧盘无此字段 → 0（首启建立基准，不衰减）。
    #[serde(default)]
    last_decay_ms: u64,
}

/// 保留小数位（仅用于展示/持久化的稳定化，避免浮点尾差）。
fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

impl SynapseGraph {
    /// 空图。
    pub fn new() -> Self {
        Self::default()
    }

    /// 同批召回的条目两两强化（Hebbian）。
    ///
    /// - 自环不计（去重后不会产生 `a==b`；仍显式防御）。
    /// - 饱和加法：权重不超过 [`MAX_WEIGHT`]。
    /// - 返回本次强化的无向共现对数。
    pub fn reinforce(&mut self, ids: &[String], now_ms: u64) -> usize {
        // 去重保序（确定性）
        let mut uniq: Vec<&String> = Vec::with_capacity(ids.len());
        for id in ids {
            if !uniq.contains(&id) {
                uniq.push(id);
            }
        }
        // 已知节点登记（孤立节点也记账，便于统计"见过的条目数"）
        for id in &uniq {
            self.adj.entry((*id).clone()).or_default();
            self.last_seen_ms.insert((*id).clone(), now_ms);
        }
        let mut pairs = 0usize;
        for i in 0..uniq.len() {
            for j in (i + 1)..uniq.len() {
                let (a, b) = (uniq[i], uniq[j]);
                if a == b {
                    continue; // 自环不计
                }
                self.bump(a, b);
                self.bump(b, a);
                pairs += 1;
            }
        }
        self.reinforced.fetch_add(pairs as u64, Ordering::Relaxed);
        pairs
    }

    /// 单侧加边 + 饱和加法。
    fn bump(&mut self, a: &str, b: &str) {
        let entry = self.adj.entry(a.to_string()).or_default();
        let w = entry.entry(b.to_string()).or_insert(0.0);
        if !w.is_finite() {
            *w = MAX_WEIGHT;
            return;
        }
        *w = (*w + REINFORCE_STEP).min(MAX_WEIGHT);
    }

    /// 扩散激活打分（确定性）：以 `seeds` 为起点做一跳/两跳加权扩散，权重归一。
    ///
    /// - 一跳：邻居得分 = 边权重之和。
    /// - 二跳：经一跳节点再扩散，衰减 [`ACTIVATION_HOP2_FACTOR`]。
    /// - 归一：除以最大得分 → 落在 `0..=1`；结果按得分降序，同分按 id 字典序稳定排列。
    /// - 排除种子自身；`limit` 截断。
    pub fn activate(&self, seeds: &[String], limit: usize) -> Vec<(String, f64)> {
        self.activations.fetch_add(1, Ordering::Relaxed);
        if limit == 0 || seeds.is_empty() {
            return Vec::new();
        }
        let seed_set: HashSet<&str> = seeds.iter().map(String::as_str).collect();
        let mut scores: HashMap<&str, f64> = HashMap::new();

        // 一跳（邻居按 id 排序 → 浮点累加顺序确定，保证同输入同输出）
        for s in seeds {
            if let Some(nbrs) = self.adj.get(s) {
                let mut ns: Vec<(&String, &f64)> = nbrs.iter().collect();
                ns.sort_by(|x, y| x.0.cmp(y.0));
                for (n, w) in ns {
                    *scores.entry(n.as_str()).or_insert(0.0) += *w;
                }
            }
        }

        // 二跳
        if !scores.is_empty() {
            let mut hop1: Vec<(&str, f64)> = scores.iter().map(|(k, v)| (*k, *v)).collect();
            hop1.sort_by(|x, y| x.0.cmp(y.0));
            for (n, base) in hop1 {
                let Some(nbrs) = self.adj.get(n) else { continue };
                let mut ns: Vec<(&String, &f64)> = nbrs.iter().collect();
                ns.sort_by(|x, y| x.0.cmp(y.0));
                let base_norm = (base / MAX_WEIGHT).min(1.0);
                for (m, w) in ns {
                    if seed_set.contains(m.as_str()) {
                        continue; // 不回注种子
                    }
                    *scores.entry(m.as_str()).or_insert(0.0) +=
                        ACTIVATION_HOP2_FACTOR * *w * base_norm;
                }
            }
        }

        let max = scores.values().copied().fold(0.0f64, f64::max);
        let mut out: Vec<(String, f64)> = scores
            .into_iter()
            .filter(|(k, _)| !seed_set.contains(*k))
            .map(|(k, v)| {
                (
                    k.to_string(),
                    if max > 0.0 { (v / max).clamp(0.0, 1.0) } else { 0.0 },
                )
            })
            .collect();
        out.sort_by(|x, y| {
            y.1.partial_cmp(&x.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| x.0.cmp(&y.0))
        });
        out.truncate(limit);
        out
    }

    /// 周期性衰减：所有权重乘以 `factor`（夹紧到 `0..=1`），低于 [`MIN_WEIGHT`] 的边剪枝，
    /// 随之删除孤立节点。返回被剪枝的边数。
    pub fn decay(&mut self, factor: f64) -> usize {
        let f = if factor.is_finite() {
            factor.clamp(0.0, 1.0)
        } else {
            1.0
        };
        for nbrs in self.adj.values_mut() {
            for w in nbrs.values_mut() {
                *w *= f;
            }
        }
        let keys: Vec<String> = self.adj.keys().cloned().collect();
        let mut pruned = 0usize;
        for a in &keys {
            let mut drop_b: Vec<String> = Vec::new();
            if let Some(nbrs) = self.adj.get(a) {
                for (b, w) in nbrs {
                    if *w < MIN_WEIGHT {
                        drop_b.push(b.clone());
                    }
                }
            }
            for b in &drop_b {
                if let Some(nbrs) = self.adj.get_mut(a) {
                    nbrs.remove(b);
                }
                if let Some(nbrs) = self.adj.get_mut(b) {
                    nbrs.remove(a);
                }
                pruned += 1;
            }
        }
        // 孤立节点清理（无边的节点不再保留）
        self.adj.retain(|_, nbrs| !nbrs.is_empty());
        let alive: HashSet<String> = self.adj.keys().cloned().collect();
        self.last_seen_ms.retain(|k, _| alive.contains(k));
        self.decayed.fetch_add(pruned as u64, Ordering::Relaxed);
        pruned
    }

    /// **时间驱动**的周期性衰减：按距上次衰减经过的**完整间隔数**施加幂次衰减。
    ///
    /// 历史缺陷：[`Self::decay`] 只有测试调用 —— 生产环境里权重**只增不减**，
    /// 老边永不遗忘，图随时间无界膨胀。本方法由召回接入点调用
    /// （见 `memory::cross_project::integrate_synapse`）。
    ///
    /// - 首次调用（`last_decay_ms == 0`）**仅建立基准**，不衰减 —— 避免「上线即清空」。
    /// - 时间回退（`now_ms <= last`）不做任何事（防御性）。
    /// - 返回被剪枝的边数。
    pub fn maybe_decay_by_elapsed(&mut self, now_ms: u64) -> usize {
        let last = self.last_decay_ms.load(Ordering::Relaxed);
        if last == 0 {
            self.last_decay_ms.store(now_ms.max(1), Ordering::Relaxed);
            return 0;
        }
        if now_ms <= last {
            return 0;
        }
        let intervals = (now_ms - last) / DECAY_INTERVAL_MS;
        if intervals == 0 {
            return 0;
        }
        // 幂次衰减；间隔数上限 64，避免 `powi` 溢出（factor 迅速趋近 0 已足够）。
        let factor = DECAY_FACTOR_PER_INTERVAL.powi(intervals.min(64) as i32);
        let pruned = self.decay(factor);
        self.last_decay_ms.store(now_ms, Ordering::Relaxed);
        pruned
    }

    /// 当前统计快照（确定性的边排序；top 边按权重降序、同分按 id 稳定）。
    pub fn stats(&self) -> SynapseStats {
        let mut edge_count = 0usize;
        let mut sum = 0.0f64;
        let mut edges: Vec<SynapseEdge> = Vec::new();
        for (a, nbrs) in &self.adj {
            for (b, w) in nbrs {
                if a < b {
                    edge_count += 1;
                    sum += *w;
                    edges.push(SynapseEdge {
                        a: a.clone(),
                        b: b.clone(),
                        weight: round4(*w),
                    });
                }
            }
        }
        edges.sort_by(|x, y| {
            y.weight
                .partial_cmp(&x.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| x.a.cmp(&y.a))
                .then_with(|| x.b.cmp(&y.b))
        });
        edges.truncate(TOP_EDGES);
        SynapseStats {
            nodes: self.adj.len(),
            edges: edge_count,
            reinforced: self.reinforced.load(Ordering::Relaxed),
            decayed: self.decayed.load(Ordering::Relaxed),
            activations: self.activations.load(Ordering::Relaxed),
            average_weight: if edge_count == 0 {
                0.0
            } else {
                round4(sum / edge_count as f64)
            },
            top_edges: edges,
        }
    }

    /// 折叠为落盘载荷（边按 `(a,b)` 字典序稳定排列）。
    fn to_persisted(&self) -> PersistedSynapse {
        let mut edges: Vec<(String, String, f64)> = Vec::new();
        for (a, nbrs) in &self.adj {
            for (b, w) in nbrs {
                if a < b {
                    edges.push((a.clone(), b.clone(), *w));
                }
            }
        }
        edges.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
        PersistedSynapse {
            edges,
            reinforced: self.reinforced.load(Ordering::Relaxed),
            decayed: self.decayed.load(Ordering::Relaxed),
            activations: self.activations.load(Ordering::Relaxed),
            last_decay_ms: self.last_decay_ms.load(Ordering::Relaxed),
        }
    }

    /// 由落盘载荷重建（含孤立节点无信息，故仅由边重建邻接）。
    fn from_persisted(p: PersistedSynapse) -> Self {
        let mut g = Self::new();
        for (a, b, w) in p.edges {
            if a == b || !w.is_finite() || w <= 0.0 {
                continue;
            }
            let w = w.min(MAX_WEIGHT);
            g.adj.entry(a.clone()).or_default().insert(b.clone(), w);
            g.adj.entry(b).or_default().insert(a, w);
        }
        g.reinforced.store(p.reinforced, Ordering::Relaxed);
        g.decayed.store(p.decayed, Ordering::Relaxed);
        g.activations.store(p.activations, Ordering::Relaxed);
        g.last_decay_ms.store(p.last_decay_ms, Ordering::Relaxed);
        g
    }

    /// 原子落盘（临时文件 + rename；UTF-8 无 BOM）。父目录不存在则创建。
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let payload = serde_json::to_string(&self.to_persisted())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, payload.as_bytes())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 从 JSON 加载（原子写的产物）。文件缺失 / 解析失败 / 读取失败一律回退空图，**绝不 panic**。
    pub fn open_from(path: &Path) -> Self {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Self::new(),
        };
        // 容忍 UTF-8 BOM（非本模块写出，但外部可能注入）
        let body = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        match serde_json::from_str::<PersistedSynapse>(body) {
            Ok(p) => Self::from_persisted(p),
            Err(_) => Self::new(),
        }
    }
}

/// 联想记忆网络默认落盘路径（`data_dir/memory/synapse.json`）。
pub fn synapse_path() -> PathBuf {
    crate::persistence::config::Config::data_dir()
        .join("memory")
        .join("synapse.json")
}

/// 是否启用联想记忆网络（配置项默认开启；读取失败也视为开启，行为与默认一致）。
pub fn synapse_enabled() -> bool {
    crate::persistence::config::Config::load()
        .map(|c| c.memory.synapse_enabled)
        .unwrap_or(true)
}

/// 进程级突触图单例（首次访问时从默认路径加载）。
pub fn global_graph() -> Arc<Mutex<SynapseGraph>> {
    static G: OnceLock<Arc<Mutex<SynapseGraph>>> = OnceLock::new();
    G.get_or_init(|| Arc::new(Mutex::new(SynapseGraph::open_from(&synapse_path()))))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "marg_synapse_{tag}_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn reinforce_creates_bidirectional_edges() {
        let mut g = SynapseGraph::new();
        let pairs = g.reinforce(&ids(&["a", "b", "c"]), 100);
        assert_eq!(pairs, 3, "3 条目两两 = 3 对");
        let s = g.stats();
        assert_eq!(s.nodes, 3);
        assert_eq!(s.edges, 3);
        assert_eq!(s.reinforced, 3);
        // 两侧对称登记 → activate 从任一端都能找到对方
        let from_a = g.activate(&ids(&["a"]), 10);
        assert!(from_a.iter().any(|(id, _)| id == "b"));
        assert!(from_a.iter().any(|(id, _)| id == "c"));
    }

    #[test]
    fn reinforce_excludes_self_loops_for_duplicate_ids() {
        let mut g = SynapseGraph::new();
        // 重复 id 去重后只剩一个 → 无自环、无对
        let pairs = g.reinforce(&ids(&["x", "x", "x"]), 1);
        assert_eq!(pairs, 0, "自环不计");
        assert!(g.activate(&ids(&["x"]), 10).is_empty());
        let s = g.stats();
        assert_eq!(s.edges, 0);
        assert_eq!(s.nodes, 1, "孤立节点仍登记");
    }

    #[test]
    fn reinforce_saturates_at_max_weight() {
        let mut g = SynapseGraph::new();
        let pair = ids(&["a", "b"]);
        for i in 0..(MAX_WEIGHT as usize + 20) {
            g.reinforce(&pair, i as u64);
        }
        let s = g.stats();
        assert_eq!(s.edges, 1);
        assert!(
            (s.average_weight - MAX_WEIGHT).abs() < 1e-9,
            "权重应饱和在 MAX_WEIGHT，实际 {}",
            s.average_weight
        );
        assert!(s.average_weight.is_finite());
    }

    #[test]
    fn activate_one_hop_ranks_stronger_neighbor_first() {
        let mut g = SynapseGraph::new();
        // a-b 共现 3 次（权重 3），a-c 共现 1 次（权重 1）
        for _ in 0..3 {
            g.reinforce(&ids(&["a", "b"]), 0);
        }
        g.reinforce(&ids(&["a", "c"]), 0);
        let out = g.activate(&ids(&["a"]), 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "b", "更强邻居应排前: {out:?}");
        assert!(out[0].1 >= out[1].1);
        // 归一：最强 = 1.0
        assert!((out[0].1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn activate_two_hop_spreads_with_lower_weight() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b"]), 0);
        g.reinforce(&ids(&["b", "c"]), 0);
        // 从 a 出发：b 一跳（权重 1），c 二跳（0.5 × ...）
        let out = g.activate(&ids(&["a"]), 10);
        let b = out.iter().find(|(id, _)| id == "b").expect("b 一跳");
        let c = out.iter().find(|(id, _)| id == "c").expect("c 二跳");
        assert!(b.1 >= c.1, "一跳应不弱于二跳: b={}, c={}", b.1, c.1);
    }

    #[test]
    fn activate_excludes_seeds_and_truncates() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["s", "n1", "n2", "n3"]), 0);
        let out = g.activate(&ids(&["s"]), 2);
        assert_eq!(out.len(), 2, "limit 生效");
        assert!(
            out.iter().all(|(id, _)| id != "s"),
            "种子自身必须排除: {out:?}"
        );
    }

    #[test]
    fn activate_empty_seeds_returns_empty() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b"]), 0);
        assert!(g.activate(&[], 10).is_empty());
        assert!(g.activate(&ids(&["a"]), 0).is_empty(), "limit=0");
        // 未知种子的邻居查询为空但不 panic
        assert!(g.activate(&ids(&["zzz"]), 10).is_empty());
    }

    #[test]
    fn decay_multiplies_and_prunes_below_threshold() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b"]), 0); // 权重 1.0
        let before = g.stats().average_weight;
        let pruned = g.decay(0.5);
        assert_eq!(pruned, 0, "0.5 仍高于阈值");
        let after = g.stats().average_weight;
        assert!((after - before * 0.5).abs() < 1e-6, "权重应减半: {before} → {after}");
        // 再衰减多次 → 低于阈值被剪枝
        let mut total_pruned = 0usize;
        for _ in 0..10 {
            total_pruned += g.decay(0.5);
        }
        assert!(total_pruned >= 1, "持续衰减应剪枝");
        assert_eq!(g.stats().edges, 0, "边被剪光");
        assert_eq!(g.stats().nodes, 0, "孤立节点被清理");
        assert!(g.stats().decayed >= 1);
    }

    /// 时间驱动衰减：首启建立基准不衰减；不足一周期 no-op；跨周期幂次衰减；时间回退 no-op。
    #[test]
    fn maybe_decay_by_elapsed_is_time_driven_and_baselined() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b"]), 1); // 权重 1.0
        let before = g.stats().average_weight;

        // 首启：仅建立基准，不衰减（避免「上线即清空」）
        assert_eq!(g.maybe_decay_by_elapsed(1_000), 0);
        assert!(
            (g.stats().average_weight - before).abs() < 1e-9,
            "首启不应衰减"
        );

        // 不足一个周期：no-op
        assert_eq!(g.maybe_decay_by_elapsed(1_000 + DECAY_INTERVAL_MS - 1), 0);
        assert!(
            (g.stats().average_weight - before).abs() < 1e-9,
            "未满周期不应衰减"
        );

        // 恰好一个周期：× 0.9
        g.maybe_decay_by_elapsed(1_000 + DECAY_INTERVAL_MS);
        let after1 = g.stats().average_weight;
        assert!(
            (after1 - before * DECAY_FACTOR_PER_INTERVAL).abs() < 1e-6,
            "{before} → {after1}（应 ×{}）",
            DECAY_FACTOR_PER_INTERVAL
        );

        // 再跨两个周期（合计 3）：× 0.9^3
        g.maybe_decay_by_elapsed(1_000 + DECAY_INTERVAL_MS * 3);
        let after3 = g.stats().average_weight;
        assert!(
            (after3 - before * DECAY_FACTOR_PER_INTERVAL.powi(3)).abs() < 1e-6,
            "{before} → {after3}（应 ×0.9^3）"
        );

        // 时间回退：no-op（防御性）
        let frozen = g.stats().average_weight;
        assert_eq!(g.maybe_decay_by_elapsed(0), 0);
        assert!(
            (g.stats().average_weight - frozen).abs() < 1e-9,
            "时间回退不应衰减"
        );
    }

    /// 衰减基准随图一起持久化（跨重启不会因基准丢失而重复/漏衰减）。
    #[test]
    fn last_decay_ms_survives_roundtrip() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b"]), 1);
        g.maybe_decay_by_elapsed(5_000); // 建立基准
        let path = tmp_path("lastdecay");
        g.save_to(&path).expect("落盘");

        let mut loaded = SynapseGraph::open_from(&path);
        // 载入后基准仍生效：紧邻时刻的调用是 no-op（不会因基准归零而重新「首启」）
        let w = loaded.stats().average_weight;
        assert_eq!(loaded.maybe_decay_by_elapsed(5_001), 0);
        assert!((loaded.stats().average_weight - w).abs() < 1e-9);
        // 但跨一个周期后仍会正常衰减
        loaded.maybe_decay_by_elapsed(5_000 + DECAY_INTERVAL_MS);
        assert!(loaded.stats().average_weight < w, "跨周期应衰减");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn persistence_roundtrip_preserves_graph_and_counters() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b", "c"]), 42);
        let _ = g.activate(&ids(&["a"]), 5);
        let path = tmp_path("roundtrip");
        g.save_to(&path).expect("落盘");
        // 原子写：临时文件不应残留
        assert!(!path.with_extension("json.tmp").exists(), "不应残留 .tmp");

        let loaded = SynapseGraph::open_from(&path);
        let a = g.stats();
        let b = loaded.stats();
        assert_eq!(a.nodes, b.nodes);
        assert_eq!(a.edges, b.edges);
        assert_eq!(a.reinforced, b.reinforced);
        assert_eq!(a.activations, b.activations);
        assert_eq!(a.top_edges, b.top_edges);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_from_bad_json_falls_back_to_empty_graph() {
        let path = tmp_path("badjson");
        std::fs::write(&path, b"{ this is not json ").unwrap();
        let g = SynapseGraph::open_from(&path);
        assert_eq!(g.stats().nodes, 0);
        assert_eq!(g.stats().edges, 0);
        // 缺失文件同样回退空图
        let missing = SynapseGraph::open_from(&tmp_path("missing"));
        assert_eq!(missing.stats().nodes, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn confirm_determinism_same_input_same_output() {
        let mut g = SynapseGraph::new();
        g.reinforce(&ids(&["a", "b", "c", "d"]), 7);
        g.reinforce(&ids(&["b", "c"]), 8);
        let r1 = g.activate(&ids(&["a"]), 5);
        let r2 = g.activate(&ids(&["a"]), 5);
        assert_eq!(r1, r2, "同输入两次激活结果必须逐位一致");
        let s1 = g.stats();
        let s2 = g.stats();
        assert_eq!(s1.top_edges, s2.top_edges);
        assert_eq!(s1.average_weight, s2.average_weight);
    }

    #[test]
    fn empty_input_and_stats_are_safe() {
        let mut g = SynapseGraph::new();
        assert_eq!(g.reinforce(&[], 0), 0);
        assert!(g.activate(&[], 10).is_empty());
        assert_eq!(g.decay(0.5), 0);
        let s = g.stats();
        assert_eq!(s.nodes, 0);
        assert_eq!(s.edges, 0);
        assert_eq!(s.average_weight, 0.0);
        assert!(s.top_edges.is_empty());
    }

    #[test]
    fn stats_top_edges_sorted_desc_and_capped() {
        let mut g = SynapseGraph::new();
        // 造一个 fan 图：hub 与 15 个 leaf 两两成边
        let mut owned = vec!["hub".to_string()];
        for i in 0..15 {
            owned.push(format!("leaf{i:02}"));
        }
        g.reinforce(&owned, 0);
        let s = g.stats();
        assert!(s.top_edges.len() <= TOP_EDGES, "top 边上限");
        for w in s.top_edges.windows(2) {
            assert!(w[0].weight >= w[1].weight, "权重降序");
        }
    }
}