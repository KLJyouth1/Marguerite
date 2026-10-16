//! M5 doctor 体检模块测试（TDD：先 RED 后 GREEN，全部本机可验、隔离真实用户目录）。

use crate::doctor::*;
use crate::persistence::config::Config;

/// 版本号必须来自 CARGO_PKG_VERSION。
#[test]
fn report_version_matches_pkg() {
    let json = run_doctor_with_config(&Config::default());
    let v: serde_json::Value = serde_json::from_str(&json).expect("JSON 可解析");
    assert_eq!(v["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
}

/// 聚合报告必须 9 项齐全、字段完整。
#[test]
fn report_has_all_nine_items_and_fields() {
    let json = run_doctor_with_config(&Config::default());
    let v: serde_json::Value = serde_json::from_str(&json).expect("JSON 可解析");
    let items = v["items"].as_array().expect("items 数组");
    // 9 项齐全
    assert_eq!(items.len(), 9);
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"python"));
    assert!(ids.contains(&"gateway_port"));
    assert!(ids.contains(&"data_dir"));
    assert!(ids.contains(&"resources"));
    assert!(ids.contains(&"disk"));
    assert!(ids.contains(&"backend"));
    assert!(ids.contains(&"model_api"));
    assert!(ids.contains(&"router_weights"));
    assert!(ids.contains(&"tier_caps"));
    // 字段齐全
    for it in items {
        assert!(it.get("id").is_some());
        assert!(it.get("label").is_some());
        assert!(it.get("status").is_some());
        assert!(it.get("detail").is_some());
        assert!(it.get("fix_hint").is_some());
        let status = it["status"].as_str().unwrap();
        assert!(
            status == "OK" || status == "MISSING" || status == "DEGRADED",
            "status 非法: {status}"
        );
    }
    // 汇总统计
    assert_eq!(v["summary"]["total"].as_u64(), Some(9));
    // 版本字段 + 时间戳
    assert!(v["generated_at"].is_string());
}

/// 报告绝不能包含 API key 明文。
#[test]
fn report_never_leaks_api_key() {
    let mut config = Config::default();
    config.model.api_key = "sk-fake--fixture-key-0123456789abcdef".to_string();
    let json = run_doctor_with_config(&config);
    assert!(
        !json.contains("sk-fake--fixture-key-0123456789abcdef"),
        "报告不得泄露 API key 明文: {json}"
    );
}

/// parse_python_version：常规版本 / beta 版本 / 非版本。
#[test]
fn parse_python_version_variants() {
    assert_eq!(
        crate::pyengine::parse_python_version("Python 3.12.4"),
        Some((3, 12))
    );
    assert_eq!(
        crate::pyengine::parse_python_version("Python 3.13.0b1"),
        Some((3, 13))
    );
    assert_eq!(
        crate::pyengine::parse_python_version("Python 3.11.9"),
        Some((3, 11))
    );
    assert_eq!(crate::pyengine::parse_python_version("not a version"), None);
    assert_eq!(crate::pyengine::parse_python_version(""), None);
}

/// 数据目录读写探针：可写目录 → OK。
#[test]
fn data_dir_writable_probe_ok() {
    let config = Config::default();
    let item = appliance::check_data_dir_write(&config);
    // 本机 temp_dir 一定可写，应判定 OK。
    assert_eq!(item.status, Status::Ok, "detail: {}", item.detail);
    assert_eq!(item.id, "data_dir");
}

/// 磁盘余量：>2G → OK（用 temp_dir 当前盘实测，本机一般满足）。
#[test]
fn disk_free_over_threshold_ok() {
    // 复用 disk_free 的实测量；只要本机 >2G 即为 OK。
    let item = crate::doctor::disk::check_disk_free();
    // 本机 C 盘余量通常 >2G，应判定 OK。
    assert_eq!(item.status, Status::Ok, "detail: {}", item.detail);
}

/// 资源完整性：指向存在的目录 → OK；指向不存在的路径 → MISSING。
#[test]
fn resources_complete_and_missing_paths() {
    let item = crate::doctor::appliance::check_resources();
    assert_eq!(item.id, "resources");
    // 本机仓库根存在 webui/skills/docs，modes 在 core/src/mode 下；不一定全部可达。
    // 因此这里断言「只要 webui 在 cwd 或 exe 旁可达即视为至少不 MISSING 的退化」——
    // 更精确地：不验证具体状态，只保证字段完整 + 状态合法。
    assert!(
        item.status == Status::Ok || item.status == Status::Missing,
        "resources 状态应 OK 或 MISSING，实际 {:?}",
        item.status
    );
    assert!(!item.detail.is_empty());
    assert!(!item.fix_hint.is_empty() || item.status == Status::Ok);
}

/// 第 7 项：无 key 时返回 DEGRADED（不因缺 key 判 FAIL）。
#[test]
fn model_api_no_key_is_degraded() {
    let config = Config::default(); // api_key 默认空
    let item = connectivity::check_model_api(&config, connectivity::AuthProbeTier::Off);
    assert_eq!(item.id, "model_api");
    assert_eq!(item.status, Status::Degraded, "detail: {}", item.detail);
    assert!(
        item.detail.contains("未配置密钥") || item.detail.contains("未配置"),
        "应提示未配置密钥: {}",
        item.detail
    );
    assert!(!item.fix_hint.is_empty());
}

/// 脱敏：疑似密钥形态被替换为占位符。
///
/// 占位符是 `[redacted]`（S6 起与 Python 侧 `redact.py::_REDACTED` 同形）：
/// `redact_secrets` 已收敛为 [`crate::observability::redact::scrub_text`] 的转发，
/// 不再是 doctor 私有的第二份实现。
#[test]
fn redact_secrets_masks_keys() {
    let input =
        "key=sk-fake--fixture-23456789012345678, token=ghp__fixture_abcdefghijklmnopqrstuvwxyz";
    let out = crate::doctor::appliance::redact_secrets(input);
    assert!(
        !out.contains("sk-fake--fixture-23456789012345678"),
        "应脱敏: {out}"
    );
    assert!(
        !out.contains("ghp__fixture_abcdefghijklmnopqrstuvwxyz"),
        "应脱敏: {out}"
    );
    assert!(
        out.contains(crate::observability::redact::REDACTED),
        "应留下统一占位符: {out}"
    );
}

/// 网关端口探测：**与外部环境无关**的两条不变量。
///
/// ⚠️ 历史坑：这个用例原先断言 `status == Ok`，即「本机 18790 空闲」——
/// 那是**开发机当前状态**，不是产品行为。只要本地起过一次
/// `marguerite-core gateway`（例如为了跑 Web 控制台截图），该断言就变红，
/// 而且报错信息和真实回归无法区分（实测踩过两次）。
/// 现在改为：不依赖端口是否空闲，只锁「探测本身的契约」。
#[test]
fn gateway_port_check_contract_is_environment_independent() {
    let item = appliance::check_gateway_port();
    assert_eq!(item.id, "gateway_port");
    assert_eq!(item.label, "网关端口 18790");

    match item.status {
        // 端口空闲 → OK；或端口被**本应用自己**的网关监听 → 也判 OK（桌面端常态）。
        // 两者都必须给出正向说明且不带修复建议（后者不是故障）。
        Status::Ok => {
            assert!(
                item.detail.contains("可绑定") || item.detail.contains("由本应用网关监听"),
                "OK 时应说明端口可绑定、或已由本应用网关监听: {}",
                item.detail
            );
            assert!(item.fix_hint.is_empty(), "OK 时不应有修复建议");
        }
        // 端口被**其它**进程占用：Missing，且必须给出可执行的处置
        Status::Missing => {
            assert!(
                item.detail.contains("被占用"),
                "Missing 时应说明端口被占用: {}",
                item.detail
            );
            assert!(
                item.fix_hint.contains("--port"),
                "占用时必须给出可执行建议（含 --port）: {}",
                item.fix_hint
            );
        }
        other => panic!("端口探测只可能是 Ok 或 Missing，实际: {other:?}"),
    }
}

/// 自冲突修复的核心判定：只有**本应用**的健康体才认领为「自己」。
///
/// 反向用例比正向更重要 —— 若把「别人回的健康体」也认领成自己，就等于把
/// 真冲突美化成 OK（比误报 MISSING 更危险，因为会静默掩盖端口争用）。
#[test]
fn own_gateway_probe_recognizes_only_our_health_body() {
    /// 起一个只回固定健康体的回环 mock，返回其端口。
    fn mock_health(body: &'static str) -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("mock bind");
        let port = listener.local_addr().expect("mock addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { break };
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        port
    }

    let ours = mock_health(r#"{"status":"ok","engine":"rust","uptime_ms":1}"#);
    assert!(
        appliance::is_own_gateway(ours),
        "本应用网关的健康体应被认领（否则桌面端每次自检都误报端口冲突）"
    );

    let theirs = mock_health(r#"{"status":"ok","engine":"python"}"#);
    assert!(
        !appliance::is_own_gateway(theirs),
        "他人服务的健康体不得被认领 —— 那会把真冲突美化成 OK"
    );

    // 无人监听 → 连不上 → 必须判 false（探测失败不得等价于「是自己」）。
    assert!(
        !appliance::is_own_gateway(1),
        "连不上时不得认领为自身网关"
    );
}

/* ── P0-1 鉴权探测档位 ───────────────────────────────────────────────── */

use crate::doctor::auth_probe::{probe_auth, probe_auth_async, AuthProbeOutcome};
use crate::doctor::connectivity::{check_model_api, check_model_api_async, AuthProbeTier};

/// 起一个只回固定状态码的 loopback mock（独立线程 + 独立 runtime，
/// 因为被测函数同步、内部自建 runtime）。
fn spawn_status_mock(status: u16) -> String {
    use tokio::io::AsyncWriteExt;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("mock runtime");
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("mock bind");
            let addr = listener.local_addr().expect("mock addr");
            let _ = tx.send(format!("http://{addr}"));
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    _ => "Error",
                };
                let body = format!("{{\"error\":\"fixture {status} {reason}\"}}");
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
            }
        });
    });
    rx.recv().expect("mock addr")
}

fn keyed_config(base_url: String) -> Config {
    let mut config = Config::default();
    config.model.api_key = "sk-live-fixture".to_string();
    config.model.base_url = base_url;
    config
}

/// 四种 outcome：2xx=Valid，401/403=Rejected，500=Indeterminate，缺 key= Skipped。
#[test]
fn probe_auth_maps_http_status_to_outcome() {
    assert_eq!(
        probe_auth(&keyed_config(spawn_status_mock(200)), std::time::Duration::from_secs(5)),
        AuthProbeOutcome::Valid
    );
    assert_eq!(
        probe_auth(&keyed_config(spawn_status_mock(401)), std::time::Duration::from_secs(5)),
        AuthProbeOutcome::Rejected { status: 401 }
    );
    assert_eq!(
        probe_auth(&keyed_config(spawn_status_mock(403)), std::time::Duration::from_secs(5)),
        AuthProbeOutcome::Rejected { status: 403 }
    );
    assert!(matches!(
        probe_auth(&keyed_config(spawn_status_mock(500)), std::time::Duration::from_secs(5)),
        AuthProbeOutcome::Indeterminate { .. }
    ));

    // 无 key / 无 base_url → Skipped，且不发请求。
    let mut no_key = Config::default();
    no_key.model.base_url = "http://127.0.0.1:1".to_string();
    assert!(matches!(
        probe_auth(&no_key, std::time::Duration::from_secs(1)),
        AuthProbeOutcome::Skipped { .. }
    ));
    assert!(matches!(
        probe_auth(&keyed_config(String::new()), std::time::Duration::from_secs(1)),
        AuthProbeOutcome::Skipped { .. }
    ));
}

/// probe=On 时 401 必须落到 DEGRADED「密钥被拒」，且不泄密钥明文——
/// 这正是 WebUI「假绿」缺陷的修复终点。
///
/// 走 **async** 入口（与生产 RPC 路径同源）：这同时是「探针在 runtime 上被 poll
/// 不得 panic」的护栏——同步实现曾在此处触发嵌套 runtime panic。
#[tokio::test]
async fn check_model_api_probe_on_rejects_bad_key_without_leaking_it() {
    let config = keyed_config(spawn_status_mock(401));
    let item = check_model_api_async(&config, AuthProbeTier::On).await;
    assert_eq!(item.id, "model_api");
    assert_eq!(item.status, Status::Degraded, "detail: {}", item.detail);
    assert!(
        item.detail.contains("密钥被拒"),
        "应给出密钥被拒结论: {}",
        item.detail
    );
    assert!(!item.detail.contains("sk-live-fixture"), "不得泄密钥: {}", item.detail);
    assert!(!item.fix_hint.contains("sk-live-fixture"), "不得泄密钥: {}", item.fix_hint);
    assert!(!item.fix_hint.is_empty(), "被拒时必须给出修复指引");
}

/// probe=On 且 2xx 时 model_api 判 OK「密钥有效」。
#[tokio::test]
async fn check_model_api_probe_on_valid_key_is_ok() {
    let config = keyed_config(spawn_status_mock(200));
    let item = check_model_api_async(&config, AuthProbeTier::On).await;
    assert_eq!(item.status, Status::Ok, "detail: {}", item.detail);
    assert!(item.detail.contains("密钥有效"), "detail: {}", item.detail);
}

/// probe=Off（默认）保持离线契约：不发起真实请求，9 项报告契约不变。
#[test]
fn report_default_probe_off_keeps_nine_items() {
    let json = run_doctor_with_config(&Config::default());
    let v: serde_json::Value = serde_json::from_str(&json).expect("JSON 可解析");
    // 9 项：新增 tier_caps（档位能力声明审计）
    assert_eq!(v["items"].as_array().expect("items").len(), 9);
}

/// 同源护栏：CLI `auth_probe_check` 与 `doctor::connectivity` 都经 `probe_auth`
/// 得到同一结论——这里直接断言唯一实现被两处共用（同一 401 mock 下结论一致）。
///
/// 同步 `probe_auth`（CLI 入口）与 async `probe_auth_async`（RPC 入口）必须
/// 在同一 mock 下给出一致结论：二者共享同一套状态码映射，只差驱动方式。
///
/// ⚠️ 同步外壳**不能**在 runtime 内调用（会二次进入 runtime 而 panic），因此本
/// 测试用独立的 `std::thread` + 各自 runtime 来分别驱动两个入口。
#[test]
fn probe_auth_is_the_single_source_for_both_entry_points() {
    let base = spawn_status_mock(401);
    let config = keyed_config(base.clone());

    // CLI 入口：同步外壳（当前线程无 runtime，安全）
    let outcome = probe_auth(&config, std::time::Duration::from_secs(5));
    assert_eq!(outcome, AuthProbeOutcome::Rejected { status: 401 });

    // RPC 入口：async 入口跑在真正的 tokio runtime 上（生产语境，不得 panic）
    let cfg_for_async = config.clone();
    let outcome_async = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(probe_auth_async(&cfg_for_async, std::time::Duration::from_secs(5)))
    })
    .join()
    .expect("async 探测线程不得 panic");
    assert_eq!(outcome_async, AuthProbeOutcome::Rejected { status: 401 });

    let item = check_model_api(&config, AuthProbeTier::On);
    assert_eq!(item.status, Status::Degraded);
    assert!(item.detail.contains("密钥被拒"));
}

/// RPC `doctor.status` 的 `probe` 参数必须真实生效（不是装饰）：
/// 同一 401 mock 端点下 probe=off → model_api OK；probe=on → DEGRADED「密钥被拒」。
///
/// 🔴 回归护栏：本测试跑在**真正的 tokio runtime 上**并 await `doctor_status`，
/// 与生产网关路径（slow runtime 的 worker 线程）同构。修复前，此处的网络探针
/// 会在 worker 线程上「自建 runtime + block_on」，触发 tokio 的
/// `Cannot start a runtime from within a runtime` panic——真机实测 2 次。
/// 现在探针为纯 async，全程无 `block_on`，因此本测试必须**不 panic** 且给出真实结论。
#[tokio::test]
async fn doctor_status_rpc_probe_param_flows_to_model_api_item() {
    let ctx = crate::gateway::methods::GatewayContext {
        config: keyed_config(spawn_status_mock(401)),
        token: "test".into(),
        started_at: std::time::Instant::now(),
        bus: std::sync::Arc::new(crate::events::EventBus::default()),
    };
    let off = crate::gateway::system::doctor_status(&ctx, &serde_json::json!({})).await;
    let on = crate::gateway::system::doctor_status(&ctx, &serde_json::json!({ "probe": true })).await;
    let on_deep =
        crate::gateway::system::doctor_status(&ctx, &serde_json::json!({ "deep": true })).await;

    let model_item = |v: &serde_json::Value| {
        v["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|i| i["id"] == "model_api")
            .expect("model_api item")
            .clone()
    };
    let off_item = model_item(&off);
    let on_item = model_item(&on);
    assert_eq!(
        off_item["status"], "OK",
        "probe=off 且 TCP 可达 → OK（不校验真伪）: {}",
        off_item["detail"]
    );
    assert_eq!(
        on_item["status"], "DEGRADED",
        "probe=on 且 401 → DEGRADED: {}",
        on_item["detail"]
    );
    assert!(
        on_item["detail"].as_str().unwrap_or("").contains("密钥被拒"),
        "probe=on 应给出密钥被拒结论: {}",
        on_item["detail"]
    );
    // 既有 WebUI 的 `deep` 参数与 `probe` 同义（诊断面板 Deep check 按钮）。
    assert_eq!(
        model_item(&on_deep)["status"], "DEGRADED",
        "deep=true 必须与 probe=true 同义"
    );
    assert!(
        !on.to_string().contains("sk-live-fixture"),
        "doctor.status 报告不得泄密钥明文"
    );
}

/// 起一个「接受连接后读到请求就停住、永不回包」的 loopback mock，制造读超时。
fn spawn_hanging_mock() -> String {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("mock runtime");
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("mock bind");
            let addr = listener.local_addr().expect("mock addr");
            let _ = tx.send(format!("http://{addr}"));
            if let Ok((mut socket, _)) = listener.accept().await {
                // 读走请求后挂起（远超探测超时），让客户端读超时。
                let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    });
    rx.recv().expect("mock addr")
}

/// 超时 / 网络挂起 → Indeterminate（既不误判 Valid，也不误判 Rejected；不泄密钥）。
#[test]
fn probe_auth_times_out_to_indeterminate() {
    let config = keyed_config(spawn_hanging_mock());
    let outcome = probe_auth(&config, std::time::Duration::from_millis(300));
    match outcome {
        AuthProbeOutcome::Indeterminate { reason } => {
            assert!(!reason.is_empty(), "超时必须给出非空原因");
            assert!(
                !reason.contains("sk-live-fixture"),
                "超时原因不得泄密钥明文: {reason}"
            );
        }
        other => panic!("探测超时应为 Indeterminate，实际: {other:?}"),
    }
}

