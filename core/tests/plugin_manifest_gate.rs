//! 清单一致性门禁（S0）：`plugins/manifests/*.plugin.json` 自身的自洽性。
//!
//! 背景：本仓曾有「清单已声明但无人装配」的静默形态 —— `plugins/manifest.rs` 的
//! DSH 清单测试因默认路径多了一层 `..` 而**空转通过**（vacuous pass），门禁全绿但零验证。
//! 本文件把「清单本身自洽」提成独立门禁，与 `manifest.rs` 内的实装测试互补。
//!
//! 只断言**不需要 patch 层**的不变量（patch 层见 P0 后续切片）。

use std::collections::HashSet;
use std::path::PathBuf;

use marguerite_core::plugins::manifest::{PluginManifest, topo_order};
// P0 S3：经 `plugins` 顶层 re-export 引入，顺带证明 re-export 可用。
use marguerite_core::plugins::{Activation, ManifestDiagnostic, Permission};

/// 清单目录：`CARGO_MANIFEST_DIR` = `<repo>/core`，上溯一层即仓库根。
fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("plugins")
        .join("manifests")
}

/// 加载目录下全部 `*.plugin.json`（*读失败/解析失败直接 panic 并带路径*，
/// 不静默丢弃 —— 与 `manifest.rs` 同口径）。
fn load_all() -> Vec<(PathBuf, PluginManifest)> {
    let dir = manifest_dir();
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
        out.push((path, m));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn manifests_are_self_consistent() {
    let list = load_all();
    assert!(
        !list.is_empty(),
        "清单目录 {} 下未找到任何 *.plugin.json",
        manifest_dir().display()
    );

    // 1) name 唯一且基础字段非空
    let mut seen: HashSet<&str> = HashSet::new();
    for (path, m) in &list {
        assert!(
            seen.insert(m.name.as_str()),
            "重名清单 `{}` @ {}",
            m.name,
            path.display()
        );
        assert!(!m.name.is_empty(), "name 空 @ {}", path.display());
        assert!(!m.version.is_empty(), "version 空 @ {}", path.display());
        assert!(!m.description.is_empty(), "description 空 @ {}", path.display());
    }

    // 2) dependencies 均可解析（同目录内存在同名清单）
    let names: HashSet<&str> = list.iter().map(|(_, m)| m.name.as_str()).collect();
    for (path, m) in &list {
        for d in &m.dependencies {
            assert!(
                names.contains(d.as_str()),
                "清单 `{}`（{}）依赖 `{}`，但 {} 中无同名清单（悬空依赖）",
                m.name,
                path.display(),
                d,
                manifest_dir().display()
            );
        }
    }

    // 3) 拓扑排序必须成功（无环、无缺失），且不丢节点
    let manifests: Vec<PluginManifest> = list.iter().map(|(_, m)| m.clone()).collect();
    let ordered = topo_order(&manifests)
        .unwrap_or_else(|e| panic!("清单依赖拓扑排序失败（环或缺失依赖）: {e}"));
    assert_eq!(ordered.len(), manifests.len(), "拓扑排序不得丢节点");

    // 4) 防空壳清单：每份至少一个实质声明
    for (path, m) in &list {
        let has_any = !m.capabilities.is_empty()
            || !m.hooks.is_empty()
            || !m.slash_commands.is_empty()
            || !m.subagents.is_empty()
            || !m.skills.is_empty()
            || !m.events.is_empty();
        assert!(
            has_any,
            "空壳清单（无 capabilities/hooks/slash_commands/subagents/skills/events 任一）: {}",
            path.display()
        );
    }
}

// ═══════════════════════ P0 S3：entry / activation / dispose / permissions ═══════════════════════

/// 新字段解析自造清单：`json` 必须能被 `PluginManifest` 解析。
fn parse(json: &str) -> PluginManifest {
    PluginManifest::from_json(json).expect("自造清单解析失败（P0 S3 新字段 schema 破坏）")
}

/// 取 `validate()` 的 code 列表。
fn codes_of(m: &PluginManifest) -> Vec<&'static str> {
    m.validate().iter().map(|d| d.code).collect()
}

/// S3-1：16 份随仓库存在的清单用**新字段**全部可解析，且缺省值正确。
#[test]
fn s3_manifests_parse_new_fields_with_defaults() {
    let list = load_all();
    assert!(
        list.len() >= 16,
        "应至少 16 份 manifest（基线），实际 {} 份",
        list.len()
    );
    for (path, m) in &list {
        assert_eq!(m.entry, None, "entry 缺省应为 None @ {}", path.display());
        assert_eq!(
            m.activation,
            Activation::Restart,
            "activation 缺省应为 Restart @ {}",
            path.display()
        );
        assert!(
            m.permissions.is_empty(),
            "permissions 缺省应为空 @ {}",
            path.display()
        );
        assert_eq!(m.dispose, None, "dispose 缺省应为 None @ {}", path.display());
    }
}

/// S3-2：把「这批 manifest 当前全部是 declarative-only」这一**事实钉死**。
///
/// 若某份不再是（例如新增了 `entry`），本用例**必须响亮失败** —— 不得放宽断言，
/// 而应如实报告「声明了但已实现」的那一份。
#[test]
fn s3_manifests_are_all_declarative_only_today() {
    let list = load_all();
    assert!(list.len() >= 16, "清单份数不足基线 16：{}", list.len());
    let non_declarative: Vec<String> = list
        .iter()
        .filter(|(_, m)| !m.is_declarative_only())
        .map(|(p, m)| format!("{} @ {}", m.name, p.display()))
        .collect();
    assert!(
        non_declarative.is_empty(),
        "当前事实应为「16 份全部 declarative-only」，但以下清单不是（declare 了 entry 或 permissions）: {non_declarative:?}"
    );
    println!(
        "S3-2：{} 份清单全部为 declarative-only（无 entry / 无 permissions）",
        list.len()
    );
}

/// S3-3a：`manifest.hot_without_entry`
#[test]
fn s3_validate_flags_hot_without_entry() {
    let m = parse(
        r#"{"name":"hot-ghost","version":"0.1.0","description":"d","activation":"hot","capabilities":["x.run"]}"#,
    );
    assert_eq!(m.activation, Activation::Hot);
    let codes = codes_of(&m);
    assert!(
        codes.contains(&"manifest.hot_without_entry"),
        "应报 hot_without_entry，实际 {codes:?}"
    );
}

/// S3-3b：`manifest.undeclared_side_effect`
#[test]
fn s3_validate_flags_undeclared_side_effect() {
    let m = parse(
        r#"{"name":"fsy","version":"0.1.0","description":"d","capabilities":["fs.read","net.get","thing.run"]}"#,
    );
    let codes = codes_of(&m);
    assert!(
        codes.contains(&"manifest.undeclared_side_effect"),
        "应报 undeclared_side_effect，实际 {codes:?}"
    );
    // 六面非空，不应同时报空壳
    assert!(!codes.contains(&"manifest.empty_shell"), "{codes:?}");
}

/// S3-3c：`manifest.empty_shell`
#[test]
fn s3_validate_flags_empty_shell() {
    let m = parse(r#"{"name":"shell","version":"0.1.0","description":"d"}"#);
    let codes = codes_of(&m);
    assert!(
        codes.contains(&"manifest.empty_shell"),
        "应报 empty_shell，实际 {codes:?}"
    );
}

/// S3-3d：`manifest.exec_without_decl`
#[test]
fn s3_validate_flags_exec_without_capability() {
    let m = parse(
        r#"{"name":"execy","version":"0.1.0","description":"d","capabilities":["thing.run"],"permissions":[{"kind":"exec","allow":["git"]}]}"#,
    );
    assert!(
        m.permissions
            .iter()
            .any(|p| matches!(p, Permission::Exec { .. })),
        "自造清单应含 Exec 权限"
    );
    let codes = codes_of(&m);
    assert!(
        codes.contains(&"manifest.exec_without_decl"),
        "应报 exec_without_decl，实际 {codes:?}"
    );
}

/// S3-4：在 16 份真实清单上跑 `validate()`，**原样打印**诊断清单。
///
/// 当前事实：零诊断（全部为纯声明式，无 `Hot`、无 `fs./exec./net.` 能力、非空壳）。
#[test]
fn s3_real_manifests_validate_and_print_diagnostics() {
    let list = load_all();
    assert!(list.len() >= 16, "清单份数不足基线 16：{}", list.len());

    let mut all: Vec<ManifestDiagnostic> = Vec::new();
    for (path, m) in &list {
        let diags = m.validate();
        if diags.is_empty() {
            println!("  [ok] {} ({})", m.name, path.display());
        }
        for d in &diags {
            println!(
                "  [{}] {} @ {} : {}",
                d.code,
                d.manifest,
                path.display(),
                d.message
            );
        }
        all.extend(diags);
    }
    println!(
        "S3-4：{} 份真实清单 validate 完成，诊断 {} 条",
        list.len(),
        all.len()
    );

    // 钉死当前事实：16 份清单零诊断。若新增了副作用能力/热加载/空壳，
    // 本门禁会失败并要求显式补齐声明 —— 这正是「声明了但没实现」要被看见的目的。
    assert!(
        all.is_empty(),
        "真实清单出现 validate 诊断（当前事实为 0 条），诊断清单: {:?}",
        all.iter()
            .map(|d| format!("[{}] {}: {}", d.code, d.manifest, d.message))
            .collect::<Vec<_>>()
    );
}

/// S3-5：自造「完整原生插件」清单零诊断（validate 不过度报错）。
#[test]
fn s3_fully_declared_native_manifest_has_no_diagnostics() {
    let m = parse(
        r#"{
            "name":"native-ok",
            "version":"0.2.0",
            "description":"完整声明的原生插件",
            "entry":"bin/native-ok",
            "activation":"hot",
            "dispose":"bin/native-ok-down",
            "capabilities":["exec.shell","native.run"],
            "permissions":[{"kind":"exec","allow":["git"]},{"kind":"fs","scope":["work"],"write":true}]
        }"#,
    );
    let diags = m.validate();
    assert!(
        diags.is_empty(),
        "完整声明应零诊断，实际 {:?}",
        diags
            .iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect::<Vec<_>>()
    );
}
