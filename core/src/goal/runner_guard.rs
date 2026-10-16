//! Transport-neutral terminal guard for autonomous goal rounds.
//!
//! The guard is deliberately independent from plan execution. It reserves a goal
//! round with the registry's race fence and returns a structured terminal reason.

use super::{global, Goal, GoalPhase, GoalRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStopReason {
    MissingGoal,
    StaleRevision,
    Paused,
    Blocked,
    Complete,
    NotArmed,
    MaxGoalRounds,
}

impl GoalStopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingGoal => "missing_goal",
            Self::StaleRevision => "stale_revision",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Complete => "complete",
            Self::NotArmed => "not_armed",
            Self::MaxGoalRounds => "max_goal_rounds",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalGuardDecision {
    /// No goal was armed when the guard was created; preserve legacy execution.
    ContinueLegacy,
    /// A goal round was reserved and must be executed by the caller.
    Continue { revision: u64, rounds_started: u32 },
    /// The caller must stop before dispatching another round.
    Stopped {
        reason: GoalStopReason,
        revision: Option<u64>,
        rounds_started: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub struct GoalRunnerGuard {
    expected_revision: Option<u64>,
}

impl GoalRunnerGuard {
    /// 授予护栏：仅当目标**已激活**（armed）时才接管，否则保持既有计划执行行为。
    ///
    /// `armed` 是会话级状态且不落库（见 `goal` 模块文档）：进程重启、会话恢复或
    /// 目标已 pause/block/complete 后，注册表里可能残留一条「存在但未激活」的
    /// 目标记录。这类记录属于它自己那条自治会话的历史，**不得**成为无关运行
    /// （例如用户刚发的交互式请求）的终止理由——此前正是在这里把每个交互式
    /// 请求都判成 `not_armed` 直接掐死。护栏一旦授予，运行中的人工解除激活
    /// （disarm）仍会被 [`Self::begin_round`] 如实拦下。
    pub fn from_registry(registry: &GoalRegistry) -> Self {
        Self {
            expected_revision: if registry.is_armed() {
                registry.revision()
            } else {
                None
            },
        }
    }

    pub fn from_global() -> Self {
        let registry = global().lock().unwrap_or_else(|e| e.into_inner());
        Self::from_registry(&registry)
    }

    pub fn expected_revision(&self) -> Option<u64> {
        self.expected_revision
    }

    /// Atomically reserve the next goal round, returning a terminal reason when
    /// the goal is paused, blocked, complete, stale, disarmed, or capped.
    pub fn begin_round(&mut self, registry: &mut GoalRegistry) -> GoalGuardDecision {
        let Some(expected) = self.expected_revision else {
            return GoalGuardDecision::ContinueLegacy;
        };
        let Some(goal) = registry.get() else {
            return GoalGuardDecision::Stopped {
                reason: GoalStopReason::MissingGoal,
                revision: None,
                rounds_started: None,
            };
        };
        // Terminal phases are authoritative even if their transition bumped
        // revision; callers need the actionable terminal reason, not a generic
        // stale result after a user pause/block/complete action.
        let phase_reason = match goal.phase {
            GoalPhase::Paused => Some(GoalStopReason::Paused),
            GoalPhase::Blocked => Some(GoalStopReason::Blocked),
            GoalPhase::Complete => Some(GoalStopReason::Complete),
            GoalPhase::Active => None,
        };
        if let Some(reason) = phase_reason {
            return stopped(reason, goal);
        }
        if goal.revision != expected {
            return stopped(GoalStopReason::StaleRevision, goal);
        }
        if !registry.is_armed() {
            return stopped(GoalStopReason::NotArmed, goal);
        }
        if !registry.begin_round_at(expected) {
            let current = registry.get();
            return current.map_or(
                GoalGuardDecision::Stopped {
                    reason: GoalStopReason::MissingGoal,
                    revision: None,
                    rounds_started: None,
                },
                |g| {
                    if g.revision != expected {
                        stopped(GoalStopReason::StaleRevision, g)
                    } else {
                        stopped(GoalStopReason::MaxGoalRounds, g)
                    }
                },
            );
        }
        let reserved = registry
            .get()
            .expect("successful round reservation keeps goal");
        self.expected_revision = Some(reserved.revision);
        GoalGuardDecision::Continue {
            revision: reserved.revision,
            rounds_started: reserved.rounds_started,
        }
    }

    pub fn begin_global(&mut self) -> GoalGuardDecision {
        let mut registry = global().lock().unwrap_or_else(|e| e.into_inner());
        self.begin_round(&mut registry)
    }
}

fn stopped(reason: GoalStopReason, goal: &Goal) -> GoalGuardDecision {
    GoalGuardDecision::Stopped {
        reason,
        revision: Some(goal.revision),
        rounds_started: Some(goal.rounds_started),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试注册表：注入线程局部配置目录覆盖，夹具绝不写进用户真实
    /// `~/.marguerite/goal.db`（这些夹具曾污染真实库 978 行）。
    fn registry(max: Option<u32>) -> GoalRegistry {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_guard_test_{}_{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir);
        let mut registry = GoalRegistry::new();
        registry.create("guard objective", max).unwrap();
        registry
    }

    static NEXT_DIR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[test]
    fn max_rounds_returns_structured_terminal() {
        let mut registry = registry(Some(1));
        let mut guard = GoalRunnerGuard::from_registry(&registry);
        assert!(matches!(
            guard.begin_round(&mut registry),
            GoalGuardDecision::Continue { .. }
        ));
        assert!(matches!(
            guard.begin_round(&mut registry),
            GoalGuardDecision::Stopped {
                reason: GoalStopReason::MaxGoalRounds,
                ..
            }
        ));
        assert_eq!(registry.get().unwrap().rounds_started, 1);
    }

    #[test]
    fn paused_blocked_complete_do_not_continue() {
        for phase in [GoalPhase::Paused, GoalPhase::Blocked, GoalPhase::Complete] {
            let mut registry = registry(None);
            let mut guard = GoalRunnerGuard::from_registry(&registry);
            match phase {
                GoalPhase::Paused => {
                    registry.pause().unwrap();
                }
                GoalPhase::Complete => {
                    registry.complete().unwrap();
                }
                GoalPhase::Blocked => {
                    for _ in 0..super::super::MIN_BLOCKED_ROUNDS {
                        registry.begin_round();
                    }
                    registry.block("blocked").unwrap();
                }
                GoalPhase::Active => unreachable!(),
            }
            let decision = guard.begin_round(&mut registry);
            let expected = match phase {
                GoalPhase::Paused => GoalStopReason::Paused,
                GoalPhase::Blocked => GoalStopReason::Blocked,
                GoalPhase::Complete => GoalStopReason::Complete,
                GoalPhase::Active => unreachable!(),
            };
            assert!(matches!(
                decision,
                GoalGuardDecision::Stopped { reason, .. } if reason == expected
            ));
        }
    }

    #[test]
    fn stale_revision_does_not_consume_round() {
        let mut registry = registry(Some(3));
        let mut guard = GoalRunnerGuard::from_registry(&registry);
        registry.begin_round();
        let rounds = registry.get().unwrap().rounds_started;
        assert!(matches!(
            guard.begin_round(&mut registry),
            GoalGuardDecision::Stopped {
                reason: GoalStopReason::StaleRevision,
                ..
            }
        ));
        assert_eq!(registry.get().unwrap().rounds_started, rounds);
    }

    #[test]
    fn no_goal_preserves_legacy_path() {
        let mut registry = GoalRegistry::new();
        let mut guard = GoalRunnerGuard::from_registry(&registry);
        assert_eq!(
            guard.begin_round(&mut registry),
            GoalGuardDecision::ContinueLegacy
        );
    }

    /// 回归：会话恢复后目标自动 disarmed（armed 为会话级、不落库）。
    /// 这种「存在但未激活」的残留目标不得掐死无关运行——此前每个交互式请求
    /// 都会以 `not_armed` 直接终止，用户在聊天里只看到「计划执行结束(not_armed)」。
    #[test]
    fn disarmed_goal_does_not_gate_unrelated_runs() {
        let mut reg = registry(None);
        reg.disarm();
        let mut guard = GoalRunnerGuard::from_registry(&reg);
        assert_eq!(
            guard.begin_round(&mut reg),
            GoalGuardDecision::ContinueLegacy
        );
        assert_eq!(reg.get().unwrap().rounds_started, 0, "不得消耗轮次");
    }

    /// 回归：已终态（complete/blocked/paused）的历史目标同样不再门禁无关运行。
    #[test]
    fn terminal_goal_before_guard_does_not_gate_unrelated_runs() {
        for finish in [
            GoalRegistry::complete as fn(&mut GoalRegistry) -> anyhow::Result<Goal>,
            GoalRegistry::pause,
        ] {
            let mut reg = registry(None);
            finish(&mut reg).unwrap();
            let mut guard = GoalRunnerGuard::from_registry(&reg);
            assert_eq!(
                guard.begin_round(&mut reg),
                GoalGuardDecision::ContinueLegacy
            );
        }
    }

    /// 护栏授予之后的人工解除激活仍须即刻拦停（授权语义未被削弱）。
    #[test]
    fn disarm_after_engagement_stops_with_not_armed() {
        let mut reg = registry(None);
        let mut guard = GoalRunnerGuard::from_registry(&reg);
        reg.disarm();
        assert!(matches!(
            guard.begin_round(&mut reg),
            GoalGuardDecision::Stopped {
                reason: GoalStopReason::NotArmed,
                ..
            }
        ));
    }
}
