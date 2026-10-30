//! 两层助理（L1 全局 / L2 项目）MVP 后端骨架 —— 计划 §4（P1-1 / P1-2）。
//!
//! 三个子模块，各管一件事：
//! - [`scope`]：当前助理作用域 `global` / `project{workspace}`，落盘 `assistant_scope.json`；
//! - [`profile`]：只记**稳定偏好**（语言 / 技术栈 / 禁忌），落盘 `user_profile.md`，可手改；
//! - [`brief`]：把 scope + profile + 跨项目记忆事实合成**结构化**简报（每条事实带 `source`/`score`），
//!   并按 `pinned → preferences → facts(按分值) → open_loops` 的顺序做字符预算裁剪。
//!
//! 路径全部基于 [`Config::config_dir`](crate::persistence::config::Config::config_dir)
//! （默认 `~/.marguerite/`），单测经线程局部的 `set_config_dir_override` 隔离，
//! 绝不读写真实用户目录。
//!
//! 本模块**只提供纯函数与薄持久化**，RPC 注册在网关侧另行接线
//! （方法名 / 入参 / 出参见 [`brief::build_brief`]、[`scope::load_state`] 等文档注释）。

pub mod brief;
pub mod profile;
pub mod scope;

pub use brief::{build_brief, compose_brief, BriefInput, DEFAULT_MAX_CHARS};
pub use profile::UserProfile;
pub use scope::{AssistantScope, AssistantScopeState};

/// 原子写文本：先写同目录 `*.tmp` 再 rename，避免半截文件被下次读取当成合法状态。
pub(crate) fn write_atomic_text(path: &std::path::Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, body).map_err(|e| format!("写入临时文件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换目标文件失败: {e}"))
}
