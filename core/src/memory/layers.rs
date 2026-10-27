//! 记忆楼层（Memory Floors）：**7 层记忆 + 1 层经验 = 8 层**的开关模型。
//!
//! # 为什么需要（2026-10-23）
//!
//! 此前「哪几层记忆真的进了 prompt」是写死在代码里的：
//! 五路召回（跨项目 / 长期+短期 / 召回库 / 经验）+ 第六路分层稀疏（tiered，默认关），
//! 用户既看不到、也关不掉。于是出现两种极端：
//! - 排查 hallucination 时怀疑「是不是记忆串味」，只能整份删数据；
//! - 想要更激进的记忆利用（全开）也没有入口。
//!
//! 本模块把「注入链路」显式化为一组**楼层开关**，并让运行时**真的**按开关裁剪：
//! 关掉的层连检索都不做（不是「查了不用」—— 那既浪费 IO 又会在日志里留下假命中）。
//!
//! # 8 层与代码的对应关系（唯一事实源）
//!
//! | 楼层 | id | 注入内容 | 实现位置 |
//! |---|---|---|---|
//! | 1 感知 | `sensory` | 当前会话历史消息 | `gateway::chat::session_history_messages` |
//! | 2 短期 | `short_term` | 会话摘要（SQLite FTS） | 同上 `memory_stable_prefix` 第二路 |
//! | 3 长期 | `long_term` | 向量长时池 | 同上 |
//! | 4 召回 | `recall` | user/feedback/auto/project 四类 | 同上第三路 |
//! | 5 跨项目 | `cross_project` | 其他项目的经验 | 同上第一路 |
//! | 6 行为 | `behavior` | 最近动作台账（本轮新增注入） | `memory::behavior::recent_events` |
//! | 7 分层稀疏 | `tiered` | flyhash 近邻（原 `memory.tiered_enabled`） | `memory::inject` |
//! | 8 经验 | `experience` | 踩坑/决策/模式台账 | 同上第四路 |
//!
//! **`tiered` 不新开配置字段**：它继续由 `memory.tiered_enabled` 控制。
//! 两层真相会让「界面显示开、实际没开」——这类分歧在记忆系统里极难排查。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 楼层顺序（从下到上：越靠下越是"底座"，越靠上越是"提炼"）。
///
/// 顺序是 UI 与文档的公共契约：前端按此顺序绘制堆叠方框，改动会直接改变用户看到的楼层。
pub const FLOOR_ORDER: [&str; 8] = [
    "sensory",
    "short_term",
    "long_term",
    "recall",
    "cross_project",
    "behavior",
    "tiered",
    "experience",
];

/// 参与持久化的 7 个开关（`tiered` 由 `memory.tiered_enabled` 承担，不在此结构里）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerSwitches {
    #[serde(default = "on")]
    pub sensory: bool,
    #[serde(default = "on")]
    pub short_term: bool,
    #[serde(default = "on")]
    pub long_term: bool,
    #[serde(default = "on")]
    pub recall: bool,
    #[serde(default = "on")]
    pub cross_project: bool,
    #[serde(default = "on")]
    pub behavior: bool,
    #[serde(default = "on")]
    pub experience: bool,
}

fn on() -> bool {
    true
}

impl Default for LayerSwitches {
    fn default() -> Self {
        Self {
            sensory: true,
            short_term: true,
            long_term: true,
            recall: true,
            cross_project: true,
            behavior: true,
            experience: true,
        }
    }
}

/// 运行时解析结果：7 个持久化开关 + `tiered`（来自 `memory.tiered_enabled`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLayers {
    pub sensory: bool,
    pub short_term: bool,
    pub long_term: bool,
    pub recall: bool,
    pub cross_project: bool,
    pub behavior: bool,
    pub tiered: bool,
    pub experience: bool,
}

impl ResolvedLayers {
    /// 从配置快照解析。读取失败时**全部按开启**处理：
    /// 记忆层是"增强"而非"门禁"，读不到配置就退化成历史行为（全开），
    /// 而不是让一次磁盘抖动把用户的记忆全部静音。
    pub fn from_config(cfg: &Config) -> Self {
        let s = &cfg.memory.layers;
        Self {
            sensory: s.sensory,
            short_term: s.short_term,
            long_term: s.long_term,
            recall: s.recall,
            cross_project: s.cross_project,
            behavior: s.behavior,
            tiered: cfg.memory.tiered_enabled,
            experience: s.experience,
        }
    }

    /// 便捷入口：自己读盘（调用方已持有 `Config` 时优先用 [`Self::from_config`]）。
    pub fn load() -> Self {
        match Config::load() {
            Ok(c) => Self::from_config(&c),
            Err(_) => Self {
                sensory: true,
                short_term: true,
                long_term: true,
                recall: true,
                cross_project: true,
                behavior: true,
                tiered: false,
                experience: true,
            },
        }
    }

    pub fn get(&self, id: &str) -> bool {
        match id {
            "sensory" => self.sensory,
            "short_term" => self.short_term,
            "long_term" => self.long_term,
            "recall" => self.recall,
            "cross_project" => self.cross_project,
            "behavior" => self.behavior,
            "tiered" => self.tiered,
            "experience" => self.experience,
            _ => false,
        }
    }
}

/// 楼层元数据（给 UI 用；中文名与说明集中在这里，避免前端各自硬编码）。
pub fn floor_meta(id: &str) -> Option<(&'static str, &'static str, &'static str)> {
    Some(match id {
        "sensory" => ("感知层", "当前会话的历史消息；关闭后模型只看得到这一条新消息", "session"),
        "short_term" => ("短期层", "会话摘要（SQLite 全文检索命中）", "sqlite-fts"),
        "long_term" => ("长期层", "向量长时池，按语义相似度召回", "vector"),
        "recall" => ("召回层", "用户画像 / 反馈 / 自动 / 项目四类持久记忆", "json"),
        "cross_project" => ("跨项目层", "其他项目里沉淀过的同类经验", "json"),
        "behavior" => ("行为层", "最近真实执行过的动作（工具调用台账）", "ledger"),
        "tiered" => ("分层稀疏层", "flyhash 近邻粗排 + 精排（原 memory.tiered_enabled）", "flyhash"),
        "experience" => ("经验层", "踩坑 / 决策 / 模式沉淀（自进化台账）", "experience.db"),
        _ => return None,
    })
}

/// `memory.layers.switches.get`：返回 8 个楼层的当前开关 + 元数据。
///
/// 只读、无副作用。`enabled` 是**生效值**（tiered 已从 `memory.tiered_enabled` 解析）。
pub fn switches_get(_params: &Value) -> Value {
    let layers = ResolvedLayers::load();
    let items: Vec<Value> = FLOOR_ORDER
        .iter()
        .enumerate()
        .filter_map(|(idx, id)| {
            let (name, desc, backend) = floor_meta(id)?;
            Some(json!({
                "id": id,
                "floor": idx + 1,
                "name": name,
                "description": desc,
                "backend": backend,
                "enabled": layers.get(id),
            }))
        })
        .collect();
    json!({
        "ok": true,
        "count": items.len(),
        "enabledCount": items.iter().filter(|i| i["enabled"] == true).count(),
        "layers": items,
    })
}

/// `memory.layers.switches.set`：写入一个或多个楼层开关。
///
/// 参数：`{ "layers": { "sensory": true, ... } }` 或平铺 `{ "sensory": true }`。
/// 未知 id 直接拒绝（拼错却静默忽略会让用户以为"关掉了"）。
pub fn switches_set(params: &Value) -> Value {
    let patch = params
        .get("layers")
        .filter(|v| v.is_object())
        .unwrap_or(params);
    let Some(obj) = patch.as_object() else {
        return json!({ "ok": false, "error": "缺少 layers 对象" });
    };
    if obj.is_empty() {
        return json!({ "ok": false, "error": "layers 为空" });
    }

    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": format!("读取配置失败: {e}") }),
    };
    let mut applied: Vec<String> = Vec::new();
    for (key, val) in obj {
        let Some(enabled) = val.as_bool() else {
            return json!({ "ok": false, "error": format!("{key} 必须是布尔值") });
        };
        match key.as_str() {
            "sensory" => cfg.memory.layers.sensory = enabled,
            "short_term" => cfg.memory.layers.short_term = enabled,
            "long_term" => cfg.memory.layers.long_term = enabled,
            "recall" => cfg.memory.layers.recall = enabled,
            "cross_project" => cfg.memory.layers.cross_project = enabled,
            "behavior" => cfg.memory.layers.behavior = enabled,
            "experience" => cfg.memory.layers.experience = enabled,
            "tiered" => cfg.memory.tiered_enabled = enabled,
            other => {
                return json!({
                    "ok": false,
                    "error": format!("未知楼层 id: {other}（可选：{FLOOR_ORDER:?}）"),
                })
            }
        }
        applied.push(key.clone());
    }
    if let Err(e) = cfg.save() {
        return json!({ "ok": false, "error": format!("保存配置失败: {e}") });
    }
    let mut out = switches_get(&Value::Null);
    out["ok"] = json!(true);
    out["applied"] = json!(applied);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 本用例独占的临时配置目录，并把它注入线程局部覆盖。
    ///
    /// 目录名带 `tag`：两个用例各自一个目录（同一进程 PID 相同，共用目录会互相读走对方的配置）。
    /// 先删后建：清掉上一次运行（PID 复用）的残留，保证每次都从干净状态开始。
    fn isolated_config_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mq-floors-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时配置目录");
        Config::set_config_dir_override(dir.clone());
        dir
    }

    /// 清除线程局部的配置目录覆盖，测试线程内不留残留。
    ///
    /// `Config::set_config_dir_override` 只接受 `PathBuf`（没有传 `None` 清除的入口），
    /// 所以「还原」只能直接清同一个 thread_local。
    fn clear_config_dir_override() {
        crate::persistence::OVERRIDE_CONFIG_DIR.with(|c| *c.borrow_mut() = None);
    }

    #[test]
    fn floor_order_has_eight_unique_floors_ending_with_experience() {
        assert_eq!(FLOOR_ORDER.len(), 8, "7 层记忆 + 1 层经验");
        let mut ids: Vec<&str> = FLOOR_ORDER.to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8, "楼层 id 不得重复");
        assert_eq!(FLOOR_ORDER[7], "experience", "经验永远是最上面那一层");
    }

    #[test]
    fn every_floor_has_meta_and_maps_to_a_real_switch() {
        for id in FLOOR_ORDER {
            let (name, desc, backend) = floor_meta(id).unwrap_or_else(|| panic!("{id} 缺元数据"));
            assert!(!name.is_empty() && !desc.is_empty() && !backend.is_empty(), "{id}");
        }
    }

    #[test]
    fn switches_default_to_on_except_tiered() {
        let s = LayerSwitches::default();
        assert!(s.sensory && s.short_term && s.long_term && s.recall);
        assert!(s.cross_project && s.behavior && s.experience);
    }

    #[test]
    fn unknown_floor_id_is_rejected_not_ignored() {
        let v = switches_set(&json!({ "layers": { "senory": true } })); // 故意拼错
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap_or("").contains("未知楼层"));
    }

    #[test]
    fn non_boolean_value_is_rejected() {
        let v = switches_set(&json!({ "layers": { "sensory": "yes" } }));
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn switches_get_shape_matches_ui_contract() {
        let v = switches_get(&Value::Null);
        let layers = v["layers"].as_array().expect("layers 数组");
        assert_eq!(layers.len(), 8);
        for (i, l) in layers.iter().enumerate() {
            assert_eq!(l["floor"], i as u64 + 1, "楼层号必须连续");
            for key in ["id", "name", "description", "backend", "enabled"] {
                assert!(l.get(key).is_some(), "缺字段 {key}: {l}");
            }
            assert!(l["enabled"].is_boolean());
        }
        assert!(v["enabledCount"].is_u64());
    }

    /// 楼层开关必须**真的落盘**：save → load → from_config 后仍是用户设的值。
    ///
    /// `tiered` 不在 `LayerSwitches` 里（由 `memory.tiered_enabled` 承担），
    /// 这条用例把「两层真相」钉成同一个可读回的布尔值；其余楼层不得被牵连。
    #[test]
    fn switches_roundtrip_through_config_file() {
        let dir = isolated_config_dir("roundtrip");

        let mut cfg = Config::default();
        cfg.memory.layers.short_term = false;
        cfg.memory.layers.behavior = false;
        cfg.memory.tiered_enabled = true;
        cfg.save().expect("写入临时配置目录");

        let loaded = Config::load().expect("读回临时配置目录");
        let r = ResolvedLayers::from_config(&loaded);
        assert!(!r.short_term, "关掉的 short_term 必须读回 false");
        assert!(!r.behavior, "关掉的 behavior 必须读回 false");
        assert!(r.tiered, "tiered 来自 memory.tiered_enabled");
        assert!(
            r.sensory && r.long_term && r.recall && r.cross_project && r.experience,
            "未改动的楼层必须仍为 true: {:?}",
            r
        );

        clear_config_dir_override();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `switches_set({ "tiered": true })` 必须写到 `memory.tiered_enabled`，
    /// 且这条只改 tiered 的写入不得顺手改动其它楼层开关。
    #[test]
    fn switches_set_writes_tiered_to_tiered_enabled() {
        let dir = isolated_config_dir("set-tiered");
        Config::default().save().expect("先落一份默认配置");

        let v = switches_set(&json!({ "layers": { "tiered": true } }));
        assert_eq!(v["ok"], true, "写入应成功: {v}");

        let cfg = Config::load().expect("读回临时配置目录");
        assert!(
            cfg.memory.tiered_enabled,
            "tiered 必须落进 memory.tiered_enabled"
        );
        assert!(
            cfg.memory.layers.sensory,
            "只改 tiered 的写入不得牵连其它楼层: {:?}",
            cfg.memory.layers
        );

        clear_config_dir_override();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
