//! 压缩与复用的**统一观测口**（缺口 1 的"这一轮到底省了多少"）。
//!
//! ## 为什么需要它
//!
//! 这个仓库里"省 token"的机制散在四处，各报各的：
//!
//! | 机制 | 位置 | 它自己的计数器 |
//! |---|---|---|
//! | 结果复用 | `agent/tool_cache.rs` | `ToolCacheStats{lookups,hits,misses,puts}` |
//! | 同参重复识别 | `agent/stream_loop.rs` | `DOOM_LOOP_THRESHOLD` 命中次数（回路内局部变量） |
//! | 中间截断 | `context/middle_truncate.rs` | 每次调用返回 `omitted_chars`，不累计 |
//! | 大输出落盘 | `context/spool.rs` | 落盘即返回指纹，不计数 |
//!
//! 于是"压缩有没有用"这个问题在 UI 上拼不出来：四个数字在四个地方，没有一处能断言。
//! [`CompressionReport`] 是**把已有数字汇到一处**的契约，不是第五个机制。
//!
//! ## 设计约束
//!
//! - **没有的数字就不出现**：每个字段都是 `Option`，`to_json` 只输出 `Some` 的那些。
//!   空报告序列化成 `{}`，不是一堆 `0` —— 全 0 的仪表盘和没有仪表盘一样误导。
//! - **只做汇总，不做统计**：命中率这类派生值由字段算出来（`hit_rate()` 已经在
//!   `ToolCacheStats` 上），这里不重复实现口径。
//! - **不做全局单例**：谁是生产者谁持有实例，避免测试之间通过全局互相污染。

use serde_json::{json, Map, Value};

use crate::agent::tool_cache::ToolCacheStats;

/// 一轮（或一个会话）的压缩/复用汇总。
///
/// 所有字段默认 `None` = "这一项没人上报"，与"上报了但为 0"语义不同。
#[derive(Debug, Clone, Default)]
pub struct CompressionReport {
    /// 工具结果复用的命中情况（来自 `ToolCache::stats()`）
    pub tool_cache: Option<ToolCacheStats>,
    /// 同参重复被识别的次数（来自 doom-loop / 熔断器，二者同语义，只报一处）
    pub doom_repeats: Option<u32>,
    /// 中间截断累计省掉的字符数（来自 `middle_truncate::Truncated::omitted_chars`）
    pub truncated_chars: Option<u64>,
    /// 落盘留底的工具输出条数（来自 `spool::spool` 返回 `Some` 的次数）
    pub spooled: Option<u64>,
    /// `read_file` 返回"未改动存根"的次数（P0#1 的去重层）
    pub read_stubs: Option<u64>,
}

impl CompressionReport {
    /// 空报告：序列化为 `{}`。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从工具缓存统计构造（唯一一条"现在就能填"的来源）。
    pub fn from_tool_cache(stats: ToolCacheStats) -> Self {
        Self {
            tool_cache: Some(stats),
            ..Self::default()
        }
    }

    /// 填同参重复次数。
    pub fn with_doom_repeats(mut self, n: u32) -> Self {
        self.doom_repeats = Some(n);
        self
    }

    /// 填中间截断累计字符数。
    pub fn with_truncated_chars(mut self, n: u64) -> Self {
        self.truncated_chars = Some(n);
        self
    }

    /// 填落盘条数。
    pub fn with_spooled(mut self, n: u64) -> Self {
        self.spooled = Some(n);
        self
    }

    /// 填未改动存根次数。
    pub fn with_read_stubs(mut self, n: u64) -> Self {
        self.read_stubs = Some(n);
        self
    }

    /// 是否有任何一项被上报（全 `None` ⇒ `false`）。
    pub fn is_empty(&self) -> bool {
        self.tool_cache.is_none()
            && self.doom_repeats.is_none()
            && self.truncated_chars.is_none()
            && self.spooled.is_none()
            && self.read_stubs.is_none()
    }

    /// 并入 `meta` 用的 JSON 对象。**未上报的项不产生键**（不写 0）。
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(c) = self.tool_cache {
            out.insert(
                "tool_cache".to_string(),
                json!({
                    "lookups": c.lookups,
                    "hits": c.hits,
                    "misses": c.misses,
                    "puts": c.puts,
                    "hit_rate": c.hit_rate(),
                }),
            );
        }
        if let Some(n) = self.doom_repeats {
            out.insert("doom_repeats".to_string(), json!(n));
        }
        if let Some(n) = self.truncated_chars {
            out.insert("truncated_chars".to_string(), json!(n));
        }
        if let Some(n) = self.spooled {
            out.insert("spooled".to_string(), json!(n));
        }
        if let Some(n) = self.read_stubs {
            out.insert("read_stubs".to_string(), json!(n));
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(lookups: u64, hits: u64, misses: u64, puts: u64) -> ToolCacheStats {
        ToolCacheStats {
            lookups,
            hits,
            misses,
            puts,
        }
    }

    #[test]
    fn empty_report_serializes_to_empty_object() {
        let v = CompressionReport::new().to_json();
        assert_eq!(v, json!({}), "空报告不能输出一堆 0");
        assert!(CompressionReport::new().is_empty());
    }

    #[test]
    fn tool_cache_section_carries_derived_hit_rate() {
        let v = CompressionReport::from_tool_cache(stats(4, 3, 1, 2)).to_json();
        assert_eq!(v["tool_cache"]["lookups"], json!(4));
        assert_eq!(v["tool_cache"]["hits"], json!(3));
        assert_eq!(v["tool_cache"]["puts"], json!(2));
        let rate = v["tool_cache"]["hit_rate"].as_f64().expect("应有 hit_rate");
        assert!((rate - 0.75).abs() < 1e-12, "got {rate}");
    }

    #[test]
    fn unreported_fields_are_absent_not_zero() {
        let v = CompressionReport::from_tool_cache(stats(1, 0, 1, 0)).to_json();
        for key in ["doom_repeats", "truncated_chars", "spooled", "read_stubs"] {
            assert!(
                v.get(key).is_none(),
                "{key} 没人上报就必须缺省，写 0 会让『没测』看起来像『省了 0』"
            );
        }
    }

    #[test]
    fn builders_only_add_their_own_key() {
        let v = CompressionReport::new()
            .with_doom_repeats(2)
            .with_read_stubs(7)
            .to_json();
        assert_eq!(v["doom_repeats"], json!(2));
        assert_eq!(v["read_stubs"], json!(7));
        assert!(v.get("tool_cache").is_none());
        assert!(v.get("truncated_chars").is_none());
    }

    #[test]
    fn zero_but_reported_is_kept() {
        // 上报了 0（例如"确实一次都没截断"）与"没人上报"必须能区分
        let v = CompressionReport::new().with_truncated_chars(0).to_json();
        assert_eq!(v["truncated_chars"], json!(0));
        assert!(!CompressionReport::new().with_truncated_chars(0).is_empty());
    }
}
