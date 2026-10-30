//! 稳定用户偏好（`user_profile.md`）—— 只存「语言 / 技术栈 / 禁忌」，不存一次性任务。
//!
//! 落盘 `Config::config_dir()/user_profile.md`（默认 `~/.marguerite/user_profile.md`），
//! 人类可读、可手改；解析宽容（认 `- `/`* ` 列表项，认中英文小节名，未知内容忽略）。
//!
//! ```markdown
//! # 用户画像（Marguerite 自动维护；只记稳定偏好，勿写一次性任务）
//!
//! ## 语言
//! - 中文
//!
//! ## 技术栈
//! - Rust
//! - Vue 3
//!
//! ## 禁忌
//! - 不要 git commit
//! ```
//!
//! 这些条目经 [`UserProfile::to_preferences`] 变成简报里的 `preferences` 区
//! （`key`/`value`/`confidence`/`evidence` 形状见计划 §4.2）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::brief::Preference;

/// 标题行（每次写回都会覆盖，保证文件自解释）
const HEADER: &str = "# 用户画像（Marguerite 自动维护；只记稳定偏好，勿写一次性任务）";

/// 用户稳定偏好
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    /// 交流/产出语言，如 `中文`
    pub language: Option<String>,
    /// 常用技术栈（顺序即优先级）
    pub stacks: Vec<String>,
    /// 禁忌：明确不要做的事
    pub taboos: Vec<String>,
}

impl UserProfile {
    /// 空画像（不含任何偏好）
    pub fn is_empty(&self) -> bool {
        self.language.is_none() && self.stacks.is_empty() && self.taboos.is_empty()
    }

    /// 归一化形态：语言去首尾空白并丢空值，技术栈/禁忌去空项并按出现顺序去重。
    /// 写入与读取两侧都对齐到它，保证「存一次再读」的往返是稳定的。
    pub fn normalized(&self) -> UserProfile {
        UserProfile {
            language: self
                .language
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            stacks: dedup(&self.stacks),
            taboos: dedup(&self.taboos),
        }
    }

    /// `user_profile.md` 的路径
    pub fn path() -> PathBuf {
        crate::persistence::config::Config::config_dir().join("user_profile.md")
    }

    /// 读取画像；文件缺失或不可读 → 空画像（不报错，冷启动路径上不能挡道）。
    pub fn load() -> UserProfile {
        match std::fs::read_to_string(Self::path()) {
            Ok(raw) => Self::parse(&raw),
            Err(_) => UserProfile::default(),
        }
    }

    /// 写回画像（原子写）。
    pub fn save(&self) -> Result<(), String> {
        super::write_atomic_text(&Self::path(), &self.render())
    }

    /// 渲染为 markdown（去重、去空白后的稳定形态）
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(HEADER);
        out.push_str("\n\n## 语言\n");
        if let Some(lang) = self.language.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            out.push_str(&format!("- {lang}\n"));
        }
        out.push_str("\n## 技术栈\n");
        for s in dedup(&self.stacks) {
            out.push_str(&format!("- {s}\n"));
        }
        out.push_str("\n## 禁忌\n");
        for t in dedup(&self.taboos) {
            out.push_str(&format!("- {t}\n"));
        }
        out
    }

    /// 解析 markdown（宽容：未知小节 / 未知列表项一律忽略）
    pub fn parse(md: &str) -> UserProfile {
        let mut profile = UserProfile::default();
        let mut section = Section::None;
        for line in md.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("##") {
                section = Section::from_heading(rest.trim());
                continue;
            }
            let item = match trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                Some(rest) => rest.trim(),
                None => continue,
            };
            if item.is_empty() {
                continue;
            }
            match section {
                Section::Language => {
                    if profile.language.is_none() {
                        profile.language = Some(item.to_string());
                    }
                }
                Section::Stacks => profile.stacks.push(item.to_string()),
                Section::Taboos => profile.taboos.push(item.to_string()),
                Section::None => {}
            }
        }
        profile.stacks = dedup(&profile.stacks);
        profile.taboos = dedup(&profile.taboos);
        profile
    }

    /// 转成简报的 `preferences` 区（`confidence`：语言/禁忌为硬偏好，技术栈略低）
    pub fn to_preferences(&self) -> Vec<Preference> {
        let mut out = Vec::new();
        if let Some(lang) = self.language.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            out.push(Preference {
                key: "language".to_string(),
                value: lang.to_string(),
                confidence: 1.0,
                evidence: 1,
            });
        }
        for s in dedup(&self.stacks) {
            out.push(Preference {
                key: "stack".to_string(),
                value: s,
                confidence: 0.9,
                evidence: 1,
            });
        }
        for t in dedup(&self.taboos) {
            out.push(Preference {
                key: "taboo".to_string(),
                value: t,
                confidence: 1.0,
                evidence: 1,
            });
        }
        out
    }
}

enum Section {
    None,
    Language,
    Stacks,
    Taboos,
}

impl Section {
    fn from_heading(heading: &str) -> Section {
        let h = heading.to_ascii_lowercase();
        if heading.contains("语言") || h.contains("language") || h.contains("lang") {
            Section::Language
        } else if heading.contains("技术栈") || heading.contains("技术") || h.contains("stack") {
            Section::Stacks
        } else if heading.contains("禁忌") || heading.contains("禁止") || h.contains("taboo") {
            Section::Taboos
        } else {
            Section::None
        }
    }
}

/// 去空白、去空项、按出现顺序去重
fn dedup(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|seen| seen == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_assist_profile_{}_{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::persistence::config::Config::set_config_dir_override(dir.clone());
        dir
    }

    #[test]
    fn profile_markdown_roundtrip() {
        let dir = isolated_dir("roundtrip");
        assert!(UserProfile::load().is_empty(), "无文件时读空画像");

        let profile = UserProfile {
            language: Some("中文".to_string()),
            stacks: vec!["Rust".to_string(), "Vue 3".to_string(), "Rust".to_string()],
            taboos: vec!["不要 git commit".to_string()],
        };
        profile.save().expect("写入 user_profile.md 应成功");

        assert!(UserProfile::path().exists());
        assert!(UserProfile::path().starts_with(&dir));
        let loaded = UserProfile::load();
        assert_eq!(loaded.language.as_deref(), Some("中文"));
        assert_eq!(loaded.stacks, vec!["Rust".to_string(), "Vue 3".to_string()]);
        assert_eq!(loaded.taboos, vec!["不要 git commit".to_string()]);
        assert_eq!(loaded, profile.clone().normalized());

        // 覆盖写：第二次写入替换而非追加
        let replaced = UserProfile {
            language: Some("English".to_string()),
            stacks: vec!["Go".to_string()],
            taboos: Vec::new(),
        };
        replaced.save().unwrap();
        assert_eq!(UserProfile::load(), replaced);
    }

    #[test]
    fn preferences_carry_key_value_confidence_evidence() {
        let _dir = isolated_dir("prefs");
        let profile = UserProfile {
            language: Some("中文".to_string()),
            stacks: vec!["Rust".to_string()],
            taboos: vec!["不要 force push".to_string()],
        };
        let prefs = profile.to_preferences();
        assert_eq!(prefs.len(), 3);
        assert_eq!(prefs[0].key, "language");
        assert_eq!(prefs[0].value, "中文");
        assert!(prefs[0].confidence > 0.0 && prefs[0].evidence >= 1);
        assert_eq!(prefs[1].key, "stack");
        assert_eq!(prefs[2].key, "taboo");
        let value = serde_json::to_value(&prefs[0]).unwrap();
        for field in ["key", "value", "confidence", "evidence"] {
            assert!(value.get(field).is_some(), "preference 缺少字段 {field}");
        }
    }

    #[test]
    fn hand_edited_file_is_parsed_leniently() {
        let _dir = isolated_dir("handedit");
        let raw = "# 我的画像\n\n## 语言\n- 中文\n\n## 技术栈\n* Rust\n\n## 禁忌\n- 不要动网关\n\n## 备注\n- 忽略我\n";
        let profile = UserProfile::parse(raw);
        assert_eq!(profile.language.as_deref(), Some("中文"));
        assert_eq!(profile.stacks, vec!["Rust".to_string()]);
        assert_eq!(profile.taboos, vec!["不要动网关".to_string()]);
    }
}
