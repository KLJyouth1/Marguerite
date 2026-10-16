//! KV-Cache 极简工程实践：客户端前缀/语义缓存与命中率计量。
//!
//! 设计原则（源自 KV Cache 研究，详见 `deliverables/KV-CACHE-OPTIMIZATION-BLUEPRINT.md`）：
//!
//! 1. **缓存不变前缀**。系统提示 / 人设 / 静态知识库是请求中稳定的部分，应作为缓存键的
//!    前缀；时间戳、会话 id、动态采样等易变数据必须放在后缀，否则前缀永远不命中。
//! 2. **精确匹配优先**。先走 hash 精确命中（诚实、零假阳性）；语义层作为可选增强，不喧宾夺主。
//! 3. **命中率可观测**。没有 hit-rate 指标的缓存是黑箱——本模块内置 [`CacheStats`]，
//!    并把"省下的 prompt token"如实计入用量台账，绝不虚报节省。
//!
//! 本模块零外部依赖（仅 `std`），可直接被 `model::llm` 与上层调度复用。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub mod semantic;
pub mod hitcache;
pub mod selfcheck;

/// 进程级多层语义缓存接驳位（Task #5）。
///
/// 设计要点：本进程把 `MultiTierCache` 的 Tier 0 绑定到调用方共享的 `Arc<PrefixCache>`
/// （网关路径即 `gateway_prefix_cache()`），使三层缓存与既有精确缓存**同源同对象**：
/// - 精确命中由共享 `PrefixCache` 统一计量（不重复计数）；
/// - `model::llm` 在精确未命中后，用 [`semantic::TierPolicy`] 逐层短路补命中；
/// - 落盘由 `install_semantic_layer` 的调用方负责（避免双写）。
///
/// 默认开启 Tier 1（规范化精确，假阳性极低）；Tier 2（语义近似）默认关闭，需显式
/// `with_semantic_enabled(true)`，呼应 CHANGELOG「100% 缓存命中率」但**绝不冒充**——
/// 命中率由 [`semantic::MultiTierStats`] 诚实统计，UI 如实展示三层来源。
static SEMANTIC_LAYER: OnceLock<Arc<semantic::MultiTierCache>> = OnceLock::new();

/// 安装进程级多层语义缓存（网关冷启动时调用一次）。`exact` 为共享精确缓存（如网关前缀缓存），
/// 可选 `restore_path` 用于从磁盘恢复 Tier 1/2（不覆盖共享的 Tier 0）。已安装则幂等返回既有实例。
///
/// 嵌入器走 [`semantic::ProviderEmbedder`]：真实后端由 `EmbeddingProvider::configure`
/// （读 `config.embeddings.backend`）在启动时定好，故 `backend=model` + `real-embed` 时
/// 语义层**真的**用真实模型向量；默认仍是零依赖本地后端，行为不变。
/// （此前这里写死 `LocalNgramEmbedder`，于是配置了真实嵌入后端也不会被语义缓存使用。）
///
/// Tier 2（语义近似）按 `MARGUERITE_SEMANTIC_CACHE=on` **显式选入**：不设时保持
/// 「精确 + 规范化精确」的零近似假阳性契约不变。
pub fn install_semantic_layer(
    exact: Arc<PrefixCache>,
    restore_path: Option<&std::path::Path>,
) -> &'static Arc<semantic::MultiTierCache> {
    SEMANTIC_LAYER.get_or_init(|| {
        let layer = match restore_path {
            Some(p) => semantic::MultiTierCache::open_with_shared_exact(
                p,
                exact,
                Box::new(semantic::ProviderEmbedder::new()),
                0.92,
            ),
            None => semantic::MultiTierCache::with_shared_exact(
                exact,
                1024,
                Box::new(semantic::ProviderEmbedder::new()),
                0.92,
            ),
        };
        let layer = layer.with_semantic_enabled(semantic::tier2_opted_in_from_env());
        Arc::new(layer)
    })
}

/// 取进程级多层语义缓存（未安装返回 `None`）。`model::llm` 据此判断是否接驳三层缓存。
#[must_use]
pub fn semantic_layer() -> Option<&'static Arc<semantic::MultiTierCache>> {
    SEMANTIC_LAYER.get()
}

/// 生产路径的进程级默认精确缓存（惰性单例），也是 `cache.status` 观测聚合所读的句柄。
///
/// 与 `default_prefix_cache()` 分离的原因：`cargo test` 在同一进程内并行跑上千个用例，
/// 若测试也共享同一份缓存，用例之间会互相污染——例如某用例写入的响应被另一用例命中，
/// 使后者「网络失败必须报错」的断言失效（已实测复现）。因此测试构建下
/// `default_prefix_cache()` 保持「每次新建」的历史隔离语义，而本函数始终编译：
/// 生产路径用它实现跨轮共享，观测端点用它聚合进程级真实指标，单测可直接验证它。
pub(crate) fn process_prefix_cache() -> Arc<PrefixCache> {
    static CACHE: OnceLock<Arc<PrefixCache>> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(PrefixCache::new(2048))).clone()
}

/// 进程级**默认精确前缀缓存**：网关路径已安装语义层时，直接复用其 Tier 0（即
/// `install_semantic_layer` 传入的共享缓存）；未安装（CLI / TUI）时退化为
/// **进程级单例**——否则 `ModelRouter::new` 每次拿到新实例，各组件各自持有独立缓存表，
/// 键虽稳定、跨轮查找却落在不同表里，Tier 0 命中率恒为 0。
///
/// 存在的意义（M2 降耗根因之一）：`ModelRouter::new` 此前无条件
/// `Arc::new(PrefixCache::new(2048))`，而 `AgentRuntime` 等组件正是走这条构造路径，
/// 于是它们各自持有**独立**缓存实例，跨轮重复请求永不命中。经此函数收敛后，同一进程内
/// 所有默认构造的 Router 与网关共享同一份缓存，跨轮重复请求方可命中。
///
/// 测试构建下刻意**不共享**：并行用例需要隔离（见 [`process_prefix_cache`]）。
#[must_use]
pub fn default_prefix_cache() -> Arc<PrefixCache> {
    if let Some(layer) = SEMANTIC_LAYER.get() {
        return layer.exact_arc().clone();
    }
    if cfg!(test) {
        return Arc::new(PrefixCache::new(2048));
    }
    process_prefix_cache()
}

/// FNV-1a 64 位哈希：快、无依赖、足够做缓存键（精确命中不需要密码学强度）。
pub fn hash_text(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `prefix_seen` 集合上限：防止「每个不同稳定前缀一条」无界增长（并随 `PersistedCache` 落盘膨胀）。
///
/// 采用最简单的安全策略：超限时驱逐任意一条（`HashSet` 无序）。被驱逐的前缀下次会被当作
/// 「新前缀」（不计复用），**绝不产生假阳性复用**——指标只会短暂少算，不会虚报命中。
const MAX_PREFIX_SEEN: usize = 4096;

/// 缓存可观测性：命中率与"省下 token"比例，没有它缓存就是黑箱。
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    /// 总查询次数（命中 + 未命中）
    pub lookups: u64,
    /// 精确命中次数
    pub hits: u64,
    /// 未命中次数
    pub misses: u64,
    /// 已被缓存的 prompt token 总量（写入侧）
    pub stored_prompt_tokens: u64,
    /// 命中缓存而省下的 prompt token 总量（读取侧）
    pub saved_prompt_tokens: u64,
    /// 稳定前缀（KV-cache 不变前缀）被复用次数（服务端 KV 块复用，非客户端命中）
    pub prefix_hits: u64,
    /// 稳定前缀复用而省下的 prompt token 总量（服务端不重复预填充的那部分）
    pub prefix_saved_tokens: u64,
    /// 服务端前缀缓存命中 token（DeepSeek `prompt_cache_hit_tokens` 归一）——诚实计量，不虚报
    pub server_cache_read_tokens: u64,
    /// 服务端前缀缓存写入 token（首次为该前缀建立缓存）
    pub server_cache_creation_tokens: u64,
}

impl CacheStats {
    /// 记录一次命中（saved = 该请求原本的 prompt token 数）
    pub fn hit(&mut self, saved_prompt_tokens: u32) {
        self.lookups += 1;
        self.hits += 1;
        self.saved_prompt_tokens += saved_prompt_tokens as u64;
    }

    /// 记录一次未命中（仅计数；stored 在 put 时累加）
    pub fn miss(&mut self) {
        self.lookups += 1;
        self.misses += 1;
    }

    /// 记录一次稳定前缀复用（KV-cache 原则 P1：不变前缀的 KV 块在服务端被复用，
    /// 无需重新预填充）。`saved_prefix_tokens` 为该前缀的 token 数（客户端如实计量，不虚报命中）。
    pub fn record_prefix_reuse(&mut self, saved_prefix_tokens: u32) {
        self.prefix_hits += 1;
        self.prefix_saved_tokens += saved_prefix_tokens as u64;
    }

    /// 记录一次服务端前缀缓存命中/写入（诚实计量：仅当 DeepSeek 真实返回 cache 字段时调用）。
    pub fn record_server_cache(&mut self, read_tokens: u32, creation_tokens: u32) {
        self.server_cache_read_tokens += read_tokens as u64;
        self.server_cache_creation_tokens += creation_tokens as u64;
    }

    /// 服务端前缀缓存命中率 ∈ [0, 1]：命中 token / (命中 + 写入) token。
    /// 反映 DeepSeek 原生 prompt caching 的真实复用程度（不是客户端精确缓存）。
    pub fn server_cache_hit_rate(&self) -> f64 {
        let total = self.server_cache_read_tokens + self.server_cache_creation_tokens;
        if total == 0 {
            0.0
        } else {
            self.server_cache_read_tokens as f64 / total as f64
        }
    }

    /// 命中率 ∈ [0, 1]
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            self.hits as f64 / self.lookups as f64
        }
    }

    /// 复用率：被命中复用的 token 占（写入 + 复用）总量的比例 ∈ [0, 1]
    pub fn reuse_ratio(&self) -> f64 {
        let total = self.stored_prompt_tokens + self.saved_prompt_tokens;
        if total == 0 {
            0.0
        } else {
            self.saved_prompt_tokens as f64 / total as f64
        }
    }
}

/// 单条缓存条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub content: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub last_used_ms: u64,
}

/// 落盘载荷（W-7b）：仅持久化可恢复的部分（条目 + 已观测的稳定前缀集合），
/// 统计字段在加载时按已存条目重新累加，避免重复计数。
#[derive(Serialize, Deserialize)]
pub(crate) struct PersistedCache {
    map: HashMap<u64, CacheEntry>,
    prefix_seen: std::collections::HashSet<u64>,
}

impl PersistedCache {
    /// 空载荷：共享 `exact` 层由外部负责落盘时，避免重复写入与双写漂移。
    fn empty() -> Self {
        Self {
            map: HashMap::new(),
            prefix_seen: std::collections::HashSet::new(),
        }
    }
}

#[derive(Debug)]
struct Inner {
    map: HashMap<u64, CacheEntry>,
    stats: CacheStats,
    /// 已观察到的稳定前缀哈希集合（KV-cache 不变前缀复用观测，用于命中率可观测）
    prefix_seen: std::collections::HashSet<u64>,
}

/// 客户端前缀缓存：精确哈希命中 + LRU 驱逐 + 内置命中率计量。
///
/// 线程安全（`Mutex` 包裹），可包进 `Arc` 在异步任务间共享。
#[derive(Debug)]
pub struct PrefixCache {
    inner: Mutex<Inner>,
    capacity: usize,
    /// 自上次落盘以来是否有写入（put / absorb 置位；save/save_if_dirty 清除）。
    /// 供网关「每轮后按需落盘」——无写入零 IO，跨轮/跨重启缓存不丢。
    dirty: AtomicBool,
}

impl PrefixCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                stats: CacheStats::default(),
                prefix_seen: std::collections::HashSet::new(),
            }),
            capacity: capacity.max(1),
            dirty: AtomicBool::new(false),
        }
    }

    /// 清空所有缓存条目和统计（仅供测试隔离使用）。
    #[cfg(test)]
    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.map.clear();
        g.stats = CacheStats::default();
        g.prefix_seen.clear();
    }

    /// 精确命中查询；命中则更新 LRU 时间戳并计入 stats，否则计入一次 miss。
    pub fn get(&self, key: u64) -> Option<CacheEntry> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // 先以不可变借用克隆出条目，释放 map 借用后再更新 stats（避免双重可变借用）
        let hit = g.map.get(&key).cloned();
        match hit {
            Some(mut e) => {
                g.stats.hit(e.prompt_tokens);
                e.last_used_ms = now_ms();
                Some(e)
            }
            None => {
                g.stats.miss();
                None
            }
        }
    }

    /// 写入缓存；累加 stored token；超出容量时驱逐最久未用条目（LRU）。
    pub fn put(&self, key: u64, entry: CacheEntry) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.stored_prompt_tokens += entry.prompt_tokens as u64;
        g.map.insert(key, entry);
        if g.map.len() > self.capacity {
            if let Some(evict) = g
                .map
                .iter()
                .min_by_key(|(_, v)| v.last_used_ms)
                .map(|(k, _)| *k)
            {
                g.map.remove(&evict);
            }
        }
        drop(g);
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// 观测一次稳定前缀：若该前缀首次出现返回 false 并记录；若已出现过返回 true 并计入前缀复用。
    ///
    /// 这是 KV-Cache 原则 P1 的客户端观测：相同稳定前缀的请求会让服务端复用已计算的 KV 块
    /// （prompt caching），无需重新预填充。`prefix_tokens` 为该前缀 token 数，如实计入
    /// `CacheStats.prefix_saved_tokens`，不冒充为「客户端命中」（那需要逐字节一致的全请求）。
    pub fn note_prefix(&self, hash: u64, prefix_tokens: u32) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.prefix_seen.contains(&hash) {
            g.stats.record_prefix_reuse(prefix_tokens);
            true
        } else {
            g.prefix_seen.insert(hash);
            // 有界：超过上限时驱逐任意一条（HashSet 无序）。驱逐只会让该前缀下次被当作
            // 「新前缀」，不计复用——绝不产生假阳性复用（见 MAX_PREFIX_SEEN 文档）。
            if g.prefix_seen.len() > MAX_PREFIX_SEEN {
                if let Some(victim) = g.prefix_seen.iter().next().copied() {
                    g.prefix_seen.remove(&victim);
                }
            }
            false
        }
    }

    /// 记录一次服务端前缀缓存命中/写入（诚实计量：仅当 DeepSeek 真实返回 cache 字段时调用）。
    pub fn record_server_cache(&self, read_tokens: u32, creation_tokens: u32) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stats.record_server_cache(read_tokens, creation_tokens);
    }

    pub fn stats(&self) -> CacheStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 从磁盘加载冷上下文缓存（W-7b）：读取 JSON 文件并恢复条目与已观测稳定前缀集合。
    ///
    /// 加载失败（文件不存在 / 解析错误 / 磁盘只读）时**静默回退为空缓存**，不致命——
    /// 技能执行等调用方不应因落盘问题而崩溃。容量取 `max(256, 已加载条目数)`，
    /// 避免刚加载的冷上下文立即被 LRU 驱逐。
    pub fn open(path: &Path) -> Self {
        let mut cache = Self::new(256);
        if let Ok(data) = std::fs::read(path) {
            if let Ok(p) = serde_json::from_slice::<PersistedCache>(&data) {
                let mut g = cache.inner.lock().unwrap_or_else(|e| e.into_inner());
                for e in p.map.values() {
                    g.stats.stored_prompt_tokens += e.prompt_tokens as u64;
                }
                g.map = p.map;
                g.prefix_seen = p.prefix_seen;
                cache.capacity = cache.capacity.max(g.map.len());
                drop(g);
                // open 恢复的是「已落盘」内容：不置脏（与 save_if_dirty 语义自洽）
                cache.dirty.store(false, Ordering::Relaxed);
            }
        }
        cache
    }

    /// 把磁盘上的冷上下文**并入**当前缓存（条目按 key 去重合并 + 已观测前缀集合并），
    /// 合并后置脏标记。文件缺失/损坏静默忽略（对齐 [`PrefixCache::open`] 的回退语义）。
    ///
    /// 供已构造的共享缓存（如网关级缓存）吸收历史落盘内容——跨进程重启后的
    /// 可重复请求无需重新预填充即可命中。
    pub fn absorb(&self, path: &Path) {
        let cold = Self::open(path);
        let mut merged_any = false;
        {
            let cold_g = cold.inner.lock().unwrap_or_else(|e| e.into_inner());
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            for (k, v) in cold_g.map.iter() {
                if !g.map.contains_key(k) {
                    g.stats.stored_prompt_tokens += v.prompt_tokens as u64;
                    g.map.insert(*k, v.clone());
                    merged_any = true;
                }
            }
            for h in cold_g.prefix_seen.iter() {
                if g.prefix_seen.insert(*h) {
                    merged_any = true;
                }
            }
        }
        if merged_any {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    /// 是否含未落盘写入（put / absorb 后为 true，save/save_if_dirty 成功后为 false）。
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    /// 有未落盘写入时才原子落盘（临时文件 + rename）并清脏标记；无写入零 IO。
    /// 返回是否实际发生了落盘。
    pub fn save_if_dirty(&self, path: &Path) -> std::io::Result<bool> {
        if !self.dirty.load(Ordering::Relaxed) {
            return Ok(false);
        }
        self.save(path)?;
        self.dirty.store(false, Ordering::Relaxed);
        Ok(true)
    }

    /// 落盘冷上下文缓存（W-7b）：原子写（临时文件 + rename），使跨进程重启可复用。
    ///
    /// 失败返回 `io::Error`（由调用方决定是否忽略，如 `SkillEngine` 静默忽略）。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = PersistedCache {
            map: g.map.clone(),
            prefix_seen: g.prefix_seen.clone(),
        };
        let data = serde_json::to_vec(&p)?;
        drop(g);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 导出当前落盘载荷（供 `MultiTierCache` 组合持久化复用）。
    pub(crate) fn snapshot(&self) -> PersistedCache {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        PersistedCache {
            map: g.map.clone(),
            prefix_seen: g.prefix_seen.clone(),
        }
    }

    /// 由落盘载荷重建（容量取 `max(capacity, 已加载条目数)`）。
    pub(crate) fn from_snapshot(p: PersistedCache, capacity: usize) -> Self {
        let cap = capacity.max(p.map.len());
        Self {
            inner: Mutex::new(Inner {
                map: p.map,
                stats: CacheStats::default(),
                prefix_seen: p.prefix_seen,
            }),
            capacity: cap.max(1),
            dirty: AtomicBool::new(false),
        }
    }
}

/// 当前毫秒时间戳（供 LRU 使用）
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 进程级前缀缓存追踪器（DeepSeek `prompt_cache_hit_tokens` 归一后统一上报）。
///
/// 设计要点（呼应 GUI_Agent `PrefixCacheTracker` 思路，纯 Rust 零依赖落地）：
/// - 服务端前缀缓存命中/写入 token 累积为进程级可观测指标；无需把 `PrefixCache` 实例
///   穿透到每一个 LLM 调用点（`model::llm` 现状即无该实例，归一 usage 后调用
///   [`report_prefix_cache_usage`] 即可）。
/// - [`PrefixCacheTracker::observe_stable_prefix`] 提供「稳定前缀漂移守卫」：压缩器/序列化层
///   在发出「应被服务端缓存的不变前缀」时登记其规范指纹，后续调用校验字节一致——
///   直接封堵「非规范重序列化」这一头号缓存杀手（四点缓存失效模式之二：工具结果
///   重序列化致字节漂移，前缀永不命中）。与 W-4 `ToolCompressor` 的逐字节确定性相辅相成。
pub struct PrefixCacheTracker {
    inner: Mutex<PrefixCacheTrackerInner>,
}

struct PrefixCacheTrackerInner {
    /// 上报次数（LLM 调用次数）
    calls: u64,
    /// 累计 prompt token（含命中 + 写入 + 未缓存）
    prompt_tokens_total: u64,
    /// 累计服务端前缀缓存命中 token（DeepSeek `prompt_cache_hit_tokens` 归一）
    cache_read_total: u64,
    /// 累计服务端前缀缓存写入 token（首次为该前缀建立 KV 缓存）
    cache_creation_total: u64,
    /// 稳定前缀规范指纹（FNV-1a）；`None` 表示尚未登记
    stable_prefix_fp: Option<u64>,
    /// 稳定前缀漂移次数（同一逻辑前缀两次登记指纹不同 = 缓存前缀被破坏）
    stable_prefix_drift: u64,
}

impl PrefixCacheTracker {
    /// 构造独立实例（单测用）；生产路径用 [`prefix_cache_tracker`] 获取进程级单例。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PrefixCacheTrackerInner {
                calls: 0,
                prompt_tokens_total: 0,
                cache_read_total: 0,
                cache_creation_total: 0,
                stable_prefix_fp: None,
                stable_prefix_drift: 0,
            }),
        }
    }

    /// 上报一次 LLM 调用的前缀缓存用量（已归一：cache_read = 命中，cache_creation = 写入）。
    pub fn report(&self, prompt_tokens: u64, cache_read: u64, cache_creation: u64) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.calls += 1;
        g.prompt_tokens_total += prompt_tokens;
        g.cache_read_total += cache_read;
        g.cache_creation_total += cache_creation;
    }

    /// 登记 / 校验「稳定前缀」：传入应被服务端缓存的不变前缀规范串。
    ///
    /// - 首次登记：记录指纹并返回 `true`（视作基线）。
    /// - 后续一致：返回 `true`（无漂移，前缀可稳定命中）。
    /// - 后续不一致：累加 [`PrefixCacheTrackerInner::stable_prefix_drift`]、更新指纹、返回 `false`
    ///   （漂移告警——压缩/序列化破坏了缓存前缀，需排查）。
    pub fn observe_stable_prefix(&self, prefix: &str) -> bool {
        let fp = hash_text(prefix);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match g.stable_prefix_fp {
            Some(prev) if prev == fp => true,
            Some(_) => {
                g.stable_prefix_drift += 1;
                g.stable_prefix_fp = Some(fp);
                false
            }
            None => {
                g.stable_prefix_fp = Some(fp);
                true
            }
        }
    }

    /// 快照当前可观测指标。
    pub fn stats(&self) -> PrefixCacheTrackerStats {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        PrefixCacheTrackerStats {
            calls: g.calls,
            prompt_tokens_total: g.prompt_tokens_total,
            cache_read_total: g.cache_read_total,
            cache_creation_total: g.cache_creation_total,
            stable_prefix_drift: g.stable_prefix_drift,
        }
    }

    /// 服务端前缀缓存命中率 ∈ [0, 1]：命中 token / (命中 + 写入) token。
    ///
    /// 反映 DeepSeek 原生 prompt caching 的真实复用程度（不是客户端精确缓存）。
    /// 稳态（无新建缓存）下趋近 1.0；首调或前缀漂移时会回落。
    pub fn hit_rate(&self) -> f64 {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let total = g.cache_read_total + g.cache_creation_total;
        if total == 0 {
            0.0
        } else {
            g.cache_read_total as f64 / total as f64
        }
    }

    /// 稳定前缀漂移次数（>0 表示压缩/序列化破坏了缓存前缀，应告警）。
    pub fn stable_prefix_drift(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stable_prefix_drift
    }
}

impl Default for PrefixCacheTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级前缀缓存追踪器快照。
#[derive(Clone, Copy, Debug, Default)]
pub struct PrefixCacheTrackerStats {
    pub calls: u64,
    pub prompt_tokens_total: u64,
    pub cache_read_total: u64,
    pub cache_creation_total: u64,
    pub stable_prefix_drift: u64,
}

static PREFIX_CACHE_TRACKER: OnceLock<PrefixCacheTracker> = OnceLock::new();

/// 获取进程级前缀缓存追踪器（惰性初始化，全局唯一）。
pub fn prefix_cache_tracker() -> &'static PrefixCacheTracker {
    PREFIX_CACHE_TRACKER.get_or_init(|| PrefixCacheTracker {
        inner: Mutex::new(PrefixCacheTrackerInner {
            calls: 0,
            prompt_tokens_total: 0,
            cache_read_total: 0,
            cache_creation_total: 0,
            stable_prefix_fp: None,
            stable_prefix_drift: 0,
        }),
    })
}

/// 便捷上报：LLM 调用点归一 usage 后调用，累积进程级前缀缓存命中率。
pub fn report_prefix_cache_usage(prompt_tokens: u64, cache_read: u64, cache_creation: u64) {
    prefix_cache_tracker().report(prompt_tokens, cache_read, cache_creation);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(content: &str, tokens: u32) -> CacheEntry {
        CacheEntry {
            content: content.into(),
            prompt_tokens: tokens,
            completion_tokens: 1,
            last_used_ms: 1,
        }
    }

    fn unique_cache_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mgt_cache_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("prefix-cache.json")
    }

    #[test]
    fn dirty_flag_tracks_puts_and_save_if_dirty_clears() {
        let path = unique_cache_path("dirty");
        let c = PrefixCache::new(8);
        assert!(!c.is_dirty(), "新建缓存应干净");
        c.put(1, entry("A", 10));
        assert!(c.is_dirty(), "写入后应置脏");
        assert!(c.save_if_dirty(&path).unwrap(), "有写入应落盘");
        assert!(!c.is_dirty(), "落盘后应清脏");
        assert!(
            !c.save_if_dirty(&path).unwrap(),
            "无新写入零 IO（不重复落盘）"
        );
    }

    #[test]
    fn absorb_merges_cold_entries_and_marks_dirty() {
        let path = unique_cache_path("absorb");
        {
            let a = PrefixCache::new(8);
            a.put(42, entry("冷上下文", 30));
            a.save(&path).unwrap();
        }
        let b = PrefixCache::new(8);
        assert!(!b.is_dirty());
        b.absorb(&path);
        assert!(b.is_dirty(), "并入后应置脏（待落盘合并结果）");
        let hit = b
            .get(42)
            .expect("冷上下文并入后应命中——可重复请求 100% 复用的证明");
        assert_eq!(hit.content, "冷上下文");
    }

    #[test]
    fn persisted_roundtrip_achieves_full_hit_on_repeat() {
        // 跨"重启"命中证明：写 → 落盘 → 新进程语义（open）→ 同一 key 100% 命中
        let path = unique_cache_path("roundtrip");
        {
            let a = PrefixCache::new(8);
            a.put(7, entry("B", 20));
            assert!(a.save_if_dirty(&path).unwrap());
        }
        let b = PrefixCache::open(&path);
        assert!(b.get(7).is_some(), "重启后相同请求必须命中（100% 复用）");
        let s = b.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 0);
    }

    #[test]
    fn put_then_get_returns_entry() {
        let c = PrefixCache::new(8);
        c.put(
            1,
            CacheEntry {
                content: "A".into(),
                prompt_tokens: 10,
                completion_tokens: 5,
                last_used_ms: 1,
            },
        );
        let e = c.get(1).expect("应命中");
        assert_eq!(e.content, "A");
        assert_eq!(e.completion_tokens, 5);
    }

    #[test]
    fn miss_records_stats() {
        let c = PrefixCache::new(8);
        assert!(c.get(999).is_none());
        let s = c.stats();
        assert_eq!(s.lookups, 1);
        assert_eq!(s.hits, 0);
        assert_eq!(s.misses, 1);
    }

    #[test]
    fn hit_records_stats_and_hit_rate() {
        let c = PrefixCache::new(8);
        c.put(
            7,
            CacheEntry {
                content: "B".into(),
                prompt_tokens: 20,
                completion_tokens: 3,
                last_used_ms: 1,
            },
        );
        c.get(7);
        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.lookups, 1);
        assert!((s.hit_rate() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn lru_eviction_keeps_capacity() {
        let c = PrefixCache::new(2);
        for k in 0..5u64 {
            c.put(
                k,
                CacheEntry {
                    content: k.to_string(),
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    last_used_ms: k,
                },
            );
        }
        assert!(c.len() <= 2, "超过容量应被 LRU 驱逐");
        // 最近写入的 key=4 必然还在
        assert!(c.get(4).is_some());
    }

    #[test]
    fn hash_is_stable_and_distinct() {
        assert_eq!(hash_text("hello"), hash_text("hello"));
        assert_ne!(hash_text("hello"), hash_text("world"));
    }

    #[test]
    fn reuse_ratio_reflects_savings() {
        let c = PrefixCache::new(8);
        c.put(
            1,
            CacheEntry {
                content: "x".into(),
                prompt_tokens: 100,
                completion_tokens: 1,
                last_used_ms: 1,
            },
        );
        c.get(1); // 命中，省下 100 prompt token
        let s = c.stats();
        // stored=100, saved=100 → reuse 0.5
        assert!((s.reuse_ratio() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn prefix_reuse_recorded_on_repeat() {
        let c = PrefixCache::new(8);
        assert!(!c.note_prefix(42, 100), "首次应记为新前缀");
        assert!(c.note_prefix(42, 100), "重复应记为前缀复用");
        let s = c.stats();
        assert_eq!(s.prefix_hits, 1);
        assert_eq!(s.prefix_saved_tokens, 100);
    }

    #[test]
    fn record_prefix_reuse_updates_stats() {
        let mut s = CacheStats::default();
        s.record_prefix_reuse(50);
        assert_eq!(s.prefix_hits, 1);
        assert_eq!(s.prefix_saved_tokens, 50);
    }

    #[test]
    fn distinct_prefixes_do_not_reuse() {
        let c = PrefixCache::new(8);
        assert!(!c.note_prefix(1, 10));
        assert!(!c.note_prefix(2, 20), "不同前缀不应复用");
        assert_eq!(c.stats().prefix_hits, 0);
    }

    #[test]
    fn prefix_seen_is_bounded() {
        // 缺陷回归：`prefix_seen` 曾无界增长（每个不同稳定前缀一条，且随落盘膨胀）。
        // 插入远超上限的互异前缀后，集合必须保持有界。
        let c = PrefixCache::new(8);
        for i in 0..(MAX_PREFIX_SEEN as u64 + 500) {
            c.note_prefix(i, 1);
        }
        let g = c.inner.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            g.prefix_seen.len() <= MAX_PREFIX_SEEN,
            "prefix_seen 必须保持有界（<= {MAX_PREFIX_SEEN}），实际 {}",
            g.prefix_seen.len()
        );
    }

    #[test]
    fn open_missing_file_returns_empty_cache() {
        let path =
            std::env::temp_dir().join(format!("marg_prefix_missing_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = PrefixCache::open(&path);
        assert!(c.is_empty(), "不存在的文件应加载为空缓存（不致命）");
    }

    #[test]
    fn prefix_cache_tracker_reports_and_hit_rate() {
        let t = PrefixCacheTracker::new();
        // 3 次调用：稳态（cache_read 远大于 creation）应趋近高命中率。
        t.report(1000, 800, 0);
        t.report(1000, 800, 0);
        t.report(1000, 800, 0);
        let s = t.stats();
        assert_eq!(s.calls, 3);
        assert_eq!(s.cache_read_total, 2400);
        assert!((t.hit_rate() - 1.0).abs() < 1e-9, "稳态应 100% 命中");
    }

    #[test]
    fn prefix_cache_tracker_detects_stable_prefix_drift() {
        let t = PrefixCacheTracker::new();
        assert!(
            t.observe_stable_prefix("system-prefix-v1"),
            "首次登记应返回 true"
        );
        assert!(
            t.observe_stable_prefix("system-prefix-v1"),
            "相同前缀应无漂移"
        );
        assert!(
            !t.observe_stable_prefix("system-prefix-v2-changed"),
            "前缀变化应检出漂移并返回 false"
        );
        assert_eq!(t.stable_prefix_drift(), 1, "应记录 1 次漂移");
    }

    #[test]
    fn global_prefix_cache_tracker_is_singleton() {
        // 进程级单例：两次获取应为同一实例（指针相等）。
        let a = prefix_cache_tracker() as *const PrefixCacheTracker;
        let b = prefix_cache_tracker() as *const PrefixCacheTracker;
        assert_eq!(a, b, "prefix_cache_tracker() 应返回同一全局实例");
    }

    #[test]
    fn process_prefix_cache_is_singleton_and_shared() {
        // 生产语义回归守卫：进程级句柄必须是同一实例，否则 `ModelRouter::new` 各自持有
        // 独立缓存表，跨轮查找永不命中（Tier 0 命中率恒 0）。
        let a = process_prefix_cache();
        let b = process_prefix_cache();
        assert!(
            Arc::ptr_eq(&a, &b),
            "process_prefix_cache() 必须是进程级单例，否则跨轮命中率恒为 0"
        );
        // 写入经一个句柄，另一个句柄必须看得到（真正共享，而非同指针巧合）
        let key = hash_text("process-prefix-cache-singleton-probe");
        a.put(
            key,
            CacheEntry {
                content: "shared".into(),
                prompt_tokens: 7,
                completion_tokens: 1,
                last_used_ms: 1,
            },
        );
        assert!(
            b.get(key).is_some(),
            "同一进程内两次获取的进程级缓存必须共享条目（跨轮复用前提）"
        );
    }

    #[test]
    fn default_prefix_cache_isolates_under_test_build() {
        // 测试隔离契约：并行用例不得互相污染，故测试构建下每次返回独立实例。
        // （生产路径由 `default_prefix_cache() -> process_prefix_cache()` 保证共享。）
        let a = default_prefix_cache();
        let b = default_prefix_cache();
        assert!(
            !Arc::ptr_eq(&a, &b),
            "测试构建下 default_prefix_cache() 必须隔离，避免并行用例互相污染缓存"
        );
    }

    #[test]
    fn save_then_open_roundtrips_entries_and_prefixes() {
        let path = std::env::temp_dir().join(format!("marg_prefix_rt_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let c = PrefixCache::new(16);
        c.put(
            1,
            CacheEntry {
                content: "A".into(),
                prompt_tokens: 10,
                completion_tokens: 5,
                last_used_ms: 1,
            },
        );
        c.put(
            2,
            CacheEntry {
                content: "B".into(),
                prompt_tokens: 20,
                completion_tokens: 3,
                last_used_ms: 2,
            },
        );
        assert!(!c.note_prefix(42, 100));
        c.save(&path).expect("落盘应成功");
        // 重新打开（模拟进程重启）
        let c2 = PrefixCache::open(&path);
        assert_eq!(c2.get(1).expect("A 应命中").content, "A");
        assert_eq!(c2.get(2).expect("B 应命中").content, "B");
        assert!(c2.note_prefix(42, 100), "已观测前缀应从落盘恢复");
        assert_eq!(c2.stats().stored_prompt_tokens, 30, "stored token 应恢复");
        let _ = std::fs::remove_file(&path);
    }

    // ---- R46：生产语义路径必须经 EmbeddingProvider（文档 1「语义缓存未接真实 embedding」）----

    /// 取 `marker` 之后第一个函数体（按花括号配对截取，故函数上方的文档注释不计入）
    fn fn_body<'a>(src: &'a str, marker: &str) -> &'a str {
        let at = src.find(marker).expect("marker 必须存在") + marker.len();
        let open = src[at..].find('{').expect("函数应带函数体") + at;
        let mut depth = 0usize;
        for (i, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..=open + i];
                    }
                }
                _ => {}
            }
        }
        panic!("函数体花括号不配对: {marker}");
    }

    /// 生产接线守卫（有牙）：两处**默认**语义相似度实现都必须经 `EmbeddingProvider`。
    ///
    /// 此前它们分别写死 `LocalNgramEmbedder` / `LocalEmbedding`，于是
    /// `embeddings.backend=model` + `real-embed` 配好了真实嵌入后端，语义缓存却仍然只用
    /// 本地特征哈希——配置形同虚设。改回写死实现，本用例必须失败。
    #[test]
    fn production_semantic_paths_route_through_embedding_provider() {
        let own = include_str!("mod.rs");
        let bloom = include_str!("../agent/bloom_cache.rs");

        let install = fn_body(own, "pub fn install_semantic_layer");
        assert!(
            install.contains("ProviderEmbedder::new()"),
            "install_semantic_layer 必须用 ProviderEmbedder（否则真实嵌入后端配了也不生效）"
        );
        assert!(
            !install.contains("LocalNgramEmbedder"),
            "install_semantic_layer 不得写死本地 n-gram 嵌入器"
        );

        let sim = fn_body(bloom, "pub fn embedding_similarity");
        assert!(
            sim.contains("EmbeddingProvider::global()"),
            "Bloom 语义意图缓存的默认相似度必须走 EmbeddingProvider"
        );
        assert!(
            !sim.contains("LocalEmbedding::"),
            "默认相似度不得直连 LocalEmbedding（否则真实嵌入后端配了也不生效）"
        );

        // ③ 光把嵌入器接对还不够：Tier 2 必须能被显式选入，且 LLM 文本路径必须按选入
        //    选策略——否则语义层在生产里永远走不到（配了真实嵌入也无从生效）。
        assert!(
            install.contains("tier2_opted_in_from_env()"),
            "install_semantic_layer 必须按选入开关决定 Tier 2 是否参与"
        );
        let llm = include_str!("../model/llm.rs");
        let at = llm
            .find("if let Some(layer) = layer {")
            .expect("llm 文本路径应存在 `if let Some(layer) = layer {` 分支");
        let window = &llm[at..(at + 600).min(llm.len())];
        assert!(
            window.contains("text_path_policy("),
            "LLM 文本路径必须用 text_path_policy 决定 Tier 2 是否参与"
        );
        assert!(
            !window.contains("TierPolicy::NormalizedOnly"),
            "LLM 文本路径不得写死 NormalizedOnly（那会让语义层在生产里永远不生效）"
        );
    }

    /// 安装点必须把 Tier 2 开关**落到真实实例**上：`semantic_enabled` 恒等于选入开关。
    ///
    /// - 全套跑（env 未设）：两边都是 false —— 属性仍成立且不因安装顺序而 flaky；
    /// - 单跑 `MARGUERITE_SEMANTIC_CACHE=on cargo test … install_applies_tier2_opt_in_to_the_layer`：
    ///   选入为 true，若安装点漏了 `.with_semantic_enabled(...)`，断言立刻失败（选入形同虚设）。
    #[test]
    fn install_applies_tier2_opt_in_to_the_layer() {
        let exact = Arc::new(PrefixCache::new(8));
        let layer = install_semantic_layer(exact, None);
        assert_eq!(
            layer.semantic_enabled(),
            crate::cache::semantic::tier2_opted_in_from_env(),
            "安装出来的多层缓存必须如实反映 Tier 2 选入开关"
        );
    }
}
