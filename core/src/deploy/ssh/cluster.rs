//! SSH 集群多主机执行：`SshCluster`（顺序、确定性可测）。
//!
//! exec 闭包注入实际传输（真实 `SystemSshTransport` 或 mock）。
//! 从 `deploy/ssh.rs` 拆分（M0.5，OD-003）。
use anyhow::Result;

use super::target::SshTarget;

/// 集群主机（alias + 连接目标）
#[derive(Clone, Debug)]
pub struct ClusterHost {
    pub alias: String,
    pub target: SshTarget,
}

/// 集群单机执行结果
#[derive(Debug, Clone)]
pub struct ClusterResult {
    pub alias: String,
    pub ok: bool,
    pub stdout: String,
    pub error: Option<String>,
}

/// 多主机并发执行（ssh_cluster 二开，Track 4）。
///
/// 顺序执行、确定性可测；真实并发（线程池/异步）留作后续增量。
/// exec 闭包注入实际传输（真实 SystemSshTransport 或 mock）。
#[derive(Default)]
pub struct SshCluster {
    hosts: Vec<ClusterHost>,
}

impl SshCluster {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, alias: &str, target: SshTarget) {
        self.hosts.push(ClusterHost {
            alias: alias.to_string(),
            target,
        });
    }

    pub fn len(&self) -> usize {
        self.hosts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }

    pub fn aliases(&self) -> Vec<String> {
        self.hosts.iter().map(|h| h.alias.clone()).collect()
    }

    /// 逐主机执行命令；exec 闭包负责真实传输（或 mock）。
    pub fn run<F>(&self, cmd: &str, mut exec: F) -> Vec<ClusterResult>
    where
        F: FnMut(&ClusterHost, &str) -> Result<String>,
    {
        self.hosts
            .iter()
            .map(|h| match exec(h, cmd) {
                Ok(stdout) => ClusterResult {
                    alias: h.alias.clone(),
                    ok: true,
                    stdout,
                    error: None,
                },
                Err(e) => ClusterResult {
                    alias: h.alias.clone(),
                    ok: false,
                    stdout: String::new(),
                    error: Some(e.to_string()),
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_run_collects_per_host() {
        let mut c = SshCluster::new();
        c.add("a", SshTarget::from_url("ssh://host-a").unwrap());
        c.add("b", SshTarget::from_url("ssh://host-b").unwrap());
        c.add("c", SshTarget::from_url("ssh://host-c").unwrap());
        let results = c.run("uptime", |h, _cmd| {
            if h.alias == "b" {
                Err(anyhow::anyhow!("boom"))
            } else {
                Ok(format!("{}:ok", h.alias))
            }
        });
        assert_eq!(results.len(), 3);
        assert!(results[0].ok);
        assert_eq!(results[0].stdout, "a:ok");
        assert!(!results[1].ok);
        assert_eq!(results[1].error.as_deref(), Some("boom"));
        assert!(results[2].ok);
        assert_eq!(results[2].stdout, "c:ok");
    }

    #[test]
    fn test_cluster_empty_and_aliases() {
        let c = SshCluster::new();
        assert!(c.is_empty());
        assert!(c.run("x", |_, _| Ok("y".into())).is_empty());
        let mut c2 = SshCluster::new();
        c2.add("web", SshTarget::from_url("ssh://h").unwrap());
        assert_eq!(c2.aliases(), vec!["web".to_string()]);
        assert_eq!(c2.len(), 1);
    }
}
