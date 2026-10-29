@echo off
REM ============================================================
REM  Marguerite 全量打包 (Release v0.1.4) - 最新版安装包
REM  打包内容：webui / 全 skills / 全工具 / 全配置 / 网关 / pyengine / btpanel-mcp / mode YAML / scripts
REM ============================================================
setlocal EnableExtensions EnableDelayedExpansion

set "REPO_ROOT=%~dp0.."
pushd "%REPO_ROOT%"
set "REPO_ROOT=%CD%"
popd

set "TAURI_DIR=%REPO_ROOT%\gui\src-tauri"
set "WEBUI_SRC=%REPO_ROOT%\webui\dist"
set "WEBUI_DST=%TAURI_DIR%\webui\dist"
set "TARGET_DIR=%CD%\marguerite_build_target"

echo ============================================================
echo  Marguerite 全量打包（Release v0.1.4）
echo  仓库 : %REPO_ROOT%
echo  Tauri: %TAURI_DIR%
echo  构建 : %TARGET_DIR%
echo ============================================================

REM --------- Step 1. 隔离构建目录 ---------
if exist "%TARGET_DIR%" (
    echo [1/7] 清理构建目录: %TARGET_DIR%
    rmdir /s /q "%TARGET_DIR%" 2>nul
)
mkdir "%TARGET_DIR%" 2>nul

REM --------- Step 2. 清理旧构建残留 ---------
echo [2/7] 清理 webui/dist.old* 旧目录 ...
for /d %%D in ("%REPO_ROOT%\webui\dist.old*") do (
    if exist "%%D" rmdir /s /q "%%D" 2>nul && echo   removed %%~nxD
)

REM --------- Step 3. webui/dist 拷贝（覆盖） ---------
echo [2/7] 嵌入 Web UI（webui/dist -^> gui/src-tauri/webui/dist）
if not exist "%WEBUI_SRC%" (
    echo [ERROR] 未找到 webui/dist！请先在 webui 下执行: npm run build
    exit /b 1
)
REM 【2026-10-23】原「桌宠图片皮肤完整性检查」已移除：
REM   桌宠皮肤改为纯本地矢量 / 参数化 3D 渲染（webui/src/components/pet/petSpec.ts 数据驱动），
REM   `webui/public/pets/*.jpg` 因带第三方水印且未再被引用，已随「无敏感无水印素材」要求下线。
REM   这里只保留产物入口检查 —— 缺 index.html 才是真的打不出可用控制台。
if not exist "%WEBUI_SRC%\index.html" (
    echo [ERROR] webui/dist 缺少 index.html（前端产物不完整）。
    echo         请确认 webui 下 npm run build 成功。
    exit /b 1
)
echo       前端产物入口检查通过（index.html 存在）。
if exist "%WEBUI_DST%" rmdir /s /q "%WEBUI_DST%" 2>nul
xcopy /E /I /Y /Q "%WEBUI_SRC%" "%WEBUI_DST%" >nul
echo       完成。

REM --------- Step 3. Skills（全量同步） ---------
echo [3/7] 同步全量 skills 到 gui/src-tauri/skills
set "SKILL_SRC=%REPO_ROOT%\skills"
set "SKILL_DST=%TAURI_DIR%\skills"
if exist "%SKILL_SRC%" (
    xcopy /E /I /Y /Q "%SKILL_SRC%\*" "%SKILL_DST%\" >nul
)
echo       完成。

REM --------- Step 4. 全量资源（配置 / pyengine / btpanel-mcp / mode / scripts） ---------
echo [4/7] 收集全量资源到 gui/src-tauri/resources/
set "RES=%TAURI_DIR%\resources"
REM 【2026-10-28】原来这里是 `if exist "%RES%" rmdir /s /q "%RES%"` —— 整目录清空。
REM 而 resources\python 是 315 MB 可移植 CPython，**不在 git 里**，只由
REM scripts\fetch_python_runtime.ps1 物化。于是每打包一次就把它删一次：下一版安装包
REM 没有内置解释器，侧车首选它（core/src/pyengine/sidecar.rs:
REM <install>\resources\python\python.exe），删了不报错 —— 只是每台装了这版的机器都
REM 静默退化成纯 Rust 能力子集。实测本轮就是这样丢了内置解释器，靠重跑 fetch 脚本才找回来。
REM 所以改成只清理「本轮确实要重制」的子树；resources\python 与 build.rs 负责的
REM resources\plugins 都不在这里动（plugins 由 build.rs 无条件重拷）。
for %%D in (modes pyengine btpanel-mcp docs scripts) do (
    if exist "%RES%\%%D" rmdir /s /q "%RES%\%%D" 2>nul
)
if exist "%RES%" (
    del /q "%RES%\*.md" 2>nul
    del /q "%RES%\config.yaml" 2>nul
    del /q "%RES%\Cargo.lock" 2>nul
    del /q "%RES%\rust-toolchain.toml" 2>nul
)
mkdir "%RES%" 2>nul

REM 4a-pre. 内置 Python 运行时（缺失则就地物化，而不是打完才发现）
REM     有了这一段，「一条命令产出双引擎安装包」才成立：fetch 脚本是幂等的，
REM     已就绪时它自己打印「已就绪」并 exit 0。
if not exist "%RES%\python\python.exe" (
    echo       未找到内置运行时，执行 scripts\fetch_python_runtime.ps1 ...
    powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\fetch_python_runtime.ps1"
    if errorlevel 1 (
        echo [ERROR] 内置运行时物化失败，打包中止。
        exit /b 1
    )
)
if not exist "%RES%\python\python.exe" (
    echo [ERROR] 仍然没有 %RES%\python\python.exe —— 侧车会静默降级为纯 Rust，不产出安装包。
    exit /b 1
)

REM 4a. 用户可见的根文档。
REM     [WARN] 这里曾经还复制 config.yaml / AGENTS.md / rust-toolchain.toml，实测都进了
REM     0.2.17 安装包：AGENTS.md 134 KB 的智能体指令（内含被引用的已泄露密钥前缀与
REM     内部门禁细节）就这样发到每个用户机器上。config.yaml 是开发用配置模板，
REM     用户实际读写的是 %LOCALAPPDATA%\Marguerite\config.yaml —— 装一份在程序目录只会误导。
if exist "%REPO_ROOT%\CHANGELOG.md" copy /Y "%REPO_ROOT%\CHANGELOG.md" "%RES%\" >nul
if exist "%REPO_ROOT%\README.md"    copy /Y "%REPO_ROOT%\README.md" "%RES%\" >nul

REM 4b. mode/*.yaml（anchoring / bloom 配置）
if exist "%REPO_ROOT%\core\src\mode\modes" (
    xcopy /E /I /Y /Q "%REPO_ROOT%\core\src\mode\modes" "%RES%\modes\" >nul
)

REM 4c. pyengine（轻量：仅 src + migrations + pyproject + 版本声明）
REM     [WARN] `migrations/` **必须**一起打进去：`resolve_migrations_dir()` 的仓库候选是
REM     `Path(__file__).parents[3]/"migrations"` —— 安装后即
REM     `<install>/resources/pyengine/migrations`。缺它时引擎启动直接
REM     `RuntimeError: opensquilla migrations directory not found`（实测），
REM     桌面端于是只剩纯 Rust 能力子集。
if exist "%REPO_ROOT%\pyengine" (
    REM /XD __pycache__ /XF *.pyc —— 直接 xcopy 会把本机跑过测试留下的字节码一起打包
    REM （实测暂存树里 599 个 __pycache__ 目录）。那是机器状态，不是发布内容。
    robocopy "%REPO_ROOT%\pyengine\src" "%RES%\pyengine\src" /E /NFL /NDL /NJH /NJS /NP /R:1 /W:1 /XD __pycache__ .pytest_cache /XF *.pyc *.pyo >nul
    if errorlevel 8 (
        echo [ERROR] pyengine\src 复制失败（robocopy）
        exit /b 1
    )
    if exist "%REPO_ROOT%\pyengine\migrations" (
        robocopy "%REPO_ROOT%\pyengine\migrations" "%RES%\pyengine\migrations" /E /NFL /NDL /NJH /NJS /NP /R:1 /W:1 /XD __pycache__ /XF *.pyc *.pyo >nul
        if errorlevel 8 (
            echo [ERROR] pyengine\migrations 复制失败（robocopy）
            exit /b 1
        )
    ) else (
        echo [ERROR] 未找到 pyengine\migrations —— 打包后的引擎无法启动（migrations 缺失）。
        exit /b 1
    )
    if exist "%REPO_ROOT%\pyengine\pyproject.toml" copy /Y "%REPO_ROOT%\pyengine\pyproject.toml" "%RES%\pyengine\" >nul
    if exist "%REPO_ROOT%\pyengine\.python-version" copy /Y "%REPO_ROOT%\pyengine\.python-version" "%RES%\pyengine\" >nul
)

REM 4c-bis. Control UI 必须同时落到 pyengine 的 static/dist
REM     Control UI 由内嵌网关提供，它只在
REM     resources/pyengine/src/opensquilla/gateway/static/dist 找已构建的 Vue 控制台
REM     （Tauri 另装一份到 <install>/webui/dist，control_ui._resolve_dist_dir 会回退命中）。
REM     缺失时桌面端只显示「资源不可用」告警 —— 2026-10-12 实测缺陷，故此处强制校验。
echo [4c-bis] 复制 webui/dist -^> resources/pyengine/src/opensquilla/gateway/static/dist
set "GW_STATIC_DIST=%RES%\pyengine\src\opensquilla\gateway\static\dist"
if exist "%GW_STATIC_DIST%" rmdir /s /q "%GW_STATIC_DIST%" 2>nul
xcopy /E /I /Y /Q "%WEBUI_SRC%\*" "%GW_STATIC_DIST%\" >nul
if not exist "%GW_STATIC_DIST%\index.html" (
    echo [ERROR] 控制台未落入 resources/pyengine/.../gateway/static/dist —— Control UI 将不可用
    exit /b 1
)
echo       桌面控制台就位（index.html + assets + pets/）.

REM 4d. btpanel-mcp（宝塔 MCP 服务端）
if exist "%REPO_ROOT%\btpanel-mcp" (
    robocopy "%REPO_ROOT%\btpanel-mcp\btpanel_mcp" "%RES%\btpanel-mcp\btpanel_mcp" /E /NFL /NDL /NJH /NJS /NP /R:1 /W:1 /XD __pycache__ .pytest_cache /XF *.pyc *.pyo >nul
    if errorlevel 8 (
        echo [ERROR] btpanel-mcp 复制失败（robocopy）
        exit /b 1
    )
    if exist "%REPO_ROOT%\btpanel-mcp\pyproject.toml" copy /Y "%REPO_ROOT%\btpanel-mcp\pyproject.toml" "%RES%\btpanel-mcp\" >nul
)

REM 4e. docs —— 只打「用户手册」白名单，不再整棵树打包。
REM     实测 0.2.17 把整棵 docs/（24 MB / 313 个文件）装了进去：Phase 进度、
REM     M2~M4 验收报告、Security_Audit_L3、竞品调研、20 MB 的 qa-shots 截图。
REM     [WARN] 目录本身**必须存在**：core/src/doctor/appliance.rs 的「嵌入资源完整性」
REM     检查要求 docs 可定位，缺了会让 doctor 报「缺失资源: docs」并阻塞就绪（实测 2026-10-21）。
REM     白名单里的 9 份手册互不引用（grep 实测 0 条相对 .md 链接），因此不会在
REM     安装包里留下死链；docs/README.md 反而链接 ../AGENTS.md 等未发布文件，故意不打。
set "USER_DOCS=overview.md cli-reference.md commands-and-modes-reference.md mcp.md skills.md memory.md goal.md heavy-runtime-guide.md ollama-local-models-e2e.md"
mkdir "%RES%\docs" 2>nul
for %%f in (%USER_DOCS%) do (
    if exist "%REPO_ROOT%\docs\%%f" copy /Y "%REPO_ROOT%\docs\%%f" "%RES%\docs\" >nul
)
if not exist "%RES%\docs\cli-reference.md" (
    echo [ERROR] docs\cli-reference.md 缺失 —— 安装包将没有用户手册，doctor 也会判「缺失资源: docs」
    exit /b 1
)

REM 4f. scripts（冒烟 / 校验子集）
if exist "%REPO_ROOT%\scripts" (
    mkdir "%RES%\scripts" 2>nul
    for %%f in (check-doc-cli-drift.py ws_full_health.py package_app.cmd) do (
        if exist "%REPO_ROOT%\scripts\%%f" copy /Y "%REPO_ROOT%\scripts\%%f" "%RES%\scripts\" >nul
    )
)

REM 4g. 不再打包 Cargo.lock —— 那是完整的内部依赖清单（含未发布 crate 名与来源），
REM     对用户没有用途，却是一张「内部技术栈地图」。依赖审计在仓库侧做。

echo       完成。

REM --------- Step 5. 更新 tauri.conf.json（追加 resources 路径 & 启用 nsis） ---------
echo [5/7] 写回 tauri.conf.json（resources = webui/dist + skills + resources/*, targets = [msi,nsis]）
set "TCONF=%TAURI_DIR%\tauri.conf.json"
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$c=Get-Content '%TCONF%' -Raw | ConvertFrom-Json;" ^
  "$c.bundle.targets = @('msi','nsis');" ^
  "$c.bundle.resources = @('webui/dist','skills','resources');" ^
  "$c.bundle.resources += 'resources/modes';" ^
  "$c.bundle.resources += 'resources/pyengine';" ^
  "$c.bundle.resources += 'resources/btpanel-mcp';" ^
  "$c.bundle.resources += 'resources/docs';" ^
  "$c.bundle.resources += 'resources/scripts';" ^
  "$c | ConvertTo-Json -Depth 100 | Out-String | Set-Content -LiteralPath '%TCONF%' -Encoding UTF8;" ^
  "$t=[System.IO.File]::ReadAllText('%TCONF%').TrimStart([char]0xFEFF);" ^
  "[System.IO.File]::WriteAllText('%TCONF%', $t, (New-Object System.Text.UTF8Encoding($false)))"
echo       完成。

REM --------- Step 5b. Git LFS 指针守卫（fail-closed） ----------
REM 2026-10-12 实测缺陷：squilla_router 的 .onnx/.pkl 在仓库里是 ~133 字节的 LFS 指针
REM （未 `git lfs pull`），被打进安装包后 V4 router **静默降级**为 fallback，本地 ML 路由失效
REM —— 安装包看起来正常、功能却悄悄少一块。此处 fail-closed：检出指针即中止。
echo [5b] Git LFS 指针守卫（模型资源必须是真实二进制）...
where python >nul 2>nul
if errorlevel 1 (
    echo [ERROR] 未找到 python，无法执行 scripts\check-lfs-models.py 守卫
    exit /b 1
)
python "scripts\check-lfs-models.py"
if errorlevel 1 (
    echo [ERROR] Git LFS 指针守卫未通过，打包中止（详见上方）
    exit /b 1
)

REM --------- Step 5c. 编译 Python 侧车为原生码（防反编译，fail-closed）----------
REM 已发布的 0.2.17 MSI 里躺着 1017 个 .py / 17.1 MB 明文引擎源码：反编译等于零成本。
REM 本步在**暂存树**上把侧车编成 .pyd（Cython -> MSVC -> 内置 3.12 ABI），并写
REM resources/pyengine/NATIVE.json 逐模块清单；Step 5d 守卫按这份清单逐文件对账，
REM Step 6 用 MARGUERITE_NATIVE_BUILD=1 让 build.rs 不再无条件重拷明文（否则加固白做，
REM 且它的 verify_pyengine_sync 会因「源有目标无」直接 panic）。
REM rc=2 = 前置缺失（内置解释器 / include / libs / MSVC），rc=1 = 编出来的模块 import 不回来。
REM 两种都必须中止构建：宁可不出包，也不出「照发源码」或「装得上但引擎起不来」的包。
echo [5c] 编译 Python 侧车为原生码（scripts\compile_pyengine_native.py）...
where python >nul 2>nul
if errorlevel 1 (
    echo [ERROR] 未找到 python，无法执行 scripts\compile_pyengine_native.py —— 构建中止
    exit /b 1
)
python "scripts\compile_pyengine_native.py"
if errorlevel 1 (
    echo [ERROR] 侧车原生编译未通过（rc=2 为前置缺失，rc=1 为原生模块 import 回归）—— 构建中止
    exit /b 1
)

REM --------- Step 5d. 暂存资源卫生守卫（fail-closed） ----------
REM bundle.resources 是三条 xcopy 出来的树，webui 自己的 verify-dist 看不见另外两条
REM （`resources/` 与 `gui/src-tauri/webui/dist` 的暂存副本）。0.2.17 实测：两处各
REM 103 个 .map、304 个内部文档、AGENTS.md/Cargo.lock/config.yaml、2 个 __pycache__。
REM 此处不通过就不构建 —— 宁可打包失败，也不要产出一个「装着源码的安装包」。
echo [5d] 暂存资源卫生守卫（源码映射 / 内部文档 / 机器状态）...
python "scripts\check-shipped-resources.py"
if errorlevel 1 (
    echo [ERROR] 暂存资源卫生守卫未通过，打包中止（详见上方）
    exit /b 1
)

REM --------- Step 6. cargo tauri build ---------
where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] 未找到 cargo！请先安装 Rust（rustup.rs）并加入 PATH。
    exit /b 1
)

echo [6/7] 执行 cargo tauri build（MSI + NSIS，WebView2 offlineInstaller 静默内置）
cd /d "%TAURI_DIR%"
set "CARGO_TARGET_DIR=%TARGET_DIR%"
set "RUST_BACKTRACE=1"
REM Step 5c 已把 resources\pyengine 换成 .pyd 树。build.rs 默认会 remove_dir_all 后
REM 全量重拷，把仓库里的明文 .py 原样复制回来（加固白做），并且 verify_pyengine_sync
REM 会因为「源有目标无」直接 panic。这个开关让它让路，资源就位改由 Step 5c 负责。
set "MARGUERITE_NATIVE_BUILD=1"
cargo tauri build --verbose
if errorlevel 1 (
    echo.
    echo [ERROR] Tauri 打包失败！见上方日志。
    exit /b 1
)

REM --------- Step 6b. 物化资源复扫（构建后再查一次） ---------
REM gui\src-tauri\build.rs 在构建**期间**会无条件重拷 resources\pyengine 与
REM 内嵌网关的 static/dist —— 也就是说 Step 5d 查过的那棵树，随后会被覆盖重做。
REM 只查暂存树因此可能「守卫绿、包里红」。实测上一版构建留下的
REM %TARGET_DIR%\release\resources 快照里：425 个 .map、4900 份可还原源码、
REM 134 KB 的 AGENTS.md、22.3 MB / 313 个内部文档。所以这一步不能省。
cd /d "%REPO_ROOT%"
echo [6b] 复扫构建实际携带的资源树 ...
if not exist "%TARGET_DIR%\release\resources" (
    echo [ERROR] 未找到 %TARGET_DIR%\release\resources —— 无法验证安装包真实携带内容
    exit /b 1
)
python "scripts\check-shipped-resources.py" --root "%TARGET_DIR%\release\resources"
if errorlevel 1 (
    echo [ERROR] 构建产物资源卫生守卫未通过，本次安装包不可发布（详见上方）
    exit /b 1
)

REM --------- Step 7. 产物输出 ---------
echo.
echo ============================================================
echo  [7/7] 打包完成！
echo  产物目录: %TARGET_DIR%\release\bundle\
echo ============================================================
dir /s /b "%TARGET_DIR%\release\bundle\*.msi" "%TARGET_DIR%\release\bundle\*.exe" 2>nul
endlocal
