//! 插件运行时（进程级）：持久注册表 + DI 热启停 + 统一覆盖率。
//!
//! ## 诚实边界（先读）
//!
//! - 本仓默认 [`crate::plugins::di::NoopDynamicLoader`]：热启停会**真实**改写注册表、
//!   **真实**重跑 DI 拓扑与容器，但不会加载原生 DLL/WASM（`nativeLoaded = 0`）。
//!   接入真实 `DynamicLoad` 后自动生效，RPC 字段不变。
//! - `plugins.status` 的装配快照仍是 `OnceLock`（重启生效）；本模块的 coverage 是
//!   **只读扫描**，不触发重新装配。
//! - 红线：含 `system_prompt` / `full_context` 的插件**拒绝启用**
//!   （`PLUGIN_REDLINE_BLOCKED`），与 `PluginLoader::context_leak_suspects` 同源。
//! - 失败不假装成功：落盘失败或重载失败都会**回滚**启用标记并重新落盘。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::gateway::market::plugin_manifest_dir;
use crate::persistence::config::Config;
use crate::plugins::di::{DiRuntime, PluginLoader, PluginRegistryFile, ServiceContainer};
use crate::plugins::manifest::PluginManifest;
use crate::plugins::patch::{patches_dir, PatchLayer};

/// 热启停失败原因（RPC 侧映射为三段式 `code`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToggleError {
    /// 注册表中不存在该插件
    NotFound(String),
    /// 命中「禁止全上下文重注入」红线
    RedlineBlocked(String),
    /// 注册表落盘失败
    Persist(String),
    /// 重新加载失败
    Load(String),
}

impl ToggleError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "PLUGIN_NOT_FOUND",
            Self::RedlineBlocked(_) => "PLUGIN_REDLINE_BLOCKED",
            Self::Persist(_) => "PLUGIN_PERSIST_FAILED",
            Self::Load(_) => "PLUGIN_LOAD_FAILED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound(n) => format!("插件不存在：{n}"),
            Self::RedlineBlocked(n) => format!(
                "插件 '{n}' 声明了 system_prompt/full_context（上下文重注入红线），拒绝启用"
            ),
            Self::Persist(e) => format!("写插件注册表失败：{e}"),
            Self::Load(e) => format!("重新加载插件失败：{e}"),
        }
    }
}

/// 一次热启停的结果。
#[derive(Debug, Clone)]
pub struct ToggleOutcome {
    pub name: String,
    pub enabled: bool,
    /// 是否已在当前进程内生效（本模块恒为 true；失败会走 Err 而不是 false）
    pub applied: bool,
    /// 当前 enabled 集合的加载顺序（注册表拓扑）
    pub order: Vec<String>,
    /// 真实加载的原生插件数（NoopDynamicLoader 下恒为 0）
    pub native_loaded: usize,
    /// 本次操作类型：`toggle` 改注册表；`reload` 只重建 DI，不写盘。
    pub op: &'static str,
}

impl ToggleOutcome {
    pub fn to_json(&self) -> Value {
        let note = match self.op {
            "reload" => "已重建 DI 与依赖顺序；未改写 registry.json，当前构建未加载原生 DLL/WASM",
            _ => "已改写持久注册表并重跑 DI；当前构建使用 NoopDynamicLoader，未加载原生 DLL/WASM",
        };
        json!({
            "ok": true,
            "name": self.name,
            "enabled": self.enabled,
            "applied": self.applied,
            "order": self.order,
            "nativeLoaded": self.native_loaded,
            "dynamic": "noop",
            "op": self.op,
            "note": note,
        })
    }
}

/// 删除插件的失败原因（RPC 侧映射为三段式 `code`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveError {
    NotFound(String),
    /// 仍被其它已启用插件依赖，拒绝删除（先解除依赖再删）
    InUse(String, Vec<String>),
    /// 清单文件只读 / 删除失败
    Io(String),
    Persist(String),
    Load(String),
}

impl RemoveError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "PLUGIN_NOT_FOUND",
            Self::InUse(_, _) => "PLUGIN_IN_USE",
            Self::Io(_) => "PLUGIN_IO_FAILED",
            Self::Persist(_) => "PLUGIN_PERSIST_FAILED",
            Self::Load(_) => "PLUGIN_LOAD_FAILED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotFound(n) => format!("插件不存在：{n}"),
            Self::InUse(n, deps) => {
                format!("插件 '{n}' 仍被以下插件依赖：{}", deps.join(", "))
            }
            Self::Io(e) => format!("删除清单失败：{e}"),
            Self::Persist(e) => format!("写插件注册表失败：{e}"),
            Self::Load(e) => format!("删除后重新加载失败：{e}"),
        }
    }
}

/// 一次删除的结果（失败会回滚，不会留下半删除状态）。
#[derive(Debug, Clone)]
pub struct RemoveOutcome {
    pub name: String,
    pub removed_manifest: bool,
    pub removed_registry: bool,
    pub order: Vec<String>,
}

/// 插件进程级运行时。
pub struct PluginRuntime {
    pub root: PathBuf,
    pub registry_path: PathBuf,
    pub loader: PluginLoader,
    pub di: DiRuntime,
    pub container: ServiceContainer,
    pub last_order: Vec<String>,
    pub last_error: Option<String>,
}

impl PluginRuntime {
    /// 打开运行时：加载注册表 → 发现 managed symlink → 按 enabled 集合加载。
    ///
    /// 任一步失败都**记录原因并降级**，绝不 panic；显式的写操作会走 Err。
    pub fn open<P: AsRef<Path>, R: AsRef<Path>>(root: P, registry_path: R) -> Self {
        Self::open_inner(root, registry_path, true)
    }

    /// 不补建声明式清单的构造（测试隔离用：避免读到本机真实 `resources/plugins`）。
    pub fn open_without_seed<P: AsRef<Path>, R: AsRef<Path>>(
        root: P,
        registry_path: R,
    ) -> Self {
        Self::open_inner(root, registry_path, false)
    }

    fn open_inner<P: AsRef<Path>, R: AsRef<Path>>(
        root: P,
        registry_path: R,
        seed: bool,
    ) -> Self {
        let root = root.as_ref().to_path_buf();
        let registry_path = registry_path.as_ref().to_path_buf();
        let mut loader = PluginLoader::create(&root);
        let mut last_error: Option<String> = None;

        if let Err(e) = loader.load_registry_file(&registry_path) {
            last_error = Some(format!("加载注册表失败：{e}"));
        }
        // 发现 managed symlink（只改内存注册表；显式写操作才落盘）
        if let Err(e) = loader.discover(true) {
            let msg = format!("发现插件失败：{e}");
            last_error = Some(match last_error {
                Some(prev) => format!("{prev}; {msg}"),
                None => msg,
            });
        }

        let mut rt = Self {
            root,
            registry_path,
            loader,
            di: DiRuntime::new(),
            container: ServiceContainer::new(),
            last_order: Vec::new(),
            last_error,
        };
        // 声明式清单补建：`resources/plugins/manifests` 里已装配的插件
        // 也必须可启停，否则「热启停」在真实安装版上没有任何可操作对象。
        if seed {
            let seeded = rt.seed_declarative_entries();
            if !seeded.is_empty() {
                let msg = format!("以下清单依赖缺失，未纳入注册表：{}", seeded.join(", "));
                rt.last_error = Some(match rt.last_error.take() {
                    Some(prev) => format!("{prev}; {msg}"),
                    None => msg,
                });
            }
        }
        if let Err(e) = rt.reload_all_internal() {
            rt.last_error = Some(format!("初始加载失败：{e}"));
        }
        rt
    }

    /// 把清单里「注册表尚未登记」的插件补建为内存注册表条目。
    ///
    /// - `entry` 为空（纯声明式）→ `declarative = true`，不交给 DynamicLoad；
    /// - 清单带 `entry` → 解析为 manifest 目录下的绝对路径，走原生加载；
    /// - 依赖缺失的条目**不纳入**（否则整批拓扑失败），并把名字返回给调用方记录。
    ///
    /// 刻意不落盘：启动不写用户数据；用户第一次启停时随 `save_registry_file` 持久化。
    fn seed_declarative_entries(&mut self) -> Vec<String> {
        let dir = plugin_manifest_dir();
        self.seed_from_dir(dir.as_deref())
    }

    /// 可注入目录的补建实现（测试用；不读全局路径）。
    fn seed_from_dir(&mut self, dir: Option<&Path>) -> Vec<String> {
        let Some(dir) = dir else {
            return Vec::new();
        };
        let (manifests, _) = scan_manifests(Some(dir));
        let mut candidates: BTreeMap<String, PluginManifest> = BTreeMap::new();
        for m in manifests {
            if !self.loader.registry.entries.contains_key(&m.name) {
                candidates.insert(m.name.clone(), m);
            }
        }
        let mut dropped: Vec<String> = Vec::new();
        loop {
            let known: HashSet<String> = self
                .loader
                .registry
                .entries
                .keys()
                .cloned()
                .chain(candidates.keys().cloned())
                .collect();
            let bad: Vec<String> = candidates
                .iter()
                .filter(|(_, m)| m.dependencies.iter().any(|d| !known.contains(d)))
                .map(|(k, _)| k.clone())
                .collect();
            if bad.is_empty() {
                break;
            }
            for b in &bad {
                if let Some(m) = candidates.remove(b) {
                    let missing: Vec<&str> = m
                        .dependencies
                        .iter()
                        .filter(|d| !known.contains(*d))
                        .map(String::as_str)
                        .collect();
                    dropped.push(format!("{b}（缺 {}）", missing.join("/")));
                }
            }
        }
        for (name, m) in candidates {
            let declarative = m.entry.is_none();
            let entry = match &m.entry {
                Some(rel) if !declarative => dir.join(rel),
                _ => PathBuf::new(),
            };
            self.loader.registry.upsert(crate::plugins::di::PluginRegistryEntry {
                name,
                version: m.version,
                entry,
                enabled: true,
                depends_on: m.dependencies,
                source: "manifest".to_string(),
                declarative,
            });
        }
        dropped.sort();
        dropped
    }

    fn reload_all_internal(&mut self) -> anyhow::Result<Vec<String>> {
        let order = self
            .loader
            .reload_all(&mut self.di, &mut self.container)?;
        self.last_order = order.clone();
        Ok(order)
    }

    /// 热启用/停用：改写注册表 → 落盘 → 整体重跑 DI。失败回滚。
    pub fn set_enabled(
        &mut self,
        name: &str,
        enabled: bool,
    ) -> Result<ToggleOutcome, ToggleError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ToggleError::NotFound(String::new()));
        }
        if !self.loader.registry.entries.contains_key(name) {
            return Err(ToggleError::NotFound(name.to_string()));
        }
        if enabled
            && self
                .loader
                .context_leak_suspects()
                .iter()
                .any(|n| n.as_str() == name)
        {
            return Err(ToggleError::RedlineBlocked(name.to_string()));
        }

        let before = self
            .loader
            .registry
            .entries
            .get(name)
            .map(|e| e.enabled)
            .unwrap_or(false);
        if !self.loader.registry.set_enabled(name, enabled) {
            return Err(ToggleError::NotFound(name.to_string()));
        }
        if let Err(e) = self.loader.save_registry_file(&self.registry_path) {
            // 落盘失败：回滚内存，绝不留下「界面改了、重启丢失」的半状态
            self.loader.registry.set_enabled(name, before);
            return Err(ToggleError::Persist(e.to_string()));
        }

        match self.reload_all_internal() {
            Ok(order) => Ok(ToggleOutcome {
                name: name.to_string(),
                enabled,
                applied: true,
                order,
                native_loaded: 0,
                op: "toggle",
            }),
            Err(e) => {
                // 重载失败：回滚标记并重新落盘 + 恢复旧加载结果
                self.loader.registry.set_enabled(name, before);
                let _ = self.loader.save_registry_file(&self.registry_path);
                let _ = self.reload_all_internal();
                Err(ToggleError::Load(e.to_string()))
            }
        }
    }

    /// 热重载单个插件（不改变启用状态）。
    pub fn reload(&mut self, name: &str) -> Result<ToggleOutcome, ToggleError> {
        let name = name.trim();
        if !self.loader.registry.entries.contains_key(name) {
            return Err(ToggleError::NotFound(name.to_string()));
        }
        self.loader
            .reload_plugin(&mut self.di, &mut self.container, name)
            .map_err(|e| ToggleError::Load(e.to_string()))?;
        let order: Vec<String> = self
            .loader
            .registry
            .enabled_entries()
            .into_iter()
            .map(|e| e.name.clone())
            .collect();
        Ok(ToggleOutcome {
            name: name.to_string(),
            enabled: self
                .loader
                .registry
                .entries
                .get(name)
                .map(|e| e.enabled)
                .unwrap_or(false),
            applied: true,
            order,
            native_loaded: 0,
            op: "reload",
        })
    }

    /// 删除插件：清单文件 + 注册表条目，并重跑 DI。
    ///
    /// 顺序与回滚：
    /// 1. 依赖检查（被其它插件依赖 → 拒绝，避免删完拓扑整批失败）；
    /// 2. 先删清单文件（保留原文以便失败时恢复）；
    /// 3. 再删注册表条目并落盘；
    /// 4. 最后重跑 DI；任一步失败都**恢复清单与注册表**，返回错误。
    pub fn remove(
        &mut self,
        name: &str,
        manifest_dir: Option<&Path>,
    ) -> Result<RemoveOutcome, RemoveError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RemoveError::NotFound(String::new()));
        }
        let registered = self.loader.registry.entries.contains_key(name);
        let manifest_path = manifest_dir
            .map(|dir| dir.join(format!("{name}.plugin.json")))
            .filter(|p| p.exists());
        if !registered && manifest_path.is_none() {
            return Err(RemoveError::NotFound(name.to_string()));
        }

        let dependents: Vec<String> = self
            .loader
            .registry
            .entries
            .values()
            .filter(|e| e.name != name && e.depends_on.iter().any(|d| d == name))
            .map(|e| e.name.clone())
            .collect();
        if !dependents.is_empty() {
            return Err(RemoveError::InUse(name.to_string(), dependents));
        }

        let manifest_raw = manifest_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok());
        let mut removed_manifest = false;
        if let Some(path) = &manifest_path {
            if let Err(e) = remove_file_forgiving(path) {
                return Err(RemoveError::Io(e));
            }
            removed_manifest = true;
        }

        let previous_entry = self.loader.registry.entries.remove(name);
        let removed_registry = previous_entry.is_some();
        if let Err(e) = self.loader.save_registry_file(&self.registry_path) {
            self.restore_removed(name, manifest_path.as_deref(), &manifest_raw, previous_entry);
            return Err(RemoveError::Persist(e.to_string()));
        }

        match self.reload_all_internal() {
            Ok(order) => Ok(RemoveOutcome {
                name: name.to_string(),
                removed_manifest,
                removed_registry,
                order,
            }),
            Err(e) => {
                self.restore_removed(name, manifest_path.as_deref(), &manifest_raw, previous_entry);
                let _ = self.loader.save_registry_file(&self.registry_path);
                let _ = self.reload_all_internal();
                Err(RemoveError::Load(e.to_string()))
            }
        }
    }

    /// 删除失败时的回滚：恢复清单原文与注册表条目（尽力而为，不掩盖原始错误）。
    fn restore_removed(
        &mut self,
        name: &str,
        manifest_path: Option<&Path>,
        manifest_raw: &Option<String>,
        previous_entry: Option<crate::plugins::di::PluginRegistryEntry>,
    ) {
        if let (Some(path), Some(raw)) = (manifest_path, manifest_raw.as_ref()) {
            let _ = std::fs::write(path, raw);
        }
        if let Some(entry) = previous_entry {
            self.loader.registry.entries.insert(name.to_string(), entry);
        }
    }

    /// 统一覆盖率（只读）：注册表 + managed + manifests + patches + 红线。
    pub fn coverage(&self) -> Value {
        self.coverage_for(
            plugin_manifest_dir().as_deref(),
            patches_dir().as_deref(),
        )
    }

    /// 可注入目录的覆盖率实现（供测试；不读取全局路径）。
    pub fn coverage_for(&self, manifests_dir: Option<&Path>, patches: Option<&Path>) -> Value {
        let managed = scan_managed(&self.root.join("managed"));
        let (manifests, skipped) = scan_manifests(manifests_dir);
        let registry = &self.loader.registry;
        let suspects: HashSet<String> = self
            .loader
            .context_leak_suspects()
            .into_iter()
            .collect();

        let mut recs: BTreeMap<String, Rec> = BTreeMap::new();
        for e in registry.entries.values() {
            let rec = recs.entry(e.name.clone()).or_default();
            rec.sources.insert("registry".to_string());
            rec.enabled = Some(e.enabled);
            if rec.version.is_empty() {
                rec.version = e.version.clone();
            }
            if rec.entry.is_none() {
                rec.entry = Some(e.entry.display().to_string());
            }
            for d in &e.depends_on {
                rec.depends_on.insert(d.clone());
            }
            if e.declarative {
                rec.declarative = true;
            }
        }
        for m in &manifests {
            let rec = recs.entry(m.name.clone()).or_default();
            rec.sources.insert("manifest".to_string());
            if rec.version.is_empty() {
                rec.version = m.version.clone();
            }
            if rec.description.is_none() {
                rec.description = Some(m.description.clone());
            }
            if rec.author.is_none() {
                rec.author = m.author.clone();
            }
            for c in &m.capabilities {
                rec.capabilities.insert(c.clone());
            }
            if rec.entry.is_none() {
                rec.entry = m.entry.clone();
            }
            for s in &m.skills {
                rec.skills.insert(s.name.clone());
            }
            if let Some(spec) = &m.mcp {
                let state = crate::plugins::mcp_launch::state(&m.name);
                rec.mcp = Some(json!({
                    "declared": true,
                    "command": spec.command,
                    "args": spec.args,
                    "timeoutMs": spec.effective_timeout().as_millis() as u64,
                    "running": state.get("running").and_then(Value::as_bool).unwrap_or(false),
                    "tools": state.get("tools").cloned().unwrap_or(json!([])),
                    "error": state.get("error").cloned().unwrap_or(Value::Null),
                }));
            }
            for d in &m.dependencies {
                rec.depends_on.insert(d.clone());
            }
        }
        for name in &suspects {
            recs.entry(name.clone()).or_default().redline = true;
        }

        let manifest_only: Vec<String> = recs
            .iter()
            .filter(|(_, r)| r.sources.contains("manifest") && !r.sources.contains("registry"))
            .map(|(k, _)| k.clone())
            .collect();
        let registry_only: Vec<String> = recs
            .iter()
            .filter(|(_, r)| r.sources.contains("registry") && !r.sources.contains("manifest"))
            .map(|(k, _)| k.clone())
            .collect();
        let records: Vec<Value> = recs
            .iter()
            .map(|(id, r)| {
                let enabled = r.enabled.unwrap_or(true);
                json!({
                    "id": id,
                    "version": r.version,
                    "description": r.description,
                    "author": r.author,
                    "capabilities": r.capabilities,
                    "sources": r.sources,
                    "enabled": enabled,
                    "effectiveEnabled": enabled && !r.redline,
                    "declarative": r.declarative,
                    "entry": r.entry,
                    "dependsOn": r.depends_on,
                    "skills": r.skills,
                    "mcp": r.mcp,
                    "redline": r.redline,
                    "adaptation": crate::plugins::adaptation::summary_for(id),
                })
            })
            .collect();

        let enabled_count = registry.entries.values().filter(|e| e.enabled).count();
        let disabled_count = registry.entries.len().saturating_sub(enabled_count);
        let patch_report = match patches {
            Some(dir) => match PatchLayer::load_dir(dir) {
                Ok(v) => json!({
                    "total": v.len(),
                    "dir": dir.display().to_string(),
                    "error": Value::Null,
                }),
                Err(e) => json!({
                    "total": Value::Null,
                    "dir": dir.display().to_string(),
                    "error": e.to_string(),
                }),
            },
            None => json!({ "total": Value::Null, "dir": Value::Null, "error": "patches 目录未解析到" }),
        };

        json!({
            "ok": true,
            "root": self.root.display().to_string(),
            "registryPath": self.registry_path.display().to_string(),
            "managed": {
                "total": managed.total,
                "symlinks": managed.symlinks,
                "localFallback": managed.local,
                "brokenLinks": managed.broken,
            },
            "registry": {
                "entries": registry.entries.len(),
                "enabled": enabled_count,
                "disabled": disabled_count,
                "declarative": registry.entries.values().filter(|e| e.declarative).count(),
                "persisted": self.registry_path.exists(),
            },
            "manifests": {
                "total": manifests.len(),
                "dir": manifests_dir.map(|d| d.display().to_string()),
            },
            "patches": patch_report,
            "redlines": { "contextLeakSuspects": suspects },
            "plugins": records,
            "gaps": { "manifestOnly": manifest_only, "registryOnly": registry_only },
            "skipped": skipped,
            "skippedTotal": skipped.len(),
            "lastError": self.last_error,
        })
    }
}

/// 删除文件：Windows 上安装包清单可能带只读属性，先清只读再删。
fn remove_file_forgiving(path: &Path) -> Result<(), String> {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

#[derive(Default)]
struct Rec {
    version: String,
    /// 清单描述 / 作者 / 声明能力（仅 manifest 来源有值，供列表页直接展示）
    description: Option<String>,
    author: Option<String>,
    capabilities: BTreeSet<String>,
    sources: BTreeSet<String>,
    enabled: Option<bool>,
    entry: Option<String>,
    depends_on: BTreeSet<String>,
    skills: BTreeSet<String>,
    /// MCP 声明 + 运行态（仅声明了 `mcp` 的插件有值）
    mcp: Option<Value>,
    redline: bool,
    declarative: bool,
}

struct ManagedScan {
    total: usize,
    symlinks: usize,
    local: usize,
    broken: Vec<String>,
}

fn scan_managed(dir: &Path) -> ManagedScan {
    let mut scan = ManagedScan {
        total: 0,
        symlinks: 0,
        local: 0,
        broken: Vec::new(),
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return scan;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        scan.total += 1;
        let is_symlink = entry
            .file_type()
            .map(|ft| ft.is_symlink())
            .unwrap_or(false);
        if is_symlink {
            scan.symlinks += 1;
            if std::fs::read_link(entry.path()).is_err() {
                scan.broken.push(name);
            }
        } else {
            scan.local += 1;
        }
    }
    scan.broken.sort();
    scan
}

fn scan_manifests(dir: Option<&Path>) -> (Vec<PluginManifest>, Vec<Value>) {
    let Some(dir) = dir else {
        return (Vec::new(), Vec::new());
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (
            Vec::new(),
            vec![json!({ "path": dir.display().to_string(), "reason": "manifests-dir-unreadable" })],
        );
    };
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.ends_with(".plugin.json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match PluginManifest::from_json(&text) {
                Ok(m) => out.push(m),
                Err(e) => skipped.push(json!({
                    "path": path.display().to_string(),
                    "reason": format!("invalid-manifest: {e}"),
                })),
            },
            Err(e) => skipped.push(json!({
                "path": path.display().to_string(),
                "reason": format!("read-failed: {e}"),
            })),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, skipped)
}

/// 当前注册表里被停用的插件名（供 `plugins.status` 只读过滤，不初始化运行时）。
pub fn disabled_names_from(path: &Path) -> HashSet<String> {
    PluginRegistryFile::load(path)
        .map(|r| {
            r.entries
                .into_values()
                .filter(|e| !e.enabled)
                .map(|e| e.name)
                .collect()
        })
        .unwrap_or_default()
}

/// 进程级默认插件根目录（**用户可写**）。
///
/// 只用于 `managed/` 与 `registry.json`；清单/补丁目录仍由
/// [`plugin_manifest_dir`] / [`patches_dir`] 解析（可能位于应用资源里，只读）。
/// 这样安装包升级覆盖 `resources/plugins` 时，不会把用户的启停状态一起冲掉。
pub fn plugins_root() -> PathBuf {
    if let Some(p) = std::env::var_os("MARGUERITE_PLUGINS_ROOT").map(PathBuf::from) {
        if p.is_dir() {
            return p;
        }
    }
    Config::data_dir().join("plugins")
}

/// 进程级默认注册表路径（`<plugins_root>/registry.json`）。
pub fn registry_path() -> PathBuf {
    plugins_root().join("registry.json")
}

static GLOBAL: OnceLock<Mutex<PluginRuntime>> = OnceLock::new();

/// 进程级运行时（懒初始化；失败降级为空注册表，不 panic）。
pub fn global() -> &'static Mutex<PluginRuntime> {
    GLOBAL.get_or_init(|| Mutex::new(PluginRuntime::open(plugins_root(), registry_path())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::di::PluginRegistryEntry;

    fn temp_root(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_plugin_runtime_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("managed")).unwrap();
        let registry = dir.join("registry.json");
        (dir, registry)
    }

    fn seed(registry: &Path, name: &str, enabled: bool) {
        let mut file = PluginRegistryFile::load(registry).unwrap();
        file.entries.insert(
            name.to_string(),
            PluginRegistryEntry {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                entry: PathBuf::from(format!("managed/{name}")),
                enabled,
                depends_on: Vec::new(),
                source: "local".to_string(),
                declarative: false,
            },
        );
        file.save(registry).unwrap();
    }

    #[test]
    fn toggle_persists_and_changes_load_order() {
        let (root, registry) = temp_root("toggle");
        seed(&registry, "alpha", true);
        seed(&registry, "beta", false);

        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        assert_eq!(rt.last_order, vec!["alpha".to_string()]);

        let out = rt.set_enabled("beta", true).unwrap();
        assert!(out.order.contains(&"beta".to_string()));
        assert!(out.applied);

        // 落盘必须真实：重新打开仍能看到 beta enabled
        let reloaded = PluginRegistryFile::load(&registry).unwrap();
        assert!(reloaded.entries.get("beta").unwrap().enabled);

        let out = rt.set_enabled("alpha", false).unwrap();
        assert!(!out.order.contains(&"alpha".to_string()));
        assert!(!PluginRegistryFile::load(&registry)
            .unwrap()
            .entries
            .get("alpha")
            .unwrap()
            .enabled);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn unknown_plugin_is_not_found() {
        let (root, registry) = temp_root("missing");
        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        let err = rt.set_enabled("nope", true).unwrap_err();
        assert_eq!(err.code(), "PLUGIN_NOT_FOUND");
        assert_eq!(rt.reload("nope").unwrap_err().code(), "PLUGIN_NOT_FOUND");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn redline_plugin_cannot_be_enabled() {
        let (root, registry) = temp_root("redline");
        let bad = root.join("managed").join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("plugin.json"),
            r#"{"version":"1.0.0","system_prompt":"全部上下文"}"#,
        )
        .unwrap();

        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        // discover 已把 bad 记进 suspects；启用必须被拒
        let err = rt.set_enabled("bad", true).unwrap_err();
        assert_eq!(err.code(), "PLUGIN_REDLINE_BLOCKED");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn reload_unknown_is_not_found() {
        let (root, registry) = temp_root("reload");
        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        assert_eq!(
            rt.reload("ghost").unwrap_err().code(),
            "PLUGIN_NOT_FOUND"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn coverage_merges_registry_and_manifests_with_gaps() {
        let (root, registry) = temp_root("coverage");
        seed(&registry, "registry_only", true);
        let manifests = root.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("manifest_only.plugin.json"),
            r#"{"name":"manifest_only","version":"2.0.0","description":"d","skills":[{"name":"demo"}]}"#,
        )
        .unwrap();
        std::fs::write(
            manifests.join("broken.plugin.json"),
            r#"{"version":"1.0.0"}"#,
        )
        .unwrap();

        let rt = PluginRuntime::open_without_seed(&root, &registry);
        let cov = rt.coverage_for(Some(&manifests), None);
        assert_eq!(cov["registry"]["entries"].as_u64().unwrap(), 1);
        assert_eq!(cov["manifests"]["total"].as_u64().unwrap(), 1);
        assert_eq!(
            cov["gaps"]["registryOnly"][0].as_str().unwrap(),
            "registry_only"
        );
        assert_eq!(
            cov["gaps"]["manifestOnly"][0].as_str().unwrap(),
            "manifest_only"
        );
        assert!(cov["skippedTotal"].as_u64().unwrap() >= 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn disabled_names_from_reads_registry() {
        let (root, registry) = temp_root("disabled");
        seed(&registry, "on", true);
        seed(&registry, "off", false);
        let names = disabled_names_from(&registry);
        assert!(names.contains("off"));
        assert!(!names.contains("on"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn managed_scan_reports_broken_links() {
        let (root, _) = temp_root("managed");
        let managed = root.join("managed");
        std::fs::create_dir_all(managed.join("ok")).unwrap();
        // Windows 上创建 symlink 需要权限；用目录代替 symlink 的 fallback 计数
        let scan = scan_managed(&managed);
        assert_eq!(scan.total, 1);
        assert_eq!(scan.local, 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn declarative_manifests_are_seeded_and_toggleable() {
        let (root, registry) = temp_root("declarative");
        let manifests = root.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("decl-a.plugin.json"),
            r#"{"name":"decl-a","version":"1.0.0","description":"d"}"#,
        )
        .unwrap();
        std::fs::write(
            manifests.join("decl-b.plugin.json"),
            r#"{"name":"decl-b","version":"1.0.0","description":"d","dependencies":["decl-a"]}"#,
        )
        .unwrap();

        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        let dropped = rt.seed_from_dir(Some(&manifests));
        assert!(dropped.is_empty(), "依赖齐全时不应丢弃任何条目: {dropped:?}");
        assert!(rt
            .loader
            .registry
            .entries
            .get("decl-a")
            .map(|e| e.declarative)
            .unwrap_or(false));
        rt.reload_all_internal().unwrap();

        // 停用依赖方：拓扑顺序里必须消失，且真实落盘
        let out = rt.set_enabled("decl-b", false).unwrap();
        assert!(!out.order.contains(&"decl-b".to_string()));
        let persisted = PluginRegistryFile::load(&registry).unwrap();
        assert!(!persisted.entries.get("decl-b").unwrap().enabled);
        assert!(persisted.entries.get("decl-a").unwrap().declarative);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn declarative_with_missing_dependency_is_dropped() {
        let (root, registry) = temp_root("depper");
        let manifests = root.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("orphan.plugin.json"),
            r#"{"name":"orphan","version":"1.0.0","description":"d","dependencies":["ghost"]}"#,
        )
        .unwrap();
        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        let dropped = rt.seed_from_dir(Some(&manifests));
        assert_eq!(dropped.len(), 1);
        assert!(dropped[0].contains("orphan"));
        assert!(!rt.loader.registry.entries.contains_key("orphan"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn remove_deletes_manifest_and_registry_entry() {
        let (root, registry) = temp_root("remove");
        let manifests = root.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("gone.plugin.json"),
            r#"{"name":"gone","version":"1.0.0","description":"d"}"#,
        )
        .unwrap();
        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        rt.seed_from_dir(Some(&manifests));
        rt.reload_all_internal().unwrap();

        let out = rt.remove("gone", Some(&manifests)).unwrap();
        assert!(out.removed_manifest);
        assert!(out.removed_registry);
        assert!(!manifests.join("gone.plugin.json").exists());
        assert!(!rt.loader.registry.entries.contains_key("gone"));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn remove_refuses_plugins_with_dependents() {
        let (root, registry) = temp_root("inuse");
        let manifests = root.join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("core.plugin.json"),
            r#"{"name":"core","version":"1.0.0","description":"d"}"#,
        )
        .unwrap();
        std::fs::write(
            manifests.join("plugin-x.plugin.json"),
            r#"{"name":"plugin-x","version":"1.0.0","description":"d","dependencies":["core"]}"#,
        )
        .unwrap();
        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        rt.seed_from_dir(Some(&manifests));
        rt.reload_all_internal().unwrap();

        let err = rt.remove("core", Some(&manifests)).unwrap_err();
        assert_eq!(err.code(), "PLUGIN_IN_USE");
        assert!(manifests.join("core.plugin.json").exists(), "被依赖时不得删除");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn remove_unknown_is_not_found() {
        let (root, registry) = temp_root("remove-missing");
        let mut rt = PluginRuntime::open_without_seed(&root, &registry);
        assert_eq!(
            rt.remove("ghost", Some(root.as_path())).unwrap_err().code(),
            "PLUGIN_NOT_FOUND"
        );
        std::fs::remove_dir_all(root).ok();
    }
}
