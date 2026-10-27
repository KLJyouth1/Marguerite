//! 内容更新源与版本同步（Rust 网关原生实现）。
//!
//! 桌面端的内容（技能 / 插件）从指定 GitHub 仓库同步，版本信息与 Release 也从
//! 同一仓库读取。落盘约定：
//! - 配置：`Config::data_dir()/content-source.json`
//!   （`repo` / `branch` / `checkedAt` / `syncedAt`）
//! - 内容：`Config::data_dir()/content/{skills,plugins}/<name>/...`
//!
//! 诚实约束（违反即缺陷）：
//! - 网络失败 / HTTP 非 2xx / 无 Release **绝不假成功**，一律返回 `ok:false` + 明确 code；
//! - 同步的 dry-run 只报告真实差异，`apply:true` 才写盘；
//! - 远端清单里的路径必须逐段校验（拒绝 `..`、绝对路径、盘符、反斜杠），
//!   否则一个恶意清单就能写到数据目录之外。
//!
//! 该方法族**不**进入 `PY_ENGINE_PREFIXES`：内容源与本地数据目录只有 Rust 侧知道。

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::gateway::methods::GatewayContext;
use crate::persistence::config::Config;

/// 默认内容源仓库（与项目 git origin 一致）。
pub const DEFAULT_REPO: &str = "KLJyouth1/Marguerite";
/// 默认分支。
pub const DEFAULT_BRANCH: &str = "main";

const CONFIG_FILE: &str = "content-source.json";
const CONTENT_DIR: &str = "content";
const HTTP_TIMEOUT_SECS: u64 = 10;
/// 单个响应体上限（防止异常响应把内存吃光）。
const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;
const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";

fn user_agent() -> String {
    format!("Marguerite-Desktop/{}", env!("CARGO_PKG_VERSION"))
}

// ───────────────────────────────────────────────────────────────────────────
// 配置
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSourceConfig {
    pub repo: String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<String>,
}

impl Default for ContentSourceConfig {
    fn default() -> Self {
        Self {
            repo: DEFAULT_REPO.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            checked_at: None,
            synced_at: None,
        }
    }
}

struct LoadedConfig {
    cfg: ContentSourceConfig,
    configured: bool,
}

fn config_path(root: &Path) -> PathBuf {
    root.join(CONFIG_FILE)
}

/// 读取配置：文件缺失 / 损坏都回落到默认值，并如实标记 `configured:false`。
fn load_config_at(root: &Path) -> LoadedConfig {
    let Ok(text) = std::fs::read_to_string(config_path(root)) else {
        return LoadedConfig {
            cfg: ContentSourceConfig::default(),
            configured: false,
        };
    };
    match serde_json::from_str::<ContentSourceConfig>(&text) {
        Ok(mut cfg) => {
            if cfg.repo.trim().is_empty() {
                cfg.repo = DEFAULT_REPO.to_string();
            }
            if cfg.branch.trim().is_empty() {
                cfg.branch = DEFAULT_BRANCH.to_string();
            }
            LoadedConfig {
                cfg,
                configured: true,
            }
        }
        Err(e) => {
            tracing::warn!(err = %e, "content-source.json 解析失败，回落到默认内容源");
            LoadedConfig {
                cfg: ContentSourceConfig::default(),
                configured: false,
            }
        }
    }
}

/// 原子写配置：先写 `.tmp`，再替换目标（Windows 下 rename 不覆盖已存在文件）。
fn save_config_at(root: &Path, cfg: &ContentSourceConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let target = config_path(root);
    let tmp = target.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, bytes)?;
    if target.exists() {
        // 配置很小，直接删旧再改名即可；失败时把 tmp 清掉并如实返回错误。
        if let Err(e) = std::fs::remove_file(&target) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn repo_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$").expect("repo 正则必须可编译")
    })
}

fn content_dir(root: &Path) -> PathBuf {
    root.join(CONTENT_DIR)
}

fn kind_dir(root: &Path, kind: &str) -> PathBuf {
    content_dir(root).join(kind)
}

/// 清单 `kind` → 磁盘目录名（调用前已校验 kind 合法）。
fn kind_dir_name(kind: &str) -> &'static str {
    if kind == "skill" {
        "skills"
    } else {
        "plugins"
    }
}

/// 真实条目数（目录不存在 / 不可读 → 0，绝不编造）。
fn count_entries(dir: &Path) -> usize {
    match std::fs::read_dir(dir) {
        Ok(iter) => iter.filter_map(Result::ok).count(),
        Err(_) => 0,
    }
}

fn config_view(root: &Path, loaded: &LoadedConfig) -> Value {
    json!({
        "ok": true,
        "configured": loaded.configured,
        "repo": loaded.cfg.repo,
        "branch": loaded.cfg.branch,
        "currentVersion": env!("CARGO_PKG_VERSION"),
        "checkedAt": loaded.cfg.checked_at,
        "syncedAt": loaded.cfg.synced_at,
        "content": {
            "skills": count_entries(&kind_dir(root, "skills")),
            "plugins": count_entries(&kind_dir(root, "plugins")),
        },
    })
}

/// `update.config.get`
pub fn update_config_get() -> Value {
    update_config_get_at(&Config::data_dir())
}

fn update_config_get_at(root: &Path) -> Value {
    let loaded = load_config_at(root);
    config_view(root, &loaded)
}

/// `update.config.set`
///
/// 校验失败返回 `{ok:false, code:"INVALID_REQUEST", message}`（不抛协议错误，
/// 让前端能一次性拿到字段级原因并留在面板里继续改）。
pub fn update_config_set(params: &Value) -> Value {
    update_config_set_at(&Config::data_dir(), params)
}

fn update_config_set_at(root: &Path, params: &Value) -> Value {
    let mut loaded = load_config_at(root);
    let repo = match params.get("repo").and_then(Value::as_str) {
        Some(v) => v.trim().to_string(),
        None => loaded.cfg.repo.clone(),
    };
    if !repo_regex().is_match(&repo) {
        return json!({
            "ok": false,
            "code": "INVALID_REQUEST",
            "message": "repo 必须形如 owner/name（仅允许字母、数字、点、下划线、连字符）",
        });
    }
    let branch = match params.get("branch").and_then(Value::as_str) {
        Some(v) => v.trim().to_string(),
        None => loaded.cfg.branch.clone(),
    };
    if branch.is_empty() || branch.chars().any(char::is_whitespace) {
        return json!({
            "ok": false,
            "code": "INVALID_REQUEST",
            "message": "branch 不可为空且不能包含空白字符",
        });
    }
    loaded.cfg.repo = repo;
    loaded.cfg.branch = branch;
    if let Err(e) = save_config_at(root, &loaded.cfg) {
        return json!({
            "ok": false,
            "code": "WRITE_FAILED",
            "message": format!("内容源配置写入失败：{e}"),
        });
    }
    config_view(
        root,
        &LoadedConfig {
            cfg: loaded.cfg,
            configured: true,
        },
    )
}

// ───────────────────────────────────────────────────────────────────────────
// HTTP（可注入，单测不触网）
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum FetchError {
    Network(String),
    Status(u16),
}

trait HttpFetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError>;
}

struct UreqFetcher;

impl HttpFetch for UreqFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        // User-Agent 是 GitHub API 的硬要求（缺失直接 403），超时 10s 兜底。
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .user_agent(&user_agent())
            .build();
        match agent.get(url).call() {
            Ok(resp) => {
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(MAX_BODY_BYTES)
                    .read_to_end(&mut buf)
                    .map_err(|e| FetchError::Network(e.to_string()))?;
                Ok(buf)
            }
            Err(ureq::Error::Status(code, _)) => Err(FetchError::Status(code)),
            Err(ureq::Error::Transport(t)) => Err(FetchError::Network(t.to_string())),
        }
    }
}

fn fetch_error_code(err: &FetchError) -> (String, String) {
    match err {
        FetchError::Status(code) => (format!("HTTP_{code}"), format!("远端返回 HTTP {code}")),
        FetchError::Network(msg) => ("NET_ERROR".into(), format!("网络请求失败：{msg}")),
    }
}

fn fetch_error_value(err: &FetchError) -> Value {
    let (code, message) = fetch_error_code(err);
    json!({ "ok": false, "code": code, "message": message })
}

fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ───────────────────────────────────────────────────────────────────────────
// Release 检查
// ───────────────────────────────────────────────────────────────────────────

/// 解析 `v1.2.3` / `1.2.3` / `1.2.3-beta.1` 为 `(major, minor, patch)`。
fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim();
    let without_prefix = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    let core = without_prefix
        .split(['-', '+'])
        .next()
        .unwrap_or(without_prefix);
    let mut parts = core.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// `update.check`
pub fn update_check() -> Value {
    update_check_at(&Config::data_dir(), &UreqFetcher, GITHUB_API_BASE)
}

fn update_check_at(root: &Path, fetcher: &dyn HttpFetch, api_base: &str) -> Value {
    let mut loaded = load_config_at(root);
    let url = format!(
        "{}/{}/releases/latest",
        api_base.trim_end_matches('/'),
        loaded.cfg.repo
    );
    let body = match fetcher.get(&url) {
        Ok(body) => body,
        Err(FetchError::Status(404)) => {
            return json!({
                "ok": false,
                "code": "NO_RELEASES",
                "message": format!("仓库 {} 还没有发布 Release", loaded.cfg.repo),
            });
        }
        Err(err) => return fetch_error_value(&err),
    };
    let release: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "ok": false,
                "code": "BAD_RESPONSE",
                "message": format!("GitHub Release 响应解析失败：{e}"),
            });
        }
    };
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let name = release
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let notes = release
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let html_url = release
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let assets: Vec<Value> = release
        .get("assets")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|a| {
                    json!({
                        "name": a.get("name").and_then(Value::as_str).unwrap_or_default(),
                        "browser_download_url": a
                            .get("browser_download_url")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        "size": a.get("size").and_then(Value::as_u64),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let current_version = env!("CARGO_PKG_VERSION");
    let latest_semver = parse_semver(&tag);
    let current_semver = parse_semver(current_version);
    let update_available = match (latest_semver, current_semver) {
        (Some(latest), Some(current)) => Some(latest > current),
        _ => None,
    };

    // 成功检查后写回 checkedAt（写失败不影响本次结论，但要留下可观测日志）。
    loaded.cfg.checked_at = Some(chrono::Utc::now().to_rfc3339());
    if let Err(e) = save_config_at(root, &loaded.cfg) {
        tracing::warn!(err = %e, "checkedAt 写入失败");
    }

    let mut out = json!({
        "ok": true,
        "latestTag": tag,
        "latestName": name,
        "notes": notes,
        "htmlUrl": html_url,
        "assets": assets,
        "currentVersion": current_version,
        "updateAvailable": update_available,
    });
    if update_available.is_none() {
        out["note"] = json!(format!(
            "无法解析版本号（tag=`{tag}`，current=`{current_version}`），请人工核对"
        ));
    }
    out
}

// ───────────────────────────────────────────────────────────────────────────
// 内容清单同步
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: u32,
    #[serde(default)]
    items: Vec<ManifestItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestItem {
    kind: String,
    name: String,
    path: String,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffKind {
    Added,
    Changed,
    Unchanged,
}

struct PreparedItem {
    kind: String,
    name: String,
    path: String,
    local_rel: PathBuf,
    bytes: Vec<u8>,
    diff: DiffKind,
}

/// 校验单段路径（kind / name）：必须是普通文件名，不能是 `..`、盘符或分隔符。
fn is_safe_segment(segment: &str) -> bool {
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    if segment.contains('/') || segment.contains('\\') || segment.contains(':') {
        return false;
    }
    // Windows 会静默去掉结尾的点/空格（`SKILL.md.` 与 `SKILL.md` 落到同一文件），
    // 这类名字会让清单校验与真实落点不一致，直接拒绝。
    if segment.ends_with('.') || segment.ends_with(' ') {
        return false;
    }
    !segment.chars().any(char::is_control)
}

/// 校验远端清单里的 `path`：拒绝绝对路径、盘符、反斜杠、空段、`.`/`..`。
///
/// 返回规范化后的相对路径（只用 `/` 分段），任何异常都返回 `None`。
fn safe_remote_rel(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() || raw.contains('\\') || raw.contains('\0') || raw.starts_with('/') {
        return None;
    }
    // 盘符（`C:`）与任何冒号一律拒绝：避免 Windows 前缀语义。
    if raw.contains(':') {
        return None;
    }
    let mut rel = PathBuf::new();
    for segment in raw.split('/') {
        if !is_safe_segment(segment) {
            return None;
        }
        rel.push(segment);
    }
    // 双保险：解析后必须全部是 Normal 组件。
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(rel)
}

/// 把清单项映射为本地相对路径：去掉 `<kind_dir>/<name>/` 前缀，保留其余目录结构。
fn item_local_rel(item: &ManifestItem) -> Result<PathBuf, String> {
    if !is_safe_segment(&item.name) {
        return Err(format!("非法条目名 `{}`", item.name));
    }
    let kind_dir = match item.kind.as_str() {
        "skill" => "skills",
        "plugin" => "plugins",
        other => return Err(format!("未知条目类型 `{other}`（仅支持 skill / plugin）")),
    };
    let rel =
        safe_remote_rel(&item.path).ok_or_else(|| format!("清单路径不安全：`{}`", item.path))?;
    let expected_prefix = PathBuf::from(kind_dir).join(&item.name);
    let tail = rel.strip_prefix(&expected_prefix).map_err(|_| {
        format!(
            "清单路径 `{}` 不在 `{kind_dir}/{}/` 之下",
            item.path, item.name
        )
    })?;
    if tail.as_os_str().is_empty() {
        return Err(format!("清单路径 `{}` 缺少文件名", item.path));
    }
    Ok(tail.to_path_buf())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn item_json(item: &PreparedItem) -> Value {
    json!({ "kind": item.kind, "name": item.name, "path": item.path })
}

fn diff_lists(items: &[PreparedItem]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    for item in items {
        match item.diff {
            DiffKind::Added => added.push(item_json(item)),
            DiffKind::Changed => changed.push(item_json(item)),
            DiffKind::Unchanged => unchanged.push(item_json(item)),
        }
    }
    (added, changed, unchanged)
}

/// 原子落盘：先写同目录 `.tmp`，旧文件备份成 `.bak-<ts>`，再 rename。
fn write_atomic(target: &Path, bytes: &[u8], stamp: &str) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_os = target.as_os_str().to_os_string();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    std::fs::write(&tmp, bytes)?;
    if target.exists() {
        let mut bak_os = target.as_os_str().to_os_string();
        bak_os.push(format!(".bak-{stamp}"));
        let bak = PathBuf::from(bak_os);
        if let Err(e) = std::fs::rename(target, &bak) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// `update.syncContent`
pub fn update_sync_content(ctx: &GatewayContext, params: &Value) -> Value {
    let apply = params
        .get("apply")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let out = sync_content_at(&Config::data_dir(), &UreqFetcher, GITHUB_RAW_BASE, apply);
    let applied = out.get("applied").and_then(Value::as_bool) == Some(true);
    if applied && out.get("ok").and_then(Value::as_bool) == Some(true) {
        // 内容变了 → 技能索引缓存必须失效，并复用既有 reload 逻辑刷新一代。
        crate::agent::tool_catalog::invalidate_skill_cache();
        let _ = crate::gateway::skills::skills_reload(ctx);
    }
    out
}

fn sync_content_at(root: &Path, fetcher: &dyn HttpFetch, raw_base: &str, apply: bool) -> Value {
    let loaded = load_config_at(root);
    let repo = loaded.cfg.repo.clone();
    let branch = loaded.cfg.branch.clone();
    let manifest_url = format!(
        "{}/{repo}/{branch}/update-manifest.json",
        raw_base.trim_end_matches('/')
    );
    let manifest_bytes = match fetcher.get(&manifest_url) {
        Ok(bytes) => bytes,
        Err(FetchError::Status(404)) => {
            return json!({
                "ok": false,
                "code": "MANIFEST_MISSING",
                "message": format!(
                    "仓库 {repo}（分支 {branch}）缺少 update-manifest.json"
                ),
            });
        }
        Err(err) => return fetch_error_value(&err),
    };
    let manifest: Manifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(m) => m,
        Err(e) => {
            return json!({
                "ok": false,
                "code": "INVALID_MANIFEST",
                "message": format!("update-manifest.json 解析失败：{e}"),
            });
        }
    };
    if manifest.version != 1 {
        return json!({
            "ok": false,
            "code": "UNSUPPORTED_MANIFEST",
            "message": format!("不认识的清单版本 {}（仅支持 version=1）", manifest.version),
        });
    }

    let mut prepared: Vec<PreparedItem> = Vec::with_capacity(manifest.items.len());
    for item in &manifest.items {
        let local_rel = match item_local_rel(item) {
            Ok(p) => p,
            Err(message) => {
                return json!({ "ok": false, "code": "INVALID_MANIFEST", "message": message });
            }
        };
        let file_url = format!(
            "{}/{repo}/{branch}/{}",
            raw_base.trim_end_matches('/'),
            encode_path(&item.path)
        );
        let bytes = match fetcher.get(&file_url) {
            Ok(b) => b,
            Err(err) => {
                let mut v = fetch_error_value(&err);
                let (_, detail) = fetch_error_code(&err);
                v["message"] = json!(format!("拉取 `{}` 失败：{detail}", item.path));
                return v;
            }
        };
        let remote_sha = sha256_hex(&bytes);
        if let Some(expected) = item.sha256.as_deref() {
            if !expected.trim().is_empty() && !expected.eq_ignore_ascii_case(&remote_sha) {
                return json!({
                    "ok": false,
                    "code": "CHECKSUM_MISMATCH",
                    "message": format!(
                        "`{}` 校验失败：清单 sha256 与下载内容不一致",
                        item.path
                    ),
                });
            }
        }
        let local_path = kind_dir(root, kind_dir_name(&item.kind))
            .join(&item.name)
            .join(&local_rel);
        let diff = if !local_path.is_file() {
            DiffKind::Added
        } else {
            match std::fs::read(&local_path) {
                Ok(local_bytes) if sha256_hex(&local_bytes) == remote_sha => DiffKind::Unchanged,
                _ => DiffKind::Changed,
            }
        };
        prepared.push(PreparedItem {
            kind: item.kind.clone(),
            name: item.name.clone(),
            path: item.path.clone(),
            local_rel,
            bytes,
            diff,
        });
    }

    let total = prepared.len();
    if !apply {
        let (added, changed, unchanged) = diff_lists(&prepared);
        return json!({
            "ok": true,
            "applied": false,
            "added": added,
            "changed": changed,
            "unchanged": unchanged,
            "total": total,
        });
    }

    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S%3f").to_string();
    for item in &prepared {
        if item.diff == DiffKind::Unchanged {
            continue;
        }
        let dir_kind = kind_dir_name(&item.kind);
        let target = kind_dir(root, dir_kind)
            .join(&item.name)
            .join(&item.local_rel);
        if let Err(e) = write_atomic(&target, &item.bytes, &stamp) {
            return json!({
                "ok": false,
                "code": "APPLY_FAILED",
                "message": format!("写入 `{}` 失败：{e}", item.path),
                "total": total,
            });
        }
    }

    // 成功 apply 后才写 syncedAt（失败时不允许出现"已同步"的假时间）。
    let mut cfg = loaded.cfg;
    cfg.synced_at = Some(chrono::Utc::now().to_rfc3339());
    if let Err(e) = save_config_at(root, &cfg) {
        tracing::warn!(err = %e, "syncedAt 写入失败");
    }
    let (added, changed, unchanged) = diff_lists(&prepared);
    json!({
        "ok": true,
        "applied": true,
        "added": added,
        "changed": changed,
        "unchanged": unchanged,
        "total": total,
        "syncedAt": cfg.synced_at,
    })
}

// ───────────────────────────────────────────────────────────────────────────
// 单测（不触网：HTTP 走 FakeFetcher，落盘走临时目录）
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let dir = std::env::temp_dir()
                .join(format!("marguerite-update-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).expect("创建临时目录");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct FakeFetcher {
        routes: Mutex<BTreeMap<String, Result<Vec<u8>, FetchError>>>,
    }

    impl FakeFetcher {
        fn new() -> Self {
            Self {
                routes: Mutex::new(BTreeMap::new()),
            }
        }

        fn ok(self, url: &str, body: &str) -> Self {
            self.routes
                .lock()
                .expect("routes lock")
                .insert(url.to_string(), Ok(body.as_bytes().to_vec()));
            self
        }

        fn status(self, url: &str, code: u16) -> Self {
            self.routes
                .lock()
                .expect("routes lock")
                .insert(url.to_string(), Err(FetchError::Status(code)));
            self
        }
    }

    impl HttpFetch for FakeFetcher {
        fn get(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            match self.routes.lock().expect("routes lock").get(url) {
                Some(Ok(body)) => Ok(body.clone()),
                Some(Err(e)) => Err(e.clone()),
                None => Err(FetchError::Status(404)),
            }
        }
    }

    fn write_config(root: &Path, repo: &str, branch: &str) {
        save_config_at(
            root,
            &ContentSourceConfig {
                repo: repo.to_string(),
                branch: branch.to_string(),
                checked_at: None,
                synced_at: None,
            },
        )
        .expect("写配置");
    }

    #[test]
    fn update_config_get_reports_defaults_and_real_counts() {
        let root = TempRoot::new();
        std::fs::create_dir_all(root.path().join("content/skills/alpha")).unwrap();
        std::fs::create_dir_all(root.path().join("content/skills/beta")).unwrap();
        std::fs::create_dir_all(root.path().join("content/plugins/gamma")).unwrap();

        let view = update_config_get_at(root.path());
        assert_eq!(view["ok"], true);
        assert_eq!(view["configured"], false, "配置不存在时不得假装已配置");
        assert_eq!(view["repo"], DEFAULT_REPO);
        assert_eq!(view["branch"], DEFAULT_BRANCH);
        assert_eq!(view["currentVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(view["content"]["skills"], 2);
        assert_eq!(view["content"]["plugins"], 1);
        assert!(view["checkedAt"].is_null());
        assert!(view["syncedAt"].is_null());
    }

    #[test]
    fn update_config_set_validates_repo_and_branch() {
        let root = TempRoot::new();

        let bad_repo = update_config_set_at(
            root.path(),
            &json!({ "repo": "not-a-repo", "branch": "main" }),
        );
        assert_eq!(bad_repo["ok"], false);
        assert_eq!(bad_repo["code"], "INVALID_REQUEST");

        let bad_repo2 =
            update_config_set_at(root.path(), &json!({ "repo": "a/b/c", "branch": "main" }));
        assert_eq!(bad_repo2["code"], "INVALID_REQUEST");

        let bad_branch = update_config_set_at(
            root.path(),
            &json!({ "repo": "owner/name", "branch": "ma in" }),
        );
        assert_eq!(bad_branch["ok"], false);
        assert_eq!(bad_branch["code"], "INVALID_REQUEST");

        let empty_branch = update_config_set_at(
            root.path(),
            &json!({ "repo": "owner/name", "branch": "  " }),
        );
        assert_eq!(empty_branch["code"], "INVALID_REQUEST");

        let ok = update_config_set_at(
            root.path(),
            &json!({ "repo": "KLJyouth1/Marguerite", "branch": "dev" }),
        );
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["configured"], true);
        assert_eq!(ok["repo"], "KLJyouth1/Marguerite");
        assert_eq!(ok["branch"], "dev");
        assert!(config_path(root.path()).is_file());

        let reread = update_config_get_at(root.path());
        assert_eq!(reread["configured"], true);
        assert_eq!(reread["branch"], "dev");
    }

    #[test]
    fn update_semver_comparison_is_honest() {
        assert_eq!(parse_semver("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("0.2.16"), Some((0, 2, 16)));
        assert_eq!(parse_semver("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_semver("v1.2.3+build.9"), Some((1, 2, 3)));
        assert_eq!(parse_semver("release-1.2.3"), None);
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver("1.2.3.4"), None);
        assert!(parse_semver("v0.3.0") > parse_semver("0.2.16"));
    }

    #[test]
    fn update_check_surfaces_network_and_missing_release() {
        let root = TempRoot::new();
        write_config(root.path(), "owner/name", "main");
        let api = "https://api.example.test";
        let url = format!("{api}/owner/name/releases/latest");

        let missing = update_check_at(root.path(), &FakeFetcher::new().status(&url, 404), api);
        assert_eq!(missing["ok"], false);
        assert_eq!(missing["code"], "NO_RELEASES");

        let forbidden = update_check_at(root.path(), &FakeFetcher::new().status(&url, 403), api);
        assert_eq!(forbidden["ok"], false);
        assert_eq!(forbidden["code"], "HTTP_403");

        struct Offline;
        impl HttpFetch for Offline {
            fn get(&self, _url: &str) -> Result<Vec<u8>, FetchError> {
                Err(FetchError::Network("timed out".into()))
            }
        }
        let offline = update_check_at(root.path(), &Offline, api);
        assert_eq!(offline["ok"], false);
        assert_eq!(offline["code"], "NET_ERROR");
        assert!(offline["message"].as_str().unwrap().contains("timed out"));
        // 失败不得写 checkedAt
        assert!(update_config_get_at(root.path())["checkedAt"].is_null());
    }

    #[test]
    fn update_check_parses_release_and_stamps_checked_at() {
        let root = TempRoot::new();
        write_config(root.path(), "owner/name", "main");
        let api = "https://api.example.test";
        let url = format!("{api}/owner/name/releases/latest");
        let body = json!({
            "tag_name": "v9.9.9",
            "name": "Marguerite 9.9.9",
            "body": "release notes",
            "html_url": "https://github.com/owner/name/releases/tag/v9.9.9",
            "assets": [{
                "name": "Marguerite-Setup.exe",
                "browser_download_url": "https://example.test/dl.exe",
                "size": 1234
            }]
        })
        .to_string();

        let res = update_check_at(root.path(), &FakeFetcher::new().ok(&url, &body), api);
        assert_eq!(res["ok"], true);
        assert_eq!(res["latestTag"], "v9.9.9");
        assert_eq!(res["latestName"], "Marguerite 9.9.9");
        assert_eq!(res["notes"], "release notes");
        assert_eq!(
            res["htmlUrl"],
            "https://github.com/owner/name/releases/tag/v9.9.9"
        );
        assert_eq!(res["updateAvailable"], true);
        assert_eq!(res["assets"][0]["name"], "Marguerite-Setup.exe");
        assert_eq!(res["assets"][0]["size"], 1234);
        assert!(res["note"].is_null());

        let view = update_config_get_at(root.path());
        assert!(
            view["checkedAt"].as_str().unwrap_or_default().contains('T'),
            "成功检查后必须写回 checkedAt：{view}"
        );
    }

    #[test]
    fn update_check_unparseable_tag_reports_null_instead_of_guessing() {
        let root = TempRoot::new();
        write_config(root.path(), "owner/name", "main");
        let api = "https://api.example.test";
        let url = format!("{api}/owner/name/releases/latest");
        let body =
            json!({ "tag_name": "nightly", "name": "n", "body": "", "html_url": "" }).to_string();

        let res = update_check_at(root.path(), &FakeFetcher::new().ok(&url, &body), api);
        assert_eq!(res["ok"], true);
        assert!(res["updateAvailable"].is_null());
        assert!(res["note"].as_str().unwrap().contains("无法解析版本号"));
    }

    #[test]
    fn update_manifest_paths_reject_traversal_and_absolute_paths() {
        assert!(safe_remote_rel("skills/a/SKILL.md").is_some());
        assert!(safe_remote_rel("skills/../evil.md").is_none());
        assert!(safe_remote_rel("skills/a/../../evil.md").is_none());
        assert!(safe_remote_rel("/etc/passwd").is_none());
        assert!(safe_remote_rel("C:/Windows/evil.md").is_none());
        assert!(safe_remote_rel("C:\\Windows\\evil.md").is_none());
        assert!(safe_remote_rel("skills//SKILL.md").is_none());
        assert!(safe_remote_rel("./skills/a/SKILL.md").is_none());
        // Windows 会吞掉结尾的点/空格：`SKILL.md.` 与 `SKILL.md` 是同一落点。
        assert!(safe_remote_rel("skills/a/SKILL.md.").is_none());
        assert!(safe_remote_rel("skills/a /SKILL.md").is_none());

        let valid = ManifestItem {
            kind: "skill".into(),
            name: "a".into(),
            path: "skills/a/SKILL.md".into(),
            sha256: None,
        };
        assert_eq!(item_local_rel(&valid).unwrap(), PathBuf::from("SKILL.md"));

        let escape = ManifestItem {
            kind: "skill".into(),
            name: "a".into(),
            path: "skills/a/../../evil.md".into(),
            sha256: None,
        };
        assert!(item_local_rel(&escape).is_err());

        let foreign = ManifestItem {
            kind: "skill".into(),
            name: "a".into(),
            path: "plugins/b/plugin.json".into(),
            sha256: None,
        };
        assert!(item_local_rel(&foreign).is_err());

        let bad_name = ManifestItem {
            kind: "skill".into(),
            name: "../a".into(),
            path: "skills/../a/SKILL.md".into(),
            sha256: None,
        };
        assert!(item_local_rel(&bad_name).is_err());

        let bad_kind = ManifestItem {
            kind: "binary".into(),
            name: "a".into(),
            path: "skills/a/SKILL.md".into(),
            sha256: None,
        };
        assert!(item_local_rel(&bad_kind).is_err());
    }

    #[test]
    fn update_sync_missing_or_unknown_manifest_is_rejected() {
        let root = TempRoot::new();
        write_config(root.path(), "owner/name", "main");
        let raw = "https://raw.example.test";
        let manifest_url = format!("{raw}/owner/name/main/update-manifest.json");

        let missing = sync_content_at(
            root.path(),
            &FakeFetcher::new().status(&manifest_url, 404),
            raw,
            false,
        );
        assert_eq!(missing["ok"], false);
        assert_eq!(missing["code"], "MANIFEST_MISSING");

        let v2 = sync_content_at(
            root.path(),
            &FakeFetcher::new().ok(&manifest_url, r#"{"version":2,"items":[]}"#),
            raw,
            false,
        );
        assert_eq!(v2["ok"], false);
        assert_eq!(v2["code"], "UNSUPPORTED_MANIFEST");

        let traversal = sync_content_at(
            root.path(),
            &FakeFetcher::new().ok(
                &manifest_url,
                r#"{"version":1,"items":[{"kind":"skill","name":"a","path":"skills/a/../../evil.md"}]}"#,
            ),
            raw,
            false,
        );
        assert_eq!(traversal["ok"], false);
        assert_eq!(traversal["code"], "INVALID_MANIFEST");
        assert!(!root.path().join("evil.md").exists());
    }

    #[test]
    fn update_sync_dry_run_diff_then_apply_writes_and_backs_up() {
        let root = TempRoot::new();
        write_config(root.path(), "owner/name", "main");
        // 既有内容：same.md 内容与远端一致；old.md 内容过期。
        std::fs::create_dir_all(root.path().join("content/skills/same")).unwrap();
        std::fs::create_dir_all(root.path().join("content/skills/old")).unwrap();
        std::fs::write(
            root.path().join("content/skills/same/SKILL.md"),
            "same-body",
        )
        .unwrap();
        std::fs::write(
            root.path().join("content/skills/old/SKILL.md"),
            "stale-body",
        )
        .unwrap();

        let raw = "https://raw.example.test";
        let manifest_url = format!("{raw}/owner/name/main/update-manifest.json");
        let manifest = json!({
            "version": 1,
            "items": [
                { "kind": "skill", "name": "same", "path": "skills/same/SKILL.md" },
                { "kind": "skill", "name": "old", "path": "skills/old/SKILL.md" },
                { "kind": "plugin", "name": "fresh", "path": "plugins/fresh/manifest.json" }
            ]
        })
        .to_string();
        let fetcher = FakeFetcher::new()
            .ok(&manifest_url, &manifest)
            .ok(
                &format!("{raw}/owner/name/main/skills/same/SKILL.md"),
                "same-body",
            )
            .ok(
                &format!("{raw}/owner/name/main/skills/old/SKILL.md"),
                "new-body",
            )
            .ok(
                &format!("{raw}/owner/name/main/plugins/fresh/manifest.json"),
                "{}",
            );

        let dry = sync_content_at(root.path(), &fetcher, raw, false);
        assert_eq!(dry["ok"], true);
        assert_eq!(dry["applied"], false);
        assert_eq!(dry["total"], 3);
        assert_eq!(dry["added"].as_array().unwrap().len(), 1);
        assert_eq!(dry["changed"].as_array().unwrap().len(), 1);
        assert_eq!(dry["unchanged"].as_array().unwrap().len(), 1);
        // dry-run 不写盘
        assert_eq!(
            std::fs::read_to_string(root.path().join("content/skills/old/SKILL.md")).unwrap(),
            "stale-body"
        );
        assert!(update_config_get_at(root.path())["syncedAt"].is_null());

        let applied = sync_content_at(root.path(), &fetcher, raw, true);
        assert_eq!(applied["ok"], true);
        assert_eq!(applied["applied"], true);
        assert_eq!(
            std::fs::read_to_string(root.path().join("content/skills/old/SKILL.md")).unwrap(),
            "new-body"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("content/plugins/fresh/manifest.json"))
                .unwrap(),
            "{}"
        );
        // 旧文件必须留下 .bak-<ts> 备份
        let backups: Vec<String> = std::fs::read_dir(root.path().join("content/skills/old"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".bak-"))
            .collect();
        assert_eq!(backups.len(), 1, "旧文件应备份一次：{backups:?}");
        assert!(update_config_get_at(root.path())["syncedAt"]
            .as_str()
            .unwrap_or_default()
            .contains('T'));
    }

    #[test]
    fn update_sync_checksum_mismatch_is_not_applied() {
        let root = TempRoot::new();
        write_config(root.path(), "owner/name", "main");
        let raw = "https://raw.example.test";
        let manifest_url = format!("{raw}/owner/name/main/update-manifest.json");
        let manifest = json!({
            "version": 1,
            "items": [{
                "kind": "skill",
                "name": "a",
                "path": "skills/a/SKILL.md",
                "sha256": "deadbeef"
            }]
        })
        .to_string();
        let fetcher = FakeFetcher::new()
            .ok(&manifest_url, &manifest)
            .ok(&format!("{raw}/owner/name/main/skills/a/SKILL.md"), "body");

        let res = sync_content_at(root.path(), &fetcher, raw, true);
        assert_eq!(res["ok"], false);
        assert_eq!(res["code"], "CHECKSUM_MISMATCH");
        assert!(!root.path().join("content/skills/a/SKILL.md").exists());
    }
}
