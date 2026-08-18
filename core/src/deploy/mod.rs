//! 一键部署（Phase I2，FR-16）：SSH 推送 + systemd/nohup + 健康探活 + 回滚。
//!
//! 设计要点（与 collab/gateway 同思路，传输层与编排解耦便于单测）：
//! - 传输抽象为 `SshTransport` trait；真实实现 `SystemSshTransport` 经系统 `ssh`/`scp`
//!   执行，**密钥免密**（`BatchMode=yes` 禁止密码交互，依赖 `~/.ssh` agent 或 identity 文件）。
//! - 演练/单测用 `RecordingTransport`（纯记录、可注入失败），`--dry-run` 即走此路径。
//! - 编排 `SshDeployer::deploy` 不依赖具体传输，覆盖：探针 → 建目录 → 本地 sha256 →
//!   上传 .new → 快照 .old → 就位 → 安装服务 → 健康探活 →（失败）回滚。
//!
//! `baota.rs`：宝塔面板一键部署（FR-16），自研 OpenAPI 客户端（无外部 MCP 依赖），
//!   传输抽象 `BaotaTransport`（真机 `ReqwestBaotaTransport` / 演练 `RecordingBaotaTransport`），
//!   面板别名与 API 密钥存加密配置（复用 `model::byok` AES-0600）。

pub mod baota;
pub mod precheck;
pub mod ssh;
