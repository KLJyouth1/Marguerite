//! 主题 token 表（Track D3，对译 `tokens.css` + `statusColors.ts`）。
//!
//! P0 视觉纪律：禁硬编码色、禁紫粉渐变、禁 emoji 图标。
//! 本模块只产出 **token 引用名**（如 `"color.bg.base"`），不内联任何 hex 字面量
//! （`#fff` / `#000` 例外，但本实现连例外都不用，全 token 化）。渲染层据 token 名
//! 向设计 token 系统取真实色值。

/// 主题 token 引用（指向设计 token 系统的键，而非颜色字面量）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRef(pub String);

impl TokenRef {
    /// 构造 token 引用。
    pub fn new(key: &str) -> Self {
        TokenRef(key.to_string())
    }
}

/// 一套皮肤的语义色 token 表。
///
/// 所有色值以 [`TokenRef`] 引用形式给出，渲染层据名取色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeTokens {
    /// 背景基色。
    pub bg: TokenRef,
    /// 前景（文字）基色。
    pub fg: TokenRef,
    /// 强调色。
    pub accent: TokenRef,
    /// 面板背景。
    pub panel_bg: TokenRef,
    /// 边框。
    pub border: TokenRef,
    /// 成功态。
    pub success: TokenRef,
    /// 警告态。
    pub warning: TokenRef,
    /// 危险态。
    pub danger: TokenRef,
    /// 次要/弱化文字。
    pub muted: TokenRef,
}

/// 真实皮肤集合。
///
/// 审计指出 `dsh-web-ui` 实际仅 8 套（非宣传的 10）。命名避免紫粉渐变语义，
/// 采用纯色基调的语义名（Indigo / Slate 系）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skin {
    /// 浅色。
    Light,
    /// 深色。
    Dark,
    /// 护眼米色。
    Sepia,
    /// 午夜蓝。
    Midnight,
    /// 森林绿。
    Forest,
    /// 海洋蓝。
    Ocean,
    /// 樱粉（纯色，非紫粉渐变）。
    Sakura,
    /// 单色（高对比）。
    Mono,
}

/// 返回某皮肤的 token 引用表（全 token 化，无 hex 字面量）。
pub fn tokens_for(skin: Skin) -> ThemeTokens {
    let s = match skin {
        Skin::Light => "light",
        Skin::Dark => "dark",
        Skin::Sepia => "sepia",
        Skin::Midnight => "midnight",
        Skin::Forest => "forest",
        Skin::Ocean => "ocean",
        Skin::Sakura => "sakura",
        Skin::Mono => "mono",
    };
    ThemeTokens {
        bg: TokenRef::new(&format!("color.{s}.bg.base")),
        fg: TokenRef::new(&format!("color.{s}.fg.base")),
        accent: TokenRef::new(&format!("color.{s}.accent")),
        panel_bg: TokenRef::new(&format!("color.{s}.bg.panel")),
        border: TokenRef::new(&format!("color.{s}.border")),
        success: TokenRef::new("color.semantic.success"),
        warning: TokenRef::new("color.semantic.warning"),
        danger: TokenRef::new("color.semantic.danger"),
        muted: TokenRef::new(&format!("color.{s}.fg.muted")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_skins_all_tokenized() {
        let skins = [
            Skin::Light,
            Skin::Dark,
            Skin::Sepia,
            Skin::Midnight,
            Skin::Forest,
            Skin::Ocean,
            Skin::Sakura,
            Skin::Mono,
        ];
        assert_eq!(skins.len(), 8, "审计确认实际仅 8 套皮肤");
        // 每皮肤返回非空 token，且无裸 hex（# 开头）—— 禁止硬编码色。
        for skin in skins {
            let t = tokens_for(skin);
            for tok in [
                &t.bg,
                &t.fg,
                &t.accent,
                &t.panel_bg,
                &t.border,
                &t.success,
                &t.warning,
                &t.danger,
                &t.muted,
            ] {
                assert!(!tok.0.is_empty(), "token 引用不能为空");
                assert!(
                    !tok.0.starts_with('#'),
                    "禁止裸 hex token（P0 视觉纪律）: {}",
                    tok.0
                );
            }
        }
    }
}
