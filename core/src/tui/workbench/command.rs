//! slash 命令注册与分发（B7，17+ 条）
//!
//! 复用 core 的模式/审批/事件接口；命令在此侧负责 TUI 注册 + 解析 + 分发，
//! 产生 [`UiEffect`] 交 `WorkbenchApp::apply_effects` 执行。

use std::collections::HashMap;

use crate::mode::ModeKind;
use crate::model::selector::profile_from_flags;
use crate::tui::workbench::slot::SlotId;
use crate::tui::workbench::state::{
    ActivityKind, ChatMessage, ChatRole, CollabOp, DeployOp, ModelOp, RailTab, UiEffect,
    WorkbenchState,
};

/// 内置命令枚举（20 条，覆盖验收 B7 的 17+）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinCommand {
    /// 切换模式
    Mode,
    /// 切到 plan 模式
    Plan,
    /// 模型管理
    Model,
    /// 列出模型
    Models,
    /// 压缩上下文
    Compact,
    /// 新会话
    New,
    /// 会话列表
    Sessions,
    /// 恢复会话
    Restore,
    /// 导出
    Export,
    /// 技能
    Skills,
    /// MCP
    Mcp,
    /// 上下文
    Ctx,
    /// 记忆
    Memory,
    /// 文件树
    Tree,
    /// 自动化
    Automation,
    /// 状态
    Status,
    /// 诊断
    Doctor,
    /// 帮助
    Help,
    /// 一键部署面板（Phase I4，FR-16）
    Deploy,
    /// 协作事件流（Phase I4，FR-15）
    Collab,
    /// 退出
    Exit,
    /// 退出（别名）
    Quit,
}

/// 全部内置命令
pub fn all_commands() -> Vec<BuiltinCommand> {
    use BuiltinCommand::*;
    vec![
        Mode, Plan, Model, Models, Compact, New, Sessions, Restore, Export, Skills, Mcp, Ctx,
        Memory, Tree, Automation, Status, Doctor, Help, Deploy, Collab, Exit, Quit,
    ]
}

impl BuiltinCommand {
    /// 命令名（含前导 /）
    pub fn name(&self) -> &'static str {
        use BuiltinCommand::*;
        match self {
            Mode => "/mode",
            Plan => "/plan",
            Model => "/model",
            Models => "/models",
            Compact => "/compact",
            New => "/new",
            Sessions => "/sessions",
            Restore => "/restore",
            Export => "/export",
            Skills => "/skills",
            Mcp => "/mcp",
            Ctx => "/ctx",
            Memory => "/memory",
            Tree => "/tree",
            Automation => "/automation",
            Status => "/status",
            Doctor => "/doctor",
            Help => "/help",
            Deploy => "/deploy",
            Collab => "/collab",
            Exit => "/exit",
            Quit => "/quit",
        }
    }

    /// 命令说明
    pub fn desc(&self) -> &'static str {
        use BuiltinCommand::*;
        match self {
            Mode => "切换模式 plan/agent/bloom",
            Plan => "切到 plan 模式",
            Model => "模型管理：use/copilot/add/remove/list",
            Models => "列出可用模型",
            Compact => "压缩上下文",
            New => "新建会话",
            Sessions => "会话列表",
            Restore => "恢复历史会话",
            Export => "导出当前会话",
            Skills => "技能管理",
            Mcp => "MCP 工具管理",
            Ctx => "查看上下文快照",
            Memory => "查看记忆引用",
            Tree => "打开文件树",
            Automation => "自动化任务",
            Status => "查看状态",
            Doctor => "系统诊断",
            Help => "显示命令帮助",
            Deploy => "部署面板：run <ssh://host|baota://别名> [--service 名称] [--no-ssl]",
            Collab => "协作事件流：join <名> / leave / send <消息>",
            Exit => "退出工作台",
            Quit => "退出工作台",
        }
    }
}

/// 解析 "/cmd args" → 产生副作用
pub fn dispatch(state: &mut WorkbenchState, raw: &str) -> Vec<UiEffect> {
    let raw = raw.trim();
    if !raw.starts_with('/') {
        return vec![UiEffect::Render];
    }
    let body = &raw[1..];
    let (cmd, args) = match body.split_once(' ') {
        Some((c, a)) => (c.trim(), a.trim()),
        None => (body.trim(), ""),
    };

    match cmd {
        "mode" => match ModeKind::from_name(args) {
            Some(m) => vec![UiEffect::SwitchMode(m)],
            None => vec![UiEffect::Render],
        },
        "plan" => vec![UiEffect::SwitchMode(ModeKind::Plan)],
        "model" | "models" => dispatch_model(state, args),
        "compact" => vec![UiEffect::Render],
        "new" => vec![UiEffect::NewSession],
        "sessions" => {
            state.active_activity = ActivityKind::Sessions;
            state.active_sidebar = SlotId::LeftSidebarSessions;
            vec![UiEffect::SwitchActivity(ActivityKind::Sessions)]
        }
        "restore" => vec![UiEffect::RestoreSession],
        "export" => vec![UiEffect::Render],
        "skills" => vec![UiEffect::Render],
        "mcp" => vec![UiEffect::Render],
        "ctx" => vec![UiEffect::SwitchRailTab(RailTab::Context)],
        "memory" => vec![UiEffect::SwitchRailTab(RailTab::Memory)],
        "tree" => {
            state.active_activity = ActivityKind::Files;
            state.active_sidebar = SlotId::LeftSidebarFileTree;
            vec![UiEffect::SwitchActivity(ActivityKind::Files)]
        }
        "automation" => vec![UiEffect::Render],
        "status" => vec![UiEffect::Render],
        "doctor" => vec![UiEffect::Render],
        "help" => {
            state.help_open = true;
            vec![UiEffect::ShowHelp]
        }
        "deploy" => dispatch_deploy(state, args),
        "collab" => dispatch_collab(state, args),
        "exit" | "quit" => vec![UiEffect::Quit],
        _ => vec![UiEffect::Render],
    }
}

/// 解析 `/deploy` 子命令 → 产生副作用（Phase I4，FR-16）
///
/// - `（无参 | list）`       打开部署面板
/// - `run <target> [--service X] [--no-ssl]`  对目标执行一次 dry-run 演练
///   `target` 为 `ssh://[user@]host[:port][/dir]` 或 `baota://<别名>`
pub fn dispatch_deploy(state: &mut WorkbenchState, args: &str) -> Vec<UiEffect> {
    let args = args.trim();
    if args.is_empty() || args == "list" {
        return vec![UiEffect::SwitchRailTab(RailTab::Deploy)];
    }
    let rest = match args.strip_prefix("run") {
        Some(r) => r.trim(),
        None => return vec![UiEffect::SwitchRailTab(RailTab::Deploy)],
    };
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let mut target = String::new();
    let mut service: Option<String> = None;
    let mut no_ssl = false;
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            "--service" => {
                if i + 1 < toks.len() {
                    service = Some(toks[i + 1].to_string());
                }
                i += 2;
            }
            "--no-ssl" => {
                no_ssl = true;
                i += 1;
            }
            t if target.is_empty() => {
                target = t.to_string();
                i += 1;
            }
            _ => i += 1,
        }
    }
    if target.is_empty() {
        state.chat.messages.push(ChatMessage {
            role: ChatRole::System,
            content: "用法：/deploy run <ssh://host|baota://别名> [--service 名称] [--no-ssl]"
                .into(),
        });
        return vec![UiEffect::Render];
    }
    vec![
        UiEffect::SwitchRailTab(RailTab::Deploy),
        UiEffect::DeployAction(DeployOp::Run {
            target,
            service,
            no_ssl,
        }),
    ]
}

/// 解析 `/collab` 子命令 → 产生副作用（Phase I4，FR-15）
///
/// - `（无参）`       打开协作面板并回放事件流
/// - `join <名>`      以成员名加入会话
/// - `leave`          本地成员离开
/// - `send <消息>`    广播一条协作消息
pub fn dispatch_collab(_state: &mut WorkbenchState, args: &str) -> Vec<UiEffect> {
    let args = args.trim();
    if args.is_empty() {
        return vec![
            UiEffect::SwitchRailTab(RailTab::Collab),
            UiEffect::CollabAction(CollabOp::Replay),
        ];
    }
    let (sub, rest) = match args.split_once(' ') {
        Some((s, r)) => (s.trim(), r.trim()),
        None => (args, ""),
    };
    match sub {
        "join" => {
            if rest.is_empty() {
                return vec![UiEffect::SwitchRailTab(RailTab::Collab)];
            }
            vec![
                UiEffect::SwitchRailTab(RailTab::Collab),
                UiEffect::CollabAction(CollabOp::Join(rest.to_string())),
            ]
        }
        "leave" => vec![
            UiEffect::SwitchRailTab(RailTab::Collab),
            UiEffect::CollabAction(CollabOp::Leave),
        ],
        "send" => {
            if rest.is_empty() {
                return vec![UiEffect::SwitchRailTab(RailTab::Collab)];
            }
            vec![
                UiEffect::SwitchRailTab(RailTab::Collab),
                UiEffect::CollabAction(CollabOp::Send(rest.to_string())),
            ]
        }
        _ => vec![
            UiEffect::SwitchRailTab(RailTab::Collab),
            UiEffect::CollabAction(CollabOp::Replay),
        ],
    }
}

/// 解析 `/model` 子命令 → 产生副作用（H5 顶栏同构）
///
/// 子命令：
/// - `use <id>`            切换主模型
/// - `copilot <id|off>`    设置 / 关闭 Bloom 副模型
/// - `add --id <id> --base-url <url> --model <name> [--provider <p>] [--label <l>] [--api-key <k>] [--kind <custom|cloud|local>]`  新增自定义 API 接入点
/// - `remove <id>`         删除档案
/// - `list`（默认）        列出全部档案
pub fn dispatch_model(state: &mut WorkbenchState, args: &str) -> Vec<UiEffect> {
    let args = args.trim();
    let (sub, rest) = match args.split_once(' ') {
        Some((s, r)) => (s.trim(), r.trim()),
        None => (args, ""),
    };
    match sub {
        "" | "list" => vec![UiEffect::ModelAction(ModelOp::List)],
        "use" => {
            if rest.is_empty() {
                vec![UiEffect::Render]
            } else {
                vec![UiEffect::ModelAction(ModelOp::UseActive(rest.to_string()))]
            }
        }
        "copilot" => {
            if rest.is_empty() || rest == "off" {
                vec![UiEffect::ModelAction(ModelOp::UseCopilot(None))]
            } else {
                vec![UiEffect::ModelAction(ModelOp::UseCopilot(Some(
                    rest.to_string(),
                )))]
            }
        }
        "add" => {
            let flags = parse_flags(rest);
            match profile_from_flags(&flags) {
                Ok(p) => vec![UiEffect::ModelAction(ModelOp::Add(p))],
                Err(e) => {
                    state.chat.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!(
                            "添加模型失败：{e}\n用法：/model add --id <id> --base-url <url> --model <name> [--provider custom] [--label <展示名>] [--api-key <key>] [--kind custom|cloud|local]"
                        ),
                    });
                    vec![UiEffect::Render]
                }
            }
        }
        "remove" | "rm" => {
            if rest.is_empty() {
                vec![UiEffect::Render]
            } else {
                vec![UiEffect::ModelAction(ModelOp::Remove(rest.to_string()))]
            }
        }
        _ => vec![UiEffect::Render],
    }
}

/// 解析 `--key value` 形式的 flag（值取下一空白分隔 token）
fn parse_flags(s: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let toks: Vec<&str> = s.split_whitespace().collect();
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i];
        if let Some(key) = tok.strip_prefix("--") {
            let val = if i + 1 < toks.len() { toks[i + 1] } else { "" };
            map.insert(key.to_string(), val.to_string());
            i += 2;
        } else {
            i += 1;
        }
    }
    map
}

/// 帮助文本（列出所有命令；供 ShowHelp 浮层渲染）
pub fn help_text() -> String {
    let mut s = String::from("可用命令：\n");
    for c in all_commands() {
        s.push_str(&format!("  {}  — {}\n", c.name(), c.desc()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::ModeKind;
    use crate::tui::workbench::state::WorkbenchState;

    #[test]
    fn test_command_count_at_least_17() {
        assert!(
            all_commands().len() >= 17,
            "命令数应 >= 17，实际 {}",
            all_commands().len()
        );
    }

    #[test]
    fn test_dispatch_mode_yolo() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/mode yolo");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::SwitchMode(ModeKind::Bloom)]
        ));
    }

    #[test]
    fn test_dispatch_mode_invalid_ignored() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/mode bogus");
        assert!(eff.iter().any(|e| matches!(e, UiEffect::Render)));
    }

    #[test]
    fn test_dispatch_plan() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/plan");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::SwitchMode(ModeKind::Plan)]
        ));
    }

    #[test]
    fn test_dispatch_help_sets_overlay() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/help");
        assert!(s.help_open);
        assert!(eff.iter().any(|e| matches!(e, UiEffect::ShowHelp)));
    }

    #[test]
    fn test_dispatch_exit_quits() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        assert!(dispatch(&mut s, "/exit")
            .iter()
            .any(|e| matches!(e, UiEffect::Quit)));
        assert!(dispatch(&mut s, "/quit")
            .iter()
            .any(|e| matches!(e, UiEffect::Quit)));
    }

    #[test]
    fn test_dispatch_tree_switches_activity() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        dispatch(&mut s, "/tree");
        assert_eq!(s.active_activity, ActivityKind::Files);
        assert_eq!(s.active_sidebar, SlotId::LeftSidebarFileTree);
    }

    #[test]
    fn test_dispatch_unknown_no_panic() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/zzz");
        assert!(eff.iter().any(|e| matches!(e, UiEffect::Render)));
    }

    #[test]
    fn test_help_text_lists_all() {
        let h = help_text();
        for c in all_commands() {
            assert!(h.contains(c.name()), "帮助应含 {}", c.name());
        }
    }

    #[test]
    fn test_dispatch_model_list() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/model");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ModelAction(ModelOp::List)]
        ));
        let eff = dispatch(&mut s, "/models list");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ModelAction(ModelOp::List)]
        ));
    }

    #[test]
    fn test_dispatch_model_use() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/model use deepseek-chat");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ModelAction(ModelOp::UseActive(id))] if id == "deepseek-chat"
        ));
    }

    #[test]
    fn test_dispatch_model_copilot_off_and_id() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/model copilot off");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ModelAction(ModelOp::UseCopilot(None))]
        ));

        let eff = dispatch(&mut s, "/model copilot openai-compatible");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ModelAction(ModelOp::UseCopilot(Some(id)))] if id == "openai-compatible"
        ));
    }

    #[test]
    fn test_dispatch_model_add_parses_flags() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(
            &mut s,
            "/model add --id my-openai --base-url https://api.openai.com/v1 --model gpt-4o --provider openai --kind cloud",
        );
        match eff.as_slice() {
            [UiEffect::ModelAction(ModelOp::Add(p))] => {
                assert_eq!(p.id, "my-openai");
                assert_eq!(p.base_url, "https://api.openai.com/v1");
                assert_eq!(p.kind, crate::model::registry::ProfileKind::Cloud);
            }
            _ => panic!("应为 ModelAction(Add)"),
        }
    }

    #[test]
    fn test_dispatch_model_remove() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/model remove deepseek-chat");
        assert!(matches!(
            eff.as_slice(),
            [UiEffect::ModelAction(ModelOp::Remove(id))] if id == "deepseek-chat"
        ));
    }

    #[test]
    fn test_dispatch_model_unknown_subcommand_no_panic() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/model bogus");
        assert!(eff.iter().any(|e| matches!(e, UiEffect::Render)));
    }

    #[test]
    fn test_dispatch_deploy_opens_panel() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/deploy");
        assert!(eff
            .iter()
            .any(|e| matches!(e, UiEffect::SwitchRailTab(RailTab::Deploy))));
    }

    #[test]
    fn test_dispatch_deploy_run_parses_target_and_flags() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(
            &mut s,
            "/deploy run ssh://deploy@host/opt/mgt --service bot --no-ssl",
        );
        match eff.last() {
            Some(UiEffect::DeployAction(DeployOp::Run {
                target,
                service,
                no_ssl,
            })) => {
                assert_eq!(target, "ssh://deploy@host/opt/mgt");
                assert_eq!(service.as_deref(), Some("bot"));
                assert!(no_ssl);
            }
            other => panic!("应为 DeployAction(Run)，实际 {:?}", other),
        }
    }

    #[test]
    fn test_dispatch_deploy_run_without_target_prompts() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/deploy run");
        // 无目标：给出用法提示（Render），不应产生 DeployAction
        assert!(!eff.iter().any(|e| matches!(e, UiEffect::DeployAction(_))));
        assert!(s
            .chat
            .messages
            .iter()
            .any(|m| m.content.contains("用法：/deploy run")));
    }

    #[test]
    fn test_dispatch_collab_join() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/collab join alice");
        match eff.last() {
            Some(UiEffect::CollabAction(CollabOp::Join(name))) => assert_eq!(name, "alice"),
            other => panic!("应为 CollabAction(Join)，实际 {:?}", other),
        }
    }

    #[test]
    fn test_dispatch_collab_send_and_leave() {
        let mut s = WorkbenchState::new("s1".into(), "m".into());
        let eff = dispatch(&mut s, "/collab send hello team");
        assert!(matches!(
            eff.last(),
            Some(UiEffect::CollabAction(CollabOp::Send(m))) if m == "hello team"
        ));
        let eff = dispatch(&mut s, "/collab leave");
        assert!(matches!(
            eff.last(),
            Some(UiEffect::CollabAction(CollabOp::Leave))
        ));
    }
}
