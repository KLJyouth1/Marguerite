//! 多人远程协作模块（Phase I1，FR-15）
//!
//! 轻量协作中心（用户 2026-08-05 决议 D-01：5-10 人团队用 SSH 隧道，非全 P2P）。
//! 聚焦编程工作台场景：成员身份 + RBAC + 协作会话（事件流广播/回放）+ 文件级冲突检测。
//! 网关与中心服务为可选增强，本模块提供零依赖、可单测的协作原语，供 CLI/TUI/GUI 共用。

pub mod auth;
pub mod collab_session;
pub mod conflict;
pub mod crdt;
pub mod gateway;
pub mod presence;
pub mod session;
pub mod workspace;
