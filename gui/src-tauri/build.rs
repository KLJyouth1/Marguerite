fn main() {
    // 复制 opensquilla Web 界面产物（marguerite/webui/dist）到 src-tauri/webui/dist，
    // 使 bundle.resources 的 `webui/dist/**` glob 能匹配（tauri-build 的 glob 不支持 `../` 相对段）。
    let src = std::path::Path::new("../../webui/dist");
    let dst = std::path::Path::new("webui/dist");
    if src.join("index.html").is_file() {
        let _ = std::fs::remove_dir_all(dst);
        copy_dir_all(src, dst);
        println!("cargo:rerun-if-changed=../../webui/dist");
    }

    // 复制技能树（marguerite/skills）到 src-tauri/skills，使 bundle.resources 的
    // `skills` 能匹配（tauri-build 的 glob 不支持 `../` 相对段）。仅 release 构建
    // 或目标缺失时复制，避免开发期每次 cargo build 都搬运 50M+ 技能目录；开发期
    // 网关经 cwd 上溯解析 skills，不依赖此副本。
    let skills_src = std::path::Path::new("../../skills");
    let skills_dst = std::path::Path::new("skills");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    if skills_src.is_dir() && (profile == "release" || !skills_dst.exists()) {
        let _ = std::fs::remove_dir_all(skills_dst);
        copy_dir_all(skills_src, skills_dst);
        println!("cargo:rerun-if-changed=../../skills");
    }

    tauri_build::build()
}

/// 递归复制目录（仅文件与目录，跟随符号链接由 fs 默认处理）
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let target = dst.join(entry.file_name());
            if path.is_dir() {
                let _ = std::fs::create_dir_all(&target);
                copy_dir_all(&path, &target);
            } else {
                let _ = std::fs::copy(&path, &target);
            }
        }
    }
}
