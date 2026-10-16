
    use super::*;
    use crate::rbac::roles::Role;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    fn seam() -> SecuritySeam {
        let path =
            std::env::temp_dir().join(format!("marguerite-seam-{}.db", uuid::Uuid::new_v4()));
        SecuritySeam::new(
            Policy,
            ApprovalGate::new(Duration::from_secs(60)),
            AuditStore::open(path).unwrap(),
            EventBus::default(),
        )
    }

    #[test]
    fn guest_admin_method_denied_and_audited() {
        let seam = seam();
        let err = seam
            .request(
                Actor::new("guest-1", Role::Guest),
                "ops.file.write",
                "doc-1",
                json!({"secret":"do-not-log"}),
                || Ok::<_, String>(json!("ran")),
            )
            .unwrap_err();
        assert_eq!(err.code(), RBAC_FORBIDDEN);
        assert_eq!(seam.audit_store().len().unwrap(), 1);
        seam.audit_store().verify_chain().unwrap();
    }

    #[test]
    fn developer_read_allowed() {
        let seam = seam();
        let value = seam
            .request(
                Actor::new("dev-1", Role::Developer),
                "status",
                "system",
                json!({}),
                || Ok::<_, String>(json!({"ok":true})),
            )
            .unwrap();
        assert_eq!(value["ok"], true);
    }

    #[tokio::test]
    async fn admin_dangerous_request_emits_event_and_does_not_call_callback() {
        let seam = seam();
        let mut events = seam.event_bus().subscribe("**");
        let called = std::sync::Arc::new(AtomicBool::new(false));
        let marker = called.clone();
        let err = seam
            .request(
                Actor::new("admin-1", Role::Admin),
                "ops.file.write",
                "doc-1",
                json!({}),
                move || {
                    marker.store(true, Ordering::SeqCst);
                    Ok::<_, String>(json!("ran"))
                },
            )
            .unwrap_err();
        assert_eq!(err.code(), OPS_CONFIRM_REQUIRED);
        assert!(!called.load(Ordering::SeqCst));
        let event = events.recv().await.unwrap();
        assert_eq!(event.payload["event"], "ops.confirm.required");
    }

    #[test]
    fn approve_then_callback_runs_and_audits() {
        let seam = seam();
        let actor = Actor::new("admin-1", Role::Admin);
        let err = seam
            .request(actor.clone(), "ops.file.write", "doc-1", json!({}), || {
                Ok::<_, String>(json!("deferred"))
            })
            .unwrap_err();
        let approval_id = err.approval_id().unwrap().to_string();
        let value = seam
            .approve_and_execute(
                &approval_id,
                actor,
                "ops.file.write",
                "doc-1",
                json!({}),
                || Ok::<_, String>(json!({"done":true})),
            )
            .unwrap();
        assert_eq!(value["done"], true);
        assert_eq!(seam.audit_store().len().unwrap(), 2);
        seam.audit_store().verify_chain().unwrap();
    }

    #[test]
    fn wrong_approval_denied() {
        let seam = seam();
        let actor = Actor::new("admin-1", Role::Admin);
        seam.request(actor.clone(), "ops.file.write", "doc-1", json!({}), || {
            Ok::<_, String>(json!("deferred"))
        })
        .unwrap_err();
        let err = seam
            .approve_and_execute(
                "ap_wrong",
                actor,
                "ops.file.write",
                "doc-1",
                json!({}),
                || Ok::<_, String>(json!("ran")),
            )
            .unwrap_err();
        assert_eq!(err.code(), OPS_CONFIRM_DENIED);
    }
