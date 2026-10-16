//! SSH 目标与主机形状解析：`SshTarget`（ssh:// URL）与 `HostSpec`（user@host:port）。
//!
//! 从 `deploy/ssh.rs` 拆分（M0.5，OD-003）：纯解析逻辑，离线可测。
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

/// 结构化主机地址（`user@host:port` 形状；用户/端口可缺省）。
///
/// 与 `SshTarget` 的区别：`HostSpec` 只描述连接端点，不携带远端部署目录与私钥，
/// 供命令行参数、`~/.ssh/config` 别名等场景做纯解析与结构校验。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    /// 登录用户（缺省 None，由调用方决定默认值）
    pub user: Option<String>,
    /// 主机名或 IP
    pub host: String,
    /// SSH 端口（缺省 22）
    pub port: u16,
}

impl HostSpec {
    /// 解析 `[user@]host[:port]`。
    ///
    /// - `host` → user=None, port=22
    /// - `user@host` → user=Some(user), port=22
    /// - `host:2222` → user=None, port=2222
    /// - `user@host:2222` → user=Some(user), port=2222
    ///
    /// 非法形状（空串、空主机名、空用户名、多个 `@`、端口非数字或为 0）报错。
    pub fn parse(s: &str) -> Result<Self> {
        let raw = s.trim();
        if raw.is_empty() {
            bail!("主机形状为空");
        }
        if raw.matches('@').count() > 1 {
            bail!("主机形状含多个 '@': {raw}");
        }
        let (user, hostport) = match raw.split_once('@') {
            Some((u, hp)) => {
                if u.is_empty() {
                    bail!("'@' 前缺少用户名: {raw}");
                }
                (Some(u.to_string()), hp)
            }
            None => (None, raw),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => {
                if h.is_empty() {
                    bail!("':' 前缺少主机名: {raw}");
                }
                let port: u16 = p
                    .parse()
                    .map_err(|_| anyhow!("端口解析失败（须为 1..=65535）: {p}"))?;
                if port == 0 {
                    bail!("端口不能为 0: {raw}");
                }
                (h.to_string(), port)
            }
            None => {
                if hostport.is_empty() {
                    bail!("缺少主机名: {raw}");
                }
                (hostport.to_string(), 22)
            }
        };
        Ok(Self { user, host, port })
    }

    /// 取登录用户（缺省时回退到 `default`）。
    pub fn user_or<'a>(&'a self, default: &'a str) -> &'a str {
        self.user.as_deref().unwrap_or(default)
    }

    /// 转成 `SshTarget`（补上远端部署目录；私钥留待调用方设置）。
    pub fn into_ssh_target(self, remote_dir: &str) -> SshTarget {
        SshTarget {
            user: self.user_or("root").to_string(),
            host: self.host,
            port: self.port,
            remote_dir: remote_dir.to_string(),
            identity: None,
        }
    }

    /// 连接端点描述（对齐 `SshTarget::endpoint`）。
    pub fn endpoint(&self) -> String {
        format!("{}@{}:{}", self.user_or("root"), self.host, self.port)
    }
}

/// 解析 `[user@]host[:port]` 为结构化 `HostSpec`（纯函数，离线可测）。
pub fn parse_hostspec(s: &str) -> Result<HostSpec> {
    HostSpec::parse(s)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_parse_hostspec_full() {
        let h = parse_hostspec("deploy@192.168.1.10:2222").unwrap();
        assert_eq!(h.user.as_deref(), Some("deploy"));
        assert_eq!(h.host, "192.168.1.10");
        assert_eq!(h.port, 2222);
        assert_eq!(h.endpoint(), "deploy@192.168.1.10:2222");
    }

    #[test]
    fn test_parse_hostspec_defaults() {
        let h = parse_hostspec("host").unwrap();
        assert_eq!(h.user, None, "缺省用户应为 None");
        assert_eq!(h.host, "host");
        assert_eq!(h.port, 22, "缺省端口应为 22");
        assert_eq!(h.user_or("root"), "root");

        let h2 = parse_hostspec("root@host").unwrap();
        assert_eq!(h2.user.as_deref(), Some("root"));
        assert_eq!(h2.port, 22);

        let h3 = parse_hostspec("host:2200").unwrap();
        assert_eq!(h3.user, None);
        assert_eq!(h3.port, 2200);

        // 前后空白应容忍
        let h4 = parse_hostspec("  user@host:2222  ").unwrap();
        assert_eq!(h4.user.as_deref(), Some("user"));
        assert_eq!(h4.port, 2222);
    }

    #[test]
    fn test_parse_hostspec_invalid_shapes() {
        assert!(parse_hostspec("").is_err());
        assert!(parse_hostspec("   ").is_err());
        assert!(parse_hostspec("@host").is_err(), "空用户名应报错");
        assert!(parse_hostspec("user@").is_err(), "空主机名应报错");
        assert!(parse_hostspec(":22").is_err(), "空主机名应报错");
        assert!(parse_hostspec("host:").is_err(), "空端口应报错");
        assert!(parse_hostspec("host:abc").is_err(), "非数字端口应报错");
        assert!(parse_hostspec("host:0").is_err(), "端口 0 应报错");
        assert!(parse_hostspec("host:99999").is_err(), "端口超 u16 应报错");
        assert!(parse_hostspec("a@b@c").is_err(), "多个 @ 应报错");
    }

    #[test]
    fn test_hostspec_into_ssh_target() {
        let t = parse_hostspec("deploy@host:22")
            .unwrap()
            .into_ssh_target("/opt/mgt");
        assert_eq!(t.user, "deploy");
        assert_eq!(t.host, "host");
        assert_eq!(t.port, 22);
        assert_eq!(t.remote_dir, "/opt/mgt");
        assert_eq!(t.endpoint(), "deploy@host:22");
    }
}
