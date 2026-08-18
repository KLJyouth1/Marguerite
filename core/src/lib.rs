//! Marguerite Core — Rust 核心引擎库
//!
//! 提供 CLI 路由、配置管理、Agent 运行时、模型路由、工具循环、MCP 客户端等核心功能。

pub mod agent;
pub mod automation;
pub mod browser;
pub mod cache;
pub mod chain;
pub mod cli;
pub mod codeintel;
pub mod collab;
pub mod context;
pub mod desktop_automation;
pub mod deploy;
pub mod domain;
pub mod events;
pub mod gateway;
/// 生成式 UI 交互契约（Track C）：版本化 `Widget` 枚举、action 事件回模型、
/// 沙箱 XSS 校验。渲染层只消费类型化 widget，且入渲染前必须过 `genui::validate`。
pub mod genui;
pub mod governance;
pub mod guard;
/// 原生 harness 层：子代理隔离运行时、生命周期钩子、动态模型路由、
/// git 检查点、能力注册表。为原生（Rust）智能体提供可复用的 Execution-Harness。
pub mod harness;
pub mod intent;
pub mod kb;
pub mod mcp;
pub mod memory;
pub mod mode;
pub mod model;
pub mod orchestration;
pub mod router;
pub mod persistence;
pub mod plan;
pub mod plugins;
pub mod pyengine;
pub mod recovery;
pub mod research;
pub mod sandbox;
pub mod security;
pub mod serve;
pub mod session;
pub mod skills;
pub mod terminal;
pub mod tools;
pub mod tui;
pub mod types;
pub mod usage;
/// 工作台领域/数据层（Track D，锚 DSH-better-sidebar）：面板注册表、AgentMeta 目录、
/// 主题 token 表、用量仪表、鲸鱼娘状态机的纯逻辑主干。渲染层（Vue/TUI）须消费此 domain，
/// 且任何进入渲染的文本须先过 `genui::validate` 类闸门防注入。
pub mod workbench;
pub mod vision;
