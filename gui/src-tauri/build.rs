fn main() {
    // 复制 Web 前端产物（仓库根 webui/dist，Vue 控制台，用户指定的 UI）到 src-tauri/webui/dist，
    // 使 bundle.resources 的 `webui/dist/**` glob 能匹配（tauri-build 的 glob 不支持 `../` 相对段）。
    // 注意 build.rs 的 cwd 是 src-tauri/，`../..` 才是仓库根 marguerite/。
    let src = std::path::Path::new("../../webui/dist");
    let dst = std::path::Path::new("webui/dist");
    if src.join("index.html").is_file() {
        // 清理失败必须可见（见 reset_target_dir 的注释）。
        let mut failures: Vec<String> = Vec::new();
        reset_target_dir(dst, &mut failures);
        if failures.is_empty() {
            copy_dir_all(src, dst, &mut failures);
        }
        ensure_synced("webui/dist", &failures);
        println!("cargo:rerun-if-changed=../../webui/dist");
    }

    // 复制技能树（marguerite/skills）到 src-tauri/skills，使 bundle.resources 的
    // `skills` 能匹配（tauri-build 的 glob 不支持 `../` 相对段）。
    // R72 修正：原策略「仅当目标缺失才复制」导致源目录后续新增的技能
    // （实测 blender3d.yaml + blender-skills/ 94 技能库）**永远进不了安装包**
    // —— 目标是首建快照，构建时被无条件冻结。现改为**每次构建都重同步**：
    // - 源非空 → 先清目标再全量复制（源是唯一真源，含新增与删除）；
    // - 源缺失/为空 → 保留既有目标（保护性回退，注释里的 50M+ 场景不再发生，
    //   因为仓库 skills/ 已是稳定真源）。
    let skills_src = std::path::Path::new("../../skills");
    let skills_dst = std::path::Path::new("skills");
    if skills_src.is_dir() && is_nonempty_dir(skills_src) {
        // 重试同步（环境韧性，非逻辑改动）：本机存在对 `skills/codebuddy` 的并发写入者
        // （自动重启的 marguerite-gui / 宿主技能同步），会在 `remove_dir_all` + 全量复制
        // 的瞬间竞争 `gui/src-tauri/skills`，偶发 `os error 3`（路径瞬时不可见）。
        // 该树在隔离环境下可完整复制（已实测 3453/3453），故对整段同步做有限重试，
        // 命中静默窗口即成功；仅当连续 N 次仍失败才走 `ensure_synced` 硬失败。
        // 守卫语义不变：真源确实缺失/残缺时仍会 panic，绝不静默放过缺内容。
        let mut last_failures: Vec<String> = Vec::new();
        let mut last_skipped: Vec<String> = Vec::new();
        let mut synced = false;
        for _attempt in 0..8 {
            // 【2026-10-22 修复：把「先删后拷」改成「覆盖 + 修剪」】
            //
            // 旧实现每次构建都 `remove_dir_all(skills)` 再全量复制。当**另一个 cargo**
            // （实测：IDE 的 rust-analyzer 周期性 `cargo check`）同时跑 build.rs 时，
            // 两边会互相删对方的树 —— 打包侧的 `generate_context!` 恰好在"刚被删、
            // 还没拷回来"的窗口里枚举资源，于是报出与本文件内容无关的
            // `resource path skills\...\SKILL.md doesn't exist`（本次实测命中）。
            //
            // 覆盖式同步没有"整棵树消失"的窗口：任何时刻每个源文件在目标侧都存在，
            // 最坏只是被写了一半。删除只发生在**本轮没有瞬态跳过**时的修剪阶段，
            // 且只删「源侧确实不存在」的条目 —— 并发写入者造成的瞬时缺失不会被误删。
            let mut failures: Vec<String> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            if let Err(e) = std::fs::create_dir_all(skills_dst) {
                failures.push(format!("建目录 {}: {e}", skills_dst.display()));
            } else {
                copy_skills_dir_all(skills_src, skills_dst, &mut failures, &mut skipped);
                // 只有本轮观测完整时才敢修剪：源侧瞬时缺条目的情况下删目标 =
                // 把竞态升级成真正的缺内容。
                if failures.is_empty() && skipped.is_empty() {
                    prune_missing_entries(skills_src, skills_dst, &mut failures);
                }
            }
            // 【2026-10-22 修复】`skipped` 不能算成功。
            //
            // 旧实现只看 `failures.is_empty()`，把"瞬态跳过"当同步完成，注释理由是
            // 「下次构建即一致」。但**打包就在同一次构建里紧接着跑**，没有"下次"——
            // 实测后果（NSIS 打包失败，错误被 `| tail` 吞掉过一次）：
            //     Error failed to bundle project resource path
            //     `skills\codebuddy\tencentos-expert\scripts\output.sh` doesn't exist
            // 而该文件在源与目标两侧**其实都存在**，失效的是"打包那一瞬的目录状态"。
            //
            // 这是典型的"上游容忍、下游硬失败"：build.rs 说跳过没问题，
            // tauri bundling 对 bundle.resources 是严格的。二者判据必须一致。
            //
            // 现在 skipped 也触发重试；8 次仍非空则走 ensure_synced 硬失败，
            // 用明确的"哪个文件没同步上"取代下游那句莫名其妙的"资源不存在"。
            if failures.is_empty() && skipped.is_empty() {
                synced = true;
                break;
            }
            if !skipped.is_empty() {
                eprintln!(
                    "[build] 技能树同步遇到 {} 个瞬态条目，重试（打包会对 bundle.resources 做严格校验，不能容忍缺口）",
                    skipped.len()
                );
            }
            last_failures = failures;
            last_skipped = skipped;
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        if !synced {
            // 把"瞬态跳过"也升级为可见失败，避免交付一个缺内容的安装包。
            let mut all = last_failures;
            all.extend(last_skipped.into_iter().map(|p| format!("未能同步（多次重试后仍缺）: {p}")));
            ensure_synced("技能树 skills/", &all);
        }
    }
    println!("cargo:rerun-if-changed=../../skills");

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
        // 这两行的失败后果不是"没清干净"而是"旧清单残留"：
        // 本目录的语义是「清单必须最新」——旧的/已下架的 .plugin.json 若残留，
        // 安装版插件市场会列出**已下架**的插件。故清理失败必须硬失败。
        let mut failures: Vec<String> = Vec::new();
        reset_target_dir(plugins_dst, &mut failures);
        if failures.is_empty() {
            copy_dir_all(plugins_src, plugins_dst, &mut failures);
        }
        ensure_synced("插件清单 plugins/", &failures);
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
        // 重试同步（环境韧性，与 skills 块同思路）：本机存在对 `pyengine/` 的并发写入者
        // （自动重启的 marguerite-gui / 宿主热同步），会在 `remove_dir_all` + 全量复制
        // 的瞬间竞争 `resources/pyengine`，偶发源目录条目瞬时不可见 ⇒ 该文件被静默漏拷，
        // 随后 tauri 打包报出误导性的 `resource path ... doesn't exist`
        // （2026-10-20 实测 bubblewrap.py 命中此坑）。故整段同步做有限重试，且每次复制后
        // **校验目标树与源树一致**（排除 __pycache__ / *.pyc / 顶层 tests 等本就跳过的
        // 条目）；命中静默窗口即成功，仅当连续 N 次仍缺内容才 `ensure_synced` 硬失败。
        // 守卫语义不变：真源确实缺失/残缺时仍会 panic，绝不静默放过缺内容。
        let mut last_failures: Vec<String> = Vec::new();
        let mut synced = false;
        for _attempt in 0..8 {
            // 清理/重建失败也进 failures —— 否则旧文件残留会被 verify_pyengine_sync
            // 漏过（它只查"源有目标无"，查不出"目标有源无"）。
            let mut failures: Vec<String> = Vec::new();
            reset_target_dir(pyengine_dst, &mut failures);
            // 清理失败时不做复制、也不做校验：目标此刻是"旧内容 + 半删"的状态，
            // 对它的校验结论只会把根因（清理失败）掩埋成"目标缺文件"。
            if failures.is_empty() {
                copy_pyengine_dir_all(
                    pyengine_src,
                    pyengine_dst,
                    std::path::Path::new(""),
                    &mut failures,
                );
                verify_pyengine_sync(
                    pyengine_src,
                    pyengine_dst,
                    std::path::Path::new(""),
                    &mut failures,
                );
            }
            if failures.is_empty() {
                synced = true;
                break;
            }
            last_failures = failures;
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        if !synced {
            ensure_synced("pyengine 源码", &last_failures);
        }
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
        // 原实现是 `let _ = remove_dir_all(...)` + `if create_dir_all(...).is_ok()`：
        // 两步失败都**没有任何输出**，连 cargo:warning 都没有。改为并入 failures
        // 交给 ensure_synced 硬失败 —— 这个目录缺失时桌面端直接显示「资源不可用」。
        let mut failures: Vec<String> = Vec::new();
        reset_target_dir(control_ui_dst, &mut failures);
        if failures.is_empty() {
            copy_dir_all(control_ui_src, control_ui_dst, &mut failures);
        }
        ensure_synced("内嵌网关控制台 static/dist", &failures);
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

    // 【2026-10-22 修复：自定义命令必须显式进入 App ACL 清单】
    //
    // 现象（安装版实机取证）：Runtime 面板渲染
    //     Command sidecar_status not allowed by ACL
    // 而同一份代码在 `npm run tauri dev`（debug）下正常。
    //
    // 根因在 tauri 2.11.5 的 `webview/mod.rs`（约 L1819-1852）：
    //     if (plugin_command.is_some() || has_app_acl_manifest || !is_local)
    //        && invoke.acl.is_none()
    //     {
    //         #[cfg(not(debug_assertions))]
    //         invoke.resolver.reject(format!("Command {} not allowed by ACL", cmd));
    //     }
    // 三个条件里，本项目同时满足两条 —— `capabilities/default.json` 让
    // `has_app_acl_manifest` 成立，主窗口加载 `http://localhost:18790` 又让
    // `is_local` 为假。于是**所有 invoke 都必须带 ACL**，不只是插件命令。
    // release 才有 `#[cfg(not(debug_assertions))]`，所以 dev 全绿、装包全红。
    //
    // `AppManifest::commands()` 会为每个命令在 `permissions/autogenerated/`
    // 生成 `allow-{slug}` / `deny-{slug}`（slug = 命令名把 `_` 换成 `-`），
    // 随后与 `capabilities/default.json` 里引用的标识符对账。清单必须与
    // 前端真实 invoke 的命令**逐条对齐**，否则该功能在安装版里静默不可用：
    //   · sidecar_*        —— utils/desktopSidecar.ts、platform/desktopBridge.ts
    //   · notify_turn_ended —— composables/useTurnNotification.ts
    //   · pet_overlay_*    —— views/PetOverlayView.vue
    //   · autostart_*      —— components/settings/SettingsAdvancedPanel.vue
    // 新增前端 invoke 时必须同步这里 + capabilities/default.json，两处缺一即失败。
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "sidecar_status",
            "sidecar_log_dir",
            "sidecar_start",
            "sidecar_stop",
            "notify_turn_ended",
            "pet_overlay_move",
            "pet_overlay_hide",
            "pet_overlay_show",
            "pet_overlay_toggle",
            "autostart_status",
            "autostart_set",
        ])),
    )
    .expect("tauri-build 失败（含 ACL 权限生成与 capabilities 校验）")
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
        None => {
            // 初始 URL 是 splash.html（本地启动动画），网关地址由 Rust 侧 control_url() 硬编码。
            // 此时跳过端口校验——守卫的目的是防止"窗口 URL 与实际网关端口不一致"，
            // 而 splash.html 不指向任何端口，不存在不一致风险。
            //
            // 用 stderr 而非 `cargo:warning=`：这是**正常路径**的说明，不是告警。
            // 打成 warning 会让每次构建都出现一条假红，稀释真正的问题（门禁要"零警告"，
            // 零警告的前提是警告只表示真问题）。需要看这条说明时用 `cargo build -vv`。
            eprintln!("[build] 端口一致性守卫：初始 URL 为 splash.html（本地启动动画），跳过 localhost:port 校验");
        }
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

/// 递归复制目录（仅文件与目录，跟随符号链接由 fs 默认处理）。
///
/// **不静默吞错**：未能写入的条目全部收进 `failures`，由调用方 [`ensure_synced`]
/// 决定如何处置。理由见 [`copy_file_settled`] —— 静默的局部失败会在下游变成
/// 误导性的"资源不存在"。实测已在 `webui/dist` 上发生：一个 CSS 瞬时没拷进去，
/// `tauri_build` 只报 `resource path ... doesn't exist`，而该文件两侧其实都在。
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path, failures: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(src) else {
        failures.push(format!("读取源目录失败: {}", src.display()));
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&target) {
                failures.push(format!("建目录 {}: {e}", target.display()));
                continue;
            }
            copy_dir_all(&path, &target, failures);
        } else if let Err(e) = copy_file_settled(&path, &target) {
            failures.push(format!("{} -> {}: {e}", path.display(), target.display()));
        }
    }
}

/// 复制技能树：与 [`copy_dir_all`] 相同的无差别全量复制，**唯一差别**是跳过
/// 版本控制内部目录（[`is_vcs_metadata_dir`]）。
///
/// 【为什么 skills 必须单独过滤，而 webui/dist、plugins 不需要】
/// 实测 `skills/blender-skills/` 是一个**内嵌的第三方 git 仓库**（自带 `.git/`），
/// 它踩了两个独立的坑，且都已在当前构建里真实发生：
///   1. **构建直接失败**：`tauri.conf.json` 的 `bundle.resources` 含 `"skills"`，
///      而目标 `src-tauri/skills` 正是它的落点；tauri-build 会遍历其中**每个**
///      文件并打印 `rerun-if-changed`。`.git/objects/pack/*.{idx,pack,rev}` 在
///      Windows 上是**只读**文件，遍历到即 `os error 5（拒绝访问）`，构建脚本
///      exit 1 → `cargo build --workspace` 整条链 exit 101（实测基线红）。
///   2. **安装包污染**：把 `.git` 打进安装包既无用途（运行期没有 git），
///      又白白带上开发历史与整个对象库。
///
/// 过滤只发生在**复制**这一步 —— 源目录原样保留，不动用户的仓库本体。
fn copy_skills_dir_all(
    src: &std::path::Path,
    dst: &std::path::Path,
    failures: &mut Vec<String>,
    skipped: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(src) else {
        failures.push(format!("读取源目录失败: {}", src.display()));
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            if is_vcs_metadata_dir(&entry.file_name()) {
                continue;
            }
            if let Err(e) = std::fs::create_dir_all(&target) {
                // 并发同步把源目录整棵移走/删掉时会瞬时 NotFound —— 属瞬态竞态，跳过不计入失败。
                if e.kind() == std::io::ErrorKind::NotFound {
                    skipped.push(format!("目录瞬时消失（竞态跳过）: {}", path.display()));
                    continue;
                }
                failures.push(format!("建目录 {}: {e}", target.display()));
                continue;
            }
            copy_skills_dir_all(&path, &target, failures, skipped);
        } else if let Err(e) = copy_file_settled(&path, &target) {
            // 关键修复：源文件在 copy 之前被「宿主技能同步 / 自动重启的 marguerite-gui」
            // 重命名或移走 → os error 2/3（NotFound）。这是构建期改写 `skills/codebuddy`
            // 的**瞬时**竞态，并非内容永久丢失（重命名完成后文件仍在树内，下次构建即一致）。
            // 故按「跳过 + 警告」处理，不计入 `failures`，避免整条构建因瞬态竞态 panic。
            // 真正的「内容缺漏」（权限拒绝等）仍由 `ensure_synced` 经 `failures` 硬失败拦截。
            if e.kind() == std::io::ErrorKind::NotFound {
                if skipped.len() < 60 {
                    eprintln!(
                        "[build] 跳过瞬时消失的技能条目（并发同步竞态，非内容丢失）：{}",
                        path.display()
                    );
                }
                skipped.push(path.display().to_string());
                continue;
            }
            failures.push(format!("{} -> {}: {e}", path.display(), target.display()));
        }
    }
}

/// 修剪语义：删除目标侧「源侧不存在」的条目，让安装包不会累积已下架/已删除的技能。
///
/// 与 [`copy_skills_dir_all`] 配套使用：覆盖式复制保证任何时刻目标都是源的超集
/// （并发构建也只会重复写同样的内容），修剪再把它收敛回与源一致。
///
/// 安全边界：
///  · 只处理**目录与普通文件**，符号链接按文件处理（源有则保留、源无则删）；
///  · `.git/.hg/.svn` 等元数据目录**一律删除**（它们本就不该进入安装包）；
///  · 删除失败（含并发下的 NotFound）不 panic —— 修剪是收敛动作，不是内容来源，
///    真正的完整性由下一轮 copy 与 `ensure_synced` 保证。
fn prune_missing_entries(
    src: &std::path::Path,
    dst: &std::path::Path,
    failures: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dst) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let src_path = src.join(&name);
        let dst_path = entry.path();
        let is_dir = dst_path.is_dir();

        if is_dir && is_vcs_metadata_dir(&name) {
            let _ = std::fs::remove_dir_all(&dst_path);
            continue;
        }
        if is_dir {
            if src_path.is_dir() {
                prune_missing_entries(&src_path, &dst_path, failures);
            } else {
                let _ = std::fs::remove_dir_all(&dst_path);
            }
            continue;
        }
        if !src_path.is_file() {
            if let Err(e) = std::fs::remove_file(&dst_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    failures.push(format!("修剪 {}: {e}", dst_path.display()));
                }
            }
        }
    }
}

/// 复制单个文件，对 Windows 上的**瞬时占用**重试若干次。
///
/// 【为什么不能沿用 `let _ = std::fs::copy`】那条静默路径已实测造成误导性失败：
/// 某个瞬时失败的技能文件没进目标，随后 `tauri_build::build()` 遍历
/// `bundle.resources` 时只报出一句 `resource path ... doesn't exist` ——
/// 真因（复制失败）被 `let _` 吃掉，排查成本极高（2026-10-20 实测：debug 构建
/// 成功、同源同目标的 release 打包却卡在这句上，而那个文件其实两侧都存在，
/// 说明失效的是「某一刻的目录状态」而非路径本身）。
///
/// 本机杀软扫描 / 搜索索引器会短时占用刚写入的文件，故重试是必要的；
/// 重试耗尽则**交回调用方记入失败清单**，绝不静默继续。
fn copy_file_settled(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<u64> {
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..3 {
        match std::fs::copy(src, dst) {
            Ok(n) => return Ok(n),
            Err(e) => {
                last = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("copy failed after retries")))
}

/// 任一条目的同步失败都必须**硬失败并给出可操作信息**，绝不静默继续。
///
/// 两种静默后果都比构建期明确失败更糟：
///   ① 侥幸过关 → 安装包**静默缺内容**（少技能、少控制台资源、少引擎模块），
///      用户侧表现为"功能莫名不可用"，极难回溯；
///   ② 不过关 → `tauri_build::build()` 遍历资源时报出**误导性**的
///      `resource path ... doesn't exist`，真因（复制失败）已被吞掉。
/// 故这里统一 panic，并把"谁占用目录"作为第一嫌疑写进信息里（本机实测：
/// 并发会话正在重建 `webui/dist`，vite 会清空 assets 再写新哈希文件名）。
fn ensure_synced(what: &str, failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    panic!(
        "{what} 同步失败：{} 个条目未能写入（前 5 条）：\n{}\n\
         请先停掉正在写该目录的进程（并发会话的 npm run build / cargo、\
         杀软扫描、搜索索引器、编辑器）后重跑构建。",
        failures.len(),
        failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// 目录名是否为「应跳过的元数据 / 缓存目录」。
///
/// 两类都跳过：
///   1. 版本控制内部目录：`.git`/`.hg`/`.svn`（只认目录名、不认全路径，故出现在
///      `blender-skills/` 还是更深一层都同样跳过）。
///   2. AI 编程助手 / 编辑器生成的缓存目录：`.claude`/`.cursor`/`.vscode`/`.idea`/
///      `.codebuddy-plugin`。这些**不是技能内容**，且会被 Claude Code / Cursor /
///      CodeBuddy 等在本机持续改写（生成 `skills/`、`mcp.json` 等），在构建期与
///      `remove_dir_all` + 全量复制竞争，偶发 `os error 3`；同时它们也不应进入
///      安装包。实测 `blender-skills/.claude`、`blender-skills/.cursor` 正是此类
///      瞬态目录，跳过即消除该路竞态。
fn is_vcs_metadata_dir(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(".git")
            | Some(".hg")
            | Some(".svn")
            | Some(".claude")
            | Some(".cursor")
            | Some(".vscode")
            | Some(".idea")
            | Some(".codebuddy-plugin")
    )
}

/// 递归删除目录，失败时**先清只读位再重试**。
///
/// `std::fs::remove_dir_all` 在 Windows 上遇到只读文件会以 `os error 5` 失败，
/// 并**留下半删的目录**。原实现是 `let _ = std::fs::remove_dir_all(dst)` ——
/// 错误被静默吞掉，于是「目标先清空、再全量复制」这条不变量在**首次失败后
/// 就永久失效**：上一轮构建残留的只读 `.git/objects/pack/*` 正是这样留下来的，
/// 此后每次构建都会把旧副本当作"已清空"并重新踩同一个坑（源目录是稳定真源，
/// 不会自愈）。因此删除必须能真正清干净，而不是"尽力而为"。
fn remove_dir_all_force(p: &std::path::Path) -> std::io::Result<()> {
    if !p.exists() {
        return Ok(());
    }
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(_) => {
            clear_readonly_recursive(p);
            std::fs::remove_dir_all(p)
        }
    }
}

/// 清空目标目录并重建；失败**记入 failures**，绝不静默。
///
/// # 为什么必须显式处理（2026-10-22）
///
/// 下游 `copy_dir_all` 只往目标写「源里有的文件」，**从不删除目标多余的文件**；
/// `ensure_synced` 也只在 `failures` 非空时才 panic。于是「删除目标目录失败」的后果是：
///
/// ```text
/// 旧文件残留 + 新文件覆盖  →  一个混合了上一版产物的目录
///                          →  failures 为空  →  ensure_synced 静默通过
/// ```
///
/// 本项目 2026-10-22 实测撞到过这种混合态：打包目录一度 765 个文件而源码只有 306、
/// 入口哈希在构建中途变化，最终 `cargo tauri build` 报出一句与真因无关的
/// `resource path ... doesn't exist`。
///
/// 同理，`verify_pyengine_sync` 只检查「源有、目标无」，**查不出「目标有、源无」**。
///
/// 所以清理失败必须并入 `failures`：让 `ensure_synced` 报出**真实原因**
/// （「清理目标目录失败：文件被占用」），而不是把排查者引向下游的资源缺失。
///
/// 清理失败时**直接返回、不再复制** —— 因为继续往未清空的目标里写，
/// 恰恰会产出一个"看起来成功"的混合目录，那比直接失败更糟。
fn reset_target_dir(dst: &std::path::Path, failures: &mut Vec<String>) {
    // 重试若干次再判定失败。
    //
    // 为什么不能"一次失败即中止"（2026-10-22 实测）：`skills` 目录在本机存在
    // **并发写入者**（本文件上方 skills 同步块也记着这件事），首次 remove 常拿到
    // `os error 5 拒绝访问`，几百毫秒后同样的调用就成功了。
    // 若一次失败就 panic，会把"瞬时锁"误判成"产物不干净"，让打包彻底卡死；
    // 若像旧实现那样直接吞掉，则会打出混合包。两者都错。
    //
    // 折中：重试 6 次 × 250ms（共 1.5s），仍失败才记入 failures 硬失败 ——
    // 既不误杀瞬时锁，也绝不带着脏目录继续构建。
    let mut last_err = None;
    for attempt in 0..6 {
        match remove_dir_all_force(dst) {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < 5 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    }
    if let Some(e) = last_err {
        failures.push(format!(
            "清理目标目录失败 {}: {e} —— 旧产物会残留，打包将混入上一版文件",
            dst.display()
        ));
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dst) {
        failures.push(format!("重建目标目录失败 {}: {e}", dst.display()));
    }
}

/// 递归清掉只读属性 —— Windows 上删除只读文件前的必要前置。
fn clear_readonly_recursive(p: &std::path::Path) {
    // 本函数只为绕过 Windows 上 `.git/objects/pack/*.pack` 的只读属性
    // （tauri-build 遍历即 os error 5）。Unix 上 set_readonly(false) 本就无此语义，
    // 这里显式允许该 lint——构建脚本产物全部在本地，不存在权限泄露面。
    #[allow(clippy::permissions_set_readonly_false)]
    for entry in std::fs::read_dir(p).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            clear_readonly_recursive(&path);
        } else if let Ok(md) = std::fs::metadata(&path) {
            let mut perm = md.permissions();
            if perm.readonly() {
                perm.set_readonly(false);
                // 这里是**刻意**允许失败，不是漏改（2026-10-22 逐条清理时确认）：
                // 本函数只是 `remove_dir_all_force` 的**前置辅助**，真正的判据是
                // 紧随其后的 `remove_dir_all` 返回值 —— 若清只读失败导致删不掉，
                // 那条返回必然是 Err，会被 `reset_target_dir` 记入 failures 并硬失败。
                // 换句话说：这条失败有下游兜底，不构成静默；而在这里提前 hard fail
                // 反而会让"某个无关文件清不掉"阻断整次构建。
                let _ = std::fs::set_permissions(&path, perm);
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
            Some("tests")
                | Some(".pytest_cache")
                | Some(".ruff_cache")
                // 开发虚拟环境（本会话 pyengine 门禁用 uv 建的 .venv 实测 431MB：
                // numpy/scipy/onnxruntime 等开发依赖，安装包用系统/自带 Python，
                // 绝不随分发）。整个目录任意层级都不应入库。
                | Some(".venv")
                | Some("venv")
                | Some(".mypy_cache")
        );
    }
    // .venv 即使出现在任意层级也跳过（开发虚拟环境，运行时由安装侧自行提供）。
    if rel
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".venv"))
    {
        return true;
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
/// 与 `copy_dir_all` 的差别仅在于跳过判定；**错误处理完全一致** —— 未能写入的
/// 条目同样收进 `failures` 并由 [`ensure_synced`] 硬失败。刻意不再沿用本条注释
/// 旧说的"忽略单文件复制失败"：那条静默路径会把真因藏进下游误导性的"资源不存在"。
fn copy_pyengine_dir_all(
    src: &std::path::Path,
    dst: &std::path::Path,
    rel: &std::path::Path,
    failures: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(src) else {
        failures.push(format!("读取源目录失败: {}", src.display()));
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
            if let Err(e) = std::fs::create_dir_all(&target) {
                failures.push(format!("建目录 {}: {e}", target.display()));
                continue;
            }
            copy_pyengine_dir_all(&path, &target, &child_rel, failures);
        } else if !is_pyengine_skipped_file(&child_rel) {
            if let Err(e) = copy_file_settled(&path, &target) {
                failures.push(format!("{} -> {}: {e}", path.display(), target.display()));
            }
        }
    }
}

/// 复制完成后校验「源树有的非跳过文件，目标树都有」，捕捉 [`copy_pyengine_dir_all`]
/// 因构建期并发竞态而**静默漏拷**的条目：`read_dir` 在源被并发改写的瞬间没列出某文件
/// ⇒ 复制阶段零失败记录 ⇒ 下游 tauri 打包才报出误导性的 `resource path ... doesn't exist`
/// （实测 bubblewrap.py 命中此坑）。本函数把这类「源有、目标无」的缺口补进 `failures`，
/// 由外层重试循环重跑整段同步（竞态是瞬时窗口，重跑即一致）；连续 N 次仍缺才硬失败。
///
/// 仅检查「源 → 目标」方向（目标多出的文件无害）；目录判定与 [`is_pyengine_skipped_dir`]、
/// 文件判定与 [`is_pyengine_skipped_file`] 共用同一排除规则，保证校验口径与复制口径一致。
fn verify_pyengine_sync(
    src: &std::path::Path,
    dst: &std::path::Path,
    rel: &std::path::Path,
    failures: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let child_rel = rel.join(name.clone());
        let s = entry.path();
        let t = dst.join(name);
        if s.is_dir() {
            if is_pyengine_skipped_dir(&child_rel) {
                continue;
            }
            if t.is_dir() {
                verify_pyengine_sync(&s, &t, &child_rel, failures);
            } else {
                failures.push(format!(
                    "目标目录缺失（源有、目标无，疑似构建期竞态）: {}",
                    t.display()
                ));
            }
        } else if !is_pyengine_skipped_file(&child_rel) && !t.is_file() {
            failures.push(format!(
                "目标文件缺失（源有、目标无，疑似构建期竞态）: {}",
                t.display()
            ));
        }
    }
}
