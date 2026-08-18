//! 用量仪表（Track D4，聚合面板打开 / Agent 调用）。
//!
//! 纯聚合，无 IO，可单测。生产环境可对接 `core/src/usage` 做持久化与时间线。

use std::collections::HashMap;

/// 用量统计：面板打开次数 + Agent 调用次数。
#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    /// 面板 id -> 打开次数。
    pub panel_opens: HashMap<String, u64>,
    /// Agent id -> 调用次数。
    pub agent_invocations: HashMap<String, u64>,
}

impl UsageStats {
    /// 空统计。
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次面板打开（累加）。
    pub fn record_panel_open(&mut self, panel_id: &str) {
        *self.panel_opens.entry(panel_id.to_string()).or_insert(0) += 1;
    }

    /// 记录一次 Agent 调用（累加）。
    pub fn record_agent_invocation(&mut self, agent_id: &str) {
        *self.agent_invocations.entry(agent_id.to_string()).or_insert(0) += 1;
    }

    /// 面板打开 Top-N（降序，平局按 id 字典序稳定）。
    pub fn top_panels(&self, n: usize) -> Vec<(String, u64)> {
        let mut v: Vec<_> = self
            .panel_opens
            .iter()
            .map(|(k, c)| (k.clone(), *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(n);
        v
    }

    /// Agent 调用 Top-N（降序，平局按 id 字典序稳定）。
    pub fn top_agents(&self, n: usize) -> Vec<(String, u64)> {
        let mut v: Vec<_> = self
            .agent_invocations
            .iter()
            .map(|(k, c)| (k.clone(), *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_top() {
        let mut u = UsageStats::new();
        u.record_panel_open("chat");
        u.record_panel_open("chat");
        u.record_panel_open("tasks");
        u.record_agent_invocation("creator");
        u.record_agent_invocation("creator");
        u.record_agent_invocation("creator");
        u.record_agent_invocation("office");

        assert_eq!(u.top_panels(1), vec![("chat".to_string(), 2)]);
        assert_eq!(u.top_agents(1), vec![("creator".to_string(), 3)]);
        assert_eq!(u.top_panels(10).len(), 2);
        assert_eq!(u.top_agents(10).len(), 2);

        // 累加正确
        u.record_panel_open("chat");
        assert_eq!(u.top_panels(1), vec![("chat".to_string(), 3)]);
    }

    #[test]
    fn top_respects_n() {
        let mut u = UsageStats::new();
        for i in 0..5 {
            u.record_panel_open(&format!("p{i}"));
        }
        assert_eq!(u.top_panels(3).len(), 3);
        assert_eq!(u.top_panels(0).len(), 0);
    }
}
