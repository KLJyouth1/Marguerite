//! 家庭共享记忆与知识图谱（WP7）：纯内存、确定性的家庭事实存储。
//!
//! 不依赖网络、模型或持久化后端；只负责按实体召回与按时间倒序取最近事实。

/// 一条家庭事实：某个实体在某个时间点被记录下来的一条知识。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeFact {
    /// 事实唯一 id（自增）。
    pub id: u64,
    /// 实体名，如 "玄关灯" / "爸爸" / "周末"。
    pub entity: String,
    /// 事实内容，如 "睡前关"。
    pub fact: String,
    /// 记录时间戳（epoch 秒）。
    pub ts: i64,
}

/// 纯内存家庭记忆存储。
#[derive(Debug, Default)]
pub struct HomeMemoryStore {
    facts: Vec<HomeFact>,
    next_id: u64,
}

impl HomeMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条事实，返回分配到的 id。
    pub fn add_fact(&mut self, entity: impl Into<String>, fact: impl Into<String>, ts: i64) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.facts.push(HomeFact {
            id,
            entity: entity.into(),
            fact: fact.into(),
            ts,
        });
        id
    }

    /// 按实体召回：精确匹配优先，其次子串包含匹配（大小写敏感）。
    /// 命中顺序为插入顺序。
    pub fn query_by_entity(&self, entity: &str) -> Vec<&HomeFact> {
        let mut exact: Vec<&HomeFact> = Vec::new();
        let mut partial: Vec<&HomeFact> = Vec::new();
        for f in &self.facts {
            if f.entity == entity {
                exact.push(f);
            } else if !entity.is_empty() && f.entity.contains(entity) {
                partial.push(f);
            }
        }
        exact.extend(partial);
        exact
    }

    /// 按时间戳倒序取最近的 n 条（ts 大者在前）。
    pub fn recent(&self, n: usize) -> Vec<&HomeFact> {
        if n == 0 || self.facts.is_empty() {
            return Vec::new();
        }
        let mut indexed: Vec<(usize, &HomeFact)> = self.facts.iter().enumerate().collect();
        // 按 ts 倒序；ts 相同则按插入顺序（后插入者更近）作为稳定次序。
        indexed.sort_by(|a, b| b.1.ts.cmp(&a.1.ts).then_with(|| b.0.cmp(&a.0)));
        indexed.into_iter().take(n).map(|(_, f)| f).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed() -> HomeMemoryStore {
        let mut store = HomeMemoryStore::new();
        store.add_fact("玄关灯", "睡前关", 100);
        store.add_fact("爸爸", "周末加班", 200);
        store.add_fact("玄关", "进门换鞋", 150);
        store.add_fact("玄关灯", "亮度调暗", 300);
        store.add_fact("妈妈", "喜欢喝温水", 120);
        store
    }

    #[test]
    fn query_by_entity_hits_multiple_and_partial() {
        let store = seed();
        // 精确匹配命中 "玄关灯" 两条。
        let hits = store.query_by_entity("玄关灯");
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|f| f.entity == "玄关灯"));
        // 子串匹配："玄关" 精确命中 1 条 + 子串命中 "玄关灯" 2 条 = 3。
        let partial = store.query_by_entity("玄关");
        assert_eq!(partial.len(), 3);
        assert_eq!(partial[0].entity, "玄关");
    }

    #[test]
    fn query_by_entity_miss_returns_empty() {
        let store = seed();
        assert!(store.query_by_entity("不存在的实体").is_empty());
        assert!(store.query_by_entity("").is_empty());
    }

    #[test]
    fn recent_is_descending_and_truncated() {
        let store = seed();
        let recent2 = store.recent(2);
        assert_eq!(recent2.len(), 2);
        // ts 最大者在前：300 -> "玄关灯/亮度调暗"，其次 200 -> "爸爸/周末加班"。
        assert_eq!(recent2[0].ts, 300);
        assert_eq!(recent2[0].fact, "亮度调暗");
        assert_eq!(recent2[1].ts, 200);
        // n 超过总数时返回全部。
        assert_eq!(store.recent(100).len(), 5);
        assert_eq!(store.recent(0).len(), 0);
    }
}
