//! OOXML / PDF 容器真实性校验（纯 Rust，零第三方依赖）。
//!
//! 完成门禁用它识破「伪办公交付物」：由 `write_file` 写出的纯文本
//! `presentation.pptx` 虽然非空，却不是合法 ZIP 容器，必须被拒。
//!
//! 本工作区没有 `zip`/`flate2` crate，故这里自行解析 ZIP 结构：定位
//! End-Of-Central-Directory（EOCD）→ 读总条目数 / 中央目录偏移 → 遍历中央
//! 目录条目收集文件名。只支持普通（非 ZIP64）归档。

use std::path::Path;

/// 本地文件头签名 `PK\x03\x04`。
const LFH_SIG: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
/// 中央目录条目签名 `PK\x01\x02`。
const CD_SIG: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
/// End-Of-Central-Directory 签名 `PK\x05\x06`。
const EOCD_SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
/// ZIP64 EOCD 定位器签名 `PK\x06\x07`。
const ZIP64_LOCATOR_SIG: [u8; 4] = [0x50, 0x4B, 0x06, 0x07];

/// EOCD 固定长度（不含注释字段内容）。
const EOCD_MIN_LEN: usize = 22;
/// 从文件尾回溯定位 EOCD 的最大窗口：注释上限 65535 + EOCD 22 字节。
const EOCD_SCAN_MAX: usize = 65_557;

fn u16_le(b: &[u8], off: usize) -> Option<u16> {
    let end = off.checked_add(2)?;
    let s = b.get(off..end)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_le(b: &[u8], off: usize) -> Option<u32> {
    let end = off.checked_add(4)?;
    let s = b.get(off..end)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// 在文件尾部窗口内定位 EOCD 记录起始偏移（取最靠后且自洽的一个）。
///
/// 自洽判据：EOCD 声明的注释长度必须正好补到文件尾，避免误命中数据区里的假签名。
fn find_eocd(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < EOCD_MIN_LEN {
        return None;
    }
    let start = bytes.len().saturating_sub(EOCD_SCAN_MAX);
    let last = bytes.len() - 4;
    for i in (start..=last).rev() {
        if bytes[i..i + 4] == EOCD_SIG {
            let comment_len = u16_le(bytes, i + 20).unwrap_or(u16::MAX) as usize;
            if i + EOCD_MIN_LEN + comment_len == bytes.len() {
                return Some(i);
            }
        }
    }
    None
}

/// 是否为 ZIP 容器：以本地文件头 `PK\x03\x04` 开头，且存在 EOCD 记录。
pub fn is_zip_container(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == LFH_SIG && find_eocd(bytes).is_some()
}

/// 读取 ZIP 并返回其中所有条目名（仅支持普通非 ZIP64 归档）。
///
/// 任一环节失败（非 ZIP / 截断 / 偏移越界 / ZIP64）均返回可读的 `Err`。
pub fn list_zip_entry_names(path: &Path) -> Result<Vec<String>, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("无法读取文件 {}: {e}", path.display()))?;
    let eocd = find_eocd(&bytes)
        .ok_or_else(|| "不是 ZIP 容器：未找到 End-Of-Central-Directory 记录".to_string())?;

    let total_entries = u16_le(&bytes, eocd + 10)
        .ok_or_else(|| "ZIP EOCD 截断：缺少总条目数".to_string())? as usize;
    let cd_offset = u32_le(&bytes, eocd + 16)
        .ok_or_else(|| "ZIP EOCD 截断：缺少中央目录偏移".to_string())? as usize;

    // ZIP64：哨兵值或紧邻 EOCD 前的 ZIP64 定位器 → 明确拒绝。
    let has_zip64_locator = eocd >= 20 && bytes[eocd - 20..eocd - 16] == ZIP64_LOCATOR_SIG;
    if total_entries == 0xFFFF || cd_offset == 0xFFFF_FFFF || has_zip64_locator {
        return Err("ZIP64 归档暂不支持（仅支持普通 ZIP）".to_string());
    }

    if cd_offset >= bytes.len() {
        return Err(format!(
            "ZIP 中央目录偏移越界：{cd_offset} >= {}",
            bytes.len()
        ));
    }

    let mut names = Vec::with_capacity(total_entries);
    let mut pos = cd_offset;
    for idx in 0..total_entries {
        if pos + 4 > bytes.len() || bytes[pos..pos + 4] != CD_SIG {
            return Err(format!("中央目录第 {idx} 项签名非法（期望 PK\\x01\\x02）"));
        }
        let name_len = u16_le(&bytes, pos + 28)
            .ok_or_else(|| format!("中央目录第 {idx} 项截断：缺少文件名长度"))?
            as usize;
        let extra_len = u16_le(&bytes, pos + 30)
            .ok_or_else(|| format!("中央目录第 {idx} 项截断：缺少扩展字段长度"))?
            as usize;
        let comment_len = u16_le(&bytes, pos + 32)
            .ok_or_else(|| format!("中央目录第 {idx} 项截断：缺少注释长度"))?
            as usize;

        let name_start = pos + 46;
        let name_end = name_start + name_len;
        if name_end > bytes.len() {
            return Err(format!("中央目录第 {idx} 项文件名越界"));
        }
        names.push(String::from_utf8_lossy(&bytes[name_start..name_end]).to_string());
        pos = name_end + extra_len + comment_len;
    }
    Ok(names)
}

/// 按扩展名校验办公交付物容器真实性。未知扩展名一律 no-op 通过。
pub fn validate_office_container(path: &Path) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "docx" => require_zip_entries(path, &["[Content_Types].xml", "word/document.xml"]),
        "xlsx" => require_zip_entries(path, &["[Content_Types].xml", "xl/workbook.xml"]),
        "pptx" => require_zip_entries(path, &["[Content_Types].xml", "ppt/presentation.xml"]),
        "pdf" => {
            let bytes = std::fs::read(path)
                .map_err(|e| format!("无法读取 PDF 文件 {}: {e}", path.display()))?;
            if bytes.starts_with(b"%PDF-") {
                Ok(())
            } else {
                Err("not a valid PDF container: 缺少 %PDF- 文件头".to_string())
            }
        }
        _ => Ok(()),
    }
}

/// 要求文件是 ZIP 容器且包含全部 `required` 条目名。
fn require_zip_entries(path: &Path, required: &[&str]) -> Result<(), String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("无法读取文件 {}: {e}", path.display()))?;
    if !is_zip_container(&bytes) {
        return Err(
            "not a valid OOXML container: 不是 ZIP 归档（缺少 PK\\x03\\x04 头或 EOCD）"
                .to_string(),
        );
    }
    let names = list_zip_entry_names(path)?;
    for req in required {
        if !names.iter().any(|n| n == req) {
            return Err(format!("not a valid OOXML container: 缺少必需条目 {req}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let d =
            std::env::temp_dir().join(format!("marg_ooxml_{}_{}_{}", std::process::id(), tag, n));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// 构造一个**未压缩（stored）**的最小 ZIP：本地头 + 空数据 + 中央目录 + EOCD。
    fn build_stored_zip(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(out.len() as u32);
            out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // compression = stored
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc32
            out.extend_from_slice(&0u32.to_le_bytes()); // compressed size
            out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(name.as_bytes());
        }
        let cd_start = out.len() as u32;
        for (i, name) in names.iter().enumerate() {
            out.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
            out.extend_from_slice(&20u16.to_le_bytes()); // version made by
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // compression
            out.extend_from_slice(&0u16.to_le_bytes()); // mod time
            out.extend_from_slice(&0u16.to_le_bytes()); // mod date
            out.extend_from_slice(&0u32.to_le_bytes()); // crc32
            out.extend_from_slice(&0u32.to_le_bytes()); // compressed size
            out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed size
            out.extend_from_slice(&(name.len() as u16).to_le_bytes()); // name len
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(&0u16.to_le_bytes()); // comment len
            out.extend_from_slice(&0u16.to_le_bytes()); // disk start
            out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            out.extend_from_slice(&offsets[i].to_le_bytes()); // local header offset
            out.extend_from_slice(name.as_bytes());
        }
        let cd_size = out.len() as u32 - cd_start;
        out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
        out.extend_from_slice(&0u16.to_le_bytes()); // disk number
        out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
        out.extend_from_slice(&(names.len() as u16).to_le_bytes()); // entries this disk
        out.extend_from_slice(&(names.len() as u16).to_le_bytes()); // total entries
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        out
    }

    #[test]
    fn is_zip_container_accepts_minimal_zip_and_rejects_text() {
        let zip = build_stored_zip(&["[Content_Types].xml"]);
        assert!(is_zip_container(&zip), "合法最小 ZIP 应被识别");
        assert!(!is_zip_container(b"this is not a zip"));
        assert!(!is_zip_container(b""));
    }

    #[test]
    fn fake_pptx_plain_text_is_rejected() {
        let d = tmp_dir("fake_pptx");
        let p = d.join("presentation.pptx");
        std::fs::write(&p, "just plain text pretending to be a pptx").unwrap();
        assert!(validate_office_container(&p).is_err(), "纯文本 .pptx 必须被拒");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn fake_docx_text_file_is_rejected() {
        let d = tmp_dir("fake_docx");
        let p = d.join("report.docx");
        std::fs::write(&p, "not a docx at all").unwrap();
        assert!(validate_office_container(&p).is_err(), "纯文本 .docx 必须被拒");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn genuine_minimal_zip_with_required_entries_is_accepted() {
        let d = tmp_dir("real_pptx");
        let p = d.join("deck.pptx");
        let zip = build_stored_zip(&["[Content_Types].xml", "ppt/presentation.xml"]);
        std::fs::write(&p, &zip).unwrap();
        assert!(validate_office_container(&p).is_ok(), "含必需条目的真 ZIP 应通过");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_missing_required_entry_is_rejected() {
        let d = tmp_dir("missing_entry");
        let p = d.join("deck.pptx");
        let zip = build_stored_zip(&["[Content_Types].xml"]);
        std::fs::write(&p, &zip).unwrap();
        assert!(validate_office_container(&p).is_err(), "缺 ppt/presentation.xml 应被拒");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn txt_artifact_is_unaffected() {
        let d = tmp_dir("txt");
        let p = d.join("notes.txt");
        std::fs::write(&p, "plain notes").unwrap();
        assert!(validate_office_container(&p).is_ok(), ".txt 应 no-op 通过");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pdf_header_is_required() {
        let d = tmp_dir("pdf");
        let good = d.join("a.pdf");
        std::fs::write(&good, b"%PDF-1.7\n...").unwrap();
        assert!(validate_office_container(&good).is_ok(), "%PDF- 头应通过");
        let bad = d.join("b.pdf");
        std::fs::write(&bad, "not a pdf").unwrap();
        assert!(validate_office_container(&bad).is_err(), "缺 %PDF- 头应被拒");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn list_zip_entry_names_reads_names() {
        let d = tmp_dir("names");
        let p = d.join("a.zip");
        let zip = build_stored_zip(&["[Content_Types].xml", "word/document.xml"]);
        std::fs::write(&p, &zip).unwrap();
        let names = list_zip_entry_names(&p).unwrap();
        assert_eq!(
            names,
            vec!["[Content_Types].xml".to_string(), "word/document.xml".to_string()]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn list_zip_entry_names_errors_on_non_zip() {
        let d = tmp_dir("nonzip");
        let p = d.join("a.zip");
        std::fs::write(&p, "not a zip").unwrap();
        assert!(list_zip_entry_names(&p).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
