//! 工具输出的**磁盘留底**与逐字节取回。
//!
//! # 为什么需要它
//!
//! D11 的中间截断（[`super::middle_truncate`]）会在工具输出进上下文前砍掉中段，
//! 标记里承诺「完整输出已记录在本次会话的工具活动台账中」—— 但那份台账是
//! `sessions.db` 里的一行，模型**没有任何工具能读到它**。承诺写了、路没修，
//! 模型于是只能看到「中间省略 N 个字符」而毫无补救手段（AGENTS.md R54 同族：
//! 文档承诺了、代码没兑现）。
//!
//! 本模块把这句话做实：被砍掉的原文按**内容指纹**落盘（进程重启也取得回），
//! 并配一个真注册的取回工具 `tool_output_retrieve`。
//!
//! # 边界
//!
//! - 指纹当文件名 ⇒ 只允许 32 位小写十六进制；这个参数最终由**模型**给，
//!   路径遍历必须挡在 `spool_path` 一处；
//! - 单文件 / 条数 / 总字节三道上限，长任务不会把盘写满；
//! - 落盘失败 ⇒ `None`，调用方必须如实标注「留底不可用」，绝不给出取不回的信封。

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 单个留底文件的字节上限：超了**不落盘**（宁可不给取回，也不写半截冒充全文）。
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// 留底条数上限（超出按 mtime 淘汰最旧）。
pub const MAX_FILES: usize = 256;
/// 留底总字节上限。
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// 留底目录。
///
/// 单测落进程专属临时目录：`Config::data_dir()` 在生产指向用户数据目录，
/// 用例往里写文件就是 AGENTS.md R64 那类「单测写进用户家目录」的事故。
pub fn spool_dir() -> PathBuf {
    #[cfg(test)]
    {
        static TEST_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        TEST_ROOT
            .get_or_init(|| {
                let dir = std::env::temp_dir().join(format!("mgt-spool-{}", std::process::id()));
                let _ = std::fs::create_dir_all(&dir);
                dir
            })
            .clone()
    }
    #[cfg(not(test))]
    {
        crate::persistence::config::Config::data_dir().join("tool_spool")
    }
}

/// 原文落盘，返回可取回的指纹；落不了盘返回 `None`（调用方要如实降级）。
pub fn spool(output: &str) -> Option<String> {
    let fp = fingerprint(output);
    write_into(&spool_dir(), &fp, output).then_some(fp)
}

/// 模型侧的取回入口。非法指纹 / 已被淘汰 / 读失败 ⇒ `None`
/// （绝不回一个空的 `Some`，那会被模型读成「原文就是空的」）。
pub fn retrieve(fp: &str) -> Option<String> {
    let path = spool_path(&spool_dir(), fp)?;
    let bytes = std::fs::read(path).ok()?;
    String::from_utf8(bytes).ok()
}

/// 内容指纹：sha256 前 16 字节的十六进制（32 字符，正好当文件名）。
///
/// 只认内容、不掺路径与时间 ⇒ 同样的输出重复被截只需留一份。
fn fingerprint(output: &str) -> String {
    let digest = Sha256::digest(output.as_bytes());
    let mut s = String::with_capacity(32);
    for b in &digest[..16] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 指纹 → 路径，**唯一的合法性闸口**：必须 32 位小写十六进制。
fn spool_path(dir: &Path, fp: &str) -> Option<PathBuf> {
    if fp.len() != 32
        || !fp
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    Some(dir.join(format!("{fp}.txt")))
}

/// 原子写入（tmp + rename），写完做淘汰。返回是否真的落盘成功。
fn write_into(dir: &Path, fp: &str, output: &str) -> bool {
    if output.len() > MAX_FILE_BYTES {
        return false;
    }
    let Some(path) = spool_path(dir, fp) else {
        return false;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let tmp = dir.join(format!("{fp}.tmp"));
    if std::fs::write(&tmp, output).is_err() {
        return false;
    }
    // 并发写同一份内容时 rename 可能被另一个写者抢先（Windows 上会失败）——
    // 目标已存在且大小一致就视为成功，别把一个取得回的文件标成「不可用」。
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        if !path.is_file() {
            return false;
        }
    }
    evict_in(dir);
    true
}

fn list_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("txt") {
                out.push(p);
            }
        }
    }
    out
}

fn evict_in(dir: &Path) {
    let mut files: Vec<(PathBuf, u64, SystemTime)> = list_in(dir)
        .into_iter()
        .map(|p| {
            let meta = p.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (p, size, mtime)
        })
        .collect();
    files.sort_by_key(|(_, _, t)| *t); // 最旧在前
    let mut total: u64 = files.iter().map(|(_, s, _)| *s).sum();
    let mut count = files.len();
    for (path, size, _) in files {
        if count <= MAX_FILES && total <= MAX_TOTAL_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            count -= 1;
            total = total.saturating_sub(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_roundtrips_the_exact_original_from_disk() {
        let raw = format!("{}\nend", "x".repeat(60_000));
        let fp = spool(&raw).expect("必须能留底");
        assert_eq!(fp.len(), 32);
        assert_eq!(retrieve(&fp).as_deref(), Some(raw.as_str()), "取回必须逐字节等于原文");
    }

    #[test]
    fn the_same_output_is_spooled_once() {
        let raw = format!("同内容-{}", "y".repeat(1_000));
        let a = spool(&raw).expect("留底");
        let b = spool(&raw).expect("留底");
        assert_eq!(a, b, "内容寻址：同输出同指纹，不重复占盘");
    }

    #[test]
    fn a_fingerprint_can_not_escape_the_spool_directory() {
        // 指纹最终由模型给 ⇒ 路径遍历必须挡在 spool_path 一处。
        assert!(retrieve("../..%2fetc/passwd").is_none());
        assert!(retrieve("../config").is_none());
        assert!(retrieve("").is_none());
        assert!(retrieve(&"a".repeat(65)).is_none());
        assert!(retrieve(&"A".repeat(32)).is_none(), "大写十六进制也必须拒绝");
    }

    #[test]
    fn an_oversized_output_is_not_half_written() {
        // 超单文件上限 ⇒ 明确失败，不能留下半截冒充全文。
        let raw = "z".repeat(MAX_FILE_BYTES + 1);
        assert!(spool(&raw).is_none());
    }

    #[test]
    fn spool_stays_within_its_cap_by_evicting_oldest() {
        // 用专属目录直测内部写入：共享目录里还有别的用例（含 E12）的文件，
        // 在这里触发淘汰会把它们删掉，那是**测试互相破坏**而不是被测行为。
        let dir = std::env::temp_dir().join(format!("mgt-spool-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for i in 0..(MAX_FILES + 5) {
            let body = format!("{i}-{}", "z".repeat(10_000));
            assert!(write_into(&dir, &fingerprint(&body), &body));
        }
        let n = list_in(&dir).len();
        assert!(n <= MAX_FILES, "留底数量必须封顶，否则长任务把盘写满：{n}");
        assert!(n > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
