//! SSH 本地端口转发隧道：`SshTunnel` + `SshTunnelRegistry`（对齐 DSH ssh_tunnel）。
//!
//! 纯逻辑、离线可测的状态与端口管理（实际端口转发由运行时接入）。
//! 从 `deploy/ssh.rs` 拆分（M0.5，OD-003）。

/// 隧道状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelState {
    Connecting,
    Forwarding,
    Failed,
}

impl TunnelState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TunnelState::Connecting => "connecting",
            TunnelState::Forwarding => "forwarding",
            TunnelState::Failed => "failed",
        }
    }
}

/// 本地端口转发隧道（对齐 DSH ssh_tunnel）
#[derive(Debug, Clone)]
pub struct SshTunnel {
    pub id: String,
    pub alias: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub state: TunnelState,
    pub error: Option<String>,
}

/// 隧道注册表（start / list / stop / stop-all；实际端口转发由运行时接入，
/// 本模块为纯逻辑、离线可测的状态与端口管理）。
#[derive(Default)]
pub struct SshTunnelRegistry {
    tunnels: Vec<SshTunnel>,
    seq: u64,
}

impl SshTunnelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动一条隧道；local_port 为 None 时自动分配（自 20000 递增、去重）。
    pub fn start(
        &mut self,
        alias: &str,
        remote_host: &str,
        remote_port: u16,
        local_port: Option<u16>,
    ) -> SshTunnel {
        self.seq += 1;
        let id = format!("tun-{}", self.seq);
        let local = local_port.unwrap_or_else(|| self.alloc_port());
        let t = SshTunnel {
            id,
            alias: alias.to_string(),
            local_port: local,
            remote_host: remote_host.to_string(),
            remote_port,
            state: TunnelState::Forwarding,
            error: None,
        };
        self.tunnels.push(t.clone());
        t
    }

    fn alloc_port(&self) -> u16 {
        let mut p = 20000u16;
        while self.tunnels.iter().any(|t| t.local_port == p) {
            p += 1;
        }
        p
    }

    pub fn list(&self) -> &[SshTunnel] {
        &self.tunnels
    }

    pub fn get(&self, id: &str) -> Option<&SshTunnel> {
        self.tunnels.iter().find(|t| t.id == id)
    }

    /// 停止单条隧道；返回是否命中。
    pub fn stop(&mut self, id: &str) -> bool {
        let before = self.tunnels.len();
        self.tunnels.retain(|t| t.id != id);
        self.tunnels.len() != before
    }

    /// 停止全部隧道；返回停止数量。
    pub fn stop_all(&mut self) -> usize {
        let n = self.tunnels.len();
        self.tunnels.clear();
        n
    }

    pub fn len(&self) -> usize {
        self.tunnels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tunnels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tunnel_start_auto_assign_port() {
        let mut r = SshTunnelRegistry::new();
        let a = r.start("db", "127.0.0.1", 3306, None);
        let b = r.start("cache", "127.0.0.1", 6379, None);
        assert_eq!(a.local_port, 20000);
        assert_eq!(b.local_port, 20001);
        assert_eq!(a.state, TunnelState::Forwarding);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn test_tunnel_explicit_port_and_stop() {
        let mut r = SshTunnelRegistry::new();
        let t = r.start("web", "127.0.0.1", 8080, Some(9000));
        assert_eq!(t.local_port, 9000);
        assert!(r.get(&t.id).is_some());
        assert!(r.stop(&t.id));
        assert!(!r.stop(&t.id), "重复 stop 应返回 false");
        assert!(r.is_empty());
    }

    #[test]
    fn test_tunnel_stop_all() {
        let mut r = SshTunnelRegistry::new();
        r.start("a", "127.0.0.1", 1, None);
        r.start("b", "127.0.0.1", 2, None);
        r.start("c", "127.0.0.1", 3, None);
        assert_eq!(r.stop_all(), 3);
        assert!(r.is_empty());
        assert_eq!(r.stop_all(), 0);
    }
}
