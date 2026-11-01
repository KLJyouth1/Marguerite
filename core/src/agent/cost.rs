//! 逐轮美元换算：单价表 + `cost_usd`（缺口 7 的"最后一步"）。
//!
//! ## 为什么需要它
//!
//! `agent/turn_budget.rs`（P2-9）已经能按 **steps / secs / tokens** 三档熔断，`usage` 台账也已经
//! 逐次记录 `prompt_tokens` / `completion_tokens`（`usage.db::usage_events`）——
//! **唯一缺的是"乘价"**：把 token 数换算成美元，才能回答"这一轮花了多少钱"。
//!
//! ## 三条刻意的设计决定
//!
//! 1. **不内置任何价格**。价格随供应商调整、随合约变化，写死在代码里就是一份会过期的谎言。
//!    价目表由调用方注入（`Pricing::from_spec` / `Pricing::from_env`），空表是**合法状态**。
//! 2. **未定价时字段缺省，绝不写 0**。`cost_usd: 0.00` 会让"没有价目表"看起来像"免费"，
//!    这是最容易被误读的一种诚实性缺陷。`TurnCost::to_json` 在 `usd == None` 时**不产生**
//!    `cost_usd` 键。
//! 3. **模型名只在唯一前缀命中时回落**。`gpt-x-2026-01-01` 可以落到 `gpt-x`；但若表里同时有
//!    `gpt` 与 `gpt-x` 两个前缀，回落就是猜——此时返回 `None`，宁可缺省也不给可能错的数字。

use std::collections::HashMap;

use serde_json::{json, Value};

/// 单一模型的单价（美元 / 百万 token）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    /// 输入（prompt）单价
    pub in_per_mtok: f64,
    /// 输出（completion）单价
    pub out_per_mtok: f64,
}

impl Price {
    /// 构造单价（美元 / 百万 token）。
    pub fn new(in_per_mtok: f64, out_per_mtok: f64) -> Self {
        Self {
            in_per_mtok,
            out_per_mtok,
        }
    }
}

/// 价目表：`(provider, model) → Price`。键统一转小写，查询大小写不敏感。
#[derive(Debug, Clone, Default)]
pub struct Pricing {
    table: HashMap<(String, String), Price>,
}

impl Pricing {
    /// 空表（合法状态：表示"没有价目表"，不是错误）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条单价。`provider` / `model` 大小写不敏感。
    pub fn insert(&mut self, provider: &str, model: &str, price: Price) {
        self.table
            .insert((provider.to_ascii_lowercase(), model.to_ascii_lowercase()), price);
    }

    /// 表是否为空。
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// 条目数。
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// 查询单价。
    ///
    /// 先精确命中；未命中再看**前缀**回落（`model` 以已登记名开头，用于吃掉日期/版本后缀）。
    /// 前缀命中**不唯一**时返回 `None` —— 多义即猜，猜价的代价是报出一个错的账单。
    pub fn lookup(&self, provider: &str, model: &str) -> Option<Price> {
        let p = provider.to_ascii_lowercase();
        let m = model.to_ascii_lowercase();
        if let Some(hit) = self.table.get(&(p.clone(), m.clone())) {
            return Some(*hit);
        }
        let mut found: Option<Price> = None;
        for ((kp, km), price) in &self.table {
            if *kp != p || !m.starts_with(km.as_str()) {
                continue;
            }
            if found.is_some() {
                return None; // 多义前缀：不猜
            }
            found = Some(*price);
        }
        found
    }

    /// 从紧凑规格串解析：`provider/model=in/out, provider/model=in/out`。
    ///
    /// 非法片段**整条跳过**（不 panic、不猜），因为这是一份可选配置，
    /// 一个笔误不该让整个计价链路失效。
    pub fn from_spec(spec: &str) -> Self {
        let mut p = Self::new();
        for entry in spec.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let Some((who, rates)) = entry.split_once('=') else {
                continue;
            };
            let Some((provider, model)) = who.trim().split_once('/') else {
                continue;
            };
            let Some((i, o)) = rates.trim().split_once('/') else {
                continue;
            };
            let (Ok(i), Ok(o)) = (i.trim().parse::<f64>(), o.trim().parse::<f64>()) else {
                continue;
            };
            // 注意 parse::<f64> 对 "nan"/"inf" 是**成功**的，且 NaN 参与任何比较都是
            // false —— 只写 `i < 0.0` 拦不住它（本条正是单测抓出来的真漏洞）。
            // 非有限值（NaN/±inf）与负价一样，都不得进表。
            if !i.is_finite() || !o.is_finite() || i < 0.0 || o < 0.0 {
                continue;
            }
            p.insert(provider.trim(), model.trim(), Price::new(i, o));
        }
        p
    }

    /// 读环境变量 `MARGUERITE_PRICE_TABLE`（未设置/为空 → 空表）。
    pub fn from_env() -> Self {
        match std::env::var("MARGUERITE_PRICE_TABLE") {
            Ok(spec) => Self::from_spec(&spec),
            Err(_) => Self::new(),
        }
    }
}

/// 按单价把 token 数换算成美元。纯函数，便于对账。
pub fn cost_usd(price: Price, tokens_in: u64, tokens_out: u64) -> f64 {
    tokens_in as f64 / 1_000_000.0 * price.in_per_mtok
        + tokens_out as f64 / 1_000_000.0 * price.out_per_mtok
}

/// 一轮对话的用量与花费。
///
/// `usd == None` 是**一等状态**：代表"这一轮没有可用的价目"，不是 0 元。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnCost {
    /// 本轮输入 token
    pub tokens_in: u64,
    /// 本轮输出 token
    pub tokens_out: u64,
    /// 本轮花费（美元）；`None` = 未定价
    pub usd: Option<f64>,
}

impl TurnCost {
    /// 有单价的一轮：直接算出美元。
    pub fn priced(price: Price, tokens_in: u64, tokens_out: u64) -> Self {
        Self {
            tokens_in,
            tokens_out,
            usd: Some(cost_usd(price, tokens_in, tokens_out)),
        }
    }

    /// 无单价的一轮：只记 token，**不伪造** `0.00`。
    pub fn unpriced(tokens_in: u64, tokens_out: u64) -> Self {
        Self {
            tokens_in,
            tokens_out,
            usd: None,
        }
    }

    /// 按 provider/model 查表；查不到即 `unpriced`。
    pub fn from_pricing(
        pricing: &Pricing,
        provider: &str,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
    ) -> Self {
        match pricing.lookup(provider, model) {
            Some(price) => Self::priced(price, tokens_in, tokens_out),
            None => Self::unpriced(tokens_in, tokens_out),
        }
    }

    /// 并入 `meta` 的 JSON。**未定价时不含 `cost_usd` 键**。
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "tokens_in": self.tokens_in,
            "tokens_out": self.tokens_out,
        });
        if let Some(usd) = self.usd {
            v["cost_usd"] = json!(usd);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing() -> Pricing {
        let mut p = Pricing::new();
        p.insert("DeepSeek", "deepseek-chat", Price::new(0.27, 1.10));
        p.insert("Ollama", "qwen2.5", Price::new(0.0, 0.0));
        p
    }

    #[test]
    fn cost_arithmetic_per_million() {
        // 1M 输入 @0.27 + 1M 输出 @1.10 = 1.37
        let c = cost_usd(Price::new(0.27, 1.10), 1_000_000, 1_000_000);
        assert!((c - 1.37).abs() < 1e-12, "got {c}");
        // 0 token 必须是 0，而不是 NaN
        assert_eq!(cost_usd(Price::new(3.0, 15.0), 0, 0), 0.0);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let p = pricing();
        assert_eq!(p.lookup("deepseek", "DeepSeek-Chat"), Some(Price::new(0.27, 1.10)));
    }

    #[test]
    fn lookup_falls_back_on_unique_prefix() {
        let p = pricing();
        // 只有 deepseek-chat 一个前缀命中 → 采用
        assert_eq!(
            p.lookup("deepseek", "deepseek-chat-2026-01-01"),
            Some(Price::new(0.27, 1.10))
        );
    }

    #[test]
    fn ambiguous_prefix_is_not_guessed() {
        let mut p = Pricing::new();
        p.insert("x", "m", Price::new(1.0, 1.0));
        p.insert("x", "m-long", Price::new(2.0, 2.0));
        // "m-long-v2" 同时以 "m" 与 "m-long" 开头 → 多义，拒绝猜
        assert_eq!(p.lookup("x", "m-long-v2"), None);
        // 精确命中仍然可用
        assert_eq!(p.lookup("x", "m-long"), Some(Price::new(2.0, 2.0)));
    }

    #[test]
    fn unpriced_omits_cost_usd_key() {
        let v = TurnCost::unpriced(120, 30).to_json();
        assert_eq!(v["tokens_in"], json!(120));
        assert_eq!(v["tokens_out"], json!(30));
        assert!(
            v.get("cost_usd").is_none(),
            "未定价必须缺省，不能写 0.00（0 会被读成免费）"
        );
    }

    #[test]
    fn priced_writes_cost_usd() {
        let v = TurnCost::priced(Price::new(2.0, 8.0), 500_000, 100_000).to_json();
        let usd = v["cost_usd"].as_f64().expect("应有 cost_usd");
        assert!((usd - 1.8).abs() < 1e-12, "got {usd}");
    }

    #[test]
    fn from_pricing_unknown_model_degrades_to_unpriced() {
        let p = pricing();
        let c = TurnCost::from_pricing(&p, "nope", "nope", 10, 20);
        assert_eq!(c.usd, None);
        assert_eq!(c.tokens_in, 10);
        assert_eq!(c.tokens_out, 20);
    }

    #[test]
    fn spec_parsing_skips_malformed_entries() {
        let p = Pricing::from_spec(
            "deepseek/deepseek-chat=0.27/1.10 , badentry, x/y, a/b=nan/1, c/d=-1/2",
        );
        assert_eq!(p.len(), 1, "只有第一条合法");
        assert_eq!(p.lookup("deepseek", "deepseek-chat"), Some(Price::new(0.27, 1.10)));
        // 负价与 NaN 都不得进表
        assert!(p.lookup("a", "b").is_none());
        assert!(p.lookup("c", "d").is_none());
    }

    #[test]
    fn empty_pricing_is_valid_state() {
        let p = Pricing::new();
        assert!(p.is_empty());
        assert_eq!(p.lookup("any", "any"), None);
    }
}
