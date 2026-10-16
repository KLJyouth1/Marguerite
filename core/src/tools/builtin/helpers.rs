//! Auto-extracted from builtin.rs — shared infrastructure for all builtin tool submodules.
//!
//! Contains: ToolFn type, LocalTool struct, boxed/make_local_tool factories,
//! ok/err result helpers, current_session, PREFLIGHT security gate,
//! walk_files, diff_lines, json_query utilities.

use super::*;

/// 工具执行闭包类型（future 生命周期绑定输入引用）
pub(crate) type ToolFn = Arc<
    dyn for<'a> Fn(
            &'a HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + 'a>>
        + Send
        + Sync,
>;

/// 通用本地工具：以闭包承载实现，消除每个工具的结构体样板
pub(crate) struct LocalTool {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) schema: serde_json::Value,
    pub(crate) run: ToolFn,
}

#[async_trait]
impl ToolExecutor for LocalTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        self.description
    }
    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        (self.run)(args).await
    }
}

pub(crate) fn boxed(
    name: &'static str,
    description: &'static str,
    schema: serde_json::Value,
    run: ToolFn,
) -> Box<dyn ToolExecutor> {
    Box::new(LocalTool {
        name,
        description,
        schema,
        run,
    })
}

/// 公开版 `boxed`：供其他工具模块（如 `tools::git`）构造 `LocalTool` 并注册，
/// 避免每个模块重复结构体样板。
pub fn make_local_tool(
    name: &'static str,
    description: &'static str,
    schema: serde_json::Value,
    run: ToolFn,
) -> Box<dyn ToolExecutor> {
    boxed(name, description, schema, run)
}

pub(crate) fn ok(data: String) -> Result<ToolResult> {
    Ok(ToolResult::Success {
        data,
        metadata: HashMap::new(),
    })
}

pub(crate) fn err(message: String, code: &str) -> Result<ToolResult> {
    Ok(ToolResult::Error {
        message,
        code: code.to_string(),
    })
}

/// 解析当前 task-local 会话（无 `SESSION` 上下文时返回空串；
/// `record_artifact` 会回退到默认会话并告警，满足 R1「尽量带上会话归属」）。
pub(crate) fn current_session() -> String {
    crate::usage::SESSION
        .try_with(|s| s.to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// L2/L3 预执行安全闸口（分层护栏的运行时接入点）
// ---------------------------------------------------------------------------

/// 进程级共享的 L2+L3 闸口。
///
/// `Preflight::new()` 会编译约 20 条 L2 正则，构造开销不小，因此全进程复用一份；
/// 扫描本身是无状态纯函数，可安全并发调用。
pub(crate) static PREFLIGHT: std::sync::LazyLock<Preflight> = std::sync::LazyLock::new(Preflight::new);

/// 把预执行裁决转成工具错误结果；放行时返回 `None`。
///
/// 用法：`if let Some(denied) = gate(&PREFLIGHT.check_write(..)) { return denied; }`
///
/// 之所以统一走这里，是为了保证**所有**写/删/执行路径的拒绝语义、错误码、
/// 提示文案完全一致——避免各工具各写一套导致"有的拦有的漏"。
pub(crate) fn gate(decision: &PreflightDecision) -> Option<Result<ToolResult>> {
    match decision {
        PreflightDecision::Allow => None,
        PreflightDecision::Deny {
            layer,
            code,
            reason,
        } => Some(err(
            format!("[{layer}] 操作被分层安全护栏阻断: {reason}"),
            code,
        )),
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 递归收集工作区内文件（跳过巨型无关目录）
pub(crate) fn walk_files(root: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(
                    name,
                    ".git" | "target" | "node_modules" | ".versions" | "build"
                ) {
                    continue;
                }
                walk_files(&p, out);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
}

/// 计算两段文本的行级 diff（LCS），返回类 unified 文本
pub(crate) fn diff_lines(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            if a[i] == b[j] {
                dp[i][j] = dp[i + 1][j + 1] + 1;
            } else {
                dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }
    let mut out = String::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push_str(&format!("  {}\n", a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push_str(&format!("- {}\n", a[i]));
            i += 1;
        } else {
            out.push_str(&format!("+ {}\n", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push_str(&format!("- {}\n", a[i]));
        i += 1;
    }
    while j < m {
        out.push_str(&format!("+ {}\n", b[j]));
        j += 1;
    }
    out
}

/// 按点分路径（支持数组下标）从 JSON 提取值
pub(crate) fn json_query(json_str: &str, path: &str) -> Result<String> {
    let v: serde_json::Value =
        serde_json::from_str(json_str).with_context(|| "invalid JSON input")?;
    let mut cur = &v;
    for seg in path.split('.') {
        if seg.is_empty() {
            continue;
        }
        if let Ok(idx) = seg.parse::<usize>() {
            cur = cur
                .get(idx)
                .ok_or_else(|| anyhow!("array index out of range: {}", seg))?;
        } else {
            cur = cur
                .get(seg)
                .ok_or_else(|| anyhow!("key not found: {}", seg))?;
        }
    }
    match cur {
        serde_json::Value::String(s) => Ok(s.clone()),
        other => Ok(other.to_string()),
    }
}
