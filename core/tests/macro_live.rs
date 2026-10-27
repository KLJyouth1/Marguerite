//! 录放闭环的**真实**端到端验证（默认 `#[ignore]`：会移动鼠标、起 PowerShell）。
//!
//! 运行方式：
//! ```text
//! cargo test -p marguerite-core --test macro_live -- --ignored --nocapture
//! ```
//!
//! 验证点（缺一不可）：
//! 1. `build_record_script` 产出的脚本能被 PowerShell 真实编译并装上全局钩子；
//! 2. 录制期间真实发生的鼠标移动会被写进 JSONL（不是空文件）；
//! 3. `build_replay_script` 产出的脚本能真实执行并按时序重放（stdout 有回执）；
//! 4. 哨兵文件能跨进程让录制进程干净退出（不留后台进程）。
//!
//! 为什么值得单独一个集成测试：单测只能断言「生成的字符串里有某几行」，
//! 而这类缺陷（C# 编译失败、钩子装不上、`DoEvents` 不泵消息、脚本编码错）
//! 只有在真机上跑才会暴露 —— 正是「看起来实现了、其实一按就崩」的高发区。

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use marguerite_core::tools::macros::{
    build_record_script, build_replay_script, parse_events, MacroEvent,
};

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "mq-macro-live-{}-{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// PowerShell 5.1 读无 BOM 的 UTF-8 会按 ANSI 解码 → 中文窗口标题乱码。
fn write_script(path: &Path, content: &str) {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(content.as_bytes());
    std::fs::write(path, bytes).unwrap();
}

fn ps_file(path: &Path) -> std::process::Output {
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &path.display().to_string(),
        ])
        .output()
        .expect("启动 PowerShell 失败")
}

#[test]
#[ignore = "会真的移动鼠标 / 起 PowerShell（桌面会话内手动运行）"]
fn record_captures_real_mouse_and_replay_runs() {
    let dir = tmp_dir("rec");
    let out = dir.join("m.jsonl");
    let sentinel = dir.join("m.stop");
    let rec_ps = dir.join("rec.ps1");
    write_script(&rec_ps, &build_record_script(&out, &sentinel, 20_000, "all"));

    let mut child = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
            &rec_ps.display().to_string(),
        ])
        .spawn()
        .expect("spawn recorder");

    // 等钩子装好（Add-Type 首次编译约 0.5–2s）
    std::thread::sleep(Duration::from_millis(2500));

    // 制造**真实输入事件**：必须用 mouse_event 注入。
    // 踩过的坑：`[Cursor]::Position = ...`（即 SetCursorPos）只改光标位置、
    // 不产生低级钩子事件，用它做验证会得到「0 条事件」的假阴性。
    let inject = r#"Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class MqInj {
  [DllImport("user32.dll")] static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
  public static void Move(int x, int y) {
    uint nx = (uint)(x * 65535 / GetSystemMetrics(0));
    uint ny = (uint)(y * 65535 / GetSystemMetrics(1));
    mouse_event(0x8001, nx, ny, 0, UIntPtr.Zero);
  }
}
"@
[MqInj]::Move(320,320); Start-Sleep -Milliseconds 120
[MqInj]::Move(640,460); Start-Sleep -Milliseconds 120
[MqInj]::Move(900,520)"#;
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", inject])
        .output();
    std::thread::sleep(Duration::from_millis(400));

    // 哨兵停止
    std::fs::write(&sentinel, b"stop").unwrap();
    let started = Instant::now();
    while !out.is_file() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.wait();
    assert!(out.is_file(), "录制未落盘：{} 不存在", out.display());

    let raw = std::fs::read_to_string(&out).unwrap();
    let (events, bad) = parse_events(&raw);
    println!("录制到 {} 条事件（坏行 {}）：{:?}", events.len(), bad, &raw[..raw.len().min(400)]);
    assert!(
        events.iter().any(|e| e.k == "move"),
        "真实鼠标移动没有被捕获（钩子没装上或事件没落盘）"
    );

    // 回放：用**这次真实录到的事件**，只回放前 3 个动作，速度 8 倍（快速收尾）
    let safe: Vec<MacroEvent> = events.iter().take(3).cloned().collect();
    let play_ps = dir.join("play.ps1");
    write_script(&play_ps, &build_replay_script(&safe, 8.0, 1, None));
    let res = ps_file(&play_ps);
    let stdout = String::from_utf8_lossy(&res.stdout);
    let stderr = String::from_utf8_lossy(&res.stderr);
    println!("回放 stdout={stdout} stderr={stderr}");
    assert!(res.status.success(), "回放脚本执行失败: {stderr}");
    assert!(
        stdout.contains("replayed"),
        "回放脚本没有回执输出: {stdout}"
    );
}
