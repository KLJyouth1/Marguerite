//! 第 7 项：模型 API 连通性诊断器。
//!
//! 配置的 provider 探活。**无 API key 时返回 `DEGRADED` +「未配置密钥」指引**，
//! 绝不因缺 key 判 FAIL。探测允许失败但明示（离线环境）。

use std::time::Duration;

use crate::doctor::{DoctorItem, Status};
use crate::persistence::config::Config;

/// 鉴权探测档位（P0-1）。
///
/// - `Off`：默认，保持 doctor 的离线契约（TCP 可达即 OK，detail 注明未校验密钥真伪）。
/// - `On`：显式 opt-in，TCP 可达后发起一次真实 `GET /models` 校验密钥有效性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProbeTier {
    Off,
    On,
}

/// 第 7 项：模型 API 连通（同步外壳）。
///
/// 仅供 **CLI / 单测**（无外层 runtime）。async 调用方（网关 RPC）必须用
/// [`check_model_api_async`]：本外壳内部会 `block_on`，在已持有 runtime 的
/// 线程上调用会 panic。
pub fn check_model_api(config: &Config, probe: AuthProbeTier) -> DoctorItem {
    if tokio::runtime::Handle::try_current().is_ok() {
        return DoctorItem {
            id: "model_api".to_string(),
            label: "模型 API 连通".to_string(),
            status: Status::Degraded,
            detail: "探测被跳过：check_model_api 同步外壳被误用于 async 上下文".to_string(),
            fix_hint: "这是程序缺陷：async 调用方应改用 check_model_api_async".to_string(),
        };
    }
    let config = config.clone();
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            return DoctorItem {
                id: "model_api".to_string(),
                label: "模型 API 连通".to_string(),
                status: Status::Degraded,
                detail: format!("探测运行时创建失败: {e}（已降级）"),
                fix_hint: "检查 tokio 运行时可用性后重试".to_string(),
            }
        }
    };
    rt.block_on(check_model_api_async(&config, probe))
}

/// 第 7 项：模型 API 连通（**纯异步**，方案 B —— 网关 RPC 的唯一正确入口）。
///
/// - 未配置 API key → `DEGRADED`（未配置密钥指引，非 FAIL）。
/// - base_url 未配置 → `DEGRADED`（无法探活）。
/// - TCP 探活成功 → probe=Off 时 `OK`（未校验密钥真伪）；probe=On 时按真实鉴权结论。
/// - 探测失败 / 超时 → `DEGRADED`（离线/网络指引，非 FAIL）。
///
/// 自身不创建 runtime、不 `block_on`，因此在 slow runtime 的 worker 线程上
/// 安全：网络探针由该 runtime 驱动，不会触发 tokio 的嵌套 runtime 保护。
pub async fn check_model_api_async(config: &Config, probe: AuthProbeTier) -> DoctorItem {
    let provider = config.model.provider.trim();
    let base_url = config.model.base_url.trim();
    let api_key = config.model.api_key.as_str();

    // 1) 未配置密钥：引导配置，不判 FAIL。
    if api_key.is_empty() {
        return DoctorItem {
            id: "model_api".to_string(),
            label: "模型 API 连通".to_string(),
            status: Status::Degraded,
            detail: format!(
                "provider={} model={} 未配置密钥（已降级，不计 FAIL）",
                provider,
                config.model.model_name
            ),
            fix_hint: "设置环境变量 MARGUERITE_API_KEY 或在 ~/.marguerite/.env 写入 MARGUERITE_API_KEY=<key>".to_string(),
        };
    }

    // 2) base_url 未配置：无法探活。
    if base_url.is_empty() {
        return DoctorItem {
            id: "model_api".to_string(),
            label: "模型 API 连通".to_string(),
            status: Status::Degraded,
            detail: format!("provider={provider} 未配置 base_url（已降级）"),
            fix_hint: "在 config.yaml 的 model.base_url 或环境变量 MARGUERITE_BASE_URL 配置 API 地址".to_string(),
        };
    }

    // 3) 解析 host:port 做 TCP 短超时探测（离线安全，不发起真实鉴权请求）。
    let (host, port) = match parse_host_port(base_url) {
        Some(hp) => hp,
        None => {
            return DoctorItem {
                id: "model_api".to_string(),
                label: "模型 API 连通".to_string(),
                status: Status::Degraded,
                detail: format!("无法解析 base_url: {base_url}"),
                fix_hint: "请检查 config.yaml 的 model.base_url 是否为合法 URL".to_string(),
            }
        }
    };

    match tcp_probe_async(&host, port).await {
        Ok(()) => match probe {
            AuthProbeTier::Off => DoctorItem {
                id: "model_api".to_string(),
                label: "模型 API 连通".to_string(),
                status: Status::Ok,
                detail: format!(
                    "provider={provider} TCP 可达 {host}:{port}（key 已配置，未校验密钥真伪；可用 doctor --probe / 诊断面板深检）"
                ),
                fix_hint: String::new(),
            },
            AuthProbeTier::On => probe_auth_item_async(config, provider).await,
        },
        Err(e) => DoctorItem {
            id: "model_api".to_string(),
            label: "模型 API 连通".to_string(),
            status: Status::Degraded,
            detail: format!("provider={provider} TCP 不可达 {host}:{port}: {e}（已降级）"),
            fix_hint: "检查网络 / 代理 / base_url 端口；离线环境此项目为 DEGRADED（不阻断启动）".to_string(),
        },
    }
}

/// probe=On 时把 [`crate::doctor::auth_probe::probe_auth_async`] 的结论映射为 doctor 项。
///
/// 任何分支都不输出密钥明文（只输出 URL 与状态码 / 原因）。
async fn probe_auth_item_async(config: &Config, provider: &str) -> DoctorItem {
    use crate::doctor::auth_probe::{probe_auth_async, AuthProbeOutcome};

    let base = config.model.base_url.trim();
    let url = format!("{}/models", base.trim_end_matches('/'));
    let item = |status: Status, detail: String, fix_hint: String| DoctorItem {
        id: "model_api".to_string(),
        label: "模型 API 连通".to_string(),
        status,
        detail,
        fix_hint,
    };

    match probe_auth_async(config, Duration::from_secs(8)).await {
        AuthProbeOutcome::Valid => item(
            Status::Ok,
            format!("provider={provider} 密钥有效（GET {url} → HTTP 2xx）"),
            String::new(),
        ),
        AuthProbeOutcome::Rejected { status } => item(
            Status::Degraded,
            format!("provider={provider} 密钥被拒（GET {url} → HTTP {status}，鉴权失败）"),
            "密钥无效或已过期：前往「设置 → 密钥」更新，或更新 MARGUERITE_API_KEY".to_string(),
        ),
        AuthProbeOutcome::Indeterminate { reason } => item(
            Status::Degraded,
            format!("provider={provider} 无法判定（{reason}）"),
            "检查网络 / 代理 / base_url 后重试（离线环境请关闭深度校验）".to_string(),
        ),
        AuthProbeOutcome::Skipped { reason } => item(
            Status::Degraded,
            format!("provider={provider} 未探测（{reason}）"),
            "先配置密钥与 base_url，再进行深度校验".to_string(),
        ),
    }
}

/// 解析 `https://host:port/path` 或 `host:port` → `(host, port)`。
fn parse_host_port(url: &str) -> Option<(String, u16)> {
    let s = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string();
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return Some((h.to_string(), port));
        }
    }
    // 无显式端口：https 默认 443。
    Some((s, 443))
}

/// TCP 短超时探活的**纯异步**实现（方案 B）。
///
/// 自身不创建 runtime、不调用 `block_on`，可安全地在 slow runtime 的 worker
/// 线程上被 poll——这正是 RPC `doctor.status` 需要的性质。
///
/// 历史上这里是「自建 runtime + `block_on`」的同步实现，被搬进 async 上下文后
/// 必然触发 tokio 的 `Cannot start a runtime from within a runtime`；现已改为
/// 纯异步，嵌套 runtime 的可能从根上消失。
async fn tcp_probe_async(host: &str, port: u16) -> std::io::Result<()> {
    match tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "探测超时 (3s)",
        )),
    }
}
