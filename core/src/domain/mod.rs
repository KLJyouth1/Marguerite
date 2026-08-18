//! Marguerite 统一领域模型（Single Source of Truth）
//!
//! 取代 nexustar-api (Python) 与 nexustar-ui (TypeScript) 中平行的领域定义。
//! CLI、TUI（Ratatui）、原生桌面 GUI（egui）三前端共用此处的类型，
//! 从根上消除 Rust / Python / TS 三套实现的类型漂移。

use serde::{Deserialize, Serialize};
use std::fmt;

pub mod capabilities;

/// 对话角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
    Orchestrator,
}

/// Agent 编排形态。
///
/// `Solo` 等价于 标准 AI 助手 的单 Agent 编码循环（基线）；
/// 其余三种为 Marguerite 的超越项——也是「比它们强数倍」的核心来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentMode {
    /// 单 Agent 编码循环（标准 AI 助手 基线等价）
    Solo,
    /// MAX 双带队：Flash 规划 + Pro 执行，原生的规划/执行分离
    DualLeader,
    /// 专家团队：角色专精的子 Agent，统一走 spawn 模型
    ExpertTeam,
    /// 集群：分配器调度的多 Worker 并行
    Cluster,
}

impl AgentMode {
    pub fn all() -> &'static [AgentMode] {
        &[
            AgentMode::Solo,
            AgentMode::DualLeader,
            AgentMode::ExpertTeam,
            AgentMode::Cluster,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            AgentMode::Solo => "Solo Agent",
            AgentMode::DualLeader => "MAX Dual-Leader",
            AgentMode::ExpertTeam => "Expert Team",
            AgentMode::Cluster => "Cluster",
        }
    }

    /// 是否超出 标准 AI 助手 基线能力。
    pub fn exceeds_baseline(&self) -> bool {
        !matches!(self, AgentMode::Solo)
    }
}

/// 记忆层级（5 层，含自进化闭环）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    Working,
    Episodic,
    Semantic,
    Project,
    /// 自进化闭环层：从踩坑中沉淀、验证、升格为决策记录
    Evolutionary,
}

impl MemoryTier {
    pub fn all() -> &'static [MemoryTier] {
        &[
            MemoryTier::Working,
            MemoryTier::Episodic,
            MemoryTier::Semantic,
            MemoryTier::Project,
            MemoryTier::Evolutionary,
        ]
    }
}

/// 模型后端种类：云端 API 或本地量化模型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Cloud,
    Local,
}

/// 强类型 ID 包装，避免字符串在系统中乱飞。
macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }
    };
}

id_newtype!(SessionId);
id_newtype!(AgentId);
id_newtype!(ToolId);
id_newtype!(ModelId);
id_newtype!(CapabilityId);
id_newtype!(TaskId);

/// 一条对话消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// 可选的结构化载荷（工具调用结果、编排快照等）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: content.into(),
            meta: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: content.into(),
            meta: None,
        }
    }
}

/// 一次会话：可承载任意编排形态与多模型。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub mode: AgentMode,
    pub title: String,
    #[serde(default)]
    pub messages: Vec<Message>,
}
