//! WP5: offline / air-gapped / on-premise network egress policy.
//!
//! Pure decision logic. No network I/O, no config wiring yet. The invariant
//! guarded here: egress is **never** default-allowed to the public Internet.
//! Loopback is always allowed; private LAN is allowed only when not air-gapped;
//! the public cloud must be explicitly authorized by the caller.

use std::net::IpAddr;

/// The outcome of asking [`OutboundPolicy::allow`] for a destination host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allowance {
    /// Destination is loopback and may always be reached.
    AllowedLoopback,
    /// Destination is private RFC1918 LAN, reachable only when not air-gapped.
    AllowedPrivateLan,
    /// Public / cloud destination. The device must not call this out by default;
    /// an explicit, operator-granted authorization is required first.
    DeniedLabeledCloud,
}

/// Decides whether a single outbound destination host may be contacted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutboundPolicy {
    /// When `true` the device is air-gapped: only loopback is reachable.
    pub airgap: bool,
}

impl OutboundPolicy {
    /// Build a policy with the given air-gap flag.
    pub const fn new(airgap: bool) -> Self {
        Self { airgap }
    }

    /// Evaluate an outbound destination.
    ///
    /// - Loopback (`127.0.0.0/8`, `::1`, `localhost`) is always allowed.
    /// - When **not** air-gapped, private RFC1918 LAN is also allowed.
    /// - When air-gapped, private LAN is treated like the cloud: denied.
    /// - Everything else (public Internet / cloud) is denied with
    ///   [`Allowance::DeniedLabeledCloud`] — never silently allowed.
    pub fn allow(&self, host: &str) -> Allowance {
        let host = normalize_host(host);
        if is_loopback(&host) {
            return Allowance::AllowedLoopback;
        }
        if !self.airgap && is_private_lan(&host) {
            return Allowance::AllowedPrivateLan;
        }
        Allowance::DeniedLabeledCloud
    }
}

/// Trim whitespace, strip a trailing port (`host:port` / `[v6]:port`) and a
/// scheme so callers may pass user-supplied strings without preprocessing.
fn normalize_host(host: &str) -> String {
    let host = host.trim();
    // Drop scheme if present.
    let host = host
        .strip_prefix("http://")
        .or_else(|| host.strip_prefix("https://"))
        .unwrap_or(host);
    // Drop any path / query.
    let host = host.split('/').next().unwrap_or(host);
    let host = host.split('?').next().unwrap_or(host);
    let host = host.trim_end_matches(':');
    // IPv6 literal in brackets: [::1]:18790 -> ::1
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.split(']').next()) {
        return inner.to_string();
    }
    // Strip a trailing :port for bare IPv4 / names (but not bare IPv6 without
    // brackets, which we already handled above).
    if let Some(idx) = host.rfind(':') {
        // If there is more than one ':' it is an unbracketed IPv6 address; keep
        // the whole thing.
        if host.matches(':').count() == 1 {
            return host[..idx].to_string();
        }
    }
    host.to_string()
}

/// Returns `true` if `host` is the loopback interface:
/// `127.0.0.0/8`, `::1`, or the literal name `localhost`.
pub fn is_loopback(host: &str) -> bool {
    let host = normalize_host(host);
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

/// Returns `true` if `host` is a private RFC1918 LAN address:
/// `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`.
///
/// Loopback is *not* considered private LAN here — it has its own allowance.
pub fn is_private_lan(host: &str) -> bool {
    let host = normalize_host(host);
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let oct = v4.octets();
            if v4.is_loopback() {
                return false;
            }
            // 10.0.0.0/8
            if oct[0] == 10 {
                return true;
            }
            // 172.16.0.0/12 -> 172.16.0.0 .. 172.31.255.255
            if oct[0] == 172 && (16..=31).contains(&oct[1]) {
                return true;
            }
            // 192.168.0.0/16
            if oct[0] == 192 && oct[1] == 168 {
                return true;
            }
            false
        }
        // IPv6 ULA (fc00::/7) is not part of the requested RFC1918 set; keep
        // this function strictly to the three documented v4 ranges.
        Ok(IpAddr::V6(_)) => false,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_recognized() {
        for h in [
            "127.0.0.1",
            "127.0.0.2",
            "127.255.255.255",
            "::1",
            "localhost",
            "LOCALHOST",
            "localhost:18790",
            "http://localhost:18790/control",
            "[::1]:18790",
        ] {
            assert!(is_loopback(h), "expected loopback: {h}");
        }
    }

    #[test]
    fn private_lan_hosts_are_recognized() {
        for h in [
            "10.0.0.1",
            "10.255.255.254",
            "192.168.0.1",
            "192.168.255.254",
            "172.16.0.1",
            "172.31.255.254",
        ] {
            assert!(is_private_lan(h), "expected private LAN: {h}");
        }
    }

    #[test]
    fn non_lan_hosts_are_not_private() {
        for h in [
            "8.8.8.8",
            "1.1.1.1",
            "203.0.113.9",
            "172.32.0.1",   // just outside /12
            "192.169.0.1",  // outside /16
            "11.0.0.1",     // outside /8
            "127.0.0.1",    // loopback, not LAN
            "example.com",
        ] {
            assert!(!is_private_lan(h), "expected NOT private LAN: {h}");
        }
    }

    #[test]
    fn airgap_blocks_everything_but_loopback() {
        let policy = OutboundPolicy::new(true);
        assert_eq!(policy.allow("127.0.0.1"), Allowance::AllowedLoopback);
        assert_eq!(policy.allow("::1"), Allowance::AllowedLoopback);
        assert_eq!(policy.allow("localhost"), Allowance::AllowedLoopback);

        // Private LAN is denied once air-gapped.
        assert_eq!(
            policy.allow("192.168.1.10"),
            Allowance::DeniedLabeledCloud
        );
        assert_eq!(policy.allow("10.0.0.5"), Allowance::DeniedLabeledCloud);
        // Public cloud denied.
        assert_eq!(policy.allow("8.8.8.8"), Allowance::DeniedLabeledCloud);
        assert_eq!(
            policy.allow("api.openai.com"),
            Allowance::DeniedLabeledCloud
        );
    }

    #[test]
    fn non_airgap_allows_loopback_and_lan_but_not_cloud() {
        let policy = OutboundPolicy::new(false);
        assert_eq!(policy.allow("127.0.0.1"), Allowance::AllowedLoopback);
        assert_eq!(
            policy.allow("192.168.1.10"),
            Allowance::AllowedPrivateLan
        );
        assert_eq!(policy.allow("10.0.0.5"), Allowance::AllowedPrivateLan);
        assert_eq!(policy.allow("172.20.4.4"), Allowance::AllowedPrivateLan);

        // Public egress is never default-allowed.
        assert_eq!(policy.allow("8.8.8.8"), Allowance::DeniedLabeledCloud);
        assert_eq!(
            policy.allow("114.114.114.114"),
            Allowance::DeniedLabeledCloud
        );
        assert_eq!(
            policy.allow("api.anthropic.com"),
            Allowance::DeniedLabeledCloud
        );
    }
}
