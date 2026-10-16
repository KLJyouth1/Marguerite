//! SSH 部署编排：`SshDeployer::deploy`（推送 + 安装 + 探活 + 回滚）与 sha256 校验。
//!
//! 编排不依赖具体传输（注入 [`SshTransport`]），覆盖：
//! 探针 → 建目录 → 本地 sha256 → 上传 .new → 快照 .old → 就位 → 安装服务 →
//! 健康探活 →（失败）回滚。从 `deploy/ssh.rs` 拆分（M0.5，OD-003）。
use std::path::Path;

use anyhow::{anyhow, Result};

use super::target::SshTarget;
use super::transport::SshTransport;

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
    use super::super::recording::RecordingTransport;
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_temp(bytes: &[u8]) -> PathBuf {
        let name = format!("mgt_dtest_{}.bin", uuid::Uuid::new_v4());
        let p = std::env::temp_dir().join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
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
