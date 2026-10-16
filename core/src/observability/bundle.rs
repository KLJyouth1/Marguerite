//! 诊断包导出（`marguerite --export-diagnostics` 的实现层，S6）。
//!
//! # 契约
//!
//! - **纯只读**：只读磁盘，绝不修改用户配置、不联网上报。
//! - **逐项降级**：单个 artifact 收集失败只变成清单里的一条 `collection_errors`，
//!   绝不让整个导出崩掉（与 Python 侧 `bundle.py` 同做法）——诊断包的价值
//!   恰恰在于「出问题时它还能不能出得来」。
//! - **所有文本必经 [`crate::observability::redact::scrub_text`]**：这是无法被
//!   绕过的收口点，新增 artifact 只能经 [`write_text`] 落盘。
//! - **硬排除**：[`redact::is_excluded_artifact`] 拒写 `~/.marguerite/.env`、
//!   `desktop-credential.json`、`*-raw.jsonl`、`skill_block.jsonl`。
//!
//! # 为什么用 `tar` 而不是 `zip`
//!
//! `zip` crate 在本仓lockfile 里不存在，引入它会牵出新的 C 工具链依赖面；
//! `tar` 已被 `session/rollback.rs` 刻意选用（未压缩，为避开 C 依赖），
//! 有现成先例。诊断包走未压缩 tar 完全够用：`tar -xf` 在三平台都自带。
//!
//! # 为什么截断时丢弃首个不完整行
//!
//! [`scrub_text`] 只能识别**完整**的 `key=value` 行。若按字节尾切后保留首行残段，
//! `api_key=sk-FAKEf` 这样的半截值会整体漏过脱敏——它既不含完整前缀也不含
//! 完整赋值结构，规则无从匹配。所以尾切后必须先丢掉到第一个换行为止的部分。

#[cfg(test)]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::observability::redact::{is_excluded_artifact, scrub_text};
use crate::persistence::config::Config;

/// 单个 artifact 收集的字节上限（1MB）。
///
/// 侧车日志与配置都是小文件；1MB 足够装下完整启动 traceback，
/// 又不至于把诊断包撑到传不动。
pub const MAX_ARTIFACT_BYTES: usize = 1024 * 1024;

/// 清单 schema 版本（消费方据此判断字段可用性）。
pub const BUNDLE_SCHEMA: u32 = 1;

/// 一次导出的结果（同时供 `--json` 输出与文本摘要使用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BundleManifest {
    /// 归档内的 schema 版本。
    pub schema: u32,
    /// CLI 版本。
    pub cli_version: String,
    /// 归档绝对路径。
    pub path: String,
    /// 成功写入的条目（tar 内路径）。
    pub entries: Vec<String>,
    /// 被硬排除而**未**收集的条目（附排除理由，便于支持方判断信息是否完整）。
    pub excluded: Vec<String>,
    /// 收集失败的条目（附错误文本）。
    pub collection_errors: Vec<String>,
    /// 被截断的条目（附源路径与上限）。
    pub truncations: Vec<String>,
}

/// 导出参数。
#[derive(Debug, Clone)]
pub struct BundleOptions {
    /// 单个 artifact 字节上限。
    pub max_artifact_bytes: usize,
}

impl Default for BundleOptions {
    fn default() -> Self {
        Self {
            max_artifact_bytes: MAX_ARTIFACT_BYTES,
        }
    }
}

/// 收集一份诊断包到 `dest`（tar，未压缩）。
///
/// 失败只可能来自**归档本身**建不出来（磁盘满 / 路径不可写）；
/// 任何单个 artifact 的问题都降级为清单条目。
pub fn export_bundle(dest: &Path, opts: &BundleOptions) -> Result<BundleManifest> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建诊断包目录失败: {}", parent.display()))?;
    }
    let file = std::fs::File::create(dest)
        .with_context(|| format!("创建诊断包文件失败: {}", dest.display()))?;
    let mut builder = tar::Builder::new(file);
    let mut entries: Vec<String> = Vec::new();
    let mut collection_errors: Vec<String> = Vec::new();
    let mut truncations: Vec<String> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();

    // 版本 / 环境摘要放在最前：支持方第一眼就该看到平台与版本。
    collect_artifact(
        &mut builder,
        "environment.json",
        opts,
        &mut entries,
        &mut truncations,
        &mut collection_errors,
        environment_summary,
    );
    collect_artifact(
        &mut builder,
        "config/config.yaml",
        opts,
        &mut entries,
        &mut truncations,
        &mut collection_errors,
        collect_config,
    );
    collect_artifact(
        &mut builder,
        "sidecar/sidecar.json",
        opts,
        &mut entries,
        &mut truncations,
        &mut collection_errors,
        collect_sidecar_record,
    );
    collect_sidecar_logs(
        &mut builder,
        opts,
        &mut entries,
        &mut truncations,
        &mut collection_errors,
    );
    // 明确记录被排除项：诊断包缺信息时，支持方需要知道「缺的是什么、为什么缺」，
    // 而不是面对一个静默的空洞。
    excluded.push(
        "security-audit/skill_block.jsonl（硬排除：含用户输入原文片段，非故障证据）".to_string(),
    );

    let manifest = BundleManifest {
        schema: BUNDLE_SCHEMA,
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        path: dest.display().to_string(),
        entries: {
            entries.sort();
            entries
        },
        excluded,
        collection_errors,
        truncations,
    };
    // 清单自身也是文本 → 同样过脱敏（路径字段可能带家目录）。
    write_text(
        &mut builder,
        "manifest.json",
        &serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string()),
    )?;
    builder
        .finish()
        .with_context(|| format!("收尾诊断包失败: {}", dest.display()))?;
    Ok(manifest)
}

/// 收集单个 artifact 并写入 tar；任何失败降级为清单条目。
///
/// `collect` 返回 `Err` 即视为「这一项收不到」（文件不存在 / 读失败），
/// 不影响其余条目——诊断包的价值恰恰在于出问题时它还能出得来。
/// 返回的 `bool` 是该 artifact 是否被截断，用于填充清单的 `truncations`
/// （与 [`collect_sidecar_logs`] 同一口径：截断必须显式告知，不能静默）。
fn collect_artifact<F>(
    builder: &mut tar::Builder<std::fs::File>,
    name: &str,
    opts: &BundleOptions,
    entries: &mut Vec<String>,
    truncations: &mut Vec<String>,
    collection_errors: &mut Vec<String>,
    collect: F,
) where
    F: FnOnce(&BundleOptions) -> Result<(String, bool)>,
{
    match collect(opts) {
        Ok((text, truncated)) => {
            if truncated {
                truncations.push(format!(
                    "{name}（上限 {} 字节）",
                    opts.max_artifact_bytes
                ));
            }
            match write_text(builder, name, &text) {
                Ok(()) => entries.push(name.to_string()),
                Err(err) => collection_errors.push(format!("{name}: 写入失败: {err:#}")),
            }
        }
        Err(err) => collection_errors.push(format!("{name}: {err:#}")),
    }
}

/// 唯一的文本写入路径：**先脱敏，再落盘**。
fn write_text(builder: &mut tar::Builder<std::fs::File>, name: &str, text: &str) -> Result<()> {
    if is_excluded_artifact(name) {
        // 这是收口处的最后一道闸：调用方漏判时仍拒写。
        anyhow::bail!("拒绝写入硬排除产物: {name}");
    }
    let scrubbed = scrub_text(text);
    let bytes = scrubbed.into_bytes();
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    builder
        .append_data(&mut header, Path::new(name), bytes.as_slice())
        .with_context(|| format!("写入归档条目失败: {name}"))
}

/// 读文件尾部最多 `cap` 字节；**截断时丢弃首个不完整行**。
///
/// 返回 `(文本, 是否被截断)`。不存在 / 读失败 → `(空串, false)`：
/// 缺失不该让「记录诊断信息」这条路径本身失败。
fn read_tail(path: &Path, cap: usize) -> (String, bool) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (String::new(), false);
    };
    let size = meta.len();
    let data = if size <= cap as u64 {
        std::fs::read(path).unwrap_or_default()
    } else {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(path) else {
            return (String::new(), false);
        };
        if f.seek(SeekFrom::Start(size.saturating_sub(cap as u64))).is_err() {
            return (String::new(), false);
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() {
            return (String::new(), false);
        }
        buf
    };
    let mut truncated = false;
    let mut body = data;
    if size > cap as u64 {
        truncated = true;
        // 丢掉首个不完整行（含它自己）：半截 `key=value` 会整体漏过脱敏。
        let idx = body.iter().position(|b| *b == b'\n').map(|i| i + 1).unwrap_or(body.len());
        body = body[idx..].to_vec();
    }
    let mut text = String::from_utf8_lossy(&body).into_owned();
    if truncated {
        text = format!("[truncated: showing last {cap} bytes]\n{text}");
    }
    (text, truncated)
}

/// 版本与运行环境摘要（JSON 文本）。
///
/// 签名与 [`collect_artifact`] 的 `FnOnce(&BundleOptions) -> Result<(String, bool)>`
/// 对齐：环境摘要由实时探测生成、不读文件，故 `truncated` 恒为 `false`。
fn environment_summary(_opts: &BundleOptions) -> Result<(String, bool)> {
    let summary = serde_json::json!({
        "schema": BUNDLE_SCHEMA,
        "cli_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "executable": std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string()),
        "config_dir": Config::config_dir().display().to_string(),
        "data_dir": Config::data_dir().display().to_string(),
        "rust_log": std::env::var("RUST_LOG").unwrap_or_default(),
        // 侧车端口：只取落盘记录里那一个（读不到则空表），不做端口扫描
        // ——保持纯只读、离线、可预期。
        "sidecar_port": crate::pyengine::sidecar::load_record().map(|r| r.port),
    });
    let text = serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".to_string());
    Ok((text, false))
}

/// `config.yaml`。
///
/// 配置文件本身**不含明文 api_key**（`persistence/config.rs` 对该字段
/// `skip_serializing`），但仍必经 [`scrub_text`]：用户可能手写了
/// `api_key: xxx` 进 YAML，那正是最需要被抹掉的形态。
fn collect_config(opts: &BundleOptions) -> Result<(String, bool)> {
    let path = Config::config_path();
    if !path.exists() {
        anyhow::bail!("配置文件不存在: {}", path.display());
    }
    Ok(read_tail(&path, opts.max_artifact_bytes))
}

/// 侧车落盘记录（含 S5 的 `last_exit` 退出证据）。
fn collect_sidecar_record(opts: &BundleOptions) -> Result<(String, bool)> {
    let path = crate::pyengine::sidecar::record_path();
    if !path.exists() {
        anyhow::bail!("侧车记录不存在: {}", path.display());
    }
    Ok(read_tail(&path, opts.max_artifact_bytes))
}

/// 侧车 stdout / stderr 日志（含轮转后的历史代）。
fn collect_sidecar_logs(
    builder: &mut tar::Builder<std::fs::File>,
    opts: &BundleOptions,
    entries: &mut Vec<String>,
    truncations: &mut Vec<String>,
    collection_errors: &mut Vec<String>,
) {
    let dir = crate::pyengine::sidecar::sidecar_log_dir();
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        collection_errors.push(format!(
            "sidecar/: 日志目录不可读: {}",
            dir.display()
        ));
        return;
    };
    let mut files: Vec<PathBuf> = read_dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".log") || n.contains(".log."))
                .unwrap_or(false)
        })
        .collect();
    // 稳定顺序：可复现的包内容比「哪条先失败」更重要。
    files.sort();
    for path in files {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let entry_name = format!("sidecar/{name}");
        let (text, truncated) = read_tail(&path, opts.max_artifact_bytes);
        if truncated {
            truncations.push(format!("{entry_name}（源 {}，上限 {} 字节）", path.display(), opts.max_artifact_bytes));
        }
        match write_text(builder, &entry_name, &text) {
            Ok(()) => entries.push(entry_name),
            Err(err) => collection_errors.push(format!("{entry_name}: {err:#}")),
        }
    }
}

/// 诊断包默认输出路径：`Config::data_dir()/diagnostics/` 下的带时间戳文件名。
///
/// 刻意**不用** [`std::path::Path::cwd`]：在仓库内运行 CLI 时那会把产物
/// 落进工作树（进而被 `git status` 看见、被误提交）。
pub fn default_bundle_path(now_ms: i64) -> PathBuf {
    Config::data_dir().join("diagnostics").join(format!(
        "marguerite-diagnostics-{}.tar",
        chrono::DateTime::from_timestamp_millis(now_ms)
            .map(|dt| dt.format("%Y%m%d-%H%M%S").to_string())
            .unwrap_or_else(|| now_ms.to_string())
    ))
}

/// 解析 `--output`（未给出时回退 [`default_bundle_path`]）。
///
/// 空串视为未给出（Windows 上传空环境变量很常见）。
pub fn resolve_output(explicit: Option<&str>, now_ms: i64) -> PathBuf {
    match explicit.map(str::trim).filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => default_bundle_path(now_ms),
    }
}

/// 文本摘要（`--json` 之外的默认输出）。
pub fn render_summary(manifest: &BundleManifest) -> String {
    let mut out = String::new();
    out.push_str("Marguerite 诊断包已导出（所有文本已脱敏）\n");
    out.push_str(&format!("  归档: {}\n", manifest.path));
    out.push_str(&format!("  条目: {} 项\n", manifest.entries.len()));
    for e in &manifest.entries {
        out.push_str(&format!("    + {e}\n"));
    }
    if !manifest.truncations.is_empty() {
        out.push_str(&format!("  截断: {} 项\n", manifest.truncations.len()));
        for t in &manifest.truncations {
            out.push_str(&format!("    ! {t}\n"));
        }
    }
    if !manifest.excluded.is_empty() {
        out.push_str(&format!("  已排除: {} 项\n", manifest.excluded.len()));
        for x in &manifest.excluded {
            out.push_str(&format!("    - {x}\n"));
        }
    }
    if !manifest.collection_errors.is_empty() {
        out.push_str(&format!(
            "  收集失败: {} 项（其余条目已正常导出）\n",
            manifest.collection_errors.len()
        ));
        for e in &manifest.collection_errors {
            out.push_str(&format!("    x {e}\n"));
        }
    }
    out
}

/// 仅供测试：构造一个可控的 artifact 映射（避免测试写真实用户目录）。
#[cfg(test)]
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marguerite-bundle-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

/// 测试辅助：tar 条目名 → 内容。
#[cfg(test)]
fn read_tar_entries(path: &Path) -> BTreeMap<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("打开归档");
    let mut archive = tar::Archive::new(file);
    let mut out = BTreeMap::new();
    for entry in archive.entries().expect("遍历归档").flatten() {
        let mut entry = entry;
        let name = entry.path().unwrap().display().to_string();
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_ok() {
            out.insert(name, text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 明显的假凭据：任何扫描器都不会把它当真密钥。
    const FAKE_SK: &str = "sk-FAKEfakeFAKEfakeFAKE1234";
    const FAKE_BEARER: &str = "Bearer abcdefghijklmnopqrstuvwx";

    #[test]
    fn exports_manifest_and_environment() {
        let dir = scratch_dir("basic");
        let dest = dir.join("out.tar");
        let manifest = export_bundle(&dest, &BundleOptions::default()).expect("导出应成功");
        assert!(dest.exists(), "归档文件必须真实产出");
        assert!(manifest.entries.contains(&"environment.json".to_string()));
        let entries = read_tar_entries(&dest);
        assert!(entries.contains_key("manifest.json"));
        assert!(entries.contains_key("environment.json"));
        assert!(
            entries["manifest.json"].contains(BUNDLE_SCHEMA.to_string().as_str()),
            "清单必须含 schema 版本"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_records_excluded_artifacts() {
        let dir = scratch_dir("excluded");
        let dest = dir.join("out.tar");
        let manifest = export_bundle(&dest, &BundleOptions::default()).unwrap();
        assert!(
            manifest.excluded.iter().any(|x| x.contains("skill_block.jsonl")),
            "排除项必须被显式记账: {:?}",
            manifest.excluded
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_text_refuses_excluded_names() {
        // 收口处闸门：即便调用方漏判，硬排除名也写不进去
        let dir = scratch_dir("gate");
        let dest = dir.join("out.tar");
        let file = std::fs::File::create(&dest).unwrap();
        let mut builder = tar::Builder::new(file);
        assert!(write_text(&mut builder, "logs/desktop-credential.json", "x").is_err());
        assert!(write_text(&mut builder, "config/.env", "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_tail_drops_partial_first_line() {
        // 回归：截断点落在**行中间**时，首行残段必须被丢。
        // 构造上刻意让末行远长于 cap，使切点必然落在该行内部。
        let dir = scratch_dir("tail");
        let path = dir.join("big.log");
        let mut content = String::new();
        for _ in 0..200 {
            content.push_str(&"x".repeat(1024));
            content.push('\n');
        }
        content.push_str(&format!("api_key={}\n", "A".repeat(3000)));
        std::fs::write(&path, &content).unwrap();
        let (text, truncated) = read_tail(&path, 1024);
        assert!(truncated, "必须标记为已截断");
        assert!(text.starts_with("[truncated:"), "{text}");
        // 半截的 `api_key=` 前缀整体被丢 → 残留不再构成可识别的赋值形态
        assert!(
            !text.contains("api_key="),
            "半截赋值行必须整体丢弃，否则半截值漏过脱敏: {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_tail_output_has_no_secret_after_scrub() {
        // `read_tail` 的职责只是「丢残行」；真正的脱敏在 `write_text`。
        // 两者合起来（read_tail → scrub_text）必须没有密钥残留。
        let dir = scratch_dir("tailscrub");
        let path = dir.join("big.log");
        let mut content = String::new();
        for _ in 0..200 {
            content.push_str(&"x".repeat(1024));
            content.push('\n');
        }
        content.push_str(&format!("api_key={FAKE_SK}\n"));
        std::fs::write(&path, &content).unwrap();
        let (text, truncated) = read_tail(&path, 1024);
        assert!(truncated, "必须标记为已截断");
        assert!(
            !scrub_text(&text).contains(FAKE_SK),
            "read_tail + scrub 合起来不得残留密钥: {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_tail_no_truncation_keeps_whole_file() {
        let dir = scratch_dir("notail");
        let path = dir.join("small.log");
        std::fs::write(&path, format!("api_key={FAKE_SK}\n")).unwrap();
        let (text, truncated) = read_tail(&path, MAX_ARTIFACT_BYTES);
        assert!(!truncated);
        assert!(text.contains(FAKE_SK), "未截断时内容须原样供后续脱敏");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_tail_missing_file_is_empty_not_error() {
        let (text, truncated) = read_tail(
            &std::env::temp_dir().join("marguerite-bundle-nope-does-not-exist"),
            1024,
        );
        assert!(text.is_empty() && !truncated);
    }

    #[test]
    fn collected_text_is_scrubbed_end_to_end() {
        // 端到端脱敏断言：把假密钥写进 config.yaml，再导出，归档里必须搜不到
        let dir = scratch_dir("e2e");
        let cfg_dir = dir.join("cfg");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.yaml"),
            format!("model:\n  api_key: {FAKE_SK}\n  auth: {FAKE_BEARER}\n"),
        )
        .unwrap();
        Config::set_config_dir_override(cfg_dir.clone());

        let dest = dir.join("out.tar");
        let manifest = export_bundle(&dest, &BundleOptions::default()).unwrap();
        assert!(
            manifest.entries.contains(&"config/config.yaml".to_string()),
            "config 应被收集: {:?}",
            manifest.entries
        );
        let entries = read_tar_entries(&dest);
        let cfg = &entries["config/config.yaml"];
        assert!(!cfg.contains(FAKE_SK), "归档内不得残留假密钥: {cfg}");
        assert!(!cfg.contains("abcdefghijklmnopqrstuvwx"), "{cfg}");
        assert!(cfg.contains("[redacted]"), "应留下占位符: {cfg}");

        // 二次导出（幂等性在包层面同样成立）
        let dest2 = dir.join("out2.tar");
        export_bundle(&dest2, &BundleOptions::default()).unwrap();
        let entries2 = read_tar_entries(&dest2);
        assert_eq!(
            entries2["config/config.yaml"], *cfg,
            "二次导出的同一 artifact 必须逐字节一致"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn individual_artifact_failure_does_not_abort_export() {
        // config 指向空目录（文件不存在）→ 记为collection_errors，其余照常导出
        let dir = scratch_dir("degrade");
        let empty_cfg = dir.join("empty-cfg");
        std::fs::create_dir_all(&empty_cfg).unwrap();
        Config::set_config_dir_override(empty_cfg);
        let dest = dir.join("out.tar");
        let manifest = export_bundle(&dest, &BundleOptions::default())
            .expect("单个 artifact 失败不得让整个导出失败");
        assert!(
            manifest
                .collection_errors
                .iter()
                .any(|e| e.contains("config/config.yaml")),
            "缺失文件应记为收集错误: {:?}",
            manifest.collection_errors
        );
        assert!(dest.exists(), "归档仍须产出");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_output_prefers_explicit_then_default() {
        assert_eq!(
            resolve_output(Some("D:/tmp/diag.tar"), 0),
            PathBuf::from("D:/tmp/diag.tar")
        );
        // 空串视为未给出（Windows 常见空环境变量）
        assert!(resolve_output(Some("   "), 0).starts_with(Config::data_dir()));
        // ⚠️ 必须用 `extension()` 而不是 `Path::ends_with(".tar")`：
        // `Path::ends_with` 比的是**路径分量**，`"x.tar"` 不等于分量 `".tar"`，
        // 那个写法恒为 false。
        assert_eq!(resolve_output(None, 0).extension(), Some(std::ffi::OsStr::new("tar")));
    }

    #[test]
    fn summary_renders_all_sections() {
        let manifest = BundleManifest {
            schema: BUNDLE_SCHEMA,
            cli_version: "0.0.0-test".to_string(),
            path: "/tmp/x.tar".to_string(),
            entries: vec!["environment.json".to_string()],
            excluded: vec!["skill_block.jsonl（硬排除）".to_string()],
            collection_errors: vec!["config/config.yaml: 不存在".to_string()],
            truncations: vec!["sidecar/a.log（上限 1024 字节）".to_string()],
        };
        let text = render_summary(&manifest);
        assert!(text.contains("environment.json"));
        assert!(text.contains("skill_block.jsonl"));
        assert!(text.contains("config/config.yaml"));
        assert!(text.contains("a.log"));
    }

    #[test]
    fn writes_archive_to_nested_missing_parent() {
        let dir = scratch_dir("nested");
        let dest = dir.join("a").join("b").join("out.tar");
        export_bundle(&dest, &BundleOptions::default()).expect("应自动建父目录");
        assert!(dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}