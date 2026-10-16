//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! System control + memory tools

use super::*;

/// user32 P/Invoke 前缀（C# 内联，经 PowerShell Add-Type 编译）
pub(crate) const SYS_CTRL_PREAMBLE: &str = r#"
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class MgtSC {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT lp);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int nIndex);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
}
"@
"#;

pub(crate) fn system_control_ps(action: &str, args: &HashMap<String, String>) -> Result<String> {
    let stmt = match action {
        "mouse_move" => {
            let x: i32 = args
                .get("x")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| anyhow!("mouse_move 需要整数 x"))?;
            let y: i32 = args
                .get("y")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| anyhow!("mouse_move 需要整数 y"))?;
            // 拟人化移动（默认开启，humanize:0 回退直线跳转）：贝塞尔三次曲线
            // 路径 + 随机控制点 + 18 步插值 + 随机步进延迟，模仿人手轨迹。
            // 终点精确等于 (x,y)，不影响落点精度。
            let humanize: bool = args
                .get("humanize")
                .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
                .unwrap_or(true);
            if !humanize {
                format!("[MgtSC]::SetCursorPos({x},{y})")
            } else {
                format!(
                    "$p=New-Object MgtSC+POINT;[MgtSC]::GetCursorPos([ref]$p)|Out-Null;\
$sx=$p.X;$sy=$p.Y;$dx={x};$dy={y};\
$cx1=$sx+($dx-$sx)*0.3+(Get-Random -Minimum -40 -Maximum 40);\
$cy1=$sy+($dy-$sy)*0.2+(Get-Random -Minimum -40 -Maximum 40);\
$cx2=$sx+($dx-$sx)*0.7+(Get-Random -Minimum -40 -Maximum 40);\
$cy2=$sy+($dy-$sy)*0.8+(Get-Random -Minimum -40 -Maximum 40);\
$steps=18;\
for($i=1;$i -le $steps;$i++){{$t=$i/$steps;$mt=1-$t;\
$px=[int]($mt*$mt*$mt*$sx+3*$mt*$mt*$t*$cx1+3*$mt*$t*$t*$cx2+$t*$t*$t*$dx);\
$py=[int]($mt*$mt*$mt*$sy+3*$mt*$mt*$t*$cy1+3*$mt*$t*$t*$cy2+$t*$t*$t*$dy);\
[MgtSC]::SetCursorPos($px,$py)|Out-Null;Start-Sleep -Milliseconds (Get-Random -Minimum 4 -Maximum 14)}};\
[MgtSC]::SetCursorPos({x},{y})|Out-Null"
                )
            }
        }
        "mouse_click" => {
            let x: Option<i32> = args.get("x").and_then(|v| v.parse().ok());
            let y: Option<i32> = args.get("y").and_then(|v| v.parse().ok());
            let button = args.get("button").map(String::as_str).unwrap_or("left");
            // "1"/"true" → 双击（bool::from_str 只认 true/false，故手动归一）
            let double: bool = args
                .get("double")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let (down, up): (u32, u32) = match button {
                "right" => (0x08, 0x10),
                "middle" => (0x20, 0x40),
                _ => (0x02, 0x04), // left
            };
            let mut s = String::new();
            if let (Some(x), Some(y)) = (x, y) {
                s.push_str(&format!("[MgtSC]::SetCursorPos({x},{y});"));
            }
            let clicks = if double { 2 } else { 1 };
            for _ in 0..clicks {
                s.push_str(&format!(
                    "[MgtSC]::mouse_event(0x{down:X},0,0,0,[UIntPtr]::Zero);[MgtSC]::mouse_event(0x{up:X},0,0,0,[UIntPtr]::Zero);"
                ));
            }
            s
        }
        "mouse_scroll" => {
            let lines: i32 = args
                .get("lines")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| anyhow!("mouse_scroll 需要整数 lines"))?;
            let mut s = String::new();
            if let (Some(x), Some(y)) = (
                args.get("x").and_then(|v| v.parse::<i32>().ok()),
                args.get("y").and_then(|v| v.parse::<i32>().ok()),
            ) {
                s.push_str(&format!("[MgtSC]::SetCursorPos({x},{y});"));
            }
            let delta = (lines * 120) as u32; // 上正下负（补码）
            s.push_str(&format!("[MgtSC]::mouse_event(0x0800,0,0,{delta},[UIntPtr]::Zero)"));
            s
        }
        "mouse_pos" => {
            "$p=New-Object MgtSC+POINT;[MgtSC]::GetCursorPos([ref]$p)|Out-Null;\"x=$($p.X) y=$($p.Y)\""
                .to_string()
        }
        "screen_size" => {
            "\"width=\"+[MgtSC]::GetSystemMetrics(0)+\" height=\"+[MgtSC]::GetSystemMetrics(1)"
                .to_string()
        }
        "key_press" => {
            let key = args.get("key").cloned().unwrap_or_default();
            if key.trim().is_empty() {
                return Err(anyhow!("key_press 需要非空 key"));
            }
            let sk = sendkeys_for(&key).ok_or_else(|| anyhow!("不支持的按键: {key}"))?;
            format!("[System.Windows.Forms.SendKeys]::SendWait('{sk}')")
        }
        "type_text" => {
            // 经单引号转义**直接内联**文本（PS 单引号串是字面量、支持换行）。
            // 历史缺陷：经进程级 env `MARG_SYS_TEXT` 传递 → `execute_parallel` 并发
            // 子代理互相覆盖同一 env → 打错字；且 Windows env 本身也有 32K 上限。
            let text = args.get("text").cloned().unwrap_or_default();
            let t = super::ps_single_quote(&text);
            format!("Add-Type -AssemblyName System.Windows.Forms;[System.Windows.Forms.Clipboard]::SetText('{t}');[System.Windows.Forms.SendKeys]::SendWait('^v')")
        }
        // 多屏坐标枚举：列出全部显示器在虚拟桌面坐标系中的边界（可为负）与主屏标记。
        // 让 Agent 先在多屏上定位，再据此给出 mouse_move/mouse_click 的绝对坐标。
        "monitors" => {
            r#"Add-Type -AssemblyName System.Windows.Forms
$i=0
foreach($s in [System.Windows.Forms.Screen]::AllScreens){
  "monitor$i x=$($s.Bounds.X) y=$($s.Bounds.Y) w=$($s.Bounds.Width) h=$($s.Bounds.Height) primary=$($s.Primary)"
  $i++
}"#
            .to_string()
        }
        // 截图 + 可选 OCR：.NET System.Drawing 截取屏幕（指定显示器/区域/全虚拟屏）存 PNG，
        // ocr=1 时用 Windows.Media.Ocr（WinRT，零新增依赖）识别文本并附带每行包围盒坐标。
        "screenshot" => {
            let out = args.get("out").cloned().unwrap_or_default();
            if out.trim().is_empty() {
                return Err(anyhow!("screenshot 需要非空 out（截图输出路径）"));
            }
            let monitor: i32 = args.get("monitor").and_then(|v| v.parse().ok()).unwrap_or(-1);
            let opt = |k: &str| -> String {
                args.get(k)
                    .and_then(|v| v.parse::<i32>().ok())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "$null".into())
            };
            let ocr: bool = args
                .get("ocr")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let ocr_block = if ocr {
                r#"
if($ocr){
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  $null=[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType=WindowsRuntime]
  $null=[Windows.Graphics.Imaging.BitmapDecoder, Windows.Foundation, ContentType=WindowsRuntime]
  $null=[Windows.Storage.StorageFile, Windows.Foundation, ContentType=WindowsRuntime]
  $asTaskGeneric=([System.WindowsRuntimeSystemExtensions].GetMethods()|Where-Object{$_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'})[0]
  function Await($t,$rt){$m=$asTaskGeneric.MakeGenericMethod($rt);$nt=$m.Invoke($null,@($t));$nt.Wait(-1)|Out-Null;$nt.Result}
  try{
    $file=Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($out)) ([Windows.Storage.StorageFile])
    $stream=Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
    $decoder=Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $bitmap=Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
    $engine=[Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
    if($null -eq $engine){"OCR 引擎不可用（Windows 10+ 需安装 OCR 语言包）"}
    else{
      $result=Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
      $lines=@($result.Lines)
      "OCR 识别 $($lines.Count) 行："
      foreach($lineObj in $lines){
        $words=@($lineObj.Words)
        if($words.Count -gt 0){
          $w0=$words[0].BoundingRect
          "  [$([int]$w0.X),$([int]$w0.Y),$([int]$w0.Width),$([int]$w0.Height)] $($lineObj.Text)"
        } else { "  [?] $($lineObj.Text)" }
      }
    }
  } catch { "OCR 失败: $($_.Exception.Message)" }
}"#
                .to_string()
            } else {
                String::new()
            };
            format!(
                r#"Add-Type -AssemblyName System.Drawing,System.Windows.Forms
$out='{out_q}'
if(-not ([IO.Path]::IsPathRooted($out))){{$out=Join-Path (Get-Location) $out}}
$monitor={monitor};$rx={rx};$ry={ry};$rw={rw};$rh={rh};$ocr={ocr_bool}
$screens=[System.Windows.Forms.Screen]::AllScreens
$b=$null
if($monitor -ge 0 -and $monitor -lt $screens.Count){{$b=$screens[$monitor].Bounds}}
if($null -eq $b -and $rx -ne $null -and $ry -ne $null -and $rw -ne $null -and $rh -ne $null){{$b=New-Object System.Drawing.Rectangle($rx,$ry,$rw,$rh)}}
if($null -eq $b){{$b=[System.Windows.Forms.SystemInformation]::VirtualScreen}}
if($b.Width -le 0 -or $b.Height -le 0){{throw "无效截图区域"}}
$bmp=New-Object System.Drawing.Bitmap($b.Width,$b.Height)
$g=[System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.X,$b.Y,0,0,$bmp.Size)
$g.Dispose()
$dir=Split-Path -Parent $out
if($dir -and -not (Test-Path $dir)){{New-Item -ItemType Directory -Force -Path $dir|Out-Null}}
$bmp.Save($out,[System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"已保存截图: $out [$($b.Width)x$($b.Height) @ ($($b.X),$($b.Y))]"
{ocr_block}"#,
                monitor = monitor,
                rx = opt("x"),
                ry = opt("y"),
                rw = opt("width"),
                rh = opt("height"),
                ocr_bool = if ocr { "$true" } else { "$false" },
                ocr_block = ocr_block,
                out_q = super::ps_single_quote(&out),
            )
        }
        other => {
            return Err(anyhow!(
                "未知 action: {other}（支持 mouse_move/mouse_click/mouse_scroll/mouse_pos/screen_size/monitors/screenshot/key_press/type_text）"
            ))
        }
    };
    Ok(format!("{SYS_CTRL_PREAMBLE}\n{stmt}"))
}

/// 把按键/组合键映射为 SendKeys 语法（`^` Ctrl、`%` Alt、`+` Shift、`{}` 命名键）
pub(crate) fn sendkeys_for(key: &str) -> Option<String> {
    let lower = key.to_lowercase();
    let named = |k: &str| -> Option<&'static str> {
        Some(match k {
            "enter" | "return" => "{ENTER}",
            "esc" | "escape" => "{ESC}",
            "tab" => "{TAB}",
            "space" => " ",
            "backspace" => "{BACKSPACE}",
            "up" => "{UP}",
            "down" => "{DOWN}",
            "left" => "{LEFT}",
            "right" => "{RIGHT}",
            "home" => "{HOME}",
            "end" => "{END}",
            "pageup" | "pgup" => "{PGUP}",
            "pagedown" | "pgdn" => "{PGDN}",
            "insert" | "ins" => "{INSERT}",
            "delete" | "del" => "{DELETE}",
            "capslock" => "{CAPSLOCK}",
            "numlock" => "{NUMLOCK}",
            "scrolllock" => "{SCROLLLOCK}",
            _ => return None,
        })
    };
    // 命名功能键 F1..F12
    if lower.starts_with('f') && lower.len() >= 2 {
        if let Ok(n) = lower[1..].parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(format!("{{F{n}}}"));
            }
        }
    }
    if let Some(v) = named(&lower) {
        return Some(v.to_string());
    }
    // 组合键：ctrl/alt/shift + 尾键
    let parts: Vec<&str> = lower.split('+').collect();
    if parts.len() > 1 {
        let mut prefix = String::new();
        for p in &parts[..parts.len() - 1] {
            match *p {
                "ctrl" | "control" => prefix.push('^'),
                "alt" => prefix.push('%'),
                "shift" => prefix.push('+'),
                _ => return None,
            }
        }
        let last = parts.last().unwrap();
        let key_part = named(last).unwrap_or(last);
        return Some(format!("{prefix}{key_part}"));
    }
    // 单字符直接发送
    if lower.chars().count() == 1 {
        return Some(lower);
    }
    None
}

pub(crate) fn system_control_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "system_control",
        "系统级鼠标键盘控制（Windows user32/SendInput）：真实操控桌面光标与键盘（第三方软硬件/系统级输入）。action=mouse_move(x,y,humanize?)|mouse_click(x,y?,button,double?)|mouse_scroll(lines,x?,y?)|mouse_pos|screen_size|monitors(多屏坐标枚举)|screenshot(out,monitor?,x?,y?,width?,height?,ocr?)|key_press(key)|type_text(text)。参数: action(必填), x, y, button(left|right|middle), double(0|1), lines, key, text, humanize, out, monitor, width, height, ocr(0|1，截图后 OCR 识别文本+坐标)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["mouse_move","mouse_click","mouse_scroll","mouse_pos","screen_size","monitors","screenshot","key_press","type_text"]},
            "x":{"type":"number"},
            "y":{"type":"number"},
            "button":{"type":"string","enum":["left","right","middle"]},
            "double":{"type":"number"},
            "lines":{"type":"number","description":"滚动行数（正上负下）"},
            "humanize":{"type":"number","description":"mouse_move 是否拟人化贝塞尔轨迹移动（1 默认；0=直线跳转）"},
            "out":{"type":"string","description":"screenshot 截图输出路径（相对工作区或绝对）"},
            "monitor":{"type":"number","description":"screenshot 目标显示器索引（0 起；缺省=全虚拟屏）"},
            "width":{"type":"number","description":"screenshot 区域宽度（配合 x/y/height 指定区域）"},
            "height":{"type":"number","description":"screenshot 区域高度"},
            "ocr":{"type":"number","description":"screenshot 是否 OCR：1=识别文本并输出每行包围盒坐标"},
            "key":{"type":"string","description":"按键或组合键：enter/esc/tab/up/down/f5/ctrl+c/alt+tab..."},
            "text":{"type":"string","description":"type_text 要输入的文本"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err(
                        "system_control 当前仅支持 Windows（user32 系统级输入）".into(),
                        "NOT_AVAILABLE",
                    );
                }
                let action = args.get("action").cloned().unwrap_or_default();
                if action.trim().is_empty() {
                    return err("system_control 需要非空 action".into(), "INVALID_PARAMS");
                }
                let ps = match system_control_ps(&action, args) {
                    Ok(p) => p,
                    Err(e) => return err(format!("{e}"), "INVALID_PARAMS"),
                };
                let ps_ref = ps.as_str();
                let mut pscmd = tokio::process::Command::new("powershell");
                pscmd.args(["-NoProfile", "-NonInteractive", "-Command", ps_ref]);
                #[cfg(windows)]
                {
                    pscmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                }
                let out = match pscmd.output().await
                {
                    Ok(o) => o,
                    Err(e) => return err(format!("启动 PowerShell 失败: {e}"), "SYSTEM_CONTROL_ERROR"),
                };
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    return err(
                        format!("system_control 执行失败: {}", stderr.trim()),
                        "SYSTEM_CONTROL_ERROR",
                    );
                }
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                ok(if stdout.is_empty() {
                    format!("{action} 已执行")
                } else {
                    stdout
                })
            })
        }),
    )
}

// ── 全程联网：轻量 web_search（DDG 主引擎 + Bing 回退，零 API key）──
//
// 对齐 Marguerite/gwwc 的 web_search：无 key 场景下以 DuckDuckGo HTML 为主引擎，
// 空结果/失败时回退 Bing HTML 第二引擎；检索结果（标题+链接+摘要）供 Agent 快速
// 发现信息，再配合 web_fetch 深读正文。
/// 多引擎联网检索（**优先内置浏览器**，无 Chromium 时回退零 API key 直接抓取）。
///
/// 7.3 锐意革新：默认优先走 **自研内置浏览器** 通道
/// （[`crate::browser::collect::browser_search`]）——CDP 驱动真实 Chromium、从**渲染后 DOM**
/// 抽取结果，比 HTML 字符串爬取更抗反爬，且**零 search API key、不依赖任何外部抓取服务**。
/// 仅当本机检测到可用 Chromium 且浏览器通道返回非空结果时才采用；否则（无 Chromium /
/// 浏览器抽取失败 / 返回空）快速回退 [`run_web_search_direct`] 的 `cn.bing` + DuckDuckGo
/// 直接抓取引擎，保证离线 / 无浏览器环境依旧可用。
///
/// 失败返回 `Err(可读原因)`，供上层回退/报错。
///
/// `pub(crate)`：网关层 `tools.web_search` / `tools.web_research` RPC 复用同一实现，
/// 保证「agent 内部工具」与「WebUI 直接调用」走同一套搜索逻辑与引擎。
/// 浏览器通道结果是否**可采纳**（非空 **且** 非低质）。
///
/// 抽成纯函数以便离线单测断言「低质结果触发回退」（等价于运行日志出现「回退直连引擎」）。
/// 关键修复：低质（引擎首页/空壳）结果**不再**被「非空即采纳」短路——否则用户截图里
/// 的引擎首页会被当成有效结果直接返回。
pub(crate) fn browser_results_adoptable(results: &[SearchResult]) -> bool {
    !results.is_empty() && !is_low_quality(results)
}

pub(crate) async fn run_web_search(q: &str, max: usize) -> Result<Vec<SearchResult>, String> {
    // 熔断门：浏览器通道连续不可采纳时，冷却期内直接走直连引擎。
    // 实测（2026-10-08）一轮 office 任务里这里被调用 14 次、每次都白试一遍浏览器通道
    // 再回退，单轮白耗数分钟 —— 见 `search_quality::BrowserSearchCircuit` 的说明。
    //
    // ⚠️ 锁**绝不能跨 await 持有**（`std::sync::MutexGuard` 非 Send，会让整个 future 非 Send）。
    // 所以每处都"取判断/记结果 → 立即放锁"，中间只留 await。
    let now_ms = chrono::Utc::now().timestamp_millis();
    // 先在**块内**取完判断并放锁，再决定是否 await（guard 绝不可跨 await 存活）。
    let (skipping, consecutive_bad) = {
        let circuit = browser_search_circuit().lock().unwrap_or_else(|e| e.into_inner());
        (circuit.should_skip(now_ms), circuit.consecutive_bad())
    };
    if skipping {
        tracing::debug!(
            consecutive_bad,
            "浏览器通道处于熔断冷却期，直接走直连引擎"
        );
        return run_web_search_direct(q, max).await;
    }

    // 优先：内置浏览器（无外部 API）。仅当 Chromium 可用时尝试，避免无谓启动浏览器。
    let bridge = BrowserBridge::new();
    if bridge.is_available() {
        let outcome = crate::browser::collect::browser_search(&bridge, q, max).await;
        let adoptable = matches!(&outcome, Ok(res) if browser_results_adoptable(res));
        match &outcome {
            Ok(res) if adoptable => {
                record_browser_outcome(true, now_ms);
                return Ok(res.clone());
            }
            Ok(res) if res.is_empty() => {
                record_browser_outcome(false, now_ms);
                warn!("内置浏览器搜索返回空结果（引擎 DOM 结构可能变化），回退直接抓取引擎");
            }
            Ok(_) => {
                record_browser_outcome(false, now_ms);
                warn!("浏览器通道结果低质（引擎首页/空壳），回退直连引擎");
            }
            Err(e) => {
                record_browser_outcome(false, now_ms);
                warn!(error = %e, "内置浏览器搜索失败，回退直接抓取引擎");
            }
        }
    }
    run_web_search_direct(q, max).await
}

/// 记录一次浏览器通道结果（取锁 → 写入 → 立即放锁）。
fn record_browser_outcome(adoptable: bool, now_ms: i64) {
    let mut circuit = browser_search_circuit().lock().unwrap_or_else(|e| e.into_inner());
    circuit.record(adoptable, now_ms);
}

/// 浏览器检索通道熔断器的进程级单例。
///
/// 进程级而非调用级：浏览器通道的失败是环境性的，会话内多次检索应共享同一判断，
/// 否则每一次调用都会重新试探一遍（正是实测的白耗来源）。
fn browser_search_circuit(
) -> &'static std::sync::Mutex<crate::tools::builtin::search_quality::BrowserSearchCircuit> {
    static CIRCUIT: std::sync::OnceLock<
        std::sync::Mutex<crate::tools::builtin::search_quality::BrowserSearchCircuit>,
    > = std::sync::OnceLock::new();
    CIRCUIT.get_or_init(|| {
        std::sync::Mutex::new(crate::tools::builtin::search_quality::BrowserSearchCircuit::new())
    })
}

