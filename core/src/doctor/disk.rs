//! 第 6 项：磁盘余量诊断器。
//!
//! 测量数据目录所在盘的可用空间，判定是否 > 2 GiB（PACKAGING.md §6.1）。

use std::path::Path;
use std::process::Command;

use crate::doctor::{DoctorItem, Status};
use crate::persistence::config::Config;
use crate::proc::SilentCommand;

/// 阈值：2 GiB（必须大于该值视为 OK）。
const THRESHOLD: u64 = 2 * 1024 * 1024 * 1024;

/// 第 6 项：磁盘余量（数据目录所在盘 > 2G）。
pub fn check_disk_free() -> DoctorItem {
    let data_dir = Config::data_dir();
    let (disk_root, free_bytes) = disk_free(&data_dir);

    if free_bytes > THRESHOLD {
        DoctorItem {
            id: "disk".to_string(),
            label: "磁盘余量".to_string(),
            status: Status::Ok,
            detail: format!(
                "盘 {} 可用 {} (>{} GiB)",
                disk_root,
                human_bytes(free_bytes),
                THRESHOLD / (1024 * 1024 * 1024)
            ),
            fix_hint: String::new(),
        }
    } else {
        DoctorItem {
            id: "disk".to_string(),
            label: "磁盘余量".to_string(),
            status: Status::Missing,
            detail: format!("盘 {} 可用 {} (<=2 GiB)", disk_root, human_bytes(free_bytes)),
            fix_hint: format!(
                "磁盘已满/不足。请清理 {} 所在盘，或把数据目录迁移到余量更大的盘",
                data_dir.display()
            ),
        }
    }
}

/// 返回 (盘根, 可用字节)。优先 Windows `fsutil volume diskfree`，回退到 `df -k`。
fn disk_free(data_dir: &Path) -> (String, u64) {
    let root = drive_root(data_dir);
    if cfg!(windows) {
        if let Some(avail) = windows_disk_free(&root) {
            return (root, avail);
        }
    }
    if let Some(avail) = unix_disk_free(&root) {
        return (root, avail);
    }
    // 兜底：无法测量时按 0 处理（会触发 MISSING，但报告明确说明）。
    (root, 0)
}

fn drive_root(data_dir: &Path) -> String {
    if cfg!(windows) {
        if let Some(prefix) = data_dir.to_string_lossy().get(0..3).map(|s| s.to_string()) {
            return prefix;
        }
    }
    "/".to_string()
}

/// Windows: 调用 `fsutil volume diskfree <drive>:` 解析可用字节数。
///
/// ⚠️ **编码陷阱（真机踩过，是本项误报的唯一根因）**：fsutil 按**控制台代码页**
/// 输出 —— 简中 Windows 是 GBK/CP936，英文或已 `chcp 65001` 的控制台才是
/// ASCII/UTF-8。因此 `String::from_utf8_lossy` 在简中机器上会得到一串
/// `U+FFFD` 乱码，按标签匹配（`"可用字节数"` / `"free bytes"` / `"available"`）
/// **必然全部落空**，函数返回 `None` → [`disk_free`] 按 0 处理 →
/// 磁盘项恒报 MISSING（实测「盘 C: 可用 0 B」而 `fsutil` 实际报 17.9 GB 可用）。
///
/// 修法：标签匹配降为**首选路径**（英文/UTF-8 环境下语义最明确），失败后回退
/// **位置式解析** —— fsutil 首行恒为「总可用字节数 / Total free bytes」，
/// 而冒号与阿拉伯数字都是 ASCII，**不受代码页影响**。
fn windows_disk_free(drive: &str) -> Option<u64> {
    let out = Command::new("fsutil")
        .silent()
        .arg("volume")
        .arg("diskfree")
        .arg(drive)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    parse_fsutil_free_bytes(&text)
}

/// 解析 fsutil 输出（纯函数，便于按代码页各态单测）。
fn parse_fsutil_free_bytes(text: &str) -> Option<u64> {
    parse_by_label(text).or_else(|| parse_first_free_bytes(text))
}

/// 首选：标签可读时（英文控制台 / UTF-8 代码页）按标签定位，语义最明确。
fn parse_by_label(text: &str) -> Option<u64> {
    for line in non_empty_lines(text) {
        let lower = line.to_lowercase();
        if line.contains("可用字节数")
            || lower.contains("free bytes")
            || lower.contains("available")
        {
            if let Some(num) = extract_number_after_colon(line) {
                return Some(num);
            }
        }
    }
    None
}

/// 回退：位置式解析。fsutil 首行恒为「总可用字节数」，标签是乱码也无妨 ——
/// 只取冒号后的 ASCII 数字组，与语言/代码页无关。
fn parse_first_free_bytes(text: &str) -> Option<u64> {
    non_empty_lines(text)
        .first()
        .and_then(|l| extract_number_after_colon(l))
}

/// 去除空行后的行切片（保持原顺序）。
fn non_empty_lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

/// Unix: `df -k <root>` 解析第 4 列（可用 KB）。
fn unix_disk_free(root: &str) -> Option<u64> {
    let out = Command::new("df").silent().arg("-k").arg(root).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    for (i, line) in text.lines().enumerate() {
        if i == 0 {
            continue; // header
        }
        let col: Vec<&str> = line.split_whitespace().collect();
        if col.len() >= 4 {
            if let Ok(kbytes) = col[3].parse::<u64>() {
                return Some(kbytes * 1024);
            }
        }
    }
    None
}

/// 从冒号后的值部分提取第一个数字（忽略逗号和括号中的辅助数字）。
fn extract_number_after_colon(line: &str) -> Option<u64> {
    let value_part = line.rsplit(':').next()?;
    let cleaned: String = value_part
        .chars()
        .take_while(|c| !c.eq(&'('))
        .filter(|c| c.is_ascii_digit())
        .collect();
    cleaned.parse::<u64>().ok()
}

/// 人类可读字节数。
fn human_bytes(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GiB", b / GB)
    } else if b >= MB {
        format!("{:.1} MiB", b / MB)
    } else if b >= KB {
        format!("{:.1} KiB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fsutil 真实输出（简中 Windows，GBK 代码页 → `from_utf8_lossy` 后标签全成乱码）。
    ///
    /// 这串 `U+FFFD` 是**真机实测**的形态：正因如此，按标签匹配一条都命中不了，
    /// 必须靠位置式回退。这个用例就是把那次误报钉死在回归里。
    const GBK_MOJIBAKE: &str = "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}                :  19,258,982,400 ( 17.9 GB)\n\
                                \u{FFFD}\u{FFFD}\u{FFFD}                     : 486,471,102,464 (453.1 GB)\n\
                                \u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}          :  19,258,982,400 ( 17.9 GB)\n\
                                \u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}          :               0 (  0.0 KB)";

    /// 英文控制台（ASCII）输出。
    const ASCII_EN: &str = "Total free bytes        :  19,258,982,400 ( 17.9 GB)\n\
                            Total bytes             : 486,471,102,464 (453.1 GB)";

    /// 关键回归：GBK 乱码下标签不可读，位置式回退仍须给出**真实**余量（而非 0）。
    #[test]
    fn parses_free_bytes_even_when_labels_are_mojibake() {
        assert_eq!(parse_by_label(GBK_MOJIBAKE), None, "乱码下标签匹配本就不该命中");
        assert_eq!(
            parse_fsutil_free_bytes(GBK_MOJIBAKE),
            Some(19_258_982_400),
            "位置式回退必须还原真实余量"
        );
    }

    /// 标签可读时走首选路径，且取的是「可用字节数」而非「总字节数」。
    #[test]
    fn prefers_label_match_when_readable() {
        assert_eq!(parse_fsutil_free_bytes(ASCII_EN), Some(19_258_982_400));
    }

    /// 中文标签可读（UTF-8 控制台）时同样命中。
    #[test]
    fn matches_chinese_label_when_decodable() {
        let text = "总可用字节数            :  19,258,982,400 ( 17.9 GB)\n\
                    总字节数                : 486,471,102,464 (453.1 GB)";
        assert_eq!(parse_fsutil_free_bytes(text), Some(19_258_982_400));
    }

    /// 空输出 / 无可解析数字 → None（调用方据此回退，而不是误判成 0 余量）。
    #[test]
    fn returns_none_on_unparsable_output() {
        assert_eq!(parse_fsutil_free_bytes(""), None);
        assert_eq!(parse_fsutil_free_bytes("   \n  \n"), None);
    }

    /// 千分位与括号中的辅助数字不得干扰（只取冒号后、左括号前的整数）。
    #[test]
    fn ignores_thousands_separators_and_parenthetical() {
        assert_eq!(extract_number_after_colon("x : 1,234 ( 5 GB)"), Some(1234));
        assert_eq!(extract_number_after_colon("x : 0 ( 0.0 KB)"), Some(0));
    }
}
