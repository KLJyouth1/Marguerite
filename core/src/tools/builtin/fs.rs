//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! File system tools: read/write/append/list/find/info/grep/codebase_search/code_intel/edit

use super::*;

// ---------------------------------------------------------------------------
// 工具工厂
// ---------------------------------------------------------------------------

pub(crate) fn read_file_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "read_file",
        "读取工作区内文本文件内容（多根会话会按注册顺序查找同名文件）。参数: path(相对路径), limit(可选, 限制行数)",
        json!({"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = ws.resolve(p)?;
                // L3 读取边界：`.env` / 网关 token / 凭据台账即使落在工作区内
                // （未绑定项目时工作区根就是数据目录）也不得读进模型上下文。
                if let Some(denied) = gate(&PREFLIGHT.check_read_multi(&abs, &guard_roots(&ws))) {
                    return denied;
                }
                if !abs.is_file() {
                    return err(format!("not a file: {}", p), "NOT_FOUND");
                }
                let content =
                    std::fs::read_to_string(&abs).map_err(|e| anyhow!("read {}: {}", p, e))?;
                let shown = match args.get("limit") {
                    Some(l) => {
                        let n: usize = l.parse().unwrap_or(usize::MAX);
                        content.lines().take(n).collect::<Vec<_>>().join("\n")
                    }
                    None => content,
                };
                ok(shown)
            })
        }),
    )
}

pub(crate) fn write_file_tool(
    workspace: impl Into<RootSet>,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "write_file",
        "写入（覆盖）工作区内文件，写入前自动快照旧版本。参数: path, content",
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let content = args
                    .get("content")
                    .ok_or_else(|| anyhow!("missing 'content'"))?;
                let abs = ws.resolve(p)?;
                // L3 路径边界 + L2 内容扫描：必须在创建父目录/落盘之前，
                // 否则被拒的写入仍会留下副作用（空目录）。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_write_multi(&abs, &guard_roots(&ws), content))
                {
                    return denied;
                }
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let existed = abs.exists();
                let before = if existed {
                    std::fs::read_to_string(&abs).unwrap_or_default()
                } else {
                    String::new()
                };
                if let Some(vm) = versions.as_ref() {
                    if existed {
                        let _ = vm.snapshot_file(&abs);
                    }
                }
                std::fs::write(&abs, content)?;
                crate::tools::file_change::record_file_change(
                    p,
                    if existed { "modify" } else { "create" },
                    &before,
                    content,
                    "write_file",
                );
                // 记录产物到当前会话（R1 + R4：无 SESSION 时回退默认会话并告警）
                let sess = current_session();
                crate::tools::file_change::record_artifact(
                    &sess,
                    p,
                    if existed { "modified" } else { "created" },
                );
                ok(format!("wrote {} bytes to {}", content.len(), p))
            })
        }),
    )
}

pub(crate) fn append_file_tool(
    workspace: impl Into<RootSet>,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "append_file",
        "向工作区内文件追加内容，追加前自动快照。参数: path, content",
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let content = args
                    .get("content")
                    .ok_or_else(|| anyhow!("missing 'content'"))?;
                let abs = ws.resolve(p)?;
                if let Some(denied) =
                    gate(&PREFLIGHT.check_write_multi(&abs, &guard_roots(&ws), content))
                {
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
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&abs)?;
                f.write_all(content.as_bytes())?;
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, p, "appended");
                ok(format!("appended {} bytes to {}", content.len(), p))
            })
        }),
    )
}

pub(crate) fn list_dir_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "list_dir",
        "列出目录内容（文件/子目录 + 大小）。参数: path(默认列出全部工作区根)",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                // 无 path 且是多根会话 → 逐个根分区展示（否则用户以为附加根不存在）
                let targets: Vec<(String, PathBuf)> = match args.get("path") {
                    Some(p) => vec![(p.to_string(), ws.resolve(p)?)],
                    None if ws.is_multi() => ws
                        .all()
                        .iter()
                        .map(|r| (r.display().to_string(), r.clone()))
                        .collect(),
                    None => vec![(".".to_string(), ws.resolve(".")?)],
                };
                let multi = targets.len() > 1;
                let mut out_sections: Vec<String> = Vec::new();
                for (label, abs) in targets {
                    if !abs.is_dir() {
                        if multi {
                            out_sections.push(format!("[{label}]\nnot a directory"));
                            continue;
                        }
                        return err(format!("not a directory: {label}"), "NOT_DIR");
                    }
                    let mut lines = Vec::new();
                    for e in std::fs::read_dir(&abs)?.flatten() {
                        let p = e.path();
                        let kind = if p.is_dir() { "dir " } else { "file" };
                        let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                        lines.push(format!("{}\t{:>10}\t{}", kind, size, name));
                    }
                    lines.sort();
                    if multi {
                        out_sections.push(format!("[{label}]\n{}", lines.join("\n")));
                    } else {
                        out_sections.push(lines.join("\n"));
                    }
                }
                ok(out_sections.join("\n\n"))
            })
        }),
    )
}

pub(crate) fn find_files_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "find_files",
        "按名称子串在工作区内查找文件。参数: name(子串), max(可选, 上限)",
        json!({"type":"object","properties":{"name":{"type":"string"},"max":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let name = args.get("name").ok_or_else(|| anyhow!("missing 'name'"))?;
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(200);
                let mut all = Vec::new();
                // 多根：遍历全部根，路径按所属根去前缀展示
                for root in ws.all() {
                    walk_files(root, &mut all);
                }
                all.sort();
                all.dedup();
                let mut hits: Vec<String> = all
                    .iter()
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .contains(name)
                    })
                    .map(|p| {
                        ws.all()
                            .iter()
                            .find_map(|root| p.strip_prefix(root).ok())
                            .unwrap_or(p)
                            .to_string_lossy()
                            .replace('\\', "/")
                    })
                    .take(max)
                    .collect();
                hits.sort();
                ok(format!(
                    "found {} file(s):\n{}",
                    hits.len(),
                    hits.join("\n")
                ))
            })
        }),
    )
}

pub(crate) fn file_info_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "file_info",
        "返回文件/目录的元信息（大小、是否为目录、修改时间）。参数: path",
        json!({"type":"object","properties":{"path":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let abs = ws.resolve(p)?;
                let meta = std::fs::metadata(&abs)?;
                let info = json!({
                    "path": p,
                    "is_dir": meta.is_dir(),
                    "is_file": meta.is_file(),
                    "size_bytes": meta.len(),
                    "modified": meta.modified().map(|t| format!("{:?}", t)).unwrap_or_default(),
                });
                ok(info.to_string())
            })
        }),
    )
}

pub(crate) fn grep_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "grep",
        "在工作区内按正则搜索文件内容，返回命中行(file:line:text)。参数: pattern, path(可选,限定目录), max(可选)",
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"max":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let pattern = args.get("pattern").ok_or_else(|| anyhow!("missing 'pattern'"))?;
                let re = regex::Regex::new(pattern).map_err(|e| anyhow!("invalid regex: {}", e))?;
                // 多根：指定 path 时只搜该路径；未指定时搜全部根
                let roots: Vec<PathBuf> = match args.get("path") {
                    Some(p) => vec![ws.resolve(p)?],
                    None => ws.all().to_vec(),
                };
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(200);
                let mut all = Vec::new();
                for root in &roots {
                    walk_files(root, &mut all);
                }
                all.sort();
                all.dedup();
                let mut hits: Vec<String> = Vec::new();
                for f in &all {
                    if hits.len() >= max {
                        break;
                    }
                    // 凭证文件不参与内容检索：grep 会把命中的原文行直接回给模型，
                    // 与 read_file 泄露的是同一份密钥。
                    if crate::security::scanner_l3::is_credential_file(f) {
                        continue;
                    }
                    // 跳过明显二进制（按扩展名粗判）
                    let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "pdf" | "zip" | "exe" | "dll" | "so" | "o" | "rlib" | "rmeta") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(f) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let rel = roots
                        .iter()
                        .find_map(|root| f.strip_prefix(root).ok())
                        .unwrap_or(f)
                        .to_string_lossy()
                        .replace('\\', "/");
                    for (i, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            hits.push(format!("{}:{}:{}", rel, i + 1, line.trim()));
                            if hits.len() >= max {
                                break;
                            }
                        }
                    }
                }
                ok(format!("{} match(es):\n{}", hits.len(), hits.join("\n")))
            })
        }),
    )
}

pub(crate) fn codebase_search_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "codebase_search",
        "在工作区内检索关键词，返回按命中次数排序的文件清单（轻量语义定位）。参数: query, max(可选)",
        json!({"type":"object","properties":{"query":{"type":"string"},"max":{"type":"number"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            Box::pin(async move {
                let query = args.get("query").ok_or_else(|| anyhow!("missing 'query'"))?;
                let max: usize = args.get("max").and_then(|m| m.parse().ok()).unwrap_or(20);
                let re = regex::Regex::new(&regex::escape(query))
                    .map_err(|e| anyhow!("invalid query: {}", e))?;
                let mut all = Vec::new();
                for root in ws.all() {
                    walk_files(root, &mut all);
                }
                all.sort();
                all.dedup();
                let mut scored: Vec<(usize, String)> = Vec::new();
                for f in &all {
                    // 同 grep：检索结果会暴露「哪一行命中了」，凭证文件一律跳过。
                    if crate::security::scanner_l3::is_credential_file(f) {
                        continue;
                    }
                    let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "png" | "jpg" | "pdf" | "zip" | "exe" | "dll" | "so" | "o" | "rlib" | "rmeta") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(f) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let count = re.find_iter(&content).count();
                    if count > 0 {
                        let rel = ws
                            .all()
                            .iter()
                            .find_map(|root| f.strip_prefix(root).ok())
                            .unwrap_or(f)
                            .to_string_lossy()
                            .replace('\\', "/");
                        scored.push((count, rel));
                    }
                }
                scored.sort_by_key(|(count, _)| std::cmp::Reverse(*count));
                let out: Vec<String> = scored.into_iter().take(max).map(|(c, r)| format!("{:>5}\t{}", c, r)).collect();
                ok(format!("{} file(s) ranked:\n{}", out.len(), out.join("\n")))
            })
        }),
    )
}
pub(crate) fn edit_file_tool(
    workspace: impl Into<RootSet>,
    versions: Arc<Option<VersionManager>>,
) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    boxed(
        "edit_file",
        "在文件中将 old_string 替换为 new_string。参数: path, old_string, new_string, all(可选,'true'替换全部)",
        json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"all":{"type":"string"}}}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let versions = versions.clone();
            Box::pin(async move {
                let p = args.get("path").ok_or_else(|| anyhow!("missing 'path'"))?;
                let old = args.get("old_string").ok_or_else(|| anyhow!("missing 'old_string'"))?;
                let new = args.get("new_string").ok_or_else(|| anyhow!("missing 'new_string'"))?;
                let all = args.get("all").map(|s| s == "true" || s == "1").unwrap_or(false);
                let abs = ws.resolve(p)?;
                if !abs.is_file() {
                    return err(format!("not a file: {}", p), "NOT_FOUND");
                }
                // L3 路径边界 + L2 扫描「本次新引入的内容」。
                // 刻意只扫 new_string 而非替换后的全文：文件里既有的历史内容
                // 不该让一次无关编辑失败（否则连 scanner_l2.rs 自身都改不了），
                // 但任何**新注入**的恶意片段都会在这里被挡下。
                if let Some(denied) =
                    gate(&PREFLIGHT.check_write_multi(&abs, &guard_roots(&ws), new))
                {
                    return denied;
                }
                if let Some(vm) = versions.as_ref() {
                    let _ = vm.snapshot_file(&abs);
                }
                let content = std::fs::read_to_string(&abs)?;
                if !content.contains(old) {
                    return err(format!("old_string not found in {}", p), "NO_MATCH");
                }
                let replaced = if all {
                    content.replace(old, new)
                } else {
                    content.replacen(old, new, 1)
                };
                std::fs::write(&abs, &replaced)?;
                crate::tools::file_change::record_file_change(p, "modify", &content, &replaced, "edit_file");
                let sess = current_session();
                crate::tools::file_change::record_artifact(&sess, p, "modified");
                ok(format!("edited {} ({}->{} bytes)", p, content.len(), replaced.len()))
            })
        }),
    )
}

// ---------------------------------------------------------------------------
// 工具输出留底取回（M1 Task 3）
// ---------------------------------------------------------------------------

/// 取回被入口截流砍掉的工具输出原文。
///
/// 指纹来自历史里的
/// `…[中间省略 N 个字符；原文全文可用 tool_output_retrieve("<fp>") 取回]…` 标注。
///
/// 放在 fs 族是因为它读的是本机留底文件；但**不**注册进 `chain/builtin.rs` ——
/// 链式执行器只有 5 个自带工具、且本就不做入口截流，那边没有可取的原文，
/// 塞进去只会多一个恒定失败的入口。
pub(crate) fn tool_output_retrieve_tool() -> Box<dyn ToolExecutor> {
    boxed(
        // 点号名会被 `register_local` 自动修正成别名（P1-E 守卫：内置名必须
        // 首次即合法 `^[a-zA-Z0-9_-]+$`），而模型只看得到合法名 —— 所以这里用下划线。
        "tool_output_retrieve",
        "取回已被入口截流砍掉的工具输出原文全文。指纹取自历史里的 \
         [中间省略 N 个字符；原文全文可用 tool_output_retrieve(\"<fp>\") 取回] 标注。 \
         参数: fingerprint(必填，32 位小写十六进制)",
        json!({"type":"object","properties":{"fingerprint":{"type":"string"}},"required":["fingerprint"]}),
        Arc::new(|args| {
            Box::pin(async move {
                let fp = args
                    .get("fingerprint")
                    .ok_or_else(|| anyhow!("missing 'fingerprint'"))?;
                match crate::context::spool::retrieve(fp) {
                    Some(text) => ok(text),
                    // 取不到必须是**明确的失败**：回 `ok("")` 会被模型读成
                    // 「原文就是空的」，比没有这个工具更糟。
                    None => err(
                        "原文不可用：指纹非法、留底已被淘汰，或该输出从未被截断".to_string(),
                        "NOT_FOUND",
                    ),
                }
            })
        }),
    )
}
