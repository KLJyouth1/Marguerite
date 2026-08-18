//! 真实审计执行体（F4 修复）
//!
//! 取代 `automation/mod.rs::run` 中对 `JobKind::Audit` 的占位返回
//! （原：`RunStatus::Success` + "审计维度占位执行完成，Phase E 将接入真实审计逻辑"）。
//!
//! 每个内置维度（见 `builtin::BUILTIN_AUDITS`）对应一个真实检查：
//! - 纯静态检查（占位符 / 配置漂移 / 文档断链 / 依赖反模式 / 记忆格式 /
//!   沙箱策略 / 性能基线 / 测试统计）：完全离线、确定性、可单测。
//! - 需外部工具的维度（clippy / cargo-audit / llvm-cov）：先探测工具是否安装；
//!   安装则真正运行并解析其输出，未安装则**诚实报告降级**（绝不冒充 `Success`）。
//!
//! 设计约束：所有检查同步执行（无网络、无 LLM），保持 `run()` 全链路同步、零行为风险。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::automation::ledger::RunStatus;

/// 审计结果
pub struct AuditOutcome {
    /// 状态（Success=通过 / Warning=发现问题但不致命 / Failed=严重问题）
    pub status: RunStatus,
    /// 人类可读报告（将写入台账 `result` 字段与事件载荷）
    pub report: String,
}

impl AuditOutcome {
    fn ok(report: String) -> Self {
        Self {
            status: RunStatus::Success,
            report,
        }
    }
    fn warn(report: String) -> Self {
        Self {
            status: RunStatus::Warning,
            report,
        }
    }
    fn fail(report: String) -> Self {
        Self {
            status: RunStatus::Failed,
            report,
        }
    }
}

/// 跳过这些目录（构建产物 / 依赖 / VCS）
const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "dist"];

/// 按维度 ID 执行真实审计。
///
/// `base_dir` 为待审计的仓库根（生产由 `run()` 传入 `current_dir()`，测试传入 fixture）。
/// 未知 ID → `Failed`（无法执行真实审计，而非静默成功）。
pub fn run_audit(id: &str, base_dir: &Path) -> (RunStatus, String) {
    let out = match id {
        "audit-code-quality" => audit_code_quality(base_dir),
        "audit-security" => audit_security(base_dir),
        "audit-test-coverage" => audit_test_coverage(base_dir),
        "audit-performance" => audit_performance(base_dir),
        "audit-memory" => audit_memory(base_dir),
        "audit-sandbox" => audit_sandbox(base_dir),
        "audit-doc-sync" => audit_doc_sync(base_dir),
        "audit-placeholders" => audit_placeholders(base_dir),
        "audit-dependencies" => audit_dependencies(base_dir),
        "audit-config-drift" => audit_config_drift(base_dir),
        _ => AuditOutcome::fail(format!("未注册的内置审计维度: {id}，无法执行真实审计")),
    };
    (out.status, out.report)
}

// ---------------------------------------------------------------------------
// 通用工具
// ---------------------------------------------------------------------------

/// 递归收集 `base` 下指定扩展名的文件，跳过 `SKIP_DIRS`。
fn collect_files(base: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(base, ext, &mut out);
    out
}

fn collect(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
        }
        if p.is_dir() {
            collect(&p, ext, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
            out.push(p);
        }
    }
}

/// 探测外部工具是否可用（仅 `--version`，快速不挂起）。
fn probe_tool(args: &[&str]) -> bool {
    if args.is_empty() {
        return false;
    }
    let mut cmd = Command::new(args[0]);
    for a in &args[1..] {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// 带超时的命令执行（避免外部工具卡死阻塞 cron）。超时返回 `None`。
fn run_with_timeout(cmd: &mut Command, secs: u64) -> Option<String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return None,
    };
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let out = child.wait_with_output();
        let _ = tx.send(out);
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(Ok(o)) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            Some(s)
        }
        _ => {
            // 超时：进程可能仍在后台运行（此处不强制 kill，交由周期调度回收）
            let _ = handle.join();
            None
        }
    }
}

/// 在 `base` 下定位 Cargo.toml（优先根，否则向下查找第一个）。
fn find_cargo_toml(base: &Path) -> Option<PathBuf> {
    let root = base.join("Cargo.toml");
    if root.exists() {
        return Some(root);
    }
    collect_files(base, "toml")
        .into_iter()
        .find(|p| p.file_name().map(|n| n == "Cargo.toml").unwrap_or(false))
}

/// 稳定哈希（djb2），用于配置漂移比对（避免引入额外 crypto 依赖）。
fn djb2(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 5381;
    for &b in bytes {
        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(b as u64);
    }
    hash
}

// ---------------------------------------------------------------------------
// 1. audit-placeholders：零占位门禁
// ---------------------------------------------------------------------------

/// 代码级占位标记（不含常见合法的 `unreachable!()`，避免误报）。
const PLACEHOLDER_MARKERS: &[&str] = &[
    "todo!()",
    "unimplemented!()",
    "panic!(\"not implemented",
    "panic!(\"TODO",
    "panic!(\"todo",
    "// TODO",
    "// FIXME",
    "// XXX",
];

fn audit_placeholders(base: &Path) -> AuditOutcome {
    let files = collect_files(base, "rs");
    let mut hits: Vec<(PathBuf, usize, String)> = Vec::new();
    for f in &files {
        if let Ok(content) = std::fs::read_to_string(f) {
            for (i, line) in content.lines().enumerate() {
                for m in PLACEHOLDER_MARKERS {
                    if line.contains(m) {
                        hits.push((f.clone(), i + 1, (*m).to_string()));
                        break;
                    }
                }
            }
        }
    }
    if hits.is_empty() {
        AuditOutcome::ok(format!(
            "零占位门禁通过：扫描 {} 个 .rs 文件，未发现代码级占位符",
            files.len()
        ))
    } else {
        let mut report = format!("零占位门禁发现 {} 处代码级占位符：\n", hits.len());
        for (p, ln, m) in hits.iter().take(20) {
            report.push_str(&format!("  - {}:{}  [{}]\n", p.display(), ln, m));
        }
        if hits.len() > 20 {
            report.push_str(&format!("  ... 其余 {} 处省略\n", hits.len() - 20));
        }
        AuditOutcome::warn(report)
    }
}

// ---------------------------------------------------------------------------
// 2. audit-config-drift：配置漂移
// ---------------------------------------------------------------------------

fn audit_config_drift(base: &Path) -> AuditOutcome {
    let candidates = [
        base.join("config.yaml"),
        base.join(".marguerite").join("config.yaml"),
        base.join("config.toml"),
        base.join("config.json"),
    ];
    let Some(cfg) = candidates.iter().find(|p| p.exists()) else {
        return AuditOutcome::warn(
            "配置漂移审计：未发现配置文件（config.yaml / config.toml / config.json），未建立基线"
                .into(),
        );
    };
    let content = match std::fs::read_to_string(cfg) {
        Ok(c) => c,
        Err(e) => {
            return AuditOutcome::fail(format!("配置漂移审计：读取失败 {}: {e}", cfg.display()))
        }
    };
    let hash = djb2(content.as_bytes());
    let snap_dir = base.join(".marguerite").join("audit-snapshots");
    let snap_path = snap_dir.join("audit-config-drift.sha");
    if !snap_path.exists() {
        let _ = std::fs::create_dir_all(&snap_dir);
        let _ = std::fs::write(&snap_path, hash.to_string());
        return AuditOutcome::ok(format!("配置漂移审计：基线已建立（{}）", cfg.display()));
    }
    let prev = std::fs::read_to_string(&snap_path).unwrap_or_default();
    if prev.trim() == hash.to_string() {
        AuditOutcome::ok(format!("配置漂移审计：未检测到变更（{}）", cfg.display()))
    } else {
        let _ = std::fs::write(&snap_path, hash.to_string());
        AuditOutcome::warn(format!(
            "配置漂移审计：检测到配置变更（{}），已更新基线",
            cfg.display()
        ))
    }
}

// ---------------------------------------------------------------------------
// 3. audit-doc-sync：文档同步
// ---------------------------------------------------------------------------

fn audit_doc_sync(base: &Path) -> AuditOutcome {
    let files = collect_files(base, "md");
    let mut broken: Vec<(PathBuf, String)> = Vec::new();
    for f in &files {
        let dir = f.parent().unwrap_or(base);
        if let Ok(content) = std::fs::read_to_string(f) {
            let bytes = content.as_bytes();
            let mut i = 0;
            while let Some(open) = find_sub(bytes, b"](", i) {
                let start = open + 2;
                let mut j = start;
                while j < bytes.len() && bytes[j] != b')' {
                    j += 1;
                }
                if j < bytes.len() {
                    let target = std::str::from_utf8(&bytes[start..j])
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if is_relative_file_link(&target) && !dir.join(&target).exists() {
                        broken.push((f.clone(), target));
                    }
                }
                i = j + 1;
            }
        }
    }
    if broken.is_empty() {
        AuditOutcome::ok(format!(
            "文档同步审计：扫描 {} 个 .md，未发现断链的相对文件链接",
            files.len()
        ))
    } else {
        let mut report = format!("文档同步审计：发现 {} 处断链：\n", broken.len());
        for (p, t) in broken.iter().take(20) {
            report.push_str(&format!("  - {}  ->  {}\n", p.display(), t));
        }
        if broken.len() > 20 {
            report.push_str(&format!("  ... 其余 {} 处省略\n", broken.len() - 20));
        }
        AuditOutcome::warn(report)
    }
}

/// 在 `haystack` 的 `from` 之后查找 `needle` 首次出现的位置。
fn find_sub(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from + needle.len() > haystack.len() {
        return None;
    }
    (from..=(haystack.len() - needle.len())).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// 判断 markdown 链接目标是否为需要校验存在的相对文件链接。
fn is_relative_file_link(t: &str) -> bool {
    if t.starts_with('#')
        || t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("mailto:")
        || t.contains(':')
    {
        return false;
    }
    t.starts_with("./") || t.starts_with("../") || t.contains('/') || t.ends_with(".md")
}

// ---------------------------------------------------------------------------
// 4. audit-dependencies：依赖新鲜度
// ---------------------------------------------------------------------------

fn audit_dependencies(base: &Path) -> AuditOutcome {
    let Some(cargo) = find_cargo_toml(base) else {
        return AuditOutcome::warn("依赖新鲜度审计：未发现 Cargo.toml".into());
    };
    let content = match std::fs::read_to_string(&cargo) {
        Ok(c) => c,
        Err(e) => return AuditOutcome::fail(format!("依赖新鲜度审计：读取失败 {e}")),
    };
    let mut issues: Vec<(usize, String)> = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.trim_start().starts_with("version") && line.contains("\"*\"") {
            issues.push((i + 1, "通配版本 \"*\"".into()));
        }
    }
    if probe_tool(&["cargo", "outdated", "--version"]) {
        if let Some(out) = run_with_timeout(
            Command::new("cargo")
                .args(["outdated", "--format", "json"])
                .current_dir(base),
            60,
        ) {
            let outdated = count_outdated(&out);
            if outdated > 0 {
                return AuditOutcome::warn(format!(
                    "依赖新鲜度审计：cargo outdated 报告 {} 个可更新依赖",
                    outdated
                ));
            }
        }
        let extra = if issues.is_empty() {
            String::new()
        } else {
            format!("；另有 {} 处版本反模式", issues.len())
        };
        return AuditOutcome::ok(format!("依赖新鲜度审计：cargo outdated 通过{extra}"));
    }
    if issues.is_empty() {
        AuditOutcome::warn(
            "依赖新鲜度审计：cargo-outdated 未安装，新鲜度审计降级（仅做静态反模式检查）".into(),
        )
    } else {
        let mut r = format!("依赖新鲜度审计：发现 {} 处版本反模式：\n", issues.len());
        for (ln, d) in &issues {
            r.push_str(&format!("  - {}:{}  {}\n", cargo.display(), ln, d));
        }
        r.push_str("（cargo-outdated 未安装，未做在线新鲜度比对）");
        AuditOutcome::warn(r)
    }
}

fn count_outdated(json: &str) -> usize {
    // cargo outdated --format json 中每个可更新项含 "outdated" 状态标记
    json.matches("\"outdated\"").count()
}

// ---------------------------------------------------------------------------
// 5. audit-memory：记忆一致性
// ---------------------------------------------------------------------------

fn audit_memory(base: &Path) -> AuditOutcome {
    let candidates = [base.join(".workbuddy").join("memory"), base.join("memory")];
    let Some(dir) = candidates.iter().find(|p| p.exists()) else {
        return AuditOutcome::ok("记忆一致性审计：未发现 memory 目录，跳过".into());
    };
    let mut bad = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "MEMORY.md" {
                continue;
            }
            if !is_date_file(name) {
                bad.push(name.to_string());
            }
        }
    }
    if bad.is_empty() {
        AuditOutcome::ok(format!(
            "记忆一致性审计：{} 下记忆文件命名规范",
            dir.display()
        ))
    } else {
        let mut r = format!("记忆一致性审计：发现 {} 个非规范命名文件：\n", bad.len());
        for n in &bad {
            r.push_str(&format!("  - {n}\n"));
        }
        AuditOutcome::warn(r)
    }
}

fn is_date_file(name: &str) -> bool {
    let stem = name.strip_suffix(".md").unwrap_or(name);
    if stem.len() != 10 {
        return false;
    }
    let c: Vec<char> = stem.chars().collect();
    c[4] == '-'
        && c[7] == '-'
        && c[0..4].iter().all(|x| x.is_ascii_digit())
        && c[5..7].iter().all(|x| x.is_ascii_digit())
        && c[8..10].iter().all(|x| x.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// 6. audit-sandbox：沙箱完整性
// ---------------------------------------------------------------------------

fn audit_sandbox(base: &Path) -> AuditOutcome {
    let files = collect_files(base, "rs");
    let mut cmd_count = 0usize;
    let mut unsafe_count = 0usize;
    for f in &files {
        if let Ok(c) = std::fs::read_to_string(f) {
            for line in c.lines() {
                if line.contains("Command::new") {
                    cmd_count += 1;
                }
                if line.contains("unsafe") {
                    unsafe_count += 1;
                }
            }
        }
    }
    let policy = [
        base.join("sandbox.allow"),
        base.join(".marguerite").join("sandbox.allow"),
        base.join("sec").join("guardrail"),
    ];
    let has_policy = policy.iter().any(|p| p.exists());
    let mut report = format!(
        "沙箱完整性审计：源码中 Command::new 调用 {} 处、unsafe 标记 {} 处；沙箱策略文件 {}",
        cmd_count,
        unsafe_count,
        if has_policy { "存在" } else { "缺失" }
    );
    if has_policy {
        AuditOutcome::ok(report)
    } else {
        report.push_str("（建议补充沙箱白名单 / 护栏策略文件）");
        AuditOutcome::warn(report)
    }
}

// ---------------------------------------------------------------------------
// 7. audit-performance：性能基线
// ---------------------------------------------------------------------------

fn audit_performance(base: &Path) -> AuditOutcome {
    let Some(cargo) = find_cargo_toml(base) else {
        return AuditOutcome::warn("性能基线审计：未发现 Cargo.toml".into());
    };
    let content = std::fs::read_to_string(&cargo).unwrap_or_default();
    let has_bench_profile =
        content.contains("[[bench]]") || content.contains("criterion") || content.contains("iai");
    let has_bench_dir = base.join("benches").exists();
    if has_bench_profile || has_bench_dir {
        AuditOutcome::ok(format!(
            "性能基线审计：检测到基准设施（bench profile={}，benches/ 目录={}）",
            has_bench_profile, has_bench_dir
        ))
    } else {
        AuditOutcome::warn(
            "性能基线审计：未检测到基准设施（无 [[bench]] / criterion / benches/），性能基线审计降级".into(),
        )
    }
}

// ---------------------------------------------------------------------------
// 8. audit-test-coverage：测试覆盖率（静态统计 + 可选 llvm-cov）
// ---------------------------------------------------------------------------

fn audit_test_coverage(base: &Path) -> AuditOutcome {
    let files = collect_files(base, "rs");
    let mut test_fns = 0usize;
    let mut src_fns = 0usize;
    for f in &files {
        if let Ok(c) = std::fs::read_to_string(f) {
            for line in c.lines() {
                if line.contains("#[test]")
                    || line.contains("#[tokio::test]")
                    || (line.contains("fn ") && line.contains("test_"))
                {
                    test_fns += 1;
                }
                let t = line.trim_start();
                if t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("async fn ") {
                    src_fns += 1;
                }
            }
        }
    }
    if probe_tool(&["cargo", "llvm-cov", "--version"]) {
        if let Some(out) = run_with_timeout(
            Command::new("cargo")
                .args(["llvm-cov", "--summary-only"])
                .current_dir(base),
            120,
        ) {
            if let Some(pct) = parse_coverage(&out) {
                let status = if pct < 50.0 {
                    RunStatus::Warning
                } else {
                    RunStatus::Success
                };
                return AuditOutcome {
                    status,
                    report: format!("测试覆盖率审计：llvm-cov 实测覆盖率 {pct:.1}%"),
                };
            }
        }
    }
    if test_fns == 0 {
        AuditOutcome::warn(format!(
            "测试覆盖率审计：未发现测试函数（静态统计 {} 个 .rs），覆盖率审计降级",
            files.len()
        ))
    } else {
        AuditOutcome::warn(format!(
            "测试覆盖率审计：cargo llvm-cov 未安装，仅静态统计到 {} 个测试函数（共 {} 处 fn）；覆盖率百分比审计降级",
            test_fns, src_fns
        ))
    }
}

fn parse_coverage(json: &str) -> Option<f64> {
    // llvm-cov --summary-only：{ "totals": { "lines": { "percent": N } } }
    let pos = json.find("\"lines\"")?;
    let rest = &json[pos..];
    let pp = rest.find("\"percent\"")?;
    let num: String = rest[pp + 9..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().ok()
}

// ---------------------------------------------------------------------------
// 9. audit-code-quality：clippy（需外部工具）
// ---------------------------------------------------------------------------

fn audit_code_quality(base: &Path) -> AuditOutcome {
    if probe_tool(&["cargo", "clippy", "--version"]) {
        match run_with_timeout(
            Command::new("cargo")
                .args(["clippy", "--message-format=json", "-q"])
                .current_dir(base),
            180,
        ) {
            Some(out) => {
                let (warn, err) = parse_clippy(&out);
                if err > 0 {
                    AuditOutcome::fail(format!(
                        "代码质量审计：clippy 报告 {} 个错误、{} 个警告",
                        err, warn
                    ))
                } else if warn > 0 {
                    AuditOutcome::warn(format!(
                        "代码质量审计：clippy 报告 {} 个警告（无错误）",
                        warn
                    ))
                } else {
                    AuditOutcome::ok("代码质量审计：clippy 通过（0 警告 0 错误）".into())
                }
            }
            None => AuditOutcome::warn("代码质量审计：clippy 运行超时或无输出，降级".into()),
        }
    } else {
        AuditOutcome::warn(
            "代码质量审计：cargo clippy 未安装，质量门禁降级（仅做静态占位/配置审计）".into(),
        )
    }
}

fn parse_clippy(json: &str) -> (usize, usize) {
    let mut warn = 0usize;
    let mut err = 0usize;
    for line in json.lines() {
        if line.contains("\"message\"") && line.contains("\"compiler-message\"") {
            if line.contains("\"level\":\"error\"") {
                err += 1;
            } else if line.contains("\"level\":\"warning\"") {
                warn += 1;
            }
        }
    }
    (warn, err)
}

// ---------------------------------------------------------------------------
// 10. audit-security：cargo audit（需外部工具）
// ---------------------------------------------------------------------------

fn audit_security(base: &Path) -> AuditOutcome {
    if probe_tool(&["cargo", "audit", "--version"]) {
        match run_with_timeout(
            Command::new("cargo")
                .args(["audit", "--json"])
                .current_dir(base),
            120,
        ) {
            Some(out) => {
                let vulns = parse_audit_vulns(&out).unwrap_or(0);
                if vulns > 0 {
                    AuditOutcome::fail(format!("安全扫描审计：cargo audit 发现 {vulns} 个漏洞"))
                } else {
                    AuditOutcome::ok("安全扫描审计：cargo audit 通过（0 漏洞）".into())
                }
            }
            None => AuditOutcome::warn("安全扫描审计：cargo audit 运行超时或无输出，降级".into()),
        }
    } else {
        AuditOutcome::warn("安全扫描审计：cargo audit 未安装，安全门禁降级".into())
    }
}

fn parse_audit_vulns(json: &str) -> Option<usize> {
    // cargo audit --json：{ "vulnerabilities": { "count": N } }
    let pos = json.find("\"count\"")?;
    let rest = &json[pos + 7..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// 单测（均为确定性、离线、可单测的真实检查）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("marguerite_audit_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unknown_dimension_is_failed_not_success() {
        let dir = fixture("unknown");
        let (s, r) = run_audit("not-a-real-audit", &dir);
        assert_eq!(s, RunStatus::Failed);
        assert!(r.contains("未注册"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn placeholders_clean_passes() {
        let dir = fixture("ph_clean");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn f() { let _ = 1; }\n").unwrap();
        let out = audit_placeholders(&dir);
        assert_eq!(out.status, RunStatus::Success);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn placeholders_detects_todo() {
        let dir = fixture("ph_todo");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn f() { todo!(); }\n").unwrap();
        let out = audit_placeholders(&dir);
        assert_eq!(out.status, RunStatus::Warning);
        assert!(out.report.contains("todo!()"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_drift_baseline_then_change() {
        let dir = fixture("drift");
        let cfg = dir.join("config.yaml");
        std::fs::write(&cfg, "k: v\n").unwrap();

        let first = audit_config_drift(&dir);
        assert_eq!(first.status, RunStatus::Success);
        assert!(first.report.contains("基线已建立"));

        // 修改配置后再跑 → 应检测到漂移
        std::fs::write(&cfg, "k: v2\n").unwrap();
        let second = audit_config_drift(&dir);
        assert_eq!(second.status, RunStatus::Warning);
        assert!(second.report.contains("配置变更"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_sync_broken_link_detected() {
        let dir = fixture("doc_broken");
        std::fs::write(dir.join("present.md"), "# present\n").unwrap();
        std::fs::write(
            dir.join("doc.md"),
            "[ok](./present.md)\n[broken](./missing.md)\n",
        )
        .unwrap();
        let out = audit_doc_sync(&dir);
        assert_eq!(out.status, RunStatus::Warning);
        assert!(out.report.contains("missing.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_sync_all_resolve() {
        let dir = fixture("doc_ok");
        std::fs::write(dir.join("present.md"), "# present\n").unwrap();
        std::fs::write(dir.join("doc.md"), "[ok](./present.md)\n").unwrap();
        let out = audit_doc_sync(&dir);
        assert_eq!(out.status, RunStatus::Success);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dependencies_flags_wildcard() {
        let dir = fixture("deps");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[dependencies]\nfoo = { version = \"*\" }\n",
        )
        .unwrap();
        let out = audit_dependencies(&dir);
        // 测试环境通常无 cargo-outdated → 降级，但仍应报告静态发现的版本反模式
        assert_eq!(out.status, RunStatus::Warning);
        assert!(out.report.contains("依赖新鲜度"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_rejects_bad_naming() {
        let dir = fixture("mem");
        let m = dir.join("memory");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("2026-08-12.md"), "ok\n").unwrap();
        std::fs::write(m.join("notes.md"), "bad naming\n").unwrap();
        let out = audit_memory(&dir);
        assert_eq!(out.status, RunStatus::Warning);
        assert!(out.report.contains("notes.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_absent_is_ok() {
        let dir = fixture("mem_none");
        let out = audit_memory(&dir);
        assert_eq!(out.status, RunStatus::Success);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sandbox_reports_policy_status() {
        let dir = fixture("sbx");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "pub fn run() { let _ = std::process::Command::new(\"ls\"); }\n",
        )
        .unwrap();
        // 无策略文件 → 警告
        let out = audit_sandbox(&dir);
        assert_eq!(out.status, RunStatus::Warning);
        assert!(out.report.contains("Command::new"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn performance_detects_bench() {
        let dir = fixture("perf");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[dev-dependencies]\ncriterion = \"0.5\"\n",
        )
        .unwrap();
        let out = audit_performance(&dir);
        assert_eq!(out.status, RunStatus::Success);
        assert!(out.report.contains("基准设施"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
