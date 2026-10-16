fn main() {
    // 复制 Web 前端产物（仓库根 webui/dist，Vue 控制台，用户指定的 UI）到 src-tauri/webui/dist，
    // 使 bundle.resources 的 `webui/dist/**` glob 能匹配（tauri-build 的 glob 不支持 `../` 相对段）。
    // 注意 build.rs 的 cwd 是 src-tauri/，`../..` 才是仓库根 marguerite/。
    let src = std::path::Path::new("../../webui/dist");
    let dst = std::path::Path::new("webui/dist");
    if src.join("index.html").is_file() {
        let _ = std::fs::remove_dir_all(dst);
        copy_dir_all(src, dst);
        println!("cargo:rerun-if-changed=../../webui/dist");
    }

    // 复制技能树（marguerite/skills）到 src-tauri/skills，使 bundle.resources 的
    // `skills` 能匹配（tauri-build 的 glob 不支持 `../` 相对段）。
    // 策略：仅当「目标副本缺失」时才从源头补齐（首次构建/清理后）；目标已存在则
    // 保留既有嵌入资源，避免源目录为空或内容不全时 remove_dir_all 清空 50M+ 真实
    // Skills，造成安装包丢失资源。源头更新时，应先删除目标再触发同步。
    let skills_src = std::path::Path::new("../../skills");
    let skills_dst = std::path::Path::new("skills");
    if skills_src.is_dir() && is_nonempty_dir(skills_src) && !skills_dst.exists() {
        let _ = std::fs::remove_dir_all(skills_dst);
        copy_dir_all(skills_src, skills_dst);
        println!("cargo:rerun-if-changed=../../skills");
    }

    // 复制插件清单（仓库根 plugins/）到 src-tauri/resources/plugins，使
    // bundle.resources 的 `resources/plugins` 能把它带进安装包，落到
    // `exe_dir/resources/plugins/manifests` —— 正是
    // core/src/gateway/market.rs 的 resolve_plugins_dir() 第 4 条候选
    // （"安装资源布局：exe_dir/resources/plugins/manifests"）所期望的位置。
    //
    // 策略：**无条件重拷**（remove_dir_all + copy_dir_all），与上面 webui/dist 一致。
    // 刻意不同于 skills 的「仅当目标缺失才拷」：skills 有 50M+，源目录空/残缺时
    // 重拷会把已嵌入的真资源清空；而 plugins/ 仅 81KB（16 份 .plugin.json + patches），
    // 且清单「必须最新」是硬需求——若沿用「缺失才拷」，旧的/被删的清单会永久残留，
    // 导致安装版插件市场列出已下架的插件。
    let plugins_src = std::path::Path::new("../../plugins");
    let plugins_dst = std::path::Path::new("resources/plugins");
    if plugins_src.is_dir() {
        let _ = std::fs::remove_dir_all(plugins_dst);
        let _ = std::fs::create_dir_all(plugins_dst);
        copy_dir_all(plugins_src, plugins_dst);
        println!("cargo:rerun-if-changed=../../plugins");
    } else {
        println!(
            "cargo:warning=插件清单源目录缺失：{} —— 安装版插件清单将为空（market.list 0 条插件）",
            plugins_src.display()
        );
}

    // 复制 Python 引擎源码（仓库根 pyengine/）到 src-tauri/resources/pyengine，使
    // bundle.resources 的 `resources/pyengine` 能把它带进安装包，落到
    // `exe_dir/resources/pyengine/src` —— 正是
    // core/src/pyengine/sidecar.rs 的 pyengine_src_candidates() 第 ① 条候选
    // （"安装资源布局：exe_dir/resources/pyengine/src"）所期望的位置，也是
    // PYTHONPATH 指向的目录（sidecar.rs spawn 时 `p.insert(0, cfg.pyengine_src)`）。
    //
    // 【为什么必须有这一段】此前 build.rs 只同步 webui/dist、skills、plugins 三处，
    // 唯独漏了 pyengine ⇒ 改 `pyengine/src/**` 后若不手工 xcopy，打包就静默沿用
    // 8 月 11 日的手工副本。实测 0.2.11 安装版因此仍在刷 WinError 10054 噪声：
    // 真源 boot.py（2026-10-10）含 `_install_asyncio_connection_noise_filter`，
    // 而打包副本（2026-08-11）grep 命中 0 次 ⇒ 本轮 pyengine 修复在用户机器上
    // 全部无效。全量 diff 1009 个 .py 仅 boot.py 一个不同，说明过去全靠手工补，
    // 这正是不可靠的根源。
    //
    // 策略：**无条件重拷**（remove_dir_all + copy_pyengine_dir_all），与上面
    // plugins 块一致，刻意不同于 skills 的「仅当目标缺失才拷」。交叉引用 plugins
    // 块（L32-36）的取舍论证：plugins 那套「缺失才拷」成立的前提是「源可能被清空」
    // 而「残留只是少几个清单」；pyengine 上这个前提**两头都不成立**——
    //   ① 体量：清理掉构建产物后仅 23MB / 1448 文件，比 skills 的 50M+ 小一个量级，
    //      不存在「源空/残缺时重拷会清空真资源」的灾难场景；
    //   ② 危害方向相反：skills 残留的代价是「旧内容多一份」，而 pyengine 残留的
    //      代价是**修复进不了包**——旧模块/已删除模块永久留在安装版里，让源码改动
    //      静默失效且难以察觉。本次 boot.py 停在 8 月 11 日正是该失效模式的实例。
    // 因此这里必须让「已删除的旧模块」也能随之消失，即每次构建都重拷。
    //
    // 前置守卫用**标记文件**而非 skills 的 is_nonempty_dir：后者只证明「目录非空」，
    // 挡不住「非空但已损坏/错位」的源；`src/opensquilla/cli/main.py` 正是
    // sidecar.rs 的 is_pyengine_src()（L145-147）判定「这是真 pyengine 源码根」的
    // 同一判据 —— 用它做守卫可保证「构建期认得的源」与「运行期认得的源」一致。
    let pyengine_src = std::path::Path::new("../../pyengine");
    let pyengine_dst = std::path::Path::new("resources/pyengine");
    if is_pyengine_source_root(pyengine_src) {
        let _ = std::fs::remove_dir_all(pyengine_dst);
        let _ = std::fs::create_dir_all(pyengine_dst);
        copy_pyengine_dir_all(pyengine_src, pyengine_dst, std::path::Path::new(""));
        println!("cargo:rerun-if-changed=../../pyengine");
    } else {
        println!(
            "cargo:warning=pyengine 源目录缺失或不可用（{}，需含 src/opensquilla/cli/main.py）\
             —— 保留既有 resources/pyengine 副本；若它也是旧的，安装版将携带陈旧引擎源码",
            pyengine_src.display()
        );
    }

    // 复制 Web 控制台到**内嵌网关**的 static/dist（`webui/dist` →
    // `resources/pyengine/src/opensquilla/gateway/static/dist`）。
    //
    // 【为什么必须放在 pyengine 重拷之后，且不能只靠打包脚本】
    // 上面 pyengine 块是**无条件重拷**（remove_dir_all + copy_pyengine_dir_all），
    // 而仓库源 `pyengine/src/opensquilla/gateway/static/` 只有 `img/`、**没有 `dist/`**。
    // 于是 build.rs 每重跑一次，就删一次这个目录；而 L10 的
    // `cargo:rerun-if-changed=../../webui/dist` 决定了「只要前端重建过，build.rs 必重跑」。
    // 打包脚本 `scripts/package_app.cmd`（L73-79）恰恰是在 `cargo tauri build` **之前**
    // 才把控制台拷进这里 —— 顺序上注定被 build.rs 覆盖。实测后果：热缓存增量构建
    // （build.rs 未重跑）时 Control UI 正常，而冷构建 / 改过前端后打出的包会退回
    // 「资源不可用」告警。在这里补一步，让「控制台就位」不再依赖人工顺序，
    // 也不再依赖 build.rs 是否恰好重跑。
    let control_ui_src = std::path::Path::new("../../webui/dist");
    let control_ui_dst = std::path::Path::new("resources/pyengine/src/opensquilla/gateway/static/dist");
    if control_ui_src.join("index.html").is_file() {
        let _ = std::fs::remove_dir_all(control_ui_dst);
        if std::fs::create_dir_all(control_ui_dst).is_ok() {
            copy_dir_all(control_ui_src, control_ui_dst);
        }
    } else {
        println!(
            "cargo:warning=webui/dist 缺失（{}，需含 index.html）—— 内嵌网关的 Control UI \
             将不可用（桌面端显示「资源不可用」）。请先在 webui/ 执行 npm run build",
            control_ui_src.display()
        );
    }

    // 端口单一事实源守卫（S1）：`tauri.conf.json` 的 main 窗口 `url` 端口必须等于
    // `core/src/gateway/mod.rs` 的 `DEFAULT_PORT`（编译期常量，Rust/前端均无法引用它）。
    // 不一致即构建期 panic，消除「改了端口忘了改 tauri.conf.json」的静默漂移。
    check_gateway_port_consistency();

    tauri_build::build()
}

/// 从 `core/src/gateway/mod.rs` 取 `DEFAULT_PORT` 数值，与 `tauri.conf.json` 的
/// main 窗口 `"url"` 端口比对。注意必须只认 `"url":` 字段 —— 配置里还有
/// `devUrl: http://localhost:1420`（端口不同），不能误取。
fn check_gateway_port_consistency() {
    let mod_rs = std::path::Path::new("../../core/src/gateway/mod.rs");
    let conf = std::path::Path::new("tauri.conf.json");
    let (mod_src, conf_src) = match (std::fs::read_to_string(mod_rs), std::fs::read_to_string(conf))
    {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            println!(
                "cargo:warning=端口一致性守卫跳过：源文件不可读（{} / {}）",
                mod_rs.display(),
                conf.display()
            );
            return;
        }
    };
    let expected = match extract_default_port(&mod_src) {
        Some(p) => p,
        None => {
            println!(
                "cargo:warning=端口一致性守卫跳过：未能从 {} 解析 DEFAULT_PORT",
                mod_rs.display()
            );
            return;
        }
    };
    match extract_main_window_url_port(&conf_src) {
        Some(p) if p == expected => {}
        Some(p) => panic!(
            "端口单一事实源不一致：tauri.conf.json 的 main 窗口 url 用 {p}，\
             而 core/src/gateway/mod.rs 的 DEFAULT_PORT={expected}。端口是编译期常量不变量，\
             请把 tauri.conf.json 的 url 端口改为 {expected}（详见 core/src/gateway/mod.rs 的注释）。"
        ),
        None => panic!(
            "端口一致性守卫：未能在 tauri.conf.json 的 \"url\" 字段找到 localhost:<port>；\
             无法校验 DEFAULT_PORT={expected}。"
        ),
    }
    println!("cargo:rerun-if-changed=../../core/src/gateway/mod.rs");
    println!("cargo:rerun-if-changed=tauri.conf.json");
}

/// 解析 `pub const DEFAULT_PORT: u16 = 18_790;` 的数值（忽略下划线分隔符）。
fn extract_default_port(src: &str) -> Option<u16> {
    let start = src.find("DEFAULT_PORT")?;
    let after_name = &src[start..];
    let eq = after_name.find('=')?;
    let digits: String = after_name[eq + 1..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .collect();
    digits.replace('_', "").parse().ok()
}

/// 取 `"url"` 字段里 `localhost:<port>` 的端口（跳过 `devUrl`，它端口不同）。
fn extract_main_window_url_port(src: &str) -> Option<u16> {
    let idx = src.find("\"url\"")?;
    let rest = &src[idx..];
    let loc = rest.find("localhost:")? + "localhost:".len();
    let digits: String = rest[loc..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 判断目录是否非空（递归计数文件/子目录，遇到错误保守返回 false 以免误删目标）。
fn is_nonempty_dir(p: &std::path::Path) -> bool {
    let mut count = 0usize;
    if let Ok(entries) = std::fs::read_dir(p) {
        for _ in entries.flatten() {
            count += 1;
            if count > 0 {
                return true;
            }
        }
    }
    count > 0
}

/// 递归复制目录（仅文件与目录，跟随符号链接由 fs 默认处理）
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) {    if let Ok(entries) = std::fs::read_dir(src) {
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

/// 判定一个目录是不是真的 pyengine 源码根。
///
/// 判据与 `core/src/pyengine/sidecar.rs` 的 `is_pyengine_src()` 完全一致
/// （`src/opensquilla/cli/main.py` 存在）——刻意复用运行期同一标准，使
/// 「构建期认为可同步的源」与「运行期认为可启动的源」不会出现分歧。
fn is_pyengine_source_root(root: &std::path::Path) -> bool {
    root.join("src")
        .join("opensquilla")
        .join("cli")
        .join("main.py")
        .is_file()
}

/// 复制 pyengine 源码时应当跳过的**目录名**。
///
/// 为什么需要过滤：`copy_dir_all`（本文件既有函数，被 webui/dist 与 plugins 复用）
/// 是无差别全量复制，直接拿来拷 pyengine 会把以下内容塞进安装包：
///   - `__pycache__`：真源实测 112 个目录 / 2687 个 `.pyc`（src 下），
///     整个 pyengine 共 5631 个 `.pyc`、约 48MB 纯字节码；
///   - `tests`：顶层 `pyengine/tests` 实测 **120MB** / 1501 个文件（还含 2842 个
///     `.pyc`），是开发期测试资产，与 pyproject.toml 的 `[tool.pytest.ini_options]
///     testpaths = ["tests"]` 对应，安装包不需要；
///   - `.pytest_cache` / `.ruff_cache`：纯开发工具缓存（1.4MB / 27KB）。
///
/// ⚠️ **`tests` 只在顶层跳过，绝不能按目录名全局过滤**：
/// `pyengine/src/opensquilla/plugins/tokenjuice/rules/tests/*.json` 看着像测试
/// 目录，其实是 **tokenjuice 插件的运行时规则数据** ——
/// `plugins/tokenjuice/rules.py` 的 `_iter_json_files()` 递归遍历 `rules/` 下
/// **所有**子目录（只跳过 `fixtures`，见该函数 L52-53）来 `load_rules()`。
/// 若按名字全局跳过 `tests`，安装版 tokenjuice 会静默丢掉全部规则。
/// 故此处按「相对顶层」判定，见 `copy_pyengine_dir_all` 的 `rel` 参数。
fn is_pyengine_skipped_dir(rel: &std::path::Path) -> bool {
    // 任意层级的 __pycache__ 都跳过（字节码缓存，Python 运行时会按需重建）
    if rel.ends_with("__pycache__") {
        return true;
    }
    // 仅顶层跳过开发期资产：`rel.parent()` 为 None ⇔ 位于 pyengine 根
    let is_top_level = rel.parent().is_none_or(|p| p.as_os_str().is_empty());
    if is_top_level {
        return matches!(
            rel.to_str(),
            Some("tests") | Some(".pytest_cache") | Some(".ruff_cache")
        );
    }
    false
}

/// 复制 pyengine 源码时应当跳过的**文件名**。
///
/// `*.pyc` 是 CPython 字节码。必须排除的两个理由：
///   1. 安装包体积：见 `is_pyengine_skipped_dir` 的 48MB 实测；
///   2. **陈旧字节码风险**（更严重）：`.pyc` 内嵌源文件 mtime/size 指纹。当
///      `.pyc` 与 `.py` 一起被复制、而指纹与实际源文件不一致时，CPython 可能
///      复用与当前源码不匹配的字节码 —— 即「改了源码但运行时执行的还是旧代码」。
///      这次的 `boot.py` 事故已经证明「打包副本与真源脱节」会静默失效；把
///      `.pyc` 一并打包等于给这类失效再加一层更难排查的来源。宁可不打包，
///      交给 Python 首次运行时自行生成。
fn is_pyengine_skipped_file(rel: &std::path::Path) -> bool {
    rel.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pyc"))
}

/// 递归复制 pyengine 源码，**带排除规则**（`__pycache__` / `*.pyc` / 顶层
/// `tests` 与工具缓存）。`rel` 是当前条目相对 `pyengine/` 根的路径，用于区分
/// 「顶层 tests」（跳过）与「深层 tests」（如 tokenjuice 规则数据，必须保留）。
///
/// 与 `copy_dir_all` 的差别仅在于跳过判定；其余行为（忽略无法读取的条目、
/// 忽略单文件复制失败）保持一致，避免因权限/占用导致整个构建失败。
fn copy_pyengine_dir_all(src: &std::path::Path, dst: &std::path::Path, rel: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let child_rel = rel.join(entry.file_name());
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            if is_pyengine_skipped_dir(&child_rel) {
                continue;
            }
            let _ = std::fs::create_dir_all(&target);
            copy_pyengine_dir_all(&path, &target, &child_rel);
        } else if !is_pyengine_skipped_file(&child_rel) {
            let _ = std::fs::copy(&path, &target);
        }
    }
}
