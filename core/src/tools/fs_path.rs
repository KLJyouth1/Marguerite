//! 路径安全：将任意（相对/绝对）路径解析到工作区内，拒绝逃逸

use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Result};

/// 逃逸错误文案（P0-1b）：必须自带工作区根 + 可执行动作。
///
/// 旧文案只有一句 `path escapes workspace: <rel>` —— 模型既不知道根在哪，
/// 也不知道"相对什么"才算合法，于是同一个错犯到触发重复失败检测为止。
/// 所有文件工具都经这里返回错误，改这一处即覆盖整个工具面。
fn escape_err(root: &Path, rel: &str, via_symlink: bool) -> anyhow::Error {
    let kind = if via_symlink {
        "path escapes workspace via symlink"
    } else {
        "path escapes workspace"
    };
    anyhow!(
        "{kind} (root: {root}): {rel} —— 请改用该目录内的相对路径重新调用",
        root = root.display(),
        rel = rel
    )
}

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

    // Windows canonicalize 产生 `\\?\` 前缀，root_can 可能也有；
    // 统一 trim 后比较前缀，避免因前缀差异误判逃逸。
    let root_norm = root_can.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let resolved_norm = resolved.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    if !resolved_norm.starts_with(&root_norm) {
        return Err(escape_err(root, rel, false));
    }

    // 存在性感知的 canonicalize：组件栈归一化只能挡住 `..`/绝对路径，
    // 挡不住「工作区内的软链指向工作区外」——`root/link -> /etc` 时
    // `link/passwd` 前缀仍是 root 但真实落点是 /etc/passwd。
    // 路径可能尚未创建（写新文件），故对「最近的存在祖先」canonicalize 后再校验。
    let real = canonicalize_existing_ancestor(&resolved);
    // Windows canonicalize 产生 `\\?\` 前缀，root_can 可能也有；
    // 统一 trim 后比较前缀，避免因前缀差异误判逃逸。
    let root_norm = root_can.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let real_norm = real.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    if !real_norm.starts_with(&root_norm) {
        return Err(escape_err(root, rel, true));
    }
    Ok(resolved)
}

/// 对 `p` 做「存在性感知」的 canonicalize：向上找到最近的存在祖先，
/// canonicalize 后把剩余（尚不存在）的组件拼回。用于在目标路径尚未创建时
/// 也能识别软链逃逸。
fn canonicalize_existing_ancestor(p: &Path) -> PathBuf {
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(c) = std::fs::canonicalize(cur) {
            let mut out = c;
            for s in suffix.iter().rev() {
                out.push(s);
            }
            return out;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                suffix.push(name.to_os_string());
                cur = parent;
            }
            // 已到根仍不存在：退回原路径（由调用方的前缀校验兜底）
            _ => return p.to_path_buf(),
        }
    }
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

    /// 缺陷回归（P0-1b）：逃逸错误必须自带工作区根和下一步动作。
    ///
    /// 旧文案只有 `path escapes workspace: <rel>`：模型不知道根在哪、也不知道
    /// 相对谁合法，于是同一个错重复到触发重复失败检测——"可执行提示而非硬失败"
    /// 在错误侧的第一公里。
    #[test]
    fn escape_error_names_the_root_and_next_action() {
        let root = std::env::temp_dir().join("mgt_px_msg_root");
        std::fs::create_dir_all(&root).unwrap();
        let err = safe_resolve(&root, "../../etc/passwd")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&root.display().to_string()),
            "必须写出工作区根，模型才知道相对谁合法：{err}"
        );
        assert!(
            err.contains("相对路径"),
            "必须给出下一步动作，否则模型只会重试同一个错：{err}"
        );
        assert!(
            err.contains("../../etc/passwd"),
            "必须回显被拒路径，便于定位：{err}"
        );
        let _ = std::fs::remove_dir_all(&root);
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

    /// 软链逃逸：工作区内 `link -> 工作区外` 时，`link/secret` 前缀虽在 root 下，
    /// 真实落点却在 root 外 —— 必须拒绝。
    #[test]
    fn rejects_symlink_escape() {
        let root_raw = std::env::temp_dir().join(format!("mgt_px_root_{}", std::process::id()));
        let outside_raw = std::env::temp_dir().join(format!("mgt_px_out_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root_raw);
        let _ = std::fs::remove_dir_all(&outside_raw);
        std::fs::create_dir_all(&root_raw).unwrap();
        std::fs::create_dir_all(&outside_raw).unwrap();
        std::fs::write(outside_raw.join("secret.txt"), "top secret").unwrap();

        let root = std::fs::canonicalize(&root_raw).unwrap_or(root_raw.clone());
        let link = root.join("link");

        // Windows 创建符号链接可能需要开发者模式/管理员权限；不可用时跳过断言。
        let made = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&outside_raw, &link).is_ok()
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(&outside_raw, &link).is_ok()
            }
            #[cfg(not(any(unix, windows)))]
            {
                false
            }
        };

        if made {
            assert!(
                safe_resolve(&root, "link/secret.txt").is_err(),
                "经软链逃逸工作区必须被拒绝"
            );
            // 工作区内正常路径仍应放行
            assert!(safe_resolve(&root, "inside.txt").is_ok());
        }

        let _ = std::fs::remove_dir_all(&root_raw);
        let _ = std::fs::remove_dir_all(&outside_raw);
    }
}
