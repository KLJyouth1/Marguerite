//! 前缀缓存与记忆层状态方法族

use crate::gateway::methods::GatewayContext;
use serde_json::{json, Value};

/// `cache.status`：当前前缀缓存状态摘要。
///
/// **契约**（消费方两处，字段名与量纲必须一致）：
/// - `webui/src/views/ControlCenterView.vue` 的 `CacheStatus`：`ok/entries/lookups/hits/misses/
///   client_hit_rate/saved_prompt_tokens/semantic_enabled/tiers/exact_only_tools`，
///   其中 `client_hit_rate` 以 `toFixed(1) + '%'` 渲染 → **百分数**（0..100）。
/// - `webui/src/composables/chat/useChatSidebarPanels.ts` 的 `CacheStrip`：
///   `ok/server_cache_hit_rate/saved_prompt_tokens/prefix_hits/client_hit_rate`，
///   `server_cache_hit_rate` 同样按 `%` 渲染 → **百分数**。
///
/// **数据源全部为真实计量**（此前本函数返回硬编码 0，导致命中率永远显示 0）：
/// - 客户端命中：网关已安装多层缓存时取 [`MultiTierCache::stats`]（其 Tier 0 即共享精确缓存，
///   与 `default_prefix_cache()` **同源同对象**，因此绝不叠加两者，避免重复计数）；
///   未安装（CLI / TUI / 单测）时退化为进程默认精确缓存，数字仍真实（只是为空）。
/// - 服务端前缀缓存：取进程级 [`PrefixCacheTracker`]（DeepSeek `prompt_cache_hit_tokens` 归一），
///   与客户端命中率**分开上报**——两者量纲不同，混算即虚报。
pub fn cache_status() -> Value {
    let tracker = crate::cache::prefix_cache_tracker();
    let server = tracker.stats();
    let provider_backend = crate::memory::embedding::EmbeddingProvider::global();

    // 真实来源：多层缓存优先；未安装时退化为进程默认精确缓存。
    let (lookups, hits, misses, saved, entries, semantic_enabled, tiers, prefix, tool) =
        match crate::cache::semantic_layer() {
            Some(layer) => {
                let s = layer.stats();
                let prefix = layer.exact_arc().stats();
                (
                    s.lookups,
                    s.total_hits(),
                    s.misses,
                    s.saved_prompt_tokens,
                    layer.exact_arc().len() + layer.semantic_len(),
                    layer.semantic_enabled(),
                    json!({
                        "exact": {"hits": s.exact_hits, "saved_tokens": s.exact_saved_tokens},
                        "normalized": {"hits": s.normalized_hits, "saved_tokens": s.normalized_saved_tokens},
                        "semantic": {"hits": s.semantic_hits, "saved_tokens": s.semantic_saved_tokens},
                    }),
                    prefix,
                    // 工具路径（永远 ExactOnly）真实计数：来自多层缓存的精确策略计数
                    (s.tool_lookups, s.tool_hits, s.tool_misses),
                )
            }
            None => {
                // 观测端点读「进程级聚合视图」句柄：它是生产路径真正共享的那一份，
                // 因此在测试构建下也能如实反映真实流量（而 default_prefix_cache()
                // 在测试下刻意隔离，无法用于聚合）。
                let cache = crate::cache::process_prefix_cache();
                let s = cache.stats();
                (
                    s.lookups,
                    s.hits,
                    s.misses,
                    s.saved_prompt_tokens,
                    cache.len(),
                    false,
                    json!({
                        "exact": {"hits": s.hits, "saved_tokens": s.saved_prompt_tokens},
                        "normalized": {"hits": 0, "saved_tokens": 0},
                        "semantic": {"hits": 0, "saved_tokens": 0},
                    }),
                    s,
                    // 未安装多层缓存：无法把工具路径从共享精确缓存里分离，诚实返回 0
                    (0u64, 0u64, 0u64),
                )
            }
        };

    // 百分数（消费方直接拼 `%`）；lookups == 0 时诚实给 0 而非臆造。
    let client_hit_rate = if lookups == 0 {
        0.0
    } else {
        (hits as f64 / lookups as f64 * 100.0).clamp(0.0, 100.0)
    };

    // 预填充去重真实计数（进程级累计；无调用时为 0，绝不编造）
    let (pf_warmups, pf_dedup_skips) = crate::model::prefetch::prefetch_counters();

    json!({
        "ok": true,
        // ── ControlCenterView.CacheStatus 契约 ──
        "entries": entries,
        "lookups": lookups,
        "hits": hits,
        "misses": misses,
        "client_hit_rate": client_hit_rate,
        "saved_prompt_tokens": saved,
        "semantic_enabled": semantic_enabled,
        // Tier 2（语义近似）显式选入开关的真实取值（`MARGUERITE_SEMANTIC_CACHE=on` 才为 true）：
        // 与 `semantic_enabled` 分开上报——前者是「用户是否要求开」，后者是「缓存真的开了没有」
        "semantic_tier2_opted_in": crate::cache::semantic::tier2_opted_in_from_env(),
        "tiers": tiers,
        // invariant 8：携带工具定义的路径永远走 ExactOnly（语义近似命中会静默丢弃工具调用）
        "exact_only_tools": true,
        // ── useChatSidebarPanels.CacheStrip 契约（服务端前缀缓存，独立量纲）──
        // 主指标（W5.1）：coverage = 命中 token / 归一后的总输入 token。
        // `server_cache_hit_rate` 保留为 **coverage 的别名**（前端旧字段）——
        // 它不再是 read/(read+creation)：那个口径在 DeepSeek 上恒等于 100%，会骗人。
        "server_cache_hit_rate": server.cache_coverage_pct(),
        "server_cache_coverage_pct": server.cache_coverage_pct(),
        "server_total_input_tokens": server.total_input_tokens,
        // 合格请求命中率：达到 provider 最小可缓存长度的请求中至少命中一次的比例。
        "server_cache_request_hit_rate_pct": server.request_hit_rate_pct(),
        "server_cache_eligible_requests": server.eligible_requests,
        "server_cache_hit_requests": server.hit_requests,
        "prefix_hits": prefix.prefix_hits,
        // ── 诚实附加诊断（不参与 UI 主指标，供排障与「缓存为何没命中」定位）──
        "prefix_saved_tokens": prefix.prefix_saved_tokens,
        "server_cache_read_tokens": server.cache_read_total,
        "server_cache_creation_tokens": server.cache_creation_total,
        // 旧诊断口径（read/(read+creation)）：保留但**不**当主指标。
        "server_cache_write_adjusted_hit_rate_pct": server.write_adjusted_hit_rate_pct(),
        "server_calls": server.calls,
        "stable_prefix_drift": server.stable_prefix_drift,
        // W5.3：按来源归因（agent.stable_half / compress.tool_results …）。
        // 稳态下每个 scope 都在、值为 0 —— 0 本身是可核对的证据，不隐藏。
        "stable_prefix_drift_by_scope": server.stable_prefix_drift_by_scope,
        // ── 预填充去重真实计数（W-9）：实际发出的预热次数与去重跳过次数 ──
        "prefetch": {
            "warmups": pf_warmups,
            "dedup_skips": pf_dedup_skips,
        },
        // ── 语义层嵌入后端（诚实上报：语义命中到底基于什么向量）──
        // `provider.*` 是进程级生效后端（`embeddings.backend` + `real-embed` 决定：
        // 默认 `local-hash` 特征哈希；配置成 model 且 feature 开启则为 `fastembed-…`）。
        // `semantic_layer.*` 是多层缓存**实际持有**的嵌入器（未安装多层缓存时为 null）——
        // 两者分开上报，因为「配置了真实后端」与「语义缓存真的用了它」是两件事。
        "embedding": {
            "provider": {
                "backend": provider_backend.name(),
                "dim": provider_backend.dim(),
            },
            "semantic_layer": match crate::cache::semantic_layer() {
                Some(layer) => json!({
                    "backend": layer.embedder_backend_name(),
                    "dim": layer.embedder_dim(),
                }),
                None => Value::Null,
            },
        },
        // ── 工具路径真实计数：工具请求永远走 ExactOnly 策略（近似命中会静默丢弃工具调用）──
        // 未安装多层缓存时三项均为 0（配合 exact_only_tools 让前端区分「无数据」与「真 0」）。
        "tool_path": {
            "lookups": tool.0,
            "hits": tool.1,
            "misses": tool.2,
        },
    })
}

/// `router.selflearning.status`：自学习状态（Rust 兜底路径）。
///
/// **诚实基线**（`persistence::config::RouterSelfLearningConfig` 的文档明确约定）：
/// 「真训练由 Python 侧车 + dream cadence 提供；Rust 离线模式保留『启用但无数据/不训练』
/// 基线（self-learning 状态 RPC 显示 disabled/no_data）」。
///
/// 旧实现把 `captureEnabled` / `trainingReachable` **硬编码为 true**，并返回
/// `samples: 0`——前者在无侧车时宣称「训练可达」，是不实声明；后者会被 UI 读成
/// 「有样本且为 0」而非「无数据」。现按侧车真实可达性上报，无数据一律 `null`。
///
/// 字段名与量纲对齐前端 `useMemoryLearningSettings.ts` 的 `SelfLearningStatus`。
pub fn router_selflearning_status(ctx: &GatewayContext) -> Value {
    selflearning_status_for(
        &ctx.config,
        crate::gateway::server::py_client().is_some(),
    )
}

/// `router.selflearning.status` 的纯函数核心：只依赖配置与「侧车是否可达」这一个事实，
/// 因此两种侧车状态都能被确定性单测覆盖（不必依赖运行环境里侧车是否真的在线）。
pub fn selflearning_status_for(
    config: &crate::persistence::config::Config,
    sidecar: bool,
) -> Value {
    let enabled = config.squilla_router.enabled;

    json!({
        "enabled": enabled,
        // 捕获与训练可达性都必须以侧车真实可达为前提，不得凭配置臆断
        "captureEnabled": sidecar && enabled,
        "trainingReachable": sidecar,
        "dream": {
            "enabled": config.dream.enabled,
            "autoSchedule": config.dream.auto_schedule,
            "killSwitchActive": false,
        },
        // Rust 无本地训练产物台账：无侧车时为 null（诚实「无数据」），而非编造版本号
        "activeModel": Value::Null,
        // null = 无数据；若返回 0，UI 会把它当成「已捕获 0 条」的真实样本
        "samples": Value::Null,
        "gate": Value::Null,
        "lastReceipt": Value::Null,
        "error": if sidecar {
            Value::Null
        } else {
            json!("python sidecar offline: self-learning samples/gate unavailable")
        },
    })
}

/// `plans.capabilities`：规划能力描述
pub fn plans_capabilities() -> Value {
    json!({
        "version": 2,
        "capabilities": [
            "analyze",
            "create",
            "run",
            "cancel",
            "revise",
            "checklist",
            "run.advance",
            "replan",
            "parallel.run",
        ],
        "modes": ["default", "plan"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约守卫：两个前端消费方读取的字段必须全部存在，且量纲正确（百分数）。
    /// 这堵住「字段名漂移导致 UI 恒显示 0」这一曾真实发生的缺陷。
    #[test]
    fn cache_status_satisfies_frontend_contract() {
        let v = cache_status();

        // ControlCenterView.CacheStatus 必需字段
        for key in [
            "ok",
            "entries",
            "lookups",
            "hits",
            "misses",
            "client_hit_rate",
            "saved_prompt_tokens",
            "semantic_enabled",
            "tiers",
            "exact_only_tools",
        ] {
            assert!(
                v.get(key).is_some(),
                "cache.status 缺少前端必需字段 `{key}`（UI 会读到 undefined）"
            );
        }

        // useChatSidebarPanels.CacheStrip 必需字段
        for key in ["server_cache_hit_rate", "prefix_hits"] {
            assert!(
                v.get(key).is_some(),
                "cache.status 缺少对话页计量条字段 `{key}`"
            );
        }

        // 量纲：两个命中率都是百分数（消费方直接拼 `%`），故必须落在 0..=100
        for key in ["client_hit_rate", "server_cache_hit_rate"] {
            let rate = v[key].as_f64().expect("命中率必须是数字");
            assert!(
                (0.0..=100.0).contains(&rate),
                "`{key}` 必须是以百分数表示（0..=100），实际 {rate}"
            );
        }

        // tiers 三层结构齐全（UI 逐层渲染折线）
        let tiers = v["tiers"].as_object().expect("tiers 必须是对象");
        for tier in ["exact", "normalized", "semantic"] {
            let row = tiers
                .get(tier)
                .unwrap_or_else(|| panic!("tiers 缺少 `{tier}` 层"))
                .as_object()
                .expect("层必须是对象");
            assert!(row.contains_key("hits"), "tiers.{tier} 缺少 hits");
            assert!(
                row.contains_key("saved_tokens"),
                "tiers.{tier} 缺少 saved_tokens"
            );
        }

        // 诚实性：无任何流量时命中率必须是 0，而不是臆造的非零值
        assert!(v["ok"].as_bool().unwrap(), "ok 应为 true");

        // W5.3：漂移必须可按来源归因，且「总量 == 各 scope 之和」
        // （两个字段来自同一次 `stats()` 快照，因此这条断言不受并行用例影响）。
        let drift_by_scope = v["stable_prefix_drift_by_scope"]
            .as_object()
            .expect("stable_prefix_drift_by_scope 必须是 scope → 计数 的对象");
        let summed: u64 = drift_by_scope.values().map(|n| n.as_u64().unwrap_or(0)).sum();
        assert_eq!(
            v["stable_prefix_drift"].as_u64().unwrap(),
            summed,
            "stable_prefix_drift 必须等于各 scope 之和"
        );
    }

    /// 计数一致性：hits + misses 必须等于 lookups（否则 UI 的命中率与明细自相矛盾）。
    #[test]
    fn cache_status_counters_are_internally_consistent() {
        let v = cache_status();
        let lookups = v["lookups"].as_u64().unwrap();
        let hits = v["hits"].as_u64().unwrap();
        let misses = v["misses"].as_u64().unwrap();
        assert_eq!(
            hits + misses,
            lookups,
            "命中 + 未命中必须等于总查询数，否则命中率与明细自相矛盾"
        );
    }

    /// 真实数据源接线：向**端点真正读取的那个**缓存写入并命中后，cache.status 必须如实反映
    /// （此前硬编码 0 —— 这条测试就是那个缺陷的回归守卫）。
    ///
    /// 为什么必须按分支选对象（R50 修复的并行串扰）：`cache_status()` 在**已安装多层缓存**时
    /// 读的是层统计（`semantic_layer().stats()`），未安装才退化为 `process_prefix_cache()`。
    /// 而安装多层缓存是**进程级**副作用（OnceLock），会被并行用例触发；此前本用例写死只往
    /// `process_prefix_cache()` 写，一旦层已安装，端点改读层统计 → 写入不可见、断言恒假红
    /// （实测：`查询数应随真实流量增长（3 → 3）`）。故这里与端点用**同一套选择逻辑**驱动流量。
    #[test]
    fn cache_status_reflects_real_prefix_cache_traffic() {
        const PROBE: &str = "cache-status-regression-probe";
        const PROBE_TOKENS: u32 = 42;

        let before = cache_status();
        let base_lookups = before["lookups"].as_u64().unwrap();
        let base_hits = before["hits"].as_u64().unwrap();
        let base_saved = before["saved_prompt_tokens"].as_u64().unwrap();

        // 与 cache_status() 同源：装了多层缓存 → 驱动层；否则 → 驱动进程级精确缓存
        match crate::cache::semantic_layer() {
            Some(layer) => {
                layer.put(PROBE, "probe", PROBE_TOKENS, 1);
                let entry = layer.get(PROBE).expect("刚写入的 key 必须命中");
                assert_eq!(entry.content, "probe");
            }
            None => {
                use crate::cache::{hash_text, now_ms, process_prefix_cache, CacheEntry};
                let cache = process_prefix_cache();
                cache.put(
                    hash_text(PROBE),
                    CacheEntry {
                        content: "probe".into(),
                        prompt_tokens: PROBE_TOKENS,
                        completion_tokens: 1,
                        last_used_ms: now_ms(),
                    },
                );
                let entry = cache.get(hash_text(PROBE)).expect("刚写入的 key 必须命中");
                assert_eq!(entry.content, "probe");
            }
        }

        let after = cache_status();
        let lookups = after["lookups"].as_u64().unwrap();
        let hits = after["hits"].as_u64().unwrap();
        let saved = after["saved_prompt_tokens"].as_u64().unwrap();

        // 命中与查询数都必须真实增长（硬编码 0 的实现会让这两条断言失败）
        assert!(
            lookups > base_lookups,
            "查询数应随真实流量增长（{base_lookups} → {lookups}）"
        );
        assert!(
            hits > base_hits,
            "命中数应随真实命中增长（{base_hits} → {hits}）"
        );
        assert!(
            after["client_hit_rate"].as_f64().unwrap() > 0.0,
            "有真实命中后客户端命中率必须 > 0（此前恒为 0）"
        );
        assert!(
            saved >= base_saved + PROBE_TOKENS as u64,
            "命中应如实累计省下的 prompt token（{base_saved} → {saved}）"
        );
    }

    /// 服务端前缀缓存指标独立上报（与客户端命中率量纲不同，不可混算）。
    ///
    /// 全局 tracker 是进程级累计，故断言按**前后差值**计算期望，避免并行/多次运行时
    /// 依赖「此前没人上报过」的隐含假设。
    #[test]
    fn cache_status_reports_server_side_hit_rate_separately() {
        use crate::cache::{min_cache_tokens, prefix_cache_tracker, total_input_tokens};

        let before = prefix_cache_tracker().stats();
        // Anthropic 口径：input=1100，read=750，creation=250 → 总输入 2100。
        prefix_cache_tracker().report(
            total_input_tokens(1100, 750, 250, true),
            750,
            250,
            min_cache_tokens(true),
        );
        let after = prefix_cache_tracker().stats();
        let v = cache_status();

        let coverage = after.cache_coverage_pct();
        let write_adjusted = after.write_adjusted_hit_rate_pct();
        assert!(
            (v["server_cache_hit_rate"].as_f64().unwrap() - coverage).abs() < 1e-6,
            "server_cache_hit_rate 必须是 coverage 的别名"
        );
        assert!(
            (v["server_cache_coverage_pct"].as_f64().unwrap() - coverage).abs() < 1e-6
        );
        assert!(
            (v["server_cache_write_adjusted_hit_rate_pct"].as_f64().unwrap() - write_adjusted)
                .abs() < 1e-6
        );
        assert_eq!(
            v["server_total_input_tokens"].as_u64().unwrap(),
            after.total_input_tokens
        );
        assert_eq!(
            v["server_cache_read_tokens"].as_u64().unwrap(),
            before.cache_read_total + 750
        );
        assert_eq!(
            v["server_cache_creation_tokens"].as_u64().unwrap(),
            before.cache_creation_total + 250
        );
        assert_eq!(
            v["server_cache_eligible_requests"].as_u64().unwrap(),
            after.eligible_requests
        );
    }

    /// 契约守卫：`router.selflearning.status` 必须给出前端 `SelfLearningStatus`
    /// 所需的字段与**正确类型**（`samples`/`gate` 是对象或 null，绝不是数字）。
    ///
    /// 旧实现返回 `samples: 0`，前端读 `status.samples?.total` 恒为 undefined；
    /// `dream.cadence_ms` 也不是前端读的 `autoSchedule`。
    #[test]
    fn selflearning_status_satisfies_frontend_contract() {
        let config = crate::persistence::config::Config::default();
        let v = selflearning_status_for(&config, true);

        for key in [
            "enabled",
            "captureEnabled",
            "trainingReachable",
            "dream",
            "samples",
            "gate",
            "lastReceipt",
        ] {
            assert!(
                v.get(key).is_some(),
                "router.selflearning.status 缺少前端必需字段 `{key}`"
            );
        }

        assert!(v["enabled"].is_boolean(), "enabled 必须是布尔");
        assert!(v["captureEnabled"].is_boolean(), "captureEnabled 必须是布尔");
        assert!(
            v["trainingReachable"].is_boolean(),
            "trainingReachable 必须是布尔"
        );
        // samples / gate 必须是对象或 null（前端按 `?.total` / `?.wouldTrain` 读取）
        for key in ["samples", "gate", "lastReceipt"] {
            assert!(
                v[key].is_null() || v[key].is_object(),
                "`{key}` 必须是对象或 null，实际 {}（数字会被前端误读为「有数据」）",
                v[key]
            );
        }
        let dream = v["dream"].as_object().expect("dream 必须是对象");
        assert!(dream.contains_key("enabled"), "dream 缺少 enabled");
        assert!(
            dream.contains_key("autoSchedule"),
            "dream 缺少 autoSchedule（前端读此字段判断训练是否随 dream 排程）"
        );
    }

    /// 诚实性守卫：Rust 兜底路径**不得**在无 Python 侧车时宣称训练可达。
    ///
    /// 依据 `persistence::config::RouterSelfLearningConfig` 的文档约定：
    /// Rust 离线模式应显示 disabled / no_data。
    #[test]
    fn selflearning_status_never_fakes_reachability_without_sidecar() {
        let config = crate::persistence::config::Config::default();

        // 无侧车：训练不可达、样本/门禁为 null、并给出可诊断说明
        let offline = selflearning_status_for(&config, false);
        assert_eq!(
            offline["trainingReachable"], false,
            "无侧车时必须 trainingReachable=false（旧实现硬编码 true，是不实声明）"
        );
        assert_eq!(offline["captureEnabled"], false, "无侧车时不可能捕获样本");
        assert!(offline["samples"].is_null(), "无侧车时 samples 必须是 null 而非 0");
        assert!(offline["gate"].is_null(), "无侧车时 gate 必须是 null");
        assert!(
            offline["error"].as_str().is_some_and(|s| s.contains("sidecar")),
            "无侧车时应给出可诊断的 error 说明"
        );

        // 有侧车：可达性与配置一致，且不再报错
        let online = selflearning_status_for(&config, true);
        assert_eq!(online["trainingReachable"], true);
        assert!(online["error"].is_null(), "侧车在线时不应有 error");
    }

    /// 预填充去重计数与工具路径计数：字段齐备，量纲自洽（hits + misses == lookups）。
    #[test]
    fn cache_status_exposes_prefetch_and_tool_path_real_counters() {
        let v = cache_status();
        assert!(v["prefetch"].get("warmups").is_some(), "缺 prefetch.warmups");
        assert!(
            v["prefetch"].get("dedup_skips").is_some(),
            "缺 prefetch.dedup_skips"
        );
        for key in ["lookups", "hits", "misses"] {
            assert!(v["tool_path"].get(key).is_some(), "缺 tool_path.{key}");
        }
        let l = v["tool_path"]["lookups"].as_u64().unwrap();
        let h = v["tool_path"]["hits"].as_u64().unwrap();
        let m = v["tool_path"]["misses"].as_u64().unwrap();
        assert_eq!(h + m, l, "工具路径命中 + 未命中必须等于查询数");
    }

    /// 语义层嵌入后端必须**如实上报**（否则「语义缓存到底接了真实 embedding 没有」无从判断）。
    ///
    /// 两条不变量：① `embedding.provider` 与进程级 `EmbeddingProvider` 逐字一致；
    /// ② 装了多层缓存时 `embedding.semantic_layer.dim` 必须等于该缓存实际持有的嵌入器维度
    /// （不允许上报一个「配置值」而不是「真的在用的值」）。
    #[test]
    fn cache_status_reports_real_semantic_embedder_backend() {
        let v = cache_status();
        let provider = crate::memory::embedding::EmbeddingProvider::global();
        assert_eq!(v["embedding"]["provider"]["backend"], provider.name());
        assert_eq!(
            v["embedding"]["provider"]["dim"].as_u64().unwrap() as usize,
            provider.dim()
        );
        // 选入开关与「缓存真的开了没有」两件事都得如实上报
        assert_eq!(
            v["semantic_tier2_opted_in"].as_bool().unwrap(),
            crate::cache::semantic::tier2_opted_in_from_env()
        );
        assert!(
            v["semantic_enabled"].is_boolean(),
            "semantic_enabled 必须是布尔（前端据此判断语义层是否参与）"
        );

        match crate::cache::semantic_layer() {
            Some(layer) => {
                assert_eq!(
                    v["embedding"]["semantic_layer"]["backend"],
                    layer.embedder_backend_name()
                );
                assert_eq!(
                    v["embedding"]["semantic_layer"]["dim"].as_u64().unwrap() as usize,
                    layer.embedder_dim(),
                    "上报的维度必须是语义缓存真的在用的嵌入器维度"
                );
            }
            None => assert!(
                v["embedding"]["semantic_layer"].is_null(),
                "未安装多层缓存时必须如实报 null，而不是编造一个维度"
            ),
        }
    }
}
