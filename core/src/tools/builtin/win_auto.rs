//! Auto-extracted from builtin.rs — see parent module for shared infrastructure.
//!
//! Windows automation: window/clipboard/process/office/powershell

use super::*;
use crate::proc::SilentCommand;

// ---------------------------------------------------------------------------
// 桌面自动化增强工具（窗口管理 / 剪贴板 / 进程管理 / 应用启动 / WPS 办公）
// ---------------------------------------------------------------------------

/// 窗口管理工具：列出/聚焦/最小化/最大化/关闭窗口
pub(crate) fn window_manage_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "window_manage",
        "窗口管理：列出所有可见窗口、聚焦/最小化/最大化/还原/关闭指定窗口。action=list|focus|minimize|maximize|restore|close。参数: action(必填), title(窗口标题模糊匹配，list 时可省略)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","focus","minimize","maximize","restore","close"]},
            "title":{"type":"string","description":"窗口标题（模糊匹配，如 'WPS'、'Chrome'、'记事本'）"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("window_manage 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let title = args.get("title").cloned().unwrap_or_default();
                let ps = match action.as_str() {
                    "list" => r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinList {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
}
"@
$sb = New-Object System.Text.StringBuilder 256
$results = @()
[WinList]::EnumWindows({ param($h,$l)
    if ([WinList]::IsWindowVisible($h)) {
        [void][WinList]::GetWindowText($h, $sb, 256)
        $t = $sb.ToString()
        if ($t.Length -gt 0) {
            $pid = 0; [void][WinList]::GetWindowThreadProcessId($h, [ref]$pid)
            $pname = (Get-Process -Id $pid -ErrorAction SilentlyContinue).ProcessName
            $results += "$pname|$t"
        }
    }
    $true
}, [IntPtr]::Zero) | Out-Null
$results | Sort-Object -Unique
"#.to_string(),
                    "focus" | "minimize" | "maximize" | "restore" | "close" => {
                        if title.is_empty() {
                            return err(format!("{action} 操作需要 title 参数"), "INVALID_PARAMS");
                        }
                        let (ps_verb, post) = match action.as_str() {
                            "focus" => ("SetForegroundWindow($h)", ""),
                            "minimize" => ("CloseWindow($h)", ""),
                            "maximize" => ("ShowWindowAsync($h,3)", ""),
                            "restore" => ("ShowWindowAsync($h,9)", ""),
                            "close" => ("PostMessage($h,0x0010,0,0)", ""),
                            _ => unreachable!(),
                        };
                        // 标题是模型可控输入：经单引号转义后再拼进脚本（见 ps_single_quote）。
                        let title_q = ps_single_quote(&title);
                        format!(r#"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinCtl {{
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool CloseWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}}
"@
$procs = Get-Process | Where-Object {{ $_.MainWindowTitle -like '*{title_q}*' }}
foreach ($p in $procs) {{
    $h = $p.MainWindowHandle
    if ($h -ne [IntPtr]::Zero) {{ [void][WinCtl]::{ps_verb}; "$($p.ProcessName): OK{post}" }}
}}
if (-not $procs) {{ '未找到标题包含 {title_q} 的窗口' }}
"#)
                    }
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                run_powershell(&ps).await
            })
        }),
    )
}

/// 剪贴板工具：读取/写入/清空剪贴板
pub(crate) fn clipboard_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "clipboard",
        "剪贴板操作：读取当前剪贴板文本、写入文本到剪贴板、清空剪贴板。action=read|write|clear。参数: action(必填), text(write 时必填)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["read","write","clear"]},
            "text":{"type":"string","description":"write 时要写入剪贴板的文本"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("clipboard 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let mut temp: Option<std::path::PathBuf> = None;
                let ps = match action.as_str() {
                    "read" => "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard::GetText()]".to_string(),
                    "write" => {
                        let text = args.get("text").cloned().unwrap_or_default();
                        let tf = match write_temp_payload(&text, "clip") {
                            Ok(p) => p,
                            Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
                        };
                        let q = ps_single_quote(&tf.to_string_lossy());
                        temp = Some(tf);
                        format!("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard::SetText((Get-Content -Raw -Encoding UTF8 '{q}')); 'clipboard written'")
                    }
                    "clear" => "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard::Clear(); 'clipboard cleared'".to_string(),
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                let r = run_powershell(&ps).await;
                if let Some(tf) = temp {
                    let _ = std::fs::remove_file(tf);
                }
                r
            })
        }),
    )
}

/// 进程管理工具：列出/启动/终止进程
pub(crate) fn process_manage_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "process_manage",
        "进程管理：列出运行中的进程（可按名称过滤）、启动应用程序、终止进程。action=list|start|kill。参数: action(必填), name(进程名或应用路径，如 'notepad'、'chrome'、'winword'), args(启动参数)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["list","start","kill"]},
            "name":{"type":"string","description":"进程名（list/kill）或应用路径/命令（start）"},
            "args":{"type":"string","description":"启动参数"}
        },"required":["action"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                if cfg!(not(target_os = "windows")) {
                    return err("process_manage 当前仅支持 Windows".into(), "NOT_AVAILABLE");
                }
                let action = args.get("action").cloned().unwrap_or_default();
                let name = args.get("name").cloned().unwrap_or_default();
                let ps = match action.as_str() {
                    "list" => {
                        if name.is_empty() {
                            "Get-Process | Where-Object { $_.MainWindowTitle -ne '' -or $_.CPU -gt 1 } | Sort-Object CPU -Descending | Select-Object -First 50 | ForEach-Object { \"$($_.Id)|$($_.ProcessName)|$([math]::Round($_.CPU,1))s|$($_.MainWindowTitle)\" }".to_string()
                        } else {
                            let name_q = ps_single_quote(&name);
                            format!("Get-Process -Name '*{name_q}*' -ErrorAction SilentlyContinue | ForEach-Object {{ \"$($_.Id)|$($_.ProcessName)|$([math]::Round($_.CPU,1))s\" }}")
                        }
                    }
                    "start" => {
                        if name.is_empty() {
                            return err("start 需要 name 参数（应用路径或命令）".into(), "INVALID_PARAMS");
                        }
                        let args_str = args.get("args").cloned().unwrap_or_default();
                        let name_q = ps_single_quote(&name);
                        if args_str.is_empty() {
                            format!("Start-Process '{name_q}'; 'started: {name_q}'")
                        } else {
                            let args_q = ps_single_quote(&args_str);
                            format!("Start-Process '{name_q}' -ArgumentList '{args_q}'; 'started: {name_q} {args_q}'")
                        }
                    }
                    "kill" => {
                        if name.is_empty() {
                            return err("kill 需要 name 参数（进程名或 PID）".into(), "INVALID_PARAMS");
                        }
                        let name_q = ps_single_quote(&name);
                        // `-Id` 只接受数字：仅当 name 确实是数字时才拼进去。
                        // 历史缺陷：无条件 `-Id {name}`，把模型可控的字符串原样塞进
                        // `-Id`（注入点）；非数字时该语句本身也会报错。
                        let by_id = match name.trim().parse::<u64>() {
                            Ok(pid) => format!(
                                "; Stop-Process -Id {pid} -Force -ErrorAction SilentlyContinue"
                            ),
                            Err(_) => String::new(),
                        };
                        format!("Stop-Process -Name '*{name_q}*' -Force -ErrorAction SilentlyContinue{by_id}; 'killed: {name_q}'")
                    }
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                run_powershell(&ps).await
            })
        }),
    )
}

/// `office_automation` 的输入解析结果：扩展名 + 导出的 PDF 路径 + 本次动作会产生哪些文件。
///
/// 抽成纯函数是为了能单测「产物路径推导」——它此前只内联在 PowerShell 模板里，
/// 于是「office 工具生成的 pptx/pdf 不进产物面板」这个缺陷没有任何测试覆盖。
pub(crate) struct OfficeOutputs {
    /// 小写扩展名（无扩展名为空串）。
    pub ext: String,
    /// `export_pdf` 落盘路径（其余动作下无意义）。
    pub pdf_path: String,
    /// 本次动作成功后会落盘的文件（`open`/`save` 为空）。
    pub produced: Vec<String>,
}

pub(crate) fn office_action_outputs(action: &str, path: &str) -> OfficeOutputs {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let pdf_path = format!("{}pdf", path.trim_end_matches(ext.as_str()));
    let produced = match action {
        "create_word" | "create_excel" | "create_ppt" => vec![path.to_string()],
        "export_pdf" => vec![pdf_path.clone()],
        _ => Vec::new(),
    };
    OfficeOutputs {
        ext,
        pdf_path,
        produced,
    }
}

/// PPT 单页：标题 + 正文。`split_ppt_slides` 的产物，直接序列化为 JSON 载荷。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct PptSlide {
    pub title: String,
    pub body: String,
}

/// 把纯文本内容切分为 PPT 分页（**纯函数**，便于单测）。
///
/// 约定：
/// - 独占一行的 `---` 是分页符；
/// - 以 `# ` / `## ` 开头的 Markdown 标题行各起一页，标题文本作该页标题，后续行为正文；
/// - **无任何标记**时退化为单页（标题取首个非空行，其余为正文），保持既有单页行为；
/// - 逐行 `trim`、丢弃行尾空行；**永不**返回空 Vec（空/全空白内容也至少一页）。
pub(crate) fn split_ppt_slides(content: &str) -> Vec<PptSlide> {
    #[derive(Default)]
    struct Raw {
        title: Option<String>,
        lines: Vec<String>,
    }
    let mut sections: Vec<Raw> = Vec::new();
    let mut current = Raw::default();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line == "---" {
            sections.push(std::mem::take(&mut current));
        } else if let Some(heading) = line.strip_prefix("## ").or_else(|| line.strip_prefix("# ")) {
            sections.push(std::mem::take(&mut current));
            current.title = Some(heading.trim().to_string());
        } else {
            current.lines.push(line.to_string());
        }
    }
    sections.push(current);

    let mut slides: Vec<PptSlide> = Vec::new();
    for mut sec in sections {
        while sec.lines.last().is_some_and(String::is_empty) {
            sec.lines.pop();
        }
        if let Some(title) = sec.title.filter(|t| !t.is_empty()) {
            while sec.lines.first().is_some_and(String::is_empty) {
                sec.lines.remove(0);
            }
            slides.push(PptSlide {
                title,
                body: sec.lines.join("\n"),
            });
        } else if let Some(i) = sec.lines.iter().position(|l| !l.is_empty()) {
            // 无显式标题（含无标记的单页）：首行作标题，其余作正文。
            slides.push(PptSlide {
                title: sec.lines[i].clone(),
                body: sec.lines[i + 1..].join("\n"),
            });
        }
        // 完全空节（行首/行尾的 `---`）直接丢弃。
    }
    if slides.is_empty() {
        slides.push(PptSlide {
            title: String::new(),
            body: String::new(),
        });
    }
    slides
}

/// 办公自动化工具：操控 WPS/Office（新建文档、打开文件、保存、导出 PDF）
pub(crate) fn office_automation_tool() -> Box<dyn ToolExecutor> {
    boxed(
        "office_automation",
        "办公软件自动化（WPS/MS Office COM）：创建/打开 Word/Excel/PPT 文档，写入内容，保存，导出 PDF。action=create_word|create_excel|create_ppt|open|save|export_pdf。参数: action(必填), path(文件路径), content(写入内容，Word/PPT 支持纯文本), data(Excel 数据，JSON 二维数组)。",
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create_word","create_excel","create_ppt","open","save","export_pdf"]},
            "path":{"type":"string","description":"文件保存路径（.docx/.xlsx/.pptx）"},
            "content":{"type":"string","description":"要写入的文本内容（Word/PPT）"},
            "data":{"type":"string","description":"Excel 数据（JSON 二维数组，如 [[\"A1\",\"B1\"],[\"A2\",\"B2\"]]）"}
        },"required":["action","path"]}),
        Arc::new(move |args| {
            Box::pin(async move {
                let action = args.get("action").cloned().unwrap_or_default();
                let path = args.get("path").cloned().unwrap_or_default();
                if path.is_empty() {
                    return err("path 参数必填".into(), "INVALID_PARAMS");
                }
                // 非 Windows：走跨平台 Python 回退（运行时探测办公库，缺失则给可操作错误）。
                if cfg!(not(target_os = "windows")) {
                    return python_office_fallback(&action, &path, args).await;
                }
                // 产物登记用：create_* 直接落 `path`；export_pdf 落同目录同名 `.pdf`。
                // 该工具此前**完全不登记产物**，于是「office_automation 生成的 pptx/pdf」
                // 在产物面板里永远看不到（与 shell_exec 同一类缺陷，见 tools/file_change.rs）。
                let OfficeOutputs {
                    ext,
                    pdf_path,
                    produced,
                } = office_action_outputs(&action, &path);
                // 路径是模型可控输入：经单引号转义后再拼进脚本（见 ps_single_quote）。
                let path_q = ps_single_quote(&path);
                let pdf_q = ps_single_quote(&pdf_path);
                let mut temp_payload: Option<std::path::PathBuf> = None;
                let ps = match action.as_str() {
                    "create_word" => {
                        let content = args.get("content").cloned().unwrap_or_default();
                        let content_q = match stage_payload(&content, "doc", &mut temp_payload) {
                            Ok(q) => q,
                            Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
                        };
                        format!(r#"
$w = New-Object -ComObject Word.Application
$w.Visible = $false
$doc = $w.Documents.Add()
$doc.Content.Text = (Get-Content -Raw -Encoding UTF8 '{content_q}')
$doc.SaveAs2('{path_q}')
$doc.Close()
$w.Quit()
'created: {path_q}'"#)
                    }
                    "create_excel" => {
                        let data = args.get("data").cloned().unwrap_or_else(|| "[]".to_string());
                        let data_q = match stage_payload(&data, "xls", &mut temp_payload) {
                            Ok(q) => q,
                            Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
                        };
                        format!(r#"
$e = New-Object -ComObject Excel.Application
$e.Visible = $false
$wb = $e.Workbooks.Add()
$ws = $wb.ActiveSheet
$rows = (Get-Content -Raw -Encoding UTF8 '{data_q}') | ConvertFrom-Json
$r = 1
foreach ($row in $rows) {{ $c = 1; foreach ($cell in $row) {{ $ws.Cells.Item($r,$c) = $cell; $c++ }}; $r++ }}
$wb.SaveAs('{path_q}')
$wb.Close()
$e.Quit()
'created: {path_q}'"#)
                    }
                    "create_ppt" => {
                        let content = args.get("content").cloned().unwrap_or_default();
                        // 分页后把 slides 序列化成 JSON 走临时文件通道（避免命令行长度/注入）。
                        let slides = split_ppt_slides(&content);
                        let json = serde_json::to_string(&slides)
                            .unwrap_or_else(|_| "[]".to_string());
                        let payload_q = match stage_payload(&json, "ppt", &mut temp_payload) {
                            Ok(q) => q,
                            Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
                        };
                        format!(r#"
$p = New-Object -ComObject PowerPoint.Application
$pres = $p.Presentations.Add()
$slides = (Get-Content -Raw -Encoding UTF8 '{payload_q}') | ConvertFrom-Json
$i = 1
foreach ($s in $slides) {{
    $slide = $pres.Slides.Add($i, 2)
    $slide.Shapes.Item(1).TextFrame.TextRange.Text = [string]$s.title
    if ($slide.Shapes.Count -ge 2) {{
        $slide.Shapes.Item(2).TextFrame.TextRange.Text = [string]$s.body
    }} else {{
        $tb = $slide.Shapes.AddTextbox(1, 50, 120, 600, 300)
        $tb.TextFrame.TextRange.Text = [string]$s.body
    }}
    $i++
}}
$pres.SaveAs('{path_q}')
$pres.Close()
$p.Quit()
'created: {path_q}'"#)
                    }
                    "open" => format!("Start-Process '{path_q}'; 'opened: {path_q}'"),
                    "save" => "'save 请通过 create_* 动作保存'".to_string(),
                    "export_pdf" => {
                        match ext.as_str() {
                            "docx" | "doc" => format!(r#"
$w = New-Object -ComObject Word.Application
$w.Visible = $false
$doc = $w.Documents.Open('{path_q}')
$doc.SaveAs2('{pdf_q}', 17)
$doc.Close()
$w.Quit()
'exported: {pdf_q}'"#),
                            "xlsx" | "xls" => format!(r#"
$e = New-Object -ComObject Excel.Application
$e.Visible = $false
$wb = $e.Workbooks.Open('{path_q}')
$wb.ExportAsFixedFormat(0, '{pdf_q}')
$wb.Close()
$e.Quit()
'exported: {pdf_q}'"#),
                            "pptx" | "ppt" => format!(r#"
$p = New-Object -ComObject PowerPoint.Application
$pres = $p.Presentations.Open('{path_q}', $true, $false, $false)
$pres.SaveAs('{pdf_q}', 32)
$pres.Close()
$p.Quit()
'exported: {pdf_q}'"#),
                            _ => return err(format!("不支持的文件类型: {ext}"), "INVALID_PARAMS"),
                        }
                    }
                    _ => return err(format!("未知 action: {action}"), "INVALID_PARAMS"),
                };
                let result = run_powershell(&ps).await;
                if let Some(tf) = temp_payload {
                    let _ = std::fs::remove_file(tf);
                }
                // 只在命令真的成功、且文件确实落盘时登记，避免登记「算出了路径但生成失败」的假产物。
                if matches!(result, Ok(ToolResult::Success { .. })) {
                    for p in &produced {
                        if std::path::Path::new(p).is_file() {
                            crate::tools::file_change::record_artifact(
                                &current_session(),
                                p,
                                "created",
                            );
                        }
                    }
                }
                result
            })
        }),
    )
}

/// `office_automation` 的**跨平台回退**：非 Windows 上借助 Python 办公库生成文件。
///
/// 运行时探测解释器与模块（`python-pptx` / `openpyxl` / `python-docx`），可用则生成，
/// 否则返回点名缺失模块的可操作错误。**不引入任何构建期依赖**，找不到就优雅降级。
async fn python_office_fallback(
    action: &str,
    path: &str,
    args: &std::collections::HashMap<String, String>,
) -> Result<ToolResult> {
    let OfficeOutputs { produced, .. } = office_action_outputs(action, path);
    // action → (Python 导入名, pip 包名)。
    let (module, pip) = match action {
        "create_word" => ("docx", "python-docx"),
        "create_excel" => ("openpyxl", "openpyxl"),
        "create_ppt" => ("pptx", "python-pptx"),
        _ => {
            return err(
                format!(
                    "office_automation 的 action `{action}` 在非 Windows 上不可用；\
                     仅支持 create_word/create_excel/create_ppt（导出 PDF 需本机 Office/WPS）"
                ),
                "NOT_AVAILABLE",
            );
        }
    };
    let interp = match find_python_with_module(module).await {
        Some(i) => i,
        None => {
            return err(
                format!(
                    "office_automation 在非 Windows 上需要 Python 模块 `{pip}`，\
                     但未检测到可用的 Python 或该模块缺失。请先安装：pip install {pip}"
                ),
                "NOT_AVAILABLE",
            );
        }
    };
    // 内容经唯一临时文件作为进程间通道（避免命令行长度与注入问题）。
    let (script, payload_path, temp_payload) = match action {
        "create_word" => {
            let content = args.get("content").cloned().unwrap_or_default();
            let p = match write_temp_payload(&content, "doc") {
                Ok(p) => p,
                Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
            };
            let s = "import sys, docx\n\
                     d = docx.Document()\n\
                     d.add_paragraph(open(sys.argv[1], encoding='utf-8').read())\n\
                     d.save(sys.argv[2])\n";
            (s, p.to_string_lossy().to_string(), Some(p))
        }
        "create_excel" => {
            let data = args.get("data").cloned().unwrap_or_else(|| "[]".to_string());
            let p = match write_temp_payload(&data, "xls") {
                Ok(p) => p,
                Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
            };
            let s = "import sys, json, openpyxl\n\
                     rows = json.load(open(sys.argv[1], encoding='utf-8'))\n\
                     wb = openpyxl.Workbook()\n\
                     ws = wb.active\n\
                     for r, row in enumerate(rows, 1):\n\
                     \x20   for c, val in enumerate(row, 1):\n\
                     \x20       ws.cell(row=r, column=c, value=val)\n\
                     wb.save(sys.argv[2])\n";
            (s, p.to_string_lossy().to_string(), Some(p))
        }
        "create_ppt" => {
            let content = args.get("content").cloned().unwrap_or_default();
            // 分页后序列化为 JSON；Python 侧逐页 add_slide。
            let slides = split_ppt_slides(&content);
            let json = serde_json::to_string(&slides).unwrap_or_else(|_| "[]".to_string());
            let p = match write_temp_payload(&json, "ppt") {
                Ok(p) => p,
                Err(e) => return err(format!("写入临时内容失败: {e}"), "IO_ERROR"),
            };
            let s = "import sys, json\n\
                     from pptx import Presentation\n\
                     from pptx.util import Inches\n\
                     slides = json.load(open(sys.argv[1], encoding='utf-8'))\n\
                     prs = Presentation()\n\
                     for s in slides:\n\
                     \x20   layout = prs.slide_layouts[1] if len(prs.slide_layouts) > 1 else prs.slide_layouts[6]\n\
                     \x20   slide = prs.slides.add_slide(layout)\n\
                     \x20   title = s.get('title', '')\n\
                     \x20   body = s.get('body', '')\n\
                     \x20   if slide.shapes.title is not None:\n\
                     \x20       slide.shapes.title.text = title\n\
                     \x20   else:\n\
                     \x20       tb = slide.shapes.add_textbox(Inches(0.5), Inches(0.5), Inches(9), Inches(1))\n\
                     \x20       tb.text_frame.text = title\n\
                     \x20   if len(slide.placeholders) > 1:\n\
                     \x20       slide.placeholders[1].text = body\n\
                     \x20   else:\n\
                     \x20       tb = slide.shapes.add_textbox(Inches(0.5), Inches(1.5), Inches(9), Inches(5))\n\
                     \x20       tb.text_frame.text = body\n\
                     prs.save(sys.argv[2])\n";
            (s, p.to_string_lossy().to_string(), Some(p))
        }
        _ => unreachable!("已在上面按 action 过滤"),
    };
    let result = run_python(&interp, script, &[&payload_path, path]).await;
    if let Some(tf) = temp_payload {
        let _ = std::fs::remove_file(tf);
    }
    // 仅当命令成功且文件确实落盘时才登记，避免登记假产物。
    if matches!(result, Ok(ToolResult::Success { .. })) {
        for p in &produced {
            if std::path::Path::new(p).is_file() {
                crate::tools::file_change::record_artifact(&current_session(), p, "created");
            }
        }
    }
    result
}

/// 依次尝试常见解释器命令，返回第一个能 `import <module>` 成功的命令名。
async fn find_python_with_module(module: &str) -> Option<String> {
    for interp in ["python3", "python", "py"] {
        let code = format!("import {module}");
        let probe = tokio::process::Command::new(interp)
            .silent()
            .args(["-c", code.as_str()])
            .output()
            .await;
        if let Ok(o) = probe {
            if o.status.success() {
                return Some(interp.to_string());
            }
        }
    }
    None
}

/// 运行 Python 脚本（脚本经 `-c` 传入，数据路径经 argv 传入，无 shell 插值）。
async fn run_python(interp: &str, script: &str, args: &[&str]) -> Result<ToolResult> {
    let mut cmd = tokio::process::Command::new(interp);
    cmd.silent().arg("-c").arg(script);
    for a in args {
        cmd.arg(a);
    }
    match cmd.output().await {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if o.status.success() {
                ok(if stdout.is_empty() {
                    "操作完成".to_string()
                } else {
                    stdout
                })
            } else {
                err(format!("{stdout}\n{stderr}"), "PYTHON_ERROR")
            }
        }
        Err(e) => err(format!("执行失败: {e}"), "EXEC_ERROR"),
    }
}

/// 把任意字符串转义为 **PowerShell 单引号字面量**的内部内容。
///
/// 单引号字符串是 PowerShell 中**唯一不做任何插值**的字符串形式：`$`、反引号、
/// 双引号都按字面处理，只有 `'` 需要写成 `''`。因此模型可控的路径 / 窗口标题 /
/// 进程名只要经此函数转义并包进 `'…'`，就无法再跳出字符串执行任意命令。
///
/// ⚠️ 历史缺陷：这些值此前被**原样**拼进 `-Command` 脚本（有的还在双引号里），
/// 于是一个含 `'` 或 `"` 的路径/标题即可注入任意 PowerShell —— 远程代码执行。
/// 新增任何拼接 PowerShell 的参数时，必须走本函数（短值）；
/// 长文本/大数据走唯一临时文件通道（见 [`write_temp_payload`]），
/// **不要**再用进程级环境变量传值（并发子代理会互相覆盖）。
pub(crate) fn ps_single_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// 单调递增计数器：为临时载荷文件生成进程内唯一名。
static PAYLOAD_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 把内容写入**唯一临时文件**并返回路径。
///
/// 历史缺陷：内容经进程级 env（`MARG_OFFICE_CONTENT` / `MARG_CLIP_TEXT` 等）传给
/// PowerShell。`execute_parallel` 下并发子代理会互相覆盖同一个 env 变量 → 内容串台
/// （A 的文档被写进 B 的内容）。改用一次性临时文件作为进程间通道：与并发无关，
/// 且不受命令行长度上限约束（长文档/大 JSON 也安全）。
pub(crate) fn write_temp_payload(content: &str, tag: &str) -> std::io::Result<std::path::PathBuf> {
    let seq = PAYLOAD_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("marg_{tag}_{}_{seq}.txt", std::process::id()));
    std::fs::write(&p, content)?;
    Ok(p)
}

/// 写入临时载荷，返回**已转义**的路径（供脚本单引号串插值），并登记待清理文件。
fn stage_payload(
    content: &str,
    tag: &str,
    slot: &mut Option<std::path::PathBuf>,
) -> std::io::Result<String> {
    let p = write_temp_payload(content, tag)?;
    let q = ps_single_quote(&p.to_string_lossy());
    *slot = Some(p);
    Ok(q)
}

/// 运行 PowerShell 脚本并返回结果（静默窗口）
pub(crate) async fn run_powershell(script: &str) -> Result<ToolResult> {
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        // 先设置 UTF-8 输出编码，避免中文路径/输出乱码
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $OutputEncoding=[System.Text.Encoding]::UTF8; ",
    ]);
    cmd.arg(script);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    match cmd.output().await {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if o.status.success() {
                ok(if stdout.is_empty() {
                    "操作完成".to_string()
                } else {
                    stdout
                })
            } else {
                err(format!("{stdout}\n{stderr}"), "POWERSHELL_ERROR")
            }
        }
        Err(e) => err(format!("执行失败: {e}"), "EXEC_ERROR"),
    }
}

#[cfg(test)]
mod office_outputs_tests {
    use super::*;

    /// 新建类动作的产物就是 `path` 本身，扩展名要能解析出来（决定 COM 分支）。
    #[test]
    fn create_actions_produce_the_requested_path() {
        for (action, file, want_ext) in [
            ("create_word", "报告.docx", "docx"),
            ("create_excel", "data.xlsx", "xlsx"),
            ("create_ppt", "presentation.pptx", "pptx"),
        ] {
            let out = office_action_outputs(action, file);
            assert_eq!(out.ext, want_ext);
            assert_eq!(out.produced, vec![file.to_string()]);
        }
    }

    /// 临时载荷通道必须**每次唯一**且内容正确 —— 这是消除并发串台的关键。
    #[test]
    fn temp_payload_is_unique_and_faithful() {
        let a = write_temp_payload("内容 A 'quoted'", "doc").expect("write A");
        let b = write_temp_payload("内容 A 'quoted'", "doc").expect("write B");
        assert_ne!(a, b, "两次调用必须得到不同文件（否则并发仍会串台）");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "内容 A 'quoted'");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "内容 A 'quoted'");
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    /// 单引号转义是内联路径/文本的唯一安全通道。
    #[test]
    fn ps_single_quote_escapes_quote() {
        assert_eq!(ps_single_quote("a'b"), "a''b");
        assert_eq!(ps_single_quote("plain"), "plain");
    }

    /// export_pdf 的产物是**去掉原扩展名后加 pdf**（真机案例：presentation.pptx → presentation.pdf）。
    #[test]
    fn export_pdf_produces_sibling_pdf() {
        let out = office_action_outputs("export_pdf", "C:/ws/presentation.pptx");
        assert_eq!(out.ext, "pptx");
        assert_eq!(out.pdf_path, "C:/ws/presentation.pdf");
        assert_eq!(out.produced, vec!["C:/ws/presentation.pdf".to_string()]);
    }

    /// open / save 不产生新文件 —— 不得凭空登记产物。
    #[test]
    fn open_and_save_produce_nothing() {
        assert!(office_action_outputs("open", "a.pptx").produced.is_empty());
        assert!(office_action_outputs("save", "a.pptx").produced.is_empty());
    }

    /// 安全守卫：模型可控的路径/标题/进程名必须能被安全地包进 PowerShell 单引号串。
    ///
    /// 历史缺陷：这些值被**原样**拼进 `-Command`，含 `'` 的路径即可闭合字符串注入
    /// 任意 PowerShell（RCE）。单引号串里唯一需要转义的是 `'` → `''`。
    #[test]
    fn ps_single_quote_neutralizes_injection() {
        // 单引号被双写，无法闭合字符串。
        assert_eq!(ps_single_quote("a'b"), "a''b");
        // 注入载荷：闭合 + 执行任意命令 → 转义后每个 `'` 都被双写，整段退化为字面量。
        let payload = "x'; Remove-Item -Recurse C:\\ ; '";
        let escaped = ps_single_quote(payload);
        // 等价性判据：把成对的 `''` 还原即得原文 → 说明没有落单的 `'` 能闭合字符串。
        assert_eq!(escaped.replace("''", "'"), payload);
        assert_eq!(escaped.matches('\'').count(), payload.matches('\'').count() * 2);
        // 中文路径 / 反引号 / `$` 均无需转义（单引号串不做插值）。
        assert_eq!(ps_single_quote("报告$v`x.docx"), "报告$v`x.docx");
    }

    /// E2：跨平台回退对不支持的动作给出可操作的 NOT_AVAILABLE（不依赖本机 Python）。
    #[tokio::test]
    async fn python_fallback_rejects_unsupported_action() {
        let args = std::collections::HashMap::new();
        let r = python_office_fallback("export_pdf", "out.pdf", &args).await;
        match r {
            Ok(ToolResult::Error { code, message }) => {
                assert_eq!(code, "NOT_AVAILABLE");
                assert!(
                    message.contains("export_pdf"),
                    "错误信息应点名动作，实际: {message}"
                );
            }
            other => panic!("期望 NOT_AVAILABLE 错误，实际 {other:?}"),
        }
    }

    /// 无任何标记 → 单页（保持既有行为）：标题取首行，其余为正文。
    #[test]
    fn ppt_no_markers_is_single_slide() {
        let slides = split_ppt_slides("Deck Title\nline one\nline two");
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].title, "Deck Title");
        assert_eq!(slides[0].body, "line one\nline two");
    }

    /// `---` 独占一行即分页；每页首行作标题，其余作正文。
    #[test]
    fn ppt_page_breaks_split_slides() {
        let slides = split_ppt_slides("Page 1 title\nbody a\n---\nPage 2 title\nbody b\n---\nPage 3");
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].title, "Page 1 title");
        assert_eq!(slides[0].body, "body a");
        assert_eq!(slides[1].title, "Page 2 title");
        assert_eq!(slides[1].body, "body b");
        assert_eq!(slides[2].title, "Page 3");
        assert_eq!(slides[2].body, "");
    }

    /// `# `/`## ` 标题各起一页，标题文本即该页标题。
    #[test]
    fn ppt_headings_become_titles() {
        let slides = split_ppt_slides("# Intro\nwelcome\n## Details\nsome detail");
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].title, "Intro");
        assert_eq!(slides[0].body, "welcome");
        assert_eq!(slides[1].title, "Details");
        assert_eq!(slides[1].body, "some detail");
    }

    /// 空/全空白内容不 panic，且至少一页（不返回空 Vec）。
    #[test]
    fn ppt_empty_content_yields_one_slide() {
        let empty = split_ppt_slides("");
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].title, "");
        assert_eq!(empty[0].body, "");

        let blank = split_ppt_slides("   \n\n");
        assert_eq!(blank.len(), 1, "全空白也应退化为单页");
    }

    /// 标题与 `---` 混用：行首/行尾的 `---` 产生的空节被丢弃。
    #[test]
    fn ppt_mixed_headings_and_breaks() {
        let slides = split_ppt_slides("# Deck\n---\n## Section A\na1\na2\n---\n## Section B\nb1");
        let titles: Vec<&str> = slides.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Deck", "Section A", "Section B"]);
        assert_eq!(slides[0].body, "");
        assert_eq!(slides[1].body, "a1\na2");
        assert_eq!(slides[2].body, "b1");
    }
}

