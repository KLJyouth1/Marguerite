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
REM 【2026-10-22 修】原实现 `rmdir /s /q "%TARGET_DIR%" 2>nul` 把删除失败**静默吞掉**，
REM 但注释却声称「每次构建前清空该目录，保证干净」。实测后果（当天连踩三次）：
REM   目录没删净 → tauri bundling 对 bundle.resources 是「合并覆盖」而非清空重建
REM   → 包内混入上一版的旧哈希 chunk（实测 92 个死文件 / 19.8 MB），
REM     且 HEVC 原件这类已移出源码的素材仍留在包里。
REM 现在：删完必须**验证真的不存在**，否则中止。
if exist "%TARGET_DIR%" (
    echo [pkg] 清空隔离构建目录 %TARGET_DIR% ...
    rmdir /s /q "%TARGET_DIR%" 2>nul
    if exist "%TARGET_DIR%" (
        echo [pkg][错误] 无法清空 %TARGET_DIR%
        echo [pkg] 常见占用者：正在运行的 marguerite-gui.exe / 杀软扫描 / 搜索索引器 / 编辑器。
        echo [pkg] 继续构建会打出混入上一版前端产物的包，故中止。请关闭占用进程后重试。
        exit /b 1
    )
)
mkdir "%TARGET_DIR%" 2>nul
if not exist "%TARGET_DIR%" (
    echo [pkg][错误] 无法创建隔离构建目录 %TARGET_DIR%
    exit /b 1
)

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
REM 桌宠素材守卫（2026-10-23 起改为「不再强制位图」）：
REM   历史：曾有 4 张精灵皮肤 JPG（pets/skin-*.jpg），缺失会让桌宠退化成静态图，
REM   因此这里硬性要求它们存在。现在皮肤系统已全部改为 canvas/3D 模型渲染
REM   （见 webui/src/components/pet/skins.ts：kind=canvas|model，列表内无 image 皮肤），
REM   位图已随「无第三方水印素材」要求下线 —— 再要求 JPG 只会让每次打包都中止。
REM   改为：有 pets/ 目录就校验内容非空（防止"半拷贝"），没有也不阻断，
REM   但必须存在 index.html（前端产物本体）。
set "PETS_SRC=%WEBUI_SRC%\pets"
if exist "%PETS_SRC%" (
    echo [pkg] 检测到 pets/ 目录（位图皮肤，可选），校验非空 ...
    REM 用 errorlevel 而不是 %VAR%：整个 if 块在解析期展开变量，for 循环里赋的值读不到
    REM （延迟展开陷阱）。find /v "" 在无输入行时返回 1。
    dir /b /a-d "%PETS_SRC%" 2^>nul | find /v "" >nul
    if errorlevel 1 echo [pkg][警告] pets/ 目录存在但为空 —— 若皮肤列表仍引用位图将 404。
) else (
    echo [pkg] 无 pets/ 目录（当前皮肤均为 canvas/3D 渲染，属正常）。
)
if not exist "%WEBUI_SRC%\index.html" (
    echo [pkg][错误] webui/dist 缺少 index.html（前端产物不完整）。
    echo [pkg] 请重新执行 npm run build。
    exit /b 1
)
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
REM 构建前再确认一次「打包层的前端副本」不存在。它是 tauri bundling 的落点，
REM 与 build.rs 管的 gui/src-tauri/webui/dist 是**两份不同副本**——历史上没人重置它。
set "PKG_WEBUI=%TARGET_DIR%\release\webui\dist"
if exist "%PKG_WEBUI%" (
    echo [pkg][错误] 打包层前端副本已存在（本应在第 1 步随 target 一起清掉）：%PKG_WEBUI%
    echo [pkg] 它会导致新产物与旧产物混合。请手动删除后重试。
    exit /b 1
)

echo [pkg] 开始 cargo tauri build（target=%TARGET_DIR%，offline=%OFFLINE%）
cd /d "%TAURI_DIR%" || exit /b 1
set "CARGO_TARGET_DIR=%TARGET_DIR%"
cargo tauri build %OFFLINE%
if errorlevel 1 (
    echo [pkg][错误] 构建失败，详见上方日志。
    exit /b 1
)

REM ---- 6. 打包后校验：包内前端必须与源码产物**逐文件一致** ----
REM 只比对文件数就能抓住「合并覆盖」造成的残留（实测差 92 个）。
REM 数量一致再加一道双向差集，防止「少一个、多一个」恰好抵消。
if not exist "%PKG_WEBUI%" (
    echo [pkg][错误] 包内前端目录不存在：%PKG_WEBUI%
    exit /b 1
)
set "SRC_COUNT=0"
set "PKG_COUNT=0"
for /f %%C in ('dir /b /s /a-d "%WEBUI_SRC%" 2^>nul ^| find /c /v ""') do set "SRC_COUNT=%%C"
for /f %%C in ('dir /b /s /a-d "%PKG_WEBUI%" 2^>nul ^| find /c /v ""') do set "PKG_COUNT=%%C"
echo [pkg] 前端文件数：源码=%SRC_COUNT% 包内=%PKG_COUNT%
if not "%SRC_COUNT%"=="%PKG_COUNT%" (
    echo [pkg][错误] 包内前端文件数与源码产物不一致 —— 打包目录混入了上一版残留。
    echo [pkg] 根因：tauri bundling 对 bundle.resources 是「合并覆盖」，不清空目标目录。
    echo [pkg] 处理：删除 "%PKG_WEBUI%" 后重打，或清空 %TARGET_DIR% 全量重建。
    exit /b 1
)
echo [pkg] 前端产物一致性校验通过（%SRC_COUNT% 个文件）。

echo [pkg] 构建完成。产物位于 %TARGET_DIR%\release\bundle\ 下（msi / nsis）。
endlocal
