//! DSH 插件 → 本工作台适配映射（单一事实源）。
//!
//! 背景：`plugins/manifests/*.plugin.json` 是从 DSH（DeepSeek Harness）生态二创的
//! **声明式清单**，全部没有 `entry`，不能像 DLL/WASM 插件那样被动态加载。这不等于
//! 功能不存在——多数能力已在本工作台以原生 RPC / 原生 UI 的形式落地，只是名字不同。
//!
//! 本表把「清单声明」逐条映射到**真实存在的** RPC 与页面，供插件页展示，并让
//! 「已适配 / 部分适配 / 仅声明」成为可审计事实，而不是一句口头承诺。
//!
//! 诚实边界：
//! - `partial`：核心功能有等价实现，但**没有同名 RPC** 或缺少某个子能力；
//! - `declarative`：仅清单声明，当前工作台没有等价实现（本表目前为空，出现即如实展示）；
//! - RPC 名必须存在于 `gateway/protocol.rs` 的 `SUPPORTED_METHODS`，由单测守护。

use serde_json::{json, Value};

/// 适配状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptationStatus {
    /// 完整等价：清单声明的能力都有可直接调用的原生 RPC / 页面
    Native,
    /// 部分等价：核心能力已有原生实现，但仍有子能力没有同名 RPC
    Partial,
    /// 仅声明：没有等价实现
    Declarative,
}

impl AdaptationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Partial => "partial",
            Self::Declarative => "declarative",
        }
    }
}

/// 一个插件的适配记录。
#[derive(Debug, Clone, Copy)]
pub struct Adaptation {
    pub plugin: &'static str,
    pub status: AdaptationStatus,
    /// 可直接调用的工作台 RPC（真实存在）
    pub rpc: &'static [&'static str],
    /// 对应的页面路由（真实存在）
    pub ui: &'static [&'static str],
    /// 一句话说明差异 / 边界
    pub note: &'static str,
}

const ADAPTATIONS: &[Adaptation] = &[
    Adaptation {
        plugin: "dsh-harness-core",
        status: AdaptationStatus::Native,
        rpc: &[
            "plugins.status",
            "plugins.coverage",
            "harness.ledger.status",
            "harness.ledger.query",
        ],
        ui: &["/plugins", "/harness"],
        note: "运行时清单 / 装配状态 / 运行台账由工作台插件系统原生提供",
    },
    Adaptation {
        plugin: "dsh-better-sidebar",
        status: AdaptationStatus::Partial,
        rpc: &["sessions.list", "sandbox.path.list"],
        ui: &["/editor", "/browser-cockpit", "/chat"],
        note: "三栏工作台由编辑器 / 浏览器驾驶舱 / 附件面板等价提供；无 dsh.sidebar.* 同名 RPC",
    },
    Adaptation {
        plugin: "dsh-modlens",
        status: AdaptationStatus::Native,
        rpc: &["modlens.analyze"],
        ui: &["/chat"],
        note: "PreToolUse 钩子已接入：图片附件自动产出 OCR / 版面 / 结构化 JSON",
    },
    Adaptation {
        plugin: "dsh-market",
        status: AdaptationStatus::Native,
        rpc: &[
            "market.list",
            "market.search",
            "market.install",
            "market.upgrade",
            "market.import",
            "market.export",
            "market.remote.search",
            "market.install_url",
        ],
        ui: &["/plugins"],
        note: "本地清单 + 远程索引 + 直链安装全部可用",
    },
    Adaptation {
        plugin: "dsh-router-standard",
        status: AdaptationStatus::Native,
        rpc: &[
            "router.decision",
            "router.selflearning.status",
            "models.routing.get",
            "models.routing.set",
            "fleet.route",
        ],
        ui: &["/fleet", "/engine"],
        note: "意图×复杂度路由与 fleet 角色路由已原生实现",
    },
    Adaptation {
        plugin: "dsh-usage",
        status: AdaptationStatus::Native,
        rpc: &[
            "usage.status",
            "usage.query",
            "usage.daily",
            "usage.cost",
            "usage.taskCost",
            "usage.budget.get",
            "usage.budget.set",
        ],
        ui: &["/usage"],
        note: "日/周/月聚合与预算阈值已原生实现；余额查询依赖上游账户系统（未接入时不显示数字）",
    },
    Adaptation {
        plugin: "dsh-pet-whale",
        status: AdaptationStatus::Native,
        rpc: &[
            "pet.status",
            "pet.list",
            "pet.setSkin",
            "pet.setMood",
            "pet.triggerAction",
            "pet.action",
        ],
        ui: &["/pet"],
        note: "桌宠状态 / 互动 / 皮肤已原生实现",
    },
    Adaptation {
        plugin: "dsh-minigames",
        status: AdaptationStatus::Native,
        rpc: &["games.list", "games.play"],
        ui: &["/games"],
        note: "游戏大厅与三款 Rust 引擎游戏已原生实现（其余条目按需扩展）",
    },
    Adaptation {
        plugin: "dsh-compaction",
        status: AdaptationStatus::Native,
        rpc: &["context.status", "context.compact", "usage.query"],
        ui: &["/chat", "/harness"],
        note: "上下文压缩与 token 计量由运行时原生实现",
    },
    Adaptation {
        plugin: "dsh-goal-round-driver",
        status: AdaptationStatus::Native,
        rpc: &[
            "goal.create",
            "goal.get",
            "goal.edit",
            "goal.pause",
            "goal.resume",
            "goal.complete",
            "goal.blocked",
            "goal.begin_round",
        ],
        ui: &["/task"],
        note: "目标生命周期与回合预留已原生实现",
    },
    Adaptation {
        plugin: "dsh-plan-mode",
        status: AdaptationStatus::Native,
        rpc: &[
            "plans.list",
            "plans.create",
            "plans.run",
            "plans.setMode",
            "mode.get",
            "mode.switch",
        ],
        ui: &["/task", "/chat"],
        note: "计划模式与运行模式切换已原生实现",
    },
    Adaptation {
        plugin: "dsh-presets",
        status: AdaptationStatus::Partial,
        rpc: &[
            "agents.list",
            "agents.get",
            "agents.create",
            "agents.update",
            "agents.delete",
        ],
        ui: &["/expert-center", "/agents"],
        note: "Agent 预设 CRUD 已原生；dsh.anchor.* 内部锚定策略未暴露同名 RPC",
    },
    Adaptation {
        plugin: "dsh-schedule",
        status: AdaptationStatus::Native,
        rpc: &[
            "cron.list",
            "cron.create",
            "cron.update",
            "cron.remove",
            "cron.run",
            "cron.runs",
        ],
        ui: &["/cron"],
        note: "定时任务（after/at/every）已原生实现",
    },
    Adaptation {
        plugin: "dsh-session-projection",
        status: AdaptationStatus::Partial,
        rpc: &[
            "sessions.get",
            "sessions.messages.snapshot",
            "sessions.insights",
        ],
        ui: &["/sessions", "/harness"],
        note: "ProjectionStore 已作为内部折叠实现并落 SQLite；无 projection.* 公共 RPC",
    },
    Adaptation {
        plugin: "dsh-skill-registry",
        status: AdaptationStatus::Native,
        rpc: &[
            "skills.list",
            "skills.get",
            "skills.search",
            "skills.reload",
            "skills.install",
            "skills.create",
            "skills.coverage",
        ],
        ui: &["/skills"],
        note: "技能注册 / 搜索 / 安装 / 覆盖统计已原生实现",
    },
    Adaptation {
        plugin: "dsh-workflow-ralph",
        status: AdaptationStatus::Partial,
        rpc: &["plans.run", "plans.parallel.run", "orchestration.status"],
        ui: &["/task"],
        note: "Ralph 子代理循环已移植为内部工作流；无 workflow.* 公共 RPC",
    },
    Adaptation {
        plugin: "echo-mcp",
        status: AdaptationStatus::Native,
        rpc: &[
            "plugins.mcp.start",
            "plugins.mcp.stop",
            "plugins.mcp.list",
            "plugins.mcp.call",
        ],
        ui: &["/mcp", "/plugins"],
        note: "MCP 子进程动态加载示例：真实 spawn + tools/list + call",
    },
];

/// 查一个插件的适配记录（未登记返回 None：不假装已适配）。
pub fn for_plugin(name: &str) -> Option<&'static Adaptation> {
    ADAPTATIONS.iter().find(|a| a.plugin == name)
}

/// 全部适配记录。
pub fn all() -> &'static [Adaptation] {
    ADAPTATIONS
}

/// 适配摘要（列表页逐行展示）。
pub fn summary_for(name: &str) -> Value {
    match for_plugin(name) {
        Some(a) => json!({
            "status": a.status.as_str(),
            "rpc": a.rpc,
            "ui": a.ui,
            "note": a.note,
        }),
        None => Value::Null,
    }
}

/// 适配详情（详情抽屉 / 覆盖率接口）。
pub fn detail_for(name: &str) -> Value {
    match for_plugin(name) {
        Some(a) => json!({
            "plugin": a.plugin,
            "status": a.status.as_str(),
            "rpc": a.rpc,
            "ui": a.ui,
            "note": a.note,
        }),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared_methods() -> std::collections::HashSet<String> {
        let src = include_str!("../gateway/protocol.rs");
        let start = src.find("SUPPORTED_METHODS").expect("SUPPORTED_METHODS 缺失");
        let open = src[start..].find('[').expect("数组起点缺失") + start;
        let close = src[open..].find("];").expect("数组终点缺失") + open;
        src[open..close]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter_map(|line| {
                let start = line.find('"')?;
                let end = line[start + 1..].find('"')?;
                Some(line[start + 1..start + 1 + end].to_string())
            })
            .collect()
    }

    #[test]
    fn every_mapped_rpc_is_declared() {
        let declared = declared_methods();
        for a in ADAPTATIONS {
            for rpc in a.rpc {
                assert!(
                    declared.contains(*rpc),
                    "插件 {} 映射的 RPC `{rpc}` 未在 SUPPORTED_METHODS 声明",
                    a.plugin
                );
            }
        }
    }

    #[test]
    fn every_mapped_ui_route_exists() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../webui/src/router/sharedRoutes.ts");
        let Ok(src) = std::fs::read_to_string(&path) else {
            return; // 只装 core 的构建环境没有 webui：不误报
        };
        let mut missing = Vec::new();
        for a in ADAPTATIONS {
            for ui in a.ui {
                let needle = format!("path: '{ui}'");
                if !src.contains(&needle) {
                    missing.push(format!("{} → {ui}", a.plugin));
                }
            }
        }
        assert!(missing.is_empty(), "适配映射指向不存在的页面路由: {missing:?}");
    }

    #[test]
    fn statuses_are_known_and_unmapped_is_null() {
        for a in ADAPTATIONS {
            assert!(matches!(
                a.status,
                AdaptationStatus::Native | AdaptationStatus::Partial | AdaptationStatus::Declarative
            ));
            assert!(!a.note.trim().is_empty(), "{} 缺少边界说明", a.plugin);
        }
        assert!(for_plugin("not-a-plugin").is_none());
        assert_eq!(summary_for("not-a-plugin"), Value::Null);
    }

    #[test]
    fn covers_all_bundled_dsh_manifests() {
        // 目录在仓库根 `plugins/manifests`；测试运行在 core/ 下。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../plugins/manifests");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return; // 打包环境下清单可能不随源码存在：不误报
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".plugin.json") {
                continue;
            }
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
            let Some(plugin) = value.get("name").and_then(Value::as_str) else { continue };
            assert!(
                for_plugin(plugin).is_some(),
                "插件 `{plugin}` 未登记适配映射（插件页会显示为「仅声明」）"
            );
        }
    }
}
