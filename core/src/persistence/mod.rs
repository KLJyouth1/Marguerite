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
        Ok(()) => match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                // rename 失败（典型：跨设备/目标被占用）→ 退回直写，至少不丢新数据
                std::fs::write(path, raw.as_bytes()).map(|_| ()).or(Err(e))
            }
        },
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
}
