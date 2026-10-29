//! 上下文压力：把台账读数折算成**跨实现一致**的 `contextStatus` 载荷。
//!
//! 存在理由（实测）：`ChatContextPressureChip` 早已上线，但它只认 `usage.status`
//! 里的 `contextStatus`，而这个键此前**只有侧车**发（`rpc_usage.py` 的 `_usage_row`）。
//! 纯 Rust 部署 —— 也就是安装包降级后的常见形态（AGENTS.md R61）—— 芯片永不出现：
//! 用户看不到「即将压缩」，直到撞上 provider 400。这里把缺的那一半补上。
//!
//! 口径与侧车逐键对齐，由本文件的跨实现守卫钉住（先例：
//! `gateway::methods` 的 `test_control_ui_security_headers_match_pyengine_middleware`）。

use serde_json::{json, Value};

/// 与 pyengine `_CONTEXT_WARNING_RATIO` 同值（`rpc_usage.py:25`）。
pub const WARNING_RATIO: f64 = 0.85;

/// 上下文压力档位 —— 给用户的**行动语言**，不是新的压缩策略。
///
/// `Watch`/`Advise` 是新增的可见提示层；真正的自动压缩仍由 `alert` 驱动
/// （`alert` 一个字节都没改，它有四个生产调用点：`chat.rs` 三处 + `plan/runner.rs`）。
/// 与 `model::tier::ModelTier`（模型档位）无关，这里说的是窗口占用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Normal,
    Watch,
    Advise,
    Compact,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Normal => "normal",
            Tier::Watch => "watch",
            Tier::Advise => "advise",
            Tier::Compact => "compact",
        }
    }
}

/// `[watch, advise, compact]` 三档下界。最紧那一档刻意压在 0.85 =
/// 既有 `WARNING_RATIO` 上：否则同一个会话会同时出现「`alert: normal`」和
/// 「`tier: compact`」，芯片和自动压缩各说各话。
pub const DEFAULT_LADDER: [f64; 3] = [0.60, 0.75, 0.85];

/// 拒绝坏配置：必须**严格递增**且都落在 `(0, 1]`，否则回落默认。
///
/// 等值阶梯（`[0.6, 0.6, 0.6]`）看着无害，实际让中间两档**永远不可达** ——
/// 用户配了四档却只看到两档，那比直接报错难查得多。
pub fn sanitize_ladder(l: [f64; 3]) -> [f64; 3] {
    let ok =
        l.iter().all(|v| v.is_finite() && *v > 0.0 && *v <= 1.0) && l[0] < l[1] && l[1] < l[2];
    if ok {
        l
    } else {
        DEFAULT_LADDER
    }
}

/// 压力 → 档位。边界**含下界**（`>=`）：0.85 就是 `compact`，与 `alert` 在 85%
/// 那条线同口径。改成 `>` 会让「正好 60%」掉回 `normal`（变异实测有牙）。
pub fn tier_for(pressure: f64, ladder: [f64; 3]) -> Tier {
    let [watch, advise, compact] = sanitize_ladder(ladder);
    if pressure >= compact {
        Tier::Compact
    } else if pressure >= advise {
        Tier::Advise
    } else if pressure >= watch {
        Tier::Watch
    } else {
        Tier::Normal
    }
}

/// 生效阶梯：`context.ladder`（坏值由 `sanitize_ladder` 回落），配置读不到时用默认。
///
/// **由调用方取一次再往下传**：`usage.status` 会逐会话产载荷，而 `Config::load()`
/// 每次都重新读盘 + 解析 YAML（无缓存）。把它放进逐行调用的 `context_status()`，
/// 就是每个会话一行 YAML 解析。
///
/// 单测下**不碰磁盘**：配置文件不存在时 `Config::load()` 会 `save()` 一份默认配置，
/// 于是「跑一次单测」可能写用户的 `~/.marguerite/config.yaml` —— 与 AGENTS.md R64
/// 「单测曾把夹具事件写进真实的 `~/.marguerite/usage.db`」是同一类缺陷，这里在源头
/// 掐掉。阶梯与配置的连线由 `persistence::config` 的纯解析用例钉住（不落盘）。
pub fn configured_ladder() -> [f64; 3] {
    if cfg!(test) {
        return DEFAULT_LADDER;
    }
    crate::persistence::config::Config::load()
        .map(|c| sanitize_ladder(c.context.ladder))
        .unwrap_or(DEFAULT_LADDER)
}

/// 窗口口径压力 `(pressure, window, window_source)`。
///
/// 分母是**窗口**而不是预算：`budget_for_window` 带 `clamp(32_000, …)`，小窗口会
/// 退到那个下限，于是预算口径系统性偏低（实测 10K 窗口 640 token 报 2%，真值是 6.4%）。
pub fn window_pressure(
    used_tokens: u64,
    budget: u64,
    context_window: u64,
) -> Option<(f64, u64, &'static str)> {
    // 窗口口径：优先台账记下的**真窗口**；只有台账从没记过窗口（升级前的旧会话）
    // 时才退到预算，并**如实改名** `windowSource` —— 退化的来源必须可见，
    // 因为预算是 `window*3/4` 再 clamp 的**不可逆**结果，此时的百分比系统性偏低。
    let (window, window_source) = if context_window > 0 {
        (context_window, "config")
    } else if budget > 0 {
        (budget, "budget_fallback")
    } else {
        return None;
    };
    let pressure = round6((used_tokens as f64 / window as f64).min(1.0));
    Some((pressure, window, window_source))
}

/// 由台账读数产出 `contextStatus`。
///
/// 窗口与预算**都**未知、或本会话还没有任何 token 时返回 `None` —— 宁可不显示，
/// 也不显示一个假的 0%。后一条是侧车的口径（`rpc_usage.py` 的 `_positive_int`
/// 把 0 判成 `None`，于是 `_context_status` 直接返回 `None`），照抄不是为了像，
/// 而是因为「新会话显示 0% 压力」和「不显示」在前端是两种可感知的状态。
pub fn context_status_from_readings(
    session: &str,
    used_tokens: u64,
    overhead_tokens: u64,
    budget: u64,
    context_window: u64,
    compaction_count: u32,
    ladder: [f64; 3],
) -> Option<Value> {
    if used_tokens == 0 {
        return None;
    }
    let (pressure, window, window_source) =
        window_pressure(used_tokens, budget, context_window)?;
    // `int(window * 0.85)`：Python 的 int() 与 Rust 的 `as u64` 都是**截断**，
    // 两边必须同为截断，否则同一个 4001/4000 窗口会在两个引擎上给出不同阈值。
    let threshold = ((window as f64) * WARNING_RATIO) as u64;
    // 阶梯**先洗再用**，载荷里的 `ladder` 就是洗过的那一份 ——
    // 否则会出现「`tier` 按默认算、`ladder` 字段回显坏配置」的自相矛盾载荷。
    let ladder = sanitize_ladder(ladder);
    // 两种拼写**共用同一个绑定**：`json!` 里同键写两遍会静默保留后者，
    // 各写一次字面量就等于给 camel/snake 两份口径留出漂移空间。
    let token_source = "session_context_tokens";
    Some(json!({
        "contextTokens": used_tokens,
        "contextWindowTokens": window,
        "thresholdTokens": threshold,
        "pressure": pressure,
        "tier": tier_for(pressure, ladder).as_str(),
        "ladder": ladder.to_vec(),
        "warningRatio": WARNING_RATIO,
        "compactionCount": compaction_count,
        "tokenSource": token_source,
        "windowSource": window_source,
        "context_tokens": used_tokens,
        "context_window_tokens": window,
        "threshold_tokens": threshold,
        "warning_ratio": WARNING_RATIO,
        "compaction_count": compaction_count,
        "token_source": token_source,
        "window_source": window_source,
        // 以下三键是 Rust 侧**附加**的（侧车没有，前端按可选处理）：
        // 分项来自台账里两个真实计数，不是猜出来的 —— 芯片要能拆出
        // 「聊天 vs 工具」才有行动价值（工具输出才是该截流的那一半）。
        "chatTokens": used_tokens.saturating_sub(overhead_tokens),
        "toolTokens": overhead_tokens,
        "session": session,
    }))
}

/// `round(x, 6)` —— 与侧车 `round(pressure, 6)` 同口径（前端按 6 位小数显示百分比）。
fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// 读活台账 → 载荷。会话不存在 = `None`（绝不凭空造一个 0 值会话）。
///
/// 阶梯由调用方给定，**这里不读配置**：`usage.status` 会为每个会话各调一次，
/// 而 `Config::load()` 没有缓存（每次都是读盘 + 解析 YAML，文件不存在时还会
/// **写回**一份默认配置）。把读配置放进本函数 = 把用户主目录的写操作塞进逐行
/// 路径，也让单测结果依赖本机那份私人配置。外层（RPC 分发）取一次再往下传。
pub fn context_status_payload(session: &str, ladder: [f64; 3]) -> Option<Value> {
    let st = crate::context::session::context_status(session);
    let used = st.get("used_tokens")?.as_u64()?;
    let overhead = st
        .get("overhead_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let budget = st.get("budget")?.as_u64()?;
    let window = st
        .get("context_window")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let count = st
        .get("compaction_count")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    context_status_from_readings(session, used, overhead, budget, window, count, ladder)
}

/// 给 `context.status` 视图补 `tier` / `ladder`，与芯片**同一个分母**（窗口压力）。
///
/// 档位只在这里算一次：`context.status` 与 `usage.status` 的行载荷都走
/// [`window_pressure`]，两条 RPC 对同一个会话不可能报出两个档位。阶梯由 RPC
/// 分发层取一次再传进来（`Config::load()` 无缓存，见 [`context_status_payload`]）。
///
/// 用 `ratio`（预算口径）算档位是**另一个数**：`budget_for_window` 的 clamp 下限会让
/// 小窗口的预算口径系统性偏低，同一个会话就会在两个 RPC 里显示两档。
pub fn attach_tier(view: &mut Value, ladder: [f64; 3]) {
    let reading = |key: &str| view.get(key).and_then(Value::as_u64).unwrap_or(0);
    let pressure = window_pressure(
        reading("used_tokens"),
        reading("budget"),
        reading("context_window"),
    )
    .map(|(p, _, _)| p)
    .unwrap_or(0.0);
    let ladder = sanitize_ladder(ladder);
    if let Some(map) = view.as_object_mut() {
        map.insert("tier".into(), json!(tier_for(pressure, ladder).as_str()));
        map.insert("ladder".into(), json!(ladder.to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// pyengine 的 `_context_status` 返回了哪些顶层键 —— 从**真实源码**解析，
    /// 不抄清单（抄来的清单会和代码一起过期，那是「恒绿守卫」的配方）。
    fn pyengine_context_status_keys() -> std::collections::BTreeSet<String> {
        const SRC: &str =
            include_str!("../../../pyengine/src/opensquilla/gateway/rpc_usage.py");
        let start = SRC
            .find("async def _context_status(")
            .expect("pyengine 里找不到 _context_status");
        // 截到**下一个顶层 def/class** 为止（`async` 与普通 `def` 都要认）：
        // 只找 `async def` 会把切片一路延伸进 `_usage_row`，把 cost_usd、sessionKey
        // 这些**别的字典**的键也抠进来 —— 那是一条会追着无关键判红的坏守卫。
        let rest = &SRC[start + 1..];
        let cut = ["\nasync def ", "\ndef ", "\nclass "]
            .iter()
            .filter_map(|needle| rest.find(needle))
            .min()
            .map(|offset| start + 1 + offset)
            .unwrap_or(SRC.len());
        let body = &SRC[start..cut];
        let ret = body
            .find("return {")
            .expect("_context_status 应返回字典字面量");
        body[ret..]
            .lines()
            .filter_map(|line| {
                let t = line.trim();
                t.strip_prefix('"')
                    .and_then(|s| s.split_once("\":"))
                    .map(|(key, _)| key.to_string())
            })
            .collect()
    }

    #[test]
    fn rust_context_status_covers_every_pyengine_key() {
        let ours = context_status_from_readings(
            "unit-t1",
            /* used_tokens */ 1000,
            /* overhead_tokens */ 0,
            /* budget */ 4000,
            /* context_window */ 8000,
            /* compaction_count */ 0,
            /* ladder */ DEFAULT_LADDER,
        );
        let ours = ours.expect("窗口与预算都给值时必须产出载荷");
        let keys = pyengine_context_status_keys();
        // 自证：解析器**真的**解析出了东西。空集合会让上面那个循环恒绿 ——
        // 那正是 AGENTS.md 里「单点采样 + 形式错误的模型 = 恒绿的守卫」的同型事故。
        assert!(
            keys.len() >= 8,
            "从 pyengine 源码解析出的键太少（{keys:?}），解析器可能已随重构失效"
        );
        let map = ours.as_object().unwrap();
        for k in &keys {
            assert!(
                map.contains_key(k),
                "pyengine 有 `{k}`，Rust 载荷没有 —— 芯片在纯 Rust 部署会永远不亮"
            );
        }
    }

    #[test]
    fn camel_and_snake_aliases_carry_the_same_value() {
        let v =
            context_status_from_readings("s", 6000, 1000, 8000, 10_000, 2, DEFAULT_LADDER).unwrap();
        for (a, b) in [
            ("contextTokens", "context_tokens"),
            ("contextWindowTokens", "context_window_tokens"),
            ("thresholdTokens", "threshold_tokens"),
            ("warningRatio", "warning_ratio"),
            ("compactionCount", "compaction_count"),
            ("tokenSource", "token_source"),
            ("windowSource", "window_source"),
        ] {
            assert_eq!(v[a], v[b], "{a} 与 {b} 必须同值（前端两种拼写都在读）");
        }
        // `pressure` 只有一种拼写（侧车就没有 snake 版），所以这里钉的是
        // 「它存在且是 (0,1] 的数」—— 计划里把 ("pressure","pressure") 当成
        // 别名对写进循环，那是一条自反恒真的断言，什么都没钉住。
        let p = v["pressure"].as_f64().expect("pressure 必须是数字");
        assert!(p > 0.0 && p <= 1.0, "pressure 应落在 (0,1]，实得 {p}");
        assert_eq!(p, 0.6, "6000/10000 必须是 0.6：{v}");
    }

    #[test]
    fn window_source_names_the_degradation_honestly() {
        let real =
            context_status_from_readings("s", 100, 0, 4000, 8000, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(real["windowSource"], "config");
        assert_eq!(real["contextWindowTokens"], 8000);
        let fallback = context_status_from_readings("s", 100, 0, 4000, 0, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(fallback["windowSource"], "budget_fallback");
        assert_eq!(fallback["contextWindowTokens"], 4000);
    }

    #[test]
    fn no_window_known_at_all_yields_no_payload() {
        // 0 窗口 + 0 预算 ⇒ 无法给出百分比。返回 None，而不是 0% 的假绿。
        assert!(context_status_from_readings("s", 100, 0, 0, 0, 0, DEFAULT_LADDER).is_none());
    }

    #[test]
    fn empty_session_yields_no_payload_like_pyengine() {
        // 侧车的 `_positive_int` 把 0 判成 None，于是 `_context_status` 返回 None
        // （rpc_usage.py:115-137）。照抄这条，新会话才不会显示一个假的 0%。
        assert!(context_status_from_readings("s", 0, 0, 4000, 8000, 0, DEFAULT_LADDER).is_none());
    }

    #[test]
    fn threshold_truncates_and_pressure_rounds_like_pyengine() {
        // 侧车写的是 `int(window * 0.85)`（截断）与 `round(pressure, 6)`。
        // 4001 * 0.85 = 3400.85 → 3400；四舍五入会得 3401，两个引擎就此分歧。
        let v = context_status_from_readings("s", 1, 0, 0, 4001, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(v["thresholdTokens"], 3400, "阈值必须截断取整，与 int() 同口径");
        // 1/3 = 0.333333… → 6 位小数
        let v = context_status_from_readings("s", 1, 0, 0, 3, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(v["pressure"], 0.333333);
        // 超窗必须夹在 1.0，不能报 1.37 让前端画出一条溢出进度条
        let v = context_status_from_readings("s", 1370, 0, 0, 1000, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(v["pressure"], 1.0);
    }

    #[test]
    fn chat_and_tool_split_is_a_real_partition_not_a_guess() {
        let v =
            context_status_from_readings("s", 6000, 1000, 8000, 10_000, 2, DEFAULT_LADDER).unwrap();
        assert_eq!(v["chatTokens"], 5000);
        assert_eq!(v["toolTokens"], 1000);
        assert_eq!(
            v["chatTokens"].as_u64().unwrap() + v["toolTokens"].as_u64().unwrap(),
            6000
        );
        // 开销**大于**总量（两条路径重复记账）时夹到 0，不给出负的聊天 token
        let v = context_status_from_readings("s", 100, 500, 8000, 10_000, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(v["chatTokens"], 0);
    }

    #[test]
    fn ladder_has_four_bands_and_uses_the_configured_thresholds() {
        let l = [0.60, 0.75, 0.85];
        assert_eq!(tier_for(0.59, l).as_str(), "normal");
        assert_eq!(tier_for(0.60, l).as_str(), "watch"); // 边界含下界
        assert_eq!(tier_for(0.74, l).as_str(), "watch");
        assert_eq!(tier_for(0.75, l).as_str(), "advise");
        assert_eq!(tier_for(0.84, l).as_str(), "advise");
        assert_eq!(tier_for(0.85, l).as_str(), "compact");
        assert_eq!(tier_for(1.00, l).as_str(), "compact");
    }

    #[test]
    fn a_misconfigured_ladder_falls_back_instead_of_silently_lying() {
        // 降序 / 越界 / 非有限值都是「配置写坏了」，不是「压力算坏了」。
        // 回落默认阶梯，并让调用方拿到的档位仍是单调的。
        for bad in [[0.9, 0.5, 0.3], [0.6, 0.6, 0.6], [f64::NAN, 0.7, 0.8]] {
            let t = tier_for(0.7, sanitize_ladder(bad));
            assert!(matches!(t, Tier::Watch | Tier::Advise), "坏阶梯应回落默认：{t:?}");
        }
    }

    /// 芯片显示的档位必须**在载荷里**，而不是让前端自备一份阈值表 ——
    /// 那正是「两处各算一遍」的起点（同一个会话在两个 RPC 上是两个数）。
    #[test]
    fn tier_and_ladder_travel_inside_the_payload() {
        let v =
            context_status_from_readings("s", 6000, 0, 8000, 10_000, 0, DEFAULT_LADDER).unwrap();
        assert_eq!(v["pressure"], 0.6);
        assert_eq!(v["tier"], "watch", "0.60 恰好踩在 watch 下界：{v}");
        assert_eq!(v["ladder"], json!(DEFAULT_LADDER.to_vec()));
        // 坏阶梯**进**、默认阶梯**出**：载荷不能回显一份和 tier 不同源的阈值
        let v =
            context_status_from_readings("s", 6000, 0, 8000, 10_000, 0, [0.9, 0.5, 0.3]).unwrap();
        assert_eq!(v["ladder"], json!(DEFAULT_LADDER.to_vec()));
        assert_eq!(v["tier"], "watch");
    }

    /// `attach_tier` 用的是**窗口**口径，不是 `ratio` 的预算口径。
    ///
    /// 这条区分是载荷与视图同源的唯一保证：`budget_for_window` 带
    /// `clamp(32_000, …)`，小窗口的预算被抬到下限，按 `ratio` 算档位就会让
    /// `context.status.tier` 与芯片的 `contextStatus.tier` 在同一个会话上显示两档。
    #[test]
    fn attach_tier_uses_the_window_denominator_not_the_budget_ratio() {
        let mut view = json!({
            "used_tokens": 6000u64,
            "budget": 32_000u64,
            "context_window": 10_000u64,
            "ratio": 0.1875,
        });
        attach_tier(&mut view, DEFAULT_LADDER);
        assert_eq!(view["tier"], "watch", "6000/10000=0.60 → watch（按 ratio 0.1875 会错判 normal）：{view}");
        assert_eq!(view["ladder"], json!(DEFAULT_LADDER.to_vec()));

        // 台账从没记过窗口（升级前的旧会话）：退到预算，档位仍与载荷同一个算法
        let mut legacy = json!({ "used_tokens": 6000u64, "budget": 8000u64 });
        attach_tier(&mut legacy, [0.5, 0.6, 0.9]);
        assert_eq!(legacy["tier"], "advise", "6000/8000=0.75 → advise：{legacy}");
        assert_eq!(legacy["ladder"], json!([0.5, 0.6, 0.9]));

        // 全新会话（一个 token 都没用）也要有档位，而且是 normal 而不是缺席
        let mut fresh =
            json!({ "used_tokens": 0u64, "budget": 8000u64, "context_window": 10_000u64 });
        attach_tier(&mut fresh, DEFAULT_LADDER);
        assert_eq!(fresh["tier"], "normal");
    }
}
