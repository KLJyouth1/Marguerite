
    use super::{AuditRecord, AuditStore, GENESIS_HASH};
    use rusqlite::Connection;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("marguerite-audit-{name}-{}.db", Uuid::new_v4()))
    }

    fn record(actor: &str) -> AuditRecord {
        AuditRecord::new("2026-08-28T12:00:00Z", actor, "tool.call", "target-1", "ok")
    }

    #[test]
    fn schema_and_full_synchronous_pragma() {
        let path = test_path("schema");
        let store = AuditStore::open(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let sync: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sync, 2);
        let columns: i64 = conn.query_row("SELECT count(*) FROM pragma_table_info('audit_log') WHERE name IN ('id','timestamp','actor','method','target','result','prev_hash','hash')", [], |row| row.get(0)).unwrap();
        assert_eq!(columns, 8);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn append_and_verify_chain() {
        let path = test_path("append");
        let store = AuditStore::open(&path).unwrap();
        let first = store.append(record("alice")).unwrap();
        let second = store.append(record("bob")).unwrap();
        assert_eq!(first.prev_hash, GENESIS_HASH);
        assert_eq!(second.prev_hash, first.hash);
        assert!(store.verify_chain().is_ok());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_rejects_update_and_delete() {
        let path = test_path("immutable");
        let store = AuditStore::open(&path).unwrap();
        store.append(record("alice")).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert!(conn
            .execute("UPDATE audit_log SET actor='mallory' WHERE id=1", [])
            .is_err());
        assert!(conn
            .execute("DELETE FROM audit_log WHERE id=1", [])
            .is_err());
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detects_hash_tamper_and_chain_break() {
        let path = test_path("tamper");
        let store = AuditStore::open(&path).unwrap();
        store.append(record("alice")).unwrap();
        store.append(record("bob")).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TRIGGER audit_log_no_update; DROP TRIGGER audit_log_no_delete;")
            .unwrap();
        conn.execute("UPDATE audit_log SET hash='bad' WHERE id=1", [])
            .unwrap();
        let reopened = AuditStore::open(&path).unwrap();
        assert!(reopened.verify_chain().is_err());
        conn.execute_batch("DROP TRIGGER audit_log_no_update; DROP TRIGGER audit_log_no_delete;")
            .unwrap();
        conn.execute("UPDATE audit_log SET prev_hash='broken' WHERE id=1", [])
            .unwrap();
        assert!(reopened.verify_chain().is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persists_after_close_and_reopen() {
        let path = test_path("persist");
        {
            let store = AuditStore::open(&path).unwrap();
            store.append(record("alice")).unwrap();
        }
        let reopened = AuditStore::open(&path).unwrap();
        assert!(reopened.verify_chain().is_ok());
        assert_eq!(reopened.len().unwrap(), 1);
        let _ = std::fs::remove_file(path);
    }
