//! 用户规则（个人规则 / 项目规则）。
//!
//! 存盘：`<config_dir>/rules/<id>.md`，frontmatter 保存元数据，正文即规则内容。
//!
//! 为什么用文件而不是数据库：规则是「用户要能直接看/带走/进版本库」的资产，
//! 纯文本最透明；解析失败的文件不会被静默丢弃（`list` 里带 `error` 字段）。
//!
//! 注入：启用中的规则按 `updated_at` 倒序拼接成一段 **静态 System 消息**，
//! 放在模式段/工作区段之后、工具指引之前 —— 内容跨轮稳定，不破坏 KV 前缀缓存。

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 注入 prompt 的规则总字符上限（超出按规则顺序截断并留标记）。
const MAX_INJECT_CHARS: usize = 6_000;
/// 单条规则正文上限（保存时校验，防止把整个仓库塞进规则）。
const MAX_RULE_CHARS: usize = 20_000;

fn rules_dir() -> PathBuf {
    Config::config_dir().join("rules")
}

fn err(msg: impl Into<String>, code: &str) -> Value {
    json!({ "ok": false, "error": msg.into(), "code": code })
}

/// 规则条目（列表视图，不含正文）。
#[derive(Debug, Clone)]
pub struct RuleEntry {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub enabled: bool,
    pub chars: usize,
    pub updated_at: String,
    pub path: PathBuf,
    pub error: Option<String>,
}

/// 从名字生成稳定 id：小写 + 仅保留字母数字与 `-`，空则用 hash 兜底。
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        format!("rule-{:x}", simple_hash(name))
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn parse_file(path: &PathBuf) -> Result<(RuleEntry, String), String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (meta, body) = split_frontmatter(&raw);
    let name = meta
        .iter()
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    let scope = meta
        .iter()
        .find(|(k, _)| k == "scope")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "personal".to_string());
    let enabled = meta
        .iter()
        .find(|(k, _)| k == "enabled")
        .map(|(_, v)| v != "false")
        .unwrap_or(true);
    let updated_at = meta
        .iter()
        .find(|(k, _)| k == "updated")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok((
        RuleEntry {
            id,
            name,
            scope,
            enabled,
            chars: body.chars().count(),
            updated_at,
            path: path.clone(),
            error: None,
        },
        body,
    ))
}

/// 极简 frontmatter 解析：只认 `key: value` 行，不做 YAML 嵌套。
fn split_frontmatter(raw: &str) -> (Vec<(String, String)>, String) {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    if !text.starts_with("---") {
        return (Vec::new(), text.trim_start_matches(['\r', '\n']).to_string());
    }
    let rest = &text[3..];
    let Some(end) = rest.find("\n---") else {
        return (Vec::new(), text.to_string());
    };
    let head = &rest[..end];
    let body = rest[end + 4..]
        .trim_start_matches(['\r', '\n'])
        .to_string();
    let meta = head
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    (meta, body)
}

/// 列出全部规则（按更新时间倒序）。解析失败的文件也会列出并带 `error`。
pub fn list() -> Vec<RuleEntry> {
    let dir = rules_dir();
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut items: Vec<RuleEntry> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .map(|p| match parse_file(&p) {
            Ok((entry, _)) => entry,
            Err(e) => RuleEntry {
                id: p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                name: p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                scope: "personal".to_string(),
                enabled: false,
                chars: 0,
                updated_at: String::new(),
                path: p.clone(),
                error: Some(e),
            },
        })
        .collect();
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    items
}

/// 读取单条规则的正文。
pub fn get(id: &str) -> Option<(RuleEntry, String)> {
    let path = rules_dir().join(format!("{id}.md"));
    if !path.is_file() {
        return None;
    }
    parse_file(&path).ok()
}

/// 保存（新建或覆盖）一条规则；返回落盘后的条目。
pub fn save(
    id: Option<&str>,
    name: &str,
    scope: &str,
    enabled: bool,
    content: &str,
) -> Result<RuleEntry, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("规则名称不能为空".to_string());
    }
    if content.chars().count() > MAX_RULE_CHARS {
        return Err(format!("规则正文超过 {MAX_RULE_CHARS} 字符上限"));
    }
    let scope = match scope {
        "project" => "project",
        _ => "personal",
    };
    let id = id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| slug(name));
    let dir = rules_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建规则目录失败：{e}"))?;
    let path = dir.join(format!("{id}.md"));
    let updated = chrono::Utc::now().to_rfc3339();
    let body = format!(
        "---\nname: {name}\nscope: {scope}\nenabled: {enabled}\nupdated: {updated}\n---\n\n{}\n",
        content.trim()
    );
    std::fs::write(&path, body).map_err(|e| format!("写入规则失败：{e}"))?;
    parse_file(&path).map(|(e, _)| e)
}

/// 删除一条规则；返回是否真的删掉了文件。
pub fn delete(id: &str) -> bool {
    let path = rules_dir().join(format!("{id}.md"));
    std::fs::remove_file(path).is_ok()
}

/// 生效文本缓存：(取样时刻, 注入文本)。2 秒内复用，避免每轮读盘。
type ActiveCache = Mutex<Option<(Instant, Option<String>)>>;

/// 启用中规则的注入文本（带 2 秒缓存，避免每个回合都读盘）。
pub fn active_text() -> Option<String> {
    static CACHE: OnceLock<ActiveCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, text)) = guard.as_ref() {
            if at.elapsed() < Duration::from_secs(2) {
                return text.clone();
            }
        }
    }
    let text = build_active_text();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), text.clone()));
    }
    text
}

fn build_active_text() -> Option<String> {
    let mut parts: Vec<(String, String)> = Vec::new();
    for entry in list() {
        if !entry.enabled || entry.error.is_some() {
            continue;
        }
        let Some((_, body)) = get(&entry.id) else {
            continue;
        };
        if body.trim().is_empty() {
            continue;
        }
        parts.push((entry.name, body.trim().to_string()));
    }
    if parts.is_empty() {
        return None;
    }
    let mut out = String::from(
        "【用户规则——必须遵守，优先级高于默认风格；与系统安全策略冲突时以安全策略为准】\n",
    );
    for (name, body) in parts {
        if out.chars().count() >= MAX_INJECT_CHARS {
            out.push_str("\n（其余规则因长度上限未注入）");
            break;
        }
        out.push_str(&format!("\n## {name}\n{body}\n"));
    }
    Some(out)
}

// ── RPC ────────────────────────────────────────────────────────────────

fn entry_json(entry: &RuleEntry) -> Value {
    json!({
        "id": entry.id,
        "name": entry.name,
        "scope": entry.scope,
        "enabled": entry.enabled,
        "chars": entry.chars,
        "updatedAt": entry.updated_at,
        "path": entry.path.to_string_lossy(),
        "error": entry.error,
    })
}

/// `rules.list`：全部规则（不含正文）。
pub fn rules_list(_params: &Value) -> Value {
    json!({
        "ok": true,
        "dir": rules_dir().to_string_lossy(),
        "rules": list().iter().map(entry_json).collect::<Vec<_>>(),
        "activeChars": active_text().map(|t| t.chars().count()).unwrap_or(0),
    })
}

/// `rules.get`：单条规则正文。
pub fn rules_get(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.trim().is_empty() {
        return err("缺少规则 id", "INVALID_REQUEST");
    }
    match get(id.trim()) {
        Some((entry, content)) => json!({
            "ok": true,
            "rule": entry_json(&entry),
            "content": content,
        }),
        None => err(format!("规则不存在：{id}"), "RULE_NOT_FOUND"),
    }
}

/// `rules.save`：新建/覆盖（按 id 或按名字生成 id）。
pub fn rules_save(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str);
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let scope = params
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("personal");
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("");
    match save(id, name, scope, enabled, content) {
        Ok(entry) => json!({ "ok": true, "rule": entry_json(&entry) }),
        Err(e) => err(e, "RULE_SAVE_FAILED"),
    }
}

/// `rules.delete`：删除规则（不可恢复）。
pub fn rules_delete(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.trim().is_empty() {
        return err("缺少规则 id", "INVALID_REQUEST");
    }
    if delete(id.trim()) {
        json!({ "ok": true, "deleted": id })
    } else {
        err(format!("规则不存在：{id}"), "RULE_NOT_FOUND")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// frontmatter 解析：缺省值、enabled=false、无 frontmatter 三种形态。
    #[test]
    fn parses_frontmatter_variants() {
        let (meta, body) = split_frontmatter("---\nname: 代码规范\nenabled: false\n---\n\n正文\n");
        assert_eq!(
            meta.iter()
                .find(|(k, _)| k == "name")
                .map(|(_, v)| v.as_str()),
            Some("代码规范")
        );
        assert!(meta.iter().any(|(k, v)| k == "enabled" && v == "false"));
        assert_eq!(body, "正文\n");

        let (meta2, body2) = split_frontmatter("没有头部的正文");
        assert!(meta2.is_empty());
        assert_eq!(body2, "没有头部的正文");
    }

    /// slug：中文名 → 空 → hash 兜底；英文名 → kebab；不会产生空 id。
    #[test]
    fn slug_is_stable_and_never_empty() {
        assert_eq!(slug("Code Style Guide"), "code-style-guide");
        assert_eq!(slug("   "), slug("   "));
        assert!(slug("中文名").starts_with("rule-"));
        assert_eq!(slug("a/b\\\\c"), "a-b-c");
    }
}
