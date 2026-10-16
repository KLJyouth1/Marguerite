//! 主程序入口
//!
//! `Program` 结构体持有路由表和配置，管理完整的生命周期：
//! 1. 解析参数 (argv)
//! 2. 加载配置
//! 3. 初始化日志
//! 4. 路由分发（TUI / 单指令 Agent / 子命令）
//! 5. 执行对应处理

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use tracing::info;

use super::argv::{parse_args, Args, CliMode, SUBCOMMANDS};
use super::output::{print_json_err, print_json_ok, ExitCode};
use super::route::RouteTable;
use crate::agent::runtime::AgentRuntime;
use crate::cli::repl::{run_repl, stream_assistant, ReplOptions, ReplStore};
use crate::deploy::baota::{
    add_panel, resolve_panel, BaotaDeployOptions, RecordingBaotaTransport, ReqwestBaotaTransport,
};
use crate::deploy::ssh::{
    DeployOptions, RecordingTransport, SshDeployer, SshTarget, SystemSshTransport,
};
use crate::domain::capabilities::{
    registry, stats, Capability, CapabilityGroup, CapabilityMaturity,
};
use crate::gateway::{GatewayState, DEFAULT_PORT};
use crate::mcp::registry::ToolRegistry;
use crate::memory::recall::KbBackedMemory;
use crate::model::llm::ToolDef;
use crate::model::selector::{models_path, profile_from_flags, ModelSelector};
use crate::orchestration::{EngineInvoker, Orchestrator, TaskAnalysis};
use crate::persistence::config::Config;
use crate::persistence::session::SessionManager;
use crate::serve::acp::listener::start_listener;
use crate::serve::acp::session::{AcpSession, AgentInvoker};
use crate::skills::loader::SkillLoader;
use crate::tools::builtin::register_builtin_tools;
use crate::types::Message;
use chrono::Utc;
use std::sync::Arc;

/// 主程序
pub struct Program {
    /// 路由表
    route_table: RouteTable,
    /// 应用配置
    config: Option<Config>,
}

impl Program {
    /// 创建新的 Program 实例，初始化默认路由
    pub fn new() -> Self {
        let mut route_table = RouteTable::new();

        // 注册默认子命令
        route_table.register("doctor", doctor_handler, "系统诊断");
        route_table.register("model", model_handler, "模型管理");
        route_table.register("mcp", mcp_handler, "MCP 工具管理");
        route_table.register("skills", skills_handler, "技能管理");
        route_table.register("session", session_handler, "会话管理");
        route_table.register("exec", exec_handler, "沙箱命令执行");
        route_table.register("sandbox", sandbox_handler, "沙箱执行与状态");
        route_table.register("subagent", subagent_handler, "子代理团队 DAG 编排");
        route_table.register("team", team_handler, "专家团队并行 spawn（Team::run）");
        route_table.register("browser", browser_handler, "真实浏览器 CDP 操控");
        route_table.register("deploy", deploy_handler, "一键部署（SSH）");
        route_table.register("serve", serve_handler, "ACP 服务（回环）");
        route_table.register("chat", chat_handler, "对话（流式 REPL / 单发）");
        route_table.register("gateway", gateway_handler, "统一网关（回环 Web 控制台）");
        route_table.register("goal", goal_handler, "目标循环（自治续轮 + blocked 门控）");
        route_table.register(
            "capabilities",
            capabilities_handler,
            "能力自检（真实落地状态）",
        );
        route_table.register("cache", cache_handler, "KV 缓存命中率与可缓存比");
        route_table.register(
            "dispatch",
            dispatch_handler,
            "无头分发任务到 11 个外部 agent CLI（meta-agent 调度一体内置）",
        );
        route_table.register(
            "games",
            games_handler,
            "内置小游戏（list / play <id> <动作>）",
        );
        route_table.register(
            "export-diagnostics",
            export_diagnostics_handler,
            "导出脱敏诊断包（tar；所有文本过 scrub_text）",
        );

        Self {
            route_table,
            config: None,
        }
    }

    /// 运行主程序
    ///
    /// 完整生命周期：
    /// 1. 解析命令行参数
    /// 2. 加载配置文件
    /// 3. 初始化日志系统
    /// 4. 根据模式路由分发
    /// 5. 执行对应处理函数
    pub fn run(&mut self) -> Result<()> {
        // 1. 解析参数
        let args = parse_args();

        // 顶层发现（P0：AC-01 / AC-02）
        if args.version {
            println!("marguerite {}", CLI_VERSION);
            std::process::exit(0);
        }
        if args.help {
            print_top_help();
            std::process::exit(0);
        }
        if !args.unknown_flags.is_empty() {
            for f in &args.unknown_flags {
                eprintln!("错误: 未知选项 '{f}'");
            }
            eprintln!("用法: marguerite [--help] [--version] [-p <指令>] [--pipe] [资源 动作]");
            eprintln!("运行 'marguerite --help' 查看完整用法与可用资源。");
            std::process::exit(2);
        }
        if let Some(sub) = &args.unknown_subcommand {
            eprintln!("错误: 未知子命令 '{sub}'");
            eprintln!("可用子命令: {}", SUBCOMMANDS.join(", "));
            eprintln!("运行 'marguerite --help' 查看完整用法。");
            std::process::exit(2);
        }
        // 子命令级帮助（P0：AC-01 子命令级），须置于 unknown 校验之后
        if let CliMode::Subcommand(name) = &args.mode {
            if args.subcommand_help {
                print_subcommand_help(name);
                std::process::exit(0);
            }
        }

        // 2. 加载配置
        let config = Config::load()?;

        // 2.1 按配置初始化嵌入后端（默认 local；backend=model 加载真实模型，失败回退）
        crate::memory::EmbeddingProvider::configure(&config);
        self.config = Some(config);

        // 3. 初始化日志
        self.init_logging();

        info!("Marguerite 启动 (mode: {:?})", args.mode);

        // 4. 路由分发
        match &args.mode {
            CliMode::Interactive => {
                // 交互模式 - 进入三栏工作台 TUI
                let config = self.config.as_ref().expect("配置已加载").clone();
                if std::env::var("MARGUERITE_LEGACY_UI").is_ok() {
                    // 回退到 v1 单栏 TUI（用于排查 Phase B 渲染问题）
                    let mut app = crate::tui::App::with_config(config);
                    app.run()
                } else {
                    let mut app = crate::tui::WorkbenchApp::from_config(&config)?;
                    app.run()
                }
            }
            CliMode::Direct => {
                // 单指令模式 - 直接执行
                self.handle_direct(&args)
            }
            CliMode::Subcommand(name) => {
                // 子命令模式 - 路由到对应处理函数
                match self.route_table.route(&args) {
                    Some(route) => {
                        info!("执行子命令: {}", route.name);
                        (route.handler)(&args, self.config.as_ref().expect("配置已加载"))
                    }
                    None => {
                        eprintln!("错误: 未知子命令 '{name}'");
                        eprintln!("可用子命令: {}", SUBCOMMANDS.join(", "));
                        eprintln!("运行 'marguerite --help' 查看完整用法。");
                        std::process::exit(2);
                    }
                }
            }
        }
    }

    /// 处理单指令模式
    ///
    /// 构建完整提示（含 `-f` 文件内容 / `--pipe` 标准输入），
    /// 优先走调用链（意图有模板时），否则通过 AgentRuntime 执行。
    ///
    /// 三段式降级（P0-B 离线首启体验）：
    /// 1. **调用链**（本地模板 / 技能，无需密钥）；
    /// 2. **离线回显**：未配置模型密钥时不再报错退出，而是用 [`Orchestrator::analyze`]
    ///    给出纯规则编排判定 + 原始输入回显（**不调用任何 LLM、不伪造模型回答**），退出码 0；
    /// 3. **真实模型**：配置了密钥才走 `AgentRuntime`。
    fn handle_direct(&self, args: &Args) -> Result<()> {
        let raw_prompt = if args.pipe {
            // 有界读取：读到 EOF 立即返回；EOF 不来时由空闲超时 + 总时长兜底收敛，
            // 绝不无界阻塞在 stdin（详见 crate::cli::pipe_input 的根因说明）。
            let got = crate::cli::pipe_input::read_stdin_pipe()?;
            if got.is_empty_timeout() {
                bail!(
                    "读取标准输入超时（{} 秒内未收到任何数据，也未读到 EOF）：--pipe 需要管道输入，\
                     例如 `echo \"指令\" | marguerite --pipe`；亦可直接用 -p \"指令\"",
                    crate::cli::pipe_input::idle_timeout().as_secs()
                );
            }
            if got.text.trim().is_empty() {
                // 空输入（`--pipe < NUL` / `echo -n "" | marguerite --pipe`）：
                // 无论 EOF 是否到达，都必须是**明确错误 + 非零退出**，绝不静默成功。
                // EOF 路径此前会落到下面那句「未提供指令」——语义含糊；这里给出可操作的提示。
                bail!(
                    "--pipe 未从标准输入读到任何内容：请通过管道传入指令，\
                     例如 `echo \"指令\" | marguerite --pipe`；亦可直接用 -p \"指令\""
                );
            }
            if got.timed_out {
                eprintln!(
                    "警告：标准输入在 {} 秒内未结束（未见 EOF），已按已读取的 {} 字节继续。",
                    crate::cli::pipe_input::idle_timeout().as_secs(),
                    got.text.len()
                );
            }
            got.text.trim().to_string()
        } else {
            args.prompt.clone().unwrap_or_default()
        };

        if raw_prompt.is_empty() {
            bail!("未提供指令，使用 -p \"指令\" 或 --pipe 从标准输入读取");
        }

        // 附加文件内容（-f）
        let mut prompt = raw_prompt.clone();
        if let Some(path) = &args.file {
            if path.exists() {
                let content = std::fs::read_to_string(path)?;
                prompt = format!("{prompt}\n\n文件 {} 内容：\n{content}", path.display());
            } else {
                eprintln!("警告：文件不存在，已忽略: {}", path.display());
            }
        }

        // 配置优先复用 `run()` 已加载的实例（已叠加 env / .env 覆盖，避免二次读盘），
        // 未注入时（单测直接调用 handle_direct）回退到 Config::load()。
        let config = match &self.config {
            Some(c) => c.clone(),
            None => Config::load()?,
        };
        let rt = tokio::runtime::Runtime::new()?;

        // 1) 调用链优先（本地模板 / 技能路径，无密钥也可离线完成）
        if let Some(chain_output) = rt.block_on(crate::chain::cli::run_direct_chain(
            &config,
            &raw_prompt,
            args.file.as_deref(),
            args.agent.as_deref(),
        )) {
            print_direct_response(args, &chain_output);
            return Ok(());
        }

        // 2) 无模型密钥 → 离线回显（纯规则，不联网；退出码 0：这是正常降级路径，不是错误）
        if needs_offline_echo(&config) {
            return print_offline_echo(args, &config, &raw_prompt);
        }

        // 3) 配置了密钥 → 真实模型执行
        let result = rt.block_on(async {
            // 克隆：`config` 之后仍需在鉴权失败降级路径（print_offline_echo_with_hint）复用。
            let mut runtime = AgentRuntime::new(config.clone());
            runtime.init()?;
            let resp = runtime.process_input(&prompt).await?;
            Ok::<String, anyhow::Error>(resp.to_string())
        });

        match result {
            Ok(resp) => print_direct_response(args, &resp),
            Err(e) => {
                // P0-1d：密钥存在但被上游拒绝（401/403 → AUTH_FAILED）时，把离线降级的触发条件
                // 从「没有 Key」扩展到「鉴权失败」——与无密钥同属「本次无法得到真实模型回答」，
                // 应走正常的离线降级路径（退出码 0）而非裸报错；同时以「鉴权失败」专用提示行
                // 如实说明密钥被拒（不谎称「未配置」）。
                // 分支条件收敛在 [`auth_failure_echo_hint`]（表驱动护栏锁死，见
                // `test_auth_failure_offline_echo_branch_condition`），防止判定被人
                // 无声改坏而门禁全绿（QA 变异 M6 的教训）。
                if let Some(hint) = auth_failure_echo_hint(&e) {
                    return print_offline_echo_with_hint(args, &config, &raw_prompt, hint);
                }
                if args.json {
                    print_json_err(
                        ExitCode::Temporary,
                        &e.to_string(),
                        Some("检查 API Key 与网络连通性"),
                        false,
                    );
                } else {
                    eprintln!("执行失败: {e}");
                }
                std::process::exit(1);
            }
        }
        Ok(())
    }

    /// 初始化日志系统
    ///
    /// 日志输出到 stderr，避免污染 stdout 业务输出；
    /// 默认级别 warn，可通过 RUST_LOG 环境变量覆盖。
    fn init_logging(&self) {
        // 使用 tracing-subscriber 初始化日志
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "warn".into()),
            )
            .try_init();
    }

    /// 获取路由表引用
    pub fn route_table(&self) -> &RouteTable {
        &self.route_table
    }
}

/// 判断是否应走离线回显：模型密钥缺失或仅剩空白即视为「未配置」。
///
/// 单指令模式只经 `AgentRuntime::new(config)` 取密钥（不查凭证台账），因此这里的判据
/// 与真实执行路径一致：空白密钥进真实调用必然失败，只应降级而非报错退出。
fn needs_offline_echo(config: &Config) -> bool {
    config.model.api_key.trim().is_empty()
}

/// P0-1d 分支判定（纯函数）：该稳定错误码是否应触发「鉴权失败 → 离线降级」。
///
/// 这是 [`auth_failure_echo_hint`]（`handle_direct` Err 分支的判定入口）引用的
/// **唯一条件出处**；`test_auth_failure_offline_echo_branch_condition` 表驱动锁死——
/// 把这里的判定改掉（例如换成永不相等的字面量）护栏测试必然变红。
fn should_offline_echo_on_auth_failure(code: Option<&str>) -> bool {
    code == Some("AUTH_FAILED")
}

/// 单指令模式真实执行失败后的降级决策：仅当错误归类为 `AUTH_FAILED` 时返回
/// 鉴权失败专用提示行（走离线回显），否则 `None`（按普通失败处理）。
///
/// 收敛为函数的原因（QA 变异 M6）：此前分支条件内联在 `handle_direct` 里，
/// 把 `== Some("AUTH_FAILED")` 改掉后全部既有测试仍然通过——只测渲染函数
/// 保护不了调用点。现调用点只允许经由本函数分流，由行为护栏测试直接驱动。
fn auth_failure_echo_hint(e: &anyhow::Error) -> Option<&'static str> {
    should_offline_echo_on_auth_failure(crate::model::error_taxonomy::code_of(e))
        .then_some(OFFLINE_ECHO_AUTH_HINT)
}

/// 单指令模式的统一成功输出（文本 / `--json` 契约共用）。
fn print_direct_response(args: &Args, resp: &str) {
    if args.json {
        print_json_ok(serde_json::json!({ "response": resp }));
    } else {
        println!("{resp}");
    }
}

/// 离线回显的固定提示行（文本与 JSON 共用同一常量，避免两处文案漂移）。
pub const OFFLINE_ECHO_HINT: &str =
    "未配置模型密钥，已离线回显；配置方法：`marguerite model add ...` 或设置 `MARGUERITE_API_KEY`。";

/// 鉴权失败的离线回显提示行（P0-1d：把离线降级触发条件从「没有 Key」扩展到「鉴权失败」）。
///
/// 与「未配置密钥」区分：这里密钥**存在但被上游拒绝**（401/403），文案必须如实说明，
/// 不得谎称「未配置」（诚实性边界）。
pub const OFFLINE_ECHO_AUTH_HINT: &str =
    "模型密钥被上游拒绝（鉴权失败 401/403），已离线回显；请检查密钥是否有效：`marguerite model add ...` 或设置 `MARGUERITE_API_KEY`。";

/// 渲染离线回显块（纯函数，便于单测）——使用默认「未配置密钥」提示行。
fn render_offline_echo(prompt: &str, analysis: &TaskAnalysis, config_path: &Path) -> String {
    render_offline_echo_with_hint(prompt, analysis, config_path, OFFLINE_ECHO_HINT)
}

/// 渲染离线回显块（可变提示行；P0-1d 的鉴权失败降级复用同一渲染逻辑，只换首行文案）。
///
/// 语义边界（不可越界）：本块**只**包含原始输入 + 本地规则编排判定 + 配置指引，
/// 不包含任何模型生成的回答，也不假装调用过模型。
fn render_offline_echo_with_hint(
    prompt: &str,
    analysis: &TaskAnalysis,
    config_path: &Path,
    hint: &str,
) -> String {
    let indented: String = prompt
        .lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[OFFLINE] {hint}\n\
         （本块由本地规则引擎生成，未调用任何模型：不包含也不伪造模型回答。）\n\
         \n\
         输入:\n{indented}\n\
         \n\
         自动编排判定（纯本地规则，不联网）:\n\
         \x20   形态: {} ({:?})\n\
         \x20   意图: {} ({:?})\n\
         \x20   复杂度: {:?}\n\
         \x20   需 LLM 消歧: {}\n\
         \x20   子任务: {} 个\n\
         \x20   判定依据: {}\n\
         \n\
         配置文件: {}\n\
         配置完成后重跑同一指令即可获得真实模型回答。",
        analysis.strategy.label(),
        analysis.strategy,
        analysis.intent.task_label(),
        analysis.intent,
        analysis.complexity,
        analysis.needs_llm,
        analysis.subtasks.len(),
        analysis.rationale,
        config_path.display()
    )
}

/// 离线回显的结构化载荷（`--json` 下由 [`print_json_ok`] 包成 `{ok,data,meta}`）。
fn offline_echo_payload(
    prompt: &str,
    analysis: &TaskAnalysis,
    config_path: &Path,
) -> serde_json::Value {
    serde_json::json!({
        "offline": true,
        "response": render_offline_echo(prompt, analysis, config_path),
        "prompt": prompt,
        "analysis": {
            "strategy": analysis.strategy,
            "strategy_label": analysis.strategy.label(),
            "intent": analysis.intent,
            "intent_label": analysis.intent.task_label(),
            "complexity": analysis.complexity,
            "needs_llm": analysis.needs_llm,
            "subtask_count": analysis.subtasks.len(),
            "rationale": analysis.rationale,
        },
        "config_path": config_path.display().to_string(),
        "hint": OFFLINE_ECHO_HINT,
    })
}

/// 打印离线回显并**正常返回**（退出码 0：这是正常的降级路径，不是错误）。
///
/// 编排判定走 [`Orchestrator::analyze`]（纯规则、零 LLM，可在无密钥 / 无网络环境运行），
/// 与交互式工作台的离线语义一致：形态只由引擎自动得出，不提供人工选择入口。
fn print_offline_echo(args: &Args, config: &Config, raw_prompt: &str) -> Result<()> {
    print_offline_echo_with_hint(args, config, raw_prompt, OFFLINE_ECHO_HINT)
}

/// 打印离线回显（可指定首行提示）：P0-1d 的「鉴权失败」降级复用同一渲染路径，
/// 只把首行换成 [`OFFLINE_ECHO_AUTH_HINT`]，其余（编排判定 / 配置指引 / 未调用模型声明）不变。
fn print_offline_echo_with_hint(
    args: &Args,
    config: &Config,
    raw_prompt: &str,
    hint: &str,
) -> Result<()> {
    let analysis = Orchestrator::new(config.clone()).analyze(raw_prompt);
    let config_path = Config::config_path();
    if args.json {
        print_json_ok(offline_echo_payload_with_hint(
            raw_prompt,
            &analysis,
            &config_path,
            hint,
        ));
    } else {
        println!(
            "{}",
            render_offline_echo_with_hint(raw_prompt, &analysis, &config_path, hint)
        );
    }
    Ok(())
}

/// 离线回显 JSON 载荷（可指定提示行；与 [`print_offline_echo_with_hint`] 共用，
/// 抽出纯函数便于护栏测试直接断言 `--json` 契约，无需真实打印）。
fn offline_echo_payload_with_hint(
    prompt: &str,
    analysis: &TaskAnalysis,
    config_path: &Path,
    hint: &str,
) -> serde_json::Value {
    let mut payload = offline_echo_payload(prompt, analysis, config_path);
    // 提示行随降级原因变化：JSON 契约里的 response/hint 都同步（避免两处文案漂移）。
    payload["hint"] = serde_json::json!(hint);
    payload["response"] =
        serde_json::json!(render_offline_echo_with_hint(prompt, analysis, config_path, hint));
    payload
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}

// 子命令处理函数

/// CLI 版本（取自 crate 包版本），供 `--version` 与帮助页使用
pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 打印顶层帮助索引（AC-01）
///
/// 列出全部资源（子命令）的一句话说明与可直接运行的示例，
/// 不依赖任何运行期状态（无 emoji、无 ANSI 色，便于被 agent 抓取）。
pub fn print_top_help() {
    print!("{}", render_top_help());
}

/// 渲染顶层帮助索引为字符串（纯函数，供 [`print_top_help`] 与单测共用）。
///
/// **不变量**：「资源（子命令）」区块必须与 [`SUBCOMMANDS`]（即路由注册表）
/// 逐项、按序一致；由 `test_top_help_lists_every_subcommand` 机械守卫，
/// 防止帮助清单再次漂移（曾漏列 sandbox / subagent / team / browser / capabilities）。
fn render_top_help() -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "Marguerite — 终端智能体工作台 (v{CLI_VERSION})");
    let _ = writeln!(out);
    let _ = writeln!(out, "用法:");
    let _ = writeln!(out, "  marguerite                        启动交互式工作台（默认）");
    let _ = writeln!(out, "  marguerite -p \"<指令>\"           单指令模式，执行后退出");
    let _ = writeln!(out, "  marguerite --pipe                从标准输入读取指令并执行后退出");
    let _ = writeln!(out, "  marguerite <资源> <动作>          运行子命令");
    let _ = writeln!(out, "  marguerite --help | -h           显示本帮助");
    let _ = writeln!(out, "  marguerite --version | -V        显示版本");
    let _ = writeln!(out);
    let _ = writeln!(out, "资源（子命令）:");
    let _ = writeln!(out, "  doctor    系统诊断与环境自检");
    let _ = writeln!(out, "  model     模型档案管理（list/current/use/copilot/add/remove/path）");
    let _ = writeln!(out, "  mcp       MCP 服务器管理（list）");
    let _ = writeln!(out, "  skills    技能管理（list/install）");
    let _ = writeln!(out, "  session   会话管理（list/delete）");
    let _ = writeln!(out, "  exec      沙箱命令执行（exec -- <命令>）");
    let _ = writeln!(out, "  deploy    一键部署（SSH / 宝塔，--target ...）");
    let _ = writeln!(out, "  serve     ACP 回环服务（serve --health | serve [--port N]）");
    let _ = writeln!(
        out,
        "  sandbox   沙箱执行与状态（sandbox exec -- <命令> / sandbox workspace）"
    );
    let _ = writeln!(
        out,
        "  subagent  子代理团队 DAG 编排（subagent team \"<目标>\" [--reflect] [--trellis]）"
    );
    let _ = writeln!(out, "  team      专家团队并行 spawn（team <目标描述>）");
    let _ = writeln!(
        out,
        "  browser   真实浏览器 CDP 操控（browser open <url> [--out <file.png>]）"
    );
    let _ = writeln!(out, "  chat      对话（chat 进流式 REPL；chat \"问题\" 单发流式）");
    let _ = writeln!(
        out,
        "  gateway   统一网关（gateway --health | gateway [--port N]，回环 Web 控制台）"
    );
    let _ = writeln!(out, "  goal      目标循环（goal create/get/update：自治续轮与 blocked 门控）");
    let _ = writeln!(
        out,
        "  capabilities 能力自检（真实落地状态；capabilities [组名] [--json]）"
    );
    let _ = writeln!(out, "  cache     KV 缓存命中率与可缓存比（cache stats / cache ratio）");
    let _ = writeln!(
        out,
        "  dispatch  无头分发任务到 11 个外部 agent CLI（meta-agent 一体内置，list / run / many）"
    );
    let _ = writeln!(out, "  games     内置小游戏（games list / games play <id> <动作...>）");
    let _ = writeln!(
        out,
        "  export-diagnostics  导出脱敏诊断包（tar；export-diagnostics [--output <路径>] [--json]）"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "示例:");
    let _ = writeln!(out, "  marguerite -p \"解释当前目录的 main.rs\"");
    let _ = writeln!(out, "  marguerite model list");
    let _ = writeln!(out, "  marguerite session list");
    let _ = writeln!(out, "  marguerite deploy --target ssh://user@host --dry-run");
    let _ = writeln!(out);
    let _ = writeln!(out, "说明:");
    let _ = writeln!(
        out,
        "  - 工作台由编排内核自动决定 Solo / MAX / 团队 / 集群 形态，无需手动选择。"
    );
    let _ = writeln!(out, "  - 未配置模型密钥时不再报错：自动降级为离线回显（本地规则编排判定 + 输入回显，不调用模型，退出码 0）。");
    let _ = writeln!(out, "  - 设置 MARGUERITE_LEGACY_UI=1 可回退到旧版单栏界面（仅排障）。");
    out
}

/// 从顶层帮助文本中解析「资源（子命令）」区块列出的子命令名（按出现顺序）。
///
/// 供 `test_top_help_lists_every_subcommand` 做机械对账：区块以
/// `资源（子命令）:` 起始，逐行 `  <名字>  <说明>`，遇空行结束。
#[cfg(test)]
fn parse_help_subcommand_names(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_section = false;
    for line in help.lines() {
        if !in_section {
            if line.starts_with("资源（子命令）") {
                in_section = true;
            }
            continue;
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("  ") {
            if let Some(name) = rest.split_whitespace().next() {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// 打印单个子命令的帮助（AC-01 子命令级）
///
/// 单一可信源：每个子命令的用法 + 至少两个可直接运行的示例。
/// 供 `marguerite <子命令> --help` 与无参 `marguerite <子命令>`（叶子动作命令除外）共用，
/// 避免帮助文本在各 handler 的 `_` 分支里漂移。
pub fn print_subcommand_help(name: &str) {
    let text = match name {
        "doctor" => {
            "\
Marguerite doctor — 系统诊断与环境自检

用法:
  marguerite doctor           运行诊断，输出环境摘要（文本；默认离线、不发起联网鉴权）
  marguerite doctor --probe   追加联网鉴权探测：GET <base_url>/models 判定密钥真伪
  marguerite doctor --json    输出可复制的 JSON 报告（版本号 + 7 项 OK/MISSING/DEGRADED + 修复动作）

说明:
  api_key 项只判断「是否配置」（存在性），不判断有效性；真伪判定由 --probe 的
  auth 项负责（2xx=有效 / 401,403=被拒 / 未加 --probe 时该项为 [SKIP]）。
  --probe 仅作用于文本模式；--json 报告的鉴权结论以 doctor 模块为准。

示例:
  marguerite doctor
  marguerite doctor --probe   # 联网校验密钥是否真的可用（会拒绝 401 的假 OK）
  marguerite doctor --json    # 完整版含版本号，离线可传给支持方
"
        }
        "model" => {
            "\
Marguerite model — 模型档案管理（Phase H2）

用法:
  marguerite model list                                   列出全部档案
  marguerite model current                                显示当前主 / 副模型
  marguerite model use <id>                               切换主模型
  marguerite model copilot <id|off>                       设置 / 关闭 Bloom 副模型
  marguerite model add --id <id> --base-url <url> \\
      --model <name> [--provider <p>] [--label <l>] \\
      [--api-key <k>] [--kind cloud|local|custom]         新增自定义接入点
  marguerite model remove <id>                            删除档案（至少保留一个）
  marguerite model path                                   显示 models.json 路径

示例:
  marguerite model list
  marguerite model use deepseek-v4-pro
"
        }
        "mcp" => {
            "\
Marguerite mcp — MCP 工具服务器管理

用法:
  marguerite mcp list     列出 config.yaml 中已配置的 MCP 服务器
  marguerite mcp tools    实时连接带 url 的 HTTP MCP 服务器并拉取工具目录

示例:
  marguerite mcp list
  marguerite mcp tools
  marguerite mcp list | cat   # 在脚本中读取服务器清单
"
        }
        "skills" => {
            "\
Marguerite skills — 技能管理（支持等级 rank / 批量注册，3.8 分级注入）

用法:
  marguerite skills list                     列出已安装技能（显示 rank 等级）
  marguerite skills install <yaml-url>       从 YAML URL 安装单个技能
  marguerite skills register-batch <dir>     按 rank 拓扑排序批量注册目录下技能文件（*.yaml）
  marguerite skills ranks                    显示五个等级：SYSTEM / PROJECT / DEFAULT / USER / COMMUNITY

说明:
  register-batch 遵循技能分级（3.8 分层注入）：低 rank（高优先级）先注册，
  依赖自动按拓扑排序，重复技能幂等跳过。

示例:
  marguerite skills list
  marguerite skills ranks
  marguerite skills install https://example.com/skill.yaml
  marguerite skills register-batch ./skills
"
        }
        "dispatch" => {
            r#"Marguerite dispatch — 无头分发任务到 11 个外部 agent CLI（一体内置，3.4 meta-agent 调度）

用法:
  marguerite dispatch list                                   列出 11 个目标（Codex / MCP-Codex / Sweep ...）
  marguerite dispatch run <target> "<task>" [工作目录]      单目标无头运行（统一超时 + 结构化结果）
  marguerite dispatch many "<task>" [工作目录]              11 路并发扇出（默认并发 11，可 --parallel N）
  marguerite dispatch run <target> "<task>" --timeout 60    单目标自定义超时（秒）

说明:
  把策划 §3.4「11 路无头外部 agent CLI」变为 marguerite 自身体内的能力：
  命令统一构造、环境变量隔离、CI+NO_COLOR 无头、超时 kill、输出 stdout/stderr 合并、
  dispatch_start / dispatch_end 事件经由 EventBus 发布（与 trellis / 编排器事件回路一致）。

示例:
  marguerite dispatch list
  marguerite dispatch run codex "为项目生成一个登录页" ./my-app
  marguerite dispatch run opencode "列出文件" . --timeout 30 --json
  marguerite dispatch many "为我的 README.md 提出三条改进" ./my-repo --parallel 6
"#
        }
        "subagent" => {
            "\
Marguerite subagent — 子代理团队 DAG 编排（trellis 事件驱动等待可选开启）

用法:
  marguerite subagent team \"<目标描述>\" [--reflect] [--trellis]

说明:
  走 DAG 分层编排（调研→规划→实现→审校），子代理真实执行。
  --reflect    启用分阶段反思（s2_5）：实现后插入自检再交付。
  --trellis    以事件驱动的 trellis_wait 代替忙轮询：订阅 EventBus 的
               subagent_end 事件流，按 id 集合收敛；默认关闭（向后兼容）。

示例:
  marguerite subagent team \"实现登录+JWT\" --trellis
  marguerite subagent team \"设计数据库 schema\" --reflect --trellis
"
        }
        "capabilities" => {
            "\
Marguerite capabilities — 能力自检（真实落地状态）

用法:
  marguerite capabilities                列出全量能力，按组分组并标注成熟度
  marguerite capabilities <group>        仅列出某组（kebab 名，如 tooling / security）
  marguerite capabilities --json         以 JSON 输出（便于脚本 / agent 接入）

示例:
  marguerite capabilities
  marguerite capabilities tooling
  marguerite capabilities --json | jq '.global'
"
        }
        "cache" => {
            "\
Marguerite cache — KV 缓存命中率与可缓存比

用法:
  marguerite cache stats                报告已落盘前缀缓存（prefix_cache.json）的命中率统计
  marguerite cache ratio [session-id]   计算某会话（默认最近）的稳定前缀可缓存比
  marguerite cache stats --json         以 JSON 输出（便于脚本 / agent 接入）

示例:
  marguerite cache stats
  marguerite cache ratio
  marguerite cache ratio <session-id>
"
        }
        "games" => {
            "\
Marguerite games — 内置小游戏（Rust 引擎，离线可玩）

用法:
  marguerite games list                  列出已实装小游戏
  marguerite games play <id> <动作...>   按动作驱动游戏引擎并渲染棋盘/结果
  marguerite games list --json           以 JSON 输出（便于脚本 / agent 接入）

示例:
  marguerite games list
  marguerite games play tictactoe 0 3 1 4 2
  marguerite games play 2048 left up left
  marguerite games play snake up left down
"
        }
        "export-diagnostics" => {
            "\
Marguerite export-diagnostics — 导出脱敏诊断包（可附到 bug 报告）

用法:
  marguerite export-diagnostics                      导出到 <数据目录>/diagnostics/*.tar
  marguerite export-diagnostics --output <路径>      指定输出路径（.tar 结尾按文件处理，否则按目录）
  marguerite export-diagnostics --json               以 JSON 输出清单（便于脚本 / agent 接入）

说明:
  收集：版本与环境摘要、config.yaml、侧车 sidecar.json（含上次退出证据）、
        侧车 stdout/stderr 日志（含轮转历史代）。
  所有文本必经统一脱敏 scrub_text（Bearer / key=value 赋值 / 裸令牌 /
  URL 内嵌凭据 / 家目录），占位符为 [redacted] 与 [USER]。
  硬排除：~/.marguerite/.env、desktop-credential.json、*-raw.jsonl、
         security-audit/skill_block.jsonl（后者存的是用户输入原文片段）。
  单个条目收集失败只记入清单的 collection_errors，不影响其余条目导出。
  纯只读、离线：不修改配置、不联网上报。

示例:
  marguerite export-diagnostics
  marguerite export-diagnostics --output D:/tmp/diag.tar
  marguerite export-diagnostics --json
"
        }
        "goal" => {
            r#"Marguerite goal — 目标循环（自治续轮 + blocked 门控）

用法:
  marguerite goal create <目标描述>            登记唯一进行中的完成目标（armed）
  marguerite goal get                         读取当前目标与激活态
  marguerite goal update edit <新目标>         编辑目标描述（revision+1）
  marguerite goal update pause                暂停
  marguerite goal update resume               恢复并重新 armed
  marguerite goal update complete             标记完成
  marguerite goal update blocked <原因>        报告阻塞（需先经过 3 轮续轮才置为 blocked）

示例:
  marguerite goal create 重构部署流水线
  marguerite goal get --json
  marguerite goal update complete
"#
        }
        "session" => {
            "\
Marguerite session — 会话管理

用法:
  marguerite session list           列出最近会话
  marguerite session delete <id>    删除指定会话

示例:
  marguerite session list
  marguerite session delete <前 8 位 ID>
"
        }
        "exec" => {
            "\
Marguerite exec — 终端沙箱命令执行

用法:
  marguerite exec -- <命令> [参数...]    经终端沙箱门控后执行（写命令在非交互 CLI 下默认拒绝）

示例:
  marguerite exec -- ls -la
  marguerite exec -- git status
"
        }
        "deploy" => {
            "\
Marguerite deploy — 一键部署（Phase I2/I3）

用法:
  SSH : marguerite deploy --target ssh://[user@]host[:port][/dir] \\
        [--bin <release二进制>] [--service <name>] [--health-url <url>] \\
        [--no-systemd] [--dry-run] [--user <u>] [--port <p>] [--identity <key>]
  宝塔: marguerite deploy --target baota://<别名> [--bin <二进制>] [--site <域名>] \\
        [--service <name>] [--health-url <url>] [--no-ssl] [--dry-run]
  注册: marguerite deploy --target baota://<别名> --add-panel \\
        --endpoint https://host:8888 --api-key <面板API密钥>

示例:
  marguerite deploy --target ssh://user@host:22/opt/marguerite --dry-run
  marguerite deploy --target baota://mysite --add-panel --endpoint https://host:8888 --api-key $KEY
"
        }
        "serve" => {
            "\
Marguerite serve — ACP 回环服务（仅 127.0.0.1，绝不暴露公网）

用法:
  marguerite serve --health             探测并返回 JSON 健康态（不启动长驻服务）
  marguerite serve [--port <N>]         在 127.0.0.1:<N> 启动回环 ACP listener（默认 18181）
                                      经 JSON-RPC 暴露 ping / session.info / agent.invoke / health
                                      agent.invoke 真实路由到编排器离线分析（不调 LLM）

示例:
  marguerite serve --health
  marguerite serve --health | jq .status
  marguerite serve --port 19000
"
        }
        "gateway" => {
            "\
Marguerite gateway — 统一网关（回环 Web 控制台，仅 127.0.0.1，绝不暴露公网）

用法:
  marguerite gateway --health             探测并返回 JSON 健康态（不启动长驻服务）
  marguerite gateway [--port <N>]         在 127.0.0.1:<N> 启动统一网关（默认 18790）
                                          同源托管 webui/dist + /ws JSON-RPC + 本地 token 认证

环境:
  MARGUERITE_WEBUI_DIST   指定 Web UI 产物目录（默认自动探测 webui/dist 或 ../webui/dist）

连接:
  浏览器打开 http://127.0.0.1:<port>/control/ 并携带 ?token=<gateway-token> 完成认证；
  token 存于 ~/.marguerite/gateway-token（首次启动自动生成）。

示例:
  marguerite gateway --health
  marguerite gateway
  marguerite gateway --port 18790
"
        }
        "chat" => {
            "\
Marguerite chat — 流式对话（标准 REPL 风格纯终端 REPL）

用法:
  marguerite chat                  进入交互式流式 REPL（Ctrl-D / /exit 退出）
  marguerite chat \"<问题>\"         单发：流式打印一个回答后退出（等价于 -p，但走流）
  marguerite chat --json \"<问题>\"  单发并以 JSON 返回 {ok, data:{response}, meta}

会话选项:
  -c, --continue                   恢复最近一次会话继续对话
      --resume <session-id>        恢复指定会话（支持 session list 里的短 id）
  -t, --tools                      启用 LLM function calling 工具回路（内置 20 个原生工具）

REPL 内斜杠命令:
  /help  /history  /clear  /save  /sessions  /load <id>  /model  /exit

REPL 每轮对话自动落盘，与 `marguerite session list/delete` 共用同一存储。
裸单发不产生会话记录；带 -c/--resume 的单发会接续上下文并落盘。
无 API Key 时离线降级。
--tools 模式下模型可在对话中调用文件/Shell/检索等内置工具，工具调用过程会实时打印。

示例:
  marguerite chat
  marguerite chat -c
  marguerite chat --resume 3f2a1b9c
  marguerite chat --tools
  marguerite chat --tools 读取 src/main.rs 并加超时
  marguerite chat \"用 Rust 写一个带超时的 TCP 探测\"
  marguerite chat -c \"接着上面，补充错误处理\"
  marguerite chat --json \"一句话解释 tokio runtime\" | jq .data.response
"
        }
        "team" => {
            "\
Marguerite team — 专家团队并行 spawn（Team::run 真实执行体）

用法:
  marguerite team <目标描述>           自动角色检测 → 专家团队并行 spawn → lead 综合

说明:
  区别于 DAG 分层版 marguerite subagent team：本命令直接走专家团队原语
  （Team::run），按目标自动检测专精角色（设计 / 编码 / 测试 / 文档 / 安全 / 调研…），
  并行 spawn 各角色成员、收集、聚合，再由 lead 综合为最终答复。
  未检测到专精角色时退化为单成员通用团队，至少触发一次真实 spawn。
  经治理门前置裁决，破坏性 / 越权操作需人工放行；LLM 不可达时成员失败如实可见，不冒充成功。

示例:
  marguerite team 为登录功能设计接口契约、实现代码、并编写单元测试
"
        }
        _ => {
            eprintln!("错误: 未知子命令 '{name}'");
            std::process::exit(2)
        }
    };
    print!("{text}");
}

/// doctor — 系统诊断（结构化校验：config_dir / schema / api_key / network / sandbox / engine）
///
/// 验收（Phase 5 / AC）：When 运行 `marguerite doctor`，系统**必须**校验 config schema / key /
/// 网络并报告；If `--json`，系统**必须**输出 `{ok, data:{checks, summary, status}, meta}`。
fn doctor_handler(args: &Args, config: &Config) -> Result<()> {
    // M5 doctor：`--json` 直接委托 doctor 模块（版本号 + 7 项 OK/MISSING/DEGRADED + 修复动作）。
    // 前置短路上探针：避免构建 legacy checks 触发日志噪音，保证 JSON 可干净复制（离线传给支持方）。
    // 注意：`--probe` 的联网鉴权探测只作用于文本路径（JSON 路径由 doctor 模块负责），
    // 显式告知用户而非静默忽略。
    if args.json {
        if args.probe {
            eprintln!(
                "提示: --probe 仅作用于文本模式；--json 报告的鉴权结论以 doctor 模块为准（本次未做联网鉴权探测）。"
            );
        }
        let report = crate::doctor::run_doctor_with_config(config);
        println!("{report}");
        return Ok(());
    }

    let config_dir = Config::config_dir();

    // 结构化自检项：每项含 名称 / 结果状态 / 详情 / 修复建议
    let mut checks: Vec<DoctorCheck> = Vec::new();

    // 1. 配置目录
    let dir_ok = config_dir.exists();
    checks.push(DoctorCheck {
        name: "config_dir",
        state: if dir_ok {
            CheckState::Ok
        } else {
            CheckState::Fail
        },
        detail: config_dir.display().to_string(),
        hint: if dir_ok {
            String::new()
        } else {
            "首次运行 marguerite 会自动创建配置目录".to_string()
        },
    });

    // 2. 配置 schema
    let schema = validate_config_schema(config);
    checks.push(schema);

    // 3. API Key（仅存在性：**不**校验有效性，避免「有 key 即 OK」的假阳性）
    checks.push(api_key_check(config));

    // 3b. 密钥有效性（**仅**在显式 --probe/--deep 时联网；默认路径离线跳过）
    checks.push(auth_probe_check(config, args.probe));

    // 4. 网络可达性（base_url 探测，短超时，离线安全）
    let net = probe_model_endpoint(config);
    checks.push(net);

    // 5. 沙箱（平台支持性，信息项，恒 ok）
    let sandbox_detail = match std::env::consts::OS {
        "linux" => "Landlock 已支持",
        "macos" => "Seatbelt 已支持",
        _ => "当前平台只读降级（Unsupported，符合安全约束）",
    };
    checks.push(DoctorCheck {
        name: "sandbox",
        state: CheckState::Ok,
        detail: sandbox_detail.to_string(),
        hint: String::new(),
    });

    // 6. 引擎就绪
    let engine = Orchestrator::new(config.clone());
    let engine_ok = engine.tool_count() > 0;
    checks.push(DoctorCheck {
        name: "engine",
        state: if engine_ok {
            CheckState::Ok
        } else {
            CheckState::Fail
        },
        detail: format!(
            "tools={} research_docs={} automations={}",
            engine.tool_count(),
            engine.research_doc_count(),
            engine.list_automations().len()
        ),
        hint: if engine_ok {
            String::new()
        } else {
            "引擎未注册任何工具，请检查构建特性".to_string()
        },
    });

    // 7. 技能子系统：可加载即健康；数量作为信息并给出落地提示（非致命）。
    //    解析统一走 resolve_skills_dir，使打包/多 cwd 场景下技能可被发现。
    let skills_dir = crate::tools::skill_tools::resolve_skills_dir(&config.skills.skills_dir);
    let skills_load = crate::skills::loader::SkillLoader::new(&skills_dir).load_all();
    let skills_count = skills_load.as_ref().map_or(0, Vec::len);
    let skills_ok = skills_load.is_ok();
    checks.push(DoctorCheck {
        name: "skills",
        state: if skills_ok {
            CheckState::Ok
        } else {
            CheckState::Fail
        },
        detail: format!("dir={} skills={}", skills_dir.display(), skills_count),
        hint: if skills_count == 0 {
            format!(
                "技能目录为空，可运行 'marguerite skills install <yaml-url>' 或将 YAML 放入 {}",
                skills_dir.display()
            )
        } else {
            String::new()
        },
    });

    // 8. 能力清单健康度（已实装 / 纯叙事 / 总规模）
    let cap_stats = crate::domain::capabilities::stats();
    checks.push(DoctorCheck {
        name: "capabilities",
        state: CheckState::Ok,
        detail: format!(
            "real={} partial={} implemented={} narrative={} total={}",
            cap_stats.real,
            cap_stats.partial,
            cap_stats.implemented,
            cap_stats.narrative,
            cap_stats.total
        ),
        hint: String::new(),
    });

    // 9. 重型运行时（ASR / 本地量化模型）：诚实占位 —— 未预下载，给下载源 + 选型指引。
    let heavy_runtimes = crate::runtime::heavy_runtime_status();
    for rt in &heavy_runtimes {
        checks.push(DoctorCheck {
            name: match rt.kind {
                "asr" => "audio_runtime",
                "local_llm" => "local_model_runtime",
                other => other,
            },
            state: CheckState::Ok,
            detail: format!("{}={} engine={}", rt.kind, rt.status, rt.engine),
            hint: format!(
                "下载源：{}
  选型：{}",
                rt.download, rt.guidance
            ),
        });
    }

    let total = checks.len();
    let passed = checks
        .iter()
        .filter(|c| c.state == CheckState::Ok)
        .count();
    let failed = checks
        .iter()
        .filter(|c| c.state == CheckState::Fail)
        .count();
    let skipped = checks
        .iter()
        .filter(|c| c.state == CheckState::Skip)
        .count();
    let status = if failed == 0 { "ok" } else { "degraded" };
    let judged = total - skipped;

    // 文本输出（ASCII 状态标记，利 NO_COLOR / 非 TTY）
    println!("Marguerite 系统诊断");
    println!(
        "  Rust: {} | OS: {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    println!("  配置目录: {}", config_dir.display());
    println!(
        "  模型: {} ({})",
        config.model.model_name, config.model.provider
    );
    println!("  MCP 服务器: {}", config.mcp.servers.len());
    println!();
    println!("自检项:");
    for c in &checks {
        println!("  {} {:<14} {}", c.state.tag(), c.name, c.detail);
        if !c.hint.is_empty() {
            if c.state == CheckState::Fail {
                println!("        修复: {}", c.hint);
            } else {
                println!("        提示: {}", c.hint);
            }
        }
    }
    println!();
    if skipped == 0 {
        println!("结果: {passed}/{total} 通过 -> {status}");
    } else {
        println!("结果: {passed}/{judged} 通过（{skipped} 项未探测）-> {status}");
    }
    Ok(())
}

/// 单条诊断检查的结果状态
///
/// `Skip` 用于「本次未探测」的项（如未加 `--probe` 时的鉴权探测）：
/// 既不计通过、也不计失败，避免把「没查」粉饰成 `[OK]`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CheckState {
    /// 通过
    Ok,
    /// 明确失败
    Fail,
    /// 未探测（默认离线路径）
    Skip,
}

impl CheckState {
    /// ASCII 状态标记（遵循 CLI 输出契约：无 emoji、尊重 NO_COLOR）
    fn tag(self) -> &'static str {
        match self {
            CheckState::Ok => "[OK]",
            CheckState::Fail => "[FAIL]",
            CheckState::Skip => "[SKIP]",
        }
    }
}

/// 单条诊断检查的结果
struct DoctorCheck {
    /// 检查项名称
    name: &'static str,
    /// 结果状态
    state: CheckState,
    /// 详情
    detail: String,
    /// 未通过时的修复建议（为空表示无需修复）
    hint: String,
}

/// 校验配置 schema 合法性（必填字段 / 枚举 / 子结构完整性）
fn validate_config_schema(config: &Config) -> DoctorCheck {
    let mut problems: Vec<String> = Vec::new();
    if config.model.model_name.trim().is_empty() {
        problems.push("model.model_name 为空".to_string());
    }
    if config.model.provider.trim().is_empty() {
        problems.push("model.provider 为空".to_string());
    }
    for s in &config.mcp.servers {
        if s.name.trim().is_empty() || s.command.trim().is_empty() {
            problems.push(format!("mcp 服务器 {} 缺少 name 或 command", s.name));
        }
    }
    if problems.is_empty() {
        DoctorCheck {
            name: "config_schema",
            state: CheckState::Ok,
            detail: "schema 合法".to_string(),
            hint: String::new(),
        }
    } else {
        DoctorCheck {
            name: "config_schema",
            state: CheckState::Fail,
            detail: problems.join("; "),
            hint: "请检查 config.yaml 的对应字段".to_string(),
        }
    }
}

/// 探测模型 base_url 的网络可达性（短超时、离线安全）
fn probe_model_endpoint(config: &Config) -> DoctorCheck {
    let base = config.model.base_url.trim();
    if base.is_empty() {
        return DoctorCheck {
            name: "network",
            state: CheckState::Ok,
            detail: "未配置 base_url，跳过网络探测".to_string(),
            hint: String::new(),
        };
    }
    let (host, port) = match parse_host_port(base) {
        Some(hp) => hp,
        None => {
            return DoctorCheck {
                name: "network",
                state: CheckState::Fail,
                detail: format!("无法解析 base_url: {base}"),
                hint: "请检查 config.yaml 的 model.base_url".to_string(),
            }
        }
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            return DoctorCheck {
                name: "network",
                state: CheckState::Fail,
                detail: format!("无法创建运行时: {e}"),
                hint: String::new(),
            }
        }
    };
    let result = rt.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::net::TcpStream::connect((host.as_str(), port)),
        )
        .await
    });
    match result {
        Ok(Ok(_)) => DoctorCheck {
            name: "network",
            state: CheckState::Ok,
            detail: format!("TCP 可达 {host}:{port}（仅端口连通，不代表鉴权有效）"),
            hint: String::new(),
        },
        Ok(Err(e)) => DoctorCheck {
            name: "network",
            state: CheckState::Fail,
            detail: format!("TCP 不可达 {host}:{port}: {e}"),
            hint: "检查网络 / 代理 / base_url 端口".to_string(),
        },
        Err(_) => DoctorCheck {
            name: "network",
            state: CheckState::Fail,
            detail: format!("探测超时 {host}:{port} (3s)"),
            hint: "检查网络 / 代理 / 防火墙".to_string(),
        },
    }
}

/// `api_key` 检查项：**只**判定「存在性」。
///
/// 存在 ≠ 有效：实测同一台机器 doctor 报「已配置」后立刻 `-p` 得到 401。
/// 因此本项详情里明确写出「未校验有效性」，并把真伪判定交给
/// [`auth_probe_check`]（联网、需显式 `--probe`）。
fn api_key_check(config: &Config) -> DoctorCheck {
    // 纯空白视为未配置：与离线回显闸门 `needs_offline_echo` 同一判据。
    let key_ok = !config.model.api_key.trim().is_empty();
    DoctorCheck {
        name: "api_key",
        state: if key_ok {
            CheckState::Ok
        } else {
            CheckState::Fail
        },
        detail: if key_ok {
            "已配置（仅存在性检查，未校验有效性）".to_string()
        } else {
            "未配置".to_string()
        },
        hint: if key_ok {
            "本项不校验密钥真伪：运行 'marguerite doctor --probe' 联网校验（默认离线不联网）"
                .to_string()
        } else {
            "设置环境变量 MARGUERITE_API_KEY 或在 config.yaml 的 model.api_key 中填写".to_string()
        },
    }
}

/// 密钥**有效性**探测（`doctor --probe` / `--deep`）。
///
/// 设计取舍（P0-D 缺陷 3）：`api_key` 项只能证明「存在」，证明不了「有效」——
/// 实测同一台机器 `[OK] api_key 已配置` 后立刻 `-p` 得到 401。真正的真伪判定
/// 必须发起一次带鉴权的只读请求，而 doctor 的默认契约是**离线 + 快速**，
/// 因此这里把它做成显式开关：
/// - 未加 `--probe`：`[SKIP]`（既不计通过也不计失败，并打印如何开启）；
/// - 加 `--probe`：`GET {base_url}/models`（OpenAI 兼容只读端点），
///   2xx = 密钥有效，401/403 = 密钥被拒，其它状态/网络失败 = 无法判定（FAIL）。
///   任何分支都不打印密钥明文。
fn auth_probe_check(config: &Config, probe: bool) -> DoctorCheck {
    use crate::doctor::auth_probe::{probe_auth, AuthProbeOutcome};

    if !probe {
        return DoctorCheck {
            name: "auth",
            state: CheckState::Skip,
            detail: "未探测（默认离线，不发起联网鉴权请求）".to_string(),
            hint: "运行 'marguerite doctor --probe' 联网校验密钥有效性".to_string(),
        };
    }

    let base = config.model.base_url.trim();
    let url = format!("{}/models", base.trim_end_matches('/'));
    // 与 RPC `doctor.status{probe:true}` 同源：唯一的探测实现在 doctor::auth_probe。
    match probe_auth(config, std::time::Duration::from_secs(8)) {
        AuthProbeOutcome::Valid => DoctorCheck {
            name: "auth",
            state: CheckState::Ok,
            detail: format!("密钥有效（GET {url} → HTTP 2xx）"),
            hint: String::new(),
        },
        AuthProbeOutcome::Rejected { status } => DoctorCheck {
            name: "auth",
            state: CheckState::Fail,
            detail: format!("密钥被拒绝（GET {url} → HTTP {status}，401/403 鉴权失败）"),
            hint: "密钥无效或已过期：运行 'marguerite model add ...' 重新配置，或检查 MARGUERITE_API_KEY"
                .to_string(),
        },
        AuthProbeOutcome::Indeterminate { reason } => DoctorCheck {
            name: "auth",
            state: CheckState::Fail,
            detail: format!("无法判定（GET {url} → {reason}）"),
            hint: "确认 base_url 是否为 OpenAI 兼容端点（应支持 GET /models）；检查网络 / 代理"
                .to_string(),
        },
        AuthProbeOutcome::Skipped { reason } => DoctorCheck {
            name: "auth",
            state: CheckState::Skip,
            detail: format!("{reason}，跳过鉴权探测"),
            hint: "先配置密钥：设置环境变量 MARGUERITE_API_KEY 或在 config.yaml 的 model.api_key 中填写"
                .to_string(),
        },
    }
}

/// 从 URL / 裸 host[:port] 解析出 (host, port)
fn parse_host_port(url: &str) -> Option<(String, u16)> {
    let s = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string();
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return Some((h.to_string(), port));
        }
    }
    if url.starts_with("https://") {
        Some((s, 443))
    } else if url.starts_with("http://") {
        Some((s, 80))
    } else {
        Some((s, 443))
    }
}

/// 记忆持久化目录。
///
/// 环境变量 `MARGUERITE_MEMORY_DIR` 优先；否则回退到当前工作目录下的
/// `.marguerite/memory/`，使 `serve` 跨会话沉淀 User/Feedback/Auto 记忆。
fn memory_dir() -> PathBuf {
    if let Ok(d) = std::env::var("MARGUERITE_MEMORY_DIR") {
        PathBuf::from(d)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".marguerite/memory")
    }
}

/// serve — ACP 回环服务（解决 G8，P2）
///
/// 验收（Phase 5 / AC）：If 运行 `marguerite serve --health`，系统**必须**返回 JSON 健康态；
/// 若 `marguerite serve [--port N]`（无 `--health`），系统**必须**在 127.0.0.1 启动回环 ACP
/// listener（仅回环，绝不暴露公网），经 JSON-RPC 暴露 ping / session.info / agent.invoke / health。
fn serve_handler(args: &Args, config: &Config) -> Result<()> {
    let port = args.port.unwrap_or(18181);

    if args.health {
        // 健康态探测：不启动长驻 listener，直接返回 JSON（AC 明确要求）
        let bindable = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
        let o = Arc::new(Orchestrator::new(config.clone()));
        let invoker: Arc<dyn AgentInvoker> = Arc::new(EngineInvoker::new(o.clone()));
        let session = AcpSession::new_full(
            "marguerite",
            invoker,
            Arc::new(build_acp_tool_registry()),
            Arc::new(KbBackedMemory::with_persistence(&memory_dir())),
        );
        let status = if bindable && !config.model.api_key.is_empty() {
            "ok"
        } else {
            "degraded"
        };
        print_json_ok(serde_json::json!({
            "status": status,
            "loopback_bindable": bindable,
            "port": port,
            "acp_methods": session.methods(),
            "engine": {
                "tools": o.tool_count(),
                "research_docs": o.research_doc_count(),
                "automations": o.list_automations().len(),
            },
            "config": {
                "model": config.model.model_name,
                "provider": config.model.provider,
                "api_key_configured": !config.model.api_key.is_empty(),
            },
        }));
        return Ok(());
    }

    // 启动真实回环 ACP listener（仅 127.0.0.1）
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let o = Arc::new(Orchestrator::new(config.clone()));
        let invoker: Arc<dyn AgentInvoker> = Arc::new(EngineInvoker::new(o));
        let session = Arc::new(AcpSession::new_full(
            "marguerite",
            invoker,
            Arc::new(build_acp_tool_registry()),
            Arc::new(KbBackedMemory::with_persistence(&memory_dir())),
        ));
        let handle = start_listener(session.clone(), port).await?;
        println!("Marguerite ACP 服务已启动（仅回环 127.0.0.1，绝不暴露公网）");
        println!("  地址  : {}", handle.addr);
        println!("  方法  : {}", session.methods().join(" / "));
        println!("  按 Ctrl+C 停止");
        // 永久阻塞于 accept 循环（listener 的 JoinHandle drop 即停）
        let _ = handle.handle.await;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

/// gateway — 统一网关（回环 Web 控制台）
///
/// 验收（融合 Phase 2）：`marguerite gateway --health` 返回 JSON 健康态（不启动长驻）；
/// `marguerite gateway [--port N]` 在 127.0.0.1:<N> 启动统一网关（默认 18790），
/// 同源托管 webui/dist + `/ws` JSON-RPC + 本地 token 认证，绝不暴露公网。
fn gateway_handler(args: &Args, config: &Config) -> Result<()> {
    let port = args.port.unwrap_or(DEFAULT_PORT);

    if args.health {
        // 健康态探测：不启动长驻服务，直接返回 JSON
        let bindable = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
        let webui = detect_webui_dist();
        let webui_found = webui.as_ref().is_some();
        let token_file = crate::persistence::config::Config::data_dir().join("gateway-token");
        print_json_ok(serde_json::json!({
            "status": if bindable { "ok" } else { "degraded" },
            "loopback_bindable": bindable,
            "port": port,
            "protocol": crate::gateway::protocol::PROTOCOL_VERSION,
            "webui": {
                "found": webui_found,
                "dist": webui.as_ref().map(|p| p.display().to_string()),
            },
            "token_file": token_file.display().to_string(),
            "token_present": token_file.is_file(),
        }));
        return Ok(());
    }

    // 启动真实回环统一网关（仅 127.0.0.1）
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let state = std::sync::Arc::new(GatewayState::new(config.clone(), None)?);
        crate::gateway::run(state, port).await?;
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(())
}

/// 探测 Web UI 产物目录（供 gateway 健康态与启动共用）
fn detect_webui_dist() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("MARGUERITE_WEBUI_DIST") {
        let p = std::path::PathBuf::from(dir);
        if p.join("index.html").is_file() {
            return Some(p);
        }
    }
    let cwd = std::env::current_dir().ok()?;
    [cwd.join("webui/dist"), cwd.join("../webui/dist")]
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// 构建注入 ACP 会话的工具注册表（复用 `register_builtin_tools`，21 个内置原生工具）。
///
/// 失败（如工作区不可读）降级为空注册表，ACP 退化为纯 agent 转发，不中断服务。
fn build_acp_tool_registry() -> ToolRegistry {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut reg = ToolRegistry::new();
    match register_builtin_tools(&mut reg, &workspace, None, None, None) {
        Ok(()) => reg,
        Err(e) => {
            eprintln!("[serve] 工具注册失败，ACP 将以纯 agent 模式运行：{e}");
            ToolRegistry::new()
        }
    }
}

/// model — 模型管理（Phase H2：注册表 + 即时切换 + 自定义 API 接入）
///
/// 子命令：
/// - `list`                    列出全部档案（标 * 主模型 / ◇ Bloom 副模型）
/// - `current`                 显示当前主模型与 Bloom 副模型
/// - `use <id>`                切换主模型
/// - `copilot <id|off>`        设置 / 关闭 Bloom 副模型
/// - `add --id --base-url --model [--provider --label --api-key --kind]`
///   新增自定义接入点（kind: cloud/local/custom）
/// - `remove <id>`             删除档案（至少保留一个）
/// - `reload-seeds`            热更新内置种子（读 models.json 同目录的 models.seed.json 覆盖层，不覆盖自定义档案）
/// - `path`                    显示 models.json 路径
fn model_handler(args: &Args, _config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    let mut sel = ModelSelector::load()?;
    match sub {
        "list" => {
            println!("模型档案（* 主模型  ◇ Bloom 副模型）：");
            for p in sel.registry().list() {
                let star = if p.id == sel.registry().active {
                    "*"
                } else {
                    " "
                };
                let diamond = if sel.registry().copilot.as_deref() == Some(&p.id) {
                    " ◇"
                } else {
                    ""
                };
                println!(
                    "  {} {:<22} {:<8} {:<8} {}{}",
                    star,
                    p.id,
                    p.provider,
                    format!("{:?}", p.kind),
                    p.model,
                    diamond
                );
                println!("      base_url: {}", p.base_url);
            }
        }
        "current" => {
            match sel.registry().active_profile() {
                Some(a) => println!("主模型: {} ({})", a.label, a.id),
                None => println!("主模型: （未设置）"),
            }
            match sel.registry().copilot_profile() {
                Some(c) => println!("Bloom 副模型: {} ({})", c.label, c.id),
                None => println!("Bloom 副模型: 关闭"),
            }
        }
        "use" => {
            let id = args
                .sub_args
                .get(1)
                .ok_or_else(|| anyhow!("用法: marguerite model use <id>"))?;
            sel.use_active(id)?;
            println!("已切换主模型 → {}", id);
        }
        "copilot" => {
            let arg = args.sub_args.get(1).map(String::as_str).unwrap_or("off");
            if arg == "off" {
                sel.use_copilot(None)?;
                println!("Bloom 副模型: 关闭");
            } else {
                sel.use_copilot(Some(arg.to_string()))?;
                println!("Bloom 副模型 → {}", arg);
            }
        }
        "add" => {
            let flags = parse_flags(&args.sub_args[1.min(args.sub_args.len())..]);
            let profile = profile_from_flags(&flags)?;
            sel.add(profile)?;
            println!("已新增模型档案: {}", flags.get("id").unwrap());
        }
        "remove" => {
            let id = args
                .sub_args
                .get(1)
                .ok_or_else(|| anyhow!("用法: marguerite model remove <id>"))?;
            sel.remove(id)?;
            println!("已删除模型档案: {}", id);
        }
        "path" => {
            println!("{}", models_path().display());
        }
        "reload-seeds" => {
            let n = sel.reload_seeds()?;
            if n == 0 {
                println!("未发现 models.seed.json 覆盖层，或未补充任何新档案（无操作）");
            } else {
                println!("已热更新 {n} 个新模型档案（来自 models.seed.json 覆盖层，未覆盖你的自定义档案）");
            }
        }
        // 三档模型分层（5c）：marguerite model tier [flagship|balanced|turbo|auto] [--effort off|normal|deep|max]
        "tier" => {
            use crate::model::thinking::ThinkingEffort;
            use crate::model::tier::{ModelTier, SessionState};
            let mut state = SessionState::load();
            // --effort <x>
            if let Some(pos) = args.sub_args.iter().position(|a| a == "--effort") {
                let e = args.sub_args.get(pos + 1).map(String::as_str).unwrap_or("normal");
                state.thinking_effort = ThinkingEffort::from_spec_str(e);
            }
            // 位置参数 = 锁定档位
            if let Some(t) = args.sub_args.get(1).map(String::as_str) {
                state.locked_tier = match t {
                    "auto" | "none" | "" => None,
                    other => Some(ModelTier::parse(other)),
                };
            }
            let _ = state.save();
            println!(
                "档位: locked={:?}  effort={:?}  effective={}  预算状态={} (剩余 {:.0}%)",
                state.locked_tier.map(|t| t.as_str()),
                state.thinking_effort.reasoning_effort(),
                state.effective_tier().as_str(),
                state.budget.state().as_str(),
                state.budget.remaining_ratio() * 100.0
            );
        }
        // 会话预算：marguerite model budget [--tokens <n>] [--usd <d>]
        "budget" => {
            use crate::model::tier::SessionState;
            let mut state = SessionState::load();
            let flags = parse_flags(&args.sub_args[1.min(args.sub_args.len())..]);
            if let Some(t) = flags.get("tokens").and_then(|v| v.parse::<u64>().ok()) {
                state.budget.tokens_cap = t;
            }
            if let Some(u) = flags.get("usd").and_then(|v| v.parse::<f64>().ok()) {
                state.budget.usd_cap = u;
            }
            let _ = state.save();
            println!(
                "会话预算: tokens={}/{}  usd=${:.2}/${:.2}  剩余={:.0}%  状态={}",
                state.budget.tokens_used,
                state.budget.tokens_cap,
                state.budget.usd_used,
                state.budget.usd_cap,
                state.budget.remaining_ratio() * 100.0,
                state.budget.state().as_str()
            );
        }
        _ => {
            print_subcommand_help("model");
        }
    }
    Ok(())
}

/// 解析 `--key value` 形式的 flag 列表（忽略位置参数）
fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(key) = a.strip_prefix("--") {
            if let Some(val) = args.get(i + 1) {
                if !val.starts_with("--") {
                    map.insert(key.to_string(), val.clone());
                    i += 2;
                    continue;
                }
            }
            // 仅有 flag 无值：记为空串
            map.insert(key.to_string(), String::new());
        }
        i += 1;
    }
    map
}

/// mcp — MCP 工具管理
fn mcp_handler(args: &Args, config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    match sub {
        "list" => {
            if config.mcp.servers.is_empty() {
                println!("未配置 MCP 服务器（在 config.yaml 的 mcp.servers 中添加）");
            } else {
                println!("已配置 MCP 服务器:");
                for server in &config.mcp.servers {
                    println!(
                        "  {} (命令: {} {})",
                        server.name,
                        server.command,
                        server.args.join(" ")
                    );
                }
            }
        }
        "tools" => {
            // 实时连接带 url 的 HTTP MCP 服务器并拉取工具目录（与编排器桥接同源）。
            let rt = tokio::runtime::Runtime::new()?;
            let mut connected = 0usize;
            for server in &config.mcp.servers {
                let Some(url) = server.url.as_deref().filter(|u| !u.trim().is_empty()) else {
                    continue;
                };
                let name = server.name.clone();
                let url_owned = url.trim().to_string();
                let result = rt.block_on(async {
                    let mut client = crate::mcp::client::McpClient::new(&url_owned);
                    client.connect().await?;
                    client.list_tools().await
                });
                match result {
                    Ok(tools) => {
                        connected += 1;
                        println!("[{}] {}（{} 个工具）", name, url_owned, tools.len());
                        for t in &tools {
                            println!("  - {} — {}", t.name, t.description);
                        }
                    }
                    Err(e) => eprintln!("[mcp] 连接/拉取 {name} ({url_owned}) 失败: {e}"),
                }
            }
            if connected == 0 {
                eprintln!("未连接任何 URL MCP 服务器（在 config.yaml 的 mcp.servers[].url 配置；命令式 stdio 桥为下一阶段）");
            }
        }
        _ => {
            print_subcommand_help("mcp");
        }
    }
    Ok(())
}

/// skills — 技能管理（一体内置：rank 暴露 + register-batch 按拓扑批量注册）
fn skills_handler(args: &Args, config: &Config) -> Result<()> {
    use crate::skills::registry::{ranks, SkillRegistry};
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    let skills_dir = crate::tools::skill_tools::resolve_skills_dir(&config.skills.skills_dir);
    let loader = SkillLoader::new(&skills_dir);
    match sub {
        "list" => match loader.load_all() {
            Ok(skills) if skills.is_empty() => {
                println!("未找到技能（目录: {}）", skills_dir.display());
            }
            Ok(skills) => {
                println!("已安装技能 ({}):", skills.len());
                // 按 rank 升序（高优先级在前）展示
                let mut sorted: Vec<_> = skills.iter().collect();
                sorted.sort_by_key(|s| s.rank);
                for skill in &sorted {
                    let bucket = match skill.rank {
                        r if r <= ranks::SYSTEM => "SYSTEM",
                        r if r <= ranks::PROJECT => "PROJECT",
                        r if r <= ranks::DEFAULT => "DEFAULT",
                        r if r <= ranks::USER => "USER",
                        _ => "COMMUNITY",
                    };
                    println!(
                        "  [{bucket:<8} rank={:>3}] {} v{} — {}",
                        skill.rank, skill.name, skill.version, skill.description
                    );
                }
            }
            Err(e) => eprintln!("加载技能失败: {e}"),
        },
        "ranks" => {
            println!("技能等级（rank 越小优先级越高；register-batch 按此拓扑排序注入）：");
            println!(
                "  SYSTEM    = {r:>3}  内置核心（锚定、冷启动、Bloom 编排）",
                r = ranks::SYSTEM
            );
            println!(
                "  PROJECT   = {r:>3}  项目级（工程脚手架、团队约定）",
                r = ranks::PROJECT
            );
            println!(
                "  DEFAULT   = {r:>3}  通用社区默认（不打标时）",
                r = ranks::DEFAULT
            );
            println!(
                "  USER      = {r:>3}  用户级（个人偏好、快捷键）",
                r = ranks::USER
            );
            println!(
                "  COMMUNITY = {r:>3}  第三方市场（低优先级覆写受保护前缀）",
                r = ranks::COMMUNITY
            );
        }
        "install" => {
            let url = args.sub_args.get(1).map(String::as_str).unwrap_or("");
            if url.is_empty() {
                eprintln!("用法: marguerite skills install <yaml-url>");
                return Ok(());
            }
            // 远程下载并注册（异步请求，同步等待）
            let rt = tokio::runtime::Runtime::new()?;
            match rt.block_on(loader.download_and_register(url)) {
                Ok(skill) => {
                    println!(
                        "技能已安装: {} v{} (rank={})",
                        skill.name, skill.version, skill.rank
                    );
                }
                Err(e) => eprintln!("安装失败: {e}"),
            }
        }
        "register-batch" => {
            let dir = args.sub_args.get(1).map(String::as_str).unwrap_or("");
            let target = if dir.is_empty() {
                skills_dir.clone()
            } else {
                PathBuf::from(dir)
            };
            if !target.exists() {
                eprintln!("目录不存在: {}", target.display());
                std::process::exit(2);
            }
            // 收集目录下 *.yaml
            let mut yamls: Vec<PathBuf> = Vec::new();
            if let Ok(read) = std::fs::read_dir(&target) {
                for entry in read.flatten() {
                    let p = entry.path();
                    if p.extension().map(|e| e == "yaml").unwrap_or(false) {
                        yamls.push(p);
                    }
                }
            }
            // 每个用 loader 解析
            let mut batch = Vec::with_capacity(yamls.len());
            for y in &yamls {
                match loader.load_single(y) {
                    Ok(s) => batch.push(s),
                    Err(e) => eprintln!("警告: 跳过 {} ({e})", y.display()),
                }
            }
            // 拓扑排序 + 批量注册（幂等）
            let mut registry = SkillRegistry::new();
            let registered = registry.register_batch_ranked(batch)?;
            println!(
                "register-batch 完成：目录 {}，扫描 {} 个 YAML，幂等注册 {} 个",
                target.display(),
                yamls.len(),
                registered.len()
            );
            if args.json {
                let out = serde_json::json!({
                    "dir": target.display().to_string(),
                    "scanned": yamls.len(),
                    "registered": registered,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            }
        }
        _ => {
            print_subcommand_help("skills");
        }
    }
    Ok(())
}

/// games — 内置小游戏（list / play）；离线可玩，输出确定性棋盘/结果。
///
/// 用法：
///   marguerite games list
///   marguerite games play <id> <动作...>
///   marguerite games list --json
fn games_handler(args: &Args, _config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    let catalog = crate::games::catalog();
    match sub {
        "" | "list" | "ls" => {
            if args.json {
                let items: Vec<serde_json::Value> = catalog
                    .iter()
                    .map(|g| {
                        serde_json::json!({
                            "id": g.id,
                            "name": g.name,
                            "controls": g.controls,
                            "status": "implemented",
                        })
                    })
                    .collect();
                print_json_ok(serde_json::json!({
                    "total": catalog.len(),
                    "games": items,
                }));
            } else {
                println!("已实装小游戏（{}）：", catalog.len());
                for g in catalog {
                    println!("  {:<8} {:<16}（{}）", g.id, g.name, g.controls);
                }
                println!("玩法：marguerite games play <id> <动作...>");
            }
            Ok(())
        }
        "play" => {
            let rest = &args.sub_args[1..];
            let id = rest.first().map(String::as_str).unwrap_or("");
            let moves: Vec<&str> = rest.iter().skip(1).map(|s| s.as_str()).collect();
            if id.is_empty() {
                eprintln!("用法: marguerite games play <id> <动作...>");
                return Ok(());
            }
            match crate::games::run_game(id, &moves) {
                Ok(out) => {
                    if args.json {
                        print_json_ok(serde_json::json!({
                            "game": id,
                            "moves": moves,
                            "render": out,
                        }));
                    } else {
                        println!("{out}");
                    }
                    Ok(())
                }
                Err(e) => {
                    // 玩法错误（未知游戏/非法动作）如实报错，不 panic
                    Err(anyhow::anyhow!("{e}"))
                }
            }
        }
        _ => {
            print_subcommand_help("games");
            Ok(())
        }
    }
}

/// capabilities — 能力自检（真实落地状态）
///
/// 从 `domain::capabilities::registry()` 读取能力清单唯一来源，按成熟度
/// （Real / Partial / Narrative）展示每项能力的真实落地状态。直接服务于
/// 「根除宣称未落地」的反虚假目标——用户可一眼看到哪些是真货、哪些是纯叙事。
///
/// 用法：
///   marguerite capabilities                列出全量能力（按组分组，带成熟度标记）
///   marguerite capabilities <group>        仅列出某组（kebab 名，如 tooling / security）
///   marguerite capabilities --json         以 JSON 输出（便于脚本 / agent 接入）
fn capabilities_handler(args: &Args, _config: &Config) -> Result<()> {
    use std::collections::BTreeMap;

    let filter = args
        .sub_args
        .first()
        .map(String::as_str)
        .unwrap_or("")
        .to_lowercase();

    let all = registry();
    let s = stats();

    // 按组过滤（group 的 kebab 序列化名，如 tooling / security / orchestration）
    let caps: Vec<&Capability> = if filter.is_empty() {
        all.iter().collect()
    } else {
        all.iter()
            .filter(|c| {
                let kebab = serde_json::to_string(&c.group)
                    .map(|v| v.trim_matches('"').to_string())
                    .unwrap_or_default();
                kebab == filter || format!("{:?}", c.group).to_lowercase() == filter
            })
            .collect()
    };

    if args.json {
        let items: Vec<serde_json::Value> = caps
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id.0,
                    "name": c.name,
                    "group": c.group,
                    "tier": c.tier,
                    "maturity": c.maturity,
                    "implemented": c.maturity.is_implemented(),
                    "description": c.description,
                    "supersedes": c.supersedes,
                })
            })
            .collect();
        let data = serde_json::json!({
            "total": caps.len(),
            "real": caps.iter().filter(|c| c.maturity == crate::domain::capabilities::CapabilityMaturity::Real).count(),
            "partial": caps.iter().filter(|c| c.maturity == crate::domain::capabilities::CapabilityMaturity::Partial).count(),
            "implemented": caps.iter().filter(|c| c.maturity.is_implemented()).count(),
            "narrative": caps.iter().filter(|c| !c.maturity.is_implemented()).count(),
            "global": {
                "total": s.total,
                "exclusive": s.exclusive,
                "enhanced": s.enhanced,
                "baseline": s.baseline,
                "real": s.real,
                "partial": s.partial,
                "implemented": s.implemented,
                "narrative": s.narrative,
            },
            "capabilities": items,
        });
        print_json_ok(data);
        return Ok(());
    }

    // 文本输出（ASCII 状态标记，利 NO_COLOR / 非 TTY）
    println!("Marguerite 能力自检（真实落地状态）");
    if filter.is_empty() {
        println!(
            "  全量 {} 项 | 已实装 {}（真货 {} / 部分 {}） | 纯叙事 {}",
            s.total, s.implemented, s.real, s.partial, s.narrative
        );
        println!(
            "  分级：独家 {} / 增强 {} / 基线 {}",
            s.exclusive, s.enhanced, s.baseline
        );
    } else {
        println!(
            "  过滤组 '{}'：命中 {} 项 | 已实装 {} | 纯叙事 {}",
            filter,
            caps.len(),
            caps.iter().filter(|c| c.maturity.is_implemented()).count(),
            caps.iter().filter(|c| !c.maturity.is_implemented()).count()
        );
    }
    println!();

    let mut grouped: BTreeMap<CapabilityGroup, Vec<&Capability>> = BTreeMap::new();
    for c in &caps {
        grouped.entry(c.group).or_default().push(c);
    }

    let mark = |m: CapabilityMaturity| match m {
        CapabilityMaturity::Real => "[x]",
        CapabilityMaturity::Partial => "[~]",
        CapabilityMaturity::Narrative => "[ ]",
    };

    for (g, items) in &grouped {
        println!("● {}", g.label());
        for c in items {
            println!(
                "  {} {} ({}) [{}]",
                mark(c.maturity),
                c.name,
                c.maturity.label(),
                c.tier.label()
            );
        }
        println!();
    }

    println!("标记： [x] 已实装  [~] 部分实装  [ ] 纯叙事（零实现）");
    println!("说明：纯叙事能力为规划中的重型/LLM 依赖项，当前未落地，不计入「真货」。");
    Ok(())
}

/// session — 会话管理
fn session_handler(args: &Args, _config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    manager.init()?;

    match sub {
        "list" => match manager.list_sessions(20)? {
            sessions if sessions.is_empty() => {
                println!("暂无会话记录");
            }
            sessions => {
                println!("最近会话 ({}):", sessions.len());
                for s in &sessions {
                    println!(
                        "  [{}] {} | {} 条消息 | {}",
                        s.session_id.chars().take(8).collect::<String>(),
                        s.summary.chars().take(40).collect::<String>(),
                        s.message_count,
                        s.updated_at
                    );
                }
            }
        },
        "delete" => {
            let id = args.sub_args.get(1).map(String::as_str).unwrap_or("");
            if id.is_empty() {
                eprintln!("用法: marguerite session delete <session-id>");
                return Ok(());
            }
            match manager.delete_session(id) {
                Ok(_) => println!("会话已删除: {id}"),
                Err(e) => eprintln!("删除失败: {e}"),
            }
        }
        _ => {
            print_subcommand_help("session");
        }
    }
    Ok(())
}

/// goal — 目标循环（自治续轮 + blocked 门控，Track 1）
///
/// 单一持久化完成目标：goal create 登记（armed）；goal update 做
/// edit/pause/resume/complete/blocked 状态机迁移；blocked 需同一阻塞条件连续
/// MIN_BLOCKED_ROUNDS 轮才成立。持久化于 ~/.marguerite/goal.db（跨进程可见）。
fn goal_handler(args: &Args, _config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    let mut reg = crate::goal::global()
        .lock()
        .map_err(|e| anyhow!("目标注册表锁中毒: {e}"))?;

    match sub {
        "create" => {
            let objective = args
                .sub_args
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if objective.trim().is_empty() {
                eprintln!("用法: marguerite goal create <目标描述>");
                return Ok(());
            }
            match reg.create(&objective, None) {
                Ok(g) => {
                    if args.json {
                        print_json_ok(crate::goal::goal_view(&g, true));
                    } else {
                        println!(
                            "目标已创建 [{}] revision={} phase={} activation=armed",
                            g.id,
                            g.revision,
                            g.phase.as_str()
                        );
                        println!("  {}", g.objective);
                    }
                }
                Err(e) => {
                    if args.json {
                        print_json_err(ExitCode::Conflict, &e.to_string(), None, false);
                    } else {
                        eprintln!("创建失败: {e}");
                    }
                }
            }
        }
        "get" => match reg.get() {
            Some(g) => {
                let armed = reg.is_armed();
                if args.json {
                    print_json_ok(crate::goal::goal_view(g, armed));
                } else {
                    let cap = g
                        .max_goal_rounds
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "不限".to_string());
                    println!("目标 [{}] revision={}", g.id, g.revision);
                    println!(
                        "  phase: {}  activation: {}",
                        g.phase.as_str(),
                        if armed { "armed" } else { "disarmed" }
                    );
                    println!("  rounds: {} / {}", g.rounds_started, cap);
                    println!("  objective: {}", g.objective);
                    if let Some(r) = &g.blocked_reason {
                        println!("  blocked_reason: {r}");
                    }
                }
            }
            None => {
                if args.json {
                    print_json_ok(serde_json::Value::Null);
                } else {
                    println!("尚无目标。用 marguerite goal create <目标> 创建。");
                }
            }
        },
        "update" => {
            let action = args.sub_args.get(1).map(String::as_str).unwrap_or("");
            let result = match action {
                "edit" => {
                    let objective = args
                        .sub_args
                        .iter()
                        .skip(2)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if objective.trim().is_empty() {
                        eprintln!("用法: marguerite goal update edit <新目标>");
                        return Ok(());
                    }
                    reg.edit(&objective)
                }
                "pause" => reg.pause(),
                "resume" => reg.resume(),
                "complete" => reg.complete(),
                "blocked" => {
                    let reason = args
                        .sub_args
                        .iter()
                        .skip(2)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if reason.trim().is_empty() {
                        eprintln!("用法: marguerite goal update blocked <原因>");
                        return Ok(());
                    }
                    reg.block(&reason)
                }
                _ => {
                    print_subcommand_help("goal");
                    return Ok(());
                }
            };
            match result {
                Ok(g) => {
                    if args.json {
                        print_json_ok(crate::goal::goal_view(&g, reg.is_armed()));
                    } else {
                        println!(
                            "目标 [{}] revision={} phase={} activation={}",
                            g.id,
                            g.revision,
                            g.phase.as_str(),
                            if reg.is_armed() { "armed" } else { "disarmed" }
                        );
                    }
                }
                Err(e) => {
                    if args.json {
                        print_json_err(ExitCode::Conflict, &e.to_string(), None, false);
                    } else {
                        eprintln!("更新失败: {e}");
                    }
                }
            }
        }
        _ => print_subcommand_help("goal"),
    }
    Ok(())
}

/// cache — KV 缓存命中率与可缓存比
///
/// `marguerite cache stats`：报告已落盘客户端前缀缓存（`~/.marguerite/prefix_cache.json`）
/// 的命中率统计（查询/命中/未命中/命中率/复用率/前缀复用）。
/// `marguerite cache ratio [session-id]`：计算某会话（默认最近）的稳定前缀可缓存比，
/// 指导把易变数据移出前缀以放大 KV 命中率。
fn cache_handler(args: &Args, _config: &Config) -> Result<()> {
    match args.sub_args.first().map(String::as_str).unwrap_or("") {
        "stats" => cache_stats(args),
        "ratio" => cache_ratio(args),
        _ => {
            print_subcommand_help("cache");
            Ok(())
        }
    }
}

/// `cache stats`：加载落盘前缀缓存并输出命中率统计。
fn cache_stats(args: &Args) -> Result<()> {
    let path = std::env::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".marguerite")
        .join("prefix_cache.json");
    let cache = crate::cache::PrefixCache::open(&path);
    let s = cache.stats();
    let entries = cache.len();
    if args.json {
        print_json_ok(serde_json::json!({
            "path": path.display().to_string(),
            "entries": entries,
            "lookups": s.lookups,
            "hits": s.hits,
            "misses": s.misses,
            "hit_rate": s.hit_rate(),
            "reuse_ratio": s.reuse_ratio(),
            "prefix_hits": s.prefix_hits,
            "prefix_saved_tokens": s.prefix_saved_tokens,
            "saved_prompt_tokens": s.saved_prompt_tokens,
        }));
        return Ok(());
    }
    println!("Marguerite KV 缓存统计");
    println!("  缓存文件: {}", path.display());
    println!("  缓存条目: {entries}");
    println!(
        "  查询 {} | 命中 {} | 未命中 {}",
        s.lookups, s.hits, s.misses
    );
    println!("  客户端命中率: {:.1}%", s.hit_rate() * 100.0);
    println!("  复用率: {:.1}%", s.reuse_ratio() * 100.0);
    println!(
        "  前缀复用: {} 次 (省 {} prompt tokens)",
        s.prefix_hits, s.prefix_saved_tokens
    );
    println!("  命中省下: {} prompt tokens", s.saved_prompt_tokens);
    Ok(())
}

/// `cache ratio [session-id]`：对指定（或最近）会话计算稳定前缀可缓存比。
fn cache_ratio(args: &Args) -> Result<()> {
    let db_path = Config::config_dir().join("sessions.db");
    let mut manager = SessionManager::new(&db_path);
    manager.init()?;
    let session_id: String = match args.sub_args.get(1) {
        Some(id) => id.clone(),
        None => match manager.list_sessions(1)?.pop() {
            Some(s) => s.session_id,
            None => {
                eprintln!("暂无会话，无法计算可缓存比。请先运行一次对话。");
                return Ok(());
            }
        },
    };
    let messages = manager.restore_session(&session_id)?;
    if messages.is_empty() {
        eprintln!("会话 {session_id} 无消息");
        return Ok(());
    }
    let family = 1.0;
    let ratio = crate::context::optimize::prefix_cacheable_ratio(&messages, family);
    let stable_tokens = crate::context::optimize::stable_prefix_tokens(&messages, family);
    let total_tokens: usize = messages
        .iter()
        .map(|m| match m {
            Message::User { content, .. }
            | Message::Assistant { content, .. }
            | Message::System { content, .. } => crate::model::llm::token_estimate(content, family),
            Message::Tool { input, output, .. } => {
                crate::model::llm::token_estimate(input, family)
                    + crate::model::llm::token_estimate(output, family)
            }
        })
        .sum();
    if args.json {
        print_json_ok(serde_json::json!({
            "session_id": session_id,
            "prefix_cacheable_ratio": ratio,
            "stable_tokens": stable_tokens,
            "total_tokens": total_tokens,
        }));
        return Ok(());
    }
    println!("Marguerite 稳定前缀可缓存比（KV-Cache P1 指导）");
    println!("  会话: {session_id}");
    println!("  可缓存比: {:.1}%", ratio * 100.0);
    println!("  稳定前缀 tokens: {stable_tokens} / 总 tokens: {total_tokens}");
    let advice = if ratio >= 0.8 {
        "前缀结构良好，KV 命中潜力高。"
    } else if ratio >= 0.5 {
        "尚可，建议把高易变内容（时间戳/动态采样）移出前缀。"
    } else {
        "偏低，稳定前缀占比不足，建议重排 system/persona/KB 前置。"
    };
    println!("  结论: {advice}");
    Ok(())
}

/// `export-diagnostics` — 导出脱敏诊断包（S6）。
///
/// `marguerite export-diagnostics [--output <路径>] [--json]`。
///
/// 纯只读、离线：只读磁盘，不改用户配置、不联网上报。归档为未压缩 tar
/// （`tar` crate 已在 lockfile 内，且 `session/rollback.rs` 已用它避开 C 工具链
/// 依赖；引入 `zip` 会新增依赖面）。
///
/// `--output` 语义：以 `.tar` 结尾按**文件**路径处理，否则按**目录**处理
/// （在目录下自动生成带时间戳的文件名）。空串视为未给出。
fn export_diagnostics_handler(args: &Args, _config: &Config) -> Result<()> {
    use crate::observability::bundle::{
        export_bundle, render_summary, resolve_output, BundleOptions,
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    let explicit = flag_value(args, "--output");
    let mut dest = resolve_output(explicit.as_deref(), now_ms);
    // `--output <目录>`：补一个带时间戳的文件名，语义对齐 bundle 层。
    if dest.extension().is_none() {
        let name = dest
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        dest = crate::observability::bundle::default_bundle_path(now_ms).join(name);
    }

    let manifest = export_bundle(&dest, &BundleOptions::default())?;
    if args.json {
        print_json_ok(&manifest);
    } else {
        print!("{}", render_summary(&manifest));
    }
    Ok(())
}

/// 从 `sub_args` 里取 `--flag <value>` 的值（缺值或紧跟另一选项 → None）。
///
/// 与 argv 层约定一致：`SUBCOMMAND_FLAGS_WITH_VALUE` 里的旗标由各 handler
/// 自行消费，此处只做取值，不做旗标识别（识别是argv 层的事）。
fn flag_value(args: &Args, flag: &str) -> Option<String> {
    let mut iter = args.sub_args.iter();
    while let Some(token) = iter.next() {
        if token == flag {
            return iter
                .next()
                .filter(|v| !v.starts_with('-'))
                .cloned();
        }
    }
    None
}

/// exec — 沙箱命令执行
///
/// `marguerite exec -- <argv...>`：命令经终端沙箱白名单/路径/模式门控后执行，
/// 审计写入 `.marguerite-audit/`。CLI 非交互 → 写类命令默认拒绝。
fn exec_handler(args: &Args, _config: &Config) -> Result<()> {
    use std::path::Path;

    let argv = args.sub_args.clone();
    if argv.is_empty() {
        print_subcommand_help("exec");
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(async {
        let sandbox = crate::sandbox::Sandbox::new(Path::new("."));
        sandbox.set_interactive(false); // CLI 非交互 → 写命令 fail-closed
        sandbox
            .execute(&argv, Path::new("."), crate::mode::ModeKind::Agent)
            .await
    });

    match result {
        Ok(r) => {
            if r.decision == "allow" {
                print!("{}", r.stdout);
                if !r.stderr.is_empty() {
                    eprint!("{}", r.stderr);
                }
                if r.truncated {
                    eprintln!("（输出已截断）");
                }
                // spawn 失败（exit_code=-1）时退出码取 1，避免掩盖失败
                std::process::exit(if r.exit_code >= 0 { r.exit_code } else { 1 });
            } else {
                eprintln!("[{}] {}", r.decision, r.stderr);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("沙箱执行失败: {e}");
            std::process::exit(1);
        }
    }
}

/// sandbox — 真实沙箱执行与状态
///
/// `marguerite sandbox exec -- <argv...>`：与 `exec` 同一真实沙箱（白名单 + 路径守卫 +
/// 模式门控 + JSONL 审计）。`marguerite sandbox workspace`：显示沙箱工作区。
fn sandbox_handler(args: &Args, _config: &Config) -> Result<()> {
    use std::path::Path;

    let sub = args.sub_args.first().map(|s| s.as_str()).unwrap_or("");
    match sub {
        "exec" => {
            let argv: Vec<String> = args.sub_args.iter().skip(1).cloned().collect();
            if argv.is_empty() {
                println!("用法: marguerite sandbox exec -- <命令...>");
                return Ok(());
            }
            let rt = tokio::runtime::Runtime::new()?;
            let result = rt.block_on(async {
                let sandbox = crate::sandbox::Sandbox::new(Path::new("."));
                sandbox.set_interactive(false);
                sandbox
                    .execute(&argv, Path::new("."), crate::mode::ModeKind::Agent)
                    .await
            });
            match result {
                Ok(r) if r.decision == "allow" => {
                    print!("{}", r.stdout);
                    if !r.stderr.is_empty() {
                        eprint!("{}", r.stderr);
                    }
                    std::process::exit(if r.exit_code >= 0 { r.exit_code } else { 1 });
                }
                Ok(r) => {
                    eprintln!("[{}] {}", r.decision, r.stderr);
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("沙箱执行失败: {e}");
                    std::process::exit(1);
                }
            }
        }
        "workspace" => {
            println!("沙箱工作区: {}", std::env::current_dir()?.display());
            println!("资源限制: 超时 60s · 输出上限 1MB · 最大子进程 8");
            println!("门控: Plan 禁写 · Agent 非交互写 fail-closed · Bloom 沙箱白名单");
            Ok(())
        }
        _ => {
            println!("用法:");
            println!("  marguerite sandbox exec -- <命令...>  在真实沙箱中执行");
            println!("  marguerite sandbox workspace          显示沙箱状态");
            Ok(())
        }
    }
}

/// subagent — 子代理团队 DAG 编排（真实执行）
///
/// `marguerite subagent team "<goal>"`：模板分解（调研→规划→实现→审校）→ Kahn 分层 →
/// 逐层并行真实 spawn 子代理（真实模型路由 + 真实工具注册表）→ lead 综合。
fn subagent_handler(args: &Args, config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(|s| s.as_str()).unwrap_or("");
    let reflect = args
        .sub_args
        .iter()
        .any(|s| s == "--reflect" || s == "--agentic");
    let use_trellis = args.sub_args.iter().any(|s| s == "--trellis");
    // 目标取 "team" 之后的第一个非旗标参数
    let goal = args
        .sub_args
        .iter()
        .skip_while(|s| *s != "team")
        .nth(1)
        .filter(|s| !s.starts_with("--"))
        .cloned()
        .unwrap_or_default();
    if sub != "team" || goal.is_empty() {
        println!("用法: marguerite subagent team \"<目标描述>\" [--reflect] [--trellis]");
        println!("说明: 走 DAG 分层编排（调研→规划→实现→审校），子代理真实执行。");
        println!(
            "      --reflect  启用 分阶段智能体 分阶段反思（s2_5）：实现后插入反思自检再交付。"
        );
        println!(
            "      --trellis  以事件驱动 trellis_wait（订阅 EventBus 的 subagent_end）收敛替代忙轮询。"
        );
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()?;
    let cfg = if config.model.api_key.is_empty() {
        Config::load().unwrap_or_else(|_| Config::default())
    } else {
        config.clone()
    };
    if cfg.model.api_key.is_empty() {
        println!(
            "警告: 未配置 MARGUERITE_API_KEY，子代理将真实启动但 LLM 调用会失败（真实失败可见）。"
        );
    }
    let mut orch = crate::orchestration::Orchestrator::new(cfg);
    println!(
        "[subagent] 分解任务 DAG（Build 形态）… trellis={}",
        if use_trellis {
            "事件驱动"
        } else {
            "默认（兼容忙轮询）"
        }
    );

    rt.block_on(async move {
        // --trellis：创建 EventBus + 订阅 subagent_end，打印每帧收敛
        let bus = if use_trellis {
            let b = Arc::new(crate::events::EventBus::new(4096));
            let session_id = format!("cli-subagent-{}", uuid::Uuid::new_v4());
            orch.set_event_bus(b.clone(), session_id);
            // 订阅：把收到的 end 事件以 [subagent] trellis: <id> ok? 形式打印
            let mut rx = b.subscribe("session.event.subagent_end");
            tokio::spawn(async move {
                while let Some(env) = rx.recv().await {
                    let id = env
                        .payload
                        .get("subagent_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let ok = env
                        .payload
                        .get("ok")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    println!("[subagent][trellis] end: id={id} ok={ok}");
                }
            });
            Some(b)
        } else {
            None
        };

        let kind = if reflect {
            crate::orchestration::dag::DagKind::Reflect
        } else {
            crate::orchestration::dag::DagKind::Build
        };
        match orch.dispatch_team_dag(&goal, kind).await {
            Ok(outcome) => {
                println!("[subagent] 策略: {}", outcome.strategy.label());
                println!(
                    "[subagent] 子代理产出 {} 段:",
                    outcome.subagent_summaries.len()
                );
                for (i, s) in outcome.subagent_summaries.iter().enumerate() {
                    println!("  [{}/{}] {}", i + 1, outcome.subagent_summaries.len(), s);
                }
                if reflect {
                    let r = crate::orchestration::dag::reflect_pass(&outcome.subagent_summaries);
                    println!("\n[subagent] s2_5 反思自检（分阶段智能体 对齐）:");
                    println!(
                        "[subagent]   决策: {}",
                        if r.go {
                            "GO · 工作记忆可信，可交付"
                        } else {
                            "NO-GO · 存在失败/空阶段，需人工复核"
                        }
                    );
                    for n in &r.notes {
                        println!("[subagent]     - {n}");
                    }
                }
                println!("\n[subagent] lead 综合:");
                println!("{}", outcome.lead_reply);
                // 若总线存在：显式 drop，关闭订阅通道
                drop(bus);
            }
            Err(e) => {
                eprintln!("[subagent] 编排失败: {e}");
                std::process::exit(1);
            }
        }
    });
    Ok(())
}

// team — 直接触发专家团队并行 spawn（`Team::run` 真实执行体，区别于 DAG 版 `subagent team`）
//
/// P4：团队运行终态的退出码裁决（纯函数，便于单测锁死）。
///
/// 门禁放行 → `None`（常规 0 退出）；未放行 → `Some(Conflict)`。
/// 之所以不返回 0/1 两值：`Err(e)` 分支已用 1 表示「编排层抛错」，
/// 二者语义需区分（前者是"跑完了但没做成"，后者是"根本没跑起来"）。
fn team_failure_exit_code(completion_allowed: bool) -> Option<ExitCode> {
    if completion_allowed {
        None
    } else {
        Some(ExitCode::Conflict)
    }
}

/// `marguerite team "<goal>"`：自动角色检测 → 专家团队并行 spawn → lead 综合。
/// 离线降级：LLM 不可达时成员失败如实可见（不静默、不冒充成功），与 `subagent team` 一致。
fn team_handler(args: &Args, config: &Config) -> Result<()> {
    let goal = args
        .sub_args
        .iter()
        .find(|s| !s.starts_with("--"))
        .cloned()
        .unwrap_or_default();
    if goal.is_empty() {
        println!("用法: marguerite team \"<目标描述>\"");
        println!("说明: 直接触发专家团队并行 spawn（Team::run），区别于 DAG 版 `subagent team`。");
        println!("      自动角色检测（设计 / 编码 / 测试 / 文档 / 安全 / 调研…）构造团队；");
        println!("      未检测到专精角色时退化为单成员通用团队，至少触发一次真实 spawn。");
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()?;
    let cfg = if config.model.api_key.is_empty() {
        Config::load().unwrap_or_else(|_| Config::default())
    } else {
        config.clone()
    };
    if cfg.model.api_key.is_empty() {
        println!("警告: 未配置 MARGUERITE_API_KEY，团队成员将真实启动但 LLM 调用会失败（真实失败可见）。");
    }
    let mut orch = crate::orchestration::Orchestrator::new(cfg);
    println!("[team] 构造专家团队（并行 spawn）…");
    rt.block_on(async {
        match orch.dispatch_team(&goal).await {
            Ok(out) => {
                println!("[team] 策略: {}", out.strategy.label());
                println!("[team] 团队成员产出 {} 段:", out.subagent_summaries.len());
                for (i, s) in out.subagent_summaries.iter().enumerate() {
                    println!("  [{}/{}] {}", i + 1, out.subagent_summaries.len(), s);
                }
                println!("\n[team] lead 综合:");
                println!("{}", out.lead_reply);
                // P4：完成门禁未放行时**如实非零退出**。此前这里只打印、随后 `Ok(())`，
                // 于是「首席明说未能完成」与「真的做完了」在退出码上完全无法区分
                // （实测：子专家 400 失败、首席答"未能完成"，退出码仍为 0）。
                // 复用既有分层退出码 Conflict(9)，不新增码以免破坏既有约定。
                if let Some(code) = team_failure_exit_code(out.completion_verdict.allowed) {
                    eprintln!(
                        "[team] 完成门禁未放行: {}",
                        out.completion_verdict.warnings.join("；")
                    );
                    std::process::exit(code.code());
                }
            }
            Err(e) => {
                eprintln!("[team] 团队编排失败: {e}");
                std::process::exit(1);
            }
        }
    });
    Ok(())
}

/// browser — 真实浏览器 CDP 操控
///
/// `marguerite browser open <url> [--out <file.png>]`：启动真实 Chromium（headless），
/// 导航 → 截图保存 → 提取 DOM 元素 getBoundingClientRect 坐标（归一化 0..1）打印。
fn browser_handler(args: &Args, _config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(|s| s.as_str()).unwrap_or("");
    if sub != "open" {
        println!("用法: marguerite browser open <url> [--out <file.png>]");
        return Ok(());
    }
    let url = args.sub_args.get(1).cloned().unwrap_or_default();
    if url.is_empty() {
        println!("用法: marguerite browser open <url> [--out <file.png>]");
        return Ok(());
    }
    let out = args
        .sub_args
        .iter()
        .position(|s| s == "--out")
        .and_then(|i| args.sub_args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "browser-screenshot.png".to_string());

    let bridge = crate::browser::BrowserBridge::new();
    if !bridge.is_available() {
        eprintln!("{}", crate::browser::browser_unavailable_message());
        std::process::exit(1);
    }
    println!("[browser] 启动 Chromium（headless）并打开 {url} …");

    let rt = tokio::runtime::Runtime::new()?;
    let res: anyhow::Result<()> = rt.block_on(async {
        match bridge.open(&url).await {
            Ok(mut page) => {
                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                let title = page
                    .session
                    .evaluate("document.title", false)
                    .await
                    .unwrap_or_else(|e| format!("<读取失败: {e}>"));
                println!("[browser] 页面标题: {title}");

                let png = match page.session.screenshot_png().await {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("[browser] 截图失败: {e}");
                        return Ok(());
                    }
                };
                std::fs::write(&out, &png)?;
                println!("[browser] 截图已保存: {out} ({} bytes)", png.len());

                // DOM 元素坐标提取（真实 getBoundingClientRect 归一化，供 GUI 高亮复用）
                let js = r#"
                    (() => {
                        const vw = window.innerWidth, vh = window.innerHeight;
                        const els = [...document.querySelectorAll('button,a,input,textarea,select,[role="button"],[role="link"]')]
                            .slice(0, 20).map((el, i) => {
                                const r = el.getBoundingClientRect();
                                const name = (el.innerText || el.getAttribute('aria-label') || el.getAttribute('placeholder') || '').trim().slice(0, 40);
                                return {
                                    id: (el.tagName + '-' + i).toLowerCase(),
                                    tag: el.tagName.toLowerCase(),
                                    role: el.getAttribute('role') || el.tagName.toLowerCase(),
                                    name,
                                    x: +(r.x / vw).toFixed(4),
                                    y: +(r.y / vh).toFixed(4),
                                    w: +(r.width / vw).toFixed(4),
                                    h: +(r.height / vh).toFixed(4)
                                };
                            });
                        return JSON.stringify({ vw, vh, els });
                    })()
                "#;
                match page.session.evaluate(js, true).await {
                    Ok(s) => {
                        println!("[browser] DOM 元素（归一化坐标）:");
                        println!("{s}");
                    }
                    Err(e) => eprintln!("[browser] DOM 坐标提取失败: {e}"),
                }
            }
            Err(e) => {
                eprintln!("[browser] 打开失败: {e}");
            }
        }
        Ok(())
    });
    res?;
    Ok(())
}

/// deploy — 一键部署（Phase I2，FR-16）
///
/// `marguerite deploy --target ssh://[user@]host[:port][/dir] [--bin <path>]`
fn deploy_handler(_args: &Args, _config: &Config) -> Result<()> {
    let cli = DeployCli::parse()?;

    // 注册面板别名：marguerite deploy --target baota://alias --add-panel --endpoint https://host:8888 --api-key xxx
    if cli.add_panel {
        let alias = cli
            .target
            .strip_prefix("baota://")
            .filter(|a| !a.is_empty())
            .ok_or_else(|| anyhow!("请使用 --target baota://<别名> 配合 --add-panel"))?;
        let endpoint = cli
            .endpoint
            .clone()
            .ok_or_else(|| anyhow!("--add-panel 需提供 --endpoint https://host:8888"))?;
        let api_key = cli
            .api_key
            .clone()
            .ok_or_else(|| anyhow!("--add-panel 需提供 --api-key <面板API密钥>"))?;
        add_panel(alias, &endpoint, &api_key)?;
        println!("已注册宝塔面板别名 '{alias}'（API 密钥已 AES-0600 加密落盘）");
        return Ok(());
    }

    if cli.target.is_empty() {
        print_subcommand_help("deploy");
        return Ok(());
    }

    if cli.target.starts_with("baota://") {
        return deploy_baota(&cli);
    }

    // 默认 SSH
    let mut target = SshTarget::from_url(&cli.target)?;
    if let Some(u) = cli.user_override {
        target.user = u;
    }
    if let Some(p) = cli.port_override {
        target.port = p;
    }
    if let Some(d) = cli.remote_dir {
        target.remote_dir = d;
    }
    target.identity = cli.identity;

    let bin = match &cli.bin {
        Some(b) => b.clone(),
        None => match auto_detect_bin() {
            Some(b) => b,
            None => {
                bail!("未找到 release 二进制，请用 --bin 指定，或先执行构建产出 release 单文件")
            }
        },
    };
    if !bin.exists() {
        bail!("二进制不存在: {}", bin.display());
    }

    let opts = DeployOptions {
        service: cli.service.clone(),
        health_url: cli.health_url.clone(),
        use_systemd: !cli.no_systemd,
        dry_run: cli.dry_run,
    };

    if cli.dry_run {
        let transport = RecordingTransport::new();
        let report = SshDeployer::deploy(&transport, &target, &opts, &bin)?;
        println!("=== SSH 部署演练 (dry-run) ===");
        println!("目标    : {}", target.endpoint());
        println!("目录    : {}", target.remote_dir);
        println!(
            "二进制  : {} (sha256: {})",
            bin.display(),
            report.binary_sha256
        );
        println!(
            "服务    : {} | systemd: {} | 健康检查: {}",
            opts.service,
            opts.use_systemd,
            opts.health_url.as_deref().unwrap_or("(systemd active)")
        );
        println!("远端步骤将依次执行:");
        for (i, s) in transport.exec_calls().iter().enumerate() {
            println!("  {}. {}", i + 1, s);
        }
        println!("上传    : {} -> {}", bin.display(), opts.service);
        println!("（dry-run 未实际连接，演练完成）");
    } else {
        let transport = SystemSshTransport::new(&target);
        let report = SshDeployer::deploy(&transport, &target, &opts, &bin)?;
        println!("=== SSH 部署完成 ===");
        println!("目标    : {}", target.endpoint());
        println!("二进制 sha256: {}", report.binary_sha256);
        println!("健康检查: {}", if report.success { "通过" } else { "失败" });
        if report.rollback_available {
            println!("[回滚] 健康未通过，已回滚到上一版本");
        } else {
            println!("服务已上线: {}", opts.service);
        }
    }
    Ok(())
}

/// 宝塔部署分支
fn deploy_baota(cli: &DeployCli) -> Result<()> {
    let alias = cli
        .target
        .strip_prefix("baota://")
        .filter(|a| !a.is_empty())
        .ok_or_else(|| anyhow!("宝塔目标格式应为 baota://<面板别名>"))?;
    let panel = resolve_panel(alias)?;

    let bin = match &cli.bin {
        Some(b) => b.clone(),
        None => match auto_detect_bin() {
            Some(b) => b,
            None => {
                bail!("未找到 release 二进制，请用 --bin 指定，或先执行构建产出 release 单文件")
            }
        },
    };
    if !bin.exists() {
        bail!("二进制不存在: {}", bin.display());
    }

    let opts = BaotaDeployOptions {
        service: cli.service.clone(),
        site: cli.site.clone(),
        health_url: cli.health_url.clone(),
        ssl: !cli.no_ssl,
        dry_run: cli.dry_run,
    };

    if cli.dry_run {
        let transport = RecordingBaotaTransport::new();
        let rt = tokio::runtime::Runtime::new()?;
        let report = rt.block_on(crate::deploy::baota::deploy(&transport, &opts, &bin))?;
        println!("=== 宝塔部署演练 (dry-run) ===");
        println!("面板    : {alias} @ {}", panel.endpoint);
        println!("站点    : {}", opts.site.as_deref().unwrap_or("(不建站)"));
        println!("二进制  : {}", bin.display());
        println!(
            "服务    : {} | SSL: {} | 健康检查: {}",
            opts.service,
            opts.ssl,
            opts.health_url.as_deref().unwrap_or("(通过)")
        );
        println!("远端步骤将依次执行:");
        for (i, (action, _)) in transport.calls().iter().enumerate() {
            println!("  {}. {action}", i + 1);
        }
        println!("（dry-run 未实际连接，演练完成）");
        let _ = report;
    } else {
        let transport = ReqwestBaotaTransport::new(panel);
        let rt = tokio::runtime::Runtime::new()?;
        let report = rt.block_on(crate::deploy::baota::deploy(&transport, &opts, &bin))?;
        println!("=== 宝塔部署完成 ===");
        println!("服务    : {}", report.service);
        println!("健康检查: {}", if report.success { "通过" } else { "失败" });
        if report.ssl_applied {
            println!("SSL     : 已申请");
        }
    }
    Ok(())
}

/// 部署 CLI 旗标解析
struct DeployCli {
    target: String,
    bin: Option<std::path::PathBuf>,
    dry_run: bool,
    service: String,
    health_url: Option<String>,
    no_systemd: bool,
    user_override: Option<String>,
    port_override: Option<u16>,
    identity: Option<String>,
    remote_dir: Option<String>,
    // 宝塔专用
    site: Option<String>,
    no_ssl: bool,
    add_panel: bool,
    endpoint: Option<String>,
    api_key: Option<String>,
}

impl DeployCli {
    fn parse() -> Result<Self> {
        let raw: Vec<String> = std::env::args().collect();
        let mut cli = DeployCli {
            target: String::new(),
            bin: None,
            dry_run: false,
            service: "marguerite".to_string(),
            health_url: None,
            no_systemd: false,
            user_override: None,
            port_override: None,
            identity: None,
            remote_dir: None,
            site: None,
            no_ssl: false,
            add_panel: false,
            endpoint: None,
            api_key: None,
        };
        // 定位 "deploy" 之后开始解析旗标
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == "deploy" {
                i += 1;
                break;
            }
            i += 1;
        }
        while i < raw.len() {
            match raw[i].as_str() {
                "--target" => {
                    i += 1;
                    cli.target = raw.get(i).cloned().unwrap_or_default();
                }
                "--bin" => {
                    i += 1;
                    cli.bin = raw.get(i).map(std::path::PathBuf::from);
                }
                "--dry-run" | "--dryrun" => cli.dry_run = true,
                "--service" => {
                    i += 1;
                    cli.service = raw
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| "marguerite".to_string());
                }
                "--health-url" => {
                    i += 1;
                    cli.health_url = raw.get(i).cloned();
                }
                "--no-systemd" => cli.no_systemd = true,
                "--user" => {
                    i += 1;
                    cli.user_override = raw.get(i).cloned();
                }
                "--port" => {
                    i += 1;
                    cli.port_override = raw.get(i).and_then(|s| s.parse().ok());
                }
                "--identity" => {
                    i += 1;
                    cli.identity = raw.get(i).cloned();
                }
                "--dir" => {
                    i += 1;
                    cli.remote_dir = raw.get(i).cloned();
                }
                "--site" => {
                    i += 1;
                    cli.site = raw.get(i).cloned();
                }
                "--no-ssl" => cli.no_ssl = true,
                "--add-panel" => cli.add_panel = true,
                "--endpoint" => {
                    i += 1;
                    cli.endpoint = raw.get(i).cloned();
                }
                "--api-key" => {
                    i += 1;
                    cli.api_key = raw.get(i).cloned();
                }
                _ => {}
            }
            i += 1;
        }
        Ok(cli)
    }
}

/// 自动探测 release 单二进制（按优先级）：CARGO_TARGET_DIR / mgt_build / 当前 exe 目录
fn auto_detect_bin() -> Option<std::path::PathBuf> {
    fn name() -> String {
        if cfg!(windows) {
            "marguerite.exe".to_string()
        } else {
            "marguerite".to_string()
        }
    }
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("CARGO_TARGET_DIR") {
        candidates.push(std::path::PathBuf::from(d).join("release").join(name()));
    }
    candidates.push(
        std::path::PathBuf::from("mgt_build")
            .join("release")
            .join(name()),
    );
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(name()));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// chat — 流式对话（标准 REPL 风格 REPL / 单发）
///
/// 验收：When 运行 `marguerite chat`，系统**必须**进入交互式流式 REPL；
/// When 运行 `marguerite chat "<问题>"`，系统**必须**流式打印一个回答后退出；
/// If `--json`，单发**必须**输出 `{ok, data:{response}, meta}`（静默收齐再包，不混流）。
fn chat_handler(args: &Args, _config: &Config) -> Result<()> {
    let config = Config::load()?;
    let mut runtime = AgentRuntime::new(config.clone());
    runtime.init()?;

    let prompt = args.sub_args.join(" ").trim().to_string();
    if !prompt.is_empty() {
        // 单发流式。带 --resume/--continue 时接续既有会话上下文并把本轮落盘；
        // 裸单发保持无副作用（一次性查询不产生会话记录）。
        let rt = tokio::runtime::Runtime::new()?;
        let result = rt.block_on(async {
            if config.model.api_key.is_empty() {
                return Err(anyhow::anyhow!(
                    "未配置 API Key，请在 {} 中设置 model.api_key",
                    Config::config_path().display()
                ));
            }
            let restore = args.resume.is_some() || args.continue_last;
            let (mut store, mut history) = if restore {
                let (s, h) = ReplStore::open(args.resume.as_deref(), args.continue_last);
                (Some(s), h)
            } else {
                (None, Vec::new())
            };
            history.push(Message::User {
                content: prompt.clone(),
                timestamp: Utc::now(),
            });

            // 工具回路单发：构建注册表 + 工具定义后跑 run_tool_loop；否则走纯流式对话
            let reply = if args.tools {
                let workspace =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                // P0-1b：CLI 的工具注册表正是按这个根解析路径——必须如实同步给
                // 运行时，否则模型猜绝对路径照样 1ms 失败后死循环重试。
                runtime.set_workspace(crate::workspace::Workspace::resolve(Some(&workspace)));
                let mut reg = ToolRegistry::new();
                register_builtin_tools(&mut reg, &workspace, None, None, None)
                    .map_err(|e| anyhow!("工具注册失败：{e}"))?;
                let defs: Vec<ToolDef> = reg
                    .describe_tools_for_llm()
                    .into_iter()
                    .map(|(name, description, parameters)| ToolDef {
                        name,
                        description,
                        parameters,
                    })
                    .collect();
                let before = history.len();
                let r = runtime
                    .run_tool_loop(&mut history, Arc::new(reg), &defs)
                    .await?;
                // 工具活动打印到 stderr（避免污染 --json 的 stdout）；交互模式正常展示
                for m in &history[before..] {
                    if let Message::Tool {
                        name,
                        input,
                        output,
                        ..
                    } = m
                    {
                        let out: String = output.chars().take(800).collect();
                        let ellipsis = if output.chars().count() > 800 {
                            "…"
                        } else {
                            ""
                        };
                        if args.json {
                            eprintln!("  [tool:{name}] {input} → {out}{ellipsis}");
                        } else {
                            println!("  [tool:{name}] {input}");
                            println!("  → {out}{ellipsis}");
                        }
                    }
                }
                r
            } else {
                let rx = runtime.chat_stream(&history).await?;
                stream_assistant(rx, !args.json).await?
            };

            if let Some(store) = store.as_mut() {
                // 工具模式下 run_tool_loop 已把最终 Assistant 推入历史；纯对话模式补推
                let already_has_assistant =
                    matches!(history.last(), Some(Message::Assistant { .. }));
                if !already_has_assistant {
                    history.push(Message::Assistant {
                        content: reply.clone(),
                        thinking: None,
                        timestamp: Utc::now(),
                    });
                }
                store.persist(&history);
            }
            Ok::<String, anyhow::Error>(reply)
        });
        match result {
            Ok(reply) => {
                if args.json {
                    print_json_ok(serde_json::json!({ "response": reply }));
                }
            }
            Err(e) => {
                if args.json {
                    print_json_err(
                        ExitCode::Temporary,
                        &e.to_string(),
                        Some("检查 API Key 与网络连通性"),
                        false,
                    );
                } else {
                    eprintln!("对话失败: {e}");
                }
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    // 无 prompt → 进入 REPL（消费 runtime 所有权，维护多轮历史并自动落盘）
    let rt = tokio::runtime::Runtime::new()?;
    let options = ReplOptions {
        resume: args.resume.clone(),
        continue_last: args.continue_last,
        db_path: None,
        enable_tools: args.tools,
    };
    rt.block_on(run_repl(runtime, options))?;
    Ok(())
}

/// dispatch — 无头分发任务到 11 个外部 agent CLI（一体内置）
///
/// 对外三个叶子动作：
/// - list     ：打印 11 个目标名称 + 描述 + 可用性探测（未配置的显示 unavailable_reason）
/// - run      ：单个目标无头执行，结构化输出 OK/失败、exit code、超时
/// - many     ：全部 11 个（可过滤 --targets a,b,c）并发扇出
fn dispatch_handler(args: &Args, _config: &Config) -> Result<()> {
    use crate::harness::dispatch::{
        available_targets, dispatch, dispatch_many, DispatchRequest, DispatchTarget,
    };
    use std::str::FromStr;

    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    if matches!(sub, "" | "--help" | "-h") {
        print_subcommand_help("dispatch");
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()?;
    match sub {
        "list" => {
            let items = available_targets();
            println!("11 路外部 agent CLI 目标（一体内置）：");
            for t in DispatchTarget::all() {
                let avail = items.get(t).cloned().flatten();
                let status = match &avail {
                    Some(ok) => format!("就绪 · {}", ok.bin_path.display()),
                    None => {
                        let probe = t.probe();
                        match probe {
                            Ok(_) => "就绪（无模板覆盖）".to_string(),
                            Err(e) => format!("未就绪 · {e}"),
                        }
                    }
                };
                println!(
                    "  - {:<18} {:<10}  {}",
                    t.as_str(),
                    if avail.is_some() { "OK" } else { "INFO" },
                    status
                );
            }
            if args.json {
                let rows: Vec<serde_json::Value> = DispatchTarget::all()
                    .iter()
                    .map(|t| {
                        let avail = t.probe();
                        serde_json::json!({
                            "target": t.as_str(),
                            "description": t.description(),
                            "ok": avail.is_ok(),
                            "detail": avail.map_err(|e| e.to_string()).err()
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "items": rows }))?
                );
            }
        }
        "run" => {
            // 参数：<target> "<task>" [workspace] [--timeout N]
            let target_s = args.sub_args.get(1).map(String::as_str).unwrap_or("");
            let task = args.sub_args.get(2).map(String::as_str).unwrap_or("");
            let workspace = args
                .sub_args
                .get(3)
                .filter(|s| !s.starts_with("--"))
                .map(String::as_str)
                .unwrap_or(".");
            let timeout_secs = args
                .sub_args
                .iter()
                .position(|s| s == "--timeout")
                .and_then(|i| args.sub_args.get(i + 1)?.parse::<u64>().ok())
                .unwrap_or(600);
            if target_s.is_empty() || task.is_empty() {
                print_subcommand_help("dispatch");
                return Ok(());
            }
            let target = DispatchTarget::from_str(target_s).map_err(|e| {
                anyhow::anyhow!(
                    "未知目标 '{target_s}'：{e}（可用 `marguerite dispatch list` 查看）"
                )
            })?;
            let mut req = DispatchRequest::new(target, task, workspace).with_timeout(timeout_secs);
            // 可选注入环境（透传 --env K=V）
            let mut i = 0;
            while i < args.sub_args.len() {
                if args.sub_args[i] == "--env" {
                    if let Some(nv) = args.sub_args.get(i + 1) {
                        if let Some((k, v)) = nv.split_once('=') {
                            req = req.with_env(k, v);
                        }
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            let result = rt.block_on(dispatch(req))?;
            render_dispatch_result(&result, args.json);
            if !result.ok {
                std::process::exit(result.exit_code.unwrap_or(1).max(1));
            }
        }
        "many" => {
            // "<task>" [workspace] [--parallel N] [--targets a,b,c] [--timeout N]
            let task = args.sub_args.get(1).map(String::as_str).unwrap_or("");
            if task.is_empty() {
                print_subcommand_help("dispatch");
                return Ok(());
            }
            let workspace = args
                .sub_args
                .get(2)
                .filter(|s| !s.starts_with("--"))
                .map(String::as_str)
                .unwrap_or(".");
            let parallel = args
                .sub_args
                .iter()
                .position(|s| s == "--parallel")
                .and_then(|i| args.sub_args.get(i + 1)?.parse::<usize>().ok());
            let timeout_secs = args
                .sub_args
                .iter()
                .position(|s| s == "--timeout")
                .and_then(|i| args.sub_args.get(i + 1)?.parse::<u64>().ok())
                .unwrap_or(600);
            let targets: Vec<DispatchTarget> = args
                .sub_args
                .iter()
                .position(|s| s == "--targets")
                .and_then(|i| {
                    let spec = args.sub_args.get(i + 1)?;
                    Some(
                        spec.split(',')
                            .filter_map(|s| DispatchTarget::from_str(s.trim()).ok())
                            .collect(),
                    )
                })
                .unwrap_or_else(|| DispatchTarget::all().to_vec());
            if targets.is_empty() {
                eprintln!("错误: --targets 未匹配到任何目标");
                std::process::exit(2);
            }
            println!(
                "[dispatch] many：目标 {} 个，并发={}，任务：{task}",
                targets.len(),
                parallel.unwrap_or(targets.len().clamp(1, 32))
            );
            let results = rt.block_on(dispatch_many(
                &targets,
                task,
                workspace,
                timeout_secs,
                parallel,
            ));
            let mut ok_n = 0usize;
            for (i, r) in results.iter().enumerate() {
                match r {
                    Ok(r) => {
                        println!(
                            "\n=== [{}/{}] {} (ok={}, {}ms) ===",
                            i + 1,
                            results.len(),
                            r.target.as_str(),
                            r.ok,
                            r.elapsed_ms
                        );
                        render_dispatch_result(r, args.json);
                        if r.ok {
                            ok_n += 1;
                        }
                    }
                    Err(e) => eprintln!("[dispatch] 内部错误（未启动）: {e}"),
                }
            }
            println!("\n[dispatch] 汇总：{ok_n}/{} 目标成功", results.len());
            if ok_n != results.len() {
                std::process::exit(3);
            }
        }
        other => {
            eprintln!("错误: dispatch 未知动作 '{other}'");
            print_subcommand_help("dispatch");
            std::process::exit(2);
        }
    }
    Ok(())
}

/// 渲染单条 DispatchResult（text / --json 两种视图）
fn render_dispatch_result(r: &crate::harness::dispatch::DispatchResult, json: bool) {
    if json {
        match serde_json::to_string_pretty(&r) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("JSON 序列化失败: {e}"),
        }
        return;
    }
    println!("target   : {}", r.target.as_str());
    println!("run_id   : {}", r.run_id);
    println!("ok       : {}", r.ok);
    println!(
        "exit     : {}",
        r.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!("elapsed  : {}ms", r.elapsed_ms);
    println!("timeout  : {}", r.timeout_killed);
    if let Some(reason) = &r.unavailable_reason {
        println!("unavail  : {reason}");
    }
    let show_out = if r.output.len() <= 6000 {
        r.output.as_str()
    } else {
        &r.output[..6000]
    };
    println!("----- output -----");
    println!("{show_out}");
    if r.output.len() > 6000 {
        println!("...（已截断到 6000 字节，共 {} 字节）", r.output.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::argv::CliMode;

    #[test]
    fn test_program_new_registers_default_routes() {
        let program = Program::new();
        let names: Vec<&str> = program
            .route_table()
            .routes()
            .iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "doctor",
                "model",
                "mcp",
                "skills",
                "session",
                "exec",
                "sandbox",
                "subagent",
                "team",
                "browser",
                "deploy",
                "serve",
                "chat",
                "gateway",
                "goal",
                "capabilities",
                "cache",
                "dispatch",
                "games",
                "export-diagnostics"
            ]
        );
    }

    /// P0-D 缺陷 2 回归：`--help` 的「资源（子命令）」区块必须与 `SUBCOMMANDS`
    /// **逐项 + 按序**一致。历史缺陷：注册 19 个路由但帮助只列 14 个，
    /// `sandbox / subagent / team / browser / capabilities` 用户从帮助里发现不了。
    #[test]
    fn test_top_help_lists_every_subcommand() {
        let help = render_top_help();
        let listed = parse_help_subcommand_names(&help);
        let expected: Vec<String> = SUBCOMMANDS.iter().map(|s| s.to_string()).collect();

        let missing: Vec<&String> = expected.iter().filter(|s| !listed.contains(s)).collect();
        assert!(
            missing.is_empty(),
            "print_top_help() 缺少子命令 {missing:?}\n帮助输出:\n{help}"
        );
        let extra: Vec<&String> = listed.iter().filter(|s| !expected.contains(s)).collect();
        assert!(
            extra.is_empty(),
            "print_top_help() 列出了未在 SUBCOMMANDS 注册的子命令 {extra:?}"
        );
        assert_eq!(
            listed.len(),
            SUBCOMMANDS.len(),
            "帮助清单 {} 项 != 路由 {} 项",
            listed.len(),
            SUBCOMMANDS.len()
        );
        assert_eq!(listed, expected, "帮助清单顺序应与 SUBCOMMANDS 一致");

        // 历史回归钉：这 5 项曾被漏列，任何一项再次消失都必须失败。
        for name in ["sandbox", "subagent", "team", "browser", "capabilities"] {
            assert!(
                listed.iter().any(|l| l == name),
                "历史漏列项 {name} 未出现在帮助中"
            );
        }
    }

    /// 帮助清单 ↔ 路由注册表同源（路由三处同步的第三条腿：
    /// `argv.rs::SUBCOMMANDS` / `route_table.register` / `print_top_help`）。
    #[test]
    fn test_top_help_matches_route_table() {
        use std::collections::BTreeSet;
        let program = Program::new();
        let routes: BTreeSet<String> = program
            .route_table()
            .routes()
            .iter()
            .map(|r| r.name.to_string())
            .collect();
        let listed: BTreeSet<String> = parse_help_subcommand_names(&render_top_help())
            .into_iter()
            .collect();
        assert_eq!(
            listed, routes,
            "帮助清单与路由注册表不一致（帮助 {:?} / 路由 {:?}）",
            listed, routes
        );
    }

    #[test]
    fn test_route_table_matches_subcommand() {
        let program = Program::new();
        let args = Args {
            mode: CliMode::Subcommand("doctor".to_string()),
            prompt: None,
            file: None,
            pipe: false,
            sub_args: vec![],
            ..Args::default()
        };
        let route = program.route_table().route(&args);
        assert!(route.is_some());
        assert_eq!(route.unwrap().name, "doctor");

        let unknown = Args {
            mode: CliMode::Subcommand("nope".to_string()),
            ..Args::default()
        };
        assert!(program.route_table().route(&unknown).is_none());
    }

    #[test]
    fn test_route_table_returns_none_for_non_subcommand() {
        let program = Program::new();
        let args = Args::default(); // Interactive
        assert!(program.route_table().route(&args).is_none());
        let direct = Args {
            mode: CliMode::Direct,
            prompt: Some("x".to_string()),
            ..Args::default()
        };
        assert!(program.route_table().route(&direct).is_none());
    }

    #[test]
    fn test_handlers_succeed() {
        let args = Args::default();
        let config = Config::default();
        assert!(doctor_handler(&args, &config).is_ok());
        assert!(model_handler(&args, &config).is_ok());
        assert!(mcp_handler(&args, &config).is_ok());
        assert!(skills_handler(&args, &config).is_ok());
        assert!(session_handler(&args, &config).is_ok());
        // serve --health 为非阻塞探测，必须成功返回（不启动长驻 listener）
        let serve_args = Args {
            mode: CliMode::Subcommand("serve".to_string()),
            health: true,
            ..Args::default()
        };
        assert!(serve_handler(&serve_args, &config).is_ok());
        // gateway --health 同理：非阻塞探测
        let gateway_args = Args {
            mode: CliMode::Subcommand("gateway".to_string()),
            health: true,
            ..Args::default()
        };
        assert!(gateway_handler(&gateway_args, &config).is_ok());
        // mcp tools：无配置服务器也应成功（打印“未连接”而非 panic）
        let mcp_tools = Args {
            mode: CliMode::Subcommand("mcp".to_string()),
            sub_args: vec!["tools".to_string()],
            ..Args::default()
        };
        assert!(mcp_handler(&mcp_tools, &config).is_ok());
        // games：无参/列出/玩法均不依赖外部状态，必须成功
        assert!(games_handler(&Args::default(), &config).is_ok());
        let games_play = Args {
            mode: CliMode::Subcommand("games".to_string()),
            sub_args: vec![
                "play".to_string(),
                "tictactoe".to_string(),
                "0".to_string(),
                "3".to_string(),
                "1".to_string(),
                "4".to_string(),
                "2".to_string(),
            ],
            ..Args::default()
        };
        assert!(games_handler(&games_play, &config).is_ok());
    }

    #[test]
    fn test_doctor_json_runs() {
        let args = Args {
            mode: CliMode::Subcommand("doctor".to_string()),
            json: true,
            ..Args::default()
        };
        let config = Config::default();
        // 结构化输出契约：--json 必须成功产出（字段由 doctor_handler 构造）
        assert!(doctor_handler(&args, &config).is_ok());
    }

    /// P0-D 缺陷 3：`api_key` 项必须自述「仅存在性、未校验有效性」，
    /// 不得再出现「已配置」这种会被读成「可用」的裸表述。
    #[test]
    fn test_api_key_check_is_honest_about_presence_only() {
        let mut config = Config::default();

        config.model.api_key = "sk-live-fixture".to_string();
        let present = api_key_check(&config);
        assert_eq!(present.state, CheckState::Ok);
        assert!(
            present.detail.contains("仅存在性") && present.detail.contains("未校验有效性"),
            "已配置分支必须写明未校验有效性，实际: {}",
            present.detail
        );
        assert!(
            present.hint.contains("--probe"),
            "已配置分支必须给出 --probe 指引，实际: {}",
            present.hint
        );

        // 纯空白 = 未配置（与 needs_offline_echo 同判据）
        config.model.api_key = "   ".to_string();
        let blank = api_key_check(&config);
        assert_eq!(blank.state, CheckState::Fail);
        assert!(blank.detail.contains("未配置"), "实际: {}", blank.detail);

        config.model.api_key = String::new();
        assert_eq!(api_key_check(&config).state, CheckState::Fail);
    }

    /// P0-D 缺陷 3：默认路径（未加 `--probe`）**必须**离线跳过鉴权探测，
    /// 既不计通过也不计失败——把「没查」粉饰成 `[OK]` 正是原缺陷。
    #[test]
    fn test_auth_probe_check_skips_without_probe_flag() {
        let mut config = Config::default();
        config.model.api_key = "sk-live-fixture".to_string();
        config.model.base_url = "https://api.invalid.example/v1".to_string();

        let skipped = auth_probe_check(&config, false);
        assert_eq!(skipped.state, CheckState::Skip, "未加 --probe 必须 Skip");
        assert!(
            skipped.detail.contains("未探测"),
            "Skip 详情须说明未探测，实际: {}",
            skipped.detail
        );
        assert!(
            skipped.hint.contains("--probe"),
            "Skip 提示须给出开启方式，实际: {}",
            skipped.hint
        );

        // 无密钥：同样 Skip（由 api_key 项负责 FAIL），并给出配置指引
        config.model.api_key = String::new();
        let no_key = auth_probe_check(&config, true);
        assert_eq!(no_key.state, CheckState::Skip);
        assert!(no_key.hint.contains("MARGUERITE_API_KEY"));
    }

    /// P0-D 缺陷 3：`--probe` 时对真实 HTTP 状态码判定真伪。
    /// 401/403 → FAIL（密钥被拒）；2xx → OK。用本地 mock 监听器回明确状态码，
    /// 不以「连接拒绝」作确定性失败源（见 crates 内 mock_http 基建说明）。
    #[test]
    fn test_auth_probe_check_judges_key_by_http_status() {
        for (status, expect_ok) in [(200u16, true), (401u16, false), (403u16, false)] {
            let base = spawn_status_mock(status);
            let mut config = Config::default();
            config.model.api_key = "sk-live-fixture".to_string();
            config.model.base_url = base;

            let check = auth_probe_check(&config, true);
            if expect_ok {
                assert_eq!(
                    check.state,
                    CheckState::Ok,
                    "HTTP {status} 应为有效密钥，实际: {} / {}",
                    check.detail,
                    check.hint
                );
            } else {
                assert_eq!(
                    check.state,
                    CheckState::Fail,
                    "HTTP {status} 应判密钥被拒，实际: {}",
                    check.detail
                );
                assert!(
                    check.detail.contains("401/403"),
                    "详情须标注鉴权失败，实际: {}",
                    check.detail
                );
            }
            // 任何分支都不得回显密钥明文
            assert!(!check.detail.contains("sk-live-fixture"));
            assert!(!check.hint.contains("sk-live-fixture"));
        }
    }

    /// 起一个只回固定状态码的 loopback mock（独立线程 + 独立 runtime，
    /// 因为被测函数是同步的、内部自建 runtime）。
    fn spawn_status_mock(status: u16) -> String {
        use tokio::io::AsyncWriteExt;
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("mock runtime");
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("mock bind");
                let addr = listener.local_addr().expect("mock addr");
                let _ = tx.send(format!("http://{addr}"));
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let reason = match status {
                        200 => "OK",
                        401 => "Unauthorized",
                        403 => "Forbidden",
                        _ => "Error",
                    };
                    let body = format!("{{\"error\":\"fixture {status} {reason}\"}}");
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            });
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("mock 监听器未在 5s 内就绪")
    }

    #[test]
    fn test_serve_health_json_contract() {
        let args = Args {
            mode: CliMode::Subcommand("serve".to_string()),
            health: true,
            ..Args::default()
        };
        let config = Config::default();
        assert!(serve_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_subcommand_help_views() {
        let config = Config::default();
        // 各子命令无附加参数时打印帮助而非报错
        let args = Args {
            mode: CliMode::Subcommand("model".to_string()),
            ..Args::default()
        };
        assert!(model_handler(&args, &config).is_ok());
        assert!(mcp_handler(&args, &config).is_ok());
        assert!(skills_handler(&args, &config).is_ok());
        assert!(session_handler(&args, &config).is_ok());
        assert!(capabilities_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_team_route_registered() {
        // team 必须注册进路由表且可被正确匹配（验证「团队 spawn 落地」表面已接线）。
        let program = Program::new();
        let args = Args {
            mode: CliMode::Subcommand("team".to_string()),
            sub_args: vec!["实现一个登录功能".to_string()],
            ..Args::default()
        };
        let route = program.route_table().route(&args);
        assert!(route.is_some(), "team 路由应已注册");
        assert_eq!(route.unwrap().name, "team");
    }

    #[test]
    fn test_team_handler_prints_usage_when_no_goal() {
        // 无目标时 team_handler 应打印用法并成功返回（不启动 tokio runtime、不 spawn），
        // 保证离线 / 误用场景下表面健壮、不 panic。
        let args = Args {
            mode: CliMode::Subcommand("team".to_string()),
            ..Args::default()
        };
        let config = Config::default();
        assert!(team_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_program_default() {
        let program = Program::default();
        // 路由数守卫：新增/删除子命令时必须同步这里（S6 加了
        // `export-diagnostics`，19 → 20）。这一条与 `SUBCOMMANDS`
        // / `print_top_help` 的同步测试互为交叉检查。
        assert_eq!(program.route_table().routes().len(), 20);
    }

    #[test]
    fn test_capabilities_handler_lists_and_routes() {
        let config = Config::default();
        // 无附加参数：打印全量能力，应成功
        let args = Args {
            mode: CliMode::Subcommand("capabilities".to_string()),
            ..Args::default()
        };
        assert!(capabilities_handler(&args, &config).is_ok());

        // --json：结构化输出成功
        let args_json = Args {
            mode: CliMode::Subcommand("capabilities".to_string()),
            json: true,
            ..Args::default()
        };
        assert!(capabilities_handler(&args_json, &config).is_ok());

        // 路由表应匹配 capabilities 子命令
        let program = Program::default();
        let route = program.route_table().route(&args);
        assert!(route.is_some());
        assert_eq!(route.unwrap().name, "capabilities");
    }

    #[test]
    fn test_cache_handler_stats_and_routes() {
        let config = Config::default();
        // stats：无会话依赖，应成功（文本 + JSON 两种输出）
        let args = Args {
            mode: CliMode::Subcommand("cache".to_string()),
            sub_args: vec!["stats".to_string()],
            ..Args::default()
        };
        assert!(cache_handler(&args, &config).is_ok());
        let args_json = Args {
            mode: CliMode::Subcommand("cache".to_string()),
            sub_args: vec!["stats".to_string()],
            json: true,
            ..Args::default()
        };
        assert!(cache_handler(&args_json, &config).is_ok());

        // 路由表应匹配 cache 子命令
        let program = Program::default();
        let route = program.route_table().route(&args);
        assert!(route.is_some());
        assert_eq!(route.unwrap().name, "cache");
    }

    #[test]
    fn test_cache_no_subcommand_shows_help() {
        let config = Config::default();
        let args = Args {
            mode: CliMode::Subcommand("cache".to_string()),
            ..Args::default()
        };
        assert!(cache_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_direct_without_prompt_fails() {
        let program = Program::default();
        let args = Args {
            mode: CliMode::Direct,
            prompt: None,
            ..Args::default()
        };
        assert!(program.handle_direct(&args).is_err());
    }

    // ===== P4：team 终态退出码（完成门禁未放行 → 非零） =====

    /// 门禁放行 → 常规成功（无特殊退出码）；未放行 → `Conflict(9)`。
    ///
    /// 锁定「跑完了但没做成」必须与「成功」在退出码上可区分：
    /// 真机现象是子专家 400 失败、首席答"未能完成"，而进程仍以 0 退出。
    #[test]
    fn test_team_failure_exit_code_reflects_completion_verdict() {
        assert_eq!(
            team_failure_exit_code(true),
            None,
            "门禁放行时不得改写退出码（应由 Ok(()) 正常返回 0）"
        );
        let blocked = team_failure_exit_code(false).expect("未放行必须有非零退出码");
        assert_eq!(blocked.code(), 9, "复用既有分层退出码 Conflict=9");
    }

    // ===== P0-B：无密钥离线回显（退出码 0，不伪造模型回答） =====

    /// 无调用链模板的输入（Question/Unknown 意图）才会落到离线回显分支。
    fn prompt_without_chain_template() -> &'static str {
        "你好，介绍一下你自己"
    }

    fn program_with_key(api_key: &str) -> Program {
        let mut program = Program::default();
        let mut config = Config::default();
        config.model.api_key = api_key.to_string();
        program.config = Some(config);
        program
    }

    #[test]
    fn test_needs_offline_echo_gate() {
        // 缺失 / 纯空白 → 离线；有真实密钥 → 走真实模型（不降级）
        let mut config = Config::default();
        config.model.api_key = String::new();
        assert!(needs_offline_echo(&config), "空密钥必须走离线回显");
        config.model.api_key = "   ".to_string();
        assert!(needs_offline_echo(&config), "纯空白密钥必须走离线回显");
        config.model.api_key = "sk-live".to_string();
        assert!(!needs_offline_echo(&config), "有密钥不得降级为离线回显");
    }

    #[test]
    fn test_handle_direct_offline_echo_without_key_returns_ok() {
        let prompt = prompt_without_chain_template();
        // 前提锁定：该输入没有调用链模板，因此必然进入「离线回显 vs 真实模型」这一段。
        let intent = crate::intent::classifier::Classifier::new()
            .classify(prompt)
            .intent;
        assert!(
            crate::chain::executor::ChainExecutor::template_for_intent(&intent).is_none(),
            "前提失效：{prompt:?} 命中调用链模板（intent={intent:?}），本用例需换输入"
        );

        let program = program_with_key("");
        let args = Args {
            mode: CliMode::Direct,
            prompt: Some(prompt.to_string()),
            ..Args::default()
        };
        // 无密钥：不再 Err/退出码 1，而是正常降级（Ok = 退出码 0）
        assert!(
            program.handle_direct(&args).is_ok(),
            "无密钥时必须走离线回显并正常返回"
        );
    }

    #[test]
    fn test_handle_direct_offline_echo_whitespace_key_returns_ok() {
        // 纯空白密钥与空密钥同语义（真实调用必失败），也必须降级而不是报错退出。
        let program = program_with_key("  \t ");
        let args = Args {
            mode: CliMode::Direct,
            prompt: Some(prompt_without_chain_template().to_string()),
            ..Args::default()
        };
        assert!(program.handle_direct(&args).is_ok());
    }

    #[test]
    fn test_handle_direct_offline_echo_json_contract() {
        // --json 下离线结果同样走 {ok,data,meta} 契约（此处只校验不 panic 且返回 Ok）
        let program = program_with_key("");
        let args = Args {
            mode: CliMode::Direct,
            prompt: Some(prompt_without_chain_template().to_string()),
            json: true,
            ..Args::default()
        };
        assert!(program.handle_direct(&args).is_ok());
    }

    #[test]
    fn test_render_offline_echo_has_input_analysis_and_config_hint() {
        let prompt = "帮我把这个目录按类型整理";
        let analysis = Orchestrator::new(Config::default()).analyze(prompt);
        let text = render_offline_echo(prompt, &analysis, Path::new("C:/tmp/config.yaml"));

        assert!(text.contains("[OFFLINE]"), "必须有离线标记: {text}");
        assert!(text.contains(prompt), "必须回显原始输入: {text}");
        assert!(
            text.contains(analysis.strategy.label()),
            "必须给出编排形态: {text}"
        );
        assert!(
            text.contains(analysis.intent.task_label()),
            "必须给出意图: {text}"
        );
        assert!(text.contains("未配置模型密钥"), "必须说明未配置密钥: {text}");
        assert!(text.contains("model add"), "必须给出配置方法: {text}");
        assert!(text.contains("MARGUERITE_API_KEY"), "必须给出环境变量法: {text}");
        assert!(text.contains("C:/tmp/config.yaml"), "必须给出配置文件路径: {text}");
        // 诚实性：必须显式声明未调用模型，且不出现任何"模型回答"式伪造
        assert!(text.contains("未调用任何模型"), "必须声明未调用模型: {text}");
        assert!(!text.contains("回答:"), "不得伪造模型回答: {text}");
    }

    #[test]
    fn test_render_offline_echo_with_auth_hint_is_honest() {
        // P0-1d：鉴权失败降级复用离线回显，但首行必须如实说「鉴权失败」，
        // 不得谎称「未配置模型密钥」（诚实性边界）。
        let prompt = "帮我把这个目录按类型整理";
        let analysis = Orchestrator::new(Config::default()).analyze(prompt);
        let text = render_offline_echo_with_hint(
            prompt,
            &analysis,
            Path::new("C:/tmp/config.yaml"),
            OFFLINE_ECHO_AUTH_HINT,
        );
        assert!(text.contains("[OFFLINE]"), "必须保留离线标记: {text}");
        assert!(text.contains("鉴权失败"), "必须说明鉴权失败: {text}");
        assert!(
            !text.contains("未配置模型密钥"),
            "鉴权失败不得谎称未配置密钥: {text}"
        );
        assert!(text.contains("未调用任何模型"), "必须声明未调用模型: {text}");
    }

    /// P0-1d 护栏（变异自检配套）：表驱动锁死「鉴权失败 → 离线降级」分支条件。
    ///
    /// 背景（QA 变异 M6）：此前条件内联在 `handle_direct`，改成永不相等的字面量后
    /// 全部既有测试仍绿。现把条件收敛到 [`should_offline_echo_on_auth_failure`]，
    /// 本测试保证「AUTH_FAILED → true，其余全部 → false」——把判定改掉必然变红。
    #[test]
    fn test_auth_failure_offline_echo_branch_condition() {
        const CASES: &[(&str, bool)] = &[
            ("AUTH_FAILED", true),
            ("RATE_LIMITED", false),
            ("NETWORK", false),
            ("SERVER", false),
            ("OTHER", false),
        ];
        for (code, expected) in CASES {
            assert_eq!(
                should_offline_echo_on_auth_failure(Some(code)),
                *expected,
                "错误码 {code} 的降级判定不符"
            );
        }
        // 无归类错误（如编排层自身错误）不触发鉴权降级。
        assert!(
            !should_offline_echo_on_auth_failure(None),
            "None 不得触发鉴权失败降级"
        );
    }

    /// P0-1d 行为护栏：用真实构造的 401 归类错误驱动 `handle_direct` Err 分支
    /// 所用的决策函数 [`auth_failure_echo_hint`]，并锁死三条行为语义——
    /// ① 走离线回显且提示「鉴权失败」（不谎称「未配置」）；
    /// ② `--json` 契约下 `offline: true`（正常降级路径，非错误）；
    /// ③ 上游原始正文（`****gEni`）不得出现在任何用户可见输出中（锁 P0-4 成果）。
    #[test]
    fn test_auth_failure_branch_drives_offline_echo_and_hides_upstream_body() {
        // 与真实 DeepSeek 401 同形的上游正文（P0-4 场景复刻）。
        let raw_upstream = "Your api key: ****gEni is invalid";
        let err = anyhow::Error::new(crate::model::ProviderError::from_status(401, raw_upstream));

        // 分支决策：AUTH_FAILED → 专用提示行（这正是 Err 分支的分流条件）。
        let hint = auth_failure_echo_hint(&err).unwrap_or_else(|| {
            panic!("AUTH_FAILED 错误必须触发鉴权失败离线降级分支");
        });
        assert_eq!(hint, OFFLINE_ECHO_AUTH_HINT);

        let prompt = prompt_without_chain_template();
        let analysis = Orchestrator::new(Config::default()).analyze(prompt);
        let path = Path::new("C:/tmp/config.yaml");

        // 语义①：文本输出如实说明「鉴权失败」，不得谎称「未配置」。
        let text = render_offline_echo_with_hint(prompt, &analysis, path, hint);
        assert!(text.contains("鉴权失败"), "必须如实说明鉴权失败: {text}");
        assert!(
            !text.contains("未配置模型密钥"),
            "鉴权失败不得谎称未配置密钥: {text}"
        );

        // 语义②：--json 契约 offline: true，且 hint 同步为鉴权失败文案。
        let payload = offline_echo_payload_with_hint(prompt, &analysis, path, hint);
        assert_eq!(payload["offline"], serde_json::json!(true));
        assert_eq!(payload["hint"], serde_json::json!(OFFLINE_ECHO_AUTH_HINT));
        let response = payload["response"].as_str().expect("response 必须是字符串");
        assert!(response.contains("鉴权失败"), "JSON response 必须如实: {response}");

        // 语义③：上游原始正文不得泄漏到用户可见输出（文本与 JSON 均查）。
        assert!(!text.contains("****gEni"), "文本输出不得回显上游正文: {text}");
        let payload_str = payload.to_string();
        assert!(
            !payload_str.contains("****gEni"),
            "JSON 输出不得回显上游正文: {payload_str}"
        );
    }

    /// 反向护栏：非 AUTH_FAILED 的归类错误不得触发离线降级——否则限流 / 网络抖动
    /// 也会被静默降级成离线回显，掩盖真实故障。
    #[test]
    fn test_non_auth_failures_do_not_trigger_offline_echo() {
        for status in [429_u16, 500, 503, 400] {
            let err = anyhow::Error::new(crate::model::ProviderError::from_status(
                status,
                "upstream body",
            ));
            assert!(
                auth_failure_echo_hint(&err).is_none(),
                "HTTP {status} 不得触发鉴权失败降级分支"
            );
        }
    }

    #[test]
    fn test_offline_echo_payload_json_contract() {
        let prompt = "帮我把这个目录按类型整理";
        let analysis = Orchestrator::new(Config::default()).analyze(prompt);
        let payload = offline_echo_payload(prompt, &analysis, Path::new("C:/tmp/config.yaml"));

        assert_eq!(payload["offline"], serde_json::json!(true));
        assert_eq!(payload["prompt"], serde_json::json!(prompt));
        assert_eq!(
            payload["analysis"]["strategy_label"],
            serde_json::json!(analysis.strategy.label())
        );
        assert_eq!(
            payload["analysis"]["subtask_count"],
            serde_json::json!(analysis.subtasks.len())
        );
        assert_eq!(
            payload["config_path"],
            serde_json::json!("C:/tmp/config.yaml")
        );
        let response = payload["response"].as_str().expect("response 必须是字符串");
        assert!(response.contains(prompt));
        assert!(response.contains("未配置模型密钥"));
    }

    #[test]
    fn test_print_offline_echo_returns_ok_for_text_and_json() {
        let config = Config::default();
        for json in [false, true] {
            let args = Args {
                mode: CliMode::Direct,
                json,
                ..Args::default()
            };
            assert!(
                print_offline_echo(&args, &config, "hello").is_ok(),
                "离线回显必须正常返回（json={json}）"
            );
        }
    }

    #[test]
    fn test_model_list_lists_rules() {
        let config = Config::default();
        let args = Args {
            mode: CliMode::Subcommand("model".to_string()),
            sub_args: vec!["list".to_string()],
            ..Args::default()
        };
        assert!(model_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_mcp_list_with_servers() {
        let mut config = Config::default();
        config
            .mcp
            .servers
            .push(crate::persistence::config::McpServerConfig {
                name: "fs".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "mcp-server-fs".to_string()],
                env: Default::default(),
                url: None,
            });
        let args = Args {
            mode: CliMode::Subcommand("mcp".to_string()),
            sub_args: vec!["list".to_string()],
            ..Args::default()
        };
        assert!(mcp_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_skills_install_missing_url() {
        let config = Config::default();
        let args = Args {
            mode: CliMode::Subcommand("skills".to_string()),
            sub_args: vec!["install".to_string()],
            ..Args::default()
        };
        assert!(skills_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_session_delete_missing_id() {
        let config = Config::default();
        let args = Args {
            mode: CliMode::Subcommand("session".to_string()),
            sub_args: vec!["delete".to_string()],
            ..Args::default()
        };
        assert!(session_handler(&args, &config).is_ok());
    }

    #[test]
    fn test_direct_with_missing_file_warns() {
        let program = Program::default();
        let args = Args {
            mode: CliMode::Direct,
            prompt: Some("解释这个文件".to_string()),
            file: Some(std::path::PathBuf::from("__nonexistent__.rs")),
            ..Args::default()
        };
        // 文件不存在 → 警告后继续（无 API Key 时提示并正常返回）
        assert!(program.handle_direct(&args).is_ok());
    }
}
