//! 配置管理模块
//!
//! 借鉴 混合检索引擎 配置设计，实现 YAML 配置的加载和保存。
//! 默认配置文件路径: `~/.marguerite/config.yaml`

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// 日志级别 (trace/debug/info/warn/error)
    #[serde(default = "default_log_level")]
    pub level: String,
    /// 日志文件路径
    #[serde(default = "default_log_file")]
    pub file: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_file() -> String {
    "logs/marguerite.log".to_string()
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: default_log_file(),
        }
    }
}

/// 模型配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// 模型提供商 (openai/anthropic/deepseek 等)
    #[serde(default = "default_provider")]
    pub provider: String,
    /// 模型名称
    #[serde(default = "default_model_name")]
    pub model_name: String,
    /// API 密钥（仅内存/env 注入用；serde 序列化跳过 —— 密钥绝不落 YAML）
    #[serde(default, skip_serializing)]
    pub api_key: String,
    /// API 基础 URL
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// 副模型（Copilot，Bloom 并行编排审校用）。为空则 Bloom 退化为同模型审校。
    #[serde(default)]
    pub copilot_model: Option<SecondaryModelConfig>,
    /// 请求超时时间（秒）。缺省 60。设置较短值可让「连接级卡死」快速失败并由重试恢复。
    #[serde(default)]
    pub timeout_secs: u64,
}

/// 副模型（Copilot）配置：Bloom 并行编排时作为审校模型，与主模型区分。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecondaryModelConfig {
    /// 副模型提供商（缺省继承主模型）
    #[serde(default)]
    pub provider: Option<String>,
    /// 副模型名称
    #[serde(default)]
    pub model_name: String,
    /// 副模型 API 密钥（缺省继承主模型；serde 序列化跳过 —— 密钥绝不落 YAML）
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// 副模型 API 基础 URL（缺省继承主模型）
    #[serde(default)]
    pub base_url: Option<String>,
}

fn default_provider() -> String {
    "deepseek".to_string()
}

fn default_model_name() -> String {
    "deepseek-v4-pro".to_string()
}

fn default_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model_name: default_model_name(),
            api_key: String::new(),
            base_url: default_base_url(),
            copilot_model: None,
            timeout_secs: 0,
        }
    }
}

/// MCP 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 服务器名称
    pub name: String,
    /// 服务器命令（子进程方式启动，可选）
    #[serde(default)]
    pub command: String,
    /// 命令行参数
    #[serde(default)]
    pub args: Vec<String>,
    /// 环境变量
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// HTTP MCP 服务 URL（如 http://127.0.0.1:18799）。
    /// 配置后引擎启动时会连接并拉取工具目录，注册进工具注册表。
    #[serde(default)]
    pub url: Option<String>,
}

/// MCP 配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    /// MCP 服务器列表
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// 记忆系统配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// 感知记忆存储路径（当前会话）
    #[serde(default = "default_sensory_path")]
    pub sensory_path: String,
    /// 短期记忆存储路径
    #[serde(default = "default_short_term_path")]
    pub short_term_path: String,
    /// 记忆自动捕获开关（webui `memory.auto_capture_enabled`；纯 Rust 离线模式持久化用）
    #[serde(default = "default_true")]
    pub auto_capture_enabled: bool,
    /// 联想记忆网络（突触图）开关：开启时用扩散激活分数参与召回重排并落盘突触图；
    /// 关闭时召回行为与引入该功能前**完全一致**（无强化、无重排、不发事件）。
    #[serde(default = "default_true")]
    pub synapse_enabled: bool,
}

fn default_sensory_path() -> String {
    "memory/sensory".to_string()
}

fn default_short_term_path() -> String {
    "memory/short_term".to_string()
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            sensory_path: default_sensory_path(),
            short_term_path: default_short_term_path(),
            auto_capture_enabled: default_true(),
            synapse_enabled: default_true(),
        }
    }
}

/// 技能系统配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// 技能目录
    #[serde(default = "default_skills_dir")]
    pub skills_dir: String,
}

fn default_skills_dir() -> String {
    "skills".to_string()
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_dir: default_skills_dir(),
        }
    }
}

/// 复杂任务自动拆解与长续航配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanConfig {
    /// 聊天主路径是否自动拆解复杂任务（关掉则退回单轮执行，简单任务不受影响）
    #[serde(default = "default_auto_decompose")]
    pub auto_decompose: bool,
    /// 单次自主执行的最大步骤数（防失控，超限自动停止）
    #[serde(default = "default_plan_max_steps")]
    pub max_steps: usize,
    /// 每执行 N 步保存一次检查点（供续跑/恢复）
    #[serde(default = "default_checkpoint_interval_steps")]
    pub checkpoint_interval_steps: usize,
    /// 长时任务心跳间隔（秒）
    #[serde(default = "default_heartbeat_interval_s")]
    pub heartbeat_interval_s: i64,
    /// 计划单步执行的**总**墙钟预算（秒）。0 = 使用默认（120s）。
    /// v3.2：可配置化——真实 LLM（DeepSeek v4 等）+ 工具回路单步常超 30s，
    /// 旧硬编码 30s 导致复杂任务自动拆解第一步即失败。
    #[serde(default)]
    pub step_timeout_secs: u64,
}

fn default_auto_decompose() -> bool {
    true
}

fn default_plan_max_steps() -> usize {
    12
}

fn default_checkpoint_interval_steps() -> usize {
    1
}

fn default_heartbeat_interval_s() -> i64 {
    30
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            auto_decompose: default_auto_decompose(),
            max_steps: default_plan_max_steps(),
            checkpoint_interval_steps: default_checkpoint_interval_steps(),
            heartbeat_interval_s: default_heartbeat_interval_s(),
            step_timeout_secs: 0, // 0 = runner 使用内置默认 120s
        }
    }
}

/// 行为命名配置（webui 设置页 Behavior 分节；纯 Rust 离线模式持久化用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingConfig {
    /// 会话标题自动生成开关（webui `behaviorForm.patches()` 写 `naming.enabled`）
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
        }
    }
}

/// 隐私配置（webui 设置页 Privacy 分节；纯 Rust 离线模式持久化用）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrivacyConfig {
    /// 禁用网络遥测/观测（webui `privacy.disable_network_observability`）
    #[serde(default)]
    pub disable_network_observability: bool,
}

/// 控制台界面配置（JSON 键 `control_ui.*`）
///
/// 与 pyengine 的 `[control_ui]`（`gateway/config.py`）对齐：`default_locale` 是
/// 「浏览器无本地偏好时首屏渲染的语言」+「渠道固定通知的网关级语言」。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ControlUiConfig {
    /// 网关级默认语言（`en` / `zh-Hans` / `ja` / `fr` / `de` / `es`）。
    ///
    /// **`en` 是基线语义**（与 Python 侧 `_resolve_locale` 一致）：仅当它被显式钉成
    /// 非英语值时才强制覆盖首屏语言；否则由请求的 `Accept-Language` 决定，
    /// 都没有才落到 `en`。客户端 localStorage 与手动切换始终优先。
    #[serde(default = "default_ui_locale")]
    pub default_locale: String,
}

fn default_ui_locale() -> String {
    "en".to_string()
}

/// 网关安全配置（JSON 键 `gateway.*`）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    /// WS 握手 Origin 白名单的**追加**项（默认白名单之外的额外合法来源）。
    ///
    /// 默认白名单覆盖桌面端全部内置来源（见 `core/src/gateway/server.rs` 的
    /// `DEFAULT_WS_ORIGIN_ALLOWLIST`）：`localhost` / `127.0.0.1` / `[::1]` 的
    /// `http(s)` 任意端口、`tauri://localhost`、`http(s)://tauri.localhost`、
    /// `app://localhost`。本字段用于放行自定义桌面壳 / 反向代理来源；语义是
    /// **在默认白名单之上做加法**，无法通过它删掉默认项（防误配置把自家
    /// 桌面壳锁死在外）。
    ///
    /// 条目形如 `scheme://host[:port]`；带端口须精确匹配，不带端口接受任意端口。
    #[serde(default)]
    pub origin_allowlist: Vec<String>,
}

/// Bloom 并行编排配置（webui 模型策略/对话路由可见；默认关闭，零行为变化）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BloomConfig {
    /// 是否启用 Captain+Copilot 双模型并行推理（对话路径走 BloomOrchestrator）
    #[serde(default)]
    pub parallel_enabled: bool,
    /// 候选模型选择策略
    #[serde(default)]
    pub selection_mode: String,
    /// 全部候选失败时的策略
    #[serde(default = "default_all_failed_policy")]
    pub all_failed_policy: String,
    /// 集成候选模型配置
    #[serde(default)]
    pub candidates: serde_json::Value,
}

fn default_all_failed_policy() -> String {
    "fallback_single".to_string()
}

/// 记忆 Dream 配置（设置页 › 高级 › 记忆与自学习组）
///
/// 默认全关闭：dream/自学习均为显式 opt-in，避免后台训练消耗 API Key。
/// 「高级选项」勾选后，self-learning 开关开启时 dream.enabled/auto_schedule
/// 会同步打开（config.patch.safe 返回 `linked: [...]`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DreamConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auto_schedule: bool,
    #[serde(default)]
    pub cadence_ms: u64,
}

/// 路由自学习配置（设置页 › 高级 › 记忆与自学习组）
///
/// 真训练由 Python 侧车 + dream cadence 提供；Rust 离线模式保留「启用但无数据/
/// 不训练」基线（self-learning 状态 RPC 显示 disabled/no_data）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RouterSelfLearningConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// 嵌入后端配置（JSON 键 embeddings.*）
///
/// - `backend = "local"`（默认）：零依赖特征哈希（`LocalEmbedding`），离线、无需模型权重。
/// - `backend = "model"`：真实语义模型后端（需 `real-embed` feature 编译进引擎；加载失败自动回退 local）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingsConfig {
    /// 后端选择："local" | "model"
    #[serde(default = "default_embedding_backend")]
    pub backend: String,
    /// 模型标识（如 "all-minilm-l6-v2"）；留空用后端默认模型
    #[serde(default)]
    pub model: Option<String>,
}

fn default_embedding_backend() -> String {
    "local".to_string()
}

/// 模型路由模式配置（JSON 键 model_routing.mode）
///
/// 由 webui 模型路由切换器（`models.routing.set`）写入，持久化到 config.yaml。
/// 取值：direct（单模型直连）/ router（Squilla 路由）/ ensemble（LLM 集成）/ bloom（Bloom 并行编排）。
/// 此前该 RPC 仅由 Python 侧车实现（`PY_ENGINE_PREFIXES` 含 `models.routing.`），
/// 侧车离线时返回 NOT_IMPLEMENTED，导致「切换模型路由失败，请检查网关连接」。
/// 现改为 Rust 原生实现，无需侧车即可切换模型路由模式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoutingConfig {
    #[serde(default = "default_routing_mode")]
    pub mode: String,
    /// 多模型融合时的汇总模型
    #[serde(default)]
    pub fusion_model: String,
    /// 候选模型列表（多模型融合/智能路由）
    #[serde(default)]
    pub candidate_models: Vec<String>,
    /// 固定模型（mode=disabled 时使用）
    #[serde(default)]
    pub fixed_model: String,
    /// 跨供应商分层路由
    #[serde(default)]
    pub cross_provider_tiers: bool,
}

fn default_routing_mode() -> String {
    "direct".to_string()
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            mode: default_routing_mode(),
            fusion_model: String::new(),
            candidate_models: Vec::new(),
            fixed_model: String::new(),
            cross_provider_tiers: false,
        }
    }
}

/// 运维只读主机表配置（`ops.hosts`）。
///
/// 主机条目复用 [`crate::ops::HostRef`]（`{id, address, credential_ref}`），
/// 是 `ops.list_hosts` 的真实数据源。`credential_ref` 只存引用不存密钥明文；
/// 响应/错误边界一律脱敏（见 `gateway/ops_dispatch.rs::redact_credentials`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpsConfig {
    /// 配置的运维主机清单；空表表示未配置（`list_hosts` 返回空，不报错）。
    #[serde(default)]
    pub hosts: Vec<crate::ops::HostRef>,
}

/// 主配置结构体
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// 模型配置
    #[serde(default)]
    pub model: ModelConfig,
    /// MCP 配置
    #[serde(default)]
    pub mcp: McpConfig,
    /// 记忆系统配置
    #[serde(default)]
    pub memory: MemoryConfig,
    /// 记忆 Dream 子配置（嵌套在 memory 下，JSON 键 memory.dream.*）
    #[serde(default)]
    pub dream: DreamConfig,
    /// 路由自学习（JSON 键 squilla_router.self_learning.*）
    #[serde(default)]
    pub squilla_router: RouterSelfLearningConfig,
    /// 技能系统配置
    #[serde(default)]
    pub skills: SkillsConfig,
    /// 护栏配置
    #[serde(default)]
    pub guard: crate::guard::GuardConfig,
    /// 日志配置
    #[serde(default)]
    pub log: LogConfig,
    /// 复杂任务自动拆解与长续航配置
    #[serde(default)]
    pub plan: PlanConfig,
    /// 行为命名配置（纯 Rust 离线模式：会话标题自动生成开关）
    #[serde(default)]
    pub naming: NamingConfig,
    /// 隐私配置（纯 Rust 离线模式：网络遥测开关）
    #[serde(default)]
    pub privacy: PrivacyConfig,
    /// Bloom 并行编排配置（对话路径 Captain+Copilot 双模型并行）
    #[serde(default)]
    pub bloom: BloomConfig,
    /// 嵌入后端配置（语义检索/语义意图缓存的向量来源）
    #[serde(default)]
    pub embeddings: EmbeddingsConfig,
    /// 模型路由模式（direct/router/ensemble/bloom），由 webui 模型路由切换器写入
    #[serde(default)]
    pub model_routing: ModelRoutingConfig,
    /// 控制台界面配置（`control_ui.default_locale`）：网关托管的 console 首屏语言
    #[serde(default)]
    pub control_ui: ControlUiConfig,
    /// 网关安全配置（`gateway.origin_allowlist` 等）：WS 握手来源白名单扩展
    #[serde(default)]
    pub gateway: GatewayConfig,
    /// 运维只读主机表（`ops.hosts`）：`ops.list_hosts` 的真实数据源。
    ///
    /// 默认空表（零行为变化），配置后 `ops.*` 才拿到真实主机；见 `ops/executor.rs`。
    #[serde(default)]
    pub ops: OpsConfig,
}

/// 解析配置目录（纯函数，便于单测；顺序：`MARGUERITE_CONFIG_DIR` > 用户主目录）。
///
/// 空串视为未设置（Windows 上传空变量很常见，不能因此把配置目录解析成当前目录）。
fn resolve_config_dir(
    env_override: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = env_override.filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".marguerite")
}

/// 解析数据目录（纯函数，便于单测；顺序：`MARGUERITE_DATA_DIR` > 平台默认）。
///
/// 与 [`resolve_config_dir`] 同构：空串视为未设置。两者必须**成对**覆盖 ——
/// 只隔离 `config_dir()` 会漏掉所有走 `data_dir()` 的运行时数据
/// （`audit.db` / `credentials.json` / `gateway-token` / cache / `sidecar.json`）。
fn resolve_data_dir(
    env_override: Option<std::ffi::OsString>,
    base: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = env_override.filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    base.unwrap_or_else(|| PathBuf::from(".")).join("Marguerite")
}

impl Config {
    /// 获取默认配置目录路径（~/.marguerite/；单测可覆盖，或多实例/隔离测试用
    /// `MARGUERITE_CONFIG_DIR` 显式指定）
    pub fn config_dir() -> PathBuf {
        if let Some(dir) = crate::persistence::OVERRIDE_CONFIG_DIR.with(|c| c.borrow().clone()) {
            return dir;
        }
        // 显式环境变量覆盖：`dirs::home_dir()` 在 Windows 上走 Known Folder API，
        // **不认 USERPROFILE/HOME**。仅改 HOME 起「隔离网关」时它会照样读写真实
        // ~/.marguerite/config.yaml，既污染用户配置、又与生产 daemon 争抢同一份文件
        // （正是并发写丢更新的现场）。隔离测试/多实例部署必须用这个变量隔离。
        resolve_config_dir(
            std::env::var_os("MARGUERITE_CONFIG_DIR"),
            dirs::home_dir(),
        )
    }

    /// 覆盖配置根目录（测试辅助）。线程局部、可重复覆盖：每个用例线程注入各自的
    /// 临时目录，并行测试互不串扰（取代 OnceLock「设一次」语义，根除跨用例 flaky）。
    #[cfg(test)]
    pub fn set_config_dir_override(path: PathBuf) {
        crate::persistence::OVERRIDE_CONFIG_DIR.with(|c| *c.borrow_mut() = Some(path));
    }

    /// 测试辅助：读取当前线程的配置目录覆盖（None = 未注入）。
    /// 供依赖 `Config::data_dir()` 的持久化模块（如桌宠状态台账）在单测时
    /// 优先落到注入的临时目录，避免读写真实用户数据。
    #[cfg(test)]
    pub fn current_config_dir_override() -> Option<PathBuf> {
        crate::persistence::OVERRIDE_CONFIG_DIR.with(|c| c.borrow().clone())
    }

    /// 获取默认配置文件路径 (~/.marguerite/config.yaml)
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.yaml")
    }

    /// 获取应用数据目录（持久化数据：经验台账 / 长期记忆）
    ///
    /// 优先 `AppData/Local`（Windows）/ `~/Library/Application Support`（macOS）/
    /// `~/AppData`（Linux XDG 回退），末尾统一加 `Marguerite`，
    /// 保证跨会话持久、不落临时目录。
    ///
    /// `MARGUERITE_DATA_DIR` 可整体覆盖（空串视为未设置），与
    /// [`Config::config_dir`] 的 `MARGUERITE_CONFIG_DIR` 同构。
    /// 这是 Safe Mode（S10）的前置条件：只有 `config_dir()` 能隔离是不够的 ——
    /// `audit.db` / `credentials.json` / `gateway-token` / 各类 cache / `sidecar.json`
    /// 都经 `data_dir()` 拼路径，不覆盖就仍会读用户真实数据。
    pub fn data_dir() -> PathBuf {
        resolve_data_dir(
            std::env::var_os("MARGUERITE_DATA_DIR"),
            dirs::data_local_dir().or_else(dirs::home_dir),
        )
    }

    /// 加载配置
    ///
    /// 从 `~/.marguerite/config.yaml` 加载配置。
    /// 如果文件不存在，则创建默认配置并写入。
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();

        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Config = serde_yaml::from_str(&content)?;
            info!("配置已加载: {}", config_path.display());
            config
        } else {
            info!("配置文件不存在，创建默认配置: {}", config_path.display());
            let config = Config::default();
            config.save()?;
            config
        };

        // 安全注入：API Key / 模型 / BaseURL / 提供商 优先从环境变量与 .env 文件读取，
        // **绝不**要求把密钥写进 config.yaml（避免误提交）。优先级：进程环境变量 > .env 文件 > YAML 默认值。
        config.apply_env_overrides();
        Ok(config)
    }

    /// 从环境变量与项目根 `.env` 文件叠加模型配置覆盖。
    ///
    /// 设计铁律：密钥只走环境变量 / `.env`，不进版本库、不进 YAML 明文。
    /// 若进程环境变量已设置则直接采用；否则尝试从候选路径读取 `.env`（手动解析，
    /// 不依赖 `dotenv` 第三方库，也不调用已弃用的 `std::env::set_var`）。
    fn apply_env_overrides(&mut self) {
        use std::collections::HashMap;

        // 1) 收集 .env 文件中的覆盖（手动解析 KEY=VALUE，忽略注释与空行）
        let mut dotenv: HashMap<String, String> = HashMap::new();
        for candidate in [PathBuf::from(".env"), Self::config_dir().join(".env")] {
            if let Ok(content) = std::fs::read_to_string(&candidate) {
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some((k, v)) = line.split_once('=') {
                        let k = k.trim();
                        let v = v.trim().trim_matches('"').trim_matches('\'');
                        if !k.is_empty() {
                            dotenv.insert(k.to_string(), v.to_string());
                        }
                    }
                }
            }
        }

        // 2) 解析单个键：进程环境变量优先，其次 .env
        let resolve = |key: &str| -> Option<String> {
            if let Ok(v) = std::env::var(key) {
                if !v.is_empty() {
                    return Some(v);
                }
            }
            dotenv.get(key).filter(|v| !v.is_empty()).cloned()
        };

        // 3) 叠加到模型配置（仅在非空时覆盖，保留 YAML 已有值）
        if let Some(v) = resolve("MARGUERITE_API_KEY") {
            self.model.api_key = v;
        }
        if let Some(v) = resolve("MARGUERITE_MODEL") {
            self.model.model_name = v;
        }
        if let Some(v) = resolve("MARGUERITE_BASE_URL") {
            self.model.base_url = v;
        }
        if let Some(v) = resolve("MARGUERITE_PROVIDER") {
            self.model.provider = v;
        }

        if !self.model.api_key.is_empty() {
            info!(
                "模型后端已启用：provider={} model={}（密钥来源：环境变量/.env，仅驻内存，save() 不落 YAML）",
                self.model.provider, self.model.model_name
            );
        }
    }

    /// 保存配置到默认路径
    ///
    /// E3/K1 凭证护栏：`api_key` 带 `#[serde(skip_serializing)]`，本方法**结构性**
    /// 不可能把密钥写进 YAML（即使内存中已由 env/.env/补丁注入）。
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path();
        self.save_to(&config_path)
    }

    /// 保存配置到指定路径（同样受 `skip_serializing` 护栏保护，密钥不落盘）
    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        // 确保目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_yaml::to_string(self)?;
        debug_assert!(
            !content.contains("api_key"),
            "config.yaml 序列化产物不得包含 api_key 字段"
        );
        std::fs::write(path, content)?;
        info!("配置已保存: {}", path.display());
        Ok(())
    }

    /// 把 API Key 写入配置目录 `.env`（MARGUERITE_API_KEY=...），返回 .env 路径。
    ///
    /// E3/K1 凭证护栏的配套写入侧：`config.patch.safe` 收到 `model.api_key` 时
    /// 调用本方法转写 `.env`（密钥唯一合法落盘位置），YAML 永不落钥。
    /// 已存在的同名键会被替换；无 `.env` 时创建。失败仅告警不阻断（密钥仍在内存生效）。
    pub fn write_env_key(key: &str) -> Option<PathBuf> {
        Self::write_env_key_to(&Self::config_dir().join(".env"), key)
    }

    /// [`Self::write_env_key`] 的路径注入版本（单测用，避免触碰真实用户目录）。
    fn write_env_key_to(env_path: &PathBuf, key: &str) -> Option<PathBuf> {
        if key.is_empty() {
            return None;
        }
        let res = (|| -> Result<(), anyhow::Error> {
            let mut lines: Vec<String> = std::fs::read_to_string(env_path)
                .map(|c| c.lines().map(str::to_string).collect())
                .unwrap_or_default();
            let new_line = format!("MARGUERITE_API_KEY={key}");
            match lines
                .iter_mut()
                .find(|l| l.starts_with("MARGUERITE_API_KEY="))
            {
                Some(l) => *l = new_line,
                None => lines.push(new_line),
            }
            if let Some(p) = env_path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::write(env_path, lines.join("\n") + "\n")?;
            Ok(())
        })();
        match res {
            Ok(()) => {
                info!("API Key 已转写 {}（YAML 不落钥）", env_path.display());
                Some(env_path.clone())
            }
            Err(e) => {
                tracing::warn!("API Key 转写 .env 失败（仅驻内存生效）: {e}");
                None
            }
        }
    }

    /// 获取日志目录路径
    pub fn log_dir(&self) -> PathBuf {
        Self::config_dir().join("logs")
    }

    /// 从配置目录 `.env` 删除 API Key（`MARGUERITE_API_KEY=` 整行移除）。
    ///
    /// 为什么必须有它：`save()` 结构性不落钥（`api_key` 带 `skip_serializing`），
    /// 所以「清空内存里的密钥再 save」**不会**改变已生效的密钥 —— 下次
    /// [`Self::load`] 的 `apply_env_overrides` 会从 `.env` 原样读回来。任何
    /// 「移除密钥/移除主档案」的入口都必须经这里真正删掉 `.env` 里那一行，
    /// 否则就是「UI 说已移除、引擎照旧在用」。其余键（provider/base_url/…）保留。
    ///
    /// 返回 `true` = 确实删掉了一行；`false` = 文件不存在或本就没有该键（如实报告，不谎称）。
    pub fn remove_env_key() -> bool {
        Self::remove_env_key_from(&Self::config_dir().join(".env"))
    }

    /// [`Self::remove_env_key`] 的路径注入版本（单测用，避免触碰真实用户目录）。
    fn remove_env_key_from(env_path: &Path) -> bool {
        let Ok(content) = std::fs::read_to_string(env_path) else {
            return false;
        };
        let kept: Vec<&str> = content
            .lines()
            .filter(|l| !l.trim_start().starts_with("MARGUERITE_API_KEY="))
            .collect();
        if kept.len() == content.lines().count() {
            return false; // 本来就没有该键
        }
        match std::fs::write(env_path, kept.join("\n") + "\n") {
            Ok(()) => {
                info!("API Key 已从 {} 移除", env_path.display());
                true
            }
            Err(e) => {
                tracing::warn!("从 .env 移除 API Key 失败: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.model.provider, "deepseek");
        assert_eq!(config.model.model_name, "deepseek-v4-pro");
        assert_eq!(config.log.level, "info");
    }

    /// 配置目录解析顺序：`MARGUERITE_CONFIG_DIR` 覆盖 > 用户主目录。
    ///
    /// 该变量是隔离测试/多实例部署的**唯一可靠手段**：Windows 上
    /// `dirs::home_dir()` 走 Known Folder API，不认 `USERPROFILE`/`HOME`，
    /// 只改 HOME 起的「隔离网关」会照样读写真实 `~/.marguerite`。
    #[test]
    fn test_resolve_config_dir_prefers_env_override() {
        use std::ffi::OsString;

        // 显式覆盖优先（且不回落到 home）
        assert_eq!(
            resolve_config_dir(
                Some(OsString::from("D:\\iso\\cfg")),
                Some(PathBuf::from("C:\\Users\\real"))
            ),
            PathBuf::from("D:\\iso\\cfg")
        );
        // 未设置 → 主目录下 .marguerite
        assert_eq!(
            resolve_config_dir(None, Some(PathBuf::from("C:\\Users\\real"))),
            PathBuf::from("C:\\Users\\real").join(".marguerite")
        );
        // 空串按未设置处理：否则配置目录会被解析成当前工作目录（配置静默丢失）
        assert_eq!(
            resolve_config_dir(Some(OsString::new()), Some(PathBuf::from("C:\\Users\\real"))),
            PathBuf::from("C:\\Users\\real").join(".marguerite")
        );
        // 主目录不可得时的兜底：相对当前目录，不 panic
        assert_eq!(
            resolve_config_dir(None, None),
            PathBuf::from(".").join(".marguerite")
        );
    }

    /// S10 前置：`data_dir()` 必须也能被环境变量整体覆盖，否则 Safe Mode
    /// 只能隔离 `config.yaml`，而 `audit.db` / `credentials.json` /
    /// `gateway-token` / cache / `sidecar.json` 仍会读用户真实数据。
    #[test]
    fn test_resolve_data_dir_prefers_env_override() {
        use std::ffi::OsString;

        // 显式覆盖优先（且不回落到平台默认）
        assert_eq!(
            resolve_data_dir(
                Some(OsString::from("D:\\iso\\data")),
                Some(PathBuf::from("C:\\Users\\real\\AppData\\Local"))
            ),
            PathBuf::from("D:\\iso\\data")
        );
        // 未设置 → 平台默认目录下 Marguerite
        assert_eq!(
            resolve_data_dir(None, Some(PathBuf::from("C:\\Users\\real\\AppData\\Local"))),
            PathBuf::from("C:\\Users\\real\\AppData\\Local").join("Marguerite")
        );
        // 空串按未设置处理：否则数据目录会被解析成当前工作目录
        assert_eq!(
            resolve_data_dir(
                Some(OsString::new()),
                Some(PathBuf::from("C:\\Users\\real\\AppData\\Local"))
            ),
            PathBuf::from("C:\\Users\\real\\AppData\\Local").join("Marguerite")
        );
        // 平台默认不可得时的兜底：相对当前目录，不 panic
        assert_eq!(
            resolve_data_dir(None, None),
            PathBuf::from(".").join("Marguerite")
        );
    }

    #[test]
    fn test_config_serialize_deserialize() {
        let config = Config::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.model.provider, config.model.provider);
        assert_eq!(deserialized.log.level, config.log.level);
    }

    #[test]
    fn test_config_ops_hosts_round_trip() {
        // `ops.hosts` 是可序列化/反序列化的主机注册表（M3 生产接线的数据源）。
        let mut config = Config::default();
        config.ops.hosts = vec![
            crate::ops::HostRef::new("node-1", "deploy@10.0.0.5:22", "cred-ref-1").unwrap(),
            crate::ops::HostRef::new("node-2", "10.0.0.6", "cred-ref-2").unwrap(),
        ];
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.ops.hosts.len(), 2);
        assert_eq!(deserialized.ops.hosts[0].id, "node-1");
        assert_eq!(deserialized.ops.hosts[0].address, "deploy@10.0.0.5:22");
        assert_eq!(deserialized.ops.hosts[1].address, "10.0.0.6");
        // 默认配置 -> ops.hosts 为空（零行为变化）
        assert!(Config::default().ops.hosts.is_empty());
    }

    #[test]
    fn test_config_save_and_load() {
        let tmp_dir = std::env::temp_dir().join("marguerite_test_config");
        let config_path = tmp_dir.join("config.yaml");

        let config = Config::default();
        config.save_to(&config_path).unwrap();

        assert!(config_path.exists());

        let loaded: Config = {
            let content = fs::read_to_string(&config_path).unwrap();
            serde_yaml::from_str(&content).unwrap()
        };
        assert_eq!(loaded.model.provider, "deepseek");

        // 清理
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_config_paths() {
        // config_dir 语义：生产时指向 ~/.marguerite；单测被 methods::tests 覆盖后
        // 指向临时目录（不再以 .marguerite 结尾）。这里断言「路径一致」即可；
        // 不强制后缀（避免依赖并行测试顺序）。
        let dir = Config::config_dir();
        assert_eq!(Config::config_path().parent(), Some(dir.as_path()));
        let config = Config::default();
        assert!(config.log_dir().ends_with("logs"));
    }

    #[test]
    fn test_save_to_nested_dir() {
        let tmp_dir = std::env::temp_dir()
            .join("marguerite_test_nested")
            .join("a")
            .join("b");
        let config_path = tmp_dir.join("config.yaml");
        let config = Config::default();
        config.save_to(&config_path).unwrap();
        assert!(config_path.exists());
        let _ = fs::remove_dir_all(std::env::temp_dir().join("marguerite_test_nested"));
    }

    #[test]
    fn test_mcp_and_memory_defaults() {
        let config = Config::default();
        assert!(config.mcp.servers.is_empty());
        assert_eq!(config.memory.sensory_path, "memory/sensory");
        assert_eq!(config.skills.skills_dir, "skills");
        assert!(config.guard.syntax);
    }

    #[test]
    fn test_mcp_server_config_roundtrip() {
        let server = McpServerConfig {
            name: "srv".to_string(),
            command: "echo".to_string(),
            args: vec!["-x".to_string()],
            env: std::collections::HashMap::new(),
            url: None,
        };
        let yaml = serde_yaml::to_string(&server).unwrap();
        let back: McpServerConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.name, "srv");
        assert_eq!(back.args, vec!["-x".to_string()]);
    }

    #[test]
    fn test_naming_privacy_persist_roundtrip() {
        // 纯 Rust 离线设置：naming/privacy 必须随其余配置一道序列化到 config.yaml，
        // 且离线（无网络/外部服务）可读写。
        let mut config = Config::default();
        config.naming.enabled = false;
        config.privacy.disable_network_observability = true;

        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(
            yaml.contains("naming:") && yaml.contains("privacy:"),
            "命名/隐私字段应写入 YAML"
        );

        let back: Config = serde_yaml::from_str(&yaml).unwrap();
        assert!(!back.naming.enabled);
        assert!(back.privacy.disable_network_observability);

        // 写盘读回（离线路径）
        let tmp = std::env::temp_dir().join(format!("mgt_cfg_{}.yaml", uuid::Uuid::new_v4()));
        config.save_to(&tmp).unwrap();
        let content = fs::read_to_string(&tmp).unwrap();
        let loaded: Config = serde_yaml::from_str(&content).unwrap();
        assert!(!loaded.naming.enabled);
        assert!(loaded.privacy.disable_network_observability);
        let _ = fs::remove_file(&tmp);
    }

    /// `gateway.origin_allowlist` 契约：缺省为空（零行为变化），
    /// YAML 显式配置后按序读回——WS Origin 白名单的配置覆盖入口。
    #[test]
    fn test_gateway_origin_allowlist_roundtrip() {
        let empty: Config = serde_yaml::from_str("model:\n  provider: deepseek\n").unwrap();
        assert!(
            empty.gateway.origin_allowlist.is_empty(),
            "未配置时白名单扩展项必须为空（默认白名单由网关内置）"
        );

        let yaml = "gateway:\n  origin_allowlist:\n    - https://console.internal.example\n    - http://127.0.0.1:9999\n";
        let loaded: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            loaded.gateway.origin_allowlist,
            vec![
                "https://console.internal.example".to_string(),
                "http://127.0.0.1:9999".to_string()
            ]
        );
    }

    /// E3/K1 凭证护栏回归：内存中的 api_key（无论来自 env/.env/补丁注入）
    /// 在 save()/save_to() 序列化时**结构性**不落 YAML。
    #[test]
    fn test_save_never_persists_api_key() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_cfg_sec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let config_path = tmp_dir.join("config.yaml");

        let mut config = Config::default();
        config.model.api_key = "sk-secret-test-key-0123456789".to_string();
        config.save_to(&config_path).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        assert!(
            !content.contains("sk-secret-test-key"),
            "密钥明文不得出现在 config.yaml"
        );
        assert!(
            !content.contains("api_key"),
            "YAML 序列化产物不应包含 api_key 字段（skip_serializing）"
        );

        // 读回后 api_key 为空（由 env/.env 注入，不由 YAML 提供）
        let loaded: Config = serde_yaml::from_str(&content).unwrap();
        assert!(loaded.model.api_key.is_empty());
        assert_eq!(loaded.model.provider, "deepseek");

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// write_env_key：写入 / 替换 MARGUERITE_API_KEY 且保留 .env 其他行
    #[test]
    fn test_write_env_key_replaces_and_preserves() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_env_sec_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let env_path = tmp_dir.join(".env");
        fs::write(&env_path, "# 注释\nMARGUERITE_MODEL=deepseek-v4-flash\n").unwrap();

        Config::write_env_key_to(&env_path, "sk-new-key-abc");

        let content = fs::read_to_string(&env_path).unwrap();
        assert!(
            content.contains("MARGUERITE_API_KEY=sk-new-key-abc"),
            "{content}"
        );
        assert!(
            content.contains("MARGUERITE_MODEL=deepseek-v4-flash"),
            "其他键必须保留"
        );
        assert!(content.contains("# 注释"), "注释必须保留");

        // 二次写入 → 替换而非追加
        Config::write_env_key_to(&env_path, "sk-new-key-def");
        let content = fs::read_to_string(&env_path).unwrap();
        assert!(content.contains("MARGUERITE_API_KEY=sk-new-key-def"));
        assert_eq!(content.matches("MARGUERITE_API_KEY=").count(), 1);

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// remove_env_key：真正删掉密钥行并保留其余内容。
    ///
    /// 这是「移除主档案」能生效的唯一途径：`save()` 结构性不落钥，所以只清内存
    /// 再保存是无效动作——`apply_env_overrides` 下次会从 `.env` 原样读回来。
    #[test]
    fn test_remove_env_key_deletes_only_the_key() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_env_rm_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let env_path = tmp_dir.join(".env");
        fs::write(
            &env_path,
            "# 注释\nMARGUERITE_API_KEY=sk-to-be-removed\nMARGUERITE_MODEL=deepseek-v4-flash\n",
        )
        .unwrap();

        assert!(Config::remove_env_key_from(&env_path), "首次移除应如实返回 true");
        let content = fs::read_to_string(&env_path).unwrap();
        assert!(!content.contains("MARGUERITE_API_KEY"), "密钥行必须删除: {content}");
        assert!(
            content.contains("MARGUERITE_MODEL=deepseek-v4-flash"),
            "其他键必须保留: {content}"
        );
        assert!(content.contains("# 注释"), "注释必须保留: {content}");

        // 二次移除：本就没有该键 → 如实返回 false（不谎称删过）
        assert!(
            !Config::remove_env_key_from(&env_path),
            "无该键时应返回 false，而不是假报成功"
        );

        // 文件不存在同样如实返回 false，不写坏文件
        let missing = tmp_dir.join("nope.env");
        assert!(!Config::remove_env_key_from(&missing));
        assert!(!missing.exists(), "不得凭空创建 .env");

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// 空密钥写入仍是 no-op（移除必须走 remove_env_key，不能靠写空串绕过）。
    #[test]
    fn test_write_env_key_keeps_empty_key_a_noop() {
        let tmp_dir = std::env::temp_dir().join(format!("mgt_env_noop_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let env_path = tmp_dir.join(".env");
        fs::write(&env_path, "MARGUERITE_API_KEY=sk-keep-me\n").unwrap();

        assert!(Config::write_env_key_to(&env_path, "").is_none());
        let content = fs::read_to_string(&env_path).unwrap();
        assert!(content.contains("MARGUERITE_API_KEY=sk-keep-me"), "{content}");

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
