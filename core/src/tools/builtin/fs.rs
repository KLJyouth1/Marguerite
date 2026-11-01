//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! File system tools: read/write/append/list/find/info/grep/codebase_search/code_intel/edit

use super::*;

// ---------------------------------------------------------------------------
// read_file：未改动文件的重复读去重（P0#1）
// ---------------------------------------------------------------------------
//
// 真实痛点：一个任务里同一份文件常被读 3–5 次（先看结构、再定位、改完复核），
// 每次都把全文重新塞进上下文。文件没变的时候，这部分 token 是纯浪费，而且会把
// 真正有用的历史挤出窗口。
//
// 语义与 Claude Code 的 `readFileCache` 对齐：**同会话 + 同路径 + 同 limit +
// (mtime, len) 都没变** → 第二次只回一行 `[FILE_UNCHANGED]` 存根；`force=true`
// 强制重发全文。存根是**成功结果**，不是错误 —— 模型读到的应该是「内容没变」，
// 而不是「读取失败」。
//
// 刻意不做成全局静态缓存：缓存活在当前 `read_file` 工具实例（= 一个 ToolRegistry 的
// 寿命）里，不落盘、不跨进程。盘上缓存一旦比文件旧，就是一类很难复现的脏读 bug，
// 省下来的那点 token 不值得。

/// `(mtime 纳秒, 文件字节数)` —— 判定「文件是否没变」的指纹。
type ReadStamp = (u64, u64);

/// 去重键：`(会话, 解析后的绝对路径, limit)`。
///
/// limit 必须进键：同一个文件按 10 行截断和按全文返回是两个不同的 payload，
/// 混在一起去重会把「要看全文」的请求答成 10 行。
type ReadKey = (String, String, Option<usize>);

/// 见过哪些文件：`ReadKey -> 上次下发时的指纹`。
///
/// 这里刻意写全 `std::sync::` 路径（不依赖父模块的 `use` 是否被 glob 带进来）——
/// 工具模块是高频改动区，少一个隐式依赖就少一类"改了别人先红"的编译错。
type ReadSeen = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<ReadKey, ReadStamp>>>;

/// 缓存条数上限。超了直接清空 —— 这里不需要 LRU：清空只是让下一轮少省一次 token，
/// 而 LRU 的实现成本和 bug 面都比它贵。
const READ_SEEN_CAP: usize = 512;

/// 取文件的 `(mtime, len)`。取不到（权限、竞态删除）就返 `None`，
/// 调用方据此**跳过**去重 —— 宁可多发一次全文，也不要在拿不准的时候回存根。
fn read_file_stamp(abs: &std::path::Path) -> Option<ReadStamp> {
    let md = std::fs::metadata(abs).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)?;
    Some((mtime, md.len()))
}

/// 宽松布尔参数：`true` / `1` / `yes`（大小写不敏感）算真。
///
/// 工具参数一律以 `String` 传递给 `ToolFn`（schema 里写 boolean，落到这里是 `"true"`），
/// 所以不能直接 `parse::<bool>()` —— 那个只认 `"true"`/`"false"`。
fn read_force_flag(args: &std::collections::HashMap<String, String>, key: &str) -> bool {
    match args.get(key) {
        Some(v) => {
            let v = v.trim();
            v.eq_ignore_ascii_case("true") || v == "1" || v.eq_ignore_ascii_case("yes")
        }
        None => false,
    }
}

/// 去重总开关：默认开，`MARGUERITE_READ_DEDUPE=0|false|off` 关掉。
///
/// 为什么留这个开关：`harness/ledger.rs` 的确定性重放是「同一份调用序列必须复现同一结果」。
/// 去重会让「第二次读」的 payload 从全文变成存根 —— 一旦某条历史 ledger 是在旧语义下录的，
/// 重放就会对不上。修 ledger 是对的长期做法，但在那之前，线上/回放出现对不上时
/// 必须有一个**不用重新编译**就能退回旧语义的把手。
fn read_dedupe_enabled() -> bool {
    match std::env::var("MARGUERITE_READ_DEDUPE") {
        Ok(v) => {
            let v = v.trim();
            !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
        }
        Err(_) => true,
    }
}

pub(crate) fn read_file_tool(workspace: impl Into<RootSet>) -> Box<dyn ToolExecutor> {
    let workspace: RootSet = workspace.into();
    let seen: ReadSeen = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::new(),
    ));
    boxed(
        "read_file",
        "读取工作区内文本文件内容（多根会话会按注册顺序查找同名文件）。参数: path(相对路径), limit(可选, 限制行数), force(可选, true=文件未改动也重发全文)",
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "limit":{"type":"number"},
            "force":{"type":"boolean","description":"true 时忽略「未改动」去重，强制重发全文"}
        }}),
        Arc::new(move |args| {
            let ws = workspace.clone();
            let seen = seen.clone();
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

                let limit: Option<usize> = args.get("limit").and_then(|l| l.parse().ok());
                let force = read_force_flag(args, "force");
                // 指纹在读取**之前**取：万一读的瞬间文件正在被改，我们记下的是旧指纹，
                // 下次读会因为指纹不匹配而重发全文 —— 错的方向是「多给一次」，
                // 而不是「该给的时候给了存根」。
                let stamp = read_file_stamp(&abs);
                let key = (current_session(), abs.to_string_lossy().into_owned(), limit);

                if !force && read_dedupe_enabled() {
                    if let (Some(now), Ok(map)) = (stamp, seen.lock()) {
                        if map.get(&key).copied() == Some(now) {
                            return ok(format!(
                                "[FILE_UNCHANGED] {} 自上次读取后未修改（{} bytes）。\
                                 内容与上次返回完全一致，已省略重复下发；\
                                 需要再次获取全文请传 force=true。",
                                p, now.1
                            ));
                        }
                    }
                }

                let content =
                    std::fs::read_to_string(&abs).map_err(|e| anyhow!("read {}: {}", p, e))?;
                let shown = match limit {
                    Some(n) => content.lines().take(n).collect::<Vec<_>>().join("\n"),
                    None => content,
                };
                if let (Some(now), Ok(mut map)) = (stamp, seen.lock()) {
                    if map.len() >= READ_SEEN_CAP {
                        map.clear();
                    }
                    map.insert(key, now);
                }
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
