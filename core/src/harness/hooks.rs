//! 生命周期钩子（HookRunner）
//!
//! 在工具调用与子代理执行的关键节点插入可编程策略：
//! `PreToolCall | PostToolCall | PreSubagent | PostSubagent | OnError`。
//!
//! 每个钩子形如 `Fn(&HookContext) -> Result<HookDecision>`，可以：
//! - [`HookDecision::Continue`]：放行，交给下一个钩子；
//! - [`HookDecision::Abort`]：**熔断**，同阶段后续钩子不再执行，动作被阻止；
//! - [`HookDecision::Modify`]：改写上下文（如重写参数）后继续。
//!
//! 安全默认：[`HookRunner::register_workspace_guard`] 注册的工作区守卫复用
//! [`crate::tools::fs_path::safe_resolve`]，任何逃逸工作区的路径参数一律
//! fail-closed 阻断，绝不自行实现路径归一化逻辑。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};

use crate::tools::fs_path::safe_resolve;

/// 钩子阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPhase {
    /// 工具调用前（可阻断）
    PreToolCall,
    /// 工具调用后（观测/审计）
    PostToolCall,
    /// 子代理启动前（可阻断）
    PreSubagent,
    /// 子代理结束后（观测/审计）
    PostSubagent,
    /// 出错时（观测/告警）
    OnError,
}

impl HookPhase {
    /// 稳定字符串标识
    pub fn as_str(&self) -> &'static str {
        match self {
            HookPhase::PreToolCall => "pre_tool_call",
            HookPhase::PostToolCall => "post_tool_call",
            HookPhase::PreSubagent => "pre_subagent",
            HookPhase::PostSubagent => "post_subagent",
            HookPhase::OnError => "on_error",
        }
    }
}

/// 钩子上下文
///
/// 钩子只读访问；需要改写时通过 [`HookDecision::Modify`] 返回新上下文。
#[derive(Debug, Clone)]
pub struct HookContext {
    /// 当前阶段
    pub phase: HookPhase,
    /// 工作区根（路径守卫的基准）
    pub workspace: PathBuf,
    /// 工具名（子代理阶段为角色名或 "subagent"）
    pub tool_name: String,
    /// 调用参数（扁平字符串映射，对齐 `tool_cycle` 的参数形态）
    pub args: HashMap<String, String>,
    /// 关联子代理 ID
    pub subagent_id: Option<String>,
    /// 错误信息（`OnError` 阶段填充）
    pub error: Option<String>,
    /// 钩子留痕（供审计链路串联）
    pub notes: Vec<String>,
}

impl HookContext {
    /// 创建上下文
    pub fn new(phase: HookPhase, workspace: impl Into<PathBuf>) -> Self {
        Self {
            phase,
            workspace: workspace.into(),
            tool_name: String::new(),
            args: HashMap::new(),
            subagent_id: None,
            error: None,
            notes: Vec::new(),
        }
    }

    /// 设置工具名
    pub fn with_tool(mut self, name: &str) -> Self {
        self.tool_name = name.to_string();
        self
    }

    /// 追加一个参数
    pub fn with_arg(mut self, key: &str, value: &str) -> Self {
        self.args.insert(key.to_string(), value.to_string());
        self
    }

    /// 关联子代理 ID
    pub fn with_subagent(mut self, id: &str) -> Self {
        self.subagent_id = Some(id.to_string());
        self
    }

    /// 设置错误信息
    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }

    /// 取路径型参数
    ///
    /// 依次尝试 `path` / `file_path` / `target`，与本仓内建工具的参数命名一致。
    pub fn path_arg(&self) -> Option<&str> {
        ["path", "file_path", "target"]
            .iter()
            .find_map(|k| self.args.get(*k).map(String::as_str))
    }
}

/// 钩子决策
#[derive(Debug, Clone)]
pub enum HookDecision {
    /// 放行
    Continue,
    /// 阻断，附原因
    Abort(String),
    /// 改写上下文后继续
    Modify(HookContext),
}

/// 钩子闭包类型
pub type HookFn = Arc<dyn Fn(&HookContext) -> Result<HookDecision> + Send + Sync>;

/// 阶段执行结果
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// 全部钩子放行，返回（可能被改写过的）最终上下文
    Proceed(HookContext),
    /// 被某个钩子阻断
    Aborted {
        /// 阻断者名称
        by: String,
        /// 阻断原因
        reason: String,
    },
}

impl HookOutcome {
    /// 是否被阻断
    pub fn is_aborted(&self) -> bool {
        matches!(self, HookOutcome::Aborted { .. })
    }
}

/// 钩子运行器（进程内注册表）
///
/// 内部用 `Mutex` 提供内部可变性，因此可以 `Arc<HookRunner>` 共享后仍能注册钩子。
/// 执行阶段先克隆钩子列表再释放锁，避免钩子内再次注册导致死锁。
#[derive(Default)]
pub struct HookRunner {
    hooks: Mutex<HashMap<HookPhase, Vec<(String, HookFn)>>>,
}

impl HookRunner {
    /// 创建空运行器
    pub fn new() -> Self {
        Self {
            hooks: Mutex::new(HashMap::new()),
        }
    }

    /// 注册钩子（已包装为 [`HookFn`]）
    pub fn register(&self, phase: HookPhase, name: &str, hook: HookFn) {
        let mut map = self.hooks.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(phase).or_default().push((name.to_string(), hook));
    }

    /// 注册钩子（直接传闭包）
    pub fn register_fn<F>(&self, phase: HookPhase, name: &str, hook: F)
    where
        F: Fn(&HookContext) -> Result<HookDecision> + Send + Sync + 'static,
    {
        self.register(phase, name, Arc::new(hook));
    }

    /// 某阶段已注册的钩子名（注册顺序即执行顺序）
    pub fn hook_names(&self, phase: HookPhase) -> Vec<String> {
        let map = self.hooks.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&phase)
            .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default()
    }

    /// 某阶段钩子数量
    pub fn count(&self, phase: HookPhase) -> usize {
        let map = self.hooks.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&phase).map(Vec::len).unwrap_or(0)
    }

    /// 执行某阶段全部钩子
    ///
    /// 任一钩子返回 [`HookDecision::Abort`] 即短路，后续钩子不再执行。
    /// 钩子自身返回 `Err` 视为**执行故障**（区别于策略性阻断），直接上抛。
    pub fn run_phase(&self, ctx: HookContext) -> Result<HookOutcome> {
        let phase = ctx.phase;
        // 先克隆再释放锁：钩子体内可安全地再次调用注册接口
        let hooks: Vec<(String, HookFn)> = {
            let map = self.hooks.lock().unwrap_or_else(|e| e.into_inner());
            map.get(&phase).cloned().unwrap_or_default()
        };

        let mut current = ctx;
        for (name, hook) in hooks {
            match hook(&current)? {
                HookDecision::Continue => {}
                HookDecision::Abort(reason) => {
                    return Ok(HookOutcome::Aborted { by: name, reason });
                }
                HookDecision::Modify(mut next) => {
                    // 阶段由运行器掌控，钩子不得篡改
                    next.phase = phase;
                    next.notes.push(format!("modified_by:{name}"));
                    current = next;
                }
            }
        }
        Ok(HookOutcome::Proceed(current))
    }

    /// 带钩子包裹地执行一次工具调用
    ///
    /// 流程：`PreToolCall` → （放行则）执行 → `PostToolCall`；
    /// 任一环节失败或被阻断则走 `OnError`。被阻断时 **`exec` 绝不会被调用**，
    /// 这是「钩子能真正拦住工具」的关键保证（见单测 `test_abort_hook_blocks_tool_call`）。
    pub fn run_tool_call<T, F>(&self, ctx: HookContext, exec: F) -> Result<T>
    where
        F: FnOnce(&HookContext) -> Result<T>,
    {
        let mut pre = ctx.clone();
        pre.phase = HookPhase::PreToolCall;

        let approved = match self.run_phase(pre)? {
            HookOutcome::Proceed(c) => c,
            HookOutcome::Aborted { by, reason } => {
                let msg = format!("钩子 '{}' 阻止工具调用 '{}'：{}", by, ctx.tool_name, reason);
                let mut err_ctx = ctx;
                err_ctx.phase = HookPhase::OnError;
                err_ctx.error = Some(msg.clone());
                // OnError 为观测阶段，其自身故障不掩盖原始阻断原因
                let _ = self.run_phase(err_ctx);
                return Err(anyhow!(msg));
            }
        };

        let result = exec(&approved);
        match &result {
            Ok(_) => {
                let mut post = approved;
                post.phase = HookPhase::PostToolCall;
                let _ = self.run_phase(post);
            }
            Err(e) => {
                let mut err_ctx = approved;
                err_ctx.phase = HookPhase::OnError;
                err_ctx.error = Some(e.to_string());
                let _ = self.run_phase(err_ctx);
            }
        }
        result
    }

    /// 注册工作区守卫（`PreToolCall`）
    ///
    /// 复用 [`safe_resolve`] 判定路径是否逃逸工作区：逃逸即阻断。
    /// 采取 fail-closed 策略——**任何**带路径参数的工具都要过守卫（不只写工具），
    /// 因为越界读取同样是数据泄露面。无路径参数的工具直接放行。
    pub fn register_workspace_guard(&self, workspace: PathBuf) {
        self.register_fn(
            HookPhase::PreToolCall,
            "workspace_guard",
            move |ctx| match ctx.path_arg() {
                None => Ok(HookDecision::Continue),
                Some(path) => match safe_resolve(&workspace, path) {
                    Ok(_) => Ok(HookDecision::Continue),
                    Err(e) => Ok(HookDecision::Abort(format!(
                        "路径 '{path}' 逃逸工作区，已按 fail-closed 阻断：{e}"
                    ))),
                },
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn workspace() -> PathBuf {
        let dir = std::env::temp_dir().join("marguerite_harness_hooks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap_or(dir)
    }

    #[test]
    fn test_hooks_run_in_registration_order() {
        let runner = HookRunner::new();
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));

        let o1 = order.clone();
        runner.register_fn(HookPhase::PreToolCall, "first", move |_| {
            o1.lock().unwrap().push("first");
            Ok(HookDecision::Continue)
        });
        let o2 = order.clone();
        runner.register_fn(HookPhase::PreToolCall, "second", move |_| {
            o2.lock().unwrap().push("second");
            Ok(HookDecision::Continue)
        });

        let ctx = HookContext::new(HookPhase::PreToolCall, workspace());
        assert!(!runner.run_phase(ctx).unwrap().is_aborted());
        assert_eq!(*order.lock().unwrap(), vec!["first", "second"]);
        assert_eq!(
            runner.hook_names(HookPhase::PreToolCall),
            vec!["first", "second"]
        );
    }

    #[test]
    fn test_abort_hook_blocks_tool_call() {
        let runner = HookRunner::new();
        let executed = Arc::new(AtomicUsize::new(0));

        runner.register_fn(HookPhase::PreToolCall, "denier", |_| {
            Ok(HookDecision::Abort("策略禁止".to_string()))
        });

        let ctx = HookContext::new(HookPhase::PreToolCall, workspace()).with_tool("fs_write");
        let counter = executed.clone();
        let result: Result<&str> = runner.run_tool_call(ctx, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok("executed")
        });

        assert!(result.is_err(), "被阻断的工具调用必须返回错误");
        assert_eq!(executed.load(Ordering::SeqCst), 0, "阻断后工具体绝不可执行");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("denier"), "错误信息应含阻断者：{msg}");
        assert!(msg.contains("策略禁止"), "错误信息应含阻断原因：{msg}");
    }

    #[test]
    fn test_abort_short_circuits_later_hooks() {
        let runner = HookRunner::new();
        let later = Arc::new(AtomicUsize::new(0));

        runner.register_fn(HookPhase::PreToolCall, "denier", |_| {
            Ok(HookDecision::Abort("stop".to_string()))
        });
        let c = later.clone();
        runner.register_fn(HookPhase::PreToolCall, "never", move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(HookDecision::Continue)
        });

        let ctx = HookContext::new(HookPhase::PreToolCall, workspace());
        let outcome = runner.run_phase(ctx).unwrap();
        assert!(outcome.is_aborted());
        assert_eq!(later.load(Ordering::SeqCst), 0, "阻断后同阶段钩子应短路");
    }

    #[test]
    fn test_tool_call_executes_when_allowed() {
        let runner = HookRunner::new();
        runner.register_fn(HookPhase::PreToolCall, "allow", |_| {
            Ok(HookDecision::Continue)
        });

        let post_seen = Arc::new(AtomicUsize::new(0));
        let p = post_seen.clone();
        runner.register_fn(HookPhase::PostToolCall, "audit", move |_| {
            p.fetch_add(1, Ordering::SeqCst);
            Ok(HookDecision::Continue)
        });

        let ctx = HookContext::new(HookPhase::PreToolCall, workspace()).with_tool("fs_read");
        let out: String = runner
            .run_tool_call(ctx, |c| Ok(format!("ran:{}", c.tool_name)))
            .unwrap();
        assert_eq!(out, "ran:fs_read");
        assert_eq!(
            post_seen.load(Ordering::SeqCst),
            1,
            "成功后应触发 PostToolCall"
        );
    }

    #[test]
    fn test_modify_decision_rewrites_context() {
        let runner = HookRunner::new();
        runner.register_fn(HookPhase::PreToolCall, "rewriter", |ctx| {
            let mut next = ctx.clone();
            next.args.insert("path".to_string(), "safe.txt".to_string());
            Ok(HookDecision::Modify(next))
        });

        let ctx = HookContext::new(HookPhase::PreToolCall, workspace())
            .with_tool("fs_write")
            .with_arg("path", "danger.txt");
        let seen: String = runner
            .run_tool_call(ctx, |c| Ok(c.path_arg().unwrap_or_default().to_string()))
            .unwrap();
        assert_eq!(seen, "safe.txt", "Modify 应把改写后的上下文交给工具体");
    }

    #[test]
    fn test_workspace_guard_blocks_escape() {
        let ws = workspace();
        let runner = HookRunner::new();
        runner.register_workspace_guard(ws.clone());

        let executed = Arc::new(AtomicUsize::new(0));
        let counter = executed.clone();
        let ctx = HookContext::new(HookPhase::PreToolCall, ws)
            .with_tool("fs_write")
            .with_arg("path", "../../etc/passwd");
        let result: Result<()> = runner.run_tool_call(ctx, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        assert!(result.is_err(), "越界写入必须被守卫阻断");
        assert_eq!(executed.load(Ordering::SeqCst), 0, "越界写入不可落地执行");
        assert!(result.unwrap_err().to_string().contains("workspace_guard"));
    }

    #[test]
    fn test_workspace_guard_allows_internal_path() {
        let ws = workspace();
        let runner = HookRunner::new();
        runner.register_workspace_guard(ws.clone());

        let ctx = HookContext::new(HookPhase::PreToolCall, ws)
            .with_tool("fs_write")
            .with_arg("path", "src/main.rs");
        let ok: bool = runner.run_tool_call(ctx, |_| Ok(true)).unwrap();
        assert!(ok, "工作区内路径应放行");
    }

    #[test]
    fn test_error_hook_receives_failure() {
        let runner = HookRunner::new();
        let seen = Arc::new(Mutex::new(String::new()));
        let s = seen.clone();
        runner.register_fn(HookPhase::OnError, "collector", move |ctx| {
            *s.lock().unwrap() = ctx.error.clone().unwrap_or_default();
            Ok(HookDecision::Continue)
        });

        let ctx = HookContext::new(HookPhase::PreToolCall, workspace()).with_tool("boom");
        let result: Result<()> = runner.run_tool_call(ctx, |_| Err(anyhow!("工具内部炸了")));
        assert!(result.is_err());
        assert!(
            seen.lock().unwrap().contains("工具内部炸了"),
            "OnError 钩子应收到失败原因"
        );
    }

    #[test]
    fn test_hook_execution_failure_propagates() {
        let runner = HookRunner::new();
        runner.register_fn(HookPhase::PreSubagent, "broken", |_| {
            Err(anyhow!("钩子自身故障"))
        });
        let ctx = HookContext::new(HookPhase::PreSubagent, workspace());
        let err = runner.run_phase(ctx).unwrap_err().to_string();
        assert!(err.contains("钩子自身故障"));
    }

    #[test]
    fn test_phases_are_independent() {
        let runner = HookRunner::new();
        runner.register_fn(HookPhase::PreSubagent, "a", |_| Ok(HookDecision::Continue));
        assert_eq!(runner.count(HookPhase::PreSubagent), 1);
        assert_eq!(runner.count(HookPhase::PreToolCall), 0);
        assert_eq!(runner.count(HookPhase::PostSubagent), 0);
    }
}
