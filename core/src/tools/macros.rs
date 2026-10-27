//! 操作录制与回放（`auto_record` / `auto_replay`）——RPA 的录放闭环。
//!
//! # 为什么需要（2026-10-23）
//!
//! 能力台账里 `auto.record` / `auto.replay` 长期是**纯叙事**：产品页写着
//! 「操作录制 / 回放编排」，工具层一行实现都没有。用户的真实诉求是
//! 「我手动做一遍，你照着重复做」——这类任务靠 `system_control` 一步步
//! 猜坐标永远做不稳。
//!
//! # 实现（真实路径，不依赖任何第三方依赖库）
//!
//! - **录制**：后台起一个 PowerShell 进程，用 `Add-Type` 内联 C# 装上
//!   `WH_MOUSE_LL` / `WH_KEYBOARD_LL` 全局钩子，边泵消息边把事件写成 JSONL；
//!   停止靠**哨兵文件**（跨进程通信最简且不会卡死）。
//! - **回放**：把 JSONL 编译成一段 `.ps1`（含时间轴 `Start-Sleep`），
//!   用 `SetCursorPos` + `mouse_event` + `keybd_event` 逐事件重放。
//!   走**临时脚本文件**而不是 `-Command`：60 秒的录制轻松上千个事件，
//!   超过 Windows 命令行 32K 上限（这正是「短宏能跑、长宏静默失败」的经典坑）。
//!
//! # 诚实边界
//!
//! - 键盘按 **虚拟键码** 回放，能复现快捷键（Ctrl+C 等）；但经输入法上屏的
//!   **中文/emoji 文本**不会逐字重现（录到的是 IME 的组合键）。需要文字时
//!   请让智能体改用 `system_control` 的 `type_text`。
//! - 录到的是**屏幕绝对坐标**：回放前目标窗口必须处于相同位置/分辨率。
//!   因此回放默认先 `AppActivate` 录制时记下的窗口标题（`focus=0` 可关闭）。
//! - 仅在 Windows 可用；其它平台明确报错，不假装成功。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::proc::SilentCommand;

/// 单条录制事件（宽松解析：字段缺失不报错，由回放端按 `k` 取用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MacroEvent {
    /// 相对录制开始的毫秒偏移
    #[serde(default)]
    pub t: u64,
    /// 事件类型：move / click / scroll / key
    #[serde(default)]
    pub k: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
    /// 滚轮增量（120 的整数倍，正上负下）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v: Option<i32>,
    /// 虚拟键码
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vk: Option<u32>,
    /// 1=按下 0=抬起
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<u8>,
    /// 鼠标键：left/right/middle
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
    /// 双击标记
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<u8>,
    /// 事件发生时前台窗口标题（回放前用它激活目标窗口）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<String>,
}

/// 解析 JSONL；返回 (事件, 丢弃行数)。空行忽略，坏行计入丢弃。
pub fn parse_events(raw: &str) -> (Vec<MacroEvent>, usize) {
    let mut out = Vec::new();
    let mut bad = 0usize;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<MacroEvent>(line) {
            Ok(e) if !e.k.is_empty() => out.push(e),
            _ => bad += 1,
        }
    }
    out.sort_by_key(|e| e.t);
    (out, bad)
}

/// 宏的存储目录（与应用配置同源，随 `MARGUERITE_CONFIG_DIR` 隔离）。
pub fn macros_dir() -> PathBuf {
    crate::persistence::config::Config::config_dir().join("macros")
}

fn safe_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 64 {
        return None;
    }
    if !n
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return None;
    }
    Some(n.to_string())
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// 生成录制脚本（纯函数，便于单测断言关键片段）。
///
/// `out` = 事件 JSONL 输出路径；`sentinel` = 出现即停止；`capture` = all/mouse/keyboard。
pub fn build_record_script(out: &Path, sentinel: &Path, duration_ms: u64, capture: &str) -> String {
    let want_mouse = capture != "keyboard";
    let want_kbd = capture != "mouse";
    format!(
        r#"$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
public static class MgtRec {{
  public delegate IntPtr Proc(int nCode, IntPtr wParam, IntPtr lParam);
  [StructLayout(LayoutKind.Sequential)] public struct POINT {{ public int X; public int Y; }}
  [StructLayout(LayoutKind.Sequential)] public struct MSLLHOOKSTRUCT {{ public POINT pt; public uint mouseData; public uint flags; public uint time; public IntPtr dwExtraInfo; }}
  [StructLayout(LayoutKind.Sequential)] public struct KBDLLHOOKSTRUCT {{ public uint vkCode; public uint scanCode; public uint flags; public uint time; public IntPtr dwExtraInfo; }}
  [DllImport("user32.dll")] static extern IntPtr SetWindowsHookExW(int id, Proc p, IntPtr h, uint t);
  [DllImport("user32.dll")] static extern bool UnhookWindowsHookEx(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr h, int n, IntPtr w, IntPtr l);
  [DllImport("kernel32.dll", CharSet=CharSet.Auto)] static extern IntPtr GetModuleHandle(string n);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  static List<string> Lines = new List<string>();
  static Stopwatch Sw = Stopwatch.StartNew();
  static long LastMove = -100000; static int LastX = -99999, LastY = -99999;
  static bool WantMouse = {want_mouse};
  static bool WantKbd = {want_kbd};
  static IntPtr HM = IntPtr.Zero, HK = IntPtr.Zero;
  static Proc PM = MouseCb, PK = KbdCb;
  static string Esc(string s) {{
    if (s == null) return "";
    var sb = new StringBuilder();
    foreach (char c in s) {{
      if (c == '"' || c == '\\') sb.Append('\\').Append(c);
      else if (c < ' ') sb.Append(' ');
      else sb.Append(c);
    }}
    return sb.ToString();
  }}
  static string Win() {{
    try {{
      var sb = new StringBuilder(256);
      GetWindowTextW(GetForegroundWindow(), sb, sb.Capacity);
      return Esc(sb.ToString());
    }} catch {{ return ""; }}
  }}
  static long Now() {{ return Sw.ElapsedMilliseconds; }}
  static IntPtr MouseCb(int n, IntPtr w, IntPtr l) {{
    try {{
      var m = (MSLLHOOKSTRUCT)Marshal.PtrToStructure(l, typeof(MSLLHOOKSTRUCT));
      long t = Now();
      int msg = w.ToInt32();
      if (msg == 0x0200) {{
        // 移动事件量极大：40ms + 4px 双阈值降采样，否则一次录制几十万行
        if (t - LastMove >= 40 && (Math.Abs(m.pt.X - LastX) > 4 || Math.Abs(m.pt.Y - LastY) > 4)) {{
          LastMove = t; LastX = m.pt.X; LastY = m.pt.Y;
          Lines.Add("{{\"t\":" + t + ",\"k\":\"move\",\"x\":" + m.pt.X + ",\"y\":" + m.pt.Y + "}}");
        }}
      }} else if (msg == 0x0201 || msg == 0x0204 || msg == 0x0207 || msg == 0x020A) {{
        string b = (msg == 0x0204 || msg == 0x020A) ? "right" : ((msg == 0x0207) ? "middle" : "left");
        int dbl = (msg == 0x020A) ? 1 : 0;
        Lines.Add("{{\"t\":" + t + ",\"k\":\"click\",\"x\":" + m.pt.X + ",\"y\":" + m.pt.Y + ",\"b\":\"" + b + "\",\"d\":" + dbl + ",\"w\":\"" + Win() + "\"}}");
      }} else if (msg == 0x020E) {{
        int delta = (short)((m.mouseData >> 16) & 0xFFFF);
        Lines.Add("{{\"t\":" + t + ",\"k\":\"scroll\",\"x\":" + m.pt.X + ",\"y\":" + m.pt.Y + ",\"v\":" + delta + "}}");
      }}
    }} catch {{ }}
    return CallNextHookEx(IntPtr.Zero, n, w, l);
  }}
  static IntPtr KbdCb(int n, IntPtr w, IntPtr l) {{
    try {{
      var k = (KBDLLHOOKSTRUCT)Marshal.PtrToStructure(l, typeof(KBDLLHOOKSTRUCT));
      int msg = w.ToInt32();
      int down = (msg == 0x0100 || msg == 0x0104) ? 1 : 0;
      Lines.Add("{{\"t\":" + Now() + ",\"k\":\"key\",\"vk\":" + k.vkCode + ",\"down\":" + down + ",\"w\":\"" + Win() + "\"}}");
    }} catch {{ }}
    return CallNextHookEx(IntPtr.Zero, n, w, l);
  }}
  public static bool Install() {{
    IntPtr h = GetModuleHandle(null);
    if (WantMouse) HM = SetWindowsHookExW(14, PM, h, 0);
    if (WantKbd) HK = SetWindowsHookExW(13, PK, h, 0);
    return (!WantMouse || HM != IntPtr.Zero) && (!WantKbd || HK != IntPtr.Zero);
  }}
  public static void Uninstall() {{
    if (HM != IntPtr.Zero) {{ UnhookWindowsHookEx(HM); HM = IntPtr.Zero; }}
    if (HK != IntPtr.Zero) {{ UnhookWindowsHookEx(HK); HK = IntPtr.Zero; }}
  }}
  public static int Count() {{ return Lines.Count; }}
  public static void Flush(string path) {{
    System.IO.File.WriteAllLines(path, Lines, new UTF8Encoding(false));
  }}
}}
"@
if (-not [MgtRec]::Install()) {{ Write-Error "hook install failed"; exit 2 }}
$out = {out}
$sentinel = {sentinel}
$deadline = (Get-Date).AddMilliseconds({duration_ms})
while ((Get-Date) -lt $deadline -and -not (Test-Path -LiteralPath $sentinel)) {{
  [System.Windows.Forms.Application]::DoEvents()
  Start-Sleep -Milliseconds 5
}}
[MgtRec]::Uninstall()
[MgtRec]::Flush($out)
Write-Output ("recorded=" + [MgtRec]::Count())
"#,
        out = ps_quote(&out.display().to_string()),
        sentinel = ps_quote(&sentinel.display().to_string()),
        want_mouse = if want_mouse { "true" } else { "false" },
        want_kbd = if want_kbd { "true" } else { "false" },
        duration_ms = duration_ms,
    )
}

/// 生成回放脚本（纯函数）。
///
/// `focus` = 录制时记录到的首个非空窗口标题（None = 不激活任何窗口）。
pub fn build_replay_script(events: &[MacroEvent], speed: f64, loops: u32, focus: Option<&str>) -> String {
    let speed = if speed.is_finite() && speed > 0.0 {
        speed.clamp(0.1, 10.0)
    } else {
        1.0
    };
    let mut body = String::new();
    let mut prev_t = 0u64;
    for e in events {
        let wait = ((e.t.saturating_sub(prev_t)) as f64 / speed).round() as u64;
        prev_t = e.t;
        if wait > 0 {
            body.push_str(&format!("Start-Sleep -Milliseconds {wait}\n"));
        }
        match e.k.as_str() {
            "move" => {
                if let (Some(x), Some(y)) = (e.x, e.y) {
                    body.push_str(&format!("[MgtMac]::SetCursorPos({x},{y}) | Out-Null\n"));
                }
            }
            "click" => {
                if let (Some(x), Some(y)) = (e.x, e.y) {
                    body.push_str(&format!("[MgtMac]::SetCursorPos({x},{y}) | Out-Null\n"));
                }
                let (down, up) = match e.b.as_deref().unwrap_or("left") {
                    "right" => (0x08, 0x10),
                    "middle" => (0x20, 0x40),
                    _ => (0x02, 0x04),
                };
                let n = if e.d.unwrap_or(0) == 1 { 2 } else { 1 };
                for _ in 0..n {
                    body.push_str(&format!(
                        "[MgtMac]::MouseEvent(0x{down:02X},0,0,0);[MgtMac]::MouseEvent(0x{up:02X},0,0,0)\n"
                    ));
                }
            }
            "scroll" => {
                if let (Some(x), Some(y)) = (e.x, e.y) {
                    body.push_str(&format!("[MgtMac]::SetCursorPos({x},{y}) | Out-Null\n"));
                }
                let v = e.v.unwrap_or(-120) as u32;
                body.push_str(&format!(
                    "[MgtMac]::MouseEvent(0x0800,0,0,{v})\n"
                ));
            }
            "key" => {
                if let Some(vk) = e.vk {
                    let up = if e.down.unwrap_or(1) == 1 { 0 } else { 2 };
                    body.push_str(&format!("[MgtMac]::KeyEvent({vk},{up})\n"));
                }
            }
            _ => {}
        }
    }
    let focus_line = match focus {
        Some(title) if !title.trim().is_empty() => format!(
            "try {{ $wshell = New-Object -ComObject WScript.Shell; [void]$wshell.AppActivate({}) }} catch {{ }}\nStart-Sleep -Milliseconds 250\n",
            ps_quote(title)
        ),
        _ => String::new(),
    };
    format!(
        r#"$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class MgtMac {{
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
  public static void MouseEvent(uint f, uint dx, uint dy, uint d) {{ mouse_event(f, dx, dy, d, UIntPtr.Zero); }}
  public static void KeyEvent(uint vk, uint f) {{ keybd_event((byte)vk, 0, f, UIntPtr.Zero); }}
}}
"@
{focus}
for ($i = 0; $i -lt {loops}; $i++) {{
{body}}}
Write-Output "replayed {count} events x {loops}"
"#,
        focus = focus_line,
        loops = loops.max(1),
        body = body,
        count = events.len()
    )
}

/// 事件摘要（给 `list` / `show` 用）。
pub fn summarize(events: &[MacroEvent]) -> Value {
    let clicks = events.iter().filter(|e| e.k == "click").count();
    let keys = events.iter().filter(|e| e.k == "key").count();
    let moves = events.iter().filter(|e| e.k == "move").count();
    let scrolls = events.iter().filter(|e| e.k == "scroll").count();
    let duration = events.iter().map(|e| e.t).max().unwrap_or(0);
    let window = events
        .iter()
        .find_map(|e| e.w.clone().filter(|w| !w.is_empty()));
    json!({
        "events": events.len(),
        "clicks": clicks,
        "keys": keys,
        "moves": moves,
        "scrolls": scrolls,
        "duration_ms": duration,
        "window": window,
    })
}

fn write_script(path: &Path, content: &str) -> std::io::Result<()> {
    // UTF-8 BOM：PowerShell 5.1 读无 BOM 的中文脚本会按 ANSI 解码 → 窗口标题乱码。
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(content.as_bytes());
    std::fs::write(path, bytes)
}

fn run_script(path: &Path, extra: &[String]) -> Result<(bool, String, String), String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.silent();
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        &path.display().to_string(),
    ]);
    cmd.args(extra);
    let out = cmd.output().map_err(|e| format!("启动 PowerShell 失败: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

// ---------------------------------------------------------------------------
// 录制工具
// ---------------------------------------------------------------------------

pub struct AutoRecordTool;

impl AutoRecordTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AutoRecordTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for AutoRecordTool {
    fn name(&self) -> &str {
        "auto_record"
    }

    fn description(&self) -> &str {
        "录制用户真实键鼠操作成宏（RPA）。action=start（后台开始录制，立即返回）|\
         stop（停止并落盘）|status（录制中？多少事件）|list（已保存的宏）。\
         录制后在回放时按屏幕坐标重放，请保持窗口位置不变。仅 Windows。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["start","stop","status","list"]},
            "name":{"type":"string","description":"宏名（字母/数字/-/_/.，最长 64）"},
            "capture":{"type":"string","enum":["all","mouse","keyboard"],"description":"录制范围（默认 all）"},
            "duration_ms":{"type":"number","description":"最长录制时长（默认 60000，上限 600000；到点自动停）"}
        },"required":["action"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        if !cfg!(target_os = "windows") {
            return Ok(ToolResult::Error {
                message: "auto_record 仅支持 Windows（依赖 Win32 低级键鼠钩子）".to_string(),
                code: "NOT_AVAILABLE".to_string(),
            });
        }
        let action = args.get("action").cloned().unwrap_or_default();
        let dir = macros_dir();
        let _ = std::fs::create_dir_all(&dir);

        match action.as_str() {
            "list" => {
                let mut items = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                            let name = p
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or_default()
                                .to_string();
                            let raw = std::fs::read_to_string(&p).unwrap_or_default();
                            let (ev, bad) = parse_events(&raw);
                            let mut s = summarize(&ev);
                            s["name"] = json!(name);
                            s["bad_lines"] = json!(bad);
                            items.push(s);
                        }
                    }
                }
                Ok(ToolResult::Success {
                    data: json!({ "ok": true, "dir": dir.display().to_string(), "macros": items })
                        .to_string(),
                    metadata: HashMap::new(),
                })
            }
            "start" => {
                let name = match safe_name(args.get("name").map(String::as_str).unwrap_or("")) {
                    Some(n) => n,
                    None => {
                        return Ok(ToolResult::Error {
                            message: "需要合法的 name（字母/数字/-/_/.，最长 64）".to_string(),
                            code: "INVALID_PARAMS".to_string(),
                        })
                    }
                };
                let out = dir.join(format!("{name}.jsonl"));
                let sentinel = dir.join(format!("{name}.stop"));
                let _ = std::fs::remove_file(&out);
                let _ = std::fs::remove_file(&sentinel);
                let duration = args
                    .get("duration_ms")
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(60_000)
                    .clamp(500, 600_000);
                let capture = args
                    .get("capture")
                    .map(|s| s.as_str())
                    .filter(|s| matches!(*s, "all" | "mouse" | "keyboard"))
                    .unwrap_or("all");
                let script_path = std::env::temp_dir()
                    .join(format!("marguerite-rec-{}.ps1", uuid::Uuid::new_v4().simple()));
                let script = build_record_script(&out, &sentinel, duration, capture);
                if let Err(e) = write_script(&script_path, &script) {
                    return Ok(ToolResult::Error {
                        message: format!("写入录制脚本失败: {e}"),
                        code: "IO_ERROR".to_string(),
                    });
                }
                let mut cmd = std::process::Command::new("powershell");
                cmd.silent();
                cmd.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-WindowStyle",
                    "Hidden",
                    "-File",
                    &script_path.display().to_string(),
                ]);
                match cmd.spawn() {
                    Ok(child) => Ok(ToolResult::Success {
                        data: json!({
                            "ok": true,
                            "recording": true,
                            "name": name,
                            "pid": child.id(),
                            "max_ms": duration,
                            "capture": capture,
                            "hint": "现在开始操作；完成后调用 auto_record {action:\"stop\", name} 落盘。",
                        })
                        .to_string(),
                        metadata: HashMap::new(),
                    }),
                    Err(e) => Ok(ToolResult::Error {
                        message: format!("启动录制进程失败: {e}"),
                        code: "SPAWN_FAILED".to_string(),
                    }),
                }
            }
            "stop" => {
                let name = match safe_name(args.get("name").map(String::as_str).unwrap_or("")) {
                    Some(n) => n,
                    None => {
                        return Ok(ToolResult::Error {
                            message: "需要合法的 name".to_string(),
                            code: "INVALID_PARAMS".to_string(),
                        })
                    }
                };
                let sentinel = dir.join(format!("{name}.stop"));
                let out = dir.join(format!("{name}.jsonl"));
                if let Err(e) = std::fs::write(&sentinel, b"stop") {
                    return Ok(ToolResult::Error {
                        message: format!("写入停止标记失败: {e}"),
                        code: "IO_ERROR".to_string(),
                    });
                }
                // 录制进程每 5ms 轮询一次哨兵，停钩子 + 落盘通常 < 300ms；
                // 给 5s 上限是为了覆盖 Add-Type 首次编译较慢的冷启动。
                let mut waited = 0u64;
                while waited < 5_000 && !out.is_file() {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    waited += 100;
                }
                let _ = std::fs::remove_file(&sentinel);
                if !out.is_file() {
                    return Ok(ToolResult::Error {
                        message: format!(
                            "停止后未找到录制结果（{name}）。可能录制进程已超时退出或钩子安装失败。"
                        ),
                        code: "RECORD_NOT_FOUND".to_string(),
                    });
                }
                let raw = std::fs::read_to_string(&out).unwrap_or_default();
                let (ev, bad) = parse_events(&raw);
                let mut s = summarize(&ev);
                s["ok"] = json!(true);
                s["name"] = json!(name);
                s["path"] = json!(out.display().to_string());
                s["bad_lines"] = json!(bad);
                Ok(ToolResult::Success {
                    data: s.to_string(),
                    metadata: HashMap::new(),
                })
            }
            "status" => {
                let name = safe_name(args.get("name").map(String::as_str).unwrap_or(""));
                let mut running = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                            // 有 jsonl 说明至少录过一次；是否在录由"脚本进程还活着"判定，
                            // 跨进程精确判定成本高 → 这里只报文件现状（诚实：不猜）。
                            running.push(p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string());
                        }
                    }
                }
                Ok(ToolResult::Success {
                    data: json!({
                        "ok": true,
                        "dir": dir.display().to_string(),
                        "named": name,
                        "recorded_macros": running,
                        "note": "如需确认是否仍在录制，请调用 stop；stop 会等待落盘并返回事件数。",
                    })
                    .to_string(),
                    metadata: HashMap::new(),
                })
            }
            other => Ok(ToolResult::Error {
                message: format!("未知 action: {other}（可选 start/stop/status/list）"),
                code: "INVALID_PARAMS".to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// 回放工具
// ---------------------------------------------------------------------------

pub struct AutoReplayTool;

impl AutoReplayTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AutoReplayTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for AutoReplayTool {
    fn name(&self) -> &str {
        "auto_replay"
    }

    fn description(&self) -> &str {
        "回放 auto_record 录制的宏。action=run（真实重放）|dry_run（只返回将执行的脚本，不动系统）\
         |show（事件摘要）。speed 变速（0.1–10），loops 循环次数，focus=0 关闭「先激活录制窗口」。仅 Windows。"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["run","dry_run","show"],"description":"默认 run"},
            "name":{"type":"string","description":"宏名"},
            "speed":{"type":"number","description":"速度倍率（默认 1.0；2=两倍速）"},
            "loops":{"type":"number","description":"循环次数（默认 1，上限 50）"},
            "focus":{"type":"number","description":"1=回放前激活录制窗口（默认），0=不激活"}
        },"required":["name"]})
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let action = args
            .get("action")
            .map(|s| s.as_str())
            .unwrap_or("run");
        let name = match safe_name(args.get("name").map(String::as_str).unwrap_or("")) {
            Some(n) => n,
            None => {
                return Ok(ToolResult::Error {
                    message: "需要合法的 name".to_string(),
                    code: "INVALID_PARAMS".to_string(),
                })
            }
        };
        let path = macros_dir().join(format!("{name}.jsonl"));
        if !path.is_file() {
            return Ok(ToolResult::Error {
                message: format!(
                    "宏不存在: {}（先调用 auto_record start/stop 录制）",
                    path.display()
                ),
                code: "MACRO_NOT_FOUND".to_string(),
            });
        }
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let (events, bad) = parse_events(&raw);
        if events.is_empty() {
            return Ok(ToolResult::Error {
                message: format!("宏 {name} 里没有可用事件（坏行 {bad}）"),
                code: "MACRO_EMPTY".to_string(),
            });
        }

        if action == "show" {
            let mut s = summarize(&events);
            s["ok"] = json!(true);
            s["bad_lines"] = json!(bad);
            return Ok(ToolResult::Success {
                data: s.to_string(),
                metadata: HashMap::new(),
            });
        }

        let speed = args
            .get("speed")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.0);
        let loops = args
            .get("loops")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 50);
        let focus_on = args
            .get("focus")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let focus = if focus_on {
            events.iter().find_map(|e| e.w.clone().filter(|w| !w.is_empty()))
        } else {
            None
        };

        let script = build_replay_script(&events, speed, loops, focus.as_deref());

        if action == "dry_run" {
            let head: String = script.chars().take(4000).collect();
            return Ok(ToolResult::Success {
                data: json!({
                    "ok": true,
                    "events": events.len(),
                    "loops": loops,
                    "speed": speed,
                    "focus_window": focus,
                    "script_head": head,
                    "script_bytes": script.len(),
                })
                .to_string(),
                metadata: HashMap::new(),
            });
        }

        if !cfg!(target_os = "windows") {
            return Ok(ToolResult::Error {
                message: "auto_replay 仅支持 Windows".to_string(),
                code: "NOT_AVAILABLE".to_string(),
            });
        }

        let script_path = std::env::temp_dir().join(format!(
            "marguerite-play-{}.ps1",
            uuid::Uuid::new_v4().simple()
        ));
        if let Err(e) = write_script(&script_path, &script) {
            return Ok(ToolResult::Error {
                message: format!("写入回放脚本失败: {e}"),
                code: "IO_ERROR".to_string(),
            });
        }
        let result = tokio::task::spawn_blocking({
            let sp = script_path.clone();
            move || run_script(&sp, &[])
        })
        .await;
        let _ = std::fs::remove_file(&script_path);

        match result {
            Ok(Ok((true, out, _err))) => Ok(ToolResult::Success {
                data: json!({
                    "ok": true,
                    "name": name,
                    "events": events.len(),
                    "loops": loops,
                    "speed": speed,
                    "focus_window": focus,
                    "stdout": out,
                })
                .to_string(),
                metadata: HashMap::new(),
            }),
            Ok(Ok((false, _out, err))) => Ok(ToolResult::Error {
                message: format!("回放失败: {err}"),
                code: "REPLAY_FAILED".to_string(),
            }),
            Ok(Err(e)) => Ok(ToolResult::Error {
                message: e,
                code: "REPLAY_FAILED".to_string(),
            }),
            Err(e) => Ok(ToolResult::Error {
                message: format!("回放任务异常: {e}"),
                code: "REPLAY_FAILED".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<MacroEvent> {
        vec![
            MacroEvent { t: 0, k: "move".into(), x: Some(10), y: Some(20), ..Default::default() },
            MacroEvent {
                t: 500,
                k: "click".into(),
                x: Some(10),
                y: Some(20),
                b: Some("left".into()),
                d: Some(0),
                w: Some("记事本".into()),
                ..Default::default()
            },
            MacroEvent { t: 900, k: "key".into(), vk: Some(65), down: Some(1), ..Default::default() },
            MacroEvent { t: 950, k: "key".into(), vk: Some(65), down: Some(0), ..Default::default() },
            MacroEvent { t: 1200, k: "scroll".into(), x: Some(5), y: Some(6), v: Some(-120), ..Default::default() },
        ]
    }

    #[test]
    fn parse_skips_bad_lines_and_sorts() {
        let raw = "{\"t\":5,\"k\":\"move\",\"x\":1,\"y\":2}\nnot json\n\n{\"t\":1,\"k\":\"click\",\"x\":3,\"y\":4}\n";
        let (ev, bad) = parse_events(raw);
        assert_eq!(ev.len(), 2);
        assert_eq!(bad, 1);
        assert_eq!(ev[0].t, 1);
    }

    #[test]
    fn replay_script_has_timing_and_actions() {
        let s = build_replay_script(&sample(), 2.0, 1, Some("记事本"));
        assert!(s.contains("SetCursorPos(10,20)"));
        assert!(s.contains("MouseEvent(0x02,0,0,0)"));
        assert!(s.contains("MouseEvent(0x0800,0,0,"), "滚轮动作应生成");
        assert!(s.contains("KeyEvent(65,0)"));
        assert!(s.contains("KeyEvent(65,2)"));
        assert!(s.contains("AppActivate('记事本')"));
        // 500ms / 2.0 = 250ms
        assert!(s.contains("Start-Sleep -Milliseconds 250"));
        assert!(s.contains("for ($i = 0; $i -lt 1; $i++)"));
    }

    #[test]
    fn replay_loops_and_no_focus() {
        let s = build_replay_script(&sample(), 1.0, 3, None);
        assert!(s.contains("-lt 3"));
        assert!(!s.contains("AppActivate"));
    }

    #[test]
    fn speed_is_clamped() {
        let s = build_replay_script(&sample(), 0.0, 1, None);
        // 非法速度回落 1.0：500ms 事件仍是 500ms 间隔
        assert!(s.contains("Start-Sleep -Milliseconds 500"));
    }

    #[test]
    fn summarize_counts_by_kind() {
        let s = summarize(&sample());
        assert_eq!(s["events"], 5);
        assert_eq!(s["clicks"], 1);
        assert_eq!(s["keys"], 2);
        assert_eq!(s["moves"], 1);
        assert_eq!(s["scrolls"], 1);
        assert_eq!(s["duration_ms"], 1200);
        assert_eq!(s["window"], "记事本");
    }

    #[test]
    fn record_script_escapes_paths_and_sets_duration() {
        let s = build_record_script(
            Path::new("C:\\a b\\m.jsonl"),
            Path::new("C:\\a b\\m.stop"),
            30_000,
            "mouse",
        );
        assert!(s.contains("'C:\\a b\\m.jsonl'"));
        assert!(s.contains("AddMilliseconds(30000)"));
        assert!(s.contains("WantMouse = true"));
        assert!(s.contains("WantKbd = false"));
    }

    #[test]
    fn safe_name_rejects_traversal() {
        assert!(safe_name("../x").is_none());
        assert!(safe_name("a/b").is_none());
        assert!(safe_name("").is_none());
        assert_eq!(safe_name("my-macro_1.v2"), Some("my-macro_1.v2".to_string()));
    }
}
