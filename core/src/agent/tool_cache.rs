//! 工具结果缓存（KV-Cache 极简工程实践 §2.8）
//!
//! 蒸馏 opencode `packages/app/src/utils/scoped-cache.ts` 的极简缓存本质：
//! - scope-keyed 存储（`(tool, args)` 哈希为键）
//! - `ttlMs`（lazy sweep + `now()` DI 接缝，可单测）
//! - `maxEntries`（容量上限，超容 FIFO 驱逐）
//! - `touch`/命中率可观测
//!
//! 本模块落地「幂等工具结果缓存」：相同 `(tool, args)` 命中缓存不再重跑工具，
//! 非幂等工具（写/发/删/建）与失败结果不入缓存（诚实，不掩盖瞬时失败）。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::agent::tool_cycle::ToolResult;
use crate::cache::hash_text;

/// 工具缓存可观测性（KV-Cache 原则 P3：命中率必须可观测，否则是黑箱）。
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolCacheStats {
    /// 总查询次数（仅幂等工具的查询参与）
    pub lookups: u64,
    /// 精确命中次数
    pub hits: u64,
    /// 未命中次数
    pub misses: u64,
    /// 成功写入缓存的次数
    pub puts: u64,
}

impl ToolCacheStats {
    /// 命中率 ∈ [0, 1]
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            self.hits as f64 / self.lookups as f64
        }
    }
}

/// 单条缓存条目
struct CachedEntry {
    result: ToolResult,
    /// 过期毫秒时间戳；`u64::MAX` 表示不过期
    expires_at_ms: u64,
}

struct Inner {
    map: HashMap<u64, CachedEntry>,
    /// FIFO 插入序，用于容量驱逐
    order: VecDeque<u64>,
    stats: ToolCacheStats,
}

/// 幂等工具结果缓存。
///
/// 线程安全（`Mutex` 包裹），可包进 `Arc` 在异步任务间共享（团队/多轮复用同一实例）。
pub struct ToolCache {
    inner: Mutex<Inner>,
    capacity: usize,
    ttl_ms: Option<u64>,
    is_idempotent: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

/// 默认幂等判定：**只对已知只读工具开启缓存（白名单）**。
///
/// ⚠️ 历史缺陷：此前是「工具名含 write/send/delete/create… 等动词才排除」的
/// **黑名单**。于是 `shell_exec` / `code_exec` / `git` / `copy_file` / `move_file` /
/// `make_dir` / `str_replace_editor` / `office_automation` / `system_control` /
/// `browser` 等**有副作用**的工具（名字里恰好没有那些动词）被当成幂等并缓存 ——
/// 第二次相同调用直接返回缓存、根本不执行：改文件、跑脚本、git 提交被静默跳过。
///
/// 白名单方向是安全的：漏判只是「少缓存、变慢」，绝不会把写操作缓存成假结果。
/// 新增只读工具时按需补入；有副作用的工具一律**不要**加进来。
pub fn default_is_idempotent(tool: &str) -> bool {
    matches!(
        tool,
        // 文件系统只读
        "read_file"
            | "read"
            | "list_dir"
            | "grep"
            | "find_files"
            | "glob_file_search"
            | "codebase_search"
            | "count_lines"
            | "file_info"
            | "json_query"
            | "read_env"
            | "diff"
            // git 只读子命令
            | "git_status"
            | "git_diff"
            | "git_log"
            // 检索 / 网络只读
            | "web_fetch"
            | "web_search"
            | "web_research"
            | "http_request"
            | "research"
            | "vision"
            // 代码智能 / 环境 / 记忆只读
            | "code_intel"
            | "doctor"
            | "memory_list"
            | "memory_search"
            // 通用只读别名（历史用例 / 外部注册）
            | "search"
    )
}

/// 把一个片段以 `len:content` 形式追加到规范化串，长度前缀消除拼接歧义。
fn push_len_prefixed(out: &mut String, piece: &str) {
    out.push_str(&piece.len().to_string());
    out.push(':');
    out.push_str(piece);
    out.push('\u{0}');
}

impl ToolCache {
    /// 用自定义幂等判定策略构造。
    pub fn new(
        capacity: usize,
        ttl: Option<Duration>,
        is_idempotent: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: VecDeque::new(),
                stats: ToolCacheStats::default(),
            }),
            capacity: capacity.max(1),
            ttl_ms: ttl.map(|d| d.as_millis() as u64),
            is_idempotent,
        }
    }

    /// 用默认幂等判定策略构造。
    pub fn with_default_policy(capacity: usize, ttl: Option<Duration>) -> Self {
        Self::new(capacity, ttl, Arc::new(default_is_idempotent))
    }

    /// 该工具是否可缓存（幂等）。非幂等工具永不进缓存。
    pub fn is_cachable(&self, tool: &str) -> bool {
        (self.is_idempotent)(tool)
    }

    /// 生成缓存键：按参数 key 排序后规范化为稳定字符串再哈希（HashMap 无序，必须排序保确定性）。
    ///
    /// ⚠️ 每个片段用**长度前缀**分隔，避免歧义拼接导致的键碰撞：`{a:"x;b=y"}`
    /// 与 `{a:"x", b:"y"}` 在旧的 `k=v;` 朴素拼接下会生成同一个键（`a=x;b=y;`），
    /// 于是两个不同调用互相命中对方的缓存结果。
    fn cache_key(tool: &str, args: &HashMap<String, String>) -> u64 {
        let mut parts: Vec<(&String, &String)> = args.iter().collect();
        parts.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = String::new();
        push_len_prefixed(&mut s, tool);
        for (k, v) in parts {
            push_len_prefixed(&mut s, k);
            push_len_prefixed(&mut s, v);
        }
        hash_text(&s)
    }

    /// 查询缓存：仅幂等工具参与；命中且未过期返回克隆结果并计入命中；否则计入未命中返回 None。
    pub fn get(&self, tool: &str, args: &HashMap<String, String>) -> Option<ToolResult> {
        if !self.is_cachable(tool) {
            return None; // 非幂等工具永不查缓存
        }
        let key = Self::cache_key(tool, args);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = crate::cache::now_ms();
        // 先克隆出条目，释放 map 借用后再更新 stats（避免双重可变借用）
        let hit = g.map.get(&key).map(|e| (e.result.clone(), e.expires_at_ms));
        if let Some((result, expires)) = hit {
            if expires > now {
                g.stats.hits += 1;
                g.stats.lookups += 1;
                return Some(result);
            }
            // 已过期：淘汰并当作未命中
            g.map.remove(&key);
            if let Some(pos) = g.order.iter().position(|k| *k == key) {
                g.order.remove(pos);
            }
        }
        g.stats.lookups += 1;
        g.stats.misses += 1;
        None
    }

    /// 写入缓存：仅幂等且成功的工具结果入缓存；携带 TTL 过期时间；超容 FIFO 驱逐。
    pub fn put(&self, tool: &str, args: &HashMap<String, String>, result: ToolResult) {
        if !self.is_cachable(tool) {
            return; // 非幂等工具不入缓存（诚实：写/发/删不重放）
        }
        if !result.is_success() {
            return; // 仅缓存成功，避免掩盖瞬时失败
        }
        let key = Self::cache_key(tool, args);
        let expires = self.ttl_ms.map_or(u64::MAX, |t| crate::cache::now_ms() + t);
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !g.map.contains_key(&key) {
            g.order.push_back(key); // 新键入队尾（FIFO 序）
        }
        g.map.insert(
            key,
            CachedEntry {
                result,
                expires_at_ms: expires,
            },
        );
        g.stats.puts += 1;
        // 容量上限：超容从队首驱逐最旧条目
        while g.order.len() > self.capacity {
            if let Some(evict) = g.order.pop_front() {
                g.map.remove(&evict);
            } else {
                break;
            }
        }
    }

    /// 带缓存执行工具：命中即返回缓存；未命中执行 `exec` 并（按幂等/成功门控）回填缓存。
    ///
    /// DI 接缝：`exec` 由调用方注入（无需真实注册表即可单测命中行为）。
    pub async fn execute_cached<F, Fut>(
        &self,
        tool: &str,
        args: &HashMap<String, String>,
        exec: F,
    ) -> ToolResult
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ToolResult>,
    {
        if let Some(cached) = self.get(tool, args) {
            return cached;
        }
        let r = exec().await;
        self.put(tool, args, r.clone());
        r
    }

    pub fn stats(&self) -> ToolCacheStats {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).stats
    }

    /// 清空缓存（含统计）。
    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.map.clear();
        g.order.clear();
        g.stats = ToolCacheStats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn execute_cached_uses_cache_on_repeat() {
        // 同一 (tool,args) 第二次应命中缓存而不再执行 exec。
        let cache = ToolCache::with_default_policy(8, None);
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let exec = || {
            c.fetch_add(1, Ordering::SeqCst);
            async {
                ToolResult::Success {
                    data: "v".into(),
                    metadata: HashMap::new(),
                }
            }
        };
        let a = args(&[("q", "rust")]);
        let _r1 = cache.execute_cached("search", &a, exec).await;
        let _r2 = cache.execute_cached("search", &a, exec).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "第二次应命中缓存，exec 仅执行 1 次"
        );
    }

    #[tokio::test]
    async fn cache_hit_returns_cached_result() {
        let cache = ToolCache::with_default_policy(8, None);
        let a = args(&[("path", "/x")]);
        cache.put(
            "read",
            &a,
            ToolResult::Success {
                data: "cached".into(),
                metadata: HashMap::new(),
            },
        );
        let hit = cache.get("read", &a).expect("应命中");
        assert!(hit.is_success());
        assert_eq!(hit.data(), Some("cached"));
    }

    #[tokio::test]
    async fn cache_miss_then_put_then_hit() {
        let cache = ToolCache::with_default_policy(8, None);
        let a = args(&[("path", "/y")]);
        assert!(cache.get("read", &a).is_none(), "未写入前应未命中");
        cache.put(
            "read",
            &a,
            ToolResult::Success {
                data: "ok".into(),
                metadata: HashMap::new(),
            },
        );
        assert!(cache.get("read", &a).is_some(), "写入后应命中");
    }

    #[tokio::test]
    async fn non_idempotent_tool_never_cached() {
        // write 工具即使成功也不入缓存、查不到。
        let cache = ToolCache::with_default_policy(8, None);
        let a = args(&[("path", "/z")]);
        cache.put(
            "write_file",
            &a,
            ToolResult::Success {
                data: "written".into(),
                metadata: HashMap::new(),
            },
        );
        assert!(
            cache.get("write_file", &a).is_none(),
            "非幂等工具不应被缓存"
        );
        assert!(!cache.is_cachable("write_file"));
    }

    #[tokio::test]
    async fn failing_result_not_cached() {
        // 失败结果不入缓存，下次仍走 exec。
        let cache = ToolCache::with_default_policy(8, None);
        let a = args(&[("q", "err")]);
        cache.put(
            "search",
            &a,
            ToolResult::Error {
                message: "boom".into(),
                code: "E".into(),
            },
        );
        assert!(cache.get("search", &a).is_none(), "错误结果不应被缓存");
    }

    #[tokio::test]
    async fn expired_entry_treated_as_miss() {
        let cache = ToolCache::with_default_policy(8, Some(Duration::from_millis(10)));
        let a = args(&[("q", "t")]);
        cache.put(
            "search",
            &a,
            ToolResult::Success {
                data: "v".into(),
                metadata: HashMap::new(),
            },
        );
        // 推进时间越过 TTL（now_ms 用真实时钟，sleep 超过 TTL 即可）。
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(cache.get("search", &a).is_none(), "过期条目应视为未命中");
    }

    #[tokio::test]
    async fn capacity_eviction_keeps_recent() {
        let cache = ToolCache::with_default_policy(2, None);
        for i in 0..4u64 {
            let a = args(&[("k", &format!("v{i}"))]);
            cache.put(
                "read",
                &a,
                ToolResult::Success {
                    data: format!("d{i}"),
                    metadata: HashMap::new(),
                },
            );
        }
        let old = args(&[("k", "v0")]);
        let recent = args(&[("k", "v3")]);
        assert!(cache.get("read", &old).is_none(), "最旧条目应被 FIFO 驱逐");
        assert!(cache.get("read", &recent).is_some(), "最近条目应仍在");
    }

    #[tokio::test]
    async fn stats_track_lookups_hits_and_puts() {
        let cache = ToolCache::with_default_policy(8, None);
        let a = args(&[("q", "s")]);
        cache.put(
            "search",
            &a,
            ToolResult::Success {
                data: "v".into(),
                metadata: HashMap::new(),
            },
        );
        let _ = cache.get("search", &a); // hit
        let _ = cache.get("search", &args(&[("q", "other")])); // miss (不同 args)
        let s = cache.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.puts, 1);
        assert!((s.hit_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn default_idempotent_classifies_tools() {
        // 只读工具 → 可缓存。
        for t in [
            "search",
            "read_file",
            "grep",
            "list_dir",
            "git_status",
            "web_search",
        ] {
            assert!(default_is_idempotent(t), "{t} 应可缓存");
        }
        // 有副作用工具 → 永不缓存。含此前被黑名单**漏掉**的真实工具名
        // （shell_exec/code_exec/git/copy_file/… 名字里没有变更动词，却被误判幂等）。
        for t in [
            "write_file",
            "send_email",
            "delete_record",
            "shell_exec",
            "code_exec",
            "git",
            "copy_file",
            "move_file",
            "make_dir",
            "str_replace_editor",
            "office_automation",
            "system_control",
            "browser",
            "apply_patch",
        ] {
            assert!(!default_is_idempotent(t), "{t} 有副作用，不得缓存");
        }
        // 未知工具 → 默认不缓存（安全方向：宁可少缓存，不可缓存写操作）。
        assert!(!default_is_idempotent("some_future_tool"));
    }

    #[test]
    fn cache_key_is_order_independent() {
        // 参数顺序不同但内容相同 → 同一缓存键。
        let k1 = ToolCache::cache_key("search", &args(&[("a", "1"), ("b", "2")]));
        let k2 = ToolCache::cache_key("search", &args(&[("b", "2"), ("a", "1")]));
        assert_eq!(k1, k2, "参数顺序不应影响缓存键");
    }

    #[test]
    fn cache_key_does_not_collide_on_separators() {
        // 旧实现用 `k=v;` 朴素拼接 → `{a:"x;b=y"}` 与 `{a:"x",b:"y"}` 同键。
        let a = ToolCache::cache_key("t", &args(&[("a", "x;b=y")]));
        let b = ToolCache::cache_key("t", &args(&[("a", "x"), ("b", "y")]));
        assert_ne!(a, b, "含分隔符的参数不得与拆分后的多参数碰撞");
    }
}
