//! 命令幂等性判定与长输出折叠（对齐 dsh-ssh 审计语义）。
//!
//! 从 `deploy/ssh.rs` 拆分（M0.5，OD-003）：纯函数，离线可测。

/// 判定命令是否幂等（可安全重放）。
///
/// 白名单为读/探活/幂等建目录类命令；写、启停、删除、上传类一律视为非幂等。
/// 保守默认 → 宁可多标非幂等、不可漏标（对齐 dsh-ssh「断线重连可能重放非幂等命令」审计语义）。
pub fn is_idempotent_command(cmd: &str) -> bool {
    let c = cmd.trim();
    if c.is_empty() {
        return true; // 空命令无副作用
    }
    let c = c.strip_prefix("sudo ").unwrap_or(c).trim_start();
    const NON_IDEMPOTENT: &[&str] = &[
        "mv ",
        "rm ",
        "cp ",
        "cat >",
        "tee ",
        "dd ",
        "install ",
        "pkill",
        "kill",
        "nohup",
        "systemctl restart",
        "systemctl start",
        "systemctl stop",
        "systemctl enable",
        "systemctl disable",
        "systemctl daemon-reload",
        "systemctl mask",
        "systemctl unmask",
        "apt ",
        "apt-get ",
        "yum ",
        "dnf ",
        "touch ",
        "mkfs",
        "scp ",
        "rsync ",
        "curl -X POST",
        "curl -X PUT",
    ];
    if NON_IDEMPOTENT.iter().any(|p| c.contains(p)) {
        return false;
    }
    const IDEMPOTENT: &[&str] = &[
        "echo ",
        "cat ",
        "ls ",
        "ll ",
        "df",
        "free",
        "uptime",
        "uname",
        "hostname",
        "id",
        "whoami",
        "pwd",
        "test ",
        "[ ",
        "grep ",
        "which ",
        "command -v ",
        "mkdir -p ",
        "systemctl is-active ",
        "systemctl status ",
        "systemctl is-enabled ",
        "systemctl is-failed ",
        "curl -fsS ",
    ];
    IDEMPOTENT.iter().any(|p| c.starts_with(p))
}

/// 生成幂等性审计警告：非幂等命令返回警告标记（供断线重连审计），幂等返回 None。
pub fn idempotency_warning(cmd: &str, idempotent: bool) -> Option<String> {
    if idempotent {
        None
    } else {
        Some(format!("[非幂等] 断线重连可能重放此命令: {cmd}"))
    }
}

/// 输出折叠保留的头部字符数（4 KB）
pub const OUTPUT_SNIP_HEAD: usize = 4096;
/// 输出折叠保留的尾部字符数（4 KB）
pub const OUTPUT_SNIP_TAIL: usize = 4096;

/// 长输出按「头 + 截断标记 + 尾」折叠（头 4KB + 尾 4KB），
/// 风格对齐 `context::session::snip_long`，供批命令/集群等大输出渲染前截断。
///
/// 未超阈值（`OUTPUT_SNIP_HEAD + OUTPUT_SNIP_TAIL` 字符）时原样返回。
pub fn snip_output(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= OUTPUT_SNIP_HEAD + OUTPUT_SNIP_TAIL {
        return text.to_string();
    }
    let head: String = chars[..OUTPUT_SNIP_HEAD].iter().collect();
    let tail: String = chars[chars.len() - OUTPUT_SNIP_TAIL..].iter().collect();
    let omitted = chars.len() - OUTPUT_SNIP_HEAD - OUTPUT_SNIP_TAIL;
    format!("{head}\n…[输出过长，已省略中间 {omitted} 字符]…\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_idempotent_classification() {
        // 读/探活/幂等建目录 → 幂等
        assert!(is_idempotent_command("echo hi"));
        assert!(is_idempotent_command("cat /etc/os-release"));
        assert!(is_idempotent_command("mkdir -p /opt/app"));
        assert!(is_idempotent_command("systemctl is-active marguerite"));
        assert!(is_idempotent_command("sudo systemctl status app"));
        assert!(is_idempotent_command(""));
        // 写/启停/删除 → 非幂等
        assert!(!is_idempotent_command("mv a b"));
        assert!(!is_idempotent_command("rm -rf /tmp/x"));
        assert!(!is_idempotent_command("systemctl restart marguerite"));
        assert!(!is_idempotent_command("nohup ./app &"));
    }

    #[test]
    fn test_idempotency_warning_semantics() {
        // 非幂等 → 有警告；幂等 → 无警告
        assert!(idempotency_warning("rm -rf x", false).is_some());
        assert!(idempotency_warning("rm -rf x", true).is_none());
        assert!(
            idempotency_warning("echo hi", false).is_some(),
            "调用方明确标记非幂等即告警"
        );
        assert!(idempotency_warning("echo hi", true).is_none());
        // 警告文案须点明「重放」审计语义
        let w = idempotency_warning("mv a b", false).unwrap();
        assert!(w.contains("重放") && w.contains("mv a b"));
    }

    #[test]
    fn test_snip_output_exactly_threshold_unchanged() {
        let s = "a".repeat(OUTPUT_SNIP_HEAD + OUTPUT_SNIP_TAIL);
        assert_eq!(snip_output(&s), s, "恰好不超阈值应原样返回");
    }

    #[test]
    fn test_snip_output_over_by_one_folds() {
        let s = "a".repeat(OUTPUT_SNIP_HEAD + OUTPUT_SNIP_TAIL + 1);
        let out = snip_output(&s);
        assert_ne!(out, s, "超 1 字节应触发折叠");
        assert!(out.contains("省略中间 1 字符"), "截断标记应含省略数: {out}");
        assert!(
            out.starts_with(&"a".repeat(OUTPUT_SNIP_HEAD)),
            "头部 4KB 应保留"
        );
    }

    #[test]
    fn test_snip_output_way_over_shrinks() {
        let s = "b".repeat(100_000);
        let out = snip_output(&s);
        assert!(out.len() < s.len(), "超很多时应显著变短");
        assert!(out.contains("省略中间"), "应含截断标记");
        assert!(
            out.starts_with(&"b".repeat(OUTPUT_SNIP_HEAD)),
            "头部 4KB 应保留"
        );
        assert!(
            out.ends_with(&"b".repeat(OUTPUT_SNIP_TAIL)),
            "尾部 4KB 应保留"
        );
    }
}
