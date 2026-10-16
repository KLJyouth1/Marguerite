use super::*;

/// 事件推送：订阅者在 dispatch 触发时收到事件，且帧映射/元数据齐全。
#[tokio::test]
async fn subscriber_receives_dispatch_triggered_event() {
    let bus = Arc::new(EventBus::default());
    let ctx = ctx_with_bus(bus.clone());
    let loopback = ConfirmLoopback::new("artifacts.list", bus.clone());
    let captured = capture_events(&bus);

    let _ = loopback.dispatch(&ctx, "artifacts.list", &json!({}));
    let events = wait_for(&captured, 1).await;
    assert!(!events.is_empty(), "订阅者应收到 dispatch 触发的事件");
    let env = &events[0];
    // 路由键与元数据契约：{session}.{type}，meta 携带 session/ts/trace_id
    assert_eq!(env.route_key(), "loopback.approval");
    assert_eq!(env.session, "loopback");
    assert!(
        !env.trace_id.is_empty(),
        "trace_id 必须存在（前端归因依赖）"
    );
    let frame = envelope_to_frame(env, 7);
    assert_eq!(frame["seq"], 7);
    assert_eq!(frame["meta"]["session"], "loopback");
    assert!(frame["meta"]["ts"].is_string());
}
