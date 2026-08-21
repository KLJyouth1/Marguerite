@echo off
chcp 65001 >nul
REM ============================================================
REM  Marguerite 全量打包 (Release v0.1.4) - 最新版安装包
REM  打包内容：webui / 全 skills / 全工具 / 全配置 / 网关 / pyengine / btpanel-mcp / mode YAML / scripts
REM ============================================================
setlocal EnableExtensions EnableDelayedExpansion

set "REPO_ROOT=%~dp0..\.."
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

REM --------- Step 2. webui/dist 拷贝（覆盖） ---------
echo [2/7] 嵌入 Web UI（webui/dist -^> gui/src-tauri/webui/dist）
if not exist "%WEBUI_SRC%" (
    echo [ERROR] 未找到 webui/dist！请先在 webui 下执行: npm run build
    exit /b 1
)
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

REM 4c. pyengine（轻量：仅 src + pyproject + 版本声明）
if exist "%REPO_ROOT%\pyengine" (
    xcopy /E /I /Y /Q "%REPO_ROOT%\pyengine\src" "%RES%\pyengine\src\" >nul
    if exist "%REPO_ROOT%\pyengine\pyproject.toml" copy /Y "%REPO_ROOT%\pyengine\pyproject.toml" "%RES%\pyengine\" >nul
    if exist "%REPO_ROOT%\pyengine\.python-version" copy /Y "%REPO_ROOT%\pyengine\.python-version" "%RES%\pyengine\" >nul
)

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
    for %%f in (check-doc-cli-drift.py smoke_gateway.cjs ws_full_health.py package_app.cmd) do (
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
  "$c.bundle.resources = @('webui/dist','skills','resources/*','resources/**');" ^
  "$c.bundle.resources += 'resources/modes/**';" ^
  "$c.bundle.resources += 'resources/pyengine/**';" ^
  "$c.bundle.resources += 'resources/btpanel-mcp/**';" ^
  "$c.bundle.resources += 'resources/docs/**';" ^
  "$c.bundle.resources += 'resources/scripts/**';" ^
  "$c | ConvertTo-Json -Depth 100 | Set-Content '%TCONF%' -Encoding UTF8"
echo       完成。

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
