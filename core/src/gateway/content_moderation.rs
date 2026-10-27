//! P2·C ContentModeration：内容安全审核层（合规分类 + 高危产出拦截审批）
//!
//! 定位：在 `gateway::security_seam`（Policy / ApprovalGate / AuditStore）之上的
//! **内容侧**审核——与能力侧 `capability_scan::SkillScan` 并列，管「产出的内容
//! 是否合规」，而不是「能力能干什么」。
//!
//! 职责：
//! - 输入/输出统一走 [`ContentModeration::classify`] 做合规标记（分类器，纯静态、
//!   零外部依赖、确定性、可单测）；
//! - 高危产出（泄露凭证、违规内容）默认拦截（fail-closed），并发起
//!   [`ApprovalGate`] 审批（`risk = High`），用户批准后经
//!   [`ContentModeration::release`] 放行；
//! - **令牌/凭证永不写入审计**：审核结果只携带脱敏 flag 与规则 ID，不复制原文；
//!   审批门参数也只写入分类标记（`args` 不含敏感片段原文）。
//!
//! 与 `guard::trust` 信任库联动：已签发（trusted）的内容通道可在审批时自动放行
//! （`auto_release_allowed`），其余一律等待人工审批。

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::guard::gateway::{ApprovalGate, ApprovalOrigin, RiskLevel};

/// 高危内容被阻断时的机器可读错误码
pub const CONTENT_MODERATION_BLOCKED: &str = "CONTENT_MODERATION_BLOCKED";
/// 内容仍等待审批
pub const CONTENT_MODERATION_PENDING: &str = "CONTENT_MODERATION_PENDING";

/// 合规标记（脱敏：只带 flag 名，不携带原文）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentFlag {
    /// 泄露凭证/密钥（sk-…、AKIA…、私钥块、明文密码对等）
    CredentialLeak,
    /// 暴力/威胁/极端违规内容
    ViolentContent,
    /// 恶意利用/钓鱼/社工等高危指令
    HarmfulInstruction,
}

impl ContentFlag {
    /// 规则 ID（供审计与前端展示，不含原文）
    pub fn rule(&self) -> &'static str {
        match self {
            ContentFlag::CredentialLeak => "content.credential-leak",
            ContentFlag::ViolentContent => "content.violent",
            ContentFlag::HarmfulInstruction => "content.harmful-instruction",
        }
    }
}

/// 内容审核结论
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationVerdict {
    /// 是否放行（fail-closed：命中高危 flag 且未批准 → false）
    pub allow: bool,
    /// 命中的合规标记（脱敏）
    pub flags: Vec<ContentFlag>,
    /// 综合风险等级
    pub risk: RiskLevel,
    /// 高危产出阻塞时的审批 ID（用户批准后可用 [`ContentModeration::release`] 放行）
    pub approval_id: Option<String>,
    /// 人类可读判定理由（脱敏）
    pub reason: String,
}

impl ModerationVerdict {
    pub fn is_allowed(&self) -> bool {
        self.allow
    }

    pub fn code(&self) -> &'static str {
        if self.allow {
            "OK"
        } else if self.approval_id.is_some() {
            CONTENT_MODERATION_PENDING
        } else {
            CONTENT_MODERATION_BLOCKED
        }
    }
}

/// 内容安全审核器
#[derive(Debug, Clone, Copy, Default)]
pub struct ContentModeration;

impl ContentModeration {
    pub fn new() -> Self {
        Self
    }

    /// 分类器：对输入/输出做合规标记，**不隐式放行**（调用方据此走不同通道）。
    pub fn classify(&self, text: &str) -> Vec<ContentFlag> {
        let mut flags = Vec::new();
        if detect_credential_leak(text) {
            flags.push(ContentFlag::CredentialLeak);
        }
        if detect_violent(text) {
            flags.push(ContentFlag::ViolentContent);
        }
        if detect_harmful_instruction(text) {
            flags.push(ContentFlag::HarmfulInstruction);
        }
        flags
    }

    /// 输入侧审核：标记问题，但输入一般不阻断（可配 `block_input` 严格模式）。
    pub fn review_input(&self, text: &str, block_input: bool) -> ModerationVerdict {
        let flags = self.classify(text);
        let risk = risk_of(&flags);
        let allow = !(block_input && risk == RiskLevel::High);
        ModerationVerdict {
            allow,
            flags: flags.clone(),
            risk,
            approval_id: None,
            reason: if flags.is_empty() {
                "输入内容合规".to_string()
            } else {
                format!(
                    "输入命中合规标记 {} 项（脱敏规则：{}）",
                    flags.len(),
                    flags
                        .iter()
                        .map(|f| f.rule())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        }
    }

    /// 输出侧审核：高危产出默认**拦截**并发起审批门（fail-closed）。
    ///
    /// 返回：
    /// - `allow=true`：放行；
    /// - `allow=false` 且 `approval_id=Some(id)`：已进入审批，由用户经 `release` 放行；
    /// - `allow=false` 且 `approval_id=None`：非交互或审批门忙，直接阻断。
    pub fn review_output(&self, gate: &ApprovalGate, text: &str) -> ModerationVerdict {
        let flags = self.classify(text);
        if flags.is_empty() {
            return ModerationVerdict {
                allow: true,
                flags,
                risk: RiskLevel::Low,
                approval_id: None,
                reason: "输出内容合规".to_string(),
            };
        }

        let risk = risk_of(&flags);
        let need_approval = risk == RiskLevel::High;
        if need_approval {
            // 非交互路径 fail-closed：不创建审批、直接阻断（与 approval_gate 同口径）
            if !gate.is_interactive() {
                return ModerationVerdict {
                    allow: false,
                    flags: flags.clone(),
                    risk,
                    approval_id: None,
                    reason: "输出命中高危合规标记且当前非交互，已直接阻断".to_string(),
                };
            }
            // 审批门 args 只写脱敏规则 ID，绝不携带凭证/违规原文
            let mask = flags
                .iter()
                .map(|f| f.rule())
                .collect::<Vec<_>>()
                .join(",");
            let approval = gate.request_with_origin(
                "content.release",
                format!("{{\"flags\":\"{mask}\"}}"),
                RiskLevel::High,
                ApprovalOrigin::Inferred,
            );
            match approval {
                Some(p) => ModerationVerdict {
                    allow: false,
                    flags: flags.clone(),
                    risk,
                    approval_id: Some(p.id),
                    reason: format!(
                        "输出命中高危合规标记 {} 项（{}），已进入审批门等待人工裁决",
                        flags.len(),
                        mask
                    ),
                },
                None => ModerationVerdict {
                    allow: false,
                    flags: flags.clone(),
                    risk,
                    approval_id: None,
                    reason: "输出命中高危合规标记且审批门忙（已有挂起审批），已阻断".to_string(),
                },
            }
        } else {
            // 中低危（如疑似违规词语但未达高危）仅标记，放行但记录
            ModerationVerdict {
                allow: true,
                flags: flags.clone(),
                risk,
                approval_id: None,
                reason: format!(
                    "输出命中合规标记 {} 项（{}），已记录",
                    flags.len(),
                    flags
                        .iter()
                        .map(|f| f.rule())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    }

    /// 用户批准后放行高危内容（消费审批门的结果）。
    ///
    /// 返回 `true` 表示审批通过、内容可正式对外；`false` 表示驳回/超时/ID 不匹配，
    /// 内容仍应视为被拦截（fail-closed）。
    pub fn release(&self, gate: &ApprovalGate, approval_id: &str) -> bool {
        gate.approve(approval_id)
    }

    /// 与 `guard::trust` 信任库联动：高危内容通道该「工具 × 模式 × 会话」下是否已签发，
    /// 已签发可自动放行（不经人工审批，但内容仍需分类标记留痕）。
    pub fn auto_release_allowed(&self, tool: &str, mode: &str, session: &str) -> bool {
        let store = crate::guard::trust::global();
        store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_trusted(tool, mode, session)
            .is_some()
    }
}

fn risk_of(flags: &[ContentFlag]) -> RiskLevel {
    if flags.is_empty() {
        RiskLevel::Low
    } else if flags.contains(&ContentFlag::CredentialLeak) {
        RiskLevel::High
    } else {
        RiskLevel::Medium
    }
}

// ---------------------------------------------------------------------------
// 静态检测器（确定性、零外部依赖；全部脱敏，不返回原文）
// ---------------------------------------------------------------------------

/// 凭证/密钥泄露检测：常见密钥前缀 + 键名后跟疑似值 + 私钥块 + 明文口令对
fn detect_credential_leak(text: &str) -> bool {
    // 常见密钥前缀（长度足够形成真实泄露判断）
    let prefixes: &[&str] = &[
        "sk-",
        "sk_",
        "AKIA",           // AWS Access Key
        "ghp_",           // GitHub PAT
        "gho_",
        "xoxb-",          // Slack Bot
        "xoxp-",
        "AIza",           // Google API
        "-----BEGIN",
    ];
    for line in text.lines() {
        let trimmed = line.trim();
        // 行内嵌密钥（如 `token: sk-...`、`Bearer sk-...`）：正则精确匹配前缀+长度
        if credential_re().is_match(line) {
            return true;
        }
        for p in prefixes {
            if trimmed.starts_with(p) && trimmed.len() >= p.len() + 12 {
                return true;
            }
        }
        // BEGIN 私钥块（PEM）
        if line.contains("PRIVATE KEY") {
            return true;
        }
        // 键值形式：key = value（值像 token）
        if let Some(eq) = trimmed.find('=') {
            let (k, v) = trimmed.split_at(eq);
            let key = k.trim().to_ascii_lowercase();
            let val = v[1..].trim().trim_matches(['"', '\'']);
            let looks_secret_key = key.contains("api_key")
                || key.contains("apikey")
                || key.contains("api-key")
                || key.contains("secret")
                || key.contains("token")
                || key.contains("credential")
                || key.contains("password")
                || key.contains("passwd");
            if looks_secret_key && val.len() >= 12 && !val.is_empty() {
                return true;
            }
        }
    }
    false
}

/// 常见密钥形态的精确正则（避免把 `sk-` 短字符串误报为泄露）
fn credential_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:sk-[A-Za-z0-9_-]{16,}|sk_[A-Za-z0-9_-]{16,}|AKIA[0-9A-Z]{16}\b|ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{30,})\b",
        )
        .expect("static credential regex should compile")
    })
}

/// 暴力/威胁/极端违规内容
fn detect_violent(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const SIGNS: &[&str] = &[
        "杀了他",
        "杀光",
        "爆炸物制作",
        "制造炸弹",
        "人体炸弹",
        "砍头",
        "强暴",
        "轮奸",
        "贩卖人口",
        "拐卖儿童",
        "自杀教程",
        "自残教程",
        "恐怖袭击",
        "圣战",
        "white power",
        "rape ",
        "kill them all",
    ];
    SIGNS.iter().any(|s| lower.contains(s))
}

/// 恶意利用/钓鱼/社工等高危指令
fn detect_harmful_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const SIGNS: &[&str] = &[
        "sql注入",
        "sql injection",
        "xss payload",
        "反弹shell",
        "reverse shell",
        "钓鱼邮件模板",
        "phishing email",
        "社工库",
        "拖库",
        "撞库",
        "提权脚本",
        "勒索软件",
        "ransomware",
        "挖矿程序",
        "免杀木马",
        "盗号木马",
        "诈骗话术",
        "冒充公检法",
        "裸聊诈骗",
        "杀猪盘",
    ];
    SIGNS.iter().any(|s| lower.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn credential_leak_output_is_blocked_with_approval() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        // 输出内容里出现 OpenAI 风格密钥
        let text = format!("found key:\nsk-{}\nplease use it", "A1B2C3D4E5F6G7H8I9J0K1L2M3");
        let v = modr.review_output(&gate, &text);
        assert!(!v.allow, "泄露凭证的内容应被拦截");
        assert!(
            v.flags.contains(&ContentFlag::CredentialLeak),
            "应命中 credential-leak 标记"
        );
        assert_eq!(v.risk, RiskLevel::High);
        assert!(v.approval_id.is_some(), "高危产出应进入审批门");
        // 审批放行
        assert!(modr.release(&gate, v.approval_id.as_deref().unwrap()));
        // 未审批时（新 gate + 非交互）直接阻断
        let gate2 = ApprovalGate::new(Duration::from_secs(60));
        gate2.set_interactive(false);
        let v2 = modr.review_output(&gate2, &text);
        assert!(!v2.allow);
        assert!(v2.approval_id.is_none(), "非交互应 fail-closed 直接阻断");
    }

    #[test]
    fn benign_text_is_allowed() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        let v = modr.review_output(&gate, "今天天气很好，去公园散步吧。");
        assert!(v.allow);
        assert!(v.flags.is_empty());
        assert_eq!(v.risk, RiskLevel::Low);
    }

    #[test]
    fn api_key_assign_output_is_blocked() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        let text = "config:\n  api_key = \"KjH8d2mN4qPw7xZb3sRgTy9cVfE\"";
        let v = modr.review_output(&gate, text);
        assert!(!v.allow);
        assert!(v.flags.contains(&ContentFlag::CredentialLeak));
    }

    #[test]
    fn pem_private_key_blocked() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
        let v = modr.review_output(&gate, text);
        assert!(!v.allow);
        assert!(v.flags.contains(&ContentFlag::CredentialLeak));
    }

    #[test]
    fn violent_content_flags_but_violent_not_high_alone() {
        // 违规内容默认中风险，标记但放行；若与凭证叠加才升高危（见 risk_of 规则）
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        let v = modr.review_output(&gate, "这本书描述了某起恐怖袭击的调查过程。");
        assert!(v.allow, "含敏感词的正常语境内容只标记不误拦");
        assert!(v.flags.contains(&ContentFlag::ViolentContent));
        assert_eq!(v.risk, RiskLevel::Medium);
    }

    #[test]
    fn harmful_instruction_marked_not_silent() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        let v = modr.review_output(&gate, "以下内容不包含任何钓鱼邮件模板，仅用于教学对比。");
        assert!(v.flags.contains(&ContentFlag::HarmfulInstruction));
        assert!(v.allow, "中风险：标记但记录，不误拦学习对比语境");
    }

    #[test]
    fn verdict_never_contains_raw_secret() {
        let gate = ApprovalGate::new(Duration::from_secs(60));
        let modr = ContentModeration::new();
        // 夹具必须自我标注（`sk-fake-…`）：scripts/check-secrets.py 只放行带 fake/test 等
        // 标记的占位串，未标注的 35 位高熵串会被判成"真钥匙形态"而让发布门禁变红。
        let secret = "sk-fake-A1B2C3D4E5F6G7H8I9J0K1L2M3";
        let text = format!("token: {secret}");
        let v = modr.review_output(&gate, &text);
        assert!(!v.allow);
        // 结论与审批门参数都不应包含密钥原文
        assert!(!v.reason.contains(secret), "reason 不得含令牌原文");
        let gate_pending = gate.pending();
        if let Some(p) = gate_pending {
            assert!(!p.args.contains(secret), "审批门 args 不得含令牌原文");
        }
        // 序列化结论也不含原文
        let wire = serde_json::to_string(&v).unwrap();
        assert!(!wire.contains(secret), "序列化结论不得含令牌原文");
    }

    #[test]
    fn input_review_marks_but_does_not_block_by_default() {
        let modr = ContentModeration::new();
        let text = "帮我阅读这个 api_key = \"AbCdEfGhIjKlMnOpQrStUvWxYz\"";
        let v = modr.review_input(text, false);
        assert!(v.allow, "输入侧默认只标记不拦截");
        assert!(v.flags.contains(&ContentFlag::CredentialLeak));
        // 严格模式才拦截
        let v2 = modr.review_input(text, true);
        assert!(!v2.allow);
    }

    #[test]
    fn trust_link_allows_auto_release_when_signed() {
        let store = crate::guard::trust::global();
        store.lock().unwrap_or_else(|e| e.into_inner()).revoke_all();
        let modr = ContentModeration::new();
        assert!(!modr.auto_release_allowed("channel:content", "omni", "s1"));
        {
            let mut g = store.lock().unwrap_or_else(|e| e.into_inner());
            g.grant(crate::guard::trust::TrustLevel::High, "channel:content", Some("omni".to_string()), "s1");
        }
        assert!(modr.auto_release_allowed("channel:content", "omni", "s1"));
        // 未签发工具仍被拒
        assert!(!modr.auto_release_allowed("channel:other", "omni", "s1"));
        store.lock().unwrap_or_else(|e| e.into_inner()).revoke_all();
    }
}