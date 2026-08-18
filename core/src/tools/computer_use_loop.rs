//! 拟人化 computer-use 视觉闭环（GUI_Agent Block 2）
//!
//! 在 C1（分块 grounding `locate`）+ C3（DPI/降采样坐标反推 + `move→click` 沉降）之上，
//! 加「观察 → 思考 → 动作 → 拟人等待 → 再观察 → 校验」的人类化节奏，让 GUI 操控更稳、更像人：
//!
//! - **人类化时序**（`HumanizedTiming`）：动作后基础等待 + 抖动 + 逐字输入延迟（打字像人）；
//! - **思考停顿**（`think_before_act`）：动作前短暂停顿，模拟「先看再点」；
//! - **再观察校验**（`verify_after_act`）：动作后等待 → 重新截图/观测 → 校验是否生效；
//! - **失败重试退避**（`LoopPolicy`）：执行失败或校验失败均按指数退避重试，直至成功或耗尽。
//!
//! 与执行层解耦：真实动作由 `LoopBackend` 注入（生产即 `computer_use::dispatch`，见
//! [`ui_action_to_dispatch_args`] 映射到其 action schema）；测试注入桩后端 + 记录型延迟器，
//! 无需真实屏幕 / LLM 即可验证闭环逻辑与拟人节奏。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// 一个高层 computer-use 动作（轻量、可序列化；映射到 `computer_use::dispatch` 的 action）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    Click { x: i64, y: i64 },
    DoubleClick { x: i64, y: i64 },
    RightClick { x: i64, y: i64 },
    TypeText { text: String },
    Key { key: String },
    Scroll { x: i64, y: i64, dx: i64, dy: i64 },
    Move { x: i64, y: i64 },
    /// 定位（对应 C1 `locate`：截图 → 视觉模型坐标 → click，可选自动点击）
    Locate { description: String, auto_click: bool },
}

/// 人类化时序配置。
#[derive(Debug, Clone)]
pub struct HumanizedTiming {
    /// 动作后基础等待（毫秒）
    pub base_ms: u64,
    /// 抖动上限（毫秒）：实际等待 = base + hash(action) % (jitter+1)（确定性，便于测试）
    pub jitter_ms: u64,
    /// 每次按字延迟（毫秒）：`TypeText` 按文本长度叠加，模拟逐字输入
    pub per_keystroke_ms: u64,
    /// 动作前「思考」停顿（毫秒，`think_before_act` 时生效）
    pub think_ms: u64,
}

impl Default for HumanizedTiming {
    fn default() -> Self {
        Self {
            base_ms: 120,
            jitter_ms: 80,
            per_keystroke_ms: 30,
            think_ms: 200,
        }
    }
}

/// 循环策略。
#[derive(Debug, Clone)]
pub struct LoopPolicy {
    /// 单动作最大重试次数（不含首次尝试）
    pub max_retries: u32,
    /// 退避基数（毫秒）
    pub backoff_base_ms: u64,
    /// 退避因子（第 n 次重试等待 = base × factor^(n-1)）
    pub backoff_factor: f64,
    /// 动作前是否「思考」停顿
    pub think_before_act: bool,
    /// 动作后是否再观察校验
    pub verify_after_act: bool,
}

impl Default for LoopPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff_base_ms: 300,
            backoff_factor: 2.0,
            think_before_act: true,
            verify_after_act: true,
        }
    }
}

/// 可注入的延迟器（生产用真实 sleep；测试用记录器，不真正 sleep）。
pub trait Delayer: Send + Sync {
    fn delay(&self, ms: u64);
}

/// 真实延迟器（`std::thread::sleep`）。
pub struct RealDelayer;
impl Delayer for RealDelayer {
    fn delay(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// 记录型延迟器（测试用）：累计每次延迟，不真正 sleep。
#[derive(Default)]
pub struct RecordingDelayer {
    pub delays: Mutex<Vec<u64>>,
}
impl Delayer for RecordingDelayer {
    fn delay(&self, ms: u64) {
        self.delays.lock().unwrap_or_else(|e| e.into_inner()).push(ms);
    }
}

/// 后端抽象：提供「观察 / 执行 / 校验」三能力，便于注入真实 dispatch 或测试桩。
pub trait LoopBackend: Send + Sync {
    /// 观察当前屏幕（返回可供校验的观测快照）。
    fn observe(&self) -> ObserveSnapshot;
    /// 执行一个动作（内部应已含 `move→click` 等人类化前置，见 C3）。
    fn act(&self, action: &UiAction) -> ActOutcome;
    /// 校验某动作是否生效（基于最新观测）。
    fn verify(&self, action: &UiAction) -> bool;
}

/// 观测快照（真实后端可放截图路径 / 坐标；此处仅承载原始串）。
#[derive(Debug, Clone, Default)]
pub struct ObserveSnapshot {
    pub raw: String,
}

/// 动作执行结果。
#[derive(Debug, Clone)]
pub struct ActOutcome {
    pub ok: bool,
    pub detail: String,
}

/// 单次动作的执行报告。
#[derive(Debug, Clone)]
pub struct StepReport {
    pub action: UiAction,
    /// 重试次数（0 = 首次即成功）
    pub retries: u32,
    pub success: bool,
    /// 该步骤累计拟人延迟（毫秒）
    pub total_delay_ms: u64,
}

/// 整个视觉闭环的报告。
#[derive(Debug, Clone)]
pub struct LoopReport {
    pub steps: Vec<StepReport>,
    pub all_success: bool,
    pub total_delay_ms: u64,
}

/// 拟人化视觉闭环执行器。
pub struct VisualLoop<'a> {
    timing: HumanizedTiming,
    policy: LoopPolicy,
    delayer: &'a dyn Delayer,
}

impl<'a> VisualLoop<'a> {
    pub fn new(timing: HumanizedTiming, policy: LoopPolicy, delayer: &'a dyn Delayer) -> Self {
        Self {
            timing,
            policy,
            delayer,
        }
    }

    /// 计算某动作的拟人等待时长（含逐字延迟与确定性抖动）。
    fn action_delay(&self, action: &UiAction) -> u64 {
        let text_len = match action {
            UiAction::TypeText { text } => text.chars().count(),
            _ => 0,
        };
        let jitter = if self.timing.jitter_ms > 0 {
            hash_str(&format!("{action:?}")) % (self.timing.jitter_ms + 1)
        } else {
            0
        };
        self.timing.base_ms + jitter + text_len as u64 * self.timing.per_keystroke_ms
    }

    /// 第 `retry_index`（从 0 计）次重试的退避等待。
    fn backoff(&self, retry_index: u32) -> u64 {
        (self.policy.backoff_base_ms as f64
            * self.policy.backoff_factor.powi(retry_index as i32)) as u64
    }

    /// 执行一个动作计划（按序），返回报告。每个动作独立重试；任一动作耗尽重试仍失败则
    /// `all_success=false`，但后续动作仍继续执行（便于整体报告）。
    pub fn run(&self, plan: &[UiAction], backend: &dyn LoopBackend) -> LoopReport {
        let mut steps = Vec::new();
        let mut total_delay = 0u64;
        for action in plan {
            let mut retries = 0u32;
            let success;
            let mut step_delay = 0u64;
            'retry: loop {
                if self.policy.think_before_act {
                    let d = self.timing.think_ms;
                    self.delayer.delay(d);
                    step_delay += d;
                    total_delay += d;
                }
                let outcome = backend.act(action);
                if !outcome.ok {
                    // 执行失败：退避重试
                    if retries < self.policy.max_retries {
                        let backoff = self.backoff(retries);
                        self.delayer.delay(backoff);
                        step_delay += backoff;
                        total_delay += backoff;
                        retries += 1;
                        continue 'retry;
                    }
                    success = false;
                    break 'retry;
                }
                // 执行成功
                if self.policy.verify_after_act {
                    let d = self.action_delay(action);
                    self.delayer.delay(d);
                    step_delay += d;
                    total_delay += d;
                    // 再观察 + 校验
                    let _ = backend.observe();
                    if backend.verify(action) {
                        success = true;
                        break 'retry;
                    }
                    // 校验失败：退避重试
                    if retries < self.policy.max_retries {
                        let backoff = self.backoff(retries);
                        self.delayer.delay(backoff);
                        step_delay += backoff;
                        total_delay += backoff;
                        retries += 1;
                        continue 'retry;
                    }
                    success = false;
                    break 'retry;
                }
                // 不校验：执行成功即视为成功
                let d = self.action_delay(action);
                self.delayer.delay(d);
                step_delay += d;
                total_delay += d;
                success = true;
                break 'retry;
            }
            steps.push(StepReport {
                action: action.clone(),
                retries,
                success,
                total_delay_ms: step_delay,
            });
        }
        let all_success = steps.iter().all(|s| s.success);
        LoopReport {
            steps,
            all_success,
            total_delay_ms: total_delay,
        }
    }
}

/// 把高层 `UiAction` 映射为 `computer_use::dispatch` 的参数表（连接到 C1/C3 真实执行层）。
///
/// 动作名与 `register_computer_use_tools` 的 schema 完全一致；`locate` 直接对接 C1 的分块
/// grounding 路径（`patch_grounding` 由调用方另行注入）。
#[must_use]
pub fn ui_action_to_dispatch_args(action: &UiAction) -> HashMap<String, String> {
    let mut m = HashMap::new();
    match action {
        UiAction::Click { x, y } => {
            m.insert("action".into(), "click".into());
            m.insert("x".into(), x.to_string());
            m.insert("y".into(), y.to_string());
        }
        UiAction::DoubleClick { x, y } => {
            m.insert("action".into(), "double_click".into());
            m.insert("x".into(), x.to_string());
            m.insert("y".into(), y.to_string());
        }
        UiAction::RightClick { x, y } => {
            m.insert("action".into(), "right_click".into());
            m.insert("x".into(), x.to_string());
            m.insert("y".into(), y.to_string());
        }
        UiAction::TypeText { text } => {
            m.insert("action".into(), "type".into());
            m.insert("text".into(), text.clone());
        }
        UiAction::Key { key } => {
            m.insert("action".into(), "key".into());
            m.insert("key".into(), key.clone());
        }
        UiAction::Scroll { x, y, dx, dy } => {
            m.insert("action".into(), "scroll".into());
            m.insert("x".into(), x.to_string());
            m.insert("y".into(), y.to_string());
            m.insert("dx".into(), dx.to_string());
            m.insert("dy".into(), dy.to_string());
        }
        UiAction::Move { x, y } => {
            m.insert("action".into(), "move".into());
            m.insert("x".into(), x.to_string());
            m.insert("y".into(), y.to_string());
        }
        UiAction::Locate {
            description,
            auto_click,
        } => {
            m.insert("action".into(), "locate".into());
            m.insert("description".into(), description.clone());
            m.insert("auto_click".into(), auto_click.to_string());
        }
    }
    m
}

/// 确定性 FNV-1a 64（用于抖动取模，避免测试中随机性）。
fn hash_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 脚本化后端：可配置「前 N 次执行失败 / 前 M 次校验失败」，并统计 observe 调用。
    struct ScriptedBackend {
        act_fails_first: usize,
        verify_fails_first: usize,
        observe_count: Mutex<usize>,
        act_calls: Mutex<usize>,
        verify_calls: Mutex<usize>,
    }

    impl LoopBackend for ScriptedBackend {
        fn observe(&self) -> ObserveSnapshot {
            *self.observe_count.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            ObserveSnapshot {
                raw: "obs".into(),
            }
        }
        fn act(&self, _a: &UiAction) -> ActOutcome {
            let n = {
                let mut g = self.act_calls.lock().unwrap_or_else(|e| e.into_inner());
                *g += 1;
                *g
            };
            ActOutcome {
                ok: n > self.act_fails_first,
                detail: String::new(),
            }
        }
        fn verify(&self, _a: &UiAction) -> bool {
            let n = {
                let mut g = self.verify_calls.lock().unwrap_or_else(|e| e.into_inner());
                *g += 1;
                *g
            };
            n > self.verify_fails_first
        }
    }

    #[test]
    fn humanized_delay_scales_with_keystrokes() {
        // 关闭抖动以得到确定性断言：打字延迟应严格等于 点击延迟 + 逐字延迟
        let timing = HumanizedTiming {
            jitter_ms: 0,
            ..HumanizedTiming::default()
        };
        let delayer = RecordingDelayer::default();
        let loop_ = VisualLoop::new(timing, LoopPolicy::default(), &delayer);
        let click = UiAction::Click { x: 1, y: 1 };
        let typed = UiAction::TypeText {
            text: "hello".into(),
        };
        let d_click = loop_.action_delay(&click);
        let d_type = loop_.action_delay(&typed);
        // 逐字延迟：5 字符 × 30ms = 150ms 额外
        assert!(
            d_type >= d_click + 5 * 30,
            "打字应比点击多 ~150ms 逐字延迟 (click={d_click} type={d_type})"
        );
    }

    #[test]
    fn loop_succeeds_on_first_try() {
        let delayer = RecordingDelayer::default();
        let loop_ = VisualLoop::new(
            HumanizedTiming::default(),
            LoopPolicy::default(),
            &delayer,
        );
        let backend = ScriptedBackend {
            act_fails_first: 0,
            verify_fails_first: 0,
            observe_count: Mutex::new(0),
            act_calls: Mutex::new(0),
            verify_calls: Mutex::new(0),
        };
        let report = loop_.run(
            &[UiAction::Click { x: 10, y: 20 }],
            &backend,
        );
        assert!(report.all_success);
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].retries, 0, "首次应成功，重试 0");
        assert!(report.steps[0].success);
        // 延迟应包含 思考 + 动作后等待
        let delays = delayer.delays.lock().unwrap();
        assert!(
            delays.iter().any(|d| *d == 200),
            "应包含 think_ms=200 停顿"
        );
    }

    #[test]
    fn loop_retries_then_succeeds_with_backoff() {
        let delayer = RecordingDelayer::default();
        let loop_ = VisualLoop::new(
            HumanizedTiming::default(),
            LoopPolicy {
                max_retries: 3,
                backoff_base_ms: 300,
                backoff_factor: 2.0,
                think_before_act: false,
                verify_after_act: true,
            },
            &delayer,
        );
        // 执行前 2 次失败，第 3 次成功；校验始终成功
        let backend = ScriptedBackend {
            act_fails_first: 2,
            verify_fails_first: 0,
            observe_count: Mutex::new(0),
            act_calls: Mutex::new(0),
            verify_calls: Mutex::new(0),
        };
        let report = loop_.run(&[UiAction::Click { x: 1, y: 1 }], &backend);
        assert!(report.all_success);
        assert_eq!(report.steps[0].retries, 2, "应重试 2 次后成功");
        // 退避：第1次重试 300ms，第2次 600ms（因子 2）
        let delays = delayer.delays.lock().unwrap();
        assert!(delays.contains(&300), "应包含 300ms 退避");
        assert!(delays.contains(&600), "应包含 600ms 退避");
    }

    #[test]
    fn loop_fails_after_max_retries() {
        let delayer = RecordingDelayer::default();
        let loop_ = VisualLoop::new(
            HumanizedTiming::default(),
            LoopPolicy {
                max_retries: 3,
                backoff_base_ms: 100,
                backoff_factor: 2.0,
                think_before_act: false,
                verify_after_act: false,
            },
            &delayer,
        );
        let backend = ScriptedBackend {
            act_fails_first: usize::MAX, // 永远执行失败
            verify_fails_first: 0,
            observe_count: Mutex::new(0),
            act_calls: Mutex::new(0),
            verify_calls: Mutex::new(0),
        };
        let report = loop_.run(&[UiAction::Click { x: 1, y: 1 }], &backend);
        assert!(!report.all_success);
        assert_eq!(report.steps[0].retries, 3, "应耗尽 3 次重试");
        assert!(!report.steps[0].success);
    }

    #[test]
    fn verify_after_act_triggers_reobserve_and_retry() {
        let delayer = RecordingDelayer::default();
        let loop_ = VisualLoop::new(
            HumanizedTiming::default(),
            LoopPolicy {
                max_retries: 2,
                backoff_base_ms: 100,
                backoff_factor: 2.0,
                think_before_act: false,
                verify_after_act: true,
            },
            &delayer,
        );
        // 执行成功，但校验前 1 次失败（即第 2 次校验才过）
        let backend = ScriptedBackend {
            act_fails_first: 0,
            verify_fails_first: 1,
            observe_count: Mutex::new(0),
            act_calls: Mutex::new(0),
            verify_calls: Mutex::new(0),
        };
        let report = loop_.run(&[UiAction::Click { x: 1, y: 1 }], &backend);
        assert!(report.all_success, "校验重试后应成功");
        assert_eq!(report.steps[0].retries, 1, "校验失败应触发 1 次重试");
        assert_eq!(*backend.observe_count.lock().unwrap(), 2, "应再观察 2 次（含重试）");
    }

    #[test]
    fn ui_action_maps_to_dispatch_schema() {
        let click = ui_action_to_dispatch_args(&UiAction::Click { x: 5, y: 6 });
        assert_eq!(click.get("action").map(String::as_str), Some("click"));
        assert_eq!(click.get("x").map(String::as_str), Some("5"));
        assert_eq!(click.get("y").map(String::as_str), Some("6"));

        let typed = ui_action_to_dispatch_args(&UiAction::TypeText {
            text: "hi".into(),
        });
        assert_eq!(typed.get("action").map(String::as_str), Some("type"));
        assert_eq!(typed.get("text").map(String::as_str), Some("hi"));

        let locate = ui_action_to_dispatch_args(&UiAction::Locate {
            description: "提交按钮".into(),
            auto_click: true,
        });
        assert_eq!(locate.get("action").map(String::as_str), Some("locate"));
        assert_eq!(
            locate.get("description").map(String::as_str),
            Some("提交按钮")
        );
        assert_eq!(locate.get("auto_click").map(String::as_str), Some("true"));
    }
}
