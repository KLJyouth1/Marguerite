//! DSH 授权缝（authorization 包移植，上游 PR #2509）
//!
//! 授权流注册表：每个流声明它写哪份 [`CredentialKey`]、面向用户的标签与
//! 提供的方法（最优先在前）；一次 `begin` 尝试经过「提示/通知 → 提交记录」
//! 生命周期，围观者经 settlement 事件得知结束方式（失败只出现在事件流上，
//! 调用方拿到的是错误而非结局——对齐上游 AuthorizationSettlement 语义）。
//!
//! 类型均为 wire-safe 纯数据（不携带 cordis/service 引用），JSON 可无损过线。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::credentials::{CredentialKey, CredentialRecord, CredentialsStore};

/// 一种获取凭证的方式（由提供它的流命名）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationMethod {
    /// 流自有的标识（调用方回显时使用）
    pub id: String,
    /// 选择器用的用户标签
    pub label: String,
}

/// 运行中的流给围观者的报告。**绝不携带密钥**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationNotice {
    /// 正在发生什么 / 人必须做的下一步
    pub message: String,
    /// 人必须打开的页面
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 人必须在该页输入的短码
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// `select` 提示的一个选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationPromptOption {
    /// 选中时返回的值
    pub id: String,
    /// 用户标签
    pub label: String,
    /// 可选补充说明
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 流必须得到回答的问题。`secret` 与 `text` 仅在呈现上不同
/// （掩码、不进日志）；`select` 以选中项的 id 作答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AuthorizationPrompt {
    /// 普通文本提问
    Text {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// 密钥提问（呈现层掩码）
    Secret {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// 选择提问
    Select {
        message: String,
        options: Vec<AuthorizationPromptOption>,
    },
}

/// 一次尝试对调用方而言的结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthorizationStatus {
    /// 记录已提交且被观察
    Authorized,
    /// 人或调用方撤回
    Cancelled,
}

/// 一次尝试对围观者而言的结局（事件流视角）。失败只存在于此：
/// 调用方收到的是抛出的错误，围观者只能靠 `failed` 区分拒绝与损坏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthorizationSettlement {
    Authorized,
    Cancelled,
    Failed,
}

/// 一次 `begin` 尝试的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationOutcome {
    pub status: AuthorizationStatus,
}

/// 注册的流在界面上的样子：它授权什么、是否忙碌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationEntry {
    /// 该流写入的凭证记录地址
    pub key: CredentialKey,
    /// 授权对象名
    pub label: String,
    /// 流提供的方法（最优先在前）
    pub methods: Vec<AuthorizationMethod>,
    /// 该地址是否正有一次尝试在跑
    pub in_flight: bool,
}

/// 流程注册的内部形状。
#[derive(Debug, Clone)]
struct FlowRegistration {
    key: CredentialKey,
    label: String,
    methods: Vec<AuthorizationMethod>,
    in_flight: bool,
}

/// 授权流注册表：登记流元数据 + 执行一次尝试（提交记录 / 结算）。
///
/// 本移植为「缝」的地基：静态 api-key 与不透明 grant 均以
/// [`AuthorizationRegistry::begin`] 提交；交互式（浏览器/OAuth）流的
/// 提示-通知往返由上层经 `notice`/`settle` 回调完成。
pub struct AuthorizationRegistry {
    store: Arc<CredentialsStore>,
    flows: Mutex<BTreeMap<String, FlowRegistration>>, // key: CredentialKey 的字符串形式
    /// 每地址最近一次结算（围观者事件流视角；failed 只存在于此）
    settlements: Mutex<BTreeMap<String, AuthorizationSettlement>>,
}

impl AuthorizationRegistry {
    /// 以既有的凭证台账构造。
    pub fn new(store: Arc<CredentialsStore>) -> Self {
        Self {
            store,
            flows: Mutex::new(BTreeMap::new()),
            settlements: Mutex::new(BTreeMap::new()),
        }
    }

    /// 登记一个流（幂等：同 key 重复登记以最新元数据覆盖，保留 in_flight）。
    pub fn register(
        &self,
        key: &CredentialKey,
        label: &str,
        methods: Vec<AuthorizationMethod>,
    ) {
        let mut guard = self.flows.lock().unwrap_or_else(|e| e.into_inner());
        let in_flight = guard
            .get(&key.to_string())
            .is_some_and(|f| f.in_flight);
        guard.insert(
            key.to_string(),
            FlowRegistration {
                key: key.clone(),
                label: label.to_string(),
                methods,
                in_flight,
            },
        );
    }

    /// 注销一个流；返回是否确实存在。
    pub fn unregister(&self, key: &CredentialKey) -> bool {
        self.flows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key.to_string())
            .is_some()
    }

    /// 全部注册的流（键序稳定）。
    pub fn entries(&self) -> Vec<AuthorizationEntry> {
        let guard = self.flows.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .values()
            .map(|f| AuthorizationEntry {
                key: f.key.clone(),
                label: f.label.clone(),
                methods: f.methods.clone(),
                in_flight: f.in_flight,
            })
            .collect()
    }

    /// 指定地址是否正有一次尝试在跑。
    pub fn is_in_flight(&self, key: &CredentialKey) -> bool {
        self.flows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key.to_string())
            .is_some_and(|f| f.in_flight)
    }

    /// 执行一次尝试：置 in_flight → 提交记录 → 清 in_flight，返回 `authorized`。
    ///
    /// 已登记同 key 的另一尝试在跑时返回错误（一次一个尝试）。
    pub fn begin(
        &self,
        key: &CredentialKey,
        record: CredentialRecord,
    ) -> Result<AuthorizationOutcome, String> {
        {
            let mut guard = self.flows.lock().unwrap_or_else(|e| e.into_inner());
            let flow = guard
                .get_mut(&key.to_string())
                .ok_or_else(|| format!("未登记的授权流: {key}"))?;
            if flow.in_flight {
                return Err(format!("授权流 {key} 已有一个尝试在跑"));
            }
            flow.in_flight = true;
        }
        let result = self.store.set_record(key, record);
        self.clear_in_flight(key);
        self.record_settlement(key, AuthorizationSettlement::Authorized);
        result.map(|()| AuthorizationOutcome {
            status: AuthorizationStatus::Authorized,
        })
    }

    /// 围观者视角结算（交互式流经此回写结局并清 in_flight）。
    /// 返回该地址是否已登记（结算只作用于已登记流）。
    pub fn settle(&self, key: &CredentialKey, settlement: AuthorizationSettlement) -> bool {
        self.clear_in_flight(key);
        let registered = self
            .flows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&key.to_string());
        if registered {
            self.record_settlement(key, settlement);
        }
        registered
    }

    /// 最近一次结算（无记录返回 None；`failed` 只存在于此——调用方拿到的是错误）。
    pub fn last_settlement(&self, key: &CredentialKey) -> Option<AuthorizationSettlement> {
        self.settlements
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key.to_string())
            .copied()
    }

    fn record_settlement(&self, key: &CredentialKey, settlement: AuthorizationSettlement) {
        self.settlements
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), settlement);
    }

    fn clear_in_flight(&self, key: &CredentialKey) {
        if let Some(flow) = self
            .flows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&key.to_string())
        {
            flow.in_flight = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_store(name: &str) -> Arc<CredentialsStore> {
        let dir = std::env::temp_dir().join(format!(
            "mgt_auth_{name}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(CredentialsStore::load(
            &PathBuf::from(&dir).join("credentials.json"),
        ))
    }

    fn methods() -> Vec<AuthorizationMethod> {
        vec![
            AuthorizationMethod {
                id: "api-key".into(),
                label: "API Key".into(),
            },
            AuthorizationMethod {
                id: "device-code".into(),
                label: "设备码".into(),
            },
        ]
    }

    #[test]
    fn register_and_entries() {
        let reg = AuthorizationRegistry::new(tmp_store("entries"));
        let key = CredentialKey::new("llm", "deepseek");
        reg.register(&key, "DeepSeek", methods());
        let entries = reg.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, key);
        assert_eq!(entries[0].label, "DeepSeek");
        assert_eq!(entries[0].methods.len(), 2);
        assert!(!entries[0].in_flight);
        assert!(reg.unregister(&key));
        assert!(reg.entries().is_empty());
    }

    #[test]
    fn begin_commits_record_and_clears_in_flight() {
        let store = tmp_store("begin");
        let reg = AuthorizationRegistry::new(store.clone());
        let key = CredentialKey::new("llm", "deepseek");
        reg.register(&key, "DeepSeek", methods());
        let outcome = reg
            .begin(&key, CredentialRecord::api_key("sk-ok"))
            .unwrap();
        assert_eq!(outcome.status, AuthorizationStatus::Authorized);
        assert!(!reg.is_in_flight(&key), "提交后应清 in_flight");
        assert_eq!(
            reg.last_settlement(&key),
            Some(AuthorizationSettlement::Authorized)
        );
        match store.get_record(&key) {
            Some(CredentialRecord::ApiKey { key: Some(k), .. }) => assert_eq!(k, "sk-ok"),
            other => panic!("凭证未提交: {other:?}"),
        }
    }

    #[test]
    fn begin_unregistered_flow_fails_without_side_effect() {
        let reg = AuthorizationRegistry::new(tmp_store("unregistered"));
        let key = CredentialKey::new("llm", "unknown");
        assert!(reg.begin(&key, CredentialRecord::api_key("sk")).is_err());
        assert!(!reg.is_in_flight(&key));
    }

    #[test]
    fn settle_reports_registration_and_records_settlement() {
        let reg = AuthorizationRegistry::new(tmp_store("settle"));
        let key = CredentialKey::new("auth", "oauth");
        reg.register(&key, "OAuth", methods());
        // 结算只作用于已登记流：已登记 → true 且记录；未登记 → false 且不记录
        assert!(reg.settle(&key, AuthorizationSettlement::Cancelled));
        assert_eq!(
            reg.last_settlement(&key),
            Some(AuthorizationSettlement::Cancelled)
        );
        let ghost = CredentialKey::new("auth", "ghost");
        assert!(!reg.settle(&ghost, AuthorizationSettlement::Failed));
        assert_eq!(reg.last_settlement(&ghost), None);
    }

    #[test]
    fn notice_and_prompt_serialize_wire_safe() {
        // wire-safe：纯数据 JSON 过线（不携带任何运行时引用）
        let notice = AuthorizationNotice {
            message: "打开页面完成授权".into(),
            url: Some("https://example.com/device".into()),
            code: Some("AB12-CD".into()),
        };
        let json = serde_json::to_value(&notice).unwrap();
        assert_eq!(json["code"], "AB12-CD");
        let prompt = AuthorizationPrompt::Select {
            message: "选择认证方式".into(),
            options: vec![AuthorizationPromptOption {
                id: "api-key".into(),
                label: "API Key".into(),
                description: None,
            }],
        };
        let json = serde_json::to_value(&prompt).unwrap();
        assert_eq!(json["kind"], "select");
    }
}
