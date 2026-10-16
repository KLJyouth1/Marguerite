//! 插件装配状态（P0 S4b）：boot 时把 patch 真装配进 `PluginSlot`，并经 `plugins.status` 可见。
//!
//! 历史缺陷：S2/S3/S4 已落地 `PluginSlot` / `PatchLayer` / `apply_layers`，但**没有任何代码在
//! boot 时实例化 `PluginSlot` 并装配 patch** —— 16 份声明式清单从未被装配，且完全静默。
//!
//! 本模块补齐「装配」这一缺口：
//! - `assemble()`：从 `plugin_manifest_dir()` 装载全部 manifest + 从 `patches_dir()` 装载全部
//!   patch，调用 `apply_layers` 真装配进 `PluginSlot`；
//! - `warm_assembly()`：幂等预热（OnceLock），boot 跑一次、结果全程复用，重启生效；
//! - `status_json()`：把装配结果按 `applied/skipped/errors` 三分类 + 清单装配状态暴露给 RPC。
//!
//! 不静默纪律（D1）：目录缺失/读不到/解析失败一律 `tracing::warn!` 带路径，绝不 `if let Ok` 吞掉。

use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::gateway::market::plugin_manifest_dir;
use crate::plugins::manifest::{Activation, PluginManifest};
use crate::plugins::patch::{self, patches_dir, PatchLayer, PatchReport};
use crate::plugins::slot::PluginSlot;

/// 单份清单的装配状态（供 `plugins.status` 的 `manifests` 数组）。
pub struct ManifestAssemblyStatus {
    pub name: String,
    pub version: String,
    /// `"declarative-only"`（无 entry 无权限）或 `"native"`（带原生入口）
    pub mode: String,
    /// 原生入口（声明式清单为 `None`）
    pub entry: Option<String>,
    /// `"restart"` / `"hot"`
    pub activation: String,
    /// 是否真装配进 `PluginSlot`（有 `source == name` 的槽位）
    pub assembled: bool,
    /// 该清单装配出的槽位名列表
    pub slots: Vec<String>,
}

/// 一次完整装配的快照。
pub struct PluginAssembly {
    pub slot: PluginSlot,
    pub manifests: Vec<PluginManifest>,
    pub patches: Vec<PatchLayer>,
    pub report: PatchReport,
    pub manifest_status: Vec<ManifestAssemblyStatus>,
}

/// 装配快照（boot 跑一次，进程内复用；重启生效）。
static ASSEMBLY: OnceLock<PluginAssembly> = OnceLock::new();

/// 幂等预热：首次调用执行 [`assemble`]，之后复用同一份快照。
pub fn warm_assembly() -> &'static PluginAssembly {
    ASSEMBLY.get_or_init(assemble)
}

/// 执行一次完整装配（读清单 + 读 patch + `apply_layers`）。
///
/// 任一步失败均**响亮 warn 但降级**，不让 boot 因「插件装配」而终止：
/// - 清单装载失败 → 不含任何 manifest（warn 带路径）；
/// - patch 装载失败 → 不含任何 patch（warn）；
/// - `apply_layers` 失败（环/缺失依赖）→ report 置空（warn）。
pub fn assemble() -> PluginAssembly {
    let manifests = match PluginManifest::load_dir(
        &plugin_manifest_dir().unwrap_or_else(|| PathBuf::from("plugins/manifests")),
    ) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error=%e, "插件清单装载失败：装配将不含任何 manifest");
            Vec::new()
        }
    };

    let layers = match patches_dir().and_then(|d| PatchLayer::load_dir(&d).ok()) {
        Some(l) => l,
        None => {
            tracing::warn!("patch 目录未解析/装载失败：装配将不含任何 patch");
            Vec::new()
        }
    };

    let mut slot = PluginSlot::new();
    let report = match patch::apply_layers(&mut slot, &manifests, &layers) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error=%e, "patch 装配失败（环/缺失依赖）");
            PatchReport::default()
        }
    };

    let manifest_status = manifests
        .iter()
        .map(|m| {
            let slots: Vec<String> = slot
                .list_all()
                .iter()
                .filter(|s| s.source.as_deref() == Some(m.name.as_str()))
                .map(|s| s.name.clone())
                .collect();
            let assembled = slot
                .list_all()
                .iter()
                .any(|s| s.source.as_deref() == Some(m.name.as_str()));
            ManifestAssemblyStatus {
                name: m.name.clone(),
                version: m.version.clone(),
                mode: if m.is_declarative_only() {
                    "declarative-only".to_string()
                } else {
                    "native".to_string()
                },
                entry: m.entry.clone(),
                activation: match m.activation {
                    Activation::Restart => "restart".to_string(),
                    Activation::Hot => "hot".to_string(),
                },
                assembled,
                slots,
            }
        })
        .collect();

    PluginAssembly {
        slot,
        manifests,
        patches: layers,
        report,
        manifest_status,
    }
}

/// `plugins.status` 的响应负载（如实分类 + 清单装配状态）。
pub fn status_json() -> Value {
    let asm = warm_assembly();

    let manifests: Vec<Value> = asm
        .manifest_status
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "version": m.version,
                "mode": m.mode,
                "entry": m.entry,
                "activation": m.activation,
                "assembled": m.assembled,
                "slots": m.slots,
            })
        })
        .collect();

    // 仅含可用项（list() 已排除停用项）；kind 用 serde 的 snake_case 名序列化。
    let slots: Vec<Value> = asm
        .slot
        .list()
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "kind": s.kind,
                "source": s.source,
                "declared_slot": s.declared_slot,
                "order": s.order,
                "disabled": false,
            })
        })
        .collect();

    let report = json!({
        "applied": asm.report.applied,
        "skipped": asm.report.skipped,
        "errors": asm.report.errors,
    });

    let summary = json!({
        "manifests_total": asm.manifests.len(),
        "assembled_count": asm.manifest_status.iter().filter(|m| m.assembled).count(),
        "patches_total": asm.patches.len(),
        "applied": asm.report.applied.len(),
        "skipped": asm.report.skipped.len(),
        "errors": asm.report.errors.len(),
    });

    json!({
        "manifests": manifests,
        "slots": slots,
        "report": report,
        "summary": summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 唯一临时目录（不依赖 tempfile crate：pid + 纳秒时间戳）。
    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("mg-asm-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("创建临时目录 {} 失败: {e}", dir.display()));
        dir
    }

    /// 装配需要**结构体**：坏清单不得静默丢弃，整体失败并带路径。
    #[test]
    fn manifest_load_dir_bails_on_bad_json() {
        let dir = unique_tmp_dir("manifest-bad");
        std::fs::write(
            dir.join("good.plugin.json"),
            r#"{"name":"good","version":"1.0.0","description":"g"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.plugin.json"), "{ not json").unwrap();

        let err = PluginManifest::load_dir(&dir).expect_err("坏清单必须整体失败");
        let text = format!("{err:#}");
        assert!(
            text.contains("broken.plugin.json"),
            "错误信息须含文件名，实际: {text}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 装配链路三分类如实：insert → applied、set → skipped（无落点），且 insert 真落进槽位。
    #[test]
    fn assemble_classifies_insert_and_set_truthfully() {
        let mdir = unique_tmp_dir("asm-manifest");
        std::fs::write(
            mdir.join("plug-tmp.plugin.json"),
            r#"{"name":"plug-tmp","version":"0.1.0","description":"tmp"}"#,
        )
        .unwrap();
        let manifests = PluginManifest::load_dir(&mdir).unwrap();
        assert_eq!(manifests.len(), 1, "load_dir 应返回结构体向量");

        let pdir = unique_tmp_dir("asm-patch");
        let insert_id = "plug-tmp.analyze";
        std::fs::write(
            pdir.join("plug-tmp.patch.json"),
            format!(
                r#"{{
                    "schema": "marguerite.patch/v1",
                    "target": "plug-tmp",
                    "layer": 1,
                    "profile": ["desktop"],
                    "ops": [
                        {{"op": "insert", "slot": "tools", "id": "{insert_id}", "order": 10, "label": "临时分析"}},
                        {{"op": "set", "key": "ui.theme", "value": "dark"}}
                    ]
                }}"#
            ),
        )
        .unwrap();
        let layers = PatchLayer::load_dir(&pdir).unwrap();

        let mut slot = PluginSlot::new();
        let report = patch::apply_layers(&mut slot, &manifests, &layers).unwrap();
        assert_eq!(report.applied.len(), 1, "applied={:?}", report.applied);
        assert_eq!(report.skipped.len(), 1, "skipped={:?}", report.skipped);
        assert!(slot.contains(insert_id), "insert 必须真注册进槽位");

        std::fs::remove_dir_all(&mdir).ok();
        std::fs::remove_dir_all(&pdir).ok();
    }
}
