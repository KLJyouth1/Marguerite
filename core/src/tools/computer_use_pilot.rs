//! 桌面操控增强：对齐 `momasiku/dsh-pilot` 与 `CursorTouch/Windows-MCP` 的关键机制。
//!
//! 两个上游都是 MIT；这里**不复制**它们的源码，而是把公开文档与脚本里可验证的做法
//! 用 Rust + PowerShell 重写成本项目的一等能力：
//!
//! | 机制 | 上游出处 | 本项目此前状态 | 本模块做法 |
//! |---|---|---|---|
//! | 每显示器 DPI 感知（PerMonitorV2 → PerMonitor → System 三级回退） | dsh-pilot `_dsh-win32.ps1` | 缺失：截图与 `SetCursorPos` 各按自己的坐标空间解释，缩放屏上必偏 | [`DPI_PRELUDE`] 注入每个生成的 `.ps1` |
//! | 一次调用跑完整串动作 | dsh-pilot `desktop_sequence` / Windows-MCP `MultiEdit` | 每步一次工具往返 + 一次 PowerShell 冷启动 | [`sequence_ps1`] 把多步合成一个脚本 |
//! | 键位按 VK 序列注入 | Windows-MCP `Shortcut` | Windows 侧把 `ctrl+c` 直接喂给 `SendKeys`（SendKeys 语法是 `^c`）→ 静默失效 | [`parse_chord`] / [`chord_ps`] |
//! | 真实窗口枚举 / 强制前台 / 移动缩放 | Windows-MCP `App`；dsh-pilot `focus` | 只有 `AppActivate(标题)`，被前台锁拒绝时静默无效 | [`window_list_ps`] / [`force_foreground_ps`] / [`window_rect_ps`] |
//!
//! 诚实的边界：本模块**没有**实现上游的常驻 PowerShell worker（动作 6–12ms）。
//! 每批动作仍会有一次 PowerShell 冷启动（约 0.3–0.8s，含 `Add-Type`）。

use anyhow::{anyhow, Result};
use serde::Deserialize;

/// DPI 感知前置：三级回退，任一成功即止。
///
/// 必须在脚本**最前面**执行：GDI（`CopyFromScreen`）、`GetSystemMetrics`、
/// UI Automation 的边界矩形都会按进程的 DPI 感知级别换算，一旦晚于调用就无效。
/// 上游 dsh-pilot 用同一套回退顺序（`-4` = `PER_MONITOR_AWARE_V2`）。
pub const DPI_PRELUDE: &str = r#"$ErrorActionPreference = 'Continue'
try {
  $null = Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v); [DllImport("shcore.dll")] public static extern int SetProcessDpiAwareness(int v); [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();' -Name Dpi -Namespace Marg -PassThru
  $ok = $false
  try { $ok = [Marg.Dpi]::SetProcessDpiAwarenessContext([IntPtr](-4)) } catch { $ok = $false }
  if (-not $ok) { try { $ok = ([Marg.Dpi]::SetProcessDpiAwareness(2) -eq 0) } catch { $ok = $false } }
  if (-not $ok) { try { $ok = [Marg.Dpi]::SetProcessDPIAware() } catch { $ok = $false } }
} catch {}
$Error.Clear()
"#;

/// Win32 桥（一次 `Add-Type` 编译，供批量动作 / 窗口动作复用）。
///
/// 与既有单动作脚本里的 `-MemberDefinition` 小类型并存：单动作保持轻量，
/// 批量与窗口动作才付这次编译成本。
pub const WIN32_CLASS: &str = r#"if (-not ('MargUi' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class MargUi {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int data, IntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, IntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);

    public static void Key(int vk, bool up) { keybd_event((byte)vk, 0, up ? 2u : 0u, IntPtr.Zero); }

    public static string Title(IntPtr h) {
        var sb = new StringBuilder(512);
        GetWindowTextW(h, sb, sb.Capacity);
        return sb.ToString();
    }

    public static int[] Rect(IntPtr h) {
        RECT r;
        if (!GetWindowRect(h, out r)) return new int[] { 0, 0, 0, 0 };
        return new int[] { r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top };
    }

    // Foreground is externally arbitrated: another process can take it at any time.
    // Try directly first, then AttachThreadInput to the current foreground thread
    // (the documented workaround), and finally read back to confirm.
    public static bool ForceForeground(IntPtr h) {
        if (GetForegroundWindow() == h) return true;
        if (IsIconic(h)) ShowWindow(h, 9);
        if (!IsWindowVisible(h)) ShowWindow(h, 5);
        IntPtr fg = GetForegroundWindow();
        uint pid = 0;
        uint fgThread = GetWindowThreadProcessId(fg, out pid);
        uint myThread = GetWindowThreadProcessId(h, out pid);
        bool attached = false;
        if (fgThread != 0 && fgThread != myThread) {
            attached = AttachThreadInput(fgThread, myThread, true);
            if (!attached) attached = AttachThreadInput(myThread, fgThread, true);
        }
        SetForegroundWindow(h);
        BringWindowToTop(h);
        if (attached) {
            AttachThreadInput(fgThread, myThread, false);
            AttachThreadInput(myThread, fgThread, false);
        }
        return GetForegroundWindow() == h;
    }

    public static void MoveResize(IntPtr h, int x, int y, int w, int hgt) {
        // SWP_NOZORDER(0x0004) | SWP_SHOWWINDOW(0x0040)
        SetWindowPos(h, IntPtr.Zero, x, y, w, hgt, 0x0044);
    }
}
'@
}
"#;

/// 批量动作里的一步。
///
/// `action` 取值与单动作同名（`click` / `double_click` / `right_click` / `middle_click`
/// / `move` / `drag` / `scroll` / `type` / `key` / `key_combo` / `wait` / `focus`），
/// 另外 `wait` 用 `ms`，`focus` 用 `title`。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Step {
    pub action: String,
    /// 指针类动作的目标点。用 `Option` 而非 `0` 默认值：`(0,0)` 是合法坐标，
    /// 只有「没给」才是错误——否则漏填坐标会静默点到屏幕左上角。
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub x2: Option<i64>,
    pub y2: Option<i64>,
    pub dx: i64,
    pub dy: i64,
    pub text: String,
    pub key: String,
    pub title: String,
    /// 该步之后的等待（毫秒）。`wait` 动作使用同一字段。
    pub settle_ms: u64,
    pub ms: u64,
}

fn x_of(s: &Step) -> i64 {
    s.x.unwrap_or(0)
}
fn y_of(s: &Step) -> i64 {
    s.y.unwrap_or(0)
}
fn x2_of(s: &Step) -> i64 {
    s.x2.unwrap_or(0)
}
fn y2_of(s: &Step) -> i64 {
    s.y2.unwrap_or(0)
}

/// 解析 `steps` 的 JSON 数组。空数组视为错误（避免「成功执行 0 步」的假完成）。
pub fn parse_steps(raw: &str) -> Result<Vec<Step>> {
    let steps: Vec<Step> = serde_json::from_str(raw)
        .map_err(|e| anyhow!("steps 不是合法的动作数组（JSON）: {e}"))?;
    if steps.is_empty() {
        return Err(anyhow!("steps 为空：批量动作至少要有一项"));
    }
    if steps.len() > 40 {
        return Err(anyhow!("steps 超过 40 步上限（当前 {}）", steps.len()));
    }
    Ok(steps)
}

/// 校验每步的必填参数。宁可在这里响亮失败，也不要在桌面上执行半串动作。
pub fn validate(steps: &[Step]) -> Result<()> {
    for (i, s) in steps.iter().enumerate() {
        let n = i + 1;
        let need_xy = |s: &Step, n: usize| -> Result<()> {
            if s.x.is_none() || s.y.is_none() {
                return Err(anyhow!("第 {n} 步 {} 缺少 x/y（屏幕物理像素）", s.action));
            }
            Ok(())
        };
        match s.action.as_str() {
            "click" | "double_click" | "right_click" | "middle_click" | "move" => {
                need_xy(s, n)?;
            }
            "drag" => {
                need_xy(s, n)?;
                if s.x2.is_none() || s.y2.is_none() {
                    return Err(anyhow!("第 {n} 步 drag 缺少 x2/y2（拖拽终点）"));
                }
            }
            "scroll" => {
                need_xy(s, n)?;
                if s.dx == 0 && s.dy == 0 {
                    return Err(anyhow!("第 {n} 步 scroll 的 dx/dy 不能同时为 0"));
                }
            }
            "type" => {
                if s.text.is_empty() {
                    return Err(anyhow!("第 {n} 步 type 缺少 text"));
                }
            }
            "key" | "key_combo" => {
                if s.key.trim().is_empty() {
                    return Err(anyhow!("第 {n} 步 {} 缺少 key", s.action));
                }
            }
            "focus" => {
                if s.title.trim().is_empty() {
                    return Err(anyhow!("第 {n} 步 focus 缺少 title"));
                }
            }
            "wait" => {}
            other => {
                return Err(anyhow!(
                    "第 {n} 步动作 {other} 不在批量白名单内（允许：click/double_click/right_click/middle_click/move/drag/scroll/type/key/key_combo/wait/focus）"
                ))
            }
        }
    }
    Ok(())
}

/// 批次风险等级：模型声明偏低时，按「危险快捷键」自动升级。
///
/// 依据：`win+r`（运行框）、`alt+f4`（关窗）、`ctrl+shift+esc`（任务管理器）等
/// 一旦误触代价高；批量动作又是「一次跑完不看中间结果」，所以不给确认就不动手。
pub fn risk_of(steps: &[Step], declared: &str) -> &'static str {
    let mut risk = match declared.trim().to_ascii_lowercase().as_str() {
        "high" => 3,
        "medium" => 2,
        _ => 1,
    };
    for s in steps {
        if matches!(s.action.as_str(), "key" | "key_combo") {
            let k = s.key.to_ascii_lowercase().replace(' ', "");
            let dangerous = (k.contains("win") && k.contains('r'))
                || k.contains("alt+f4")
                || k.contains("ctrl+shift+esc")
                || k.contains("ctrl+alt+del")
                || k.contains("shift+delete");
            if dangerous {
                risk = risk.max(2);
            }
        }
    }
    match risk {
        3 => "high",
        2 => "medium",
        _ => "low",
    }
}

/// `dry_run` 时返回的计划文本：把要做的每一件事故障前置地列出来。
pub fn plan_text(steps: &[Step]) -> String {
    let mut out = format!("计划执行 {} 步（dry_run，未触碰桌面）：\n", steps.len());
    for (i, s) in steps.iter().enumerate() {
        out.push_str(&format!("  {}. {}", i + 1, describe_step(s)));
        out.push('\n');
    }
    out
}

fn describe_step(s: &Step) -> String {
    match s.action.as_str() {
        "click" | "double_click" | "right_click" | "middle_click" | "move" => {
            format!("{} ({},{})", s.action, x_of(s), y_of(s))
        }
        "drag" => format!(
            "drag ({},{}) → ({},{})",
            x_of(s),
            y_of(s),
            x2_of(s),
            y2_of(s)
        ),
        "scroll" => format!(
            "scroll ({},{}) dx={} dy={}",
            x_of(s),
            y_of(s),
            s.dx,
            s.dy
        ),
        "type" => format!("type 「{}」", clip(&s.text, 40)),
        "key" | "key_combo" => format!("{} {}", s.action, s.key),
        "wait" => format!("wait {}ms", s.ms.max(s.settle_ms)),
        "focus" => format!("focus 「{}」", clip(&s.title, 40)),
        other => other.to_string(),
    }
}

fn clip(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// 键位 token → VK 码。
///
/// 只覆盖可确定的按键；未知 token **报错**而不是交给 `SendKeys` 猜——
/// 猜错的后果是「按键没生效但工具回报成功」。
pub fn vk_for(token: &str) -> Option<u16> {
    let t = token.trim().to_ascii_lowercase();
    let vk = match t.as_str() {
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" | "menu_key" => 0x12,
        "win" | "meta" | "cmd" | "super" => 0x5B,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "esc" | "escape" => 0x1B,
        "space" => 0x20,
        "backspace" | "bs" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "capslock" => 0x14,
        "printscreen" | "prtsc" => 0x2C,
        "contextmenu" | "apps" => 0x5D,
        other => {
            if let Some(num) = other.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                if (1..=24).contains(&num) {
                    return Some(0x70 + (num as u16 - 1));
                }
            }
            let mut it = other.chars();
            let c = it.next()?;
            if it.next().is_some() {
                return None;
            }
            if c.is_ascii_alphabetic() {
                return Some(c.to_ascii_uppercase() as u16);
            }
            if c.is_ascii_digit() {
                return Some(c as u16);
            }
            return punctuation_vk(c);
        }
    };
    Some(vk)
}

fn punctuation_vk(c: char) -> Option<u16> {
    Some(match c {
        ';' => 0xBA,
        '=' => 0xBB,
        ',' => 0xBC,
        '-' => 0xBD,
        '.' => 0xBE,
        '/' => 0xBF,
        '`' => 0xC0,
        '[' => 0xDB,
        '\\' => 0xDC,
        ']' => 0xDD,
        '\'' => 0xDE,
        _ => return None,
    })
}

/// 解析组合键（`ctrl+shift+s` / `alt+tab` / `win` / `enter`）为 VK 序列：
/// 除最后一个外都按住，最后一个点按。
pub fn parse_chord(chord: &str) -> Result<Vec<u16>> {
    let raw = chord.trim();
    if raw.is_empty() {
        return Err(anyhow!("组合键为空"));
    }
    // SendKeys 专有语法（`^c` / `%{F4}` / `{ENTER}`）保持原样透传，由调用方判定。
    let tokens: Vec<&str> = raw.split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
    if tokens.is_empty() {
        return Err(anyhow!("组合键 {raw} 解析为空"));
    }
    let mut out = Vec::with_capacity(tokens.len());
    for t in &tokens {
        match vk_for(t) {
            Some(vk) => out.push(vk),
            None => {
                return Err(anyhow!(
                    "无法识别的按键 `{t}`（支持 ctrl/shift/alt/win、a-z、0-9、f1-f24、enter/tab/esc/space/backspace/delete/home/end/pageup/pagedown/up/down/left/right 与常见符号；SendKeys 语法如 `^c`、`%{{F4}}` 可直接传）"
                ))
            }
        }
    }
    Ok(out)
}

/// 判断是否是 SendKeys 专有语法（应以原样交给 SendKeys）。
pub fn is_sendkeys_syntax(s: &str) -> bool {
    let t = s.trim();
    t.starts_with('^') || t.starts_with('%') || t.starts_with('+') || t.starts_with('~')
        || t.contains('{')
}

/// 组合键 → PowerShell 片段（按住前面的键，点按最后一个）。
pub fn chord_ps(chord: &str) -> Result<String> {
    let keys = parse_chord(chord)?;
    if keys.len() == 1 {
        return Ok(format!("[MargUi]::Key({0},$true); [MargUi]::Key({0},$false)", keys[0]));
    }
    let mut s = String::new();
    for vk in &keys[..keys.len() - 1] {
        s.push_str(&format!("[MargUi]::Key({vk},$true); "));
    }
    let last = keys[keys.len() - 1];
    s.push_str(&format!(
        "Start-Sleep -Milliseconds 15; [MargUi]::Key({last},$true); [MargUi]::Key({last},$false); "
    ));
    for vk in keys[..keys.len() - 1].iter().rev() {
        s.push_str(&format!("[MargUi]::Key({vk},$false); "));
    }
    Ok(s)
}

/// 中文/特殊字符文本 → `SendKeys` 片段（与既有 `type` 行为一致）。
fn sendkeys_ps(text: &str) -> String {
    let escaped = text.replace('\'', "''");
    format!("[System.Windows.Forms.SendKeys]::SendWait('{escaped}')")
}

/// 按标题/进程名定位窗口的 PowerShell 片段，结果写入 `$hwnd`。
pub fn resolve_window_ps(title: &str) -> String {
    let t = title.trim().replace('\'', "''");
    format!(
        "$hwnd=[IntPtr]::Zero; \
$proc=Get-Process | Where-Object {{ $_.MainWindowHandle -ne 0 -and ($_.MainWindowTitle -like '*{t}*' -or $_.ProcessName -like '*{t}*') }} | Select-Object -First 1; \
if ($proc) {{ $hwnd=$proc.MainWindowHandle }}; \
if ($hwnd -eq [IntPtr]::Zero) {{ throw '找不到窗口: {t}' }}"
    )
}

/// 强制前台（带回读校验）片段。
pub fn force_foreground_ps(title: &str) -> String {
    format!(
        "{}; if (-not [MargUi]::ForceForeground($hwnd)) {{ throw '系统拒绝把窗口切到前台: {t}' }}",
        resolve_window_ps(title),
        t = title.trim()
    )
}

/// 窗口列表：可见且带标题的顶层窗口（hwnd / pid / 是否前台 / 是否最小化 / 矩形 / 标题）。
pub fn window_list_ps() -> String {
    r#"
$fg=[MargUi]::GetForegroundWindow()
$rows=@()
foreach ($p in Get-Process) {
  try {
    $h = $p.MainWindowHandle
    if ($h -eq [IntPtr]::Zero) { continue }
    if (-not [MargUi]::IsWindowVisible($h)) { continue }
    $r = [MargUi]::Rect($h)
    $min = if ([MargUi]::IsIconic($h)) { 1 } else { 0 }
    $isFg = if ($h -eq $fg) { 1 } else { 0 }
    $rows += ('hwnd=' + $h.ToInt64() + "`tpid=" + $p.Id + "`tfg=" + $isFg + "`tmin=" + $min + "`tproc=" + $p.ProcessName + "`trect=" + $r[0] + ',' + $r[1] + ',' + $r[2] + ',' + $r[3] + "`ttitle=" + $p.MainWindowTitle)
  } catch {}
}
if ($rows.Count -eq 0) { Write-Output 'NO_VISIBLE_WINDOW' } else { $rows | Sort-Object { $_ -match 'fg=1' } -Descending }
"#
    .to_string()
}

/// 窗口移动/缩放片段（`w`/`h` 为 0 表示不改该维）。
pub fn window_rect_ps(title: &str, x: i64, y: i64, w: i64, h: i64) -> String {
    let mut lines = vec![resolve_window_ps(title)];
    let (cx, cy, cw, ch) = ("$r[0]", "$r[1]", "$r[2]", "$r[3]");
    lines.push("$r=[MargUi]::Rect($hwnd)".to_string());
    let nx = if x >= 0 { x.to_string() } else { cx.to_string() };
    let ny = if y >= 0 { y.to_string() } else { cy.to_string() };
    let nw = if w > 0 { w.to_string() } else { cw.to_string() };
    let nh = if h > 0 { h.to_string() } else { ch.to_string() };
    lines.push(format!("[MargUi]::MoveResize($hwnd,{nx},{ny},{nw},{nh})"));
    lines.push(
        "$r2=[MargUi]::Rect($hwnd); Write-Output ('rect=' + $r2[0] + ',' + $r2[1] + ',' + $r2[2] + ',' + $r2[3])"
            .to_string(),
    );
    lines.join("; ")
}

/// 启动应用：优先按可执行路径，其次按开始菜单显示名（AppID 走 `shell:AppsFolder`）。
///
/// 与 Windows-MCP 的 `App` 同思路：不要求英文系统，也不假设用户知道 exe 路径。
pub fn app_launch_ps(app: &str, argv: &[String], cwd: &str) -> String {
    let a = app.trim();
    let looks_like_path = a.contains('\\') || a.contains('/') || a.to_ascii_lowercase().ends_with(".exe");
    if looks_like_path {
        let esc = a.replace('\'', "''");
        let args = if argv.is_empty() {
            String::new()
        } else {
            let joined = argv
                .iter()
                .map(|v| format!("'{}'", v.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            format!(" -ArgumentList @({joined})")
        };
        let wd = if cwd.trim().is_empty() {
            String::new()
        } else {
            format!(" -WorkingDirectory '{}'", cwd.trim().replace('\'', "''"))
        };
        return format!("$p=Start-Process -FilePath '{esc}'{args}{wd} -PassThru; Write-Output ('started=' + $p.ProcessName)");
    }
    let esc = a.replace('\'', "''");
    format!(
        "$hit=Get-StartApps | Where-Object {{ $_.Name -like '*{esc}*' }} | Select-Object -First 1; \
if (-not $hit) {{ throw '开始菜单里找不到应用: {esc}' }}; \
Start-Process ('shell:AppsFolder\\' + $hit.AppID); \
Write-Output ('started=' + $hit.Name + ' appid=' + $hit.AppID)"
    )
}

/// 单步 → PowerShell 片段（批量脚本与单动作共用同一套注入语义）。
fn step_ps(step: &Step) -> Result<String> {
    let settle = step.settle_ms;
    let settle_ps = if settle > 0 {
        format!("; Start-Sleep -Milliseconds {settle}")
    } else {
        String::new()
    };
    let body = match step.action.as_str() {
        "click" => format!(
            "[MargUi]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 25; [MargUi]::mouse_event(0x0002,0,0,0,[IntPtr]::Zero); [MargUi]::mouse_event(0x0004,0,0,0,[IntPtr]::Zero)",
            x = x_of(step),
            y = y_of(step)
        ),
        "double_click" | "double" => format!(
            "[MargUi]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 25; [MargUi]::mouse_event(0x0002,0,0,0,[IntPtr]::Zero); [MargUi]::mouse_event(0x0004,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 60; [MargUi]::mouse_event(0x0002,0,0,0,[IntPtr]::Zero); [MargUi]::mouse_event(0x0004,0,0,0,[IntPtr]::Zero)",
            x = x_of(step),
            y = y_of(step)
        ),
        "right_click" => format!(
            "[MargUi]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 25; [MargUi]::mouse_event(0x0008,0,0,0,[IntPtr]::Zero); [MargUi]::mouse_event(0x0010,0,0,0,[IntPtr]::Zero)",
            x = x_of(step),
            y = y_of(step)
        ),
        "middle_click" => format!(
            "[MargUi]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 25; [MargUi]::mouse_event(0x0020,0,0,0,[IntPtr]::Zero); [MargUi]::mouse_event(0x0040,0,0,0,[IntPtr]::Zero)",
            x = x_of(step),
            y = y_of(step)
        ),
        "move" => format!(
            "[MargUi]::SetCursorPos({x},{y})",
            x = x_of(step),
            y = y_of(step)
        ),
        "drag" => format!(
            "[MargUi]::SetCursorPos({x1},{y1}); Start-Sleep -Milliseconds 25; [MargUi]::mouse_event(0x0002,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 40; [MargUi]::SetCursorPos({x2},{y2}); Start-Sleep -Milliseconds 40; [MargUi]::mouse_event(0x0004,0,0,0,[IntPtr]::Zero)",
            x1 = x_of(step),
            y1 = y_of(step),
            x2 = x2_of(step),
            y2 = y2_of(step)
        ),
        "scroll" => {
            let mut s = format!("[MargUi]::SetCursorPos({},{})", x_of(step), y_of(step));
            if step.dy != 0 {
                s.push_str(&format!(
                    "; [MargUi]::mouse_event(0x0800,0,0,{},[IntPtr]::Zero)",
                    -(step.dy * 120)
                ));
            }
            if step.dx != 0 {
                s.push_str(&format!(
                    "; [MargUi]::mouse_event(0x1000,0,0,{},[IntPtr]::Zero)",
                    step.dx * 120
                ));
            }
            s
        }
        "type" => sendkeys_ps(&step.text),
        "key" | "key_combo" => {
            if is_sendkeys_syntax(&step.key) {
                sendkeys_ps(&step.key)
            } else {
                chord_ps(&step.key)?
            }
        }
        "wait" => format!("Start-Sleep -Milliseconds {}", step.ms.max(settle).max(1)),
        "focus" => force_foreground_ps(&step.title),
        other => return Err(anyhow!("不支持的批内动作: {other}")),
    };
    Ok(format!("{body}{settle_ps}"))
}

/// 批量动作 → 单个 PowerShell 脚本。
///
/// `stop_on_error` 为真时首错即停（并 `exit 2`）；为假时记录失败继续跑。
/// 每步输出 `STEP n ok` / `STEP n failed: <reason>`，让调用方看到**哪一步**失败，
/// 而不是只有「整串失败」。
pub fn sequence_ps1(steps: &[Step], stop_on_error: bool) -> Result<String> {
    validate(steps)?;
    let mut out = String::new();
    out.push_str(DPI_PRELUDE);
    out.push_str(WIN32_CLASS);
    let needs_sendkeys = steps.iter().any(|s| {
        s.action == "type"
            || (matches!(s.action.as_str(), "key" | "key_combo") && is_sendkeys_syntax(&s.key))
    });
    if needs_sendkeys {
        out.push_str("Add-Type -AssemblyName System.Windows.Forms\n");
    }
    out.push_str("$ErrorActionPreference='Stop'\n");
    for (i, s) in steps.iter().enumerate() {
        let n = i + 1;
        let snippet = step_ps(s)?;
        if stop_on_error {
            out.push_str(&format!(
                "try {{ {snippet}; Write-Output 'STEP {n} ok' }} catch {{ Write-Output ('STEP {n} failed: ' + $_.Exception.Message); exit 2 }}\n"
            ));
        } else {
            out.push_str(&format!(
                "try {{ {snippet}; Write-Output 'STEP {n} ok' }} catch {{ Write-Output ('STEP {n} failed: ' + $_.Exception.Message) }}\n"
            ));
        }
    }
    out.push_str(&format!(
        "Write-Output 'SEQUENCE DONE {}'\n",
        steps.len()
    ));
    Ok(out)
}

/// 截图帧指纹（sha256 前 16 个十六进制字符）：同会话重复帧可据此免于再次投喂。
pub fn frame_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let hex = format!("{:x}", h.finalize());
    hex.chars().take(16).collect()
}

/// 坐标契约：只有「交付尺寸 == 采集尺寸」时才允许模型直接照用读到的像素。
///
/// 上游 dsh-pilot 把这条写进信封（`delivered_scale`），因为宿主常把图压小，
/// 而模型仍按图里的像素点——差一点点就是点错按钮。
pub fn scale_contract(capture: (u32, u32), delivered: (u32, u32), origin: (i64, i64)) -> String {
    let (cw, ch) = capture;
    let (dw, dh) = delivered;
    if cw == 0 || ch == 0 || dw == 0 || dh == 0 {
        return "capture=unknown delivered=unknown scale=1（无法确认尺寸，坐标仅供参考）".to_string();
    }
    if dw == cw && dh == ch {
        return format!(
            "capture={cw}x{ch} delivered={dw}x{dh} scale=1（图内读数即屏幕像素，原点 {},{})",
            origin.0, origin.1
        );
    }
    let sx = cw as f64 / dw as f64;
    let sy = ch as f64 / dh as f64;
    format!(
        "capture={cw}x{ch} delivered={dw}x{dh} scale={:.3}（图内读数需乘 {:.3}x/{:.3}y 才是屏幕像素；原点 {},{})",
        dw as f64 / cw as f64,
        sx,
        sy,
        origin.0,
        origin.1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_prelude_sets_per_monitor_v2_first() {
        assert!(DPI_PRELUDE.contains("SetProcessDpiAwarenessContext"));
        assert!(DPI_PRELUDE.contains("[IntPtr](-4)"), "-4 = PER_MONITOR_AWARE_V2");
        // 三级回退顺序：V2 → PerMonitor → System（按**调用点**判定，
        // 类型声明里同样出现这些名字，不能拿裸名字比位置）
        let v2 = DPI_PRELUDE.find("[Marg.Dpi]::SetProcessDpiAwarenessContext").unwrap();
        let sh = DPI_PRELUDE.find("[Marg.Dpi]::SetProcessDpiAwareness(2)").unwrap();
        let old = DPI_PRELUDE.find("[Marg.Dpi]::SetProcessDPIAware()").unwrap();
        assert!(v2 < sh && sh < old, "回退顺序必须是 V2 → PerMonitor → System");
    }

    #[test]
    fn chord_parses_modifiers_in_order() {
        assert_eq!(parse_chord("ctrl+c").unwrap(), vec![0x11, 0x43]);
        assert_eq!(parse_chord("ctrl+shift+s").unwrap(), vec![0x11, 0x10, 0x53]);
        assert_eq!(parse_chord("alt+tab").unwrap(), vec![0x12, 0x09]);
        assert_eq!(parse_chord("enter").unwrap(), vec![0x0D]);
        assert_eq!(parse_chord("f5").unwrap(), vec![0x74]);
        assert_eq!(parse_chord("win").unwrap(), vec![0x5B]);
    }

    #[test]
    fn chord_rejects_unknown_key_loudly() {
        let err = parse_chord("ctrl+notakey").unwrap_err().to_string();
        assert!(err.contains("无法识别的按键"), "err={err}");
        assert!(parse_chord("").is_err());
    }

    #[test]
    fn chord_ps_presses_and_releases_in_order() {
        let ps = chord_ps("ctrl+c").unwrap();
        let down_ctrl = ps.find("[MargUi]::Key(17,$true)").unwrap();
        let down_c = ps.find("[MargUi]::Key(67,$true)").unwrap();
        let up_c = ps.find("[MargUi]::Key(67,$false)").unwrap();
        let up_ctrl = ps.find("[MargUi]::Key(17,$false)").unwrap();
        assert!(down_ctrl < down_c && down_c < up_c && up_c < up_ctrl, "ps={ps}");
    }

    #[test]
    fn sendkeys_syntax_is_detected() {
        assert!(is_sendkeys_syntax("^c"));
        assert!(is_sendkeys_syntax("%{F4}"));
        assert!(!is_sendkeys_syntax("ctrl+c"));
        assert!(!is_sendkeys_syntax("enter"));
    }

    #[test]
    fn steps_reject_missing_coordinates() {
        let steps = parse_steps(r#"[{"action":"click"}]"#).unwrap();
        let err = validate(&steps).unwrap_err().to_string();
        assert!(err.contains("缺少 x/y"), "err={err}");
        // (0,0) 是合法坐标，不能因为默认值 0 就误判为「漏填」或「越界」。
        let origin = parse_steps(r#"[{"action":"click","x":0,"y":0}]"#).unwrap();
        assert!(validate(&origin).is_ok());
    }

    #[test]
    fn steps_reject_empty_and_unknown_action() {
        assert!(parse_steps("[]").is_err(), "空批次必须拒绝");
        let steps = parse_steps(r#"[{"action":"screenshot"}]"#).unwrap();
        let err = validate(&steps).unwrap_err().to_string();
        assert!(err.contains("不在批量白名单内"), "err={err}");
    }

    #[test]
    fn risk_escalates_for_dangerous_shortcuts() {
        let steps = parse_steps(r#"[{"action":"key_combo","key":"win+r"}]"#).unwrap();
        assert_eq!(risk_of(&steps, "low"), "medium");
        let safe = parse_steps(r#"[{"action":"click","x":1,"y":2}]"#).unwrap();
        assert_eq!(risk_of(&safe, "low"), "low");
        assert_eq!(risk_of(&safe, "high"), "high", "模型声明的更高风险必须保留");
    }

    #[test]
    fn plan_text_lists_every_step_without_touching_desktop() {
        let steps = parse_steps(
            r#"[{"action":"click","x":10,"y":20},{"action":"type","text":"hello"},{"action":"wait","ms":300}]"#,
        )
        .unwrap();
        let plan = plan_text(&steps);
        assert!(plan.contains("3 步"), "plan={plan}");
        assert!(plan.contains("click (10,20)"), "plan={plan}");
        assert!(plan.contains("type 「hello」"), "plan={plan}");
        assert!(plan.contains("wait 300ms"), "plan={plan}");
    }

    #[test]
    fn sequence_script_has_dpi_class_and_step_markers() {
        let steps = parse_steps(
            r#"[{"action":"move","x":5,"y":6},{"action":"key_combo","key":"ctrl+s"}]"#,
        )
        .unwrap();
        let ps = sequence_ps1(&steps, true).unwrap();
        assert!(ps.contains("SetProcessDpiAwarenessContext"), "必须带 DPI 前置");
        assert!(ps.contains("class MargUi"), "必须带 Win32 桥");
        assert!(ps.contains("STEP 1 ok") && ps.contains("STEP 2 ok"), "ps={ps}");
        assert!(ps.contains("exit 2"), "stop_on_error=true 时应首错即停");
        assert!(ps.contains("SEQUENCE DONE 2"));
    }

    #[test]
    fn sequence_without_stop_on_error_continues() {
        let steps = parse_steps(r#"[{"action":"click","x":1,"y":1}]"#).unwrap();
        let ps = sequence_ps1(&steps, false).unwrap();
        assert!(!ps.contains("exit 2"), "stop_on_error=false 不应退出");
    }

    #[test]
    fn sequence_loads_sendkeys_only_when_needed() {
        let click_only = parse_steps(r#"[{"action":"click","x":1,"y":1}]"#).unwrap();
        assert!(!sequence_ps1(&click_only, true).unwrap().contains("System.Windows.Forms"));
        let typing = parse_steps(r#"[{"action":"type","text":"hi"}]"#).unwrap();
        assert!(sequence_ps1(&typing, true)
            .unwrap()
            .contains("Add-Type -AssemblyName System.Windows.Forms"));
    }

    #[test]
    fn frame_hash_is_stable_and_short() {
        let a = frame_hash(b"same-bytes");
        assert_eq!(a, frame_hash(b"same-bytes"));
        assert_eq!(a.len(), 16);
        assert_ne!(a, frame_hash(b"other-bytes"));
    }

    #[test]
    fn scale_contract_promises_direct_pixels_only_when_equal() {
        let same = scale_contract((2560, 1600), (2560, 1600), (0, 0));
        assert!(same.contains("scale=1"), "same={same}");
        assert!(same.contains("图内读数即屏幕像素"), "same={same}");

        let scaled = scale_contract((2560, 1600), (1974, 873), (-1920, 0));
        assert!(scaled.contains("需乘"), "scaled={scaled}");
        assert!(scaled.contains("origin") || scaled.contains("原点 -1920,0"), "scaled={scaled}");
    }

    #[test]
    fn window_rect_ps_keeps_unspecified_dimensions() {
        let ps = window_rect_ps("记事本", -1, -1, 800, 600);
        assert!(ps.contains("MoveResize"), "ps={ps}");
        assert!(ps.contains("$r[0]"), "未指定 x 时应沿用当前值: {ps}");
        assert!(ps.contains("800") && ps.contains("600"), "ps={ps}");
    }

    #[test]
    fn app_launch_prefers_exe_path_and_quotes_args() {
        let ps = app_launch_ps(r"C:\Windows\notepad.exe", &["a b".to_string()], r"C:\tmp");
        assert!(ps.contains("Start-Process -FilePath"), "ps={ps}");
        assert!(ps.contains("'a b'"), "ps={ps}");
        assert!(ps.contains("WorkingDirectory"), "ps={ps}");

        let menu = app_launch_ps("记事本", &[], "");
        assert!(menu.contains("Get-StartApps"), "ps={menu}");
        assert!(menu.contains("shell:AppsFolder"), "ps={menu}");
    }

    #[test]
    fn window_list_reads_real_windows() {
        let ps = window_list_ps();
        assert!(ps.contains("MainWindowHandle"));
        assert!(ps.contains("IsWindowVisible"));
        assert!(ps.contains("NO_VISIBLE_WINDOW"));
    }
}
