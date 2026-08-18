//! Bloom TUI 原生美学（去 WebGL）
//!
//! 把 Marguerite 的 Bloom 视觉身份（品牌色 + 粒子溶解隐喻）用终端原生美学实现：
//! - [`spectral`]：Rust 音频频谱分析（窗口化 FFT），为 Bloom 音乐粒子提供能量源
//! - [`particles`]：终端粒子场模拟（Bloom 粒子溶解隐喻，纯数学可测）
//! - [`theme`]：Bloom 品牌色板与辉光梯度（青/teal 辉光，避开紫粉渐变 AI 模板套路）
//! - [`capabilities`]：把引擎能力（原生工具 / 沙箱 / KB / 治理门）聚合成快照
//! - [`widget`]：Bloom 原生能力面板（ratatui Widget，可渲染到 Buffer 做视觉门禁）
//!
//! 全部零 WebGL、零 emoji、零硬编码色（仅经 [`theme::BloomTheme`] 常量）。

pub mod capabilities;
pub mod particles;
pub mod spectral;
pub mod theme;
pub mod widget;

pub use capabilities::{CapabilityProvider, CapabilitySnapshot};
pub use particles::{Particle, ParticleField};
pub use spectral::{analyze_bands, SpectralBands};
pub use theme::BloomTheme;
pub use widget::CapabilityPanel;
