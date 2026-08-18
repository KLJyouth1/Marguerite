//! 视觉能力桥（E 期 vision/）。
//!
//! 让纯文本模型获得看图能力，对齐 nexustar-api `vision.py`：
//! - 零依赖手工解析图片元数据（PNG/JPEG/GIF/BMP/WebP 头部）；
//! - 经可注入的 `VisionTransport` 转发到具备视觉能力的模型（OpenAI 兼容多模态接口）；
//! - 无可用视觉能力或上游失败时降级返回「元数据 + 明确原因」，绝不静默失败。

pub mod binary_html;
pub mod client;
pub mod evidence;
pub mod metadata;

pub use binary_html::{binary_to_html, binary_to_img_tag, image_metadata_to_prompt};
pub use client::{
    describe_image, LlmVisionTransport, LocateResult, MockVisionResolver, MockVisionTransport,
    VisionProviderResolver, VisionTransport,
};
pub use metadata::image_metadata;
pub use metadata::ImageMeta;

use std::sync::{Arc, Mutex};

/// 视觉能力门面：把可注入的「解析器 + 转发器」收敛为编排器 / 工具可持有的单例能力。
///
/// 默认 [`VisionCapability::offline_default`] 使用「无视觉能力」mock 解析器，
/// [`describe_image`](VisionCapability::describe_image) 会走优雅降级（明确原因 + 元数据），
/// 绝不静默失败、绝不联网。生产侧通过 [`VisionCapability::set_transport`] 注入真实
/// [`LlmVisionTransport`](client::LlmVisionTransport)（OpenAI 兼容多模态接口）。
///
/// 内部用 `Arc` 做内部可变性：仅短暂持锁克隆出 `Arc` 即释放，锁不跨 `.await`，
/// 因此能力实例是 `Send + Sync`，可被异步工具跨线程调度。
pub struct VisionCapability {
    resolver: Mutex<Arc<dyn VisionProviderResolver + Send + Sync>>,
    transport: Mutex<Arc<dyn VisionTransport + Send + Sync>>,
}

impl VisionCapability {
    /// 离线默认：无可用视觉模型，describe 始终优雅降级（不发起任何网络请求）。
    pub fn offline_default() -> Self {
        Self {
            resolver: Mutex::new(Arc::new(MockVisionResolver { capable: false })),
            transport: Mutex::new(Arc::new(MockVisionTransport {
                reply: String::new(),
            })),
        }
    }

    /// 注入视觉模型解析器（线程安全；`&self` 即可）。
    pub fn set_resolver(&self, r: Box<dyn VisionProviderResolver + Send + Sync>) {
        *self.resolver.lock().unwrap() = Arc::from(r);
    }

    /// 注入视觉转发器（线程安全；`&self` 即可）。
    pub fn set_transport(&self, t: Box<dyn VisionTransport + Send + Sync>) {
        *self.transport.lock().unwrap() = Arc::from(t);
    }

    /// 分析图片字节并返回文字描述（元信息解析 → 能力判定 → 转发；无能力/失败降级）。
    pub async fn describe_image(&self, data: &[u8], hint: &str) -> String {
        let resolver = self.resolver.lock().unwrap().clone();
        let transport = self.transport.lock().unwrap().clone();
        describe_image(data, &*resolver, &*transport, hint).await
    }

    /// 视觉定位：截图 → 元素中心坐标（相对截图像素）。无视觉能力 / transport 不支持 /
    /// 解析失败时返回 None（调用方降级为描述或报错，绝不静默）。
    pub async fn locate(&self, data: &[u8], hint: &str) -> Option<LocateResult> {
        let resolver = self.resolver.lock().unwrap().clone();
        let transport = self.transport.lock().unwrap().clone();
        resolver.resolve_vision_model().await.as_ref()?;
        let meta = crate::vision::metadata::image_metadata(data);
        if meta.format == "unknown" {
            return None;
        }
        transport.locate(data, meta.mime, hint).await
    }

    /// 带 HTML 兜底的图片理解（无多模态能力时 fallback）。
    ///
    /// `native` = 是否走原生视觉能力强路径；`content` = 描述文本（原生）或
    /// 二进制→HTML 字符串（兜底，供无视觉模型据此"读"图）。区别于旧 `describe_image`
    /// 的降级文本，兜底返回可被模型直接消费的 HTML，而非仅元数据告警。
    pub async fn describe_with_fallback(&self, data: &[u8], hint: &str) -> (bool, String) {
        let native = self.describe_image(data, hint).await;
        // describe_image 无法区分"无视觉能力"与"调用失败"——统一用元数据判定能力：
        // 若解析器无视觉模型，则标为 fallback 并给 HTML。
        let resolver = self.resolver.lock().unwrap().clone();
        let capable = resolver.resolve_vision_model().await.is_some();
        if capable && !native.starts_with("[视觉降级]") {
            (true, native)
        } else {
            (false, crate::vision::binary_html::binary_to_html(data))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[0, 0, 0, 13]);
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&64u32.to_be_bytes());
        v.extend_from_slice(&48u32.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v
    }

    struct LocateResolver;
    #[async_trait]
    impl VisionProviderResolver for LocateResolver {
        async fn resolve_vision_model(&self) -> Option<String> {
            Some("locate-model".into())
        }
    }

    struct LocateTransport {
        result: Option<LocateResult>,
    }
    #[async_trait]
    impl VisionTransport for LocateTransport {
        async fn describe(&self, _: &[u8], _: &str, _: &str) -> Result<String, String> {
            Ok("描述".into())
        }
        async fn locate(&self, _: &[u8], _: &str, _: &str) -> Option<LocateResult> {
            self.result.clone()
        }
    }

    #[tokio::test]
    async fn locate_returns_result_when_capable() {
        let cap = VisionCapability::offline_default();
        cap.set_resolver(Box::new(LocateResolver));
        cap.set_transport(Box::new(LocateTransport {
            result: Some(LocateResult {
                x: 30.0,
                y: 20.0,
                confidence: 0.9,
            }),
        }));
        let r = cap.locate(&png(), "按钮").await;
        assert_eq!(
            r,
            Some(LocateResult {
                x: 30.0,
                y: 20.0,
                confidence: 0.9
            })
        );
    }

    #[tokio::test]
    async fn locate_none_when_incapable() {
        // 默认 offline_default：resolver capable=false → 直接 None（不触碰 transport）
        let cap = VisionCapability::offline_default();
        assert!(cap.locate(&png(), "按钮").await.is_none());
    }

    #[tokio::test]
    async fn locate_none_when_transport_unsupported() {
        let cap = VisionCapability::offline_default();
        cap.set_resolver(Box::new(LocateResolver));
        // transport 未实现 locate（默认返回 None）
        cap.set_transport(Box::new(MockVisionTransport { reply: "x".into() }));
        assert!(cap.locate(&png(), "按钮").await.is_none());
    }

    #[tokio::test]
    async fn describe_with_fallback_returns_html_when_incapable() {
        // 无视觉能力 → fallback 返回 HTML（含 img/base64），而非仅降级文本
        let cap = VisionCapability::offline_default();
        let (native, content) = cap.describe_with_fallback(&png(), "描述").await;
        assert!(!native, "无视觉能力应标为 fallback");
        assert!(content.contains("<html>"), "兜底应返回 HTML");
        assert!(
            content.contains("data:image/png;base64,"),
            "HTML 内嵌 base64 图片"
        );
    }

    #[tokio::test]
    async fn describe_with_fallback_uses_native_when_capable() {
        let cap = VisionCapability::offline_default();
        cap.set_resolver(Box::new(LocateResolver));
        cap.set_transport(Box::new(MockVisionTransport {
            reply: "看到了猫".into(),
        }));
        let (native, content) = cap.describe_with_fallback(&png(), "描述").await;
        assert!(native, "具备视觉能力应走原生路径");
        assert_eq!(content, "看到了猫");
    }
}
