//! 鲸鱼娘桌宠原生状态机（Track D6，对译 `dsh-deep-whale`）。
//!
//! 纯逻辑、无渲染、可单测。能量夹紧 `0..=100`；低能转 `Sleepy`；
//! 互动（`pet`）/ 投喂（`feed`）回能并升 `Happy`。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 鲸鱼娘心情。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetMood {
    /// 空闲。
    Idle,
    /// 开心。
    Happy,
    /// 困倦（低能）。
    Sleepy,
    /// 工作中。
    Working,
}

/// 鲸鱼娘状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PetState {
    /// 当前心情。
    pub mood: PetMood,
    /// 能量（夹紧 `0..=100`）。
    pub energy: u8,
}

/// 能量低于此阈值视为困倦。
const LOW_ENERGY: u8 = 20;

/// 夹紧能量到 `0..=100`。
fn clamp_energy(e: i32) -> u8 {
    e.clamp(0, 100) as u8
}

/// 推进时间：能量按流逝秒数衰减；低能转 `Sleepy`。
pub fn tick(state: &PetState, dt_secs: u64) -> PetState {
    // 每 10 秒掉 1 点能量（整数化，避免浮点）。
    let decay = (dt_secs / 10) as i32;
    let energy = clamp_energy(state.energy as i32 - decay);
    let mood = if energy < LOW_ENERGY {
        PetMood::Sleepy
    } else {
        state.mood
    };
    PetState { mood, energy }
}

/// 互动：升 `Happy` 并回能。
pub fn pet(state: &PetState) -> PetState {
    PetState {
        mood: PetMood::Happy,
        energy: clamp_energy(state.energy as i32 + 15),
    }
}

/// 投喂：回能并升 `Happy`。
pub fn feed(state: &PetState) -> PetState {
    PetState {
        mood: PetMood::Happy,
        energy: clamp_energy(state.energy as i32 + 25),
    }
}

// ───────────────────────────── 持久化层（网关 RPC 使用）─────────────────────────────

/// 桌宠持久化状态（落盘 `data_dir/pet_state.json`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetStore {
    pub mood: String,
    pub energy: u8,
    pub affection: u32,
    pub last_ms: u64,
}

/// 返回给前端的桌宠状态视图。
#[derive(Debug, Clone, Serialize)]
pub struct PetView {
    pub mood: String,
    pub energy: u8,
    pub affection: u32,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn store_path() -> PathBuf {
    // 单测隔离：注入的临时配置目录优先（避免读写真实用户数据）
    #[cfg(test)]
    {
        if let Some(dir) = crate::persistence::config::Config::current_config_dir_override() {
            return dir.join("pet_state.json");
        }
    }
    crate::persistence::config::Config::data_dir().join("pet_state.json")
}

fn load_store() -> PetStore {
    let default = || PetStore {
        mood: "idle".into(),
        energy: 100,
        affection: 0,
        last_ms: now_ms(),
    };
    match std::fs::read_to_string(store_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| default()),
        Err(_) => default(),
    }
}

fn save_store(p: &PetStore) {
    if let Ok(json) = serde_json::to_string(p) {
        let _ = std::fs::write(store_path(), json);
    }
}

fn mood_from_str(s: &str) -> PetMood {
    match s {
        "happy" => PetMood::Happy,
        "sleepy" => PetMood::Sleepy,
        "working" => PetMood::Working,
        _ => PetMood::Idle,
    }
}

fn mood_str(m: PetMood) -> &'static str {
    match m {
        PetMood::Idle => "idle",
        PetMood::Happy => "happy",
        PetMood::Sleepy => "sleepy",
        PetMood::Working => "working",
    }
}

/// 读取桌宠状态（按流逝时间 tick 衰减），供网关 RPC `pet.status`。
pub fn status() -> PetView {
    let mut store = load_store();
    let dt = now_ms().saturating_sub(store.last_ms) / 1000;
    let cur = PetState {
        mood: mood_from_str(&store.mood),
        energy: store.energy,
    };
    let next = tick(&cur, dt);
    store.energy = next.energy;
    store.mood = mood_str(next.mood).to_string();
    store.last_ms = now_ms();
    save_store(&store);
    PetView {
        mood: store.mood.clone(),
        energy: store.energy,
        affection: store.affection,
    }
}

/// 互动（pet/feed），供网关 RPC `pet.action`。
pub fn interact(action: &str) -> Result<PetView, String> {
    let mut store = load_store();
    let cur = PetState {
        mood: mood_from_str(&store.mood),
        energy: store.energy,
    };
    let next = match action {
        "pet" => pet(&cur),
        "feed" => feed(&cur),
        other => return Err(format!("未知互动: {other}")),
    };
    store.energy = next.energy;
    store.mood = mood_str(next.mood).to_string();
    store.affection = store.affection.saturating_add(1);
    store.last_ms = now_ms();
    save_store(&store);
    Ok(PetView {
        mood: store.mood.clone(),
        energy: store.energy,
        affection: store.affection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_decays_and_sleeps() {
        let s = PetState {
            mood: PetMood::Idle,
            energy: 100,
        };
        let after = tick(&s, 120); // 120s -> -12
        assert!(
            after.energy < s.energy,
            "tick 必须衰减能量（变异测试锚点：若衰减被删，此断言失败）"
        );
        assert_eq!(after.energy, 88);

        // 低能转 Sleepy
        let low = PetState {
            mood: PetMood::Idle,
            energy: 30,
        };
        let after_low = tick(&low, 200); // 200s -> -20 -> 10
        assert_eq!(after_low.energy, 10);
        assert_eq!(after_low.mood, PetMood::Sleepy);
    }

    #[test]
    fn pet_and_feed_restore() {
        let s = PetState {
            mood: PetMood::Sleepy,
            energy: 10,
        };
        let p = pet(&s);
        assert_eq!(p.mood, PetMood::Happy);
        assert!(p.energy > s.energy);

        let f = feed(&s);
        assert_eq!(f.mood, PetMood::Happy);
        assert!(f.energy > s.energy);
        assert!(f.energy >= p.energy, "投喂回能应不少于互动");
    }

    #[test]
    fn energy_clamped() {
        let s = PetState {
            mood: PetMood::Idle,
            energy: 100,
        };
        let f = feed(&s); // 100 + 25 夹紧 100
        assert_eq!(f.energy, 100);

        let zero = PetState {
            mood: PetMood::Idle,
            energy: 0,
        };
        let t = tick(&zero, 1000); // 不低于 0
        assert_eq!(t.energy, 0);
    }
}
