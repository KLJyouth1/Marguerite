//! 会话级思考强度 + 上下文持久化（T3：成本治理刚需）。
//!
//! 目标：
//! 1. **会话级思考强度**：reasoning effort 按会话配置（会话创建/切换时设定），
//!    请求级可覆盖，随会话持久化到 `data_dir/thinking/<session>.json`。
//! 2. **上下文持久化**：会话恢复时恢复思考档位与关键上下文锚点
//!    （当前模式 / 工作区归属 / 最近请求时间），支撑成本核算
//!    （每次请求的 effort/预算档位进出历史，可审计）。
//!
//! 与既有设施对齐（不另起炉灶）：
//! - 档位语义复用 `model::thinking::ThinkingEffort`（Off/Normal/Deep/Max，
//!   与 `mode/spec.rs` 的 reasoning_effort 字符串同源）。
//! - 落点复用 `router::RouteDecision`：会话档位以 `effort_override` 形式
//!   覆盖本回合 classify 的自适应决策；请求级 > 会话级 > 自适应。
//! - 与 3×3 模式矩阵的关系：模式规格（mode spec）声明的 reasoning_effort
//!   是**模式默认档**；会话级档位是**用户显式选择**，显式选择优先。

use std::path::PathBuf;
// 注意：下方 149/253 行的静态项用全限定 `std::sync::OnceLock`，顶层 `use` 会
// 因未直接使用而触发 unused_imports 警告，故不再在此重复导入。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::thinking::ThinkingEffort;

/// 档位来源（成本核算审计用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortSource {
    /// 会话创建/切换时由前端显式设定（持久档位）
    Session,
    /// 本轮请求显式覆盖（请求级 > 会话级）
    Request,
    /// 系统迁移/默认初始化
    System,
}

impl EffortSource {
    pub fn as_str(self) -> &'static str {
        match self {
            EffortSource::Session => "session",
            EffortSource::Request => "request",
            EffortSource::System => "system",
        }
    }
}

/// 一次档位变更/请求记录（成本核算的最小审计单元）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffortRecord {
    pub effort: ThinkingEffort,
    pub source: EffortSource,
    pub at: i64,
    /// 成本备注（如 tier/mode 快照；不臆造 token 数——真实用量由 usage 台账记录）
    pub note: String,
}

/// 关键上下文锚点：会话恢复时随思考档位一起恢复的状态快照。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextAnchors {
    /// 当前模式（3×3 模式矩阵的一格）
    pub mode: Option<String>,
    /// 工作区/项目归属
    pub workspace: Option<String>,
    /// 最近一次请求时间（unix 秒）
    pub last_request_at: Option<i64>,
    /// 已注入工具 schema 摘要（数量 + 摘要串；由接线方在组装工具集时更新）
    pub tool_schema_digest: Option<String>,
}

/// 会话级思考偏好（落盘结构）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionThinkingPrefs {
    pub session_key: String,
    /// 当前思考档位（会话持久档位；请求级覆盖不写这里）
    pub effort: ThinkingEffort,
    /// 档位更新时间（unix 秒）
    pub updated_at: i64,
    /// 变更/请求历史（最近 N 条，供成本核算）
    #[serde(default)]
    pub history: Vec<EffortRecord>,
    /// 关键上下文锚点
    #[serde(default)]
    pub anchors: ContextAnchors,
    /// 结构版本（未来迁移用）
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 {
    1
}

/// 历史保留上限（防无限增长）。
const HISTORY_CAP: usize = 64;

impl SessionThinkingPrefs {
    fn new(session_key: &str) -> Self {
        Self {
            session_key: session_key.to_string(),
            effort: ThinkingEffort::default(),
            updated_at: now_ts(),
            history: Vec::new(),
            anchors: ContextAnchors::default(),
            version: default_version(),
        }
    }

    /// 记录一次请求（不改变持久档位；用于成本核算）。
    pub fn note_request(&mut self, effort: ThinkingEffort, note: impl Into<String>) {
        self.anchors.last_request_at = Some(now_ts());
        self.push_record(EffortRecord {
            effort,
            source: EffortSource::Request,
            at: now_ts(),
            note: note.into(),
        });
    }

    /// 设定持久档位（会话创建/切换/请求级持久化都走这里）。
    pub fn set_effort(&mut self, effort: ThinkingEffort, source: EffortSource, note: impl Into<String>) {
        self.effort = effort;
        self.updated_at = now_ts();
        self.push_record(EffortRecord {
            effort,
            source,
            at: self.updated_at,
            note: note.into(),
        });
    }

    fn push_record(&mut self, rec: EffortRecord) {
        self.history.push(rec);
        if self.history.len() > HISTORY_CAP {
            let excess = self.history.len() - HISTORY_CAP;
            self.history.drain(0..excess);
        }
    }
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 数据目录覆盖（测试注入用；生产恒为 None → 走 `Config::data_dir()`）。
static DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn thinking_dir() -> PathBuf {
    if let Some(dir) = DIR_OVERRIDE.get() {
        return dir.clone();
    }
    crate::persistence::config::Config::data_dir().join("thinking")
}

/// session key → 安全文件名（与 context/session.rs 同口径的字符白名单）。
fn safe_key(session: &str) -> String {
    session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn path_for(session: &str) -> PathBuf {
    thinking_dir().join(format!("{}.json", safe_key(session)))
}

/// 读取会话思考偏好（不存在返回 None；文件损坏按 None 处理并告警——
/// 思考档位缺失只降级为自适应，不阻断对话）。
pub fn load(session: &str) -> Option<SessionThinkingPrefs> {
    let path = path_for(session);
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<SessionThinkingPrefs>(&raw) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!(
                    "thinking prefs parse failed ({}): {} —— 降级为自适应档位",
                    path.display(),
                    e
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::warn!("thinking prefs read failed ({}): {}", path.display(), e);
            None
        }
    }
}

/// 落盘会话思考偏好。
pub fn save(prefs: &SessionThinkingPrefs) -> Result<()> {
    let dir = thinking_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create {}", dir.display()))?;
    let raw = serde_json::to_vec_pretty(prefs)?;
    let tmp = path_for(&prefs.session_key).with_extension("tmp");
    std::fs::write(&tmp, &raw).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path_for(&prefs.session_key))
        .with_context(|| format!("commit prefs for {}", prefs.session_key))?;
    Ok(())
}

/// 读取或新建会话偏好（新建不落盘，保存由调用方决定）。
pub fn load_or_new(session: &str) -> SessionThinkingPrefs {
    load(session).unwrap_or_else(|| SessionThinkingPrefs::new(session))
}

/// 设定会话持久档位并落盘。返回更新后的偏好。
pub fn set_effort(
    session: &str,
    effort: ThinkingEffort,
    source: EffortSource,
    note: impl Into<String>,
) -> Result<SessionThinkingPrefs> {
    let mut prefs = load_or_new(session);
    prefs.set_effort(effort, source, note);
    save(&prefs)?;
    Ok(prefs)
}

/// 清除会话档位覆盖（「继承全局 / 自适应」）。
///
/// 语义区别很关键：`effort = Off` 是**显式要求模型不思考**，
/// 而「清除」是**没有覆盖**、让模式/任务分类自适应决定 —— 两者不能互相代替。
/// 因此清除 = 删除偏好文件（`load()` 返回 None），而不是把档位写成 Off。
pub fn clear(session: &str) -> Result<()> {
    let path = path_for(session);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", path.display())),
    }
}

/// 记录一次请求的成本锚点（不改变持久档位）。best-effort：写盘失败仅告警。
pub fn note_request(session: &str, effort: ThinkingEffort, note: impl Into<String>) {
    let mut prefs = load_or_new(session);
    prefs.note_request(effort, note);
    if let Err(e) = save(&prefs) {
        tracing::warn!("thinking prefs note_request save failed: {}", e);
    }
}

/// 上下文锚点更新（best-effort）。
pub fn update_anchors(session: &str, f: impl FnOnce(&mut ContextAnchors)) {
    let mut prefs = load_or_new(session);
    f(&mut prefs.anchors);
    if let Err(e) = save(&prefs) {
        tracing::warn!("thinking prefs update_anchors save failed: {}", e);
    }
}

/// 会话档位 → 请求体档位解析：会话持久档位优先，缺失/损坏回落 spec 档位。
///
/// 供编排层等生产注入点复用（模式规格声明是默认档，用户显式选择的会话档位覆盖它）。
pub fn resolve_effort(
    session: Option<&str>,
    spec_effort: ThinkingEffort,
) -> ThinkingEffort {
    session
        .and_then(load)
        .map(|p| p.effort)
        .unwrap_or(spec_effort)
}

/// 测试：把数据目录固定到进程级临时目录（只设一次，进程内所有本模块测试共享；
/// 不用环境变量，避免与并行测试竞态）。
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
pub(crate) fn ensure_test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mgt_thinking_tests_{}", std::process::id()));
    let _ = DIR_OVERRIDE.set(dir.clone());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_session(tag: &str) -> String {
        format!("test.thinking_prefs.{tag}.{}", std::process::id())
    }

    #[test]
    fn set_and_load_roundtrip_with_history_cap() {
        let _g = test_guard();
        let dir = ensure_test_dir();
        let sk = tmp_session("roundtrip");
        let _ = std::fs::remove_file(path_for(&sk));

        let p = set_effort(&sk, ThinkingEffort::Deep, EffortSource::Session, "unit").unwrap();
        assert_eq!(p.effort, ThinkingEffort::Deep);
        assert_eq!(p.history.len(), 1);
        assert_eq!(p.history[0].source, EffortSource::Session);

        let loaded = load(&sk).expect("落盘后必须能读回");
        assert_eq!(loaded.effort, ThinkingEffort::Deep);
        assert_eq!(loaded.anchors.last_request_at, None);

        // 请求记录只进历史不动档位
        note_request(&sk, ThinkingEffort::Max, "req1");
        let loaded = load(&sk).unwrap();
        assert_eq!(loaded.effort, ThinkingEffort::Deep, "note_request 不得改持久档位");
        assert_eq!(loaded.history.len(), 2);
        assert!(loaded.anchors.last_request_at.is_some());

        // 历史封顶
        for i in 0..(HISTORY_CAP + 10) {
            note_request(&sk, ThinkingEffort::Normal, format!("bulk{i}"));
        }
        let loaded = load(&sk).unwrap();
        assert_eq!(loaded.history.len(), HISTORY_CAP, "历史必须封顶");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_file_degrades_to_none_not_panic() {
        let _g = test_guard();
        let dir = ensure_test_dir();
        let sk = tmp_session("corrupt");
        std::fs::create_dir_all(thinking_dir()).unwrap();
        std::fs::write(path_for(&sk), "{not json").unwrap();
        assert!(load(&sk).is_none(), "损坏文件必须降级 None 而非 panic");
        // 降级后仍可正常新建
        let p = set_effort(&sk, ThinkingEffort::Off, EffortSource::System, "reset").unwrap();
        assert_eq!(p.effort, ThinkingEffort::Off);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
