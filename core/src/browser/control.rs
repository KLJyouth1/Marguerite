//! 浏览器控制权租约（`browser.control.*`）：用户接管 / 交还。
//!
//! # 为什么需要它
//!
//! 「智能体操控浏览器」与「用户临时自己点」如果只是两条并行通道，谁都能动同一个
//! 页面：用户点了一半，智能体下一步直接把它带走了。真正的接管必须是**互斥**的：
//!
//! - `user` 控制时，`tool.browser` 的一切页面动作被拒绝（带可执行提示），
//!   智能体只能向用户确认或等交还；
//! - `agent` 控制时（默认），前端把面板设为只读镜像，用户点「接管」才切换。
//!
//! 租约是**进程级**的（与浏览器会话注册表同生命周期），不落盘：重启后回到
//! 智能体侧默认值。这符合「接管是交互期的临时状态」的语义，也避免残留的
//! `user` 锁让重启后的智能体永久无法用浏览器。
//!
//! 未被接管时 `agent_block_reason()` 返回 `None`，调用方零开销。

use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// 控制权归属。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Agent,
    User,
}

impl Owner {
    pub fn as_str(self) -> &'static str {
        match self {
            Owner::Agent => "agent",
            Owner::User => "user",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "agent" => Some(Owner::Agent),
            "user" => Some(Owner::User),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Lease {
    owner: Owner,
    /// 接管/交还发生的时刻（毫秒时间戳）；默认值 0 表示从未切换过。
    since_ms: u64,
    /// 切换原因（前端展示；空串表示未提供）
    reason: String,
    /// 发起切换的会话键（便于把「谁接管了」对上对话）
    session: String,
}

impl Default for Lease {
    fn default() -> Self {
        Self {
            owner: Owner::Agent,
            since_ms: 0,
            reason: String::new(),
            session: String::new(),
        }
    }
}

fn lease() -> &'static Mutex<Lease> {
    static L: OnceLock<Mutex<Lease>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Lease::default()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 把毫秒时间戳格式化成**本地**时分秒（给模型看的可读时间）。
fn local_hms(ms: u64) -> String {
    use chrono::TimeZone;
    if ms == 0 {
        return "未知时间".to_string();
    }
    match chrono::Local.timestamp_millis_opt(ms as i64).single() {
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => "未知时间".to_string(),
    }
}

/// `browser.control.get`：当前控制权快照（只读）。
pub fn snapshot() -> Value {
    let g = lease().lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "ok": true,
        "owner": g.owner.as_str(),
        "sinceMs": g.since_ms,
        "sinceLocal": if g.since_ms == 0 { Value::Null } else { json!(local_hms(g.since_ms)) },
        "reason": g.reason,
        "session": g.session,
    })
}

/// `browser.control.set {owner: "user"|"agent", reason?, session?}`：切换控制权。
///
/// 幂等：切到同一个 owner 只更新原因/时间戳（视为一次显式确认）。
pub fn set_owner(params: &Value) -> Result<Value, String> {
    let raw = params
        .get("owner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let owner = Owner::parse(raw)
        .ok_or_else(|| format!("owner 只能是 agent 或 user（收到: {raw}）"))?;
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect::<String>();
    let session = params
        .get("session")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect::<String>();
    {
        let mut g = lease().lock().unwrap_or_else(|e| e.into_inner());
        g.owner = owner;
        g.since_ms = now_ms();
        g.reason = reason;
        g.session = session;
    }
    Ok(snapshot())
}

/// 智能体侧闸门：用户接管期间返回拒绝理由（直接作为工具错误文案）。
///
/// 文案必须**可执行**：说明谁在控制、从什么时候开始、下一步该做什么，
/// 否则模型会反复重试同一个动作直到触发重复失败检测。
pub fn agent_block_reason() -> Option<String> {
    let g = lease().lock().unwrap_or_else(|e| e.into_inner());
    if g.owner != Owner::User {
        return None;
    }
    let since = local_hms(g.since_ms);
    let why = if g.reason.is_empty() {
        String::new()
    } else {
        format!("（原因：{}）", g.reason)
    };
    Some(format!(
        "浏览器当前由**用户接管**（自 {since} 起{why}），智能体的浏览器操作已被暂停。\
         请不要重试、不要新开浏览器会话：先向用户说明你原本要做什么并询问是否可以继续；\
         用户交还控制权后（browser.control.set owner=agent）再继续。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单条用例串起「默认放行 → 用户接管拦截 → 交还放行 → 非法值拒绝」。
    ///
    /// **必须只有一条用例**：租约是进程级全局状态，多个 `#[test]` 并行跑会互相
    /// 覆盖，出现「单独跑绿、全量跑红」的假失败。
    #[test]
    fn lease_blocks_agent_only_while_user_owns_the_browser() {
        // 默认：智能体可用，且从未切换过时不编造时间
        let mut g = lease().lock().unwrap_or_else(|e| e.into_inner());
        *g = Lease::default();
        drop(g);
        assert!(agent_block_reason().is_none(), "默认必须放行智能体");
        let initial = snapshot();
        assert_eq!(initial["owner"], json!("agent"));
        assert_eq!(initial["sinceMs"], json!(0));
        assert!(initial["sinceLocal"].is_null(), "从未切换过时不编造时间");

        // 用户接管 → 拦（且文案可执行：说明原因与下一步）
        let snap = set_owner(&json!({
            "owner": "user",
            "reason": "我在手动登录",
            "session": "agent:main:webchat:test",
        }))
        .expect("合法 owner 必须成功");
        assert_eq!(snap["owner"], json!("user"));
        assert_eq!(snap["reason"], json!("我在手动登录"));
        assert_eq!(snap["session"], json!("agent:main:webchat:test"));
        assert!(snap["sinceMs"].as_u64().unwrap_or(0) > 0);
        let block = agent_block_reason().expect("用户接管期间必须拦截");
        assert!(block.contains("用户接管"), "必须说明谁在控制: {block}");
        assert!(block.contains("交还"), "必须给出恢复路径: {block}");
        assert!(block.contains("我在手动登录"), "必须带上接管原因: {block}");

        // 交还 → 放行
        set_owner(&json!({ "owner": "agent" })).expect("交还必须成功");
        assert!(agent_block_reason().is_none(), "交还后必须放行");

        // 非法值：拒绝，且不得改动现状
        let err = set_owner(&json!({ "owner": "root" })).expect_err("非法 owner 必须失败");
        assert!(err.contains("agent") && err.contains("user"), "错误要给出合法值: {err}");
        assert!(agent_block_reason().is_none());
    }
}
