//! ask_user 提问模型（上游 ask-user 二开落地，Track 3）。
//!
//! 纯逻辑、离线可测的交互提问队列：
//! - 一次可携带多个问题（每个有稳定 id），等待回答后按 id 回填；
//! - 支持单选/多选与自定义回答；
//! - 无答案时 fail-closed（回答端未就绪则 pending 等待，而非静默吞掉）。

use std::collections::HashMap;

/// 单个提问选项
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

/// 单个提问
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

impl Question {
    /// 校验：id 与 question 非空。
    pub fn validate(&self) -> bool {
        !self.id.trim().is_empty() && !self.question.trim().is_empty()
    }
}

/// 一次提问的回答
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub id: String,
    pub selected: Vec<String>,
    pub custom: Option<String>,
}

/// 提问队列（离线可测；真实 UI 层消费 pending，回答经 answer 回填）
#[derive(Debug, Default)]
pub struct QuestionQueue {
    pending: Vec<Question>,
    answered: HashMap<String, Answer>,
}

impl QuestionQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 提交一批提问；返回成功入队的提问 id。
    pub fn ask_many(&mut self, questions: Vec<Question>) -> Vec<String> {
        let mut ids = Vec::new();
        for q in questions {
            if q.validate() {
                let id = q.id.clone();
                ids.push(id.clone());
                self.pending.push(q);
            }
        }
        ids
    }

    /// 回填某个问题的回答；返回是否命中并移除 pending。
    pub fn answer(&mut self, answer: Answer) -> bool {
        if answer.id.trim().is_empty() {
            return false;
        }
        let hit = self.pending.iter().any(|q| q.id == answer.id);
        if hit {
            self.pending.retain(|q| q.id != answer.id);
            self.answered.insert(answer.id.clone(), answer);
        }
        hit
    }

    /// 读取某问题回答。
    pub fn answer_of(&self, id: &str) -> Option<&Answer> {
        self.answered.get(id)
    }

    pub fn pending(&self) -> &[Question] {
        &self.pending
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(label: &str) -> QuestionOption {
        QuestionOption {
            label: label.to_string(),
            description: None,
        }
    }

    #[test]
    fn ask_and_answer_roundtrip() {
        let mut q = QuestionQueue::new();
        let ids = q.ask_many(vec![Question {
            id: "confirm".into(),
            header: Some("Confirm".into()),
            question: "继续吗？".into(),
            options: vec![opt("是"), opt("否")],
            multi_select: false,
        }]);
        assert_eq!(ids, vec!["confirm".to_string()]);
        assert_eq!(q.pending_count(), 1);
        assert!(q.answer(Answer {
            id: "confirm".into(),
            selected: vec!["是".into()],
            custom: None,
        }));
        assert_eq!(q.pending_count(), 0);
        assert_eq!(q.answer_of("confirm").unwrap().selected, vec!["是"]);
    }

    #[test]
    fn invalid_question_rejected() {
        let mut q = QuestionQueue::new();
        let ids = q.ask_many(vec![Question {
            id: "  ".into(),
            header: None,
            question: "".into(),
            options: vec![],
            multi_select: false,
        }]);
        assert!(ids.is_empty());
        assert!(q.is_empty());
    }

    #[test]
    fn answer_unknown_id_returns_false() {
        let mut q = QuestionQueue::new();
        assert!(!q.answer(Answer {
            id: "ghost".into(),
            selected: vec![],
            custom: None,
        }));
    }
}
