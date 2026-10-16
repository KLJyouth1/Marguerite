//! DSH 二创斜杠命令解析器（Task T-1.3）
//!
//! 覆盖 FR-A2 的 8 条 DSH 专用命令：
//! | 命令 | 载荷 kind |
//! |---|---|
//! | `/modlens <mode> [path]`  `modlens` |
//! | `/usage [day|week|month|balance|heatmap]`  `usage` |
//! | `/pet [pet|feed|status]`  `pet` |
//! | `/games list` 或 `/games play <id>`  `games` |
//! | `/market list|search <kw>|install <name>|upgrade <name>`  `market` |
//! | `/router set <plan|execute|weak>` 或 `/router query` 或 `/router auto <task>`  `router` |
//! | `/sidebar list|pin <slot>|refresh|open <path>`  `sidebar` |
//! | `/schedule list|after <sec> <msg>|at <iso> <msg>|every <sec> <msg>|cancel <id>`  `schedule` |
//!
//! 设计：
//! - 解析函数 `parse_dslash` 独立、纯函数，便于离线单测。
//! - 不直接依赖 stdin/stdout；由上层（REPL / TUI workbench）拿到 `DshSlashCommand`
//!   后再调用 `publish_dslash_event` 写进 EventBus。
//! - 事件 envelope：`EventType::Status` + `kind = "slash.command"`，
//!   负载包含 `{ name, kind, args }`，避免为 8 条命令单独扩展 EventType enum 造成破坏性变动。

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::events::{Envelope, EventBus, EventType};

/// DSH 8 条命令分类（kind）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DshSlashKind {
    Modlens,
    Usage,
    Pet,
    Games,
    Market,
    Router,
    Sidebar,
    Schedule,
}

impl DshSlashKind {
    pub const ALL: [DshSlashKind; 8] = [
        DshSlashKind::Modlens,
        DshSlashKind::Usage,
        DshSlashKind::Pet,
        DshSlashKind::Games,
        DshSlashKind::Market,
        DshSlashKind::Router,
        DshSlashKind::Sidebar,
        DshSlashKind::Schedule,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DshSlashKind::Modlens => "modlens",
            DshSlashKind::Usage => "usage",
            DshSlashKind::Pet => "pet",
            DshSlashKind::Games => "games",
            DshSlashKind::Market => "market",
            DshSlashKind::Router => "router",
            DshSlashKind::Sidebar => "sidebar",
            DshSlashKind::Schedule => "schedule",
        }
    }

    /// 对应 manifest 声明的命令名（不含前缀 `/`）
    pub fn command_name(&self) -> &'static str {
        self.as_str()
    }
}

/// `/modlens` 子命令
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModlensMode {
    /// `/modlens paste` — 识别剪贴板
    Paste,
    /// `/modlens json <path>` — 结构化 JSON 输出
    Json { path: String },
    /// `/modlens <path>` — 默认图文识别，path 可为本地或 url
    Image { path: String },
}

/// `/usage` 作用域
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    #[default]
    Day,
    Week,
    Month,
    Balance,
    Heatmap,
}

/// `/pet` 动作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PetAction {
    Pet,
    Feed,
    #[default]
    Status,
}

/// `/games` 子命令
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GamesCmd {
    List,
    Play { game_id: String },
}

/// `/market` 子命令
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketCmd {
    List,
    Search { keyword: String },
    Install { name: String },
    Upgrade { name: String },
}

/// Router 3 模式（与 T-1.5 RouterProfile 对齐；T-1.5 再实现真实切换逻辑）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterMode {
    Plan,
    Execute,
    Weak,
}

impl RouterMode {
    #[allow(clippy::should_implement_trait)] // 命名仿 FromStr 但返回 Option，保留为解析辅助
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "plan" => RouterMode::Plan,
            "execute" | "exec" | "run" => RouterMode::Execute,
            "weak" | "auto" | "default" => RouterMode::Weak,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RouterMode::Plan => "plan",
            RouterMode::Execute => "execute",
            RouterMode::Weak => "weak",
        }
    }
}

/// `/router` 子命令
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouterCmd {
    Set { mode: RouterMode },
    Query,
    Auto { task: String },
}

/// `/sidebar` 子命令
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarCmd {
    List,
    Pin { slot: String },
    Refresh,
    Open { path: String },
}

/// `/schedule` 子命令（DSH schedule 会话级提醒；注册面，执行见 core/src/schedule）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleCmd {
    List,
    After { seconds: u64, message: String },
    At { iso: String, message: String },
    Every { seconds: u64, message: String },
    Cancel { id: String },
}

/// 8 类命令 → 载荷结构体（归一化枚举）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args")]
pub enum DshSlashPayload {
    Modlens(ModlensMode),
    Usage(UsageScope),
    Pet(PetAction),
    Games(GamesCmd),
    Market(MarketCmd),
    Router(RouterCmd),
    Sidebar(SidebarCmd),
    Schedule(ScheduleCmd),
}

/// 归一化命令结构
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DshSlashCommand {
    pub kind: DshSlashKind,
    pub payload: DshSlashPayload,
    /// 原始输入（去除头尾空白），用于日志/审计
    pub raw: String,
}

impl DshSlashCommand {
    /// 路由键第二段（EventBus 主题匹配，供 manifest.events 使用）
    pub fn topic(&self) -> String {
        format!("slash.{}", self.kind.as_str())
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SlashParseError {
    #[error("非 DSH 斜杠命令（不以 /modlens /usage /pet /games /market /router /sidebar /schedule 开头）")]
    NotDsh,
    #[error("缺少子命令或参数: {0}")]
    MissingArg(String),
    #[error("无效子命令 {0}, 期望: {1}")]
    InvalidSubcommand(String, String),
    #[error("参数解析失败: {0}")]
    InvalidArg(String),
}

/// 解析一条斜杠命令（纯函数，无 I/O）。
///
/// 非 DSH 7 类命令（如 `/exit` `/help` 等）返回 `Err(SlashParseError::NotDsh)`，
/// 由上层（repl.handle_slash）继续走原有分支。
pub fn parse_dslash(input: &str) -> Result<DshSlashCommand, SlashParseError> {
    let raw = input.trim().to_string();
    let mut parts = input.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    if !cmd.starts_with('/') {
        return Err(SlashParseError::NotDsh);
    }
    let name = cmd.trim_start_matches('/');
    let kind = match name {
        "modlens" => DshSlashKind::Modlens,
        "usage" => DshSlashKind::Usage,
        "pet" => DshSlashKind::Pet,
        "games" => DshSlashKind::Games,
        "market" => DshSlashKind::Market,
        "router" => DshSlashKind::Router,
        "sidebar" => DshSlashKind::Sidebar,
        "schedule" => DshSlashKind::Schedule,
        _ => return Err(SlashParseError::NotDsh),
    };
    let args: Vec<&str> = parts.collect();
    let payload = match kind {
        DshSlashKind::Modlens => parse_modlens(&args)?,
        DshSlashKind::Usage => parse_usage(&args)?,
        DshSlashKind::Pet => parse_pet(&args)?,
        DshSlashKind::Games => parse_games(&args)?,
        DshSlashKind::Market => parse_market(&args)?,
        DshSlashKind::Router => parse_router(&args)?,
        DshSlashKind::Sidebar => parse_sidebar(&args)?,
        DshSlashKind::Schedule => parse_schedule(&args)?,
    };
    Ok(DshSlashCommand { kind, payload, raw })
}

fn parse_modlens(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    if args.is_empty() {
        return Err(SlashParseError::MissingArg(
            "/modlens 需要 path 或 paste 或 json <path>".to_string(),
        ));
    }
    let mode = match args[0] {
        "paste" => ModlensMode::Paste,
        "json" => {
            let path = args
                .get(1)
                .copied()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    SlashParseError::MissingArg("/modlens json 需要 <path>".to_string())
                })?
                .to_string();
            ModlensMode::Json { path }
        }
        other => ModlensMode::Image {
            path: other.to_string(),
        },
    };
    Ok(DshSlashPayload::Modlens(mode))
}

fn parse_usage(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let scope = match args.first().copied().unwrap_or("day") {
        "" | "day" | "today" => UsageScope::Day,
        "week" | "w" => UsageScope::Week,
        "month" | "m" => UsageScope::Month,
        "balance" | "bal" | "$" => UsageScope::Balance,
        "heatmap" | "heat" | "h" => UsageScope::Heatmap,
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "day|week|month|balance|heatmap".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Usage(scope))
}

fn parse_pet(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let act = match args.first().copied().unwrap_or("status") {
        "" | "status" | "info" => PetAction::Status,
        "pet" | "head" | "touch" => PetAction::Pet,
        "feed" | "fish" | "eat" => PetAction::Feed,
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "pet|feed|status".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Pet(act))
}

fn parse_games(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let sub = args.first().copied().unwrap_or("list");
    let cmd = match sub {
        "" | "list" | "ls" => GamesCmd::List,
        "play" | "start" | "open" => {
            let id = args
                .get(1)
                .copied()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    SlashParseError::MissingArg(
                        "/games play 需要 <id>（如 2048/tictactoe/snake）".to_string(),
                    )
                })?
                .to_string();
            GamesCmd::Play { game_id: id }
        }
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "list|play <id>".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Games(cmd))
}

fn parse_market(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let sub = args.first().copied().unwrap_or("list");
    let rest: Vec<&str> = args.iter().skip(1).copied().collect();
    let cmd = match sub {
        "" | "list" | "ls" => MarketCmd::List,
        "search" | "find" | "s" => {
            let kw = if rest.is_empty() {
                return Err(SlashParseError::MissingArg(
                    "/market search 需要关键词".to_string(),
                ));
            } else {
                rest.join(" ")
            };
            MarketCmd::Search { keyword: kw }
        }
        "install" | "add" | "i" => {
            let name = rest
                .first()
                .copied()
                .ok_or_else(|| {
                    SlashParseError::MissingArg("/market install 需要 <name>".to_string())
                })?
                .to_string();
            MarketCmd::Install { name }
        }
        "upgrade" | "update" | "u" => {
            let name = rest
                .first()
                .copied()
                .ok_or_else(|| {
                    SlashParseError::MissingArg("/market upgrade 需要 <name>".to_string())
                })?
                .to_string();
            MarketCmd::Upgrade { name }
        }
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "list|search <kw>|install <name>|upgrade <name>".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Market(cmd))
}

fn parse_router(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let sub = args.first().copied().ok_or_else(|| {
        SlashParseError::MissingArg(
            "/router 需要子命令: set <plan|execute|weak> | query | auto <task>".to_string(),
        )
    })?;
    let cmd = match sub {
        "set" => {
            let mode_str = args.get(1).copied().ok_or_else(|| {
                SlashParseError::MissingArg("/router set 需要 <plan|execute|weak>".to_string())
            })?;
            let mode = RouterMode::from_str(mode_str).ok_or_else(|| {
                SlashParseError::InvalidSubcommand(
                    mode_str.to_string(),
                    "plan|execute|weak".to_string(),
                )
            })?;
            RouterCmd::Set { mode }
        }
        "query" | "get" | "status" => RouterCmd::Query,
        "auto" | "smart" => {
            let task: Vec<&str> = args.iter().skip(1).copied().collect();
            if task.is_empty() {
                return Err(SlashParseError::MissingArg(
                    "/router auto 需要 <task> 描述".to_string(),
                ));
            }
            RouterCmd::Auto {
                task: task.join(" "),
            }
        }
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "set <mode> | query | auto <task>".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Router(cmd))
}

fn parse_sidebar(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let sub = args.first().copied().unwrap_or("list");
    let rest: Vec<&str> = args.iter().skip(1).copied().collect();
    let cmd = match sub {
        "" | "list" | "ls" => SidebarCmd::List,
        "pin" | "fix" => {
            let slot = rest
                .first()
                .copied()
                .ok_or_else(|| {
                    SlashParseError::MissingArg("/sidebar pin 需要 <slotId>".to_string())
                })?
                .to_string();
            SidebarCmd::Pin { slot }
        }
        "refresh" | "reload" | "r" => SidebarCmd::Refresh,
        "open" | "cd" => {
            let path = if rest.is_empty() {
                return Err(SlashParseError::MissingArg(
                    "/sidebar open 需要 <path>".to_string(),
                ));
            } else {
                rest.join(" ")
            };
            SidebarCmd::Open { path }
        }
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "list|pin <slot>|refresh|open <path>".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Sidebar(cmd))
}

/// `/schedule` 解析（DSH schedule 会话级提醒；解析面，执行见 core/src/schedule）。
fn parse_schedule(args: &[&str]) -> Result<DshSlashPayload, SlashParseError> {
    let sub = args.first().copied().unwrap_or("list");
    let rest: Vec<&str> = args.iter().skip(1).copied().collect();
    let cmd = match sub {
        "" | "list" | "ls" => ScheduleCmd::List,
        "after" | "in" => {
            let seconds = rest
                .first()
                .copied()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|s| *s > 0)
                .ok_or_else(|| {
                    SlashParseError::InvalidArg("/schedule after 需要正整数秒数".to_string())
                })?;
            let message = rest
                .iter()
                .skip(1)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            if message.trim().is_empty() {
                return Err(SlashParseError::MissingArg(
                    "/schedule after 需要提醒消息".to_string(),
                ));
            }
            ScheduleCmd::After { seconds, message }
        }
        "at" => {
            let iso = rest
                .first()
                .copied()
                .ok_or_else(|| {
                    SlashParseError::MissingArg("/schedule at 需要 ISO 时刻".to_string())
                })?
                .to_string();
            let message = rest
                .iter()
                .skip(1)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            if message.trim().is_empty() {
                return Err(SlashParseError::MissingArg(
                    "/schedule at 需要提醒消息".to_string(),
                ));
            }
            ScheduleCmd::At { iso, message }
        }
        "every" => {
            let seconds = rest
                .first()
                .copied()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|s| *s > 0)
                .ok_or_else(|| {
                    SlashParseError::InvalidArg("/schedule every 需要正整数秒数".to_string())
                })?;
            let message = rest
                .iter()
                .skip(1)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            if message.trim().is_empty() {
                return Err(SlashParseError::MissingArg(
                    "/schedule every 需要提醒消息".to_string(),
                ));
            }
            ScheduleCmd::Every { seconds, message }
        }
        "cancel" | "rm" => {
            let id = rest
                .first()
                .copied()
                .ok_or_else(|| {
                    SlashParseError::MissingArg("/schedule cancel 需要 <id>".to_string())
                })?
                .to_string();
            ScheduleCmd::Cancel { id }
        }
        other => {
            return Err(SlashParseError::InvalidSubcommand(
                other.to_string(),
                "list|after <sec> <msg>|at <iso> <msg>|every <sec> <msg>|cancel <id>".to_string(),
            ))
        }
    };
    Ok(DshSlashPayload::Schedule(cmd))
}

/// 把解析出的命令以 Envelope 的方式发布进 EventBus。
///
/// 使用 `EventType::Status` + `kind = "slash.command"` 作为统一事件，
/// 负载结构：`{ "kind": "slash.command", "slash": { "name": "router", "payload": {...}, "raw": "..." } }`。
/// 路由键 `{session}.status`；同时 topic 用于 manifest `event.slash.xxx` 的按命令分类对账。
pub fn publish_dslash_event(bus: &EventBus, cmd: &DshSlashCommand, session: &str) -> Envelope {
    let payload = json!({
        "kind": "slash.command",
        "slash": {
            "name": cmd.kind.as_str(),
            "topic": cmd.topic(),
            "payload": cmd.payload,
            "raw": cmd.raw,
        }
    });
    let env = Envelope::new(EventType::Status, payload, session);
    bus.publish(env.clone());
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-1.3 TR-R1：10 个 case 覆盖 7 条命令 + 默认值变体
    #[test]
    fn parse_8_slash_commands_all_kinds() {
        // modlens
        let c = parse_dslash("/modlens /tmp/a.png").unwrap();
        assert_eq!(c.kind, DshSlashKind::Modlens);
        assert_eq!(
            c.payload,
            DshSlashPayload::Modlens(ModlensMode::Image {
                path: "/tmp/a.png".into()
            })
        );
        let c = parse_dslash("/modlens paste").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Modlens(ModlensMode::Paste));
        let c = parse_dslash("/modlens json /tmp/b.png").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Modlens(ModlensMode::Json {
                path: "/tmp/b.png".into()
            })
        );

        // usage (default Day)
        let c = parse_dslash("/usage").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Usage(UsageScope::Day));
        let c = parse_dslash("/usage month").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Usage(UsageScope::Month));
        let c = parse_dslash("/usage balance").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Usage(UsageScope::Balance));

        // pet (default Status)
        let c = parse_dslash("/pet").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Pet(PetAction::Status));
        let c = parse_dslash("/pet feed").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Pet(PetAction::Feed));

        // games
        let c = parse_dslash("/games list").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Games(GamesCmd::List));
        let c = parse_dslash("/games play snake").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Games(GamesCmd::Play {
                game_id: "snake".into()
            })
        );

        // market
        let c = parse_dslash("/market").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Market(MarketCmd::List));
        let c = parse_dslash("/market search dsh pet").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Market(MarketCmd::Search {
                keyword: "dsh pet".into()
            })
        );
        let c = parse_dslash("/market install dsh-modlens").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Market(MarketCmd::Install {
                name: "dsh-modlens".into()
            })
        );

        // router
        let c = parse_dslash("/router set plan").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Router(RouterCmd::Set {
                mode: RouterMode::Plan
            })
        );
        let c = parse_dslash("/router query").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Router(RouterCmd::Query));
        let c = parse_dslash("/router auto 修复登录页中文显示").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Router(RouterCmd::Auto {
                task: "修复登录页中文显示".into()
            })
        );

        // sidebar
        let c = parse_dslash("/sidebar").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Sidebar(SidebarCmd::List));
        let c = parse_dslash("/sidebar pin LeftSidebar").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Sidebar(SidebarCmd::Pin {
                slot: "LeftSidebar".into()
            })
        );

        // schedule
        let c = parse_dslash("/schedule").unwrap();
        assert_eq!(c.payload, DshSlashPayload::Schedule(ScheduleCmd::List));
        let c = parse_dslash("/schedule after 600 休息一下").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Schedule(ScheduleCmd::After {
                seconds: 600,
                message: "休息一下".into()
            })
        );
        let c = parse_dslash("/schedule at 2026-08-22T10:00:00Z 开会").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Schedule(ScheduleCmd::At {
                iso: "2026-08-22T10:00:00Z".into(),
                message: "开会".into()
            })
        );
        let c = parse_dslash("/schedule every 300 打卡").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Schedule(ScheduleCmd::Every {
                seconds: 300,
                message: "打卡".into()
            })
        );
        let c = parse_dslash("/schedule cancel rem_abc").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Schedule(ScheduleCmd::Cancel {
                id: "rem_abc".into()
            })
        );
        let c = parse_dslash("/sidebar open C:\\Work\\Project").unwrap();
        assert_eq!(
            c.payload,
            DshSlashPayload::Sidebar(SidebarCmd::Open {
                path: "C:\\Work\\Project".into()
            })
        );
    }

    /// T-1.3 TR-R3：错误边界 case → ParseError 不 panic
    #[test]
    fn slash_error_cases_return_err_not_panic() {
        // 非 DSH 命令
        assert_eq!(parse_dslash("/exit").unwrap_err(), SlashParseError::NotDsh);
        assert_eq!(parse_dslash("/help").unwrap_err(), SlashParseError::NotDsh);
        assert!(matches!(
            parse_dslash("普通文本输入").unwrap_err(),
            SlashParseError::NotDsh
        ));

        // 缺参数
        assert!(matches!(
            parse_dslash("/modlens").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));
        assert!(matches!(
            parse_dslash("/games play").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));
        assert!(matches!(
            parse_dslash("/router").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));
        assert!(matches!(
            parse_dslash("/router set").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));
        assert!(matches!(
            parse_dslash("/market search").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));

        // 无效子命令
        assert!(matches!(
            parse_dslash("/usage yearly").unwrap_err(),
            SlashParseError::InvalidSubcommand(_, _)
        ));
        assert!(matches!(
            parse_dslash("/pet hug").unwrap_err(),
            SlashParseError::InvalidSubcommand(_, _)
        ));
        assert!(matches!(
            parse_dslash("/router foobar").unwrap_err(),
            SlashParseError::InvalidSubcommand(_, _)
        ));
        assert!(matches!(
            parse_dslash("/router set UltraMode").unwrap_err(),
            SlashParseError::InvalidSubcommand(_, _)
        ));
        // schedule 错误路径
        assert!(matches!(
            parse_dslash("/schedule foobar").unwrap_err(),
            SlashParseError::InvalidSubcommand(_, _)
        ));
        assert!(matches!(
            parse_dslash("/schedule after abc 消息").unwrap_err(),
            SlashParseError::InvalidArg(_)
        ));
        assert!(matches!(
            parse_dslash("/schedule after 0 消息").unwrap_err(),
            SlashParseError::InvalidArg(_)
        ));
        assert!(matches!(
            parse_dslash("/schedule after 60").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));
        assert!(matches!(
            parse_dslash("/schedule cancel").unwrap_err(),
            SlashParseError::MissingArg(_)
        ));
    }

    #[test]
    fn topic_and_name_matches_manifest_events() {
        let c = parse_dslash("/pet pet").unwrap();
        assert_eq!(c.kind.as_str(), "pet");
        assert_eq!(c.topic(), "slash.pet");

        let c = parse_dslash("/router set weak").unwrap();
        assert_eq!(c.topic(), "slash.router");
    }

    /// T-1.3 TR-R2：7 条命令发布 → EventBus 入队 7 条 SlashCommand Status 事件
    #[test]
    fn slash_7_publish_events_roundtrip() {
        use crate::events::bus::DEFAULT_HISTORY_CAPACITY;
        let bus = EventBus::new(DEFAULT_HISTORY_CAPACITY);
        let inputs = [
            "/modlens paste",
            "/usage week",
            "/pet feed",
            "/games play tictactoe",
            "/market list",
            "/router set execute",
            "/sidebar refresh",
        ];
        let mut order = Vec::with_capacity(7);
        for s in inputs.iter() {
            let cmd = parse_dslash(s).expect(s);
            let env = publish_dslash_event(&bus, &cmd, "ws.test");
            order.push((cmd.kind, env.r#type));
        }
        // 历史回放 buffer 验证 7 条事件
        use std::time::Instant;
        let evs = bus.history(Instant::now() - std::time::Duration::from_secs(5));
        assert_eq!(
            evs.len(),
            7,
            "总线历史应包含 7 条 slash 事件，实际 {}. 事件列表: {:?}",
            evs.len(),
            evs.iter()
                .map(|e| (e.r#type.as_str(), e.payload["kind"].as_str()))
                .collect::<Vec<_>>()
        );
        for (i, env) in evs.iter().enumerate() {
            assert_eq!(env.r#type, EventType::Status);
            assert_eq!(env.payload["kind"], "slash.command");
            let slash = &env.payload["slash"];
            let expected_name = order[i].0.as_str();
            assert_eq!(slash["name"], expected_name, "第 {i} 条命令顺序错误");
        }
    }
}
