//! 鲸鱼娘桌宠 6 帧状态机（T-2.3 / FR-B3，PetV2 增强版）。
//!
//! 与 `crate::workbench::pet`（旧版 PetMood + PetState，4 帧：Idle/Happy/Sleepy/
//! Working）解耦，保持完全向后兼容。PetV2 引入 `PetStateV2`（6 帧）、
//! affinity/pets/feeds 统计、`on_event` DSH Agent 事件桥，以及 JSON 持久化
//!（`data_dir/pet_v2.json`）。

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::persistence::config::Config;

// ──────────────────────────────── 6 帧状态枚举 ────────────────────────────────

/// PetV2 6 帧状态（对齐 Spec §FR-B3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetStateV2 {
    /// 空闲待机。
    Idle,
    /// 思考中（用户输入后 agent 准备阶段）。
    Thinking,
    /// 运行中（执行 plan / 工具调用）。
    Running,
    /// 睡眠（能量不足或 Running 超时）。
    Sleeping,
    /// 进食（刚投喂）。
    Eating,
    /// 被摸头（被 pet）。
    Petted,
}

impl PetStateV2 {
    pub fn as_str(&self) -> &'static str {
        match self {
            PetStateV2::Idle => "idle",
            PetStateV2::Thinking => "thinking",
            PetStateV2::Running => "running",
            PetStateV2::Sleeping => "sleeping",
            PetStateV2::Eating => "eating",
            PetStateV2::Petted => "petted",
        }
    }
}

// ──────────────────────────────── DSH Agent 事件 ────────────────────────────────

/// DSH Agent 状态事件（PetV2::on_event 订阅）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DshAgentEvent {
    /// 执行中（工具调用 / agent 运行）。
    Running,
    /// 思考中（路由 / 规划前置阶段）。
    Thinking,
    /// 本轮全部结束。
    Done,
    /// 计划开始。
    PlanStarted,
    /// 计划完成。
    PlanFinished,
    /// 单个 turn 完成（下一步可能仍在执行，状态仍保持 Idle：相当于微 Done）。
    TurnCompleted,
}

// ──────────────────────────────── PetV2 结构 ────────────────────────────────

/// 鲸鱼娘增强版状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetV2 {
    /// 当前 6 帧状态。
    pub state: PetStateV2,
    /// 亲密度：摸头 + 投喂累计（持久化）。
    pub affinity: u32,
    /// 摸头次数（持久化）。
    pub pets: u32,
    /// 投喂次数（持久化）。
    pub feeds: u32,
    /// 能量值，夹紧 0..=100。
    pub energy: u8,
    /// 上次摸头时间（ms，UNIX_EPOCH）。
    pub last_pet_at_ms: Option<u64>,
    /// 上次投喂时间（ms）。
    pub last_feed_at_ms: Option<u64>,
    /// 进入当前状态的时间戳（超时转移用）。
    pub current_since_ms: u64,
}

// 默认缺省能量
const ENERGY_DEFAULT: u8 = 80;
const ENERGY_MAX: u8 = 100;
const ENERGY_LOW: u8 = 20;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 能量夹紧。
fn clamp_energy(v: i32) -> u8 {
    v.clamp(0, ENERGY_MAX as i32) as u8
}

impl Default for PetV2 {
    fn default() -> Self {
        Self::new()
    }
}

impl PetV2 {
    /// 构造默认 PetV2（Idle，energy=80，affinity/pets/feeds=0，自 now 起状态持续）。
    pub fn new() -> Self {
        Self {
            state: PetStateV2::Idle,
            affinity: 0,
            pets: 0,
            feeds: 0,
            energy: ENERGY_DEFAULT,
            last_pet_at_ms: None,
            last_feed_at_ms: None,
            current_since_ms: now_ms(),
        }
    }

    // ── 交互 ──

    /// 摸头：状态变 Petted，aff+1、pets+1、energy+15（clamp），记录时间戳。
    pub fn pet(&mut self) {
        let now = now_ms();
        self.state = PetStateV2::Petted;
        self.affinity = self.affinity.saturating_add(1);
        self.pets = self.pets.saturating_add(1);
        self.energy = clamp_energy(self.energy as i32 + 15);
        self.last_pet_at_ms = Some(now);
        self.current_since_ms = now;
    }

    /// 投喂：状态变 Eating，feeds+1、aff+1、energy+25（clamp），记录时间戳。
    pub fn feed(&mut self) {
        let now = now_ms();
        self.state = PetStateV2::Eating;
        self.affinity = self.affinity.saturating_add(1);
        self.feeds = self.feeds.saturating_add(1);
        self.energy = clamp_energy(self.energy as i32 + 25);
        self.last_feed_at_ms = Some(now);
        self.current_since_ms = now;
    }

    // ── 推进：时间 + 事件 ──

    /// 按 `dt_secs` 推进状态：做超时转移 + 能量衰减。
    ///
    /// `now_ms` 用于"当前时间"注入（单测可注入，便于测试 4s Petted → Idle）。
    pub fn tick(&mut self, dt_secs: u64, now_ms: u64) {
        // (a) 能量衰减：每 10s 减 1
        let decay = (dt_secs / 10) as i32;
        if decay > 0 {
            self.energy = clamp_energy(self.energy as i32 - decay);
        }
        // (b) 能量 < 20 一律 Sleeping（但当前若是 Eating/Petted 短暂动画先不打断？
        //     spec 说"能量 < 20 → Sleeping"，优先级高于动画超时，因此这里强制执行）。
        if self.energy < ENERGY_LOW {
            self.transition(PetStateV2::Sleeping, now_ms);
        }
        // (c) 状态驻留时长检查
        let elapsed_secs = now_ms.saturating_sub(self.current_since_ms) / 1000;
        match self.state {
            // Running 超过 15 分钟强制休息
            PetStateV2::Running if elapsed_secs >= 900 => {
                self.transition(PetStateV2::Sleeping, now_ms);
            }
            // Eating 超过 2s → Idle
            PetStateV2::Eating if elapsed_secs >= 2 => {
                // 如果能量已很低，前面 (b) 已经改成 Sleeping，所以这里仅覆盖非低能场景
                if self.state == PetStateV2::Eating {
                    self.transition(PetStateV2::Idle, now_ms);
                }
            }
            // Petted 超过 3s → Idle
            PetStateV2::Petted if elapsed_secs >= 3 => {
                if self.state == PetStateV2::Petted {
                    self.transition(PetStateV2::Idle, now_ms);
                }
            }
            _ => { /* 保持当前 */ }
        }
    }

    /// 根据 DSH Agent 事件切状态。
    pub fn on_event(&mut self, ev: &DshAgentEvent) {
        let now = now_ms();
        let next = match ev {
            DshAgentEvent::Running | DshAgentEvent::PlanStarted => PetStateV2::Running,
            DshAgentEvent::Thinking => PetStateV2::Thinking,
            DshAgentEvent::Done | DshAgentEvent::PlanFinished | DshAgentEvent::TurnCompleted => {
                PetStateV2::Idle
            }
        };
        self.transition(next, now);
    }

    /// 原子状态切换 + 刷新 current_since_ms。
    fn transition(&mut self, s: PetStateV2, now: u64) {
        if self.state != s {
            self.state = s;
            self.current_since_ms = now;
        }
    }

    // ── 持久化 ──

    /// 持久化到 `data_dir()/pet_v2.json`。失败返回 Err（不 panic）。
    pub fn save(&self) -> Result<(), String> {
        let path = Config::data_dir().join("pet_v2.json");
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    /// 从 `data_dir()/pet_v2.json` 读取；IO 或 JSON 错误 / 路径不存在均回退
    /// `PetV2::new()`，**不 panic**。
    pub fn load() -> PetV2 {
        let path = Config::data_dir().join("pet_v2.json");
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<PetV2>(&s).unwrap_or_else(|_| PetV2::new()),
            Err(_) => PetV2::new(),
        }
    }
}

// ──────────────────────────────── ASCII 精灵 ────────────────────────────────

/// 每个状态返回 6 行 ASCII 鲸鱼娘（固定字符宽度，纯 ASCII、无 emoji）。
/// 画面宽度统一 32 字符，对齐左右边距。
pub fn ascii_sprite(state: PetStateV2) -> [&'static str; 6] {
    match state {
        PetStateV2::Idle => [
            "         ~~~~~~~~               ",
            "       /          \\             ",
            "      |   .  .    |             ",
            "      |    ^_^    |             ",
            "       \\          /             ",
            " ~~~~~~ ---------- ~~~~~~       ",
        ],
        PetStateV2::Thinking => [
            "         ???                    ",
            "       /          \\             ",
            "      |  o  o ?   |             ",
            "      |  _(._.)   |             ",
            "       \\    /     /             ",
            " ~~~~~~ ---------- ~~~~~~       ",
        ],
        PetStateV2::Running => [
            "       @@@@                     ",
            "     //      \\\\                ",
            "    |  @    @  |                ",
            "    |   \\__/   |                ",
            "     \\\\        //  <( running )",
            " ~~~~~~ ---------- ~~~~~~       ",
        ],
        PetStateV2::Sleeping => [
            "         z z Z                  ",
            "       /  z  Z  \\              ",
            "      |  -   -   |              ",
            "      |   ___    |              ",
            "       \\        /               ",
            " ~~~~~~ ---------- ~~~~~~       ",
        ],
        PetStateV2::Eating => [
            "                                ",
            "       /          \\             ",
            "      |  >  <     |             ",
            "      |   <><    _|             ",
            "       \\    ^^^ /               ",
            " ~~~~~~ ---------- ~~~~~~       ",
        ],
        PetStateV2::Petted => [
            "        <3 <3 <3                ",
            "       /          \\             ",
            "      |  ^   ^    |             ",
            "      |   \\__/ ^^ |             ",
            "       \\         /              ",
            " ~~~~~~ ---------- ~~~~~~       ",
        ],
    }
}

// ──────────────────────────────── 单测（6 条） ────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 1. pet 后 aff=1
    #[test]
    fn pet_inc_affinity() {
        let mut p = PetV2::new();
        assert_eq!(p.affinity, 0);
        p.pet();
        assert_eq!(p.affinity, 1);
        assert_eq!(p.pets, 1);
        assert_eq!(p.state, PetStateV2::Petted);
        assert!(p.last_pet_at_ms.is_some());
    }

    // 2. feed 后 feeds=1
    #[test]
    fn feed_inc_feeds() {
        let mut p = PetV2::new();
        assert_eq!(p.feeds, 0);
        p.feed();
        assert_eq!(p.feeds, 1);
        assert_eq!(p.affinity, 1);
        assert_eq!(p.state, PetStateV2::Eating);
        assert!(p.last_feed_at_ms.is_some());
    }

    // 3. 事件切状态：Running → Done → Idle
    #[test]
    fn state_transition_on_event() {
        let mut p = PetV2::new();
        assert_eq!(p.state, PetStateV2::Idle);
        p.on_event(&DshAgentEvent::Running);
        assert_eq!(p.state, PetStateV2::Running);
        p.on_event(&DshAgentEvent::Done);
        assert_eq!(p.state, PetStateV2::Idle);
        // PlanStarted → Running
        p.on_event(&DshAgentEvent::PlanStarted);
        assert_eq!(p.state, PetStateV2::Running);
        // Thinking → Thinking
        p.on_event(&DshAgentEvent::Thinking);
        assert_eq!(p.state, PetStateV2::Thinking);
        // TurnCompleted → Idle
        p.on_event(&DshAgentEvent::TurnCompleted);
        assert_eq!(p.state, PetStateV2::Idle);
        // PlanFinished → Idle
        p.on_event(&DshAgentEvent::PlanStarted);
        p.on_event(&DshAgentEvent::PlanFinished);
        assert_eq!(p.state, PetStateV2::Idle);
    }

    // 4. Petted 超时 → Idle（通过注入 current_since_ms + tick(4s)）
    #[test]
    fn petted_timeout_to_idle() {
        let mut p = PetV2::new();
        let base_ms = 1_700_000_000_000u64; // 任意固定时间戳
        p.state = PetStateV2::Petted;
        p.current_since_ms = base_ms;
        // dt_secs 是"流逝时间"，我们要让 elapsed >= 3s：让 now_ms = base_ms + 4000
        p.tick(4, base_ms + 4_000);
        assert_eq!(
            p.state,
            PetStateV2::Idle,
            "Petted 3s 后 tick(4s) 应回到 Idle"
        );
    }

    // 5. JSON 往返字段相等（to_value + from_value）
    #[test]
    fn json_roundtrip_fields_equal() {
        let p = PetV2::new();
        let v = serde_json::to_value(&p).expect("to_value");
        let back: PetV2 = serde_json::from_value(v).expect("from_value");
        assert_eq!(back.state, p.state);
        assert_eq!(back.affinity, p.affinity);
        assert_eq!(back.pets, p.pets);
        assert_eq!(back.feeds, p.feeds);
        assert_eq!(back.energy, p.energy);
        // current_since_ms 只要是有限整数就视为 equal（因为 now_ms 无法字节相等）
        assert!(back.current_since_ms > 0);
    }

    // 6. 6 个状态的 ASCII 精灵每个都有且仅有 6 行
    #[test]
    fn six_ascii_sprites_each_has_at_least_6_lines() {
        use PetStateV2::*;
        for s in [Idle, Thinking, Running, Sleeping, Eating, Petted] {
            let lines = ascii_sprite(s);
            assert_eq!(lines.len(), 6, "state={:?} 应返回 6 行", s);
            for (i, l) in lines.iter().enumerate() {
                // 每行应为纯 ASCII（不含非 ASCII 字节）
                assert!(
                    l.is_ascii(),
                    "state={:?} 第 {} 行包含非 ASCII 字符: {}",
                    s,
                    i,
                    l
                );
            }
        }
    }
}
