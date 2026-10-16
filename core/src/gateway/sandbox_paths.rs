//! 沙箱路径方法族（sandbox.path.*）
//!
//! 提供目录选择器（pick）、目录浏览（list）与新建目录（create-directory）。
//! 纯 Rust 本地实现，不依赖 Python 侧车。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::proc::SilentCommand;

/// 解析 `sandbox.path.*` 的目标根目录：优先 HOME/USERPROFILE，回退进程 cwd。
pub fn sandbox_path_root() -> PathBuf {
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("HOME") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    std::path::PathBuf::from(".")
}

/// `sandbox.path.pick`：桌面端原生目录选择器（Rust 本地实现）。
///
/// 历史问题：该 RPC 未在 Rust 实现 → dispatch 回退代理 Python 侧车；侧车在
/// Windows 上 spawn 一个 tkinter 子进程弹「选择目录」对话框，且以
/// CREATE_NO_WINDOW 方式创建，在无窗口会话下永久阻塞 → WS 代理超时
/// （「Python 侧车响应超时（sandbox.path.pick）」），目录选择器无法打开。
///
/// 现改为 Rust 直接以 PowerShell + System.Windows.Forms.FolderBrowserDialog
/// 弹原生目录对话框（STA 线程，与 system_control 同源的纯系统 API）：
/// - 不依赖 Python 侧车（离线也可用）；
/// - 异步 await 子进程，不阻塞网关事件循环；
/// - 用户取消 / 无桌面会话 → 返回 `path: null`，WebUI 回退到路径浏览。
pub async fn sandbox_path_pick(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    if cfg!(not(target_os = "windows")) {
        return Ok(json!({ "path": null, "kind": "workspace" }));
    }
    let initial = params
        .get("initialPath")
        .and_then(Value::as_str)
        .unwrap_or("");
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Windows.Forms | Out-Null
$d = New-Object System.Windows.Forms.FolderBrowserDialog
$d.Description = 'Marguerite · 选择项目目录'
$d.ShowNewFolderButton = $true
$initial = $env:MARG_PICK_INITIAL
if ($initial) { try { $d.SelectedPath = $initial } catch {} }
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    # 以 UTF-8 裸字节写出选中路径，规避 PowerShell 控制台默认 GBK(codepage 936)
    # 编码导致的中文路径乱码：Rust 侧用 from_utf8_lossy 按 UTF-8 读取 stdout。
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($d.SelectedPath + [char]10)
    $stdout = [System.Console]::OpenStandardOutput()
    $stdout.Write($bytes, 0, $bytes.Length)
    $stdout.Flush()
}
"#;
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.silent().args([
        "-NoProfile",
        "-NonInteractive",
        "-STA",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    cmd.env("MARG_PICK_INITIAL", initial);
    // 目录对话框可能长时间打开（用户人工选择），上限 15 分钟
    let out = tokio::time::timeout(std::time::Duration::from_secs(900), cmd.output())
        .await
        .map_err(|_| RpcError::new("PICK_TIMEOUT", "目录选择超时（15 分钟内未完成选择）"))?
        .map_err(|e| RpcError::new("PICK_SPAWN", format!("无法启动目录选择器: {e}")))?;
    if !out.status.success() {
        return Ok(json!({ "path": null, "kind": "workspace" }));
    }
    let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(json!({
        "path": if selected.is_empty() { Value::Null } else { Value::String(selected) },
        "kind": "workspace",
    }))
}

/// `sandbox.path.list`：Rust 本地目录浏览（项目工作区选择器「平台目录」加载）。
pub async fn sandbox_path_list(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    let raw_path = params
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let raw_base = params
        .get("basePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let target: PathBuf = if !raw_path.is_empty() && std::path::Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else if !raw_path.is_empty() && !raw_base.is_empty() {
        std::path::Path::new(raw_base).join(raw_path)
    } else if !raw_base.is_empty() {
        PathBuf::from(raw_base)
    } else if !raw_path.is_empty() {
        sandbox_path_root().join(raw_path)
    } else {
        sandbox_path_root()
    };

    if !target.exists() {
        return Err(RpcError::new(
            "PATH_NOT_FOUND",
            format!("目录不存在：{}", target.to_string_lossy()),
        ));
    }
    if !target.is_dir() {
        return Err(RpcError::new(
            "NOT_A_DIRECTORY",
            format!("路径不是目录：{}", target.to_string_lossy()),
        ));
    }

    let mut entries: Vec<Value> = Vec::new();
    match std::fs::read_dir(&target) {
        Ok(rd) => {
            for entry in rd.filter_map(Result::ok) {
                let fname = entry.file_name().to_string_lossy().to_string();
                let fpath = entry.path();
                let is_dir = fpath.is_dir();
                entries.push(json!({
                    "name": fname,
                    "path": fpath.to_string_lossy(),
                    "kind": if is_dir { "directory" } else { "file" },
                    "selectable": is_dir,
                    "hidden": fname.starts_with('.'),
                }));
            }
        }
        Err(e) => {
            return Err(RpcError::new(
                "READ_DIR_FAILED",
                format!("无法读取目录 {}：{e}", target.to_string_lossy()),
            ));
        }
    }
    // 目录优先、再按名称排序
    entries.sort_by(|a, b| {
        let da = a.get("kind").and_then(Value::as_str) == Some("directory");
        let db = b.get("kind").and_then(Value::as_str) == Some("directory");
        match (da, db) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a
                .get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str)),
        }
    });

    let current = target.to_string_lossy().to_string();
    let parent = target.parent().map(|p| p.to_string_lossy().to_string());
    Ok(json!({
        "currentPath": current,
        "path": current,
        "parentPath": parent,
        "systemPickerAvailable": cfg!(target_os = "windows"),
        "entries": entries,
    }))
}

/// `sandbox.path.create-directory`：Rust 本地新建子目录（工作区选择器「新建文件夹」）。
pub async fn sandbox_path_create_directory(
    params: &Value,
) -> Result<Value, crate::gateway::protocol::RpcError> {
    use crate::gateway::protocol::RpcError;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return Err(RpcError::new("INVALID_REQUEST", "name 不可为空"));
    }
    let raw_parent = params
        .get("parentPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let base: PathBuf = if raw_parent.is_empty() {
        sandbox_path_root()
    } else {
        PathBuf::from(raw_parent)
    };
    let created = base.join(name);
    std::fs::create_dir_all(&created).map_err(|e| {
        RpcError::new(
            "MKDIR_FAILED",
            format!("创建目录失败 {}：{e}", created.to_string_lossy()),
        )
    })?;
    Ok(json!({ "path": created.to_string_lossy().to_string() }))
}
