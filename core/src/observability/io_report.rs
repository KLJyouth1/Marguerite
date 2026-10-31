//! 写入量自观测（W7）：把「这一版少写了多少盘」变成界面可见的数字。
//!
//! 为什么必须做这一层：W1–W5 全是一类改动 —— 少写、合并写、有界写。这类收益
//! 有个共同的死法：下一次重构有人顺手把 `to_string_pretty` 加回去、有人把
//! `PRAGMA` 撤了、有人把预算阈值调大十倍，**没有任何测试会因此变红**，
//! 因为功能照样对。于是 SSD 磨损回来，而用户只能信我们的口头承诺。
//! 本模块把「现在到底写了多少 / 库到底开没开 WAL / 目录占了多大」摊成 JSON，
//! 交给 `system.health` 的 `io` 段与「引擎健康」面板显示，同时给回归留一个可对账的数。
//!
//! 诚实约定（R57）：所有读数区分三态 —— **读不到**（`null`）、
//! **不存在**（`present: false`）、**存在且为 0**。
//! 前者不能渲染成后者，否则「journal 模式未知」会被看成「没开 WAL」，
//! 「目录还没建」会被看成「0 字节、很健康」。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use crate::persistence::config::Config;

/// 本进程成功落盘的次数与字节（挂在写出口上，见 [`tally_write`]）。
static JSON_WRITES: AtomicU64 = AtomicU64::new(0);
static JSON_WRITE_BYTES: AtomicU64 = AtomicU64::new(0);

/// 记一次**成功**的落盘。
///
/// 只挂在少数几条真正的写出口（`write_json_atomic`、工具输出 spool、路由决策日志）
/// 上，而不是挂在每次调用点：失败的写不计，否则「写了几次」会把磁盘满、权限错
/// 一起算进来，那个数字就没有意义了。
///
/// ⚠️ 计的是**本进程**、且不含 SQLite 事务（库里的 insert/update 由 WAL 自己合并，
/// 量级由 `databases.*.wal_bytes` 体现）。上报时把这个口径一起给出去，
/// 别让面板把它说成「全部写入次数」。
pub fn tally_write(bytes: u64) {
    JSON_WRITES.fetch_add(1, Ordering::Relaxed);
    JSON_WRITE_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

/// `(次数, 字节)`。
pub fn write_tally() -> (u64, u64) {
    (
        JSON_WRITES.load(Ordering::Relaxed),
        JSON_WRITE_BYTES.load(Ordering::Relaxed),
    )
}

/// 单文件字节数；不存在或不可读 ⇒ `None`（区别于「存在且为 0 字节」）。
fn file_bytes(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|md| md.len())
}

/// 平铺目录的 `(文件数, 总字节)`；目录缺失 ⇒ `None`。
///
/// 不递归：本模块关心的 `context/`、`tool_spool/` 都是平铺的一层目录，
/// 递归会把「一次 health 调用」变成一次全盘扫描。
fn dir_usage(dir: &Path) -> Option<(usize, u64)> {
    let read = std::fs::read_dir(dir).ok()?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in read.flatten() {
        if let Ok(md) = entry.path().metadata() {
            if md.is_file() {
                files += 1;
                bytes += md.len();
            }
        }
    }
    Some((files, bytes))
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 一个普通文件条目：`present` 与 `bytes` 各自如实。
fn file_entry(path: &Path) -> Value {
    match file_bytes(path) {
        Some(bytes) => json!({ "present": true, "bytes": bytes, "path": path_str(path) }),
        None => json!({ "present": false, "bytes": Value::Null, "path": path_str(path) }),
    }
}

/// 同一个库的附属文件（WAL 模式下 `-wal` 才是「还没合并下去的写入量」）。
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// 一个 SQLite 库的条目。
///
/// `journal_mode` 走 `persistence::sqlite_journal_mode` 的**只读回读**：它报的是
/// SQLite 当下实际生效的模式，不是我们请求的那个。W3 的教训正是
/// `PRAGMA journal_mode=WAL` 在 Windows 上会被 `-wal/-shm` 占用而**静默失败**，
/// 所以这里必须读、不能猜。
fn db_entry(path: &Path) -> Value {
    json!({
        "present": path.exists(),
        "bytes": file_bytes(path),
        "journal_mode": crate::persistence::sqlite_journal_mode(path),
        "wal_bytes": file_bytes(&sidecar(path, "-wal")),
        "shm_present": sidecar(path, "-shm").exists(),
    })
}

/// 决策日志：当前代 + 保留代逐条列出，合计才是「这份日志占了多大」。
fn decision_log_entry(dir: &Path) -> Value {
    let base = dir.join(crate::router::decision_log::LOG_FILE_NAME);
    let keep = crate::observability::rotation::KEEP_GENERATIONS;
    let mut generations = Vec::new();
    let mut total = 0u64;
    let mut any = false;
    for gen in 0..keep {
        let path = if gen == 0 {
            base.clone()
        } else {
            let mut name = base
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(format!(".{gen}"));
            base.with_file_name(name)
        };
        match file_bytes(&path) {
            Some(bytes) => {
                any = true;
                total += bytes;
                generations.push(json!({ "generation": gen, "bytes": bytes }));
            }
            None => generations.push(json!({ "generation": gen, "bytes": Value::Null })),
        }
    }
    json!({
        "present": any,
        "bytes": if any { Value::from(total) } else { Value::Null },
        "path": path_str(&base),
        "rotate_at_bytes": crate::router::decision_log::ROTATE_AT_BYTES,
        "keep_generations": keep,
        "generations": generations,
    })
}

/// 侧车 stdout/stderr 日志目录。
///
/// 为什么这条必须出现在报告里：这一份是**子进程**写的，父进程既不经手也计不到
/// （`process_writes` 明确只算本进程的 JSON/追加出口）。侧车日志的有界化只发生在
/// spawn 时刻（[`crate::observability::rotation::open_rotating`]），长跑期间单个文件
/// 不再切 —— 想改成「写时轮转」也不行：句柄在子进程手里，父进程 rename 在 Windows
/// 上是 sharing violation。所以这里把它**摊成数字**：真涨起来了，面板上看得见，
/// 而不是等下次有人问「%TEMP% 怎么多了 2 GB」。
fn sidecar_log_entry(dir: &Path) -> Value {
    let usage = dir_usage(dir);
    json!({
        "present": usage.is_some(),
        "files": usage.map(|(f, _)| f).unwrap_or(0),
        "bytes": usage.map(|(_, b)| Value::from(b)).unwrap_or(Value::Null),
        "budget_bytes": crate::observability::rotation::MAX_DIR_BYTES,
        "path": path_str(dir),
    })
}

/// `system.health` 的 `io` 段（真实路径）。
///
/// 成本：三次只读 SQLite open（只为回读 `journal_mode`）+ 三个平铺目录的
/// `read_dir`。所以它挂在 `health`（手动刷新/健康面板）而不是 WS 心跳上 ——
/// 别把 health 放进秒级轮询，那本身就变成新的写入观测噪声源。
pub fn report() -> Value {
    report_at(
        &Config::data_dir(),
        &crate::pyengine::sidecar::sidecar_log_dir(),
    )
}

/// [`report`] 的「目录由调用方给」版本。
///
/// 单测必须走这个口子：直接调 `report()` 会把读数指向**真实用户的 `~/.marguerite`**
/// （本仓探针纪律明令禁止），而且断言会随别人机器上装了什么而变红 —— 那就不是回归网，
/// 是环境探测器。台账与 spool 两个目录本来就在 `#[cfg(test)]` 里指向临时目录，
/// 所以这里只需要把「数据目录 + 侧车日志目录」两路注入。
fn report_at(data: &Path, sidecar_dir: &Path) -> Value {
    let (writes, write_bytes) = write_tally();
    let context_dir = crate::context::session::context_dir();
    let spool_dir = crate::context::spool::spool_dir();
    let context = dir_usage(&context_dir);
    let spool = dir_usage(&spool_dir);

    json!({
        // 面板显示必须带上这个口径说明。⚠️ 这里给的是**机器键**不是文案：
        // 后端拼中文字符串进界面，是本项目 i18n 纪律明令禁止的（6 个 locale 里
        // 搜不到那句话），翻译权在前端（见 `webui/src/utils/engine/ioHealth.ts`）。
        // 计的是「哪几条出口」而不是全量 IO，口径必须随数字下发，否则面板会把它
        // 说成「全部写入次数」。
        "process_writes": {
            "count": writes,
            "bytes": write_bytes,
            "scope": "json_append_exits",
        },
        "databases": {
            "sessions": db_entry(&data.join("sessions.db")),
            "usage": db_entry(&data.join("usage.db")),
            "audit": db_entry(&data.join("audit.db")),
        },
        "context_ledger": {
            "present": context.is_some(),
            "files": context.map(|(f, _)| f).unwrap_or(0),
            "bytes": context.map(|(_, b)| Value::from(b)).unwrap_or(Value::Null),
            "budget_bytes": crate::context::session::CONTEXT_DIR_BUDGET_BYTES,
            "partial_flush_interval_ms": crate::context::session::PARTIAL_FLUSH_INTERVAL_MS,
        },
        "plans": file_entry(&data.join("plans.json")),
        "decision_log": decision_log_entry(data),
        "tool_spool": {
            "present": spool.is_some(),
            "files": spool.map(|(f, _)| f).unwrap_or(0),
            "bytes": spool.map(|(_, b)| Value::from(b)).unwrap_or(Value::Null),
            "budget_bytes": crate::context::spool::MAX_TOTAL_BYTES,
            "max_files": crate::context::spool::MAX_FILES,
        },
        "sidecar_logs": sidecar_log_entry(sidecar_dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 计数只加在成功写之后，且口径是「次数 + 字节」两个数都要有。
    #[test]
    fn tally_counts_calls_and_bytes() {
        let (before_count, before_bytes) = write_tally();
        tally_write(128);
        tally_write(256);
        let (after_count, after_bytes) = write_tally();
        assert_eq!(after_count - before_count, 2, "两次调用应计两次");
        assert_eq!(after_bytes - before_bytes, 384, "字节应按实累加");
    }

    /// 写出口真的在计数：`write_json_atomic` 成功一次 ⇒ 计数 +1；失败不计数。
    #[test]
    fn atomic_json_write_is_counted_and_failures_are_not() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_tally_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.json");

        let (c0, b0) = write_tally();
        crate::persistence::write_json_atomic(&path, "{\"ok\":true}").unwrap();
        let (c1, b1) = write_tally();
        assert_eq!(c1 - c0, 1, "成功落盘必须计数");
        assert_eq!(b1 - b0, 11, "计数的是实际写出字节");

        // 失败（父目录在一个已存在的普通文件下面）不得计数，否则「写了 N 次」失真。
        let not_a_dir = dir.join("a.json");
        let before = write_tally();
        assert!(crate::persistence::write_json_atomic(&not_a_dir.join("x").join("y.json"), "{}").is_err());
        assert_eq!(write_tally(), before, "失败的写不得计入");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 三态区分（R57）：不存在 ≠ 0 字节 ≠ 读不到。
    #[test]
    fn missing_file_is_reported_as_absent_not_zero() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_absent_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ghost = dir.join("nope.db");

        assert_eq!(file_bytes(&ghost), None, "不存在必须是 None，不是 Some(0)");
        let entry = db_entry(&ghost);
        assert_eq!(entry["present"], false, "不存在应如实标 present=false");
        assert!(
            entry["journal_mode"].is_null(),
            "读不到模式应报 null（未知），不能报成 delete 让人以为 WAL 没开"
        );

        // 存在且为 0 字节 ⇒ present=true、bytes=0，和上面两种情况都不同。
        std::fs::write(&ghost, b"").unwrap();
        let entry = db_entry(&ghost);
        assert_eq!(entry["present"], true);
        assert_eq!(
            entry["bytes"], 0,
            "空文件是「0 字节」这个事实，不是「没有这个文件」"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 目录读数：只统计平铺一层，且目录缺失 ⇒ present=false。
    #[test]
    fn dir_usage_counts_only_flat_files() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_dir_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("a.json"), "xxxx").unwrap();
        std::fs::write(dir.join("b.json"), "yy").unwrap();
        std::fs::write(dir.join("nested").join("c.json"), "zzzzzzzz").unwrap();

        let (files, bytes) = dir_usage(&dir).unwrap();
        assert_eq!(files, 2, "只数平铺文件，子目录不算");
        assert_eq!(bytes, 6, "子目录里的字节不计入：本模块不递归");

        assert!(dir_usage(&dir.join("does-not-exist")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 决策日志代计数：`.1` 存在时合计要把它算进去。
    #[test]
    fn decision_log_totals_all_generations() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_dlog_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(crate::router::decision_log::LOG_FILE_NAME), "0123456789").unwrap();
        std::fs::write(
            dir.join(format!("{}.1", crate::router::decision_log::LOG_FILE_NAME)),
            "01234",
        )
        .unwrap();

        let entry = decision_log_entry(&dir);
        assert_eq!(entry["present"], true);
        assert_eq!(entry["bytes"], 15, "当前代 + 历史代必须合计");
        assert_eq!(entry["generations"][0]["bytes"], 10);
        assert_eq!(entry["generations"][1]["bytes"], 5);
        assert!(
            entry["generations"][2]["bytes"].is_null(),
            "不存在的代要如实为 null，而不是补一个 0 假装它在"
        );
        assert_eq!(
            entry["rotate_at_bytes"],
            crate::router::decision_log::ROTATE_AT_BYTES,
            "阈值必须随报告下发，否则面板无从判断「多大算超」"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// WAL 附属文件：`-wal` 的大小就是「还没合并下去的写入量」，必须单独上报。
    #[test]
    fn wal_sidecar_size_is_reported() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_wal_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("x.db");
        std::fs::write(&db, "head").unwrap();
        assert_eq!(db_entry(&db)["wal_bytes"], Value::Null, "没有 -wal 时不得报 0");
        std::fs::write(dir.join("x.db-wal"), "walswal").unwrap();
        assert_eq!(db_entry(&db)["wal_bytes"], 7, "-wal 字节必须单独可见");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 侧车日志读数：目录缺失 ⇒ `present:false`（还没起过侧车），存在 ⇒ 只数平铺的
    /// 日志文件。这份是**子进程**写的，父进程计数够不到，只能靠目录读数补上。
    #[test]
    fn sidecar_log_entry_distinguishes_missing_dir_from_empty_dir() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_sidelog_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let missing = sidecar_log_entry(&dir);
        assert_eq!(missing["present"], false, "没起过侧车 ⇒ 目录不存在，不是 0 字节");
        assert!(
            missing["bytes"].is_null(),
            "目录不存在时字节必须为 null，补 0 就是把「未知」说成「很健康」"
        );

        std::fs::create_dir_all(&dir).unwrap();
        let empty = sidecar_log_entry(&dir);
        assert_eq!(empty["present"], true);
        assert_eq!(empty["bytes"], 0, "空目录是「0 字节」这个事实");

        std::fs::write(dir.join("sidecar-1.err.log"), "panic").unwrap();
        let one = sidecar_log_entry(&dir);
        assert_eq!(one["files"], 1);
        assert_eq!(one["bytes"], 5);
        assert_eq!(
            one["budget_bytes"],
            crate::observability::rotation::MAX_DIR_BYTES,
            "预算必须随读数下发，否则「200 MB」在面板上只是一个孤立的数字"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 端到端：一个**全新的空目录**也必须出得来、键齐全，且各读数如实标 `present:false`。
    /// 走 `report_at` 而不是 `report()`：后者会去读真实用户的 `~/.marguerite`，
    /// 断言会随这台机器装了什么而变 —— 那是环境探测，不是回归。
    #[test]
    fn report_has_stable_shape_on_a_fresh_dir() {
        let dir = std::env::temp_dir().join(format!(
            "mgt_io_shape_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let v = report_at(&dir, &dir);
        for key in [
            "process_writes",
            "databases",
            "context_ledger",
            "plans",
            "decision_log",
            "tool_spool",
            "sidecar_logs",
        ] {
            assert!(v.get(key).is_some(), "io 段缺键 {key}：{v}");
        }
        assert!(v["process_writes"]["count"].is_number(), "写入计数必须是数字");
        assert_eq!(
            v["process_writes"]["scope"].as_str(),
            Some("json_append_exits"),
            "口径必须下发**机器键**而不是中文文案 —— 后端拼句子进界面违反 i18n 纪律，\
             而且它一旦不是键，前端就没有地方把它说清（哪几条出口、SQLite 不计）"
        );
        // 空目录 = 「这些文件不存在」，不是「都是 0 字节、很健康」。
        assert_eq!(v["databases"]["sessions"]["present"], false);
        assert!(v["databases"]["sessions"]["journal_mode"].is_null());
        assert_eq!(v["plans"]["present"], false);
        assert_eq!(v["decision_log"]["present"], false);
        assert_eq!(v["sidecar_logs"]["present"], true, "传进去的日志目录是刚建的");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
