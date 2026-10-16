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
use tracing::warn;

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
pub(crate) use crate::tools::trash;
use crate::tools::register_computer_use_tools;
use crate::tools::versions::VersionManager;
use crate::vision::{image_metadata, VisionCapability};

// ---------------------------------------------------------------------------
// 子模块（按职责拆分，原 4738 行 → 各 <400 行子模块）
// ---------------------------------------------------------------------------

mod helpers;         // ToolFn, LocalTool, boxed, make_local_tool, ok, err, gate, PREFLIGHT, walk_files, diff_lines, json_query
mod fs;              // read/write/append/list/find/info/grep/codebase_search/edit_file
mod file_ops;        // copy/move/delete/make_dir
mod editor;          // str_replace_editor/apply_patch/glob/todo_write
mod code_intel;     // code_intel (incremental index + symbol/text search)
mod memory_tools;    // user_memory_dir + memory_save/search/list
mod web;             // shell_exec/web_fetch/http_request/html_parse
mod browser;         // browser sessions + browser_tool
mod browser_actions; // run_browser_action dispatcher
mod system;          // system_control + sendkeys + run_web_search
mod web_search;      // run_web_search_direct + web_search_tool/web_research_tool
mod query;           // distill_query（整句 → 检索词，P1-1）
mod search_quality;  // 检索质量门禁（引擎首页/空壳识别，P0-3）
mod utils;           // json_query_tool/diff_tool/count_lines/read_env
mod patch;           // apply_patch_tool/glob_file_search
mod win_auto;        // window_manage/clipboard/process_manage/office_automation
#[cfg(test)]
mod tests;           // helpers (setup/put) + registration + FS/editor/utils tests
#[cfg(test)]
mod tests_security;  // L2/L3 guard regression tests + shell/web
#[cfg(test)]
mod tests_trash;     // T1 删除转回收站工具层测试
#[cfg(test)]
mod tests_web;       // web search/parse/browser tests
#[cfg(test)]
mod tests_system;    // system control + memory tests
#[cfg(test)]
mod tests_live;      // all #[ignore] live integration tests

// Re-export sub-module items for register_builtin_tools
pub use helpers::make_local_tool;       // pub: tools::git/skill_tools/code/baota use it
pub(crate) use helpers::*;              // boxed, ok, err, gate, PREFLIGHT, ToolFn, LocalTool, etc.
pub(crate) use fs::*;
pub(crate) use file_ops::*;
pub(crate) use editor::*;
pub(crate) use code_intel::*;
pub(crate) use memory_tools::*;
pub(crate) use web::*;
pub(crate) use browser::*;
pub(crate) use browser_actions::*;
pub(crate) use system::*;
pub(crate) use web_search::*;
pub(crate) use query::*;
pub(crate) use search_quality::*;
pub(crate) use utils::*;
pub(crate) use patch::*;
pub(crate) use win_auto::*;

// Cross-module type re-exports
pub(crate) use web_search::SearchResult;
pub(crate) use web::BrowserEntry;
pub(crate) use web::{looks_like_html, extract_readable};
pub(crate) use system::run_web_search;

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
        str_replace_editor_tool(ws.clone(), versions.clone()),
        apply_patch_tool(ws.clone(), versions.clone()),
        glob_file_search_tool(ws.clone()),
        todo_write_tool(ws.clone()),
        copy_file_tool(ws.clone(), versions.clone()),
        move_file_tool(ws.clone()),
        delete_file_tool(ws.clone()),
        delete_file_permanent_tool(ws.clone()),
        restore_from_trash_tool(ws.clone()),
        empty_trash_tool(ws.clone()),
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
    // 抽取自 baota_bridge（xiaoyuangou），注册为 baota_* 工具（下划线，合法 function name）；
    // 安全默认 dry_run/confirm_exec。
    if let Err(e) = crate::tools::baota::register_baota_tools(registry) {
        // 注册失败不阻断内置工具加载（环境变量缺失时 BT 工具仍注册，调用时返回配置引导错误）
        eprintln!("[baota] 注册宝塔工具失败: {e}");
    }

    Ok(())
}
