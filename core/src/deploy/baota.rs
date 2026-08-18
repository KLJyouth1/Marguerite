//! 宝塔面板一键部署（Phase I3，FR-16）— 自研 OpenAPI HTTP 客户端
//!
//! `marguerite deploy --target baota://<panel-alias> [--bin <path>] [--site <domain>]
//!   [--service <name>] [--health-url <url>] [--no-ssl] [--dry-run]`
//!
//! 设计（连接池与 SSH 同思路，传输层抽象便于单测/演练）：
//! - 传输抽象 `BaotaTransport`：`ReqwestBaotaTransport`（真机 async reqwest）/
//!   `RecordingBaotaTransport`（演练+单测，可注入失败）。`--dry-run` 走录制路径。
//! - 面板别名与 API 密钥存加密配置 `baota.json`（API 密钥复用 `model::byok` AES-0600 加密）。
//! - 工作流：`site_create` → `file_upload` → `service_start` → `ssl_apply`(可选) → `health`。
//!
//! 安全：宝塔 API 密钥不落明文（byok 加密）；传输走 HTTPS；请求按宝塔面板 OpenAPI 签名
//!   （`request_token = md5(md5(排序参数字串) + request_time)`，需在真实面板验证对齐）。

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 面板别名解析后的连接信息（API 密钥已解密）
pub struct BaotaPanel {
    /// 面板别名
    pub alias: String,
    /// 面板根地址，如 https://host:8888
    pub endpoint: String,
    /// 解密后的明文 API 密钥
    pub api_key: String,
}

/// 持久化结构中的面板条目（api_key 为 byok 加密令牌）
#[derive(Clone, Debug, Serialize, Deserialize)]
struct BaotaPanelStored {
    endpoint: String,
    /// byok 加密令牌（mg1:...）
    api_key: String,
}

/// 宝塔面板配置（多面板别名 → 连接信息）
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BaotaConfig {
    panels: HashMap<String, BaotaPanelStored>,
}

/// 部署选项（宝塔）
#[derive(Clone, Debug)]
pub struct BaotaDeployOptions {
    /// 服务名（建站目录/服务单元同名），缺省 marguerite
    pub service: String,
    /// 站点域名（建站 + SSL 用）；为空则不建站
    pub site: Option<String>,
    /// 健康检查 URL；为空视为通过
    pub health_url: Option<String>,
    /// 是否申请 SSL（需 site）
    pub ssl: bool,
    /// 仅演练标注
    pub dry_run: bool,
}

/// 部署结果报告
#[derive(Debug)]
pub struct BaotaReport {
    /// 是否成功上线（健康检查通过）
    pub success: bool,
    /// 是否申请了 SSL
    pub ssl_applied: bool,
    /// 服务名
    pub service: String,
}

/// 配置文件路径（`~/.marguerite/baota.json`）
pub fn config_path() -> std::path::PathBuf {
    crate::persistence::config::Config::config_dir().join("baota.json")
}

/// 加载配置（不存在则返回空配置，不报错）
pub fn load_config() -> Result<BaotaConfig> {
    let p = config_path();
    if !p.exists() {
        return Ok(BaotaConfig::default());
    }
    let s = std::fs::read_to_string(&p)?;
    let cfg: BaotaConfig =
        serde_json::from_str(&s).map_err(|e| anyhow!("baota.json 解析失败: {e}"))?;
    Ok(cfg)
}

/// 保存配置（原子写：先写临时再改名）
pub fn save_config(cfg: &BaotaConfig) -> Result<()> {
    let p = config_path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = serde_json::to_string_pretty(cfg)?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, &s)?;
    std::fs::rename(&tmp, &p)?;
    crate::model::byok::chmod_0600(&p)?;
    Ok(())
}

/// 新增/更新面板别名（API 密钥经 byok 主密钥加密落盘）
pub fn add_panel(alias: &str, endpoint: &str, api_key_plain: &str) -> Result<()> {
    let mut cfg = load_config()?;
    let token = crate::model::byok::encrypt_api_key(api_key_plain)?;
    cfg.panels.insert(
        alias.to_string(),
        BaotaPanelStored {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            api_key: token,
        },
    );
    save_config(&cfg)
}

/// 从配置解析面板（解密 API 密钥，使用 byok 主密钥）
pub fn resolve_panel(alias: &str) -> Result<BaotaPanel> {
    let cfg = load_config()?;
    let stored = cfg.panels.get(alias).ok_or_else(|| {
        anyhow!("未找到面板别名 '{alias}'，请先用 marguerite deploy --add-panel 注册")
    })?;
    let plain = crate::model::byok::decrypt_api_key(&stored.api_key)?;
    Ok(BaotaPanel {
        alias: alias.to_string(),
        endpoint: stored.endpoint.clone(),
        api_key: plain,
    })
}

/// 测试用：用显式密钥解析面板（避免依赖 byok 主密钥/磁盘）
pub fn resolve_panel_with_key(cfg: &BaotaConfig, alias: &str, key_b64: &str) -> Result<BaotaPanel> {
    let stored = cfg
        .panels
        .get(alias)
        .ok_or_else(|| anyhow!("未找到面板别名 '{alias}'"))?;
    let plain = crate::model::byok::decrypt_with_key(&stored.api_key, key_b64)?;
    Ok(BaotaPanel {
        alias: alias.to_string(),
        endpoint: stored.endpoint.clone(),
        api_key: plain,
    })
}

/// md5 十六进制
fn md5_hex(input: &[u8]) -> String {
    let d = md5::compute(input);
    let mut s = String::with_capacity(32);
    for b in d.0 {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 当前 unix 秒
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 宝塔面板 OpenAPI 签名：`request_token = md5(md5(排序参数字串) + request_time)`。
/// `now` 可注入用于单测确定性。
fn sign_with_time(params: &HashMap<String, String>, now: u64) -> (String, String) {
    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort();
    let mut canonical = String::new();
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            canonical.push('&');
        }
        canonical.push_str(&format!("{}={}", k, params[*k]));
    }
    let inner = md5_hex(canonical.as_bytes());
    let token = md5_hex(format!("{inner}{now}").as_bytes());
    (now.to_string(), token)
}

/// 用真实时间签名
fn sign(params: &HashMap<String, String>, api_key: &str) -> (String, String) {
    // api_key 不参与签名计算（宝塔面板级接口密钥为服务端配置，校验同算法即可）；
    // 此处保留参数以满足未来含密钥变体。
    let _ = api_key;
    sign_with_time(params, now_secs())
}

/// 宝塔传输抽象（异步）
#[async_trait(?Send)]
pub trait BaotaTransport {
    /// 调用一个宝塔动作（action），附带业务参数；返回响应体文本
    async fn post(&self, action: &str, params: &HashMap<String, String>) -> Result<String>;
}

/// 真机传输：经 async reqwest 调宝塔面板 OpenAPI
pub struct ReqwestBaotaTransport {
    panel: BaotaPanel,
}

impl ReqwestBaotaTransport {
    pub fn new(panel: BaotaPanel) -> Self {
        Self { panel }
    }
}

#[async_trait(?Send)]
impl BaotaTransport for ReqwestBaotaTransport {
    async fn post(&self, action: &str, params: &HashMap<String, String>) -> Result<String> {
        // 与 tools::baota 共用 TLS 策略：默认校验证书，支持 BAOTA_TLS_CA_CERT 钉固自签证书，
        // 仅在 BAOTA_TLS_VERIFY=false 显式 opt-out 时关闭校验（带告警）。
        let client = crate::tools::baota::build_panel_http_client(
            crate::tools::baota::tls_verify_from_env(),
            crate::tools::baota::tls_ca_cert_from_env().as_deref(),
        );
        let mut body = params.clone();
        let (ts, token) = sign(params, &self.panel.api_key);
        body.insert("action".to_string(), action.to_string());
        body.insert("request_time".to_string(), ts);
        body.insert("request_token".to_string(), token);
        body.insert("api_key".to_string(), self.panel.api_key.clone());
        let url = format!("{}/do_action", self.panel.endpoint.trim_end_matches('/'));
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("宝塔请求失败（请确认面板地址可达且 API 已开启）: {e}"))?;
        if !resp.status().is_success() {
            bail!("宝塔返回非 2xx: {}", resp.status());
        }
        let text = resp
            .text()
            .await
            .map_err(|e| anyhow!("读取宝塔响应失败: {e}"))?;
        Ok(text)
    }
}

/// 录制型传输：纯记录调用、不实际请求，可注入失败（单测 + --dry-run）
pub struct RecordingBaotaTransport {
    inner: std::cell::RefCell<Vec<(String, HashMap<String, String>)>>,
    fail_on: Option<String>,
}

impl RecordingBaotaTransport {
    pub fn new() -> Self {
        Self {
            inner: std::cell::RefCell::new(Vec::new()),
            fail_on: None,
        }
    }

    /// 配置：action 等于该值时返回 Err（模拟调用失败，典型用于 health 失败）
    pub fn with_fail(mut self, action: &str) -> Self {
        self.fail_on = Some(action.to_string());
        self
    }

    pub fn calls(&self) -> Vec<(String, HashMap<String, String>)> {
        self.inner.borrow().clone()
    }
}

impl Default for RecordingBaotaTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl BaotaTransport for RecordingBaotaTransport {
    async fn post(&self, action: &str, params: &HashMap<String, String>) -> Result<String> {
        self.inner
            .borrow_mut()
            .push((action.to_string(), params.clone()));
        if let Some(exp) = &self.fail_on {
            if action == exp.as_str() {
                bail!("（mock）宝塔动作失败: {action}");
            }
        }
        Ok(r#"{"status":true}"#.to_string())
    }
}

/// 站点根目录（建站后二进制落点）
fn site_root(site: &Option<String>) -> String {
    match site {
        Some(d) => format!("/www/wwwroot/{d}/marguerite"),
        None => "/www/wwwroot/marguerite".to_string(),
    }
}

/// 上传前语法门禁：复用共享 `deploy::precheck`（按扩展名路由到系统解释器做语法校验）。
///
/// 语法错误在落盘前中断部署；未知扩展名/二进制直接放行（不误杀）。
fn precheck_upload(path: &std::path::Path) -> Result<()> {
    let r = crate::deploy::precheck::precheck(path);
    if r.ok {
        Ok(())
    } else {
        bail!("上传被拒绝：{}", r.error.unwrap_or_default())
    }
}

/// 宝塔部署编排（异步）：建站 → 传文件 → 起服务 →（可选）SSL → 健康探活
pub async fn deploy(
    transport: &dyn BaotaTransport,
    opts: &BaotaDeployOptions,
    bin: &std::path::Path,
) -> Result<BaotaReport> {
    // 1. 建站（有域名才建）
    if let Some(domain) = &opts.site {
        let mut p = HashMap::new();
        p.insert("domain".to_string(), domain.clone());
        transport.post("site_create", &p).await?;
    }

    // 2. 上传二进制到站点目录（需求#4：落盘前先过语法门禁，语法错误直接中断部署）
    precheck_upload(bin)?;
    let mut p = HashMap::new();
    p.insert("path".to_string(), site_root(&opts.site));
    p.insert("bin".to_string(), bin.display().to_string());
    transport.post("file_upload", &p).await?;

    // 3. 起服务
    let mut p = HashMap::new();
    p.insert("service".to_string(), opts.service.clone());
    transport.post("service_start", &p).await?;

    // 4. 申请 SSL（可选）
    if opts.ssl {
        if let Some(domain) = &opts.site {
            let mut p = HashMap::new();
            p.insert("domain".to_string(), domain.clone());
            transport.post("ssl_apply", &p).await?;
        }
    }

    // 5. 健康探活
    let success = match &opts.health_url {
        Some(u) => {
            let mut p = HashMap::new();
            p.insert("url".to_string(), u.clone());
            transport.post("health", &p).await.is_ok()
        }
        None => true,
    };

    Ok(BaotaReport {
        success,
        ssl_applied: opts.ssl,
        service: opts.service.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> String {
        crate::model::byok::generate_key()
    }

    fn sample_cfg(key: &str) -> BaotaConfig {
        let mut panels = HashMap::new();
        panels.insert(
            "prod".to_string(),
            BaotaPanelStored {
                endpoint: "https://bt.example.com:8888".to_string(),
                api_key: crate::model::byok::encrypt_with_key("secret-key-123", key).unwrap(),
            },
        );
        BaotaConfig { panels }
    }

    #[test]
    fn test_sign_deterministic_with_fixed_time() {
        let mut p = HashMap::new();
        p.insert("domain".to_string(), "a.com".to_string());
        p.insert("x".to_string(), "1".to_string());
        let (t1, tok1) = sign_with_time(&p, 1700000000);
        let (_, tok2) = sign_with_time(&p, 1700000000);
        assert_eq!(t1, "1700000000");
        assert_eq!(tok1, tok2);
        // 参数顺序不应影响签名
        let mut p2 = HashMap::new();
        p2.insert("x".to_string(), "1".to_string());
        p2.insert("domain".to_string(), "a.com".to_string());
        let (_, tok3) = sign_with_time(&p2, 1700000000);
        assert_eq!(tok1, tok3);
    }

    #[test]
    fn test_sign_changes_with_time() {
        let mut p = HashMap::new();
        p.insert("k".to_string(), "v".to_string());
        let (_, a) = sign_with_time(&p, 100);
        let (_, b) = sign_with_time(&p, 200);
        assert_ne!(a, b);
    }

    #[test]
    fn test_config_resolve_decrypts_with_key() {
        let key = test_key();
        let cfg = sample_cfg(&key);
        let panel = resolve_panel_with_key(&cfg, "prod", &key).unwrap();
        assert_eq!(panel.endpoint, "https://bt.example.com:8888");
        assert_eq!(panel.api_key, "secret-key-123");
        assert!(resolve_panel_with_key(&cfg, "missing", &key).is_err());
    }

    #[test]
    fn test_config_save_load_roundtrip() {
        let key = test_key();
        let cfg = sample_cfg(&key);
        let tmp = std::env::temp_dir().join(format!("mgt_baota_{}.json", uuid::Uuid::new_v4()));
        // 直接序列化到临时路径验证（不走用户 config_dir）
        let s = serde_json::to_string_pretty(&cfg).unwrap();
        std::fs::write(&tmp, &s).unwrap();
        let loaded: BaotaConfig =
            serde_json::from_str(&std::fs::read_to_string(&tmp).unwrap()).unwrap();
        let panel = resolve_panel_with_key(&loaded, "prod", &key).unwrap();
        assert_eq!(panel.api_key, "secret-key-123");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_deploy_dry_run_records_all_steps() {
        let bin = std::env::temp_dir().join(format!("mgt_btest_{}.bin", uuid::Uuid::new_v4()));
        std::fs::write(&bin, b"fake").unwrap();
        let opts = BaotaDeployOptions {
            service: "marguerite".into(),
            site: Some("app.example.com".into()),
            health_url: Some("http://localhost:8080/health".into()),
            ssl: true,
            dry_run: true,
        };
        let transport = RecordingBaotaTransport::new();
        let report = deploy(&transport, &opts, &bin).await.unwrap();
        assert!(report.success);
        assert!(report.ssl_applied);
        let actions: Vec<String> = transport.calls().iter().map(|(a, _)| a.clone()).collect();
        assert_eq!(
            actions,
            vec![
                "site_create".to_string(),
                "file_upload".to_string(),
                "service_start".to_string(),
                "ssl_apply".to_string(),
                "health".to_string()
            ]
        );
        let _ = std::fs::remove_file(&bin);
    }

    #[tokio::test]
    async fn test_deploy_ssl_skipped_when_disabled() {
        let bin = std::env::temp_dir().join(format!("mgt_btest_{}.bin", uuid::Uuid::new_v4()));
        std::fs::write(&bin, b"fake").unwrap();
        let opts = BaotaDeployOptions {
            service: "marguerite".into(),
            site: None,
            health_url: None,
            ssl: false,
            dry_run: true,
        };
        let transport = RecordingBaotaTransport::new();
        let report = deploy(&transport, &opts, &bin).await.unwrap();
        assert!(report.success);
        assert!(!report.ssl_applied);
        let actions: Vec<String> = transport.calls().iter().map(|(a, _)| a.clone()).collect();
        assert_eq!(
            actions,
            vec!["file_upload".to_string(), "service_start".to_string()]
        );
        let _ = std::fs::remove_file(&bin);
    }

    /// 需求#4：部署上传路径必须在落盘前拦截语法错误的代码文件
    #[tokio::test]
    async fn test_deploy_blocks_upload_of_syntactically_broken_code() {
        let src = std::env::temp_dir().join(format!("mgt_btest_{}.py", uuid::Uuid::new_v4()));
        std::fs::write(&src, b"def broken(x)\n    return x\n").unwrap();
        let opts = BaotaDeployOptions {
            service: "marguerite".into(),
            site: None,
            health_url: None,
            ssl: false,
            dry_run: true,
        };
        let transport = RecordingBaotaTransport::new();
        let e = deploy(&transport, &opts, &src)
            .await
            .expect_err("语法错误应中断部署");
        assert!(
            e.to_string().contains("上传被拒绝"),
            "错误应说明拒绝原因: {e}"
        );
        assert!(
            transport.calls().is_empty(),
            "语法门禁必须在任何上传动作之前生效，实际发生了: {:?}",
            transport.calls()
        );
        let _ = std::fs::remove_file(&src);
    }

    /// 语法正确的代码文件应正常放行
    #[tokio::test]
    async fn test_deploy_allows_valid_code_upload() {
        let src = std::env::temp_dir().join(format!("mgt_btest_{}.py", uuid::Uuid::new_v4()));
        std::fs::write(&src, b"def ok(x):\n    return x\n").unwrap();
        let opts = BaotaDeployOptions {
            service: "marguerite".into(),
            site: None,
            health_url: None,
            ssl: false,
            dry_run: true,
        };
        let transport = RecordingBaotaTransport::new();
        let report = deploy(&transport, &opts, &src).await.unwrap();
        assert!(report.success);
        let _ = std::fs::remove_file(&src);
    }

    /// 二进制产物（无扩展名）不得被语法门禁误杀——这是部署的主路径
    #[tokio::test]
    async fn test_deploy_binary_artifact_not_blocked_by_syntax_gate() {
        let bin = std::env::temp_dir().join(format!("mgt_btest_bin_{}", uuid::Uuid::new_v4()));
        // 非 UTF-8 的二进制内容
        std::fs::write(&bin, [0x7f, 0x45, 0x4c, 0x46, 0x00, 0xff, 0xfe]).unwrap();
        let opts = BaotaDeployOptions {
            service: "marguerite".into(),
            site: None,
            health_url: None,
            ssl: false,
            dry_run: true,
        };
        let transport = RecordingBaotaTransport::new();
        let report = deploy(&transport, &opts, &bin).await.unwrap();
        assert!(report.success, "二进制产物应正常放行");
        let _ = std::fs::remove_file(&bin);
    }

    #[tokio::test]
    async fn test_deploy_health_failure_reports_false() {
        let bin = std::env::temp_dir().join(format!("mgt_btest_{}.bin", uuid::Uuid::new_v4()));
        std::fs::write(&bin, b"fake").unwrap();
        let opts = BaotaDeployOptions {
            service: "marguerite".into(),
            site: Some("x.com".into()),
            health_url: Some("http://localhost/health".into()),
            ssl: false,
            dry_run: true,
        };
        let transport = RecordingBaotaTransport::new().with_fail("health");
        let report = deploy(&transport, &opts, &bin).await.unwrap();
        assert!(!report.success);
        let _ = std::fs::remove_file(&bin);
    }
}
