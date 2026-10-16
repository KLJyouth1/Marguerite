//! 补丁层（patch layer）：把「**安装**」与「**装配**」解耦。
//!
//! 现状缺陷：`plugins/manifests/*.plugin.json` 只被当**数据**读（市场列表 / 能力对账），
//! 没有任何代码把一份清单**装配**进运行期组合 —— 即「装了不动」在本项目是**完全静默**的。
//!
//! 本模块沿用 DSH `cordis.patch.yml` 的形态：一个 patch 文件**声明**如何把某个已安装插件
//! 装配进组合（往哪个槽位插什么、覆盖哪些配置、停用哪些条目）。
//!
//! ## 切片边界
//!
//! **S2（已落地）**：声明与只读装载 —— 解析（`load_dir`）→ 校验（`validate`）→
//! 排序（`order_layers`）→ 摘要（`summarize`）。
//!
//! **S4（本切片）**：新增 `apply_layers` —— 把排序后的 patch **真正装配**进
//! `PluginSlot`。装配结果按 `PatchReport`（applied / skipped / errors）**如实分类**：
//! 不能落地的 op 记 `skipped`、失败记 `errors`，**绝不假装成功**。
//! patch 声明的槽位统一使用 `UnimplementedExecutor` —— 调用即显式 `Err`。
//!
//! ## 诊断类型的归属
//!
//! `PatchDiagnostic` 是本模块**自有**类型，刻意不复用 / 不修改 `manifest.rs` 的类型 ——
//! 两套诊断在 S3 才统一，此处保持解耦，避免提前耦合引发回归。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::manifest::PluginManifest;
use super::slot::{PluginSlot, SlotDescriptor, SlotExecutor, SlotKind, SlotResult};

/// patch schema 版本号：`PatchLayer.schema` 必须恒等于此值。
pub const PATCH_SCHEMA: &str = "marguerite.patch/v1";

/// `profile` 缺省值：桌面端。
fn default_profile() -> Vec<String> {
    vec!["desktop".to_string()]
}

/// 补丁层：把「安装」与「装配」解耦 —— 一个 patch 文件声明如何把某插件装配进组合。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchLayer {
    /// schema 标识，必须 == `PATCH_SCHEMA`（`"marguerite.patch/v1"`）
    pub schema: String,
    /// 目标插件名，必须命中 `plugins/manifests/<name>.plugin.json` 的 `name`
    pub target: String,
    /// 装载层级，小者先
    #[serde(default)]
    pub layer: i32,
    /// 生效 profile 列表（缺省 `["desktop"]`）
    #[serde(default = "default_profile")]
    pub profile: Vec<String>,
    /// 依赖的其它 patch target
    #[serde(default)]
    pub requires: Vec<String>,
    /// 装配操作序列
    #[serde(default)]
    pub ops: Vec<PatchOp>,
}

/// 单条装配操作。
///
/// 序列化形态为内部标签：`{"op": "insert", "slot": "...", "id": "..."}`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    /// 往某槽位插入一个条目
    Insert {
        /// 目标槽位名
        slot: String,
        /// 条目 id
        id: String,
        /// 同槽位内排序权重（小者先）
        #[serde(default)]
        order: i32,
        /// 可选显示标签
        #[serde(default)]
        label: Option<String>,
    },
    /// 覆盖某个配置键
    Set {
        /// 配置键
        key: String,
        /// 配置值（任意 JSON）
        value: serde_json::Value,
    },
    /// 停用某个已装配条目
    Disable {
        /// 条目 id
        id: String,
    },
}

/// 单条诊断（本模块自有，S3 才与 `manifest.rs` 统一）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchDiagnostic {
    /// 机器可读诊断码，如 `patch.bad_schema`
    pub code: &'static str,
    /// 产生该诊断的 patch target
    pub target: String,
    /// 人类可读说明
    pub message: String,
}

impl PatchDiagnostic {
    fn new(code: &'static str, target: &str, message: String) -> Self {
        Self {
            code,
            target: target.to_string(),
            message,
        }
    }
}

/// 装载摘要（供启动日志）。
#[derive(Debug, Clone)]
pub struct PatchSummary {
    /// patch 总数
    pub total: usize,
    /// 按 profile 计数（profile 名 → 该 profile 下的 patch 数），按 profile 名有序
    pub by_profile: Vec<(String, usize)>,
    /// 汇总的全部诊断
    pub diagnostics: Vec<PatchDiagnostic>,
}

impl PatchLayer {
    /// 扫目录下全部 `*.patch.json` 并解析。
    ///
    /// - 按**文件路径**排序，保证返回顺序稳定；
    /// - **解析失败必须带文件名报错**（绝不静默丢弃 —— 静默丢弃会表现为「份数莫名少一条」，
    ///   是比 FAIL 更隐蔽的失效）。
    pub fn load_dir(dir: &Path) -> Result<Vec<PatchLayer>> {
        let rd = std::fs::read_dir(dir)
            .with_context(|| format!("读取 patch 目录失败: {}", dir.display()))?;

        let mut files: Vec<PathBuf> = Vec::new();
        for entry in rd {
            let entry =
                entry.with_context(|| format!("遍历 patch 目录失败: {}", dir.display()))?;
            let path = entry.path();
            let is_patch = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".patch.json"))
                .unwrap_or(false);
            if is_patch {
                files.push(path);
            }
        }
        // 稳定性：read_dir 的顺序由文件系统决定，排序后才与平台无关。
        files.sort();

        let mut out = Vec::with_capacity(files.len());
        for path in files {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("读取 patch 文件失败: {}", path.display()))?;
            let layer: PatchLayer = serde_json::from_str(&text)
                .with_context(|| format!("解析 patch 文件失败: {}", path.display()))?;
            out.push(layer);
        }
        Ok(out)
    }

    /// 单层校验：返回全部诊断（不短路，一次报全）。
    ///
    /// 覆盖：
    /// ① `schema` 不是 `PATCH_SCHEMA` → `patch.bad_schema`；
    /// ② `target` 不在已知 manifest 里 → `patch.orphan_target`；
    /// ③ `Insert` 的 `id` 为空 → `patch.empty_id`；
    /// ④ `requires` 指向不存在的 target → `patch.missing_requirement`。
    ///
    /// ④ 的参照集合：本模块的 `target` 必须命中 manifest 的 `name`，故 `requires`
    /// 里的每一项也应能命中已知 manifest 名；命中不了即为悬空依赖。`validate` 只有
    /// 单层视图（拿不到同批其它 patch），用 `known_manifests` 是此处唯一可用的参照集。
    pub fn validate(&self, known_manifests: &[PluginManifest]) -> Vec<PatchDiagnostic> {
        let mut diags = Vec::new();

        // ① schema
        if self.schema != PATCH_SCHEMA {
            diags.push(PatchDiagnostic::new(
                "patch.bad_schema",
                &self.target,
                format!(
                    "schema `{}` 不合法，应为 `{}`",
                    self.schema, PATCH_SCHEMA
                ),
            ));
        }

        let known: HashSet<&str> = known_manifests.iter().map(|m| m.name.as_str()).collect();

        // ② target 悬空
        if !known.contains(self.target.as_str()) {
            diags.push(PatchDiagnostic::new(
                "patch.orphan_target",
                &self.target,
                format!(
                    "target `{}` 未命中任何已知 manifest（{} 份已装载清单）",
                    self.target,
                    known_manifests.len()
                ),
            ));
        }

        // ③ Insert 空 id
        for op in &self.ops {
            if let PatchOp::Insert { id, slot, .. } = op {
                if id.trim().is_empty() {
                    diags.push(PatchDiagnostic::new(
                        "patch.empty_id",
                        &self.target,
                        format!("Insert 到槽位 `{slot}` 的 id 为空"),
                    ));
                }
            }
        }

        // ④ requires 悬空
        for r in &self.requires {
            if !known.contains(r.as_str()) {
                diags.push(PatchDiagnostic::new(
                    "patch.missing_requirement",
                    &self.target,
                    format!("requires 依赖的 target `{r}` 不存在"),
                ));
            }
        }

        diags
    }
}

/// 装载顺序：`layer` 升序 → 同层按 `target` 字典序 → `requires` 拓扑排序。
///
/// 实现为「基序排序 + 逐个挑依赖已就绪的最前者」，因此：
/// - **确定性**：不依赖任何 `HashMap` 迭代序；
/// - **稳定**：无依赖的 patch 严格保持 `layer` / `target` 基序；
/// - `requires` 缺失或有环 → `Err`（拒绝整批，不静默降级）。
pub fn order_layers(layers: &[PatchLayer]) -> Result<Vec<PatchLayer>> {
    // 1) 基序：layer 升序 → target 字典序（两者皆确定，无哈希参与）
    let mut remaining: Vec<usize> = (0..layers.len()).collect();
    remaining.sort_by(|&a, &b| {
        layers[a]
            .layer
            .cmp(&layers[b].layer)
            .then_with(|| layers[a].target.cmp(&layers[b].target))
    });

    // 2) 缺失依赖：响亮失败，不静默忽略
    let present: HashSet<&str> = layers.iter().map(|l| l.target.as_str()).collect();
    for l in layers {
        for r in &l.requires {
            if !present.contains(r.as_str()) {
                anyhow::bail!(
                    "patch `{}` 依赖的 target `{}` 不存在（缺失依赖）",
                    l.target,
                    r
                );
            }
        }
    }

    // 3) 逐个挑「基序最前且依赖已满足」的一个；一轮挑不出 → 环
    let mut done: HashSet<String> = HashSet::new();
    let mut out: Vec<PatchLayer> = Vec::with_capacity(layers.len());
    while !remaining.is_empty() {
        let picked = remaining
            .iter()
            .position(|&i| layers[i].requires.iter().all(|r| done.contains(r)));
        match picked {
            Some(pos) => {
                let i = remaining.remove(pos);
                done.insert(layers[i].target.clone());
                out.push(layers[i].clone());
            }
            None => {
                let stuck: Vec<String> = remaining
                    .iter()
                    .map(|&i| layers[i].target.clone())
                    .collect();
                anyhow::bail!("patch 依赖存在环: {stuck:?}");
            }
        }
    }
    Ok(out)
}

/// 汇总摘要：总数 + 按 profile 计数 + 全部 diagnostics。
///
/// `by_profile` 用 `BTreeMap` 聚合，保证输出按 profile 名有序（不依赖哈希迭代序）。
pub fn summarize(layers: &[PatchLayer], known: &[PluginManifest]) -> PatchSummary {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for l in layers {
        for p in &l.profile {
            *counts.entry(p.clone()).or_insert(0) += 1;
        }
    }
    let diagnostics = layers.iter().flat_map(|l| l.validate(known)).collect();
    PatchSummary {
        total: layers.len(),
        by_profile: counts.into_iter().collect(),
        diagnostics,
    }
}

/// patch 应用报告：逐 op **如实分类**的结果。
#[derive(Debug, Clone, Default)]
pub struct PatchReport {
    /// 已应用的 op 描述
    pub applied: Vec<String>,
    /// (op 描述, 原因) —— 本轮无法落地但**并非错误**
    pub skipped: Vec<(String, String)>,
    /// (op 描述 / 层描述, 原因) —— 失败项
    pub errors: Vec<(String, String)>,
}

/// patch 声明槽位的占位执行器：**调用即显式失败**，绝不假装成功。
struct UnimplementedExecutor;

impl SlotExecutor for UnimplementedExecutor {
    fn execute(&self, name: &str, _args: &HashMap<String, String>) -> Result<SlotResult, String> {
        Err(format!(
            "槽位 `{name}` 由 patch 声明但尚无真实执行器（未实现）"
        ))
    }
}

/// 把补丁层**真正装配**进 `PluginSlot`。
///
/// 语义：
/// - 先 `order_layers(layers)?`（复用 S2 排序），再逐层逐 op 应用；
/// - **整层 target 校验**：`target` 未命中 `manifests` 的同名清单 → 该层所有 op
///   记入 `errors` 并跳过（**不** `Err` 整个函数）；
/// - `Insert`：注册 `SlotDescriptor`（`name=id` / `kind=Command` /
///   `description=label.unwrap_or(target)` / `order` / `source=target` /
///   `declared_slot=slot` / `requires_approval=true`），执行器为
///   `UnimplementedExecutor`。成功 → `applied`；重名 → `errors`。
/// - `Disable`：`slot.disable(id)`，`true` → `applied`，`false` → `skipped`（「无此槽位」）；
/// - `Set`：本轮**无配置覆盖层**，**如实**记入 `skipped`（不假装写入、不静默丢弃）。
pub fn apply_layers(
    slot: &mut PluginSlot,
    manifests: &[PluginManifest],
    layers: &[PatchLayer],
) -> Result<PatchReport> {
    let ordered = order_layers(layers)?;
    let known: HashSet<&str> = manifests.iter().map(|m| m.name.as_str()).collect();

    let mut report = PatchReport::default();

    for layer in &ordered {
        if !known.contains(layer.target.as_str()) {
            report.errors.push((
                format!("patch `{}`", layer.target),
                format!(
                    "target `{}` 未命中任何已知 manifest（{} 份已装载清单），整层跳过",
                    layer.target,
                    manifests.len()
                ),
            ));
            continue;
        }

        for op in &layer.ops {
            match op {
                PatchOp::Insert {
                    slot: logical_slot,
                    id,
                    order,
                    label,
                } => {
                    let desc = format!(
                        "insert `{id}` → 槽位 `{logical_slot}`（target `{}`，order {order}）",
                        layer.target
                    );
                    let descriptor = SlotDescriptor {
                        name: id.clone(),
                        kind: SlotKind::Command,
                        description: label.clone().unwrap_or_else(|| layer.target.clone()),
                        schema: serde_json::Value::Null,
                        source: Some(layer.target.clone()),
                        requires_approval: true,
                        order: *order,
                        declared_slot: Some(logical_slot.clone()),
                    };
                    match slot.register(descriptor, Arc::new(UnimplementedExecutor)) {
                        Ok(()) => report.applied.push(desc),
                        Err(reason) => report.errors.push((desc, reason)),
                    }
                }
                PatchOp::Disable { id } => {
                    let desc = format!("disable `{id}`（target `{}`）", layer.target);
                    if slot.disable(id) {
                        report.applied.push(desc);
                    } else {
                        report.skipped.push((desc, "无此槽位".to_string()));
                    }
                }
                PatchOp::Set { key, .. } => {
                    let desc = format!("set `{key}`（target `{}`）", layer.target);
                    report
                        .skipped
                        .push((desc, "`set` 尚无落点：无配置覆盖层".to_string()));
                }
            }
        }
    }

    Ok(report)
}

/// patch 目录候选：`MARGUERITE_PATCHES_DIR` 环境变量 > `cwd/plugins/patches`
/// > exe 同目录/上溯 > 安装资源布局 `resources/plugins/patches`。
///
/// 与 `crate::gateway::market::plugin_manifest_dir()` 同构。
/// 全部候选都不存在时返回 `None`。
pub fn patches_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MARGUERITE_PATCHES_DIR") {
        if !p.trim().is_empty() {
            let path = PathBuf::from(p);
            // 与其余候选同口径：目录不存在则继续下溯，最终无匹配 → None。
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    let cwd = std::env::current_dir().ok()?;
    let exe = std::env::current_exe().ok()?;
    let mut candidates = vec![
        cwd.join("plugins").join("patches"),
        exe.parent()?.join("plugins").join("patches"),
        exe.parent()?.parent()?.join("plugins").join("patches"),
        exe.parent()?
            .join("resources")
            .join("plugins")
            .join("patches"),
    ];
    candidates.retain(|p| p.is_dir());
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest::from_json(&format!(
            r#"{{"name":"{name}","version":"0.1.0","description":"{name}"}}"#
        ))
        .unwrap()
    }

    fn layer_json(target: &str) -> String {
        format!(
            r#"{{
                "schema": "{PATCH_SCHEMA}",
                "target": "{target}",
                "layer": 1,
                "ops": [
                    {{"op": "insert", "slot": "tools", "id": "t1", "order": 5}},
                    {{"op": "set", "key": "ui.theme", "value": "dark"}},
                    {{"op": "disable", "id": "legacy"}}
                ]
            }}"#
        )
    }

    /// 唯一临时目录（不依赖 tempfile crate）。
    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("mg-patch-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_all_three_ops_and_defaults() {
        let l: PatchLayer = serde_json::from_str(&layer_json("dsh-modlens")).unwrap();
        assert_eq!(l.schema, PATCH_SCHEMA);
        assert_eq!(l.target, "dsh-modlens");
        assert_eq!(l.layer, 1);
        // profile 缺省 → ["desktop"]
        assert_eq!(l.profile, vec!["desktop".to_string()]);
        assert!(l.requires.is_empty());
        assert_eq!(l.ops.len(), 3);
        match &l.ops[0] {
            PatchOp::Insert {
                slot,
                id,
                order,
                label,
            } => {
                assert_eq!(slot, "tools");
                assert_eq!(id, "t1");
                assert_eq!(*order, 5);
                assert_eq!(*label, None);
            }
            other => panic!("期望 Insert，得到 {other:?}"),
        }
        assert!(matches!(&l.ops[1], PatchOp::Set { key, .. } if key == "ui.theme"));
        assert!(matches!(&l.ops[2], PatchOp::Disable { id } if id == "legacy"));
    }

    #[test]
    fn load_dir_reads_sorted_patch_files() {
        let dir = unique_tmp_dir("load");
        std::fs::write(dir.join("b.patch.json"), layer_json("plug-b")).unwrap();
        std::fs::write(dir.join("a.patch.json"), layer_json("plug-a")).unwrap();
        // 非 patch 后缀应被忽略
        std::fs::write(dir.join("notes.txt"), "ignore me").unwrap();

        let layers = PatchLayer::load_dir(&dir).unwrap();
        assert_eq!(layers.len(), 2);
        // 按文件名排序 → a 在前
        assert_eq!(layers[0].target, "plug-a");
        assert_eq!(layers[1].target, "plug-b");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_dir_bad_json_errors_with_filename() {
        let dir = unique_tmp_dir("badjson");
        std::fs::write(dir.join("broken.patch.json"), "{ not json").unwrap();

        let err = PatchLayer::load_dir(&dir).expect_err("坏 JSON 必须报错，不得静默丢弃");
        let text = format!("{err:#}");
        assert!(
            text.contains("broken.patch.json"),
            "错误信息须含文件名，实际: {text}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_dir_missing_dir_errors() {
        let dir = unique_tmp_dir("missing");
        std::fs::remove_dir_all(&dir).ok();
        assert!(PatchLayer::load_dir(&dir).is_err());
    }

    #[test]
    fn validate_flags_bad_schema_orphan_and_empty_id() {
        let l = PatchLayer {
            schema: "marguerite.patch/v0".to_string(),
            target: "ghost".to_string(),
            layer: 0,
            profile: vec!["desktop".to_string()],
            requires: vec!["nowhere".to_string()],
            ops: vec![PatchOp::Insert {
                slot: "tools".to_string(),
                id: "   ".to_string(),
                order: 0,
                label: None,
            }],
        };
        let diags = l.validate(&[manifest("real")]);
        let codes: Vec<&str> = diags.iter().map(|d| d.code).collect();
        assert!(codes.contains(&"patch.bad_schema"), "codes={codes:?}");
        assert!(codes.contains(&"patch.orphan_target"), "codes={codes:?}");
        assert!(codes.contains(&"patch.empty_id"), "codes={codes:?}");
        assert!(
            codes.contains(&"patch.missing_requirement"),
            "codes={codes:?}"
        );
        assert!(diags.iter().all(|d| d.target == "ghost"));
    }

    #[test]
    fn validate_clean_layer_has_no_diagnostics() {
        let l: PatchLayer = serde_json::from_str(&layer_json("dsh-modlens")).unwrap();
        assert!(l.validate(&[manifest("dsh-modlens")]).is_empty());
    }

    fn layer(target: &str, layer: i32, requires: &[&str]) -> PatchLayer {
        PatchLayer {
            schema: PATCH_SCHEMA.to_string(),
            target: target.to_string(),
            layer,
            profile: vec!["desktop".to_string()],
            requires: requires.iter().map(|s| s.to_string()).collect(),
            ops: Vec::new(),
        }
    }

    #[test]
    fn order_layers_sorts_by_layer_then_target() {
        let input = vec![
            layer("zeta", 1, &[]),
            layer("alpha", 1, &[]),
            layer("later", 9, &[]),
        ];
        let out = order_layers(&input).unwrap();
        let names: Vec<&str> = out.iter().map(|l| l.target.as_str()).collect();
        // layer 升序；同层按 target 字典序（输入里 zeta 在 alpha 前，须被纠正）
        assert_eq!(names, vec!["alpha", "zeta", "later"]);
    }

    #[test]
    fn order_layers_respects_requires_topology() {
        // b 依赖 a：即使 a 的 layer 更大，a 也必须先于 b
        let input = vec![layer("b", 1, &["a"]), layer("a", 5, &[])];
        let out = order_layers(&input).unwrap();
        let names: Vec<&str> = out.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn order_layers_is_deterministic_for_same_layer() {
        let input = vec![layer("m", 2, &[]), layer("a", 2, &[]), layer("z", 2, &[])];
        let first = order_layers(&input).unwrap();
        let second = order_layers(&input).unwrap();
        let n1: Vec<&str> = first.iter().map(|l| l.target.as_str()).collect();
        let n2: Vec<&str> = second.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(n1, vec!["a", "m", "z"]);
        assert_eq!(n1, n2);
    }

    #[test]
    fn order_layers_detects_cycle() {
        let input = vec![layer("a", 0, &["b"]), layer("b", 0, &["a"])];
        assert!(order_layers(&input).is_err(), "a<->b 环应被拒绝");
    }

    #[test]
    fn order_layers_detects_missing_dependency() {
        let input = vec![layer("a", 0, &["ghost"])];
        let err = order_layers(&input).expect_err("缺失依赖应被拒绝");
        assert!(format!("{err:#}").contains("ghost"));
    }

    #[test]
    fn summarize_counts_profiles_and_collects_diagnostics() {
        let known = vec![manifest("good")];
        let mut good = layer("good", 0, &[]);
        good.profile = vec!["desktop".to_string(), "web".to_string()];
        let bad = PatchLayer {
            schema: PATCH_SCHEMA.to_string(),
            target: "ghost".to_string(),
            layer: 0,
            profile: vec!["desktop".to_string()],
            requires: Vec::new(),
            ops: Vec::new(),
        };
        let s = summarize(&[good, bad], &known);
        assert_eq!(s.total, 2);
        assert_eq!(
            s.by_profile,
            vec![("desktop".to_string(), 2), ("web".to_string(), 1)]
        );
        assert_eq!(s.diagnostics.len(), 1);
        assert_eq!(s.diagnostics[0].code, "patch.orphan_target");
    }

    #[test]
    fn summarize_empty_input() {
        let s = summarize(&[], &[]);
        assert_eq!(s.total, 0);
        assert!(s.by_profile.is_empty());
        assert!(s.diagnostics.is_empty());
    }

    // ── S4：apply_layers ─────────────────────────────────────────────

    fn insert_op(slot: &str, id: &str, order: i32) -> PatchOp {
        PatchOp::Insert {
            slot: slot.to_string(),
            id: id.to_string(),
            order,
            label: None,
        }
    }

    fn layer_with_ops(target: &str, layer: i32, ops: Vec<PatchOp>) -> PatchLayer {
        PatchLayer {
            schema: PATCH_SCHEMA.to_string(),
            target: target.to_string(),
            layer,
            profile: vec!["desktop".to_string()],
            requires: Vec::new(),
            ops,
        }
    }

    #[test]
    fn apply_insert_set_disable_are_classified_truthfully() {
        let mut slot = PluginSlot::new();
        let manifests = vec![manifest("plug")];
        let l = layer_with_ops(
            "plug",
            1,
            vec![
                insert_op("tools", "cmd.new", 5),
                PatchOp::Set {
                    key: "ui.theme".to_string(),
                    value: serde_json::json!("dark"),
                },
                PatchOp::Disable {
                    id: "cmd.new".to_string(),
                },
            ],
        );

        let report = apply_layers(&mut slot, &manifests, &[l]).unwrap();

        // insert → applied；disable（槽位存在）→ applied；set → skipped
        assert_eq!(report.applied.len(), 2, "applied={:?}", report.applied);
        assert_eq!(report.skipped.len(), 1, "skipped={:?}", report.skipped);
        assert!(report.errors.is_empty(), "errors={:?}", report.errors);
        assert!(report.applied[0].contains("cmd.new"));
        assert!(report.skipped[0].1.contains("无配置覆盖层"));

        // 落点事实：槽位已注册且 order 正确；随后被停用
        assert!(slot.contains("cmd.new"));
        assert_eq!(slot.get("cmd.new").unwrap().order, 5);
        assert!(slot.is_disabled("cmd.new"));
    }

    #[test]
    fn apply_insert_orphan_target_is_reported_as_error() {
        let mut slot = PluginSlot::new();
        let l = layer_with_ops("ghost", 1, vec![insert_op("tools", "cmd.x", 0)]);

        let report = apply_layers(&mut slot, &[manifest("real")], &[l]).unwrap();

        assert!(report.applied.is_empty());
        assert_eq!(report.errors.len(), 1, "errors={:?}", report.errors);
        assert!(report.errors[0].1.contains("ghost"), "{:?}", report.errors[0]);
        assert!(!slot.contains("cmd.x"), "悬空 target 不得注册任何槽位");
    }

    #[test]
    fn apply_duplicate_insert_is_reported_as_error() {
        let mut slot = PluginSlot::new();
        let manifests = vec![manifest("plug-a"), manifest("plug-b")];
        // 两层各自 insert 同一 id：第一层成功，第二层重名 → errors
        let a = layer_with_ops("plug-a", 1, vec![insert_op("tools", "dup", 0)]);
        let b = layer_with_ops("plug-b", 2, vec![insert_op("tools", "dup", 0)]);

        let report = apply_layers(&mut slot, &manifests, &[a, b]).unwrap();

        assert_eq!(report.applied.len(), 1, "applied={:?}", report.applied);
        assert_eq!(report.errors.len(), 1, "errors={:?}", report.errors);
        assert!(report.errors[0].1.contains("dup"), "{:?}", report.errors[0]);
        // 先注册者胜出：source 保持 plug-a
        assert_eq!(
            slot.get("dup").unwrap().source.as_deref(),
            Some("plug-a"),
            "重名不得覆盖先注册项"
        );
    }

    #[test]
    fn apply_disable_missing_slot_is_skipped() {
        let mut slot = PluginSlot::new();
        let l = layer_with_ops(
            "plug",
            1,
            vec![PatchOp::Disable {
                id: "nope".to_string(),
            }],
        );

        let report = apply_layers(&mut slot, &[manifest("plug")], &[l]).unwrap();
        assert!(report.applied.is_empty());
        assert_eq!(report.skipped.len(), 1, "skipped={:?}", report.skipped);
        assert_eq!(report.skipped[0].1, "无此槽位");
    }

    #[test]
    fn apply_inserted_slot_uses_unimplemented_executor_and_errors_on_invoke() {
        let mut slot = PluginSlot::new();
        let l = layer_with_ops("plug", 1, vec![insert_op("tools", "t1", 0)]);

        let report = apply_layers(&mut slot, &[manifest("plug")], &[l]).unwrap();
        assert_eq!(report.applied.len(), 1, "applied={:?}", report.applied);

        // patch 声明的槽位只登记了「未实现」占位执行器：调用必须**显式失败**。
        let err = slot
            .invoke("t1", &HashMap::new())
            .expect_err("patch 声明的槽位尚无真实执行器，invoke 必须返回 Err");
        assert_eq!(
            err, "槽位 `t1` 由 patch 声明但尚无真实执行器（未实现）",
            "占位执行器的错误原文必须稳定"
        );

        // 元数据事实
        let d = slot.get("t1").unwrap();
        assert_eq!(d.kind, SlotKind::Command);
        assert_eq!(d.source.as_deref(), Some("plug"));
        assert_eq!(d.declared_slot.as_deref(), Some("tools"));
        assert_eq!(d.description, "plug", "无 label 时 description 回落到 target");
        assert!(d.requires_approval);
    }

    #[test]
    fn apply_layers_propagates_order_layers_failure() {
        // 环依赖：order_layers 必须拒绝，apply_layers 随之 Err（不静默降级）
        let a = layer("a", 0, &["b"]);
        let b = layer("b", 0, &["a"]);
        let mut slot = PluginSlot::new();
        assert!(apply_layers(&mut slot, &[manifest("a"), manifest("b")], &[a, b]).is_err());
    }
}
