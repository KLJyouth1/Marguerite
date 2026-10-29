//! P2·D 精细化权限管理：3×3 权限矩阵（越智能越受控越能干）
//!
//! 在既有基座之上（`security_seam` 的 Policy / ApprovalGate / AuditStore、
//! `guard::trust` 信任库、`mode` 的 3×3 用户模式与运行时档位）新增**权限矩阵**
//! 配置层：为「3×3 档位 × 工具自主档位」判定放行 / 确认 / 拒绝，并提供审计强度。
//!
//! 联动语义（对齐计划）：
//! - `AgentMode` / skill / tool / mcp 各自声明 capability 与 trust tier，这里按
//!   工具**自主档位**（[`ToolAutonomy`]）归行；
//! - **极简档位默认锁高自主工具**：Minimal·DoIt 对高自主工具默认 Deny，显式授权
//!   （`guard::trust` 签发）后才放行；
//! - **极致·执行档位在用户显式授权后放开**：Standard/Extreme·DoIt 对高自主工具
//!   默认 Confirm（有宿主人工确认），显式授权后 Allow；
//! - 咨询（Ask）一律只读：写 / 高自主全部拒绝；
//! - 方案（Plan）默认只读规划：写需确认、高自主拒绝（与 `mode::runtime_denies_tool`
//!   Plan 只读纪律同口径）。
//!
//! 审计强度随档位与授权变化：只读 none → 写（执行档）standard → 高自主授权后 full。
//! 判定是纯函数（`trusted: bool` 由调用方注入），可单测、可复用；不新建权限系统。

use serde::{Deserialize, Serialize};

use crate::domain::{ModeIntensity, UserMode, WorkingMode};

/// 工具自主档位（权限矩阵的行）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolAutonomy {
    /// 只读工具（查询 / 读文件 / 搜索）：任何档位自动放行
    ReadOnly,
    /// 写工具（修改工作区 / 写代码 / 写文件）：咨询拦截、方案需确认、执行档放行
    Mutating,
    /// 高自主工具（执行 / 出网 / 桌面控制 / 安装 / 外部写）：极简默认锁、高档位显式授权放开
    HighAutonomy,
}

impl ToolAutonomy {
    /// 人类可读档位名
    pub fn label(&self) -> &'static str {
        match self {
            ToolAutonomy::ReadOnly => "只读",
            ToolAutonomy::Mutating => "写",
            ToolAutonomy::HighAutonomy => "高自主",
        }
    }

    /// 机器可读键（`readonly` / `mutating` / `high_autonomy`）。
    ///
    /// 与 [`Self::label`] 分开：label 给人看、可随文案调整；键要稳定，供前端按
    /// 档案着色与排序。此前只有中文 label，前端想按档位配色只能匹配中文字符串。
    pub fn as_key(&self) -> &'static str {
        match self {
            ToolAutonomy::ReadOnly => "readonly",
            ToolAutonomy::Mutating => "mutating",
            ToolAutonomy::HighAutonomy => "high_autonomy",
        }
    }
}

/// 权限矩阵判定结论
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "lowercase")]
pub enum MatrixDecision {
    /// 放行
    Allow {
        /// 审计强度：none / standard / full
        audit: AuditStrength,
    },
    /// 需用户确认（有宿主时交互审批；无宿主 fail-closed）
    Confirm { reason: String },
    /// 拒绝
    Deny { reason: String },
}

impl MatrixDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, MatrixDecision::Allow { .. })
    }
}

/// 审计强度（能力↑ → 审计↑）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditStrength {
    /// 只读：无强制审计
    None,
    /// 常规写：记录工具与结果
    Standard,
    /// 高自主：全量审计（含参数脱敏快照）
    Full,
}

/// 3×3 权限矩阵（核心判定，纯函数）。
#[derive(Debug, Clone, Copy, Default)]
pub struct PermissionMatrix;

impl PermissionMatrix {
    pub fn new() -> Self {
        Self
    }

    /// 判定：3×3 档位 × 工具自主档位。
    ///
    /// `trusted` 表示该「工具 × 模式 × 会话」是否已被 `guard::trust` 显式授权。
    pub fn decide(
        &self,
        intensity: ModeIntensity,
        working: WorkingMode,
        autonomy: ToolAutonomy,
        trusted: bool,
    ) -> MatrixDecision {
        match working {
            // 咨询 Ask：一律只读
            WorkingMode::Ask => match autonomy {
                ToolAutonomy::ReadOnly => MatrixDecision::Allow {
                    audit: AuditStrength::None,
                },
                ToolAutonomy::Mutating => MatrixDecision::Deny {
                    reason: "咨询档位只读，不允许修改类工具".to_string(),
                },
                ToolAutonomy::HighAutonomy => MatrixDecision::Deny {
                    reason: "咨询档位只读，高自主工具一律拒绝".to_string(),
                },
            },
            // 方案 Plan：默认只读规划
            WorkingMode::Plan => match autonomy {
                ToolAutonomy::ReadOnly => MatrixDecision::Allow {
                    audit: AuditStrength::None,
                },
                ToolAutonomy::Mutating => MatrixDecision::Confirm {
                    reason: "方案档位默认只读规划，写操作需人工确认".to_string(),
                },
                ToolAutonomy::HighAutonomy => MatrixDecision::Deny {
                    reason: "方案档位默认只读规划，高自主工具拒绝（与 Plan 只读纪律同口径）"
                        .to_string(),
                },
            },
            // 执行 DoIt：按强度递进
            WorkingMode::DoIt => match autonomy {
                ToolAutonomy::ReadOnly => MatrixDecision::Allow {
                    audit: AuditStrength::None,
                },
                ToolAutonomy::Mutating => match intensity {
                    // 极简执行：写操作受控确认
                    ModeIntensity::Minimal => MatrixDecision::Confirm {
                        reason: "极简执行档写操作需人工确认".to_string(),
                    },
                    // 标准 / 极致执行：写操作放行，随强度提升审计
                    ModeIntensity::Standard => MatrixDecision::Allow {
                        audit: AuditStrength::Standard,
                    },
                    ModeIntensity::Extreme => MatrixDecision::Allow {
                        audit: AuditStrength::Standard,
                    },
                },
                ToolAutonomy::HighAutonomy => match intensity {
                    // 极简执行：默认锁高自主工具，显式授权才放行
                    ModeIntensity::Minimal => {
                        if trusted {
                            MatrixDecision::Allow {
                                audit: AuditStrength::Full,
                            }
                        } else {
                            MatrixDecision::Deny {
                                reason: "极简档位默认锁高自主工具，需显式授权后放行".to_string(),
                            }
                        }
                    }
                    // 标准 / 极致执行：无授权走人工确认（有宿主可裁），显式授权后放开
                    ModeIntensity::Standard | ModeIntensity::Extreme => {
                        if trusted {
                            MatrixDecision::Allow {
                                audit: AuditStrength::Full,
                            }
                        } else {
                            MatrixDecision::Confirm {
                                reason: "高自主工具需要显式授权或人工确认（能力和控制同步提升）"
                                    .to_string(),
                            }
                        }
                    }
                },
            },
        }
    }

    /// 便捷入口：3×3 UserMode 判定（trust 由调用方注入）。
    pub fn check(
        &self,
        user_mode: UserMode,
        autonomy: ToolAutonomy,
        trusted: bool,
    ) -> MatrixDecision {
        self.decide(user_mode.intensity(), user_mode.working(), autonomy, trusted)
    }
}

/// `guard.permission_matrix.status`：3×3 权限矩阵只读观测面（W-O3）。
///
/// # 为什么需要它
///
/// 本模块是**执行侧真实判据**（`gateway::orchestrate` 用它给子代理分类放行），
/// 但此前没有任何 RPC 入口：用户在界面上看不到「当前档位下，写类工具到底是放行、
/// 要确认、还是被拒」，只能撞到拒绝后才知道。本函数把**整张矩阵**（9 档位 ×
/// 3 自主档位 × 授权/未授权两态）一次性摊开，纯只读、无副作用。
///
/// 返回结构：
/// - `modes[]`：每个用户档位，含 `mode`（稳定 key）、`intensity`、`working`；
/// - `modes[].rows[]`：每个工具自主档位，含 `autonomy`（kebab key）与两态裁决
///   `untrusted` / `trusted`（直接复用 [`MatrixDecision`] 的 Serialize 形态，
///   前端无需另建一套枚举映射）；
/// - `auditLevels`：审计强度枚举，供前端渲染图例。
pub fn matrix_status() -> serde_json::Value {
    let m = PermissionMatrix::new();
    let modes: Vec<serde_json::Value> = UserMode::all()
        .iter()
        .map(|um| {
            let rows: Vec<serde_json::Value> = [
                ToolAutonomy::ReadOnly,
                ToolAutonomy::Mutating,
                ToolAutonomy::HighAutonomy,
            ]
            .iter()
            .map(|a| {
                serde_json::json!({
                    "autonomy": a.label(),
                    "autonomyKey": serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
                    "untrusted": serde_json::to_value(m.check(*um, *a, false))
                        .unwrap_or(serde_json::Value::Null),
                    "trusted": serde_json::to_value(m.check(*um, *a, true))
                        .unwrap_or(serde_json::Value::Null),
                })
            })
            .collect();
            serde_json::json!({
                "mode": um.as_key(),
                "intensity": um.intensity().as_key(),
                "working": um.working().as_key(),
                "rows": rows,
            })
        })
        .collect();
    serde_json::json!({
        "ok": true,
        "modes": modes,
        "auditLevels": ["none", "standard", "full"],
    })
}

/// 工具自主档位内置分类表（真实注册名 + 历史别名，与 `mode::classify_tool` 同一张表口径）。
///
/// 默认只读（安全）；写 / 高自主需显式声明。高自主 = 命令执行（写类）/ 出网 / 桌面控制
/// / 安装；写 = 文件与代码修改。
pub fn classify_tool_autonomy(tool: &str) -> ToolAutonomy {
    match tool {
        // 桌面 / 系统级控制（高自主）
        "computer_use" | "system_control" | "window_manage" | "clipboard"
        | "office_automation" | "control_desktop" | "browser_control" | "auto_record"
        | "auto_replay" => ToolAutonomy::HighAutonomy,
        // 出网（高自主）
        "web_fetch" | "web_search" | "web_research" | "http_request" | "research" | "vision"
        | "browser" | "browser_a11y" | "see" => ToolAutonomy::HighAutonomy,
        // 写类命令执行 + 安装（高自主）
        "shell_exec" | "code_exec" | "process_manage" | "automation" | "exec" | "terminal"
        | "install_dependency" | "install" | "npm_install" | "pip_install" => {
            ToolAutonomy::HighAutonomy
        }
        // 只读命令执行（只读）
        "run_tests" | "git_status" | "git_diff" | "git_log" | "diff" | "doctor" => {
            ToolAutonomy::ReadOnly
        }
        // 写操作（Mutating）
        "write_file" | "append_file" | "edit_file" | "write_code" | "create_file"
        | "delete_file" | "rename_file" | "copy_file" | "move_file" | "make_dir"
        | "apply_patch" | "rollback" | "git_commit" | "str_replace_editor" | "todo_write"
        | "memory_save" => ToolAutonomy::Mutating,
        // 其它默认只读（read_file / list_dir / grep / find_files / memory_search …）
        _ => ToolAutonomy::ReadOnly,
    }
}

/// 引擎侧**真实裁决**的结果（写进 ToolCall/ToolResult 事件的 `approval` 字段）。
///
/// `status` 只有三个诚实取值：
/// - `approved`：矩阵放行（按 `audit` 强度审计），工具会执行；
/// - `pending`：矩阵要求确认 —— 有宿主审批流时进入既有审批闸门；没有宿主时
///   fail-closed 拒绝执行（结果里写明原因）；
/// - `denied`：矩阵拒绝，工具**不执行**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolApproval {
    /// `approved` | `pending` | `denied`
    pub status: &'static str,
    /// 该工具的自主档位（与既有 `autonomy` 字段同源）
    pub autonomy: ToolAutonomy,
    /// 裁决理由（pending / denied 必有；approved 为 None）
    pub reason: Option<String>,
    /// 放行时的审计强度（仅 approved）
    pub audit: Option<AuditStrength>,
}

/// 引擎执行前会**真实拦截**的高自主工具集合（本机 / 系统级控制）。
///
/// 为什么不把权限矩阵里的全部 `HighAutonomy` 都纳入执行拦截：
/// [`classify_tool_autonomy`] 把「出网检索」也算高自主，而 `mode::classify_tool`
/// 把 Network 视为只读探索允许 —— 若严格按矩阵拦截，Ask / Plan 档的 `web_search`
/// 会被整体锁死，与「只读探索允许查资料」冲突。因此执行拦截面收敛为**直接驱动本机**
/// 的工具（与 `mode::is_desktop_class` 的 Desktop 类同口径）；其余高自主工具仍上报
/// `autonomy`，交由既有策略（tool_cycle 审批闸门 / 网络策略）裁决。
pub fn approval_surface(tool: &str) -> bool {
    matches!(
        tool,
        "computer_use"
            | "system_control"
            | "control_desktop"
            | "window_manage"
            | "clipboard"
            | "office_automation"
            | "auto_record"
            | "auto_replay"
            | "browser_control"
    )
}

/// 「标准 / 极致·执行」是否等于用户对高自主工具的显式授权。
///
/// 抽成纯函数是为了可单测、也可被设置页复用（前端要能解释「为什么现在能操作电脑」）。
pub fn mode_authorizes_high_autonomy(user_mode: UserMode, autonomy: ToolAutonomy) -> bool {
    autonomy == ToolAutonomy::HighAutonomy
        && matches!(user_mode.working(), WorkingMode::DoIt)
        && matches!(
            user_mode.intensity(),
            ModeIntensity::Standard | ModeIntensity::Extreme
        )
}

/// 对**审批面内**的工具做真实矩阵裁决；面外工具返回 `None`（不伪造字段）。
///
/// `trusted` 来自 `guard::trust` 的真实信任库（工具 × 模式 × 会话 × 参数）。
/// 模式取进程级 `mode::runtime_mode()` 反推的 3×3 档位，与对话页 `/mode` 同源。
///
/// `args` 是本次工具调用的参数（解析失败时传 `None`）：参数级信任条目只在参数
/// 指纹一致时命中，拿不到参数就不命中（fail-closed）。
pub fn decide_tool(
    tool: &str,
    session: &str,
    args: Option<&serde_json::Value>,
) -> Option<ToolApproval> {
    if !approval_surface(tool) {
        return None;
    }
    let mode = crate::mode::runtime_mode();
    let user_mode = crate::mode::default_user_mode_for(mode);
    let args_fp = args.map(crate::guard::trust::args_fingerprint);
    let trusted = crate::guard::trust::global()
        .lock()
        .map(|store| {
            store
                .is_trusted_for(tool, mode.as_str(), session, args_fp.as_deref())
                .is_some()
        })
        .unwrap_or(false);
    let autonomy = classify_tool_autonomy(tool);
    // 「标准 / 极致·执行」本身就是用户对该档位能力的**显式授权**：这两档的产品语义
    // 就是"让它真的动手"（界面文案：极客 + 执行）。此前这两档对高自主工具走 Confirm，
    // 而桌面主路径没有同步审批通道 → pending 被 fail-closed 成「审批待定，已拒绝执行」，
    // 于是用户选了极致·执行，模型却回答「我无法直接操控你的电脑」（真机复现）。
    // 这里把该组合视为已授权（审计提升到 Full）；Ask / Plan / Minimal 的行为保持不变，
    // 极简档仍然默认锁高自主工具，必须走信任库显式授权。
    let authorized_by_mode = mode_authorizes_high_autonomy(user_mode, autonomy);
    let decision = PermissionMatrix::new().check(user_mode, autonomy, trusted || authorized_by_mode);
    Some(match decision {
        MatrixDecision::Allow { audit } => ToolApproval {
            status: "approved",
            autonomy,
            reason: None,
            audit: Some(audit),
        },
        MatrixDecision::Confirm { reason } => ToolApproval {
            status: "pending",
            autonomy,
            reason: Some(reason),
            audit: None,
        },
        MatrixDecision::Deny { reason } => ToolApproval {
            status: "denied",
            autonomy,
            reason: Some(reason),
            audit: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::UserMode::*;

    fn d(m: UserMode, a: ToolAutonomy, t: bool) -> MatrixDecision {
        PermissionMatrix::new().check(m, a, t)
    }

    /// 标准 / 极致·执行 = 显式授权；其余组合不得被这条规则放行。
    ///
    /// 事故背景：这两档此前对高自主工具走 Confirm，而桌面主路径没有同步审批通道，
    /// pending 被 fail-closed —— 用户选了「极致·执行」，模型却回答「我无法直接操控
    /// 你的电脑」（真机截图）。这条规则就是那个故障的修复点。
    #[test]
    fn standard_and_extreme_execute_authorize_high_autonomy() {
        assert!(mode_authorizes_high_autonomy(
            StandardDoIt,
            ToolAutonomy::HighAutonomy
        ));
        assert!(mode_authorizes_high_autonomy(
            ExtremeDoIt,
            ToolAutonomy::HighAutonomy
        ));
        // 极简执行仍锁高自主；咨询 / 方案不放行；只读档位无关。
        assert!(!mode_authorizes_high_autonomy(
            MinimalDoIt,
            ToolAutonomy::HighAutonomy
        ));
        assert!(!mode_authorizes_high_autonomy(
            StandardAsk,
            ToolAutonomy::HighAutonomy
        ));
        assert!(!mode_authorizes_high_autonomy(
            ExtremePlan,
            ToolAutonomy::HighAutonomy
        ));
        assert!(!mode_authorizes_high_autonomy(
            ExtremeDoIt,
            ToolAutonomy::Mutating
        ));
        // 组合语义自洽：授权后矩阵必须真的给 Allow + 全量审计。
        assert_eq!(
            d(ExtremeDoIt, ToolAutonomy::HighAutonomy, true),
            MatrixDecision::Allow {
                audit: AuditStrength::Full
            }
        );
        // 未授权时仍是 Confirm（CLI 无宿主场景保持 fail-closed 的能力）。
        assert!(matches!(
            d(ExtremeDoIt, ToolAutonomy::HighAutonomy, false),
            MatrixDecision::Confirm { .. }
        ));
    }

    /// 契约守卫：`guard.permission_matrix.status` 必须摊开**整张**矩阵
    /// （9 档位 × 3 自主档位 × 授权两态），且每个裁决都带 `verdict` 判别字段。
    #[test]
    fn matrix_status_exposes_full_grid() {
        let v = matrix_status();
        assert_eq!(v["ok"], true);
        let modes = v["modes"].as_array().expect("modes 必须是数组");
        assert_eq!(modes.len(), UserMode::all().len(), "档位数必须与 UserMode::all 一致");
        for m in modes {
            assert!(m["mode"].as_str().is_some_and(|s| !s.is_empty()), "mode key 非空");
            assert!(m["intensity"].is_string() && m["working"].is_string());
            let rows = m["rows"].as_array().expect("rows 必须是数组");
            assert_eq!(rows.len(), 3, "三个工具自主档位都要有");
            for r in rows {
                assert!(r["autonomyKey"].is_string(), "autonomyKey 必须是 kebab 串");
                for state in ["untrusted", "trusted"] {
                    assert!(
                        r[state]["verdict"].is_string(),
                        "{state} 裁决必须带 verdict 判别字段：{r}"
                    );
                }
            }
        }
        assert_eq!(v["auditLevels"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn read_only_always_allowed() {
        for m in UserMode::all() {
            let v = d(*m, ToolAutonomy::ReadOnly, false);
            assert!(v.is_allowed(), "{} 只读应放行: {:?}", m.as_key(), v);
        }
    }

    #[test]
    fn ask_blocks_write_and_high() {
        for m in [MinimalAsk, StandardAsk, ExtremeAsk] {
            assert!(matches!(
                d(m, ToolAutonomy::Mutating, false),
                MatrixDecision::Deny { .. }
            ));
            assert!(matches!(
                d(m, ToolAutonomy::HighAutonomy, true),
                MatrixDecision::Deny { .. }
            ));
        }
    }

    #[test]
    fn plan_write_confirm_high_deny() {
        for m in [MinimalPlan, StandardPlan, ExtremePlan] {
            assert!(matches!(
                d(m, ToolAutonomy::Mutating, false),
                MatrixDecision::Confirm { .. }
            ));
            assert!(matches!(
                d(m, ToolAutonomy::HighAutonomy, true),
                MatrixDecision::Deny { .. }
            ));
        }
    }

    #[test]
    fn minimal_doit_locks_high_autonomy_without_trust() {
        // 极简档位默认锁高自主工具（无授权 → 拒绝）
        let v = d(MinimalDoIt, ToolAutonomy::HighAutonomy, false);
        assert!(matches!(v, MatrixDecision::Deny { .. }), "极简执行应锁高自主: {v:?}");
        // 显式授权后放行
        let v2 = d(MinimalDoIt, ToolAutonomy::HighAutonomy, true);
        assert!(matches!(
            v2,
            MatrixDecision::Allow { audit: AuditStrength::Full }
        ));
    }

    #[test]
    fn standard_doit_high_autonomy_confirm_without_trust() {
        // 标准执行：无授权 → 人工确认
        let v = d(StandardDoIt, ToolAutonomy::HighAutonomy, false);
        assert!(matches!(v, MatrixDecision::Confirm { .. }));
        // 显式授权 → 放行 + 全量审计
        let v2 = d(StandardDoIt, ToolAutonomy::HighAutonomy, true);
        assert!(matches!(
            v2,
            MatrixDecision::Allow { audit: AuditStrength::Full }
        ));
    }

    #[test]
    fn extreme_doit_high_autonomy_opens_after_explicit_grant() {
        // 极致·执行：显式授权后放开
        let v = d(ExtremeDoIt, ToolAutonomy::HighAutonomy, true);
        assert!(matches!(
            v,
            MatrixDecision::Allow { audit: AuditStrength::Full }
        ));
        // 无授权仍需确认（fail-open 到人工，fail-closed 到拒绝）
        let v2 = d(ExtremeDoIt, ToolAutonomy::HighAutonomy, false);
        assert!(matches!(v2, MatrixDecision::Confirm { .. }));
    }

    #[test]
    fn minimal_doit_mutating_confirm_standard_extreme_allow() {
        assert!(matches!(
            d(MinimalDoIt, ToolAutonomy::Mutating, false),
            MatrixDecision::Confirm { .. }
        ));
        for m in [StandardDoIt, ExtremeDoIt] {
            let v = d(m, ToolAutonomy::Mutating, false);
            assert!(matches!(
                v,
                MatrixDecision::Allow { audit: AuditStrength::Standard }
            ));
        }
    }

    #[test]
    fn classify_known_tools() {
        let high = [
            "shell_exec", "exec", "terminal", "control_desktop", "web_fetch", "install_dependency",
            "browser",
        ];
        for t in high {
            assert_eq!(
                classify_tool_autonomy(t),
                ToolAutonomy::HighAutonomy,
                "{t} 应归类高自主"
            );
        }
        let mutating = ["edit_file", "write_file", "delete_file", "apply_patch", "git_commit"];
        for t in mutating {
            assert_eq!(
                classify_tool_autonomy(t),
                ToolAutonomy::Mutating,
                "{t} 应归类写"
            );
        }
        let read = ["read_file", "list_dir", "grep", "find_files", "memory_search", "run_tests"];
        for t in read {
            assert_eq!(
                classify_tool_autonomy(t),
                ToolAutonomy::ReadOnly,
                "{t} 应归类只读"
            );
        }
    }

    #[test]
    fn decision_serializes_plain() {
        let v = d(ExtremeDoIt, ToolAutonomy::HighAutonomy, true);
        let wire = serde_json::to_string(&v).unwrap();
        assert!(wire.contains("\"allow\""));
        assert!(wire.contains("\"full\""));
    }

    /// `computer_use` 必须被识别为高自主，并进入引擎的真实审批面。
    #[test]
    fn computer_use_is_high_autonomy_and_approval_surface() {
        assert_eq!(
            classify_tool_autonomy("computer_use"),
            ToolAutonomy::HighAutonomy
        );
        assert!(approval_surface("computer_use"));
        // 出网检索不属于「直接驱动本机」的拦截面（避免把 Ask/Plan 的查资料锁死），
        // 因此不产出 approval 字段（不伪造）。
        assert!(!approval_surface("web_search"));
        assert!(decide_tool("web_search", "s", None).is_none());
        assert!(decide_tool("read_file", "s", None).is_none());
    }

    /// 引擎裁决必须按真实档位变化：极简执行 → denied；极致执行（未授权）→ pending；
    /// 显式信任后 → approved。三种取值都来自 `PermissionMatrix::decide`，不是写死。
    #[test]
    fn runtime_approval_reflects_mode_and_trust() {
        let _g = crate::mode::test_runtime_mode_guard();
        let prev = crate::mode::runtime_mode();

        crate::mode::set_runtime_mode(crate::mode::ModeKind::Ensemble);
        let denied = decide_tool("computer_use", "sess-approval-untouched", None).unwrap();
        assert_eq!(denied.status, "denied", "极简执行默认锁高自主");
        assert!(denied.reason.is_some());

        crate::mode::set_runtime_mode(crate::mode::ModeKind::OmniTranscend);
        let approved_by_mode = decide_tool("computer_use", "sess-approval-untouched", None).unwrap();
        // 极致·执行本身就是用户对该档位的显式授权（见 `mode_authorizes_high_autonomy`）：
        // 此前这里要求 pending，但桌面主路径没有同步审批通道 → fail-closed，
        // 于是「选了极致·执行却连截图都做不了」。现在按授权放行，并带全量审计。
        assert_eq!(approved_by_mode.status, "approved", "极致执行视为已授权");
        assert_eq!(approved_by_mode.audit, Some(AuditStrength::Full));

        // 显式信任（落盘级 High）后放行；用 auto_replay 隔离，避免影响其它用例。
        let id = {
            let mut store = crate::guard::trust::global()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            store.grant(
                crate::guard::trust::TrustLevel::High,
                "auto_replay",
                None,
                "sess-approval-granted",
            )
        };
        let approved = decide_tool("auto_replay", "sess-approval-granted", None).unwrap();
        assert_eq!(approved.status, "approved", "信任后应放行");
        assert!(approved.audit.is_some(), "高自主放行必须带全量审计强度");
        crate::guard::trust::global()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .revoke(&id);

        crate::mode::set_runtime_mode(prev);
    }
}