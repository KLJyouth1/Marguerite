//! Onboarding 方法族（从 methods.rs 分治迁出）
//!
//! 覆盖 `onboarding.*` 全量方法：provider 目录 / 引导状态 / 路由配置 / Bloom 集成 / 服务商配置。
//! 侧车离线时由此模块提供最小可用实现，让设置页 / 渠道编辑器仍可渲染 provider 选择。

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::persistence::config::Config;

/// `onboarding.catalog`（Rust 离线回退）：返回内置 LLM / 搜索 / 图像 / 嵌入 provider 目录。
pub(crate) fn onboarding_catalog() -> Value {
    let openai_compatible_fields = json!([
        { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
        { "name": "base_url", "label": "Base URL", "type": "text", "required": false, "placeholder": "https://api.example.com/v1" },
        { "name": "model", "label": "Model", "type": "text", "required": true, "placeholder": "model-name" },
    ]);

    // 自定义端点的字段**不能**复用上面那份：它把 `api_key` 标成 required，
    // 而自建/回环端点（llama.cpp、vLLM、内网网关）多数无鉴权 —— 用户被
    // 强制填一个不存在的密钥才能保存，等于「自定义不可用」。
    // `base_url` 反过来必须是 required：没有它是唯一确定性的失败。
    let custom_openai_fields = json!([
        { "name": "api_key", "label": "API Key", "type": "password", "required": false, "secret": true,
          "description": "端点要求鉴权时填写；本地/无鉴权端点留空" },
        { "name": "base_url", "label": "Base URL", "type": "text", "required": true,
          "placeholder": "https://api.example.com/v1" },
        { "name": "model", "label": "Model", "type": "text", "required": true, "placeholder": "model-name" },
        { "name": "wire_api", "label": "请求协议", "type": "select", "required": false, "default": "chat",
          "choices": ["chat", "responses"],
          "description": "chat = /v1/chat/completions（绝大多数端点）；responses = /v1/responses（仅 OpenAI Responses）" },
    ]);

    let custom_anthropic_fields = json!([
        { "name": "api_key", "label": "API Key", "type": "password", "required": false, "secret": true,
          "description": "兼容网关按 Authorization: Bearer 鉴权；留空则不带密钥" },
        { "name": "base_url", "label": "Base URL", "type": "text", "required": true,
          "placeholder": "https://api.example.com" },
        { "name": "model", "label": "Model", "type": "text", "required": true,
          "placeholder": "claude-sonnet-4-20250514" },
    ]);

    let providers = json!([
        {
            "providerId": "deepseek",
            "label": "DeepSeek",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "DEEPSEEK_API_KEY",
            "defaultBaseUrl": "https://api.deepseek.com/v1",
            "defaultDirectModel": "deepseek-chat",
            "suggestedModels": ["deepseek-chat", "deepseek-reasoner"],
            "whatYouNeed": ["DeepSeek API Key"],
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
                { "name": "model", "label": "Model", "type": "text", "required": false, "default": "deepseek-chat" },
            ],
        },
        {
            "providerId": "openai",
            "label": "OpenAI",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "OPENAI_API_KEY",
            "defaultBaseUrl": "https://api.openai.com/v1",
            "defaultDirectModel": "gpt-4o",
            "suggestedModels": ["gpt-4o", "gpt-4o-mini", "o1", "o3-mini"],
            "whatYouNeed": ["OpenAI API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "anthropic",
            "label": "Anthropic",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "ANTHROPIC_API_KEY",
            "defaultBaseUrl": "https://api.anthropic.com/v1",
            "defaultDirectModel": "claude-sonnet-4-20250514",
            "suggestedModels": ["claude-sonnet-4-20250514", "claude-opus-4-20250514", "claude-haiku-4-20250514"],
            "whatYouNeed": ["Anthropic API Key"],
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
                { "name": "base_url", "label": "Base URL", "type": "text", "required": false,
                  "default": "https://api.anthropic.com",
                  "description": "官方端点留空即可；走代理/中转时填它的地址" },
                { "name": "model", "label": "Model", "type": "text", "required": false, "default": "claude-sonnet-4-20250514" },
            ],
        },
        {
            // id 必须与 pyengine/provider/registry.py 一致：此前这里是 `qwen`，
            // 而注册表里叫 `dashscope` —— 离线目录能选中、能存盘，等侧车上线
            // 就变成 UnknownProvider（`rust_fallback_catalog_ids_exist_in_the_python_registry`
            // 这条守卫就是为此而设）。
            "providerId": "dashscope",
            "label": "Qwen (Alibaba Cloud DashScope)",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "DASHSCOPE_API_KEY",
            "defaultBaseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "defaultDirectModel": "qwen-max",
            "suggestedModels": ["qwen-max", "qwen-plus", "qwen-turbo", "qwen3-235b-a22b"],
            "whatYouNeed": ["DashScope API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "zhipu",
            "label": "Zhipu (Z.AI)",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "ZHIPU_API_KEY",
            "defaultBaseUrl": "https://open.bigmodel.cn/api/paas/v4",
            "defaultDirectModel": "glm-4-plus",
            "suggestedModels": ["glm-4-plus", "glm-4-air", "glm-4-flash"],
            "whatYouNeed": ["Zhipu API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "moonshot",
            "label": "Moonshot AI (Kimi)",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "MOONSHOT_API_KEY",
            "defaultBaseUrl": "https://api.moonshot.cn/v1",
            "defaultDirectModel": "moonshot-v1-128k",
            "suggestedModels": ["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"],
            "whatYouNeed": ["Moonshot API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "siliconflow",
            "label": "SiliconFlow",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SILICONFLOW_API_KEY",
            "defaultBaseUrl": "https://api.siliconflow.cn/v1",
            "defaultDirectModel": "deepseek-ai/DeepSeek-V3",
            "suggestedModels": ["deepseek-ai/DeepSeek-V3", "deepseek-ai/DeepSeek-R1", "Qwen/Qwen2.5-72B-Instruct"],
            "whatYouNeed": ["SiliconFlow API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "openrouter",
            "label": "OpenRouter",
            "runtimeSupported": true,
            "routerSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "OPENROUTER_API_KEY",
            "defaultBaseUrl": "https://openrouter.ai/api/v1",
            "defaultDirectModel": "deepseek/deepseek-chat",
            "whatYouNeed": ["OpenRouter API Key"],
            "fields": &openai_compatible_fields,
        },
        {
            "providerId": "custom",
            "label": "Custom OpenAI-compatible endpoint",
            "runtimeSupported": true,
            "routerSupported": false,
            "acceptsApiKey": true,
            "requiresApiKey": false,
            "deployment": "custom",
            "whatYouNeed": ["OpenAI 兼容端点（/v1/chat/completions 或 /v1/responses）", "API Key（若端点要求）"],
            "fields": &custom_openai_fields,
        },
        {
            "providerId": "custom_anthropic",
            "label": "Custom Anthropic-compatible endpoint",
            "runtimeSupported": true,
            "routerSupported": false,
            "acceptsApiKey": true,
            "requiresApiKey": false,
            "deployment": "custom",
            "whatYouNeed": ["Anthropic Messages 兼容端点（/v1/messages）", "API Key（若端点要求）"],
            "fields": &custom_anthropic_fields,
        },
        {
            "providerId": "ollama",
            "label": "Ollama (local)",
            "runtimeSupported": true,
            "routerSupported": false,
            "acceptsApiKey": false,
            "requiresApiKey": false,
            "envKey": "",
            "defaultBaseUrl": "http://localhost:11434",
            "defaultDirectModel": "",
            "deployment": "local",
            "blocking": true,
            "canProbe": true,
            "whatYouNeed": ["Ollama 已在本机运行", "一个已 ollama pull 的模型 id"],
            "fields": [
                { "name": "model", "label": "Model id", "type": "text", "required": true, "placeholder": "qwen3.8:27b", "description": "本地已拉取的模型标签，例如 qwen3.8:27b" },
                { "name": "base_url", "label": "Base URL", "type": "text", "required": false, "default": "http://localhost:11434", "description": "Ollama 服务地址，默认本机 11434" },
                { "name": "proxy", "label": "HTTP proxy", "type": "text", "required": false, "default": "", "description": "可选 HTTP 代理" },
            ],
        },
    ]);

    let search_providers = json!([
        {
            "providerId": "tavily",
            "label": "Tavily Search",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "TAVILY_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
        {
            "providerId": "serper",
            "label": "Serper.dev",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SERPER_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    let image_providers = json!([
        {
            "providerId": "siliconflow",
            "label": "SiliconFlow Image Generation",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SILICONFLOW_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    let embedding_providers = json!([
        {
            "providerId": "siliconflow",
            "label": "SiliconFlow Embeddings",
            "runtimeSupported": true,
            "acceptsApiKey": true,
            "requiresApiKey": true,
            "envKey": "SILICONFLOW_API_KEY",
            "fields": [
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    let channels = json!([
        {
            "type": "slack",
            "label": "Slack",
            "description": "Slack 频道（Bot Token + App Token）",
            "transport": "websocket",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "bot_token", "label": "Bot Token (xoxb-)", "type": "password", "required": true, "secret": true },
                { "name": "app_token", "label": "App Token (xapp-)", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "discord",
            "label": "Discord",
            "description": "Discord 机器人（Gateway WebSocket）",
            "transport": "websocket",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "bot_token", "label": "Bot Token", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "telegram",
            "label": "Telegram",
            "description": "Telegram 机器人（Long Polling）",
            "transport": "polling",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "bot_token", "label": "Bot Token", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "feishu",
            "label": "飞书 / Lark",
            "description": "飞书自建应用（事件回调）",
            "transport": "webhook",
            "requiresPublicUrl": true,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "app_id", "label": "App ID", "type": "text", "required": true },
                { "name": "app_secret", "label": "App Secret", "type": "password", "required": true, "secret": true },
                { "name": "verification_token", "label": "Verification Token", "type": "password", "required": false, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "wecom",
            "label": "企业微信",
            "description": "企业微信自建应用（回调）",
            "transport": "webhook",
            "requiresPublicUrl": true,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "corp_id", "label": "Corp ID", "type": "text", "required": true },
                { "name": "agent_id", "label": "Agent ID", "type": "text", "required": true },
                { "name": "secret", "label": "Secret", "type": "password", "required": true, "secret": true },
                { "name": "token", "label": "Callback Token", "type": "password", "required": false, "secret": true },
                { "name": "encoding_aes_key", "label": "Encoding AES Key", "type": "password", "required": false, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
        {
            "type": "dingtalk",
            "label": "钉钉",
            "description": "钉钉机器人（Stream 模式）",
            "transport": "websocket",
            "requiresPublicUrl": false,
            "restartRequired": true,
            "canProbe": true,
            "fields": [
                { "name": "name", "label": "Channel name", "type": "text", "required": true },
                { "name": "client_id", "label": "Client ID", "type": "text", "required": true },
                { "name": "client_secret", "label": "Client Secret", "type": "password", "required": true, "secret": true },
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": true, "advanced": true },
            ],
        },
    ]);

    let audio_providers = json!([
        {
            "providerId": "elevenlabs",
            "label": "ElevenLabs Audio",
            "runtimeSupported": true,
            "requiresApiKey": true,
            "envKey": "ELEVENLABS_API_KEY",
            "defaultBaseUrl": "https://api.elevenlabs.io",
            "defaultTtsModel": "eleven_multilingual_v2",
            "defaultTtsVoice": "21m00Tcm4TlvDq8ikWAM",
            "deployment": "cloud",
            "blocking": false,
            "canProbe": true,
            "whatYouNeed": ["ElevenLabs API Key"],
            "fields": [
                { "name": "enabled", "label": "Enabled", "type": "bool", "required": false, "default": false },
                { "name": "api_key", "label": "API Key", "type": "password", "required": true, "secret": true },
            ],
        },
    ]);

    json!({
        "providers": providers,
        "channels": channels,
        "searchProviders": search_providers,
        "routerProfiles": { "profiles": [], "defaultTier": "auto" },
        "memoryEmbeddingProviders": embedding_providers,
        "imageGenerationProviders": image_providers,
        "audioProviders": audio_providers,
    })
}

/// `llmProfileStatus`：从 models.json 汇总各服务商档案的就绪行。
///
/// 与 `config.get llm_profiles` 同一数据源与过滤规则（非内置模板 + 当前主
/// provider）。凭据可用性**按真实解析链**判定（档案声明 env → 分槽 → 受标记约束的
/// 全局槽 → 密文/明文），而不是只看不落盘的 `p.api_key`——旧实现让非 active
/// 档案永远显示未就绪，用户会以为密钥没保存。
fn llm_profile_status() -> Value {
    use crate::model::byok;
    use crate::model::selector::ModelSelector;
    let Ok(sel) = ModelSelector::load() else {
        return json!([]);
    };
    let active_id = sel.registry().active.clone();
    let rows: Vec<Value> = sel
        .registry()
        .list()
        .iter()
        .filter(|p| !(p.builtin && p.api_key.is_empty() && p.id != active_id))
        .map(|p| {
            let (resolved, source) =
                byok::resolve_api_key_with_source(&p.id, &p.provider, &p.api_key, &p.api_key_env, None);
            // 【2026-10-23 修复：本地/回环端点免密钥 ⇒ 就绪】
            //
            // 原判据是「解析出非空密钥」，于是所有 Ollama / LM Studio 档案都回
            // `ready:false`（实测 11 条本地档案全红），设置页「模型服务」被标
            // 「需要处理」，而本地模型其实完全可用 —— 与 `llm_readiness` /
            // `chat::api_key_gate` 的判据自相矛盾。
            // 这里统一走同一个纯函数 `byok::is_keyless_endpoint`，并用前端已认识的
            // `keyless` 口径（`profileCredentialUiSource` 把它映射成「无需密钥」）。
            let keyless = byok::is_keyless_endpoint(&p.provider, &p.base_url);
            let ready = keyless || !resolved.is_empty();
            // 来源由解析链本身给出：环境变量名 = env；显式/落盘 = explicit。
            // 不再按「api_key 字符串非空」猜，否则 .env 与残留字段的优先级会被说谎。
            let (credential_source, credential_env) = if keyless && resolved.is_empty() {
                ("keyless", String::new())
            } else if !ready {
                ("none", String::new())
            } else if source == "explicit" || source == "stored" {
                ("explicit", String::new())
            } else {
                ("env", source)
            };
            json!({
                "provider": p.provider,
                "ready": ready,
                "credentialSource": credential_source,
                "credentialEnv": credential_env,
                "endpointSource": "",
                "reason": if keyless && resolved.is_empty() {
                    "本地/回环端点，无需 API Key"
                } else {
                    ""
                },
                "primaryEligible": true,
                "primaryBlockReason": "",
            })
        })
        .collect();
    json!(rows)
}

/// 模型服务就绪判定（纯函数）：**有密钥，或本地/回环端点（免密钥）**。
///
/// 缺陷背景（2026-10-21 实测）：此前只看 `model.api_key` 是否为空，于是 Ollama
/// 本地模型永远 `llmConfigured=false` → `needsOnboarding=true` → 设置页横幅
/// 「需要配置 / 请设置模型提供商以开始对话」，而用户其实已经能离线跑。
/// 判据统一走 `byok::is_keyless_endpoint`，与 `chat::api_key_gate` / `doctor` 同源。
///
/// 返回值第二项是**来源口径**：`config`（有密钥）/ `local`（本地免密钥）/ `none`。
fn llm_readiness(provider: &str, base_url: &str, api_key: &str) -> (bool, &'static str) {
    if !api_key.trim().is_empty() {
        return (true, "config");
    }
    if crate::model::byok::is_keyless_endpoint(provider, base_url) {
        return (true, "local");
    }
    (false, "none")
}

/// `onboarding.status`（Rust 离线回退）：返回最小引导状态。
pub(crate) fn onboarding_status(ctx: &GatewayContext) -> Value {
    // 【2026-10-23 修复：就绪度必须问「执行侧真正在用的模型」】
    //
    // 真机取证（安装包 0.2.16，模型选择器显示 ollama/omnipilot17:latest）：
    // 本函数回 `hasConfig:false` / `llmConfigured:false` / `llmSource:none`，
    // 侧栏因此常驻「需要设置 · 网关尚未就绪」，模型服务页也标「需要处理」。
    //
    // 根因：这里只读 `config.yaml` 的 `model` 段（该段仍是厂商默认 deepseek、
    // 无密钥），而用户实际选中的档案存在 `models.json.active` —— 会话入口
    // （`model::selector::apply_effective_model`）正是以它覆盖 model 段后才发给模型。
    // 于是「判就绪的配置」与「真正发请求的配置」是两份东西。
    //
    // 修复：与 `chat.send` 走**同一条**解析链（`apply_effective_model`），
    // 就绪判据与执行侧单源。models.json 不存在时该函数原样返回 config.yaml，
    // 行为与旧版一致（不会把"从没选过档案"的机器静默切到种子默认模型）。
    let live = crate::model::selector::apply_effective_model(
        &Config::load().unwrap_or_else(|_| ctx.config.clone()),
    );
    // v3 修复：Config::load() 已叠加 env/.env 覆盖，api_key 非空即视为已配置。
    // 此前这里只看 YAML 的 api_key 字段（现已 skip_serializing 恒为空），
    // 导致 .env 有密钥也误报「网关尚未就绪」。
    // 就绪判据见 `llm_readiness`（纯函数，可单测；本地端点免密钥）。
    let (llm_configured, llm_source) = llm_readiness(
        &live.model.provider,
        &live.model.base_url,
        &live.model.api_key,
    );
    // 主凭据的来源与变量名必须由**同一条解析链**产出（见 byok::resolve_api_key_with_source）：
    // 旧实现恒报 `MARGUERITE_API_KEY`，档案分槽/声明变量用户会看到错误的变量名，
    // 照它去设置环境变量永远不生效。
    let (credential_source, credential_env) = if llm_source == "local" {
        // 本地端点不需要凭据：如实说「本地」，不要谎称读到了某个环境变量。
        ("local", String::new())
    } else if llm_configured {
        use crate::model::byok;
        let (_, source) = match Config::active_profile_identity() {
            Some((id, provider, declared)) => byok::resolve_api_key_with_source(
                &id,
                &provider,
                "",
                &declared,
                None,
            ),
            // 无档案：只有全局槽可能提供密钥，归属标记按配置 provider 校验。
            None => byok::resolve_api_key_with_source(
                "",
                &live.model.provider,
                "",
                "",
                None,
            ),
        };
        if source == "explicit" || source == "stored" {
            ("explicit", String::new())
        } else if source.is_empty() {
            // 内存里已有密钥但解析链读不到环境变量（例如启动期注入），
            // 不谎称它来自某个变量。
            ("config", String::new())
        } else {
            ("env", source)
        }
    } else {
        ("none", String::new())
    };
    // configPath 的语义是**配置文件路径**（客户端据此展示/复制「更多选项在此文件」并拼
    // `--config <path>`）。此前误传 `Config::data_dir()`（数据目录），导致前端页脚把目录
    // 当成配置文件、`--config <目录>` 亦是坏命令；pyengine 侧契约测试明确要求
    // `payload["configPath"] == cfg.config_path`（文件），两侧语义必须一致。
    let config_path = Config::config_path().to_string_lossy().to_string();
    // 隔离模式（`MARGUERITE_CONFIG_DIR` 指向临时目录，见 scripts/verify.sh）：
    // 前端页脚不直接暴露 `E:\temp\...` 这类隔离路径，改为「用户配置目录（隔离模式）」，
    // 真路径只放在 title / 复制内容里。回传布尔值，不让前端按路径形状猜。
    let config_isolated = std::env::var_os("MARGUERITE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .is_some();
    json!({
        "needsOnboarding": !llm_configured,
        "hasConfig": llm_configured,
        "llmConfigured": llm_configured,
        "llmSource": llm_source,
        "channelCount": 0,
        "channelsConfigured": false,
        "searchConfigured": false,
        "imageGenerationConfigured": false,
        "configPath": config_path,
        "configIsolated": config_isolated,
        "sections": {
            "llm": if llm_configured { "complete" } else { "incomplete" },
            // 非阻断分区：本地能力默认可用，不再恒报 incomplete 触发横幅
            "search": "optional",
            "imageGeneration": "optional",
            "memory": if llm_configured { "complete" } else { "incomplete" },
        },
        // 就绪摘要依赖 sectionDetails 判定 blocking/actionRequired；
        // 空对象曾使 needsOnboarding 单独驱动横幅。现在如实给出明细。
        "sectionDetails": {
            "llm": {
                "status": if llm_configured { "complete" } else { "missing" },
                "blocking": !llm_configured,
                "actionRequired": !llm_configured,
                "label": "Model Service",
                "detail": if llm_configured {
                    format!("provider={} model={}", live.model.provider, live.model.model_name)
                } else {
                    "尚未配置模型服务商或 API Key".to_string()
                },
            },
        },
        // v3.1：主凭据状态（webui 凭证面板/档案卡 ready 判定的数据源）
        "llmCredentialStatus": {
            "provider": live.model.provider,
            "available": llm_configured,
            "source": credential_source,
            "envKey": credential_env,
            "masked": "",
            "revealAllowed": false,
        },
        // v3.1：已存档案的就绪状态行（与 config.get llm_profiles 对齐）。
        // 数据源 = models.json 非内置模板 + 当前主 provider；webui 档案卡的
        // ready/primaryEligible 判定依赖此列表。
        "llmProfileStatus": llm_profile_status(),
        "warnings": if llm_configured { Value::Array(vec![]) } else { json!(["LLM API Key 未配置（离线降级可用）"]) },
    })
}

/// `onboarding.router.configure`：保存模型路由配置。
pub(crate) fn onboarding_router_configure(ctx: &GatewayContext, params: &Value) -> Value {
    let mut cfg = Config::load().unwrap_or_else(|_| ctx.config.clone());
    if let Some(mode) = params.get("mode").and_then(Value::as_str) {
        cfg.model_routing.mode = match mode {
            "disabled" => "direct".to_string(),
            "recommended" => "router".to_string(),
            other => other.to_string(),
        };
    }
    if let Some(fusion) = params.get("fusionModel").and_then(Value::as_str) {
        cfg.model_routing.fusion_model = fusion.to_string();
    }
    if let Some(candidates) = params.get("candidateModels").and_then(Value::as_array) {
        cfg.model_routing.candidate_models = candidates
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(fixed) = params.get("fixedModel").and_then(Value::as_str) {
        cfg.model_routing.fixed_model = fixed.to_string();
    }
    if let Some(v) = params.get("crossProviderTiers").and_then(Value::as_bool) {
        cfg.model_routing.cross_provider_tiers = v;
    }
match cfg.save() {
        Ok(()) => {
            // R72+：保存即生效 —— 更新进程级生效配置快照，ModelRouter / chat.send
            // 下一轮即用新路由模式，无需重启（与 onboarding_provider_configure 一致）。
            crate::gateway::chat::publish_effective_config(cfg.clone());
            // P3 打通：路由配置保存广播 models.changed，路由面板 / 聊天指示被动刷新。
            crate::events::product::emit(
                "models.changed",
                "",
                json!({ "reason": "router_configure" }),
            );
            json!({ "ok": true, "saved": true, "restart_required": false })
        }
        Err(e) => json!({ "ok": false, "error": format!("保存路由配置失败: {e}") }),
    }
}

/// `onboarding.ensemble.configure`：保存 Bloom 集成配置。
pub(crate) fn onboarding_ensemble_configure(ctx: &GatewayContext, params: &Value) -> Value {
    let mut cfg = ctx.config.clone();
    if let Some(v) = params.get("enabled").and_then(Value::as_bool) {
        cfg.bloom.parallel_enabled = v;
    }
    if let Some(v) = params.get("selectionMode").and_then(Value::as_str) {
        cfg.bloom.selection_mode = v.to_string();
    }
    if let Some(v) = params.get("allFailedPolicy").and_then(Value::as_str) {
        cfg.bloom.all_failed_policy = v.to_string();
    }
    if let Some(candidates) = params.get("candidates") {
        cfg.bloom.candidates = candidates.clone();
    }
    match cfg.save() {
        Ok(()) => {
            // R72+：保存即生效 —— 同步进程级生效配置快照（Bloom 并行编排开关即时可用）。
            crate::gateway::chat::publish_effective_config(cfg.clone());
            json!({ "ok": true, "saved": true, "restart_required": false })
        }
        Err(e) => json!({ "ok": false, "error": format!("保存集成配置失败: {e}") }),
    }
}

/// `onboarding.provider.configure`：保存模型服务商配置。
///
/// v3.1 修复「保存了但没生效」：同时接受 webui 发送的 camelCase `model`
/// 字段（此前只认 modelName/model_name，模型名永远存不上 → 页面仍显示空状态）；
/// 收到 apiKey 时经 [`Config::write_env_key_for`] 转写 `.env`（凭证护栏，YAML 不落钥）。
pub(crate) fn onboarding_provider_configure(ctx: &GatewayContext, params: &Value) -> Value {
    // 空负载保持原语义（响亮拒绝，不做无意义的「重存一遍」）。
    let touches_model = ["provider", "providerId", "model", "modelName", "model_name",
        "baseUrl", "base_url", "apiKey", "api_key", "apiKeyEnv", "api_key_env"]
        .iter()
        .any(|k| params.get(*k).is_some());
    if !touches_model {
        return json!({ "ok": false, "error": "没有需要保存的字段" });
    }
    // 委托给「档案优先」实现：本函数此前只写 config.yaml + .env 全局槽 + 内存快照，
    // 从不碰 models.json，而请求侧 `apply_effective_model` 会用 models.json.active
    // 覆盖 config.yaml —— 于是「保存成功」等于什么都没发生（详见
    // `llm_profile::configure_primary_provider` 的文档注释与实测证据）。
    match crate::gateway::llm_profile::configure_primary_provider(ctx, params) {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": e.message, "code": e.code }),
    }
}

/// 探测请求的鉴权方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeAuth {
    /// `Authorization: Bearer <key>`（OpenAI 兼容，以及 Bearer 风格的 Anthropic 兼容网关）
    Bearer(String),
    /// `x-api-key: <key>` + `anthropic-version`（Anthropic 官方 / TokenHub）
    XApiKey(String),
    /// 不带鉴权头（本机推理服务）
    None,
}

/// 探测请求的落点：URL + 鉴权 + 请求体。
#[derive(Debug, Clone)]
pub(crate) struct ProbeTarget {
    pub url: String,
    pub auth: ProbeAuth,
    pub body: Value,
}

/// 按**端点真实协议**解析探测请求（纯函数，便于表驱动单测）。
///
/// # 为什么必须分开算（原实现的三个确定性失败）
///
/// 旧实现恒发 `{base}/chat/completions` + `Authorization: Bearer`：
/// 1. **Anthropic 家族**（`anthropic` / `custom_anthropic` / `*_anthropic`）只有
///    `/v1/messages`，且官方要 `x-api-key` —— 恒 404 / 401，界面上却显示
///    「连接失败」，用户据此以为密钥或模型名写错了；
/// 2. **`responses` 协议**的值机只提供 `/v1/responses`，同样恒 404；
/// 3. `{base}/chat/completions` 少了 `/v1`：`https://api.openai.com` 这类
///    不带版本段的 base 会拼出 `/chat/completions`（与真实请求用的
///    [`crate::model::llm::build_chat_url`] 规则不一致 —— 探测说通不过，
///    实际对话却能用，或者反过来）。
///
/// 所以这里复用与真实请求**同一套** URL 规则与鉴权规则，避免"探测通过但对话 404"。
pub(crate) fn probe_target(
    provider_id: &str,
    base_url: &str,
    api_key: &str,
    model: &str,
    wire_api_raw: Option<&str>,
) -> Result<ProbeTarget, String> {
    use crate::model::wire::WireApi;

    let provider = crate::model::provider_protocol::provider_for_id(provider_id, base_url);
    let is_anthropic = provider.speaks_anthropic_messages();
    let wire = match wire_api_raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => WireApi::from_key(raw).ok_or_else(|| {
            format!("未知 wire_api: {raw}（可选：chat / responses）")
        })?,
        None => WireApi::Chat,
    };
    crate::model::wire::validate(
        wire,
        crate::model::provider_protocol::is_anthropic_family(provider_id),
    )?;

    let (url, body) = match wire {
        WireApi::Responses => (
            crate::model::wire::responses_url(base_url),
            json!({
                "model": model,
                "input": "hi",
                // Responses 的输出上限字段名与 Chat 不同
                "max_output_tokens": 5,
                "stream": false,
            }),
        ),
        WireApi::Chat if is_anthropic => (
            crate::model::llm::build_chat_url(base_url, true),
            // Anthropic 的 max_tokens 是**必填**；messages 的 content 可以是纯字符串
            json!({
                "model": model,
                "max_tokens": 5,
                "messages": [{ "role": "user", "content": "hi" }],
            }),
        ),
        WireApi::Chat => (
            crate::model::llm::build_chat_url(base_url, false),
            json!({
                "model": model,
                "messages": [{ "role": "user", "content": "hi" }],
                "max_tokens": 5,
                "stream": false,
            }),
        ),
    };

    let auth = if api_key.is_empty() {
        ProbeAuth::None
    } else if is_anthropic {
        match provider.anthropic_auth() {
            crate::model::llm::AnthropicAuth::XApiKey => ProbeAuth::XApiKey(api_key.to_string()),
            crate::model::llm::AnthropicAuth::Bearer => ProbeAuth::Bearer(api_key.to_string()),
        }
    } else {
        ProbeAuth::Bearer(api_key.to_string())
    };

    Ok(ProbeTarget { url, auth, body })
}

/// `onboarding.provider.probe`：验证 API Key 并尝试连接模型服务商。
pub(crate) async fn onboarding_provider_probe(params: &Value) -> Value {
    let api_key = params
        .get("apiKey")
        .or_else(|| params.get("api_key"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let base_url = params
        .get("baseUrl")
        .or_else(|| params.get("base_url"))
        .and_then(Value::as_str)
        .unwrap_or("https://api.deepseek.com")
        .to_string();
    let model_name = params
        .get("modelName")
        .or_else(|| params.get("model_name"))
        .or_else(|| params.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("deepseek-chat")
        .to_string();
    // 控制台发的是 `providerId`（见 useSetupProviderForm.connectionParams）。
    // 只认 `provider` 会让每次探测都退化成默认的 deepseek —— 于是无论用户选
    // 什么服务商，探测都按 OpenAI 兼容 + Bearer 发。
    let provider = params
        .get("providerId")
        .or_else(|| params.get("provider_id"))
        .or_else(|| params.get("provider"))
        .and_then(Value::as_str)
        .unwrap_or("deepseek")
        .to_string();
    let wire_api_raw = params
        .get("wire_api")
        .or_else(|| params.get("wireApi"))
        .and_then(Value::as_str);

    // R72：**本地/回环端点豁免密钥**。Ollama / LM Studio 等本地推理服务不需要
    // API Key，原先一刀切「密钥不能为空」让用户永远无法添加本地模型
    // （用户口径「本地模型和 Ollama 都就绪了但手动添加无法连接」）。
    let is_local = provider.eq_ignore_ascii_case("ollama")
        || {
            let lower = base_url.to_lowercase();
            ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"]
                .iter()
                .any(|h| lower.contains(h))
        };
    if api_key.is_empty() && !is_local {
        return json!({ "ok": false, "error": "API 密钥不能为空" });
    }

    let target = match probe_target(&provider, &base_url, &api_key, &model_name, wire_api_raw) {
        Ok(t) => t,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let url = target.url;
    let body = target.body;

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("HTTP 客户端创建失败: {e}") }),
    };

    let mut req = client.post(&url).header("Content-Type", "application/json");
    req = match &target.auth {
        ProbeAuth::Bearer(key) => req.header("Authorization", format!("Bearer {key}")),
        ProbeAuth::XApiKey(key) => req
            .header("x-api-key", key.clone())
            .header("anthropic-version", "2023-06-01"),
        // 本地端点：不带任何鉴权头（此前硬塞 `Bearer local` 会让严格要求
        // 无鉴权的自建服务把一次本来能通的探测判成 401）
        ProbeAuth::None => req,
    };

    match req.json(&body).send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                json!({
                    "ok": true,
                    "connected": true,
                    "provider": provider,
                    "model": model_name,
                    "message": "连接成功"
                })
            } else {
                let text = resp.text().await.unwrap_or_default();
                let detail = if text.len() > 200 {
                    &text[..200]
                } else {
                    &text
                };
                json!({
                    "ok": false,
                    "connected": false,
                    "error": format!("服务商返回错误 {}: {}", status.as_u16(), detail)
                })
            }
        }
        Err(e) => json!({
            "ok": false,
            "connected": false,
            "error": format!("连接失败: {e}")
        }),
    }
}

#[cfg(test)]
mod tests {
    /// `onboarding.status` 的 `configPath` 必须是**配置文件**路径，不是数据目录。
    ///
    /// 回归背景：Rust 侧曾传 `Config::data_dir()`（`%LOCALAPPDATA%\Marguerite`，一个目录），
    /// 而 pyengine 侧契约测试要求 `payload["configPath"] == cfg.config_path`（文件）——
    /// 同一 RPC 字段两套语义。前端据此展示「More options live in &lt;path&gt;」并拼
    /// `--config &lt;path&gt;`，传目录即是坏命令。真机实测（18801）：字段值
    /// `C:\Users\...\AppData\Local\Marguerite`，`isDirectory()==true`。
    ///
    /// 钉法：只看 `onboarding_status` 函数体窗口，确认唯一合法来源 = `Config::config_path()`。
    /// 改回 `Config::data_dir()` 或另起字符串拼接，本用例必须失败。
    #[test]
    fn onboarding_status_config_path_points_at_the_config_file() {
        let src = include_str!("onboarding.rs");
        let start = src
            .find("pub(crate) fn onboarding_status")
            .expect("onboarding_status 必须存在");
        let rest = &src[start..];
        // 函数体窗口：到下一个顶层 `pub(crate) fn` 为止，避免命中文件里其它函数
        let end = rest[1..]
            .find("\npub(crate) fn ")
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        let body = &rest[..end];

        // 按**行**取窗口（而非字节切片）：本文件含中文注释，按字节切会落在字符中间 panic。
        // 再剥掉注释行 —— 守卫注释里正当地引用了 `Config::data_dir()` 说明历史缺陷，
        // 那是噪声不是违规（仓库既有教训：源码扫描必须处理「注释里的方法名」）。
        // 窗口 = 整个函数体的**代码行**（已按函数边界收窄，无需再猜偏移）。
        let window = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(window.contains("\"configPath\""), "必须返回 configPath 字段");

        assert!(
            window.contains("Config::config_path()"),
            "configPath 必须来自 Config::config_path()（配置文件），窗口:\n{window}"
        );
        assert!(
            !window.contains("Config::data_dir()"),
            "configPath 不得使用 Config::data_dir()（那是数据目录），窗口:\n{window}"
        );
        // 反向自检：扫描规则真的能命中违规写法（否则本守卫是空转的）
        let probe = "let x = Config::data_dir().to_string_lossy().to_string();\n\"configPath\": x,";
        let pwindow = probe
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(pwindow.contains("Config::data_dir()"), "扫描窗口必须能覆盖到赋值处");
    }
    /// 就绪判定：本地/回环端点**不需要密钥**也必须算「已配置」。
    ///
    /// 回归背景（2026-10-21 实测）：此前只看 `api_key.is_empty()`，Ollama 用户
    /// 永远看到「需要配置 / 请设置模型提供商」，而模型其实能跑。
    #[test]
    fn llm_readiness_exempts_local_endpoints_and_still_gates_cloud() {
        use super::llm_readiness;
        // 本地：免密钥即就绪，来源如实标 local
        assert_eq!(llm_readiness("ollama", "http://127.0.0.1:11434", ""), (true, "local"));
        assert_eq!(llm_readiness("custom", "http://localhost:1234/v1", ""), (true, "local"));
        // 云端：无密钥即未就绪
        assert_eq!(llm_readiness("deepseek", "https://api.deepseek.com", ""), (false, "none"));
        // 有密钥：不论端点一律就绪，来源 config
        assert_eq!(
            llm_readiness("deepseek", "https://api.deepseek.com", "sk-x"),
            (true, "config")
        );
        // 本地即使配了密钥，也按 config 口径（用户显式给了凭据，不要覆盖他的认知）
        assert_eq!(llm_readiness("ollama", "http://127.0.0.1:11434", "sk-x"), (true, "config"));
    }

    /// 自定义端点目录必须**真的能用**：这是「本地/云端 API 自定义选项缺失」
    /// 的回归守卫。
    ///
    /// 三个具体缺陷各自对应一条断言，全部来自真机现象：
    /// ① 没有 `custom_anthropic` → 用户自建的 Anthropic 兼容网关无从选择，
    ///    只能被迫选 openai 兼容项，请求打到 /chat/completions 得 404；
    /// ② `custom` 复用 openai 字段表，`api_key` 被标成 required →
    ///    无鉴权的自建端点（llama.cpp / 内网网关）填不出密钥就存不下来；
    /// ③ 没有协议选择字段 → 只提供 /v1/responses 的端点接不进来。
    #[test]
    fn custom_endpoint_catalog_offers_anthropic_and_protocol_choice() {
        use super::onboarding_catalog;
        use serde_json::Value;

        let catalog = onboarding_catalog();
        let providers = catalog["providers"].as_array().expect("providers 必须是数组");
        let find = |id: &str| {
            providers
                .iter()
                .find(|p| p["providerId"] == Value::String(id.to_string()))
                .unwrap_or_else(|| panic!("目录里缺少 provider {id}"))
        };
        let field_required = |p: &Value, name: &str| -> Option<bool> {
            p["fields"]
                .as_array()?
                .iter()
                .find(|f| f["name"] == Value::String(name.to_string()))?
                .get("required")?
                .as_bool()
        };

        // ① Anthropic 兼容自定义端点存在，且 protocol 由 provider id 决定（无需再选）
        let ca = find("custom_anthropic");
        assert_eq!(ca["requiresApiKey"], false, "自定义端点不得强制要求密钥");
        assert_eq!(field_required(ca, "base_url"), Some(true), "base_url 必须必填");
        assert_eq!(field_required(ca, "api_key"), Some(false), "api_key 必须可选");
        assert!(
            crate::model::provider_protocol::is_anthropic_family("custom_anthropic"),
            "custom_anthropic 必须落进 Anthropic Messages 协议族"
        );

        // ② OpenAI 兼容自定义端点：密钥可选、base_url 必填
        let cu = find("custom");
        assert_eq!(cu["requiresApiKey"], false);
        assert_eq!(field_required(cu, "base_url"), Some(true));
        assert_eq!(field_required(cu, "api_key"), Some(false));

        // ③ 协议下拉：chat / responses 两个值都要给，且默认 chat（存量零迁移）
        let wire = cu["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "wire_api")
            .expect("custom 必须暴露 wire_api 字段");
        assert_eq!(wire["type"], "select");
        assert_eq!(wire["default"], "chat");
        let choices: Vec<&str> = wire["choices"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(choices, vec!["chat", "responses"]);

        // 官方 anthropic 也能改端点（走代理/中转），但密钥仍必填
        let an = find("anthropic");
        assert_eq!(field_required(an, "api_key"), Some(true));
        assert_eq!(field_required(an, "base_url"), Some(false));
    }

    /// 两侧目录的 provider id 必须真实存在。
    ///
    /// 离线回退目录（本文件）与 Python 侧车目录各写一份 id 名单，最容易发生的
    /// 漂移是「Rust 这边凭空捏一个 id」—— 用户离线时能选中、配好，等侧车上线
    /// 却发现 `get_provider_spec` 抛 UnknownProvider。故这里直接读侧车注册表
    /// 源码做对账（读不到即测试失败，绝不静默跳过）。
    #[test]
    fn rust_fallback_catalog_ids_exist_in_the_python_registry() {
        use super::onboarding_catalog;

        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pyengine/src/opensquilla/provider/registry.py"
        );
        let registry = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("读不到 Python provider 注册表 {path}: {e}"));

        let catalog = onboarding_catalog();
        let mut checked = 0usize;
        for p in catalog["providers"].as_array().unwrap() {
            let id = p["providerId"].as_str().expect("providerId 必须是字符串");
            // 形如 `"openai",` / `"custom_anthropic",` 的一行 —— 注册表里每个 id
            // 都是 _spec(...) 的第一个位置参数。
            let needle = format!("\"{id}\"");
            assert!(
                registry.contains(&needle),
                "Rust 目录里的 provider id `{id}` 在 pyengine/provider/registry.py 中不存在：\
                 离线可配、上线即 UnknownProvider"
            );
            checked += 1;
        }
        assert!(checked >= 10, "目录条目过少（{checked}），这条守卫已失去意义");
    }

    /// 「测试连接」必须按端点的**真实协议**发请求。
    ///
    /// 旧实现恒发 `{base}/chat/completions` + `Authorization: Bearer`：
    /// Anthropic 家族与 Responses 端点一律失败，且失败文案只写「连接失败」，
    /// 把协议错配伪装成密钥/模型名错。
    #[test]
    fn probe_target_speaks_each_endpoints_real_protocol() {
        use super::{probe_target, ProbeAuth};

        // OpenAI 兼容：/v1/chat/completions + Bearer
        let t = probe_target("openai", "https://api.openai.com", "sk-a", "gpt-4o", None).unwrap();
        assert_eq!(t.url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(t.auth, ProbeAuth::Bearer("sk-a".into()));
        assert_eq!(t.body["max_tokens"], 5);

        // 已带 /v1 的 base 不得拼出 /v1/v1（Ollama、OpenRouter 的老坑）
        let t = probe_target("custom", "http://127.0.0.1:11434/v1/", "", "qwen3:8b", Some("chat"))
            .unwrap();
        assert_eq!(t.url, "http://127.0.0.1:11434/v1/chat/completions");
        assert_eq!(t.auth, ProbeAuth::None, "本地端点不得硬塞 Bearer local");

        // Anthropic 官方：/v1/messages + x-api-key
        let t = probe_target("anthropic", "https://api.anthropic.com", "sk-ant", "claude-sonnet-4-20250514", None)
            .unwrap();
        assert_eq!(t.url, "https://api.anthropic.com/v1/messages");
        assert_eq!(t.auth, ProbeAuth::XApiKey("sk-ant".into()));
        assert_eq!(t.body["max_tokens"], 5, "Anthropic 的 max_tokens 是必填");

        // Anthropic 兼容网关（Bearer 风格）：协议同 /messages，密钥头不同
        let t = probe_target("custom_anthropic", "https://gw.example.com/v1", "sk-b", "claude-3-7-sonnet", None)
            .unwrap();
        assert_eq!(t.url, "https://gw.example.com/v1/messages");
        assert_eq!(t.auth, ProbeAuth::Bearer("sk-b".into()));

        // Responses：/v1/responses + max_output_tokens，且不带 chat 的 messages
        let t = probe_target("custom", "https://gw.example.com", "sk-c", "gpt-4o", Some("responses"))
            .unwrap();
        assert_eq!(t.url, "https://gw.example.com/v1/responses");
        assert_eq!(t.body["max_output_tokens"], 5);
        assert!(t.body.get("messages").is_none(), "Responses 用 input，不用 messages");
        assert_eq!(t.body["input"], "hi");

        // 不兼容组合与拼写错误都要响亮失败，而不是偷偷按 chat 探
        assert!(probe_target("anthropic", "https://api.anthropic.com", "k", "m", Some("responses"))
            .unwrap_err()
            .contains("anthropic"));
        assert!(probe_target("openai", "https://api.openai.com", "k", "m", Some("respones"))
            .unwrap_err()
            .contains("respones"));
    }
}
