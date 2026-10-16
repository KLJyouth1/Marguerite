//! 密钥**有效性**探测（唯一实现）。
//!
//! 原实现私有于 `cli/program.rs::auth_probe_check`；P0-1 把它下沉到 doctor 模块，
//! 使 CLI `doctor --probe` 与 RPC `doctor.status{probe:true}` **同源**（结论一致，
//! 不再各写一份）。
//!
//! # 契约
//! - 显式 opt-in 才发起联网请求（默认离线契约由调用方保证）。
//! - `GET {base_url}/models`（OpenAI 兼容只读端点）：2xx = 密钥有效，
//!   401/403 = 密钥被拒，其它状态码 / 网络失败 / 超时 = 无法判定。
//! - 任何分支都不输出密钥明文。

use std::time::Duration;

use crate::persistence::config::Config;

/// 鉴权探测结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthProbeOutcome {
    /// 2xx：密钥有效。
    Valid,
    /// 401/403：密钥被拒。
    Rejected { status: u16 },
    /// 其它状态码 / 网络失败 / 超时：无法判定。
    Indeterminate { reason: String },
    /// 缺少前置条件（无 key / 无 base_url）：未探测。
    Skipped { reason: String },
}

/// 探测密钥有效性。`timeout` 为单次 HTTP 请求超时。
///
/// 同步外壳：仅供 **CLI / 单测** 这类「已处于 `main` 同步上下文、无外层
/// runtime」的调用方使用。它会自建 runtime 并 `block_on`。
///
/// ⚠️ 该外壳**不可**在 async 上下文（如网关 dispatch）里调用：tokio 的
/// `enter_runtime` 保护会在同一线程上二次进入 runtime 时 panic
/// （`Cannot start a runtime from within a runtime`）。async 调用方必须改用
/// [`probe_auth_async`]——它把网络请求交给调用方已有的 runtime 驱动。
pub fn probe_auth(config: &Config, timeout: Duration) -> AuthProbeOutcome {
    // 同步外壳的唯一合法语境是「无外层 runtime」。若误在 runtime 内被调用，
    // 不静默、也不 panic：直接给出可判定的结论，把契约违约暴露给调用方。
    if tokio::runtime::Handle::try_current().is_ok() {
        return AuthProbeOutcome::Indeterminate {
            reason: "probe_auth 同步外壳被误用于 async 上下文（应改用 probe_auth_async）".to_string(),
        };
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            return AuthProbeOutcome::Indeterminate {
                reason: format!("无法创建运行时: {e}"),
            }
        }
    };
    let config = config.clone();
    rt.block_on(probe_auth_async(&config, timeout))
}

/// 探测密钥有效性的**纯异步**实现（方案 B）。
///
/// 这是网关（RPC `doctor.status`）与任何 async 调用方的唯一正确入口：它自身
/// **不**创建 runtime、**不**调用 `block_on`，因此可以安全地在 slow runtime
/// 的 worker 线程上被 poll。真实 HTTP 请求由调用方所在的 runtime 驱动。
///
/// 前置校验（缺 key / 缺 base_url → [`AuthProbeOutcome::Skipped`]）与
/// HTTP 状态码映射逻辑与同步外壳完全同源（后者只是本函数的驱动封装）。
pub async fn probe_auth_async(config: &Config, timeout: Duration) -> AuthProbeOutcome {
    let key = config.model.api_key.trim();
    if key.is_empty() {
        return AuthProbeOutcome::Skipped {
            reason: "未配置密钥".to_string(),
        };
    }
    let base = config.model.base_url.trim();
    if base.is_empty() {
        return AuthProbeOutcome::Skipped {
            reason: "未配置 base_url".to_string(),
        };
    }
    let url = format!("{}/models", base.trim_end_matches('/'));

    let outcome: Result<u16, String> = async {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;
        client
            .get(&url)
            .bearer_auth(key)
            .send()
            .await
            .map(|resp| resp.status().as_u16())
            .map_err(|e| format!("请求失败: {e}"))
    }
    .await;

    match outcome {
        Ok(status) if (200..300).contains(&status) => AuthProbeOutcome::Valid,
        Ok(status @ (401 | 403)) => AuthProbeOutcome::Rejected { status },
        Ok(status) => AuthProbeOutcome::Indeterminate {
            reason: format!("探测端点返回 HTTP {status}"),
        },
        Err(reason) => AuthProbeOutcome::Indeterminate { reason },
    }
}