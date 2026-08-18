//! 协作网关：WebSocket 长连接 + 心跳（Phase I1，轻量抽象）
//!
//! 复用 agent_runtime 心跳思路；本模块用 `Instant` 记录最后心跳，超时即判定离线，
//! 不依赖具体传输层，便于单测与后续接入真实 WebSocket / SSH 隧道。

use std::time::{Duration, Instant};

/// 心跳超时（超过该间隔未收到心跳即判离线）
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

/// 协作网关连接状态
#[derive(Debug, Clone)]
pub struct CollabGateway {
    /// 远端地址（面板别名 / URL）
    endpoint: String,
    /// 是否已连接
    connected: bool,
    /// 最后心跳时刻
    last_heartbeat: Option<Instant>,
    /// 累计心跳次数
    heartbeats: u64,
}

impl CollabGateway {
    /// 构造（未连接）
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.into(),
            connected: false,
            last_heartbeat: None,
            heartbeats: 0,
        }
    }

    /// 建立连接（重置心跳）
    pub fn connect(&mut self) {
        self.connected = true;
        self.last_heartbeat = Some(Instant::now());
        self.heartbeats = 0;
    }

    /// 断开连接
    pub fn disconnect(&mut self) {
        self.connected = false;
        self.last_heartbeat = None;
    }

    /// 收到一次心跳
    pub fn tick(&mut self) {
        if self.connected {
            self.last_heartbeat = Some(Instant::now());
            self.heartbeats += 1;
        }
    }

    /// 是否在线（已连接 + 心跳在超时窗口内）
    pub fn is_alive(&self) -> bool {
        match (self.connected, self.last_heartbeat) {
            (true, Some(t)) => t.elapsed() < HEARTBEAT_TIMEOUT,
            _ => false,
        }
    }

    /// 心跳次数
    pub fn heartbeat_count(&self) -> u64 {
        self.heartbeats
    }

    /// 端点
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_and_alive() {
        let mut g = CollabGateway::new("relay.example:9000");
        assert!(!g.is_alive());
        g.connect();
        assert!(g.is_alive());
        assert_eq!(g.endpoint(), "relay.example:9000");
    }

    #[test]
    fn test_disconnect_goes_offline() {
        let mut g = CollabGateway::new("x");
        g.connect();
        g.disconnect();
        assert!(!g.is_alive());
    }

    #[test]
    fn test_heartbeat_count_increments() {
        let mut g = CollabGateway::new("x");
        g.connect();
        g.tick();
        g.tick();
        assert_eq!(g.heartbeat_count(), 2);
    }
}
