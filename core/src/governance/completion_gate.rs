//! 完成门禁（Completion Gate）—— 防止 agent「声称完成但实际未完成 / 产出未经验证」。
//!
//! 这是对标 标准 AI 助手 的「Exclusive」差异化能力：**执行前过宪法（PlanGate），
//! 产出后再验真伪（CompletionGate）**，形成完整的治理闭环。
//!
//! 与 `completion_gate.py`（nexustar-api 参考）对齐的四道检查：
//! 1. 反幻觉验证（Verification）—— 完成判据须客观通过，否则标为幻觉
//! 2. 产出物 / 结果非空（Artifacts）—— 空结果 / 过短结果阻断
//! 3. 时间压力检测（Time Pressure）—— 执行时间远低于预估最小时间 → 可疑草率完成
//! 4. 敷衍话术检测（Placation）—— 声称完成但无实质内容
//!
//! 设计原则：纯逻辑、零网络依赖；文件校验复用 `tools::fs_path::safe_resolve` 防路径逃逸，
//! 命令校验复用 `security::exec::sandboxed_exec` + `CommandGuard`。Shell 执行通过
//! `ShellRunner` trait 注入，离线单测用 `MockShellRunner`，运行时用 `RealShellRunner`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::proc::SilentCommand;
use crate::security::command_guard::CommandGuard;
use crate::tools::fs_path::safe_resolve;

/// 单次判据验证状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    Passed,
    Failed,
    Skipped,
    Error,
}

/// 单次判据的验证结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationOutcome {
    pub status: VerificationStatus,
    pub checker: String,
    pub message: String,
    pub evidence: HashMap<String, String>,
}

/// 一组完成判据的验证汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSummary {
    /// 全部有效判据是否通过
    pub all_passed: bool,
    /// 是否检测到「声称完成但验证失败」的幻觉
    pub is_hallucination: bool,
    pub summary: String,
    pub results: Vec<VerificationOutcome>,
}

/// 自定义判据验证器（可注入，便于离线测试）。
pub trait CriterionVerifier: Send + Sync {
    fn verify(&self, ctx: &VerificationCtx) -> VerificationOutcome;
}

/// 判据验证上下文：工作区根 + 可注入的 shell 执行器。
pub struct VerificationCtx {
    pub workspace: PathBuf,
    pub shell: Arc<dyn ShellRunner>,
}

/// 命令执行器抽象（便于离线注入）。
pub trait ShellRunner: Send + Sync {
    /// 返回 (是否成功, 合并的输出文本)
    fn run(&self, cmd: &str) -> (bool, String);
}

/// 运行时真实执行器：同步子进程 + `CommandGuard` 静态拦截（测试走 MockShellRunner）。
pub struct RealShellRunner;
impl ShellRunner for RealShellRunner {
    fn run(&self, cmd: &str) -> (bool, String) {
        let guard = CommandGuard::new();
        if let Err(e) = guard.check(cmd) {
            return (false, format!("command blocked by guard: {e}"));
        }
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        match std::process::Command::new(shell)
            .silent()
            .arg(flag)
            .arg(cmd)
            .output()
        {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                (out.status.success(), format!("{stdout}{stderr}"))
            }
            Err(e) => (false, format!("execution error: {e}")),
        }
    }
}

/// 完成判据：可验证的客观完成条件。
pub enum Criterion {
    /// 文件必须存在（路径相对工作区，经 safe_resolve 防逃逸）
    FileExists(String),
    /// 文件必须包含指定内容至少 min_count 次
    FileContains {
        path: String,
        content: String,
        min_count: usize,
    },
    /// 命令必须返回 0（经命令护栏）
    CommandSucceeds(String),
    /// 自定义验证器
    Custom(Box<dyn CriterionVerifier>),
}

/// 完成门禁阻断原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockReason {
    VerificationFailed,
    VerificationMissing,
    TimePressure,
    PlacationDetected,
    EmptyResult,
    /// 契约声明的必需产物未落盘或为空（P1-2：杜绝「声称完成却无产物」）。
    MissingArtifact,
}

/// 必需产物核验报告（P1-2）：把「契约声明的交付物」逐一核验是否真实落盘且非空。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredArtifactsReport {
    /// 声明为必需的产物路径（原样）。
    pub required: Vec<String>,
    /// 已落盘且非空的产物。
    pub present: Vec<String>,
    /// 缺失（不存在 / 非文件 / 路径解析失败）的产物。
    pub missing: Vec<String>,
    /// 存在但为空的产物。
    pub empty: Vec<String>,
    /// 存在且非空、但容器格式非法（如纯文本伪装的 .pptx）的产物。
    pub invalid: Vec<String>,
}

impl RequiredArtifactsReport {
    /// 全部必需产物均已落盘、非空且容器格式合法。
    pub fn all_ok(&self) -> bool {
        self.missing.is_empty() && self.empty.is_empty() && self.invalid.is_empty()
    }
}

/// 完成门禁裁决结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionVerdict {
    pub allowed: bool,
    pub blocked_by: Vec<BlockReason>,
    pub warnings: Vec<String>,
    pub evidence: HashMap<String, String>,
}

impl CompletionVerdict {
    pub fn allowed() -> Self {
        Self {
            allowed: true,
            blocked_by: Vec::new(),
            warnings: Vec::new(),
            evidence: HashMap::new(),
        }
    }
}

/// 一次完成声明的待检材料。
pub struct CompletionClaim<'a> {
    pub task_description: &'a str,
    pub execution_result: &'a str,
    pub artifacts: &'a [&'a str],
    /// 反幻觉验证汇总（如未执行验证则为 None）
    pub verification: Option<VerificationSummary>,
    /// 是否尝试过验证（未尝试则不因「缺失验证」阻断）
    pub verification_attempted: bool,
    /// 实际执行时长（秒），用于时间压力检测
    pub execution_duration: Option<f64>,
}

// ============================================================================
// 敷衍话术检测
// ============================================================================

/// 实质性产出关键词：输出含这些则不太可能是敷衍。
const SUBSTANCE_KEYWORDS: &[&str] = &[
    "def ",
    "class ",
    "import ",
    "function ",
    "const ",
    "export ",
    "git add",
    "git commit",
    "npm run",
    "cargo build",
    "python -m",
    "http://",
    "https://",
    "200 OK",
    "exit code 0",
    "```python",
    "```typescript",
    "```rust",
    "```json",
    "文件已写入",
    "端口",
    "PID",
    "进程",
];

/// 敷衍话术模式（声称完成但无实质内容支撑）。
const PLACATION_PATTERNS: &[(&str, &str)] = &[
    (
        r"已(?:经)?完成[^。\n]*(?:请|您|可以)(?:查看|测试|使用)",
        "声称完成后引导查看，但未提供产出物",
    ),
    (
        r"一切正常[^。\n]*(?:使用|测试|运行)",
        "声称一切正常但无验证证据",
    ),
    (
        r"(?:全部|所有)(?:功能|任务|代码)(?:已|都)(?:完成|实现|就绪)",
        "笼统声称全部完成但无具体产出",
    ),
    (
        r"已(?:为您|帮你)(?:完成|搞定|解决)[^。\n]*$",
        "口语化声称完成但无产出细节",
    ),
    (
        r"成功(?:创建|修改|删除|启动)[^。\n]*(?:！|。)\s*$",
        "声称成功操作但无验证",
    ),
    (
        r"(?:测试通过|验证通过)(?:[，。]\s*$|$)",
        "声称测试通过但无测试报告",
    ),
    (r"没有问题[^。\n]*可以(?:正常)?使用", "声称没问题但无验证"),
    (
        r"已(?:经)?修复[^。\n]*(?:问题|bug)[^。\n]*[。.]?$",
        "声称修复但无修复证据",
    ),
    (
        r"(?:全部|所有)(?:已|都)(?:就绪|OK|ok|Ok)",
        "简短声称全部就绪",
    ),
    (r"完成[了]?\s*[！。]\s*$", "极简完成声明，无任何细节"),
];

/// 敷衍检测器。
pub struct PlacationDetector {
    patterns: Vec<(regex::Regex, &'static str)>,
}

impl Default for PlacationDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl PlacationDetector {
    pub fn new() -> Self {
        let patterns = PLACATION_PATTERNS
            .iter()
            .filter_map(|(p, desc)| {
                regex::RegexBuilder::new(p)
                    .case_insensitive(true)
                    .multi_line(true)
                    .build()
                    .ok()
                    .map(|re| (re, *desc))
            })
            .collect();
        Self { patterns }
    }

    /// 返回 (是否敷衍, 说明, 是否有实质内容)
    pub fn detect(&self, output: &str, has_artifacts: bool) -> (bool, String, bool) {
        if output.trim().is_empty() {
            return (false, "输出为空".into(), false);
        }
        let has_substance = SUBSTANCE_KEYWORDS.iter().any(|kw| output.contains(kw));
        let matched: Vec<&str> = self
            .patterns
            .iter()
            .filter(|(re, _)| re.is_match(output))
            .map(|(_, desc)| *desc)
            .collect();
        if matched.is_empty() {
            return (false, String::new(), has_substance);
        }
        if !has_substance && !has_artifacts {
            return (
                true,
                format!(
                    "检测到敷衍话术（{} 处），但输出中无实质性产出内容或产出文件",
                    matched.len()
                ),
                false,
            );
        }
        (
            false,
            format!(
                "检测到 {} 处完成话术，但输出中有实质内容（可能是正常完成报告）",
                matched.len()
            ),
            true,
        )
    }
}

// ============================================================================
// 时间压力检测
// ============================================================================

/// 任务类型预估最小执行时间（秒）。
///
/// 2026-10-24：不再只按关键词写死，而是先取**自适应预算**的任务档位给一个下限，
/// 再叠加动作关键词（同一档内「打包」显然比「改一行」久）。这样「一句话问答」
/// 不会被 5s 门槛误判，而「重构 + 验证」也不会被 3s 门槛放过。
fn estimate_min_time(task_description: &str) -> f64 {
    let desc = task_description.to_lowercase();
    let rules: &[(&[&str], f64)] = &[
        (&["编译", "compile", "build", "cargo"], 5.0),
        (&["打包", "package", "bundle", "nsis"], 10.0),
        (&["部署", "deploy", "启动服务", "start_server"], 8.0),
        (&["写", "write", "create", "创建"], 0.05),
        (&["编辑", "edit", "修改", "modify", "update"], 0.3),
        (&["测试", "test", "验证", "verify"], 5.0),
        (&["重构", "refactor", "重写"], 3.0),
        (&["api", "接口", "router"], 2.0),
        (&["修复", "fix", "bug", "补丁"], 2.0),
    ];
    let mut matched: f64 = 0.0;
    for (kws, t) in rules {
        if kws.iter().any(|kw| desc.contains(*kw)) {
            matched = matched.max(*t);
        }
    }
    // 档位下限：任务越重，「快得离谱」的嫌疑越大。
    let floor = match crate::agent::budget_policy::resolve(task_description, 0, 0).scale {
        crate::agent::budget_policy::TaskScale::Simple => 0.0,
        crate::agent::budget_policy::TaskScale::Standard => 1.0,
        crate::agent::budget_policy::TaskScale::Complex => 5.0,
        crate::agent::budget_policy::TaskScale::Deep => 15.0,
    };
    matched.max(floor)
}

/// 时间压力检测。
///
/// 关键改动：**有客观证据（产物 / 通过的验证）时不因「快」而阻断**。
/// 旧实现只看耗时，快速但真做完的任务会被判「草率完成」——那是最伤信任的
/// 假阳性。没有证据 + 明显低于预估，才视为可疑。
fn check_time_pressure(
    task_description: &str,
    duration: f64,
    has_evidence: bool,
) -> (bool, String) {
    let min_time = estimate_min_time(task_description);
    if min_time <= 0.0 {
        return (false, String::new());
    }
    if duration < min_time * 0.5 {
        if has_evidence {
            return (
                false,
                format!(
                    "任务执行时间 {}s 低于预估 {}s，但已有客观产物/验证证据，不判为草率完成",
                    duration, min_time
                ),
            );
        }
        return (
            true,
            format!(
                "任务执行时间 {}s 低于预估最小时间 {}s 的 50%，且无产物/验证证据，可能是时间压力导致的草率完成",
                duration, min_time
            ),
        );
    }
    if duration < min_time {
        return (
            false,
            format!("任务执行时间 {}s 低于预估最小时间 {}s", duration, min_time),
        );
    }
    (false, String::new())
}

// ============================================================================
// CompletionGate
// ============================================================================

/// 完成门禁：硬性阻断不合规的完成声明。
pub struct CompletionGate {
    placation: PlacationDetector,
}

impl Default for CompletionGate {
    fn default() -> Self {
        Self::new()
    }
}

impl CompletionGate {
    pub fn new() -> Self {
        Self {
            placation: PlacationDetector::new(),
        }
    }

    /// 验证一组完成判据（客观真伪）。
    pub fn verify_criteria(
        &self,
        criteria: &[Criterion],
        workspace: &Path,
        shell: &Arc<dyn ShellRunner>,
    ) -> VerificationSummary {
        if criteria.is_empty() {
            return VerificationSummary {
                all_passed: true,
                is_hallucination: false,
                summary: "无验证判据，跳过验证".into(),
                results: Vec::new(),
            };
        }
        let ctx = VerificationCtx {
            workspace: workspace.to_path_buf(),
            shell: shell.clone(),
        };
        let mut results = Vec::new();
        let mut passed = 0;
        let mut failed = 0;
        for c in criteria {
            let out = self.verify_one(c, &ctx);
            match out.status {
                VerificationStatus::Passed => passed += 1,
                VerificationStatus::Failed => failed += 1,
                _ => {}
            }
            results.push(out);
        }
        let effective = passed + failed;
        let all_passed = effective > 0 && failed == 0;
        let summary = if all_passed {
            format!("验证通过：{passed}/{} 项判据满足", criteria.len())
        } else {
            format!("验证失败：{failed}/{} 项未通过", criteria.len())
        };
        VerificationSummary {
            all_passed,
            is_hallucination: false, // 由调用方在检测到「声称完成但验证失败」时置位
            summary,
            results,
        }
    }

    fn verify_one(&self, c: &Criterion, ctx: &VerificationCtx) -> VerificationOutcome {
        match c {
            Criterion::FileExists(p) => {
                let resolved = safe_resolve(&ctx.workspace, p);
                match resolved {
                    Ok(path) => {
                        let exists = path.exists();
                        VerificationOutcome {
                            status: if exists {
                                VerificationStatus::Passed
                            } else {
                                VerificationStatus::Failed
                            },
                            checker: "file_exists".into(),
                            message: format!(
                                "文件{}: {}",
                                if exists { "存在" } else { "不存在" },
                                path.display()
                            ),
                            evidence: HashMap::from([
                                ("path".into(), path.display().to_string()),
                                ("exists".into(), exists.to_string()),
                            ]),
                        }
                    }
                    Err(e) => VerificationOutcome {
                        status: VerificationStatus::Error,
                        checker: "file_exists".into(),
                        message: format!("路径解析失败: {e}"),
                        evidence: HashMap::from([("error".into(), e.to_string())]),
                    },
                }
            }
            Criterion::FileContains {
                path,
                content,
                min_count,
            } => {
                let resolved = safe_resolve(&ctx.workspace, path);
                match resolved {
                    Ok(p) => {
                        if !p.exists() {
                            return VerificationOutcome {
                                status: VerificationStatus::Failed,
                                checker: "file_contains".into(),
                                message: format!("文件不存在: {}", p.display()),
                                evidence: HashMap::from([("path".into(), p.display().to_string())]),
                            };
                        }
                        match std::fs::read_to_string(&p) {
                            Ok(text) => {
                                let count = text.matches(content).count();
                                let ok = count >= *min_count;
                                VerificationOutcome {
                                    status: if ok {
                                        VerificationStatus::Passed
                                    } else {
                                        VerificationStatus::Failed
                                    },
                                    checker: "file_contains".into(),
                                    message: format!(
                                        "文件包含「{}」{} 次{}",
                                        content,
                                        count,
                                        if ok { "（满足）" } else { "（不足）" }
                                    ),
                                    evidence: HashMap::from([
                                        ("path".into(), p.display().to_string()),
                                        ("expected".into(), content.clone()),
                                        ("actual_count".into(), count.to_string()),
                                        ("min_count".into(), min_count.to_string()),
                                    ]),
                                }
                            }
                            Err(e) => VerificationOutcome {
                                status: VerificationStatus::Error,
                                checker: "file_contains".into(),
                                message: format!("读取失败: {e}"),
                                evidence: HashMap::from([("error".into(), e.to_string())]),
                            },
                        }
                    }
                    Err(e) => VerificationOutcome {
                        status: VerificationStatus::Error,
                        checker: "file_contains".into(),
                        message: format!("路径解析失败: {e}"),
                        evidence: HashMap::from([("error".into(), e.to_string())]),
                    },
                }
            }
            Criterion::CommandSucceeds(cmd) => {
                let (ok, out) = ctx.shell.run(cmd);
                VerificationOutcome {
                    status: if ok {
                        VerificationStatus::Passed
                    } else {
                        VerificationStatus::Failed
                    },
                    checker: "command_succeeds".into(),
                    message: format!("命令返回{}", if ok { "成功" } else { "失败" }),
                    evidence: HashMap::from([
                        ("command".into(), cmd.clone()),
                        ("output".into(), out.chars().take(500).collect()),
                    ]),
                }
            }
            Criterion::Custom(v) => v.verify(ctx),
        }
    }

    /// 完成门禁检查（四道检查）。任何一项阻断 → allowed=false。
    pub fn check_completion(&self, claim: CompletionClaim) -> CompletionVerdict {
        let mut blocked_by = Vec::new();
        let mut warnings = Vec::new();
        let mut evidence = HashMap::new();

        // ---- 检查 1：反幻觉验证 ----
        if let Some(v) = &claim.verification {
            if v.all_passed {
                evidence.insert("verification_passed".into(), "true".into());
            } else {
                blocked_by.push(BlockReason::VerificationFailed);
                warnings.push(format!("反幻觉验证未通过: {}", v.summary));
                evidence.insert("verification_failed".into(), "true".into());
            }
        } else if claim.verification_attempted {
            blocked_by.push(BlockReason::VerificationMissing);
            warnings.push("任务未执行反幻觉验证，禁止标记为完成".to_string());
        }

        // ---- 检查 2：产出物 / 结果非空 ----
        let artifact_count = claim.artifacts.len();
        evidence.insert("artifacts_count".into(), artifact_count.to_string());
        if artifact_count == 0 {
            let res = claim.execution_result.trim();
            let scale = crate::agent::budget_policy::resolve(claim.task_description, 0, 0).scale;
            if res.is_empty() {
                blocked_by.push(BlockReason::EmptyResult);
                warnings.push("执行结果为空，无任何产出".to_string());
            } else if res.chars().count() < 10
                && scale != crate::agent::budget_policy::TaskScale::Simple
            {
                // 只有非简单任务才把「极短回复」当未执行：寒暄/确认类任务一句话就是
                // 完整答案，按 10 字符一刀切会把正常回答判成空结果。
                blocked_by.push(BlockReason::EmptyResult);
                warnings.push(format!(
                    "执行结果过短（{} 字符），可能未实际执行",
                    res.chars().count()
                ));
            }
        }

        // ---- 检查 3：时间压力检测 ----
        if let Some(d) = claim.execution_duration {
            // 有客观证据（产物 / 通过的验证）时不再因「快」阻断。
            let has_evidence = artifact_count > 0
                || claim
                    .verification
                    .as_ref()
                    .is_some_and(|v| v.all_passed);
            let (suspicious, msg) = check_time_pressure(claim.task_description, d, has_evidence);
            if suspicious {
                blocked_by.push(BlockReason::TimePressure);
                warnings.push(msg);
            } else if !msg.is_empty() {
                warnings.push(format!("[警告] {msg}"));
            }
            evidence.insert("execution_duration".into(), d.to_string());
            evidence.insert("time_suspicious".into(), suspicious.to_string());
        }

        // ---- 检查 4：敷衍话术检测 ----
        let (placation, msg, has_substance) = self
            .placation
            .detect(claim.execution_result, artifact_count > 0);
        if placation {
            blocked_by.push(BlockReason::PlacationDetected);
            warnings.push(format!("敷衍检测: {msg}"));
        } else if !msg.is_empty() {
            warnings.push(format!("[警告] {msg}"));
        }
        evidence.insert("placation_detected".into(), placation.to_string());
        evidence.insert("has_substance".into(), has_substance.to_string());

        CompletionVerdict {
            allowed: blocked_by.is_empty(),
            blocked_by,
            warnings,
            evidence,
        }
    }

    /// 从综合产出中**提取 agent 自报的产出文件**，转为可验证的客观判据。
    ///
    /// 这正是「完成门禁吃到自动验证结果」的核心：不再信任 agent 的口头声称，
    /// 而是把它在回复里**真正声明创建/修改过的文件**拎出来，逐一核验是否落盘。
    /// 仅提取两类高置信信号，避免误伤正常解释性回复：
    ///   1. 代码围栏文件名：` ```rust:src/login.rs ` 或 ` ```tsx components/Login.tsx `
    ///   2. 显式创建声明：「创建/新建/生成/写入 … path.ext」；
    ///      仅当路径含分隔符（相对路径）才纳入，过滤掉散文中的孤立文件名。
    pub fn extract_claimed_artifacts(&self, reply: &str) -> Vec<Criterion> {
        use std::sync::OnceLock;
        static RE: OnceLock<Vec<regex::Regex>> = OnceLock::new();
        let res = RE.get_or_init(|| {
            vec![
                // 代码围栏文件名：` ```lang:path ` 或 ` ```lang path `
                regex::RegexBuilder::new(
                    r"(?m)^```[A-Za-z0-9]*\s*[:\s]\s*([A-Za-z0-9_./\\-]+\.[A-Za-z0-9]{1,6})",
                )
                .case_insensitive(true)
                .build()
                .unwrap(),
                // 显式创建声明
                regex::RegexBuilder::new(
                    r"(?:已创建|创建|新建|生成|写入|已写入|created|wrote|new file)[^。\n]{0,60}?([A-Za-z0-9_./\\-]+\.[A-Za-z0-9]{1,6})",
                )
                .case_insensitive(true)
                .build()
                .unwrap(),
            ]
        });
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        // 认得出的交付物扩展名：无分隔符的文件名（「已创建 hello.txt」）也要核验，
        // 否则模型只要省掉 `./` 就能绕过反幻觉核验（虚假完成的真实绕过路径）。
        // 用白名单而不是「任意点号后缀」，避免把「已创建 v1.0」这类版本号当文件。
        const KNOWN_EXTS: &[&str] = &[
            "md", "txt", "html", "htm", "py", "js", "ts", "tsx", "jsx", "vue", "rs", "json",
            "yaml", "yml", "csv", "pptx", "docx", "xlsx", "pdf", "png", "jpg", "jpeg", "svg",
            "mp4", "zip", "sh", "ps1", "toml", "css", "scss", "sql",
        ];
        for re in res {
            for cap in re.captures_iter(reply) {
                if let Some(m) = cap.get(1) {
                    let p = m.as_str().to_string();
                    if p.len() > 120 || !seen.insert(p.clone()) {
                        continue;
                    }
                    let has_sep = p.contains('/') || p.contains('\\');
                    let ext_ok = p
                        .rsplit('.')
                        .next()
                        .map(|e| KNOWN_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                        .unwrap_or(false);
                    if has_sep || ext_ok {
                        out.push(Criterion::FileExists(p));
                    }
                }
            }
        }
        out
    }

    /// 综合完成门禁（P1-2 单一事实源）：把「提取自报产物 → 客观核验 → 四道检查 →
    /// 必需产物强约束」串成一步，供 [`crate::orchestration::Orchestrator`] 与计划
    /// 路径 runner 共用，杜绝两处逻辑漂移。
    ///
    /// - `extract_claimed_artifacts`：从回复里提取 agent 自报创建/修改的文件，逐一核验
    ///   是否真实落盘（反幻觉）；
    /// - `check_completion`：非空 / 时间压力 / 敷衍四检查；
    /// - `verify_required_artifacts`：调用方**显式声明**的必需产物路径逐一核验真实落盘
    ///   且非空，任一缺失/为空 → 强制 `allowed=false` + [`BlockReason::MissingArtifact`]。
    ///
    /// 这正是「计划路径绝不因回复够长就放行」的核心：必需产物由契约/步骤标题决定，
    /// 与 agent 口头声称无关。
    pub fn evaluate(
        &self,
        task_description: &str,
        execution_result: &str,
        required_artifacts: &[&str],
        workspace: &Path,
        shell: &Arc<dyn ShellRunner>,
        duration: Option<f64>,
    ) -> CompletionVerdict {
        let criteria = self.extract_claimed_artifacts(execution_result);
        let verification = if criteria.is_empty() {
            None
        } else {
            Some(self.verify_criteria(&criteria, workspace, shell))
        };
        let mut verdict = self.check_completion(CompletionClaim {
            task_description,
            execution_result,
            artifacts: required_artifacts,
            verification,
            verification_attempted: !criteria.is_empty(),
            execution_duration: duration,
        });
        if !required_artifacts.is_empty() {
            let report = self.verify_required_artifacts(required_artifacts, workspace);
            if !report.all_ok() {
                verdict.allowed = false;
                if !verdict.blocked_by.contains(&BlockReason::MissingArtifact) {
                    verdict.blocked_by.push(BlockReason::MissingArtifact);
                }
                let mut details = Vec::new();
                if !report.missing.is_empty() {
                    details.push(format!("缺失: {}", report.missing.join(", ")));
                }
                if !report.empty.is_empty() {
                    details.push(format!("空文件: {}", report.empty.join(", ")));
                }
                if !report.invalid.is_empty() {
                    details.push(format!("格式非法: {}", report.invalid.join(", ")));
                }
                verdict.warnings.push(format!(
                    "必需产物未落盘、为空或格式非法（{}），不得标记为完成",
                    details.join("；")
                ));
                verdict
                    .evidence
                    .insert("required_artifacts_total".into(), report.required.len().to_string());
                verdict.evidence.insert(
                    "required_artifacts_present".into(),
                    report.present.len().to_string(),
                );
                verdict.evidence.insert(
                    "required_artifacts_invalid".into(),
                    report.invalid.len().to_string(),
                );
            }
        }
        verdict
    }

    /// 核验**必需产物**是否真实落盘且非空（P1-2）。
    ///
    /// 与「从回复里猜测产物」的 [`Self::extract_claimed_artifacts`] 互补：这里的路径由
    /// 调用方（交付契约）显式声明，属于**强约束**。路径经 `safe_resolve` 防逃逸；
    /// 仅接受**非空常规文件**（目录 / 缺失 / 解析失败均计入缺失，空文件单列）。
    pub fn verify_required_artifacts(
        &self,
        required: &[&str],
        workspace: &Path,
    ) -> RequiredArtifactsReport {
        let mut report = RequiredArtifactsReport::default();
        for raw in required {
            let name = (*raw).to_string();
            report.required.push(name.clone());
            match safe_resolve(workspace, raw) {
                Ok(path) => match std::fs::metadata(&path) {
                    Ok(md) if md.is_file() && md.len() > 0 => {
                        // 非空只是必要条件：办公交付物还必须是**合法容器**
                        // （.pptx/.docx/.xlsx 为 OOXML ZIP，.pdf 带 %PDF- 头），
                        // 否则纯文本伪装的「假交付物」会被误判为真实落盘。
                        match crate::tools::ooxml::validate_office_container(&path) {
                            Ok(()) => report.present.push(name),
                            Err(_) => report.invalid.push(name),
                        }
                    }
                    Ok(md) if md.is_file() => report.empty.push(name),
                    _ => report.missing.push(name),
                },
                Err(_) => report.missing.push(name),
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockShellRunner {
        ok: bool,
        out: String,
    }
    impl ShellRunner for MockShellRunner {
        fn run(&self, _cmd: &str) -> (bool, String) {
            (self.ok, self.out.clone())
        }
    }

    fn tmp_workspace() -> (tempdir_guard::TempDir, PathBuf) {
        // 轻量临时目录：每调用唯一，避免并行测试共享目录竞态（同进程同 pid 重名）。
        use std::sync::atomic::{AtomicU64, Ordering};
        static CG_SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = CG_SEQ.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("bloom_cg_{}_{}", std::process::id(), seq));
        let _ = std::fs::create_dir_all(&base);
        (tempdir_guard::TempDir(base.clone()), base)
    }

    mod tempdir_guard {
        use std::path::PathBuf;
        pub struct TempDir(pub PathBuf);
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn file_exists_passes_when_present() {
        let (_g, ws) = tmp_workspace();
        let f = ws.join("a.txt");
        std::fs::write(&f, "hello").unwrap();
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: String::new(),
        });
        let gate = CompletionGate::new();
        let v = gate.verify_criteria(&[Criterion::FileExists("a.txt".into())], &ws, &shell);
        assert!(v.all_passed, "存在文件应验证通过");
    }

    #[test]
    fn file_exists_fails_when_absent() {
        let (_g, ws) = tmp_workspace();
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: String::new(),
        });
        let gate = CompletionGate::new();
        let v = gate.verify_criteria(&[Criterion::FileExists("nope.txt".into())], &ws, &shell);
        assert!(!v.all_passed, "缺失文件应验证失败");
    }

    #[test]
    fn file_contains_checks_count() {
        let (_g, ws) = tmp_workspace();
        let f = ws.join("b.txt");
        std::fs::write(&f, "foo foo foo").unwrap();
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: String::new(),
        });
        let gate = CompletionGate::new();
        let v = gate.verify_criteria(
            &[Criterion::FileContains {
                path: "b.txt".into(),
                content: "foo".into(),
                min_count: 2,
            }],
            &ws,
            &shell,
        );
        assert!(v.all_passed, "包含 3 次『foo』应满足 min_count=2");
    }

    #[test]
    fn command_succeeds_uses_shell() {
        let (_g, ws) = tmp_workspace();
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: "0".into(),
        });
        let gate = CompletionGate::new();
        let v = gate.verify_criteria(&[Criterion::CommandSucceeds("true".into())], &ws, &shell);
        assert!(v.all_passed, "命令返回成功应验证通过");
    }

    #[test]
    fn verify_criteria_detects_hallucination() {
        // 声称完成但 file_exists 判据失败 → all_passed=false
        let (_g, ws) = tmp_workspace();
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: String::new(),
        });
        let gate = CompletionGate::new();
        let v = gate.verify_criteria(&[Criterion::FileExists("missing.rs".into())], &ws, &shell);
        assert!(!v.all_passed);
    }

    #[test]
    fn verification_failed_blocks_completion() {
        let gate = CompletionGate::new();
        let empty: &[Criterion] = &[];
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: false,
            out: String::new(),
        });
        let v = gate.verify_criteria(empty, Path::new("."), &shell);
        let claim = CompletionClaim {
            task_description: "实现登录",
            execution_result: "已创建 login.rs 含实现",
            artifacts: &[],
            verification: Some(VerificationSummary {
                all_passed: false,
                is_hallucination: true,
                summary: "1/1 判据失败".into(),
                results: v.results,
            }),
            verification_attempted: true,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(!d.allowed);
        assert!(d.blocked_by.contains(&BlockReason::VerificationFailed));
    }

    #[test]
    fn empty_result_blocks() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "测试",
            execution_result: "",
            artifacts: &[],
            verification: None,
            verification_attempted: false,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(!d.allowed);
        assert!(d.blocked_by.contains(&BlockReason::EmptyResult));
    }

    #[test]
    fn time_pressure_blocks_fast_build() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "编译整个项目 cargo build",
            execution_result: "build done",
            artifacts: &[],
            verification: None,
            verification_attempted: false,
            execution_duration: Some(0.1),
        };
        let d = gate.check_completion(claim);
        assert!(!d.allowed);
        assert!(d.blocked_by.contains(&BlockReason::TimePressure));
    }

    /// 快但有产物：不再判「时间压力」——旧实现只看耗时，会把「读缓存秒完成」
    /// 这类真实完成误判为草率。
    #[test]
    fn fast_completion_with_artifacts_is_not_time_pressure() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "编译整个项目 cargo build",
            execution_result: "build done",
            artifacts: &["dist/app.exe"],
            verification: None,
            verification_attempted: false,
            execution_duration: Some(0.1),
        };
        let d = gate.check_completion(claim);
        assert!(
            !d.blocked_by.contains(&BlockReason::TimePressure),
            "有产物时不得因快而阻断: {d:?}"
        );
        assert!(d.allowed, "有产物且无其它问题时应放行: {d:?}");
    }

    /// 简单任务的极短回复是完整答案，不再按 10 字符一刀切判空。
    #[test]
    fn short_answer_for_simple_task_is_not_empty_result() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "你好",
            execution_result: "你好！",
            artifacts: &[],
            verification: None,
            verification_attempted: false,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(
            !d.blocked_by.contains(&BlockReason::EmptyResult),
            "简单寒暄不应被判空结果: {d:?}"
        );
        assert!(d.allowed, "{d:?}");
    }

    #[test]
    fn placation_blocks_without_substance() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "修复 bug",
            execution_result: "已修复问题，请查看。",
            artifacts: &[],
            verification: None,
            verification_attempted: false,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(!d.allowed);
        assert!(d.blocked_by.contains(&BlockReason::PlacationDetected));
    }

    #[test]
    fn placation_with_substance_warns_not_block() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "修复 bug",
            execution_result: "已修复问题。改动：\n```rust\nfn f() {}\n```\nexit code 0",
            artifacts: &[],
            verification: None,
            verification_attempted: false,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(d.allowed, "有实质内容不应因完成话术阻断");
    }

    #[test]
    fn clean_completion_allowed() {
        let gate = CompletionGate::new();
        let claim = CompletionClaim {
            task_description: "解释闭包",
            execution_result: "闭包是捕获环境的匿名函数，示例：\n```rust\nlet f = |x| x + 1;\n```",
            artifacts: &[],
            verification: None,
            verification_attempted: false,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(d.allowed, "正常完成报告应过门");
    }

    #[test]
    fn extract_artifacts_from_fence_filename() {
        let gate = CompletionGate::new();
        let reply = "实现如下：\n```rust:src/login.rs\npub fn login() {}\n```";
        let crit = gate.extract_claimed_artifacts(reply);
        assert_eq!(crit.len(), 1, "应从代码围栏文件名提取 1 个判据");
        assert!(matches!(crit[0], Criterion::FileExists(ref p) if p == "src/login.rs"));
    }

    #[test]
    fn extract_artifacts_from_creation_claim() {
        let gate = CompletionGate::new();
        let reply = "已创建 src/auth/session.ts 包含会话逻辑。";
        let crit = gate.extract_claimed_artifacts(reply);
        assert_eq!(crit.len(), 1, "应从显式创建声明提取 1 个判据");
        assert!(matches!(crit[0], Criterion::FileExists(ref p) if p == "src/auth/session.ts"));
    }

    #[test]
    fn extract_artifacts_ignores_plain_explanation() {
        let gate = CompletionGate::new();
        // 普通解释性回复（代码块无文件名、无创建声明）→ 不应误提取
        let reply = "闭包是捕获环境的匿名函数，示例：\n```rust\nlet f = |x| x + 1;\n```";
        let crit = gate.extract_claimed_artifacts(reply);
        assert!(crit.is_empty(), "纯解释不应误提取文件判据");
    }

    #[test]
    fn extract_artifacts_skips_bare_filename() {
        let gate = CompletionGate::new();
        // 散文中孤立的文件名（无路径分隔符）应被过滤，避免误伤
        let reply = "请参考 login.rs 这个文件的设计。";
        let crit = gate.extract_claimed_artifacts(reply);
        assert!(crit.is_empty(), "无分隔符的孤立文件名不应被提取");
    }

    /// 虚假完成的真实绕过路径：模型声称「已创建 hello.txt」，只要省略 `./`，
    /// 旧实现就提取不到判据 → 门禁拿不到证据 → 直接放行。
    #[test]
    fn extract_artifacts_keeps_bare_name_in_creation_claim() {
        let gate = CompletionGate::new();
        let crit = gate.extract_claimed_artifacts("已创建 hello.txt，内容是一句问候。");
        assert_eq!(crit.len(), 1, "创建声明里的无分隔符文件名也必须核验");
        assert!(matches!(crit[0], Criterion::FileExists(ref p) if p == "hello.txt"));
    }

    /// 版本号/普通词不能被白名单吞成文件判据。
    #[test]
    fn extract_artifacts_rejects_unknown_extension_in_claim() {
        let gate = CompletionGate::new();
        let crit = gate.extract_claimed_artifacts("已创建 v1.0 版本说明。");
        assert!(crit.is_empty(), "未知扩展名（版本号）不应被当成产物");
    }

    #[test]
    fn extraction_feeds_real_verification() {
        // 端到端：提取 → 核验缺失文件 → 反幻觉拦截
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        let reply = "已创建 src/missing.rs 实现登录。";
        let crit = gate.extract_claimed_artifacts(reply);
        assert_eq!(crit.len(), 1);
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: String::new(),
        });
        let v = gate.verify_criteria(&crit, &ws, &shell);
        assert!(!v.all_passed, "声称创建但缺失的文件应验证失败");
        let claim = CompletionClaim {
            task_description: "实现登录",
            execution_result: reply,
            artifacts: &[],
            verification: Some(v),
            verification_attempted: true,
            execution_duration: None,
        };
        let d = gate.check_completion(claim);
        assert!(!d.allowed);
        assert!(d.blocked_by.contains(&BlockReason::VerificationFailed));
    }

    #[test]
    fn verify_required_artifacts_detects_missing_empty_and_present() {
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        std::fs::write(ws.join("ok.html"), "<html>hi</html>").unwrap();
        std::fs::write(ws.join("blank.html"), "").unwrap();
        let report = gate.verify_required_artifacts(
            &["ok.html", "blank.html", "gone.pptx"],
            &ws,
        );
        assert_eq!(report.required.len(), 3);
        assert_eq!(report.present, vec!["ok.html".to_string()]);
        assert_eq!(report.empty, vec!["blank.html".to_string()]);
        assert_eq!(report.missing, vec!["gone.pptx".to_string()]);
        assert!(!report.all_ok(), "存在缺失/空文件时不得 all_ok");
    }

    #[test]
    fn verify_required_artifacts_all_ok_when_present() {
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        std::fs::write(ws.join("a.txt"), "a").unwrap();
        std::fs::write(ws.join("b.txt"), "b").unwrap();
        let report = gate.verify_required_artifacts(&["a.txt", "b.txt"], &ws);
        assert!(report.all_ok(), "全部落盘且非空应 all_ok");
        assert!(gate.verify_required_artifacts(&[], &ws).all_ok(), "无要求恒 ok");
    }

    /// 构造一个未压缩（stored）的最小 ZIP：仅中央目录 + EOCD（无需真实压缩）。
    fn build_minimal_zip(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(out.len() as u32);
            out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
        let cd_start = out.len() as u32;
        for (i, name) in names.iter().enumerate() {
            out.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&offsets[i].to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
        let cd_size = out.len() as u32 - cd_start;
        out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&(names.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// 缺陷 E1 回归：纯文本伪装的 .pptx 必须进 `invalid`，不得算 present。
    #[test]
    fn verify_required_artifacts_flags_fake_pptx_as_invalid() {
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        std::fs::write(ws.join("deck.pptx"), "plain text, not a real pptx").unwrap();
        let report = gate.verify_required_artifacts(&["deck.pptx"], &ws);
        assert_eq!(report.invalid, vec!["deck.pptx".to_string()]);
        assert!(report.present.is_empty(), "假交付物不得计入 present");
        assert!(!report.all_ok(), "invalid 必须使 all_ok=false");
    }

    /// 缺陷 E1 回归：含必需条目的真 ZIP（.pptx）应被接受。
    #[test]
    fn verify_required_artifacts_accepts_genuine_pptx_container() {
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        let zip = build_minimal_zip(&["[Content_Types].xml", "ppt/presentation.xml"]);
        std::fs::write(ws.join("deck.pptx"), &zip).unwrap();
        let report = gate.verify_required_artifacts(&["deck.pptx"], &ws);
        assert_eq!(report.present, vec!["deck.pptx".to_string()]);
        assert!(report.invalid.is_empty());
        assert!(report.all_ok());
    }

    /// 缺陷 E1 回归：非办公扩展名（.txt）不受容器校验影响。
    #[test]
    fn verify_required_artifacts_leaves_txt_unaffected() {
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        std::fs::write(ws.join("notes.txt"), "just text").unwrap();
        let report = gate.verify_required_artifacts(&["notes.txt"], &ws);
        assert_eq!(report.present, vec!["notes.txt".to_string()]);
        assert!(report.invalid.is_empty());
    }

    /// 缺陷 E1 回归：`evaluate` 对 invalid 产物告警并阻断（计入不满足）。
    #[test]
    fn evaluate_warns_and_blocks_on_invalid_artifact() {
        let (_g, ws) = tmp_workspace();
        let gate = CompletionGate::new();
        std::fs::write(ws.join("deck.pptx"), "plain text, not a real pptx").unwrap();
        let shell: Arc<dyn ShellRunner> = Arc::new(MockShellRunner {
            ok: true,
            out: String::new(),
        });
        let v = gate.evaluate(
            "生成一份演示文稿",
            "已完成，产物为 deck.pptx。",
            &["deck.pptx"],
            &ws,
            &shell,
            None,
        );
        assert!(!v.allowed, "假交付物不得放行");
        assert!(v.blocked_by.contains(&BlockReason::MissingArtifact));
        assert!(
            v.warnings.iter().any(|w| w.contains("格式非法")),
            "告警须点名格式非法，实际: {:?}",
            v.warnings
        );
        assert_eq!(
            v.evidence.get("required_artifacts_present").map(String::as_str),
            Some("0"),
            "invalid 不得计入 present"
        );
        assert_eq!(
            v.evidence.get("required_artifacts_invalid").map(String::as_str),
            Some("1")
        );
    }
}
