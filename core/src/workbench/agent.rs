//! AgentMeta 单一真相源（Track D2）。
//!
//! 收敛上游 `agent_catalog.json`（88 条 `AgentMeta`）为 Rust 具名结构体。
//! 反序列化采用具名字段而非裸 `serde_json::Value` —— 字段错配在反序列化即暴露，
//! 不会静默吞掉。未知字段默认忽略（防御个别目录条目带扩展字段）。

use crate::workbench::WorkbenchError;
use serde::Deserialize;

/// 单条 Agent 元信息（对齐 `agent_catalog.json` schema，全 88 条字段一致）。
///
/// 字段保持具名（非裸 `Value`）：`id` / `name` / `role` / `permission` /
/// `source` / `cluster` / `tags` / `skills` / `tools` / `description` /
/// `model` / `readonly` / `navigable`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentMeta {
    /// 稳定标识（如 `"creator"`、`"office"`）。
    pub id: String,
    /// 展示名（如 `"Creator"`）。
    pub name: String,
    /// 角色分组（如 `"创意内容"`）。
    pub role: String,
    /// 权限状态（如 `"可用"`）。
    pub permission: String,
    /// 来源（如 `"内置 persona"`）。
    pub source: String,
    /// 能力聚类（如 `"reasoning"` / `"execution"`）。
    pub cluster: String,
    /// 标签。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 关联技能 id。
    #[serde(default)]
    pub skills: Vec<String>,
    /// 关联工具名。
    #[serde(default)]
    pub tools: Vec<String>,
    /// 一句话描述。
    pub description: String,
    /// 绑定模型（如 `"default"`）。
    pub model: String,
    /// 只读模式。
    pub readonly: bool,
    /// 是否可导航进入。
    pub navigable: bool,
}

/// 从任意 reader 解析 AgentMeta 目录（测试用内联小样本）。
pub fn load_catalog(reader: impl std::io::Read) -> Result<Vec<AgentMeta>, WorkbenchError> {
    let catalog: Vec<AgentMeta> = serde_json::from_reader(reader)?;
    Ok(catalog)
}

/// 从文件路径加载目录（生产环境从 `data_dir/agent_catalog.json` 读取）。
pub fn load_catalog_path(
    path: impl AsRef<std::path::Path>,
) -> Result<Vec<AgentMeta>, WorkbenchError> {
    let file = std::fs::File::open(path)?;
    load_catalog(file)
}

/// 按 id 在目录中查找 AgentMeta（线性查找，目录规模 88 条足够）。
pub fn catalog_by_id<'a>(catalog: &'a [AgentMeta], id: &str) -> Option<&'a AgentMeta> {
    catalog.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("tests_data/sample_catalog.json");

    #[test]
    fn parse_inline_catalog() {
        let catalog = load_catalog(SAMPLE.as_bytes()).expect("sample 必须可解析");
        assert_eq!(catalog.len(), 4);
        let creator = catalog_by_id(&catalog, "creator").expect("creator 应存在");
        assert_eq!(creator.name, "Creator");
        assert_eq!(creator.cluster, "reasoning");
        assert_eq!(creator.role, "创意内容");
        assert!(creator.tags.iter().any(|t| t == "persona"));
        assert!(creator.tools.contains(&"search".to_string()));
        assert!(!creator.readonly);
        assert!(creator.navigable);
        // 命中 + 未命中
        assert!(catalog_by_id(&catalog, "office").is_some());
        assert!(catalog_by_id(&catalog, "nope").is_none());
    }

    #[test]
    fn parse_real_catalog_if_present() {
        // third_party 仅在本地审计环境存在；CI 无该目录时跳过（不视为失败）。
        let candidates = [
            std::path::Path::new(
                "../third_party/external/GUI_Agent/nexustar-api/app/data/agent_catalog.json",
            ),
            std::path::Path::new(
                "third_party/external/GUI_Agent/nexustar-api/app/data/agent_catalog.json",
            ),
            std::path::Path::new(
                "../../third_party/external/GUI_Agent/nexustar-api/app/data/agent_catalog.json",
            ),
        ];
        let Some(path) = candidates.iter().find(|p| p.exists()) else {
            eprintln!("skip parse_real_catalog_if_present: third_party 不存在（CI 环境）");
            return;
        };
        let catalog = load_catalog_path(path).expect("真实 agent_catalog.json 必须可解析");
        assert!(!catalog.is_empty(), "目录不应为空");
        assert!(catalog_by_id(&catalog, "creator").is_some());
        assert!(catalog_by_id(&catalog, "office").is_some());
    }
}
