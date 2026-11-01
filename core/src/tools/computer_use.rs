//! 计算机操控工具（三层架构）：屏幕感知 / 键鼠控制 / GUI 定位
//!
//! 融合自 10 个 computer use 类社区仓库（见 `harness::distill::UPSTREAM_REPOS` 与本仓库
//! `third_party/computer_use/` 下已克隆的参考实现）：
//!
//! | 层 | 职责 | 主参考仓库 |
//! |----|------|-----------|
//! | `ScreenPerception` | 屏幕截图采集（主屏 / 多屏 / 活动窗口 + VLMM 降采样） | OpenCUA / computer_use_ootb / ScreenSpot-Pro / Ros_Qt5_Gui_App |
//! | `InputControl`     | 鼠标键盘控制（点击 / 双击 / 拖拽 / 滚轮 / 组合键） | robotgo（跨平台键鼠） / OpenCUA / computer_use_ootb |
//! | `GuiGrounding`     | 元素定位与视觉 grounding（坐标归一化 + 候选置信度） | ScreenSpot-Pro-GUI-Grounding / OpenCUA（VLMM） / Guided-Attention-Inference-Network（注意力推断辅助） |
//!
//! 本模块给出三层抽象与基于操作系统原生命令的可用实现；视觉 grounding 层依赖
//! 视觉语言模型（VLMM），由调用方（agent 运行时 + `vision::VisionCapability`）在
//! 「截图 → VLMM 输出坐标 → `InputControl` 执行」闭环中完成，本工具负责感知与执行两端。
//!
//! 诚实性：原生输入在 Linux（xdotool）/ macOS（cliclick/osascript）路径直接可用；
//! Windows 路径生成临时 `.ps1` 后以 **argv 直连** spawn `powershell`（见 [`NativeCmd::Ps1`]）
//! ——不能再把 `powershell -File "…"` 拼成命令行交给 `sandboxed_exec`：Windows 侧走 `cmd /C`，
//! Rust 的 `\"` 转义 cmd 不认，`-File` 会收到带字面双引号的路径而直接失败
//! （实测 `Processing -File '"C:\…\marg_cu_*.ps1"' failed: Illegal characters in path.`）。
//! 所有动作的退出码都被如实检查：非零退出即 `ToolResult::Error`，绝不把「没生效」报成成功。
//! 多屏坐标采用 Windows 虚拟桌面坐标系，与全屏截图一致。

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::agent::tool_cycle::ToolResult;
use crate::desktop_automation::driver::{AppKind, DesktopError, UiAction};
use crate::desktop_automation::policy::Policy;
use crate::desktop_automation::registry::AppRegistry;
use crate::mcp::registry::ToolRegistry;
use crate::security::{sandboxed_exec, CommandGuard, DEFAULT_TIMEOUT};
use crate::tools::builtin::make_local_tool;
use crate::vision::image_metadata;
use crate::vision::{CoordSpace, VisionCapability};

/// 桌面操控增强（DPI 感知 / 批量动作 / 窗口与应用操控 / VK 键位）。
#[path = "computer_use_pilot.rs"]
mod pilot;

/// 工具闭包类型（与 `tools::builtin::ToolFn` 透明等价）
type ToolFn = std::sync::Arc<
    dyn for<'a> Fn(
            &'a HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + 'a>>
        + Send
        + Sync,
>;

/// 一条待执行的原生命令（跨平台命令构造与执行方式的统一载体）
///
/// Windows 的 PowerShell 路径**不能**拼成命令行交给 `sandboxed_exec`：Windows 侧走 `cmd /C`，
/// Rust 的 `\"` 转义 cmd 不认，`powershell -File` 会收到带字面双引号的路径
/// （实测 `Processing -File '"C:\…\marg_cu_*.ps1"' failed: Illegal characters in path.`），
/// 脚本根本没跑起来。故 `.ps1` 路径以 argv 直连 spawn `powershell`，
/// 与仓库既有正确范式 `tools::builtin::win_auto::run_powershell` 一致。
#[derive(Debug, Clone)]
pub enum NativeCmd {
    /// 经 `sandboxed_exec` 执行的命令行（POSIX `sh -c` / Windows `cmd /C`）
    Shell(String),
    /// Windows：argv 直连执行已写好的 `.ps1`（`sandboxed_exec` 无法传达的路径）
    Ps1(PathBuf),
}

/// 把子进程结果统一转成 `ToolResult`。
///
/// 诚实性（D2）：**非零退出即 `Error`**——动作没生效却回报「成功」会让 agent 与用户同时被
/// 误导（历史上 `locate` 就因此把「.ps1 根本没跑起来」显示成「读取截图失败: 系统找不到指定的文件」）。
fn finish(stdout: &str, stderr: &str, exit_code: i32) -> ToolResult {
    let mut body = stdout.to_string();
    if !stderr.trim().is_empty() {
        body.push_str("\n[stderr]\n");
        body.push_str(stderr);
    }
    let body = body.trim().to_string();
    if exit_code == 0 {
        ToolResult::Success {
            data: body,
            metadata: HashMap::new(),
        }
    } else {
        ToolResult::Error {
            message: format!("命令以非零退出码 {exit_code} 结束\n{body}"),
            code: "NONZERO_EXIT".to_string(),
        }
    }
}

/// 截图采集模式：主屏 / 全部显示器（虚拟桌面） / 当前活动窗口
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    Primary,
    All,
    Window,
}

impl CaptureMode {
    fn from_str(s: &str) -> CaptureMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" | "multi" | "virtual" => CaptureMode::All,
            "window" | "active" | "focus" => CaptureMode::Window,
            _ => CaptureMode::Primary,
        }
    }
}

/// 三层架构之一：屏幕感知（截图采集）
pub trait ScreenPerception {
    /// 返回采集当前屏幕的命令（由 [`OsComputerUse::run_cmd`] 执行）。
    /// `mode` 控制主屏/多屏/活动窗口；`max_edge` 为缩边上限（>0 时按长边等比缩到该像素，
    /// 用于降低发给 VLMM 的截图体积，对齐 OpenCUA/computer_use_ootb 的降采样策略）。
    fn capture_cmd(&self, out_path: &str, mode: CaptureMode, max_edge: u32) -> Result<NativeCmd>;
}

/// 三层架构之一：键鼠控制（输入执行）
pub trait InputControl {
    /// 鼠标点击（移动到 x,y 后按下/抬起左键）
    fn click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd>;
    /// 鼠标双击
    fn double_click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd>;
    /// 鼠标右键点击
    fn right_click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd>;
    /// 鼠标中键点击
    fn middle_click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd>;
    /// 鼠标拖拽（从 x1,y1 到 x2,y2）
    fn drag_cmd(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<NativeCmd>;
    /// 鼠标滚轮（dx 横向、dy 纵向，单位=滚动格）
    fn scroll_cmd(&self, x: i64, y: i64, dx: i64, dy: i64) -> Result<NativeCmd>;
    /// 移动鼠标到 x,y（不点击）
    fn move_cmd(&self, x: i64, y: i64) -> Result<NativeCmd>;
    /// 键盘输入文本
    fn type_cmd(&self, text: &str) -> Result<NativeCmd>;
    /// 键盘按键
    fn key_cmd(&self, key: &str) -> Result<NativeCmd>;
    /// 键盘组合键（如 "ctrl+c"，按 SendKeys/cliclick 约定）
    fn key_combo_cmd(&self, keys: &str) -> Result<NativeCmd>;
    /// 剪贴板：写入文本
    fn clipboard_set_cmd(&self, text: &str) -> Result<NativeCmd>;
    /// 剪贴板：读取文本
    fn clipboard_get_cmd(&self) -> Result<NativeCmd>;
    /// 窗口聚焦（按窗口标题匹配，激活到前台）
    fn window_focus_cmd(&self, title: &str) -> Result<NativeCmd>;
    /// 获取当前鼠标位置（输出 "x,y"）
    fn cursor_pos_cmd(&self) -> Result<NativeCmd>;
    /// 查询按键状态（输出 "pressed"/"released"）
    fn key_state_cmd(&self, key: &str) -> Result<NativeCmd>;
    /// 获取系统空闲时间（毫秒）
    fn idle_time_cmd(&self) -> Result<NativeCmd>;
}

/// 三层架构之一：GUI 定位（视觉 grounding）
///
/// 真实 grounding 由 VLMM 完成；本层负责把「描述/坐标」落到可执行动作，
/// 并返回供上层 VLMM 消费的截图路径。
pub trait GuiGrounding {
    /// 基于描述定位：返回截图路径，由调用方交给 VLMM 产出坐标
    fn locate_by_description(&self, out_path: &str) -> String;
    /// 基于已知坐标定位（已由上层的 VLMM/脚本给出）
    fn locate_by_coords(&self, x: i64, y: i64) -> String;
}

/// 基于操作系统原生命令的计算机操控实现
pub struct OsComputerUse {
    workspace: PathBuf,
}

impl OsComputerUse {
    /// 创建实现
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }

    /// 生成一个临时截图路径
    fn screenshot_path(&self) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("marg_cu_{stamp}.png"))
    }

    /// 生成 Windows 临时 .ps1 并写入内容，返回脚本路径
    ///
    /// P2-8 中文乱码修复：PowerShell 5.1 重定向 stdout 时用控制台代码页
    /// （中文系统 = GBK/936），而 Rust 侧按 UTF-8 解码 → 中文必乱码。
    /// 统一在脚本头部强制 `[Console]::OutputEncoding = UTF-8`，使子进程
    /// stdout 字节流恒为 UTF-8；同时以 UTF-8 写入 .ps1（含中文命令文本时必需）。
    ///
    /// 诚实性（D2 补强）：脚本尾部追加「有错误记录即 exit 1」。实测 PowerShell 的 `-File`
    /// 对**语句级** .NET 异常（如 `SendKeys` 收到非法按键串）仍返回退出码 0——若只看退出码，
    /// 「动作没生效」会被报成成功。终止性错误（`throw`）PowerShell 自身已退出 1，故无需额外处理：
    /// 两类错误由此全覆盖。
    fn write_ps1(&self, content: &str) -> Result<PathBuf> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("marg_cu_{stamp}.ps1"));
        let prologue = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n$OutputEncoding = [System.Text.Encoding]::UTF8\n{}",
            pilot::DPI_PRELUDE
        );
        let trailer = "if ($Error.Count -gt 0) { exit 1 }\n";
        // 必须带 UTF-8 BOM：Windows PowerShell 5.1 对**无 BOM** 的 .ps1 按 ANSI 代码页解码，
        // 脚本里的中文（注释也会）会变成乱码字节；实测把中文注释写进 Add-Type 的 C# 源码后，
        // 紧随其后的方法签名被解析成非法语句（`if (IsIconic(h))` 报「应输入标识符」）。
        std::fs::write(&p, format!("\u{feff}{prologue}{content}\n{trailer}"))
            .map_err(|e| anyhow!("写入 ps1 失败: {e}"))?;
        Ok(p)
    }

    /// 写入 `.ps1` 并包装为 argv 直连命令（Windows 专用执行路径）
    fn ps1_cmd(&self, content: &str) -> Result<NativeCmd> {
        Ok(NativeCmd::Ps1(self.write_ps1(content)?))
    }

    /// argv 直连执行 `.ps1`（不经 shell，路径无需转义），返回（stdout, stderr, 退出码）
    ///
    /// 与 `sandboxed_exec` 对齐的安全属性：工作目录限定工作区、`CREATE_NO_WINDOW` 静默执行、
    /// 超时强杀（`kill_on_drop`）、stdin 置空。
    async fn spawn_ps1(&self, path: &Path) -> Result<(String, String, i32)> {
        let mut command = tokio::process::Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(path)
            .current_dir(&self.workspace)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = command
            .spawn()
            .map_err(|e| anyhow!("failed to spawn `powershell`: {e}"))?;
        match tokio::time::timeout(DEFAULT_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(out)) => Ok((
                String::from_utf8_lossy(&out.stdout).to_string(),
                String::from_utf8_lossy(&out.stderr).to_string(),
                out.status.code().unwrap_or(-1),
            )),
            Ok(Err(e)) => Err(anyhow!("command execution error: {e}")),
            Err(_) => Err(anyhow!(
                "command timed out after {:?}: {}",
                DEFAULT_TIMEOUT,
                path.display()
            )),
        }
    }

    /// 执行一条命令（`Shell` 走沙箱 shell，`Ps1` 走 argv 直连）并如实转成 `ToolResult`
    async fn run_cmd(&self, cmd: NativeCmd) -> Result<ToolResult> {
        let guard = CommandGuard::new();
        match cmd {
            NativeCmd::Shell(c) => {
                let out = sandboxed_exec(&c, &self.workspace, DEFAULT_TIMEOUT, &guard).await?;
                Ok(finish(&out.stdout, &out.stderr, out.exit_code))
            }
            NativeCmd::Ps1(path) => {
                // 护栏沿用既有语义：校验「等价的 PowerShell 调用命令行」。
                //
                // 刻意**不**改为校验脚本全文：脚本里的 `text`/`key` 是「要被输入到第三方应用的
                // 文本数据」，而非 shell 命令——computer_use 的核心用途之一恰恰是往终端里打字，
                // 拿 shell 命令模式去匹配这些数据会误伤正常输入（例如输入 `sudo make install`）。
                // 真正的高危 shell 命令仍由 `NativeCmd::Shell` 路径经 `sandboxed_exec` 拦截，
                // 与改动前的检查对象逐字一致，语义未削弱。
                guard.check(&format!(
                    "powershell -ExecutionPolicy Bypass -File {}",
                    path.display()
                ))?;
                let (stdout, stderr, exit_code) = self.spawn_ps1(&path).await?;
                Ok(finish(&stdout, &stderr, exit_code))
            }
        }
    }

    /// 执行一个动作并只回报「是否真正生效」（非零退出即 false）
    async fn run_ok(&self, cmd: NativeCmd) -> Result<bool> {
        Ok(self.run_cmd(cmd).await?.is_success())
    }

    /// locate 的自动点击闭环：移动 + 点击两步都真正生效才算成功
    async fn auto_click(&self, x: i64, y: i64) -> Result<bool> {
        let moved = self.run_ok(self.move_cmd(x, y)?).await?;
        let clicked = self.run_ok(self.click_cmd(x, y)?).await?;
        Ok(moved && clicked)
    }

    /// 构造截图命令（跨平台；多屏/活动窗口/降采样由 `mode`/`max_edge` 控制）
    ///
    /// `rulers`：在图上画坐标刻度（上沿/左沿每 200px，标注**屏幕坐标**）。
    /// 这是 dsh-pilot 的「刻度尺」做法：模型直接读刻度，不用数像素，也不怕图被压小。
    fn build_capture_cmd(
        &self,
        path: &str,
        mode: CaptureMode,
        max_edge: u32,
        rulers: bool,
    ) -> Result<NativeCmd> {
        if cfg!(windows) {
            // PowerShell 内容写入 .ps1 后 argv 直连执行（不经 cmd，路径无需引号转义）
            let bounds = match mode {
                // 进程已在脚本前置里声明 PerMonitorV2 → `Screen.Bounds` 本身就是物理像素。
                // 旧实现再乘一次 `DpiX/96` 会在 150%/200% 缩放下把图放大一倍（点必偏）。
                CaptureMode::Primary => "$s=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $w=[int]$s.Width; $h=[int]$s.Height; $ox=[int]$s.X; $oy=[int]$s.Y;".to_string(),
                // 虚拟屏幕设备像素（与 SetCursorPos 同一坐标系）；P/Invoke GetSystemMetrics(SM_X/Y/CX/CYVIRTUALSCREEN=76/77/78/79)
                CaptureMode::All => "$sm=Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern int GetSystemMetrics(int n);' -Name SM -Namespace W32 -PassThru; $ox=[W32.SM]::GetSystemMetrics(76); $oy=[W32.SM]::GetSystemMetrics(77); $w=[W32.SM]::GetSystemMetrics(78); $h=[W32.SM]::GetSystemMetrics(79);".to_string(),
                // 前台窗口设备像素矩形（GetWindowRect 本就返回设备像素）
                CaptureMode::Window => "$sig=Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); [DllImport(\"user32.dll\")] public static extern bool GetWindowRect(IntPtr h, ref RECT r); [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }' -Name W32 -Namespace W -PassThru; $hnd=[W.W32]::GetForegroundWindow(); $rc=New-Object W.W32+RECT; [W.W32]::GetWindowRect($hnd,[ref]$rc) | Out-Null; $w=$rc.Right-$rc.Left; $h=$rc.Bottom-$rc.Top; $ox=$rc.Left; $oy=$rc.Top;".to_string(),
            };
            let save = if max_edge > 0 {
                format!(
                    "if ($b.Width -gt {me} -or $b.Height -gt {me}) {{ $r=[double]{me}/[double][math]::Max($b.Width,$b.Height); $nw=[int]($b.Width*$r); $nh=[int]($b.Height*$r); $nb=New-Object System.Drawing.Bitmap($nw,$nh); $ng=[System.Drawing.Graphics]::FromImage($nb); $ng.InterpolationMode=[System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic; $ng.DrawImage($b,0,0,$nw,$nh); $b.Dispose(); $g.Dispose(); $nb.Save('{p}'); }} else {{ $b.Save('{p}'); }}",
                    me = max_edge, p = path
                )
            } else {
                format!("$b.Save('{p}');", p = path)
            };
            let ruler_ps = if rulers {
                // 刻度失败绝不影响截图本身：整段包在 try/catch 里。
                format!(
                    "try {{ $pen=New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(210,255,48,48),1); $fnt=New-Object System.Drawing.Font('Consolas',10); $br=[System.Drawing.Brushes]::OrangeRed; for($i=0;$i -lt $b.Width;$i+=200){{ $g.DrawLine($pen,$i,0,$i,10); $g.DrawString(([string]($i+{ox})),$fnt,$br,($i+2),10) }}; for($j=0;$j -lt $b.Height;$j+=200){{ $g.DrawLine($pen,0,$j,10,$j); $g.DrawString(([string]($j+{oy})),$fnt,$br,2,($j+2)) }}; $pen.Dispose(); $fnt.Dispose() }} catch {{}}",
                    ox = "$ox", oy = "$oy"
                )
            } else {
                String::new()
            };
            let content = format!(
                "Add-Type -AssemblyName System.Windows.Forms\nAdd-Type -AssemblyName System.Drawing\n{bounds}\n$b=New-Object System.Drawing.Bitmap($w,$h)\n$g=[System.Drawing.Graphics]::FromImage($b)\n$g.CopyFromScreen($ox,$oy,0,0,$b.Size)\n{rulers}\n{save}\n\"$w $h $ox $oy\" | Set-Content -Path '{p}.dim' -Encoding ascii\n",
                bounds = bounds, rulers = ruler_ps, save = save, p = path
            );
            self.ps1_cmd(&content)
        } else if cfg!(target_os = "macos") {
            let base = match mode {
                CaptureMode::Window => "screencapture -m".to_string(), // macOS 无可靠前台窗口非交互截图，退化为全屏
                CaptureMode::All => "screencapture -m".to_string(),
                CaptureMode::Primary => "screencapture".to_string(),
            };
            let mut cmd = format!("{base} {path}");
            if max_edge > 0 {
                cmd = format!(
                    "{cmd} && sips -Z {me} '{path}' >/dev/null 2>&1",
                    me = max_edge
                );
            }
            Ok(NativeCmd::Shell(cmd))
        } else {
            let base = match mode {
                CaptureMode::Window => "import -window $(xdotool getactivewindow)".to_string(),
                _ => "scrot".to_string(), // X11 下 scrot 默认抓取整个视口（含多屏）
            };
            let mut cmd = format!("{base} {path} || gnome-screenshot -f {path}");
            if max_edge > 0 {
                cmd = format!(
                    "{cmd} && (command -v convert >/dev/null && convert '{path}' -resize '{me}>' '{path}' || true)",
                    me = max_edge
                );
            }
            Ok(NativeCmd::Shell(cmd))
        }
    }

    /// 构造输入命令（跨平台）。`action` 可为 click/double/right_click/middle_click/drag/scroll/move/type/key/combo/clipboard_set/clipboard_get/window_focus。
    #[allow(clippy::too_many_arguments)] // 输入动作参数天然较多，集中传参优于再包一层
    fn build_input_cmd(
        &self,
        action: &str,
        x: i64,
        y: i64,
        x2: i64,
        y2: i64,
        dx: i64,
        dy: i64,
        text: &str,
        key: &str,
    ) -> Result<NativeCmd> {
        if cfg!(windows) {
            // 统一经 user32（SetCursorPos + mouse_event）与 SendKeys，写入 .ps1 执行
            let sig = "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool SetCursorPos(int X,int Y); [DllImport(\"user32.dll\")] public static extern void mouse_event(int f,int a,int b,int d,int e); [DllImport(\"user32.dll\")] public static extern void keybd_event(byte b,int a,int f,int e);' -Name MU -Namespace W32 -PassThru";
            let mut needs_win32 = false;
            let body = match action {
                "click" => format!(
                    "[W32.MU]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 30; [W32.MU]::mouse_event(0x0002,0,0,0,0); [W32.MU]::mouse_event(0x0004,0,0,0,0)"
                ),
                "double" => format!(
                    "[W32.MU]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 30; [W32.MU]::mouse_event(0x0002,0,0,0,0); [W32.MU]::mouse_event(0x0004,0,0,0,0); Start-Sleep -Milliseconds 60; [W32.MU]::mouse_event(0x0002,0,0,0,0); [W32.MU]::mouse_event(0x0004,0,0,0,0)"
                ),
                "right_click" => format!(
                    "[W32.MU]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 30; [W32.MU]::mouse_event(0x0008,0,0,0,0); [W32.MU]::mouse_event(0x0010,0,0,0,0)"
                ),
                "middle_click" => format!(
                    "[W32.MU]::SetCursorPos({x},{y}); Start-Sleep -Milliseconds 30; [W32.MU]::mouse_event(0x0020,0,0,0,0); [W32.MU]::mouse_event(0x0040,0,0,0,0)"
                ),
                "drag" => format!(
                    "[W32.MU]::SetCursorPos({x1},{y1}); Start-Sleep -Milliseconds 30; [W32.MU]::mouse_event(0x0002,0,0,0,0); [W32.MU]::SetCursorPos({x2},{y2}); [W32.MU]::mouse_event(0x0004,0,0,0,0)",
                    x1 = x, y1 = y, x2 = x2, y2 = y2
                ),
                "move" => format!("[W32.MU]::SetCursorPos({x},{y})"),
                "scroll" => {
                    // 纵向 dy>0 向下（wheel 负向），横向 dx 用 HWHEEL
                    let v = if dy != 0 {
                        let dw = -(dy * 120);
                        format!("[W32.MU]::mouse_event(0x0800,0,0,{dw},0)", dw = dw)
                    } else {
                        String::new()
                    };
                    let h = if dx != 0 {
                        let dw = dx * 120;
                        format!("[W32.MU]::mouse_event(0x01000,0,0,{dw},0)", dw = dw)
                    } else {
                        String::new()
                    };
                    // 先移动到 (x,y) 再滚动（参考 robotgo/computer_use_ootb）
                    format!("[W32.MU]::SetCursorPos({x},{y}); {v} {h}", x = x, y = y, v = v, h = h)
                }
                "type" => {
                    // SendKeys 用单引号包裹；转义内部单引号
                    let escaped = text.replace('\'', "''");
                    format!(
                        "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{e}')",
                        e = escaped
                    )
                }
                // key / combo：按 VK 序列注入。
                //
                // 旧实现把 `key` 的值喂给 `text`（恒为空）→ `key` 动作静默无效；
                // 且把文档里的 `ctrl+c` 直接交给 SendKeys（SendKeys 语法是 `^c`）→ 组合键也不生效。
                // 现在：SendKeys 专有语法原样透传，其余一律解析成 VK 按住/点按序列。
                "key" | "combo" => {
                    let t = key.trim();
                    if t.is_empty() {
                        return Err(anyhow!("key/key_combo 需要非空的 key 参数"));
                    }
                    if pilot::is_sendkeys_syntax(t) {
                        let escaped = t.replace('\'', "''");
                        format!(
                            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{e}')",
                            e = escaped
                        )
                    } else {
                        needs_win32 = true;
                        pilot::chord_ps(t)?
                    }
                }
                "clipboard_set" => {
                    let escaped = text.replace('\'', "''");
                    format!(
                        "Set-Clipboard -Value '{e}'",
                        e = escaped
                    )
                }
                "clipboard_get" => "Get-Clipboard".to_string(),
                "window_focus" => {
                    // 旧实现走 `AppActivate(标题)`：被前台锁拒绝时它可能既不抛错也不生效，
                    // 于是「窗口没切过去」被报成成功。改为 hwnd + ForceForeground + 回读校验。
                    needs_win32 = true;
                    pilot::force_foreground_ps(key)
                }
                _ => return Err(anyhow!("未知输入动作: {action}")),
            };
            // 只有需要窗口/VK 能力的动作才付 `MargUi` 的编译成本；纯键鼠仍走轻量小类型。
            let prelude = if needs_win32 {
                pilot::WIN32_CLASS.to_string()
            } else {
                sig.to_string()
            };
            let content = format!("{prelude}\n{body}\n", prelude = prelude, body = body);
            self.ps1_cmd(&content)
        } else if cfg!(target_os = "macos") {
            let cmd = match action {
                "click" => format!("cliclick c:{x},{y}"),
                "double" => format!("cliclick dc:{x},{y}"),
                "right_click" => format!("cliclick rc:{x},{y}"),
                "middle_click" => format!("cliclick m:{x},{y}"),
                "drag" => format!(
                    "cliclick dd:{x1},{y1}:{x2},{y2}",
                    x1 = x,
                    y1 = y,
                    x2 = x2,
                    y2 = y2
                ),
                "move" => format!("cliclick m:{x},{y}"),
                "scroll" => format!("cliclick w:{dy}"),
                "type" => format!(
                    "osascript -e 'tell application \"System Events\" to keystroke \"{t}\"'",
                    t = text
                ),
                "key" => format!(
                    "osascript -e 'tell application \"System Events\" to key code \"{k}\"'",
                    k = key
                ),
                "combo" => format!(
                    "osascript -e 'tell application \"System Events\" to keystroke \"{k}\"'",
                    k = key
                ),
                "clipboard_set" => format!("printf '%s' '{t}' | pbcopy", t = text),
                "clipboard_get" => "pbpaste".to_string(),
                "window_focus" => format!(
                    "osascript -e 'tell application \"{k}\" to activate'",
                    k = key
                ),
                _ => return Err(anyhow!("未知输入动作: {action}")),
            };
            Ok(NativeCmd::Shell(cmd))
        } else {
            let cmd = match action {
                "click" => format!("xdotool mousemove {x} {y} click 1"),
                "double" => format!("xdotool mousemove {x} {y} click --repeat 2 1"),
                "right_click" => format!("xdotool mousemove {x} {y} click 3"),
                "middle_click" => format!("xdotool mousemove {x} {y} click 2"),
                "drag" => format!(
                    "xdotool mousemove {x1} {y1} mousedown 1 mousemove {x2} {y2} mouseup 1",
                    x1 = x, y1 = y, x2 = x2, y2 = y2
                ),
                "move" => format!("xdotool mousemove {x} {y}"),
                "scroll" => {
                    // 纵向优先；正 dy=向下(button 5)，负=向上(4)；横向 dx 用 6/7
                    if dy != 0 {
                        let btn = if dy > 0 { 5 } else { 4 };
                        format!("xdotool mousemove {x} {y} click {btn}")
                    } else if dx != 0 {
                        let btn = if dx > 0 { 7 } else { 6 };
                        format!("xdotool mousemove {x} {y} click {btn}")
                    } else {
                        String::new()
                    }
                }
                "type" => format!("xdotool type -- '{t}'", t = text),
                "key" => format!("xdotool key '{k}'", k = key),
                "combo" => format!("xdotool key '{k}'", k = key),
                "clipboard_set" => format!("printf '%s' '{t}' | xclip -selection clipboard", t = text),
                "clipboard_get" => "xclip -selection clipboard -o".to_string(),
                "window_focus" => format!("xdotool search --name '{k}' windowactivate --sync 2>/dev/null || wmctrl -a '{k}'", k = key),
                _ => return Err(anyhow!("未知输入动作: {action}")),
            };
            Ok(NativeCmd::Shell(cmd))
        }
    }

    /// 构造无障碍树 / 窗口枚举命令（辅助感知通道，对齐 awesome-computer-use / computer-vision-guide）
    fn perceive_cmd(&self) -> Result<NativeCmd> {
        if cfg!(windows) {
            let content = "Add-Type -AssemblyName UIAutomationClient; Add-Type -AssemblyName UIAutomationTypes; \
$tries=0; \
$root=[System.Windows.Automation.AutomationElement]::RootElement; \
$cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Window); \
$root.FindAll([System.Windows.Automation.TreeScope]::Subtree,$cond) | \
  ForEach-Object { ($_.Current.Name+' | '+($_.Current.ControlType.ProgrammaticName -replace 'ControlType.','')) } | \
  Where-Object { $_.Length -gt 0 } | Select-Object -First 60";
            self.ps1_cmd(content)
        } else if cfg!(target_os = "macos") {
            Ok(NativeCmd::Shell("osascript -e 'tell application \"System Events\" to get name of every process whose visible is true'".to_string()))
        } else {
            // 可见窗口名列表（辅助定位，避免单纯依赖像素 grounding）
            Ok(NativeCmd::Shell("xdotool search --onlyvisible --name '.*' 2>/dev/null | while read id; do xdotool getwindowname \"$id\" 2>/dev/null; done | head -60".to_string()))
        }
    }

    /// 获取当前鼠标位置
    fn cursor_pos_cmd(&self) -> Result<NativeCmd> {
        if cfg!(windows) {
            let content = "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool GetCursorPos(out POINT lpPoint); [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }' -Name CUR -Namespace W32; $p=New-Object W32.CUR+POINT; [W32.CUR]::GetCursorPos([ref]$p) | Out-Null; Write-Output \"$($p.X),$($p.Y)\"";
            self.ps1_cmd(content)
        } else if cfg!(target_os = "macos") {
            Ok(NativeCmd::Shell("cliclick p".to_string()))
        } else {
            Ok(NativeCmd::Shell("xdotool getmouselocation --shell 2>/dev/null | grep -E 'X=|Y=' | tr '\\n' ',' | sed 's/,$/\\n/'".to_string()))
        }
    }

    /// 查询按键状态
    fn key_state_cmd(&self, key: &str) -> Result<NativeCmd> {
        if cfg!(windows) {
            // VK 码映射（常用键）
            let vk = match key.to_lowercase().as_str() {
                "shift" => "0x10",
                "control" | "ctrl" => "0x11",
                "alt" | "menu" => "0x12",
                "capslock" => "0x14",
                "lbutton" | "leftmouse" => "0x01",
                "rbutton" | "rightmouse" => "0x02",
                "mbutton" | "middlemouse" => "0x04",
                "space" => "0x20",
                "return" | "enter" => "0x0D",
                "escape" | "esc" => "0x1B",
                _ => "0x00",
            };
            let content = format!(
                "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern short GetAsyncKeyState(int vKey);' -Name KS -Namespace W32; $s=[W32.KS]::GetAsyncKeyState({vk}); if ($s -band 0x8000) {{ Write-Output 'pressed' }} else {{ Write-Output 'released' }}"
            );
            self.ps1_cmd(&content)
        } else if cfg!(target_os = "macos") {
            Ok(NativeCmd::Shell(format!(
                "osascript -e 'tell application \"System Events\" to return key code of {key}'"
            )))
        } else {
            Ok(NativeCmd::Shell(
                "xset q | grep -i 'LED mask' | grep -q '00000002' && echo pressed || echo released"
                    .to_string(),
            ))
        }
    }

    /// 获取系统空闲时间（毫秒）
    fn idle_time_cmd(&self) -> Result<NativeCmd> {
        if cfg!(windows) {
            let content = "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool GetLastInputInfo(ref LASTINPUTINFO plii); [StructLayout(LayoutKind.Sequential)] public struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }' -Name IDLE -Namespace W32; $lii=New-Object W32.IDLE+LASTINPUTINFO; $lii.cbSize=[uint32][System.Runtime.InteropServices.Marshal]::SizeOf($lii); [W32.IDLE]::GetLastInputInfo([ref]$lii) | Out-Null; $idle=([Environment]::TickCount - $lii.dwTime); Write-Output $idle";
            self.ps1_cmd(content)
        } else if cfg!(target_os = "macos") {
            Ok(NativeCmd::Shell("echo $(( $(python3 -c 'import CoreFoundation; print(int(CoreFoundation.CGEventSourceSecondsSinceLastEventType(1, 0x05)*1000))' 2>/dev/null || echo 0) ))".to_string()))
        } else {
            Ok(NativeCmd::Shell("xprintidle 2>/dev/null || echo 0".to_string()))
        }
    }
}

impl ScreenPerception for OsComputerUse {
    fn capture_cmd(&self, out_path: &str, mode: CaptureMode, max_edge: u32) -> Result<NativeCmd> {
        self.build_capture_cmd(out_path, mode, max_edge, false)
    }
}

impl InputControl for OsComputerUse {
    fn click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd> {
        self.build_input_cmd("click", x, y, 0, 0, 0, 0, "", "")
    }
    fn double_click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd> {
        self.build_input_cmd("double", x, y, 0, 0, 0, 0, "", "")
    }
    fn right_click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd> {
        self.build_input_cmd("right_click", x, y, 0, 0, 0, 0, "", "")
    }
    fn middle_click_cmd(&self, x: i64, y: i64) -> Result<NativeCmd> {
        self.build_input_cmd("middle_click", x, y, 0, 0, 0, 0, "", "")
    }
    fn drag_cmd(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<NativeCmd> {
        self.build_input_cmd("drag", x1, y1, x2, y2, 0, 0, "", "")
    }
    fn scroll_cmd(&self, x: i64, y: i64, dx: i64, dy: i64) -> Result<NativeCmd> {
        self.build_input_cmd("scroll", x, y, 0, 0, dx, dy, "", "")
    }
    fn move_cmd(&self, x: i64, y: i64) -> Result<NativeCmd> {
        self.build_input_cmd("move", x, y, 0, 0, 0, 0, "", "")
    }
    fn type_cmd(&self, text: &str) -> Result<NativeCmd> {
        self.build_input_cmd("type", 0, 0, 0, 0, 0, 0, text, "")
    }
    fn key_cmd(&self, key: &str) -> Result<NativeCmd> {
        self.build_input_cmd("key", 0, 0, 0, 0, 0, 0, "", key)
    }
    fn key_combo_cmd(&self, keys: &str) -> Result<NativeCmd> {
        self.build_input_cmd("combo", 0, 0, 0, 0, 0, 0, "", keys)
    }
    fn clipboard_set_cmd(&self, text: &str) -> Result<NativeCmd> {
        self.build_input_cmd("clipboard_set", 0, 0, 0, 0, 0, 0, text, "")
    }
    fn clipboard_get_cmd(&self) -> Result<NativeCmd> {
        self.build_input_cmd("clipboard_get", 0, 0, 0, 0, 0, 0, "", "")
    }
    fn window_focus_cmd(&self, title: &str) -> Result<NativeCmd> {
        self.build_input_cmd("window_focus", 0, 0, 0, 0, 0, 0, "", title)
    }
    fn cursor_pos_cmd(&self) -> Result<NativeCmd> {
        OsComputerUse::cursor_pos_cmd(self)
    }
    fn key_state_cmd(&self, key: &str) -> Result<NativeCmd> {
        OsComputerUse::key_state_cmd(self, key)
    }
    fn idle_time_cmd(&self) -> Result<NativeCmd> {
        OsComputerUse::idle_time_cmd(self)
    }
}

impl GuiGrounding for OsComputerUse {
    fn locate_by_description(&self, out_path: &str) -> String {
        out_path.to_string()
    }
    fn locate_by_coords(&self, x: i64, y: i64) -> String {
        format!("x={x},y={y}")
    }
}

/// 将视觉模型输出的坐标映射到真实屏幕「设备像素」坐标（与 `SetCursorPos` / `CopyFromScreen` 同一坐标系）。
///
/// 两步变换，缺一不可：
/// 1. **归一化还原**：模型返回归一化坐标(<=1)时，先 × 截图像素得到「截图空间像素」。
/// 2. **降采样反推（DPI / scale 修复核心）**：截图常经 `max_edge` 降采样以减小 VLMM 输入体积，
///    模型看到的是降采样后的像素；真实点击必须落在「降采样前」的真实屏幕设备像素上，
///    故按 `real/shot` 比例反推。当未降采样（real == shot）时比例为 1，行为退化为原实现。
///
/// 这是 C3（computer-use 硬缺陷：DPI / 降采样坐标错位）的修复核心；纯函数便于单测。
pub fn map_locate_coords(
    model_x: f64,
    model_y: f64,
    space: CoordSpace,
    shot_w: f64,
    shot_h: f64,
    real_w: f64,
    real_h: f64,
) -> (f64, f64) {
    let (px, py) = space.to_pixels(model_x, model_y, shot_w, shot_h);
    let sx = if shot_w > 0.0 { real_w / shot_w } else { 1.0 };
    let sy = if shot_h > 0.0 { real_h / shot_h } else { 1.0 };
    (px * sx, py * sy)
}

/// grounding 结果元数据（网关据此透出 `grounding` 字段；字段名与 [`GuiGrounding`] 既有返回一致）。
fn grounding_metadata(
    x: f64,
    y: f64,
    confidence: f64,
    description: &str,
) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("grounding.x".to_string(), format!("{x:.1}"));
    m.insert("grounding.y".to_string(), format!("{y:.1}"));
    m.insert("grounding.confidence".to_string(), format!("{confidence:.2}"));
    if !description.is_empty() {
        m.insert("grounding.description".to_string(), description.to_string());
    }
    m
}

/// 读取 capture 写入的「真实屏幕区域设备像素」侧车文件（`<截图>.dim`，内容 `"w h"`）。
/// capture 在 Windows 路径以设备像素（含 DPI 修正）写入；缺失/损坏时返回 `None`，
/// 由调用方退化为截图像素（scale=1）。
fn read_dim_sidecar(path: &str) -> Option<(u32, u32)> {
    read_dim_sidecar_full(path).map(|(w, h, _, _)| (w, h))
}

/// 同上，但连同采集**原点**一起读（新格式 `"w h ox oy"`；旧格式缺原点时按 0,0）。
fn read_dim_sidecar_full(path: &str) -> Option<(u32, u32, i64, i64)> {
    let s = std::fs::read_to_string(path).ok()?;
    let mut it = s.split_whitespace();
    let w = it.next()?.parse::<u32>().ok()?;
    let h = it.next()?.parse::<u32>().ok()?;
    let ox = it.next().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let oy = it.next().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    Some((w, h, ox, oy))
}

/// 上一帧指纹（进程级）：同一画面重复截图时标记 unchanged，避免模型为同一帧重复推理。
fn last_frame_hash() -> &'static std::sync::Mutex<Option<String>> {
    static LAST: std::sync::OnceLock<std::sync::Mutex<Option<String>>> = std::sync::OnceLock::new();
    LAST.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只读冒烟：真实调用 `window_list` 与截图，验证 DPI 前置 + Win32 桥能跑通。
    ///
    /// 不碰键鼠、不改任何窗口，但需要**桌面会话**（无桌面的 CI 里截图为空会失败），
    /// 因此按仓库既有惯例标 `#[ignore]`，在真机上手动跑。
    #[tokio::test]
    #[ignore = "真机：需要桌面会话才能截图"]
    async fn live_window_list_and_screenshot_contract() {
        if !cfg!(windows) {
            return;
        }
        let mut a = HashMap::new();
        a.insert("action".to_string(), "window_list".to_string());
        let out = dispatch(a, None).await.expect("window_list 不应报错");
        match out {
            ToolResult::Success { data, .. } => {
                assert!(
                    data.contains("NO_VISIBLE_WINDOW") || data.contains("hwnd="),
                    "窗口列表应以行式输出: {data}"
                );
            }
            other => panic!("window_list 应成功: {other:?}"),
        }

        let mut s = HashMap::new();
        s.insert("action".to_string(), "screenshot".to_string());
        s.insert("rulers".to_string(), "true".to_string());
        let out = dispatch(s, None).await.expect("screenshot 不应报错");
        match out {
            ToolResult::Success { data, metadata } => {
                assert!(data.contains("capture="), "必须给出坐标契约: {data}");
                assert!(metadata.contains_key("screenshot.hash"), "必须给出帧指纹");
                assert!(metadata.contains_key("screenshot.contract"), "必须给出契约元数据");
            }
            other => panic!("screenshot 应成功: {other:?}"),
        }
    }

    /// 真机端到端：启动记事本 → 聚焦 → 一次 `sequence` 打字 + `ctrl+a`/`ctrl+c` → 读剪贴板校验。
    ///
    /// 会真实抢前台并注入键鼠，故 `#[ignore]`：只在桌面会话里手动跑
    /// （`cargo test -p marguerite-core --lib live_sequence -- --ignored --nocapture`）。
    #[tokio::test]
    #[ignore = "真机：会启动记事本并注入键鼠，只在桌面会话下手动运行"]
    async fn live_sequence_types_into_notepad_and_verifies_clipboard() {
        if !cfg!(windows) {
            return;
        }
        let mut launch = HashMap::new();
        launch.insert("action".to_string(), "app_launch".to_string());
        launch.insert("app".to_string(), "notepad.exe".to_string());
        let _ = dispatch(launch, None).await.expect("启动记事本");
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        let steps = r#"[
            {"action":"focus","title":"notepad"},
            {"action":"wait","ms":400},
            {"action":"type","text":"MARGUERITE-PILOT"},
            {"action":"key_combo","key":"ctrl+a"},
            {"action":"key_combo","key":"ctrl+c"},
            {"action":"wait","ms":400}
        ]"#;
        let mut seq = HashMap::new();
        seq.insert("action".to_string(), "sequence".to_string());
        seq.insert("steps".to_string(), steps.to_string());
        seq.insert("capture".to_string(), "none".to_string());
        let out = dispatch(seq, None).await.expect("sequence 不应报错");
        let data = match out {
            ToolResult::Success { data, .. } => data,
            other => panic!("sequence 应成功: {other:?}"),
        };
        assert!(
            data.contains("STEP 6 ok") && data.contains("SEQUENCE DONE 6"),
            "六步都应报 ok: {data}"
        );

        let mut clip = HashMap::new();
        clip.insert("action".to_string(), "clipboard_get".to_string());
        let clip_out = dispatch(clip, None).await.expect("读剪贴板");
        let text = match clip_out {
            ToolResult::Success { data, .. } => data,
            other => panic!("clipboard_get 应成功: {other:?}"),
        };
        // 收尾：结束记事本（不保存）
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", "notepad.exe", "/F"])
            .output();
        assert!(
            text.contains("MARGUERITE-PILOT"),
            "打字 + ctrl+a/ctrl+c 的闭环未生效，剪贴板内容: {text}"
        );
    }

    #[test]
    fn map_normalized_no_downsample_identity() {
        // 未降采样：归一化(0.5,0.5) × 截图(1920×1080) = (960,540)，real==shot → 不变
        let (x, y) = map_locate_coords(0.5, 0.5, CoordSpace::Normalized, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 960.0).abs() < 1e-6, "x={x}");
        assert!((y - 540.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_absolute_no_downsample_identity() {
        // 绝对坐标且未降采样：直接透传
        let (x, y) = map_locate_coords(123.0, 456.0, CoordSpace::Pixels, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 123.0).abs() < 1e-6, "x={x}");
        assert!((y - 456.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_downsample_inverts_scale_normalized() {
        // 截图降采样到 1280 长边（real 2560×1440 → shot 1280×720），模型归一化(0.5,0.5)
        // 截图空间 = (640,360)；反推 real = (640×2, 360×2) = (1280,720) → 必须回到真实设备像素
        let (x, y) = map_locate_coords(0.5, 0.5, CoordSpace::Normalized, 1280.0, 720.0, 2560.0, 1440.0);
        assert!((x - 1280.0).abs() < 1e-6, "x={x}");
        assert!((y - 720.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_downsample_inverts_scale_absolute() {
        // 即便模型给绝对截图坐标(640,360)，也必须反推回真实(1280,720)
        let (x, y) = map_locate_coords(640.0, 360.0, CoordSpace::Pixels, 1280.0, 720.0, 2560.0, 1440.0);
        assert!((x - 1280.0).abs() < 1e-6, "x={x}");
        assert!((y - 720.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_zero_shot_dim_keeps_ratio_one() {
        // 截图尺寸未知（0）→ 比例退化为 1（无除零），绝对坐标原样透传，不产生 NaN/inf
        let (x, y) = map_locate_coords(123.0, 456.0, CoordSpace::Pixels, 0.0, 0.0, 2560.0, 1440.0);
        assert!((x - 123.0).abs() < 1e-6, "x={x}");
        assert!((y - 456.0).abs() < 1e-6, "y={y}");
        // 归一化坐标在缺截图尺寸时必然塌缩为 (0,0)：无法凭空还原，但绝不能 panic/除零
        let (nx, ny) = map_locate_coords(0.5, 0.5, CoordSpace::Normalized, 0.0, 0.0, 2560.0, 1440.0);
        assert!(
            nx.is_finite() && ny.is_finite(),
            "normalized collapse must stay finite"
        );
        assert_eq!((nx, ny), (0.0, 0.0));
    }

    #[test]
    fn read_dim_sidecar_parses() {
        let p = std::env::temp_dir().join(format!("marg_cu_dimtest_{}.dim", std::process::id()));
        std::fs::write(&p, "2560 1440").unwrap();
        assert_eq!(read_dim_sidecar(p.to_str().unwrap()), Some((2560, 1440)));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn read_dim_sidecar_missing_returns_none() {
        assert_eq!(read_dim_sidecar("__definitely_missing__.dim"), None);
    }

    /// D2 诚实性守卫：非零退出必须是 `Error`（历史实现无条件 `Success`，把「没生效」报成成功）
    #[test]
    fn finish_nonzero_exit_is_error() {
        match finish("", "Processing -File '…' failed: Illegal characters in path.", -196608) {
            ToolResult::Error { message, code } => {
                assert_eq!(code, "NONZERO_EXIT");
                assert!(message.contains("-196608"), "message={message}");
                assert!(message.contains("Illegal characters"), "message={message}");
            }
            other => panic!("非零退出必须是 Error，实际: {other:?}"),
        }
    }

    /// 退出码 0 才允许报成功；stderr 里的诊断信息不得被吞掉
    #[test]
    fn finish_zero_exit_is_success_and_keeps_diagnostics() {
        let ok = finish("960,540\n", "", 0);
        assert!(ok.is_success());
        assert_eq!(ok.data(), Some("960,540"));

        let warn = finish("ok", "warning: something", 0);
        assert!(warn.is_success(), "退出码 0 即成功");
        assert!(warn.data().unwrap().contains("warning: something"));
    }

    /// D1 回归守卫：Windows 的 PowerShell 路径必须走 argv 直连（`NativeCmd::Ps1`），
    /// 绝不能再拼成 `powershell -File "…"` 交给 `cmd /C`（cmd 不认 Rust 的 `\"` 转义，
    /// 实测会让 `-File` 收到带字面引号的路径 → 脚本根本没跑起来）。
    #[test]
    fn windows_ps1_paths_use_direct_argv() {
        if !cfg!(windows) {
            return; // 仅 Windows 走 PowerShell 路径
        }
        let cu = OsComputerUse::new(std::env::temp_dir());
        let cmds = [
            cu.capture_cmd("C:/tmp/a.png", CaptureMode::Primary, 0)
                .unwrap(),
            cu.click_cmd(1, 2).unwrap(),
            cu.cursor_pos_cmd().unwrap(),
            cu.key_state_cmd("shift").unwrap(),
            cu.idle_time_cmd().unwrap(),
            cu.perceive_cmd().unwrap(),
        ];
        for c in cmds {
            match c {
                NativeCmd::Ps1(path) => {
                    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("ps1"));
                    let body = std::fs::read_to_string(&path).expect("脚本必须真的落盘");
                    // P2-8 中文乱码修复不得回归
                    assert!(body.contains("[Console]::OutputEncoding"));
                    let _ = std::fs::remove_file(&path);
                }
                NativeCmd::Shell(s) => {
                    panic!("Windows PowerShell 路径不得拼成 shell 命令行: {s}")
                }
            }
        }
    }

    /// D1 端到端单元复验（Windows）：真跑一次 PowerShell，退出码必须被如实透出。
    /// 修复前 `.ps1` 从未被执行，却是「ok=true + 引号报错」；此用例锁死两个方向。
    #[tokio::test]
    async fn ps1_roundtrip_reports_real_exit_code() {
        if !cfg!(windows) {
            return;
        }
        let cu = OsComputerUse::new(std::env::temp_dir());

        let ok_cmd = cu.ps1_cmd("Write-Output 'marg_cu_ok'").unwrap();
        let r = cu.run_cmd(ok_cmd).await.unwrap();
        assert!(r.is_success(), "脚本真的执行成功: {r:?}");
        assert!(r.data().unwrap().contains("marg_cu_ok"), "data={:?}", r.data());

        let bad_cmd = cu.ps1_cmd("exit 3").unwrap();
        let r = cu.run_cmd(bad_cmd).await.unwrap();
        match r {
            ToolResult::Error { message, code } => {
                assert_eq!(code, "NONZERO_EXIT");
                assert!(message.contains('3'), "message={message}");
            }
            other => panic!("非零退出必须报错，实际: {other:?}"),
        }

        // 语句级 .NET 异常：实测 PowerShell 自身退出码仍为 0，必须靠脚本尾部兜底判错
        let soft = cu
            .ps1_cmd("[System.IO.File]::ReadAllText('C:/__marg_cu_missing_zz__')")
            .unwrap();
        let r = cu.run_cmd(soft).await.unwrap();
        assert!(r.is_error(), "语句级异常也不得报成功: {r:?}");
    }

    /// 护栏在 Ps1 路径上仍然接线（改走 argv 直连后不得被静默摘掉）
    #[tokio::test]
    async fn ps1_path_still_passes_command_guard() {
        let dir = std::env::temp_dir().join("marg-cu-guard-probe");
        std::fs::create_dir_all(&dir).unwrap();
        // 文件名刻意让 `sudo` 两侧为非单词字符（`_sudo_` 因下划线算 `\w` 不构成词边界）
        let path = dir.join("marg-cu-sudo.ps1");
        std::fs::write(&path, "Write-Output 'never'").unwrap();
        let cu = OsComputerUse::new(std::env::temp_dir());
        let err = cu
            .run_cmd(NativeCmd::Ps1(path.clone()))
            .await
            .expect_err("命令行命中拒绝模式时必须被拦下");
        assert!(err.to_string().contains("command blocked"), "err={err}");
        let _ = std::fs::remove_file(&path);
    }
}

/// ───────────────────────────────────────────────────────────────────────────
/// C1 借鉴：ScreenSpot-Pro 高分辨率分块 grounding 核心
///
/// ScreenSpot-Pro 的核心发现：对 4K/retina 高 DPI 截图做整图降采样再 grounding，
/// 会丢失小控件细节；改为「原生分辨率切重叠 patch → 逐 patch grounding → 缝回全图
/// 坐标 → 跨 patch 置信度排序」可显著提升定位准确率。本模块实现其中「与视觉模型
/// 无关的纯几何/排序核心」，真实像素裁剪由 [`crop_png`] 完成（PNG 原生分辨率，
/// 不重采样）。坐标缝合与 C3 的 `map_locate_coords` 共用同一套 scale 反推思路。
/// ───────────────────────────────────────────────────────────────────────────
/// 全图坐标系下的一个 patch 矩形（截图像素，含相邻重叠）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Patch {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Patch {
    pub fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }
    pub fn height(&self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }
}

/// 生成覆盖 `img_w × img_h` 的重叠网格 patch。
/// `patch_size` 为单 patch 长边像素，`overlap` 为相邻 patch 重叠比例(0..1)。
/// 始终包含底边/右边对齐的 patch，保证整图被完整覆盖。
pub fn build_patch_grid(img_w: u32, img_h: u32, patch_size: u32, overlap: f64) -> Vec<Patch> {
    if img_w == 0 || img_h == 0 || patch_size == 0 {
        return Vec::new();
    }
    let overlap = overlap.clamp(0.0, 0.95);
    let stride = ((patch_size as f64) * (1.0 - overlap)).max(1.0) as u32;
    let mut patches = Vec::new();
    let mut ys: Vec<u32> = Vec::new();
    let mut y = 0u32;
    while y + patch_size < img_h {
        ys.push(y);
        y += stride;
    }
    ys.push(img_h.saturating_sub(patch_size.min(img_h)));
    ys.dedup();
    for y in ys {
        let mut xs: Vec<u32> = Vec::new();
        let mut x = 0u32;
        while x + patch_size < img_w {
            xs.push(x);
            x += stride;
        }
        xs.push(img_w.saturating_sub(patch_size.min(img_w)));
        xs.dedup();
        for x in xs {
            let x1 = (x + patch_size).min(img_w);
            let y1 = (y + patch_size).min(img_h);
            patches.push(Patch {
                x0: x,
                y0: y,
                x1,
                y1,
            });
        }
    }
    patches
}

/// 把某 patch 内模型输出的坐标缝回「全图像素」坐标系。
/// - `model_x/y`：模型在该 patch 上的输出（归一化时 <=1，否则为 patch 渲染像素）
/// - `patch_render_w/h`：喂给模型的 patch 图像实际像素尺寸（原生裁剪时 = patch 尺寸）
pub fn stitch_patch_coords(
    patch: &Patch,
    model_x: f64,
    model_y: f64,
    space: CoordSpace,
    patch_render_w: f64,
    patch_render_h: f64,
) -> (f64, f64) {
    let (px, py) = space.to_pixels(model_x, model_y, patch_render_w, patch_render_h);
    let sx = if patch_render_w > 0.0 {
        patch.width() as f64 / patch_render_w
    } else {
        1.0
    };
    let sy = if patch_render_h > 0.0 {
        patch.height() as f64 / patch_render_h
    } else {
        1.0
    };
    (patch.x0 as f64 + px * sx, patch.y0 as f64 + py * sy)
}

/// 一个 grounding 候选（已缝回全图像素坐标）
#[derive(Debug, Clone, Copy)]
pub struct GroundedCandidate {
    pub x: f64,
    pub y: f64,
    pub confidence: f64,
}

/// 跨 patch 候选去重 + 排序：邻近(<= `merge_dist` 像素)候选取置信度最高者，整体按置信度降序。
pub fn rank_grounding_candidates(
    mut cands: Vec<GroundedCandidate>,
    merge_dist: f64,
) -> Vec<GroundedCandidate> {
    cands.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out: Vec<GroundedCandidate> = Vec::new();
    for c in cands {
        let near = out
            .iter()
            .any(|o| (o.x - c.x).hypot(o.y - c.y) <= merge_dist);
        if !near {
            out.push(c);
        }
    }
    out
}

/// 多 patch grounding 编排（与模型无关）：对每个 patch 调用 `locate_patch` 得该 patch 内
/// 模型输出，缝合为全图候选后排序返回。真实像素裁剪由 `locate_patch` 注入。
pub fn ground_by_patches<F>(
    patches: &[Patch],
    patch_render_w: u32,
    patch_render_h: u32,
    merge_dist: f64,
    mut locate_patch: F,
) -> Vec<GroundedCandidate>
where
    F: FnMut(&Patch) -> Option<(f64, f64, CoordSpace, f64)>,
{
    let mut cands = Vec::new();
    for p in patches {
        if let Some((mx, my, space, conf)) = locate_patch(p) {
            let (fx, fy) = stitch_patch_coords(
                p,
                mx,
                my,
                space,
                patch_render_w as f64,
                patch_render_h as f64,
            );
            cands.push(GroundedCandidate {
                x: fx,
                y: fy,
                confidence: conf,
            });
        }
    }
    rank_grounding_candidates(cands, merge_dist)
}

/// 从整图字节裁剪出 `patch` 矩形子图，**原生分辨率、不重采样**。
///
/// 这是分块 grounding 能不能真正生效的唯一门槛：喂给模型的必须是 1024×1024 的
/// 原始像素，而不是整张 4K 图缩到 1024 —— 后者正是 ScreenSpot-Pro 报告里小控件
/// 定位崩掉的原因。
///
/// 刻意只处理**非隔行 8bit PNG**（截图链路的实际形状：Windows `Bitmap.Save` /
/// macOS `screencapture` / Linux `scrot` 都出 `.png`）。其它形状一律 `None`：
///
/// - 调色板 / 16bit / <8bit 灰度先经 `normalize_to_color8` 归一，因此这三类实际
///   是被支持的（解码后恒为 8bit RGB(A)/Gray(A)）；
/// - **隔行（Adam7）图拒绝**：它的行是按交互行分多趟产出的，按 `y` 直接切会裁出
///   错位的图。宁可退化成整图单定位，也不给一个「坐标看着对、点下去偏半屏」的结果。
fn crop_png(bytes: &[u8], patch: &Patch) -> Option<Vec<u8>> {
    // 64 MiB 是 png crate 的默认上限，按行读用不到那么多，但 8K 截图的解码缓冲
    // 会顶破它——这里的字节数只用于逐行缓冲，放宽没有内存风险。
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes.to_vec()),
        png::Limits {
            bytes: 256 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    if reader.info().interlaced {
        return None;
    }
    let (w, h) = reader.info().size();
    if w == 0 || h == 0 {
        return None;
    }
    let (color_type, bit_depth) = reader.output_color_type();
    let line = reader.output_line_size(w)?;
    let x0 = patch.x0.min(w);
    let y0 = patch.y0.min(h);
    let x1 = patch.x1.min(w).max(x0);
    let y1 = patch.y1.min(h).max(y0);
    let (cw, ch) = (x1 - x0, y1 - y0);
    if cw == 0 || ch == 0 {
        return None;
    }
    // 归一到 8bit 后行宽必为宽度的整数倍；`line / w` 即字节/像素。
    let bpp = line / w as usize;
    let crop_line = cw as usize * bpp;

    let mut rows: Vec<u8> = Vec::with_capacity(crop_line * ch as usize);
    let mut y = 0u32;
    while y < h {
        let Some(row) = reader.next_row().ok()? else { break };
        if y >= y0 && y < y1 {
            let start = x0 as usize * bpp;
            let data = row.data();
            if data.len() < start + crop_line {
                // 行比声明的宽度短 = 输入不是普通 PNG。截错比不截更糟。
                return None;
            }
            rows.extend_from_slice(&data[start..start + crop_line]);
        }
        y += 1;
    }
    if rows.len() != crop_line * ch as usize {
        return None;
    }

    let mut out: Vec<u8> = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, cw, ch);
        encoder.set_color(color_type);
        encoder.set_depth(bit_depth);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rows).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

/// 分块 grounding 的**调用预算**上限。
///
/// 每个 patch 是一次视觉模型往返：4K 屏按 1024/0.25 切是 15 块，不设闸就等于
/// 一次定位花 15 次调用。超出预算时按「行主序保留前 N 块」——覆盖左上角，
/// 并把截断如实写进 `grounding.patches_truncated`，绝不假装覆盖了整屏。
const MAX_PATCH_CALLS: usize = 9;

/// ScreenSpot-Pro 分块 grounding 编排（高 DPI 刚需）：高分辨率截图切重叠 patch，
/// 逐 patch 在**原生分辨率**上定位后缝回全图坐标并排序。
///
/// 返回 `None` 的含义是「本路径不适用，请走整图定位」：长边 <1600、非 PNG、
/// 裁剪失败或一个候选都没拿到。返回 `Some(vec![])` 不出现（用 `None` 表达）。
pub async fn ground_via_patches(
    bytes: &[u8],
    v: &Arc<VisionCapability>,
    description: &str,
    _threshold: f64,
) -> Option<PatchGrounding> {
    let meta = image_metadata(bytes);
    let (w, h) = (meta.width?, meta.height?);
    let long = w.max(h);
    // 仅高分辨率截图才值得分块（ScreenSpot-Pro 针对 4K/retina）
    if long < 1600 {
        return None;
    }
    let grid = build_patch_grid(w, h, 1024, 0.25);
    let total = grid.len();
    let patches = &grid[..total.min(MAX_PATCH_CALLS)];
    let mut cands = Vec::new();
    let mut calls = 0usize;
    for p in patches {
        // 裁剪失败（非 PNG / 隔行 / 坏数据）→ 整条分块路径不适用，交回整图定位。
        let cropped = crop_png(bytes, p)?;
        calls += 1;
        if let Some(r) = v.locate(&cropped, description).await {
            // 单位由产出方声明；越界或非有限的坐标是模型输出错误，不猜、不夹取。
            let Some((px, py)) = r.pixels(p.width() as f64, p.height() as f64) else {
                continue;
            };
            let (fx, fy) =
                stitch_patch_coords(p, px, py, CoordSpace::Pixels, p.width() as f64, p.height() as f64);
            cands.push(GroundedCandidate {
                x: fx,
                y: fy,
                confidence: r.confidence,
            });
        }
    }
    if cands.is_empty() {
        return None;
    }
    Some(PatchGrounding {
        candidates: rank_grounding_candidates(cands, (long as f64) * 0.02),
        model_calls: calls,
        patches_total: total,
        patches_used: patches.len(),
    })
}

/// 分块 grounding 的结果 + 它的**成本事实**（调用了几次、网格本来多大）。
///
/// 单独成类型而不是只返 `Vec<GroundedCandidate>` 的原因：分块路径比整图贵一个
/// 数量级，回执里必须能看出「这次花了 N 次模型调用、还剩 M-N 个区域没看」，
/// 否则它就是个隐性的成本黑洞。
#[derive(Debug, Clone)]
pub struct PatchGrounding {
    pub candidates: Vec<GroundedCandidate>,
    pub model_calls: usize,
    pub patches_total: usize,
    pub patches_used: usize,
}


#[cfg(test)]
mod patch_grounding_tests {
    use super::*;

    #[test]
    fn build_patch_grid_overlap_half() {
        // 100×100, patch 50, overlap 0.5 → stride 25 → 起点 {0,25,50} → 3×3=9 patch
        let g = build_patch_grid(100, 100, 50, 0.5);
        assert_eq!(g.len(), 9);
        assert!(g.contains(&Patch {
            x0: 0,
            y0: 0,
            x1: 50,
            y1: 50
        }));
        assert!(g.contains(&Patch {
            x0: 50,
            y0: 50,
            x1: 100,
            y1: 100
        }));
    }

    #[test]
    fn build_patch_grid_patch_larger_than_image() {
        // patch 大于图像 → 退化为单张整图 patch
        let g = build_patch_grid(100, 100, 200, 0.0);
        assert_eq!(g.len(), 1);
        assert_eq!(
            g[0],
            Patch {
                x0: 0,
                y0: 0,
                x1: 100,
                y1: 100
            }
        );
    }

    #[test]
    fn build_patch_grid_no_overlap() {
        // overlap 0 → stride=patch → 2×2=4
        let g = build_patch_grid(100, 100, 50, 0.0);
        assert_eq!(g.len(), 4);
        assert!(g.contains(&Patch {
            x0: 50,
            y0: 50,
            x1: 100,
            y1: 100
        }));
    }

    #[test]
    fn stitch_native_resolution_identity() {
        // patch (100,100,300,300) 原生裁剪渲染 200×200，模型归一化(0.5,0.5) → 全图(200,200)
        let p = Patch {
            x0: 100,
            y0: 100,
            x1: 300,
            y1: 300,
        };
        let (x, y) = stitch_patch_coords(&p, 0.5, 0.5, CoordSpace::Normalized, 200.0, 200.0);
        assert!((x - 200.0).abs() < 1e-6, "x={x}");
        assert!((y - 200.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn stitch_downscaled_patch_rescales_up() {
        // patch 原生 200×200，但模型看到降采样到 100×100；绝对(50,50) → 全图(200,200)
        let p = Patch {
            x0: 100,
            y0: 100,
            x1: 300,
            y1: 300,
        };
        let (x, y) = stitch_patch_coords(&p, 50.0, 50.0, CoordSpace::Pixels, 100.0, 100.0);
        assert!((x - 200.0).abs() < 1e-6, "x={x}");
        assert!((y - 200.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn rank_keeps_highest_conf_within_merge_dist() {
        let cands = vec![
            GroundedCandidate {
                x: 100.0,
                y: 100.0,
                confidence: 0.9,
            },
            GroundedCandidate {
                x: 101.0,
                y: 100.0,
                confidence: 0.95,
            }, // 邻近，置信度更高
            GroundedCandidate {
                x: 500.0,
                y: 500.0,
                confidence: 0.5,
            }, // 远离
        ];
        let out = rank_grounding_candidates(cands, 10.0);
        assert_eq!(out.len(), 2);
        assert!(
            (out[0].confidence - 0.95).abs() < 1e-9,
            "top={}",
            out[0].confidence
        );
    }

    #[test]
    fn ground_by_patches_abs_and_normalized() {
        let patches = build_patch_grid(200, 200, 100, 0.0);
        // 绝对坐标：patch(0,0,100,100) 内 (50,50) → 全图(50,50)
        let abs = ground_by_patches(&patches, 100, 100, 10.0, |p: &Patch| {
            if p.x0 == 0 && p.y0 == 0 {
                Some((50.0, 50.0, CoordSpace::Pixels, 0.9))
            } else {
                None
            }
        });
        assert_eq!(abs.len(), 1);
        assert!((abs[0].x - 50.0).abs() < 1e-6 && (abs[0].y - 50.0).abs() < 1e-6);

        // 归一化：patch(0,0,100,100) 内 (0.5,0.5) → 全图(50,50)
        let norm = ground_by_patches(&patches, 100, 100, 10.0, |p: &Patch| {
            if p.x0 == 0 && p.y0 == 0 {
                Some((0.5, 0.5, CoordSpace::Normalized, 0.8))
            } else {
                None
            }
        });
        assert_eq!(norm.len(), 1);
        assert!((norm[0].x - 50.0).abs() < 1e-6 && (norm[0].y - 50.0).abs() < 1e-6);
    }

    /// grounding 结果元数据契约：坐标/置信度齐备，字段名稳定（网关据此透出 `grounding`）。
    #[test]
    fn grounding_metadata_exposes_coords_and_confidence() {
        let m = grounding_metadata(12.34, 56.78, 0.912, "右上角搜索按钮");
        assert_eq!(m.get("grounding.x").map(String::as_str), Some("12.3"));
        assert_eq!(m.get("grounding.y").map(String::as_str), Some("56.8"));
        assert_eq!(
            m.get("grounding.confidence").map(String::as_str),
            Some("0.91")
        );
        assert_eq!(
            m.get("grounding.description").map(String::as_str),
            Some("右上角搜索按钮")
        );
        // 空描述不注入 description（保持字段最小化）
        let empty = grounding_metadata(0.0, 0.0, 0.0, "");
        assert!(!empty.contains_key("grounding.description"));
    }

    // ---- `crop_png`：分块 grounding 的唯一真门槛 --------------------------------
    //
    // 这组测试存在的意义：`crop_png` 返回 `None` 时整条分块路径会**静默退化成整图定位**，
    // 而整图定位在 4K 屏上正是失效的那一档。所以「裁剪到底裁没裁对」必须有像素级断言，
    // 不能只看它返回了 `Some`。

    /// 确定性可辨图案：`R=x`、`G=y`、`B=(x+y)%256` —— 裁错位置或裁错宽高都能立刻看出。
    fn rgb_pattern(w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(x as u8);
                px.push(y as u8);
                px.push(((x + y) % 256) as u8);
            }
        }
        px
    }

    fn encode_png(w: u32, h: u32, color: png::ColorType, px: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().expect("header");
            wr.write_image_data(px).expect("pixels");
            wr.finish().expect("finish");
        }
        out
    }

    fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut d = png::Decoder::new(std::io::Cursor::new(bytes.to_vec()));
        d.set_transformations(png::Transformations::normalize_to_color8());
        let mut r = d.read_info().expect("read_info");
        let (w, h) = r.info().size();
        let mut buf = vec![0u8; r.output_buffer_size().expect("buffer size")];
        r.next_frame(&mut buf).expect("decode");
        (w, h, buf)
    }

    /// 逐像素比对裁剪结果与原图在 `patch` 处的对应区域。
    fn assert_crop_matches_source(cropped: &[u8], src: &[u8], w: u32, bpp: usize, p: &Patch) {
        let (cw, ch, buf) = decode_png(cropped);
        assert_eq!((cw, ch), (p.width(), p.height()), "裁剪尺寸必须等于 patch 尺寸");
        assert_eq!(buf.len(), (cw * ch as u32 * bpp as u32) as usize, "字节数必须等于尺寸×bpp");
        for y in 0..ch {
            for x in 0..cw {
                let src_x = p.x0 + x;
                let src_y = p.y0 + y;
                let s = (src_y as usize * w as usize + src_x as usize) * bpp;
                let d = (y as usize * cw as usize + x as usize) * bpp;
                assert_eq!(
                    &buf[d..d + bpp],
                    &src[s..s + bpp],
                    "像素 ({x},{y}) 处裁剪结果与源图不一致 → 偏移或行宽算错了"
                );
            }
        }
    }

    #[test]
    fn crop_png_crops_at_native_resolution() {
        let (w, h) = (8u32, 6u32);
        let px = rgb_pattern(w, h);
        let png_bytes = encode_png(w, h, png::ColorType::Rgb, &px);
        let patch = Patch {
            x0: 2,
            y0: 1,
            x1: 5,
            y1: 4,
        };
        let cropped = crop_png(&png_bytes, &patch).expect("裁剪必须成功");
        assert_crop_matches_source(&cropped, &px, w, 3, &patch);
    }

    #[test]
    fn crop_png_clamps_a_patch_that_overruns_the_image() {
        // 网格最后一块经常超出图宽（1024 步进 × N > 3840）。超出部分必须夹到图内，
        // 而不是把行尾多读出来的字节当数据、也不是整块放弃。
        let (w, h) = (8u32, 6u32);
        let px = rgb_pattern(w, h);
        let png_bytes = encode_png(w, h, png::ColorType::Rgb, &px);
        let patch = Patch {
            x0: 6,
            y0: 4,
            x1: 20,
            y1: 20,
        };
        let cropped = crop_png(&png_bytes, &patch).expect("越界应夹取而非拒绝");
        let (cw, ch, _) = decode_png(&cropped);
        assert_eq!((cw, ch), (2, 2), "夹取后必须是图内剩余区域");
        assert_crop_matches_source(&cropped, &px, w, 3, &Patch {
            x0: 6,
            y0: 4,
            x1: 8,
            y1: 6,
        });
    }

    #[test]
    fn crop_png_keeps_alpha_and_gray_channels() {
        // `bpp` 是从 `output_line_size / w` 推的（png 的 `bytes_per_pixel` 不是 pub）。
        // 只测 RGB 的话，4 通道与 1 通道的错位会完全测不出来。
        let (w, h) = (6u32, 4u32);
        let rgba: Vec<u8> = rgb_pattern(w, h)
            .chunks(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 0x80])
            .collect();
        let patch = Patch {
            x0: 1,
            y0: 1,
            x1: 4,
            y1: 3,
        };
        let out = encode_png(w, h, png::ColorType::Rgba, &rgba);
        assert_crop_matches_source(
            &crop_png(&out, &patch).expect("RGBA 裁剪"),
            &rgba,
            w,
            4,
            &patch,
        );

        let gray: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| (x * 16 + y) as u8)).collect();
        let out = encode_png(w, h, png::ColorType::Grayscale, &gray);
        assert_crop_matches_source(
            &crop_png(&out, &patch).expect("灰度裁剪"),
            &gray,
            w,
            1,
            &patch,
        );
    }

    #[test]
    fn crop_png_refuses_instead_of_guessing() {
        let (w, h) = (4u32, 4u32);
        let px = rgb_pattern(w, h);
        let png_bytes = encode_png(w, h, png::ColorType::Rgb, &px);
        let ok = Patch {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 2,
        };

        // 非 PNG（例如 JPEG / 半截文件）：解码失败 → None，让调用方退回整图路径。
        assert_eq!(crop_png(b"\xff\xd8\xff\xe0not a png at all", &ok), None);
        assert_eq!(crop_png(&png_bytes[..png_bytes.len() / 2], &ok), None);
        assert_eq!(crop_png(&[], &ok), None);

        // 退化矩形（零宽 / 零高 / 完全在图外）：产出 0 字节的图比不产出更糟——
        // 视觉模型会对着空图给出一个看起来合法的坐标。
        assert_eq!(
            crop_png(
                &png_bytes,
                &Patch {
                    x0: 1,
                    y0: 1,
                    x1: 1,
                    y1: 3
                }
            ),
            None,
            "零宽 patch 必须拒绝"
        );
        assert_eq!(
            crop_png(
                &png_bytes,
                &Patch {
                    x0: 10,
                    y0: 10,
                    x1: 20,
                    y1: 20
                }
            ),
            None,
            "完全在图外的 patch 必须拒绝"
        );
    }

    #[test]
    fn crop_png_refuses_interlaced_images() {
        // Adam7 的行是按 pass 分趟产出的，按 `y` 直接切会裁出错位的图——
        // 而错位的图给出的坐标**看着完全合法**。所以必须显式拒绝。
        let (w, h) = (4u32, 4u32);
        let px = rgb_pattern(w, h);
        let mut bytes = encode_png(w, h, png::ColorType::Rgb, &px);

        // signature(8) + len(4) + type(4) + IHDR data(13) + CRC(4)
        let (type_start, data_start, data_end, crc_end) = (12usize, 16usize, 29usize, 33usize);
        assert_eq!(&bytes[type_start..data_start], b"IHDR");
        assert_eq!(bytes[data_end - 1], 0, "原图必须是非隔行，否则本测试没改到东西");
        bytes[data_end - 1] = 1; // Adam7
        let crc = crc32_ieee(&bytes[type_start..data_end]);
        bytes[data_end..crc_end].copy_from_slice(&crc.to_be_bytes());

        assert_eq!(
            crop_png(
                &bytes,
                &Patch {
                    x0: 0,
                    y0: 0,
                    x1: 2,
                    y1: 2
                }
            ),
            None,
            "隔行图必须拒绝（宁可退回整图定位）"
        );
    }

    /// PNG 用的 CRC-32（IEEE 多项式，无表实现，只服务上面那个 IHDR 改写）。
    fn crc32_ieee(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn patch_budget_truncates_on_a_4k_screen_and_says_so() {
        // 预算必须**真的会触发**，否则 `grounding.patches_truncated` 是个装饰字段；
        // 同时它必须**截断在行主序的前 N 块**（覆盖左上角），不能随机丢块。
        let grid = build_patch_grid(3840, 2160, 1024, 0.25);
        assert!(
            grid.len() > MAX_PATCH_CALLS,
            "4K 屏按 1024/0.25 应切出 {} 块，必须大于预算 {MAX_PATCH_CALLS} 才会需要如实上报截断",
            grid.len()
        );
        let used = &grid[..grid.len().min(MAX_PATCH_CALLS)];
        assert_eq!(used.len(), MAX_PATCH_CALLS);
        assert_eq!(used[0].x0, 0);
        assert_eq!(used[0].y0, 0);
        assert_eq!(used[1].y0, used[0].y0, "第二个块必须与第一个同行（行主序）");
        assert!(used[1].x0 < used[0].x1, "相邻块必须仍带重叠");
    }
}

/// 工具入口：根据 `action` 分派三层能力；`locate` 在注入视觉能力时完成
/// 「截图 → VLMM 坐标 → （可选）自动点击」闭环，否则降级为仅截图返回路径。
pub async fn dispatch(
    args: HashMap<String, String>,
    vision: Option<Arc<VisionCapability>>,
) -> Result<ToolResult> {
    let workspace = args
        .get("workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let cu = OsComputerUse::new(workspace);
    let action = args
        .get("action")
        .map(String::as_str)
        .unwrap_or("screenshot");

    // 通用参数解析
    let parse_i = |k: &str| -> i64 { args.get(k).and_then(|v| v.parse().ok()).unwrap_or(0) };
    let x = parse_i("x");
    let y = parse_i("y");
    let x2 = parse_i("x2");
    let y2 = parse_i("y2");
    let dx = parse_i("dx");
    let dy = parse_i("dy");
    let text = args.get("text").cloned().unwrap_or_default();
    let key = args.get("key").cloned().unwrap_or_default();
    let description = args.get("description").cloned().unwrap_or_default();
    let auto_click = args
        .get("auto_click")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let mode = CaptureMode::from_str(args.get("mode").map(String::as_str).unwrap_or("primary"));
    let max_edge = args
        .get("max_edge")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let threshold = args
        .get("threshold")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let patch_grounding = args
        .get("patch_grounding")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    // 坐标刻度尺：在截图上画屏幕坐标刻度（dsh-pilot 的做法），让模型不必数像素。
    let rulers = args
        .get("rulers")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    match action {
        "screenshot" => {
            let path = cu.screenshot_path();
            let p = path.to_string_lossy().to_string();
            let cmd = cu.build_capture_cmd(&p, mode, max_edge, rulers)?;
            match cu.run_cmd(cmd).await? {
                // 如实回报产物路径：否则调用方拿到「成功」却无从使用这张图
                ToolResult::Success { data, mut metadata } => {
                    metadata.insert("screenshot.path".to_string(), p.clone());
                    // 坐标契约 + 帧指纹：模型据前者决定「能不能直接照用图里的像素」，
                    // 据后者知道自己是不是盯着同一帧在重复推理。
                    let mut contract = String::new();
                    if let Ok(bytes) = std::fs::read(&path) {
                        let meta = image_metadata(&bytes);
                        let delivered = (meta.width.unwrap_or(0), meta.height.unwrap_or(0));
                        let (cw, ch, ox, oy) = read_dim_sidecar_full(&format!("{p}.dim"))
                            .unwrap_or((delivered.0, delivered.1, 0, 0));
                        contract = pilot::scale_contract((cw, ch), delivered, (ox, oy));
                        metadata.insert("screenshot.contract".to_string(), contract.clone());
                        let hash = pilot::frame_hash(&bytes);
                        let unchanged = last_frame_hash()
                            .lock()
                            .map(|mut g| {
                                let same = g.as_deref() == Some(hash.as_str());
                                *g = Some(hash.clone());
                                same
                            })
                            .unwrap_or(false);
                        metadata.insert("screenshot.hash".to_string(), hash);
                        metadata.insert("screenshot.unchanged".to_string(), unchanged.to_string());
                        if unchanged {
                            contract.push_str("；与上一帧逐字节相同（unchanged=true），坐标继续有效，无需重复推理");
                        }
                    }
                    let mut out = format!("截图已保存: {p}");
                    if !contract.is_empty() {
                        out.push('\n');
                        out.push_str(&contract);
                    }
                    if !data.is_empty() {
                        out.push('\n');
                        out.push_str(&data);
                    }
                    Ok(ToolResult::Success {
                        data: out,
                        metadata,
                    })
                }
                other => Ok(other),
            }
        }
        "click" => cu.run_cmd(cu.click_cmd(x, y)?).await,
        "double_click" => cu.run_cmd(cu.double_click_cmd(x, y)?).await,
        "right_click" => cu.run_cmd(cu.right_click_cmd(x, y)?).await,
        "middle_click" => cu.run_cmd(cu.middle_click_cmd(x, y)?).await,
        "drag" => cu.run_cmd(cu.drag_cmd(x, y, x2, y2)?).await,
        "scroll" => cu.run_cmd(cu.scroll_cmd(x, y, dx, dy)?).await,
        "move" => cu.run_cmd(cu.move_cmd(x, y)?).await,
        "type" => cu.run_cmd(cu.type_cmd(&text)?).await,
        "key" => cu.run_cmd(cu.key_cmd(&key)?).await,
        "key_combo" => cu.run_cmd(cu.key_combo_cmd(&key)?).await,
        "clipboard_set" => cu.run_cmd(cu.clipboard_set_cmd(&text)?).await,
        "clipboard_get" => cu.run_cmd(cu.clipboard_get_cmd()?).await,
        "window_focus" => cu.run_cmd(cu.window_focus_cmd(&key)?).await,
        // 窗口与应用操控（对齐 Windows-MCP 的 `App` 能力）
        "window_list" => cu.run_cmd(cu.ps1_cmd(&format!("{}\n{}", pilot::WIN32_CLASS, pilot::window_list_ps()))?).await,
        "window_move" | "window_resize" => {
            let title = args
                .get("title")
                .cloned()
                .unwrap_or_else(|| key.clone());
            if title.trim().is_empty() {
                return Ok(ToolResult::Error {
                    message: "window_move/window_resize 需要 title（窗口标题或进程名的子串）".into(),
                    code: "MISSING_TITLE".into(),
                });
            }
            let w = parse_i("width");
            let h = parse_i("height");
            let ps = pilot::window_rect_ps(&title, x, y, w, h);
            cu.run_cmd(cu.ps1_cmd(&format!("{}\n{}", pilot::WIN32_CLASS, ps))?).await
        }
        "app_launch" => {
            let app = args
                .get("app")
                .cloned()
                .unwrap_or_else(|| text.clone());
            if app.trim().is_empty() {
                return Ok(ToolResult::Error {
                    message: "app_launch 需要 app（可执行文件路径或开始菜单显示名）".into(),
                    code: "MISSING_APP".into(),
                });
            }
            let argv: Vec<String> = args
                .get("argv")
                .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
                .unwrap_or_default();
            let cwd = args.get("cwd").cloned().unwrap_or_default();
            let ps = pilot::app_launch_ps(&app, &argv, &cwd);
            cu.run_cmd(cu.ps1_cmd(&ps)?).await
        }
        // 一次调用跑完整串动作（对齐 dsh-pilot 的 desktop_sequence）
        "sequence" => {
            let raw = args.get("steps").cloned().unwrap_or_default();
            let steps = match pilot::parse_steps(&raw) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: "BAD_STEPS".into(),
                    })
                }
            };
            let risk = pilot::risk_of(&steps, args.get("risk").map(String::as_str).unwrap_or("low"));
            let confirm = args
                .get("confirm")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            let dry_run = args
                .get("dry_run")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            if dry_run {
                return Ok(ToolResult::Success {
                    data: pilot::plan_text(&steps),
                    metadata: HashMap::new(),
                });
            }
            if risk != "low" && !confirm {
                return Ok(ToolResult::Error {
                    message: format!(
                        "该批次风险等级为 {risk}，必须带 confirm=true 才会执行（先用 dry_run=true 检查计划）：\n{}",
                        pilot::plan_text(&steps)
                    ),
                    code: "SEQUENCE_NEEDS_CONFIRM".into(),
                });
            }
            let stop_on_error = args
                .get("stop_on_error")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            let script = match pilot::sequence_ps1(&steps, stop_on_error) {
                Ok(s) => s,
                Err(e) => {
                    return Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: "BAD_STEPS".into(),
                    })
                }
            };
            let executed = cu.run_cmd(cu.ps1_cmd(&script)?).await?;
            // 批次结束默认补一帧：让模型「看一眼结果」，否则它只能盲猜下一步。
            let capture = args
                .get("capture")
                .map(|v| v != "none" && v != "false")
                .unwrap_or(true);
            if !capture {
                return Ok(executed);
            }
            let shot = cu.screenshot_path();
            let sp = shot.to_string_lossy().to_string();
            let cmd = cu.build_capture_cmd(&sp, mode, max_edge, rulers)?;
            let shot_res = cu.run_cmd(cmd).await?;
            let shot_ok = shot_res.is_success();
            Ok(match executed {
                ToolResult::Success { data, mut metadata } => {
                    if shot_ok {
                        metadata.insert("screenshot.path".to_string(), sp.clone());
                        ToolResult::Success {
                            data: format!("{data}\n批次结束截图: {sp}"),
                            metadata,
                        }
                    } else {
                        ToolResult::Success { data, metadata }
                    }
                }
                // 失败/超时原样透出：不把「批次失败」粉饰成带截图的成功。
                other => other,
            })
        }
        "cursor_pos" => cu.run_cmd(cu.cursor_pos_cmd()?).await,
        "key_state" => cu.run_cmd(cu.key_state_cmd(&key)?).await,
        "idle_time" => cu.run_cmd(cu.idle_time_cmd()?).await,
        "perceive" => cu.run_cmd(cu.perceive_cmd()?).await,
        "locate" => {
            let path = cu.screenshot_path();
            let p = path.to_string_lossy().to_string();
            // 1) 截图（屏幕感知层；多屏/降采样由 mode/max_edge 控制）
            //    失败必须如实上报：否则下游会把「读不到截图文件」误报成根因
            let cmd = cu.capture_cmd(&p, mode, max_edge)?;
            if let ToolResult::Error { message, code } = cu.run_cmd(cmd).await? {
                return Ok(ToolResult::Error {
                    message: format!("截图失败: {message}"),
                    code,
                });
            }
            // 2) 读字节，交给视觉能力做 GUI 定位（grounding 层）
            match std::fs::read(&path) {
                Ok(bytes) => match vision.as_ref() {
                    Some(v) => {
                        // 截图元数据 + 真实屏幕维度（DPI/降采样反推所需）
                        let meta = image_metadata(&bytes);
                        let shot_w = meta.width.unwrap_or(0) as f64;
                        let shot_h = meta.height.unwrap_or(0) as f64;
                        let (rw, rh) = read_dim_sidecar(&format!("{p}.dim"))
                            .unwrap_or((meta.width.unwrap_or(0), meta.height.unwrap_or(0)));
                        // 分块 grounding 的编排（2026-11-01 重排）：**先整图、后分块**。
                        // 原顺序是分块优先，于是开着 `patch_grounding` 的每一次定位都要
                        // 为高分辨率付满一网格的模型调用；而整图一次调用通常就够。
                        // 只在整图「没结果 / 置信度不过阈 / 坐标非法」时升级——那恰恰是
                        // 整图降采样把小控件抹平的情形，此时分块才买得到它值的那点钱。
                        let direct = v.locate(&bytes, &description).await;
                        let escalate = patch_grounding
                            && match direct.as_ref() {
                                Some(r) => {
                                    r.confidence < threshold
                                        || r.pixels(shot_w, shot_h).is_none()
                                }
                                None => true,
                            };
                        if escalate {
                            if let Some(pg) =
                                ground_via_patches(&bytes, v, &description, threshold).await
                            {
                                if let Some(top) = pg.candidates.first() {
                                    // 分块候选不比整图好就退回整图结果：升级是为了纠错，
                                    // 不是为了换一个同样差的坐标再多花一次点击。
                                    let beats_direct = match direct.as_ref() {
                                        Some(r) => top.confidence > r.confidence,
                                        None => true,
                                    };
                                    if beats_direct {
                                        // 分块候选已在 `ground_via_patches` 内缝回全图像素。
                                        let (cx, cy) = map_locate_coords(
                                            top.x,
                                            top.y,
                                            CoordSpace::Pixels,
                                            shot_w,
                                            shot_h,
                                            rw as f64,
                                            rh as f64,
                                        );
                                        let low_conf = top.confidence < threshold;
                                        let clicked = auto_click
                                            && !low_conf
                                            && cu
                                                .auto_click(cx.round() as i64, cy.round() as i64)
                                                .await?;
                                        let _ = std::fs::remove_file(format!("{p}.dim"));
                                        let status = if low_conf { "[低置信度] " } else { "" };
                                        let mut metadata =
                                            grounding_metadata(cx, cy, top.confidence, &description);
                                        // 成本与**覆盖面**一起透出：`used/total` 不相等时
                                        // 说明网格被预算截断，右下区域根本没看过。
                                        metadata.insert(
                                            "grounding.mode".to_string(),
                                            "patches".to_string(),
                                        );
                                        metadata.insert(
                                            "grounding.model_calls".to_string(),
                                            pg.model_calls.to_string(),
                                        );
                                        metadata.insert(
                                            "grounding.patches_used".to_string(),
                                            format!("{}/{}", pg.patches_used, pg.patches_total),
                                        );
                                        metadata.insert(
                                            "grounding.patches_truncated".to_string(),
                                            if pg.patches_used < pg.patches_total { "true" } else { "false" }.to_string(),
                                        );
                                        metadata.insert(
                                            "grounding.candidates".to_string(),
                                            pg.candidates.len().to_string(),
                                        );
                                        return Ok(ToolResult::Success {
                                            data: format!(
                                                "{status}[分块grounding {}/{}块·{}次调用] 定位成功: x={:.1}, y={:.1}, confidence={:.2}\n截图: {}\n自动点击: {}",
                                                pg.patches_used,
                                                pg.patches_total,
                                                pg.model_calls,
                                                cx,
                                                cy,
                                                top.confidence,
                                                p,
                                                clicked
                                            ),
                                            metadata,
                                        });
                                    }
                                }
                            }
                        }
                        // 整图单定位（既有路径）
                        match direct {
                            Some(r) => {
                                // 单位由 `LocateResult.space` 声明；越界/非有限坐标直接
                                // 报错，绝不按数值范围猜归一化（4K 图上的像素 0.7 会被猜成图中心）。
                                let (cx, cy) = match r.pixels(shot_w, shot_h) {
                                    Some((px, py)) => map_locate_coords(
                                        px,
                                        py,
                                        CoordSpace::Pixels,
                                        shot_w,
                                        shot_h,
                                        rw as f64,
                                        rh as f64,
                                    ),
                                    None => {
                                        return Ok(ToolResult::Error {
                                            message: format!(
                                                "视觉定位返回了非法坐标: x={}, y={}, space={:?}（已拒绝按范围猜测）",
                                                r.x, r.y, r.space
                                            ),
                                            code: "VISION_INVALID_COORDINATES".to_string(),
                                        });
                                    }
                                };
                                let low_conf = r.confidence < threshold;
                                let clicked = auto_click
                                    && !low_conf
                                    && cu.auto_click(cx.round() as i64, cy.round() as i64).await?;
                                let _ = std::fs::remove_file(format!("{p}.dim"));
                                let status = if low_conf {
                                    "[低置信度] "
                                } else {
                                    ""
                                };
                                Ok(ToolResult::Success {
                                    data: format!(
                                        "{status}定位成功: x={:.1}, y={:.1}, confidence={:.2}\n截图: {}\n自动点击: {}",
                                        cx, cy, r.confidence, p, clicked
                                    ),
                                    metadata: grounding_metadata(cx, cy, r.confidence, &description),
                                })
                            }
                            None => Ok(ToolResult::Success {
                                data: format!(
                                    "[视觉降级] 未定位到「{}」或当前无可用视觉模型；截图已就绪: {}",
                                    description, p
                                ),
                                metadata: HashMap::new(),
                            }),
                        }
                    },
                    None => Ok(ToolResult::Success {
                        data: format!(
                            "[grounding] 截图已就绪: {}；请交由视觉模型(ScreenSpot-Pro/OpenCUA)输出坐标后调用 click",
                            p
                        ),
                        metadata: HashMap::new(),
                    }),
                },
                Err(e) => Ok(ToolResult::Error {
                    message: format!("读取截图失败: {e}"),
                    code: "screenshot_read".to_string(),
                }),
            }
        }
        other => Ok(ToolResult::Error {
            message: format!(
                "未知 action: {other}（支持 screenshot/click/double_click/right_click/middle_click/type/key/key_combo/scroll/drag/move/clipboard_set/clipboard_get/window_focus/cursor_pos/key_state/idle_time/perceive/locate）"
            ),
            code: "unknown_action".to_string(),
        }),
    }
}

/// 桌面动作错误 → 工具错误码（模型可按码决定重试/换窗口/报错）。
fn desktop_error_code(e: &DesktopError) -> &'static str {
    match e {
        DesktopError::AppUnknown(_) => "DESKTOP_APP_UNKNOWN",
        DesktopError::ActionDenied(_) => "ACTION_DENIED",
        DesktopError::UnsupportedAction(_) => "UNSUPPORTED_ACTION",
        DesktopError::NotRunning(_) => "DESKTOP_NOT_RUNNING",
        DesktopError::ElementNotFound(_) => "ELEMENT_NOT_FOUND",
        DesktopError::WindowNotFound(_) => "WINDOW_NOT_FOUND",
        DesktopError::Timeout(_) => "ACTION_TIMEOUT",
        DesktopError::ComError(_) => "DESKTOP_COM_ERROR",
    }
}

/// W6.2：`desktop_action` 的目标选择（纯函数，便于单测）。
#[derive(Debug, PartialEq, Eq)]
enum ActionTarget {
    /// 调用方给了已登记的句柄（desktop_launch / desktop_attach 返回），直接复用。
    Handle(String),
    /// 没给句柄：按 window 附着；`None` = 附着当前前台窗口。
    Attach(Option<String>),
}

/// 解析目标：非空 `handle` 优先；否则 `window`（去空白，空串视为未提供）→
/// `Attach(Some)`，再缺省退化到前台窗口 `Attach(None)`。
fn action_target(handle: &str, window: &str) -> ActionTarget {
    let handle = handle.trim();
    if !handle.is_empty() {
        return ActionTarget::Handle(handle.to_string());
    }
    let window = window.trim();
    if window.is_empty() {
        ActionTarget::Attach(None)
    } else {
        ActionTarget::Attach(Some(window.to_string()))
    }
}

/// 附着窗口提示：工具参数 `window` 优先；为空时回退动作参数里的 `window`。
///
/// 模型经常把 `window` 写进 `action.params`（与网关 `desktop.action` 的两条来源
/// 口径一致）；若不回退，会静默附着到前台窗口，操作错目标。
fn window_hint(window_param: &str, action: &UiAction) -> String {
    let trimmed = window_param.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    action.param_str_nonempty("window").unwrap_or_default()
}

/// 注册桌面应用操控工具：让 LLM 智能体直接驱动已注册的桌面应用（如 WPS 演示）。
///
/// 提供三个工具：
/// - `desktop_launch`：按应用标识**启动新应用**（generic 是附着语义），返回句柄 id；
/// - `desktop_attach`：附着**已经打开**的应用（按窗口标题/进程名），返回可复用句柄；
/// - `desktop_action`：执行一次 UI 动作（`handle` 可选：缺省时按 `window` 附着，
///   再退化到当前前台窗口），并复用 gateway/methods.rs 的安全闸门
///   （敏感动作二次确认 + 注入扫描）。
///
/// 桌面自动化为 Windows-only 驱动（`AppRegistry::global()` 仅在 `#[cfg(windows)]` 下
/// 注册 WpsPpt 等驱动）。非 Windows 环境下 `launch` 自然返回 `AppUnknown` 错误，
/// 工具优雅降级，因此此处**不**加 `#[cfg(windows)]` 门控，保持与既有工具一致。
fn register_desktop_app_tools(registry: &mut ToolRegistry) -> Result<()> {
    // 工具 1：desktop_launch —— 启动一个新应用（generic 是「附着当前窗口」，不会开新进程）。
    let launch_run: ToolFn = std::sync::Arc::new(move |args: &HashMap<String, String>| {
        let a = args.clone();
        Box::pin(async move {
            let app_kind = a.get("app_kind").cloned().unwrap_or_default();
            let window = a.get("window").cloned().unwrap_or_default();
            let kind = match AppKind::from_str(&app_kind) {
                Some(k) => k,
                None => {
                    return Ok(ToolResult::Error {
                        message: format!("未知桌面应用: {app_kind}"),
                        code: "DESKTOP_APP_UNKNOWN".into(),
                    })
                }
            };
            match AppRegistry::global()
                .launch(kind, &json!({"app_kind": app_kind, "window": window}))
            {
                Ok(h) => Ok(ToolResult::Success {
                    data: h.id.clone(),
                    metadata: HashMap::new(),
                }),
                Err(e) => Ok(ToolResult::Error {
                    message: e.to_string(),
                    code: desktop_error_code(&e).into(),
                }),
            }
        })
    });
    registry.register_local(make_local_tool(
        "desktop_launch",
        "启动**新**桌面应用，返回句柄 id；后续 desktop_action 使用该句柄。\
         支持 wps_ppt（WPS 演示，COM 驱动）与 generic（附着语义：绑定前台窗口或 window 指定的窗口，不会新开进程）。\
         若目标应用**已经打开**，不要用本工具：直接用 desktop_action 的 window 参数，或先调 desktop_attach。",
        json!({
            "type": "object",
            "properties": {
                "app_kind": { "type": "string", "description": "应用标识：wps_ppt（WPS 演示）或 generic（附着窗口，走 UI Automation）" },
                "window": { "type": "string", "description": "generic 专用：附着哪个窗口（标题或进程名子串）；省略则附着当前前台窗口" }
            },
            "required": ["app_kind"]
        }),
        launch_run,
    ));

    // 工具 2：desktop_attach —— 附着已打开的应用，返回可复用句柄（W6.2）。
    let attach_run: ToolFn = std::sync::Arc::new(move |args: &HashMap<String, String>| {
        let a = args.clone();
        Box::pin(async move {
            let window = a.get("window").cloned().unwrap_or_default();
            let query = (!window.trim().is_empty()).then_some(window.as_str());
            match AppRegistry::global().attach(query) {
                Ok(h) => Ok(ToolResult::Success {
                    data: h.id,
                    metadata: HashMap::new(),
                }),
                Err(e) => Ok(ToolResult::Error {
                    message: e.to_string(),
                    code: desktop_error_code(&e).into(),
                }),
            }
        })
    });
    registry.register_local(make_local_tool(
        "desktop_attach",
        "附着**已经打开**的桌面应用（按窗口标题或进程名，大小写不敏感子串），返回可复用句柄 id。\
         省略 window 时附着当前前台窗口。找不到/空窗口返回 WINDOW_NOT_FOUND。\
         拿到句柄后即可反复调用 desktop_action；不需要 desktop_launch。",
        json!({
            "type": "object",
            "properties": {
                "window": { "type": "string", "description": "窗口标题或进程名子串（如「记事本」或 notepad.exe）；省略则附着前台窗口" }
            }
        }),
        attach_run,
    ));

    // 工具 3：desktop_action —— 执行一次 UI 动作（handle 可选，缺省自动附着）。
    let action_run: ToolFn = std::sync::Arc::new(move |args: &HashMap<String, String>| {
        let a = args.clone();
        Box::pin(async move {
            let action_json = a.get("action").cloned().unwrap_or_default();
            let action: UiAction = match serde_json::from_str(&action_json) {
                Ok(a) => a,
                Err(e) => {
                    return Ok(ToolResult::Error {
                        message: format!("动作参数解析失败: {e}"),
                        code: "ACTION_PARSE".into(),
                    })
                }
            };
            // 未知动作在碰 UI 之前就拒绝：报错确定、不依赖桌面会话。
            if !crate::desktop_automation::driver::is_known_action_kind(&action.kind) {
                return Ok(ToolResult::Error {
                    message: format!(
                        "未知桌面动作: {}（支持 snapshot/find/click/invoke/set_value/type/toggle/select/expand/collapse/scroll/focus/wait/window_activate；WPS 另有 generate_presentation）",
                        action.kind
                    ),
                    code: "UNSUPPORTED_ACTION".into(),
                });
            }
            let approved = a.get("approved").map(|s| s == "true").unwrap_or(false);
            if Policy::require_confirmation(&action) && !approved {
                let reason = Policy::sensitive_target(&action)
                    .map(|term| {
                        format!("该动作需要二次确认：目标命中危险操作词「{term}」（请携带 approved=true）")
                    })
                    .unwrap_or_else(|| "该动作需要二次确认（请携带 approved=true）".into());
                return Ok(ToolResult::Error {
                    message: reason,
                    code: "ACTION_NEEDS_CONFIRM".into(),
                });
            }
            if let Some(reason) = Policy::scan_command_text(&action) {
                return Ok(ToolResult::Error {
                    message: reason,
                    code: "ACTION_INJECTION".into(),
                });
            }
            // handle 缺省 → 按 window 附着；window 也缺省 → 前台窗口（W6.2）。
            let window_param = a.get("window").cloned().unwrap_or_default();
            let target_window = window_hint(&window_param, &action);
            let handle = match action_target(
                &a.get("handle").cloned().unwrap_or_default(),
                &target_window,
            ) {
                ActionTarget::Handle(handle) => handle,
                ActionTarget::Attach(window) => match AppRegistry::global().attach(window.as_deref()) {
                    Ok(h) => h.id,
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: e.to_string(),
                            code: desktop_error_code(&e).into(),
                        })
                    }
                },
            };
            let session = a.get("session_key").cloned().unwrap_or_default();
            match AppRegistry::global().run_action(&handle, &action, &session) {
                Ok(receipt) => {
                    let data = serde_json::to_string(&receipt).unwrap_or_default();
                    Ok(ToolResult::Success {
                        data,
                        metadata: HashMap::new(),
                    })
                }
                Err(e) => Ok(ToolResult::Error {
                    message: e.to_string(),
                    code: desktop_error_code(&e).into(),
                }),
            }
        })
    });
    registry.register_local(make_local_tool(
        "desktop_action",
        "对桌面应用执行一次 UI 动作（结构化 UIA）。参数 action 是 UiAction 的 JSON 字符串。\
         **结构化优先：能用 desktop_action（按控件 ref_id/name）就不要用像素坐标（computer_use）。**\
         **操作已打开的应用用本工具 + window（标题/进程名）或 desktop_attach 返回的 handle；启动新应用用 desktop_launch。**\
         handle 可省略：缺省按 window 附着，再退化到当前前台窗口。\
         generic（UIA）支持的动作：\
         {\"kind\":\"snapshot\",\"params\":{}} 读取控件树（每项含 ref_id/role/name/automation_id/bounds/enabled/patterns）；\
         {\"kind\":\"find\",\"params\":{\"name\":\"保存\",\"role\":\"button\"}} 按条件找节点；\
         {\"kind\":\"invoke\",\"params\":{\"ref_id\":\"<snapshot/find 的 ref_id>\"}} 或 {\"kind\":\"click\",\"params\":{\"name\":\"确定\"}}；\
         {\"kind\":\"set_value\",\"params\":{\"ref_id\":\"…\",\"value\":\"文本\"}} / {\"kind\":\"type\",\"params\":{\"ref_id\":\"…\",\"text\":\"文本\",\"clear\":true}}；\
         toggle / select / expand / collapse / scroll / focus / wait（state=exists|enabled|focused|value_non_empty, timeout_ms）/ window_activate（title 或 hwnd）。\
         回执含 ok / before / after / verified / effect（value_changed|node_removed|state_changed|no_observable_change）。\
         wps_ppt 支持 generate_presentation，例如：\
         {\"kind\":\"generate_presentation\",\"params\":{\"outline\":{\"slides\":[{\"title\":\"周一\",\"bullets\":[\"晨会\",\"需求评审\"]}]},\"output_path\":\"C:/Users/Administrator/Desktop/weekly-plan.pptx\"}}。\
         可选 approved（敏感动作需 \"true\"）、session_key（审计会话键）。",
        json!({
            "type": "object",
            "properties": {
                "handle": { "type": "string", "description": "句柄 id（desktop_launch / desktop_attach 返回）。省略时按 window 附着，再退化到前台窗口" },
                "window": { "type": "string", "description": "目标窗口标题或进程名子串；handle 缺省时用它附着已打开的应用" },
                "action": { "type": "string", "description": "UiAction 的 JSON 字符串" },
                "approved": { "type": "string", "description": "敏感动作二次确认开关，携带 \"true\" 表示已确认" },
                "session_key": { "type": "string", "description": "审计会话键，用于在审计快照中定位本次会话" }
            },
            "required": ["action"]
        }),
        action_run,
    ));

    Ok(())
}

/// 注册计算机操控工具到注册表（三层架构接入 Marguerite 原生工具集）
///
/// `vision` 为可选视觉能力：注入后 `locate` 完成「截图 → VLMM 坐标 → 自动点击」闭环；
/// 为 `None` 时 `locate` 降级为仅截图并返回路径（由上层视觉模型接手）。
pub fn register_computer_use_tools(
    registry: &mut ToolRegistry,
    workspace: &std::path::Path,
    vision: Option<Arc<VisionCapability>>,
) -> Result<()> {
    let ws = workspace.to_path_buf();
    let run: ToolFn = std::sync::Arc::new(move |args: &HashMap<String, String>| {
        let mut a = args.clone();
        a.insert("workspace".to_string(), ws.to_string_lossy().to_string());
        let vision = vision.clone();
        let a = a.clone();
        Box::pin(async move { dispatch(a, vision).await })
    });
    registry.register_local(make_local_tool(
        "computer_use",
        "计算机操控：三层架构（屏幕感知 screenshot / 键鼠控制 click·double_click·right_click·middle_click·type·key·key_combo·scroll·drag·move / 剪贴板 clipboard_set·clipboard_get / 窗口聚焦 window_focus / GUI 定位 locate / 辅助感知 perceive）。\
         **结构化优先：能读到控件的场景请先用 desktop_action（UIA 的 ref_id/name/invoke/set_value），只有画布类应用或 UIA 不可用时才退回本工具的像素坐标与键鼠。**\
         另含桌面直控增强：\
         sequence（一次调用跑完整串动作，steps 为动作数组；风险批次需 confirm=true，可先 dry_run=true 看计划）；\
         window_list（列出可见窗口 hwnd/pid/是否前台/矩形/标题）；\
         window_move / window_resize（按标题移动/缩放，配合 title 与 x/y/width/height）；\
         app_launch（按可执行文件路径或开始菜单显示名启动应用）。\
         坐标系：截图与点击同为**物理屏幕像素**（每显示器 DPI 感知）；坐标契约见截图结果里的 capture/delivered/scale。\
         融合自 OpenCUA、ScreenSpot-Pro-GUI-Grounding、robotgo、computer_use_ootb、Guided-Attention-Inference-Network 等仓库，\
         并对齐 dsh-pilot 的「批量动作 + 刻度尺」与 Windows-MCP 的「窗口/应用操控 + VK 快捷键」。",
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["screenshot","click","double_click","right_click","middle_click","type","key","key_combo","scroll","drag","move","clipboard_set","clipboard_get","window_focus","window_list","window_move","window_resize","app_launch","sequence","cursor_pos","key_state","idle_time","perceive","locate"], "description": "操作类型" },
                "x": { "type": "integer", "description": "click/double_click/move/scroll 的屏幕 X 坐标（locate 时由视觉模型产出，无需手填）" },
                "y": { "type": "integer", "description": "屏幕 Y 坐标" },
                "x2": { "type": "integer", "description": "drag 的终点 X 坐标" },
                "y2": { "type": "integer", "description": "drag 的终点 Y 坐标" },
                "dx": { "type": "integer", "description": "scroll 横向格数（正=右）" },
                "dy": { "type": "integer", "description": "scroll 纵向格数（正=下）" },
                "text": { "type": "string", "description": "type 输入的文本" },
                "key": { "type": "string", "description": "key/key_combo 的按键（如 Enter / ctrl+c）" },
                "description": { "type": "string", "description": "locate 要定位的 GUI 元素描述（如「提交按钮」）" },
                "auto_click": { "type": "boolean", "description": "locate 定位成功后是否自动点击（默认 false）" },
                "mode": { "type": "string", "enum": ["primary","all","window"], "description": "screenshot/locate 的采集模式：主屏/多屏/活动窗口" },
                "max_edge": { "type": "integer", "description": "截图长边上限像素（>0 时降采样，降低 VLMM 输入体积）" },
                "threshold": { "type": "number", "description": "locate 置信度阈值（0..1），低于则视为低置信度不自动点击" },
                "patch_grounding": { "type": "boolean", "description": "locate 的整图定位没拿到/低置信时，升级到 ScreenSpot-Pro 分块 grounding（仅长边≥1600 生效；每块一次视觉调用，预算上限 9 块，回执的 grounding.patches_used 会如实标注是否被截断）" },
                "rulers": { "type": "boolean", "description": "截图上画坐标刻度（上沿/左沿每 200px，标注屏幕坐标；默认 false）" },
                "steps": { "type": "string", "description": "sequence 的动作数组（JSON 字符串）。每项形如 {\"action\":\"click\",\"x\":100,\"y\":200}；可用动作：click/double_click/right_click/middle_click/move/drag/scroll/type/key/key_combo/wait(ms)/focus(title)，可选 settle_ms" },
                "risk": { "type": "string", "enum": ["low","medium","high"], "description": "sequence 的风险声明（默认 low；含 win+r / alt+f4 等危险快捷键会自动升级）" },
                "confirm": { "type": "boolean", "description": "sequence 风险为 medium/high 时必须为 true，否则拒绝执行" },
                "dry_run": { "type": "boolean", "description": "sequence 只返回执行计划、不触碰桌面" },
                "stop_on_error": { "type": "boolean", "description": "sequence 首错即停（默认 true）" },
                "capture": { "type": "string", "description": "sequence 结束后是否补一帧：end（默认，截图）/ none（不截）" },
                "title": { "type": "string", "description": "window_move/window_resize 的目标窗口（标题或进程名子串，大小写不敏感）" },
                "width": { "type": "integer", "description": "window_resize 的目标宽度（物理像素）" },
                "height": { "type": "integer", "description": "window_resize 的目标高度（物理像素）" },
                "app": { "type": "string", "description": "app_launch 的应用：可执行文件路径，或开始菜单里的显示名（如「记事本」）" },
                "argv": { "type": "string", "description": "app_launch 的参数数组（JSON 字符串，如 [\"--new-window\"]）" },
                "cwd": { "type": "string", "description": "app_launch 的工作目录（可选）" }
            },
            "required": ["action"]
        }),
        run,
    ));

    // 桌面应用操控工具（desktop_launch / desktop_action）一并注册进 LLM 工具表。
    register_desktop_app_tools(registry)?;

    Ok(())
}

/// W6.2：`desktop_action` 目标解析的纯函数测试（不触碰真实桌面 / AppRegistry）。
#[cfg(test)]
mod desktop_target_tests {
    use super::{action_target, window_hint, ActionTarget};
    use crate::desktop_automation::driver::UiAction;
    use serde_json::json;

    #[test]
    fn explicit_handle_is_reused_and_never_attaches() {
        assert_eq!(
            action_target("  mock-handle ", "记事本"),
            ActionTarget::Handle("mock-handle".into())
        );
    }

    #[test]
    fn missing_handle_attaches_the_window_or_falls_back_to_the_foreground() {
        assert_eq!(
            action_target("", "  记事本 "),
            ActionTarget::Attach(Some("记事本".into()))
        );
        assert_eq!(
            action_target("   ", "  "),
            ActionTarget::Attach(None),
            "空 handle + 空 window = 附着当前前台窗口"
        );
    }

    #[test]
    fn window_hint_prefers_the_tool_param_and_falls_back_to_action_params() {
        let action = UiAction {
            kind: "invoke".into(),
            params: json!({ "window": "计算器" }),
        };
        assert_eq!(window_hint(" 记事本 ", &action), "记事本");
        assert_eq!(
            window_hint("   ", &action),
            "计算器",
            "工具参数为空时必须回退 action.params.window"
        );
        let none = UiAction {
            kind: "invoke".into(),
            params: json!({}),
        };
        assert_eq!(window_hint("", &none), "");
    }
}
