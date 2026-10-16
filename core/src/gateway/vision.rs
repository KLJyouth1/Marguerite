//! 视觉方法族（modlens 文档分析）

use serde_json::{json, Value};

/// modlens.analyze：图片(base64) → 结构化 JSON 文档
///
/// 无视觉模型时降级为元数据兜底（source=metadata-fallback），绝不静默失败。
pub async fn modlens_analyze(params: &Value) -> Value {
    use base64::Engine as _;
    let b64 = params
        .get("imageBase64")
        .or_else(|| params.get("image_base64"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let hint = params.get("hint").and_then(Value::as_str).unwrap_or("");
    let bytes = match base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
        Ok(b) => b,
        Err(e) => return json!({ "ok": false, "error": format!("图片 base64 解码失败: {e}") }),
    };
    if bytes.is_empty() {
        return json!({ "ok": false, "error": "图片数据为空" });
    }
    let doc = vision_singleton().analyze_document(&bytes, hint).await;
    match serde_json::from_str::<Value>(&doc.to_json()) {
        Ok(document) => json!({ "ok": true, "document": document }),
        Err(_) => json!({ "ok": false, "error": "视觉文档序列化失败" }),
    }
}

/// 进程级视觉能力单例（离线默认；后续可注入真实视觉模型 transport）
pub fn vision_singleton() -> &'static crate::vision::VisionCapability {
    static V: std::sync::OnceLock<crate::vision::VisionCapability> = std::sync::OnceLock::new();
    V.get_or_init(crate::vision::VisionCapability::offline_default)
}

/// 从 RPC 参数解析是否请求视觉 grounding（`vision: true` 或 `grounding: true`，值可为
/// 布尔或字符串 "true"/"1"）。纯函数，便于单测。
pub fn wants_vision_grounding(params: &Value) -> bool {
    fn truthy(v: Option<&Value>) -> bool {
        match v {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s == "true" || s == "1",
            Some(Value::Number(n)) => n.as_i64() == Some(1),
            _ => false,
        }
    }
    truthy(params.get("vision")) || truthy(params.get("grounding"))
}

/// 按配置构造计算机使用的视觉能力（`computer_use.action` 的 grounding 闭环）。
///
/// 复用仓库既有的 [`crate::vision::LlmVisionTransport`]（OpenAI 兼容多模态接口）：
/// - 模型名命中视觉关键词 **且** 配置了 API Key → 返回可用的 [`VisionCapability`]；
/// - 否则返回 `None`（调用方应诚实软失败：`code: "VISION_UNAVAILABLE"`，**不得**静默降级为普通截图）。
pub fn computer_use_vision(
    config: &crate::persistence::config::Config,
) -> Option<std::sync::Arc<crate::vision::VisionCapability>> {
    use crate::model::llm::{LLMConfig, LLMProvider};
    let m = &config.model;
    if m.api_key.trim().is_empty() || !crate::orchestration::is_vision_model(&m.model_name) {
        return None;
    }
    let cap = std::sync::Arc::new(crate::vision::VisionCapability::offline_default());
    let llm = LLMConfig {
        provider: LLMProvider::Custom {
            base_url: m.base_url.clone(),
        },
        model: m.model_name.clone(),
        api_key: m.api_key.clone(),
        base_url: m.base_url.clone(),
        max_tokens: 2048,
        temperature: 0.2,
        timeout_secs: 30,
        thinking: crate::model::thinking::ThinkingEffort::Off,
        context_window: crate::model::thinking::CONTEXT_WINDOW_DEFAULT,
    };
    cap.set_transport(Box::new(crate::vision::LlmVisionTransport::new(llm)));
    cap.set_resolver(Box::new(VisionNameResolver(m.model_name.clone())));
    Some(cap)
}

/// 视觉模型解析器：声明当前配置模型具备视觉能力（与 transport 配对注入）。
struct VisionNameResolver(String);

#[async_trait::async_trait]
impl crate::vision::VisionProviderResolver for VisionNameResolver {
    async fn resolve_vision_model(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::config::Config;

    fn cfg(model: &str, api_key: &str) -> Config {
        let mut c = Config::default();
        c.model.model_name = model.to_string();
        c.model.api_key = api_key.to_string();
        c
    }

    #[test]
    fn wants_vision_grounding_parses_flags() {
        assert!(wants_vision_grounding(&json!({ "vision": true })));
        assert!(wants_vision_grounding(&json!({ "grounding": true })));
        assert!(wants_vision_grounding(&json!({ "vision": "true" })));
        assert!(!wants_vision_grounding(&json!({ "vision": false })));
        assert!(!wants_vision_grounding(&json!({})));
    }

    #[test]
    fn computer_use_vision_none_without_key_or_non_vision_model() {
        // 无 key → None（调用方据此软失败 VISION_UNAVAILABLE）
        assert!(computer_use_vision(&cfg("gpt-4o", "")).is_none());
        // 有 key 但模型不具视觉 → None
        assert!(computer_use_vision(&cfg("deepseek-chat", "sk-x")).is_none());
    }

    #[tokio::test]
    async fn computer_use_vision_builds_for_vision_model_with_key() {
        // 构造期不发任何网络请求；仅验证注入了可用能力。
        let cap = computer_use_vision(&cfg("gpt-4o", "sk-x")).expect("应构造视觉能力");
        // 非法图片：在触碰 transport 之前即因格式未知返回 None（无任何网络请求）
        assert!(cap.locate(b"not-an-image", "按钮").await.is_none());
    }
}
