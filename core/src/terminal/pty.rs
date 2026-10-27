//! 真实 PTY 终端（Windows ConPTY / Unix openpty）
//!
//! # 为什么需要
//!
//! 旧终端是「两根本地管道 + `cmd` 常驻」：`stdin`/`stdout` 是普通 pipe，
//! 子进程看到的是「非 tty」，于是 —— 颜色被关掉、进度条退化成一堆文本、
//! `isatty()` 分支全部走脚本模式、Ctrl+C 没有前台进程组可中断、窗口尺寸未知
//! （`COLUMNS`/`LINES` 永远缺失）。用户看到的「终端很差」本质是**没有终端**。
//!
//! 本模块用 [`portable_pty`]（wezterm 的 PTY 抽象）把 shell 真正挂到伪终端上：
//! - **ConPTY**（Windows 10 1809+）让 PowerShell / cmd / Git Bash 都按交互式终端行为运行；
//! - **openpty**（Unix）给出真实 tty，`TERM` 生效；
//! - **resize** 可把前端面板尺寸透传给内核，full-screen 程序才有正确折行；
//! - **子进程退出**由独立等待线程捕获退出码，不再靠「读不到输出」猜。
//!
//! # 输出语义
//!
//! `read_from(cursor)` 是**游标读取**：返回从 `cursor` 起的全部字节与下一个游标。
//! 与旧实现的「一次取空」相比有两点收益：
//! 1. 多个消费者（面板 + 独立终端页）不会互相偷数据；
//! 2. 前端断线重连后可以用游标补齐，而不是丢掉中间输出。
//!
//! 缓冲区有界（[`MAX_BUFFER_BYTES`]），超限时从头部淘汰并累计 `lost` 字节，
//! 读取方据此如实告知「有输出已被丢弃」，而不是静默裁掉。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use portable_pty::{
    native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize,
};

use super::shells::{ShellKind, ShellProfile};

/// 默认列数（前端未上报时）
pub const DEFAULT_COLS: u16 = 120;
/// 默认行数（前端未上报时）
pub const DEFAULT_ROWS: u16 = 30;
/// 单会话输出缓冲上限（超出部分从头部淘汰并计数，不静默丢弃）
pub const MAX_BUFFER_BYTES: usize = 512 * 1024;

const MIN_COLS: u16 = 20;
const MAX_COLS: u16 = 500;
const MIN_ROWS: u16 = 4;
const MAX_ROWS: u16 = 200;

/// 把前端上报的尺寸收敛到合法区间（0 / 异常值一律回落到默认值）。
pub fn clamp_size(cols: u16, rows: u16) -> (u16, u16) {
    let c = if cols == 0 {
        DEFAULT_COLS
    } else {
        cols.clamp(MIN_COLS, MAX_COLS)
    };
    let r = if rows == 0 {
        DEFAULT_ROWS
    } else {
        rows.clamp(MIN_ROWS, MAX_ROWS)
    };
    (c, r)
}

// --------------------------------------------------------------------------- #
// 流式 UTF-8 解码
// --------------------------------------------------------------------------- #

/// 增量 UTF-8 解码器：管道按 4–8KB 切片，中文/emoji 极易被切在码点中间。
///
/// 旧实现每块直接 `String::from_utf8_lossy`，被切断的半个字符会变成 `�`，
/// 且下一个块开头又变成 `�` —— 中文输出成片乱码。这里保留不完整尾字节，
/// 等下一块到达后拼完整再输出。
#[derive(Debug, Default)]
pub struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一块字节，返回已能完整解码的文本（不完整尾字节留在内部）。
    pub fn push(&mut self, bytes: &[u8]) -> String {
        if bytes.is_empty() && self.pending.is_empty() {
            return String::new();
        }
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            let pending = std::mem::take(&mut self.pending);
            match std::str::from_utf8(&pending) {
                Ok(s) => {
                    out.push_str(s);
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    let err_len = e.error_len();
                    if valid > 0 {
                        out.push_str(std::str::from_utf8(&pending[..valid]).unwrap_or_default());
                    }
                    match err_len {
                        Some(n) => {
                            out.push('\u{FFFD}');
                            self.pending = pending[valid + n..].to_vec();
                            continue;
                        }
                        None => {
                            self.pending = pending[valid..].to_vec();
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    /// 结束流：把残留的不完整字节按替换字符输出。
    pub fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let raw = std::mem::take(&mut self.pending);
        String::from_utf8_lossy(&raw).to_string()
    }
}

// --------------------------------------------------------------------------- #
// 有界输出环（游标读取）
// --------------------------------------------------------------------------- #

/// 会话输出缓冲：按字节游标读取 + 有界淘汰。
#[derive(Debug, Default)]
pub struct OutputRing {
    inner: Mutex<RingInner>,
}

#[derive(Debug, Default)]
struct RingInner {
    bytes: Vec<u8>,
    /// 缓冲中第一个字节的全局偏移（淘汰后递增）
    base: u64,
    /// 累计被淘汰的字节数（读取方据此判断「丢了多少」）
    lost: u64,
}

impl OutputRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_str(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.bytes.extend_from_slice(text.as_bytes());
        if inner.bytes.len() > MAX_BUFFER_BYTES {
            let drop_to = inner.bytes.len() - MAX_BUFFER_BYTES;
            let boundary = utf8_boundary_after(&inner.bytes, drop_to);
            inner.bytes.drain(..boundary);
            inner.base += boundary as u64;
            inner.lost += boundary as u64;
        }
    }

    /// 从 `cursor` 读取增量，返回 `(文本, 下一个游标, 本次因淘汰丢失的字节数)`。
    pub fn read_from(&self, cursor: u64) -> (String, u64, u64) {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let start = if cursor < inner.base {
            (inner.base - cursor, 0usize)
        } else {
            let offset = cursor - inner.base;
            (0, offset.min(inner.bytes.len() as u64) as usize)
        };
        let (lost, offset) = start;
        let text = String::from_utf8_lossy(&inner.bytes[offset..]).to_string();
        let next = inner.base + inner.bytes.len() as u64;
        (text, next, lost)
    }

    /// 当前末尾游标（新建会话时为 0）。
    pub fn end_cursor(&self) -> u64 {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.base + inner.bytes.len() as u64
    }

    pub fn lost_bytes(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .lost
    }
}

/// 找 `idx` 之后最近的 UTF-8 字符边界（避免把多字节字符切成两半）。
fn utf8_boundary_after(buf: &[u8], idx: usize) -> usize {
    let mut i = idx.min(buf.len());
    while i < buf.len() && (buf[i] & 0xC0) == 0x80 {
        i += 1;
    }
    i
}

// --------------------------------------------------------------------------- #
// PTY 会话
// --------------------------------------------------------------------------- #

/// 一个真实 PTY 会话。
pub struct PtyTerminal {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Arc<OutputRing>,
    alive: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
    cursor: AtomicU64,
    size: Mutex<(u16, u16)>,
    profile: ShellProfile,
    cwd: PathBuf,
    pid: Option<u32>,
    started_at_ms: u64,
}

impl PtyTerminal {
    /// 在 `cwd` 下用指定 shell 打开一个 PTY 会话。
    ///
    /// `env` 会逐条覆盖子进程环境（在用户环境之上叠加，不清空 —— 清空会让
    /// PATH / 代理 / 许可证变量全部消失，shell 起来即残）。
    pub fn spawn(
        profile: &ShellProfile,
        cwd: &Path,
        cols: u16,
        rows: u16,
        env: &[(String, String)],
    ) -> Result<Self> {
        if !cwd.is_dir() {
            return Err(anyhow!("工作目录不存在: {}", cwd.display()));
        }
        let (cols, rows) = clamp_size(cols, rows);

        // ConPTY 的加载在便携库内部用 `expect`：老旧系统上会 panic。
        // 网关是长驻进程，任何一次 panic 都不该由「选了个不支持的终端」触发。
        let open = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            native_pty_system().openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
        }))
        .map_err(|_| anyhow!("当前系统不支持伪终端（Windows 需要 10 1809+ 的 ConPTY）"))?;
        let pair = open.context("创建伪终端失败")?;

        let mut cmd = CommandBuilder::new(&profile.program);
        for arg in &profile.args {
            cmd.arg(arg);
        }
        if profile.kind == ShellKind::Wsl {
            // wsl.exe 支持 Windows 路径形式的 --cd：直接把会话落到用户工作区，
            // 而不是默认的 /mnt/c/Users/<name>。
            cmd.arg("--cd");
            cmd.arg(cwd.as_os_str());
        }
        cmd.cwd(cwd);
        // 终端能力：没有 TERM 的 PTY 等于「有 tty 但程序仍走无色分支」。
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Marguerite");
        for (key, value) in env {
            cmd.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow!("终端启动失败（{}）: {e}", profile.program))?;
        // slave 必须立刻关闭：否则 master 永远等不到 EOF，子进程退出后会话仍「活着」。
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow!("无法读取终端输出: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow!("无法写入终端: {e}"))?;
        let killer = child.clone_killer();
        let pid = child.process_id();

        let output = Arc::new(OutputRing::new());
        let alive = Arc::new(AtomicBool::new(true));
        let exit_code = Arc::new(Mutex::new(None));

        spawn_reader_thread(reader, Arc::clone(&output));
        spawn_waiter_thread(child, Arc::clone(&alive), Arc::clone(&exit_code));

        let terminal = Self {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            output,
            alive,
            exit_code,
            cursor: AtomicU64::new(0),
            size: Mutex::new((cols, rows)),
            profile: profile.clone(),
            cwd: cwd.to_path_buf(),
            pid,
            started_at_ms: now_ms(),
        };

        // cmd.exe 默认代码页是 GBK(936)：中文路径/输出会乱码。PowerShell / bash 不走这条。
        if profile.kind == ShellKind::Cmd {
            let _ = terminal.write_bytes(b"chcp 65001 >nul\r\n");
        }
        Ok(terminal)
    }

    /// 写入一整行（末尾补 `\r`，等价于按 Enter）。
    pub fn write_line(&self, data: &str) -> Result<()> {
        let mut bytes = Vec::with_capacity(data.len() + 2);
        bytes.extend_from_slice(data.as_bytes());
        bytes.extend_from_slice(b"\r");
        self.write_bytes(&bytes)
    }

    /// 写入原始字节（**只允许**控制键白名单与初始化序列；业务输入必须走 [`Self::write_line`]）。
    pub fn write_bytes(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        writer
            .write_all(bytes)
            .map_err(|e| anyhow!("写入终端失败: {e}"))?;
        writer.flush().map_err(|e| anyhow!("刷新终端失败: {e}"))
    }

    /// 调整 PTY 尺寸（前端面板缩放时调用）。
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(u16, u16)> {
        let (cols, rows) = clamp_size(cols, rows);
        let mut size = self.size.lock().unwrap_or_else(|e| e.into_inner());
        if *size == (cols, rows) {
            return Ok(*size);
        }
        self.master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| anyhow!("调整终端尺寸失败: {e}"))?;
        *size = (cols, rows);
        Ok(*size)
    }

    /// 从会话内部游标读取增量（读取后游标前移）。
    pub fn read(&self) -> (String, bool) {
        let cursor = self.cursor.load(Ordering::Relaxed);
        let (text, next, _lost) = self.output.read_from(cursor);
        self.cursor.store(next, Ordering::Relaxed);
        let running = self.is_alive();
        (text, running)
    }

    /// 从指定游标读取（无状态，多个消费者可各用各的游标）。
    pub fn read_from(&self, cursor: u64) -> (String, u64, u64) {
        self.output.read_from(cursor)
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn exit_code(&self) -> Option<i32> {
        *self.exit_code.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 强制终止（TerminateProcess / SIGHUP）。同步、幂等。
    ///
    /// # 为什么不采信 `ChildKiller::kill()` 的返回值
    ///
    /// `portable-pty` 0.9 的 Windows `WinChildKiller::kill()` 把
    /// `TerminateProcess` 的返回值**判反了**（成功时 `res != 0`，却被当错误返回）。
    /// 旧实现据此决定是否置 `alive=false`，于是：进程其实已经被杀掉，但会话
    /// 永远显示「运行中」——真机表现就是「点了终止，终端还在转」。
    ///
    /// 现在的判据是**等待线程的真实退出信号**：发完 kill 后最多等 300ms 看
    /// `alive` 是否翻假；没翻就退回按 pid 强杀（`taskkill /T /F` / `kill -9`），
    /// 再等 1s。仍未停止才返回 `false`，由调用方如实告知「可能仍在运行」。
    pub fn kill(&self) -> bool {
        if !self.is_alive() {
            return true;
        }
        // 返回值不可信（见上），只当作「已尝试」。
        let _ = self.killer.lock().unwrap_or_else(|e| e.into_inner()).kill();
        if self.wait_stopped(300) {
            return self.finish_stop();
        }
        if let Some(pid) = self.pid {
            force_kill_pid(pid);
        }
        if self.wait_stopped(1000) {
            return self.finish_stop();
        }
        false
    }

    /// 轮询等待进程真正退出（以等待线程写入的 `alive` 为准）。
    fn wait_stopped(&self, timeout_ms: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if !self.is_alive() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// 已确认停止：补一个退出码占位，避免 UI 显示 `exitCode: null` 却已结束。
    fn finish_stop(&self) -> bool {
        let mut code = self.exit_code.lock().unwrap_or_else(|e| e.into_inner());
        if code.is_none() {
            *code = Some(-1);
        }
        true
    }

    pub fn shell_id(&self) -> &'static str {
        self.profile.id()
    }

    pub fn shell_label(&self) -> &'static str {
        self.profile.label()
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    pub fn cursor(&self) -> u64 {
        self.output.end_cursor()
    }

    pub fn lost_bytes(&self) -> u64 {
        self.output.lost_bytes()
    }

    /// 序列化视图（供 RPC / 前端展示）。
    pub fn view(&self) -> serde_json::Value {
        let (cols, rows) = self.size();
        serde_json::json!({
            "cwd": self.cwd.to_string_lossy(),
            "running": self.is_alive(),
            "pty": true,
            "shell": self.profile.id(),
            "shellLabel": self.profile.label(),
            "shellProgram": self.profile.program,
            "pid": self.pid,
            "cols": cols,
            "rows": rows,
            "startedAtMs": self.started_at_ms,
            "exitCode": self.exit_code(),
            "cursor": self.cursor(),
            "lostBytes": self.lost_bytes(),
        })
    }
}

impl Drop for PtyTerminal {
    fn drop(&mut self) {
        // 会话被淘汰 / 进程回收时必须带走子进程，否则会留下孤儿 shell。
        if self.is_alive() {
            let _ = self.kill();
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 按 pid 强杀的兜底路径（见 [`PtyTerminal::kill`] 的说明）。
///
/// 只在 `portable-pty` 的 killer 没让进程真正退出时使用；不带 shell、不加管道，
/// 避免「杀进程」本身变成一条可注入的命令。
fn force_kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        use crate::proc::SilentCommand;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .silent()
            .output();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
}

fn spawn_reader_thread(mut reader: Box<dyn Read + Send>, output: Arc<OutputRing>) {
    let _ = thread::Builder::new()
        .name("marguerite-pty-reader".to_string())
        .spawn(move || {
            let mut decoder = Utf8StreamDecoder::new();
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = decoder.push(&buf[..n]);
                        if !text.is_empty() {
                            output.push_str(&text);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let tail = decoder.flush();
            if !tail.is_empty() {
                output.push_str(&tail);
            }
        });
}

fn spawn_waiter_thread(
    mut child: Box<dyn Child + Send + Sync>,
    alive: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
) {
    let _ = thread::Builder::new()
        .name("marguerite-pty-wait".to_string())
        .spawn(move || {
            let code = match child.wait() {
                Ok(status) => status.exit_code() as i32,
                Err(_) => -1,
            };
            *exit_code.lock().unwrap_or_else(|e| e.into_inner()) = Some(code);
            alive.store(false, Ordering::SeqCst);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::shells::{discover, default_profile};

    #[test]
    fn decoder_survives_split_codepoints() {
        let mut d = Utf8StreamDecoder::new();
        let bytes = "玛格丽特".as_bytes();
        // 故意在第二个字符中间切一刀
        let cut = 4;
        let a = d.push(&bytes[..cut]);
        assert_eq!(a, "玛", "首块应只输出完整字符");
        let b = d.push(&bytes[cut..]);
        assert_eq!(format!("{a}{b}"), "玛格丽特");
    }

    #[test]
    fn decoder_replaces_invalid_bytes_without_panicking() {
        let mut d = Utf8StreamDecoder::new();
        let out = d.push(&[0xff, 0xfe, b'o', b'k']);
        assert!(out.contains('\u{FFFD}'));
        assert!(out.contains("ok"));
    }

    #[test]
    fn clamp_size_rejects_zero_and_extremes() {
        assert_eq!(clamp_size(0, 0), (DEFAULT_COLS, DEFAULT_ROWS));
        assert_eq!(clamp_size(1, 1), (MIN_COLS, MIN_ROWS));
        assert_eq!(clamp_size(9999, 9999), (MAX_COLS, MAX_ROWS));
        assert_eq!(clamp_size(132, 43), (132, 43));
    }

    #[test]
    fn output_ring_reads_by_cursor_and_reports_loss() {
        let ring = OutputRing::new();
        ring.push_str("abc");
        let (text, next, lost) = ring.read_from(0);
        assert_eq!(text, "abc");
        assert_eq!(next, 3);
        assert_eq!(lost, 0);

        // 同一游标重复读：内容稳定（多消费者各自游标互不影响）
        let (again, _, _) = ring.read_from(0);
        assert_eq!(again, "abc");

        ring.push_str("def");
        let (delta, next2, _) = ring.read_from(next);
        assert_eq!(delta, "def");
        assert_eq!(next2, 6);

        // 越界游标（缓冲淘汰过）如实报告丢失量
        let (_t, _n, lost2) = ring.read_from(999);
        assert_eq!(lost2, 0, "游标在未来时应从末尾起读而不是报丢失");
    }

    #[test]
    fn output_ring_trims_on_char_boundary() {
        let ring = OutputRing::new();
        let unit = "汉".repeat(MAX_BUFFER_BYTES / 3 + 1024);
        ring.push_str(&unit);
        assert!(ring.lost_bytes() > 0, "超限必须淘汰");
        let (text, _next, lost) = ring.read_from(0);
        assert!(lost > 0, "从头读取时应如实报告丢失字节");
        assert!(
            text.chars().all(|c| c == '汉'),
            "淘汰必须落在字符边界上，不能出现半个汉字"
        );
    }

    fn shell_for_test() -> Option<ShellProfile> {
        let list = discover();
        default_profile(&list).cloned()
    }

    #[test]
    fn pty_roundtrip_echo_and_exit() {
        // 真实 shell 用例全局串行：并行冷启动会抢 CPU，让「等输出」的窗口失效。
        let _gate = crate::terminal::pty_test_lock().blocking_lock();
        let Some(profile) = shell_for_test() else {
            eprintln!("本机没有可用 shell，跳过 PTY 实跑");
            return;
        };
        let dir = std::env::temp_dir().join(format!("mgt_pty_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let term = PtyTerminal::spawn(&profile, &dir, 100, 30, &[]).expect("PTY 应可创建");

        // 先等 shell 吐出启动序列 / 提示符，再写命令：冷启动期间写入可能被
        // ConPTY 的输入缓冲推迟（高负载下尤其明显），直接断言回显会误判成缺陷。
        let mut cursor = 0;
        let mut boot = String::new();
        let boot_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < boot_deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (delta, next, _) = term.read_from(cursor);
            cursor = next;
            boot.push_str(&delta);
            if !boot.trim().is_empty() {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));

        term.write_line("echo marguerite-pty-check").unwrap();

        let mut collected = boot;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (delta, next, _) = term.read_from(cursor);
            cursor = next;
            collected.push_str(&delta);
            if collected.contains("marguerite-pty-check") {
                break;
            }
        }
        assert!(
            collected.contains("marguerite-pty-check"),
            "PTY 应回显命令输出，实际: {collected:?}"
        );
        assert!(term.is_alive(), "shell 应保持存活");
        assert!(term.kill(), "kill 应确认进程已停止");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
