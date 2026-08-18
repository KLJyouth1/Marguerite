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
use crate::orchestration::{EngineInvoker, Orchestrator};
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
        route_table.register(
            "capabilities",
            capabilities_handler,
            "能力自检（真实落地状态）",
        );
        route_table.register("cache", cache_handler, "KV 缓存命中率与可缓存比");

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
    fn handle_direct(&self, args: &Args) -> Result<()> {
        let raw_prompt = if args.pipe {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
            input.trim().to_string()
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

        // 创建 tokio runtime 执行：优先调用链，回退 Agent
        let config = Config::load()?;
        let rt = tokio::runtime::Runtime::new()?;
        let result = rt.block_on(async {
            if let Some(chain_output) = crate::chain::cli::run_direct_chain(
                &config,
                &raw_prompt,
                args.file.as_deref(),
                args.agent.as_deref(),
            )
            .await
            {
                return Ok::<String, anyhow::Error>(chain_output);
            }
            if config.model.api_key.is_empty() {
                return Err(anyhow::anyhow!(
                    "未配置 API Key，请在 {} 中设置 model.api_key",
                    Config::config_path().display()
                ));
            }
            let mut runtime = AgentRuntime::new(config);
            runtime.init()?;
            let resp = runtime.process_input(&prompt).await?;
            Ok::<String, anyhow::Error>(resp.to_string())
        });

        match result {
            Ok(resp) => {
                if args.json {
                    print_json_ok(serde_json::json!({ "response": resp }));
                } else {
                    println!("{resp}");
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
    println!("Marguerite — 终端智能体工作台 (v{CLI_VERSION})");
    println!();
    println!("用法:");
    println!("  marguerite                        启动交互式工作台（默认）");
    println!("  marguerite -p \"<指令>\"           单指令模式，执行后退出");
    println!("  marguerite --pipe                从标准输入读取指令并执行后退出");
    println!("  marguerite <资源> <动作>          运行子命令");
    println!("  marguerite --help | -h           显示本帮助");
    println!("  marguerite --version | -V        显示版本");
    println!();
    println!("资源（子命令）:");
    println!("  doctor    系统诊断与环境自检");
    println!("  model     模型档案管理（list/current/use/copilot/add/remove/path）");
    println!("  mcp       MCP 服务器管理（list）");
    println!("  skills    技能管理（list/install）");
    println!("  session   会话管理（list/delete）");
    println!("  exec      沙箱命令执行（exec -- <命令>）");
    println!("  deploy    一键部署（SSH / 宝塔，--target ...）");
    println!("  serve     ACP 回环服务（serve --health | serve [--port N]）");
    println!("  chat      对话（chat 进流式 REPL；chat \"问题\" 单发流式）");
    println!("  gateway   统一网关（gateway --health | gateway [--port N]，回环 Web 控制台）");
    println!("  cache     KV 缓存命中率与可缓存比（cache stats / cache ratio）");
    println!();
    println!("示例:");
    println!("  marguerite -p \"解释当前目录的 main.rs\"");
    println!("  marguerite model list");
    println!("  marguerite session list");
    println!("  marguerite deploy --target ssh://user@host --dry-run");
    println!();
    println!("说明:");
    println!("  - 工作台由编排内核自动决定 Solo / MAX / 团队 / 集群 形态，无需手动选择。");
    println!("  - 未配置 API Key 时自动降级为离线回显。");
    println!("  - 设置 MARGUERITE_LEGACY_UI=1 可回退到旧版单栏界面（仅排障）。");
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
  marguerite doctor           运行诊断，输出环境摘要（文本）
  marguerite doctor --json    以 JSON 输出（便于脚本 / agent 接入）

示例:
  marguerite doctor
  marguerite doctor --json | jq .api_key_configured
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

示例:
  marguerite mcp list
  marguerite mcp list | cat   # 在脚本中读取服务器清单
"
        }
        "skills" => {
            "\
Marguerite skills — 技能管理

用法:
  marguerite skills list           列出已安装技能
  marguerite skills install <url>  从 YAML URL 安装技能

示例:
  marguerite skills list
  marguerite skills install https://example.com/skill.yaml
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
    let config_dir = Config::config_dir();

    // 结构化自检项：每项含 名称 / 是否通过 / 详情 / 修复建议
    let mut checks: Vec<DoctorCheck> = Vec::new();

    // 1. 配置目录
    let dir_ok = config_dir.exists();
    checks.push(DoctorCheck {
        name: "config_dir",
        ok: dir_ok,
        detail: config_dir.display().to_string(),
        hint: if dir_ok {
            String::new()
        } else {
            "首次运行 marguerite 会自动创建配置目录".to_string()
        },
    });

    // 2. 配置 schema
    let schema = validate_config_schema(config);
    checks.push(DoctorCheck {
        name: "config_schema",
        ok: schema.ok,
        detail: schema.detail,
        hint: schema.hint,
    });

    // 3. API Key
    let key_ok = !config.model.api_key.is_empty();
    checks.push(DoctorCheck {
        name: "api_key",
        ok: key_ok,
        detail: if key_ok {
            "已配置".to_string()
        } else {
            "未配置".to_string()
        },
        hint: if key_ok {
            String::new()
        } else {
            "设置环境变量 MARGUERITE_API_KEY 或在 config.yaml 的 model.api_key 中填写".to_string()
        },
    });

    // 4. 网络可达性（base_url 探测，短超时，离线安全）
    let net = probe_model_endpoint(config);
    checks.push(DoctorCheck {
        name: "network",
        ok: net.ok,
        detail: net.detail,
        hint: net.hint,
    });

    // 5. 沙箱（平台支持性，信息项，恒 ok）
    let sandbox_detail = match std::env::consts::OS {
        "linux" => "Landlock 已支持",
        "macos" => "Seatbelt 已支持",
        _ => "当前平台只读降级（Unsupported，符合安全约束）",
    };
    checks.push(DoctorCheck {
        name: "sandbox",
        ok: true,
        detail: sandbox_detail.to_string(),
        hint: String::new(),
    });

    // 6. 引擎就绪
    let engine = Orchestrator::new(config.clone());
    let engine_ok = engine.tool_count() > 0;
    checks.push(DoctorCheck {
        name: "engine",
        ok: engine_ok,
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

    let total = checks.len();
    let passed = checks.iter().filter(|c| c.ok).count();
    let failed = total - passed;
    let status = if failed == 0 { "ok" } else { "degraded" };

    if args.json {
        let data = serde_json::json!({
            "rust_arch": std::env::consts::ARCH,
            "os": std::env::consts::OS,
            "config_dir": config_dir.display().to_string(),
            "model": { "name": config.model.model_name, "provider": config.model.provider },
            "api_key_configured": key_ok,
            "mcp_servers": config.mcp.servers.len(),
            "skills_dir": config.skills.skills_dir,
            "checks": checks.iter().map(|c| serde_json::json!({
                "name": c.name, "ok": c.ok, "detail": c.detail, "hint": c.hint
            })).collect::<Vec<_>>(),
            "summary": { "total": total, "passed": passed, "failed": failed },
            "status": status
        });
        print_json_ok(data);
        return Ok(());
    }

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
        let tag = if c.ok { "[OK]" } else { "[FAIL]" };
        println!("  {tag} {:<14} {}", c.name, c.detail);
        if !c.ok && !c.hint.is_empty() {
            println!("        修复: {}", c.hint);
        }
    }
    println!();
    println!("结果: {passed}/{total} 通过 -> {status}");
    Ok(())
}

/// 单条诊断检查的结果
struct DoctorCheck {
    /// 检查项名称
    name: &'static str,
    /// 是否通过
    ok: bool,
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
            ok: true,
            detail: "schema 合法".to_string(),
            hint: String::new(),
        }
    } else {
        DoctorCheck {
            name: "config_schema",
            ok: false,
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
            ok: true,
            detail: "未配置 base_url，跳过网络探测".to_string(),
            hint: String::new(),
        };
    }
    let (host, port) = match parse_host_port(base) {
        Some(hp) => hp,
        None => {
            return DoctorCheck {
                name: "network",
                ok: false,
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
                ok: false,
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
            ok: true,
            detail: format!("TCP 可达 {host}:{port}"),
            hint: String::new(),
        },
        Ok(Err(e)) => DoctorCheck {
            name: "network",
            ok: false,
            detail: format!("TCP 不可达 {host}:{port}: {e}"),
            hint: "检查网络 / 代理 / base_url 端口".to_string(),
        },
        Err(_) => DoctorCheck {
            name: "network",
            ok: false,
            detail: format!("探测超时 {host}:{port} (3s)"),
            hint: "检查网络 / 代理 / 防火墙".to_string(),
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
        _ => {
            print_subcommand_help("mcp");
        }
    }
    Ok(())
}

/// skills — 技能管理
fn skills_handler(args: &Args, config: &Config) -> Result<()> {
    let sub = args.sub_args.first().map(String::as_str).unwrap_or("");
    let loader = SkillLoader::new(Path::new(&config.skills.skills_dir));
    match sub {
        "list" => match loader.load_all() {
            Ok(skills) if skills.is_empty() => {
                println!("未找到技能（目录: {}）", config.skills.skills_dir);
            }
            Ok(skills) => {
                println!("已安装技能 ({}):", skills.len());
                for skill in &skills {
                    println!(
                        "  {} v{} — {}",
                        skill.name, skill.version, skill.description
                    );
                }
            }
            Err(e) => eprintln!("加载技能失败: {e}"),
        },
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
                    println!("技能已安装: {} v{}", skill.name, skill.version);
                }
                Err(e) => eprintln!("安装失败: {e}"),
            }
        }
        _ => {
            print_subcommand_help("skills");
        }
    }
    Ok(())
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
            "implemented": caps.iter().filter(|c| c.maturity.is_implemented()).count(),
            "narrative": caps.iter().filter(|c| !c.maturity.is_implemented()).count(),
            "global": {
                "total": s.total,
                "exclusive": s.exclusive,
                "enhanced": s.enhanced,
                "baseline": s.baseline,
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
            "  全量 {} 项 | 已实装 {} | 纯叙事 {}",
            s.total, s.implemented, s.narrative
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
        println!("用法: marguerite subagent team \"<目标描述>\" [--reflect]");
        println!("说明: 走 DAG 分层编排（调研→规划→实现→审校），子代理真实执行。");
        println!(
            "      --reflect  启用 分阶段智能体 分阶段反思（s2_5）：实现后插入反思自检再交付。"
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
    println!("[subagent] 分解任务 DAG（Build 形态）…");
    rt.block_on(async {
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
            }
            Err(e) => {
                eprintln!("[subagent] 编排失败: {e}");
                std::process::exit(1);
            }
        }
    });
    Ok(())
}

/// team — 直接触发专家团队并行 spawn（`Team::run` 真实执行体，区别于 DAG 版 `subagent team`）
///
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
                "capabilities",
                "cache"
            ]
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
        assert_eq!(program.route_table().routes().len(), 16);
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
