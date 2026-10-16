//! 第 2 项：WSL / Docker 可用性诊断器。
//!
//! 本机可能无 WSL / Docker；缺省时应返回 `DEGRADED`（不装也不失败），并给出安装引导。
//! 探测逻辑与 `terminal::backend` 的 `TerminalBackend::{Wsl,Docker}` 等价的
//! `wsl.exe -l -q` / `docker info` 探针保持一致（不修改 terminal 模块）。

use std::process::Command;

use crate::doctor::{DoctorItem, Status};
use crate::proc::SilentCommand;

/// 第 2 项：WSL / Docker 可用性。
pub fn check_linux_backend() -> DoctorItem {
    let wsl = probe_wsl();
    let docker = probe_docker();

    match (wsl, docker) {
        (Some(wl), Some(dk)) => DoctorItem {
            id: "backend".to_string(),
            label: "WSL / Docker".to_string(),
            status: Status::Ok,
            detail: format!("WSL={wl} Docker={dk}"),
            fix_hint: String::new(),
        },
        (Some(wl), None) => DoctorItem {
            id: "backend".to_string(),
            label: "WSL / Docker".to_string(),
            status: Status::Ok,
            detail: format!("WSL={wl} Docker=不可用"),
            fix_hint: String::new(),
        },
        (None, Some(dk)) => DoctorItem {
            id: "backend".to_string(),
            label: "WSL / Docker".to_string(),
            status: Status::Ok,
            detail: format!("WSL=不可用 Docker={dk}"),
            fix_hint: String::new(),
        },
        (None, None) => DoctorItem {
            id: "backend".to_string(),
            label: "WSL / Docker".to_string(),
            status: Status::Degraded,
            detail: "本机无 WSL 且无 Docker".to_string(),
            fix_hint: LINUX_BACKEND_HINT.to_string(),
        },
    }
}

/// 探测 `wsl.exe -l -q`，返回发行版名；失败/无则返回 `None`。
fn probe_wsl() -> Option<String> {
    // WSL 仅在 Windows 上有意义；其它平台视为不可用。
    if !cfg!(windows) {
        return None;
    }
    let out = Command::new("wsl.exe")
        .silent()
        .arg("-l")
        .arg("-q")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let distro = text.lines().find(|l| !l.trim().is_empty())?.trim();
    if distro.contains("no installed") || distro.contains("未安装") || distro.is_empty() {
        return None;
    }
    Some(distro.to_string())
}

/// 探测 `docker info`，成功返回镜像信息或 `available`；失败/无则返回 `None`。
fn probe_docker() -> Option<String> {
    let out = Command::new("docker").silent().arg("info").output().ok()?;
    if !out.status.success() {
        return None;
    }
    // `docker info` 里通常有 `Server Version`；取出来作为可用证据。
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let version = text
        .lines()
        .find(|l| l.trim().starts_with("Server Version:"))
        .map(|l| l.split(':').nth(1).map(str::trim).unwrap_or_default().to_string())
        .unwrap_or_else(|| "available".to_string());
    if version.is_empty() {
        Some("available".to_string())
    } else {
        Some(version)
    }
}

const LINUX_BACKEND_HINT: &str = "安装引导：\n1) WSL2: 管理员运行 `wsl --install`（启用 Windows 功能后重启）\n2) Docker Desktop: 从 https://www.docker.com/products/docker-desktop/ 安装并启动\n两者可只装其一；都不装则本项 DEGRADED（不阻断启动）";
