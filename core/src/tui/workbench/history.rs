//! 命令历史：跨会话持久化 + 上/下召回 + Ctrl+R 反向搜索（Phase 4 / AC-08）
//!
//! 设计：单例式历史存于 [`ComposerState.history`]，按时间正序保存已提交命令；
//! 提交去重（与最近一条相同则跳过）、上限 [`MAX_ENTRIES`] 防止无限增长。
//! 落盘为纯文本（一行一条，`~/.marguerite/history`），不引入新依赖（无 `toml` / `dirs`）。
//! `enabled=false` 时不读写文件（用于单元测试，避免副作用）。

use std::path::PathBuf;

/// 历史最大条数
const MAX_ENTRIES: usize = 500;

/// 反向搜索瞬时状态
#[derive(Debug, Clone, Default)]
struct SearchState {
    /// 查询串（大小写不敏感子串匹配）
    query: String,
    /// 当前匹配结果（按时间倒序的 entries 下标）
    matches: Vec<usize>,
    /// 当前选中匹配下标
    sel: usize,
}

/// 命令历史
#[derive(Debug, Clone, Default)]
pub struct CommandHistory {
    /// 历史条目（时间正序）
    entries: Vec<String>,
    /// 召回导航索引：Some(i) 表示正在浏览 entries[i]；None 表示编辑当前缓冲
    nav: Option<usize>,
    /// Ctrl+R 反向搜索状态（None 表示未进入搜索）
    search: Option<SearchState>,
    /// 是否启用落盘（false 时 push 不写文件，用于测试）
    enabled: bool,
}

impl CommandHistory {
    /// 空历史（不启用落盘）
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 `~/.marguerite/history` 加载并启用落盘；文件缺失/损坏则空，不报错
    pub fn load() -> Self {
        let mut h = Self::new();
        h.enabled = true;
        if let Some(path) = history_path() {
            if let Ok(s) = std::fs::read_to_string(&path) {
                for line in s.lines() {
                    let line = line.trim_end_matches(['\r', '\n']);
                    if !line.is_empty() {
                        h.entries.push(line.to_string());
                    }
                }
            }
        }
        h.entries.truncate(MAX_ENTRIES);
        h
    }

    /// 是否已启用落盘
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 提交一条命令：去重（与最近相同则跳过）、入栈、落盘
    pub fn push(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        if self.entries.last().map(|s| s.as_str()) == Some(cmd) {
            // 与最近一条重复，仅重置导航/搜索态，不重复记录
            self.nav = None;
            self.search = None;
            return;
        }
        self.entries.push(cmd.to_string());
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.nav = None;
        self.search = None;
        self.persist();
    }

    /// 上翻：返回应填入缓冲的历史条目（从最近往旧走）
    pub fn recall_up(&mut self, _current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = match self.nav {
            Some(i) if i > 0 => i - 1,
            Some(i) => i, // 已在最旧，停住
            None => self.entries.len() - 1,
        };
        self.nav = Some(idx);
        Some(self.entries[idx].clone())
    }

    /// 下翻：回到更新的条目；越过最新则清空缓冲
    pub fn recall_down(&mut self) -> Option<String> {
        match self.nav {
            Some(i) if i + 1 < self.entries.len() => {
                let n = i + 1;
                self.nav = Some(n);
                Some(self.entries[n].clone())
            }
            Some(_) => {
                self.nav = None;
                Some(String::new())
            }
            None => None,
        }
    }

    /// 进入 Ctrl+R 反向搜索（按时间倒序列出全部条目供筛选）
    pub fn start_search(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let matches: Vec<usize> = (0..self.entries.len()).rev().collect();
        self.search = Some(SearchState {
            query: String::new(),
            matches,
            sel: 0,
        });
    }

    /// 是否处于搜索态
    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    /// 当前搜索查询串
    pub fn search_query(&self) -> &str {
        self.search.as_ref().map(|s| s.query.as_str()).unwrap_or("")
    }

    /// 搜索态输入字符
    pub fn search_type(&mut self, c: char) {
        if let Some(s) = self.search.as_mut() {
            s.query.push(c);
            self.recompute_matches();
        }
    }

    /// 搜索态退格
    pub fn search_backspace(&mut self) {
        if let Some(s) = self.search.as_mut() {
            s.query.pop();
            self.recompute_matches();
        }
    }

    /// 搜索态下翻候选（更旧）
    pub fn search_next(&mut self) {
        if let Some(s) = self.search.as_mut() {
            if !s.matches.is_empty() {
                s.sel = (s.sel + 1) % s.matches.len();
            }
        }
    }

    /// 搜索态上翻候选（更新）
    pub fn search_prev(&mut self) {
        if let Some(s) = self.search.as_mut() {
            if !s.matches.is_empty() {
                s.sel = (s.sel + s.matches.len() - 1) % s.matches.len();
            }
        }
    }

    /// 当前选中的搜索候选文本
    pub fn search_current(&self) -> Option<String> {
        self.search
            .as_ref()
            .and_then(|s| s.matches.get(s.sel).map(|&i| self.entries[i].clone()))
    }

    /// 退出搜索；accept=true 返回选中命令填入缓冲，否则返回 None
    pub fn exit_search(&mut self, accept: bool) -> Option<String> {
        let chosen = if accept { self.search_current() } else { None };
        self.search = None;
        self.nav = None;
        chosen
    }

    fn recompute_matches(&mut self) {
        if let Some(s) = self.search.as_mut() {
            let q = s.query.to_lowercase();
            s.matches = (0..self.entries.len())
                .rev()
                .filter(|&i| q.is_empty() || self.entries[i].to_lowercase().contains(&q))
                .collect();
            s.sel = 0;
        }
    }

    fn persist(&self) {
        if !self.enabled {
            return;
        }
        if let Some(path) = history_path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let content: String = self.entries.iter().map(|e| format!("{e}\n")).collect();
            let _ = std::fs::write(&path, content);
        }
    }
}

/// 历史文件路径：`~/.marguerite/history`（纯文本，一行一条）
pub fn history_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join(".marguerite").join("history"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_dedups_consecutive_and_trims() {
        let mut h = CommandHistory::new();
        h.push("  hello  ");
        h.push("hello"); // 与最近重复，应跳过
        h.push("world");
        assert_eq!(h.entries, vec!["hello".to_string(), "world".to_string()]);
    }

    #[test]
    fn test_push_skips_empty() {
        let mut h = CommandHistory::new();
        h.push("");
        h.push("   ");
        assert!(h.entries.is_empty());
    }

    #[test]
    fn test_recall_up_down_cycle() {
        let mut h = CommandHistory::new();
        for c in ["a", "b", "c"] {
            h.push(c);
        }
        assert_eq!(h.recall_up(""), Some("c".to_string()));
        assert_eq!(h.recall_up(""), Some("b".to_string()));
        assert_eq!(h.recall_up(""), Some("a".to_string()));
        assert_eq!(h.recall_up(""), Some("a".to_string())); // 停在最旧
        assert_eq!(h.recall_down(), Some("b".to_string()));
        assert_eq!(h.recall_down(), Some("c".to_string()));
        assert_eq!(h.recall_down(), Some(String::new())); // 越过末尾清空
    }

    #[test]
    fn test_search_filters_and_navigates() {
        let mut h = CommandHistory::new();
        for c in ["ls src", "cat main.rs", "ls target", "git status"] {
            h.push(c);
        }
        h.start_search();
        assert!(h.is_searching());
        h.search_type('l');
        h.search_type('s');
        // 倒序匹配：ls target(2), ls src(0)
        assert_eq!(h.search_current(), Some("ls target".to_string()));
        h.search_next();
        assert_eq!(h.search_current(), Some("ls src".to_string()));
        h.search_prev();
        assert_eq!(h.search_current(), Some("ls target".to_string()));
        // 接受选中
        let chosen = h.exit_search(true);
        assert_eq!(chosen, Some("ls target".to_string()));
        assert!(!h.is_searching());
    }

    #[test]
    fn test_search_empty_query_lists_all_newest_first() {
        let mut h = CommandHistory::new();
        for c in ["a", "b", "c"] {
            h.push(c);
        }
        h.start_search();
        assert_eq!(h.search_current(), Some("c".to_string()));
        let none = h.exit_search(false);
        assert_eq!(none, None);
    }

    #[test]
    fn test_disabled_does_not_enable() {
        let h = CommandHistory::new();
        assert!(!h.enabled());
        let loaded = CommandHistory::load();
        assert!(loaded.enabled());
    }
}
