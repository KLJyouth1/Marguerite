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
//! Windows 路径通过生成临时 `.ps1` 规避 cmd 引号陷阱，运行时正确性可经 `sandboxed_exec`
//! 返回的 stdout/stderr 观测并调优。多屏坐标采用 Windows 虚拟桌面坐标系，与全屏截图一致。

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
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
use crate::vision::VisionCapability;

/// 工具闭包类型（与 `tools::builtin::ToolFn` 透明等价）
type ToolFn = std::sync::Arc<
    dyn for<'a> Fn(
            &'a HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + 'a>>
        + Send
        + Sync,
>;

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
    /// 返回采集当前屏幕的命令（由 `sandboxed_exec` 执行）。
    /// `mode` 控制主屏/多屏/活动窗口；`max_edge` 为缩边上限（>0 时按长边等比缩到该像素，
    /// 用于降低发给 VLMM 的截图体积，对齐 OpenCUA/computer_use_ootb 的降采样策略）。
    fn capture_cmd(&self, out_path: &str, mode: CaptureMode, max_edge: u32) -> Result<String>;
}

/// 三层架构之一：键鼠控制（输入执行）
pub trait InputControl {
    /// 鼠标点击（移动到 x,y 后按下/抬起左键）
    fn click_cmd(&self, x: i64, y: i64) -> Result<String>;
    /// 鼠标双击
    fn double_click_cmd(&self, x: i64, y: i64) -> Result<String>;
    /// 鼠标右键点击
    fn right_click_cmd(&self, x: i64, y: i64) -> Result<String>;
    /// 鼠标中键点击
    fn middle_click_cmd(&self, x: i64, y: i64) -> Result<String>;
    /// 鼠标拖拽（从 x1,y1 到 x2,y2）
    fn drag_cmd(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<String>;
    /// 鼠标滚轮（dx 横向、dy 纵向，单位=滚动格）
    fn scroll_cmd(&self, x: i64, y: i64, dx: i64, dy: i64) -> Result<String>;
    /// 移动鼠标到 x,y（不点击）
    fn move_cmd(&self, x: i64, y: i64) -> Result<String>;
    /// 键盘输入文本
    fn type_cmd(&self, text: &str) -> Result<String>;
    /// 键盘按键
    fn key_cmd(&self, key: &str) -> Result<String>;
    /// 键盘组合键（如 "ctrl+c"，按 SendKeys/cliclick 约定）
    fn key_combo_cmd(&self, keys: &str) -> Result<String>;
    /// 剪贴板：写入文本
    fn clipboard_set_cmd(&self, text: &str) -> Result<String>;
    /// 剪贴板：读取文本
    fn clipboard_get_cmd(&self) -> Result<String>;
    /// 窗口聚焦（按窗口标题匹配，激活到前台）
    fn window_focus_cmd(&self, title: &str) -> Result<String>;
    /// 获取当前鼠标位置（输出 "x,y"）
    fn cursor_pos_cmd(&self) -> Result<String>;
    /// 查询按键状态（输出 "pressed"/"released"）
    fn key_state_cmd(&self, key: &str) -> Result<String>;
    /// 获取系统空闲时间（毫秒）
    fn idle_time_cmd(&self) -> Result<String>;
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
    fn write_ps1(&self, content: &str) -> Result<PathBuf> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("marg_cu_{stamp}.ps1"));
        std::fs::write(&p, content).map_err(|e| anyhow!("写入 ps1 失败: {e}"))?;
        Ok(p)
    }

    /// 执行一条命令并返回合并输出（成功/错误统一为 `ToolResult`）
    async fn run_cmd(&self, cmd: &str) -> Result<ToolResult> {
        let guard = CommandGuard::new();
        let out = sandboxed_exec(cmd, &self.workspace, DEFAULT_TIMEOUT, &guard).await?;
        let mut s = String::new();
        s.push_str(&out.stdout);
        if !out.stderr.is_empty() {
            s.push_str("\n[stderr]\n");
            s.push_str(&out.stderr);
        }
        s.push_str(&format!("\n[exit_code={}]", out.exit_code));
        Ok(ToolResult::Success {
            data: s,
            metadata: HashMap::new(),
        })
    }

    /// 构造截图命令（跨平台；多屏/活动窗口/降采样由 `mode`/`max_edge` 控制）
    fn build_capture_cmd(&self, path: &str, mode: CaptureMode, max_edge: u32) -> Result<String> {
        if cfg!(windows) {
            // PowerShell 内容（写入 .ps1 规避 cmd 引号陷阱）；单引号包裹路径
            let bounds = match mode {
                // 设备像素：逻辑 Bounds × DPI 比例（SetCursorPos 使用设备/虚拟坐标空间，非 DPI 缩放逻辑像素）
                CaptureMode::Primary => "$dpi=(([System.Drawing.Graphics]::FromHwnd([IntPtr]::Zero)).DpiX)/96.0; $s=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $w=[int]($s.Width*$dpi); $h=[int]($s.Height*$dpi); $ox=[int]($s.X*$dpi); $oy=[int]($s.Y*$dpi);".to_string(),
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
            let content = format!(
                "Add-Type -AssemblyName System.Windows.Forms\nAdd-Type -AssemblyName System.Drawing\n{bounds}\n$b=New-Object System.Drawing.Bitmap($w,$h)\n$g=[System.Drawing.Graphics]::FromImage($b)\n$g.CopyFromScreen($ox,$oy,0,0,$b.Size)\n{save}\n\"$w $h\" | Set-Content -Path '{p}.dim' -Encoding ascii\n",
                bounds = bounds, save = save, p = path
            );
            let p = self.write_ps1(&content)?;
            return Ok(format!(
                "powershell -ExecutionPolicy Bypass -File \"{}\"",
                p.display()
            ));
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
            return Ok(cmd);
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
            return Ok(cmd);
        }
    }

    /// 构造输入命令（跨平台）。`action` 可为 click/double/right_click/middle_click/drag/scroll/move/type/key/combo/clipboard_set/clipboard_get/window_focus。
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
    ) -> Result<String> {
        if cfg!(windows) {
            // 统一经 user32（SetCursorPos + mouse_event）与 SendKeys，写入 .ps1 执行
            let sig = "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool SetCursorPos(int X,int Y); [DllImport(\"user32.dll\")] public static extern void mouse_event(int f,int a,int b,int d,int e); [DllImport(\"user32.dll\")] public static extern void keybd_event(byte b,int a,int f,int e);' -Name MU -Namespace W32 -PassThru";
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
                "type" | "key" | "combo" => {
                    let t = if action == "combo" { key } else { text };
                    // SendKeys 用单引号包裹；转义内部单引号
                    let escaped = t.replace('\'', "''");
                    format!(
                        "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('{e}')",
                        e = escaped
                    )
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
                    let escaped = key.replace('\'', "''");
                    format!(
                        "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.Interaction]::AppActivate('{e}')",
                        e = escaped
                    )
                }
                _ => return Err(anyhow!("未知输入动作: {action}")),
            };
            let content = format!("{sig}\n{body}\n", sig = sig, body = body);
            let p = self.write_ps1(&content)?;
            return Ok(format!(
                "powershell -ExecutionPolicy Bypass -File \"{}\"",
                p.display()
            ));
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
            Ok(cmd)
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
            Ok(cmd)
        }
    }

    /// 构造无障碍树 / 窗口枚举命令（辅助感知通道，对齐 awesome-computer-use / computer-vision-guide）
    fn perceive_cmd(&self) -> Result<String> {
        if cfg!(windows) {
            let content = "Add-Type -AssemblyName UIAutomationClient; Add-Type -AssemblyName UIAutomationTypes; \
$tries=0; \
$root=[System.Windows.Automation.AutomationElement]::RootElement; \
$cond=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Window); \
$root.FindAll([System.Windows.Automation.TreeScope]::Subtree,$cond) | \
  ForEach-Object { ($_.Current.Name+' | '+($_.Current.ControlType.ProgrammaticName -replace 'ControlType.','')) } | \
  Where-Object { $_.Length -gt 0 } | Select-Object -First 60";
            let p = self.write_ps1(&content)?;
            return Ok(format!(
                "powershell -ExecutionPolicy Bypass -File \"{}\"",
                p.display()
            ));
        } else if cfg!(target_os = "macos") {
            Ok("osascript -e 'tell application \"System Events\" to get name of every process whose visible is true'".to_string())
        } else {
            // 可见窗口名列表（辅助定位，避免单纯依赖像素 grounding）
            Ok("xdotool search --onlyvisible --name '.*' 2>/dev/null | while read id; do xdotool getwindowname \"$id\" 2>/dev/null; done | head -60".to_string())
        }
    }

    /// 获取当前鼠标位置
    fn cursor_pos_cmd(&self) -> Result<String> {
        if cfg!(windows) {
            let content = "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool GetCursorPos(out POINT lpPoint); [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }' -Name CUR -Namespace W32; $p=New-Object W32.CUR+POINT; [W32.CUR]::GetCursorPos([ref]$p) | Out-Null; Write-Output \"$($p.X),$($p.Y)\"";
            let p = self.write_ps1(content)?;
            Ok(format!(
                "powershell -ExecutionPolicy Bypass -File \"{}\"",
                p.display()
            ))
        } else if cfg!(target_os = "macos") {
            Ok("cliclick p".to_string())
        } else {
            Ok("xdotool getmouselocation --shell 2>/dev/null | grep -E 'X=|Y=' | tr '\\n' ',' | sed 's/,$/\\n/'".to_string())
        }
    }

    /// 查询按键状态
    fn key_state_cmd(&self, key: &str) -> Result<String> {
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
            let p = self.write_ps1(&content)?;
            Ok(format!(
                "powershell -ExecutionPolicy Bypass -File \"{}\"",
                p.display()
            ))
        } else if cfg!(target_os = "macos") {
            Ok(format!(
                "osascript -e 'tell application \"System Events\" to return key code of {key}'"
            ))
        } else {
            Ok(format!(
                "xset q | grep -i 'LED mask' | grep -q '00000002' && echo pressed || echo released"
            ))
        }
    }

    /// 获取系统空闲时间（毫秒）
    fn idle_time_cmd(&self) -> Result<String> {
        if cfg!(windows) {
            let content = "Add-Type -MemberDefinition '[DllImport(\"user32.dll\")] public static extern bool GetLastInputInfo(ref LASTINPUTINFO plii); [StructLayout(LayoutKind.Sequential)] public struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }' -Name IDLE -Namespace W32; $lii=New-Object W32.IDLE+LASTINPUTINFO; $lii.cbSize=[uint32][System.Runtime.InteropServices.Marshal]::SizeOf($lii); [W32.IDLE]::GetLastInputInfo([ref]$lii) | Out-Null; $idle=([Environment]::TickCount - $lii.dwTime); Write-Output $idle";
            let p = self.write_ps1(content)?;
            Ok(format!(
                "powershell -ExecutionPolicy Bypass -File \"{}\"",
                p.display()
            ))
        } else if cfg!(target_os = "macos") {
            Ok("echo $(( $(python3 -c 'import CoreFoundation; print(int(CoreFoundation.CGEventSourceSecondsSinceLastEventType(1, 0x05)*1000))' 2>/dev/null || echo 0) ))".to_string())
        } else {
            Ok("xprintidle 2>/dev/null || echo 0".to_string())
        }
    }
}

impl ScreenPerception for OsComputerUse {
    fn capture_cmd(&self, out_path: &str, mode: CaptureMode, max_edge: u32) -> Result<String> {
        self.build_capture_cmd(out_path, mode, max_edge)
    }
}

impl InputControl for OsComputerUse {
    fn click_cmd(&self, x: i64, y: i64) -> Result<String> {
        self.build_input_cmd("click", x, y, 0, 0, 0, 0, "", "")
    }
    fn double_click_cmd(&self, x: i64, y: i64) -> Result<String> {
        self.build_input_cmd("double", x, y, 0, 0, 0, 0, "", "")
    }
    fn right_click_cmd(&self, x: i64, y: i64) -> Result<String> {
        self.build_input_cmd("right_click", x, y, 0, 0, 0, 0, "", "")
    }
    fn middle_click_cmd(&self, x: i64, y: i64) -> Result<String> {
        self.build_input_cmd("middle_click", x, y, 0, 0, 0, 0, "", "")
    }
    fn drag_cmd(&self, x1: i64, y1: i64, x2: i64, y2: i64) -> Result<String> {
        self.build_input_cmd("drag", x1, y1, x2, y2, 0, 0, "", "")
    }
    fn scroll_cmd(&self, x: i64, y: i64, dx: i64, dy: i64) -> Result<String> {
        self.build_input_cmd("scroll", x, y, 0, 0, dx, dy, "", "")
    }
    fn move_cmd(&self, x: i64, y: i64) -> Result<String> {
        self.build_input_cmd("move", x, y, 0, 0, 0, 0, "", "")
    }
    fn type_cmd(&self, text: &str) -> Result<String> {
        self.build_input_cmd("type", 0, 0, 0, 0, 0, 0, text, "")
    }
    fn key_cmd(&self, key: &str) -> Result<String> {
        self.build_input_cmd("key", 0, 0, 0, 0, 0, 0, "", key)
    }
    fn key_combo_cmd(&self, keys: &str) -> Result<String> {
        self.build_input_cmd("combo", 0, 0, 0, 0, 0, 0, "", keys)
    }
    fn clipboard_set_cmd(&self, text: &str) -> Result<String> {
        self.build_input_cmd("clipboard_set", 0, 0, 0, 0, 0, 0, text, "")
    }
    fn clipboard_get_cmd(&self) -> Result<String> {
        self.build_input_cmd("clipboard_get", 0, 0, 0, 0, 0, 0, "", "")
    }
    fn window_focus_cmd(&self, title: &str) -> Result<String> {
        self.build_input_cmd("window_focus", 0, 0, 0, 0, 0, 0, "", title)
    }
    fn cursor_pos_cmd(&self) -> Result<String> {
        OsComputerUse::cursor_pos_cmd(self)
    }
    fn key_state_cmd(&self, key: &str) -> Result<String> {
        OsComputerUse::key_state_cmd(self, key)
    }
    fn idle_time_cmd(&self) -> Result<String> {
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
    normalized: bool,
    shot_w: f64,
    shot_h: f64,
    real_w: f64,
    real_h: f64,
) -> (f64, f64) {
    let (px, py) = if normalized {
        (model_x * shot_w, model_y * shot_h)
    } else {
        (model_x, model_y)
    };
    let sx = if shot_w > 0.0 { real_w / shot_w } else { 1.0 };
    let sy = if shot_h > 0.0 { real_h / shot_h } else { 1.0 };
    (px * sx, py * sy)
}

/// 读取 capture 写入的「真实屏幕区域设备像素」侧车文件（`<截图>.dim`，内容 `"w h"`）。
/// capture 在 Windows 路径以设备像素（含 DPI 修正）写入；缺失/损坏时返回 `None`，
/// 由调用方退化为截图像素（scale=1）。
fn read_dim_sidecar(path: &str) -> Option<(u32, u32)> {
    let s = std::fs::read_to_string(path).ok()?;
    let mut it = s.split_whitespace();
    let w = it.next()?.parse::<u32>().ok()?;
    let h = it.next()?.parse::<u32>().ok()?;
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_normalized_no_downsample_identity() {
        // 未降采样：归一化(0.5,0.5) × 截图(1920×1080) = (960,540)，real==shot → 不变
        let (x, y) = map_locate_coords(0.5, 0.5, true, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 960.0).abs() < 1e-6, "x={x}");
        assert!((y - 540.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_absolute_no_downsample_identity() {
        // 绝对坐标且未降采样：直接透传
        let (x, y) = map_locate_coords(123.0, 456.0, false, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 123.0).abs() < 1e-6, "x={x}");
        assert!((y - 456.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_downsample_inverts_scale_normalized() {
        // 截图降采样到 1280 长边（real 2560×1440 → shot 1280×720），模型归一化(0.5,0.5)
        // 截图空间 = (640,360)；反推 real = (640×2, 360×2) = (1280,720) → 必须回到真实设备像素
        let (x, y) = map_locate_coords(0.5, 0.5, true, 1280.0, 720.0, 2560.0, 1440.0);
        assert!((x - 1280.0).abs() < 1e-6, "x={x}");
        assert!((y - 720.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_downsample_inverts_scale_absolute() {
        // 即便模型给绝对截图坐标(640,360)，也必须反推回真实(1280,720)
        let (x, y) = map_locate_coords(640.0, 360.0, false, 1280.0, 720.0, 2560.0, 1440.0);
        assert!((x - 1280.0).abs() < 1e-6, "x={x}");
        assert!((y - 720.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn map_zero_shot_dim_keeps_ratio_one() {
        // 截图尺寸未知（0）→ 比例退化为 1（无除零），绝对坐标原样透传，不产生 NaN/inf
        let (x, y) = map_locate_coords(123.0, 456.0, false, 0.0, 0.0, 2560.0, 1440.0);
        assert!((x - 123.0).abs() < 1e-6, "x={x}");
        assert!((y - 456.0).abs() < 1e-6, "y={y}");
        // 归一化坐标在缺截图尺寸时必然塌缩为 (0,0)：无法凭空还原，但绝不能 panic/除零
        let (nx, ny) = map_locate_coords(0.5, 0.5, true, 0.0, 0.0, 2560.0, 1440.0);
        assert!(nx.is_finite() && ny.is_finite(), "normalized collapse must stay finite");
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
}

/// ───────────────────────────────────────────────────────────────────────────
/// C1 借鉴：ScreenSpot-Pro 高分辨率分块 grounding 核心
///
/// ScreenSpot-Pro 的核心发现：对 4K/retina 高 DPI 截图做整图降采样再 grounding，
/// 会丢失小控件细节；改为「原生分辨率切重叠 patch → 逐 patch grounding → 缝回全图
/// 坐标 → 跨 patch 置信度排序」可显著提升定位准确率。本模块实现其中「与视觉模型
/// 无关的纯几何/排序核心」，真实像素裁剪由 `crop_png` 注入（当前未编入 image 后端，
/// 退化为整图单 patch）。坐标缝合与 C3 的 `map_locate_coords` 共用同一套 scale 反推思路。
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
    normalized: bool,
    patch_render_w: f64,
    patch_render_h: f64,
) -> (f64, f64) {
    let px = if normalized {
        model_x * patch_render_w
    } else {
        model_x
    };
    let py = if normalized {
        model_y * patch_render_h
    } else {
        model_y
    };
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
    F: FnMut(&Patch) -> Option<(f64, f64, bool, f64)>,
{
    let mut cands = Vec::new();
    for p in patches {
        if let Some((mx, my, norm, conf)) = locate_patch(p) {
            let (fx, fy) = stitch_patch_coords(
                p,
                mx,
                my,
                norm,
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

/// 从整图字节裁剪出 `patch` 矩形子图。
/// ⚠️ marguerite-core 当前未编入 `image` 后端，恒返回 None（裁剪不可用）；
/// 接入 `image` crate 后即可实现原生分辨率 patch 裁剪，使分块 grounding 真正生效。
fn crop_png(_bytes: &[u8], _patch: &Patch) -> Option<Vec<u8>> {
    None
}

/// ScreenSpot-Pro 分块 grounding 编排（高 DPI 刚需）：高分辨率截图切重叠 patch，
/// 逐 patch 定位后缝合排序。无 image 后端时 `crop_png` 恒 None → 整体返回 None
/// （调用方 `locate` 退化为整图单定位）。
pub async fn ground_via_patches(
    bytes: &[u8],
    v: &Arc<VisionCapability>,
    description: &str,
    _threshold: f64,
) -> Option<Vec<GroundedCandidate>> {
    let meta = image_metadata(bytes);
    let (w, h) = (meta.width? as u32, meta.height? as u32);
    let long = w.max(h);
    // 仅高分辨率截图才值得分块（ScreenSpot-Pro 针对 4K/retina）
    if long < 1600 {
        return None;
    }
    let patches = build_patch_grid(w, h, 1024, 0.25);
    let mut cands = Vec::new();
    for p in &patches {
        // 真实像素裁剪需 image 后端；当前退化为整图单 patch（crop_png 恒 None → ? 提前返回）
        let cropped = crop_png(bytes, p)?;
        if let Some(r) = v.locate(&cropped, description).await {
            let norm = r.x.abs() <= 1.0 && r.y.abs() <= 1.0;
            let (fx, fy) = stitch_patch_coords(p, r.x, r.y, norm, p.width() as f64, p.height() as f64);
            cands.push(GroundedCandidate {
                x: fx,
                y: fy,
                confidence: r.confidence,
            });
        }
    }
    if cands.is_empty() {
        None
    } else {
        Some(rank_grounding_candidates(cands, (long as f64) * 0.02))
    }
}

#[cfg(test)]
mod patch_grounding_tests {
    use super::*;

    #[test]
    fn build_patch_grid_overlap_half() {
        // 100×100, patch 50, overlap 0.5 → stride 25 → 起点 {0,25,50} → 3×3=9 patch
        let g = build_patch_grid(100, 100, 50, 0.5);
        assert_eq!(g.len(), 9);
        assert!(g.contains(&Patch { x0: 0, y0: 0, x1: 50, y1: 50 }));
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
        assert_eq!(g[0], Patch { x0: 0, y0: 0, x1: 100, y1: 100 });
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
        let p = Patch { x0: 100, y0: 100, x1: 300, y1: 300 };
        let (x, y) = stitch_patch_coords(&p, 0.5, 0.5, true, 200.0, 200.0);
        assert!((x - 200.0).abs() < 1e-6, "x={x}");
        assert!((y - 200.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn stitch_downscaled_patch_rescales_up() {
        // patch 原生 200×200，但模型看到降采样到 100×100；绝对(50,50) → 全图(200,200)
        let p = Patch { x0: 100, y0: 100, x1: 300, y1: 300 };
        let (x, y) = stitch_patch_coords(&p, 50.0, 50.0, false, 100.0, 100.0);
        assert!((x - 200.0).abs() < 1e-6, "x={x}");
        assert!((y - 200.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn rank_keeps_highest_conf_within_merge_dist() {
        let cands = vec![
            GroundedCandidate { x: 100.0, y: 100.0, confidence: 0.9 },
            GroundedCandidate { x: 101.0, y: 100.0, confidence: 0.95 }, // 邻近，置信度更高
            GroundedCandidate { x: 500.0, y: 500.0, confidence: 0.5 },  // 远离
        ];
        let out = rank_grounding_candidates(cands, 10.0);
        assert_eq!(out.len(), 2);
        assert!((out[0].confidence - 0.95).abs() < 1e-9, "top={}", out[0].confidence);
    }

    #[test]
    fn ground_by_patches_abs_and_normalized() {
        let patches = build_patch_grid(200, 200, 100, 0.0);
        // 绝对坐标：patch(0,0,100,100) 内 (50,50) → 全图(50,50)
        let abs = ground_by_patches(&patches, 100, 100, 10.0, |p: &Patch| {
            if p.x0 == 0 && p.y0 == 0 {
                Some((50.0, 50.0, false, 0.9))
            } else {
                None
            }
        });
        assert_eq!(abs.len(), 1);
        assert!((abs[0].x - 50.0).abs() < 1e-6 && (abs[0].y - 50.0).abs() < 1e-6);

        // 归一化：patch(0,0,100,100) 内 (0.5,0.5) → 全图(50,50)
        let norm = ground_by_patches(&patches, 100, 100, 10.0, |p: &Patch| {
            if p.x0 == 0 && p.y0 == 0 {
                Some((0.5, 0.5, true, 0.8))
            } else {
                None
            }
        });
        assert_eq!(norm.len(), 1);
        assert!((norm[0].x - 50.0).abs() < 1e-6 && (norm[0].y - 50.0).abs() < 1e-6);
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

    match action {
        "screenshot" => {
            let path = cu.screenshot_path();
            let p = path.to_string_lossy().to_string();
            let cmd = cu.capture_cmd(&p, mode, max_edge)?;
            cu.run_cmd(&cmd).await
        }
        "click" => cu.run_cmd(&cu.click_cmd(x, y)?).await,
        "double_click" => cu.run_cmd(&cu.double_click_cmd(x, y)?).await,
        "right_click" => cu.run_cmd(&cu.right_click_cmd(x, y)?).await,
        "middle_click" => cu.run_cmd(&cu.middle_click_cmd(x, y)?).await,
        "drag" => cu.run_cmd(&cu.drag_cmd(x, y, x2, y2)?).await,
        "scroll" => cu.run_cmd(&cu.scroll_cmd(x, y, dx, dy)?).await,
        "move" => cu.run_cmd(&cu.move_cmd(x, y)?).await,
        "type" => cu.run_cmd(&cu.type_cmd(&text)?).await,
        "key" => cu.run_cmd(&cu.key_cmd(&key)?).await,
        "key_combo" => cu.run_cmd(&cu.key_combo_cmd(&key)?).await,
        "clipboard_set" => cu.run_cmd(&cu.clipboard_set_cmd(&text)?).await,
        "clipboard_get" => cu.run_cmd(&cu.clipboard_get_cmd()?).await,
        "window_focus" => cu.run_cmd(&cu.window_focus_cmd(&key)?).await,
        "cursor_pos" => cu.run_cmd(&cu.cursor_pos_cmd()?).await,
        "key_state" => cu.run_cmd(&cu.key_state_cmd(&key)?).await,
        "idle_time" => cu.run_cmd(&cu.idle_time_cmd()?).await,
        "perceive" => cu.run_cmd(&cu.perceive_cmd()?).await,
        "locate" => {
            let path = cu.screenshot_path();
            let p = path.to_string_lossy().to_string();
            // 1) 截图（屏幕感知层；多屏/降采样由 mode/max_edge 控制）
            let cmd = cu.capture_cmd(&p, mode, max_edge)?;
            cu.run_cmd(&cmd).await?;
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
                        // C1 分块 grounding：高分辨率截图 + 开启 patch_grounding 时，
                        // 先走 ScreenSpot-Pro 分块路径；返回 None（长边<1600 或当前无 image
                        // 后端裁剪 patch）则退化为整图单定位（既有路径）。
                        if patch_grounding {
                            if let Some(cands) =
                                ground_via_patches(&bytes, v, &description, threshold).await
                            {
                                if let Some(top) = cands.first() {
                                    let (cx, cy) = map_locate_coords(
                                        top.x, top.y, false, shot_w, shot_h, rw as f64, rh as f64,
                                    );
                                    let low_conf = top.confidence < threshold;
                                    if auto_click && !low_conf {
                                        let _ = cu
                                            .run_cmd(&cu.move_cmd(cx.round() as i64, cy.round() as i64)?)
                                            .await;
                                        let _ = cu
                                            .run_cmd(&cu.click_cmd(cx.round() as i64, cy.round() as i64)?)
                                            .await;
                                    }
                                    let _ = std::fs::remove_file(format!("{p}.dim"));
                                    let status = if low_conf { "[低置信度] " } else { "" };
                                    return Ok(ToolResult::Success {
                                        data: format!(
                                            "{status}[分块grounding] 定位成功: x={:.1}, y={:.1}, confidence={:.2}\n截图: {}\n自动点击: {}",
                                            cx, cy, top.confidence, p, auto_click && !low_conf
                                        ),
                                        metadata: HashMap::new(),
                                    });
                                }
                            }
                        }
                        // 整图单定位（既有路径）
                        match v.locate(&bytes, &description).await {
                            Some(r) => {
                                let normalized = r.x.abs() <= 1.0 && r.y.abs() <= 1.0;
                                let (cx, cy) = map_locate_coords(
                                    r.x, r.y, normalized, shot_w, shot_h, rw as f64, rh as f64,
                                );
                                let low_conf = r.confidence < threshold;
                                if auto_click && !low_conf {
                                    let _ = cu
                                        .run_cmd(&cu.move_cmd(cx.round() as i64, cy.round() as i64)?)
                                        .await;
                                    let _ = cu
                                        .run_cmd(&cu.click_cmd(cx.round() as i64, cy.round() as i64)?)
                                        .await;
                                }
                                let _ = std::fs::remove_file(format!("{p}.dim"));
                                let status = if low_conf {
                                    "[低置信度] "
                                } else {
                                    ""
                                };
                                Ok(ToolResult::Success {
                                    data: format!(
                                        "{status}定位成功: x={:.1}, y={:.1}, confidence={:.2}\n截图: {}\n自动点击: {}",
                                        cx, cy, r.confidence, p, auto_click && !low_conf
                                    ),
                                    metadata: HashMap::new(),
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

/// 注册桌面应用操控工具：让 LLM 智能体直接驱动已注册的桌面应用（如 WPS 演示）。
///
/// 提供两个工具：
/// - `desktop_launch`：按应用标识启动一个桌面应用，返回句柄 id；
/// - `desktop_action`：对句柄执行一次 UI 动作（如「生成一周计划 PPT」），
///   并复用 gateway/methods.rs 的安全闸门（敏感动作二次确认 + 注入扫描）。
///
/// 桌面自动化为 Windows-only 驱动（`AppRegistry::global()` 仅在 `#[cfg(windows)]` 下
/// 注册 WpsPpt 等驱动）。非 Windows 环境下 `launch` 自然返回 `AppUnknown` 错误，
/// 工具优雅降级，因此此处**不**加 `#[cfg(windows)]` 门控，保持与既有工具一致。
fn register_desktop_app_tools(registry: &mut ToolRegistry) -> Result<()> {
    // 工具 1：desktop_launch —— 启动一个已注册的桌面应用，返回句柄 id。
    let launch_run: ToolFn = std::sync::Arc::new(move |args: &HashMap<String, String>| {
        let a = args.clone();
        Box::pin(async move {
            let app_kind = a.get("app_kind").cloned().unwrap_or_default();
            let kind = match AppKind::from_str(&app_kind) {
                Some(k) => k,
                None => {
                    return Ok(ToolResult::Error {
                        message: format!("未知桌面应用: {app_kind}"),
                        code: "DESKTOP_APP_UNKNOWN".into(),
                    })
                }
            };
            match AppRegistry::global().launch(kind, &json!({"app_kind": app_kind})) {
                Ok(h) => Ok(ToolResult::Success {
                    data: h.id.clone(),
                    metadata: HashMap::new(),
                }),
                Err(e) => {
                    let code = match e {
                        DesktopError::AppUnknown(_) => "DESKTOP_APP_UNKNOWN",
                        DesktopError::ComError(_) => "DESKTOP_COM_ERROR",
                        _ => "DESKTOP_COM_ERROR",
                    };
                    Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: code.into(),
                    })
                }
            }
        })
    });
    registry.register_local(make_local_tool(
        "desktop_launch",
        "启动一个已注册的桌面应用，返回句柄 id；后续 desktop_action 使用该句柄。\
         目前支持 wps_ppt（WPS 演示）。调用约定：先用本工具拿到句柄，再把句柄传给 desktop_action 执行具体动作。",
        json!({
            "type": "object",
            "properties": {
                "app_kind": { "type": "string", "description": "应用标识，如 wps_ppt" }
            },
            "required": ["app_kind"]
        }),
        launch_run,
    ));

    // 工具 2：desktop_action —— 对句柄执行一次 UI 动作（如生成 WPS PPT）。
    let action_run: ToolFn = std::sync::Arc::new(move |args: &HashMap<String, String>| {
        let a = args.clone();
        Box::pin(async move {
            let handle = a.get("handle").cloned().unwrap_or_default();
            if handle.is_empty() {
                return Ok(ToolResult::Error {
                    message: "缺少 handle".into(),
                    code: "MISSING_HANDLE".into(),
                });
            }
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
            let approved = a.get("approved").map(|s| s == "true").unwrap_or(false);
            if Policy::require_confirmation(&action) && !approved {
                return Ok(ToolResult::Error {
                    message: "该动作需要二次确认（请携带 approved=true）".into(),
                    code: "ACTION_NEEDS_CONFIRM".into(),
                });
            }
            if let Some(reason) = Policy::scan_injection(&action) {
                return Ok(ToolResult::Error {
                    message: reason,
                    code: "ACTION_INJECTION".into(),
                });
            }
            let session = a.get("session_key").cloned().unwrap_or_default();
            match AppRegistry::global().run_action(&handle, &action, &session) {
                Ok(receipt) => {
                    let data = serde_json::to_string(&receipt).unwrap_or_default();
                    Ok(ToolResult::Success {
                        data,
                        metadata: HashMap::new(),
                    })
                }
                Err(e) => {
                    let code = match e {
                        DesktopError::AppUnknown(_) => "DESKTOP_APP_UNKNOWN",
                        DesktopError::ActionDenied(_) => "ACTION_DENIED",
                        _ => "DESKTOP_COM_ERROR",
                    };
                    Ok(ToolResult::Error {
                        message: e.to_string(),
                        code: code.into(),
                    })
                }
            }
        })
    });
    registry.register_local(make_local_tool(
        "desktop_action",
        "对句柄执行一次 UI 动作。参数 action 是 UiAction 的 JSON 字符串；\
         例如生成 WPS PPT：\
         {\"kind\":\"generate_presentation\",\"params\":{\"outline\":{\"slides\":[{\"title\":\"周一\",\"bullets\":[\"晨会\",\"需求评审\"]},{\"title\":\"周二\",\"bullets\":[\"编码\"]}]},\"output_path\":\"C:/Users/Administrator/Desktop/weekly-plan.pptx\"}}。\
         可选 approved（敏感动作需 \"true\"）、session_key（审计会话键）。",
        json!({
            "type": "object",
            "properties": {
                "handle": { "type": "string", "description": "desktop_launch 返回的句柄 id" },
                "action": { "type": "string", "description": "UiAction 的 JSON 字符串" },
                "approved": { "type": "string", "description": "敏感动作二次确认开关，携带 \"true\" 表示已确认" },
                "session_key": { "type": "string", "description": "审计会话键，用于在审计快照中定位本次会话" }
            },
            "required": ["handle", "action"]
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
         融合自 OpenCUA、ScreenSpot-Pro-GUI-Grounding、robotgo、computer_use_ootb、Guided-Attention-Inference-Network 等 10 个 computer use 仓库；\
         grounding 由视觉语言模型在「截图→坐标→执行」闭环中完成。",
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["screenshot","click","double_click","right_click","middle_click","type","key","key_combo","scroll","drag","move","clipboard_set","clipboard_get","window_focus","cursor_pos","key_state","idle_time","perceive","locate"], "description": "操作类型" },
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
                "patch_grounding": { "type": "boolean", "description": "locate 高分辨率截图时启用 ScreenSpot-Pro 分块 grounding（仅长边≥1600 生效；需 image 后端裁剪 patch）" }
            },
            "required": ["action"]
        }),
        run,
    ));

    // 桌面应用操控工具（desktop_launch / desktop_action）一并注册进 LLM 工具表。
    register_desktop_app_tools(registry)?;

    Ok(())
}
