//! 宝塔面板（BT Panel）MCP 工具集
//!
//! 从 `C:\Users\Administrator\WorkBuddy\xiaoyuangou\tools\baota_bridge` 抽取归纳的
//! 宝塔面板操控能力，封装为标准 `ToolExecutor` 工具集，通过 `ToolRegistry` 注册。
//!
//! 能力面：站点 / 数据库 / 文件 / FTP / Shell / 系统信息（对齐 baota_bridge 20 方法）。
//!
//! 配置（环境变量，沿用 baota_bridge Config 语义）：
//! - `BAOTA_PANEL_URL`   面板地址（如 https://1.2.3.4:8888）
//! - `BAOTA_USERNAME`    登录用户名
//! - `BAOTA_PASSWORD`    登录密码
//! - `BAOTA_SECRET_KEY`  会话加密密钥（解密 `BAOTA_COOKIE_JSON` 中的 byok 密文）
//! - `BAOTA_COOKIE_JSON` 已登录会话 cookie（JSON 对象；可为 byok 密文，配合 SECRET_KEY）
//! - `BAOTA_DRY_RUN`     默认 true：所有写操作仅校验不实发
//! - `BAOTA_CONFIRM_EXEC` 默认 true：Shell 等危险操作需二次确认字段
//! - `BAOTA_TLS_VERIFY`  默认 true：TLS 证书校验（F-004；显式设 false 才关闭，且会告警）
//! - `BAOTA_TLS_CA_CERT` 面板自签证书 PEM 路径：钉固该证书同时保留校验（推荐替代关校验）
//! - `BAOTA_SKIP_CODE_CHECK` 默认 false：设 1 可跳过写文件前的语法门禁
//!
//! 安全默认：dry_run / confirm_exec / tls_verify / 代码语法门禁 均为开启；
//! 危险操作在未显式放行时返回需要确认的错误。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::guard::syntax::{Language, SyntaxChecker};
use crate::guard::Severity;
use crate::mcp::registry::ToolExecutor;
use crate::mcp::registry::ToolRegistry;
use crate::tools::baota_guard::{guarded_write, BackupLedger, PrecheckGate};
use crate::tools::builtin::make_local_tool;

/// 工具执行闭包类型（与 builtin.rs 保持一致）
type ToolFn = Arc<
    dyn for<'a> Fn(
            &'a HashMap<String, String>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + 'a>,
        > + Send
        + Sync,
>;

fn ok(data: String) -> Result<ToolResult> {
    Ok(ToolResult::Success {
        data,
        metadata: HashMap::new(),
    })
}

fn err(message: String, code: &str) -> Result<ToolResult> {
    Ok(ToolResult::Error {
        message,
        code: code.to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────
// 配置
// ─────────────────────────────────────────────────────────────────────────

/// 严格布尔解析：仅显式真值返回 true，其余（含空串/乱码）一律 false。
///
/// 用于所有安全闸门的 fail-closed 语义——无法确认为「是」时必须视为「否」。
fn parse_bool_strict(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// 读取布尔型环境变量；未设置时用 `default`，设置了则按严格布尔解析。
pub(crate) fn env_bool(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => parse_bool_strict(&v),
        _ => default,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct BaotaConfig {
    panel_url: String,
    username: String,
    password: String,
    /// 会话密钥：解密 `BAOTA_COOKIE_JSON` 中的 byok 密文（`mg1:` 前缀）
    secret_key: String,
    dry_run: bool,
    confirm_exec: bool,
    /// F-004：TLS 证书校验开关，默认 true（安全默认）
    tls_verify: bool,
    /// F-004：面板自签证书 PEM 路径——钉固该证书为受信根，兼容自签且不牺牲校验
    tls_ca_cert: Option<String>,
}

impl BaotaConfig {
    fn from_env() -> Self {
        Self {
            panel_url: std::env::var("BAOTA_PANEL_URL").unwrap_or_default(),
            username: std::env::var("BAOTA_USERNAME").unwrap_or_default(),
            password: std::env::var("BAOTA_PASSWORD").unwrap_or_default(),
            secret_key: std::env::var("BAOTA_SECRET_KEY").unwrap_or_default(),
            dry_run: env_bool("BAOTA_DRY_RUN", true),
            confirm_exec: env_bool("BAOTA_CONFIRM_EXEC", true),
            // 安全默认：证书校验开启；只有用户显式 opt-out 才关闭
            tls_verify: tls_verify_from_env(),
            tls_ca_cert: tls_ca_cert_from_env(),
        }
    }

    fn is_configured(&self) -> bool {
        !self.panel_url.is_empty() && !self.username.is_empty() && !self.password.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 宝塔 API 客户端（Rust 实现，对齐 baota_bridge baota_api.py）
// ─────────────────────────────────────────────────────────────────────────

/// 轻量宝塔 API 客户端：cookie 登录 + x-http-token + JSON POST。
///
/// 说明：宝塔面板的登录需要获取 request_token 并签名。本实现提供两种模式：
/// 1. `direct`（默认）：若面板已开启 API 免密/已有合法 cookie（state 文件），直接携带调用；
/// 2. `env`：从环境变量读取登录态（BAOTA_USERNAME/PASSWORD 走 /login 签名登录）。
///
/// 由于不同面板版本签名算法差异，登录失败返回结构化错误，引导用户配置 state 文件。
pub(crate) struct BaotaClient {
    config: BaotaConfig,
    http: reqwest::Client,
    cookies: HashMap<String, String>,
}

/// 构造面板 HTTP 客户端（F-004 TLS 策略集中于此）。
///
/// 优先级：
/// 1. `BAOTA_TLS_CA_CERT` 存在 → 把面板自签证书加为受信根，**保留**证书校验（推荐路径）；
/// 2. `BAOTA_TLS_VERIFY=false` → 显式 opt-out，关闭校验并打醒目告警；
/// 3. 默认 → 走系统根证书，正常校验。
pub(crate) fn build_panel_http_client(
    tls_verify: bool,
    tls_ca_cert: Option<&str>,
) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20));

    if let Some(path) = tls_ca_cert {
        match std::fs::read(path) {
            Ok(pem) => match reqwest::Certificate::from_pem(&pem) {
                Ok(cert) => {
                    builder = builder.add_root_certificate(cert);
                    tracing::info!("[baota] 已钉固面板证书: {path}（保留 TLS 校验）");
                }
                Err(e) => tracing::warn!(
                    "[baota] BAOTA_TLS_CA_CERT 解析失败({path}): {e}；回退系统根证书。\
                     请确认文件为 PEM 格式（-----BEGIN CERTIFICATE-----）"
                ),
            },
            Err(e) => {
                tracing::warn!("[baota] BAOTA_TLS_CA_CERT 读取失败({path}): {e}；回退系统根证书")
            }
        }
    }

    if !tls_verify {
        tracing::warn!(
            "[baota] ⚠ 安全警告：TLS 证书校验已被显式关闭（BAOTA_TLS_VERIFY=false）。\
             面板用户名/密码与会话 cookie 将经未校验的 TLS 通道传输，存在中间人窃密与篡改风险。\
             建议改用 BAOTA_TLS_CA_CERT 钉固面板自签证书以恢复校验。"
        );
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder.build().unwrap_or_default()
}

/// 读取 `BAOTA_TLS_VERIFY`（默认 true）。供部署路径复用同一 TLS 策略。
pub(crate) fn tls_verify_from_env() -> bool {
    env_bool("BAOTA_TLS_VERIFY", true)
}

/// 读取 `BAOTA_TLS_CA_CERT`（自签证书 PEM 路径）。供部署路径复用同一 TLS 策略。
pub(crate) fn tls_ca_cert_from_env() -> Option<String> {
    std::env::var("BAOTA_TLS_CA_CERT")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 给 TLS 类传输错误补上可操作的指引，避免「默认开启校验」把自签面板用户直接锁死。
fn decorate_transport_error(e: &reqwest::Error) -> String {
    let raw = e.to_string();
    let lower = raw.to_ascii_lowercase();
    let tls_like = lower.contains("certificate")
        || lower.contains("self signed")
        || lower.contains("self-signed")
        || lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("unknown issuer")
        || lower.contains("verify");
    if !tls_like {
        return raw;
    }
    format!(
        "{raw}\n\
         提示：宝塔面板常用自签证书（https://IP:8888），证书校验默认开启故此处失败。两种解法：\n\
         1）【推荐，保留校验】导出面板证书为 PEM 后设置 BAOTA_TLS_CA_CERT=/path/to/panel.pem；\n\
         例如 openssl s_client -showcerts -connect <IP>:8888 </dev/null 2>/dev/null \\\n\
          | openssl x509 -outform PEM > panel.pem\n\
         2）【不推荐，有中间人风险】确认处于可信内网后显式设置 BAOTA_TLS_VERIFY=false 关闭校验。"
    )
}

impl BaotaClient {
    pub(crate) fn new(config: BaotaConfig) -> Self {
        let http = build_panel_http_client(config.tls_verify, config.tls_ca_cert.as_deref());
        Self {
            config,
            http,
            cookies: HashMap::new(),
        }
    }

    fn api_base(&self) -> String {
        let url = self.config.panel_url.trim_end_matches('/');
        // API 接口不带安全入口路径（如 /C70AGNQm），只取 scheme://host
        if let Ok(parsed) = url.parse::<reqwest::Url>() {
            format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""))
        } else {
            url.to_string()
        }
    }

    pub(crate) async fn post(
        &self,
        endpoint: &str,
        data: HashMap<String, String>,
    ) -> Result<Value> {
        let base = self.api_base();
        let url = format!(
            "{}/{}",
            base.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        );
        let mut req = self.http.post(&url).form(&data);
        for (k, v) in &self.cookies {
            req = req.header(reqwest::header::COOKIE, format!("{}={}", k, v));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("{}", decorate_transport_error(&e)))?;
        let text = resp.text().await?;
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("面板返回非 JSON: {e}: {text}"))
    }

    /// 载入外部注入的会话 cookie（`BAOTA_COOKIE_JSON`）。
    ///
    /// 支持两种载荷：
    /// - 明文 JSON 对象，如 `{"request_token":"...","BT_PANEL":"..."}`；
    /// - byok 密文（`mg1:` 前缀）——用 `BAOTA_SECRET_KEY` 解密后再解析。
    ///   这正是 `secret_key` 的设计用途：会话凭据不落明文。
    ///
    /// 未设置该环境变量时返回 `Ok(None)`，由调用方继续走登录回退路径。
    fn load_injected_cookies(&self) -> Result<Option<HashMap<String, String>>> {
        let Ok(raw) = std::env::var("BAOTA_COOKIE_JSON") else {
            return Ok(None);
        };
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            return Ok(None);
        }

        let payload = if crate::model::byok::is_encrypted(&raw) {
            if self.config.secret_key.is_empty() {
                return Err(anyhow::anyhow!(
                    "BAOTA_COOKIE_JSON 是加密载荷，但未设置 BAOTA_SECRET_KEY 用于解密"
                ));
            }
            crate::model::byok::decrypt_with_key(&raw, &self.config.secret_key).map_err(|e| {
                anyhow::anyhow!("BAOTA_COOKIE_JSON 解密失败（请检查 BAOTA_SECRET_KEY）: {e}")
            })?
        } else {
            raw
        };

        let parsed: Value = serde_json::from_str(&payload)
            .map_err(|e| anyhow::anyhow!("BAOTA_COOKIE_JSON 不是合法 JSON: {e}"))?;
        let Some(obj) = parsed.as_object() else {
            return Err(anyhow::anyhow!("BAOTA_COOKIE_JSON 必须是 JSON 对象"));
        };
        let cookies: HashMap<String, String> = obj
            .iter()
            .map(|(k, v)| {
                let val = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), val)
            })
            .collect();
        if cookies.is_empty() {
            return Ok(None);
        }
        Ok(Some(cookies))
    }

    /// 登录：优先使用外部注入的会话 cookie；否则回退签名登录（暂不支持，返回指引）。
    async fn ensure_login(&mut self) -> Result<()> {
        if !self.cookies.is_empty() || !self.config.is_configured() {
            return Ok(());
        }
        // 1. 外部注入会话：此前仅在错误文案里承诺、实际从未读取（功能缺失），现补齐
        if let Some(cookies) = self.load_injected_cookies()? {
            self.cookies = cookies;
            return Ok(());
        }
        // 2. 回退：宝塔 /login 需要 request_token；先探测面板可达性以给出准确错误
        self.http
            .get(format!("{}/login", self.api_base()))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("面板不可达: {}", decorate_transport_error(&e)))?;
        // 简化：无法在纯 Rust 侧稳定复刻各版本签名算法，返回需要外部 state 的指引
        Err(anyhow::anyhow!(
            "宝塔登录签名需面板版本匹配；请先运行 baota_bridge cli.py login 生成 state 文件，\
             或配置已登录 cookie 到 BAOTA_COOKIE_JSON（JSON 对象；可用 byok 加密并配合 BAOTA_SECRET_KEY）"
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 工具工厂
// ─────────────────────────────────────────────────────────────────────────

fn schema(required: Vec<&str>, props: serde_json::Map<String, Value>) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

fn str_prop(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}

fn int_prop(desc: &str) -> Value {
    json!({ "type": "integer", "description": desc })
}

fn bool_prop(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}

/// 构造单工具（闭包承载，复用 builtin::make_local_tool）
fn bt_tool(
    name: &'static str,
    description: &'static str,
    schema: Value,
    run: ToolFn,
) -> Box<dyn ToolExecutor> {
    make_local_tool(name, description, schema, run)
}

/// 检查配置与登录态（所有工具共用前置）
pub(crate) fn require_ready(
    cfg: &BaotaConfig,
    needs_write: bool,
) -> std::result::Result<(), String> {
    if !cfg.is_configured() {
        return Err(
            "宝塔面板未配置：请设置 BAOTA_PANEL_URL / BAOTA_USERNAME / BAOTA_PASSWORD".into(),
        );
    }
    if needs_write && cfg.dry_run {
        return Err("当前为 BAOTA_DRY_RUN=true（安全默认），写操作未实发；如需实发请显式设置 BAOTA_DRY_RUN=false".into());
    }
    Ok(())
}

/// 校验写操作是否带确认标记（F-003）。
///
/// 修复前只校验「键是否存在」，导致 `confirm=false` / 空串 / 任意乱码都能穿过这道二次确认闸门。
/// 现在必须解析**值**，且 fail-closed：只有显式真值才放行。
pub(crate) fn check_confirm(
    args: &HashMap<String, String>,
    cfg: &BaotaConfig,
) -> std::result::Result<(), String> {
    if !cfg.confirm_exec {
        return Ok(());
    }
    match args.get("confirm") {
        Some(v) if parse_bool_strict(v) => Ok(()),
        Some(v) => Err(format!(
            "危险操作被拒绝：confirm 必须为 true，收到 {v:?}（BAOTA_CONFIRM_EXEC 默认开启）"
        )),
        None => Err("危险操作需 confirm=true 字段确认（BAOTA_CONFIRM_EXEC 默认开启）".into()),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// 上传前语法门禁（需求#4）
//
// 复用已实现但此前未接线的 `guard::syntax::SyntaxChecker`（tree-sitter AST）。
// 策略：按扩展名选择语言；识别不出语言的文件（.txt/.conf/.json/无扩展名等）
// 走宽松路径直接放行，避免误杀配置与文本资产。
// ─────────────────────────────────────────────────────────────────────────

/// 全局共享语法检查器：tree-sitter Parser 构造开销大，进程内只建一次。
fn syntax_checker() -> &'static SyntaxChecker {
    static CHECKER: std::sync::OnceLock<SyntaxChecker> = std::sync::OnceLock::new();
    CHECKER.get_or_init(SyntaxChecker::new)
}

/// 是否跳过上传前代码校验（与 Python MCP 侧 `BAOTA_SKIP_CODE_CHECK` 语义对齐）。
fn skip_code_check() -> bool {
    env_bool("BAOTA_SKIP_CODE_CHECK", false)
}

/// 由路径扩展名判定语言；非代码文件返回 `None`（宽松放行）。
pub(crate) fn language_for_path(path: &str) -> Option<Language> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    Language::from_extension(ext)
}

/// 该路径是否需要做语法校验（供上传路径先判类型、避免读取大二进制）。
/// 注：部署上传路径已改用 `deploy::precheck`（系统解释器校验），此处仅保留
/// `precheck_code` 供工具集合内部使用；`is_checkable_code_path` 已移除。
pub(crate) fn precheck_code(path: &str, content: &str) -> std::result::Result<(), String> {
    if skip_code_check() {
        return Ok(());
    }
    // 非代码文件（配置/文本/二进制）宽松放行，不误杀
    let Some(lang) = language_for_path(path) else {
        return Ok(());
    };

    let issues = syntax_checker().check(content, lang);
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }

    const MAX_SHOWN: usize = 5;
    let mut msg = format!(
        "上传被拒绝：{} 语法校验未通过（{} 处错误）\n文件: {}",
        lang.display(),
        errors.len(),
        path
    );
    for issue in errors.iter().take(MAX_SHOWN) {
        let loc = issue
            .range
            .map(|r| format!("L{}:{}", r.start_line + 1, r.start_column + 1))
            .unwrap_or_else(|| "-".to_string());
        msg.push_str(&format!("\n  ✗ {} {}", loc, issue.message));
        if let Some(s) = &issue.suggestion {
            msg.push_str(&format!("（建议：{s}）"));
        }
    }
    if errors.len() > MAX_SHOWN {
        msg.push_str(&format!("\n  … 其余 {} 处已省略", errors.len() - MAX_SHOWN));
    }
    msg.push_str("\n请修正语法后重试；如确需跳过校验，设置 BAOTA_SKIP_CODE_CHECK=1。");
    Err(msg)
}

// ─────────────────────────────────────────────────────────────────────────
// 工具集合构建
// ─────────────────────────────────────────────────────────────────────────

/// 注册全部宝塔面板工具到 registry。
///
/// 工具命名：`baota_<action>`（下划线形式，与 `baota_bridge` 方法一一对应），
/// 且**必须在调用方就写合法**——OpenAI/DeepSeek function name 规范 `^[a-zA-Z0-9_-]+$`
/// 不接受点号。曾经这里用 `baota.<action>`，导致注册表每次都要「自动修正」
/// 并刷 WARN（首启 stderr 噪音，P1-E）；改名后首次注册即合法，修正路径不再触发。
/// 全部工具为真实 HTTP 调用（登录态由 client 承载）；dry_run 时写操作返回引导错误。
pub fn register_baota_tools(registry: &mut ToolRegistry) -> Result<()> {
    let cfg = BaotaConfig::from_env();
    let client = Arc::new(tokio::sync::Mutex::new(BaotaClient::new(cfg.clone())));

    // ── 系统 ──
    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_system_info",
        "获取宝塔面板系统信息（CPU/内存/磁盘）",
        schema(vec![], serde_json::Map::new()),
        Arc::new(move |_args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let r = g
                    .post("/system?action=GetSystemTotal", HashMap::new())
                    .await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_network_info",
        "获取面板网络信息",
        schema(vec![], serde_json::Map::new()),
        Arc::new(move |_args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let r = g.post("/system?action=GetNetWork", HashMap::new()).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_exec_shell",
        "在宝塔面板执行 Shell 命令（危险，需 confirm）",
        schema(
            vec!["cmd"],
            serde_json::Map::from_iter(vec![
                ("cmd".into(), str_prop("要执行的命令")),
                ("confirm".into(), bool_prop("危险操作确认，必须为 true")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let cmd = args.get("cmd").cloned().unwrap_or_default();
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, true) {
                    return err(e, "CONFIG");
                }
                if let Err(e) = check_confirm(args, &cfg2) {
                    return err(e, "CONFIRM");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("shell".into(), cmd);
                let r = g.post("/ajax?action=GetTaskSpeed", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    // ── 站点 ──
    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_list_sites",
        "列出宝塔面板站点（分页）",
        schema(
            vec![],
            serde_json::Map::from_iter(vec![
                ("page".into(), int_prop("页码，默认 1")),
                ("limit".into(), int_prop("每页数量，默认 15")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let page = args.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
            let limit = args.get("limit").and_then(|v| v.parse().ok()).unwrap_or(15);
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("p".into(), page.to_string());
                d.insert("limit".into(), limit.to_string());
                let r = g.post("/site/get_site_list", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_get_site_info",
        "获取单个站点详情",
        schema(
            vec!["site_name"],
            serde_json::Map::from_iter(vec![("site_name".into(), str_prop("站点名称"))]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let name = args.get("site_name").cloned().unwrap_or_default();
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("siteName".into(), name);
                let r = g.post("/site?action=GetSiteInfo", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_create_site",
        "创建站点（写操作，需 dry_run=false）",
        schema(
            vec!["webname", "path"],
            serde_json::Map::from_iter(vec![
                ("webname".into(), str_prop("站点域名，多个用逗号分隔")),
                ("path".into(), str_prop("站点根目录路径")),
                ("php_version".into(), str_prop("PHP 版本，如 82")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let webname = args.get("webname").cloned().unwrap_or_default();
            let path = args.get("path").cloned().unwrap_or_default();
            let php = args
                .get("php_version")
                .cloned()
                .unwrap_or_else(|| "82".into());
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, true) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("webname".into(), webname);
                d.insert("path".into(), path);
                d.insert("php_version".into(), php);
                let r = g.post("/site?action=AddSite", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_set_site_status",
        "启用/停用站点（写操作，需 dry_run=false）",
        schema(
            vec!["site_name", "status"],
            serde_json::Map::from_iter(vec![
                ("site_name".into(), str_prop("站点名称")),
                ("status".into(), int_prop("0 停用 / 1 启用")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let name = args.get("site_name").cloned().unwrap_or_default();
            let status = args.get("status").and_then(|v| v.parse().ok()).unwrap_or(0);
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, true) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("id".into(), name);
                d.insert("status".into(), status.to_string());
                let r = g.post("/site?action=SetSiteStatus", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    // ── 文件 ──
    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_list_files",
        "列出目录文件",
        schema(
            vec![],
            serde_json::Map::from_iter(vec![(
                "path".into(),
                str_prop("目录路径，默认 /www/wwwroot"),
            )]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let path = args
                .get("path")
                .cloned()
                .unwrap_or_else(|| "/www/wwwroot".into());
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("path".into(), path);
                let r = g.post("/files?action=GetDir", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_read_file",
        "读取文件内容",
        schema(
            vec!["path"],
            serde_json::Map::from_iter(vec![("path".into(), str_prop("文件路径"))]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let path = args.get("path").cloned().unwrap_or_default();
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("path".into(), path);
                let r = g.post("/files?action=GetFileBody", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_write_file",
        "写入文件内容（写操作，需 dry_run=false；统一门禁：语法校验 → 远端备份 → 写入 → 读回校验，任一失败自动回滚）",
        schema(
            vec!["path", "content"],
            serde_json::Map::from_iter(vec![
                ("path".into(), str_prop("文件路径")),
                ("content".into(), str_prop("文件内容")),
                ("confirm".into(), str_prop("二次确认；仅当 BAOTA_CONFIRM_WRITE_FILE=1 时必填")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone(); let cfg2 = cfg2.clone();
            let args = args.clone();
            let path = args.get("path").cloned().unwrap_or_default();
            let content = args.get("content").cloned().unwrap_or_default();
            Box::pin(async move {
                // 需求#6①：三道闸门收口成一次 admit，任一不过即 fail-closed
                if let Err(reject) =
                    PrecheckGate::for_write_file().admit(&cfg2, &args, &path, &content)
                {
                    return err(reject.message, reject.code);
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await { return err(e.to_string(), "AUTH"); }
                // 备份 → 写入 → 读回校验 → 失败回滚
                let mut ledger = BackupLedger::default_ledger();
                match guarded_write(&*g, &path, &content, &mut ledger).await {
                    Ok(report) => ok(serde_json::to_string(&report)
                        .unwrap_or_else(|_| "{\"verified\":false}".into())),
                    Err(message) => err(message, "WRITE_GUARD"),
                }
            })
        }),
    ));

    // ── 数据库 ──
    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_list_databases",
        "列出数据库（分页）",
        schema(
            vec![],
            serde_json::Map::from_iter(vec![
                ("page".into(), int_prop("页码，默认 1")),
                ("limit".into(), int_prop("每页数量，默认 15")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let page = args.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
            let limit = args.get("limit").and_then(|v| v.parse().ok()).unwrap_or(15);
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("p".into(), page.to_string());
                d.insert("limit".into(), limit.to_string());
                let r = g.post("/database?action=GetList", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_create_database",
        "创建数据库（写操作，需 dry_run=false）",
        schema(
            vec!["name"],
            serde_json::Map::from_iter(vec![
                ("name".into(), str_prop("数据库名")),
                ("password".into(), str_prop("数据库密码；留空自动生成")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let name = args.get("name").cloned().unwrap_or_default();
            let password = args.get("password").cloned().unwrap_or_default();
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, true) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("name".into(), name);
                d.insert("password".into(), password);
                let r = g.post("/database?action=AddDatabase", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    // ── FTP ──
    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_list_ftp",
        "列出 FTP 账号（分页）",
        schema(
            vec![],
            serde_json::Map::from_iter(vec![
                ("page".into(), int_prop("页码，默认 1")),
                ("limit".into(), int_prop("每页数量，默认 15")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let page = args.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
            let limit = args.get("limit").and_then(|v| v.parse().ok()).unwrap_or(15);
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, false) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("p".into(), page.to_string());
                d.insert("limit".into(), limit.to_string());
                let r = g.post("/ftp?action=GetList", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    let c = client.clone();
    let cfg2 = cfg.clone();
    registry.register_local(bt_tool(
        "baota_create_ftp",
        "创建 FTP 账号（写操作，需 dry_run=false）",
        schema(
            vec!["name"],
            serde_json::Map::from_iter(vec![
                ("name".into(), str_prop("FTP 用户名")),
                ("password".into(), str_prop("FTP 密码；留空自动生成")),
                ("path".into(), str_prop("FTP 根目录，默认 /www/wwwroot")),
            ]),
        ),
        Arc::new(move |args: &HashMap<String, String>| {
            let c = c.clone();
            let cfg2 = cfg2.clone();
            let name = args.get("name").cloned().unwrap_or_default();
            let password = args.get("password").cloned().unwrap_or_default();
            let path = args
                .get("path")
                .cloned()
                .unwrap_or_else(|| "/www/wwwroot".into());
            Box::pin(async move {
                if let Err(e) = require_ready(&cfg2, true) {
                    return err(e, "CONFIG");
                }
                let mut g = c.lock().await;
                if let Err(e) = g.ensure_login().await {
                    return err(e.to_string(), "AUTH");
                }
                let mut d = HashMap::new();
                d.insert("ftp_username".into(), name);
                d.insert("ftp_password".into(), password);
                d.insert("path".into(), path);
                let r = g.post("/ftp?action=AddUser", d).await;
                match r {
                    Ok(v) => ok(v.to_string()),
                    Err(e) => err(e.to_string(), "API"),
                }
            })
        }),
    ));

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// 测试
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造测试配置（避免逐个测试重复写全字段）
    fn cfg_with(configured: bool, dry_run: bool, confirm_exec: bool) -> BaotaConfig {
        BaotaConfig {
            panel_url: if configured {
                "https://1.2.3.4:8888".into()
            } else {
                String::new()
            },
            username: if configured {
                "u".into()
            } else {
                String::new()
            },
            password: if configured {
                "p".into()
            } else {
                String::new()
            },
            secret_key: String::new(),
            dry_run,
            confirm_exec,
            tls_verify: true,
            tls_ca_cert: None,
        }
    }

    #[test]
    fn test_config_from_env_defaults_safe() {
        let cfg = BaotaConfig::from_env();
        // 安全默认：dry_run / confirm_exec / tls_verify 应为 true（未显式设置时）
        assert!(cfg.dry_run, "dry_run 默认应为 true（安全默认）");
        assert!(cfg.confirm_exec, "confirm_exec 默认应为 true（安全默认）");
        assert!(
            cfg.tls_verify,
            "F-004: tls_verify 默认应为 true（证书校验默认开启）"
        );
    }

    #[test]
    fn test_require_ready_unconfigured() {
        assert!(require_ready(&cfg_with(false, true, true), false).is_err());
    }

    #[test]
    fn test_require_ready_dry_run_blocks_write() {
        let cfg = cfg_with(true, true, true);
        assert!(require_ready(&cfg, false).is_ok());
        assert!(
            require_ready(&cfg, true).is_err(),
            "写操作在 dry_run 下应被阻止"
        );
    }

    // ── F-003：confirm 必须校验「值」而非仅「键存在」 ──────────────────────

    #[test]
    fn test_parse_bool_strict_fail_closed() {
        for truthy in ["true", "TRUE", "True", "1", "yes", "on", "  true  "] {
            assert!(parse_bool_strict(truthy), "{truthy:?} 应被识别为真");
        }
        for falsy in [
            "false", "FALSE", "0", "no", "off", "", "   ", "乱码", "tru", "2", "null",
        ] {
            assert!(
                !parse_bool_strict(falsy),
                "{falsy:?} 必须 fail-closed 视为假"
            );
        }
    }

    /// F-003 回归：`confirm=false` / 空串 / 乱码 三种情况均必须被拒绝
    #[test]
    fn test_check_confirm_rejects_false_empty_and_garbage() {
        let cfg = cfg_with(true, false, true);
        for bad in ["false", "", "乱码abc", "0", "no", "off"] {
            let mut args = HashMap::new();
            args.insert("confirm".to_string(), bad.to_string());
            assert!(
                check_confirm(&args, &cfg).is_err(),
                "F-003: confirm={bad:?} 仅凭键存在就放行是漏洞，必须拒绝"
            );
        }
    }

    #[test]
    fn test_check_confirm_accepts_true_case_insensitive() {
        let cfg = cfg_with(true, false, true);
        for good in ["true", "TRUE", "True", "1", "yes"] {
            let mut args = HashMap::new();
            args.insert("confirm".to_string(), good.to_string());
            assert!(
                check_confirm(&args, &cfg).is_ok(),
                "confirm={good:?} 应放行"
            );
        }
    }

    #[test]
    fn test_check_confirm_missing_key_rejected() {
        let cfg = cfg_with(true, false, true);
        assert!(
            check_confirm(&HashMap::new(), &cfg).is_err(),
            "缺 confirm 键应拒绝"
        );
    }

    #[test]
    fn test_check_confirm_skipped_when_gate_disabled() {
        // 显式关闭二次确认时不再要求 confirm 字段
        let cfg = cfg_with(true, false, false);
        assert!(check_confirm(&HashMap::new(), &cfg).is_ok());
    }

    // ── F-004：TLS 校验默认开启，opt-out 可用且有指引 ─────────────────────

    #[test]
    fn test_build_panel_http_client_both_modes_succeed() {
        // 默认（校验开启）与显式 opt-out 都必须能构造出可用客户端，
        // 保证「默认开启」不会把自签面板用户直接锁死。
        let _verified = build_panel_http_client(true, None);
        let _optout = build_panel_http_client(false, None);
        // 无效 CA 路径应降级回落而非 panic
        let _bad_ca = build_panel_http_client(true, Some("/definitely/not/exist.pem"));
    }

    #[test]
    fn test_env_bool_defaults_when_unset() {
        assert!(env_bool("BAOTA_TEST_UNSET_KEY_XYZ", true));
        assert!(!env_bool("BAOTA_TEST_UNSET_KEY_XYZ", false));
    }

    // ── 需求#4：上传前语法门禁 ────────────────────────────────────────────

    #[test]
    fn test_language_for_path_maps_known_extensions() {
        assert_eq!(
            language_for_path("/www/app/main.py"),
            Some(Language::Python)
        );
        assert_eq!(
            language_for_path("/www/app/index.js"),
            Some(Language::JavaScript)
        );
        assert_eq!(
            language_for_path("/www/app/a.ts"),
            Some(Language::TypeScript)
        );
        assert_eq!(language_for_path("/www/app/lib.rs"), Some(Language::Rust));
        assert_eq!(language_for_path("/www/app/svc.go"), Some(Language::Go));
        // 非代码文件不参与语法校验
        assert_eq!(language_for_path("/www/app/readme.txt"), None);
        assert_eq!(language_for_path("/www/app/nginx.conf"), None);
        assert_eq!(language_for_path("/www/app/data.json"), None);
        assert_eq!(language_for_path("/www/app/Makefile"), None);
    }

    #[test]
    fn test_precheck_blocks_syntax_error_and_reports_detail() {
        // Python 缺冒号
        let e = precheck_code("/www/app/main.py", "def foo(x)\n    return x\n")
            .expect_err("语法错误必须阻止上传");
        assert!(e.contains("上传被拒绝"), "应明确告知被拒绝: {e}");
        assert!(e.contains("Python"), "应带上语言: {e}");
        assert!(e.contains('L'), "应带上错误行号定位: {e}");
        assert!(e.contains("BAOTA_SKIP_CODE_CHECK"), "应告知如何跳过: {e}");
    }

    #[test]
    fn test_precheck_blocks_broken_javascript() {
        assert!(
            precheck_code("/www/app/index.js", "function foo() {\n  return 1;\n").is_err(),
            "JS 缺右括号必须被拦截"
        );
    }

    #[test]
    fn test_precheck_allows_valid_code() {
        assert!(precheck_code("/www/app/main.py", "def foo(x):\n    return x\n").is_ok());
        assert!(precheck_code("/www/app/index.js", "const x = 1;\nconsole.log(x);\n").is_ok());
    }

    /// 宽松路径：非代码文件即便内容「不像代码」也不得误杀
    #[test]
    fn test_precheck_lenient_for_non_code_files() {
        let junk = "这是一段说明文字 {{{ 未闭合括号 ((( ";
        for p in [
            "/www/app/README.txt",
            "/www/app/nginx.conf",
            "/www/app/config.json",
            "/www/app/data.csv",
            "/www/app/binary_no_ext",
        ] {
            assert!(
                precheck_code(p, junk).is_ok(),
                "{p} 属非代码文件，应宽松放行"
            );
        }
    }

    #[test]
    fn test_register_baota_tools() {
        let mut registry = ToolRegistry::new();
        register_baota_tools(&mut registry).expect("注册应成功");
        let tools = registry.list_tools();
        // 工具名在调用方即是合法 function name（`^[a-zA-Z0-9_-]+$`），
        // `list_tools()` / `execute_local_tool()` / MCP `tools/call` 用的是同一个键，
        // 因此这里断言的名字就是对外可派发的名字。
        for expected in [
            "baota_system_info",
            "baota_network_info",
            "baota_exec_shell",
            "baota_list_sites",
            "baota_get_site_info",
            "baota_create_site",
            "baota_set_site_status",
            "baota_list_files",
            "baota_read_file",
            "baota_write_file",
            "baota_list_databases",
            "baota_create_database",
            "baota_list_ftp",
            "baota_create_ftp",
        ] {
            assert!(tools.contains(&expected.to_string()), "缺少工具 {expected}");
        }
        assert_eq!(tools.len(), 14, "应注册 14 个 BT 工具");

        // 根因回归（P1-E）：内置工具名必须「首次注册即合法」。
        // `legacy_aliases` 非空 ⇔ 注册时走过「非法名 → 自动修正」分支 ⇔ 又会产生
        // stderr 噪音（曾经的 14 行 WARN 正是这条路径）。因此这张表必须为空。
        assert!(
            registry.legacy_aliases().is_empty(),
            "内置 baota 工具名不得依赖注册后修正，实际别名: {:?}",
            registry.legacy_aliases()
        );

        // 反向守卫：目录（tools.catalog 对外名）同样必须合法且可派发。
        for item in registry.catalog() {
            assert_eq!(
                crate::mcp::registry::sanitize_tool_name(&item.name),
                item.name,
                "对外工具名 {} 含非法字符，注册后会退化为「注册后修正」",
                item.name
            );
            assert!(
                registry.has_tool(&item.name),
                "目录名必须可直接派发: {}",
                item.name
            );
        }
    }
}
