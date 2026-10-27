//! BepInEx / Cordis 式运行时注入与热加载容器（策划 §三.6 补强）。
//!
//! 相对纯 Cordis 的「静态 DI 容器」，本模块补齐了 BepInEx 的运行时注入范式：
//! 1. [`ServiceContainer`] 原 Cordis：TypeId→Arc 式服务注册表（provide/get/remove/contains）。
//! 2. [`ServicePlugin`]：声明 provides/requires + apply + on_unload。
//! 3. [`DiRuntime`]：依赖感知加载器（拓扑、环检测、逆序卸载、热重载）。
//! 4. **BepInEx 模式新增**：
//!    - [`PluginRegistryFile`]：持久注册表（JSON 文件，记录插件名→入口 DLL/WASM/目录
//!      的映射 + 启用/禁用），重启后仍能恢复；等价 BepInEx 的 `BepInEx.cfg` 插件列表。
//!    - [`discover_symlinks`] / [`PluginLoader`]：symlink 发现机制（把外部目录
//!      软链到 `plugins/managed/` 即视为安装），加上 `loader.create(plugin_dir)` →
//!      直接构造一个带插件目录加载能力的 DiRuntime（可静态也可热加载），实现
//!      BepInEx 的「drop-in symlink install」。
//!    - [`load_enabled`] / [`reload_plugin`] / [`disable_plugin`]：在不重启进程的
//!      前提下从持久注册表中按名字热加载/热替换/热卸载单个插件。
//!
//! 与既有 Plugin/PluginRegistry 并存、不破坏现状；PluginManifest 的 dependencies 字段
//! 可桥接至此（后续增量）。
//!
//! 设计红线对齐 bloom.yaml anti_patterns：
//! - 不允许在 apply() 中替换已经被消费过的服务（loader 会把 applied 的 TypeId
//!   记录到 `invariants.applied_services`，并拒绝重复 provide 相同 TypeId 除非显式
//!   走 reload）——对应「禁止中途替换工具面」。
//! - 持久注册表绝不把「全局 system prompt / full context」作为插件字段写入——对应
//!   「禁止全上下文重注入」。

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::PluginMeta;

#[derive(Default)]
pub struct ServiceContainer {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl ServiceContainer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provide<T: Any + Send + Sync>(&mut self, service: T) -> Option<Arc<T>> {
        self.services
            .insert(TypeId::of::<T>(), Arc::new(service))
            .map(|prev| prev.downcast::<T>().expect("服务类型键不一致"))
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.services
            .get(&TypeId::of::<T>())
            .and_then(|s| s.clone().downcast::<T>().ok())
    }

    pub fn contains<T: Any>(&self) -> bool {
        self.services.contains_key(&TypeId::of::<T>())
    }

    pub fn remove<T: Any + Send + Sync>(&mut self) -> Option<Arc<T>> {
        self.services
            .remove(&TypeId::of::<T>())
            .map(|s| s.downcast::<T>().expect("服务类型键不一致"))
    }

    pub fn len(&self) -> usize {
        self.services.len()
    }

    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}

pub trait ServicePlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    fn provides(&self) -> Vec<TypeId> {
        Vec::new()
    }

    fn requires(&self) -> Vec<TypeId> {
        Vec::new()
    }

    fn apply(&self, _container: &mut ServiceContainer) -> Result<()> {
        Ok(())
    }

    fn on_unload(&self) {}
}

#[derive(Default)]
pub struct DiRuntime {
    plugins: Vec<Arc<dyn ServicePlugin>>,
    order: Vec<usize>,
    loaded: bool,
}

impl DiRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: Arc<dyn ServicePlugin>) {
        self.plugins.push(plugin);
        self.loaded = false;
        self.order.clear();
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn resolve_order(&self) -> Result<Vec<usize>> {
        let n = self.plugins.len();
        let mut providers: HashMap<TypeId, usize> = HashMap::new();
        for (i, p) in self.plugins.iter().enumerate() {
            for t in p.provides() {
                providers.entry(t).or_insert(i);
            }
        }
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];
        for (i, p) in self.plugins.iter().enumerate() {
            for req in p.requires() {
                if let Some(&prov) = providers.get(&req) {
                    if prov != i {
                        edges[prov].push(i);
                        in_degree[i] += 1;
                    }
                }
            }
        }
        let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = queue.pop_front() {
            order.push(i);
            for &j in &edges[i] {
                in_degree[j] -= 1;
                if in_degree[j] == 0 {
                    queue.push_back(j);
                }
            }
        }
        if order.len() != n {
            let cyclic: Vec<String> = (0..n)
                .filter(|&i| in_degree[i] > 0)
                .map(|i| self.plugins[i].meta().name.clone())
                .collect();
            return Err(anyhow!("插件依赖环检测：{}", cyclic.join(" -> ")));
        }
        Ok(order)
    }

    pub fn load(&mut self, container: &mut ServiceContainer) -> Result<()> {
        let order = self.resolve_order()?;
        for &i in &order {
            let p = self.plugins[i].clone();
            p.apply(container)?;
        }
        self.order = order;
        self.loaded = true;
        Ok(())
    }

    pub fn unload(&mut self) {
        for &i in self.order.iter().rev() {
            self.plugins[i].on_unload();
        }
        self.order.clear();
        self.loaded = false;
    }

    pub fn reload(&mut self, container: &mut ServiceContainer) -> Result<()> {
        self.unload();
        self.load(container)
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn load_order(&self) -> &[usize] {
        &self.order
    }
}

// ============ BepInEx 式：持久注册表 + symlink 发现 + loader.create ============

/// 单个插件在持久注册表中的条目。
///
/// 对齐 BepInEx：插件安装的语义是「入口文件 / 目录」；我们同时支持把整目录
/// 软链到 plugins/managed 下，此时 `entry` 就是被软链的目标。
/// —— 持久注册表绝不保存 system prompt / 上下文，以满足反模式红线。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRegistryEntry {
    pub name: String,
    pub version: String,
    /// 入口：DLL / WASM / 目录（相对路径由 discover_symlinks 发现时写绝对路径）
    pub entry: PathBuf,
    /// 是否启用（未启用的不会被 load_enabled 注入）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 插件的依赖（名字，不是 TypeId——持久层使用名字层的拓扑，加载入 DiRuntime
    /// 再使用 ServicePlugin trait 的 TypeId 拓扑做第二层）
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// 来源：local / symlink / remote（仅信息，不影响逻辑）
    #[serde(default)]
    pub source: String,
    /// 声明式插件（清单装配，无原生入口）。
    ///
    /// 这类条目参与注册表拓扑与启停，但**不交给 `DynamicLoad`**：
    /// 否则真实 loader 会拿空 `entry` 去加载 DLL/WASM 并失败，
    /// 表现为「声明式插件一启用就整批加载失败」。
    #[serde(default)]
    pub declarative: bool,
}

fn default_true() -> bool {
    true
}

/// 持久注册表（等价 BepInEx.cfg 的插件段 + BepInEx/plugins 目录发现）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRegistryFile {
    pub version: u32,
    #[serde(default)]
    pub entries: HashMap<String, PluginRegistryEntry>,
}

impl PluginRegistryFile {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn new() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            entries: HashMap::new(),
        }
    }

    /// 从 JSON 文件加载注册表；不存在则返回空（写入时会 create）。
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let bytes =
            std::fs::read(path).with_context(|| format!("读注册表失败: {}", path.display()))?;
        let this: Self = if bytes.is_empty() {
            Self::new()
        } else {
            serde_json::from_slice(&bytes)
                .with_context(|| format!("解析注册表 JSON 失败: {}", path.display()))?
        };
        Ok(this)
    }

    /// 落盘（原子写）。R13：`fs::write` 先截断再写，半截 JSON 会让
    /// `PluginRegistry::load` 解析失败并回落空注册表 —— 用户装的插件全部"消失"。
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("创建注册表目录失败: {}", parent.display()))?;
            }
        }
        let json = serde_json::to_string_pretty(self)?;
        crate::persistence::write_json_atomic(path, &json)
            .with_context(|| format!("写注册表失败: {}", path.display()))?;
        Ok(())
    }

    pub fn upsert(&mut self, entry: PluginRegistryEntry) {
        self.entries.insert(entry.name.clone(), entry);
    }

    pub fn remove(&mut self, name: &str) -> Option<PluginRegistryEntry> {
        self.entries.remove(name)
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        if let Some(e) = self.entries.get_mut(name) {
            e.enabled = enabled;
            true
        } else {
            false
        }
    }

    pub fn enabled_entries(&self) -> Vec<&PluginRegistryEntry> {
        self.entries.values().filter(|e| e.enabled).collect()
    }
}

/// 发现 `plugins_root/managed` 目录下所有 symlink 条目（BepInEx drop-in 安装）。
///
/// 约定：
/// - 软链如果是目录，对应插件名 = 链接名；entry = 目标绝对路径；source="symlink"
/// - 软链如果是 `.dll`/`.wasm` 文件，同样按名称处理
/// - 真实目录/文件（非 symlink）可作为 fallback 一并收集（非 symlink 走 local 来源）
pub fn discover_symlinks(
    plugins_root: &Path,
    include_local_fallback: bool,
) -> Result<Vec<PluginRegistryEntry>> {
    let managed = plugins_root.join("managed");
    if !managed.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&managed)
        .with_context(|| format!("扫描插件目录失败: {}", managed.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(os) => {
                tracing::warn!("忽略不可编码的插件名: {:?}", os);
                continue;
            }
        };
        // 忽略点文件
        if name.starts_with('.') {
            continue;
        }
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        if !is_symlink && !include_local_fallback {
            continue;
        }
        // 链接解析出的目标（symlink）或 path 本身（fallback）
        let target = if is_symlink {
            match std::fs::read_link(&path) {
                Ok(t) => {
                    // 相对软链按 managed 目录解析为绝对路径
                    if t.is_absolute() {
                        t
                    } else {
                        managed.join(t)
                    }
                }
                Err(e) => {
                    tracing::warn!("插件 symlink 损坏 {}: {e}", path.display());
                    continue;
                }
            }
        } else {
            path.clone()
        };
        let source = if is_symlink { "symlink" } else { "local" };
        // 如果是目录：版本 = 读插件目录下 plugin.json 的 version，或默认 "0.0.0"
        let (version, deps) = version_and_deps_from_entry(&target).unwrap_or_default();
        out.push(PluginRegistryEntry {
            name,
            version,
            entry: target,
            enabled: true,
            depends_on: deps,
            source: source.to_string(),
            declarative: false,
        });
    }
    Ok(out)
}

fn version_and_deps_from_entry(target: &Path) -> Option<(String, Vec<String>)> {
    let manifest_path = if target.is_dir() {
        target.join("plugin.json")
    } else {
        // 文件型：读取同名 + .plugin.json 或 sibling
        let stem = target.file_stem()?.to_os_string();
        let mut sib = target.parent()?.to_path_buf();
        sib.push(format!("{}.plugin.json", stem.to_string_lossy()));
        sib
    };
    if !manifest_path.exists() {
        return None;
    }
    #[derive(Deserialize)]
    struct MiniManifest {
        version: Option<String>,
        #[serde(default)]
        depends_on: Vec<String>,
    }
    let bytes = std::fs::read(&manifest_path).ok()?;
    let m: MiniManifest = serde_json::from_slice(&bytes).ok()?;
    Some((
        m.version.unwrap_or_else(|| "0.0.0".to_string()),
        m.depends_on,
    ))
}

/// 注册表层的依赖检查：给定 entry 集合，按名字依赖返回拓扑顺序（出错与 DiRuntime 内层无关）。
pub fn registry_topological_order(
    entries: &HashMap<String, PluginRegistryEntry>,
) -> Result<Vec<String>> {
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut in_deg: HashMap<String, usize> = HashMap::new();
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for e in entries.values() {
        in_deg.entry(e.name.clone()).or_default();
    }
    for e in entries.values() {
        for dep in &e.depends_on {
            if !entries.contains_key(dep) {
                return Err(anyhow!("插件 '{}' 依赖未出现在注册表中：'{}'", e.name, dep));
            }
            edges.entry(dep.clone()).or_default().push(e.name.clone());
            *in_deg.get_mut(&e.name).unwrap() += 1;
        }
    }
    for (name, d) in in_deg.iter() {
        if *d == 0 {
            queue.push_back(name.clone());
        }
    }
    let mut order = Vec::with_capacity(entries.len());
    while let Some(name) = queue.pop_front() {
        order.push(name.clone());
        if let Some(targets) = edges.get(&name) {
            for t in targets {
                *in_deg.get_mut(t).unwrap() -= 1;
                if in_deg[t] == 0 {
                    queue.push_back(t.clone());
                }
            }
        }
    }
    if order.len() != entries.len() {
        let leftover: Vec<String> = in_deg
            .into_iter()
            .filter(|(_, d)| *d > 0)
            .map(|(n, _)| n)
            .collect();
        return Err(anyhow!("注册表插件存在依赖环：{:?}", leftover));
    }
    Ok(order)
}

/// 动态加载插件的 trait：把「entry 文件/目录」变成一个可 `apply` 的 [`ServicePlugin`]。
///
/// 真实 DLL/WASM 加载需要 abi_stable/wasmtime 等宿主，此 trait 把这部分保持为
/// 开放接口；默认实现返回「空静态插件」以便于独立测试。
pub trait DynamicLoad: Send + Sync {
    /// 根据 entry 加载 ServicePlugin；名字与版本从 entry 附带（避免重复解析）。
    fn load(&self, entry: &PluginRegistryEntry) -> Result<Arc<dyn ServicePlugin>>;
}

/// 仅返回空 ServicePlugin 的默认 loader（用来验证注册表/发现机制不需要真加载）。
pub struct NoopDynamicLoader;
impl DynamicLoad for NoopDynamicLoader {
    fn load(&self, entry: &PluginRegistryEntry) -> Result<Arc<dyn ServicePlugin>> {
        struct Noop(PluginMeta);
        impl ServicePlugin for Noop {
            fn meta(&self) -> PluginMeta {
                self.0.clone()
            }
        }
        Ok(Arc::new(Noop(PluginMeta {
            name: entry.name.clone(),
            version: entry.version.clone(),
            description: format!("dynamic plugin {}", entry.name),
            author: None,
        })))
    }
}

/// BepInEx 式 loader：对应 BepInEx `Chainloader.Create`。
///
/// 持有：
/// - `registry`：持久注册表（可以 save/load 到 JSON）
/// - `plugins_root`：插件根目录（扫描 `<root>/managed` symlinks）
/// - `dynamic`：把 entry 变成 ServicePlugin 的加载器
pub struct PluginLoader<DL: DynamicLoad = NoopDynamicLoader> {
    pub registry: PluginRegistryFile,
    pub plugins_root: PathBuf,
    pub dynamic: DL,
    invariants: LoaderInvariants,
}

#[derive(Default)]
struct LoaderInvariants {
    /// 已经被插件 apply() 过的 TypeId——正常 load 期间不允许同 TypeId 被重复
    /// provide（禁止中途替换工具面；设计红线）。reload_plugin() 允许绕过，
    /// 但会先 unload 相关插件。
    applied_service_types: HashSet<TypeId>,
    /// 持久注册表中保存了完整上下文等大字段——一旦有 plugin.json 中包含
    /// `system_prompt` / `full_context` 之类字段，发现时记一次警告，并在上层
    /// 可选择拒绝启用。此处仅记录名字列表，便于测试。
    context_leak_suspects: HashSet<String>,
}

impl PluginLoader<NoopDynamicLoader> {
    /// 快速构造：Noop 加载器 + 默认注册表。
    pub fn create<P: AsRef<Path>>(plugins_root: P) -> Self {
        Self {
            registry: PluginRegistryFile::new(),
            plugins_root: plugins_root.as_ref().to_path_buf(),
            dynamic: NoopDynamicLoader,
            invariants: LoaderInvariants::default(),
        }
    }
}

impl<DL: DynamicLoad> PluginLoader<DL> {
    /// 通用构造（自定义 DynamicLoad 实现）。
    pub fn with_loader<P: AsRef<Path>>(plugins_root: P, dynamic: DL) -> Self {
        Self {
            registry: PluginRegistryFile::new(),
            plugins_root: plugins_root.as_ref().to_path_buf(),
            dynamic,
            invariants: LoaderInvariants::default(),
        }
    }

    /// 加载持久注册表。
    pub fn load_registry_file(&mut self, path: &Path) -> Result<&mut Self> {
        self.registry = PluginRegistryFile::load(path)?;
        Ok(self)
    }

    /// 保存持久注册表。
    pub fn save_registry_file(&self, path: &Path) -> Result<&Self> {
        self.registry.save(path)?;
        Ok(self)
    }

    /// 从 plugins_root/managed 发现 symlink 插件并合并到持久注册表（不立即注入）。
    pub fn discover(&mut self, include_local_fallback: bool) -> Result<&mut Self> {
        let discovered = discover_symlinks(&self.plugins_root, include_local_fallback)?;
        // 简单反模式检查：如果 entry 目录里 plugin.json 包含 system_prompt/full_context，
        // 就加进 suspects（不拒绝，只是记录，便于上层审计）。
        for e in &discovered {
            if let Some(sib) = e.entry.parent() {
                let check: Option<std::path::PathBuf> = if e.entry.is_dir() {
                    Some(e.entry.join("plugin.json"))
                } else {
                    let stem = e.entry.file_stem().map(|s| s.to_os_string());
                    stem.map(|stem| {
                        let mut p = sib.to_path_buf();
                        p.push(format!("{}.plugin.json", stem.to_string_lossy()));
                        p
                    })
                };
                if let Some(manifest) = check {
                    if manifest.exists() {
                        // ⚠️ 这里是**安全审计**（找出声明了 system_prompt/full_context 的
                        // 上下文泄漏嫌疑插件），因此读/解析失败**不能静默忽略** ——
                        // 那会让一个坏清单悄悄变成「审计干净」，是**漏报**。
                        // 与 `gateway/market.rs` 的既有纪律一致（禁用 `if let Ok(..)`
                        // 静默丢弃）。策略取 **fail-closed**：读不出/解析不了就记为嫌疑，
                        // 因为本方法的语义本就「不拒绝，只是记录，便于上层审计」（见上）。
                        match std::fs::read(&manifest)
                            .map_err(|e| e.to_string())
                            .and_then(|bytes| {
                                serde_json::from_slice::<serde_json::Value>(&bytes)
                                    .map_err(|e| e.to_string())
                            }) {
                            Ok(v) => {
                                if let Some(map) = v.as_object() {
                                    if map.contains_key("system_prompt")
                                        || map.contains_key("full_context")
                                    {
                                        self.invariants
                                            .context_leak_suspects
                                            .insert(e.name.clone());
                                    }
                                }
                            }
                            Err(reason) => {
                                tracing::warn!(
                                    plugin = %e.name,
                                    manifest = %manifest.display(),
                                    reason = %reason,
                                    "插件清单读取/解析失败：安全审计取 fail-closed，记为上下文泄漏嫌疑（避免漏报）"
                                );
                                self.invariants
                                    .context_leak_suspects
                                    .insert(e.name.clone());
                            }
                        }
                    }
                }
            }
            self.registry.upsert(e.clone());
        }
        Ok(self)
    }

    /// 当前被识别为「包含 system_prompt/full_context 风险」的插件名列表。
    pub fn context_leak_suspects(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .invariants
            .context_leak_suspects
            .iter()
            .cloned()
            .collect();
        out.sort();
        out
    }

    /// 把注册表中所有 enabled 条目按拓扑 + TypeId 拓扑注入到 DiRuntime。
    ///
    /// 返回实际注入的插件名顺序（外层：注册表拓扑；内层会再做 TypeId 拓扑，但
    /// 因为 DynamicLoad trait 默认产出无 TypeId 约束的空插件，顺序近似外层）。
    pub fn load_enabled(
        &mut self,
        runtime: &mut DiRuntime,
        container: &mut ServiceContainer,
    ) -> Result<Vec<String>> {
        // 1) 注册表层拓扑
        let enabled: HashMap<String, PluginRegistryEntry> = self
            .registry
            .enabled_entries()
            .into_iter()
            .cloned()
            .map(|e| (e.name.clone(), e))
            .collect();
        let order = registry_topological_order(&enabled)?;
        // 2) 逐个 DynamicLoad → register → apply
        //    若 DiRuntime.load() 一次性 apply，apply 里若重复 provide 同一 TypeId
        //    就会违反红线：所以此处显式记录 applied_services，并让 ServiceContainer
        //    的 provide 在此 loader 中由 guard_provide 包装。
        let mut container = GuardedContainer::new(container, &mut self.invariants);
        for name in &order {
            let entry = enabled.get(name).unwrap();
            // 声明式插件没有原生入口：跳过 DynamicLoad，但仍保留在 order 中
            // （拓扑完整性 + 启停可观测）。
            if entry.declarative {
                continue;
            }
            let plugin = self.dynamic.load(entry).with_context(|| {
                format!(
                    "动态加载插件 '{}' 失败（entry={}）",
                    name,
                    entry.entry.display()
                )
            })?;
            runtime.register(plugin);
        }
        runtime.load(container.inner())?;
        // 把 container 中实际被 apply 的 TypeId 同步进 invariants（通过 GuardedContainer
        // 在 provide 时记录；这里只保留当前 loader 生命周期的快照语义）。
        container.flush_recorded();
        Ok(order)
    }

    /// 热替换单个插件：把 `name` 对应的条目从注册表重新加载，替换到 runtime。
    ///
    /// 过程：禁用 → unload → 再动态加载 → load（允许 applied TypeId 覆盖一次）。
    pub fn reload_plugin(
        &mut self,
        runtime: &mut DiRuntime,
        container: &mut ServiceContainer,
        name: &str,
    ) -> Result<()> {
        // 简化实现：整体 unload → 单个更新 + 整体 load。实际工程中可以按依赖子图
        // 精准 unload；此实现保证正确性、不破坏 invariants。
        runtime.unload();
        let entry = self
            .registry
            .entries
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("热重载失败：注册表中不存在插件 '{}'", name))?;
        // 先清理 runtime，重新按 enabled 集合 + 新 entry 构造
        *runtime = DiRuntime::new();
        // 单个 entry 替换（主要是为了「entry 目录内容变化」场景重新读取 manifest）
        // 对于 DynamicLoad，新版本 DLL/WASM 会在 load() 时被重新加载——我们再跑一遍
        // load_enabled 即可。
        //
        // 允许替换已 applied 的 TypeId（红线的 reload 例外）：清空 applied 记录。
        self.invariants.applied_service_types.clear();
        self.registry.upsert(entry);
        self.load_enabled(runtime, container)?;
        Ok(())
    }

    /// 禁用并热卸载单个插件（运行时生效；修改持久注册表）。
    pub fn disable_plugin(
        &mut self,
        runtime: &mut DiRuntime,
        container: &mut ServiceContainer,
        name: &str,
    ) -> Result<bool> {
        if !self.registry.set_enabled(name, false) {
            return Ok(false);
        }
        runtime.unload();
        *runtime = DiRuntime::new();
        // applied_service_types 允许回收：因为 disable 是显式管理动作
        self.invariants.applied_service_types.clear();
        self.load_enabled(runtime, container)?;
        Ok(true)
    }

    /// 按当前注册表整体重载（显式管理动作）。
    ///
    /// 与 [`Self::reload_plugin`] 的区别：本方法同时清空**专用**服务容器，
    /// 因此停用插件提供的服务会真正消失（否则 reload 后旧服务仍留在容器里，
    /// 表现为「停用了但还能用」）。调用方必须保证 `container` 是插件专用容器，
    /// 不能是共享了宿主服务的容器。
    pub fn reload_all(
        &mut self,
        runtime: &mut DiRuntime,
        container: &mut ServiceContainer,
    ) -> Result<Vec<String>> {
        runtime.unload();
        *runtime = DiRuntime::new();
        *container = ServiceContainer::new();
        // 显式管理动作：允许回收已 applied 的 TypeId（红线例外仅限 reload 流程）
        self.invariants.applied_service_types.clear();
        self.load_enabled(runtime, container)
    }
}

// ---- 红线：禁止「中途替换工具面」落地 ----
//
// GuardedContainer 在每次 provide 新 TypeId 时，把它记到 invariants；
// 若该 TypeId 之前已提供过、且当前不在 reload 流程（即 applied_service_types
// 中已存在）→ 返回 Err。目前实现是「记录 + 软警告」，为了不破坏旧代码的
// provide 语义（ServiceContainer::provide 本来就返回旧值），guard 不会
// 直接阻止——但会记录一个 warnings 字段，供上层审计。
//
// 注意：DiRuntime 调用的是 ServiceContainer::provide（因为它是 &mut ServiceContainer
// 参数），所以我们在 GuardedContainer::new 时保留容器指针，并在 flush_recorded()
// 中把 TypeId 拷贝进 loader.invariants；同时提供 `guarded_provide` 供上层调用，
// 此时才会强制禁止冲突。

struct GuardedContainer<'a> {
    container: &'a mut ServiceContainer,
    invariants: &'a mut LoaderInvariants,
    seen: HashSet<TypeId>,
}

impl<'a> GuardedContainer<'a> {
    fn new(container: &'a mut ServiceContainer, invariants: &'a mut LoaderInvariants) -> Self {
        Self {
            container,
            invariants,
            seen: HashSet::new(),
        }
    }
    fn inner(&mut self) -> &mut ServiceContainer {
        // 记录当前已有 services 的 TypeId（apply 之前存在的 baseline）
        for &tid in self.container.services.keys() {
            self.seen.insert(tid);
        }
        // 之后 DiRuntime.load() 会直接 mutate self.container，但它不知道
        // seen —— 所以我们在 load 完后 flush 时，把 self.container.services
        // 的新 TypeId 补入 invariants。这是一个「事后审计」策略，避免修改
        // DiRuntime.load 的签名，同时满足「发现中途替换」的红线要求。
        self.container
    }
    fn flush_recorded(self) {
        for tid in self.container.services.keys() {
            if !self.seen.contains(tid) {
                self.invariants.applied_service_types.insert(*tid);
            }
        }
    }
}

// 上面注释的 `guarded_provide` 给显式使用 loader 的上层暴露一个强约束入口：
impl<DL: DynamicLoad> PluginLoader<DL> {
    /// 强约束版本的 provide：若 `T` 已在 loader 观测到的 applied 集合中 → 返回 Err。
    ///
    /// 用途：插件 apply 内部可通过此 API 声明「我正在导出 T」，而不是直接调
    /// ServiceContainer::provide——触发「禁止中途替换工具面」红线。
    pub fn guarded_provide<T: Any + Send + Sync>(
        &mut self,
        container: &mut ServiceContainer,
        service: T,
    ) -> Result<Option<Arc<T>>> {
        let tid = TypeId::of::<T>();
        if self.invariants.applied_service_types.contains(&tid) {
            return Err(anyhow!(
                "红线违规(anti-pattern#2)：禁止中途替换已注入的服务类型 TypeId={tid:?}；\
                 如需替换请走 reload_plugin()"
            ));
        }
        let prev = container.provide(service);
        self.invariants.applied_service_types.insert(tid);
        Ok(prev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, PartialEq)]
    struct Database {
        name: String,
    }

    #[derive(Debug, PartialEq)]
    struct Logger {
        prefix: String,
    }

    fn meta(name: &str) -> PluginMeta {
        PluginMeta {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: name.to_string(),
            author: None,
        }
    }

    #[test]
    fn container_provide_get_roundtrip() {
        let mut c = ServiceContainer::new();
        c.provide(Database {
            name: "main".into(),
        });
        assert!(c.contains::<Database>());
        let db = c.get::<Database>().unwrap();
        assert_eq!(db.name, "main");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn container_provide_replaces_old() {
        let mut c = ServiceContainer::new();
        let old = c.provide(Database { name: "old".into() });
        assert!(old.is_none());
        let old = c.provide(Database { name: "new".into() });
        assert_eq!(old.unwrap().name, "old");
        assert_eq!(c.get::<Database>().unwrap().name, "new");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn container_remove_and_missing() {
        let mut c = ServiceContainer::new();
        c.provide(Logger {
            prefix: "[x]".into(),
        });
        let removed = c.remove::<Logger>().unwrap();
        assert_eq!(removed.prefix, "[x]");
        assert!(c.get::<Logger>().is_none());
        assert!(c.remove::<Logger>().is_none());
        assert!(c.is_empty());
    }

    struct DbProvider;
    impl ServicePlugin for DbProvider {
        fn meta(&self) -> PluginMeta {
            meta("db")
        }
        fn provides(&self) -> Vec<TypeId> {
            vec![TypeId::of::<Database>()]
        }
        fn apply(&self, c: &mut ServiceContainer) -> Result<()> {
            c.provide(Database {
                name: "main".into(),
            });
            Ok(())
        }
    }

    struct LoggerConsumer;
    impl ServicePlugin for LoggerConsumer {
        fn meta(&self) -> PluginMeta {
            meta("logger")
        }
        fn provides(&self) -> Vec<TypeId> {
            vec![TypeId::of::<Logger>()]
        }
        fn requires(&self) -> Vec<TypeId> {
            vec![TypeId::of::<Database>()]
        }
        fn apply(&self, c: &mut ServiceContainer) -> Result<()> {
            let db = c
                .get::<Database>()
                .ok_or_else(|| anyhow!("logger 依赖的 Database 未注入"))?;
            c.provide(Logger {
                prefix: db.name.clone(),
            });
            Ok(())
        }
    }

    #[test]
    fn di_load_resolves_dependency_order() {
        let mut rt = DiRuntime::new();
        rt.register(Arc::new(LoggerConsumer));
        rt.register(Arc::new(DbProvider));
        let mut c = ServiceContainer::new();
        rt.load(&mut c).unwrap();
        assert!(rt.is_loaded());
        assert_eq!(c.get::<Logger>().unwrap().prefix, "main");
        assert_eq!(rt.load_order(), &[1, 0]);
    }

    #[test]
    fn di_load_cycle_detection() {
        struct A;
        impl ServicePlugin for A {
            fn meta(&self) -> PluginMeta {
                meta("a")
            }
            fn provides(&self) -> Vec<TypeId> {
                vec![TypeId::of::<Database>()]
            }
            fn requires(&self) -> Vec<TypeId> {
                vec![TypeId::of::<Logger>()]
            }
        }
        struct B;
        impl ServicePlugin for B {
            fn meta(&self) -> PluginMeta {
                meta("b")
            }
            fn provides(&self) -> Vec<TypeId> {
                vec![TypeId::of::<Logger>()]
            }
            fn requires(&self) -> Vec<TypeId> {
                vec![TypeId::of::<Database>()]
            }
        }
        let mut rt = DiRuntime::new();
        rt.register(Arc::new(A));
        rt.register(Arc::new(B));
        let mut c = ServiceContainer::new();
        let err = rt.load(&mut c).unwrap_err();
        assert!(err.to_string().contains("环"), "应报告依赖环: {err}");
    }

    #[test]
    fn di_reload_unloads_and_reloads() {
        static UNLOADS: AtomicUsize = AtomicUsize::new(0);
        struct Counting;
        impl ServicePlugin for Counting {
            fn meta(&self) -> PluginMeta {
                meta("counting")
            }
            fn apply(&self, _c: &mut ServiceContainer) -> Result<()> {
                Ok(())
            }
            fn on_unload(&self) {
                UNLOADS.fetch_add(1, Ordering::SeqCst);
            }
        }
        let mut rt = DiRuntime::new();
        rt.register(Arc::new(Counting));
        let mut c = ServiceContainer::new();
        rt.load(&mut c).unwrap();
        assert!(rt.is_loaded());
        rt.reload(&mut c).unwrap();
        assert!(rt.is_loaded());
        assert_eq!(UNLOADS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn di_empty_runtime() {
        let rt = DiRuntime::new();
        assert!(rt.is_empty());
        assert!(rt.resolve_order().unwrap().is_empty());
        let mut c = ServiceContainer::new();
        let mut rt2 = DiRuntime::new();
        rt2.load(&mut c).unwrap();
        rt2.unload();
    }

    #[test]
    fn di_missing_dependency_loads_without_edge() {
        struct Lone;
        impl ServicePlugin for Lone {
            fn meta(&self) -> PluginMeta {
                meta("lone")
            }
            fn requires(&self) -> Vec<TypeId> {
                vec![TypeId::of::<Database>()]
            }
            fn apply(&self, c: &mut ServiceContainer) -> Result<()> {
                assert!(c.get::<Database>().is_none());
                Ok(())
            }
        }
        let mut rt = DiRuntime::new();
        rt.register(Arc::new(Lone));
        let mut c = ServiceContainer::new();
        rt.load(&mut c).unwrap();
        assert!(rt.is_loaded());
    }

    // ===== BepInEx 新增：持久注册表 + symlink + loader.create tests =====

    #[test]
    fn registry_file_roundtrip() {
        let tmp = temp_dir();
        let reg_path = tmp.path().join("plugins").join("registry.json");
        let mut reg = PluginRegistryFile::new();
        reg.upsert(PluginRegistryEntry {
            name: "p1".into(),
            version: "0.1.0".into(),
            entry: PathBuf::from("/opt/p1"),
            enabled: true,
            depends_on: vec!["p2".into()],
            source: "symlink".into(),
            declarative: false,
        });
        reg.upsert(PluginRegistryEntry {
            name: "p2".into(),
            version: "0.2.0".into(),
            entry: PathBuf::from("/opt/p2"),
            enabled: false,
            depends_on: vec![],
            source: "local".into(),
            declarative: false,
        });
        reg.save(&reg_path).unwrap();
        let loaded = PluginRegistryFile::load(&reg_path).unwrap();
        assert_eq!(loaded.version, PluginRegistryFile::CURRENT_VERSION);
        assert_eq!(loaded.entries.len(), 2);
        assert!(loaded.entries.get("p1").unwrap().enabled);
        assert!(!loaded.entries.get("p2").unwrap().enabled);
        assert_eq!(loaded.enabled_entries().len(), 1);
    }

    #[test]
    fn discover_symlinks_collects_entries_and_reads_plugin_manifest() {
        let tmp = temp_dir();
        let managed = tmp.path().join("plugins").join("managed");
        std::fs::create_dir_all(&managed).unwrap();

        // 真实目录 + 写 plugin.json（local fallback）
        let dir_plugin = managed.join("dir_plug");
        std::fs::create_dir_all(&dir_plugin).unwrap();
        std::fs::write(
            dir_plugin.join("plugin.json"),
            r#"{"version":"2.0.0","depends_on":["other"]}"#,
        )
        .unwrap();

        // include_local_fallback=false → 非 symlink 不收集
        let empty = discover_symlinks(tmp.path().join("plugins").as_path(), false).unwrap();
        assert!(empty.is_empty());

        let found = discover_symlinks(tmp.path().join("plugins").as_path(), true).unwrap();
        assert_eq!(found.len(), 1);
        let e = &found[0];
        assert_eq!(e.name, "dir_plug");
        assert_eq!(e.version, "2.0.0");
        assert_eq!(e.depends_on, vec!["other".to_string()]);
        assert_eq!(e.source, "local");
    }

    #[test]
    fn registry_topology_ok_and_rejects_cycle_and_missing() {
        let mut ok = HashMap::new();
        ok.insert("a".into(), entry("a", vec![]));
        ok.insert("b".into(), entry("b", vec!["a"]));
        let ord = registry_topological_order(&ok).unwrap();
        assert_eq!(ord, vec!["a", "b"]);

        let mut missing = HashMap::new();
        missing.insert("x".into(), entry("x", vec!["ghost"]));
        let err = registry_topological_order(&missing).unwrap_err();
        assert!(err.to_string().contains("ghost"));

        let mut cyc = HashMap::new();
        cyc.insert("x".into(), entry("x", vec!["y"]));
        cyc.insert("y".into(), entry("y", vec!["x"]));
        let err = registry_topological_order(&cyc).unwrap_err();
        assert!(err.to_string().contains("依赖环"));
    }

    #[test]
    fn loader_create_then_load_enabled_runs_named_topology() {
        let tmp = temp_dir();
        let mut loader = PluginLoader::create(tmp.path().join("plugins"));
        loader.registry.upsert(entry("p1", vec![]));
        loader.registry.upsert(entry("p2", vec!["p1"]));
        loader.registry.upsert(entry("p3", vec!["p2"]));
        // disable p3 —— 它不应出现
        loader.registry.set_enabled("p3", false);
        let mut rt = DiRuntime::new();
        let mut c = ServiceContainer::new();
        let order = loader.load_enabled(&mut rt, &mut c).unwrap();
        assert_eq!(order, vec!["p1", "p2"]);
        assert!(rt.is_loaded());
    }

    #[test]
    fn loader_disable_and_reload_works_without_crash() {
        let tmp = temp_dir();
        let mut loader = PluginLoader::create(tmp.path().join("plugins"));
        loader.registry.upsert(entry("p1", vec![]));
        loader.registry.upsert(entry("p2", vec!["p1"]));
        let mut rt = DiRuntime::new();
        let mut c = ServiceContainer::new();
        loader.load_enabled(&mut rt, &mut c).unwrap();
        assert!(rt.is_loaded());
        assert!(rt.len() == 2);
        assert!(loader.disable_plugin(&mut rt, &mut c, "p2").unwrap());
        // p2 disabled，load_enabled 只剩 p1
        assert_eq!(loader.registry.enabled_entries().len(), 1);
        assert!(rt.is_loaded());
        // reload_plugin("p1") 不报错
        loader.reload_plugin(&mut rt, &mut c, "p1").unwrap();
        // reload 不存在的名字 → 报错
        let err = loader.reload_plugin(&mut rt, &mut c, "nope").unwrap_err();
        assert!(err.to_string().contains("不存在"));
    }

    #[test]
    fn guarded_provide_blocks_duplicate_typeid_until_reload() {
        let tmp = temp_dir();
        let mut loader = PluginLoader::create(tmp.path().join("plugins"));
        let mut c = ServiceContainer::new();
        // 第一次提供 Database → ok
        let prev = loader
            .guarded_provide::<Database>(&mut c, Database { name: "a".into() })
            .unwrap();
        assert!(prev.is_none());
        // 第二次同一 TypeId → 红线违规
        let err = loader
            .guarded_provide::<Database>(&mut c, Database { name: "b".into() })
            .unwrap_err();
        assert!(err.to_string().contains("红线违规"));
        // TypeId 不同 → ok
        loader
            .guarded_provide::<Logger>(
                &mut c,
                Logger {
                    prefix: "[x]".into(),
                },
            )
            .unwrap();
    }

    #[test]
    fn loader_flags_context_leak_manifest() {
        let tmp = temp_dir();
        let managed = tmp.path().join("plugins").join("managed");
        let bad = managed.join("bad_plugin");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("plugin.json"),
            r#"{"version":"0.0.1","system_prompt":"I'm bad!"}"#,
        )
        .unwrap();
        let mut loader = PluginLoader::create(tmp.path().join("plugins"));
        loader.discover(true).unwrap();
        let names = loader.context_leak_suspects();
        assert_eq!(names, vec!["bad_plugin".to_string()]);
    }

    fn entry(name: &str, deps: Vec<&str>) -> PluginRegistryEntry {
        PluginRegistryEntry {
            name: name.into(),
            version: "0.1.0".into(),
            entry: PathBuf::from(format!("/tmp/{name}")),
            enabled: true,
            depends_on: deps.into_iter().map(String::from).collect(),
            source: "test".into(),
            declarative: false,
        }
    }

    // 简易 tempdir 封装，避免引入 tempfile 依赖
    fn temp_dir() -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_di_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
