//! 项目级关系图谱：**每个项目文件夹只有一份图谱**（一项目一图）。
//!
//! 持久化于 `<data_dir>/project_graphs/<key>.json`（key = 项目路径的稳定哈希）。
//! `analyze` 是离线静态扫描（零 LLM 依赖）：README 标题 -> 主题节点；
//! Cargo.toml / package.json / requirements.txt -> 依赖节点；
//! 源文件 import / use / mod 引用 -> 模块节点 + 依赖边。
//! 帮助智能体理解、识别、剖析、分析项目：模块构成、依赖关系、主题脉络一图看清。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub kind: String, // project | module | topic | dependency
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String, // uses | contains | mentions
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectGraph {
    pub project: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub summary: String,
}

/// 稳定哈希键：归一化分隔符后取 DefaultHasher（std 固定种子，跨进程稳定）。
pub fn key_for(project: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project
        .replace(&std::path::MAIN_SEPARATOR.to_string(), "/")
        .hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn store_dir() -> Option<PathBuf> {
    let d = crate::persistence::config::Config::data_dir().join("project_graphs");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn path_for(project: &str) -> Option<PathBuf> {
    store_dir().map(|d| d.join(format!("{}.json", key_for(project))))
}

/// 加载某项目的图谱；不存在时返回默认空图（project 已填）。
pub fn load(project: &str) -> Result<ProjectGraph, String> {
    let Some(path) = path_for(project) else {
        return Err("图谱存储目录不可用".to_string());
    };
    if !path.exists() {
        return Ok(ProjectGraph {
            project: project.to_string(),
            ..Default::default()
        });
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取图谱失败: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("图谱解析失败: {e}"))
}

/// 原子保存（tmp + rename）；一项目一图：同项目重复保存即覆盖。
pub fn save(graph: &ProjectGraph) -> Result<(), String> {
    let Some(path) = path_for(&graph.project) else {
        return Err("图谱存储目录不可用".to_string());
    };
    let text = serde_json::to_string_pretty(graph).map_err(|e| format!("序列化失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入临时图谱失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("原子替换失败: {e}"))
}

pub fn delete(project: &str) -> Result<(), String> {
    let Some(path) = path_for(project) else {
        return Err("图谱存储目录不可用".to_string());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除图谱失败: {e}")),
    }
}

/// 列出全部项目图谱元信息（key/project/节点数/边数）。
pub fn list() -> Vec<(String, String, usize, usize)> {
    let mut out = Vec::new();
    let Some(dir) = store_dir() else { return out };
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(g) = serde_json::from_str::<ProjectGraph>(&text) {
                    out.push((key_for(&g.project), g.project, g.nodes.len(), g.edges.len()));
                }
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

// ── 静态分析 ──

const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    ".venv",
    "venv",
    "__pycache__",
];
const CODE_EXTS: &[&str] = &[
    "rs", "ts", "js", "mjs", "cjs", "vue", "py", "go", "java", "c", "cpp", "h", "hpp",
];

/// 离线静态扫描项目目录，生成图谱（有界：深度 <= 4、文件 <= 400、单文件 <= 128KB）。
/// 幂等：每次 analyze 全量重建，语义上同一项目只有一张图。
pub fn analyze(project: &str, max_files: usize) -> Result<ProjectGraph, String> {
    let root = PathBuf::from(project);
    if !root.is_dir() {
        return Err(format!("项目目录不存在或不可读: {project}"));
    }
    let cap = max_files.clamp(8, 400);
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_files = 0usize;

    nodes.push(GraphNode {
        id: "project".to_string(),
        kind: "project".to_string(),
        label: root.file_name().map(|n| n.to_string_lossy().to_string()),
    });

    // 1) README 一级标题 -> 主题节点
    let readme = root.join("README.md");
    if let Ok(text) = std::fs::read_to_string(&readme) {
        let mut topics = 0;
        for line in text.lines().take(200) {
            let t = line.trim();
            if t.starts_with("#") && !t.starts_with("##") {
                let title = t.trim_start_matches("#").trim();
                if title.is_empty() {
                    continue;
                }
                let label = title.chars().take(60).collect::<String>();
                let id = format!("topic:{}", topics);
                nodes.push(GraphNode {
                    id: id.clone(),
                    kind: "topic".to_string(),
                    label: Some(label),
                });
                edges.push(GraphEdge {
                    from: "project".to_string(),
                    to: id,
                    kind: "mentions".to_string(),
                });
                topics += 1;
                if topics >= 12 {
                    break;
                }
            }
        }
    }

    // 2) 依赖清单 -> 依赖节点
    let mut dep_names: Vec<String> = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) {
        let mut in_deps = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with("[dependencies]") || t.starts_with("[dev-dependencies]") {
                in_deps = true;
                continue;
            }
            if t.starts_with("[") {
                in_deps = false;
                continue;
            }
            if in_deps {
                if let Some(name) = t
                    .split("=")
                    .next()
                    .map(|s| s.trim().trim_matches(|c: char| c as u32 == 34))
                {
                    if !name.is_empty() && !dep_names.iter().any(|d| d == name) {
                        dep_names.push(name.to_string());
                    }
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            for sec in ["dependencies", "devDependencies"] {
                if let Some(map) = v.get(sec).and_then(|x| x.as_object()) {
                    for key in map.keys() {
                        if !dep_names.iter().any(|d| d == key) {
                            dep_names.push(key.clone());
                        }
                    }
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join("requirements.txt")) {
        for line in text.lines().take(200) {
            let t = line.trim();
            if t.is_empty() || t.starts_with("#") {
                continue;
            }
            let name = t
                .split(|c: char| {
                    !(c.is_alphanumeric() || c as u32 == 45 || c as u32 == 95 || c as u32 == 46)
                })
                .next()
                .unwrap_or("")
                .to_string();
            if name.is_empty() || name == "." {
                continue;
            }
            if !dep_names.iter().any(|d| d == &name) {
                dep_names.push(name);
            }
        }
    }
    for dep in &dep_names {
        let id = format!("dep:{}", dep);
        nodes.push(GraphNode {
            id: id.clone(),
            kind: "dependency".to_string(),
            label: None,
        });
        edges.push(GraphEdge {
            from: "project".to_string(),
            to: id,
            kind: "uses".to_string(),
        });
    }

    // 3) 源文件 -> 模块节点 + 引用边
    let mut collected: Vec<(String, PathBuf, String)> = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = vec![(root.clone(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 4 {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        stack.push((path, depth + 1));
                    }
                    continue;
                }
                let Some(ext) = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                if !CODE_EXTS.contains(&ext.as_str()) {
                    continue;
                }
                if seen_files >= cap {
                    break;
                }
                if let Ok(meta) = entry.metadata() {
                    if meta.len() > 128 * 1024 {
                        continue;
                    }
                }
                let rel = path
                    .strip_prefix(&root)
                    .map(|p| {
                        p.to_string_lossy()
                            .replace(&std::path::MAIN_SEPARATOR.to_string(), "/")
                    })
                    .unwrap_or_else(|_| name.clone());
                collected.push((rel, path, ext));
                seen_files += 1;
            }
        }
        if seen_files >= cap {
            break;
        }
    }

    // 第一遍：全部模块节点先行入图（保证第二遍引用解析可匹配任意文件）
    for (rel, _path, _ext) in &collected {
        let module_id = format!("file:{}", rel);
        nodes.push(GraphNode {
            id: module_id.clone(),
            kind: "module".to_string(),
            label: None,
        });
        edges.push(GraphEdge {
            from: "project".to_string(),
            to: module_id,
            kind: "contains".to_string(),
        });
    }
    // 第二遍：解析引用并连线
    for (rel, path, ext) in &collected {
        let module_id = format!("file:{}", rel);
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let refs: Vec<String> = match ext.as_str() {
            "rs" => extract_rust_refs(&text),
            "ts" | "js" | "mjs" | "cjs" | "vue" => extract_js_refs(&text),
            "py" => extract_py_refs(&text),
            _ => Vec::new(),
        };
        for r in refs {
            let target_file = format!("file:{}", r);
            if nodes.iter().any(|n| n.id == target_file) {
                edges.push(GraphEdge {
                    from: module_id.clone(),
                    to: target_file,
                    kind: "uses".to_string(),
                });
                continue;
            }
            // 模块短名匹配（mod util -> src/util.rs 等）
            if let Some(node_id) = nodes
                .iter()
                .find(|n| {
                    n.kind == "module"
                        && n.id
                            .rsplit('/')
                            .next()
                            .unwrap_or("")
                            .split('.')
                            .next()
                            .unwrap_or("")
                            == r.as_str()
                })
                .map(|n| n.id.clone())
            {
                edges.push(GraphEdge {
                    from: module_id.clone(),
                    to: node_id,
                    kind: "uses".to_string(),
                });
                continue;
            }
            if dep_names.iter().any(|d| d == &r) {
                edges.push(GraphEdge {
                    from: module_id.clone(),
                    to: format!("dep:{}", r),
                    kind: "uses".to_string(),
                });
            }
        }
    }

    let summary = format!(
        "项目 {} 静态扫描：{} 模块 / {} 依赖 / {} 主题（一项目一图，离线零 LLM）",
        root.display(),
        collected.len(),
        dep_names.len(),
        nodes.iter().filter(|n| n.kind == "topic").count()
    );
    Ok(ProjectGraph {
        project: project.to_string(),
        nodes,
        edges,
        summary,
    })
}

fn extract_rust_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("use ") {
            let first = rest.split("::").next().unwrap_or("").trim();
            if !first.is_empty() && first != "self" && first != "super" {
                out.push(first.to_string());
            }
        } else if let Some(rest) = t.strip_prefix("mod ") {
            let name = rest
                .split(|c: char| c as u32 == 59 || c as u32 == 32 || c as u32 == 123)
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

fn extract_js_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let Some(idx) = t.find("from ") else { continue };
        let rest = &t[idx + 5..];
        let quoted = rest
            .trim()
            .trim_matches(|c: char| c as u32 == 34 || c as u32 == 39);
        let module = quoted
            .split(|c: char| c as u32 == 34 || c as u32 == 39)
            .next()
            .unwrap_or("")
            .trim();
        if module.is_empty() || module.starts_with(".") || module.starts_with("/") {
            continue;
        }
        let pkg = if module.starts_with("@") {
            module.split("/").take(2).collect::<Vec<_>>().join("/")
        } else {
            module.split("/").next().unwrap_or(module).to_string()
        };
        if !pkg.is_empty() {
            out.push(pkg);
        }
    }
    out
}

fn extract_py_refs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("import ") {
            out.push(
                rest.split(" ")
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(",")
                    .to_string(),
            );
        } else if let Some(rest) = t.strip_prefix("from ") {
            out.push(
                rest.split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        }
    }
    out.retain(|s| !s.is_empty() && !s.contains("."));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("marguerite_pg_test_{ts}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    #[test]
    fn key_for_is_stable_and_normalizes() {
        // `key_for` 只把**本机分隔符**归一化成 `/`：Unix 上 `\` 是普通字符，
        // 拿它当分隔符断言会得到两个不同的键（Linux CI 曾因此判红）。
        let sep = std::path::MAIN_SEPARATOR;
        let native = format!("D:{sep}a{sep}b");
        assert_eq!(key_for("D:/a/b"), key_for(&native), "分隔符归一化后键相同");
        assert_eq!(key_for("D:/a/b"), key_for("D:/a/b"), "同路径键稳定");
        assert_ne!(key_for("D:/a/b"), key_for("D:/a/c"));
    }

    #[test]
    fn analyze_builds_modules_deps_topics() {
        let dir = temp_project();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("README.md"))
            .unwrap();
        for line in ["# 测试项目", "描述", "## 小节", "# 主题二"] {
            writeln!(f, "{}", line).unwrap();
        }
        drop(f);
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src").join("main.rs"),
            "use serde::Serialize;\nmod util;\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("util.rs"), "pub fn f() {}").unwrap();

        let g = analyze(dir.to_str().unwrap(), 200).unwrap();
        let ids: std::collections::HashSet<String> = g.nodes.iter().map(|n| n.id.clone()).collect();
        assert!(ids.contains("project"));
        assert!(ids.contains("dep:serde"));
        assert!(
            ids.contains("file:src/main.rs"),
            "相对路径模块节点: {:?}",
            ids
        );
        assert_eq!(
            g.nodes.iter().filter(|n| n.kind == "topic").count(),
            2,
            "两个一级标题主题"
        );
        assert!(g
            .edges
            .iter()
            .any(|e| e.from == "project" && e.to == "dep:serde" && e.kind == "uses"));
        assert!(g
            .edges
            .iter()
            .any(|e| e.from == "file:src/main.rs" && e.to == "dep:serde"));
        assert!(
            g.edges
                .iter()
                .any(|e| e.from == "file:src/main.rs" && e.to == "file:src/util.rs"),
            "mod util 引用边"
        );
    }

    #[test]
    fn save_load_roundtrip_one_graph_per_project() {
        let dir = temp_project();
        let project = dir.to_str().unwrap().to_string();
        let mut g = analyze(&project, 200).unwrap();
        g.nodes.push(GraphNode {
            id: "extra".to_string(),
            kind: "topic".to_string(),
            label: Some("手工补充".to_string()),
        });
        save(&g).unwrap();

        let loaded = load(&project).unwrap();
        assert_eq!(loaded.nodes.len(), g.nodes.len());
        assert_eq!(loaded.project, project);
        // 同一项目再次保存 = 覆盖（一项目一图）
        let mut g2 = loaded.clone();
        g2.nodes.pop();
        save(&g2).unwrap();
        let loaded2 = load(&project).unwrap();
        assert_eq!(loaded2.nodes.len(), g2.nodes.len());
        assert!(list()
            .iter()
            .any(|(_, p, n, e)| p == &project && *n == g2.nodes.len() && *e == g2.edges.len()));

        delete(&project).unwrap();
        assert!(load(&project).unwrap().nodes.is_empty(), "删除后回空图");
    }

    #[test]
    fn analyze_rejects_missing_dir() {
        assert!(analyze("D:/__no_such_project_marguerite__", 200).is_err());
    }
}
