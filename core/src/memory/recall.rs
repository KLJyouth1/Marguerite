//! claude-mem 融合层：四类持久记忆（跨会话）+ 统一回忆
//!
//! claude-mem 的核心能力是「跨会话持久记忆」——把用户偏好、项目约定、
//! 反馈纠错、自动提取的常识沉淀下来，供后续会话召回。
//!
//! 融合策略（守 D3 不搬运）：原生实现，复用既有资产——
//! - `Project` 记忆走 **KB 2.0** 的 `KnowledgeBase`（BM25 检索），即 RAG 主面；
//! - `User` / `Feedback` / `Auto` 走内存映射（会话级，未来可接 `memory/` 持久层）。
//!
//! 暴露 `MemoryBackend` trait，使上层（ACP `memory.*`、编排器）能以统一接口读写。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{bail, Result};

use crate::kb::{KnowledgeBase, RetrievalResult};

/// claude-mem 四类持久记忆
///
/// 对应 claude-mem 的 USER / PROJECT / FEEDBACK / auto 记忆体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryCategory {
    /// 用户画像与偏好（claude-mem: USER.md）
    User,
    /// 项目上下文与约定（claude-mem: PROJECT.md）
    Project,
    /// 反馈与纠错（claude-mem: FEEDBACK.md）
    Feedback,
    /// 自动提取的常识（claude-mem: auto）
    Auto,
}

impl MemoryCategory {
    /// 规范字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryCategory::User => "user",
            MemoryCategory::Project => "project",
            MemoryCategory::Feedback => "feedback",
            MemoryCategory::Auto => "auto",
        }
    }

    /// 全部类别（用于 `memory.list` 等枚举场景）
    pub fn all() -> &'static [MemoryCategory] {
        &[
            MemoryCategory::User,
            MemoryCategory::Project,
            MemoryCategory::Feedback,
            MemoryCategory::Auto,
        ]
    }

    /// 从字符串解析（大小写不敏感），非法值返回 None
    pub fn parse(s: &str) -> Option<MemoryCategory> {
        match s.to_lowercase().as_str() {
            "user" => Some(MemoryCategory::User),
            "project" => Some(MemoryCategory::Project),
            "feedback" => Some(MemoryCategory::Feedback),
            "auto" => Some(MemoryCategory::Auto),
            _ => None,
        }
    }
}

/// 单条回忆命中
#[derive(Debug, Clone)]
pub struct RecallHit {
    /// 命中所属记忆类别
    pub category: MemoryCategory,
    /// 条目键（project 记忆中为文档标题）
    pub key: String,
    /// 召回片段
    pub snippet: String,
    /// 相关性得分（BM25 或子串匹配得分）
    pub score: f64,
}

/// 持久化键值存储：内存 HashMap + JSON 文件落盘（零外部依赖）。
///
/// 用于 `User` / `Feedback` / `Auto` 三类记忆的跨会话沉淀。
/// `dir` 为 `None` 时为纯内存（不落盘，向后兼容 `KbBackedMemory::new()`）。
pub struct PersistentKV {
    map: HashMap<String, String>,
    /// `Some(p)` 时每次写入落盘到 `p`（JSON）；`None` 时纯内存。
    path: Option<PathBuf>,
}

impl PersistentKV {
    /// 构造。
    ///
    /// `dir` 为 `Some(d)` 时：从 `d/<name>.json` 加载既有记忆（文件不存在则空），
    /// 后续写入落盘到该路径。`None` 时纯内存。
    pub fn new(name: &str, dir: Option<&Path>) -> Self {
        match dir {
            Some(d) => {
                let path = d.join(format!("{name}.json"));
                let map = load_json_map(&path).unwrap_or_default();
                Self {
                    map,
                    path: Some(path),
                }
            }
            None => Self {
                map: HashMap::new(),
                path: None,
            },
        }
    }

    /// 只读访问底层 map（供召回打分）。
    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.map
    }

    /// 插入/覆盖；`path` 有值时落盘（失败静默，不破坏主流程）。
    pub fn insert(&mut self, key: String, value: String) {
        self.map.insert(key, value);
        self.save();
    }

    /// 删除；`path` 有值时落盘。返回是否确有删除。
    pub fn remove(&mut self, key: &str) -> bool {
        let removed = self.map.remove(key).is_some();
        if removed {
            self.save();
        }
        removed
    }

    /// 条目数。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 落盘（仅 `path` 有值）。任何失败静默忽略，保证主流程健壮。
    ///
    /// **原子写**：先写同目录临时文件，再 `rename` 覆盖目标。
    ///
    /// 历史缺陷用 `std::fs::write`（**先截断再写**）：进程在写入中途被杀 / 断电会留下
    /// **半截 JSON**，而 [`load_json_map`] 解析失败即返回 `None` —— 等于**整份召回记忆丢失**。
    /// `rename` 在同一文件系统内是原子的，读者要么看到旧内容、要么看到完整新内容。
    fn save(&self) {
        let Some(p) = &self.path else { return };
        let Ok(json) = serde_json::to_string_pretty(&self.map) else {
            return;
        };
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, json.as_bytes()).is_ok() && std::fs::rename(&tmp, p).is_ok() {
            return;
        }
        // 退路：临时文件不可用（权限 / 跨设备 rename）时退回直接写 ——
        // 尽力而为，至少不让本次更新静默丢失。
        let _ = std::fs::write(p, json);
    }
}

/// 从 JSON 文件加载 map（`{"k":"v",...}`）；文件不存在/损坏返回 None。
fn load_json_map(path: &Path) -> Option<HashMap<String, String>> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<HashMap<String, String>>(&content).ok()
}

/// 统一记忆后端
///
/// claude-mem 概念对齐后的单一读写接口。实现方决定各类别的底层存储。
pub trait MemoryBackend: Send + Sync {
    /// 写入一条记忆（相同 key 覆盖）
    fn remember(&self, category: MemoryCategory, key: &str, value: &str) -> Result<()>;

    /// 按查询召回 top_k 条记忆
    fn recall(&self, category: MemoryCategory, query: &str, top_k: usize) -> Vec<RecallHit>;

    /// 删除一条记忆，返回是否确实存在并删除
    fn forget(&self, category: MemoryCategory, key: &str) -> Result<bool>;

    /// 某类别下记忆条数
    fn count(&self, category: MemoryCategory) -> usize;
}

/// 基于 KB 2.0 的记忆后端（默认实现）
///
/// 诚实标注：
/// - `Project` 走 BM25 索引（持久化与图谱增强在 `kb` 模块闭环）；
/// - `User` / `Feedback` / `Auto` 走 JSON 文件持久层（`with_persistence` 指定目录），
///   跨会话沉淀；`new()` / `from_project_kb()` 保持纯内存向后兼容。
pub struct KbBackedMemory {
    project: RwLock<KnowledgeBase>,
    user: RwLock<PersistentKV>,
    feedback: RwLock<PersistentKV>,
    auto: RwLock<PersistentKV>,
}

impl Default for KbBackedMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl KbBackedMemory {
    /// 构造空后端（纯内存，不持久化；向后兼容常驻调用点）。
    ///
    /// 若需跨会话持久化，使用 `with_persistence(dir)`。
    pub fn new() -> Self {
        Self::with_persistence_opt(None)
    }

    /// 由既有 KB 2.0 文档种子构造（纯内存，向后兼容）。
    pub fn from_project_kb(kb: KnowledgeBase) -> Self {
        Self {
            project: RwLock::new(kb),
            user: RwLock::new(PersistentKV::new("user", None)),
            feedback: RwLock::new(PersistentKV::new("feedback", None)),
            auto: RwLock::new(PersistentKV::new("auto", None)),
        }
    }

    /// 构造并启用持久化：User/Feedback/Auto 三类落盘到 `dir` 下的
    /// `user.json` / `feedback.json` / `auto.json`（启动时加载既有记忆）。
    pub fn with_persistence(dir: &Path) -> Self {
        Self::with_persistence_opt(Some(dir))
    }

    /// 内部入口：`dir` 为 `None` 时纯内存，`Some` 时持久化。
    fn with_persistence_opt(dir: Option<&Path>) -> Self {
        Self {
            project: RwLock::new(KnowledgeBase::new()),
            user: RwLock::new(PersistentKV::new("user", dir)),
            feedback: RwLock::new(PersistentKV::new("feedback", dir)),
            auto: RwLock::new(PersistentKV::new("auto", dir)),
        }
    }

    fn map_for(&self, category: MemoryCategory) -> &RwLock<PersistentKV> {
        match category {
            MemoryCategory::User => &self.user,
            MemoryCategory::Feedback => &self.feedback,
            MemoryCategory::Auto => &self.auto,
            MemoryCategory::Project => {
                // project 走 KB，此处不应被调用；返回 user 以保类型，逻辑在 trait 方法中分流
                &self.user
            }
        }
    }
}

impl MemoryBackend for KbBackedMemory {
    fn remember(&self, category: MemoryCategory, key: &str, value: &str) -> Result<()> {
        if key.trim().is_empty() {
            bail!("memory key 不能为空");
        }
        match category {
            MemoryCategory::Project => {
                self.project
                    .write()
                    .map_err(|_| anyhow::anyhow!("project 记忆锁 poisoned"))?
                    .add_document(key, value);
            }
            other => {
                self.map_for(other)
                    .write()
                    .map_err(|_| anyhow::anyhow!("{:?} 记忆锁 poisoned", other))?
                    .insert(key.to_string(), value.to_string());
            }
        }
        Ok(())
    }

    fn recall(&self, category: MemoryCategory, query: &str, top_k: usize) -> Vec<RecallHit> {
        let top_k = top_k.max(1);
        match category {
            MemoryCategory::Project => self
                .project
                .read()
                .map(|kb| kb.retrieve(query, top_k))
                .unwrap_or_default()
                .into_iter()
                .map(|r: RetrievalResult| RecallHit {
                    category: MemoryCategory::Project,
                    key: r.title,
                    snippet: r.chunk,
                    score: r.score,
                })
                .collect(),
            other => {
                let guard = self
                    .map_for(other)
                    .read()
                    .expect("memory map lock poisoned");
                recall_from_map(guard.as_map(), other, query, top_k)
            }
        }
    }

    fn forget(&self, category: MemoryCategory, key: &str) -> Result<bool> {
        let removed = match category {
            MemoryCategory::Project => {
                // KB 2.0 当前为追加式索引，无单键删除；诚实返回 false 并标注
                // 未来可加 tombstone。此处保持接口完整。
                let _ = key;
                false
            }
            other => self
                .map_for(other)
                .write()
                .map_err(|_| anyhow::anyhow!("{:?} 记忆锁 poisoned", other))?
                .remove(key),
        };
        Ok(removed)
    }

    fn count(&self, category: MemoryCategory) -> usize {
        match category {
            MemoryCategory::Project => self
                .project
                .read()
                .map(|kb| kb.document_count())
                .unwrap_or(0),
            other => self.map_for(other).read().map(|m| m.len()).unwrap_or(0),
        }
    }
}

/// 对内存映射做轻量子串/词重叠召回（确定性，便于单测）
fn recall_from_map(
    map: &HashMap<String, String>,
    category: MemoryCategory,
    query: &str,
    top_k: usize,
) -> Vec<RecallHit> {
    let q = query.to_lowercase();
    let q_tokens: Vec<&str> = q.split_whitespace().filter(|t| t.len() >= 2).collect();
    let mut scored: Vec<(String, String, f64)> = Vec::new();
    for (key, value) in map {
        let hay = format!("{} {}", key.to_lowercase(), value.to_lowercase());
        let mut score = 0.0f64;
        if !q.is_empty() {
            if hay.contains(&q) {
                score += 2.0;
            }
            for t in &q_tokens {
                if hay.contains(*t) {
                    score += 1.0;
                }
            }
        } else {
            // 空查询：返回全部（score 0），便于列出
            score = 0.0;
        }
        if score > 0.0 || q.is_empty() {
            scored.push((key.clone(), value.clone(), score));
        }
    }
    scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(top_k)
        .map(|(key, value, score)| RecallHit {
            category,
            key,
            snippet: value,
            score,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_parse_roundtrip() {
        assert_eq!(MemoryCategory::parse("user"), Some(MemoryCategory::User));
        assert_eq!(
            MemoryCategory::parse("PROJECT"),
            Some(MemoryCategory::Project)
        );
        assert_eq!(
            MemoryCategory::parse("feedback"),
            Some(MemoryCategory::Feedback)
        );
        assert_eq!(MemoryCategory::parse("auto"), Some(MemoryCategory::Auto));
        assert_eq!(MemoryCategory::parse("nonsense"), None);
        for c in MemoryCategory::all() {
            assert_eq!(MemoryCategory::parse(c.as_str()), Some(*c));
        }
    }

    #[test]
    fn project_memory_uses_bm25() {
        let mem = KbBackedMemory::new();
        mem.remember(
            MemoryCategory::Project,
            "deploy",
            "Kubernetes orchestrates containers. Helm charts package releases.",
        )
        .unwrap();
        mem.remember(
            MemoryCategory::Project,
            "lang",
            "Rust emphasizes memory safety without a garbage collector.",
        )
        .unwrap();
        assert_eq!(mem.count(MemoryCategory::Project), 2);

        let hits = mem.recall(MemoryCategory::Project, "kubernetes orchestration", 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, "deploy");
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn user_feedback_auto_memory_maps() {
        let mem = KbBackedMemory::new();
        mem.remember(MemoryCategory::User, "name", "喜欢用 Rust 写后端")
            .unwrap();
        mem.remember(MemoryCategory::Feedback, "style", "不要emoji图标")
            .unwrap();
        mem.remember(MemoryCategory::Auto, "fact", "项目使用 Cargo workspace")
            .unwrap();

        assert_eq!(mem.count(MemoryCategory::User), 1);
        assert_eq!(mem.count(MemoryCategory::Feedback), 1);
        assert_eq!(mem.count(MemoryCategory::Auto), 1);

        let hits = mem.recall(MemoryCategory::User, "rust 后端", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, "name");
        assert!(hits[0].score >= 1.0);
    }

    #[test]
    fn recall_substring_and_token_overlap() {
        let mem = KbBackedMemory::new();
        mem.remember(MemoryCategory::User, "pref", "偏好深色主题与等宽字体")
            .unwrap();
        // 完整子串命中应得更高分
        let a = mem.recall(MemoryCategory::User, "深色主题", 5);
        assert_eq!(a.len(), 1);
        assert!(a[0].score >= 2.0);
        // 单 token 命中仍应召回
        let b = mem.recall(MemoryCategory::User, "字体", 5);
        assert_eq!(b.len(), 1);
        assert!(b[0].score >= 1.0);
    }

    #[test]
    fn recall_confidence_is_measurable_and_orders_relevance() {
        let mem = KbBackedMemory::new();
        mem.remember(MemoryCategory::User, "stack", "偏好 Rust 与 Python 后端栈")
            .unwrap();
        mem.remember(MemoryCategory::User, "lang", "rust 语言基础")
            .unwrap();
        let hits = mem.recall(MemoryCategory::User, "rust 后端", 5);
        assert_eq!(hits.len(), 2);
        // 置信度可度量：每条命中都应带 >0 的 score
        assert!(hits.iter().all(|h| h.score > 0.0));
        // 置信度降序（相关性排序）
        let scores: Vec<f64> = hits.iter().map(|h| h.score).collect();
        for w in scores.windows(2) {
            assert!(w[0] >= w[1], "置信度应降序：{scores:?}");
        }
        // 同时命中完整词 + 单 token 的条目置信度最高
        assert_eq!(hits[0].key, "stack");
        assert!(hits[0].score > hits[1].score, "完整词命中应更高置信度");
    }

    #[test]
    fn empty_query_lists_all_in_map() {
        let mem = KbBackedMemory::new();
        mem.remember(MemoryCategory::Feedback, "a", "one").unwrap();
        mem.remember(MemoryCategory::Feedback, "b", "two").unwrap();
        let hits = mem.recall(MemoryCategory::Feedback, "", 10);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn forget_removes_from_map() {
        let mem = KbBackedMemory::new();
        mem.remember(MemoryCategory::User, "k", "v").unwrap();
        assert!(mem.forget(MemoryCategory::User, "k").unwrap());
        assert!(!mem.forget(MemoryCategory::User, "k").unwrap());
        assert_eq!(mem.count(MemoryCategory::User), 0);
    }

    #[test]
    fn empty_key_rejected() {
        let mem = KbBackedMemory::new();
        assert!(mem.remember(MemoryCategory::User, "  ", "v").is_err());
    }

    #[test]
    fn project_forget_is_noop_but_safe() {
        // KB 2.0 当前无单键删除；接口返回 false 而不报错
        let mem = KbBackedMemory::new();
        mem.remember(MemoryCategory::Project, "doc", "content")
            .unwrap();
        assert!(!mem.forget(MemoryCategory::Project, "doc").unwrap());
        assert_eq!(mem.count(MemoryCategory::Project), 1);
    }

    #[test]
    fn persistent_kv_survives_restart() {
        let dir = std::env::temp_dir().join(format!("marg_mem_kv_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("user.json");
        let _ = std::fs::remove_file(&path); // 确保干净起点

        // 第一次实例：写入并落盘
        {
            let mut kv = PersistentKV::new("user", Some(&dir));
            kv.insert("name".into(), "喜欢 Rust".into());
            kv.insert("pref".into(), "深色主题".into());
        }
        // 文件应已落盘
        assert!(path.exists(), "记忆应已落盘到 JSON 文件");

        // 第二次实例：从同目录重新加载（模拟进程重启）
        let kv = PersistentKV::new("user", Some(&dir));
        assert_eq!(kv.len(), 2);
        assert_eq!(
            kv.as_map().get("name").map(String::as_str),
            Some("喜欢 Rust")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 原子写：每次落盘后**不残留** `.tmp`，且目标文件始终是**完整可解析** JSON
    /// （`std::fs::write` 的「先截断再写」在崩溃时会留下半截 → 整份记忆丢失）。
    #[test]
    fn persistent_kv_save_is_atomic_and_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!("marg_mem_atomic_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("user.json");
        let _ = std::fs::remove_file(&path);
        let tmp = path.with_extension("json.tmp");

        let mut kv = PersistentKV::new("user", Some(&dir));
        for i in 0..20 {
            kv.insert(format!("k{i}"), format!("v{i}"));
            assert!(!tmp.exists(), "不应残留临时文件");
            let raw = std::fs::read_to_string(&path).expect("目标文件应存在");
            let parsed: std::collections::HashMap<String, String> =
                serde_json::from_str(&raw).expect("目标文件必须是完整 JSON（原子写）");
            assert_eq!(parsed.len(), i + 1);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kb_backed_memory_persists_user_across_instances() {
        let dir = std::env::temp_dir().join(format!("marg_mem_kb_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        // 实例 A：持久化写入 user / feedback 记忆
        {
            let mem = KbBackedMemory::with_persistence(&dir);
            mem.remember(MemoryCategory::User, "name", "喜欢用 Rust 写后端")
                .unwrap();
            mem.remember(MemoryCategory::Feedback, "style", "不要 emoji 图标")
                .unwrap();
        }

        // 实例 B：同目录重建，应召回 A 写入的记忆（跨会话沉淀）
        let mem = KbBackedMemory::with_persistence(&dir);
        assert_eq!(mem.count(MemoryCategory::User), 1);
        assert_eq!(mem.count(MemoryCategory::Feedback), 1);
        let hits = mem.recall(MemoryCategory::User, "rust", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].key, "name");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
