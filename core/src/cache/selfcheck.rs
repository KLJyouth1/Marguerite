use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandResult {
    pub success: bool,
    pub output: String,
    pub diagnostics: String,
}

impl CommandResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            diagnostics: String::new(),
        }
    }

    pub fn failure(output: impl Into<String>, diagnostics: impl Into<String>) -> Self {
        Self {
            success: false,
            output: output.into(),
            diagnostics: diagnostics.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckResult {
    accepted: bool,
    retry: bool,
    circuit_open: bool,
    attempts: usize,
    repair_hint: String,
}

impl CheckResult {
    pub fn is_success(&self) -> bool {
        self.accepted
    }
    pub fn should_retry(&self) -> bool {
        self.retry
    }
    pub fn is_circuit_open(&self) -> bool {
        self.circuit_open
    }
    pub fn is_terminal(&self) -> bool {
        !self.retry
    }
    pub fn attempts(&self) -> usize {
        self.attempts
    }
    pub fn repair_hint(&self) -> &str {
        &self.repair_hint
    }
}

impl fmt::Display for CheckResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "accepted={}, retry={}, attempts={}",
            self.accepted, self.retry, self.attempts
        )
    }
}

/// Deterministic post-command self-check with bounded retries and circuit breaking.
pub struct SelfCheck {
    attempts: usize,
    max_retries: usize,
    circuit_open: bool,
}

impl SelfCheck {
    pub fn new() -> Self {
        Self {
            attempts: 0,
            max_retries: 2,
            circuit_open: false,
        }
    }

    pub fn evaluate(&self, result: CommandResult) -> CheckResult {
        if self.circuit_open {
            return CheckResult {
                accepted: false,
                retry: false,
                circuit_open: true,
                attempts: self.attempts,
                repair_hint: "自检熔断已打开，请修复后重新执行".into(),
            };
        }
        if result.success {
            return CheckResult {
                accepted: true,
                retry: false,
                circuit_open: false,
                attempts: self.attempts,
                repair_hint: String::new(),
            };
        }
        let retryable = is_retryable(&result.diagnostics) && is_retryable(&result.output);
        let next_attempt = self.attempts + 1;
        let retry = retryable && next_attempt <= self.max_retries;
        let circuit_open = retryable && next_attempt > self.max_retries;
        CheckResult {
            accepted: false,
            retry,
            circuit_open,
            attempts: next_attempt,
            repair_hint: repair_hint(&result.diagnostics, retry, circuit_open),
        }
    }

    /// Record the outcome and return its structured decision.
    pub fn check(&mut self, result: CommandResult) -> CheckResult {
        let decision = self.evaluate(result);
        if decision.is_success() {
            self.attempts = 0;
        } else {
            self.attempts = decision.attempts;
            self.circuit_open = decision.circuit_open;
        }
        decision
    }

    pub fn attempts(&self) -> usize {
        self.attempts
    }
    pub fn is_circuit_open(&self) -> bool {
        self.circuit_open
    }
}

impl Default for SelfCheck {
    fn default() -> Self {
        SelfCheck::new()
    }
}

fn is_retryable(message: &str) -> bool {
    // T05：步骤重试裁决收口到 crate::errors（文本兜底口径与旧实现一致）。
    crate::errors::is_retryable_step_error_message(message)
}

fn repair_hint(diagnostics: &str, retry: bool, circuit_open: bool) -> String {
    if circuit_open {
        return format!("连续重试失败，已熔断；请检查诊断信息：{}", diagnostics);
    }
    if retry {
        return format!("命令失败，将重试；请检查诊断信息：{}", diagnostics);
    }
    format!("命令失败且不可重试；请修复确定性约束：{}", diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_result_is_accepted() {
        let mut check = SelfCheck::new();
        let result = check.check(CommandResult::success("ok"));
        assert!(result.is_success());
        assert_eq!(result.attempts(), 0);
    }

    #[test]
    fn retryable_failure_retries_twice_then_opens_circuit() {
        let mut check = SelfCheck::new();
        let failure = || CommandResult::failure("network timeout", "upstream unavailable");
        assert!(check.check(failure()).should_retry());
        assert!(check.check(failure()).should_retry());
        let final_result = check.check(failure());
        assert!(final_result.is_circuit_open());
        assert_eq!(final_result.attempts(), 3);
        assert!(!check
            .check(CommandResult::success("late success"))
            .is_success());
    }

    #[test]
    fn deterministic_governance_failure_does_not_retry() {
        let mut check = SelfCheck::new();
        let result = check.check(CommandResult::failure(
            "governance denied",
            "治理门裁决未通过",
        ));
        assert!(!result.should_retry());
        assert!(result.is_terminal());
        assert!(result.repair_hint().contains("治理"));
    }
}
