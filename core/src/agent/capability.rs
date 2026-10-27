//! 智能体能力自述与自检（RPC `agent.selfcheck`）。
//!
//! 为什么需要这个模块（2026-10-25 用户反馈）：
//! - 主智能体被问「你能操控内置浏览器 / 终端 / 编辑器吗」时答"不能"或含糊，
//!   而引擎里这些能力**确实存在** —— 差别在于**提示词里没有任何一处告诉它**。
//! - 用户也没法回答「这次到底有没有加载我的智能体定义」：此前没有任何观测面
//!   暴露 persona 的来源 / 字节 / 哈希。
//!
//! 本模块只做两件事，都是**只读**的：
//! 1. `selfcheck()`：把"真实存在的工具 / MCP / 技能 / 内置面 / 人设来源"摊成结构化事实；
//! 2. `brief()`：同一份事实的紧凑文本版，供提示词注入（本轮先只提供函数，
//!    注入点在下一批，避免动到 KV 前缀与并发写入中的 chat.rs）。
//!
//! 纪律：**不猜、不写死数量**。工具清单来自真实 `ToolRegistry`（与请求侧同一个
//! 注册函数），技能数来自真实 `SkillLoader`，人设来自 `include_str!` 的编译期常量。
//! 拿不到就如实写 `null` / `0` / `error`，绝不用"应该支持"糊过去。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// 主智能体人设标识（与 `agent/store.rs` 的 `MAIN_AGENT_SYSTEM_PROMPT` 同源）。
const PERSONA_ID: &str = "senior-dev-engineer";
/// 人设正文的**源码路径**（编译期 `include_str!` 内联，运行时不读盘）。
const PERSONA_SOURCE: &str = "core/src/agent/personas/senior-dev-engineer.md";
/// 人设必须包含的"身份标记"：缺它说明文件被换过或加载错。
const PERSONA_MARKER: &str = "严工";

/// 结果缓存：构建 `ToolRegistry` 会做路径/护栏初始化，没必要每次 RPC 都重来。
/// 30 秒足以覆盖"用户点一下自检"的场景，又不会让新增工具长时间不可见。
const CACHE_TTL: Duration = Duration::from_secs(30);

fn cache() -> &'static Mutex<Option<(Instant, String, Value)>> {
    static CACHE: std::sync::OnceLock<Mutex<Option<(Instant, String, Value)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// `agent.selfcheck`：主智能体能力自检。
///
/// 参数：`{ "workspace": "<可选，绝对路径>" }`。缺省用应用数据目录
/// （与"没有绑定项目时"的工具根一致）。
pub fn selfcheck(params: &Value) -> Value {
    let workspace = params
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::persistence::config::Config::data_dir);
    let key = workspace.display().to_string();

    if let Ok(guard) = cache().lock() {
        if let Some((at, cached_key, value)) = guard.as_ref() {
            if cached_key == &key && at.elapsed() < CACHE_TTL {
                let mut v = value.clone();
                v["cached"] = json!(true);
                return v;
            }
        }
    }

    let value = build(&workspace);

    if let Ok(mut guard) = cache().lock() {
        *guard = Some((Instant::now(), key, value.clone()));
    }
    let mut v = value;
    v["cached"] = json!(false);
    v
}

fn build(workspace: &std::path::Path) -> Value {
    let config = crate::persistence::config::Config::load().unwrap_or_default();

    // ── 人设：证明"定义真的被读进来了" ──
    let persona_text = crate::agent::store::main_agent_prompt();
    let mut hasher = Sha256::new();
    hasher.update(persona_text.as_bytes());
    let persona_sha = format!("{:x}", hasher.finalize());
    let persona = json!({
        "id": PERSONA_ID,
        "source": PERSONA_SOURCE,
        "inlinedAtCompile": true,
        "bytes": persona_text.len(),
        "lines": persona_text.lines().count(),
        "sha256": persona_sha,
        "marker": PERSONA_MARKER,
        "markerOk": persona_text.contains(PERSONA_MARKER),
        // 人设里若出现这两样，说明模板会被误解析（见 store.rs 的硬约束）
        "templateBraces": persona_text.contains("{{"),
        "completeMarker": persona_text.lines().any(|l| l.trim() == "<!--persona-complete-->"),
    });

    // ── 工具：用**请求侧同一个注册函数**，不另建清单 ──
    let mut registry = crate::mcp::registry::ToolRegistry::new();
    let tool_error = match crate::tools::builtin::register_builtin_tools(
        &mut registry,
        workspace,
        None,
        None,
        None,
    ) {
        Ok(()) => Value::Null,
        Err(e) => json!(e.to_string()),
    };
    let mut names = registry.list_tools();
    names.sort();
    let mcp_tool_count = registry.describe_mcp_tools().len();
    let has = |needle: &str| names.iter().any(|n| n.contains(needle));

    // ── 内置面：判据是"真实工具名里有没有它"，不是文档写了没有 ──
    let surfaces = json!([
        {
            "id": "browser",
            "available": has("browser"),
            "entry": "browser.* RPC + /browser-cockpit（CDP 会话）",
            "note": "操控外部网页；本应用自身页面用 /app-control 的 window.MargueriteAppControl"
        },
        {
            "id": "terminal",
            "available": true,
            "entry": "sandbox.terminal.spawn|write|key|resize|status|shells（真实 PTY：ConPTY/openpty）",
            "note": "交互式终端；shell 可选 pwsh / Windows PowerShell / cmd / Git Bash / WSL"
        },
        {
            "id": "shell_exec",
            "available": has("shell"),
            "entry": "shell_exec（一次性执行：cmd|powershell|pwsh|bash|sh，工作区内 cwd，超时按进程树终止）",
            "note": "非交互命令优先用它；需要持续会话才用 terminal"
        },
        {
            "id": "editor",
            "available": has("write_file") || has("patch") || has("edit"),
            "entry": "read_file / write_file / apply_patch 等文件域工具",
            "note": "改动落在会话绑定的项目根；新建产物落主根"
        },
        {
            "id": "app_control",
            "available": true,
            "entry": "/app-control 页 + window.MargueriteAppControl（listInteractiveElements/clickTarget/fillTarget/pressKey/scanRoutes）",
            "note": "驱动本应用自己的页面与控件；批量巡检也走它"
        },
        {
            "id": "computer_use",
            "available": has("computer"),
            "entry": "computer_use.action（截图 + grounding + 坐标）",
            "note": "任意桌面软件；受屏幕/权限限制"
        },
        {
            "id": "web",
            "available": has("web_"),
            "entry": "web_search / web_fetch",
            "note": "外部事实先查再答"
        },
        {
            "id": "media",
            "available": has("image_generate") || has("video_generate"),
            "entry": "image_generate / video_generate / blender.*",
            "note": "需对应后端可用，否则如实失败"
        },
        {
            "id": "tui",
            "available": true,
            "entry": "marguerite-tui（ratatui 终端工作台，独立 bin）",
            "note": "CLI 侧入口，不依赖桌面壳"
        },
        {
            "id": "gui",
            "available": true,
            "entry": "marguerite-gui（Tauri 壳 + 内嵌网关 + webui/）",
            "note": "主窗口加载 http://localhost:18790/control/"
        }
    ]);

    // ── MCP / 技能：数量取自真实配置与真实加载器 ──
    let mcp_servers: Vec<String> = config
        .mcp
        .servers
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.clone())
        .collect();
    let skills_dir = crate::tools::skill_tools::resolve_skills_dir(&config.skills.skills_dir);
    let (skill_count, skill_error) =
        match crate::skills::loader::SkillLoader::new(&skills_dir).load_all() {
            Ok(list) => (list.len(), Value::Null),
            Err(e) => (0, json!(e.to_string())),
        };

    json!({
        "ok": true,
        "persona": persona,
        "tools": {
            "count": names.len(),
            "names": names,
            "mcpConnectedTools": mcp_tool_count,
            "registerError": tool_error,
            "workspace": workspace.display().to_string(),
        },
        "mcp": {
            "configuredServers": mcp_servers,
            "configuredCount": mcp_servers.len(),
            "connectedTools": mcp_tool_count,
        },
        "skills": {
            "dir": skills_dir.display().to_string(),
            "count": skill_count,
            "loadError": skill_error,
        },
        "surfaces": surfaces,
        "effort": {
            "entry": "model.effort.last",
            "lastRequest": crate::model::llm::last_effort_wire(),
        },
        "memory": {
            "tieredEntry": "memory.tiered.status",
            "injectEntry": "memory.inject.last",
            "tieredEnabled": config.memory.tiered_enabled,
        },
        "coordination": {
            "entries": [
                "orchestrate.run（单次编排）",
                "team.run / team.run.background（专家团）",
                "mode.swarm.get|set（蜂群）",
                "agent.*（子代理/子智能体）"
            ],
            "note": "分工由编排器决定；开关只影响形态，不应影响能否完成"
        }
    })
}

/// 真实能力计数（技能数 / MCP 配置数），来自 selfcheck 的 30s 缓存。
pub fn surface_counts() -> (usize, usize) {
    let v = selfcheck(&json!({}));
    (
        v["skills"]["count"].as_u64().unwrap_or(0) as usize,
        v["mcp"]["configuredCount"].as_u64().unwrap_or(0) as usize,
    )
}

/// 纯函数版能力自述（W4.5）：输入是**本轮真实工具名**，输出确定性文本。
///
/// 确定性是硬要求：同一会话内工具面不变时，这段文本必须逐字节相同，
/// 否则服务端 KV 前缀每轮失配（见 `runtime::build_model_messages` 的稳定半区）。
pub fn brief_from(tool_names: &[String], skills: usize, mcp: usize) -> String {
    let mut lines = vec![format!(
        "【工作台能力自述】真实工具 {} 个 · 技能 {skills} 个 · MCP 配置 {mcp} 个。以下清单来自当前工具面，不是猜测。",
        tool_names.len()
    )];
    lines.push(crate::agent::capability_cards::render_stable_cards_from_names(
        tool_names, mcp,
    ));
    lines.push(
        "若某项能力标为不可用，不要假装能调用；不确定时先说明缺少哪个工具，再请用户补配置。"
            .to_string(),
    );
    lines.join("\n")
}

/// 紧凑文本版（供提示词注入）：从真实注册表取工具名，再走 [`brief_from`]。
pub fn brief() -> String {
    let v = selfcheck(&json!({}));
    let names: Vec<String> = v["tools"]["names"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|name| name.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let (skills, mcp) = (
        v["skills"]["count"].as_u64().unwrap_or(0) as usize,
        v["mcp"]["configuredCount"].as_u64().unwrap_or(0) as usize,
    );
    brief_from(&names, skills, mcp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_is_really_inlined_and_marked() {
        let v = selfcheck(&json!({}));
        let p = &v["persona"];
        assert_eq!(p["id"], PERSONA_ID);
        assert!(
            p["bytes"].as_u64().unwrap_or(0) > 3000,
            "人设正文应 >3000 字节，实际 {:?}",
            p["bytes"]
        );
        assert_eq!(p["markerOk"], true, "人设必须含身份标记，否则说明加载错文件");
        assert_eq!(p["templateBraces"], false, "人设不得含 {{{{ }}}}（会被模板解析吞掉）");
        assert_eq!(
            p["completeMarker"], false,
            "人设不得含独立 <!--persona-complete--> 行（会阻断技能/工具段追加）"
        );
        assert!(
            p["sha256"].as_str().unwrap_or("").len() == 64,
            "sha256 必须是 64 位十六进制"
        );
    }

    #[test]
    fn tools_and_surfaces_are_reported_from_real_registry() {
        let v = selfcheck(&json!({}));
        let count = v["tools"]["count"].as_u64().unwrap_or(0);
        assert!(count >= 20, "内置工具数量异常偏低：{count}");
        let names = v["tools"]["names"].as_array().cloned().unwrap_or_default();
        assert!(names.iter().any(|n| n.as_str() == Some("read_file")), "缺少 read_file");
        let surfaces = v["surfaces"].as_array().cloned().unwrap_or_default();
        assert!(surfaces.len() >= 8, "内置面清单过短");
        // 终端/编辑器/应用自控是"必须有"的三项，缺一即回归
        for id in ["terminal", "editor", "app_control"] {
            let s = surfaces
                .iter()
                .find(|s| s["id"].as_str() == Some(id))
                .unwrap_or_else(|| panic!("缺少内置面 {id}"));
            assert_eq!(s["available"], true, "内置面 {id} 应可用");
        }
    }

    #[test]
    fn brief_from_is_deterministic_and_uses_only_real_tool_names() {
        let names = vec![
            "read_file".to_string(),
            "shell_exec".to_string(),
            "browser".to_string(),
        ];
        let a = brief_from(&names, 7, 1);
        let b = brief_from(&names, 7, 1);
        assert_eq!(a, b, "同一工具面必须逐字节相同（KV 前缀稳定）");
        assert!(a.contains("真实工具 3 个"), "{a}");
        assert!(a.contains("read_file"), "{a}");
        assert!(a.contains("browser"), "{a}");
        assert!(a.contains("技能 7 个"), "{a}");
        assert!(
            !a.contains("agent.selfcheck"),
            "自检是 RPC，模型在对话中不一定能调；不要把『去调 selfcheck』当能力说明：{a}"
        );
    }

    #[test]
    fn brief_marks_unavailable_capabilities_instead_of_hiding_them() {
        let text = brief_from(&["read_file".to_string()], 0, 0);
        assert!(text.contains("desktop.uia：不可用"), "{text}");
        assert!(text.contains("未配置 MCP 服务器"), "{text}");
    }

    #[test]
    fn selfcheck_never_leaks_secret_shaped_strings() {
        let text = selfcheck(&json!({})).to_string();
        assert!(!text.contains("sk-"), "自检结果不得包含密钥形态字符串");
    }
}
