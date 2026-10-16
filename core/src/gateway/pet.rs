//! 桌宠 RPC 扩展类型（V2.1-P1a：统一工作台契约 · Pet Skin/Mood/Kind/Status/Action）
//!
//! 前端 `webui/src/rpc/petContract.ts` 由本文件同构维护：Rust 类型 → serde JSON → TS 界面。
//! 无 ts-rs 依赖时，契约直接手编并在注释里标注 TS 对应结构，V2.1-V 阶段以 Playwright
//! 握手断言「字段集合对称」作为 R5 契约三同步门禁。

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

// ──────────────────────────────────────────────────────────────────────
// 枚举：皮肤/心情/种类/动作
// ──────────────────────────────────────────────────────────────────────

/// 桌宠皮肤 ID（与 WebUI `PET_SKINS` / `ELF_SPECS` 一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetSkinId {
    /// 茉莉猫（兽形 · 默认）
    Cat,
    /// 雏菊精灵 3D · 少女花嫁
    Blue,
    /// 樱花精灵 3D · 少女花嫁
    Pink,
    /// 薰衣草精灵 3D · 少女花嫁
    Purple,
    /// 四叶草精灵 3D · 少女花嫁
    Green,
}

impl PetSkinId {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cat => "cat",
            Self::Blue => "blue",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Green => "green",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Cat => "茉莉猫 3D",
            Self::Blue => "雏菊精灵 3D · 少女花嫁",
            Self::Pink => "樱花精灵 3D · 少女花嫁",
            Self::Purple => "薰衣草精灵 3D · 少女花嫁",
            Self::Green => "四叶草精灵 3D · 少女花嫁",
        }
    }

    /// 造型种类：cat=兽形，精灵=人型
    pub fn kind(&self) -> PetKind {
        match self {
            Self::Cat => PetKind::Beast,
            _ => PetKind::Humanoid,
        }
    }

    pub fn accent_hex(&self) -> u32 {
        match self {
            Self::Cat => 0x60a5fa,
            Self::Blue => 0x8ec7ef,
            Self::Pink => 0xf4a8c0,
            Self::Purple => 0xb8a0d8,
            Self::Green => 0x8ec9a0,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "blue" => Self::Blue,
            "pink" => Self::Pink,
            "purple" => Self::Purple,
            "green" => Self::Green,
            _ => Self::Cat,
        }
    }
}

/// 桌宠心情（与前端 `petContract::PetMood3D` 对齐：6 种）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetMood {
    Normal,
    Happy,
    Sleepy,
    Working,
    Excited,
    Sad,
}

impl PetMood {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Happy => "happy",
            Self::Sleepy => "sleepy",
            Self::Working => "working",
            Self::Excited => "excited",
            Self::Sad => "sad",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "happy" => Self::Happy,
            "sleepy" => Self::Sleepy,
            "working" => Self::Working,
            "excited" => Self::Excited,
            "sad" => Self::Sad,
            _ => Self::Normal,
        }
    }
}

/// 桌宠造型种类（兽形 / 人型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetKind {
    /// 兽形（圆滚体型，有耳/尾）
    Beast,
    /// 人型（少女花嫁，有头发/裙子/翅膀/花束）
    Humanoid,
}

/// 用户触发的桌宠动作（R5 契约单一来源：前端按钮/窗口事件的互操作编码）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetAction {
    /// 顺毛抚摸（桌面双击 / 按钮）
    Pet,
    /// 投喂猫条（按钮）
    Feed,
    /// 换班庆祝（会话达成 goal.complete）
    Celebrate,
    /// 唤醒（桌面悬浮宠被单击）
    Wake,
    /// 困倦（能量衰减）
    Doze,
}

impl PetAction {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pet" => Self::Pet,
            "feed" => Self::Feed,
            "celebrate" => Self::Celebrate,
            "wake" => Self::Wake,
            "doze" => Self::Doze,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pet => "pet",
            Self::Feed => "feed",
            Self::Celebrate => "celebrate",
            Self::Wake => "wake",
            Self::Doze => "doze",
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// 对外契约 DTO（对齐 TS 前端 petContract）
// ──────────────────────────────────────────────────────────────────────

/// 皮肤列表项（`pet.list` 返回）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSkinEntry {
    pub id: String,
    pub label: String,
    pub kind: PetKind,
    pub accent_hex: String,
}

impl From<PetSkinId> for PetSkinEntry {
    fn from(id: PetSkinId) -> Self {
        Self {
            id: id.as_str().to_string(),
            label: id.label().to_string(),
            kind: id.kind(),
            accent_hex: format!("#{:06x}", id.accent_hex()),
        }
    }
}

/// 桌宠完整状态（`pet.status` 返回 · V2.1 全字段：皮肤 + 心情 + 能量 + 好感度）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetStatus {
    pub skin: String,
    pub kind: PetKind,
    pub mood: String,
    pub energy: u8,
    pub affection: u32,
    pub running: bool,
    pub accent_hex: String,
}

/// 皮肤切换请求（`pet.setSkin` 入参）。
#[derive(Debug, Clone, Deserialize)]
pub struct SetSkinReq {
    pub skin: String,
}

/// 心情切换请求（`pet.setMood` 入参）。
#[derive(Debug, Clone, Deserialize)]
pub struct SetMoodReq {
    pub mood: String,
}

/// 动作触发请求（`pet.triggerAction` 入参）。
#[derive(Debug, Clone, Deserialize)]
pub struct TriggerActionReq {
    pub action: String,
}

// ──────────────────────────────────────────────────────────────────────
// 进程内共享状态（内存 · 跨 WS 客户端共享；持久化仍由 workbench/pet 负责）
// ──────────────────────────────────────────────────────────────────────

struct PetShared {
    skin: PetSkinId,
    mood: PetMood,
}

fn shared() -> &'static Arc<Mutex<PetShared>> {
    static CELL: OnceLock<Arc<Mutex<PetShared>>> = OnceLock::new();
    CELL.get_or_init(|| Arc::new(Mutex::new(PetShared { skin: PetSkinId::Cat, mood: PetMood::Normal })))
}

pub fn get_skin() -> PetSkinId {
    shared().lock().map(|s| s.skin).unwrap_or(PetSkinId::Cat)
}

pub fn set_skin(id: PetSkinId) {
    if let Ok(mut s) = shared().lock() {
        s.skin = id;
    }
}

pub fn get_mood() -> PetMood {
    shared().lock().map(|s| s.mood).unwrap_or(PetMood::Normal)
}

pub fn set_mood(m: PetMood) {
    if let Ok(mut s) = shared().lock() {
        s.mood = m;
    }
}

/// 5 款皮肤（单一来源 · 与前端 PET_SKINS/ELF_SPECS 对齐）。
pub const SKIN_CATALOG: &[PetSkinId] = &[
    PetSkinId::Cat,
    PetSkinId::Blue,
    PetSkinId::Pink,
    PetSkinId::Purple,
    PetSkinId::Green,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_catalog_size_equals_five() {
        assert_eq!(SKIN_CATALOG.len(), 5, "皮肤目录必须是猫 + 4 精灵");
    }

    #[test]
    fn kind_is_consistent_with_skin_id() {
        assert_eq!(PetSkinId::Cat.kind(), PetKind::Beast);
        for s in [PetSkinId::Blue, PetSkinId::Pink, PetSkinId::Purple, PetSkinId::Green] {
            assert_eq!(s.kind(), PetKind::Humanoid, "精灵 {:?} 必须为人型", s);
        }
    }

    #[test]
    fn skin_roundtrip_parse() {
        for s in SKIN_CATALOG {
            let id = PetSkinId::parse(s.as_str());
            assert_eq!(id, *s, "{s:?} parse 往返失败");
        }
        assert_eq!(PetSkinId::parse("unknown"), PetSkinId::Cat, "未知皮肤回退到 Cat");
    }

    #[test]
    fn accent_hex_in_range() {
        for s in SKIN_CATALOG {
            assert!(s.accent_hex() <= 0xff_ff_ff, "{s:?} accent 溢出 24bit");
        }
    }

    #[test]
    fn shared_state_arc_equality() {
        let a = shared();
        let b = shared();
        assert!(Arc::ptr_eq(a, b), "共享状态必须是同一 Arc（跨连接一致）");
    }

    #[test]
    fn set_and_get_skin_is_consistent() {
        set_skin(PetSkinId::Blue);
        assert_eq!(get_skin(), PetSkinId::Blue);
        set_skin(PetSkinId::Cat);
        assert_eq!(get_skin(), PetSkinId::Cat);
    }

    #[test]
    fn mood_roundtrip_parse() {
        for m in [PetMood::Normal, PetMood::Happy, PetMood::Sleepy, PetMood::Working, PetMood::Excited, PetMood::Sad] {
            assert_eq!(PetMood::parse(m.as_str()), m);
        }
        assert_eq!(PetMood::parse("nonsense"), PetMood::Normal);
    }

    #[test]
    fn action_parse_unknown_returns_none() {
        assert!(PetAction::parse("not-an-action").is_none());
        for (k, _) in ["pet", "feed", "celebrate", "wake", "doze"].iter().zip(0..) {
            assert!(PetAction::parse(k).is_some());
        }
    }
}
