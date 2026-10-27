//! 多模态能力解析：把 `models.json` 里的「能力声明」翻译成**可调用的端点**。
//!
//! # 为什么单独一个模块（2026-10-23）
//!
//! 生图 / 生视频 / 语音合成 / 语音转写四类工具的失败方式有个共同点：
//! 「能力存在，但工具不知道往哪打」。此前每个工具各写一份 URL 拼接，
//! 结果三份实现各自漏了 `/v1` 去重、各自漏了端点覆盖、各自漏了密钥回退链。
//!
//! 这里把三件事收敛成一条链，工具只拿结果：
//! 1. **选档案**：主模型优先，其次任意声明了该能力的档案；
//! 2. **选端点**：档案的 `*_endpoint` 覆盖 > 供应商默认路径（并处理 `/v1` 重复）；
//! 3. **取密钥**：走 BYOK 同一条回退链（显式 > 声明变量 > 分槽 > 全局槽）。
//!
//! 解析失败一律返回**可执行的中文指引**（告诉用户去哪个页面勾哪个框），
//! 绝不返回一个「看起来成功」的空结果——假成功比失败更难排查。

use serde_json::Value;

use crate::model::registry::{ModelCapabilities, ModelProfile};

/// 四类多模态调用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// 文本 → 图片
    Image,
    /// 文本 → 视频
    Video,
    /// 文本 → 语音
    Speech,
    /// 音频 → 文本
    Transcription,
}

impl MediaKind {
    /// 供应商默认路径（OpenAI 兼容）。
    pub fn default_path(self) -> &'static str {
        match self {
            MediaKind::Image => "/v1/images/generations",
            MediaKind::Video => "/v1/videos/generations",
            MediaKind::Speech => "/v1/audio/speech",
            MediaKind::Transcription => "/v1/audio/transcriptions",
        }
    }

    /// 该能力在档案里的开关字段。
    pub fn enabled_in(self, caps: &ModelCapabilities) -> bool {
        match self {
            MediaKind::Image => caps.image_output,
            MediaKind::Video => caps.video_output,
            MediaKind::Speech => caps.audio_output,
            MediaKind::Transcription => caps.audio_input,
        }
    }

    /// 该能力的端点覆盖字段。
    pub fn endpoint_override(self, caps: &ModelCapabilities) -> Option<&str> {
        match self {
            MediaKind::Image => caps.image_endpoint.as_deref(),
            MediaKind::Video => caps.video_endpoint.as_deref(),
            MediaKind::Speech | MediaKind::Transcription => caps.audio_endpoint.as_deref(),
        }
    }

    /// 该能力的自定义输入/输出参数（已合并到请求体）。
    pub fn params(self, caps: &ModelCapabilities) -> Option<&Value> {
        match self {
            MediaKind::Image => caps.image_params.as_ref(),
            MediaKind::Video => caps.video_params.as_ref(),
            MediaKind::Speech | MediaKind::Transcription => caps.audio_params.as_ref(),
        }
    }

    /// 面向用户的能力名（错误提示里用）。
    pub fn label(self) -> &'static str {
        match self {
            MediaKind::Image => "图像生成",
            MediaKind::Video => "视频生成",
            MediaKind::Speech => "语音合成",
            MediaKind::Transcription => "音频转写",
        }
    }

    /// 设置页里对应的勾选框名（错误提示里指路用）。
    pub fn setting_hint(self) -> &'static str {
        match self {
            MediaKind::Image => "图像输出",
            MediaKind::Video => "视频输出",
            MediaKind::Speech => "语音输出",
            MediaKind::Transcription => "音频输入",
        }
    }
}

/// 解析完成的调用目标。
#[derive(Debug, Clone)]
pub struct MediaEndpoint {
    /// 命中的档案 id（回传给出图/转写结果，便于用户核对是哪份配置生效）
    pub profile_id: String,
    pub provider: String,
    /// 请求体里 `model` 字段的取值
    pub model: String,
    /// 完整 URL（已含路径，可直接 POST）
    pub url: String,
    /// 已解析的密钥（本地免密钥端点为空串）
    pub api_key: String,
    /// 档案里用户填的自定义参数（未填为 None）
    pub params: Option<Value>,
}

/// 拼接 base_url 与路径，避免 `/v1/v1/...`。
///
/// 实测痛点：用户填 `https://api.openai.com/v1`（最常见），而默认路径自带 `/v1/`；
/// 直接拼会得到 `/v1/v1/images/generations` → 404，且错误信息毫无线索。
fn join_url(base: &str, path: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    let p = path.trim();
    if p.starts_with("http://") || p.starts_with("https://") {
        return p.to_string();
    }
    let path_rest = p.trim_start_matches('/');
    // base 已带 /v1 且 path 以 v1/ 开头 → 去掉重复的一段
    if b.ends_with("/v1") && path_rest.starts_with("v1/") {
        format!("{b}/{}", &path_rest[3..])
    } else {
        format!("{b}/{path_rest}")
    }
}

/// 选档案：主模型优先，其次按注册顺序找第一个声明了该能力的。
///
/// 只按**生效能力**（声明优先、否则名字识别）判断，不按 provider 猜——
/// 用户把「图像输出」勾掉后，即使模型名带 `dall-e` 也不该再被选中。
fn candidate_profiles() -> Vec<ModelProfile> {
    use crate::model::selector::ModelSelector;
    let Ok(sel) = ModelSelector::load() else {
        return Vec::new();
    };
    let reg = sel.registry();
    let mut out = Vec::new();
    if let Some(active) = reg.active_profile() {
        out.push(active.clone());
    }
    for p in reg.list() {
        if p.id != reg.active {
            out.push(p.clone());
        }
    }
    out
}

/// 解析某类多模态调用的目标端点。
pub fn resolve(kind: MediaKind) -> Result<MediaEndpoint, String> {
    let profiles = candidate_profiles();
    if profiles.is_empty() {
        return Err(format!(
            "没有可用的模型档案，无法执行{}。请先在「设置 → 模型服务」接入一个模型。",
            kind.label()
        ));
    }

    let hit = profiles
        .into_iter()
        .find(|p| kind.enabled_in(&p.effective_capabilities()));

    let Some(profile) = hit else {
        return Err(format!(
            "没有档案声明「{}」能力，无法执行{}。\n\
             修复方式：设置 → 模型服务 → 选中你的模型 → 多模态能力 → 勾选「{}」\
             （可同时填写该能力的端点路径与额外参数；留空则用供应商默认路径 {}）。",
            kind.setting_hint(),
            kind.label(),
            kind.setting_hint(),
            kind.default_path()
        ));
    };

    let caps = profile.effective_capabilities();
    let url = match kind.endpoint_override(&caps) {
        Some(ep) if !ep.trim().is_empty() => join_url(&profile.base_url, ep.trim()),
        _ => join_url(&profile.base_url, kind.default_path()),
    };
    let api_key = crate::model::byok::resolve_api_key(
        &profile.id,
        &profile.provider,
        &profile.api_key,
        &profile.api_key_env,
        None,
    );
    if api_key.is_empty()
        && !crate::model::byok::is_keyless_endpoint(&profile.provider, &profile.base_url)
    {
        return Err(format!(
            "档案「{}」缺少 API Key，无法调用{}。\n\
             修复方式：设置 → 模型服务 → 选中该档案 → 填入密钥（或选择环境变量）。",
            profile.id,
            kind.label()
        ));
    }

    Ok(MediaEndpoint {
        profile_id: profile.id.clone(),
        provider: profile.provider.clone(),
        model: profile.model.clone(),
        url,
        api_key,
        params: kind.params(&caps).cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_url_does_not_double_v1() {
        assert_eq!(
            join_url("https://api.openai.com/v1", "/v1/images/generations"),
            "https://api.openai.com/v1/images/generations"
        );
        assert_eq!(
            join_url("https://api.openai.com", "/v1/images/generations"),
            "https://api.openai.com/v1/images/generations"
        );
        assert_eq!(
            join_url("http://localhost:1234/v1/", "images/generations"),
            "http://localhost:1234/v1/images/generations"
        );
    }

    #[test]
    fn absolute_override_wins() {
        assert_eq!(
            join_url("https://api.x.com", "https://other.host/custom/gen"),
            "https://other.host/custom/gen"
        );
    }

    #[test]
    fn kind_reads_the_right_flag() {
        let caps = ModelCapabilities {
            image_output: true,
            declared: true,
            ..Default::default()
        };
        assert!(MediaKind::Image.enabled_in(&caps));
        assert!(!MediaKind::Video.enabled_in(&caps));
        assert!(!MediaKind::Speech.enabled_in(&caps));
    }
}
