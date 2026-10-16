//! DSH 插件市场方法族（从 methods.rs 分治迁出）
//!
//! 覆盖 `market.*` 全量方法：本地插件清单 + 技能市场 + deepseek harness
//! 生态浏览（27 上游仓库蒸馏条目）统一市场视图，以及安装 / 升级 / 审计 /
//! 导入导出 / 源管理。离线诚实：零网络抓取，仅静态蒸馏表与本地清单。
//!
//! 分发臂留在 `methods::dispatch_rust`（`"market.*" => market::…`），本模块
//! 只承载实现——契约守卫 #4 的源扫描仍然覆盖这些臂。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::gateway::_helpers::param_str;

/// 上溯查找 `plugins/manifests` 的最大层数（含起点自身）。
///
/// D3：从 `<repo>/core` 启动时，`plugins/manifests` 在第 2 层（`<repo>`）即可命中；
/// 4 层足以覆盖「cwd 在 `core/`」「exe 在 `target/debug/`」「exe 在 `bin/`」等布局，
/// 又不会一路游走到盘根误命中无关目录。
const MAX_PLUGIN_DIR_WALK_UP: usize = 4;

/// 插件清单目录解析 —— **纯函数**（不读进程 env / cwd，全部由参数注入），便于单测。
///
/// 候选优先级：
/// 1. `env_override`（`MARGUERITE_PLUGINS_DIR`）：非空**且 `is_dir()`** 才生效；否则打
///    warning 后继续下溯（避免「env 指向不存在目录 → 静默 0 条」）；
/// 2. 从 `cwd` 逐级上溯（≤ [`MAX_PLUGIN_DIR_WALK_UP`] 层）找 `plugins/manifests`；
/// 3. 从 `exe_dir` 逐级上溯（同上）；
/// 4. 安装资源布局：`exe_dir/resources/plugins/manifests`（Tauri resources 目录）。
///
/// 命中即返回并 `tracing::debug!` 记录**来自哪条候选**；全部不命中 → `None`。
///
/// 历史缺陷（D3，QA 真机实测）：旧实现只试 `cwd/plugins/manifests` 与
/// `exe/…`，从 `core/` 启动（无 env）时**恒为 `None`** → `market.list` 的插件
/// 静默变 0 条（total 65→49），无任何报错。
pub(crate) fn resolve_plugins_dir(
    env_override: Option<&Path>,
    cwd: &Path,
    exe_dir: &Path,
) -> Option<PathBuf> {
    if let Some(p) = env_override.filter(|p| !p.as_os_str().is_empty()) {
        if p.is_dir() {
            tracing::debug!(
                source = "MARGUERITE_PLUGINS_DIR",
                dir = %p.display(),
                "插件清单目录已选定"
            );
            return Some(p.to_path_buf());
        }
        tracing::warn!(
            dir = %p.display(),
            "MARGUERITE_PLUGINS_DIR 指向的目录不存在（非目录），已忽略该覆盖并继续候选上溯"
        );
    }
    for (source, base) in [("cwd 上溯", cwd), ("exe 上溯", exe_dir)] {
        if base.as_os_str().is_empty() {
            continue;
        }
        for anc in base.ancestors().take(MAX_PLUGIN_DIR_WALK_UP) {
            let cand = anc.join("plugins").join("manifests");
            if cand.is_dir() {
                tracing::debug!(
                    source,
                    dir = %cand.display(),
                    "插件清单目录已选定"
                );
                return Some(cand);
            }
        }
    }
    if !exe_dir.as_os_str().is_empty() {
        let cand = exe_dir
            .join("resources")
            .join("plugins")
            .join("manifests");
        if cand.is_dir() {
            tracing::debug!(
                source = "exe/resources 安装布局",
                dir = %cand.display(),
                "插件清单目录已选定"
            );
            return Some(cand);
        }
    }
    None
}

/// DSH 插件市场：插件清单目录（环境变量 > cwd 上溯 > exe 上溯 > 安装资源布局）。
///
/// D4：只负责「取默认目录」，解析逻辑全在纯函数 [`resolve_plugins_dir`] 里；
/// 测试通过注入目录（[`collect_plugin_items`]）验证，**不再 `set_var`**（进程级
/// 环境变量在并行测试线程下与 `market_items()` 竞态）。
pub(crate) fn plugin_manifest_dir() -> Option<PathBuf> {
    let env_override = std::env::var_os("MARGUERITE_PLUGINS_DIR").map(PathBuf::from);
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "读取进程 cwd 失败（插件清单目录候选将跳过 cwd 分支）");
        PathBuf::new()
    });
    let exe_dir = match std::env::current_exe() {
        Ok(exe) => exe.parent().map(Path::to_path_buf).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(error = %e, "读取 current_exe 失败（插件清单目录候选将跳过 exe 分支）");
            PathBuf::new()
        }
    };
    let dir = resolve_plugins_dir(env_override.as_deref(), &cwd, &exe_dir);
    match &dir {
        Some(d) => tracing::debug!(
            dir = %d.display(),
            cwd = %cwd.display(),
            exe_dir = %exe_dir.display(),
            "插件清单目录 = {}",
            d.display()
        ),
        None => tracing::warn!(
            cwd = %cwd.display(),
            exe_dir = %exe_dir.display(),
            "插件清单目录未解析到：market.list 的插件条目将为 0（请设 MARGUERITE_PLUGINS_DIR 或从仓库布局启动）"
        ),
    }
    dir
}

/// 单份插件清单 → 市场条目（`kind=plugin`）。
///
/// P0 S3：**纯增量**追加 `mode` / `entry` / `activation` 三字段，使「声明了但没实现」
/// （`mode == "declarative-only"`）在控制台可见。既有 5 字段
/// （name/version/description/kind/author）名与类型保持不变。
fn plugin_market_item(m: &crate::plugins::manifest::PluginManifest) -> Value {
    use crate::plugins::manifest::Activation;
    json!({
        "name": m.name,
        "version": m.version,
        "description": m.description,
        "kind": "plugin",
        "author": m.author.clone().unwrap_or_default(),
        // ── P0 S3 纯增量字段 ──
        "mode": if m.is_declarative_only() { "declarative-only" } else { "native" },
        "entry": m.entry.clone(),
        "activation": match m.activation {
            Activation::Restart => "restart",
            Activation::Hot => "hot",
        },
    })
}

/// 单条 `skipped` 记录（可观测明细：路径 + 原因）。
///
/// **`skipped` 的唯一含义 = 「本该有、但未出现在 `items` 里的条目」**：
/// 读不到目录 / 读不到目录项 / 读不到文件，以及**解析失败且因而产不出条目**的清单。
/// 反例（刻意**不**计入，避免双重披露、避免语义模糊）：
/// - `market.audit` 里解析失败的清单**仍在 `items` 里**（`verdict="broken"`）→ 它没有被跳过；
/// - `market_sources` 某条源读不到 `.git/config` → 条目仍在，只是 `url` 为空；
/// - 技能目录装载失败 → 不是「清单文件」，只 `tracing::warn!`。
///
/// 消费口径：`items.len() + skipped.len()` **不等于**「磁盘上的文件总数」时，才说明有丢失。
fn skipped_record(file: &Path, reason: impl std::fmt::Display) -> Value {
    json!({ "file": file.display().to_string(), "reason": reason.to_string() })
}

/// 从**指定**清单目录收集插件市场条目 —— 目录由参数注入（可测），不读进程 env / cwd。
///
/// 返回 `(items, skipped)`。**逐项失败一律响亮**（D1）：读不到目录 / 读不到目录项 /
/// 读文件失败 / 解析失败 → 先 `tracing::warn!`（带**文件路径 + 原因**）再写入 `skipped`；
/// 绝不使用 `if let Ok(..)` 或 `.flatten()` 静默丢弃。
///
/// 历史缺陷（D1，QA 真机实测）：旧实现三层 `if let Ok` + `entries.flatten()`，清单一旦
/// 不可解析，16 条插件**全部凭空消失**，`market.list` 仍正常返回（只是少 16 条），
/// 前端只显示「0 项」且无任何报错。
fn collect_plugin_items(dir: Option<&Path>) -> (Vec<Value>, Vec<Value>) {
    let mut items: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let Some(dir) = dir else {
        // 目录本身未解析到时无「文件」可记；响亮告警由 `plugin_manifest_dir()` 负责。
        tracing::debug!("插件清单目录为 None，跳过插件条目收集");
        return (items, skipped);
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                dir = %dir.display(),
                error = %e,
                "读取插件清单目录失败：本次市场视图将不含任何插件条目"
            );
            skipped.push(skipped_record(dir, format!("读取目录失败: {e}")));
            return (items, skipped);
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %e,
                    "读取插件清单目录项失败（跳过该项）"
                );
                skipped.push(skipped_record(dir, format!("读取目录项失败: {e}")));
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "读取插件清单文件失败（跳过该清单）"
                );
                skipped.push(skipped_record(&path, format!("读取文件失败: {e}")));
                continue;
            }
        };
        match crate::plugins::manifest::PluginManifest::from_json(&text) {
            Ok(m) => items.push(plugin_market_item(&m)),
            Err(e) => {
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "插件清单解析失败（跳过该清单，已记入 skipped）"
                );
                skipped.push(skipped_record(&path, format!("解析失败: {e}")));
            }
        }
    }
    (items, skipped)
}

/// 市场条目：16 份 DSH 插件清单（kind=plugin）+ 技能市场技能（kind=skill）。
///
/// [`market_items`] 是该函数 `plugins_dir = None`（用默认候选目录）的薄封装。
/// `skipped` 只覆盖**插件清单**层面的逐项失败；技能目录装载失败单独 `tracing::warn!`
/// （技能不是「清单文件」，语义不同，不混入 `skipped`）。
fn market_items_report_in(plugins_dir: Option<&Path>) -> (Vec<Value>, Vec<Value>) {
    let resolved = match plugins_dir {
        Some(d) => Some(d.to_path_buf()),
        None => plugin_manifest_dir(),
    };
    let (mut items, skipped) = collect_plugin_items(resolved.as_deref());
    // 技能市场：skills 目录下全部 YAML 技能
    let skills_dir = crate::tools::skill_tools::resolve_skills_dir(&skills_dir_default());
    match crate::skills::loader::SkillLoader::new(&skills_dir).load_all() {
        Ok(skills) => {
            for skill in skills {
                items.push(json!({
                    "name": skill.name,
                    "version": skill.version,
                    "description": skill.description,
                    "kind": "skill",
                    "author": String::new(),
                }));
            }
        }
        Err(e) => {
            tracing::warn!(
                dir = %skills_dir.display(),
                error = %e,
                "技能目录装载失败：本次市场视图将不含技能条目"
            );
        }
    }
    items.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    (items, skipped)
}

/// 市场条目（插件 + 技能）：用默认候选目录（[`plugin_manifest_dir`]）。
pub(crate) fn market_items() -> Vec<Value> {
    market_items_report_in(None).0
}

/// deepseek harness 生态条目（来源蒸馏表）：把 `third_party/{harness,computer_use}`
/// 的 27 个上游仓库映射为可浏览市场条目。与本地插件清单（kind=plugin）/技能（kind=skill）
/// 并列展示，统一市场视图；`source` 标注生态归属，`categories` 供前端 facet 过滤。
/// 离线诚实：仅静态蒸馏表，不做网络抓取。
pub(crate) fn harness_catalog_items() -> Vec<Value> {
    use crate::harness::distill::{CapabilityTag, RepoCategory, UpstreamRepo, UPSTREAM_REPOS};
    let mut items: Vec<Value> = Vec::new();
    for r in UPSTREAM_REPOS {
        let _: &UpstreamRepo = r;
        let caps: Vec<&str> = r
            .capabilities
            .iter()
            .map(|c: &CapabilityTag| c.as_str())
            .collect();
        // deepseek 对齐：仓库名含 deepseek 或能力含 auto_evolve（deepseek-auto-evolving-harness）
        // 或蒸馏去向含 harness::evolution → 标记为 deepseek 生态友好。
        let deepseek_aligned = r.name.to_lowercase().contains("deepseek")
            || r.distilled_via.to_lowercase().contains("evolution")
            || r.capabilities
                .iter()
                .any(|c| matches!(c, CapabilityTag::AutoEvolve));
        items.push(json!({
            "name": r.name,
            "version": String::new(),
            "description": format!("{} 上游仓库 · 主语言 {} · 许可 {}", r.category.as_str(), r.primary_language, r.license),
            "kind": "harness_repo",
            "author": String::new(),
            "source": "deepseek-harness",
            "category": r.category.as_str(),
            "ecosystem": if matches!(r.category, RepoCategory::ComputerUse) { "computer_use" } else { "harness" },
            "primary_language": r.primary_language,
            "license": r.license,
            "capabilities": caps,
            "distilled_via": r.distilled_via,
            "deepseek_aligned": deepseek_aligned,
            "installed": false,
        }));
    }
    items.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    items
}

/// `market.harness`：deepseek harness 生态浏览目录（27 个上游仓库蒸馏条目）。
pub(crate) fn market_harness() -> Value {
    let items = harness_catalog_items();
    json!({ "items": items, "total": items.len(), "hint": "条目来自上游蒸馏表（third_party 27 仓库），导入/落地用 market.import（红线审计）+ market.install（收录）" })
}

/// `market.categories`：市场 facet 计数（本地插件/技能 + harness 生态合并视图）。
/// 返回 `by_kind` / `by_category` / `by_ecosystem` 三组分面，前端直接渲染过滤 chips。
pub(crate) fn market_categories() -> Value {
    let all: Vec<Value> = market_items()
        .into_iter()
        .chain(harness_catalog_items())
        .collect();
    let mut by_kind: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut by_category: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    let mut by_ecosystem: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    for it in &all {
        let kind = it["kind"].as_str().unwrap_or("unknown").to_string();
        *by_kind.entry(kind).or_insert(0) += 1;
        if let Some(cat) = it["category"].as_str() {
            *by_category.entry(cat.to_string()).or_insert(0) += 1;
        }
        if let Some(eco) = it["ecosystem"].as_str() {
            *by_ecosystem.entry(eco.to_string()).or_insert(0) += 1;
        }
    }
    json!({
        "total": all.len(),
        "by_kind": by_kind,
        "by_category": by_category,
        "by_ecosystem": by_ecosystem,
    })
}

/// 技能目录默认配置（与编排器注册同源）。
pub(crate) fn skills_dir_default() -> String {
    crate::persistence::config::Config::default()
        .skills
        .skills_dir
        .clone()
}

/// `market.list`：DSH 插件市场（本地清单 + 技能市场 + deepseek harness 生态）。
///
/// D1 纯增量：新增 `skipped: [{file, reason}]` —— 逐条列出被跳过的插件清单
/// （读目录/读文件/解析失败）。既有 `items` / `total` 名与类型不变。
pub(crate) fn market_list() -> Value {
    let (mut items, skipped) = market_items_report_in(None);
    // 合并 deepseek harness 生态条目（source 标注，kind=harness_repo），统一市场视图。
    items.extend(harness_catalog_items());
    json!({ "items": items, "total": items.len(), "skipped": skipped })
}

/// `market.search <q>`：按名称/描述模糊过滤。
pub(crate) fn market_search(params: &Value) -> Value {
    let q = params
        .get("q")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .unwrap_or_default();
    let items: Vec<Value> = market_items()
        .into_iter()
        .filter(|i| {
            q.is_empty()
                || i["name"].as_str().unwrap_or("").to_lowercase().contains(&q)
                || i["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        })
        .collect();
    json!({ "items": items, "total": items.len() })
}

/// `market.install <name>`：清单解析校验 + 收录（运行期注册由网关重启生效，
/// 诚实回报不做假成功）。
pub(crate) fn market_install(params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return json!({ "success": false, "message": "market.install 需要 name" });
    };
    let Some(dir) = plugin_manifest_dir() else {
        return json!({ "success": false, "message": "插件清单目录不可用" });
    };
    let path = dir.join(format!("{name}.plugin.json"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return json!({ "success": false, "message": format!("未找到插件清单 {name}") });
    };
    match crate::plugins::manifest::PluginManifest::from_json(&text) {
        Ok(m) => json!({
            "success": true,
            "message": format!("插件 {} v{} 校验通过并已收录（运行期注册随网关重启生效）", m.name, m.version),
        }),
        Err(e) => json!({ "success": false, "message": format!("清单解析失败: {e}") }),
    }
}

/// `market.upgrade <name>`：同插件新版本 manifest 原子替换（语义化升序校验）。
/// 参数: `name`(必填) + `content`(新 manifest JSON) 或 `path`(新 manifest 文件路径)。
/// 版本未升序 / 名称不匹配 / 解析失败 → 诚实失败，不做假成功；原子写（tmp+rename）。
pub(crate) fn market_upgrade(params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return json!({ "success": false, "message": "market.upgrade 需要 name" });
    };
    let Some(dir) = plugin_manifest_dir() else {
        return json!({ "success": false, "message": "插件清单目录不可用" });
    };
    let content = match params.get("content").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => match params.get("path").and_then(Value::as_str) {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(c) => c,
                Err(e) => {
                    return json!({ "success": false, "message": format!("读取新清单失败: {e}") })
                }
            },
            None => {
                return json!({ "success": false, "message": "market.upgrade 需要 content 或 path" })
            }
        },
    };
    upgrade_manifest(name, &dir, &content)
}

/// 执行 upgrade 核心：解析→名称一致→语义化升序→原子替换。
fn upgrade_manifest(name: &str, dir: &std::path::Path, content: &str) -> Value {
    let Ok(new_m) = crate::plugins::manifest::PluginManifest::from_json(content) else {
        return json!({ "success": false, "message": "新清单解析失败" });
    };
    if new_m.name != name {
        return json!({
            "success": false,
            "message": format!("新清单名称 {} 与 {name} 不一致", new_m.name),
        });
    }
    let path = dir.join(format!("{name}.plugin.json"));
    let Ok(old_text) = std::fs::read_to_string(&path) else {
        return json!({ "success": false, "message": format!("未找到现有清单 {name}") });
    };
    let Ok(old_m) = crate::plugins::manifest::PluginManifest::from_json(&old_text) else {
        return json!({ "success": false, "message": "现有清单解析失败" });
    };
    if !semver_gt(&new_m.version, &old_m.version) {
        return json!({
            "success": false,
            "message": format!("版本未升序（{} → {}），不做假成功", old_m.version, new_m.version),
        });
    }
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        return json!({ "success": false, "message": format!("写入临时清单失败: {e}") });
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return json!({ "success": false, "message": format!("原子替换失败: {e}") });
    }
    json!({ "success": true, "message": format!("插件 {} 已升级 {} → {}", name, old_m.version, new_m.version) })
}

/// 语义化版本（主.次.补丁，允许 v 前缀）比较：a > b？
fn semver_gt(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let pa = parse(a);
    let pb = parse(b);
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// `market.audit`：插件市场安全审计（本地清单全量或单插件）。
/// 红线扫描（AGENTS 不变量）：清单含 `system_prompt` / `full_context` 字段 = 全上下文重注入风险；
/// 高危能力扫描：exec/shell/network/delete 等能力声明 → risk 标注。纯静态、零网络，可批量。
///
/// D1 同口径：读目录 / 读目录项 / 读清单文件失败 → `tracing::warn!` + 记入纯增量字段
/// `skipped: [{file, reason}]`（既有字段名与类型不变）。
///
/// 裁决（R66 二轮）：**源目录整体不可读/不存在 → `ok: false` + `error`（含目录路径与原因）**，
/// 因为 `ok:true` + `items:[]` 在「连源目录都读不到」时是一种**新的静默**；而 webui 的
/// `runAudit` 对 `ok===false` 已会显示错误。**逐份清单解析失败仍 `ok: true`**（审计确实
/// 跑过，只是有坏件，该坏件以 `verdict="broken"` 出现在 `items` 里 —— 故**不计入
/// `skipped`**，见 [`skipped_record`] 的语义定义）—— 两条边界已在单测里分别钉死。
pub(crate) fn market_audit(params: &Value) -> Value {
    let name_filter = param_str(params, "name", "name", "").to_string();
    let dir = plugin_manifest_dir();
    market_audit_in(dir.as_deref(), &name_filter)
}

/// `market.audit` 主体：目录由参数注入（D4 同口径，测试不依赖进程 env/cwd）。
fn market_audit_in(dir: Option<&Path>, name_filter: &str) -> Value {
    let Some(dir) = dir else {
        return json!({ "ok": false, "error": "插件清单目录不可用" });
    };
    let mut items: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut total_risks = 0usize;
    let mut total_suspects = 0usize;
    let risk_kw = [
        "exec",
        "shell",
        "bash",
        "cmd",
        "network",
        "http",
        "delete_file",
        "file_delete",
        "rm",
        "sudo",
        "registry",
        "install",
    ];
    // D1 同口径：读目录 / 读目录项 / 读文件失败一律响亮告警并记入 `skipped`，
    // 不再 `if let Ok(..)` + `.flatten()` 静默丢弃（旧行为会「审计结果凭空少几条」）。
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            // 源目录整体不可读 → 诚实失败（ok:false），不是「审计通过、零问题」。
            tracing::warn!(
                dir = %dir.display(),
                error = %e,
                "读取插件清单目录失败：market.audit 返回 ok=false（源目录不可读）"
            );
            skipped.push(skipped_record(dir, format!("读取目录失败: {e}")));
            return json!({
                "ok": false,
                "error": format!("插件清单目录 {} 不可读: {e}", dir.display()),
                "items": items,
                "total": items.len(),
                "total_suspects": total_suspects,
                "total_risks": total_risks,
                "skipped": skipped,
            });
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %e,
                    "读取插件清单目录项失败（跳过该项）"
                );
                skipped.push(skipped_record(dir, format!("读取目录项失败: {e}")));
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "读取插件清单文件失败（跳过该清单，已记入 skipped）"
                );
                skipped.push(skipped_record(&path, format!("读取文件失败: {e}")));
                continue;
            }
        };
        match crate::plugins::manifest::PluginManifest::from_json(&text) {
            Ok(m) => {
                if !name_filter.is_empty() && m.name != name_filter {
                    continue;
                }
                let suspects: Vec<String> = [
                    ("system_prompt", "全上下文重注入（红线 #1）"),
                    ("full_context", "全上下文重注入（红线 #1）"),
                ]
                .iter()
                .filter(|(k, _)| text.contains(k))
                .map(|(k, d)| format!("{}: {}", k, d))
                .collect();
                let risks: Vec<String> = m
                    .capabilities
                    .iter()
                    .filter(|c| risk_kw.iter().any(|k| c.to_lowercase().contains(k)))
                    .map(|c| format!("高危能力声明: {c}"))
                    .collect();
                total_suspects += suspects.len();
                total_risks += risks.len();
                items.push(json!({
                    "name": m.name,
                    "version": m.version,
                    "capabilities": m.capabilities.len(),
                    "hooks": m.hooks.len(),
                    "subagents": m.subagents.len(),
                    "skills": m.skills.len(),
                    "suspects": suspects,
                    "risks": risks,
                    "verdict": if !suspects.is_empty() || !risks.is_empty() { "warn" } else { "clean" },
                }));
            }
            Err(e) => {
                // 解析失败：**仍有可见条目**（`verdict="broken"`），故**不进 `skipped`**
                // —— `skipped` 的唯一含义是「未出现在 items 里的条目」（见 `skipped_record`），
                // 这里再塞一条属双重披露，会让 skipped 语义变模糊。只补响亮告警。
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "插件清单解析失败：审计结果标记为 broken（该清单仍在 items 里，不计入 skipped）"
                );
                items.push(json!({ "name": path.file_stem().and_then(|s| s.to_str()).unwrap_or("?"), "version": "", "capabilities": 0, "hooks": 0, "subagents": 0, "skills": 0, "suspects": [], "risks": [format!("清单解析失败: {e}")], "verdict": "broken" }));
            }
        }
    }
    items.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    json!({ "ok": true, "items": items, "total": items.len(), "total_suspects": total_suspects, "total_risks": total_risks, "skipped": skipped })
}

/// `market.import`：导入外部插件清单（第三方/第四方生态）。
/// 默认拒绝含红线字段的清单（全上下文重注入）；`ack: true` 显式确认后允许收录（仍打标）。
pub(crate) fn market_import(params: &Value) -> Value {
    let Some(content) = params.get("content").and_then(Value::as_str) else {
        return json!({ "ok": false, "error": "market.import 需要 content（插件清单 JSON 文本）" });
    };
    let ack = params.get("ack").and_then(Value::as_bool).unwrap_or(false);
    let Some(dir) = plugin_manifest_dir() else {
        return json!({ "ok": false, "error": "插件清单目录不可用" });
    };
    let m = match crate::plugins::manifest::PluginManifest::from_json(content) {
        Ok(m) => m,
        Err(e) => return json!({ "ok": false, "error": format!("清单解析失败: {e}") }),
    };
    if m.name.contains("/")
        || m.name.contains(&std::path::MAIN_SEPARATOR.to_string())
        || m.name.contains("..")
    {
        return json!({ "ok": false, "error": "插件名含路径字符，已拒绝（防目录穿越）" });
    }
    let leak = content.contains("system_prompt") || content.contains("full_context");
    if leak && !ack {
        return json!({ "ok": false, "error": "清单包含 system_prompt/full_context（红线 #1 全上下文重注入），已拒绝；如确认安全请带 ack:true 重试（仍会打标）" });
    }
    let target = dir.join(format!("{}.plugin.json", m.name));
    let tmp = dir.join(format!("{}.plugin.json.tmp", m.name));
    if let Err(e) = std::fs::write(&tmp, content) {
        return json!({ "ok": false, "error": format!("写入临时清单失败: {e}") });
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return json!({ "ok": false, "error": format!("原子落盘失败: {e}") });
    }
    json!({ "ok": true, "name": m.name, "version": m.version, "redline_flagged": leak, "message": if leak { "已收录并打标（红线字段存在，运行期注册前需人工复核）" } else { "导入成功（运行期注册随网关重启生效）" } })
}

/// `market.export`：导出插件清单文本（导入/分享/迁移用）。
pub(crate) fn market_export(params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return json!({ "ok": false, "error": "market.export 需要 name" });
    };
    let Some(dir) = plugin_manifest_dir() else {
        return json!({ "ok": false, "error": "插件清单目录不可用" });
    };
    let path = dir.join(format!("{name}.plugin.json"));
    match std::fs::read_to_string(&path) {
        Ok(content) => json!({ "ok": true, "name": name, "content": content }),
        Err(e) => json!({ "ok": false, "error": format!("未找到清单 {name}: {e}") }),
    }
}

/// `market.install.batch`：批量安装（逐个走 market.install 同款校验），返回逐项结果。
pub(crate) fn market_install_batch(params: &Value) -> Value {
    let Some(names) = params.get("names").and_then(Value::as_array) else {
        return json!({ "ok": false, "error": "market.install.batch 需要 names 数组" });
    };
    let mut results = Vec::new();
    let mut ok_all = true;
    for n in names.iter().take(64) {
        let Some(name) = n.as_str() else { continue };
        let r = market_install(&json!({ "name": name }));
        let success = r["success"].as_bool().unwrap_or(false);
        ok_all = ok_all && success;
        results.push(json!({ "name": name, "success": success, "message": r["message"].as_str().unwrap_or("") }));
    }
    json!({ "ok": ok_all, "results": results, "total": results.len() })
}

#[cfg(test)]
mod market_plugin_tests {
    use super::{
        collect_plugin_items, market_items_report_in, plugin_market_item, resolve_plugins_dir,
    };
    use crate::plugins::manifest::PluginManifest;
    use serde_json::Value;
    use std::path::{Path, PathBuf};

    /// 随仓库的插件清单目录：`CARGO_MANIFEST_DIR` = `<repo>/core`，上溯一层即仓库根。
    ///
    /// **不依赖测试进程的 cwd**。D3 之后 `plugin_manifest_dir()` 也能从 `cwd=<repo>/core`
    /// 上溯解析到同一目录；但测试仍优先直接注入目录（D4：不 `set_var`、不读进程 env），
    /// 使断言与「测试进程 cwd / env」完全解耦 —— 旧实现下 `plugin_manifest_dir()` 在
    /// 测试期解析为 `None`，直接断言 `market_items()` 的插件数会因「0 条」而**空转通过**。
    fn manifest_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("plugins")
            .join("manifests")
    }

    /// 唯一临时目录（不新增 dev-dependency：pid + 纳秒时间戳）。
    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("mg-market-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("创建临时目录 {} 失败: {e}", dir.display()));
        dir
    }

    /// 保证不存在的深层路径：从 `base` 上溯 4 层（[`resolve_plugins_dir`] 的上界）
    /// 也不会走出 `base` 自身，故不会误命中机器上任何真实目录。
    fn deep_nonexistent(tag: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("mg-no-such-{tag}-{}", std::process::id()))
            .join("a")
            .join("b")
            .join("c")
            .join("d")
    }

    fn canonical(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|e| panic!("规范化 {} 失败: {e}", p.display()))
    }

    fn load_manifests() -> Vec<PluginManifest> {
        let dir = manifest_dir();
        let rd = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("清单目录不可读 {}: {e}", dir.display()));
        let mut out = Vec::new();
        // D5 同口径：逐项 io 错误显式 panic，不用 `.flatten()` 静默丢弃。
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
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));
            out.push(
                PluginManifest::from_json(&text)
                    .unwrap_or_else(|e| panic!("解析 {} 失败: {e:?}", path.display())),
            );
        }
        assert!(
            !out.is_empty(),
            "清单目录 {} 下未找到任何 *.plugin.json",
            dir.display()
        );
        out
    }

    /// S3：`plugin_market_item()` 对随仓库清单**全部**产出 `mode == "declarative-only"`，
    /// 且既有 5 字段（name/version/description/kind/author）一个不少。
    #[test]
    fn market_plugin_items_are_all_declarative_only_today() {
        let manifests = load_manifests();
        assert!(
            manifests.len() >= 16,
            "应至少 16 份清单（基线），实际 {}",
            manifests.len()
        );
        for m in &manifests {
            let item = plugin_market_item(m);
            assert_eq!(item["kind"].as_str(), Some("plugin"));
            assert_eq!(
                item["mode"].as_str(),
                Some("declarative-only"),
                "清单 {} 当前应为 declarative-only，实际 {:?}",
                m.name,
                item["mode"]
            );
            assert!(item["entry"].is_null(), "清单 {} 不应声明 entry", m.name);
            assert_eq!(
                item["activation"].as_str(),
                Some("restart"),
                "清单 {} activation 应为 restart",
                m.name
            );
            for k in ["name", "version", "description", "kind", "author"] {
                assert!(
                    item.get(k).is_some(),
                    "既有字段 `{k}` 丢失（纯增量被破坏）@ {}",
                    m.name
                );
            }
        }
        println!(
            "S3：{} 份清单的 market 条目全部 declarative-only",
            manifests.len()
        );
    }

    /// S3：直接断言真市场条目负载（插件 + 技能）—— 指向仓库清单目录后，
    /// 全部 `kind == "plugin"` 条目必须 `mode == "declarative-only"`。
    ///
    /// D4：**不再 `set_var`**（进程级 env 在并行测试线程下与 `market_items()` /
    /// `market_sources()` 竞态，Rust 已不推荐多线程 `set_var`）；改为注入目录，
    /// 对外行为与负载结构不变。
    #[test]
    fn market_items_payload_pins_declarative_only() {
        let dir = manifest_dir();
        let (items, skipped) = market_items_report_in(Some(&dir));

        let plugins: Vec<&Value> = items
            .iter()
            .filter(|i| i["kind"].as_str() == Some("plugin"))
            .collect();
        assert!(
            plugins.len() >= 16,
            "market_items 负载应含至少 16 个 plugin 条目（基线），实际 {}（items total {}，dir {}）",
            plugins.len(),
            items.len(),
            dir.display()
        );
        for it in &plugins {
            assert_eq!(
                it["mode"].as_str(),
                Some("declarative-only"),
                "plugin 条目 {} 非 declarative-only",
                it["name"]
            );
        }
        assert!(
            skipped.is_empty(),
            "仓库清单目录不应有坏清单，实际 skipped={skipped:?}"
        );
        println!(
            "S3：market_items 负载含 {} 个 plugin 条目，全部 declarative-only（skipped=0）",
            plugins.len()
        );
    }

    /// D1：坏清单**不得**再悄悄消失 —— 好清单仍在、坏清单进 `skipped` 且 `reason` 非空。
    #[test]
    fn bad_manifest_goes_to_skipped_not_silently_dropped() {
        let dir = unique_tmp_dir("skipped");
        std::fs::write(
            dir.join("good.plugin.json"),
            r#"{"name":"good-plugin","version":"1.0.0","description":"好清单"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.plugin.json"), "{ this is not json").unwrap();
        // 非 json 后缀不是清单，不应出现在 items，也不应被当成「坏清单」。
        std::fs::write(dir.join("README.md"), "not a manifest").unwrap();

        let (items, skipped) = collect_plugin_items(Some(&dir));

        assert_eq!(
            items.len(),
            1,
            "好清单必须仍在（旧实现会连它一起吞掉），实际 items={items:?}"
        );
        assert_eq!(items[0]["name"].as_str(), Some("good-plugin"));
        assert_eq!(items[0]["kind"].as_str(), Some("plugin"));
        for k in ["name", "version", "description", "kind", "author"] {
            assert!(items[0].get(k).is_some(), "既有字段 `{k}` 丢失");
        }
        assert_eq!(
            skipped.len(),
            1,
            "坏清单必须出现在 skipped（读文件/解析失败不得静默丢弃），实际 skipped={skipped:?}"
        );
        let file = skipped[0]["file"].as_str().unwrap_or("");
        assert!(
            file.ends_with("broken.plugin.json"),
            "skipped[0].file 应为坏清单路径，实际 {file}"
        );
        let reason = skipped[0]["reason"].as_str().unwrap_or("");
        assert!(
            !reason.is_empty(),
            "skipped[0].reason 必须非空，实际 {skipped:?}"
        );

        // 目录本身不可读（不存在）→ 同样响亮：items 空 + skipped 一条（不是「静默 0 条」）。
        let missing = dir.join("no-such-subdir");
        let (items2, skipped2) = collect_plugin_items(Some(&missing));
        assert!(items2.is_empty(), "不存在的目录不应产出条目");
        assert_eq!(
            skipped2.len(),
            1,
            "不可读目录必须进 skipped，实际 {skipped2:?}"
        );
        assert!(!skipped2[0]["reason"].as_str().unwrap_or("").is_empty());

        // 无目录（None）时不产出、也不误报：告警由 plugin_manifest_dir() 负责。
        let (items3, skipped3) = collect_plugin_items(None);
        assert!(items3.is_empty() && skipped3.is_empty());

        println!(
            "D1：好清单保留 {} 条；坏清单 → skipped={}（reason={:?}）",
            items.len(),
            skipped.len(),
            reason
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// D3：`cwd = <repo>/core`、无 env → 仍能上溯解析到 `<repo>/plugins/manifests`。
    ///
    /// 复现 QA 真机缺陷：旧实现只试 `cwd/plugins/manifests`（= `<repo>/core/plugins/…`，
    /// 不存在）与 exe 相对路径 → `None` → `market.list` 的插件静默变 0 条（total 65→49）。
    #[test]
    fn resolve_plugins_dir_walks_up_from_repo_core_cwd() {
        let repo = canonical(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
        let cwd = repo.join("core"); // 模拟「从 core/ 启动」
        let got = resolve_plugins_dir(None, &cwd, &deep_nonexistent("exe"));
        let want = canonical(&repo.join("plugins").join("manifests"));
        assert_eq!(
            got.as_deref().map(canonical),
            Some(want),
            "cwd=<repo>/core 且无 env 时必须上溯命中 <repo>/plugins/manifests，实际 {got:?}"
        );
        println!(
            "D3：cwd={} → {}",
            cwd.display(),
            got.expect("应命中").display()
        );
    }

    /// D3：`MARGUERITE_PLUGINS_DIR` 覆盖优先；空值 / 不存在目录不生效（继续下溯）。
    #[test]
    fn resolve_plugins_dir_env_override_wins() {
        let override_dir = unique_tmp_dir("env-override");
        let bogus = deep_nonexistent("cwd");

        let got = resolve_plugins_dir(Some(&override_dir), &bogus, &bogus);
        assert_eq!(
            got.as_deref().map(canonical),
            Some(canonical(&override_dir)),
            "env 覆盖存在且是目录时必须优先"
        );
        // 空 env / env 指向不存在目录 → 沿用旧语义「忽略覆盖」，继续候选上溯
        // （此处候选全不存在 → None，而不是把不存在路径当成命中）。
        assert_eq!(resolve_plugins_dir(Some(Path::new("")), &bogus, &bogus), None);
        assert_eq!(resolve_plugins_dir(Some(&bogus), &bogus, &bogus), None);

        std::fs::remove_dir_all(&override_dir).ok();
    }

    /// D3：全部候选都不存在 → `None`（诚实失败，不返回伪目录）。
    #[test]
    fn resolve_plugins_dir_none_when_nothing_matches() {
        let cwd = deep_nonexistent("cwd-none");
        let exe = deep_nonexistent("exe-none");
        assert_eq!(resolve_plugins_dir(None, &cwd, &exe), None);
        assert_eq!(resolve_plugins_dir(Some(Path::new("")), &cwd, &exe), None);
    }

    /// 裁决 1 边界 A：**源目录整体不可读** → `ok:false` + `error`（含目录路径与原因）+ `skipped`。
    #[test]
    fn market_audit_unreadable_source_dir_is_ok_false() {
        let missing = unique_tmp_dir("audit-unreadable").join("no-such-subdir");
        let v = super::market_audit_in(Some(&missing), "");

        assert_eq!(
            v["ok"].as_bool(),
            Some(false),
            "源目录不可读必须 ok:false（ok:true + items:[] 是新的静默），实际 {v}"
        );
        let err = v["error"].as_str().unwrap_or("");
        assert!(
            err.contains(&missing.display().to_string()),
            "error 必须含目录路径，实际 {err}"
        );
        assert!(!err.is_empty(), "error 必须非空");
        assert_eq!(
            v["skipped"].as_array().map(Vec::len),
            Some(1),
            "不可读目录必须记入 skipped，实际 {v}"
        );
        assert!(
            !v["skipped"][0]["reason"].as_str().unwrap_or("").is_empty(),
            "skipped[0].reason 必须非空"
        );
        // 既有字段名与类型不变（纯增量 + ok 值语义变更）
        for k in [
            "ok",
            "error",
            "items",
            "total",
            "total_suspects",
            "total_risks",
            "skipped",
        ] {
            assert!(v.get(k).is_some(), "既有字段 `{k}` 丢失");
        }
        assert_eq!(v["items"].as_array().map(Vec::len), Some(0));
        println!("裁决1-A：源目录不可读 → ok=false，error={err}");
    }

    /// 裁决 1 边界 B（**不得改坏**）：逐份清单解析失败 → **仍 `ok:true`**；
    /// 该坏件以 `verdict="broken"` 出现在 `items` 里 → **`skipped == 0`**
    /// （`skipped` 的唯一含义是「未出现在 items 里的条目」，见 `skipped_record`）。
    #[test]
    fn market_audit_broken_manifest_still_ok_true() {
        let dir = unique_tmp_dir("audit-broken");
        std::fs::write(
            dir.join("good.plugin.json"),
            r#"{"name":"audit-good","version":"1.0.0","description":"好清单"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.plugin.json"), "{ this is not json").unwrap();

        let v = super::market_audit_in(Some(&dir), "");

        assert_eq!(
            v["ok"].as_bool(),
            Some(true),
            "审计确实跑过（只是有坏件）→ 必须仍 ok:true，实际 {v}"
        );
        assert!(
            v.get("error").is_none(),
            "ok:true 时不应带 error，实际 {:?}",
            v.get("error")
        );
        // 逐份都被审计：好件 clean + 坏件 broken（坏件有可见条目，未被丢弃）
        assert_eq!(v["total"].as_u64(), Some(2), "实际 {v}");
        let verdicts: Vec<&str> = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["verdict"].as_str().unwrap_or("?"))
            .collect();
        assert!(
            verdicts.contains(&"broken"),
            "坏清单须有可见的 broken 条目，实际 {verdicts:?}"
        );
        assert!(
            verdicts.contains(&"clean"),
            "好清单须有可见的 clean 条目，实际 {verdicts:?}"
        );
        // 坏件仍在 items 里 → 不计入 skipped（撤回双重披露，语义：skipped = 未出现在 items 里的条目）
        assert_eq!(
            v["skipped"].as_array().map(Vec::len),
            Some(0),
            "坏件已在 items 里（verdict=broken），不得重复披露到 skipped，实际 {v}"
        );
        println!("裁决1-B：逐份解析失败 → ok=true，verdicts={verdicts:?}，skipped=0");
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// `market.sources`：deepseek harness 生态下载源（本地检出仓库 + git remote 枚举）。
/// 离线诚实：不做网络抓取，仅枚举已检出仓库的 remote 与本地插件清单目录。
///
/// D1 同口径：读目录 / 读目录项失败 → `tracing::warn!`（带路径 + 原因）+ 记入纯增量
/// `skipped: [{file, reason}]`；既有 `ok` / `items` / `total` / `hint` 不变。
/// 候选目录**不存在**属预期（cwd/exe 相对路径天然可能没有），仅 `debug!` 记录。
pub(crate) fn market_sources() -> Value {
    let mut items: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("MARGUERITE_THIRD_PARTY_DIR").map(PathBuf::from) {
        if p.as_os_str().is_empty() {
            tracing::debug!("MARGUERITE_THIRD_PARTY_DIR 为空，忽略该覆盖");
        } else {
            roots.push(p);
        }
    }
    match std::env::current_dir() {
        Ok(cwd) => roots.push(cwd.join("third_party").join("external")),
        Err(e) => tracing::warn!(
            error = %e,
            "读取 cwd 失败：下载源将不含 cwd 下的 third_party/external"
        ),
    }
    match std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        Some(dir) => {
            roots.push(dir.join("third_party").join("external"));
            if let Some(gp) = dir.parent() {
                roots.push(gp.join("third_party").join("external"));
            }
        }
        None => tracing::warn!(
            "读取 current_exe 失败：下载源将不含 exe 相对 third_party/external"
        ),
    }
    for root in roots {
        let entries = match std::fs::read_dir(&root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(dir = %root.display(), "下载源候选目录不存在，跳过");
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    dir = %root.display(),
                    error = %e,
                    "读取下载源候选目录失败：该源下的已检出仓库将被漏计"
                );
                skipped.push(skipped_record(&root, format!("读取目录失败: {e}")));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        dir = %root.display(),
                        error = %e,
                        "读取下载源目录项失败（跳过该项）"
                    );
                    skipped.push(skipped_record(&root, format!("读取目录项失败: {e}")));
                    continue;
                }
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let git_cfg = path.join(".git").join("config");
            let remote = match std::fs::read_to_string(&git_cfg) {
                Ok(t) => parse_git_remote(&t),
                // 非 git 检出（无 .git/config）属预期：条目仍在，只是没有 remote url。
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    tracing::warn!(
                        file = %git_cfg.display(),
                        error = %e,
                        "读取 git config 失败：该源的 remote url 将为空（条目保留）"
                    );
                    None
                }
            };
            items.push(json!({ "name": name, "path": path.display().to_string(), "url": remote, "kind": "external_repo" }));
        }
    }
    if let Some(dir) = plugin_manifest_dir() {
        items.push(json!({ "name": "本地插件清单目录", "path": dir.display().to_string(), "url": Value::Null, "kind": "manifest_dir" }));
    }
    items.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    json!({ "ok": true, "items": items, "total": items.len(), "skipped": skipped, "hint": "下载源 = 本地检出仓库（git remote 枚举）+ 本地清单目录；导入清单用 market.import，收录安装用 market.install（可批量）" })
}

/// 解析 `.git/config` 的 origin remote url。
fn parse_git_remote(config_text: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config_text.lines() {
        let t = line.trim();
        if t.starts_with("[remote") && t.contains("origin") {
            in_origin = true;
            continue;
        }
        if in_origin && t.starts_with("[") {
            break;
        }
        if in_origin {
            if let Some(rest) = t.strip_prefix("url = ") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod market_upgrade_tests {
    use super::{parse_git_remote, semver_gt};

    #[test]
    fn parse_git_remote_extracts_origin_url() {
        let cfg = "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = https://github.com/example/openclaw.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";
        assert_eq!(
            parse_git_remote(cfg).as_deref(),
            Some("https://github.com/example/openclaw.git")
        );
        assert_eq!(
            parse_git_remote("[remote \"upstream\"]\nurl = x\n"),
            None,
            "非 origin 不算"
        );
    }

    #[test]
    fn semver_gt_orders_correctly() {
        assert!(semver_gt("1.2.0", "1.1.9"));
        assert!(semver_gt("2.0.0", "1.99.99"));
        assert!(semver_gt("v1.2.0", "1.1.9"), "允许 v 前缀");
        assert!(semver_gt("1.10.0", "1.9.0"), "主次数字升序比字符串大");
        assert!(semver_gt("1.2", "1.1.5"), "两段式版本");
        assert!(!semver_gt("1.2.0", "1.2.0"), "相同版本不视为升级");
        assert!(!semver_gt("1.1.9", "1.2.0"));
        assert!(!semver_gt("0.9.0", "1.0.0"));
    }
}

#[cfg(test)]
mod market_harness_tests {
    use super::{harness_catalog_items, market_categories, market_list};

    #[test]
    fn harness_catalog_covers_all_27_upstream_repos() {
        // 蒸馏表须覆盖全部 27 个仓库（harness/distill.rs 守卫）；市场目录必须同源一致。
        let items = harness_catalog_items();
        assert_eq!(items.len(), 27, "harness 生态目录须覆盖全部 27 上游仓库");
        // 每条必须是 harness_repo kind + deepseek-harness source，且含 distilled_via 与能力标签。
        for it in &items {
            assert_eq!(it["kind"].as_str(), Some("harness_repo"));
            assert_eq!(it["source"].as_str(), Some("deepseek-harness"));
            assert!(
                !it["distilled_via"].as_str().unwrap_or("").is_empty(),
                "每条须有蒸馏去向"
            );
            assert!(
                it["capabilities"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false),
                "每条须有能力标签"
            );
        }
    }

    #[test]
    fn harness_auto_evolving_is_deepseek_aligned() {
        let items = harness_catalog_items();
        let auto = items
            .iter()
            .find(|i| i["name"].as_str() == Some("deepseek-auto-evolving-harness"))
            .expect("deepseek-auto-evolving-harness 必须在目录中");
        assert!(
            auto["deepseek_aligned"].as_bool().unwrap_or(false),
            "含 deepseek 名须标记为对齐"
        );
    }

    #[test]
    fn market_list_merges_harness_entries_tagged() {
        let v = market_list();
        let items = v["items"].as_array().expect("items 为数组");
        let harness = items
            .iter()
            .filter(|i| i["source"].as_str() == Some("deepseek-harness"))
            .count();
        assert_eq!(harness, 27, "market.list 须合并 27 条 harness 生态条目");
    }

    /// D1：`market.list` 负载**纯增量**暴露 `skipped`（既有 `items`/`total` 不动）。
    #[test]
    fn market_list_payload_exposes_skipped_incrementally() {
        let v = market_list();
        for k in ["items", "total"] {
            assert!(v.get(k).is_some(), "既有字段 `{k}` 丢失（纯增量被破坏）");
        }
        assert!(
            v["skipped"].is_array(),
            "market.list 负载必须含 skipped 数组（逐项失败可观测），实际 {:?}",
            v.get("skipped")
        );
    }

    #[test]
    fn market_categories_exposes_facets() {
        let v = market_categories();
        let by_kind = v["by_kind"].as_object().expect("by_kind 为对象");
        assert_eq!(
            by_kind.get("harness_repo").cloned(),
            Some(serde_json::json!(27)),
            "facet 须含 27 条 harness_repo"
        );
        assert!(v["by_category"].as_object().is_some());
        assert!(v["by_ecosystem"].as_object().is_some());
    }
}
