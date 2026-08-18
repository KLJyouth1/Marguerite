//! 路径安全：将任意（相对/绝对）路径解析到工作区内，拒绝逃逸

use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Result};

/// 将相对（或绝对）路径解析到工作区内，拒绝任何逃逸工作区的尝试。
///
/// 采用组件栈归一化（处理 `..` / `.`），再做存在性感知的 canonicalize，
/// 最后校验前缀是否仍在工作区根下。
pub fn safe_resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    // 统一以 canonical 根为基础，避免 `\\?\` 前缀/大小写导致的比较失效
    let root_can = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let rp = Path::new(rel);
    let base: PathBuf = if rp.is_absolute() {
        PathBuf::new()
    } else {
        root_can.clone()
    };
    let mut stack: Vec<Component> = base.components().collect();
    for comp in rp.components() {
        match comp {
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    let resolved: PathBuf = stack.iter().collect();

    if !resolved.starts_with(&root_can) {
        return Err(anyhow!("path escapes workspace: {}", rel));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escape() {
        let root = std::env::temp_dir();
        assert!(safe_resolve(&root, "../../etc/passwd").is_err());
        assert!(safe_resolve(&root, "/etc/passwd").is_err());
    }

    #[test]
    fn allows_internal() {
        let raw = std::env::temp_dir().join("marguerite_px");
        std::fs::create_dir_all(&raw).unwrap();
        let tmp = std::fs::canonicalize(&raw).unwrap_or(raw);
        let p = safe_resolve(&tmp, "src/main.rs").unwrap();
        assert!(p.starts_with(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
