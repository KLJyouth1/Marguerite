//! 沙箱方法族（策略层能力 / 安全模式状态）
//!
//! 诚实原则：区分策略层（Rust 网关恒可用）与 OS 级隔离运行时（未内置）。

use crate::gateway::methods::GatewayContext;
use crate::persistence::config::Config;
use crate::sandbox::policy_store;
use serde_json::{json, Value};

/// `sandbox.status`：沙箱运行时状态（策略层 + OS 级）
pub fn sandbox_status(_ctx: &GatewayContext) -> Value {
    let run_mode = policy_store::run_mode_get();
    json!({
        "policy": "ready",
        "isolation": "not_bundled",
        "platform": std::env::consts::OS,
        "run_mode": run_mode,
        "data_dir": Config::data_dir().to_string_lossy(),
    })
}

/// `sandbox.capability.status`（对齐 webui `SandboxCapabilityReport`）
///
/// 如实区分布（根绝「安全模式不可用」的误导文案）：
/// - **策略层（policy_store）**：安全模式的文件/命令/网络护栏由 Rust 网关提供，**恒可用** → `available=true`、`code="ready"`。
/// - **OS 级隔离运行时**（Windows Job 对象 / 浏览器沙箱）：未内置 → 单独 `isolation="not_bundled"` 标注，不混淆。
/// - **内置浏览器桥**（CDP，需本机 Chromium）：探测后纳入 `capabilities`（诚实标注）。
pub fn sandbox_capability_status() -> Value {
    let browser_ready = crate::browser::BrowserBridge::new().is_available();
    let mut capabilities = vec!["policy".to_string()];
    if browser_ready {
        capabilities.push("browser".to_string());
    }
    json!({
        "available": true,
        "backend": "policy-only",
        "platform": std::env::consts::OS,
        "code": "ready",
        "isolation": "not_bundled",
        "reason": if browser_ready {
            "安全模式策略层（文件/命令/网络护栏）已就绪；OS 级隔离运行时未内置（内置浏览器桥可用，见 browser.status）"
        } else {
            "安全模式策略层（文件/命令/网络护栏）已就绪；OS 级隔离运行时未内置"
        },
        "setupSupported": false,
        "restartRequired": false,
        "probeVersion": 1,
        "capabilities": capabilities,
    })
}

/// `sandbox.setup.status`（对齐 webui `SandboxSetupStatusPayload`）
pub fn sandbox_setup_status() -> Value {
    json!({
        "state": "unavailable",
        "platform": std::env::consts::OS,
        "message": "Rust 网关未内置沙箱运行时（策略层可用）",
        "requiresAdmin": false,
        "detail": "sandbox.policy.* / sandbox.run_mode.* 已由 Rust 网关提供",
    })
}
