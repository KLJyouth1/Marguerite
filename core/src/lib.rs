//! Marguerite Core — Rust 核心引擎库
//!
//! 提供 CLI 路由、配置管理、Agent 运行时、模型路由、工具循环、MCP 客户端等核心功能。

pub mod agent;
pub mod automation;
pub mod browser;
pub mod cache;
/// Track E — 第三阶段研究主干（RAG/知识图谱/NLP/缓存命中率/记忆增强）纯逻辑切片：
/// context_budget 四态、self-learning 提升/闸门策略、NLP 原语、KG 召回接口。
/// 重 ML 推理（ONNX/lgbm 实际推理）与推背图（零来源）明确不落地，仅透明标注。
pub mod e;
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
/// 目标循环（Goal Round Driver，Track 1）：单一持久化完成目标 + 自治续轮
/// （armed/disarmed）+ resume/pause/complete/blocked 状态机。对齐 DSH goal 子系统。
pub mod goal;
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
/// 德州扑克游戏引擎（Game Hall / 赌 Token）：纯逻辑、确定性 RNG、可序列化。
/// 房间模型传输无关；跨机联机需 relay 服务（本切片落地本地多人对战 + AI 座位）。
pub mod poker;
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

/// T-1.8 / DSH 跨模块闭环测试骨架（仅 `cfg(test)` 编译，**不导出任何生产符号**）。
///
/// 测试都放在 `src/tests/*.rs`，并通过 `pub mod tests` 作为库的子模块暴露——
/// 这使得 `cargo test -p core` 能把它们当作 unit test 运行并访问
/// `crate::*` 的 pub(crate) 助手（比 `core/tests/` 集成测试节省样板、无需重导）。
#[cfg(test)]
pub mod tests {
    pub mod dsh_e2e;
    pub mod mock_http;
}
