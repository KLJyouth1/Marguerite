//! Marguerite Core — Rust 核心引擎库
//!
//! 提供 CLI 路由、配置管理、Agent 运行时、模型路由、工具循环、MCP 客户端等核心功能。

pub mod agent;
pub mod assistant;
/// v3.4 PSI 意识引擎（Aris Brain）：量子认知循环 + 八维情感 + GWT 广播
pub mod rules;
pub mod aris_brain;
pub mod audit_store;
pub mod automation;
pub mod browser;
pub mod cache;
pub mod chain;
pub mod cli;
pub mod codeintel;
pub mod collab;
pub mod context;
/// DSH 凭证缝（credentials/authorization 移植，上游 PR #2509）：CredentialKey
/// `<scope>/<id>` 与 CredentialRef 双名义键空间、api-key/grant 记录、JSON 原子
/// 落盘台账 + 授权流注册表（方法/通知/提示/结算）。
pub mod credentials;
pub mod deploy;
pub mod dev;
pub mod desktop_automation;
/// 万物互联设备总线（WP1）：统一设备注册 + 危险等级 L0-L3 + 协议 adapter
/// （HA REST/WS、MQTT）。仅 127.0.0.1 网关出向 LAN；物理致动默认 fail-closed。
pub mod devices;
/// M5 doctor 体检模块：纯本机、可亲验、可复制 JSON 报告（PACKAGING.md §6.1 七项）。
pub mod doctor;
pub mod domain;
/// Track E — 第三阶段研究主干（RAG/知识图谱/NLP/缓存命中率/记忆增强）纯逻辑切片：
/// context_budget 四态、self-learning 提升/闸门策略、NLP 原语、KG 召回接口。
/// 重 ML 推理（ONNX/lgbm 实际推理）与推背图（零来源）明确不落地，仅透明标注。
pub mod e;
/// 结构化步骤错误（P1-5 / T05）：用确定性类别替代「中文子串嗅探」做重试裁决。
pub mod errors;
pub mod events;
/// 小游戏引擎（Game Hall）：纯逻辑、确定性。当前含井字棋（TicTacToe）引擎，
/// 落地 `dsh.games.tictactoe` 能力；TUI/Web 渲染层消费引擎状态，不重复实现规则。
pub mod games;
pub mod gateway;
/// 生成式 UI 交互契约（Track C）：版本化 `Widget` 枚举、action 事件回模型、
/// 沙箱 XSS 校验。渲染层只消费类型化 widget，且入渲染前必须过 `genui::validate`。
pub mod genui;
/// 目标循环（Goal Round Driver，Track 1）：单一持久化完成目标 + 自治续轮
/// （armed/disarmed）+ resume/pause/complete/blocked 状态机。对齐 DSH goal 子系统。
pub mod goal;
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
/// S6 可观测性：按大小轮转 + 目录封顶的日志、全仓唯一脱敏（`scrub_text`）、
/// `--export-diagnostics` 诊断包导出。三者共用一条纪律：任何离开本机的文本
/// 必经 `observability::redact::scrub_text`。
pub mod observability;
pub mod orchestration;
pub mod ops;
pub mod persistence;
/// DSH persona 组合渲染移植（上游 persona 包）：PersonaSection 有序拼接 +
/// complete 阻断后续段 + `{{model}}`/`{{cwd}}` 模板解析。
pub mod persona;
/// Vault 主口令闭环（P0-2）：用户主口令 + PBKDF2-HMAC-SHA256 派生的 32 字节主密钥，
/// 替换本地随机 `model.key`。提供首启门禁与可诊断错误，由 `model::byok` 无缝接入。
pub mod vault;
pub mod plan;
pub mod plugins;
/// 德州扑克游戏引擎（Game Hall / 赌 Token）：纯逻辑、确定性 RNG、可序列化。
/// 房间模型传输无关；跨机联机需 relay 服务（本切片落地本地多人对战 + AI 座位）。
pub mod poker;
/// 子进程静默执行助手（Windows `CREATE_NO_WINDOW`）：GUI 宿主下启动控制台子进程
/// 不再闪黑窗。见 [`proc::SilentCommand`]。
pub mod proc;
/// DSH session-projection fold 契约移植（上游 PR #2730/#2702）：ProjectionUnit
/// 纯 init/apply fold + 可选 wire.view（host-only 单元无视图）、
/// SessionProjectionStateMap SQLite 状态表、stateOf/viewOf/replay。
pub mod projection;
pub mod pyengine;
pub mod rbac;
pub mod recovery;
pub mod research;
pub mod router;
/// 重型运行时状态（ASR / 本地量化模型）：诚实占位 + 下载源指引，不预下载。
pub mod runtime;
pub mod sandbox;
/// DSH schedule 会话级提醒移植（上游 dsh-schedule）：after/at/every 三种触发、
/// SQLite 持久台账、due 纯查询 + every 锚点重挂载（不枚举积压）、cancel。
pub mod schedule;
pub mod security;
pub mod serve;
pub mod session;
pub mod skills;
pub mod terminal;
pub mod tools;
pub mod tui;
pub mod types;
pub mod usage;
pub mod workspace;
pub mod vision;
/// 工作台领域/数据层（Track D，锚 DSH-better-sidebar）：面板注册表、AgentMeta 目录、
/// 主题 token 表、用量仪表、鲸鱼娘状态机的纯逻辑主干。渲染层（Vue/TUI）须消费此 domain，
/// 且任何进入渲染的文本须先过 `genui::validate` 类闸门防注入。
pub mod workbench;
/// DSH workflow/ralph 移植（上游 dsh-tool-workflow + dsh-tool-ralph）：
/// 声明式 WorkflowSpec（阶段内 fan-out、阶段间屏障、单条失败不拖垮整轮）+
/// Ralph 每轮全新子代理循环（工作区共享记忆、有界结构化报告）。
pub mod workflow;

/// T-1.8 / DSH 跨模块闭环测试骨架（仅 `cfg(test)` 编译，**不导出任何生产符号**）。
///
/// 测试都放在 `src/tests/*.rs`，并通过 `pub mod tests` 作为库的子模块暴露——
/// 这使得 `cargo test -p core` 能把它们当作 unit test 运行并访问
/// `crate::*` 的 pub(crate) 助手（比 `core/tests/` 集成测试节省样板、无需重导）。
#[cfg(test)]
pub mod tests {
    pub mod dsh_e2e;
    /// 联合调用闭环集成测试（v10）：模型路由 → 智能体 → 工具 串起来（离线 mock）。
    pub mod joint_call;
    pub mod mock_http;
}
