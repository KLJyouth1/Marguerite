//! Python 引擎侧车（Phase 3：双引擎桥）
//!
//! 将 Marguerite Python 引擎以子进程方式托管为侧车，实现：
//! - 子进程生命周期：spawn / 健康探测 / 优雅停止 / 崩溃重启（指数退避）
//! - 统一命令行：`python -m Marguerite.cli.main gateway run --bind 127.0.0.1 --port <port>`
//! - 环境传递：`PYTHONPATH=pyengine/src` + `Marguerite_HOME=<data_dir>/pyengine-home`
//! - 客户端：[`client::PyGatewayClient`] 转发 §5.2 方法组到 Python 网关 WS
//! - 事件桥：[`event_bridge`] 订阅侧车 WS event 帧并 emit 到 Rust EventBus
//!
//! 安全：侧车只监听 `127.0.0.1`（与 Rust 网关一致），绝不暴露公网。
//! Python 网关自己的 token 由 [`auth::load_or_create_token`] 共享同一 SSOT。

pub mod auth;
pub mod client;
pub mod event_bridge;
pub mod memfence;
pub mod sidecar;

pub use sidecar::{
    detect_python, free_loopback_port, parse_python_version, PyEngineSidecar, PyEngineStatus,
    SidecarConfig,
};
