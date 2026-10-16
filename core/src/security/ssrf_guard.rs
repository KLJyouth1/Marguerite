//! SSRF 防护：拦截对内网 / 回环 / 云元数据地址的出站请求
//!
//! 大模型驱动的 `web_fetch` / `http_request` 若不加约束，可能被诱导访问
//! `http://169.254.169.254/`（云元数据）、`http://localhost`、`http://10.x`
//! 等内网资源。本模块在发起请求前强制校验目标地址。

use anyhow::{anyhow, Result};
use reqwest::Url;
use std::net::{IpAddr, ToSocketAddrs};

/// SSRF 防护器
#[derive(Debug, Clone, Default)]
pub struct SsrfGuard;

impl SsrfGuard {
    pub fn new() -> Self {
        Self
    }

    /// 校验 URL 是否可安全访问。拒绝非 http(s) 协议、私有/回环/链路本地/元数据地址。
    pub fn check(&self, url: &str) -> Result<()> {
        let parsed = Url::parse(url).map_err(|e| anyhow!("invalid url `{}`: {}", url, e))?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => return Err(anyhow!("only http/https allowed, got scheme `{}`", other)),
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("url `{}` missing host", url))?;

        // 1) 直接是 IP 字面量：立即判断
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(&ip) {
                return Err(anyhow!(
                    "blocked address (private/loopback/metadata): {}",
                    ip
                ));
            }
            return Ok(());
        }

        // 2) 主机名语义拦截
        let h = host.to_lowercase();
        if h == "localhost"
            || h == "localhost.localdomain"
            || h.ends_with(".local")
            || h.ends_with(".internal")
            || h.ends_with(".svc")
            || h.ends_with(".cluster.local")
            || h.contains("metadata")
        {
            return Err(anyhow!("blocked host by name: {}", host));
        }

        // 3) 解析后逐个地址判断（防 DNS 指向内网）
        let key = format!("{}:0", host);
        if let Ok(addrs) = key.to_socket_addrs() {
            let mut saw_public = false;
            for a in addrs {
                if is_blocked_ip(&a.ip()) {
                    return Err(anyhow!(
                        "blocked resolved private IP {} for host {}",
                        a.ip(),
                        host
                    ));
                }
                saw_public = true;
            }
            if saw_public {
                return Ok(());
            }
        }

        // 解析失败：保守拒绝（避免泄露/绕过）
        Err(anyhow!(
            "could not resolve host `{}`; refusing for safety",
            host
        ))
    }
}

/// 判断 IP 是否应被拦截（私有 / 回环 / 链路本地 / 文档 / CGNAT / 云元数据）
fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_documentation()
            || v4.octets()[0] == 100 // CGNAT 100.64.0.0/10
            || *v4 == IpAddr::V4(std::net::Ipv4Addr::new(169, 254, 169, 254)) // 云元数据
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> SsrfGuard {
        SsrfGuard::new()
    }

    #[test]
    fn allows_public_https() {
        assert!(g().check("https://example.com/api").is_ok());
        assert!(g().check("http://news.ycombinator.com").is_ok());
    }

    #[test]
    fn blocks_non_http_schemes() {
        assert!(g().check("file:///etc/passwd").is_err());
        assert!(g().check("ftp://example.com/x").is_err());
        assert!(g().check("gopher://x").is_err());
    }

    #[test]
    fn blocks_loopback_and_private_ips() {
        assert!(g().check("http://127.0.0.1:8080/").is_err());
        assert!(g().check("http://localhost/").is_err());
        assert!(g().check("http://10.0.0.5/").is_err());
        assert!(g().check("http://192.168.1.1/").is_err());
        assert!(g().check("http://172.16.5.5/").is_err());
        assert!(g()
            .check("http://169.254.169.254/latest/meta-data/")
            .is_err());
    }

    #[test]
    fn blocks_localhost_by_name() {
        assert!(g().check("https://localhost.localdomain/").is_err());
        assert!(g().check("http://app.internal/").is_err());
        assert!(g().check("http://metadata.google.internal/").is_err());
    }

    #[test]
    fn invalid_url_rejected() {
        assert!(g().check("not a url").is_err());
        assert!(g().check("https://").is_err());
    }
}
