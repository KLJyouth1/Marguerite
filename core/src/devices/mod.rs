//! 万物互联设备总线（WP1）。
//!
//! 设计契约（见 next-gen 方案 §六 WP1 / IoT 调研 §4.1）：
//! - [`crate::devices::registry::DeviceRegistry`]：已登记设备 id → 能力描述；
//! - [`crate::devices::adapter::DeviceAdapter`] trait：list_entities / call_service / subscribe_events；
//! - 能力 schema 含危险等级 HazardLevel L0-L3；物理致动默认 fail-closed；
//! - 仅 127.0.0.1 网关，出向限 LAN，凭证加密隔离。
//!
//! # 接通现状（如实，2026-10-14 核对）
//!
//! - **已实装**：[`adapter::HomeAssistantAdapter`]（HTTP 轮询 `/api/states`、REST
//!   `call_service`、`validate_base_url` 仅放行回环/RFC1918、凭证经 credentials 台账加密落盘），
//!   由 `gateway::devices` 暴露 `devices.list` / `devices.status` / `devices.call`，
//!   L2/L3 危险动作经审批门人在回路（fail-closed）。
//! - **骨架留尾**：[`mqtt`] 不真正连接 broker（默认 [`mqtt::UnconnectedBus`] 永不连网，
//!   publish 诚实报错），仅 topic→entity 解析与发现白名单可测；WebSocket 事件订阅未做。
//!   未配置 HA 时 list/status 返回 `configured:false` 空列表、call 返回
//!   `DEVICE_NOT_CONFIGURED`——绝不伪造设备。

pub mod adapter;
pub mod mqtt;
pub mod registry;

pub use adapter::{
    AdapterError, DeviceAdapter, DeviceEntry, DeviceRegistry, HomeAssistantAdapter,
};
pub use registry::{Capability, HazardLevel, Transport};
