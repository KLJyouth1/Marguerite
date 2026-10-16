//! 设备注册表 + 协议适配器（WP1）。
//!
//! 本文件持有内存 [`DeviceRegistry`]、统一错误 [`AdapterError`]、同步抽象
//! [`DeviceAdapter`] trait，以及 Home Assistant 适配器骨架 [`HomeAssistantAdapter`]。
//! 真实网络经可注入的 [`HttpClient`] trait 触达（默认实现 [`ReqwestClient`] 走 reqwest），
//! 单测注入桩客户端即可不打真网。

use std::sync::Arc;

use async_trait::async_trait;

use super::{Capability, HazardLevel, Transport};
use crate::audit_store::airgap::OutboundPolicy;

/// 单个设备条目（注册表值）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceEntry {
    pub cap: Capability,
    pub last_seen: Option<i64>,
}

/// 设备注册表（当前为内存占位；WP1 接 HA 后填充）。
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    entries: Vec<DeviceEntry>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, cap: Capability) {
        self.entries.push(DeviceEntry {
            cap,
            last_seen: None,
        });
    }

    pub fn list(&self) -> &[DeviceEntry] {
        &self.entries
    }

    pub fn find(&self, id: &str) -> Option<&DeviceEntry> {
        self.entries.iter().find(|e| e.cap.id == id)
    }
}

/// Adapter 错误类型。
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("device not found: {0}")]
    NotFound(String),
    #[error("device is read-only")]
    ReadOnly,
    #[error("transport forbidden: only LAN / loopback allowed")]
    TransportForbidden,
    #[error("http error: {0}")]
    Http(String),
    #[error("malformed payload: {0}")]
    Parse(String),
}

/// 协议适配器 trait：把 HA / MQTT / ADB / ... 包装成统一设备面。
///
/// 注意：真实适配器走异步网络（见 [`HomeAssistantAdapter`]）。该同步 trait 是
/// 进程内内存设备面的占位契约；WP1 接网关时再把异步调用包成 spawn/block_on 或
/// 演进为 async trait。
pub trait DeviceAdapter {
    fn name(&self) -> &'static str;
    fn list_entities(&self) -> Result<Vec<Capability>, AdapterError>;
    /// 调用设备服务；危险等级 ≥ L2 的调用须经审批层（调用方负责）。
    fn call_service(&self, entity_id: &str, action: &str) -> Result<(), AdapterError>;
}

// ---------------------------------------------------------------------------
// Home Assistant 适配器骨架
// ---------------------------------------------------------------------------

/// 把 HA entity_id（`light.living_room`）解析成 domain（`light`）。
///
/// entity_id 形如 `<domain>.<object_id>`；取第一个 `.` 之前的部分。
/// 若没有 `.`，整体视为 domain（保守：交由 [`HazardLevel::classify`] 的未知域分支处理）。
pub fn parse_domain(entity_id: &str) -> &str {
    entity_id.split('.').next().unwrap_or(entity_id)
}

/// 把 HA `/api/states` 返回的单个 state 对象映射成 [`Capability`]（纯函数，不打网）。
///
/// - `id` = `entity_id`
/// - `name` = `attributes.friendly_name`，缺失时回退 `entity_id`
/// - `domain` = [`parse_domain`]
/// - `read` 恒为 true（state 接口本来就是只读快照）
/// - `write` = 非只读域（`sensor` / `binary_sensor` 不可写）
/// - `hazard` = [`HazardLevel::classify`]
/// - `transport` = [`Transport::Local`]（HA 默认视为 LAN 本地实例；远程云实例后续标注）
pub fn capability_from_state(state: &serde_json::Value) -> Result<Capability, AdapterError> {
    let entity_id = state
        .get("entity_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AdapterError::Parse("state 缺少 entity_id 字段".to_string()))?;

    let domain = parse_domain(entity_id).to_string();
    let name = state
        .get("attributes")
        .and_then(|a| a.get("friendly_name"))
        .and_then(|v| v.as_str())
        .unwrap_or(entity_id)
        .to_string();

    let read_only_domain = matches!(domain.as_str(), "sensor" | "binary_sensor");

    Ok(Capability {
        id: entity_id.to_string(),
        name,
        hazard: HazardLevel::classify(&domain),
        read: true,
        write: !read_only_domain,
        transport: Transport::Local,
        domain,
    })
}

/// 可注入 HTTP 客户端：默认实现走 reqwest；单测注入桩避免打真网。
#[async_trait]
pub trait HttpClient: Send + Sync + 'static {
    /// GET 并把响应体解析为 JSON（带 HA 长期令牌 Bearer 认证）。
    async fn get_json(
        &self,
        url: &str,
        token: &str,
    ) -> Result<serde_json::Value, AdapterError>;
    /// POST JSON body 并把响应体解析为 JSON。
    async fn post_json(
        &self,
        url: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError>;
}

/// 默认 reqwest 实现（真实出向请求；仅连用户配置的 LAN HA 地址）。
#[derive(Default)]
pub struct ReqwestClient;

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn get_json(
        &self,
        url: &str,
        token: &str,
    ) -> Result<serde_json::Value, AdapterError> {
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| AdapterError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AdapterError::Http(format!("GET {url} -> {status}")));
        }
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| AdapterError::Parse(e.to_string()))
    }

    async fn post_json(
        &self,
        url: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AdapterError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AdapterError::Http(format!("POST {url} -> {status}")));
        }
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| AdapterError::Parse(e.to_string()))
    }
}

/// 校验 HA `base_url` 是否落在允许的出向白名单内（WP1 安全边界）。
///
/// 仅回环（`127.0.0.0/8` / `::1` / `localhost`）与 RFC1918 LAN（`10/8`、`172.16/12`、
/// `192.168/16`）放行；公网 / 云域名一律 [`AdapterError::TransportForbidden`]。
///
/// `airgap=true` 时连 LAN 也拒（仅回环）；WP1 默认非 airgap（HA 本就部署在 LAN），
/// 但策略对象把 airgap 开关暴露给调用方，未来可接全局 airgap 配置。
pub fn validate_base_url(base_url: &str, airgap: bool) -> Result<(), AdapterError> {
    let policy = OutboundPolicy::new(airgap);
    match policy.allow(base_url) {
        crate::audit_store::airgap::Allowance::AllowedLoopback
        | crate::audit_store::airgap::Allowance::AllowedPrivateLan => Ok(()),
        crate::audit_store::airgap::Allowance::DeniedLabeledCloud => {
            Err(AdapterError::TransportForbidden)
        }
    }
}

/// Home Assistant 适配器骨架：持有 base_url + 长期令牌，经可注入 [`HttpClient`] 出向。
///
/// - `list_entities`：`GET /api/states`，把每个实体映射成 [`Capability`]。
/// - `call_service`：`POST /api/services/<domain>/<service>`，domain 取自 entity_id。
///
/// 泛型 `H` 默认 [`ReqwestClient`]；测试时传桩客户端。
pub struct HomeAssistantAdapter<H = ReqwestClient> {
    base_url: String,
    token: String,
    client: Arc<H>,
}

impl HomeAssistantAdapter<ReqwestClient> {
    /// 用默认 reqwest 客户端构造。`base_url` 例如 `http://192.168.1.10:8123`。
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            client: Arc::new(ReqwestClient),
        }
    }

    /// 安全构造：先经 airgap 白名单校验 base_url，公网 / 云地址直接拒绝（fail-closed）。
    ///
    /// `airgap=true` 时仅回环可达；生产默认传 `false`（LAN HA 是预期部署形态）。
    pub fn try_new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        airgap: bool,
    ) -> Result<Self, AdapterError> {
        let base_url = base_url.into();
        validate_base_url(&base_url, airgap)?;
        Ok(Self {
            base_url,
            token: token.into(),
            client: Arc::new(ReqwestClient),
        })
    }
}

impl<H: HttpClient> HomeAssistantAdapter<H> {
    /// 注入自定义（测试用）HTTP 客户端。
    pub fn with_client(
        base_url: impl Into<String>,
        token: impl Into<String>,
        client: H,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            client: Arc::new(client),
        }
    }

    pub fn name(&self) -> &'static str {
        "home_assistant"
    }

    /// 拉取全部实体状态并映射为能力列表（不打真网 —— 经注入客户端）。
    pub async fn list_entities(&self) -> Result<Vec<Capability>, AdapterError> {
        let url = format!("{}/api/states", self.base_url);
        let value = self.client.get_json(&url, &self.token).await?;
        let arr = value
            .as_array()
            .ok_or_else(|| AdapterError::Parse("/api/states 响应不是数组".to_string()))?;
        arr.iter().map(capability_from_state).collect()
    }

    /// 调用 HA 服务：`POST /api/services/<domain>/<service>`。
    /// domain 从 `entity_id` 前缀解析；`service` 例如 `turn_on` / `lock`。
    /// 危险等级 ≥ L2 的调用须由审批层在外层把关（本方法不做权限决策）。
    pub async fn call_service(
        &self,
        entity_id: &str,
        service: &str,
    ) -> Result<serde_json::Value, AdapterError> {
        let domain = parse_domain(entity_id);
        let url = format!("{}/api/services/{domain}/{service}", self.base_url);
        let body = serde_json::json!({ "entity_id": entity_id });
        self.client.post_json(&url, &self.token, body).await
    }
}

/// 仅用于测试：记录请求并回桩 JSON 的 [`HttpClient`]。
#[cfg(test)]
struct StubClient {
    states: serde_json::Value,
    last_post_url: std::sync::Mutex<Option<String>>,
}

#[cfg(test)]
impl StubClient {
    fn new(states: serde_json::Value) -> Self {
        Self {
            states,
            last_post_url: std::sync::Mutex::new(None),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl HttpClient for StubClient {
    async fn get_json(
        &self,
        _url: &str,
        _token: &str,
    ) -> Result<serde_json::Value, AdapterError> {
        Ok(self.states.clone())
    }

    async fn post_json(
        &self,
        url: &str,
        _token: &str,
        _body: serde_json::Value,
    ) -> Result<serde_json::Value, AdapterError> {
        *self.last_post_url.lock().unwrap() = Some(url.to_string());
        Ok(serde_json::json!({"ok": true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(id: &str, domain: &str, hazard: HazardLevel, write: bool) -> Capability {
        Capability {
            id: id.to_string(),
            name: id.to_string(),
            domain: domain.to_string(),
            read: true,
            write,
            hazard,
            transport: Transport::Local,
        }
    }

    #[test]
    fn registry_register_find_list() {
        let mut reg = DeviceRegistry::new();
        assert_eq!(reg.list().len(), 0);

        reg.register(cap("light.a", "light", HazardLevel::SafeAuto, true));
        reg.register(cap("sensor.b", "sensor", HazardLevel::ReadOnly, false));
        reg.register(cap("lock.c", "lock", HazardLevel::PhysicalActuation, true));

        assert_eq!(reg.list().len(), 3);
        assert!(reg.find("missing").is_none());

        let got = reg.find("sensor.b").expect("sensor.b 应存在");
        assert_eq!(got.cap.domain, "sensor");
        assert!(!got.cap.write);
        assert_eq!(got.cap.hazard, HazardLevel::ReadOnly);

        let lock = reg.find("lock.c").expect("lock.c 应存在");
        // 物理致动 fail-closed：锁类实体必须判 L3。
        assert_eq!(lock.cap.hazard, HazardLevel::PhysicalActuation);
    }

    #[test]
    fn parse_domain_splits_entity_id() {
        assert_eq!(parse_domain("light.living_room"), "light");
        assert_eq!(parse_domain("lock.front_door"), "lock");
        assert_eq!(parse_domain("binary_sensor.motion_hall"), "binary_sensor");
        // 没有点号时整体当作 domain（交由 classify 未知域分支）。
        assert_eq!(parse_domain("weird"), "weird");
    }

    #[test]
    fn capability_from_state_maps_fields() {
        let s = serde_json::json!({
            "entity_id": "lock.front_door",
            "state": "locked",
            "attributes": {"friendly_name": "前门锁"}
        });
        let c = capability_from_state(&s).unwrap();
        assert_eq!(c.id, "lock.front_door");
        assert_eq!(c.name, "前门锁");
        assert_eq!(c.domain, "lock");
        assert_eq!(c.hazard, HazardLevel::PhysicalActuation);
        assert!(c.write, "lock 是可写域");

        // 传感器：只读
        let s2 = serde_json::json!({"entity_id": "sensor.temp", "attributes": {}});
        let c2 = capability_from_state(&s2).unwrap();
        assert!(!c2.write);
        assert!(c2.read);
        assert_eq!(c2.hazard, HazardLevel::ReadOnly);
        // 缺 friendly_name 时回退 entity_id
        assert_eq!(c2.name, "sensor.temp");

        // 缺 entity_id 报错
        assert!(capability_from_state(&serde_json::json!({"state": "on"})).is_err());
    }

    #[tokio::test]
    async fn ha_adapter_list_entities_via_injected_client() {
        let states = serde_json::json!([
            {"entity_id": "light.living_room", "attributes": {"friendly_name": "客厅灯"}},
            {"entity_id": "lock.front_door", "attributes": {"friendly_name": "前门锁"}},
            {"entity_id": "sensor.outside_temp", "attributes": {}},
            {"entity_id": "climate.bedroom", "attributes": {"friendly_name": "卧室空调"}},
        ]);
        let stub = StubClient::new(states);
        let adapter = HomeAssistantAdapter::with_client("http://ha.local:8123", "tok", stub);

        let caps = adapter.list_entities().await.expect("注入客户端不应失败");
        assert_eq!(caps.len(), 4);

        let by_id: std::collections::HashMap<&str, &Capability> =
            caps.iter().map(|c| (c.id.as_str(), c)).collect();

        assert_eq!(by_id["light.living_room"].hazard, HazardLevel::SafeAuto);
        assert!(by_id["light.living_room"].write);
        assert_eq!(by_id["lock.front_door"].hazard, HazardLevel::PhysicalActuation);
        assert_eq!(
            by_id["sensor.outside_temp"].hazard,
            HazardLevel::ReadOnly
        );
        assert!(!by_id["sensor.outside_temp"].write);
        assert_eq!(
            by_id["climate.bedroom"].hazard,
            HazardLevel::RequiresApproval
        );
        assert_eq!(by_id["lock.front_door"].name, "前门锁");
    }

    #[tokio::test]
    async fn ha_adapter_call_service_posts_to_domain_service_url() {
        let stub = StubClient::new(serde_json::json!([]));
        let adapter =
            HomeAssistantAdapter::with_client("http://ha.local:8123", "tok", stub);
        let _ = adapter.call_service("lock.front_door", "unlock").await.unwrap();
        let posted = adapter.client.last_post_url.lock().unwrap().clone();
        assert_eq!(posted.as_deref(), Some("http://ha.local:8123/api/services/lock/unlock"));
    }

    #[test]
    fn validate_base_url_allows_loopback_and_lan_denies_cloud() {
        assert!(validate_base_url("http://127.0.0.1:8123", false).is_ok());
        assert!(validate_base_url("http://localhost:8123", false).is_ok());
        assert!(validate_base_url("http://192.168.1.10:8123", false).is_ok());
        assert!(validate_base_url("http://10.0.0.5:8123", false).is_ok());
        assert!(validate_base_url("http://172.20.4.4:8123", false).is_ok());
        assert!(matches!(validate_base_url("https://ha.example.com:8123", false), Err(AdapterError::TransportForbidden)));
        assert!(matches!(validate_base_url("http://8.8.8.8:8123", false), Err(AdapterError::TransportForbidden)));
    }

    #[test]
    fn validate_base_url_airgap_blocks_lan() {
        assert!(validate_base_url("http://127.0.0.1:8123", true).is_ok());
        assert!(matches!(validate_base_url("http://192.168.1.10:8123", true), Err(AdapterError::TransportForbidden)));
    }

    #[test]
    fn try_new_rejects_cloud_base_url() {
        assert!(matches!(HomeAssistantAdapter::try_new("https://ha.example.com:8123", "tok", false), Err(AdapterError::TransportForbidden)));
        assert!(HomeAssistantAdapter::try_new("http://192.168.1.10:8123", "tok", false).is_ok());
    }

    // 保证 Serialize derive 真的可用（编译期 + 一个往返）。
    #[test]
    fn serde_roundtrip_device_entry() {
        let e = DeviceEntry {
            cap: cap("light.a", "light", HazardLevel::SafeAuto, true),
            last_seen: Some(12345),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["cap"]["hazard"], "SafeAuto");
        assert_eq!(v["cap"]["transport"], "Local");
        assert_eq!(v["last_seen"], 12345);
        let back: DeviceEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back.cap.id, "light.a");
        assert_eq!(back.last_seen, Some(12345));
    }
}
