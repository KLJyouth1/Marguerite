//! WP1 设备总线 RPC 接线（`devices.list` / `devices.status` / `devices.call`）。
//!
//! 安全契约（违反即缺陷）：
//! - **base_url airgap 白名单**：仅回环 / RFC1918 LAN；公网 / 云域名在适配器构造期
//!   [`crate::devices::adapter::validate_base_url`] 即拒绝（fail-closed）。
//! - **凭证加密落盘**：Home Assistant 长期令牌绝不写 `config.yaml`，一律经
//!   [`crate::credentials`] 台账（`mg1:` 密文）落盘；base_url（非机密）存
//!   `<data_dir>/devices.json`。
//! - **危险分级人在回路**：L0 只读拒写；L1 安全写自动执行；L2(RequiresApproval) /
//!   L3(PhysicalActuation) 必须经 [`crate::guard::approval_registry`] 抬升审批，
//!   未批准 / 超时一律**不执行**（fail-closed）。
//!
//! 诚实软失败：未配置 HA（base_url 缺失或令牌缺失）时，list/status 返回空列表与
//! `configured:false`，call 返回 `DEVICE_NOT_CONFIGURED`，**绝不崩、绝不伪造设备**。
//!
//! 留尾：WebSocket 事件订阅（HA `/api/websocket` 状态推送）本期未做；`list_entities`
//! 走 HTTP 轮询 `/api/states` 快照。事件推送为后续阶段。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::credentials::{CredentialKey, CredentialRecord, CredentialsStore};
use crate::devices::{HomeAssistantAdapter, HazardLevel};
use crate::devices::adapter::{parse_domain, validate_base_url};
use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::RpcError;
use crate::guard::approval_registry::global_registry;
use crate::guard::gateway::{ApprovalGate, ApprovalOutcome, RiskLevel};
use crate::persistence::config::Config;

/// 凭证台账里 HA 长期令牌的键空间（scope=拥有者 `devices`，id 固定 `home_assistant_token`）。
const CRED_SCOPE: &str = "devices";
const CRED_ID: &str = "home_assistant_token";

/// 最近一次 `devices.list` 成功拉取的实体数（`devices.status.entity_count` 的真实数据源）。
/// 未拉取过 / 拉取失败 / 未配置时为 0——如实上报，绝不冒充。
static LAST_ENTITY_COUNT: AtomicUsize = AtomicUsize::new(0);

/// 已解析的 HA 连接（base_url 过 airgap + 令牌已加密落盘才为 Some）。
struct ResolvedHa {
    base_url: String,
    token: String,
    airgap: bool,
}

impl ResolvedHa {
    /// 经 airgap 白名单构造适配器（公网 / 云地址在此最后一道防线拒绝）。
    fn build_adapter(&self) -> Result<HomeAssistantAdapter, String> {
        HomeAssistantAdapter::try_new(self.base_url.clone(), self.token.clone(), self.airgap)
            .map_err(|e| e.to_string())
    }
}

/// 从 `<data_dir>/devices.json` + credentials 台账解析 HA 连接。
///
/// 任一环节缺失 / 损坏 / airgap 拒绝 → `None`（诚实软失败）。调用方据此报
/// `configured:false`，不打真网、不崩。
fn resolve_ha() -> Option<ResolvedHa> {
    let path = Config::data_dir().join("devices.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let cfg: Value = serde_json::from_str(&text).ok()?;
    let ha_cfg = cfg.get("home_assistant")?;
    let base_url = ha_cfg.get("base_url")?.as_str()?.to_string();
    let airgap = ha_cfg
        .get("airgap")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // airgap 白名单：公网 / 云地址直接视为未配置（fail-closed，不打网）。
    validate_base_url(&base_url, airgap).ok()?;

    // 令牌从加密台账读取；明文旧格式在 CredentialsStore 反序列化期即被拒绝。
    let store = CredentialsStore::open_default();
    let key = CredentialKey::new(CRED_SCOPE, CRED_ID);
    match store.get_record(&key) {
        Some(CredentialRecord::ApiKey {
            key: Some(token), ..
        }) if !token.is_empty() => Some(ResolvedHa {
            base_url,
            token,
            airgap,
        }),
        _ => None,
    }
}

/// `devices.list`：返回注册表实体列表。
///
/// - 未配置 HA → `configured:false` + 空列表（诚实软失败）。
/// - 已配置 → `GET /api/states` 拉全量实体映射为能力 schema；拉取失败如实带 error，
///   不崩、不回陈旧空列表。
pub async fn devices_list(_ctx: &GatewayContext, _params: &Value) -> Result<Value, RpcError> {
    let Some(ha) = resolve_ha() else {
        LAST_ENTITY_COUNT.store(0, Ordering::Relaxed);
        return Ok(json!({
            "ok": true,
            "configured": false,
            "transport": "none",
            "entities": Vec::<Value>::new(),
        }));
    };

    let adapter = ha
        .build_adapter()
        .map_err(|e| RpcError::new("DEVICE_TRANSPORT", e))?;
    match adapter.list_entities().await {
        Ok(caps) => {
            LAST_ENTITY_COUNT.store(caps.len(), Ordering::Relaxed);
            Ok(json!({
                "ok": true,
                "configured": true,
                "transport": "local",
                "entity_count": caps.len(),
                "entities": caps,
            }))
        }
        Err(e) => {
            // 拉取失败：如实上报，把错误带给前端；实体数归零（不冒充上次的陈旧列表）。
            LAST_ENTITY_COUNT.store(0, Ordering::Relaxed);
            Ok(json!({
                "ok": true,
                "configured": true,
                "transport": "local",
                "entity_count": 0,
                "entities": Vec::<Value>::new(),
                "error": e.to_string(),
            }))
        }
    }
}

/// `devices.status`：`{ configured, transport, entity_count, airgap }`。
///
/// 只读快照，不打真网；`entity_count` 是最近一次 `devices.list` 的真实计数。
pub fn devices_status(_ctx: &GatewayContext) -> Value {
    let resolved = resolve_ha();
    let configured = resolved.is_some();
    let airgap = resolved.map(|h| h.airgap).unwrap_or(false);
    let entity_count = LAST_ENTITY_COUNT.load(Ordering::Relaxed);
    json!({
        "ok": true,
        "configured": configured,
        "transport": if configured { "local" } else { "none" },
        "entity_count": entity_count,
        "airgap": airgap,
    })
}

/// `devices.call`：按 entity_id + action 调用设备服务。
///
/// 分级执行（[`HazardLevel::classify`]）：
/// - L0 只读 → 拒写（`DEVICE_READ_ONLY`）；
/// - L1 安全写 → 直接执行；
/// - L2 / L3 → 抬升审批（[`ApprovalGate`] + 全局注册表），立即返回
///   `{status:"pending", approval_id}`；后台任务轮询裁决，**批准才执行，
///   拒绝 / 超时一律不执行**。
pub async fn devices_call(_ctx: &GatewayContext, params: &Value) -> Result<Value, RpcError> {
    let entity_id = params
        .get("entity_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let action = params.get("action").and_then(Value::as_str).unwrap_or("");
    if entity_id.is_empty() || action.is_empty() {
        return Err(RpcError::new(
            "INVALID_REQUEST",
            "entity_id 与 action 均不可为空",
        ));
    }

    let ha = resolve_ha().ok_or_else(|| {
        RpcError::new(
            "DEVICE_NOT_CONFIGURED",
            "设备总线未配置 Home Assistant（<data_dir>/devices.json 的 base_url 与加密令牌缺失）",
        )
    })?;

    let domain = parse_domain(entity_id).to_string();
    let hazard = HazardLevel::classify(&domain);

    // L0 只读域：HA 读状态走 /api/states，不发服务调用。
    if hazard == HazardLevel::ReadOnly {
        return Err(RpcError::new(
            "DEVICE_READ_ONLY",
            format!("实体 {entity_id} 属只读域 {domain}，不接受服务调用"),
        ));
    }

    match hazard {
        HazardLevel::ReadOnly => unreachable!(),
        HazardLevel::SafeAuto => {
            let adapter = ha
                .build_adapter()
                .map_err(|e| RpcError::new("DEVICE_TRANSPORT", e))?;
            adapter
                .call_service(entity_id, action)
                .await
                .map(|v| json!({"ok": true, "status": "executed", "hazard": "L1", "entity_id": entity_id, "action": action, "response": v}))
                .map_err(|e| RpcError::new("DEVICE_HTTP", e.to_string()))
        }
        HazardLevel::RequiresApproval | HazardLevel::PhysicalActuation => {
            let risk = if hazard == HazardLevel::PhysicalActuation {
                RiskLevel::High
            } else {
                RiskLevel::Medium
            };
            let hazard_name = if hazard == HazardLevel::PhysicalActuation {
                "L3"
            } else {
                "L2"
            };

            // 一次仅一个挂起审批：request 返回 None 说明已有审批在途，fail-closed 拒绝。
            let gate: Arc<ApprovalGate> = Arc::new(ApprovalGate::default());
            let args = serde_json::json!({
                "entity_id": entity_id,
                "action": action,
                "domain": domain,
                "hazard": hazard_name,
            })
            .to_string();
            let pending = gate
                .request("devices.call", args, risk)
                .ok_or_else(|| {
                    RpcError::new(
                        "APPROVAL_BUSY",
                        "已有挂起审批在等待裁决；一次仅一个危险操作审批",
                    )
                })?;
            let approval_id = pending.id.clone();
            global_registry().register(gate.clone(), &pending);

            // 后台等待裁决：批准才执行；拒绝 / 超时一律不执行（fail-closed）。
            let approval_id_task = approval_id.clone();
            let base_url = ha.base_url.clone();
            let token = ha.token.clone();
            let airgap = ha.airgap;
            let eid = entity_id.to_string();
            let act = action.to_string();
            tokio::spawn(async move {
                loop {
                    match gate.outcome(&approval_id_task) {
                        ApprovalOutcome::Approved => {
                            if let Ok(adapter) =
                                HomeAssistantAdapter::try_new(base_url, token, airgap)
                            {
                                let _ = adapter.call_service(&eid, &act).await;
                            }
                            break;
                        }
                        ApprovalOutcome::Denied | ApprovalOutcome::Timeout => break,
                        ApprovalOutcome::Pending => {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
            });

            Ok(json!({
                "ok": true,
                "status": "pending",
                "approval_id": approval_id,
                "hazard": hazard_name,
                "entity_id": entity_id,
                "action": action,
                "message": "危险操作已挂起审批；批准后才会执行（fail-closed）",
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_without_config_is_honestly_empty() {
        // 未配置 HA（默认测试环境无 devices.json / 无台账）：必须 configured:false，
        // entity_count=0，transport=none —— 绝不能崩或伪造设备。
        // 注意：测试环境 Config::data_dir 可能指向临时目录；只要不崩、字段齐全即可。
        let value = devices_status(&GatewayContext {
            config: Config::default(),
            token: String::new(),
            started_at: std::time::Instant::now(),
            bus: Arc::new(crate::events::EventBus::default()),
        });
        assert_eq!(value["configured"], false);
        assert_eq!(value["transport"], "none");
        assert_eq!(value["entity_count"], 0);
        assert!(value.get("airgap").is_some());
    }

    #[test]
    fn classify_drives_hazard_levels() {
        // 与 devices::registry 的分级保持一致（双保险）。
        assert_eq!(HazardLevel::classify("lock"), HazardLevel::PhysicalActuation);
        assert_eq!(HazardLevel::classify("climate"), HazardLevel::RequiresApproval);
        assert_eq!(HazardLevel::classify("light"), HazardLevel::SafeAuto);
        assert_eq!(HazardLevel::classify("sensor"), HazardLevel::ReadOnly);
    }
}
