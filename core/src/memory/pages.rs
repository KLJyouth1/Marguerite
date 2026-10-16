//! 内置知识页（本地、免 Key、离线）——"Hindsight 替代"：
//!
//! 持久化知识页 + 语义×关键词混合检索 + 使用计数；数据存于应用数据目录，
//! 零外部 API、默认零依赖特征哈希嵌入后端（可注入真实模型后端升级语义质量），
//! 隐私完全本地化、可离线。相比云端知识库：无 Key/无外网/无费用 + 可审计、可移植。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::memory::embedding::EmbeddingProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgePage {
    pub id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub use_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    next_id: u64,
    pages: HashMap<String, KnowledgePage>,
}

#[derive(Debug, Clone)]
pub struct PageStore {
    path: PathBuf,
    state: State,
}

impl PageStore {
    pub fn open_at(path: PathBuf) -> Result<Self> {
        let state = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
                Err(_) => State::default(),
            }
        } else {
            State::default()
        };
        Ok(Self { path, state })
    }

    pub fn default_path() -> PathBuf {
        crate::persistence::config::Config::data_dir().join("knowledge-pages.json")
    }

    pub fn open_default() -> Result<Self> {
        Self::open_at(Self::default_path())
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.state)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn create(&mut self, title: &str, content: &str, tags: &[&str]) -> Result<String> {
        let now = now_secs();
        self.state.next_id += 1;
        let id = format!("p{}", self.state.next_id);
        self.state.pages.insert(
            id.clone(),
            KnowledgePage {
                id: id.clone(),
                title: title.to_string(),
                content: content.to_string(),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                created_at: now,
                updated_at: now,
                use_count: 0,
            },
        );
        self.save()?;
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Option<&KnowledgePage> {
        self.state.pages.get(id)
    }

    pub fn touch(&mut self, id: &str) -> Result<bool> {
        if let Some(p) = self.state.pages.get_mut(id) {
            p.use_count += 1;
            p.updated_at = now_secs();
            self.save()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let ok = self.state.pages.remove(id).is_some();
        if ok {
            self.save()?;
        }
        Ok(ok)
    }

    pub fn list(&self) -> Vec<&KnowledgePage> {
        let mut v: Vec<_> = self.state.pages.values().collect();
        v.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        v
    }

    pub fn len(&self) -> usize {
        self.state.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.pages.is_empty()
    }

    /// 混合检索：query 嵌入×页内容嵌入 余弦(0.6) + 关键词重合(0.4)；返回 (页, 得分)。
    pub fn search(&self, query: &str, top_k: usize) -> Vec<(KnowledgePage, f64)> {
        let backend = EmbeddingProvider::global();
        let qemb = backend.embed(query);
        let qtoks = tokenize(query);
        let mut scored: Vec<(KnowledgePage, f64)> = self
            .state
            .pages
            .values()
            .map(|p| {
                let hay = format!("{} {}", p.title, p.content);
                let semb = backend.embed(&hay);
                let cosine = backend.cosine(&qemb, &semb);
                let ptoks = tokenize(&hay);
                let overlap = if qtoks.is_empty() {
                    0.0
                } else {
                    let k = qtoks.iter().filter(|t| ptoks.contains(t)).count();
                    k as f64 / qtoks.len() as f64
                };
                let score = 0.6 * cosine.max(0.0) + 0.4 * overlap;
                (p.clone(), score)
            })
            .filter(|(_, s)| *s > 0.01)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }
}

fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每次调用生成唯一临时目录（并行测试互不干扰）
    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "marguerite-pages-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn clean(p: &PathBuf) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn create_list_get_roundtrip() {
        let p = tmp();
        clean(&p);
        let mut s = PageStore::open_at(p.join("pages.json")).unwrap();
        let id = s
            .create(
                "Claude 使用规范",
                "提交信息用中文，附测试基线 2121。",
                &["规范"],
            )
            .unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(&id).unwrap().title, "Claude 使用规范");
        assert_eq!(s.list().len(), 1);
        clean(&p);
    }

    #[test]
    fn search_ranks_relevant_page_first() {
        let p = tmp();
        clean(&p);
        let mut s = PageStore::open_at(p.join("pages.json")).unwrap();
        s.create(
            "Rust 并发",
            "Arc<Mutex> 跨 await 克隆锁，避免 MutexGuard 非 Send。",
            &["rust"],
        )
        .unwrap();
        s.create("做菜", "番茄炒蛋先炒蛋后下番茄。", &["cooking"])
            .unwrap();
        let hits = s.search("rust 并发 锁", 2);
        assert!(!hits.is_empty());
        assert!(
            hits[0].0.title.contains("Rust"),
            "相关页应排前: {:?}",
            hits[0].0.title
        );
        if hits.len() > 1 {
            assert!(
                hits[0].1 >= hits[1].1,
                "得分应降序: {:?} vs {:?}",
                hits[0].1,
                hits[1].1
            );
        }
        clean(&p);
    }

    #[test]
    fn touch_and_remove() {
        let p = tmp();
        clean(&p);
        let mut s = PageStore::open_at(p.join("pages.json")).unwrap();
        let id = s.create("A", "b", &[]).unwrap();
        assert!(s.touch(&id).unwrap());
        assert_eq!(s.get(&id).unwrap().use_count, 1);
        assert!(s.remove(&id).unwrap());
        assert_eq!(s.len(), 0);
        assert!(!s.remove(&id).unwrap());
        clean(&p);
    }

    #[test]
    fn persistence_survives_reopen() {
        let p = tmp();
        clean(&p);
        let path = p.join("pages.json");
        {
            let mut s = PageStore::open_at(path.clone()).unwrap();
            s.create("持久", "跨重启命中", &[]).unwrap();
        }
        let s2 = PageStore::open_at(path.clone()).unwrap();
        assert_eq!(s2.len(), 1);
        assert!(s2.search("命中", 1)[0].0.content.contains("跨重启"));
        clean(&p);
    }
}
