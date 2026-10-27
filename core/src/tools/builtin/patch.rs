//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Apply patch + glob search + todo_write

use super::*;

// ---------------------------------------------------------------------------
// 上游等价工具（二开）：apply_patch / glob_file_search / todo_write
//
// 对齐上游 CLI 核心工具面中本工作台此前缺失的三件套：
// - `apply_patch`：统一 diff 补丁应用（上游的「major lever」，比单串替换
//   `edit_file` 更强：一个补丁可精确改动多处、支持新增/删除整行块）。
// - `glob_file_search`：glob 模式文件检索（`find_files` 只做名字子串，glob 支持
//   `**`/`*`/`?` 通配，更贴近「按模式定位」的真实检索需求）。
// - `todo_write`：结构化任务清单（对齐上游 todo_write/update_plan，让 Agent
//   在长任务中自报进度，落盘 .marguerite/todos.json 可跨轮读取）。
// ---------------------------------------------------------------------------

/// 补丁 hunk 内的单行（上下文 / 删除 / 新增）
enum HunkLine {
    Ctx(String),
    Del(String),
    Add(String),
}

/// 一个 `@@ -a,b +c,d @@` hunk
pub(crate) struct Hunk {
    old_start: usize,
    body: Vec<HunkLine>,
}

/// 解析 hunk 头 `@@ -1,3 +1,3 @@` → (old_start, old_count, new_start, new_count)
pub(crate) fn parse_hunk_header(line: &str) -> Result<(usize, usize, usize, usize)> {
    let s = line.trim().trim_start_matches('@').trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(anyhow!("无效 hunk 头: {line}"));
    }
    let old = parse_hunk_range(parts[0].trim_start_matches('-'))?;
    let new = parse_hunk_range(parts[1].trim_start_matches('+'))?;
    Ok((old.0, old.1, new.0, new.1))
}

/// 解析 `a,b` 或 `a` → (start, count)
pub(crate) fn parse_hunk_range(s: &str) -> Result<(usize, usize)> {
    if let Some((a, b)) = s.split_once(',') {
        Ok((a.parse()?, b.parse()?))
    } else {
        Ok((s.parse()?, 1usize))
    }
}

/// 在 `haystack` 中定位 `needle` 子序列：优先从 `start` 精确匹配，失败则线性扫描。
pub(crate) fn locate_sequence(haystack: &[String], needle: &[String], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    let start = start.min(haystack.len());
    if start + needle.len() <= haystack.len() && &haystack[start..start + needle.len()] == needle {
        return Some(start);
    }
    (0..=haystack.len().saturating_sub(needle.len()))
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// 应用 unified diff 补丁到文件内容，返回 (新内容, 已应用 hunk 数)。
///
/// 只处理 `@@` hunk（`---`/`+++` 头与 `\ No newline` 等辅助行由调用方剥离或忽略）。
pub(crate) fn apply_hunks(content: &str, patch: &str) -> Result<(String, usize)> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        if lines[i].starts_with("@@") {
            let (old_start, _old_count, _new_start, _new_count) = parse_hunk_header(lines[i])?;
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].starts_with("@@") {
                let l = lines[i];
                if let Some(rest) = l.strip_prefix('+') {
                    if !rest.starts_with('+') {
                        body.push(HunkLine::Add(rest.to_string()));
                    }
                } else if let Some(rest) = l.strip_prefix('-') {
                    if !rest.starts_with('-') {
                        body.push(HunkLine::Del(rest.to_string()));
                    }
                } else if let Some(rest) = l.strip_prefix(' ') {
                    body.push(HunkLine::Ctx(rest.to_string()));
                }
                i += 1;
            }
            hunks.push(Hunk { old_start, body });
        } else {
            i += 1;
        }
    }
    if hunks.is_empty() {
        return Err(anyhow!("补丁中未找到任何 @@ hunk 头"));
    }

    let mut cur: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut offset: isize = 0;
    let mut applied = 0usize;
    for hunk in &hunks {
        let old_lines: Vec<String> = hunk
            .body
            .iter()
            .filter_map(|l| match l {
                HunkLine::Ctx(s) | HunkLine::Del(s) => Some(s.clone()),
                HunkLine::Add(_) => None,
            })
            .collect();
        let new_lines: Vec<String> = hunk
            .body
            .iter()
            .filter_map(|l| match l {
                HunkLine::Ctx(s) | HunkLine::Add(s) => Some(s.clone()),
                HunkLine::Del(_) => None,
            })
            .collect();
        let start = (hunk.old_start as isize - 1 + offset).max(0) as usize;
        let pos = locate_sequence(&cur, &old_lines, start).ok_or_else(|| {
            anyhow!(
                "hunk 上下文不匹配（old_start={}），无法应用补丁",
                hunk.old_start
            )
        })?;
        let end = pos + old_lines.len();
        let mut next: Vec<String> =
            Vec::with_capacity(cur.len() - old_lines.len() + new_lines.len());
        next.extend_from_slice(&cur[..pos]);
        next.extend(new_lines.iter().cloned());
        next.extend_from_slice(&cur[end..]);
        offset += new_lines.len() as isize - old_lines.len() as isize;
        cur = next;
        applied += 1;
    }
    Ok((cur.join("\n"), applied))
}

/// 从补丁文本提取 `+++ b/...` 或 `+++ a/...` 头中的目标路径（无则 None）。
pub(crate) fn patch_target_path(patch: &str) -> Option<String> {
    patch.lines().find_map(|l| {
        let rest = l.strip_prefix("+++ ")?.trim();
        let rest = rest
            .strip_prefix("b/")
            .or_else(|| rest.strip_prefix("a/"))
            .unwrap_or(rest);
        Some(rest.to_string())
    })
}

pub(crate) fn apply_patch_tool(
    workspace: impl Into<RootSet>,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "apply_patch",
        "应用 unified diff 补丁到工作区文件（对齐上游 apply_patch）。patch 为包含 @@ hunk 头的补丁文本，可含 +++ b/path 头；若省略头则需显式 path。参数: patch(必填), path(可选，补丁含 +++ 头时可省略)",
        json!({"type":"object","properties":{
            "patch":{"type":"string"},
            "path":{"type":"string"}
        },"required":["patch"]}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let patch = args.get("patch").ok_or_else(|| anyhow!("missing 'patch'"))?;
                let header = patch_target_path(patch);
                let p = args
                    .get("path")
                    .map(|s| s.as_str())
                    .or(header.as_deref())
                    .ok_or_else(|| anyhow!("missing 'path'（且补丁无 +++ 头）"))?;
                let abs = ws.resolve(p)?;
                let existed = abs.is_file();
                let old_content = if existed {
                    std::fs::read_to_string(&abs)?
                } else {
                    String::new()
                };
                let (new_content, applied) = apply_hunks(&old_content, patch)?;
                // L3 路径边界 + L2 内容扫描：补丁引入的新内容必须过护栏。
                if let Some(denied) = gate(&PREFLIGHT.check_write_multi(&abs, &guard_roots(&ws), &new_content)) {
                    return denied;
                }
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Some(vm) = versions.as_ref() {
                    if abs.exists() {
                        let _ = vm.snapshot_file(&abs);
                    }
                }
                std::fs::write(&abs, &new_content)?;
                crate::tools::file_change::record_file_change(
                    p,
                    if existed { "modify" } else { "create" },
                    &old_content,
                    &new_content,
                    "apply_patch",
                );
                // 产物登记（补缺口）：apply_patch 此前只记「文件变更卡片」，不进产物台账，
                // 于是「用 apply_patch 生成的交付物」永远不出现在前端产物面板里。
                let sess = current_session();
                crate::tools::file_change::record_artifact(
                    &sess,
                    p,
                    if existed { "modified" } else { "created" },
                );
                ok(format!("已应用 {applied} 个 hunk 到 {p}"))
            })
        }),
    )
}

/// 单段 glob 匹配（`*` 任意字符、`?` 单字符，带回溯）
pub(crate) fn segment_glob_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 跨段 glob 匹配（支持 `**` 匹配零个或多个目录段）
pub(crate) fn glob_segments_match(pat: &[&str], path: &[&str]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat[0] == "**" {
        if glob_segments_match(&pat[1..], path) {
            return true;
        }
        if !path.is_empty() && glob_segments_match(pat, &path[1..]) {
            return true;
        }
        return false;
    }
    if path.is_empty() {
        return false;
    }
    if segment_glob_match(pat[0], path[0]) {
        return glob_segments_match(&pat[1..], &path[1..]);
    }
    false
}

/// 顶层 glob 匹配：按 `/` 分段，`**` 跨段、`*`/`?` 段内。
pub(crate) fn glob_matches(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    glob_segments_match(&pat, &segs)
}

pub(crate) fn glob_file_search_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "glob_file_search",
        "按 glob 模式检索工作区文件（多根会话会跨全部登记根检索）。参数: pattern(必填), max(可选，默认200)",
        json!({"type":"object","properties":{"pattern":{"type":"string"},"max":{"type":"number"}},"required":["pattern"]}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let pattern = args.get("pattern").ok_or_else(|| anyhow!("missing 'pattern'"))?;
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(200);
                let mut all = Vec::new();
                for root in ws.all() {
                    walk_files(root, &mut all);
                }
                all.sort();
                all.dedup();
                let mut hits: Vec<String> = all
                    .iter()
                    .filter_map(|p| {
                        let rel = ws
                            .all()
                            .iter()
                            .find_map(|root| p.strip_prefix(root).ok())
                            .unwrap_or(p)
                            .to_string_lossy()
                            .replace('\\', "/");
                        if glob_matches(pattern, &rel) { Some(rel) } else { None }
                    })
                    .take(max)
                    .collect();
                hits.sort();
                ok(format!("匹配 {} 个文件:\n{}", hits.len(), hits.join("\n")))
            })
        }),
    )
}

pub(crate) fn todo_write_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "todo_write",
        "写入/读取结构化任务清单（对齐 todo_write 机制）。action=set(替换清单)|get(读取)。set 时 items 为 JSON 数组 [{step,status}]，status=completed|in_progress|pending。持久化到 .marguerite/todos.json。参数: action(必填), items(set 时必填，JSON 数组字符串)",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["set","get"]},
            "items":{"type":"string"}
        },"required":["action"]}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let store = ws.primary().join(".marguerite").join("todos.json");
                let action = args.get("action").map(|s| s.as_str()).unwrap_or("get");
                match action {
                    "set" => {
                        let items_json = args.get("items").cloned().unwrap_or_default();
                        let items: serde_json::Value = serde_json::from_str(&items_json)
                            .map_err(|e| anyhow!("items 需为 JSON 数组字符串: {e}"))?;
                        if !items.is_array() {
                            return err("items 需为 JSON 数组".into(), "INVALID_PARAMS");
                        }
                        if let Some(parent) = store.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&store, items.to_string())?;
                        let n = items.as_array().map(|a| a.len()).unwrap_or(0);
                        ok(format!("已记录 {n} 项任务清单"))
                    }
                    _ => {
                        if !store.exists() {
                            return ok("任务清单为空".into());
                        }
                        ok(std::fs::read_to_string(&store)?)
                    }
                }
            })
        }),
    )
}

