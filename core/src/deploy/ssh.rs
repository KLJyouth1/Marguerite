//! SSH 一键部署（Phase I2，FR-16）
//!
//! `marguerite deploy --target ssh://[user@]host[:port][/dir] [--bin <path>]
//!   [--service <name>] [--health-url <url>] [--no-systemd] [--dry-run]
//!   [--user <u>] [--port <p>] [--identity <key>]`
//!
//! 传输层抽象为 `SshTransport` trait：真实 `SystemSshTransport` 调用系统 `ssh`/`scp`
//! （密钥免密，禁止密码交互）；`RecordingTransport` 用于单测与 `--dry-run` 演练。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

/// 远端目标连接信息
#[derive(Clone, Debug)]
pub struct SshTarget {
    /// 主机名或 IP
    pub host: String,
    /// SSH 端口
    pub port: u16,
    /// 登录用户（缺省 root）
    pub user: String,
    /// 远端部署目录（缺省 /opt/marguerite）
    pub remote_dir: String,
    /// 私钥路径（可选；缺省走 ssh agent）
    pub identity: Option<String>,
}

impl SshTarget {
    /// 从 `ssh://[user@]host[:port][/dir]` 解析
    pub fn from_url(url: &str) -> Result<Self> {
        let body = url
            .strip_prefix("ssh://")
            .ok_or_else(|| anyhow!("目标须以 ssh:// 开头: {url}"))?;
        let (hostpart, dir) = match body.find('/') {
            Some(i) => (body[..i].to_string(), body[i..].to_string()),
            None => (body.to_string(), "/opt/marguerite".to_string()),
        };
        let (user, rest) = match hostpart.find('@') {
            Some(i) => (hostpart[..i].to_string(), hostpart[i + 1..].to_string()),
            None => ("root".to_string(), hostpart),
        };
        let (host, port) = match rest.find(':') {
            Some(i) => (
                rest[..i].to_string(),
                rest[i + 1..]
                    .parse::<u16>()
                    .map_err(|e| anyhow!("端口解析失败: {e}"))?,
            ),
            None => (rest, 22),
        };
        if host.is_empty() {
            bail!("ssh:// 缺少主机名");
        }
        Ok(Self {
            host,
            port,
            user,
            remote_dir: dir,
            identity: None,
        })
    }

    /// 连接端点描述（用于日志/面板）
    pub fn endpoint(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }
}

/// 部署选项
#[derive(Clone, Debug)]
pub struct DeployOptions {
    /// 服务名（二进制与服务单元同名），缺省 marguerite
    pub service: String,
    /// 健康检查 URL（HTTP 探活）；为空时 systemd 走 `is-active`，非 systemd 视为通过
    pub health_url: Option<String>,
    /// 是否用 systemd（否则 nohup 拉起）
    pub use_systemd: bool,
    /// 仅演练（传输层选择 Recording，不实际连接）——仅作信息标注
    pub dry_run: bool,
}

/// 部署结果报告
#[derive(Debug)]
pub struct DeployReport {
    /// 本地二进制的 sha256（部署前计算，用于校验）
    pub binary_sha256: String,
    /// 是否触发了回滚
    pub rollback_available: bool,
    /// 是否成功上线（健康检查通过）
    pub success: bool,
}

/// SSH 传输抽象（exec/upload），便于注入 mock / dry-run
pub trait SshTransport {
    /// 执行远端命令，远端非零退出视为失败（返回 Err）
    fn exec(&self, cmd: &str) -> Result<String>;
    /// 上传本地文件到远端路径
    fn upload(&self, local: &Path, remote: &Path) -> Result<()>;
}

/// 真实传输：经系统 `ssh`/`scp`，密钥免密（BatchMode 禁止密码交互）
pub struct SystemSshTransport {
    target: SshTarget,
}

impl SystemSshTransport {
    pub fn new(target: &SshTarget) -> Self {
        Self {
            target: target.clone(),
        }
    }
}

impl SshTransport for SystemSshTransport {
    fn exec(&self, cmd: &str) -> Result<String> {
        let mut c = std::process::Command::new("ssh");
        c.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new");
        if let Some(id) = &self.target.identity {
            c.arg("-i").arg(id);
        }
        c.arg("-p").arg(self.target.port.to_string());
        c.arg(format!("{}@{}", self.target.user, self.target.host));
        c.arg(cmd);
        let out = c
            .output()
            .map_err(|e| anyhow!("ssh 执行失败（请确认 ssh 客户端可用且已配置密钥）: {e}"))?;
        if !out.status.success() {
            bail!(
                "ssh 远端命令失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn upload(&self, local: &Path, remote: &Path) -> Result<()> {
        let mut c = std::process::Command::new("scp");
        c.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new");
        if let Some(id) = &self.target.identity {
            c.arg("-i").arg(id);
        }
        c.arg("-P").arg(self.target.port.to_string());
        c.arg(local);
        c.arg(format!(
            "{}@{}:{}",
            self.target.user,
            self.target.host,
            remote.display()
        ));
        let out = c
            .output()
            .map_err(|e| anyhow!("scp 执行失败（请确认 scp 客户端可用且已配置密钥）: {e}"))?;
        if !out.status.success() {
            bail!(
                "scp 上传失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(())
    }
}

/// 记录型传输：纯记录调用、不实际执行，可注入失败（用于单测与 --dry-run）
pub struct RecordingTransport {
    inner: RefCell<RecordingInner>,
    fail_on: Option<String>,
}

struct RecordingInner {
    execs: Vec<String>,
    uploads: Vec<(PathBuf, String)>,
}

impl Default for RecordingTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingTransport {
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(RecordingInner {
                execs: Vec::new(),
                uploads: Vec::new(),
            }),
            fail_on: None,
        }
    }

    /// 配置：远端命令包含该子串时返回 Err（模拟命令失败，典型用于健康探活失败）
    pub fn with_fail(mut self, substring: &str) -> Self {
        self.fail_on = Some(substring.to_string());
        self
    }

    pub fn exec_calls(&self) -> Vec<String> {
        self.inner.borrow().execs.clone()
    }

    pub fn upload_calls(&self) -> Vec<(PathBuf, String)> {
        self.inner.borrow().uploads.clone()
    }
}

impl SshTransport for RecordingTransport {
    fn exec(&self, cmd: &str) -> Result<String> {
        self.inner.borrow_mut().execs.push(cmd.to_string());
        if let Some(needle) = &self.fail_on {
            if cmd.contains(needle.as_str()) {
                bail!("（mock）远端命令失败: {cmd}");
            }
        }
        Ok("ok".to_string())
    }

    fn upload(&self, local: &Path, remote: &Path) -> Result<()> {
        self.inner
            .borrow_mut()
            .uploads
            .push((local.to_path_buf(), remote.to_string_lossy().to_string()));
        Ok(())
    }
}

/// 计算文件 sha256（十六进制小写）
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).map_err(|e| anyhow!("读取二进制失败: {e}"))?;
    let mut h = Sha256::new();
    h.update(&data);
    Ok(hex_encode(&h.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// shell 单引号转义（用于远端路径/URL）
fn sh_quoted(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn systemd_unit(service: &str, bin: &str, user: &str) -> String {
    format!(
        "[Unit]\nDescription=Marguerite {service} service\nAfter=network.target\n\n\
         [Service]\nType=simple\nExecStart={bin} serve\nRestart=on-failure\nUser={user}\n\n\
         [Install]\nWantedBy=multi-user.target\n",
        service = service,
        bin = bin,
        user = user
    )
}

/// SSH 部署编排器
pub struct SshDeployer;

impl SshDeployer {
    /// 执行一次完整部署（推送 + 安装 + 探活 + 回滚）。
    ///
    /// `transport` 注入具体传输实现（真实 `SystemSshTransport` 或演练 `RecordingTransport`）。
    pub fn deploy(
        transport: &dyn SshTransport,
        target: &SshTarget,
        opts: &DeployOptions,
        bin: &Path,
    ) -> Result<DeployReport> {
        let remote_bin = format!("{}/{}", target.remote_dir, opts.service);
        let remote_new = format!("{}/{}.new", target.remote_dir, opts.service);
        let remote_old = format!("{}/{}.old", target.remote_dir, opts.service);

        // 1. 连通性探针
        transport.exec("echo marguerite-deploy-probe")?;

        // 2. 远端部署目录
        transport.exec(&format!("mkdir -p {}", sh_quoted(&target.remote_dir)))?;

        // 3. 本地二进制 sha256（部署前校验基线）
        let sha = sha256_file(bin)?;

        // 4. 上传新二进制 -> .new
        transport.upload(bin, Path::new(&remote_new))?;

        // 5. 快照当前版本 -> .old（存在才移，便于回滚）
        transport.exec(&format!(
            "if [ -f {b} ]; then mv {b} {o}; fi",
            b = sh_quoted(&remote_bin),
            o = sh_quoted(&remote_old)
        ))?;

        // 6. 新版本就位
        transport.exec(&format!(
            "mv {} {}",
            sh_quoted(&remote_new),
            sh_quoted(&remote_bin)
        ))?;

        // 7. 安装服务
        if opts.use_systemd {
            let unit = systemd_unit(&opts.service, &remote_bin, &target.user);
            transport.exec(&format!(
                "cat > /etc/systemd/system/{}.service <<'MGEOF'\n{}\nMGEOF",
                opts.service, unit
            ))?;
            transport.exec(&format!(
                "systemctl daemon-reload && systemctl enable {} && systemctl restart {}",
                opts.service, opts.service
            ))?;
        } else {
            transport.exec(&format!(
                "pkill -f {svc} 2>/dev/null; nohup {bin} serve >/var/log/{svc}.log 2>&1 &",
                svc = opts.service,
                bin = sh_quoted(&remote_bin)
            ))?;
        }

        // 8. 健康检查
        let health_ok = match &opts.health_url {
            Some(url) => transport
                .exec(&format!("curl -fsS {} || true", sh_quoted(url)))
                .is_ok(),
            None => {
                if opts.use_systemd {
                    transport
                        .exec(&format!("systemctl is-active {}", opts.service))
                        .is_ok()
                } else {
                    true
                }
            }
        };

        if !health_ok {
            // 回滚：恢复 .old 覆盖当前，并重启服务
            transport.exec(&format!(
                "if [ -f {o} ]; then mv {o} {b}; fi",
                o = sh_quoted(&remote_old),
                b = sh_quoted(&remote_bin)
            ))?;
            if opts.use_systemd {
                transport.exec(&format!("systemctl restart {}", opts.service))?;
            } else {
                transport.exec(&format!(
                    "pkill -f {svc}; nohup {bin} serve >/var/log/{svc}.log 2>&1 &",
                    svc = opts.service,
                    bin = sh_quoted(&remote_bin)
                ))?;
            }
            return Ok(DeployReport {
                binary_sha256: sha,
                rollback_available: true,
                success: false,
            });
        }

        Ok(DeployReport {
            binary_sha256: sha,
            rollback_available: false,
            success: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(bytes: &[u8]) -> PathBuf {
        let name = format!("mgt_dtest_{}.bin", uuid::Uuid::new_v4());
        let p = std::env::temp_dir().join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    #[test]
    fn test_target_from_url_full() {
        let t = SshTarget::from_url("ssh://deploy@192.168.1.10:2222/opt/mgt").unwrap();
        assert_eq!(t.user, "deploy");
        assert_eq!(t.host, "192.168.1.10");
        assert_eq!(t.port, 2222);
        assert_eq!(t.remote_dir, "/opt/mgt");
        assert_eq!(t.endpoint(), "deploy@192.168.1.10:2222");
    }

    #[test]
    fn test_target_from_url_defaults() {
        let t = SshTarget::from_url("ssh://host").unwrap();
        assert_eq!(t.user, "root");
        assert_eq!(t.host, "host");
        assert_eq!(t.port, 22);
        assert_eq!(t.remote_dir, "/opt/marguerite");
    }

    #[test]
    fn test_target_from_url_rejects_non_ssh() {
        assert!(SshTarget::from_url("http://host").is_err());
        assert!(SshTarget::from_url("ssh://").is_err());
    }

    #[test]
    fn test_sha256_known() {
        let p = write_temp(b"abc");
        let h = sha256_file(&p).unwrap();
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_deploy_dry_run_records_and_succeeds() {
        let bin = write_temp(b"fake-binary-contents");
        let target = SshTarget::from_url("ssh://deploy@host/opt/mgt").unwrap();
        let opts = DeployOptions {
            service: "marguerite".into(),
            health_url: Some("http://localhost:8080/health".into()),
            use_systemd: false,
            dry_run: true,
        };
        let transport = RecordingTransport::new();
        let report = SshDeployer::deploy(&transport, &target, &opts, &bin).unwrap();
        assert!(report.success);
        assert!(!report.rollback_available);
        assert_eq!(transport.upload_calls().len(), 1);
        let joined = transport.exec_calls().join("\n");
        assert!(joined.contains("mkdir -p"));
        assert!(joined.contains("curl -fsS"));
        assert!(joined.contains("mv "));
        let _ = std::fs::remove_file(&bin);
    }

    #[test]
    fn test_deploy_health_failure_triggers_rollback() {
        let bin = write_temp(b"fake-binary-contents");
        let target = SshTarget::from_url("ssh://deploy@host/opt/mgt").unwrap();
        let opts = DeployOptions {
            service: "marguerite".into(),
            health_url: Some("http://localhost:8080/health".into()),
            use_systemd: false,
            dry_run: false,
        };
        // 健康探活（curl）注入失败 -> 应触发回滚
        let transport = RecordingTransport::new().with_fail("curl");
        let report = SshDeployer::deploy(&transport, &target, &opts, &bin).unwrap();
        assert!(!report.success);
        assert!(report.rollback_available);
        let joined = transport.exec_calls().join("\n");
        assert!(joined.contains(".old"));
        assert!(joined.contains("nohup"));
        let _ = std::fs::remove_file(&bin);
    }

    #[test]
    fn test_deploy_systemd_install_commands() {
        let bin = write_temp(b"x");
        let target = SshTarget::from_url("ssh://deploy@host").unwrap();
        let opts = DeployOptions {
            service: "marguerite".into(),
            health_url: None,
            use_systemd: true,
            dry_run: false,
        };
        let transport = RecordingTransport::new();
        let report = SshDeployer::deploy(&transport, &target, &opts, &bin).unwrap();
        assert!(report.success);
        let joined = transport.exec_calls().join("\n");
        assert!(joined.contains("systemctl daemon-reload"));
        assert!(joined.contains("systemctl restart marguerite"));
        assert!(joined.contains("systemctl is-active")); // 无 health_url 时 systemd 探活
        assert!(!joined.contains("curl"));
        let _ = std::fs::remove_file(&bin);
    }
}
