//! 任务树的**运行时状态**（计划 P3-3）。
//!
//! 分解与拓扑排序已经存在（[`crate::orchestration::dag`] 的 `TaskNode` / `topo_layers`），
//! 这里只补「跑起来之后的状态机」：就绪集、派发、完成、失败传播、并行批。
//! **不重复造 DAG**，也不碰模型路由。
//!
//! 一条硬规则：**单个节点失败只能阻塞它的下游**，与它无关的分支必须继续跑
//! （历史教训：编排层一处失败就 abort 全树，用户看到的是「整单任务消失」）。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// 节点运行态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    /// 依赖未满足，还不能跑
    Pending,
    /// 依赖全部完成，可以派发
    Ready,
    /// 已派发（有 owner）
    Running,
    /// 完成
    Done,
    /// 失败（自身出错）
    Failed,
    /// 被上游失败连累，未执行也不会执行
    Blocked,
}

/// 单个节点的运行时记录
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRuntime {
    pub id: String,
    pub deps: Vec<String>,
    pub state: NodeState,
    pub owner: Option<String>,
    pub checkpoint: Option<String>,
}

/// 状态机错误（都带可诊断上下文，不返回笼统的失败）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// 节点不存在
    Unknown(String),
    /// 状态迁移不合法（例如 Pending 直接 Done）
    IllegalTransition { id: String, from: NodeState, to: NodeState },
    /// 依赖里出现未知节点
    UnknownDep { node: String, dep: String },
    /// 成环
    Cycle,
    /// 重复 id
    Duplicate(String),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::Unknown(id) => write!(f, "未知节点: {id}"),
            StateError::IllegalTransition { id, from, to } => {
                write!(f, "非法状态迁移: {id} {from:?} → {to:?}")
            }
            StateError::UnknownDep { node, dep } => {
                write!(f, "节点 {node} 依赖了不存在的 {dep}")
            }
            StateError::Cycle => write!(f, "任务树成环"),
            StateError::Duplicate(id) => write!(f, "重复节点 id: {id}"),
        }
    }
}

/// 任务树运行时
#[derive(Debug, Clone, Default)]
pub struct TaskState {
    nodes: BTreeMap<String, NodeRuntime>,
}

impl TaskState {
    /// 由「id + 依赖」序列构建：校验重复 id、未知依赖、成环，并把无依赖者置 Ready。
    pub fn from_deps<I>(items: I) -> Result<Self, StateError>
    where
        I: IntoIterator<Item = (String, Vec<String>)>,
    {
        let mut nodes = BTreeMap::new();
        for (id, deps) in items {
            if nodes.contains_key(&id) {
                return Err(StateError::Duplicate(id));
            }
            nodes.insert(
                id.clone(),
                NodeRuntime {
                    id: id.clone(),
                    deps,
                    state: NodeState::Pending,
                    owner: None,
                    checkpoint: None,
                },
            );
        }

        // 未知依赖
        let known: BTreeSet<String> = nodes.keys().cloned().collect();
        for node in nodes.values() {
            for dep in &node.deps {
                if !known.contains(dep) {
                    return Err(StateError::UnknownDep {
                        node: node.id.clone(),
                        dep: dep.clone(),
                    });
                }
            }
        }

        // 成环（DFS 三色）
        let mut colors: BTreeMap<String, u8> = BTreeMap::new();
        for id in nodes.keys() {
            if dfs_cycle(&nodes, id, &mut colors) {
                return Err(StateError::Cycle);
            }
        }

        let mut state = Self { nodes };
        state.recompute_ready();
        Ok(state)
    }

    /// 适配既有的分解结果（[`crate::orchestration::dag::TaskNode`]）。
    pub fn from_nodes(nodes: &[crate::orchestration::dag::TaskNode]) -> Result<Self, StateError> {
        Self::from_deps(nodes.iter().map(|n| (n.id.clone(), n.deps.clone())))
    }

    pub fn get(&self, id: &str) -> Option<&NodeRuntime> {
        self.nodes.get(id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// 当前就绪集（依赖全 Done 的 Pending 节点），按 id 排序保证确定性。
    pub fn ready_set(&self) -> Vec<String> {
        self.nodes
            .values()
            .filter(|n| n.state == NodeState::Ready)
            .map(|n| n.id.clone())
            .collect()
    }

    /// 派发：只有 Ready 能变 Running；同一节点重复派发给不同 owner 视为非法。
    pub fn dispatch(&mut self, id: &str, owner: &str) -> Result<(), StateError> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| StateError::Unknown(id.to_string()))?;
        if node.state != NodeState::Ready {
            return Err(StateError::IllegalTransition {
                id: id.to_string(),
                from: node.state,
                to: NodeState::Running,
            });
        }
        node.state = NodeState::Running;
        node.owner = Some(owner.to_string());
        Ok(())
    }

    /// 完成：Running → Done，然后重算下游就绪。
    pub fn complete(&mut self, id: &str) -> Result<(), StateError> {
        self.transition(id, NodeState::Running, NodeState::Done)?;
        self.recompute_ready();
        Ok(())
    }

    /// 失败：Running → Failed，**只**把下游标 Blocked，无关分支不受影响。
    pub fn fail(&mut self, id: &str) -> Result<(), StateError> {
        self.transition(id, NodeState::Running, NodeState::Failed)?;
        self.block_downstream(id);
        Ok(())
    }

    /// 当前可并行批：把还没跑完的节点按拓扑层分组，只保留「同层内无依赖关系」的批次。
    pub fn parallel_batches(&self) -> Vec<Vec<String>> {
        let mut layers: Vec<Vec<String>> = Vec::new();
        let mut placed: BTreeSet<String> = BTreeSet::new();
        let mut frontier: Vec<String> = self
            .nodes
            .values()
            .filter(|n| n.deps.is_empty())
            .map(|n| n.id.clone())
            .collect();
        frontier.sort();

        while !frontier.is_empty() {
            let layer: Vec<String> = frontier
                .iter()
                .filter(|id| !placed.contains(*id))
                .cloned()
                .collect();
            for id in &layer {
                placed.insert(id.clone());
            }
            layers.push(layer);

            let mut next: Vec<String> = Vec::new();
            for node in self.nodes.values() {
                if placed.contains(&node.id) {
                    continue;
                }
                if node.deps.iter().all(|d| placed.contains(d)) {
                    next.push(node.id.clone());
                }
            }
            next.sort();
            frontier = next;
        }
        layers
    }

    fn transition(&mut self, id: &str, from: NodeState, to: NodeState) -> Result<(), StateError> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| StateError::Unknown(id.to_string()))?;
        if node.state != from {
            return Err(StateError::IllegalTransition {
                id: id.to_string(),
                from: node.state,
                to,
            });
        }
        node.state = to;
        Ok(())
    }

    /// 依赖满足的 Pending → Ready
    fn recompute_ready(&mut self) {
        let done: BTreeSet<String> = self
            .nodes
            .values()
            .filter(|n| n.state == NodeState::Done)
            .map(|n| n.id.clone())
            .collect();
        for node in self.nodes.values_mut() {
            if node.state == NodeState::Pending && node.deps.iter().all(|d| done.contains(d)) {
                node.state = NodeState::Ready;
            }
        }
    }

    /// 从一个失败节点出发，把「直接或间接依赖它」且尚未跑完的节点标 Blocked
    fn block_downstream(&mut self, failed: &str) {
        // 先把下游闭包算出来（失败节点本身不在里面）
        let mut blocked: BTreeSet<String> = BTreeSet::new();
        let mut frontier: VecDeque<String> = VecDeque::new();
        frontier.push_back(failed.to_string());
        while let Some(cur) = frontier.pop_front() {
            let dependents: Vec<String> = self
                .nodes
                .values()
                .filter(|n| n.deps.contains(&cur))
                .map(|n| n.id.clone())
                .collect();
            for dep in dependents {
                if blocked.insert(dep.clone()) {
                    frontier.push_back(dep);
                }
            }
        }

        for node in self.nodes.values_mut() {
            if blocked.contains(&node.id)
                && matches!(
                    node.state,
                    NodeState::Pending | NodeState::Ready | NodeState::Running
                )
            {
                node.state = NodeState::Blocked;
            }
        }
    }
}

fn dfs_cycle(
    nodes: &BTreeMap<String, NodeRuntime>,
    id: &str,
    colors: &mut BTreeMap<String, u8>,
) -> bool {
    // 0/缺省 = 未访问，1 = 在栈上，2 = 已完成
    match colors.get(id).copied().unwrap_or(0) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    colors.insert(id.to_string(), 1);
    if let Some(node) = nodes.get(id) {
        for dep in &node.deps {
            // 边方向是 node → dep；从任一节点出发都能覆盖，环即自指闭环
            if dfs_cycle(nodes, dep, colors) {
                return true;
            }
        }
    }
    colors.insert(id.to_string(), 2);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(items: &[(&str, &[&str])]) -> Result<TaskState, StateError> {
        TaskState::from_deps(
            items
                .iter()
                .map(|(id, deps)| {
                    (
                        id.to_string(),
                        deps.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let err = build(&[("a", &[]), ("a", &[])]).unwrap_err();
        assert_eq!(err, StateError::Duplicate("a".to_string()));
    }

    #[test]
    fn cycle_is_rejected() {
        let err = build(&[("a", &["b"]), ("b", &["a"])]).unwrap_err();
        assert_eq!(err, StateError::Cycle);
    }

    #[test]
    fn ready_set_is_recomputed_after_completion() {
        let mut s = build(&[("a", &[]), ("b", &["a"]), ("c", &["a", "b"])]).unwrap();
        assert_eq!(s.ready_set(), vec!["a".to_string()]);

        s.dispatch("a", "worker-1").unwrap();
        s.complete("a").unwrap();
        assert_eq!(s.ready_set(), vec!["b".to_string()]);

        s.dispatch("b", "worker-2").unwrap();
        s.complete("b").unwrap();
        assert_eq!(s.ready_set(), vec!["c".to_string()]);
        assert_eq!(s.get("a").unwrap().state, NodeState::Done);
    }

    #[test]
    fn failure_blocks_only_downstream() {
        // a → b → d，a 失败；c 是独立分支，必须继续可跑
        let mut s = build(&[("a", &[]), ("b", &["a"]), ("c", &[]), ("d", &["b"])]).unwrap();
        s.dispatch("a", "w1").unwrap();
        s.fail("a").unwrap();

        assert_eq!(s.get("a").unwrap().state, NodeState::Failed);
        assert_eq!(s.get("b").unwrap().state, NodeState::Blocked);
        assert_eq!(s.get("d").unwrap().state, NodeState::Blocked);
        assert_eq!(
            s.get("c").unwrap().state,
            NodeState::Ready,
            "无关分支不能被连坐"
        );
        assert_eq!(s.ready_set(), vec!["c".to_string()]);
    }

    #[test]
    fn parallel_batches_never_put_dependency_pairs_together() {
        let s = build(&[("a", &[]), ("b", &[]), ("c", &["a"]), ("d", &["b", "c"])]).unwrap();
        let batches = s.parallel_batches();
        assert_eq!(batches.len(), 3, "四节点三层: {batches:?}");

        // 同一批内任意两点之间不能有依赖关系
        for batch in &batches {
            for x in batch {
                for y in batch {
                    if x == y {
                        continue;
                    }
                    assert!(
                        !s.get(x).unwrap().deps.contains(y)
                            && !s.get(y).unwrap().deps.contains(x),
                        "{batch:?} 内出现了依赖对 {x}/{y}"
                    );
                }
            }
        }
        assert!(batches[0].contains(&"a".to_string()));
        assert!(batches[0].contains(&"b".to_string()));
    }
}
