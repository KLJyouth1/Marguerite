//! 桌面应用驱动抽象与共享类型。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 受支持的桌面应用种类（对齐 SPEC §6 DesktopAppDriver）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    WpsPpt,
    Browser,
    Jianying,
    ThreeD,
    Generic,
}

impl AppKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppKind::WpsPpt => "wps_ppt",
            AppKind::Browser => "browser",
            AppKind::Jianying => "jianying",
            AppKind::ThreeD => "three_d",
            AppKind::Generic => "generic",
        }
    }

    /// 从字符串解析（接受 snake_case 或原始名），未知返回 None。
    #[allow(clippy::should_implement_trait)] // 命名仿 FromStr 但返回 Option，保留为解析辅助
    pub fn from_str(s: &str) -> Option<AppKind> {
        match s {
            "wps_ppt" | "WpsPpt" => Some(AppKind::WpsPpt),
            "browser" | "Browser" => Some(AppKind::Browser),
            "jianying" | "Jianying" => Some(AppKind::Jianying),
            "three_d" | "ThreeD" => Some(AppKind::ThreeD),
            "generic" | "Generic" => Some(AppKind::Generic),
            _ => None,
        }
    }
}

/// 框架级错误（映射到网关 RPC 错误码，见 protocol.rs）。
#[derive(Debug)]
pub enum DesktopError {
    AppUnknown(String),
    ActionDenied(String),
    ComError(String),
    UnsupportedAction(String),
    NotRunning(String),
    /// 元素级定位失败：应用在跑、通道也通，但按 `ref_id` / `name` 找不到那个控件。
    ///
    /// 单列一个变体是为了不把「找不到控件」混进 `ComError`（那是通道/OS 层失败）或
    /// `NotRunning`（那是应用层失败）——UIA 驱动的 `click` 恰恰最容易命中这一种。
    ElementNotFound(String),
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesktopError::AppUnknown(s) => write!(f, "未知应用: {s}"),
            DesktopError::ActionDenied(s) => write!(f, "动作被安全闸门拒绝: {s}"),
            DesktopError::ComError(s) => write!(f, "COM 错误: {s}"),
            DesktopError::UnsupportedAction(s) => write!(f, "不支持的桌面动作: {s}"),
            DesktopError::NotRunning(s) => write!(f, "应用未运行: {s}"),
            DesktopError::ElementNotFound(s) => write!(f, "控件未找到: {s}"),
        }
    }
}

impl std::error::Error for DesktopError {}

/// 已启动应用的句柄（进程内标识）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppHandle {
    pub id: String,
    pub kind: AppKind,
    pub launched_at: i64,
}

impl AppHandle {
    pub fn new(kind: AppKind) -> Self {
        AppHandle {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            launched_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// 应用注册信息（前端「已注册 + 运行中」展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub kind: AppKind,
    pub name: String,
    pub registered: bool,
    pub running: bool,
}

/// 单次 UI 动作（驱动无关，供 Recorder 落盘 + Policy 裁决）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiAction {
    /// 动作类型（generate_presentation / open / click / type_text ...）。
    pub kind: String,
    #[serde(default)]
    pub params: Value,
}

impl UiAction {
    pub fn param_str(&self, key: &str) -> Option<String> {
        self.params
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }
    pub fn param_bool(&self, key: &str) -> bool {
        self.params
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

/// 动作执行回执（每步经 Recorder 落盘；敏感动作先查 Policy）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub handle: String,
    pub action: String,
    pub ok: bool,
    pub detail: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
    pub ts: i64,
}

impl ActionReceipt {
    pub fn ok(handle: &str, action: &str, detail: Value, snapshot: Option<PathBuf>) -> Self {
        ActionReceipt {
            handle: handle.to_string(),
            action: action.to_string(),
            ok: true,
            detail,
            snapshot_path: snapshot.map(|p| p.to_string_lossy().to_string()),
            ts: chrono::Utc::now().timestamp(),
        }
    }
    pub fn failed(handle: &str, action: &str, detail: Value) -> Self {
        ActionReceipt {
            handle: handle.to_string(),
            action: action.to_string(),
            ok: false,
            detail,
            snapshot_path: None,
            ts: chrono::Utc::now().timestamp(),
        }
    }
}

/// 审计快照元信息（目录 `.marguerite/audit-snapshots/<session>/`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: String,
    pub ts: i64,
    pub action_kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_snapshot: Option<String>,
}

/// 桌面应用驱动接口：启动 / 激活 / 执行动作 / 退出。
///
/// 设计为同步（COM / 进程操作均为阻塞 IO），便于在网关 RPC 内直接调用。
/// `run_action` 额外接收 `session` 用于审计快照落盘与产物登记归属。
pub trait DesktopAppDriver: Send + Sync {
    fn kind(&self) -> AppKind;
    fn launch(&self, args: &Value) -> Result<AppHandle, DesktopError>;
    fn activate(&self, h: &AppHandle) -> Result<(), DesktopError>;
    fn run_action(
        &self,
        h: &AppHandle,
        action: &UiAction,
        session: &str,
    ) -> Result<ActionReceipt, DesktopError>;
    fn quit(&self, h: &AppHandle) -> Result<(), DesktopError>;
}
