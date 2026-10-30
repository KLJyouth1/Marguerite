//! doctor 第 8 项：ML 路由权重完整性（SquillaRouter v4.2 bundle）。
//!
//! # 为什么需要这一项（M02）
//! `squilla_router` v4.2 的权重在仓库里**全是 Git LFS 指针**（`lgbm_main.bin`
//! 磁盘 133 B vs 清单 39,684,725 B），本机未装 `git-lfs`。后果是「自动模型路由」
//! 从未真正可用，而失败只体现为**开关是开的**——用户没有任何可亲验的证据。
//!
//! 本项把「权重到底在不在」变成一条可亲验的体检结果：
//! - 只读文件、**不起子进程**（因此不进 `cached_expensive_item` 的昂贵项缓存）；
//! - 不 import lightgbm/torch（那正是坏掉的东西）；
//! - 与 Python 侧 `inspect_bundle()` 同口径：**指针判定先于尺寸判定**。
//!
//! # 定级口径
//! ML 路由是**可选增强**：权重缺失时引擎回退规则路由，主功能不受影响。
//! 因此三种缺失一律 `DEGRADED`（如实标注、给修复动作），**不判 MISSING**——
//! 判 MISSING 会让发布门禁在「没跑过 git lfs pull 的开发机」上无谓变红。

use std::io::Read;
use std::path::{Path, PathBuf};

use super::{degraded_item, ok_item, DoctorItem};

/// bundle 在 pyengine 源码树内的相对路径（与 py 侧 `default_bundle_dir()` 同源）。
const BUNDLE_REL: &str = "opensquilla/squilla_router/models/v4.2_phase3_inference";

/// bundle 布局必备项（与 py 侧 `_REQUIRED_BUNDLE_FILES` 同源）。
const REQUIRED_BUNDLE_FILES: [&str; 2] = ["runtime_src", "router.runtime.yaml"];

/// Git LFS 指针文件的魔数头（与 py 侧 `_GIT_LFS_POINTER_PREFIX` 逐字节相同）。
const LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";

const LFS_FIX_HINT: &str = "权重是 Git LFS 指针而非真实模型数据：在源码检出中执行 \
`git lfs install && git lfs pull --include=\"src/opensquilla/squilla_router/models/**\"`，\
或安装随包附带权重的正式发布版。在此之前自动模型路由回退默认档（功能不受影响）。";

const MISSING_FIX_HINT: &str = "重装 Marguerite 以补齐 pyengine 资源；若为源码检出，\
请确认 `pyengine/src/opensquilla/squilla_router/models/` 完整。";

const PYENGINE_FIX_HINT: &str = "未定位到 pyengine 源码树（需含 `opensquilla/cli/main.py`）。\
请完整安装 Marguerite，或用 `MARGUERITE_PYENGINE_SRC` 指定源码根。";

/// 清单校验结果。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ManifestScan {
    /// 人类可读问题列表（顺序与清单一致）。
    pub problems: Vec<String>,
    /// 明确是 Git LFS 指针的文件（`problems` 的子集）。
    pub pointers: Vec<String>,
    /// 通过校验的真实文件数。
    pub checked: usize,
}

/// 逐条核对 `artifact_manifest.json` 与磁盘字节。
///
/// 清单缺失时返回空结果（与 py 侧一致：没有清单就不算问题，不误报）。
pub(crate) fn scan_manifest(bundle: &Path) -> ManifestScan {
    let mut scan = ManifestScan::default();

    let Ok(raw) = std::fs::read_to_string(bundle.join("artifact_manifest.json")) else {
        return scan;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&raw) else {
        scan.problems
            .push("artifact_manifest.json: unreadable".to_string());
        return scan;
    };
    let Some(files) = manifest.get("files").and_then(|v| v.as_array()) else {
        return scan;
    };

    for entry in files {
        let Some(rel) = entry.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        // 与 py 侧同口径拒绝绝对路径与 `..`（清单本身不可信时不给它逃逸的机会）。
        if Path::new(rel).is_absolute() || rel.split(['/', '\\']).any(|c| c == "..") {
            scan.problems
                .push(format!("{rel}: invalid manifest path"));
            continue;
        }

        let path = bundle.join(rel);
        let Ok(meta) = std::fs::metadata(&path) else {
            scan.problems.push(format!("{rel}: missing"));
            continue;
        };

        // 指针判定**必须先于**尺寸判定：指针恒小于它所代表的产物，尺寸优先会把
        // 一条 `git lfs pull` 能修好的问题报成「尺寸漂移」——这正是本项存在的原因。
        if let Ok(mut f) = std::fs::File::open(&path) {
            let mut head = vec![0u8; LFS_POINTER_PREFIX.len()];
            let n = f.read(&mut head).unwrap_or(0);
            if n == LFS_POINTER_PREFIX.len() && head == LFS_POINTER_PREFIX {
                scan.problems
                    .push(format!("{rel}: Git LFS pointer, not model data"));
                scan.pointers.push(rel.to_string());
                continue;
            }
        }

        if let Some(expected) = entry.get("size_bytes").and_then(|v| v.as_u64()) {
            if meta.len() != expected {
                scan.problems
                    .push(format!("{rel}: size {} != manifest {expected}", meta.len()));
                continue;
            }
        }

        scan.checked += 1;
    }

    scan
}

/// bundle 目录（由 pyengine 源码根推导）。
pub(crate) fn bundle_path(pyengine_src: &Path) -> PathBuf {
    pyengine_src.join(BUNDLE_REL)
}

/// 纯函数体检：`None` = 未定位到 pyengine 源码树。
///
/// 抽成接收显式路径的函数，单测才能喂临时目录而不依赖本机检出。
pub(crate) fn inspect(bundle: Option<&Path>) -> DoctorItem {
    const ID: &str = "router_weights";
    const LABEL: &str = "路由权重完整性";

    let Some(bundle) = bundle else {
        return degraded_item(ID, LABEL, "未定位到 pyengine 源码树".to_string(), PYENGINE_FIX_HINT.to_string());
    };

    if !bundle.is_dir() {
        return degraded_item(
            ID,
            LABEL,
            format!("路由权重目录不存在：{}", bundle.display()),
            MISSING_FIX_HINT.to_string(),
        );
    }

    let missing: Vec<&str> = REQUIRED_BUNDLE_FILES
        .iter()
        .copied()
        .filter(|name| !bundle.join(name).exists())
        .collect();
    if !missing.is_empty() {
        return degraded_item(
            ID,
            LABEL,
            format!("路由 bundle 布局不完整，缺少：{}", missing.join(", ")),
            MISSING_FIX_HINT.to_string(),
        );
    }

    let scan = scan_manifest(bundle);
    if let Some(first_pointer) = scan.pointers.first() {
        return degraded_item(
            ID,
            LABEL,
            format!(
                "路由权重未拉取：{} 个文件是 Git LFS 指针（如 {first_pointer}），非真实模型数据；\
                 自动模型路由已回退默认档",
                scan.pointers.len()
            ),
            LFS_FIX_HINT.to_string(),
        );
    }
    if let Some(problem) = scan.problems.first() {
        return degraded_item(
            ID,
            LABEL,
            format!("路由 bundle 与清单不符：{problem}"),
            MISSING_FIX_HINT.to_string(),
        );
    }

    ok_item(
        ID,
        LABEL,
        format!(
            "{} 个权重文件经清单校验（尺寸一致、非 Git LFS 指针）",
            scan.checked
        ),
    )
}

/// 体检入口：定位 pyengine 源码 → 推导 bundle → 校验。
pub(crate) fn check_router_weights() -> DoctorItem {
    let bundle = crate::pyengine::sidecar::locate_pyengine_src().map(|src| bundle_path(&src));
    inspect(bundle.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Status;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mgt_router_weights_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    fn write_bundle(bundle: &Path, files: &[(&str, &[u8])]) {
        std::fs::create_dir_all(bundle.join("runtime_src")).expect("建 runtime_src");
        std::fs::write(bundle.join("router.runtime.yaml"), "v4: {}\n").expect("写 yaml");
        let entries: Vec<serde_json::Value> = files
            .iter()
            .map(|(rel, bytes)| {
                let target = bundle.join(rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).expect("建父目录");
                }
                std::fs::write(&target, bytes).expect("写权重文件");
                serde_json::json!({ "path": rel, "size_bytes": bytes.len() })
            })
            .collect();
        let manifest = serde_json::json!({ "files": entries });
        std::fs::write(
            bundle.join("artifact_manifest.json"),
            serde_json::to_string(&manifest).expect("序列化清单"),
        )
        .expect("写清单");
    }

    /// LFS 指针常量必须是**真**指针头，否则本项会静默失效。
    #[test]
    fn lfs_prefix_matches_a_real_pointer_file() {
        let dir = temp_dir("prefix");
        std::fs::write(
            dir.join("lgbm_main.bin"),
            "version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 39684725\n",
        )
        .expect("写指针");
        let mut f = std::fs::File::open(dir.join("lgbm_main.bin")).expect("打开");
        let mut head = vec![0u8; LFS_POINTER_PREFIX.len()];
        f.read_exact(&mut head).expect("读头");
        assert_eq!(head, LFS_POINTER_PREFIX);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 指针恒小于产物 → 必须报「指针」而不是「尺寸漂移」。
    #[test]
    fn pointer_wins_over_size_mismatch() {
        let bundle = temp_dir("pointer_first");
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 39684725\n";
        write_bundle(&bundle, &[("lgbm_main.bin", pointer)]);

        let scan = scan_manifest(&bundle);
        assert_eq!(scan.pointers, vec!["lgbm_main.bin".to_string()]);
        assert!(
            scan.problems.iter().any(|p| p.contains("Git LFS pointer")),
            "{:?}",
            scan.problems
        );
        assert!(
            !scan.problems.iter().any(|p| p.contains("!= manifest")),
            "指针不得被报成尺寸漂移: {:?}",
            scan.problems
        );

        let item = inspect(Some(&bundle));
        assert_eq!(item.id, "router_weights");
        assert_eq!(item.status, Status::Degraded, "detail: {}", item.detail);
        assert!(item.detail.contains("Git LFS 指针"), "detail: {}", item.detail);
        assert!(!item.fix_hint.is_empty(), "降级必须给修复动作");
        assert!(item.fix_hint.contains("git lfs pull"), "hint: {}", item.fix_hint);
        let _ = std::fs::remove_dir_all(&bundle);
    }

    /// 完整 bundle → OK，且如实报出校验过的文件数。
    #[test]
    fn complete_bundle_is_ok() {
        let bundle = temp_dir("complete");
        write_bundle(
            &bundle,
            &[
                ("lgbm_main.bin", b"\x00real-weights\x01"),
                ("features/tfidf.pkl", b"\x00real-pickle\x01"),
            ],
        );

        let item = inspect(Some(&bundle));
        assert_eq!(item.status, Status::Ok, "detail: {}", item.detail);
        assert!(item.detail.contains('2'), "detail: {}", item.detail);
        assert!(item.fix_hint.is_empty(), "OK 项不该有修复动作");
        let _ = std::fs::remove_dir_all(&bundle);
    }

    /// 尺寸漂移（非指针）→ Degraded + 具体问题，不谎报 OK。
    #[test]
    fn size_drift_is_degraded() {
        let bundle = temp_dir("drift");
        write_bundle(&bundle, &[("lgbm_main.bin", b"\x00real\x01")]);
        // 篡改清单里的尺寸，制造漂移
        let manifest = serde_json::json!({
            "files": [{ "path": "lgbm_main.bin", "size_bytes": 999_999 }]
        });
        std::fs::write(
            bundle.join("artifact_manifest.json"),
            serde_json::to_string(&manifest).expect("序列化"),
        )
        .expect("写清单");

        let item = inspect(Some(&bundle));
        assert_eq!(item.status, Status::Degraded);
        assert!(item.detail.contains("与清单不符"), "detail: {}", item.detail);
        let _ = std::fs::remove_dir_all(&bundle);
    }

    /// 目录不存在 / 未定位到 pyengine → Degraded（ML 路由可选，不判 MISSING）。
    #[test]
    fn absent_bundle_and_absent_pyengine_are_degraded_not_missing() {
        let ghost = std::env::temp_dir().join("mgt_router_weights_definitely_absent");
        let _ = std::fs::remove_dir_all(&ghost);

        let by_dir = inspect(Some(&ghost));
        assert_eq!(by_dir.status, Status::Degraded);
        assert!(by_dir.detail.contains("不存在"), "detail: {}", by_dir.detail);

        let by_none = inspect(None);
        assert_eq!(by_none.status, Status::Degraded);
        assert!(by_none.detail.contains("pyengine"), "detail: {}", by_none.detail);
        assert!(!by_none.fix_hint.is_empty());
    }

    /// 必备文件缺失 → Degraded 且点名缺了什么。
    #[test]
    fn missing_required_layout_file_is_named() {
        let bundle = temp_dir("layout");
        std::fs::create_dir_all(bundle.join("runtime_src")).expect("建 runtime_src");
        // 刻意不写 router.runtime.yaml

        let item = inspect(Some(&bundle));
        assert_eq!(item.status, Status::Degraded);
        assert!(
            item.detail.contains("router.runtime.yaml"),
            "detail: {}",
            item.detail
        );
        let _ = std::fs::remove_dir_all(&bundle);
    }

    /// 清单里的绝对路径 / `..` 必须被拒（清单文件本身不可信）。
    #[test]
    fn manifest_path_escape_is_rejected() {
        let bundle = temp_dir("escape");
        std::fs::create_dir_all(bundle.join("runtime_src")).expect("建 runtime_src");
        std::fs::write(bundle.join("router.runtime.yaml"), "v4: {}\n").expect("写 yaml");
        // 绝对路径要按平台给：`C:/...` 在 Unix 上不是绝对路径，会被当成普通相对名。
        #[cfg(windows)]
        let abs = "C:/Windows/System32/config";
        #[cfg(not(windows))]
        let abs = "/etc/passwd";
        let manifest = serde_json::json!({
            "files": [
                { "path": "../../etc/passwd", "size_bytes": 1 },
                { "path": abs, "size_bytes": 1 },
            ]
        });
        std::fs::write(
            bundle.join("artifact_manifest.json"),
            serde_json::to_string(&manifest).expect("序列化"),
        )
        .expect("写清单");

        let scan = scan_manifest(&bundle);
        assert_eq!(scan.problems.len(), 2, "{:?}", scan.problems);
        assert!(scan.problems.iter().all(|p| p.contains("invalid manifest path")));
        assert!(scan.checked == 0);
        let _ = std::fs::remove_dir_all(&bundle);
    }

    /// 与本机真实检出对账：体检结论必须等于直接扫描的结论（不漂移）。
    ///
    /// 两种状态都成立：未拉 LFS → Degraded「Git LFS 指针」；已拉 → Ok。
    #[test]
    fn verdict_agrees_with_a_direct_scan_of_the_repo_bundle() {
        let Some(src) = crate::pyengine::sidecar::locate_pyengine_src() else {
            // 未定位到 pyengine 时不静默通过：走 inspect(None) 的契约。
            assert_eq!(inspect(None).status, Status::Degraded);
            return;
        };
        let bundle = bundle_path(&src);
        let scan = scan_manifest(&bundle);
        let item = inspect(Some(&bundle));

        if !scan.pointers.is_empty() {
            assert_eq!(item.status, Status::Degraded, "detail: {}", item.detail);
            assert!(
                item.detail.contains("Git LFS 指针"),
                "detail: {}",
                item.detail
            );
            assert!(item.detail.contains(&scan.pointers[0]), "detail: {}", item.detail);
        } else if scan.problems.is_empty() && bundle.is_dir() {
            assert_eq!(item.status, Status::Ok, "detail: {}", item.detail);
        }
    }
}
