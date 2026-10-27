//! 系统状态 / 健康检查方法族
//!
//! 诚实原则：各方法返回真实运行时状态，绝不静默降级。

use serde_json::{json, Value};

use crate::gateway::methods::GatewayContext;
use crate::gateway::protocol::PROTOCOL_VERSION;
use crate::persistence::config::Config;

/// `health`：网关健康探针（最轻量，供前端心跳）。
pub fn health(ctx: &GatewayContext) -> Value {
    json!({
        "status": "ok",
        "engine": "rust",
        "protocol": PROTOCOL_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": ctx.started_at.elapsed().as_millis(),
        "data_dir": Config::data_dir().to_string_lossy(),
        "api_key_configured": !ctx.config.model.api_key.is_empty(),
    })
}

/// `status`：系统状态摘要。
pub fn system_status(ctx: &GatewayContext) -> Value {
    json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": ctx.started_at.elapsed().as_millis(),
        "provider": &ctx.config.model.provider,
        "model": &ctx.config.model.model_name,
        "engine": "rust",
    })
}

/// `doctor.status`：诊断报告。
///
/// 复用 [`crate::doctor::run_doctor_with_config`]（与 CLI `marguerite doctor --json`
/// **同源**），返回完整 9 项检查 + summary + 报告级状态。此前的 3 项简化版
/// （config/loopback/model）与 CLI 的报告互不互通，属契约漂移（R5），
/// 会造成 WebUI 拿到的诊断信息远少于 CLI。现统一到完整的 9 项报告契约。
/// （第 9 项 `tier_caps` 是后加的：核验各档案的档位能力声明是否被证伪，
/// 见 `crate::doctor::check_tier_caps`；本文件此前写死「8 项」是陈旧描述。）
///
/// P0-1：可选 `params.probe`（默认 false）。`probe = true` 时第 7 项（model_api）
/// 在 TCP 可达后发起一次真实 `GET /models` 校验密钥真伪（401 → 密钥被拒），
/// 与 CLI `doctor --probe` 共用 [`crate::doctor::auth_probe::probe_auth_async`]，结论同源。
/// 兼容既有 WebUI 的 `deep` 参数（诊断面板「Deep check」按钮），二者同义。
///
/// # 为什么是 async（而非同步 `block_on`）
///
/// RPC dispatch 运行在 tokio worker 线程上。若在此同步地「自建 runtime +
/// `block_on`」跑网络探针，tokio 的 `enter_runtime` 保护会二次进入 runtime 并
/// panic（`Cannot start a runtime from within a runtime`）——实测在真机上出现 2 次。
/// 本函数因此直接 await [`crate::doctor::run_doctor_with_config_probe_async`]：
/// 探针由当前 slow runtime 驱动，全程无 `block_on`，因此既能跑在 worker 线程上，
/// 又不会拖住承载 WebSocket 的那个 runtime（见 `server.rs` 的 `SLOW_RUNTIME`）。
pub async fn doctor_status(ctx: &GatewayContext, params: &Value) -> Value {
    let probe = params
        .get("probe")
        .and_then(Value::as_bool)
        .or_else(|| params.get("deep").and_then(Value::as_bool))
        .unwrap_or(false);
    let tier = if probe {
        crate::doctor::connectivity::AuthProbeTier::On
    } else {
        crate::doctor::connectivity::AuthProbeTier::Off
    };
    let report = crate::doctor::run_doctor_with_config_probe_async(&ctx.config, tier).await;
    // run_doctor 输出的是可解析 JSON 字符串；解析失败时按诚实原则回退为错误对象，
    // 而非静默返回空/假成功。
    serde_json::from_str(&report).unwrap_or_else(|e| {
        json!({
            "status": "MISSING",
            "version": env!("CARGO_PKG_VERSION"),
            "detail": format!("doctor 报告解析失败: {e}"),
        })
    })
}

/// `runtime.status`：重型运行时（ASR / 本地量化模型）诚实状态清单。
pub fn runtime_status() -> Value {
    serde_json::to_value(crate::runtime::heavy_runtime_status()).unwrap_or_else(|_| json!([]))
}

/// `host.exit_evidence`：宿主（Python 侧车）最近一次退出的结构化证据（S5）。
///
/// 回答的是「侧车为什么没了」——此前 Rust 两处拿到 `ExitStatus` 都直接丢弃，
/// 界面上只剩一句「健康探测失败」，用户无从判断是该重启、该改端口、还是该查 Key。
///
/// # 契约：`exited: false` **不是错误**
///
/// 「从未退出过」是完全正常的状态（侧车健康跑着、或刚启动还没崩过），此时返回
/// `exited: false` + `evidence: null`。刻意**不**返回 `RpcError`：前端要的是
/// 「有没有证据」这个二元事实，把它表达成异常会逼迫每个调用方写 try/catch，
/// 且在正常路径上刷无意义的错误日志。
///
/// # 安全边界
///
/// `stderr_tail` 可能含文件路径 / URL / 用户名。它只允许进日志与 UI 的折叠详情，
/// **绝不可**做主文案或错误码插值（与 `crate::model::error_taxonomy` 的纪律一致）。
pub fn host_exit_evidence() -> Value {
    // 优先读内存槽（最新，含本次运行早退）；没有再退到落盘记录（跨重启可见）。
    let (evidence, source) = match crate::pyengine::sidecar::last_exit_evidence() {
        Some(ev) => (Some(ev), "memory"),
        None => (
            crate::pyengine::sidecar::load_record()
                .and_then(|rec| rec.last_exit)
                .map(|rec| crate::pyengine::sidecar::HostExitEvidence {
                    code: rec.code,
                    uptime_ms: rec.uptime_ms,
                    stderr_tail: rec.stderr_tail,
                    observed: rec.observed,
                }),
            "record",
        ),
    };
    // 「算不算证据」的判据：**观测到进程消失**即为证据。
    //
    // 旧判据是 `code.is_some() || !stderr_tail.is_empty()`，在复用路径
    // （`PyEngineSidecar::attach`）下会漏掉真实发生的退出：那不是我们的子进程 →
    // `code` 恒为 `None`；被 `taskkill /F` 掉的健康侧车 stderr 又是空的。两个条件
    // 同时不成立 ⇒ 明明崩了却回 `exited:false`，正是 S5 实机验证失败的成因。
    //
    // `observed` 缺省为 `false`（老记录无此字段），所以历史记录仍走旧判据，
    // 不会被误当成新的观测结果。
    let evidence = evidence.filter(|ev| ev.observed || ev.code.is_some() || !ev.stderr_tail.is_empty());
    let sidecar = crate::gateway::server::py_sidecar();
    json!({
        "exited": evidence.is_some(),
        "source": source,
        "evidence": evidence,
        // 侧车当前是否在位 + 已存活多久：让 UI 能在「已退出」与「正在重启」之间
        // 给出不同提示，而不是一律显示成失败。
        "sidecar_present": sidecar.is_some(),
        "sidecar_uptime_ms": sidecar.map(|s| s.uptime_ms()),
    })
}

/// `host.metrics`：宿主（Python 侧车）最近一次资源采样（P1-6）。
///
/// 只做采集暴露：守护任务每 30s 读一次子进程 RSS（Windows
/// `GetProcessMemoryInfo` / Linux `/proc/<pid>/status`），这里把最近一次
/// 采样如实回给 UI。**不做预警重启**——告警与重启路径解耦，避免与
/// P0-1 熔断互相干扰。
///
/// # 契约：从未采样过（侧车未跑过 / 刚启动）→ `available: false`，不是 RPC 错误。
pub fn host_metrics() -> Value {
    let m = crate::pyengine::sidecar::sidecar_metrics_snapshot();
    json!({
        "available": m.is_some(),
        "pid": m.map(|x| x.pid),
        "rss_bytes": m.map(|x| x.rss_bytes),
        "rss_mb": m.map(|x| x.rss_bytes / 1_048_576),
        "sampled_at_ms": m.map(|x| x.sampled_at_ms),
        "uptime_ms": m.map(|x| x.uptime_ms),
    })
}

#[cfg(test)]
mod host_exit_tests {
    use super::*;

    /// S5 契约守卫：`host.exit_evidence` 在**没有发生过退出**时必须返回
    /// `exited: false` + `evidence: null`，而**不是** RPC 错误。
    ///
    /// 这条路径是最容易被误写成 `Err` 的（"没有证据"看起来像"查询失败"），
    /// 一旦回归，前端每次启动都会记一条无意义的错误。
    ///
    /// ⚠️ 必须**隔离**：`host_exit_evidence()` 会读内存槽 + 落盘的
    /// `sidecar.json`（`record_path()` 默认落在真实用户 data_dir）。任何一次
    /// 真实运行过侧车退出的机器上，那个文件都存在 —— 不隔离的话，本用例在
    /// 「干净 CI」上绿、在「跑过一次的开发者机器」上红，属于环境相关假红。
    #[test]
    fn host_exit_evidence_reports_no_record_without_erroring() {
        // 记录路径指向一个刻意不存在的临时文件；并清掉内存槽里可能由
        // 同进程其它用例留下的证据。
        let dir = std::env::temp_dir().join(format!(
            "marguerite-host-exit-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        crate::pyengine::sidecar::set_record_path_override(dir.join("absent-sidecar.json"));
        crate::pyengine::sidecar::clear_last_exit_evidence();

        let value = host_exit_evidence();
        assert_eq!(
            value["exited"], json!(false),
            "从未退出过时exited 必须是 false"
        );
        assert!(
            value["evidence"].is_null(),
            "从未退出过时 evidence 必须是 null，实际 {}",
            value["evidence"]
        );
        // 形状守卫：这三个字段必须一直在，否则前端会拿到 undefined 而崩。
        for field in ["exited", "source", "sidecar_present", "sidecar_uptime_ms"] {
            assert!(
                value.get(field).is_some(),
                "host.exit_evidence 缺字段 {field}: {value}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 证据里的 stderr 尾部必须能原样透传（UI 折叠详情依赖它），
    /// 且**只**作为 data 字段存在，不得被塞进任何文案字段。
    #[test]
    fn host_exit_evidence_carries_tail_as_data_not_as_message() {
        let ev = crate::pyengine::sidecar::HostExitEvidence {
            code: Some(3),
            uptime_ms: 4321,
            stderr_tail: "Traceback: <redacted-path>".to_string(),
            observed: true,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["code"], json!(3));
        assert_eq!(json["uptime_ms"], json!(4321));
        assert!(json.get("stderr_tail").is_some());
        // 结构里不存在任何叫 message / text / detail 的文案字段
        for forbidden in ["message", "text", "detail", "summary"] {
            assert!(
                json.get(forbidden).is_none(),
                "证据里不得出现文案字段 {forbidden}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use std::sync::Arc;
    use std::time::Instant;

    fn ctx() -> GatewayContext {
        GatewayContext {
            config: Config::default(),
            token: "test".into(),
            started_at: Instant::now(),
            bus: Arc::new(EventBus::default()),
        }
    }

    /// RPC `doctor.status` 必须与 CLI `--json` 同源（复用 run_doctor 的完整 9 项报告），
    /// 而不是另一套 3 项简化版——否则 WebUI 拿到的诊断信息远少于 CLI。
    ///
    /// 用 `#[tokio::test]`：这正是生产路径的语境（跑在 runtime 上 await），
    /// 若探针内部再 `block_on` 就会 panic，因此本测试同时是嵌套 runtime 的护栏。
    #[tokio::test]
    async fn doctor_status_returns_full_nine_item_report() {
        let value = doctor_status(&ctx(), &json!({})).await;
        let items = value
            .get("items")
            .and_then(Value::as_array)
            .expect("doctor.status must carry an items array");
        assert_eq!(items.len(), 9, "doctor.status must report 9 items like CLI --json");
        // 每项都带 id/label/status/detail/fix_hint
        for item in items {
            for field in ["id", "label", "status", "detail", "fix_hint"] {
                assert!(item.get(field).is_some(), "item missing field {field}: {item}");
            }
        }
        // 含有 summary 与报告级 status
        assert!(value.get("summary").is_some(), "doctor.status must carry summary");
        assert!(value.get("version").is_some(), "doctor.status must carry version");
        // 顶层 status 只允许 OK / MISSING / DEGRADED
        let top = value.get("status").and_then(Value::as_str).unwrap_or("");
        assert!(
            matches!(top, "OK" | "MISSING" | "DEGRADED"),
            "doctor.status top status must be one of OK/MISSING/DEGRADED, got {top}"
        );
        // 顶层字段名必须是完整报告形态，而非旧 3 项（checks/config/loopback/model）
        assert!(
            value.get("checks").is_none(),
            "doctor.status must not return the legacy 3-item checks shape"
        );
    }

    /// doctor.status 绝不把 API key 明文带出 RPC 边界。
    #[tokio::test]
    async fn doctor_status_never_leaks_api_key() {
        let mut c = ctx();
        c.config.model.api_key = "sk-super-secret".into();
        let value = doctor_status(&c, &json!({})).await;
        let s = value.to_string();
        assert!(!s.contains("sk-super-secret"), "doctor.status leaked api key: {s}");
        assert!(!s.contains("api_key"), "doctor.status must not expose api_key field: {s}");
        // status 字段不允许硬编码 key
        assert!(value.get("status").and_then(Value::as_str).is_some());
    }
}
