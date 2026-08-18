# Marguerite 干净机器打包手册（含运行环境 / 后端 / WebUI 内嵌）

> 目标：产出一个**自包含 Windows 安装包**，安装后开箱即用，守住三条红线：
> 1. **中文不乱码** —— Rust `String`(UTF-8) + WebUI `vue-i18n` 全量 locale；
> 2. **网关可用** —— `marguerite-core` 编译进同一个 exe，进程内监听 `127.0.0.1:18790`；
> 3. **不弹依赖缺失** —— WebView2 以 `offlineInstaller` 随包静默内置。

---

## 0. 架构与「内嵌什么」

| 组件 | 形态 | 打包方式 |
|------|------|----------|
| 运行环境（WebView2 Runtime） | `offlineInstaller` 静默内置 | `tauri.conf.json` → `bundle.windows.webviewInstallMode = { type: "offlineInstaller" }` |
| 后端（marguerite-core 网关） | 编译进 `gui` exe（同进程） | `cargo tauri build` 链接 `core` crate |
| 前端（webui） | 静态产物 `webui/dist` | 声明为 Tauri 资源 `bundle.resources = ["webui/dist"]`，由 `package_app.cmd` 预拷贝进 `gui/src-tauri/webui/dist` |
| 应用窗口 | 加载 `http://localhost:18790/control/` | `tauri.conf.json` → `app.windows[0].url` |

> 关键：`tauri.conf.json` 的 `build.frontendDist = "webui/dist"`，但 `beforeBuildCommand` 仅打印提示——
> **前端必须在打包前自行 `npm run build`**，再由 `package_app.cmd` 把根 `webui/dist` 覆盖进 `gui/src-tauri/webui/dist`。

---

## 1. 前置条件（干净机器）

> 「干净机器」定义：一台**没有本仓开发文件锁 / 注册表锁**的 Windows（≥10 64-bit）。
> 本开发机存在 `os error 5 / Permission denied` 文件锁，background cargo 会静默失败——**本机只用于产出脚本，不在本机编译验证**。

1. **Windows 10/11 x64**，管理员权限。
2. **Rust 工具链**：`rustup` 安装 Stable MSVC target（`cargo` 须在 `PATH`）。
   - 安装：https://rustup.rs  →  勾选「Add to PATH」。
3. **Node.js ≥ 18 LTS** + npm（`webui` 构建依赖）。
4. **Visual Studio Build Tools**（MSVC C++ 工作负载）——Tauri 原生模块需要。
5. **首次构建必须联网**：需拉取两个 Tauri 插件（`dialog` / `single-instance`）及 WebView2 `offlineInstaller`。
   - 联网跑通一次、`cargo` 缓存预热后，可设 `set OFFLINE=--offline` 走离线构建。
6. **确保 `Cargo.lock` 可写**：在 sandbox / 只读挂载下 `Cargo.lock` 可能被锁，导致 `cargo tauri build` 失败。在干净机器上以普通可写目录打开仓库即可。

---

## 2. 打包步骤（标准路径）

```bat
REM 步骤 A：构建前端（在仓库根目录）
cd D:\path\to\marguerite
cd webui
npm install
npm run build          REM 产物写入 webui/dist
cd ..

REM 步骤 B：执行打包脚本（自动嵌入 dist + 隔离构建 + 产出 MSI/NSIS）
scripts\package_app.cmd
```

`package_app.cmd` 内部动作（已校验）：
1. 设隔离 `CARGO_TARGET_DIR=D:\marg_build_target` 并清空——**规避本机文件锁**；
2. 校验 `webui/dist` 存在，否则报错退出；
3. `xcopy /E /I /Y webui\dist gui\src-tauri\webui\dist`——嵌入最新前端；
4. `where cargo` 校验工具链；
5. `cd gui\src-tauri && cargo tauri build`（默认 `OFFLINE` 空；可 `set OFFLINE=--offline`）；
6. 产物位于 `D:\marg_build_target\release\bundle\`（含 `msi/` 与 `nsis/`）。

---

## 3. 产物核对清单

安装包产出后，确认以下三项已内嵌（三条红线的物理保证）：

| 红线 | 核对点 | 在哪看 |
|------|--------|--------|
| 不弹依赖缺失 | WebView2 `offlineInstaller` 随包 | `tauri.conf.json` `bundle.windows.webviewInstallMode.type = "offlineInstaller"`；安装日志无「请下载 WebView2」弹窗 |
| 网关可用 | core 编译进 exe | 安装后启动 app，访问 `http://127.0.0.1:18790/control/` 返回控制台 HTML |
| 中文不乱码 | webui locale 完整 | `webui/dist/assets/*.js` 含 `zh-Hans` 文案；设置页切换语言正常 |

---

## 4. 三条红线验证（安装后冒烟）

```bat
REM 红线②：网关可用
curl -s -o nul -w "%{http_code}" http://127.0.0.1:18790/control/
REM 期望 200

REM 红线③：WebView2 已内置（无弹窗即达标；可查注册表/安装目录）
REM 控制面板 → 程序 → 应能看到 "Microsoft WebView2 Runtime"

REM 红线①：中文不乱码
REM 打开 app → 设置 → 语言切到 简体中文 → 界面与日志无豆腐块/乱码
```

网关健康检查也可走项目既有冒烟脚本（如 `smoke_gateway.cjs` / `ws_full_health.py`），确认 `18790` 握手与事件总线可达。

---

## 5. 常见问题（Troubleshooting）

| 现象 | 原因 | 处理 |
|------|------|------|
| `背景 cargo 静默失败 / os error 5` | 本机注册表/文件锁 | 必须在**干净机器**跑；或确保 `CARGO_TARGET_DIR` 指向无锁目录 |
| `未找到 cargo` | PATH 未含 Rust | 重装 rustup 并勾选 Add to PATH；或在脚本前 `set PATH=%PATH%;%USERPROFILE%\.cargo\bin` |
| `未找到已构建的 webui/dist` | 前端未 build | 先 `cd webui && npm install && npm run build` |
| `Cargo.lock` 只读 / 构建中断 | sandbox / 只读挂载 | 在可写目录打开仓库；不要从只读镜像直接构建 |
| 首次构建卡在拉取插件/WebView2 | 未联网 | 首次必须联网；预热后 `set OFFLINE=--offline` |
| 安装后打开空白 / 网关连不上 | `webui/dist` 未更新或 core 未链接 | 重跑 `package_app.cmd`；确认 `gui/src-tauri/webui/dist` 是最新构建 |

---

## 6. 产物位置与交付

- MSI：`D:\marg_build_target\release\bundle\msi\Marguerite_0.1.0_x64_en-US.msi`
- NSIS：`D:\marg_build_target\release\bundle\nsis\Marguerite_0.1.0_x64-setup.exe`

两者均**自包含**（WebView2 + core-in-exe + webui/dist），目标机器无需任何前置运行时。
