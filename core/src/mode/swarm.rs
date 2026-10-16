//! 蜂群控制：**数量上限设置**（用户可配置，持久化 `<data_dir>/swarm_config.json`）。
//!
//! 供智能体 / 智能体团队 / 专家团队在 Bloom / OmniTranscend 模式下启动蜂群时读取：
//! 分片数（并行攻坚方向）与子专家数（每方向专家席位）均有界钳制，防「上千 subagent」失控。

use serde::{Deserialize, Serialize};

/// 蜂群配置（0 表示用内置默认值）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SwarmConfig {
    /// 蜂群分片数量上限（0 = 默认 11 / OmniTranscend 13）
    pub max_swarm_shards: usize,
    /// 子专家数量上限（0 = 默认 5）
    pub max_sub_experts: usize,
}

/// 分片数量上限的硬边界（防误配）。
pub const MAX_SHARDS_CAP: usize = 200;
/// 子专家数量上限的硬边界。
pub const MAX_SUB_EXPERTS_CAP: usize = 50;

fn path() -> std::path::PathBuf {
    crate::persistence::config::Config::data_dir().join("swarm_config.json")
}

pub fn load() -> SwarmConfig {
    load_from(&path())
}

pub fn load_from(p: &std::path::Path) -> SwarmConfig {
    match std::fs::read_to_string(p) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => SwarmConfig::default(),
    }
}

pub fn save(cfg: &SwarmConfig) -> Result<(), String> {
    save_to(cfg, &path())
}

pub fn save_to(cfg: &SwarmConfig, p: &std::path::Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化失败: {e}"))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入临时配置失败: {e}"))?;
    std::fs::rename(&tmp, p).map_err(|e| format!("原子替换失败: {e}"))
}

/// 生效的蜂群分片上限（用户配置优先，钳制 1..=200）。
pub fn effective_shards(default_shards: usize) -> usize {
    let cfg = load();
    if cfg.max_swarm_shards == 0 {
        default_shards.clamp(1, MAX_SHARDS_CAP)
    } else {
        cfg.max_swarm_shards.clamp(1, MAX_SHARDS_CAP)
    }
}

/// 生效的子专家上限（用户配置优先，钳制 1..=50）。
pub fn effective_sub_experts(default_sub_experts: usize) -> usize {
    let cfg = load();
    if cfg.max_sub_experts == 0 {
        default_sub_experts.clamp(1, MAX_SUB_EXPERTS_CAP)
    } else {
        cfg.max_sub_experts.clamp(1, MAX_SUB_EXPERTS_CAP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path() -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("marguerite_swarm_test_{ts}.json"))
    }

    #[test]
    fn default_falls_back_to_builtin() {
        let cfg = SwarmConfig::default();
        assert_eq!(cfg.max_swarm_shards, 0);
        assert_eq!(effective_shards(11), 11, "未配置时用内置默认");
        assert_eq!(effective_sub_experts(5), 5);
    }

    #[test]
    fn override_and_clamp() {
        let p = temp_path();
        let mut cfg = SwarmConfig {
            max_swarm_shards: 40,
            ..Default::default()
        };
        save_to(&cfg, &p).unwrap();
        assert_eq!(load_from(&p).max_swarm_shards, 40);
        cfg.max_swarm_shards = 9999;
        cfg.max_sub_experts = 3;
        assert_eq!(
            cfg.max_swarm_shards.clamp(1, MAX_SHARDS_CAP),
            MAX_SHARDS_CAP,
            "超上限钳制"
        );
        assert_eq!(cfg.max_sub_experts.clamp(1, MAX_SUB_EXPERTS_CAP), 3);
    }

    #[test]
    fn roundtrip_serialization() {
        let p = temp_path();
        let cfg = SwarmConfig {
            max_swarm_shards: 17,
            max_sub_experts: 6,
        };
        save_to(&cfg, &p).unwrap();
        assert_eq!(load_from(&p), cfg);
    }
}
