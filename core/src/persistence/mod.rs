//! 持久化模块：配置管理、会话持久化与任务队列

use std::cell::RefCell;
use std::path::PathBuf;

// 测试用配置根目录覆盖（单测注入后 ~/.marguerite 不落盘，防止并发测试互踩）。
//
// 采用**线程局部**存储：并行测试各跑在独立配置目录、互不串扰。
// - 生产：未设置 → `Config::config_dir()` 走默认 ~/.marguerite
// - 单测：`Config::set_config_dir_override(temp)` 注入后「该线程」复用此目录，
//   且可重复覆盖（同一线程先后跑不同用例时各自生效，避免「OnceLock 设一次」导致
//   后续用例回落到真实 ~/.marguerite 而引发的 flaky / 跨用例串扰）。
thread_local! {
    pub(crate) static OVERRIDE_CONFIG_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub mod activity_store;
pub mod config;
pub mod safe_mode;
pub mod session;
pub mod task_queue;

pub use task_queue::{Task, TaskHandler, TaskQueue, TaskStatus};

/// 打开一个 SQLite 库后统一施加的写盘三件套：`journal_mode=WAL` + `synchronous=NORMAL`
/// + `busy_timeout=5s`。**返回实际生效的 journal 模式**（不是请求的那个）。
///
/// 为什么要集中在一处：默认组合是 `journal_mode=DELETE` + `synchronous=FULL`，于是
/// **每条语句 = 一次独立事务 + 建/删 journal + 一次 fsync**。本仓已经两次因此栽过：
/// `automation/long_running.rs` 的注释记着「8279 条后全量重写实测 49.5 s，同库同数据
/// 改 WAL/NORMAL 仅 0.22 s」，用户侧的表现是「点了发送没反应」。写放大在库里看不见，
/// 只体现在界面卡住 —— 所以新库一律走这里，别各自 `Connection::open` 完就建表。
///
/// ⚠️ 必须读返回值而不是只看 execute 是否成功：Windows 上同目录的 `-wal/-shm` 被别的
/// 进程持有时，`PRAGMA journal_mode=WAL` **静默返回原模式**且不报错。拿到的模式还要
/// 能在 `system.health` 里看到（W7），否则「我们开了 WAL」就是一句口头承诺。
///
/// `busy_timeout` 不许省：`usage/mod.rs` 的注释记着 init 有 5+ 个入口并发建表，
/// 没有 timeout 时直接 `SQLITE_BUSY` ⇒ `UsageLedger::new` 失败 ⇒ 进程内所有用量记录
/// 静默变成 no-op。
///
/// `synchronous` 由调用方给：普通台账用 `"NORMAL"`；**审计库用 `"FULL"`** ——
/// WAL 已经免掉「每条语句建/删一个 journal 文件」的大头，但最后一条已提交记录
/// 不能为了省写入而冒掉电丢失的风险，那是安全属性不是性能参数。
pub fn apply_sqlite_write_pragmas(
    db: &rusqlite::Connection,
    label: &str,
    synchronous: &str,
) -> String {
    let mode: String = db
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap_or_else(|err| {
            tracing::warn!(
                db = %label,
                error = %err,
                "设置 journal_mode=WAL 失败，本次沿用默认回滚日志"
            );
            String::from("unknown")
        });
    if mode != "wal" {
        tracing::warn!(
            db = %label,
            journal_mode = %mode,
            "WAL 未生效（Windows 上常见原因：同目录 -wal/-shm 被其他进程占用）；\
             该库的写入量将显著高于预期"
        );
    }
    if let Err(err) = db.pragma_update(None, "synchronous", synchronous) {
        tracing::warn!(
            db = %label,
            synchronous,
            error = %err,
            "设置 synchronous 失败，沿用库默认值"
        );
    }
    if let Err(err) = db.busy_timeout(std::time::Duration::from_secs(5)) {
        tracing::warn!(db = %label, error = %err, "设置 busy_timeout 失败");
    }
    mode
}

/// 只读查询某个库文件当前生效的 journal 模式（供 `system.health` 上报，不改设置）。
pub fn sqlite_journal_mode(path: &std::path::Path) -> Option<String> {
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.query_row("PRAGMA journal_mode", [], |row| row.get(0)).ok()
}

/// 原子写 JSON 文本到 `path`（先写同目录 `*.tmp`，再 `rename` 覆盖）。
///
/// **为什么需要**：`std::fs::write` 是「先截断目标、再写入」——进程在写入中途
/// 被杀（崩溃/断电/任务管理器）就会留下**半截文件**。所有按 `read_to_string`
/// 与 `serde_json::from_str` 读回的台账（agents.json / 记忆库 / 缓存 / 突触图）
/// 在解析失败时的兜底都是「当空处理」，于是**半截文件 = 整份数据静默丢失**，
/// 且表面无感知（内置数据会被重新 seed，看起来「只是重置了」）。
///
/// `rename` 在同一文件系统内是**原子**的：读者要么看到旧内容、要么看到完整
/// 新内容，没有中间态。临时路径不可用（权限不足/跨设备）时退回直写——
/// 保证调用方主流程不被持久化失败阻断（与仓库各处 `let _ =` 的健壮性约定一致）。
///
/// 已按此模式修复的调用点：`agent::store`（R13）、`memory::recall`（R9）、
/// `memory::synapse::save_to`、`cache::save`。新增 JSON 落盘一律走本函数。
pub fn write_json_atomic(path: &std::path::Path, raw: &str) -> std::io::Result<()> {
    // 【2026-10-23 修复】先确保父目录存在。
    //
    // 实证：全新数据目录下 `~/.marguerite/context/` 从未被创建，而
    // `context::session::ContextLedger::persist` 用的是 `let _ = write_json_atomic(..)`
    // —— 写入以 NotFound 失败并被吞掉，于是**上下文台账永远为空**：
    // `context.status` 恒为 0 token、压缩永不触发、令牌计量页面没有上下文数据。
    // 这类失败最阴险的地方是"一切看起来都正常"（会话能聊、DB 有记录），
    // 只有台账这一层静默为零。
    //
    // 放在这里而不是让每个调用方自己 `create_dir_all`：本函数是全仓 JSON 落盘的
    // 唯一原子写入口，父目录保证属于它的职责；幂等，且失败会让下面的 write 如实报错。
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    // 临时名 = 原文件名 + `.tmp`（不用 `with_extension`：无扩展名的文件如
    // `token` 会被替换成 `token.json.tmp`，改名意图不清晰）。
    let mut tmp_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("atomic"));
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    match std::fs::write(&tmp, raw.as_bytes()) {
        Ok(()) => {
            // W7：写出口计数。挂在「成功写出」之后，失败的写不计 ——
            // 否则磁盘满/权限错也会被算进「本进程写了 N 次」，那个数就没意义了。
            crate::observability::io_report::tally_write(raw.len() as u64);
            match std::fs::rename(&tmp, path) {
                Ok(()) => Ok(()),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    // rename 失败（典型：跨设备/目标被占用）→ 退回直写，至少不丢新数据
                    std::fs::write(path, raw.as_bytes()).map(|_| ()).or(Err(e))
                }
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 把凭证类文件收紧为「仅属主可读写」（POSIX 0600）。
///
/// 这是**落盘侧**的最后一道：`.env`（明文密钥）、`gateway-token`（本地网关 bearer
/// 凭据）、`credentials.json`（加密密钥 + 明文 Grant 载荷）此前都是默认 umask
/// 落盘（多用户机器上同组/其他用户可读），只有 `vault.json` 做了 0600。
///
/// Windows 上是有意为之的 no-op：`std::fs::set_permissions` 只有只读位，无法表达
/// 「仅属主」，属主隔离由用户目录继承的 NTFS ACL 负责 —— 这是平台边界，不是遗漏。
///
/// 失败按 best-effort 处理（调用方 `let _ =`）：权限收紧失败不应阻断主流程，
/// 但也不能静默假装成功 —— 返回 `Result` 让需要判定的地方（测试/诊断）能看见。
pub fn harden_private_file(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_json_atomic_creates_missing_parent_dirs() {
        // 回归：`~/.marguerite/context/` 在全新数据目录下不存在，而台账用
        // `let _ = write_json_atomic(..)` 吞错 → 上下文台账永远为空（2026-10-23 实测）。
        // 本用例锁死"父目录缺失时也必须写成功"。
        let dir = std::env::temp_dir().join(format!(
            "marg_atomic_parent_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let target = dir.join("context").join("session-1.json");
        assert!(!dir.exists(), "前置：父目录必须确实不存在");

        write_json_atomic(&target, r#"{"entries":[]}"#).expect("父目录缺失时应自动创建并写入");
        assert_eq!(
            std::fs::read_to_string(&target).expect("读回"),
            r#"{"entries":[]}"#,
            "内容应逐字节一致"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_json_atomic_leaves_no_tmp_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!(
            "marg_atomic_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join("agents.json");
        let payload = r#"{"agents":[{"id":"main","name":"严工"}]}"#;

        write_json_atomic(&target, payload).expect("原子写应成功");
        // 1) 无临时文件残留
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不得残留临时文件：{leftovers:?}");
        // 2) 目标是完整可解析内容（不是半截）
        let back = std::fs::read_to_string(&target).expect("读回");
        assert_eq!(back, payload, "原子写后内容应逐字节一致");
        // 3) 覆写路径同样无残留
        write_json_atomic(&target, r#"{"agents":[]}"#).expect("二次原子写应成功");
        let back2 = std::fs::read_to_string(&target).expect("读回 2");
        assert_eq!(back2, r#"{"agents":[]}"#);
        assert!(
            !std::path::Path::new(&dir.join("agents.json.tmp")).exists(),
            "覆写后也不得有临时文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `.env` / `gateway-token` / `credentials.json` 都走这个助手收紧权限：
    /// 这里直接钉住「收紧后同进程读回 mode == 0600」这个可观测事实。
    #[cfg(unix)]
    #[test]
    fn harden_private_file_makes_it_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "marg_harden_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let target = dir.join(".env");
        std::fs::write(&target, "MARGUERITE_API_KEY=sk-test").expect("写测试文件");
        // 前置：默认 umask 下不是 0600（否则本用例什么都没证明）
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );

        harden_private_file(&target).expect("收紧权限应成功");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600,
            "凭证文件必须只有属主可读写"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- W3（2026-10-30）：SQLite 写盘三件套 -----------------------------------------

    fn tmp_db(tag: &str) -> (std::path::PathBuf, rusqlite::Connection) {
        // 三个 WAL 用例共调本助手：只用 pid+line 会让它们拿到同一个目录，
        // 彼此的 `remove_dir_all` 会把对方刚建的库连目录一起删掉
        // （2026-10-30 门禁实测：reopen 时报 unable to open database file）。
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "marg_wal_{}_{}_{}",
            std::process::id(),
            n,
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(format!("{tag}.db"));
        let conn = rusqlite::Connection::open(&path).expect("开库");
        (path, conn)
    }

    #[test]
    fn write_pragmas_report_the_mode_that_actually_took_effect() {
        let (_p, conn) = tmp_db("mode");
        // 断言的是**读回来的值**，不是「execute 没报错」：
        // Windows 上 -wal/-shm 被占用时 PRAGMA 会静默回到原模式。
        assert_eq!(
            apply_sqlite_write_pragmas(&conn, "unit.db", "NORMAL").as_str(),
            "wal"
        );
        let effective: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(effective, "wal", "返回值必须与库里的实际模式一致");
        let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
        assert_eq!(sync, 1, "NORMAL == 1");
    }

    #[test]
    fn write_pragmas_are_idempotent_and_keep_wal_on_reopen() {
        let (p, conn) = tmp_db("reopen");
        apply_sqlite_write_pragmas(&conn, "unit.db", "NORMAL");
        conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);")
            .unwrap();
        drop(conn);
        // 重开同一个库：WAL 是写在库头里的持久属性，第二次调用不该把它降级或报错。
        let conn2 = rusqlite::Connection::open(&p).unwrap();
        assert_eq!(
            apply_sqlite_write_pragmas(&conn2, "unit.db", "NORMAL").as_str(),
            "wal"
        );
        let rows: i64 = conn2.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1, "WAL 往返后数据仍在");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn durable_callers_can_keep_synchronous_full() {
        // 审计库的口径：借 WAL 省 journal 反复建删，但不降 durability。
        let (_p, conn) = tmp_db("full");
        apply_sqlite_write_pragmas(&conn, "audit.db", "FULL");
        let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap();
        assert_eq!(sync, 2, "FULL == 2；审计记录不能为省写入而冒掉电丢失");
    }
}
