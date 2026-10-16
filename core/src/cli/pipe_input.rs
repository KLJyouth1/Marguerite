//! `--pipe` 标准输入的**有界**读取（P0-B 离线首启体验）。
//!
//! ## 实测根因（P0-F，2026-09-22，运行时探针证据）
//!
//! 挂起的**直接原因不在本模块**：`core/src/cli/argv.rs` 的 `--pipe` 分支此前只置位
//! `args.pipe = true`，却把 `mode` 留在默认的 `CliMode::Interactive`。于是
//! `Program::run()` 落进「交互模式 - 进入三栏工作台 TUI」分支，`handle_direct()`
//! （即 stdin 读取）**根本没被调用**：
//!
//! ```text
//! $ marguerite-core.exe --pipe < in.txt     # 临时探针实测输出
//! [TMP-PROBE-0] parsed mode=Interactive pipe=true prompt=false unknown_flags=[]
//! # 之后 stdout 是 TUI 转义序列（[?1049h ...），进程 25s 内不退出
//! ```
//!
//! TUI 在**非 TTY stdin**（管道 / 文件重定向）上无人驱动事件流，于是永久挂起 ——
//! 这正是「即使 `< in.txt` 是真文件重定向也不返回」的原因（EOF 到达并不能让 TUI 退出）。
//! 修复：`--pipe` 显式提升为 `CliMode::Direct`（子命令优先），见 `argv.rs`。
//!
//! 本模块解决**第二层**问题：一旦真的进入 `handle_direct()`，原先的
//! `read_to_string(&mut std::io::stdin(), ...)` 是**无界阻塞**——当 stdin 是一个被父进程持有、
//! 既不写入也不关闭的管道句柄时（IDE 终端复用 / agent harness 等），读取端永远等不到 EOF，
//! 而没有任何超时能救回已经阻塞在 `read()` 上的主线程。因此这里用独立读取线程 +
//! 空闲超时收敛，把「无界」变成「有界」。
//!
//! ## 修复契约
//!
//! 1. **显式 EOF 优先**：读到 EOF 立即返回（正常管道 `echo hi | marguerite --pipe` 零等待）；
//! 2. **空闲超时**：连续 [`idle_timeout`] 秒既无新数据也无 EOF 即收敛（默认 5s，
//!    `MARGUERITE_PIPE_TIMEOUT_SECS` 可覆盖），不再无界阻塞；
//! 3. **总时长兜底**：[`TOTAL_CAP_SECS`] 是硬上限，防止持续灌数据导致的无限读取；
//! 4. **交互终端例外**：stdin 是 TTY 时用户自己控制 EOF，仅受总时长兜底约束
//!    （否则「慢慢打字」会被空闲超时误伤），并打印一句结束方式提示。
//!
//! 读取结果如实回报 `eof_seen` / `timed_out`，由调用方决定「空输入报错」还是
//! 「拿到部分内容继续」——本模块不做业务判断，也不吞掉 IO 错误。

use std::io::{IsTerminal, Read};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// 空闲超时默认值（秒）：连续 N 秒无新数据且未见 EOF 即收敛。
///
/// 面向「管道被父进程持有」的挂起场景；正常管道读到 EOF 时该超时根本不参与。
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 5;

/// 读取总时长硬兜底（秒）：无论数据是否持续到达，超过即收敛。
pub const TOTAL_CAP_SECS: u64 = 300;

/// 空闲超时环境变量：`MARGUERITE_PIPE_TIMEOUT_SECS=<秒>`（非法值回退默认）。
pub const IDLE_TIMEOUT_ENV: &str = "MARGUERITE_PIPE_TIMEOUT_SECS";

/// 一次有界读取的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedRead {
    /// 已读取的文本（UTF-8 有损解码：管道可能携带非 UTF-8 字节）。
    pub text: String,
    /// 是否真的读到了 EOF（false = 因空闲超时 / 总时长兜底收敛）。
    pub eof_seen: bool,
    /// 是否在未见 EOF 的情况下收敛（即「stdin 被持有」场景被边界截断）。
    pub timed_out: bool,
}

impl BoundedRead {
    /// 是否因未收到任何数据而收敛（调用方据此判定「没有输入」）。
    pub fn is_empty_timeout(&self) -> bool {
        self.timed_out && self.text.trim().is_empty()
    }
}

/// 解析空闲超时（纯函数，便于单测）：仅接受 >=1 的整数秒。
fn idle_timeout_from_value(raw: Option<&str>) -> Duration {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS))
}

/// 当前生效的空闲超时（`MARGUERITE_PIPE_TIMEOUT_SECS` 优先）。
pub fn idle_timeout() -> Duration {
    idle_timeout_from_value(std::env::var(IDLE_TIMEOUT_ENV).ok().as_deref())
}

/// 读取线程 → 主线程的消息。
enum PipeMsg {
    /// 一段新数据
    Chunk(Vec<u8>),
    /// 读到 EOF（`Ok(0)`）
    Eof,
    /// 读失败（原样上报，不伪装成 EOF）
    Failed(String),
}

/// 在独立 OS 线程中读取 `reader`，主线程按 `idle` / `total` 两个边界收敛。
///
/// 为什么用线程：`std::io::Stdin::read` 无超时版本，一旦阻塞在主线程上就无法再施加
/// 任何边界。读取线程可被安全放弃（进程退出时随进程回收），主线程永远有界返回。
///
/// - `idle`：两次消息之间的最大等待（无数据且无 EOF 达到该时长即收敛）；
/// - `total`：从开始读取算起的总时长上限（硬兜底，防止持续灌数据）。
pub fn read_bounded<R>(mut reader: R, idle: Duration, total: Duration) -> std::io::Result<BoundedRead>
where
    R: Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<PipeMsg>();
    // 读取线程：只做 IO + 上报，不做业务判断。
    std::thread::spawn(move || pump(&mut reader, &tx));

    let start = Instant::now();
    let mut bytes: Vec<u8> = Vec::new();
    let mut eof_seen = false;
    loop {
        let elapsed = start.elapsed();
        if elapsed >= total {
            break;
        }
        let wait = idle.min(total - elapsed);
        match rx.recv_timeout(wait) {
            Ok(PipeMsg::Chunk(chunk)) => bytes.extend_from_slice(&chunk),
            Ok(PipeMsg::Eof) => {
                eof_seen = true;
                break;
            }
            Ok(PipeMsg::Failed(msg)) => {
                return Err(std::io::Error::other(msg));
            }
            // 空闲超时 / 总时长兜底 → 收敛（timed_out 由 eof_seen 推导）
            Err(RecvTimeoutError::Timeout) => break,
            // 发送端已销毁但未送 EOF：视为流结束（不会再有数据）
            Err(RecvTimeoutError::Disconnected) => {
                eof_seen = true;
                break;
            }
        }
    }

    Ok(BoundedRead {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        eof_seen,
        timed_out: !eof_seen,
    })
}

/// 读取线程主体：把数据 / EOF / 错误逐段送进通道。
fn pump<R: Read>(reader: &mut R, tx: &Sender<PipeMsg>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(PipeMsg::Eof);
                return;
            }
            Ok(n) => {
                if tx.send(PipeMsg::Chunk(buf[..n].to_vec())).is_err() {
                    return; // 主线程已收敛并放弃接收
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = tx.send(PipeMsg::Failed(e.to_string()));
                return;
            }
        }
    }
}

/// 读取 `--pipe` 的标准输入（生产入口）。
///
/// - TTY（交互终端）：提示结束方式，仅受 [`TOTAL_CAP_SECS`] 兜底（用户自己控制 EOF）；
/// - 非 TTY（管道 / 重定向）：空闲超时 [`idle_timeout`] + 总时长兜底。
pub fn read_stdin_pipe() -> std::io::Result<BoundedRead> {
    let total = Duration::from_secs(TOTAL_CAP_SECS);
    if std::io::stdin().is_terminal() {
        eprintln!(
            "提示: --pipe 面向管道输入；在终端中请直接输入内容，输入完成后按 Ctrl+Z 回车（Windows）/ Ctrl+D（Unix）结束。"
        );
        // 交互终端下用户手动结束输入：把空闲窗口放宽到总兜底，避免「打字停顿」被截断。
        return read_bounded(std::io::stdin(), total, total);
    }
    read_bounded(std::io::stdin(), idle_timeout(), total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// 永不 EOF 且不产出数据的读取器（模拟被父进程持有、不写不关的 stdin 管道）。
    /// `stop` 置位后返回 EOF，让测试收尾时线程可以退出。
    struct HangingReader {
        stop: Arc<AtomicBool>,
    }

    impl Read for HangingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            while !self.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(0)
        }
    }

    /// 持续产出数据、永不 EOF 的读取器（验证总时长兜底）。
    struct EndlessReader {
        stop: Arc<AtomicBool>,
    }

    impl Read for EndlessReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(5));
            buf[0] = b'x';
            Ok(1)
        }
    }

    #[test]
    fn test_read_bounded_returns_immediately_on_eof() {
        // 正常管道：写到 EOF 即返回，空闲超时完全不参与。
        let sw = Instant::now();
        let got = read_bounded(
            Cursor::new(b"hi\n".to_vec()),
            Duration::from_secs(5),
            Duration::from_secs(300),
        )
        .expect("Cursor 读取不应失败");
        assert_eq!(got.text, "hi\n");
        assert!(got.eof_seen, "读到 EOF 必须如实标记");
        assert!(!got.timed_out);
        assert!(!got.is_empty_timeout());
        assert!(
            sw.elapsed() < Duration::from_secs(2),
            "EOF 路径必须立即返回，实测 {:?}",
            sw.elapsed()
        );
    }

    #[test]
    fn test_read_bounded_empty_eof_is_not_timeout() {
        // `marguerite --pipe < /dev/null`：EOF 立即到达，空内容但不是超时。
        let got = read_bounded(
            Cursor::new(Vec::new()),
            Duration::from_secs(5),
            Duration::from_secs(300),
        )
        .expect("空输入不应失败");
        assert!(got.text.is_empty());
        assert!(got.eof_seen);
        assert!(!got.timed_out);
        assert!(!got.is_empty_timeout(), "EOF 到达的空输入不算超时");
    }

    #[test]
    fn test_read_bounded_times_out_without_eof() {
        // 回归（P0-B 挂起修复）：stdin 被持有且不产生 EOF 时，必须有界收敛。
        let stop = Arc::new(AtomicBool::new(false));
        let sw = Instant::now();
        let got = read_bounded(
            HangingReader { stop: stop.clone() },
            Duration::from_millis(200),
            Duration::from_secs(300),
        )
        .expect("超时收敛不是 IO 错误");
        let elapsed = sw.elapsed();
        stop.store(true, Ordering::Relaxed);
        assert!(
            elapsed < Duration::from_secs(3),
            "必须被空闲超时截断（实测 {elapsed:?}）"
        );
        assert!(got.text.is_empty());
        assert!(!got.eof_seen);
        assert!(got.timed_out);
        assert!(got.is_empty_timeout(), "无数据超时 → 调用方应报「没有输入」");
    }

    #[test]
    fn test_read_bounded_keeps_partial_data_on_timeout() {
        // 先给数据、再挂住：拿到部分内容 + 明确标记 timed_out（不丢用户已输入的内容）。
        let stop = Arc::new(AtomicBool::new(false));
        let reader = PartialThenHang {
            stop: stop.clone(),
            sent: false,
        };
        let got = read_bounded(reader, Duration::from_millis(200), Duration::from_secs(300))
            .expect("部分读取不应失败");
        stop.store(true, Ordering::Relaxed);
        assert_eq!(got.text, "abc");
        assert!(got.timed_out);
        assert!(!got.is_empty_timeout());
    }

    /// 先吐一次 "abc"，随后挂住不出 EOF。
    struct PartialThenHang {
        stop: Arc<AtomicBool>,
        sent: bool,
    }

    impl Read for PartialThenHang {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.sent {
                self.sent = true;
                buf[..3].copy_from_slice(b"abc");
                return Ok(3);
            }
            while !self.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(0)
        }
    }

    #[test]
    fn test_read_bounded_total_cap_stops_endless_stream() {
        // 总时长兜底：数据一直到达（空闲超时永不触发）也必须被总量上限收敛。
        let stop = Arc::new(AtomicBool::new(false));
        let sw = Instant::now();
        let got = read_bounded(
            EndlessReader { stop: stop.clone() },
            Duration::from_secs(30),
            Duration::from_millis(400),
        )
        .expect("兜底收敛不应失败");
        let elapsed = sw.elapsed();
        stop.store(true, Ordering::Relaxed);
        assert!(
            elapsed < Duration::from_secs(5),
            "总时长兜底必须生效（实测 {elapsed:?}）"
        );
        assert!(got.timed_out, "未见 EOF → timed_out");
        assert!(!got.text.is_empty(), "已收到的数据必须保留");
    }

    #[test]
    fn test_read_bounded_reports_io_error_without_masking_as_eof() {
        // 读失败必须原样上报，不得伪装成 EOF（否则「读取失败」会被当成「用户没输入」）。
        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        let err = read_bounded(
            FailingReader,
            Duration::from_secs(5),
            Duration::from_secs(300),
        )
        .expect_err("IO 错误必须上报");
        assert!(err.to_string().contains("boom"), "错误信息应保留: {err}");
    }

    #[test]
    fn test_idle_timeout_env_parsing() {
        // 纯函数：合法值生效、非法 / 缺失 / 0 回退默认。
        assert_eq!(idle_timeout_from_value(None), Duration::from_secs(5));
        assert_eq!(idle_timeout_from_value(Some("")), Duration::from_secs(5));
        assert_eq!(idle_timeout_from_value(Some("0")), Duration::from_secs(5));
        assert_eq!(idle_timeout_from_value(Some("-3")), Duration::from_secs(5));
        assert_eq!(idle_timeout_from_value(Some("abc")), Duration::from_secs(5));
        assert_eq!(idle_timeout_from_value(Some(" 12 ")), Duration::from_secs(12));
        assert_eq!(idle_timeout_from_value(Some("1")), Duration::from_secs(1));
    }

    #[test]
    fn test_read_bounded_reads_multiline_input_completely() {
        // 回归（P0-F，多行契约）：多行输入必须被**完整**读取，绝不允许「读一行就返回」。
        // 抽出 `read_bounded(reader, idle, total)` 正是为了能在此确定性地喂多行字节流：
        // 参数化 reader 走的是与生产完全相同的 pump/通道/EOF 路径（`read_stdin_pipe()`
        // 只是把 `std::io::stdin()` 传进来），故此处对有界读取内核的覆盖等价有效。
        let text = "line one\nline two\nline three\n第四行 with CJK\n";
        let got = read_bounded(
            Cursor::new(text.as_bytes().to_vec()),
            Duration::from_secs(5),
            Duration::from_secs(300),
        )
        .expect("多行输入不应失败");
        assert_eq!(got.text, text, "多行输入必须逐字节完整保留");
        assert_eq!(got.text.lines().count(), 4, "不得只读到第一行");
        assert!(got.eof_seen);
        assert!(!got.timed_out);
    }

    #[test]
    fn test_read_bounded_multiline_arriving_in_chunks_is_concatenated() {
        // 多段到达（真实管道会分多次 read 返回）：必须按序拼接，不丢段、不乱序。
        struct Chunked {
            chunks: Vec<&'static [u8]>,
            idx: usize,
            stop: Arc<AtomicBool>,
        }
        impl Read for Chunked {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.idx < self.chunks.len() {
                    let c = self.chunks[self.idx];
                    self.idx += 1;
                    buf[..c.len()].copy_from_slice(c);
                    return Ok(c.len());
                }
                // 数据发完后挂住（不发 EOF），验证「已收内容 + 有界收敛」两点。
                while !self.stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(0)
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let reader = Chunked {
            chunks: vec![b"alpha\n", b"beta\n", b"gamma\n"],
            idx: 0,
            stop: stop.clone(),
        };
        let got = read_bounded(reader, Duration::from_millis(300), Duration::from_secs(300))
            .expect("分段多行读取不应失败");
        stop.store(true, Ordering::Relaxed);
        assert_eq!(got.text, "alpha\nbeta\ngamma\n", "分段数据必须按序完整拼接");
        assert!(got.timed_out, "未见 EOF 必须如实标记");
        assert!(!got.is_empty_timeout(), "有内容就不算空输入");
    }

    #[test]
    fn test_read_bounded_empty_input_fails_fast_instead_of_hanging() {
        // 回归（P0-F，空输入快速失败）：`--pipe < NUL`（EOF 立即到达但零字节）必须**立即**
        // 返回，而不是挂起，也不能被误判成「超时」——调用方据此给出明确错误 + 非零退出。
        let sw = Instant::now();
        let got = read_bounded(
            Cursor::new(Vec::new()),
            Duration::from_secs(300), // 故意给一个很大的空闲窗口：正确实现靠 EOF 立即返回
            Duration::from_secs(300),
        )
        .expect("空输入不应是 IO 错误");
        let elapsed = sw.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "空输入必须在 EOF 上立即收敛（实测 {elapsed:?}），绝不等待空闲超时"
        );
        assert!(got.text.trim().is_empty(), "调用方据此报错并非零退出");
        assert!(got.eof_seen, "空输入是正常 EOF，不是超时");
        assert!(!got.timed_out);
    }

    #[test]
    fn test_read_bounded_whitespace_only_input_is_empty_for_caller() {
        // 纯空白等价于空输入：调用方 `trim().is_empty()` 的判据必须有测试锚定。
        let got = read_bounded(
            Cursor::new(b"   \n\t\n".to_vec()),
            Duration::from_secs(5),
            Duration::from_secs(300),
        )
        .expect("空白输入不应失败");
        assert!(got.text.trim().is_empty(), "纯空白必须被视为未提供指令");
        assert!(!got.is_empty_timeout(), "纯空白有数据到达，不属于「无数据超时」");
    }

    #[test]
    fn test_total_cap_is_a_sane_backstop() {
        // 兜底值必须远大于默认空闲超时，避免把「正常慢速管道」误判成超时。
        // 用编译期断言（常量两侧恒真，运行期断言会被 clippy::assertions_on_constants 判为无效；
        // const 块在编译期即校验，强度更高）。
        const _: () = assert!(TOTAL_CAP_SECS > DEFAULT_IDLE_TIMEOUT_SECS * 10);
        const _: () = assert!(TOTAL_CAP_SECS >= 60);
    }
}
