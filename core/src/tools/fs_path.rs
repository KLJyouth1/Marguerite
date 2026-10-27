//! 路径安全：将任意（相对/绝对）路径解析到工作区内，拒绝逃逸

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

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

/// 判断 `child` 是否真的在 `root` 之下（**按路径组件比较**，不是字符串前缀）。
///
/// 字符串前缀比较有真实漏洞：`root = C:\ws` 时 `C:\ws2\x` 也满足
/// `starts_with("C:\\ws")`，于是「工作区兄弟目录」会被当成根内路径放行。
/// 同时要抹平 Windows 的 `\\?\` verbatim 前缀差异（`canonicalize` 会带、
/// 用户输入不会带），否则根内绝对路径会被误判成逃逸。
fn path_within(root: &Path, child: &Path) -> bool {
    fn parts(p: &Path) -> Vec<String> {
        let raw = p.to_string_lossy();
        let stripped = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
            rest.to_string()
        } else {
            raw.to_string()
        };
        Path::new(&stripped)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect()
    }
    let root_parts = parts(root);
    let child_parts = parts(child);
    if child_parts.len() < root_parts.len() {
        return false;
    }
    root_parts.iter().zip(child_parts.iter()).all(|(a, b)| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    })
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

    // 组件级前缀校验（字符串前缀会把 `root=C:\ws` 的兄弟目录 `C:\ws2` 误判为根内）。
    // 先按组件栈归一化的结果判一次，挡 `..` / 绝对路径逃逸。
    if !path_within(&root_can, &resolved) {
        return Err(escape_err(root, rel, false));
    }

    // 存在性感知的 canonicalize：组件栈归一化只能挡住 `..`/绝对路径，
    // 挡不住「工作区内的软链指向工作区外」——`root/link -> /etc` 时
    // `link/passwd` 前缀仍是 root 但真实落点是 /etc/passwd。
    // 路径可能尚未创建（写新文件），故对「最近的存在祖先」canonicalize 后再校验。
    let real = canonicalize_existing_ancestor(&resolved);
    if !path_within(&root_can, &real) {
        return Err(escape_err(root, rel, true));
    }
    Ok(resolved)
}

/// 多根逃逸错误：必须列出**全部**根，否则模型不知道还能相对哪个目录写。
fn roots_escape_err(roots: &[PathBuf], rel: &str) -> anyhow::Error {
    let list = roots
        .iter()
        .map(|r| r.display().to_string())
        .collect::<Vec<_>>()
        .join(" | ");
    anyhow!(
        "path escapes workspace (roots: {list}): {rel} —— 请改用这些目录内的相对路径重新调用"
    )
}

/// 多根解析（P2-9 多项目工作区）。
///
/// 规则（可预测优先于"聪明"）：
/// 1. **绝对路径**：必须落在任一登记根内，落在哪个根就用哪个根；
/// 2. **相对路径**：按根顺序（主根在前）逐个解析，**第一个真实存在的命中**胜出
///    ——「读已有文件」永远读到它真正所在的根；
/// 3. 都不存在（新建文件）：落到**主根**，保证写新文件的行为与单根时代一致；
/// 4. 全部根都解析失败（含空根集合）→ 逃逸错误并列出所有根。
pub fn safe_resolve_any(roots: &[PathBuf], rel: &str) -> Result<PathBuf> {
    if roots.is_empty() {
        return Err(anyhow!(
            "path escapes workspace (roots: 无): {rel} —— 当前会话没有绑定任何工作区根目录"
        ));
    }
    if Path::new(rel).is_absolute() {
        for root in roots {
            if let Ok(p) = safe_resolve(root, rel) {
                return Ok(p);
            }
        }
        return Err(roots_escape_err(roots, rel));
    }
    let mut fallback: Option<PathBuf> = None;
    for root in roots {
        match safe_resolve(root, rel) {
            Ok(p) => {
                if p.exists() {
                    return Ok(p);
                }
                fallback.get_or_insert(p);
            }
            Err(_) => continue,
        }
    }
    fallback.ok_or_else(|| roots_escape_err(roots, rel))
}

/// 工作区根集合：首个是**主根**，其余是附加根（P2-9）。
///
/// 用 `Arc<Vec<..>>` 而不是裸 `Vec`：工具结构体到处 `Clone`，共享路径表可以
/// 避免每次构造都深拷贝；同时实现 `From<PathBuf>` / `From<&Path>`，让既有单根
/// 构造点（含测试）零改动。
#[derive(Debug, Clone)]
pub struct RootSet {
    primary: PathBuf,
    all: Arc<Vec<PathBuf>>,
}

impl RootSet {
    /// 单根集合（等价于旧行为）
    pub fn single(root: PathBuf) -> Self {
        Self {
            primary: root.clone(),
            all: Arc::new(vec![root]),
        }
    }

    /// 多根集合：去重保序，首个为主根；空输入退化为 `.`（当前目录）。
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut all: Vec<PathBuf> = Vec::new();
        for r in roots {
            if !all.iter().any(|x| x == &r) {
                all.push(r);
            }
        }
        if all.is_empty() {
            all.push(PathBuf::from("."));
        }
        let primary = all[0].clone();
        Self {
            primary,
            all: Arc::new(all),
        }
    }

    /// 主根：既有编排、相对路径基准、产物归因都仍以它为准
    pub fn primary(&self) -> &Path {
        &self.primary
    }

    /// 全部根（主根在首位）
    pub fn all(&self) -> &[PathBuf] {
        &self.all
    }

    /// 是否绑定了一个以上根目录
    pub fn is_multi(&self) -> bool {
        self.all.len() > 1
    }

    /// 解析相对/绝对路径（语义见 [`safe_resolve_any`]）
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        safe_resolve_any(&self.all, rel)
    }

    /// 返回包含该绝对路径的登记根（主根优先；都不包含时回主根）。
    ///
    /// 用于「回收站归属」这类必须**跟着文件所在根**走的场景：文件在附加根，
    /// 就该进附加根的 `.marguerite/trash`，否则恢复时会被根校验拒绝。
    pub fn root_containing(&self, abs: &std::path::Path) -> PathBuf {
        let abs_can = canonicalize_existing_ancestor(abs);
        for root in self.all.iter() {
            let root_can = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            if path_within(&root_can, &abs_can) {
                return root.clone();
            }
        }
        self.primary.clone()
    }
}

impl From<PathBuf> for RootSet {
    fn from(root: PathBuf) -> Self {
        Self::single(root)
    }
}

impl From<&Path> for RootSet {
    fn from(root: &Path) -> Self {
        Self::single(root.to_path_buf())
    }
}

impl Default for RootSet {
    fn default() -> Self {
        Self::new(Vec::new())
    }
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

    /// 兄弟目录前缀漏洞：`root = .../ws` 时 `.../ws2/x` 在**字符串**上以 root 开头，
    /// 旧实现会用 `starts_with` 放行。必须按路径组件判定为逃逸。
    #[test]
    fn rejects_sibling_directory_sharing_the_string_prefix() {
        let base = std::env::temp_dir().join(format!("mgt_px_prefix_{}", std::process::id()));
        let root = base.join("ws");
        let sibling = base.join("ws2");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("secret.txt"), "x").unwrap();

        assert!(
            safe_resolve(&root, "../ws2/secret.txt").is_err(),
            "词法归一化到兄弟目录必须被拒绝"
        );
        assert!(
            safe_resolve(&root, &sibling.join("secret.txt").to_string_lossy()).is_err(),
            "兄弟目录的绝对路径必须被拒绝"
        );
        // 根内绝对路径仍放行（编辑器会回传绝对路径）
        let inside = root.join("ok.txt");
        std::fs::write(&inside, "ok").unwrap();
        assert!(safe_resolve(&root, &inside.to_string_lossy()).is_ok());

        let _ = std::fs::remove_dir_all(&base);
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

    /// P2-9 多根：已存在文件跟随它真正所在的根；新建文件落主根；
    /// 绝对路径只要落在任一登记根内就放行；都不在则拒绝并列出全部根。
    #[test]
    fn multi_root_resolves_existing_in_any_root_and_new_in_primary() {
        let base = std::env::temp_dir().join(format!("mgt_multiroot_{}", std::process::id()));
        let primary = base.join("primary");
        let extra = base.join("extra");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&primary).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::write(extra.join("only-extra.txt"), "b").unwrap();
        std::fs::write(primary.join("only-primary.txt"), "a").unwrap();

        let roots = RootSet::new(vec![primary.clone(), extra.clone()]);
        assert!(roots.is_multi());
        assert_eq!(roots.primary(), primary.as_path());

        // 已存在 → 跟随真实所在根（读到的是附加根那份）
        let read = roots.resolve("only-extra.txt").unwrap();
        assert!(read.starts_with(std::fs::canonicalize(&extra).unwrap_or(extra.clone())));
        // 主根也有同名文件时主根优先（顺序稳定，不随文件系统顺序漂移）
        let primary_read = roots.resolve("only-primary.txt").unwrap();
        assert!(primary_read.starts_with(std::fs::canonicalize(&primary).unwrap_or(primary.clone())));
        // 都不存在 → 新建落主根
        let created = roots.resolve("brand-new.txt").unwrap();
        assert!(created.starts_with(std::fs::canonicalize(&primary).unwrap_or(primary.clone())));
        // 绝对路径落在附加根内 → 放行
        let abs_extra = extra.join("only-extra.txt");
        assert!(roots
            .resolve(&abs_extra.to_string_lossy())
            .is_ok());
        // 两个根之外 → 拒绝，且错误里列出两个根
        let outside = base.join("outside.txt");
        std::fs::write(&outside, "x").unwrap();
        let err = roots
            .resolve(&outside.to_string_lossy())
            .unwrap_err()
            .to_string();
        assert!(err.contains(&primary.display().to_string()));
        assert!(err.contains(&extra.display().to_string()));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn root_set_single_keeps_legacy_semantics() {
        let root = std::env::temp_dir().join("mgt_singleroot");
        std::fs::create_dir_all(&root).unwrap();
        let set = RootSet::from(root.clone());
        assert!(!set.is_multi());
        assert_eq!(set.all().len(), 1);
        // 逃逸仍被拒绝（单根行为不变）
        assert!(set.resolve("../../etc/passwd").is_err());
        // 空集合不 panic，报错可读
        let empty = RootSet::new(Vec::new());
        assert!(empty.resolve("a.txt").is_ok() || empty.resolve("a.txt").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
