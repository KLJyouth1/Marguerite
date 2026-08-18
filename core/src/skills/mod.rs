//! 技能系统模块
//!
//! 提供技能引擎、技能加载器和技能注册表三大核心功能：
//!
//! - `engine`：技能执行引擎，负责 Prompt 渲染、技能查找和执行
//! - `loader`：技能加载器，从 YAML 文件加载技能定义
//! - `registry`：技能注册表，管理技能注册、查找和索引

pub mod engine;
pub mod loader;
pub mod registry;

pub use engine::{CombinationOutput, SkillEngine, SkillOutput};
pub use loader::SkillLoader;
pub use registry::{Skill, SkillRegistry, SkillSource};
