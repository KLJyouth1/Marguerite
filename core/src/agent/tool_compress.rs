//! 工具输出压缩器（Headroom / FlashCompact / ACON 启示，纯 Rust 确定性实现）
//!
//! 设计目标——直接放大 DeepSeek 前缀缓存命中率（呼应 Reasonix 99.82% 工程）：
//! 1. **压缩**：降低进入上下文的工具输出 token 体积（JSON 感知 canonical + 日志 verbatim 折叠）。
//! 2. **稳定**：输出**逐字节确定性**——相同逻辑内容永远产出相同压缩串，消除
//!    「非规范序列化」这一头号缓存杀手（四点缓存失效模式之二：工具结果重序列化致字节漂移）。
//! 3. **可逆（CCR, Cacheable Compressed Representation）**：完整原文按内容指纹存入内存 LRU，
//!    模型可凭返回指纹经 `retrieve_full` 取回细节。与现有 `ToolCache` 互补——
//!    `ToolCache` 按 `(tool,args)` 去重重跑，本器按**内容**去重保原文，专治「模型偶尔要看全文」。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde_json::Value;

use crate::agent::tool_cycle::ToolResult;
use crate::cache::hash_text;
use crate::cache::prefix_cache_tracker;

/// 压缩可观测性（KV-Cache 原则 P3：命中率/压缩比必须可观测，否则是黑箱）。
#[derive(Clone, Copy, Debug, Default)]
pub struct CompressStats {
    /// 触发压缩的次数（仅超预算的大输出计入）
    pub compressed: u64,
    /// 压缩前原文总字节
    pub bytes_in: u64,
    /// 压缩后入上下文总字节
    pub bytes_out: u64,
    /// 模型经指纹取回原文的次数
    pub retrieves: u64,
}

impl CompressStats {
    /// 压缩比 ∈ (0, 1]，越小越省；1.0 表示未实际压缩。
    pub fn ratio(&self) -> f64 {
        if self.bytes_in == 0 {
            1.0
        } else {
            self.bytes_out as f64 / self.bytes_in as f64
        }
    }
}

/// 单条原文缓存条目（CCR 可逆存储）
struct OriginalEntry {
    text: String,
}

/// 内容指纹 → 原文 的内存 LRU（`std::sync::Mutex` 包裹，纯内存、锁从不跨 `.await`，安全）。
struct OriginalStore {
    map: HashMap<u64, OriginalEntry>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl OriginalStore {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// 写入原文并返回 16 位十六进制指纹；已存在则复用既有指纹（确定性）。
    fn store(&mut self, text: &str) -> String {
        let h = hash_text(text);
        if !self.map.contains_key(&h) {
            self.order.push_back(h);
            self.map.insert(
                h,
                OriginalEntry {
                    text: text.to_string(),
                },
            );
            while self.order.len() > self.capacity {
                if let Some(ev) = self.order.pop_front() {
                    self.map.remove(&ev);
                } else {
                    break;
                }
            }
        }
        format!("{:016x}", h)
    }

    fn get(&self, fp: u64) -> Option<String> {
        self.map.get(&fp).map(|e| e.text.clone())
    }
}

/// 工具输出压缩器。
///
/// 线程安全（`Arc<ToolCompressor>` 可在异步任务间共享，团队/多轮复用同一实例）。
pub struct ToolCompressor {
    /// 进入上下文前保留的最大字符预算；**未超预算的输出零改动返回**（保真 + 零回归风险）。
    budget_chars: usize,
    /// 单条 JSON 数组最多保留的元素数（超出折叠为 `__truncated_remaining__` 计数）。
    max_json_array_items: usize,
    /// 单条 JSON 字符串字段最大长度（超出截断 + 计数标记）。
    max_string_len: usize,
    /// 日志/文本 verbatim 折叠：保留的头部行数。
    keep_head: usize,
    /// 日志/文本 verbatim 折叠：保留的尾部行数。
    keep_tail: usize,
    /// 内容指纹 → 原文 存储。
    originals: Mutex<OriginalStore>,
    /// 压缩统计（可观测）。
    stats: Mutex<CompressStats>,
}

impl Default for ToolCompressor {
    fn default() -> Self {
        // 默认预算 2000 字符：典型大工具输出（长日志、大 JSON、diff）均超此阈值触发压缩；
        // 小输出（绝大多数普通工具）零改动，规避任何下游精确字符串断言的回归风险。
        Self::with_budget(2000, 20, 512, 40, 20, 256)
    }
}

impl ToolCompressor {
    /// 全参数构造（测试/调优用）。
    pub fn with_budget(
        budget_chars: usize,
        max_json_array_items: usize,
        max_string_len: usize,
        keep_head: usize,
        keep_tail: usize,
        original_capacity: usize,
    ) -> Self {
        Self {
            budget_chars: budget_chars.max(1),
            max_json_array_items: max_json_array_items.max(1),
            max_string_len: max_string_len.max(1),
            keep_head: keep_head.max(1),
            keep_tail: keep_tail.max(1),
            originals: Mutex::new(OriginalStore::new(original_capacity)),
            stats: Mutex::new(CompressStats::default()),
        }
    }

    /// 压缩单次工具结果。
    ///
    /// - 错误/超时结果**不压缩**（需完整保真且体量小）。
    /// - 成功且原文 ≤ 预算：零改动返回（保真 + 零缓存风险）。
    /// - 成功且原文 > 预算：JSON 感知 canonical 压缩或文本 verbatim 折叠，并附内容指纹
    ///   供模型按需 `retrieve_full` 取回原文。
    pub fn compress(&self, _tool: &str, result: &ToolResult) -> ToolResult {
        let (data, metadata) = match result {
            ToolResult::Success { data, metadata } => (data, metadata),
            other => return other.clone(),
        };

        // 未超预算：零改动返回，不计入统计（与 `result` 逐字节一致）。
        if data.len() <= self.budget_chars {
            return result.clone();
        }

        let original = data.clone();
        let compressed = self.shrink(&original);

        // 指纹后缀固定长度（见下方 format! 模板）：
        //   \n[compressed sha256:<16hex> — retrieve full via tool_compress.retrieve(<16hex>)]
        // 防御性：仅当「压缩串 + 指纹后缀」真正短于原文才采用压缩，否则保留原文——
        // 避免小输出因附加管理开销反而膨胀，违背 W-4「缩小上下文、放大前缀缓存命中率」目标。
        const SUFFIX_LEN: usize = 19 + 16 + 40 + 16 + 2;
        if compressed.len() + SUFFIX_LEN >= original.len() {
            return result.clone();
        }

        let fp = {
            let mut g = self.originals.lock().unwrap_or_else(|e| e.into_inner());
            g.store(&original)
        };

        let out = format!(
            "{}\n[compressed sha256:{} — retrieve full via tool_compress.retrieve({})]",
            compressed, fp, fp
        );

        let mut meta = metadata.clone();
        meta.insert("compressed".to_string(), "true".to_string());
        meta.insert("original_fingerprint".to_string(), fp);

        {
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            s.compressed += 1;
            s.bytes_in += original.len() as u64;
            s.bytes_out += out.len() as u64;
        }

        ToolResult::Success {
            data: out,
            metadata: meta,
        }
    }

    /// 模型凭指纹取回被压缩前的完整原文（CCR 可逆检索）。指纹非法或已驱逐返回 None。
    pub fn retrieve_full(&self, fp: &str) -> Option<String> {
        let h = u64::from_str_radix(fp, 16).ok()?;
        let r = {
            let g = self.originals.lock().unwrap_or_else(|e| e.into_inner());
            g.get(h)
        };
        if r.is_some() {
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            s.retrieves += 1;
        }
        r
    }

    /// 登记 / 校验「稳定前缀」漂移（缓存×压缩联动守卫）。
    ///
    /// W-4 压缩器逐字节确定性输出，本方法把压缩器贡献的不变前缀指纹上报给进程级
    /// [`crate::cache::PrefixCacheTracker`]，返回 `true` 表示与历史一致（无漂移）、
    /// 前缀可持续命中 DeepSeek 服务端 KV 缓存；`false` 表示发生漂移，需排查压缩逻辑。
    pub fn observe_stable_prefix(&self, prefix: &str) -> bool {
        prefix_cache_tracker().observe_stable_prefix(prefix)
    }

    pub fn stats(&self) -> CompressStats {
        *self.stats.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 收缩主入口：优先 JSON 感知压缩，失败则文本 verbatim 折叠。
    fn shrink(&self, text: &str) -> String {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            let mut canon = String::new();
            render(&v, &mut canon);
            if canon.len() <= self.budget_chars {
                return canon;
            }
            // canonical 仍超预算：再做数组/字符串折叠
            let shrunk = self.json_shrink(&v);
            let mut w = String::new();
            render(&shrunk, &mut w);
            if w.len() <= self.budget_chars {
                return w;
            }
            // 仍超：对 canonical 文本做 verbatim 折叠
            return self.verbatim_fold(&canon);
        }
        self.verbatim_fold(text)
    }

    /// 递归折叠 JSON：截断超长字符串与超长数组（保留头 + 计数标记）。
    fn json_shrink(&self, v: &Value) -> Value {
        match v {
            Value::String(s) => {
                if s.len() > self.max_string_len {
                    Value::String(format!(
                        "{}…[+{} chars]",
                        &s[..self.max_string_len],
                        s.len() - self.max_string_len
                    ))
                } else {
                    v.clone()
                }
            }
            Value::Array(arr) => {
                if arr.len() > self.max_json_array_items {
                    let mut kept: Vec<Value> = arr
                        .iter()
                        .take(self.max_json_array_items)
                        .map(|e| self.json_shrink(e))
                        .collect();
                    let mut m = serde_json::Map::new();
                    m.insert(
                        "__truncated_remaining__".to_string(),
                        Value::Number(((arr.len() - self.max_json_array_items) as i64).into()),
                    );
                    kept.push(Value::Object(m));
                    Value::Array(kept)
                } else {
                    Value::Array(arr.iter().map(|e| self.json_shrink(e)).collect())
                }
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, val)| (k.clone(), self.json_shrink(val)))
                    .collect(),
            ),
            _ => v.clone(),
        }
    }

    /// 文本/日志 verbatim 折叠：保留头 + 尾，折叠中部，并去重连续重复行（确定性）。
    fn verbatim_fold(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= self.keep_head + self.keep_tail {
            return dedupe_consecutive(&lines).join("\n");
        }
        let head: Vec<&str> = lines[..self.keep_head].to_vec();
        let tail_start = lines.len() - self.keep_tail;
        let tail: Vec<&str> = lines[tail_start..].to_vec();
        let mut out = dedupe_consecutive(&head);
        out.push(format!(
            "… [{} lines collapsed] …",
            lines.len() - self.keep_head - self.keep_tail
        ));
        out.extend(dedupe_consecutive(&tail));
        out.join("\n")
    }
}

/// 递归渲染为规范 JSON（对象键排序、紧凑无空格）——保证相同逻辑内容逐字节一致。
fn render(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            // 用 serde 自带标量序列化保证转义规范且确定性
            out.push_str(&Value::String(s.clone()).to_string());
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                render(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push(':');
                render(&map[*k], out);
            }
            out.push('}');
        }
    }
}

/// 去重连续重复行（保留每行一次），输出确定性。
fn dedupe_consecutive(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for l in lines {
        if out.last().map_or(true, |p| p != l) {
            out.push((*l).to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(data: &str) -> ToolResult {
        ToolResult::Success {
            data: data.to_string(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn small_text_is_identity() {
        // 未超预算的输出逐字节不变，规避下游精确断言回归。
        let c = ToolCompressor::default();
        let r = c.compress("echo", &success("echo: hello"));
        assert_eq!(r.data(), Some("echo: hello"));
        if let ToolResult::Success { metadata, .. } = &r {
            assert!(metadata.get("compressed").is_none());
        }
    }

    #[test]
    fn large_json_is_canonical_and_smaller() {
        // 多行多空白的大对象：应被规范化为紧凑确定串（去空白、键排序）并显著变短。
        let c = ToolCompressor::with_budget(120, 5, 64, 4, 4, 16);
        let mut obj = String::from("{\n");
        for i in 0..80 {
            let comma = if i < 79 { "," } else { "" };
            obj.push_str(&format!("  \"k{}\": {}{}\n", i, i, comma));
        }
        obj.push('}');
        let r = c.compress("search", &success(&obj));
        let out = r.data().expect("应有压缩输出");
        // 规范后无多余空白、紧凑键排序
        assert!(out.contains("\"k0\":0"), "应规范紧凑: {out}");
        assert!(out.contains("\"k79\":79"), "应保留尾部字段: {out}");
        assert!(out.len() < obj.len(), "应比原文短");
    }

    #[test]
    fn same_input_compresses_deterministically() {
        // 相同逻辑内容两次压缩必须逐字节一致——这是缓存命中率的前提。
        let c = ToolCompressor::with_budget(50, 4, 16, 4, 4, 16);
        let json = r#"{ "z": 9, "y": 8, "x": 7 }"#;
        let a = c.compress("read", &success(json));
        let b = c.compress("read", &success(json));
        assert_eq!(a.data(), b.data(), "相同输入必须产出相同压缩串");
    }

    #[test]
    fn json_array_is_truncated_with_count() {
        // 大数组应截断并附剩余计数；压缩后（含指纹后缀）必须更短。
        let c = ToolCompressor::with_budget(100, 5, 64, 4, 4, 16);
        let arr: Vec<String> = (1..=100).map(|i| i.to_string()).collect();
        let arr = format!("[{}]", arr.join(","));
        let r = c.compress("list", &success(&arr));
        let out = r.data().expect("应有输出");
        assert!(out.contains("__truncated_remaining__"), "应带截断计数: {out}");
        assert!(out.contains("1"), "应保留前 5 项之首");
        assert!(out.contains("5"), "应保留前 5 项之尾");
        assert!(out.len() < arr.len(), "压缩后必须更短");
    }

    #[test]
    fn long_string_field_is_truncated() {
        // 超长字符串字段应截断并标记；压缩后必须更短。
        let c = ToolCompressor::with_budget(100, 8, 50, 4, 4, 16);
        let name = "a".repeat(300);
        let json = format!(r#"{{"name":"{}","id":1}}"#, name);
        let r = c.compress("read", &success(&json));
        let out = r.data().expect("应有输出");
        assert!(out.contains("…[+"), "超长字符串应被截断并标记: {out}");
        assert!(out.len() < json.len(), "压缩后必须更短");
    }

    #[test]
    fn log_verbatim_fold_keeps_head_and_tail() {
        // 多行日志：保留头尾、折叠中部；压缩后必须更短。
        let c = ToolCompressor::with_budget(120, 8, 64, 2, 2, 64);
        let log = (1..=40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let r = c.compress("tail", &success(&log));
        let out = r.data().expect("应有输出");
        assert!(out.contains("line 1"), "应保留头部");
        assert!(out.contains("line 40"), "应保留尾部");
        assert!(out.contains("lines collapsed"), "应折叠中部");
        assert!(!out.contains("line 20"), "中部应被折叠");
        assert!(out.len() < log.len(), "压缩后必须更短");
    }

    #[test]
    fn duplicate_lines_deduped() {
        // 大量重复行：去重后应显著变短。
        let c = ToolCompressor::with_budget(120, 8, 64, 4, 4, 64);
        let text: String = (0..60).map(|_| "a\nb\nc\n").collect();
        let text = text.trim_end();
        let r = c.compress("cat", &success(text));
        let out = r.data().expect("应有输出");
        assert!(out.contains("a\nb\nc"), "连续重复行应去重: {out}");
        assert!(out.len() < text.len(), "压缩后必须更短");
    }

    #[test]
    fn retrieve_full_roundtrip() {
        // 压缩后附指纹，模型可凭指纹取回原文（CCR 可逆）。
        let c = ToolCompressor::with_budget(60, 4, 32, 4, 4, 16);
        let mut items = String::new();
        for i in 0..30 {
            if i > 0 {
                items.push_str(", ");
            }
            items.push_str(&format!(
                "{{\"id\": {}, \"desc\": \"long description text padded to exceed budget and force real compression\"}}",
                i
            ));
        }
        let big = format!("{{\"items\": [{}]}}", items);
        let r = c.compress("read", &success(&big));
        let fp = if let ToolResult::Success { metadata, .. } = &r {
            metadata
                .get("original_fingerprint")
                .expect("应带指纹")
                .clone()
        } else {
            panic!("expected success result with fingerprint");
        };
        let original = c.retrieve_full(&fp).expect("应能取回原文");
        assert_eq!(original, big);
        assert_eq!(c.stats().retrieves, 1);
    }

    #[test]
    fn error_result_not_compressed() {
        let c = ToolCompressor::default();
        let err = ToolResult::Error {
            message: "boom ".repeat(500),
            code: "E".to_string(),
        };
        let r = c.compress("fail", &err);
        assert!(r.is_error());
        assert_eq!(r.data(), None);
    }

    #[test]
    fn stable_prefix_no_drift_on_deterministic_compression() {
        // 相同逻辑内容两次压缩必产出相同串 → 登记同一稳定前缀不应漂移（缓存×压缩联动守卫）。
        let c = ToolCompressor::with_budget(50, 4, 16, 4, 4, 16);
        let json = r#"{ "a": 1, "b": 2, "c": 3 }"#;
        let a = c.compress("read", &success(json));
        let b = c.compress("read", &success(json));
        assert_eq!(a.data(), b.data(), "相同输入必须产出相同压缩串");
        let p1 = a.data().unwrap_or("");
        let p2 = b.data().unwrap_or("");
        assert!(c.observe_stable_prefix(p1), "首次登记应返回 true");
        assert!(
            c.observe_stable_prefix(p2),
            "确定性压缩不应造成前缀漂移"
        );
    }

    #[test]
    fn stats_track_bytes_and_ratio() {
        // 大数组压缩后必须记录统计且 ratio < 1（防御性压缩保证有收益）。
        let c = ToolCompressor::with_budget(100, 5, 64, 4, 4, 16);
        let big: Vec<String> = (1..=100).map(|i| i.to_string()).collect();
        let big = format!("[{}]", big.join(","));
        let _ = c.compress("read", &success(&big));
        let s = c.stats();
        assert_eq!(s.compressed, 1);
        assert!(s.bytes_in >= big.len() as u64);
        assert!(s.ratio() < 1.0, "压缩后比例应小于 1");
    }
}
