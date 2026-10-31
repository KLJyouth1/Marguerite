//! 日志轮转：按大小轮转 + 目录总量封顶（S6）。
//!
//! # 要解决的缺陷
//!
//! 侧车日志此前是裸 [`std::fs::File`]：既不轮转也不设上限，落在
//! `%TEMP%\marguerite-sidecar\`。长跑 + 反复崩溃 = 该目录无限增长；
//! 又因为它在 `git ls-files` 视野之外，`check-secrets.py` 扫不到，
//! 于是「无上限」与「无人审计」叠加。
//!
//! # 为什么自己写而不用 `tracing-appender`
//!
//! `tracing-appender` 的 `RollingFileAppender` 按**时间**轮转（日/小时），
//! 没有「单文件超 10MB 就切」这一档，也没有目录总量上限——而 S6 要的是后者。
//! 本模块只做「打开一个日志文件」这一件事，约60 行、可单测，
//! 不必为此把整个 CLI 的日志初始化改写成 appender 句柄。
//!
//! # 时序与S5 证据采集的关系
//!
//! S5（退出证据）要求：**重启之前**取走stderr 尾部，因为重启会重开日志文件。
//! 本模块不改变这个契约——轮转只在 [`open_rotating`] 被调用（即 spawn 时刻）
//! 发生，且**只切当前这一个文件**，上一代内容保留在 `.1` 里。
//! 换言之S5 的时序约束依然成立（证据在 spawn 前采集），
//! 而崩溃现场不再像以前那样被`File::create` 直接抹掉。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 单个日志文件的大小上限（10MB）。
///
/// 与 S6 规格一致。超出即在下次打开时切一代。
pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// 日志目录总量上限（200MB）。
///
/// 封顶目录而非只封单文件：真实机器上同目录会有多个 `sidecar-{port}.*`
/// （换端口复用、多实例并存），只封单文件的话总量仍可无界增长。
pub const MAX_DIR_BYTES: u64 = 200 * 1024 * 1024;

/// 保留的历史代数上限（含当前文件，故实际保留 `KEEP_GENERATIONS - 1` 份历史）。
pub const KEEP_GENERATIONS: usize = 3;

/// 日志文件名的可识别前缀（用于目录封顶时筛选属于本模块的文件）。
const LOG_NAME_PREFIX: &str = "sidecar-";

/// 打开一个「按大小轮转 + 目录封顶」的日志文件，返回追加写的句柄。
///
/// 与 [`File::create`] 的关键差异：
///
/// - **追加**而非截断：崩溃现场留在 `.1`，重启不销毁上一代现场
///   （S5 的证据采集时序因此更宽松，但代码仍保持「先取证再重启」）。
/// - 超限时先轮转（`x.log` → `x.log.1` → `x.log.2` → 丢弃），
///   随后对整个目录执行总量封顶。
///
/// 轮转/封顶的失败**一律不阻断打开**：日志写不进去不该让侧车起不来
/// （那会把「排障手段不可用」变成「故障本身」）。失败时退化为普通追加。
pub fn open_rotating(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        // 目录不存在时 `rotate` / `prune` 都会自然失败（metadata 读不到），
        // 但 `create_dir_all` 必须做，否则 `OpenOptions::create` 直接报错。
        let _ = fs::create_dir_all(parent);
    }
    // 轮转与封顶都是 best-effort：任何一步失败都只是少一份历史。
    let _ = rotate_if_oversized(path);
    let file = open_append(path)?;
    if let Some(parent) = path.parent() {
        let _ = prune_dir_to_budget(parent);
    }
    Ok(file)
}

/// 以追加模式打开（文件不存在则创建）。
fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// 当前日志代数（含 `x.log` 自身）的字节数；文件不存在返回 0。
fn current_generation_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// 若当前文件已达上限则切一代。返回是否发生了轮转。
///
/// 纯函数式的路径运算便于单测；不做 IO 之外的任何决策。
pub fn rotate_if_oversized(path: &Path) -> io::Result<bool> {
    rotate_if_oversized_with(path, MAX_LOG_BYTES)
}

/// [`rotate_if_oversized`] 的显式上限版本（供单测用小阈值驱动）。
pub fn rotate_if_oversized_with(path: &Path, max_bytes: u64) -> io::Result<bool> {
    if current_generation_bytes(path) < max_bytes {
        return Ok(false);
    }
    // 丢掉最老的一代，为新的一代腾位置。
    let oldest = generation_path(path, KEEP_GENERATIONS - 1);
    if oldest.exists() {
        let _ = fs::remove_file(&oldest);
    }
    // 从倒数第二代开始整体后移一代（`x.log.1` → `x.log.2`）。
    for gen in (1..KEEP_GENERATIONS - 1).rev() {
        let from = generation_path(path, gen);
        if from.exists() {
            fs::rename(&from, generation_path(path, gen + 1))?;
        }
    }
    // 当前文件本身降为 `.1`。
    fs::rename(path, generation_path(path, 1))?;
    Ok(true)
}

/// 第 `gen` 代历史文件路径（`gen = 1` 即 `x.log.1`）。
fn generation_path(path: &Path, gen: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{gen}"));
    PathBuf::from(name)
}

/// 把目录内属于本模块的日志文件总量压回 [`MAX_DIR_BYTES`]，删最旧。
///
/// 返回被删除的文件路径（供调用方记账 / 单测断言）。
///
/// 「最旧」按**修改时间**判定而非文件名：不同端口的日志文件名不可比大小，
/// 而轮转产生的 `.1`/`.2` 在语义上确实更老。三重保护：
///
/// - 只动 `sidecar-` 前缀的文件，目录里别的东西（用户手记的 note.txt）不受影响；
/// - **绝不删当前正在写的文件**（无`.N` 后缀且 mtime 最新那一个）——
///   删掉它等于让子进程写到已被 unlink 的 inode（Windows 上直接失败）；
/// - 逐个累加，删到落回预算内即止，不做「先全算再一次性删」的破坏性批量操作。
pub fn prune_dir_to_budget(dir: &Path) -> io::Result<Vec<PathBuf>> {
    prune_dir_to_budget_with(dir, MAX_DIR_BYTES)
}

/// [`prune_dir_to_budget`] 的显式预算版本（供单测用小预算驱动）。
pub fn prune_dir_to_budget_with(dir: &Path, budget: u64) -> io::Result<Vec<PathBuf>> {
    prune_dir_matching_to_budget(dir, budget, &|name| name.starts_with(LOG_NAME_PREFIX))
}

/// 通用版：**由调用方判定哪些文件属于自己**，把目录总量压回 `budget`，
/// 按 mtime 删最旧、永不删最新那一份。返回被删的路径。
///
/// 为什么要抽这一层：`context/` 台账目录与路由决策日志（W4）需要的正是同一套
/// 「从最旧开始删到预算内」的逻辑，而本仓已经有两份各自实现的轮转，不该再来第三份。
/// 判定条件用闭包而不是前缀：台账目录认的是 `.json` 后缀，而且要**排除**
/// `*.corrupt-*.json` 那份隔离件 —— 它是损坏台账唯一的人工恢复凭据，
/// 不该在预算压力下被静默回收。
///
/// ⚠️ 与侧车版同样的取舍：**mtime 最新的一份永不删**。对台账目录来说「最新」
/// 极可能就是正在被这个进程读写的那份会话，删掉它等于把历史就地抹掉。
pub fn prune_dir_matching_to_budget(
    dir: &Path,
    budget: u64,
    belongs: &dyn Fn(&str) -> bool,
) -> io::Result<Vec<PathBuf>> {
    let mut candidates: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !belongs(name) {
            continue;
        }
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        candidates.push((path, mtime, meta.len()));
    }
    let mut total: u64 = candidates.iter().map(|(_, _, len)| *len).sum();
    if total <= budget {
        return Ok(Vec::new());
    }
    // 最新的那个文件是当前正在写的，永不删除。
    candidates.sort_by_key(|(_, mtime, _)| *mtime);
    let active = candidates.last().map(|(p, _, _)| p.clone());
    let mut removed = Vec::new();
    // 升序遍历即「从最旧开始删」；删到落回预算内即止。
    for (path, _, len) in candidates {
        if total <= budget {
            break;
        }
        if active.as_ref() == Some(&path) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
            removed.push(path);
        }
    }
    Ok(removed)
}

/// 往日志文件追加一行（自动补换行）。
///
/// 供调用方在需要结构化打点时使用；`tracing` 自身的文件日志
/// 走 [`crate::cli::program`] 的初始化，不经过这里。
pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut file = open_append(path)?;
    writeln!(file, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例独占一个临时目录（用线程 id 区分，避免并行用例互踩）。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "marguerite-logrotate-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    #[test]
    fn opens_append_mode_and_creates_file() {
        let dir = temp_dir("append");
        let path = dir.join("sidecar-1.err.log");
        let mut f = open_rotating(&path).expect("首次打开应成功");
        writeln!(f, "first").unwrap();
        drop(f);
        let mut f = open_rotating(&path).expect("二次打开应成功");
        writeln!(f, "second").unwrap();
        drop(f);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("first"), "追加而非截断: {text}");
        assert!(text.contains("second"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_when_file_exceeds_threshold() {
        let dir = temp_dir("rotate");
        let path = dir.join("sidecar-61750.err.log");
        fs::write(&path, "x".repeat(2048)).unwrap();
        assert!(
            rotate_if_oversized_with(&path, 1024).unwrap(),
            "超限必须轮转"
        );
        assert!(!path.exists(), "当前文件应已被移走");
        assert_eq!(
            fs::read_to_string(generation_path(&path, 1)).unwrap().len(),
            2048,
            "上一代内容必须完整保留在 .1"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn does_not_rotate_below_threshold() {
        let dir = temp_dir("norotate");
        let path = dir.join("sidecar-2.err.log");
        fs::write(&path, "x".repeat(16)).unwrap();
        assert!(!rotate_if_oversized_with(&path, 1024).unwrap());
        assert!(path.exists(), "未超限时当前文件必须原地保留");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_keeps_only_bounded_generations() {
        let dir = temp_dir("gens");
        let path = dir.join("sidecar-3.err.log");
        // 连续切 4 代，历史代数必须有界。
        // `KEEP_GENERATIONS` 含当前文件（见常量文档），故保留 `.1`/`.2`，
        // 第 `KEEP_GENERATIONS` 代（=`.3`）必须已被丢弃。
        for i in 0..4 {
            fs::write(&path, format!("gen{i}").repeat(2048)).unwrap();
            rotate_if_oversized_with(&path, 1024).unwrap();
        }
        assert!(
            !generation_path(&path, KEEP_GENERATIONS).exists(),
            "超出保留代数的历史必须被丢弃"
        );
        assert!(generation_path(&path, 1).exists(), ".1 必须保留");
        assert!(
            generation_path(&path, KEEP_GENERATIONS - 1).exists(),
            ".{} 仍在保留窗口内",
            KEEP_GENERATIONS - 1
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_removes_oldest_until_within_budget() {
        let dir = temp_dir("prune");
        let active = dir.join("sidecar-9.err.log");
        let older = dir.join("sidecar-8.err.log");
        let newest_hist = dir.join("sidecar-7.err.log");
        fs::write(&older, "a".repeat(1000)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&newest_hist, "b".repeat(1000)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&active, "c".repeat(1000)).unwrap();
        // 总量 3000（含正在写的 active），预算 2500：删掉最旧的一个即落回 2000。
        // ⚠️ 预算是**整个目录**的量，active 也计入——所以预算必须给出
        // 足以「删一个就达标」的余量，否则会连带删掉 newest_hist。
        let removed = prune_dir_to_budget_with(&dir, 2500).unwrap();
        assert!(!older.exists(), "最旧文件必须被删");
        assert!(active.exists(), "当前正在写的文件绝不可删");
        assert!(newest_hist.exists(), "预算内则保留历史");
        assert_eq!(removed.len(), 1, "只应删一个: {removed:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_never_touches_unrelated_files() {
        let dir = temp_dir("unrelated");
        let log = dir.join("sidecar-1.err.log");
        let note = dir.join("notes.txt");
        fs::write(&log, "x".repeat(4000)).unwrap();
        fs::write(&note, "y".repeat(4000)).unwrap();
        prune_dir_to_budget_with(&dir, 100).unwrap();
        assert!(note.exists(), "非 sidecar- 前缀文件不在封顶范围内");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_is_noop_within_budget() {
        let dir = temp_dir("noop");
        let path = dir.join("sidecar-4.err.log");
        fs::write(&path, "x".repeat(10)).unwrap();
        assert!(prune_dir_to_budget_with(&dir, 1_000_000).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// W4：判定条件由调用方给 ⇒ 台账目录可以只认 `.json`，而侧车规则不会误伤它。
    /// 两侧都要断言：既不能被侧车口径捞走，也不能反过来误删侧车文件。
    #[test]
    fn prune_with_custom_predicate_scopes_to_that_namespace() {
        let dir = temp_dir("prefix");
        let ledger_old = dir.join("sess-a.json");
        let ledger_new = dir.join("sess-b.json");
        let sidecar = dir.join("sidecar-1.err.log");
        fs::write(&ledger_old, "a".repeat(1000)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&ledger_new, "b".repeat(1000)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&sidecar, "c".repeat(1000)).unwrap();

        // 只算 `.json`：总量 2000，预算 1500 ⇒ 删最旧那一份就达标。
        let is_ledger = |n: &str| n.ends_with(".json");
        let removed = prune_dir_matching_to_budget(&dir, 1_500, &is_ledger).unwrap();
        assert_eq!(removed.len(), 1, "预算内只需删最旧一份: {removed:?}");
        assert!(!ledger_old.exists(), "命名空间内最旧的一份必须被回收");
        assert!(ledger_new.exists(), "最新一份永不删");
        assert!(sidecar.exists(), "不属于这个命名空间的文件不得被波及");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 台账命名空间的真实口径：认 `.json`，但**排除** `*.corrupt-*.json` 隔离件 ——
    /// 那是损坏台账唯一的人工恢复凭据，不该在预算压力下被静默回收。
    #[test]
    fn ledger_predicate_skips_quarantined_copies() {
        let dir = temp_dir("quarantine");
        let ledger_old = dir.join("sess-a.json");
        let ledger_new = dir.join("sess-b.json");
        let corrupt = dir.join("sess-z.corrupt-1730000000000.json");
        fs::write(&ledger_old, "a".repeat(1000)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&corrupt, "c".repeat(1000)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&ledger_new, "b".repeat(1000)).unwrap();

        let is_ledger = |n: &str| n.ends_with(".json") && !n.contains(".corrupt-");
        let removed = prune_dir_matching_to_budget(&dir, 100, &is_ledger).unwrap();
        assert_eq!(removed.len(), 1, "只有真台账参与回收: {removed:?}");
        assert!(!ledger_old.exists(), "最旧的一份台账必须被回收");
        assert!(corrupt.exists(), "隔离件必须活过预算回收");
        assert!(ledger_new.exists(), "最新一份仍受保护");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let missing = std::env::temp_dir().join("marguerite-logrotate-does-not-exist-xyz");
        assert!(prune_dir_to_budget(&missing).unwrap().is_empty());
    }
}