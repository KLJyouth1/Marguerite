//! 沙箱交互式终端：常驻受限 shell 会话（spawn / input / read / kill）
//!
//! 与 `sandboxed_exec`（一次性执行）互补：终端保持进程存活，支持逐步写入
//! 与增量读取，工作目录限定为 spawn 时传入的 cwd，子进程 `kill_on_drop` 防挂死。
//!
//! 会话为内存态注册表（仿 `long_running` 单例），跨进程不持久。
//!
//! # 安全接线（任务 #9 / P0-E）
//!
//! 本模块是 Marguerite 的**第三条命令执行通道**（前两条：`tools::builtin::shell_exec`、
//! `terminal::TerminalSession::submit`）。接线前它完全无护栏，构成持久化旁路：
//! 只要走 `sandbox.terminal.*` RPC，前两条通道焊死的 L1/L2/L3 全部形同虚设。
//!
//! 现在的裁决口径与 `tools::builtin` **完全一致**：
//! - **L2/L3 + L1 在 `write()` 内**：护栏放在库方法而非仅 RPC 处理器里，
//!   任何调用方（RPC / 内部代码）写入常驻 shell 之前都必须过闸；
//! - **工作区约束在 RPC 边界**：`terminal_spawn` 用 `safe_resolve` 把不可信的
//!   `cwd` 参数收敛进工作区。`TerminalSession::spawn` 本身保持接受任意已存在目录
//!   （受信内部调用方 / 测试需要），**不可信输入的收敛职责归 RPC 层**；
//! - **全量审计**：放行与拒绝都落 JSONL，与 `terminal::TerminalSession` 同一套 sink。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;

use crate::sandbox::audit::{default_audit_dir, AuditRecord, AuditSink};
use crate::security::scanner_l3::OpKind;
use crate::security::{normalize_workspace, CommandGuard, Preflight};
use crate::tools::fs_path::safe_resolve;

/// 并发终端会话上限（防资源耗尽）
pub const MAX_SESSIONS: usize = 16;

/// 单会话输出缓冲上限（有界：防止 `pipe_reader` 在消费者滞后时无限堆积）
pub const TERMINAL_OUTPUT_BUFFER: usize = 4096;

/// 进程级共享的护栏实例（L2 要编译约 20 条正则，避免每个会话重复构造）。
/// 与 `tools::builtin::PREFLIGHT` 同源同序。
static PREFLIGHT: LazyLock<Preflight> = LazyLock::new(Preflight::new);
static GUARD: LazyLock<CommandGuard> = LazyLock::new(CommandGuard::new);

/// 单个终端会话（进程常驻，stdin/stdout/stderr 全管道）
pub struct TerminalSession {
    inner: Mutex<TerminalInner>,
    /// spawn 时的工作目录副本（不变量：会话生命周期内固定）。
    /// 单独存一份是为了做安全裁决时不必抢 `inner` 锁。
    cwd: PathBuf,
    /// 命令审计（None = 仅关闭记录，**不影响拦截**）
    audit: Option<AuditSink>,
}

struct TerminalInner {
    id: String,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: mpsc::Receiver<String>,
    cwd: PathBuf,
    spawned_at_ms: u64,
    closed: bool,
}

impl TerminalSession {
    /// 启动交互式 shell 会话。`cwd` 必须为已存在目录。
    pub async fn spawn(cwd: &Path) -> Result<Arc<TerminalSession>> {
        if !cwd.is_dir() {
            return Err(anyhow!("工作目录不存在: {}", cwd.display()));
        }
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let mut cmd = tokio::process::Command::new(shell);
        cmd.current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("终端启动失败（{}）: {}", shell, e))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("无法获取终端 stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("无法获取终端 stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("无法获取终端 stderr"))?;

        let (tx, rx) = mpsc::channel::<String>(TERMINAL_OUTPUT_BUFFER);
        tokio::spawn(pipe_reader(stdout, tx.clone()));
        tokio::spawn(pipe_reader(stderr, tx));

        let id = format!(
            "term_{}_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(6)
                .collect::<String>(),
        );
        let session = Arc::new(TerminalSession {
            inner: Mutex::new(TerminalInner {
                id,
                child: Some(child),
                stdin: Some(stdin),
                rx,
                cwd: cwd.to_path_buf(),
                spawned_at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                closed: false,
            }),
            cwd: cwd.to_path_buf(),
            audit: Some(AuditSink::new(default_audit_dir(cwd))),
        });
        register_session(session.clone());
        Ok(session)
    }

    /// 记录一条命令审计（写失败静默忽略，审计不阻塞业务）。
    fn audit_input(&self, decision: &str, cmd: &str) {
        let Some(sink) = self.audit.as_ref() else {
            return;
        };
        sink.record(&AuditRecord {
            ts: chrono::Local::now().to_rfc3339(),
            trace_id: uuid::Uuid::new_v4().simple().to_string(),
            decision: decision.to_string(),
            command: cmd.split_whitespace().next().unwrap_or("").to_string(),
            args: vec![cmd.to_string()],
            cwd: self.cwd.to_string_lossy().to_string(),
            exit_code: -1,
            duration_ms: 0,
            stdout_bytes: 0,
            stderr_bytes: 0,
        });
    }

    /// 写入常驻 shell 之前的统一裁决：L2（整段）→ L3 + L1（逐行）。
    ///
    /// 分层理由：
    /// - **L2 扫整段**：`scan_text` 本身是多行感知的，跨行拼装的反弹 shell /
    ///   here-doc 必须整段看才抓得到；
    /// - **L1/L3 逐行**：这两层面向「一条命令」设计，且 `write()` 会补 `\r\n` 提交，
    ///   一次 `data` 里的每一行都会被 shell 当作独立命令执行——只查整段会漏掉
    ///   `echo ok\nchroot /mnt /bin/sh` 这种把危险命令藏在第二行的载荷。
    fn security_check(&self, data: &str) -> Result<()> {
        let cwd = normalize_workspace(&self.cwd);

        // L2：整段内容扫描（多行感知）
        let decision = PREFLIGHT.check_content(data);
        if let Some(reason) = decision.reason() {
            return Err(anyhow!(
                "[{}] 终端输入被分层安全护栏阻断: {}",
                decision.layer().unwrap_or("L?"),
                reason
            ));
        }

        // L3 + L1：逐行裁决
        for line in data.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let d = PREFLIGHT.check_path(OpKind::Exec, Path::new(line), &cwd);
            if let Some(reason) = d.reason() {
                return Err(anyhow!(
                    "[{}] 终端输入被分层安全护栏阻断: {}",
                    d.layer().unwrap_or("L?"),
                    reason
                ));
            }
            GUARD
                .check(line)
                .map_err(|e| anyhow!("[L1] 终端输入被安全策略阻断: {e}"))?;
        }
        Ok(())
    }

    /// 向终端写入输入（自动追加回车提交，等价于按 Enter）。
    ///
    /// **安全**：写入常驻 shell 之前必须过 L2/L3/L1（见 `security_check`）。
    /// 护栏放在这里而不是只放在 RPC 处理器里，是为了让任何调用方都绕不过去。
    pub async fn write(&self, data: &str) -> Result<()> {
        // —— 安全闸口：任何字节进入 shell stdin 之前必须先裁决 ——
        // 这是 P0-E 的修复点：此前本行以下直接写 stdin，形成持久化护栏旁路。
        if let Err(e) = self.security_check(data) {
            self.audit_input("deny", data);
            return Err(e);
        }
        self.audit_input("allow", data);

        // 锁内取出 stdin 所有权，避免跨 await 持有非 Send 的 MutexGuard
        let stdin = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.closed {
                return Err(anyhow!("终端已关闭"));
            }
            inner
                .stdin
                .take()
                .ok_or_else(|| anyhow!("终端 stdin 不可用"))?
        };
        let write_result = async {
            let mut stdin = stdin;
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| anyhow!("写入终端失败: {e}"))?;
            stdin
                .write_all(b"\r\n")
                .await
                .map_err(|e| anyhow!("写入终端失败: {e}"))?;
            stdin
                .flush()
                .await
                .map_err(|e| anyhow!("刷新终端失败: {e}"))?;
            Ok::<ChildStdin, anyhow::Error>(stdin)
        }
        .await;
        match write_result {
            Ok(stdin) => {
                self.inner.lock().unwrap_or_else(|e| e.into_inner()).stdin = Some(stdin);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 增量读取输出（一次取空当前缓冲），并返回进程是否存活。
    pub fn read(&self) -> (String, bool) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = String::new();
        while let Ok(chunk) = inner.rx.try_recv() {
            out.push_str(&chunk);
        }
        let running = inner.running_locked();
        (out, running)
    }

    /// 终止终端会话（SIGKILL/强制结束 + 关闭管道）。
    pub async fn kill(&self) -> Result<()> {
        let child = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.closed = true;
            inner.stdin.take();
            inner.child.take()
        };
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
        Ok(())
    }

    /// 序列化视图（供 RPC / 前端展示）
    pub fn view(&self) -> serde_json::Value {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        serde_json::json!({
            "id": inner.id,
            "cwd": inner.cwd.to_string_lossy(),
            "spawnedAtMs": inner.spawned_at_ms,
            "running": inner.running_locked(),
        })
    }

    pub fn id(&self) -> String {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .id
            .clone()
    }
}

impl TerminalInner {
    /// 进程存活判定（try_wait 非阻塞）
    fn running_locked(&mut self) -> bool {
        if self.closed {
            return false;
        }
        match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => {
                    self.closed = true;
                    false
                }
                Ok(None) => true,
                Err(_) => false,
            },
            None => false,
        }
    }
}

/// 后台读线程：把 stdout / stderr 增量推入（有界）mpsc 队列。
/// 消费者滞后导致缓冲满时 `send` 挂起（反压），订阅方断开则 `send` 失败即退出。
async fn pipe_reader<R>(mut reader: R, tx: mpsc::Sender<String>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if tx
                    .send(String::from_utf8_lossy(&buf[..n]).to_string())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

// --------------------------------------------------------------------------- #
// 全局会话注册表
// --------------------------------------------------------------------------- #

static TERMINALS: LazyLock<Mutex<HashMap<String, Arc<TerminalSession>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 全局终端注册表（内存态）
pub fn global() -> &'static Mutex<HashMap<String, Arc<TerminalSession>>> {
    &TERMINALS
}

fn register_session(session: Arc<TerminalSession>) {
    let mut map = global().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_SESSIONS {
        // 超限：按注册顺序淘汰最早启动的会话
        if let Some(evict) = map.values().min_by_key(|s| s.id()) {
            let id = evict.id();
            map.remove(&id);
        }
    }
    map.insert(session.id(), session);
}

fn get_session(id: &str) -> Option<Arc<TerminalSession>> {
    global()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned()
}

// --------------------------------------------------------------------------- #
// RPC handlers
// --------------------------------------------------------------------------- #

/// `sandbox.terminal.spawn {cwd?}`：启动终端会话
///
/// **安全**：`cwd` 来自不可信 RPC 参数，必须经 `safe_resolve` 收敛进工作区，
/// 否则调用方可以 `{"cwd": "/"}` 直接在根目录起一个常驻 shell。
/// 工作区口径与网关其它 sandbox 方法一致（`std::env::current_dir()`）。
pub async fn terminal_spawn(params: &serde_json::Value) -> serde_json::Value {
    let ws = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd = match params
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(raw) => match safe_resolve(&ws, raw) {
            Ok(p) => p,
            Err(e) => {
                return serde_json::json!({
                    "ok": false,
                    "code": "PATH_ESCAPES_WORKSPACE",
                    "error": format!("cwd 越出工作区，已拒绝: {e}"),
                });
            }
        },
        None => ws,
    };
    match TerminalSession::spawn(&cwd).await {
        Ok(session) => serde_json::json!({ "ok": true, "terminal": session.view() }),
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `sandbox.terminal.input {terminalId, data}`：写入输入
pub async fn terminal_input(params: &serde_json::Value) -> serde_json::Value {
    let id = params
        .get("terminalId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let data = params.get("data").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return serde_json::json!({ "ok": false, "error": "terminalId 不可为空" });
    }
    let session = match get_session(id) {
        Some(s) => s,
        None => return serde_json::json!({ "ok": false, "error": "终端会话不存在" }),
    };
    match session.write(data).await {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `sandbox.terminal.read {terminalId}`：增量读取输出
pub fn terminal_read(params: &serde_json::Value) -> serde_json::Value {
    let id = params
        .get("terminalId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return serde_json::json!({ "ok": false, "error": "terminalId 不可为空" });
    }
    let session = match get_session(id) {
        Some(s) => s,
        None => return serde_json::json!({ "ok": false, "error": "终端会话不存在" }),
    };
    let (output, running) = session.read();
    serde_json::json!({ "ok": true, "output": output, "running": running })
}

/// `sandbox.terminal.kill {terminalId}`：终止会话
pub async fn terminal_kill(params: &serde_json::Value) -> serde_json::Value {
    let id = params
        .get("terminalId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return serde_json::json!({ "ok": false, "error": "terminalId 不可为空" });
    }
    let session = match get_session(id) {
        Some(s) => s,
        None => return serde_json::json!({ "ok": false, "error": "终端会话不存在" }),
    };
    match session.kill().await {
        Ok(()) => {
            global()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(id);
            serde_json::json!({ "ok": true, "running": false })
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    }
}

/// `sandbox.terminal.list`：列出全部会话
pub fn terminal_list() -> serde_json::Value {
    let map = global().lock().unwrap_or_else(|e| e.into_inner());
    let sessions: Vec<serde_json::Value> = map.values().map(|s| s.view()).collect();
    serde_json::json!({ "ok": true, "terminals": sessions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ws() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("marguerite_terminal_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn spawn_write_read_kill_roundtrip() {
        let dir = ws();
        let session = TerminalSession::spawn(&dir).await.expect("spawn ok");
        // cmd.exe 输出含 "Microsoft Windows"，sh 输出含 "$"
        session
            .write("echo marguerite-terminal-check")
            .await
            .unwrap();
        // 给 shell 回显留时间
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (output, running) = session.read();
        assert!(running, "终端应保持存活");
        assert!(
            output.contains("marguerite-terminal-check"),
            "输出应包含回显，实际: {:?}",
            output
        );
        session.kill().await.unwrap();
        let (_, running_after) = session.read();
        assert!(!running_after, "kill 后应停止");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn spawn_rejects_missing_cwd() {
        let missing = std::env::temp_dir().join("marguerite_no_such_dir_xyz");
        let res = TerminalSession::spawn(&missing).await;
        assert!(res.is_err(), "不存在的工作目录应被拒绝");
    }

    #[tokio::test]
    async fn registry_list_and_eviction() {
        let dir = ws();
        let session = TerminalSession::spawn(&dir).await.unwrap();
        let listed = terminal_list();
        assert_eq!(listed["ok"], true);
        assert!(listed["terminals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| { t["id"].as_str() == Some(session.id().as_str()) }));
        session.kill().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
