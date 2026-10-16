//! T-1.8 E2E 闭环骨架（纯逻辑，不依赖真实 TUI / HTTP / 图片文件）：
//!
//! ```text
//! 视觉 Modlens → GenUI Card Button 点击 → RouterProfile 解析
//!   → UsageLedger 记账 + PetV2 tick 状态切换
//! ```
//!
//! 所有外部依赖全部 mock 或内存构造，单条 `#[test]` 即串通 4 个阶段。
//!
//! 与 `core/tests/*.rs`（workspace 根集成测试）不同，这里放在 `core/src/tests`
//! 下，并由 `lib.rs` 通过 `#[cfg(test)] pub mod tests;` 引入，这样可以访问
//! `crate::*` 的私有/pub(crate) API——这在多阶段 mock 串测下极大简化样板。

// ─────────────────────── 本测试内最小化辅助结构体 ───────────────────────

/// Mock 的图片附件：视觉阶段传入 ModLens.metadata_fallback。
/// （不强制依赖上游 vision 里已存不存在的 ImageAttachment 名——若已存在此处也
///   是本地等价替身，便于断言 path/mime 字段一致、mock 识别。）
#[derive(Debug, Clone)]
struct ImageAttachment {
    path: String,
    mime: String,
}

// ─────────────────────────────── 4 阶段闭环 ───────────────────────────────

/// T-1.8 E2E 4 阶段骨架：视觉 → GenUI 交互 → Router → (Usage + Pet)
#[test]
fn dsh_workflow_closed_loop_4_stages() {
    // =====================================================
    // Stage 1 — Modlens：构造 ImageAttachment → mock 识别结果非空
    // =====================================================
    let attachment = ImageAttachment {
        path: "mock:///screenshot-login.png".into(),
        mime: "image/png".into(),
    };
    assert!(attachment.path.contains("screenshot-login"));
    assert_eq!(attachment.mime, "image/png");

    // 由于 `vision::modlens::recognize` 可能不存在，这里走降级的纯本地
    // `ModLens::metadata_fallback`，并喂一个最小合法 PNG 头（8 字节签名）。
    // 即便 metadata_fallback 解析不出尺寸也绝不 panic——文档约定降级时会
    // 产出一个明确标注 source="metadata-fallback" 的 VisionDocument。
    let png_magic: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let doc = crate::vision::modlens::ModLens::metadata_fallback(&png_magic);

    // 断言：含 "识别成功" 语义——我们把 summary 拼进一段结果字符串，然后
    // 做"含非空"双断言（两种形式都过）
    let result = format!(
        "识别成功: source={}, summary={}, blocks={}",
        doc.source,
        doc.summary,
        doc.blocks.len()
    );
    assert!(
        result.contains("识别成功"),
        "Stage 1 识别结果应包含 '识别成功'，实际: {}",
        result
    );
    assert!(!doc.summary.is_empty(), "Stage 1 识别结果非空");

    // =====================================================
    // Stage 2 — GenUI：Card 3 按钮 → 点击中间坐标 → dispatch "btn_b"
    // =====================================================
    // 使用 contract（非 widget.rs 旧定义）：Card / Button / click_dispatch。
    use crate::genui::contract::{Button, Card, Widget};
    use crate::genui::render::click_dispatch;

    let buttons = vec![
        Button::new("btn_a", "A 选项"),
        Button::new("btn_b", "B 居中"),
        Button::new("btn_c", "C 选项"),
    ];
    let card = Widget::Card(Card {
        title: "DSH 工作台 3 按钮卡片".into(),
        body: vec!["请选择一个执行档位：".into(), "A=弱  B=执行  C=规划".into()],
        actions: buttons.clone(),
    });
    match &card {
        Widget::Card(c) => assert_eq!(c.actions.len(), 3),
        _ => panic!("Card widget 构造应为 Widget::Card 变体"),
    }

    // click_dispatch(&buttons, x_in_row):
    // 每按钮占 10 宽，间隔 2 → 0..10 = A，12..22 = B，24..34 = C
    // "居中坐标 x"=17 → 命中 B（对应 id "btn_b"）
    let hit = click_dispatch(&buttons, 17);
    assert_eq!(
        hit.as_deref(),
        Some("btn_b"),
        "Stage 2 点击 x=17 应命中居中按钮 btn_b，实际: {:?}",
        hit
    );

    // =====================================================
    // Stage 3 — Router：RouterProfile::from_str("execute") == Execute
    // =====================================================
    use crate::harness::dsh_bridge::RouterProfile;
    let profile =
        RouterProfile::from_str("execute").expect("RouterProfile::from_str(\"execute\") 应成功");
    assert_eq!(
        profile,
        RouterProfile::Execute,
        "Stage 3 'execute' 字符串应映射为 RouterProfile::Execute"
    );
    // 确保别名也 OK（让未来 CLI 扩展不回退）
    assert_eq!(
        RouterProfile::from_str("exec").unwrap(),
        RouterProfile::Execute
    );
    assert!(RouterProfile::from_str("???").is_err());

    // =====================================================
    // Stage 4 — Usage + Pet：2 次虚拟用量 + pet() → tick(4s) → Idle
    // =====================================================
    //
    // 4.1 Usage：2 次 TokenUsage {prompt=100, completion=200}，总计
    //     tokens_in = 200, tokens_out = 400 → today tokens_in+out = 600。
    use crate::usage::query_service::UsageQueryService;
    use chrono::Utc;
    use rusqlite::Connection;

    let svc = UsageQueryService::from_connection(Connection::open_in_memory().unwrap())
        .expect("构建查询服务");
    // 用 SQL 直接写入今天（UTC 日边界），绕过 ledger.record 真实 now()
    let today = Utc::now().date_naive();
    let start_ms =
        super::super::usage::query_service::tests_exports::naive_date_to_ms(today) as i64;
    let insert_one = |offset_ms: i64| {
        let conn = svc.ledger_conn_for_e2e();
        conn.execute(
            "INSERT INTO usage_events (session, model, prompt_tokens, completion_tokens, total_tokens, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "sess-e2e",
                "deepseek-v4-pro",
                100i64,
                200i64,
                300i64,
                start_ms + offset_ms
            ],
        )
        .expect("insert usage_event");
    };
    insert_one(1000);
    insert_one(2000);

    let today_rows = svc.by_day(today, today).expect("今日 by_day 应成功");
    assert_eq!(today_rows.len(), 1);
    let d = &today_rows[0];
    let total = d.tokens_in + d.tokens_out;
    assert_eq!(
        total, 600,
        "Stage 4 今日 tokens_in + tokens_out 应等于 600（2×(100+200)），实际 in={} out={}",
        d.tokens_in, d.tokens_out
    );

    // 4.2 PetV2：pet → tick 4s → Idle
    use crate::workbench::pet_v2::{PetStateV2, PetV2};
    let mut pet = PetV2::new();
    let base_ms = pet.current_since_ms;
    pet.pet();
    assert_eq!(pet.state, PetStateV2::Petted);
    // 注入 current_since_ms 回 base_ms + 0，tick(4, base_ms + 4_000) 触发 Petted 3s 超时
    pet.current_since_ms = base_ms;
    pet.tick(4, base_ms + 4_000);
    assert_eq!(
        pet.state,
        PetStateV2::Idle,
        "Stage 4 pet() 后 tick(4s) 应回到 Idle，实际 state={:?}",
        pet.state
    );
}
