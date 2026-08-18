//! 配置管理模块
//!
//! 借鉴 混合检索引擎 配置设计，实现 YAML 配置的加载和保存。
//! 默认配置文件路径: `~/.marguerite/config.yaml`

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
    /// API 密钥
    #[serde(default)]
    pub api_key: String,
    /// API 基础 URL
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// 副模型（Copilot，Bloom 并行编排审校用）。为空则 Bloom 退化为同模型审校。
    #[serde(default)]
    pub copilot_model: Option<SecondaryModelConfig>,
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
    /// 副模型 API 密钥（缺省继承主模型）
    #[serde(default)]
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

/// Bloom 并行编排配置（webui 模型策略/对话路由可见；默认关闭，零行为变化）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BloomConfig {
    /// 是否启用 Captain+Copilot 双模型并行推理（对话路径走 BloomOrchestrator）
    #[serde(default)]
    pub parallel_enabled: bool,
    /// Bloom 子模式：deep（四阶段）/ auto（自主闭环）/ swarm（双领队蜂群）
    #[serde(default = "default_bloom_submode")]
    pub submode: String,
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

fn default_bloom_submode() -> String {
    "deep".to_string()
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
}

impl Config {
    /// 获取默认配置目录路径（~/.marguerite/；单测可覆盖）
    pub fn config_dir() -> PathBuf {
        if let Some(dir) = crate::persistence::OVERRIDE_CONFIG_DIR.get() {
            return dir.clone();
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".marguerite")
    }

    /// 覆盖配置根目录（测试辅助）；一旦设置不可再改（OnceLock 语义）。
    #[cfg(test)]
    pub fn set_config_dir_override(path: PathBuf) {
        let _ = crate::persistence::OVERRIDE_CONFIG_DIR.set(path);
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
    pub fn data_dir() -> PathBuf {
        dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Marguerite")
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
                "模型后端已启用：provider={} model={}（密钥来源：环境变量/.env，未落 YAML）",
                self.model.provider, self.model.model_name
            );
        }
    }

    /// 保存配置到默认路径
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path();
        self.save_to(&config_path)
    }

    /// 保存配置到指定路径
    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        // 确保目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_yaml::to_string(self)?;
        std::fs::write(path, content)?;
        info!("配置已保存: {}", path.display());
        Ok(())
    }

    /// 获取日志目录路径
    pub fn log_dir(&self) -> PathBuf {
        Self::config_dir().join("logs")
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

    #[test]
    fn test_config_serialize_deserialize() {
        let config = Config::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(deserialized.model.provider, config.model.provider);
        assert_eq!(deserialized.log.level, config.log.level);
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
}
