//! 技能注册表（v2：按 rank 分级注入，策划 §三.8）。
//!
//! 对齐 dsh-agent-plugins-market 的「project rank 250 / user rank 450」
//! 分级注入模式：
//! - 每个技能带一个整数 `rank`（越小越先注入，默认 300，可覆盖）
//! - 注册时先做依赖拓扑排序 + rank 分组升序：
//!   * rank 低 → 先注入（基础 / 工程纪律技能默认低 rank 常驻）
//!   * rank 相同 → 按名字稳定排序
//! - `find_by_intent` 返回的结果按「rank 升序 → rating 降序 → uses 降序」
//!   排序，调用端拿到的 JIT 候选优先级正确。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

/// 技能注入优先级约定（策划 §三.8 参考 dsh-agent-plugins-market）。
/// 数值越小注入越靠前：
/// - SYSTEM: Marguerite 内置宪法/红线（不能被覆盖）
/// - PROJECT: 项目级常驻（工程纪律包等，对应 dsh market 的 project rank 250）
/// - USER: 用户级安装（对应 dsh market 的 user rank 450）
/// - COMMUNITY: 社区市场默认（最高，500+）
pub mod ranks {
    pub const SYSTEM: u32 = 100;
    pub const PROJECT: u32 = 250;
    pub const DEFAULT: u32 = 300;
    pub const USER: u32 = 450;
    pub const COMMUNITY: u32 = 600;
}

/// 技能来源
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum SkillSource {
    /// 内置技能
    #[default]
    Builtin,
    /// 本地文件
    Local { path: std::path::PathBuf },
    /// 远程安装
    Remote { url: String },
}

/// 技能定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 技能名称
    pub name: String,
    /// 技能描述
    pub description: String,
    /// 版本号
    pub version: String,
    /// 作者
    pub author: Option<String>,
    /// 触发的意图关键词
    #[serde(default)]
    pub intent: Vec<String>,
    /// 需要的工具列表
    #[serde(default)]
    pub tools: Vec<String>,
    /// Prompt 模板
    pub prompt_template: String,
    /// 依赖的其他技能（注册时必须先于本技能注入）
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// 技能来源
    #[serde(skip)]
    pub source: SkillSource,
    /// 效果评分 (0.0 - 5.0)
    #[serde(default)]
    pub rating: f64,
    /// 使用次数
    #[serde(default)]
    pub uses: u32,
    /// 注入优先级（越小越先注入；默认 ranks::DEFAULT=300）
    #[serde(default = "default_rank")]
    pub rank: u32,
}

fn default_rank() -> u32 {
    ranks::DEFAULT
}

/// 技能注册表
///
/// 管理所有技能的注册、查找和索引。
pub struct SkillRegistry {
    /// 技能名称 → 技能映射
    skills: HashMap<String, Arc<Skill>>,
    /// 意图关键词 → 技能名称列表
    intent_index: HashMap<String, Vec<String>>,
}

impl SkillRegistry {
    /// 创建新的空技能注册表
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            intent_index: HashMap::new(),
        }
    }

    /// 注册单个技能（不校验依赖：用于已有代码、保留向后兼容；推荐使用
    /// [`SkillRegistry::register_batch_ranked`] 以获得依赖 + rank 排序注入）。
    pub fn register(&mut self, skill: Skill) -> Result<()> {
        self.register_internal(skill)
    }

    /// 批量注册：按「rank 升序 + 依赖拓扑」排序后依次注入。
    ///
    /// 返回实际注入顺序（技能名称列表），便于调用端验证/日志。
    /// 失败：依赖缺失、环、重复注册都会返回 Err，并保证注册表回滚到未注入状态。
    pub fn register_batch_ranked(&mut self, batch: Vec<Skill>) -> Result<Vec<String>> {
        // 1) 预检查：名字在注册表或批内重复
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (i, s) in batch.iter().enumerate() {
            if self.skills.contains_key(&s.name) {
                bail!("技能 '{}' 已注册", s.name);
            }
            if let Some(&j) = seen.get(&s.name) {
                bail!("批内技能名重复：'{}' 位于索引 {} 与 {}", s.name, j, i);
            }
            seen.insert(s.name.clone(), i);
        }

        // 2) 依赖 + rank 拓扑：
        //    - 在「同一 rank」内，按 dependency 构造 DAG，Kahn 拓扑
        //    - 跨 rank：rank 低的整个分组先于 rank 高的分组（保证 SYSTEM < PROJECT < ...）
        //    - 跨 rank 的 dependency：若 A(rank=100) 依赖 B(rank=600) → 视为缺依赖
        //      （因为 B 应该是更早注入的 SYSTEM 级；这里仅要求 batch 内 + 注册表中满足）
        let mut groups: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, s) in batch.iter().enumerate() {
            groups.entry(s.rank).or_default().push(i);
        }
        let mut ranks_order: Vec<u32> = groups.keys().copied().collect();
        ranks_order.sort_unstable();

        // 注册表已有名字集合（用于 dependency 已满足判断）
        let existing: std::collections::HashSet<String> = self.skills.keys().cloned().collect();

        let mut inject_order: Vec<String> = Vec::with_capacity(batch.len());
        let mut committed: Vec<String> = Vec::new();

        // 因为失败要回滚，我们把每一步实际注册进 self，但在失败时 unregister
        // 已 committed 的，保持注册表干净。
        let rollback = |this: &mut SkillRegistry, committed: &[String]| {
            for name in committed.iter().rev() {
                let _ = this.unregister(name);
            }
        };

        for r in ranks_order {
            let group_idx = groups.get(&r).cloned().unwrap_or_default();
            // 按名字稳定排序作为「并列」兜底
            let mut group = group_idx;
            group.sort_by(|&a, &b| batch[a].name.cmp(&batch[b].name));

            // 构建子图：节点 = group 内索引；边 u→v 表示 batch[v] 依赖 batch[u]
            // （dep 指向注册表已有或本组中的其他节点，且 dep 必须 rank ≤ 当前 rank；
            //  若 dep 存在于更高 rank 的 batch 节点，则是拓扑错序直接报错）
            let n = group.len();
            let idx_by_name: HashMap<String, usize> = group
                .iter()
                .enumerate()
                .map(|(local, &gi)| (batch[gi].name.clone(), local))
                .collect();
            let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
            let mut in_deg: Vec<usize> = vec![0; n];
            for (local, &gi) in group.iter().enumerate() {
                for dep in &batch[gi].dependencies {
                    if existing.contains(dep) {
                        continue; // 已注册，不必入拓扑
                    }
                    if let Some(&dep_local) = idx_by_name.get(dep) {
                        // dep 在同 group 中：增加边 dep_local → local
                        edges[dep_local].push(local);
                        in_deg[local] += 1;
                    } else {
                        // 既不在注册表，也不在同 group；看看是否在 batch 的更高 rank 中
                        // （这种情况应视为错序依赖，直接报错）
                        if let Some(&dep_gi) = seen.get(dep) {
                            let dep_rank = batch[dep_gi].rank;
                            if dep_rank > r {
                                rollback(self, &committed);
                                bail!(
                                    "技能 '{}'（rank={r}）依赖 '{}'（rank={dep_rank}），\
                                     依赖 rank 更高 → 不满足 rank 注入顺序",
                                    batch[gi].name,
                                    dep
                                );
                            }
                            // dep_rank < r：应该在更低 rank 的分组里处理过；但
                            // existing 中没有、且 seen 有说明还未 inject →
                            // 我们把它视为「应该在前面 group」，而 loop 是按升序，
                            // 所以前面 group 必须已经 committed 过它；committed 里
                            // 没有就报错。
                            if !committed.contains(dep) {
                                rollback(self, &committed);
                                bail!(
                                    "技能 '{}' 依赖 '{}'，但更低 rank 的注入分组仍未注入它",
                                    batch[gi].name,
                                    dep
                                );
                            }
                        } else {
                            rollback(self, &committed);
                            bail!("技能 '{}' 依赖未找到：'{}'", batch[gi].name, dep);
                        }
                    }
                }
            }

            // Kahn 拓扑（同 group）
            let mut q: VecDeque<usize> = (0..n).filter(|&i| in_deg[i] == 0).collect();
            let mut processed = 0usize;
            while let Some(local) = q.pop_front() {
                let gi = group[local];
                let skill = batch[gi].clone();
                let name = skill.name.clone();
                match self.register_internal(skill) {
                    Ok(()) => {
                        committed.push(name.clone());
                        inject_order.push(name);
                    }
                    Err(e) => {
                        rollback(self, &committed);
                        return Err(e);
                    }
                }
                processed += 1;
                for &nxt in &edges[local] {
                    in_deg[nxt] -= 1;
                    if in_deg[nxt] == 0 {
                        q.push_back(nxt);
                    }
                }
            }
            if processed != n {
                // 拓扑有环：找 remaining 的节点名
                let mut cycle_names: Vec<String> = (0..n)
                    .filter(|&i| in_deg[i] > 0)
                    .map(|i| batch[group[i]].name.clone())
                    .collect();
                cycle_names.sort();
                rollback(self, &committed);
                bail!(
                    "技能分组（rank={r}）存在依赖环，涉及：{}",
                    cycle_names.join(", ")
                );
            }
        }

        Ok(inject_order)
    }

    fn register_internal(&mut self, skill: Skill) -> Result<()> {
        let name = skill.name.clone();
        if self.skills.contains_key(&name) {
            bail!("技能 '{}' 已注册", name);
        }
        for keyword in &skill.intent {
            self.intent_index
                .entry(keyword.to_lowercase())
                .or_default()
                .push(name.clone());
        }
        self.skills.insert(name, Arc::new(skill));
        Ok(())
    }

    /// 注销技能
    pub fn unregister(&mut self, name: &str) -> Result<()> {
        let skill = self
            .skills
            .remove(name)
            .ok_or_else(|| anyhow!("技能 '{}' 未注册", name))?;
        for keyword in &skill.intent {
            if let Some(names) = self.intent_index.get_mut(&keyword.to_lowercase()) {
                names.retain(|n| n != name);
                if names.is_empty() {
                    self.intent_index.remove(&keyword.to_lowercase());
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<Skill>> {
        self.skills.get(name).cloned()
    }

    /// 列出所有技能：按「rank 升序 → 名字升序」稳定返回（便于 UI 展示 + 调试）。
    pub fn list(&self) -> Vec<Arc<Skill>> {
        let mut out: Vec<Arc<Skill>> = self.skills.values().cloned().collect();
        out.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.name.cmp(&b.name)));
        out
    }

    /// 按意图关键词查找技能：返回结果按「rank 升序 → rating 降序 → uses 降序」排序。
    pub fn find_by_intent(&self, intent: &str) -> Vec<Arc<Skill>> {
        let intent_lower = intent.to_lowercase();
        let mut out: Vec<Arc<Skill>> = self
            .skills
            .values()
            .filter(|skill| {
                skill
                    .intent
                    .iter()
                    .any(|kw| kw.to_lowercase() == intent_lower)
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then_with(|| {
                    b.rating
                        .partial_cmp(&a.rating)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.uses.cmp(&a.uses))
        });
        out
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn update_rating(&mut self, name: &str, rating: f64) {
        if let Some(skill) = self.skills.get_mut(name) {
            let skill = Arc::make_mut(skill);
            skill.rating = rating.clamp(0.0, 5.0);
        }
    }
    pub fn increment_uses(&mut self, name: &str) {
        if let Some(skill) = self.skills.get_mut(name) {
            let skill = Arc::make_mut(skill);
            skill.uses = skill.uses.saturating_add(1);
        }
    }
    pub fn has(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }

    /// 注册表中所有 rank 的分组情况（调试/断言用）。
    pub fn rank_histogram(&self) -> Vec<(u32, usize)> {
        let mut h: HashMap<u32, usize> = HashMap::new();
        for s in self.skills.values() {
            *h.entry(s.rank).or_default() += 1;
        }
        let mut out: Vec<(u32, usize)> = h.into_iter().collect();
        out.sort_by_key(|(r, _)| *r);
        out
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_skill(name: &str, intents: Vec<&str>) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("测试技能: {}", name),
            version: "1.0.0".to_string(),
            author: None,
            intent: intents.into_iter().map(String::from).collect(),
            tools: vec![],
            prompt_template: "模板: {{text}}".to_string(),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
            rank: 300,
        }
    }

    fn skill_with(name: &str, rank: u32, deps: Vec<&str>) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("ranked skill {}", name),
            version: "0.1.0".to_string(),
            author: None,
            intent: vec![name.to_string()],
            tools: vec![],
            prompt_template: String::new(),
            dependencies: deps.into_iter().map(String::from).collect(),
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
            rank,
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("explain", vec!["explain", "解释"]);
        registry.register(skill).unwrap();

        let found = registry.get("explain");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "explain");
    }

    #[test]
    fn test_register_duplicate() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("test", vec!["test"]);
        registry.register(skill).unwrap();

        let duplicate = create_test_skill("test", vec!["test"]);
        assert!(registry.register(duplicate).is_err());
    }

    #[test]
    fn test_unregister() {
        let mut registry = SkillRegistry::new();
        let skill = create_test_skill("refactor", vec!["refactor", "重构"]);
        registry.register(skill).unwrap();
        assert!(registry.has("refactor"));

        registry.unregister("refactor").unwrap();
        assert!(!registry.has("refactor"));
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mut registry = SkillRegistry::new();
        assert!(registry.unregister("nonexistent").is_err());
    }

    #[test]
    fn test_list() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("skill_a", vec!["a"]))
            .unwrap();
        registry
            .register(create_test_skill("skill_b", vec!["b"]))
            .unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_find_by_intent() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("explain", vec!["explain", "解释"]))
            .unwrap();
        registry
            .register(create_test_skill("refactor", vec!["refactor", "重构"]))
            .unwrap();

        let skills = registry.find_by_intent("explain");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "explain");

        let skills = registry.find_by_intent("解释");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "explain");

        // 不存在的意图
        let skills = registry.find_by_intent("nonexistent");
        assert!(skills.is_empty());
    }

    #[test]
    fn test_len_and_is_empty() {
        let mut registry = SkillRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        registry
            .register(create_test_skill("a", vec!["a"]))
            .unwrap();
        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_update_rating() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("test", vec!["test"]))
            .unwrap();

        registry.update_rating("test", 4.5);
        let skill = registry.get("test").unwrap();
        assert!((skill.rating - 4.5).abs() < 1e-10);

        // 超出范围应被 clamp
        registry.update_rating("test", 6.0);
        let skill = registry.get("test").unwrap();
        assert!((skill.rating - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_increment_uses() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("test", vec!["test"]))
            .unwrap();

        registry.increment_uses("test");
        let skill = registry.get("test").unwrap();
        assert_eq!(skill.uses, 1);

        registry.increment_uses("test");
        let skill = registry.get("test").unwrap();
        assert_eq!(skill.uses, 2);
    }

    #[test]
    fn test_has() {
        let mut registry = SkillRegistry::new();
        registry
            .register(create_test_skill("exists", vec!["yes"]))
            .unwrap();

        assert!(registry.has("exists"));
        assert!(!registry.has("nonexistent"));
    }

    #[test]
    fn batch_ranked_injects_by_rank_then_topology() {
        let mut reg = SkillRegistry::new();
        // rank 分组：USER(450) 组 [C 依赖 B，B 依赖 A]；PROJECT(250) 组 [P2, P1]
        // 期望：
        // 1) 先 PROJECT 组，按名字排序 → [P1, P2]
        // 2) 再 USER 组，按依赖拓扑 → [A, B, C]
        let order = reg
            .register_batch_ranked(vec![
                skill_with("C", ranks::USER, vec!["B"]),
                skill_with("B", ranks::USER, vec!["A"]),
                skill_with("A", ranks::USER, vec![]),
                skill_with("P2", ranks::PROJECT, vec![]),
                skill_with("P1", ranks::PROJECT, vec![]),
            ])
            .unwrap();
        assert_eq!(order, vec!["P1", "P2", "A", "B", "C"]);
        // 验证 list 也是按 rank 升序 + 名字升序（因为拓扑不影响 list，只影响注入时机）
        let names: Vec<String> = reg.list().iter().map(|s| s.name.clone()).collect();
        // rank PROJECT: P1, P2；rank DEFAULT 本组没有；rank USER: A,B,C →
        assert_eq!(names, vec!["P1", "P2", "A", "B", "C"]);
    }

    #[test]
    fn batch_ranked_detects_cycle() {
        let mut reg = SkillRegistry::new();
        let err = reg
            .register_batch_ranked(vec![
                skill_with("x", ranks::USER, vec!["y"]),
                skill_with("y", ranks::USER, vec!["x"]),
            ])
            .expect_err("应检测到依赖环");
        assert!(err.to_string().contains("依赖环"));
        // 注册表应为空（回滚语义）
        assert!(reg.is_empty());
    }

    #[test]
    fn batch_ranked_rejects_missing_dep_and_rollsback() {
        let mut reg = SkillRegistry::new();
        // 先注入 P1 再注入一个依赖 ghost 的 USER 级 → 应失败，且 P1 不应残留
        // （因为整个 batch 是一次调用；若 P1 也在同一 batch 中，它会先被
        // committed，缺 dep 时会回滚）
        let err = reg
            .register_batch_ranked(vec![
                skill_with("P1", ranks::PROJECT, vec![]),
                skill_with("U1", ranks::USER, vec!["ghost"]),
            ])
            .expect_err("应检测到缺依赖");
        assert!(err.to_string().contains("依赖未找到"));
        assert!(reg.is_empty());
    }

    #[test]
    fn batch_ranked_rejects_reverse_rank_dep() {
        let mut reg = SkillRegistry::new();
        // SYSTEM 级依赖 COMMUNITY 级 → 错序
        let err = reg
            .register_batch_ranked(vec![
                skill_with("S1", ranks::SYSTEM, vec!["M1"]),
                skill_with("M1", ranks::COMMUNITY, vec![]),
            ])
            .expect_err("应检测到 rank 顺序冲突");
        assert!(err.to_string().contains("依赖 rank 更高"));
        assert!(reg.is_empty());
    }

    #[test]
    fn find_by_intent_sorted_by_rank_then_rating_then_uses() {
        let mut reg = SkillRegistry::new();
        // 两个共享同一 intent="x" 的技能：
        // - low 高优先级(rank=100) 但 rating=1 uses=0
        // - hi  低优先级(rank=500) 但 rating=5 uses=100
        // find_by_intent 仍应以 rank 为先
        let mut low = skill_with("low", 100, vec![]);
        low.intent = vec!["x".into()];
        low.rating = 1.0;
        low.uses = 0;
        let mut hi = skill_with("hi", 500, vec![]);
        hi.intent = vec!["x".into()];
        hi.rating = 5.0;
        hi.uses = 100;
        // 同 rank 组：验证 rating/uses 生效
        let mut a = skill_with("mid_a", 300, vec![]);
        a.intent = vec!["x".into()];
        a.rating = 4.0;
        a.uses = 1;
        let mut b = skill_with("mid_b", 300, vec![]);
        b.intent = vec!["x".into()];
        b.rating = 4.0;
        b.uses = 5; // uses 更高 → 同 rating 下 b 排在 a 前面
        let mut c = skill_with("mid_c", 300, vec![]);
        c.intent = vec!["x".into()];
        c.rating = 4.5; // rating 更高 → 排最前

        reg.register_batch_ranked(vec![hi.clone(), low.clone(), a, b, c])
            .unwrap();
        let names: Vec<String> = reg
            .find_by_intent("x")
            .into_iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, vec!["low", "mid_c", "mid_b", "mid_a", "hi"]);
    }

    #[test]
    fn rank_histogram_tracks_groups() {
        let mut reg = SkillRegistry::new();
        reg.register(skill_with("s", ranks::SYSTEM, vec![]))
            .unwrap();
        reg.register(skill_with("p", ranks::PROJECT, vec![]))
            .unwrap();
        reg.register(skill_with("u", ranks::USER, vec![])).unwrap();
        assert_eq!(reg.rank_histogram(), vec![(100, 1), (250, 1), (450, 1)]);
    }

    #[test]
    fn default_rank_for_legacy_deserialization() {
        // 反序列化时不带 rank 的旧 JSON 也应落到 DEFAULT=300（serde default）
        let json = r#"{
            "name":"old_skill","description":"old","version":"0.0.1",
            "prompt_template":".","intent":[],"tools":[],"dependencies":[],
            "rating":0.0,"uses":0
        }"#;
        let s: Skill = serde_json::from_str(json).unwrap();
        assert_eq!(s.rank, ranks::DEFAULT);
    }
}
