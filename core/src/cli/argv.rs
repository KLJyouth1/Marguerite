//! 命令行参数解析器
//!
//! 借鉴 opencode argv.ts 的设计，实现轻量级参数解析，
//! 支持单指令模式、管道模式、子命令和交互模式。

use std::path::PathBuf;

/// CLI 运行模式
#[derive(Debug, Clone, PartialEq)]
pub enum CliMode {
    /// 交互模式（无参数）
    Interactive,
    /// 单指令模式（-p/--prompt）
    Direct,
    /// 子命令模式（doctor/model/mcp/skills/session）
    Subcommand(String),
}

/// 解析后的命令行参数
#[derive(Debug, Clone)]
pub struct Args {
    /// 运行模式
    pub mode: CliMode,
    /// 单指令内容（-p/--prompt 的值）
    pub prompt: Option<String>,
    /// 输入文件路径（-f/--file 的值）
    pub file: Option<PathBuf>,
    /// 是否从 stdin 读取（--pipe）
    pub pipe: bool,
    /// 子命令后的附加参数（如 `mcp list` 中的 `list`）
    pub sub_args: Vec<String>,
    /// 顶层 `--help` / `-h`（AC-01）
    pub help: bool,
    /// 顶层 `--version` / `-V`（AC-01）
    pub version: bool,
    /// `--json` / `--output-format json`：机器可读输出（AC-03）
    pub json: bool,
    /// 未知选项（以 `-` 开头但未被识别），供调用方按 AC-02 报错
    pub unknown_flags: Vec<String>,
    /// 未知子命令（非 `-` 开头、不在 SUBCOMMANDS 中的裸词），供调用方按 AC-02 报错
    pub unknown_subcommand: Option<String>,
    /// 子命令级 `--help` / `-h`（如 `marguerite doctor --help`，AC-01 子命令级），
    /// 仅在 `--` 分隔符之前出现时生效（`exec -- -h` 透传给命令本身，不触发帮助）
    pub subcommand_help: bool,
    /// `--health`：健康态探测（供 `serve --health` 等叶子探测命令消费）
    pub health: bool,
    /// `--port <n>`：端口（供 `serve --port` 等消费）
    pub port: Option<u16>,
    /// `--resume <session-id>`：恢复指定会话继续对话（供 `chat --resume` 消费）
    pub resume: Option<String>,
    /// `--continue` / `-c`：恢复最近一次会话（供 `chat --continue` 消费）
    pub continue_last: bool,
    /// `--tools` / `-t`：启用 LLM function calling 工具回路（供 `chat` 消费）
    pub tools: bool,
    /// `--agent <id>`：以指定智能体的人设执行单指令链（供 `marguerite -p "..." --agent <id>` 消费）
    pub agent: Option<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            mode: CliMode::Interactive,
            prompt: None,
            file: None,
            pipe: false,
            sub_args: Vec::new(),
            help: false,
            version: false,
            json: false,
            unknown_flags: Vec::new(),
            unknown_subcommand: None,
            subcommand_help: false,
            health: false,
            port: None,
            resume: None,
            continue_last: false,
            tools: false,
            agent: None,
        }
    }
}

/// 已知的子命令列表（与 `program.rs` 中注册的路由保持一致）
pub const SUBCOMMANDS: &[&str] = &[
    "doctor",
    "model",
    "mcp",
    "skills",
    "session",
    "exec",
    "deploy",
    "serve",
    "sandbox",
    "subagent",
    "team",
    "browser",
    "chat",
    "gateway",
    "capabilities",
    "cache",
];

/// 解析命令行参数
///
/// 支持的参数格式：
/// - `-p "指令"` 或 `--prompt "指令"` — 单指令模式
/// - `-f <文件>` 或 `--file <文件>` — 指定输入文件
/// - `--pipe` — 管道模式，从 stdin 读取
/// - `--help` / `-h` / `--version` / `-V` — 顶层发现（打印后退出 0）
/// - `--json` / `--output-format json` — 机器可读输出（AC-03）
/// - 子命令: `doctor`, `model`, `mcp`, `skills`, `session`, `exec`, `deploy`, `serve`,
///   `sandbox`, `subagent`, `team`, `browser`, `chat`, `gateway`, `capabilities`
/// - 子命令级帮助: `marguerite doctor --help`（AC-01 子命令级，退出 0）
/// - `chat` 子命令旗标: `--resume <id>` / `--continue`(`-c`) / `--tools`(`-t`) / `--json`
/// - 无参数 — 交互模式
pub fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    parse_args_from(&raw)
}

/// 从参数列表解析（供 `parse_args` 与测试复用）
fn parse_args_from(raw: &[String]) -> Args {
    let mut args = Args::default();

    let mut i = 1; // 跳过程序名
    while i < raw.len() {
        match raw[i].as_str() {
            "-h" | "--help" => {
                args.help = true;
            }
            "-V" | "--version" => {
                args.version = true;
            }
            "--json" => {
                args.json = true;
            }
            "--output-format" => {
                i += 1;
                if let Some(v) = raw.get(i) {
                    if v == "json" {
                        args.json = true;
                    }
                }
            }
            "-p" | "--prompt" => {
                i += 1;
                if i < raw.len() {
                    args.mode = CliMode::Direct;
                    args.prompt = Some(raw[i].clone());
                }
            }
            "-f" | "--file" => {
                i += 1;
                if i < raw.len() {
                    args.file = Some(PathBuf::from(&raw[i]));
                }
            }
            "--pipe" => {
                args.pipe = true;
            }
            "--agent" => {
                // 指定执行人设来源智能体（direct 模式）：`marguerite -p "..." --agent <id>`
                i += 1;
                if let Some(v) = raw.get(i) {
                    if !v.starts_with('-') {
                        args.agent = Some(v.clone());
                    } else {
                        // 值缺失或紧跟选项：不吞后续选项，回退为未指定
                        i -= 1;
                    }
                }
            }
            // 检查是否为子命令
            sub => {
                if SUBCOMMANDS.contains(&sub) {
                    args.mode = CliMode::Subcommand(sub.to_string());
                    // 收集子命令后的附加参数（如 `session delete <id>`）
                    // `--` 之后的所有参数原样收集（含 `-` 前缀），供 `exec -- <命令>` 使用
                    let mut j = i + 1;
                    let mut after_sep = false;
                    while j < raw.len() {
                        if !after_sep {
                            if raw[j] == "--" {
                                after_sep = true;
                                j += 1;
                                continue;
                            }
                            if raw[j] == "-h" || raw[j] == "--help" {
                                // 子命令级帮助请求（AC-01）：记录后停止收集，避免被当作未知选项
                                args.subcommand_help = true;
                                break;
                            }
                            if raw[j] == "--health" {
                                // 健康探测旗标：被 serve 等消费，不进入 sub_args
                                args.health = true;
                                j += 1;
                                continue;
                            }
                            if raw[j] == "--port" {
                                // 端口旗标：被 serve 等消费，不进入 sub_args
                                if let Some(v) = raw.get(j + 1).and_then(|s| s.parse::<u16>().ok())
                                {
                                    args.port = Some(v);
                                }
                                j += 2;
                                continue;
                            }
                            if raw[j] == "--json" {
                                // JSON 输出旗标：被 doctor/chat 等消费，不进入 sub_args
                                args.json = true;
                                j += 1;
                                continue;
                            }
                            if raw[j] == "--resume" {
                                // 会话恢复旗标：被 chat 消费，不进入 sub_args。
                                // 值缺失或紧跟另一个选项时视为「未指定 id」，由调用方回退到最近会话。
                                match raw.get(j + 1) {
                                    Some(v) if !v.starts_with('-') => {
                                        args.resume = Some(v.clone());
                                        j += 2;
                                    }
                                    _ => {
                                        args.continue_last = true;
                                        j += 1;
                                    }
                                }
                                continue;
                            }
                            if raw[j] == "--continue" || raw[j] == "-c" {
                                // 恢复最近一次会话：被 chat 消费，不进入 sub_args
                                args.continue_last = true;
                                j += 1;
                                continue;
                            }
                            if raw[j] == "--tools" || raw[j] == "-t" {
                                // 启用 LLM 工具回路：被 chat 消费，不进入 sub_args
                                args.tools = true;
                                j += 1;
                                continue;
                            }
                            if raw[j].starts_with('-') {
                                // 其它未知选项：停止收集（交由顶层 unknown_flags 处理）
                                break;
                            }
                        }
                        args.sub_args.push(raw[j].clone());
                        j += 1;
                    }
                    if args.subcommand_help {
                        // 帮助请求：越过 `-h/--help`，不再做后续顶层解析（防被 unknown_flags 捕获）
                        i = raw.len();
                    } else {
                        i = j - 1;
                    }
                } else if sub.starts_with('-') {
                    // 未知选项：记录后由调用方按 AC-02 报错，而非静默忽略
                    args.unknown_flags.push(sub.to_string());
                } else {
                    // 未知裸词（疑似拼错的子命令）：记录后由调用方按 AC-02 报错
                    args.unknown_subcommand = Some(sub.to_string());
                }
                // 其它非 '-' 开头的参数也被忽略（顶层参数已处理）
            }
        }
        i += 1;
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试辅助：将 `&[&str]` 转为 `&[String]` 后走真实解析逻辑
    fn parse_args_with(raw: &[&str]) -> Args {
        let owned: Vec<String> = raw.iter().map(|s| s.to_string()).collect();
        parse_args_from(&owned)
    }

    #[test]
    fn test_parse_prompt() {
        let args = parse_args_with(&["marguerite", "-p", "hello"]);
        assert_eq!(args.mode, CliMode::Direct);
        assert_eq!(args.prompt, Some("hello".to_string()));
    }

    #[test]
    fn test_parse_prompt_long() {
        let args = parse_args_with(&["marguerite", "--prompt", "fix bug"]);
        assert_eq!(args.mode, CliMode::Direct);
        assert_eq!(args.prompt, Some("fix bug".to_string()));
    }

    #[test]
    fn test_parse_file() {
        let args = parse_args_with(&["marguerite", "-f", "main.rs"]);
        assert_eq!(args.file, Some(PathBuf::from("main.rs")));
    }

    #[test]
    fn test_parse_pipe() {
        let args = parse_args_with(&["marguerite", "--pipe"]);
        assert!(args.pipe);
    }

    #[test]
    fn test_parse_subcommand() {
        let args = parse_args_with(&["marguerite", "doctor"]);
        assert_eq!(args.mode, CliMode::Subcommand("doctor".to_string()));
    }

    #[test]
    fn test_parse_no_args() {
        let args = parse_args_with(&["marguerite"]);
        assert_eq!(args.mode, CliMode::Interactive);
    }

    #[test]
    fn test_parse_combined() {
        let args = parse_args_with(&["marguerite", "-p", "explain", "-f", "src/main.rs", "--pipe"]);
        assert_eq!(args.mode, CliMode::Direct);
        assert_eq!(args.prompt, Some("explain".to_string()));
        assert_eq!(args.file, Some(PathBuf::from("src/main.rs")));
        assert!(args.pipe);
    }

    #[test]
    fn test_parse_unknown_option_ignored() {
        let args = parse_args_with(&["marguerite", "-x", "value"]);
        assert_eq!(args.mode, CliMode::Interactive);
        assert!(args.prompt.is_none());
    }

    #[test]
    fn test_parse_subcommand_after_options() {
        let args = parse_args_with(&["marguerite", "--pipe", "skills"]);
        assert_eq!(args.mode, CliMode::Subcommand("skills".to_string()));
        assert!(args.pipe);
    }

    #[test]
    fn test_parse_prompt_missing_value() {
        let args = parse_args_with(&["marguerite", "-p"]);
        assert_eq!(args.mode, CliMode::Interactive);
        assert!(args.prompt.is_none());
    }

    #[test]
    fn test_parse_unknown_subcommand_recorded() {
        let args = parse_args_with(&["marguerite", "unknown"]);
        assert_eq!(args.mode, CliMode::Interactive);
        assert_eq!(args.unknown_subcommand, Some("unknown".to_string()));
    }

    #[test]
    fn test_parse_subcommand_with_args() {
        let args = parse_args_with(&["marguerite", "session", "list"]);
        assert_eq!(args.mode, CliMode::Subcommand("session".to_string()));
        assert_eq!(args.sub_args, vec!["list".to_string()]);
    }

    #[test]
    fn test_parse_subcommand_with_multi_args() {
        let args = parse_args_with(&["marguerite", "session", "delete", "abc123"]);
        assert_eq!(args.mode, CliMode::Subcommand("session".to_string()));
        assert_eq!(
            args.sub_args,
            vec!["delete".to_string(), "abc123".to_string()]
        );
    }

    #[test]
    fn test_parse_subcommand_without_args() {
        let args = parse_args_with(&["marguerite", "doctor"]);
        assert!(args.sub_args.is_empty());
    }

    #[test]
    fn test_parse_subcommand_args_stop_at_option() {
        let args = parse_args_with(&["marguerite", "session", "list", "--pipe"]);
        assert_eq!(args.mode, CliMode::Subcommand("session".to_string()));
        assert_eq!(args.sub_args, vec!["list".to_string()]);
        assert!(args.pipe);
    }

    #[test]
    fn test_parse_exec_with_dashdash_keeps_flags() {
        // `exec -- rm -rf /`：`--` 之后的参数（含 `-` 前缀）原样进入 sub_args
        let args = parse_args_with(&["marguerite", "exec", "--", "rm", "-rf", "/"]);
        assert_eq!(args.mode, CliMode::Subcommand("exec".to_string()));
        assert_eq!(
            args.sub_args,
            vec!["rm".to_string(), "-rf".to_string(), "/".to_string()]
        );
    }

    #[test]
    fn test_parse_exec_without_sep_stops_at_option() {
        // 未使用 `--` 时保持原行为：遇 `-` 前缀参数停止收集
        let args = parse_args_with(&["marguerite", "exec", "ls", "-la"]);
        assert_eq!(args.mode, CliMode::Subcommand("exec".to_string()));
        assert_eq!(args.sub_args, vec!["ls".to_string()]);
    }

    #[test]
    fn test_parse_subcommand_help_flag() {
        let args = parse_args_with(&["marguerite", "doctor", "--help"]);
        assert_eq!(args.mode, CliMode::Subcommand("doctor".to_string()));
        assert!(args.subcommand_help);
        assert!(args.sub_args.is_empty());
    }

    #[test]
    fn test_parse_subcommand_short_help_flag() {
        let args = parse_args_with(&["marguerite", "model", "-h"]);
        assert_eq!(args.mode, CliMode::Subcommand("model".to_string()));
        assert!(args.subcommand_help);
    }

    #[test]
    fn test_parse_subcommand_help_before_dashdash_is_help() {
        // `--` 之前的 -h 视为帮助请求，并短路停止收集（帮助优先）
        let args = parse_args_with(&["marguerite", "exec", "--help", "--", "ls", "-h"]);
        assert!(args.subcommand_help);
        assert!(args.sub_args.is_empty());
    }

    #[test]
    fn test_parse_serve_health_flag() {
        let args = parse_args_with(&["marguerite", "serve", "--health"]);
        assert_eq!(args.mode, CliMode::Subcommand("serve".to_string()));
        assert!(args.health);
        assert!(args.sub_args.is_empty());
    }

    #[test]
    fn test_parse_serve_port_flag() {
        let args = parse_args_with(&["marguerite", "serve", "--port", "9000"]);
        assert_eq!(args.mode, CliMode::Subcommand("serve".to_string()));
        assert_eq!(args.port, Some(9000));
    }

    #[test]
    fn test_parse_serve_health_and_port() {
        let args = parse_args_with(&["marguerite", "serve", "--health", "--port", "18181"]);
        assert!(args.health);
        assert_eq!(args.port, Some(18181));
    }

    #[test]
    fn test_parse_exec_still_stops_at_unknown_option() {
        // 非 --health/--port 的未知选项仍停止收集（保持既有行为）
        let args = parse_args_with(&["marguerite", "exec", "ls", "-la"]);
        assert_eq!(args.mode, CliMode::Subcommand("exec".to_string()));
        assert_eq!(args.sub_args, vec!["ls".to_string()]);
        assert!(!args.health);
    }

    #[test]
    fn test_parse_subcommand_json_flag_collects_prompt() {
        // `chat --json "解释 tokio"`：`--json` 被消费为 args.json，prompt 进入 sub_args
        // 回归：曾因循环遇 `-` 即 break，导致 prompt 被误判为未知子命令
        let args = parse_args_with(&["marguerite", "chat", "--json", "解释 tokio"]);
        assert_eq!(args.mode, CliMode::Subcommand("chat".to_string()));
        assert!(args.json);
        assert_eq!(args.sub_args, vec!["解释 tokio".to_string()]);
    }

    #[test]
    fn test_parse_subcommand_json_flag_then_health_is_separate() {
        // 确认 `--json` 不影响其它子命令旗标识别
        let args = parse_args_with(&["marguerite", "chat", "--json", "hi"]);
        assert!(args.json);
        assert_eq!(args.sub_args, vec!["hi".to_string()]);
        assert!(!args.health);
    }

    #[test]
    fn test_parse_chat_resume_with_id() {
        // `chat --resume <id>`：id 被消费为 args.resume，不进入 sub_args
        let args = parse_args_with(&["marguerite", "chat", "--resume", "sess-abc123"]);
        assert_eq!(args.mode, CliMode::Subcommand("chat".to_string()));
        assert_eq!(args.resume, Some("sess-abc123".to_string()));
        assert!(!args.continue_last);
        assert!(args.sub_args.is_empty());
    }

    #[test]
    fn test_parse_chat_resume_without_id_falls_back_to_continue() {
        // `chat --resume` 缺少 id：回退为「恢复最近一次会话」，不误吞后续参数
        let args = parse_args_with(&["marguerite", "chat", "--resume"]);
        assert_eq!(args.resume, None);
        assert!(args.continue_last);
    }

    #[test]
    fn test_parse_chat_continue_short_and_long() {
        for flag in ["--continue", "-c"] {
            let args = parse_args_with(&["marguerite", "chat", flag]);
            assert_eq!(args.mode, CliMode::Subcommand("chat".to_string()));
            assert!(args.continue_last, "flag {flag} 未被识别");
            assert!(args.sub_args.is_empty());
        }
    }

    #[test]
    fn test_parse_chat_tools_short_and_long() {
        for flag in ["--tools", "-t"] {
            let args = parse_args_with(&["marguerite", "chat", flag]);
            assert_eq!(args.mode, CliMode::Subcommand("chat".to_string()));
            assert!(args.tools, "flag {flag} 未被识别");
            assert!(args.sub_args.is_empty());
        }
    }

    #[test]
    fn test_parse_chat_tools_not_swallowing_prompt() {
        // `chat --tools 帮我写个脚本`：--tools 消费后，prompt 仍进入 sub_args
        let args = parse_args_with(&["marguerite", "chat", "--tools", "帮我写个脚本"]);
        assert!(args.tools);
        assert_eq!(args.sub_args, vec!["帮我写个脚本".to_string()]);
    }

    #[test]
    fn test_parse_chat_resume_with_prompt_and_json() {
        // 组合场景：恢复会话 + 单发 prompt + JSON 输出，三者互不干扰
        let args = parse_args_with(&[
            "marguerite",
            "chat",
            "--resume",
            "sess-xyz",
            "--json",
            "继续上次的话题",
        ]);
        assert_eq!(args.resume, Some("sess-xyz".to_string()));
        assert!(args.json);
        assert_eq!(args.sub_args, vec!["继续上次的话题".to_string()]);
    }

    #[test]
    fn test_parse_chat_resume_not_swallowing_next_flag() {
        // `chat --resume --json`：`--json` 不应被当作 session id 吞掉
        let args = parse_args_with(&["marguerite", "chat", "--resume", "--json"]);
        assert_eq!(args.resume, None);
        assert!(args.continue_last);
        assert!(args.json);
    }

    #[test]
    fn test_parse_agent_flag() {
        // `marguerite -p "..." --agent <id>`：--agent 被消费为 args.agent，不干扰 prompt
        let args = parse_args_with(&["marguerite", "-p", "解释代码", "--agent", "code-reviewer"]);
        assert_eq!(args.mode, CliMode::Direct);
        assert_eq!(args.prompt, Some("解释代码".to_string()));
        assert_eq!(args.agent, Some("code-reviewer".to_string()));
    }

    #[test]
    fn test_parse_agent_flag_before_prompt() {
        // `--agent` 出现在 -p 之前也应被正确消费
        let args = parse_args_with(&["marguerite", "--agent", "main", "-p", "fix"]);
        assert_eq!(args.agent, Some("main".to_string()));
        assert_eq!(args.prompt, Some("fix".to_string()));
    }

    #[test]
    fn test_parse_agent_flag_missing_value_falls_back() {
        // `--agent` 缺值：不吞后续参数，回退为未指定
        let args = parse_args_with(&["marguerite", "-p", "fix", "--agent"]);
        assert_eq!(args.prompt, Some("fix".to_string()));
        assert_eq!(args.agent, None);
    }

    #[test]
    fn test_parse_team_subcommand() {
        // `marguerite team "<goal>"`：team 必须为已知子命令（SUBCOMMANDS 已含），
        // 目标进入 sub_args，供 team_handler 构造专家团队并行 spawn。
        let args = parse_args_with(&["marguerite", "team", "实现一个登录功能"]);
        assert_eq!(args.mode, CliMode::Subcommand("team".to_string()));
        assert_eq!(args.sub_args, vec!["实现一个登录功能".to_string()]);
    }
}
