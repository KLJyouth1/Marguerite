//! 上下文分页引擎
//!
//! 借鉴 NexusStar context_paging：按语义边界（函数/类/段落）切分大上下文，
//! 摘要页自动生成，用户可展开/折叠。极致利用 token 的核心机制。

use serde::{Deserialize, Serialize};

use crate::types::Message;

/// 分页选项
#[derive(Debug, Clone)]
pub struct PagingOptions {
    /// 单页最大字符数
    pub max_chars: usize,
    /// 语义边界标记是否启用
    pub semantic_split: bool,
}

impl Default for PagingOptions {
    fn default() -> Self {
        Self {
            max_chars: 4000,
            semantic_split: true,
        }
    }
}

/// 上下文页类型
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageKind {
    /// 完整内容页
    Content,
    /// 摘要页（折叠态）
    Summary,
}

/// 上下文页
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPage {
    /// 页 ID
    pub id: usize,
    /// 页类型
    pub kind: PageKind,
    /// 内容
    pub content: String,
    /// 摘要（摘要页时为空）
    pub summary: Option<String>,
    /// 语义标题（如函数名）
    pub title: Option<String>,
    /// 是否展开
    pub expanded: bool,
}

impl ContextPage {
    /// 展开/折叠
    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    /// 渲染后的有效内容（折叠时用摘要）
    pub fn effective(&self) -> &str {
        if self.expanded {
            &self.content
        } else if let Some(summary) = &self.summary {
            summary
        } else {
            &self.content
        }
    }
}

/// 分页引擎
pub struct PagingEngine {
    options: PagingOptions,
}

impl PagingEngine {
    pub fn new(options: PagingOptions) -> Self {
        Self { options }
    }

    /// 将消息历史切分为语义页
    ///
    /// 规则：
    /// - 单条消息超限 → 独立页，自动折叠为摘要
    /// - 相邻小消息 → 合并为块，按语义边界分隔
    pub fn paginate(&self, messages: &[Message]) -> Vec<ContextPage> {
        let mut pages: Vec<ContextPage> = Vec::new();
        let mut buffer = String::new();
        let mut buffer_start = 0usize;

        for (idx, msg) in messages.iter().enumerate() {
            let text = render_message(msg);
            // 单条消息超限 → 立即独立分页
            if text.len() > self.options.max_chars {
                self.flush(&mut pages, &mut buffer, buffer_start, idx, &text);
                pages.push(self.summary_page(pages.len(), idx, &text));
                buffer_start = idx + 1;
                continue;
            }
            // 语义边界：角色变化 → 强制分页
            if self.options.semantic_split && !buffer.is_empty() {
                if let Some(prev) = messages.get(idx - 1) {
                    if prev.role() != msg.role() {
                        self.flush(&mut pages, &mut buffer, buffer_start, idx - 1, "");
                        buffer_start = idx;
                    }
                }
            }
            buffer.push_str(&text);
            buffer.push('\n');
            // 缓冲超限 → 分页
            if buffer.len() >= self.options.max_chars {
                self.flush(&mut pages, &mut buffer, buffer_start, idx, "");
                buffer_start = idx + 1;
            }
        }
        if !buffer.is_empty() {
            self.flush(
                &mut pages,
                &mut buffer,
                buffer_start,
                messages.len().saturating_sub(1),
                "",
            );
        }
        if pages.is_empty() {
            pages.push(ContextPage {
                id: 0,
                kind: PageKind::Content,
                content: String::new(),
                summary: None,
                title: None,
                expanded: true,
            });
        }
        pages
    }

    /// 汇总全部页的 token 估算
    pub fn total_chars(&self, pages: &[ContextPage]) -> usize {
        pages.iter().map(|p| p.effective().len()).sum()
    }

    /// 折叠超过阈值的页（预算压缩入口）
    pub fn collapse_excess(&self, pages: &mut [ContextPage], max_total: usize) {
        let mut total = self.total_chars(pages);
        // 优先折叠大页
        pages.sort_by_key(|p| std::cmp::Reverse(p.content.len()));
        for page in pages.iter_mut().filter(|p| p.expanded) {
            if total <= max_total {
                break;
            }
            if page.summary.is_some() {
                page.expanded = false;
                total =
                    total.saturating_sub(page.content.len() - page.summary.as_ref().unwrap().len());
            }
        }
        pages.sort_by_key(|p| p.id);
    }

    /// 生成摘要页（简单截断，完整摘要由 LLM 异步补充）
    fn summary_page(&self, id: usize, msg_idx: usize, text: &str) -> ContextPage {
        let summary = Self::local_summary(text);
        ContextPage {
            id,
            kind: PageKind::Summary,
            content: text.to_string(),
            summary: Some(summary),
            title: Some(format!("msg[{msg_idx}]")),
            expanded: false,
        }
    }

    /// 本地摘要：保留开头与结尾
    fn local_summary(text: &str) -> String {
        let limit = 200;
        if text.len() <= limit * 2 {
            return text.to_string();
        }
        let head: String = text.chars().take(limit).collect();
        let tail: String = text
            .chars()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!(
            "{head}\n... [已折叠，共 {} 字符] ...\n{tail}",
            text.chars().count()
        )
    }

    /// 冲刷缓冲为内容页
    #[allow(clippy::too_many_arguments)]
    fn flush(
        &self,
        pages: &mut Vec<ContextPage>,
        buffer: &mut String,
        start_idx: usize,
        end_idx: usize,
        extra: &str,
    ) {
        if !buffer.is_empty() {
            pages.push(ContextPage {
                id: pages.len(),
                kind: PageKind::Content,
                content: std::mem::take(buffer),
                summary: None,
                title: Some(format!("msgs[{start_idx}..={end_idx}]")),
                expanded: true,
            });
        }
        if !extra.is_empty() {
            buffer.push_str(extra);
        }
    }
}

/// 将消息渲染为文本
fn render_message(msg: &Message) -> String {
    let role = msg.role();
    match msg {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::System { content, .. } => format!("[{role}] {content}"),
        Message::Tool {
            name,
            input,
            output,
            ..
        } => {
            format!("[tool:{name}] in: {input}\nout: {output}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn user(content: &str) -> Message {
        Message::User {
            content: content.to_string(),
            timestamp: Utc::now(),
        }
    }

    fn assistant(content: &str) -> Message {
        Message::Assistant {
            content: content.to_string(),
            timestamp: Utc::now(),
            thinking: None,
        }
    }

    #[test]
    fn test_paginate_empty() {
        let engine = PagingEngine::new(PagingOptions::default());
        let pages = engine.paginate(&[]);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn test_paginate_small_messages() {
        let engine = PagingEngine::new(PagingOptions::default());
        let messages = vec![user("hello"), assistant("world")];
        let pages = engine.paginate(&messages);
        // 角色变化 → 两页
        assert!(pages.len() >= 2);
        assert!(pages.iter().all(|p| p.kind == PageKind::Content));
    }

    #[test]
    fn test_paginate_large_message_summary() {
        let engine = PagingEngine::new(PagingOptions::default());
        let big = "x".repeat(10_000);
        let messages = vec![user(&big)];
        let pages = engine.paginate(&messages);
        assert!(pages.iter().any(|p| p.kind == PageKind::Summary));
        assert!(pages.iter().any(|p| !p.expanded));
    }

    #[test]
    fn test_collapse_excess() {
        let engine = PagingEngine::new(PagingOptions::default());
        // 构造带摘要的大内容页（模拟超长消息折叠后的状态）
        let mut pages = vec![
            ContextPage {
                id: 0,
                kind: PageKind::Content,
                content: "a".repeat(3_000),
                summary: Some("summary-a".to_string()),
                title: Some("big".to_string()),
                expanded: true,
            },
            ContextPage {
                id: 1,
                kind: PageKind::Content,
                content: "b".repeat(3_000),
                summary: Some("summary-b".to_string()),
                title: Some("big2".to_string()),
                expanded: true,
            },
        ];
        engine.collapse_excess(&mut pages, 1_000);
        // 预算压缩后内容量显著下降
        assert!(engine.total_chars(&pages) < 3_000);
        assert!(pages.iter().any(|p| !p.expanded));
    }

    #[test]
    fn test_toggle_page() {
        let mut page = ContextPage {
            id: 0,
            kind: PageKind::Summary,
            content: "full".to_string(),
            summary: Some("sum".to_string()),
            title: None,
            expanded: false,
        };
        assert_eq!(page.effective(), "sum");
        page.toggle();
        assert!(page.expanded);
        assert_eq!(page.effective(), "full");
    }
}
