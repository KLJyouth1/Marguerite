@echo off
chcp 65001 >nul
REM ^ switch console to UTF-8 so the Chinese comments below parse correctly (GBK default mangles them).
REM ============================================================================
REM Marguerite 桌面端打包脚本（Windows / MSI + NSIS）
REM
REM 环境锁说明：本机存在文件锁（os error 5 / Permission denied），background
REM cargo 构建会静默失败或日志被锁。因此本脚本：
REM   1) 使用隔离的 CARGO_TARGET_DIR（D:\marg_build_target），避免与默认 target
REM      目录的锁文件冲突；
REM   2) 每次构建前清空该目录，保证干净；
REM   3) 必须在“干净机器”上运行（本机仅用于产出脚本，不在本机编译验证）。
REM
REM 首次构建必须联网：需要拉取两个新增的 Tauri 插件（dialog / single-instance）
REM 以及 WebView2 离线安装包（offlineInstaller）。cargo 缓存预热后，可设
REM   set OFFLINE=--offline
REM 进行离线构建，进一步规避本机注册表文件锁。
REM ============================================================================

setlocal

REM ---- 0. 路径 ----
set "REPO_ROOT=%~dp0.."
set "TAURI_DIR=%REPO_ROOT%\gui\src-tauri"
REM 前端源码根 = 仓库根 webui/（Vue Web 控制台，用户指定的 UI）。
set "WEBUI_SRC=%REPO_ROOT%\webui\dist"
set "WEBUI_DST=%TAURI_DIR%\webui\dist"
set "TARGET_DIR=D:\marg_build_target"

REM ---- 1. 隔离构建目录（规避文件锁）----
if exist "%TARGET_DIR%" (
    echo [pkg] 清空隔离构建目录 %TARGET_DIR% ...
    rmdir /s /q "%TARGET_DIR%" 2>nul
)
mkdir "%TARGET_DIR%" 2>nul

REM ---- 2. 清理旧构建残留（防止旧产物污染打包）----
echo [pkg] 清理 webui/dist.old* 旧目录 ...
for /d %%D in ("%REPO_ROOT%\webui\dist.old*") do (
    if exist "%%D" rmdir /s /q "%%D" 2>nul && echo   removed %%D
)

REM ---- 3. 嵌入已构建的 webui/dist ----
REM     资源已声明于 tauri.conf.json 的 bundle.resources（"webui/dist"，相对
REM     src-tauri）。此处用根目录最新构建覆盖 gui/src-tauri/webui/dist，确保打包
REM     的是最新控制台产物（resolve_webui_dir 会优先命中 resource_dir/webui/dist）。
if not exist "%WEBUI_SRC%" (
    echo [pkg][错误] 未找到已构建的 webui/dist：%WEBUI_SRC%
    echo [pkg] 请先构建 webui（npm run build）再打包。
    exit /b 1
)
REM 桌宠图片皮肤完整性检查：打包产物若缺 pets/ 目录，精灵皮肤会 404，
REM 桌面宠物退化为「一张静态图」（已知 P0）。此处强制校验，缺失即中止。
set "PETS_SRC=%WEBUI_SRC%\pets"
if not exist "%PETS_SRC%" (
    echo [pkg][错误] webui/dist 缺少 pets/ 目录（桌宠图片皮肤）。
    echo [pkg] 请确认 webui/public/pets/*.jpg 存在并已构建（npm run build）。
    exit /b 1
)
for %%F in (skin-blue.jpg skin-green.jpg skin-pink.jpg skin-purple.jpg) do (
    if not exist "%PETS_SRC%\%%F" (
        echo [pkg][错误] 桌宠皮肤缺失：pets\%%F
        echo [pkg] 请确认 webui/public/pets\%%F 已存在并重新构建。
        exit /b 1
    )
)
echo [pkg] 桌宠皮肤完整性检查通过（4 张精灵图均存在）。
echo [pkg] 嵌入 webui/dist -^> %WEBUI_DST%
if exist "%WEBUI_DST%" rmdir /s /q "%WEBUI_DST%" 2>nul
xcopy /E /I /Y "%WEBUI_SRC%" "%WEBUI_DST%" >nul
REM 桌面控制台还必须落到内嵌网关的 static/dist：Control UI 只在该目录找已构建的控制台，
REM 缺失时桌面端只显示「资源不可用」告警（2026-10-12 实测缺陷）。此处强制校验。
REM 注意（2026-10-13）：`gui/src-tauri/build.rs` 现在**也会**在 pyengine 重拷之后自动把
REM webui/dist 拷到这里 —— 因为 build.rs 的 pyengine 块是无条件重拷，且仓库源
REM pyengine/src/opensquilla/gateway/static/ 没有 dist/，所以本脚本这一步会被构建期覆盖。
REM 这里保留为**前置守卫**（在编译前就发现前端产物缺失），不再承担唯一的「就位」职责。
set "GW_STATIC_DIST=%TAURI_DIR%\resources\pyengine\src\opensquilla\gateway\static\dist"
if exist "%GW_STATIC_DIST%" rmdir /s /q "%GW_STATIC_DIST%" 2>nul
xcopy /E /I /Y "%WEBUI_SRC%" "%GW_STATIC_DIST%" >nul
if not exist "%GW_STATIC_DIST%\index.html" (
    echo [pkg][错误] 控制台未就位于 resources\pyengine\...\gateway\static\dist —— Control UI 将不可用
    exit /b 1
)
echo [pkg] 桌面控制台就位于网关 static/dist（index.html + pets/）.

REM ---- 3. 定位 cargo ----
where cargo >nul 2>nul
if errorlevel 1 (
    echo [pkg][错误] 未找到 cargo，请先安装 Rust 并加入 PATH（https://rustup.rs）。
    exit /b 1
)

REM ---- 4. 离线开关（默认空；cargo 缓存预热后可设 --offline）----
set "OFFLINE="

REM ---- 5. 构建（MSI + NSIS；WebView2 以 offlineInstaller 静默内置）----
echo [pkg] 开始 cargo tauri build（target=%TARGET_DIR%，offline=%OFFLINE%）
cd /d "%TAURI_DIR%" || exit /b 1
set "CARGO_TARGET_DIR=%TARGET_DIR%"
cargo tauri build %OFFLINE%
if errorlevel 1 (
    echo [pkg][错误] 构建失败，详见上方日志。
    exit /b 1
)

echo [pkg] 构建完成。产物位于 %TARGET_DIR%\release\bundle\ 下（msi / nsis）。
endlocal
