//! ffmpeg / ffprobe 二进制解析（内置优先，PATH 兜底）
//!
//! # 为什么需要（2026-10-24）
//!
//! 媒体桥（[`crate::tools::media`]）、代码生视频、Manim 生视频都要调 ffmpeg。
//! 此前一律 `Command::new("ffmpeg")`，完全依赖用户机器 PATH：
//!
//! - 安装版用户没装 ffmpeg → 所有视频能力直接不可用；
//! - 用户装了多个版本 → 用的是哪一个不可控，出了问题无法复现。
//!
//! 本模块把「二进制在哪」收敛成一条优先级链，安装包内置的那份永远优先：
//!
//! 1. 环境变量覆盖（`MARGUERITE_FFMPEG` / `MARGUERITE_FFPROBE` / `MARGUERITE_BIN_DIR`）
//! 2. exe 同级 `resources/bin/`（Tauri 安装布局）
//! 3. exe 同级 `bin/`
//! 4. exe 同级目录本身
//! 5. 从 exe 目录上溯 ≤3 层找 `resources/bin`（开发布局：`target/debug` → 仓库）
//! 6. 从 cwd 上溯 ≤3 层找 `gui/src-tauri/resources/bin`（`cargo tauri dev`）
//! 7. PATH 上第一个存在的 `<name>.exe`
//!
//! # 诚实边界
//!
//! 找不到就返回 `None`，由调用方给出**带安装命令**的错误；本模块不会假装存在，
//! 也不会悄悄下载。

use std::path::{Path, PathBuf};

/// 解析某个外部二进制（`ffmpeg` / `ffprobe`，不带扩展名）。
pub fn resolve(name: &str) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let cwd = std::env::current_dir().unwrap_or_default();
    let env_override = env_override_for(name);
    let path_dirs = path_dirs();
    pick(name, env_override.as_deref(), &exe_dir, &cwd, &path_dirs)
}

/// 解析并给出可执行的路径；找不到时返回带修复指引的错误文案。
pub fn resolve_or_error(name: &str) -> Result<PathBuf, String> {
    resolve(name).ok_or_else(|| missing_message(name))
}

/// 找不到二进制时的用户可见文案（统一口径，避免各工具各写一套）。
pub fn missing_message(name: &str) -> String {
    if name == "ffmpeg" || name == "ffprobe" {
        format!(
            "找不到 {name}。\n\
             修复方式（任选其一）：\n\
             ① 重新安装 Marguerite（安装包已内置 ffmpeg）；\n\
             ② Windows: winget install Gyan.FFmpeg 或 choco install ffmpeg；\n\
             ③ 设置环境变量 MARGUERITE_FFMPEG 指向 ffmpeg.exe 的绝对路径。"
        )
    } else {
        format!("找不到 {name}。请确认它已安装并在 PATH 中，或用环境变量指定绝对路径。")
    }
}

fn env_override_for(name: &str) -> Option<PathBuf> {
    let direct = match name {
        "ffmpeg" => std::env::var_os("MARGUERITE_FFMPEG"),
        "ffprobe" => std::env::var_os("MARGUERITE_FFPROBE"),
        _ => None,
    };
    if let Some(p) = direct.map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let dir = std::env::var_os("MARGUERITE_BIN_DIR").map(PathBuf::from)?;
    let cand = dir.join(file_name(name));
    cand.is_file().then_some(cand)
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

/// 平台相关文件名（Windows 补 `.exe`）。
fn file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// 纯函数：按优先级挑出第一个存在的文件。
///
/// 参数全部注入，便于单测断言优先级顺序而不触碰真实进程环境。
pub fn pick(
    name: &str,
    env_override: Option<&Path>,
    exe_dir: &Path,
    cwd: &Path,
    path_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(p) = env_override.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    let fname = file_name(name);
    for dir in bin_dir_candidates(exe_dir, cwd) {
        let cand = dir.join(&fname);
        if cand.is_file() {
            return Some(cand);
        }
    }
    for dir in path_dirs {
        let cand = dir.join(&fname);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 纯函数：按优先级列出可能放着二进制的目录（去重，保持顺序）。
pub fn bin_dir_candidates(exe_dir: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !p.as_os_str().is_empty() && !out.contains(&p) {
            out.push(p);
        }
    };
    if !exe_dir.as_os_str().is_empty() {
        push(exe_dir.join("resources").join("bin"));
        push(exe_dir.join("bin"));
        push(exe_dir.to_path_buf());
        for anc in exe_dir.ancestors().skip(1).take(3) {
            push(anc.join("resources").join("bin"));
        }
    }
    if !cwd.as_os_str().is_empty() {
        push(cwd.join("gui").join("src-tauri").join("resources").join("bin"));
        push(cwd.join("resources").join("bin"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmp(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("mq-ffbin-{}-{tag}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    fn touch(p: &Path) {
        if let Some(d) = p.parent() {
            fs::create_dir_all(d).expect("建父目录");
        }
        fs::write(p, b"x").expect("写文件");
    }

    #[test]
    fn env_override_beats_bundled_and_path() {
        let root = tmp("env");
        let envp = root.join("custom-ffmpeg.exe");
        touch(&envp);
        let exe = root.join("app");
        touch(&exe.join("resources").join("bin").join(file_name("ffmpeg")));
        let pathdir = root.join("pathdir");
        touch(&pathdir.join(file_name("ffmpeg")));

        let got = pick(
            "ffmpeg",
            Some(envp.as_path()),
            &exe,
            &root,
            &[pathdir],
        );
        assert_eq!(got.as_deref(), Some(envp.as_path()), "env 覆盖必须最优先");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bundled_resources_beat_path() {
        let root = tmp("bundled");
        let exe = root.join("app");
        let bundled = exe.join("resources").join("bin").join(file_name("ffmpeg"));
        touch(&bundled);
        let pathdir = root.join("pathdir");
        touch(&pathdir.join(file_name("ffmpeg")));

        let got = pick("ffmpeg", None, &exe, &root, &[pathdir]);
        assert_eq!(got.as_deref(), Some(bundled.as_path()), "内置必须压过 PATH");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn falls_back_to_path_when_nothing_bundled() {
        let root = tmp("pathonly");
        let exe = root.join("app");
        fs::create_dir_all(&exe).unwrap();
        let pathdir = root.join("pathdir");
        let onpath = pathdir.join(file_name("ffprobe"));
        touch(&onpath);

        let got = pick("ffprobe", None, &exe, &root, &[pathdir]);
        assert_eq!(got.as_deref(), Some(onpath.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn returns_none_when_absent() {
        let root = tmp("absent");
        let exe = root.join("app");
        fs::create_dir_all(&exe).unwrap();
        assert!(pick("ffmpeg", None, &exe, &root, &[]).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dev_layout_is_discovered_from_exe_ancestors() {
        let root = tmp("dev");
        let exe = root.join("target").join("debug");
        fs::create_dir_all(&exe).unwrap();
        // 仓库布局：<root>/resources/bin/ffmpeg.exe（exe 上溯 3 层命中）
        let bundled = root.join("resources").join("bin").join(file_name("ffmpeg"));
        touch(&bundled);
        let got = pick("ffmpeg", None, &exe, &root.join("elsewhere"), &[]);
        assert_eq!(got.as_deref(), Some(bundled.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_message_names_the_install_command() {
        let m = missing_message("ffmpeg");
        assert!(m.contains("winget install Gyan.FFmpeg"), "必须给可执行修复命令: {m}");
        assert!(m.contains("MARGUERITE_FFMPEG"), "必须给环境变量兜底: {m}");
        assert!(!missing_message("manim").contains("winget"), "非 ffmpeg 不复用该文案");
    }
}
