//! MQTT 适配器骨架 + 局域网发现（WP1 留尾）。
//!
//! 本模块**不真正连接 broker**：所有出向经可注入的 [`MqttBus`] trait 触达，
//! 默认实现 [`UnconnectedBus`] 永不连网（publish 直接报错、observed_topics 为空）。
//! 单测注入 [`StubBus`] 即可零真实出向地验证：
//! - topic → entity 解析（纯函数 [`entity_from_topic`]）；
//! - call_service 编码出的 MQTT publish 报文形状（[`MqttMessage`]）；
//! - 局域网发现白名单（[`is_discovery_target`]：仅回环 / RFC1918 放行）。

use std::sync::Arc;

use super::{AdapterError, Capability, HazardLevel, Transport};
use super::adapter::parse_domain;

// ---------------------------------------------------------------------------
// 可注入 MQTT 总线
// ---------------------------------------------------------------------------

/// 一条待发布的 MQTT 报文（topic + payload，纯数据；不涉及连接）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Vec<u8>,
}

impl MqttMessage {
    /// payload 按 UTF-8 字符串读取（MQTT 报文多为 JSON 文本）。
    pub fn payload_str(&self) -> &str {
        // 骨架阶段 payload 一律由 serde_json::to_vec 产出，必为合法 UTF-8；
        // 非法时回退为_lossy_视图，但测试只走合法路径。
        std::str::from_utf8(&self.payload).unwrap_or("")
    }
}

/// 可注入的 MQTT 总线：默认实现不连 broker；单测注入桩避免真实出向。
///
/// 设计上刻意把「连 broker / 收报文」这件副作用封在 trait 后面，
/// 让 [`MqttAdapter`] 的核心逻辑（topic 推导、报文编码）保持纯函数可测。
pub trait MqttBus: Send + Sync + 'static {
    /// 发布一条报文。真实实现走 broker；桩只记录（不打网）。
    fn publish(&self, msg: &MqttMessage) -> Result<(), AdapterError>;
    /// 返回当前已观察/已订阅的 topic 列表（用于 [`MqttAdapter::list_entities`] 推导）。
    fn observed_topics(&self) -> Result<Vec<String>, AdapterError>;
}

/// 默认「未连接」总线：**绝不连 broker**。
///
/// - `publish` 直接报错（诚实告知：骨架阶段没有真实 broker 会话）；
/// - `observed_topics` 返回空（没有任何已观察 topic）。
///
/// 这样即使调用方忘记注入测试桩，也不会意外向局域网发报文。
#[derive(Default)]
pub struct UnconnectedBus;

impl MqttBus for UnconnectedBus {
    fn publish(&self, _msg: &MqttMessage) -> Result<(), AdapterError> {
        Err(AdapterError::Http(
            "mqtt broker not connected (WP1 skeleton; inject a bus to publish)".to_string(),
        ))
    }

    fn observed_topics(&self) -> Result<Vec<String>, AdapterError> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// 局域网发现白名单
// ---------------------------------------------------------------------------

/// 判断 `host` 是否是合法的 MQTT broker / 局域网发现目标。
///
/// 仅放行：
/// - 回环（`127.0.0.0/8` / `::1` / `localhost`）；
/// - RFC1918 私网（`10/8`、`172.16/12`、`192.168/16`）。
///
/// 公网 / 云域名 / 不可解析主机一律拒绝（fail-closed）。
/// 复用 [`crate::audit_store::airgap`] 的既有判定，避免重复造网段表。
pub fn is_discovery_target(host: &str) -> bool {
    crate::audit_store::airgap::is_loopback(host)
        || crate::audit_store::airgap::is_private_lan(host)
}

// ---------------------------------------------------------------------------
// topic → entity 纯函数
// ---------------------------------------------------------------------------

/// 把一个 state topic 解析成 [`Capability`]（纯函数，不打网）。
///
/// 约定 topic 形如 `<prefix>/<domain>/<object_id>/state`，例如：
/// `marguerite/devices/light/living_room/state` → entity `light.living_room`。
///
/// 不满足形状（前缀不符、缺 `/state` 后缀、中间段为空）返回 [`None`]。
pub fn entity_from_topic(topic: &str, prefix: &str) -> Option<Capability> {
    // 规范化：前缀末尾不带斜杠。
    let prefix = prefix.trim_end_matches('/');
    // 必须以 `<prefix>/` 开头。
    let rest = topic.strip_prefix(prefix)?.strip_prefix('/')?;
    // 必须以 `/state` 结尾。
    let mid = rest.strip_suffix("/state")?;
    // mid 形如 `light/living_room`。
    let mut parts = mid.split('/');
    let domain = parts.next()?;
    let object_id = parts.collect::<Vec<_>>().join("_");
    if domain.is_empty() || object_id.is_empty() {
        return None;
    }

    let entity_id = format!("{domain}.{object_id}");
    let read_only_domain = matches!(domain, "sensor" | "binary_sensor");

    Some(Capability {
        id: entity_id.clone(),
        name: entity_id,
        domain: domain.to_string(),
        read: true,
        write: !read_only_domain,
        hazard: HazardLevel::classify(domain),
        transport: Transport::Local,
    })
}

// ---------------------------------------------------------------------------
// MQTT 适配器骨架
// ---------------------------------------------------------------------------

/// MQTT 适配器骨架：持有 broker 地址 + topic 前缀，经可注入 [`MqttBus`] 触达。
///
/// - `list_entities`：从 [`MqttBus::observed_topics`] 返回的 topic 列表，
///   用 [`entity_from_topic`] 纯函数推导出能力列表。
/// - `call_service`：把动作编成一条 publish 报文（[`MqttMessage`]），再交给 bus；
///   默认 [`UnconnectedBus`] 不会真发。
///
/// 泛型 `B` 默认 [`UnconnectedBus`]；测试时传 [`StubBus`]。
pub struct MqttAdapter<B = UnconnectedBus> {
    host: String,
    port: u16,
    topic_prefix: String,
    bus: Arc<B>,
}

impl MqttAdapter<UnconnectedBus> {
    /// 构造一个**未连接**的 MQTT 适配器（不碰 broker）。
    ///
    /// `host` 会先过 [`is_discovery_target`] 白名单；公网地址直接拒绝。
    pub fn new(host: impl Into<String>, port: u16, topic_prefix: impl Into<String>) -> Result<Self, AdapterError> {
        let host = host.into();
        if !is_discovery_target(&host) {
            return Err(AdapterError::TransportForbidden);
        }
        Ok(Self {
            host,
            port,
            topic_prefix: topic_prefix.into(),
            bus: Arc::new(UnconnectedBus),
        })
    }
}

impl<B: MqttBus> MqttAdapter<B> {
    /// 注入自定义（测试用）总线。
    pub fn with_bus(
        host: impl Into<String>,
        port: u16,
        topic_prefix: impl Into<String>,
        bus: B,
    ) -> Result<Self, AdapterError> {
        let host = host.into();
        if !is_discovery_target(&host) {
            return Err(AdapterError::TransportForbidden);
        }
        Ok(Self {
            host,
            port,
            topic_prefix: topic_prefix.into(),
            bus: Arc::new(bus),
        })
    }

    pub fn name(&self) -> &'static str {
        "mqtt"
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// 从已观察 topic 推导实体列表（纯推导；副作用只在 `bus.observed_topics()`）。
    pub fn list_entities(&self) -> Result<Vec<Capability>, AdapterError> {
        let topics = self.bus.observed_topics()?;
        Ok(topics
            .iter()
            .filter_map(|t| entity_from_topic(t, &self.topic_prefix))
            .collect())
    }

    /// 把一次服务调用编码成 publish 报文（纯函数部分）。
    ///
    /// topic：`<prefix>/<domain>/<object_id>/set`；payload：JSON
    /// `{"action":"<action>","entity_id":"<entity_id>"}`。
    /// domain 取自 entity_id 前缀（复用 [`parse_domain`]）。
    pub fn build_publish(&self, entity_id: &str, action: &str) -> MqttMessage {
        let domain = parse_domain(entity_id);
        // entity_id 形如 `light.living_room` → object_id = `living_room`。
        let object_id = entity_id.split_once('.').map(|(_, o)| o).unwrap_or(entity_id);
        let prefix = self.topic_prefix.trim_end_matches('/');
        let topic = format!("{prefix}/{domain}/{object_id}/set");
        let payload = serde_json::json!({
            "action": action,
            "entity_id": entity_id,
        })
        .to_string()
        .into_bytes();
        MqttMessage { topic, payload }
    }

    /// 编码并（经注入 bus）发送一次服务调用。
    ///
    /// 默认 [`UnconnectedBus`] 会报错（未连 broker）；危险等级 ≥ L2 的审批由外层把关。
    pub fn call_service(&self, entity_id: &str, action: &str) -> Result<(), AdapterError> {
        let msg = self.build_publish(entity_id, action);
        self.bus.publish(&msg)
    }
}

// ---------------------------------------------------------------------------
// 测试桩
// ---------------------------------------------------------------------------

/// 仅用于测试：记录已发报文、回桩已观察 topic 的 [`MqttBus`]（零真实出向）。
#[cfg(test)]
struct StubBus {
    observed: Vec<String>,
    published: std::sync::Mutex<Vec<MqttMessage>>,
}

#[cfg(test)]
impl StubBus {
    fn new(observed: Vec<&str>) -> Self {
        Self {
            observed: observed.into_iter().map(|s| s.to_string()).collect(),
            published: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(test)]
impl MqttBus for StubBus {
    fn publish(&self, msg: &MqttMessage) -> Result<(), AdapterError> {
        self.published.lock().unwrap().push(msg.clone());
        Ok(())
    }

    fn observed_topics(&self) -> Result<Vec<String>, AdapterError> {
        Ok(self.observed.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- topic → entity 解析 ---

    #[test]
    fn entity_from_topic_parses_standard_shape() {
        let c = entity_from_topic("marguerite/devices/light/living_room/state", "marguerite/devices")
            .expect("标准 topic 应解析成功");
        assert_eq!(c.id, "light.living_room");
        assert_eq!(c.domain, "light");
        assert_eq!(c.hazard, HazardLevel::SafeAuto);
        assert!(c.write);
        assert!(c.read);
        assert_eq!(c.transport, Transport::Local);
    }

    #[test]
    fn entity_from_topic_sensors_are_readonly() {
        let c = entity_from_topic(
            "home/sensor/hall_temp/state",
            "home",
        )
        .unwrap();
        assert_eq!(c.id, "sensor.hall_temp");
        assert!(!c.write, "sensor 不可写");
        assert_eq!(c.hazard, HazardLevel::ReadOnly);
    }

    #[test]
    fn entity_from_topic_lock_is_physical_actuation() {
        let c = entity_from_topic(
            "home/lock/front_door/state",
            "home",
        )
        .unwrap();
        assert_eq!(c.id, "lock.front_door");
        assert_eq!(c.hazard, HazardLevel::PhysicalActuation);
    }

    #[test]
    fn entity_from_topic_rejects_wrong_prefix_or_suffix() {
        // 前缀不符
        assert!(entity_from_topic("other/light/living_room/state", "home").is_none());
        // 缺 /state 后缀
        assert!(entity_from_topic("home/light/living_room", "home").is_none());
        // 中间为空
        assert!(entity_from_topic("home//state", "home").is_none());
        // 前缀带尾斜杠也应兼容
        assert!(entity_from_topic("home/light/l/state", "home/").is_some());
    }

    // --- publish 报文形状 ---

    #[test]
    fn build_publish_topic_and_payload_shape() {
        let adapter = MqttAdapter::with_bus(
            "192.168.1.10",
            1883,
            "home",
            StubBus::new(vec![]),
        )
        .unwrap();
        let msg = adapter.build_publish("light.living_room", "turn_on");
        assert_eq!(msg.topic, "home/light/living_room/set");
        let body: serde_json::Value = serde_json::from_str(msg.payload_str()).unwrap();
        assert_eq!(body["action"], "turn_on");
        assert_eq!(body["entity_id"], "light.living_room");
    }

    #[test]
    fn build_publish_strips_prefix_trailing_slash() {
        let adapter =
            MqttAdapter::with_bus("127.0.0.1", 1883, "home/", StubBus::new(vec![])).unwrap();
        let msg = adapter.build_publish("lock.front_door", "unlock");
        assert_eq!(msg.topic, "home/lock/front_door/set");
    }

    #[test]
    fn call_service_records_publish_via_stub_bus() {
        let stub = StubBus::new(vec![]);
        let adapter = MqttAdapter::with_bus("192.168.1.10", 1883, "home", stub).unwrap();
        adapter.call_service("lock.front_door", "unlock").unwrap();
        let published = &adapter.bus.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].topic, "home/lock/front_door/set");
    }

    #[test]
    fn default_unconnected_bus_does_not_publish() {
        let adapter = MqttAdapter::new("127.0.0.1", 1883, "home").unwrap();
        // 未连接总线：publish 报错，绝不真发。
        assert!(adapter.call_service("light.a", "on").is_err());
        assert!(adapter.list_entities().unwrap().is_empty());
    }

    // --- list_entities 从已观察 topic 推导 ---

    #[test]
    fn list_entities_derives_from_observed_topics() {
        let stub = StubBus::new(vec![
            "home/light/living_room/state",
            "home/sensor/temp/state",
            "home/lock/front_door/state",
            "home/other/thing/config", // 非 state topic，应被过滤
        ]);
        let adapter = MqttAdapter::with_bus("127.0.0.1", 1883, "home", stub).unwrap();
        let caps = adapter.list_entities().unwrap();
        assert_eq!(caps.len(), 3, "非 state topic 应被过滤");
        let ids: Vec<&str> = caps.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"light.living_room"));
        assert!(ids.contains(&"sensor.temp"));
        assert!(ids.contains(&"lock.front_door"));
    }

    // --- 局域网发现白名单 ---

    #[test]
    fn is_discovery_target_allows_loopback_and_lan() {
        for h in [
            "127.0.0.1",
            "127.0.0.1:1883",
            "::1",
            "localhost",
            "192.168.1.10",
            "10.0.0.5",
            "172.20.4.4",
        ] {
            assert!(is_discovery_target(h), "应放行: {h}");
        }
    }

    #[test]
    fn is_discovery_target_rejects_public() {
        for h in ["8.8.8.8", "1.1.1.1", "broker.example.com", "8.8.8.8:1883"] {
            assert!(!is_discovery_target(h), "公网应拒绝: {h}");
        }
    }

    #[test]
    fn new_rejects_public_broker_host() {
        assert!(matches!(
            MqttAdapter::new("broker.example.com", 1883, "home"),
            Err(AdapterError::TransportForbidden)
        ));
        assert!(MqttAdapter::new("192.168.1.10", 1883, "home").is_ok());
    }
}
