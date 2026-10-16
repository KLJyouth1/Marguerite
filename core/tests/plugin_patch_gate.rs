//! patch 层集成门禁（P0 S2）：`*.patch.json` 的装载 / 校验 / 排序。
//!
//! 本文件**不依赖仓库里存在任何 patch 文件** —— 全部用临时目录自造输入，
//! 以覆盖「文件不存在时门禁仍然有效」的诉求。
//!
//! 唯一一处对仓库真实目录的断言是**条件性**的：若 `plugins/patches/` 存在且非空，
//! 则其中每个 `target` 都必须命中 `plugins/manifests/*.plugin.json` 的 `name` 集合；
//! 目录不存在则跳过并打印说明（因为本切片尚未随仓库提供任何 patch 文件）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use marguerite_core::plugins::manifest::PluginManifest;
use marguerite_core::plugins::patch::{self, PatchLayer, PatchOp};
use marguerite_core::plugins::PluginSlot;

/// 仓库根：`CARGO_MANIFEST_DIR` = `<repo>/core`，上溯一层。
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .to_path_buf()
}

/// 唯一临时目录（避免新增 dev-dependency，用 pid + 纳秒时间戳）。
fn unique_tmp_dir(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("mg-gate-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("创建临时目录 {} 失败: {e}", dir.display()));
    dir
}

/// 合法 patch 文本。
fn valid_patch(target: &str, layer: i32) -> String {
    format!(
        r#"{{
            "schema": "marguerite.patch/v1",
            "target": "{target}",
            "layer": {layer},
            "profile": ["desktop"],
            "ops": [{{"op": "insert", "slot": "tools", "id": "t1"}}]
        }}"#
    )
}

/// 加载仓库真实清单（读失败 / 解析失败直接 panic，不静默丢弃）。
fn load_manifests(repo: &Path) -> Vec<PluginManifest> {
    let dir = repo.join("plugins").join("manifests");
    let rd =
        std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("清单目录不可读 {}: {e}", dir.display()));
    let mut out = Vec::new();
    for entry in rd {
        let path = entry
            .unwrap_or_else(|e| panic!("读取目录项失败（{}）: {e}", dir.display()))
            .path();
        let is_manifest = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".plugin.json"))
            .unwrap_or(false);
        if !is_manifest {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
        let m = PluginManifest::from_json(&text)
            .unwrap_or_else(|e| panic!("解析 {} 失败: {e:?}", path.display()));
        out.push(m);
    }
    out
}

#[test]
fn load_dir_parses_valid_patch() {
    let dir = unique_tmp_dir("valid");
    std::fs::write(dir.join("one.patch.json"), valid_patch("dsh-modlens", 3)).unwrap();

    let layers = PatchLayer::load_dir(&dir).expect("合法 patch 应能装载");
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].target, "dsh-modlens");
    assert_eq!(layers[0].layer, 3);
    assert_eq!(layers[0].schema, patch::PATCH_SCHEMA);
    assert!(matches!(&layers[0].ops[0], PatchOp::Insert { id, .. } if id == "t1"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn load_dir_bad_json_errors_with_filename() {
    let dir = unique_tmp_dir("badjson");
    std::fs::write(dir.join("broken.patch.json"), "{ this is not json").unwrap();

    let err = PatchLayer::load_dir(&dir).expect_err("坏 JSON 必须返回 Err，不得静默丢弃");
    let text = format!("{err:#}");
    assert!(
        text.contains("broken.patch.json"),
        "错误信息必须含文件名，实际: {text}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn orphan_target_produces_diagnostic() {
    let dir = unique_tmp_dir("orphan");
    std::fs::write(dir.join("ghost.patch.json"), valid_patch("no-such-plugin", 0)).unwrap();
    let layers = PatchLayer::load_dir(&dir).unwrap();

    // 真实清单里没有 no-such-plugin
    let known = load_manifests(&repo_root());
    let summary = patch::summarize(&layers, &known);

    assert_eq!(summary.total, 1);
    assert_eq!(summary.diagnostics.len(), 1, "应恰好一条 orphan 诊断");
    let d = &summary.diagnostics[0];
    assert_eq!(d.code, "patch.orphan_target");
    assert_eq!(d.target, "no-such-plugin");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn order_layers_requires_overrides_layer() {
    let dir = unique_tmp_dir("order");
    // 基序 = layer 升序：pa(1) → pb(2) → pc(3)。
    // 但 pb 依赖 pc：requires 必须把 pc 提到 pb 之前（即使 pc 的 layer 更大）。
    std::fs::write(dir.join("a.patch.json"), valid_patch("pa", 1)).unwrap();
    std::fs::write(
        dir.join("b.patch.json"),
        r#"{"schema":"marguerite.patch/v1","target":"pb","layer":2,"requires":["pc"],"ops":[]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("c.patch.json"), valid_patch("pc", 3)).unwrap();

    let layers = PatchLayer::load_dir(&dir).unwrap();
    let out = patch::order_layers(&layers).unwrap();
    let names: Vec<&str> = out.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(names, vec!["pa", "pc", "pb"]);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn order_layers_layer_is_primary_tiebreak() {
    let dir = unique_tmp_dir("primary");
    // 语义钉死：`layer` 是**主序**，requires 只在「阻塞」时把被依赖者提前。
    //   modlens(5) / market(2, requires modlens) / zeta(2)
    // 基序 = [market(2), zeta(2), modlens(5)]；market 被阻塞 → 先出 zeta，
    // 再出 modlens，最后 market。**不是** [modlens, market, zeta]（那会违反 layer 主序）。
    std::fs::write(dir.join("a.patch.json"), valid_patch("dsh-modlens", 5)).unwrap();
    std::fs::write(
        dir.join("b.patch.json"),
        r#"{"schema":"marguerite.patch/v1","target":"dsh-market","layer":2,"requires":["dsh-modlens"],"ops":[]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("c.patch.json"), valid_patch("dsh-zeta", 2)).unwrap();

    let layers = PatchLayer::load_dir(&dir).unwrap();
    let out = patch::order_layers(&layers).unwrap();
    let names: Vec<&str> = out.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(names, vec!["dsh-zeta", "dsh-modlens", "dsh-market"]);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn order_layers_detects_cycle() {
    let dir = unique_tmp_dir("cycle");
    std::fs::write(
        dir.join("a.patch.json"),
        r#"{"schema":"marguerite.patch/v1","target":"pa","requires":["pb"],"ops":[]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("b.patch.json"),
        r#"{"schema":"marguerite.patch/v1","target":"pb","requires":["pa"],"ops":[]}"#,
    )
    .unwrap();

    let layers = PatchLayer::load_dir(&dir).unwrap();
    assert!(patch::order_layers(&layers).is_err(), "pa<->pb 环应被拒绝");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn order_layers_same_layer_is_lexicographic_and_stable() {
    let dir = unique_tmp_dir("lex");
    for name in ["zzz", "aaa", "mmm"] {
        std::fs::write(dir.join(format!("{name}.patch.json")), valid_patch(name, 1)).unwrap();
    }
    let layers = PatchLayer::load_dir(&dir).unwrap();
    let out = patch::order_layers(&layers).unwrap();
    let names: Vec<&str> = out.iter().map(|l| l.target.as_str()).collect();
    assert_eq!(names, vec!["aaa", "mmm", "zzz"]);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn repo_patches_dir_targets_hit_known_manifests() {
    let patches_dir = repo_root().join("plugins").join("patches");
    if !patches_dir.is_dir() {
        eprintln!(
            "skip: {} 不存在（本切片未随仓库提供 patch 文件，apply 见后续切片）",
            patches_dir.display()
        );
        return;
    }
    // D5：逐项 io 错误显式 panic，不用 `.flatten()` 静默丢弃（静默丢弃会让
    // 「patch 目录里少了一个文件」表现为门禁通过）。
    let rd = std::fs::read_dir(&patches_dir)
        .unwrap_or_else(|e| panic!("patch 目录不可读 {}: {e}", patches_dir.display()));
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in rd {
        let path = entry
            .unwrap_or_else(|e| panic!("读取目录项失败（{}）: {e}", patches_dir.display()))
            .path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".patch.json"))
            .unwrap_or(false)
        {
            entries.push(path);
        }
    }
    if entries.is_empty() {
        eprintln!(
            "skip: {} 存在但无 *.patch.json（仅 README.md）",
            patches_dir.display()
        );
        return;
    }

    let layers = PatchLayer::load_dir(&patches_dir)
        .unwrap_or_else(|e| panic!("装载 {} 失败: {e:#}", patches_dir.display()));
    let names: HashSet<String> = load_manifests(&repo_root())
        .into_iter()
        .map(|m| m.name)
        .collect();
    for l in &layers {
        assert!(
            names.contains(&l.target),
            "patch `{}` 的 target 未命中任何 manifest（{} 份已知清单）",
            l.target,
            names.len()
        );
    }
}

/// P0 S4 端到端：`load_dir` → `apply_layers` → 槽位注册结果与 report 分类。
///
/// 造两份 patch：一份**合法 insert**（外加一条 `set`），一份 **orphan target**。
/// 断言三分类各自内容、以及「真 apply 到 `PluginSlot`」的槽位事实。
#[test]
fn apply_layers_registers_insert_and_reports_orphan_layer() {
    let dir = unique_tmp_dir("apply");
    // 合法层：insert（order=7，带 label）+ set（本轮无落点）
    std::fs::write(
        dir.join("a.patch.json"),
        r#"{
            "schema": "marguerite.patch/v1",
            "target": "plug-good",
            "layer": 1,
            "ops": [
                {"op": "insert", "slot": "tools", "id": "good.cmd", "order": 7, "label": "好命令"},
                {"op": "set", "key": "ui.theme", "value": "dark"}
            ]
        }"#,
    )
    .unwrap();
    // 悬空层：target 不在已知清单里 → 整层记 errors
    std::fs::write(
        dir.join("b.patch.json"),
        r#"{
            "schema": "marguerite.patch/v1",
            "target": "plug-orphan",
            "layer": 2,
            "ops": [{"op": "insert", "slot": "tools", "id": "orphan.cmd"}]
        }"#,
    )
    .unwrap();

    let layers = PatchLayer::load_dir(&dir).expect("合法 patch 应能装载");
    assert_eq!(layers.len(), 2);

    let manifests = vec![
        PluginManifest::from_json(r#"{"name":"plug-good","version":"0.1.0","description":"good"}"#)
            .unwrap(),
    ];

    let mut slot = PluginSlot::new();
    let report = patch::apply_layers(&mut slot, &manifests, &layers).expect("apply 不应整函数失败");

    // 三分类
    assert_eq!(report.applied.len(), 1, "applied={:?}", report.applied);
    assert_eq!(report.skipped.len(), 1, "skipped={:?}", report.skipped);
    assert_eq!(report.errors.len(), 1, "errors={:?}", report.errors);
    assert!(report.applied[0].contains("good.cmd"), "{:?}", report.applied);
    assert_eq!(report.skipped[0].1, "`set` 尚无落点：无配置覆盖层");
    assert!(
        report.errors[0].1.contains("plug-orphan"),
        "errors={:?}",
        report.errors
    );

    // 槽位注册结果（真 apply）
    assert!(slot.contains("good.cmd"), "合法 insert 必须真的注册进槽位");
    assert!(!slot.contains("orphan.cmd"), "悬空层的 insert 不得注册");
    let d = slot.get("good.cmd").expect("good.cmd 应存在");
    assert_eq!(d.order, 7, "patch 的 order 必须落到 SlotDescriptor");
    assert_eq!(d.source.as_deref(), Some("plug-good"));
    assert_eq!(d.declared_slot.as_deref(), Some("tools"));
    assert!(d.requires_approval, "patch 声明槽位默认需要批准");

    std::fs::remove_dir_all(&dir).ok();
}
