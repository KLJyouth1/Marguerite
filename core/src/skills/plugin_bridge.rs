//! 插件技能桥接：把 `PluginManifest.skills` 声明的技能并入技能系统。
//!
//! ## 为什么需要
//!
//! `PluginManifest` 早就有 `skills: Vec<SkillRef>` 字段，`PluginSlot` 也会把
//! skill 槽装配进统一槽位；但 `SkillLoader` 只扫技能目录 —— 于是**插件声明的技能
//! 在 `/skills` 页面完全不可见**，两条技能通道各自为政。
//!
//! ## 诚实边界（重要）
//!
//! - `SkillRef` 只有 `{ name, from }`，**没有 prompt 正文**。
//! - 若 `from` 指向一个真实文件/目录（相对清单所在目录），我们复用
//!   [`parse_skill_md`] 解析出完整技能（`resolved = true`）。
//! - 否则只能登记为**引用**：`prompt_template` 为空，描述里明确写「仅引用」。
//!   前端/网关不得把这种条目描述成「可直接执行的技能」。
//! - `from` 拒绝绝对路径与 `..`（防止清单把加载范围引到技能根之外）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::skills::loader::parse_skill_md;
use crate::skills::registry::{Skill, SkillSource};

/// 一条已桥接的插件技能。
#[derive(Debug, Clone)]
pub struct BridgedSkill {
    /// 归一到技能系统的定义（引用型条目 prompt 为空）
    pub skill: Skill,
    /// 声明它的插件名
    pub plugin: String,
    /// 是否从 `from` 解析出了真实正文
    pub resolved: bool,
    /// 清单里的原始 `from`（相对路径）
    pub from: Option<String>,
}

/// 桥接报告（供 `skills.coverage` 如实展示）。
#[derive(Debug, Clone, Default)]
pub struct BridgeReport {
    pub manifests_scanned: usize,
    pub refs_total: usize,
    pub resolved: usize,
    pub unresolved: usize,
    /// (来源, 原因)：来源是 `<plugin>/<skill>` 或清单文件名
    pub skipped: Vec<(String, String)>,
}

fn skip(report: &mut BridgeReport, source: impl Into<String>, reason: impl Into<String>) {
    report.skipped.push((source.into(), reason.into()));
}

/// 扫描清单目录，收集全部插件技能。
///
/// `manifests_dir = None` → 返回空（调用方如未配置插件目录时不报错）。
pub fn collect(manifests_dir: Option<&Path>) -> (Vec<BridgedSkill>, BridgeReport) {
    let mut report = BridgeReport::default();
    let Some(dir) = manifests_dir else {
        return (Vec::new(), report);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        skip(&mut report, dir.display().to_string(), "manifests-dir-unreadable");
        return (Vec::new(), report);
    };

    let mut out: Vec<BridgedSkill> = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut manifest_paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".plugin.json"))
                .unwrap_or(false)
        })
        .collect();
    manifest_paths.sort();

    for path in manifest_paths {
        report.manifests_scanned += 1;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                skip(&mut report, path.display().to_string(), format!("read-failed: {e}"));
                continue;
            }
        };
        let manifest: crate::plugins::manifest::PluginManifest =
            match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(e) => {
                    skip(
                        &mut report,
                        path.display().to_string(),
                        format!("invalid-manifest: {e}"),
                    );
                    continue;
                }
            };
        for reference in &manifest.skills {
            let name = reference.name.trim();
            if name.is_empty() {
                skip(&mut report, manifest.name.clone(), "empty-skill-name");
                continue;
            }
            report.refs_total += 1;
            let tag = format!("{}/{}", manifest.name, name);
            if !names.insert(name.to_string()) {
                skip(&mut report, tag, "duplicate-name");
                continue;
            }

            let resolved = reference
                .from
                .as_deref()
                .and_then(|from| resolve_from(dir, &path, from, name, &mut report, &tag));

            match resolved {
                Some(skill) => {
                    report.resolved += 1;
                    out.push(BridgedSkill {
                        skill,
                        plugin: manifest.name.clone(),
                        resolved: true,
                        from: reference.from.clone(),
                    });
                }
                None => {
                    report.unresolved += 1;
                    out.push(BridgedSkill {
                        skill: reference_skill(&manifest, name, &path),
                        plugin: manifest.name.clone(),
                        resolved: false,
                        from: reference.from.clone(),
                    });
                }
            }
        }
    }
    (out, report)
}

/// 尝试把 `from` 解析为真实技能；失败返回 None（由调用方登记为引用）。
///
/// 安全边界：允许 `../`（常见插件布局 `manifests/x.plugin.json` + `../skills/xxx`），
/// 但 canonicalize 后必须仍在**插件根**（`manifests_dir` 的父目录）内 —— 逃逸即拒绝。
fn resolve_from(
    manifests_dir: &Path,
    manifest_path: &Path,
    from: &str,
    declared_name: &str,
    report: &mut BridgeReport,
    tag: &str,
) -> Option<Skill> {
    let rel = Path::new(from);
    if rel.is_absolute() {
        skip(report, tag, "absolute-from-rejected");
        return None;
    }
    // 相对清单文件所在目录解析（插件目录布局：manifests/x.plugin.json + ../skills/...）
    let base = manifest_path.parent().unwrap_or(manifests_dir);
    let target = base.join(rel);
    let (file, root) = if target.is_dir() {
        (target.join("SKILL.md"), target.clone())
    } else {
        (target.clone(), target.parent().map(Path::to_path_buf).unwrap_or(base.to_path_buf()))
    };
    if !file.is_file() {
        skip(report, tag, format!("from-not-found: {}", target.display()));
        return None;
    }
    // 逃逸校验：以插件根为界（不存在则退回清单目录）
    let boundary = manifests_dir.parent().unwrap_or(manifests_dir);
    let (Ok(bound), Ok(resolved_root)) = (boundary.canonicalize(), root.canonicalize()) else {
        skip(report, tag, "from-unresolvable");
        return None;
    };
    if !resolved_root.starts_with(&bound) {
        skip(report, tag, "from-escapes-plugin-root");
        return None;
    }
    let content = match std::fs::read_to_string(&file) {
        Ok(c) => c,
        Err(e) => {
            skip(report, tag, format!("from-read-failed: {e}"));
            return None;
        }
    };
    match parse_skill_md(&file, &content, &resolved_root) {
        Ok(mut skill) => {
            // 插件契约以声明的 name 为准（`skill.get` 按声明名查找）；
            // 解析出的名字仅用于日志，不静默改名。
            if skill.name != declared_name {
                skip(report, tag, format!("resolved-name-differs: {}", skill.name));
            }
            skill.name = declared_name.to_string();
            Some(skill)
        }
        Err(e) => {
            skip(report, tag, format!("from-parse-failed: {e}"));
            None
        }
    }
}

/// 构造「仅引用」条目：没有 prompt 正文，描述如实标注。
fn reference_skill(
    manifest: &crate::plugins::manifest::PluginManifest,
    name: &str,
    manifest_path: &Path,
) -> Skill {
    Skill {
        name: name.to_string(),
        description: format!(
            "由插件 {} 声明的技能引用（清单未内联 prompt；如需执行请在技能目录提供同名实现）",
            manifest.name
        ),
        version: manifest.version.clone(),
        author: manifest.author.clone(),
        intent: Vec::new(),
        tools: Vec::new(),
        prompt_template: String::new(),
        dependencies: Vec::new(),
        source: SkillSource::Local {
            path: manifest_path.to_path_buf(),
        },
        rating: 0.0,
        uses: 0,
        rank: crate::skills::registry::ranks::DEFAULT,
    }
}

/// 覆盖报告视图（`skills.coverage.plugins`）。
pub fn report_json(report: &BridgeReport) -> serde_json::Value {
    let skipped: Vec<serde_json::Value> = report
        .skipped
        .iter()
        .take(30)
        .map(|(source, reason)| json!({ "source": source, "reason": reason }))
        .collect();
    json!({
        "manifestsScanned": report.manifests_scanned,
        "refsTotal": report.refs_total,
        "resolved": report.resolved,
        "unresolved": report.unresolved,
        "skipped": skipped,
        "skippedTotal": report.skipped.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_plugin_bridge_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("manifests")).unwrap();
        dir
    }

    #[test]
    fn ref_without_from_is_honest_reference() {
        let dir = temp("ref");
        std::fs::write(
            dir.join("manifests/p.plugin.json"),
            r#"{"name":"p","version":"1.2.3","description":"d","skills":[{"name":"ghost-skill"}]}"#,
        )
        .unwrap();
        let (skills, report) = collect(Some(&dir.join("manifests")));
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].resolved);
        assert!(skills[0].skill.prompt_template.is_empty(), "引用型不得伪造 prompt");
        assert!(skills[0].skill.description.contains("技能引用"));
        assert_eq!(report.unresolved, 1);
        assert_eq!(report.resolved, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn ref_with_from_resolves_real_skill_md() {
        let dir = temp("resolve");
        let skill_dir = dir.join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: real-skill\ndescription: 真实技能\nversion: 9.9.9\n---\n\n正文内容\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("manifests/p.plugin.json"),
            r#"{"name":"p","version":"1.0.0","description":"d","skills":[{"name":"alias","from":"../my-skill"}]}"#,
        )
        .unwrap();
        let (skills, report) = collect(Some(&dir.join("manifests")));
        assert_eq!(skills.len(), 1);
        assert!(skills[0].resolved);
        assert_eq!(skills[0].skill.name, "alias", "以插件声明名为准");
        assert!(skills[0].skill.prompt_template.contains("正文内容"));
        assert_eq!(report.resolved, 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn traversal_from_is_rejected() {
        // 插件根 = dir/plug，越界技能 = dir/outside（真实存在，才能命中逃逸校验
        // 而不是 from-not-found）
        let dir = temp("traversal");
        let plug = dir.join("plug");
        std::fs::create_dir_all(plug.join("manifests")).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("SKILL.md"),
            "---\nname: outside\ndescription: 越界\n---\n正文\n",
        )
        .unwrap();
        std::fs::write(
            plug.join("manifests/p.plugin.json"),
            r#"{"name":"p","version":"1.0.0","description":"d","skills":[{"name":"evil","from":"../../outside"}]}"#,
        )
        .unwrap();
        let (skills, report) = collect(Some(&plug.join("manifests")));
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].resolved, "越界技能必须退化为引用");
        assert!(report
            .skipped
            .iter()
            .any(|(_, r)| r.contains("from-escapes-plugin-root")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn duplicate_names_keep_first() {
        let dir = temp("dup");
        std::fs::write(
            dir.join("manifests/a.plugin.json"),
            r#"{"name":"a","version":"1","description":"d","skills":[{"name":"same"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("manifests/b.plugin.json"),
            r#"{"name":"b","version":"1","description":"d","skills":[{"name":"same"}]}"#,
        )
        .unwrap();
        let (skills, report) = collect(Some(&dir.join("manifests")));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].plugin, "a");
        assert!(report.skipped.iter().any(|(_, r)| r == "duplicate-name"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_manifests_dir_is_empty_not_error() {
        let (skills, report) = collect(None);
        assert!(skills.is_empty());
        assert_eq!(report.refs_total, 0);
        let (skills, report) = collect(Some(Path::new("Z:/definitely/not/here")));
        assert!(skills.is_empty());
        assert_eq!(report.skipped.len(), 1);
    }
}
