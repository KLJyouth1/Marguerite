@echo off
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
set "WEBUI_SRC=%REPO_ROOT%\webui\dist"
set "WEBUI_DST=%TAURI_DIR%\webui\dist"
set "TARGET_DIR=C:\marg_build_target"

REM ---- 1. 隔离构建目录（规避文件锁）----
if exist "%TARGET_DIR%" (
    echo [pkg] 清空隔离构建目录 %TARGET_DIR% ...
    rmdir /s /q "%TARGET_DIR%" 2>nul
)
mkdir "%TARGET_DIR%" 2>nul

REM ---- 2. 嵌入已构建的 webui/dist ----
REM     资源已声明于 tauri.conf.json 的 bundle.resources（"webui/dist"，相对
REM     src-tauri）。此处用根目录最新构建覆盖 gui/src-tauri/webui/dist，确保打包
REM     的是最新控制台产物（resolve_webui_dir 会优先命中 resource_dir/webui/dist）。
if not exist "%WEBUI_SRC%" (
    echo [pkg][错误] 未找到已构建的 webui/dist：%WEBUI_SRC%
    echo [pkg] 请先构建 webui（npm run build）再打包。
    exit /b 1
)
echo [pkg] 嵌入 webui/dist -^> %WEBUI_DST%
if exist "%WEBUI_DST%" rmdir /s /q "%WEBUI_DST%" 2>nul
xcopy /E /I /Y "%WEBUI_SRC%" "%WEBUI_DST%" >nul

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
