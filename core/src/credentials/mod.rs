//! DSH 凭证缝（credentials 包移植，上游 PR #2509）
//!
//! 两个名义键空间（互不碰撞）：
//! - [`CredentialKey`]：`<scope>/<id>`——scope 是**拥有记录的插件注册名**（记录的
//!   payload 以拥有者格式书写，两个插件同服务名也不会互读彼此的记录）；
//!   id 是拥有者自己的寻址单元（LLM 适配器通常用 provider route key）。
//! - [`CredentialRef`]：POSIX 风格环境变量名，是「引用」而非地址。
//!
//! 记录两种（tagged）：
//! - `api-key`：密钥值 + 可选 provider 环境值（如 `AWS_PROFILE`）；
//! - `grant`：一次授权流的产物，payload 对缝完全**不透明**（JSON roundtrip 是
//!   唯一约束），只有拥有它的插件能解释。
//!
//! 持久化：`<data_dir>/credentials.json`（原子写：临时文件 + rename）。

pub mod authorization;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// 名义凭证地址：`<scope>/<id>`（scope = 拥有记录的插件注册名）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CredentialKey {
    /// 拥有记录的插件注册名（而非领域名——payload 按拥有者格式书写）
    pub scope: String,
    /// 拥有者自己的寻址单元（如 provider route key）
    pub id: String,
}

impl CredentialKey {
    /// 构造凭证地址。
    pub fn new(scope: &str, id: &str) -> Self {
        Self {
            scope: scope.to_string(),
            id: id.to_string(),
        }
    }

    /// 解析 `"<scope>/<id>"`；scope 或 id 为空（`"/x"`、`"x/"`、`""`）返回 None。
    pub fn parse(s: &str) -> Option<Self> {
        let (scope, id) = s.split_once('/')?;
        if scope.is_empty() || id.is_empty() {
            return None;
        }
        Some(Self::new(scope, id))
    }
}

impl std::fmt::Display for CredentialKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.scope, self.id)
    }
}

/// 名义凭证引用：POSIX 风格环境变量名（如 `DEEPSEEK_API_KEY`）。
///
/// 与 [`CredentialKey`] 文法不相交（ref 无 `/` 语义），两个键空间永不碰撞。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CredentialRef(pub String);

impl std::fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 一份持久化凭证记录（tagged by kind）。
///
/// `api-key`：harness 自己理解的凭证（密钥 / provider 环境值 / 二者皆有；
/// 两者皆缺表示拥有者确认该路由从**自身环境发现**认证——这不同于「无记录」）。
/// `grant`：授权流产物，对缝不透明（JSON roundtrip 为唯一约束）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CredentialRecord {
    /// API 密钥记录
    #[serde(rename = "api-key")]
    ApiKey {
        /// 非空密钥值（可缺省）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        /// Provider 环境值（名字为 POSIX 标识符）
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    /// 授权 grant 记录（payload 不透明）
    Grant {
        /// 拥有者定义的 JSON 值
        payload: serde_json::Value,
    },
}

impl CredentialRecord {
    /// 便捷构造：api-key 记录（仅密钥）。
    pub fn api_key(key: impl Into<String>) -> Self {
        CredentialRecord::ApiKey {
            key: Some(key.into()),
            env: BTreeMap::new(),
        }
    }

    /// 便捷构造：grant 记录（不透明 payload）。
    pub fn grant(payload: serde_json::Value) -> Self {
        CredentialRecord::Grant { payload }
    }

    /// 记录种类名（"api-key" / "grant"）。
    pub fn kind(&self) -> &'static str {
        match self {
            CredentialRecord::ApiKey { .. } => "api-key",
            CredentialRecord::Grant { .. } => "grant",
        }
    }
}

/// 落盘形状（records: "scope/id" → 记录；references: ref → "scope/id"）。
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    records: BTreeMap<String, CredentialRecord>,
    #[serde(default)]
    references: BTreeMap<String, String>,
}

/// 凭证台账：内存态 + JSON 原子落盘。
///
/// 突变（set/delete/unset）先改内存再落盘；落盘失败返回 Err 但内存态保留
/// （下次成功落盘覆盖），保证同一进程内的读写一致性优先。
pub struct CredentialsStore {
    path: PathBuf,
    state: Mutex<StoreFile>,
}

impl CredentialsStore {
    /// 从指定路径加载；文件缺失或损坏 → 空台账（损坏时不静默吞错，打印告警）。
    pub fn load(path: &Path) -> Self {
        let state = match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<StoreFile>(&text) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[credentials] 台账损坏（{path:?}），以空台账启动: {e}");
                    StoreFile::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoreFile::default(),
            Err(e) => {
                eprintln!("[credentials] 台账读取失败（{path:?}），以空台账启动: {e}");
                StoreFile::default()
            }
        };
        Self {
            path: path.to_path_buf(),
            state: Mutex::new(state),
        }
    }

    /// 打开默认台账：`<data_dir>/credentials.json`。
    pub fn open_default() -> Self {
        Self::load(&crate::persistence::config::Config::data_dir().join("credentials.json"))
    }

    /// 写入/覆盖一份记录，并落盘。
    pub fn set_record(
        &self,
        key: &CredentialKey,
        record: CredentialRecord,
    ) -> Result<(), String> {
        {
            let mut guard = self.state.lock().map_err(|e| e.to_string())?;
            guard.records.insert(key.to_string(), record);
        }
        self.save()
    }

    /// 删除记录；返回是否确实存在并删除。
    pub fn delete_record(&self, key: &CredentialKey) -> Result<bool, String> {
        let removed = {
            let mut guard = self.state.lock().map_err(|e| e.to_string())?;
            guard.records.remove(&key.to_string()).is_some()
        };
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// 读取记录。
    pub fn get_record(&self, key: &CredentialKey) -> Option<CredentialRecord> {
        let guard = self.state.lock().ok()?;
        guard.records.get(&key.to_string()).cloned()
    }

    /// 全部记录（键序稳定）。
    pub fn list_records(&self) -> Vec<(CredentialKey, CredentialRecord)> {
        let guard = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return vec![],
        };
        guard
            .records
            .iter()
            .filter_map(|(k, v)| CredentialKey::parse(k).map(|ck| (ck, v.clone())))
            .collect()
    }

    /// 建立引用：ref → 凭证地址（对齐上游 credentials/reference-updated 语义）。
    pub fn set_reference(
        &self,
        r#ref: &CredentialRef,
        key: &CredentialKey,
    ) -> Result<(), String> {
        {
            let mut guard = self.state.lock().map_err(|e| e.to_string())?;
            guard.references.insert(r#ref.0.clone(), key.to_string());
        }
        self.save()
    }

    /// 解除引用；返回是否确实存在并解除。
    pub fn unset_reference(&self, r#ref: &CredentialRef) -> Result<bool, String> {
        let removed = {
            let mut guard = self.state.lock().map_err(|e| e.to_string())?;
            guard.references.remove(&r#ref.0).is_some()
        };
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// 经引用解析记录（ref → 记录；无引用或无记录返回 None）。
    pub fn resolve_reference(&self, r#ref: &CredentialRef) -> Option<CredentialRecord> {
        let guard = self.state.lock().ok()?;
        let key_str = guard.references.get(&r#ref.0)?;
        let key = CredentialKey::parse(key_str)?;
        guard.records.get(&key.to_string()).cloned()
    }

    /// 原子落盘：写临时文件 → rename（同目录保证原子性）。
    pub fn save(&self) -> Result<(), String> {
        let snapshot = self.state.lock().map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(&*snapshot).map_err(|e| e.to_string())?;
        drop(snapshot);
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("写临时台账失败: {e}"))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("台账落盘失败: {e}"))
    }
}

/// 解析 LLM API Key（凭证缝优先，config 直读回退）：
/// ① `CredentialKey("llm", provider)`（LLM 适配器以 provider route key 寻址）；
/// ② 常见 `CredentialRef`（`MARGUERITE_API_KEY` / `DEEPSEEK_API_KEY` /
///    `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`）；
/// ③ 回退 `fallback`（config/env 直读值）。
///
/// 只有非空密钥才算命中（api-key 记录 `key: None` = 拥有者确认走自身环境发现，
/// 不覆盖回退值）。provider 为空时跳过 ①。
pub fn resolve_llm_key(store: &CredentialsStore, provider: &str, fallback: &str) -> String {
    let key_hit = |record: Option<CredentialRecord>| -> Option<String> {
        match record {
            Some(CredentialRecord::ApiKey { key: Some(k), .. }) if !k.trim().is_empty() => {
                Some(k)
            }
            _ => None,
        }
    };

    // ① provider 路由键（最精确）
    if !provider.trim().is_empty() {
        if let Some(k) = key_hit(store.get_record(&CredentialKey::new("llm", provider))) {
            return k;
        }
    }
    // ② 常见引用（显式引用优先于裸回退）
    for name in [
        "MARGUERITE_API_KEY",
        "DEEPSEEK_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
    ] {
        if let Some(k) = key_hit(store.resolve_reference(&CredentialRef(name.into()))) {
            return k;
        }
    }
    // ③ 回退
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> (CredentialsStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mgt_credentials_{name}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.json");
        (CredentialsStore::load(&path), path)
    }

    #[test]
    fn key_parse_and_display_roundtrip() {
        let k = CredentialKey::parse("llm-pi-ai/deepseek").unwrap();
        assert_eq!(k.scope, "llm-pi-ai");
        assert_eq!(k.id, "deepseek");
        assert_eq!(k.to_string(), "llm-pi-ai/deepseek");
        assert!(CredentialKey::parse("no-slash").is_none());
        assert!(CredentialKey::parse("/x").is_none());
        assert!(CredentialKey::parse("x/").is_none());
        assert!(CredentialKey::parse("").is_none());
    }

    #[test]
    fn set_get_delete_record_roundtrip() {
        let (store, path) = tmp_store("record");
        let key = CredentialKey::new("llm", "deepseek");
        assert!(store.get_record(&key).is_none());
        store
            .set_record(&key, CredentialRecord::api_key("sk-test"))
            .unwrap();
        match store.get_record(&key).unwrap() {
            CredentialRecord::ApiKey { key: k, env } => {
                assert_eq!(k.as_deref(), Some("sk-test"));
                assert!(env.is_empty());
            }
            _ => panic!("应为 api-key 记录"),
        }
        assert!(store.delete_record(&key).unwrap());
        assert!(!store.delete_record(&key).unwrap());
        assert!(store.get_record(&key).is_none());
        // 落盘文件存在且可重新加载
        assert!(path.exists());
        let reloaded = CredentialsStore::load(&path);
        assert!(reloaded.get_record(&key).is_none());
    }

    #[test]
    fn grant_record_payload_is_opaque_but_roundtrips() {
        let (store, _) = tmp_store("grant");
        let key = CredentialKey::new("auth", "oauth");
        let payload = serde_json::json!({"token": "t", "expires": 123});
        store.set_record(&key, CredentialRecord::grant(payload.clone())).unwrap();
        match store.get_record(&key).unwrap() {
            CredentialRecord::Grant { payload: p } => assert_eq!(p, payload),
            _ => panic!("应为 grant 记录"),
        }
        assert_eq!(store.get_record(&key).unwrap().kind(), "grant");
    }

    #[test]
    fn references_resolve_to_records() {
        let (store, _) = tmp_store("reference");
        let key = CredentialKey::new("llm", "deepseek");
        let r#ref = CredentialRef("DEEPSEEK_API_KEY".into());
        store
            .set_record(&key, CredentialRecord::api_key("sk-x"))
            .unwrap();
        store.set_reference(&r#ref, &key).unwrap();
        match store.resolve_reference(&r#ref) {
            Some(CredentialRecord::ApiKey { key: k, .. }) => assert_eq!(k.as_deref(), Some("sk-x")),
            other => panic!("resolve 结果错误: {other:?}"),
        }
        assert!(store.unset_reference(&r#ref).unwrap());
        assert!(store.resolve_reference(&r#ref).is_none());
    }

    #[test]
    fn corrupt_file_falls_back_to_empty() {
        let dir = std::env::temp_dir().join(format!("mgt_credentials_corrupt_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.json");
        std::fs::write(&path, "{ not json").unwrap();
        let store = CredentialsStore::load(&path);
        assert!(store.list_records().is_empty());
    }

    #[test]
    fn persistence_survives_reload() {
        let (store, path) = tmp_store("persist");
        let key = CredentialKey::new("llm", "deepseek");
        store
            .set_record(&key, CredentialRecord::api_key("sk-persist"))
            .unwrap();
        let reloaded = CredentialsStore::load(&path);
        match reloaded.get_record(&key) {
            Some(CredentialRecord::ApiKey { key: Some(k), .. }) => assert_eq!(k, "sk-persist"),
            other => panic!("重载后记录缺失/错误: {other:?}"),
        }
    }

    #[test]
    fn resolve_llm_key_prefers_provider_route_key() {
        let (store, _) = tmp_store("resolve_provider");
        store
            .set_record(
                &CredentialKey::new("llm", "deepseek"),
                CredentialRecord::api_key("sk-provider"),
            )
            .unwrap();
        store
            .set_record(
                &CredentialKey::new("llm", "other"),
                CredentialRecord::api_key("sk-other"),
            )
            .unwrap();
        assert_eq!(
            resolve_llm_key(&store, "deepseek", "sk-fallback"),
            "sk-provider"
        );
        // 未登记 provider → 走引用/回退
        assert_eq!(
            resolve_llm_key(&store, "openai", "sk-fallback"),
            "sk-fallback"
        );
    }

    #[test]
    fn resolve_llm_key_falls_back_through_references() {
        let (store, _) = tmp_store("resolve_ref");
        let key = CredentialKey::new("llm", "deepseek");
        store
            .set_record(&key, CredentialRecord::api_key("sk-ref"))
            .unwrap();
        store
            .set_reference(&CredentialRef("MARGUERITE_API_KEY".into()), &key)
            .unwrap();
        // provider 未直接登记，但引用命中
        assert_eq!(
            resolve_llm_key(&store, "unknown-provider", "sk-fallback"),
            "sk-ref"
        );
    }

    #[test]
    fn resolve_llm_key_ignores_empty_and_missing() {
        let (store, _) = tmp_store("resolve_empty");
        // api-key 记录 key=None（拥有者确认走环境发现）→ 不覆盖回退
        store
            .set_record(
                &CredentialKey::new("llm", "deepseek"),
                CredentialRecord::ApiKey {
                    key: None,
                    env: BTreeMap::new(),
                },
            )
            .unwrap();
        assert_eq!(
            resolve_llm_key(&store, "deepseek", "sk-fallback"),
            "sk-fallback",
            "空密钥记录不得覆盖回退值"
        );
        // 空 provider 跳过路由键查询
        assert_eq!(resolve_llm_key(&store, "", "sk-fallback"), "sk-fallback");
    }
}
