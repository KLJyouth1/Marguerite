//! SSH 一键部署（Phase I2，FR-16）——目录模块（M0.5 拆分自单文件 `ssh.rs`，OD-003）。
//!
//! `marguerite deploy --target ssh://[user@]host[:port][/dir] [--bin <path>]
//!   [--service <name>] [--health-url <url>] [--no-systemd] [--dry-run]
//!   [--user <u>] [--port <p>] [--identity <key>]`
//!
//! 传输层抽象为 [`SshTransport`] trait：真实 [`SystemSshTransport`] 调用系统 `ssh`/`scp`
//! （密钥免密，禁止密码交互）；[`RecordingTransport`] 用于单测与 `--dry-run` 演练。
//!
//! 拆分后单一职责分文件（公开 API 经本模块 re-export 保持不变，调用方零改动）：
//! - [`target`]：`SshTarget`（ssh:// URL）与 `HostSpec`（user@host:port）解析
//! - [`transport`]：传输抽象 + 真实实现 + 批命令执行（`exec_batch`）
//! - [`recording`]：演练/单测用记录型传输
//! - [`policy`]：命令幂等性判定 + 长输出折叠
//! - [`deploy`]：部署编排（推送/安装/探活/回滚）+ sha256 校验
//! - [`cluster`]：多主机顺序执行
//! - [`tunnel`]：本地端口转发隧道状态与端口管理

pub mod cluster;
pub mod deploy;
pub mod policy;
pub mod recording;
pub mod target;
pub mod transport;
pub mod tunnel;

// ---- 公开 API re-export（与拆分前单文件 `deploy/ssh.rs` 完全一致）----
pub use cluster::{ClusterHost, ClusterResult, SshCluster};
pub use deploy::{sha256_file, DeployOptions, DeployReport, SshDeployer};
pub use policy::{
    idempotency_warning, is_idempotent_command, snip_output, OUTPUT_SNIP_HEAD, OUTPUT_SNIP_TAIL,
};
pub use recording::RecordingTransport;
pub use target::{parse_hostspec, HostSpec, SshTarget};
pub use transport::{exec_batch, SshTransport, SystemSshTransport};
pub use tunnel::{SshTunnel, SshTunnelRegistry, TunnelState};
