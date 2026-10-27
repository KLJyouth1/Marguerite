@echo off
chcp 65001 >nul
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
if exist "%RES%" rmdir /s /q "%RES%" 2>nul
mkdir "%RES%" 2>nul

REM 4a. 根配置文件
if exist "%REPO_ROOT%\config.yaml"  copy /Y "%REPO_ROOT%\config.yaml" "%RES%\" >nul
if exist "%REPO_ROOT%\AGENTS.md"    copy /Y "%REPO_ROOT%\AGENTS.md" "%RES%\" >nul
if exist "%REPO_ROOT%\CHANGELOG.md" copy /Y "%REPO_ROOT%\CHANGELOG.md" "%RES%\" >nul
if exist "%REPO_ROOT%\README.md"    copy /Y "%REPO_ROOT%\README.md" "%RES%\" >nul
if exist "%REPO_ROOT%\rust-toolchain.toml" copy /Y "%REPO_ROOT%\rust-toolchain.toml" "%RES%\" >nul

REM 4b. mode/*.yaml（anchoring / bloom 配置）
if exist "%REPO_ROOT%\core\src\mode\modes" (
    xcopy /E /I /Y /Q "%REPO_ROOT%\core\src\mode\modes" "%RES%\modes\" >nul
)

REM 4c. pyengine（轻量：仅 src + migrations + pyproject + 版本声明）
REM     ⚠️ `migrations/` **必须**一起打进去：`resolve_migrations_dir()` 的仓库候选是
REM     `Path(__file__).parents[3]/"migrations"` —— 安装后即
REM     `<install>/resources/pyengine/migrations`。缺它时引擎启动直接
REM     `RuntimeError: opensquilla migrations directory not found`（实测），
REM     桌面端于是只剩纯 Rust 能力子集。
if exist "%REPO_ROOT%\pyengine" (
    xcopy /E /I /Y /Q "%REPO_ROOT%\pyengine\src" "%RES%\pyengine\src\" >nul
    if exist "%REPO_ROOT%\pyengine\migrations" (
        xcopy /E /I /Y /Q "%REPO_ROOT%\pyengine\migrations" "%RES%\pyengine\migrations\" >nul
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
    xcopy /E /I /Y /Q "%REPO_ROOT%\btpanel-mcp\btpanel_mcp" "%RES%\btpanel-mcp\btpanel_mcp\" >nul
    if exist "%REPO_ROOT%\btpanel-mcp\pyproject.toml" copy /Y "%REPO_ROOT%\btpanel-mcp\pyproject.toml" "%RES%\btpanel-mcp\" >nul
)

REM 4e. docs（用户手册子集）
if exist "%REPO_ROOT%\docs" (
    xcopy /E /I /Y /Q "%REPO_ROOT%\docs" "%RES%\docs\" >nul
)

REM 4f. scripts（冒烟 / 校验子集）
if exist "%REPO_ROOT%\scripts" (
    mkdir "%RES%\scripts" 2>nul
    for %%f in (check-doc-cli-drift.py ws_full_health.py package_app.cmd) do (
        if exist "%REPO_ROOT%\scripts\%%f" copy /Y "%REPO_ROOT%\scripts\%%f" "%RES%\scripts\" >nul
    )
)

REM 4g. Cargo.lock（便于依赖审计）
if exist "%REPO_ROOT%\Cargo.lock" copy /Y "%REPO_ROOT%\Cargo.lock" "%RES%\" >nul

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
cargo tauri build --verbose
if errorlevel 1 (
    echo.
    echo [ERROR] Tauri 打包失败！见上方日志。
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
