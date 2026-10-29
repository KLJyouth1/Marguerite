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

/// 提问的前置依赖（分支表达的最小扩展）：
/// 问题只在 `question` 这条 gate 被回答、且（若给了 `answer`）所选**选项标签**命中后才就绪。
///
/// 为什么用选项标签而不是布尔/枚举：分支语义的真实键就是用户点选的那一项
/// （如「操控电脑 / 脚本生成」），而自定义回答（allow_other 文本）**永不匹配标签** ——
/// 「其他」分支据此自动跳过只属于特定分支的追问。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionDependency {
    pub question: String,
    /// `Some(label)`：仅当该选项被选中才就绪；`None`：gate 已回答即就绪（任意答案，含自定义）。
    pub answer: Option<String>,
}

/// 单个提问
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
    /// 允许在选项之外自填（对齐澄清卡片的「其他」输入口）。
    pub allow_other: bool,
    /// 分支依赖；`None` = 无条件就绪。
    pub depends_on: Option<QuestionDependency>,
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

    /// 分支调度：先丢弃「gate 已回答但选了别的选项」的挂起问题（其分支永不走到，
    /// 不丢弃则队列永不排空 —— 例如「脚本生成」分支会卡死在「操控电脑」的追问上），
    /// 再返回当前就绪问题的克隆批次。
    ///
    /// 判定口径：gate 必须已在 `answered` 中；`dep.answer == None` 时任意答案
    /// （含自定义文本）即就绪；`Some(label)` 时仅当选中该选项标签才就绪 ——
    /// 自定义回答的 `selected` 恒不含标签，据此把「其他」分支与具体分支区分开。
    pub fn ready_batch(&mut self) -> Vec<Question> {
        let answered = &self.answered;
        self.pending.retain(|q| match &q.depends_on {
            Some(dep) => match dep.answer.as_ref() {
                Some(label) => match answered.get(&dep.question) {
                    Some(ans) => ans.selected.iter().any(|s| s == label),
                    None => true,
                },
                None => true,
            },
            None => true,
        });
        self.pending
            .iter()
            .filter(|q| match &q.depends_on {
                None => true,
                Some(dep) => answered.get(&dep.question).is_some_and(|ans| {
                    dep.answer
                        .as_ref()
                        .is_none_or(|label| ans.selected.iter().any(|s| s == label))
                }),
            })
            .cloned()
            .collect()
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
            allow_other: false,
            depends_on: None,
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
            allow_other: false,
            depends_on: None,
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

    /// 分支问卷的最小复刻（deck_craft 开工问卷的形状）：
    /// form 是 gate；software 仅「操控电脑」分支追问；outline 任何分支都要确认。
    fn branch_fixture() -> QuestionQueue {
        let mut q = QuestionQueue::new();
        q.ask_many(vec![
            Question {
                id: "form".into(),
                header: None,
                question: "这份 PPT 希望怎样生成？".into(),
                options: vec![opt("操控电脑"), opt("脚本生成")],
                multi_select: false,
                allow_other: true,
                depends_on: None,
            },
            Question {
                id: "software".into(),
                header: None,
                question: "操控哪个软件？".into(),
                options: vec![opt("WPS")],
                multi_select: false,
                allow_other: true,
                depends_on: Some(QuestionDependency {
                    question: "form".into(),
                    answer: Some("操控电脑".into()),
                }),
            },
            Question {
                id: "outline".into(),
                header: None,
                question: "大纲如何确认？".into(),
                options: vec![opt("按需求中的大纲")],
                multi_select: false,
                allow_other: true,
                depends_on: Some(QuestionDependency {
                    question: "form".into(),
                    answer: None,
                }),
            },
        ]);
        q
    }

    #[test]
    fn ready_batch_only_ungated_questions_before_gate_answered() {
        let mut q = branch_fixture();
        let batch = q.ready_batch();
        assert_eq!(
            batch.iter().map(|it| it.id.as_str()).collect::<Vec<_>>(),
            vec!["form"],
            "gate 未回答前，带依赖的问题不得进入批次"
        );
    }

    #[test]
    fn branch_answer_unlocks_its_followups() {
        let mut q = branch_fixture();
        q.answer(Answer {
            id: "form".into(),
            selected: vec!["操控电脑".into()],
            custom: None,
        });
        let batch = q.ready_batch();
        assert!(batch.iter().any(|it| it.id == "software"), "A 分支应追问软件");
        assert!(batch.iter().any(|it| it.id == "outline"), "两分支都要确认大纲");
        // 队列不因 ready 展示而消费 pending：回答后才移除
        assert_eq!(q.pending_count(), 2);
        q.answer(Answer {
            id: "software".into(),
            selected: vec!["WPS".into()],
            custom: None,
        });
        q.answer(Answer {
            id: "outline".into(),
            selected: vec![],
            custom: Some("我自己贴大纲".into()),
        });
        assert!(q.is_empty());
        assert!(q.ready_batch().is_empty());
    }

    #[test]
    fn other_branch_discards_unreachable_followups() {
        // 选「脚本生成」：software 的分支永不走到，ready_batch 必须把它丢弃，
        // 否则队列永不排空（这就是当年脚本分支会卡死的形态）。
        let mut q = branch_fixture();
        q.answer(Answer {
            id: "form".into(),
            selected: vec!["脚本生成".into()],
            custom: None,
        });
        let batch = q.ready_batch();
        assert_eq!(batch.iter().map(|it| it.id.as_str()).collect::<Vec<_>>(), vec!["outline"]);
        assert!(q.answer_of("software").is_none(), "丢弃 ≠ 回答：不能伪造一条回答记录");
        q.answer(Answer {
            id: "outline".into(),
            selected: vec!["按需求中的大纲".into()],
            custom: None,
        });
        assert!(q.is_empty(), "不可达追问被丢弃后队列应能排空");
    }

    #[test]
    fn custom_answer_never_matches_option_gated_followups() {
        // C 分支（自填「其他」）：selected 为空，只匹配 answer=None 的依赖。
        let mut q = branch_fixture();
        q.answer(Answer {
            id: "form".into(),
            selected: vec![],
            custom: Some("用在线协作工具做".into()),
        });
        let batch = q.ready_batch();
        assert_eq!(batch.iter().map(|it| it.id.as_str()).collect::<Vec<_>>(), vec!["outline"]);
    }
}
