# Marguerite Workbench — 统一工作台（唯一维护入口）

> 本目录是 **File Intelligence + LLM 实验台 + 智能体循环** 工作台的融合根。
> 版本单一来源：[`VERSION`](VERSION)（由 `packaging/check_version_sync.py` 强制与
> backend/frontend 声明同步）。

## 目录布局（权威）

```
workbench/
├─ VERSION                    # X.Y.Z — 唯一版本声明源
├─ backend/
│  └─ marguerite_workbench/   # Python 包：engine · API · CLI · llmlab · ttc · agentloop
│     └─ tests/               # 71 个离线测试（pytest，无 key 可跑）
├─ frontend/                  # React SPA（原 workbench-web）
│  └─ src/ …                  # registry 注册制面板；tokens.css 与 webui 主题契约同步
├─ packaging/
│  ├─ package_workbench.py    # 构建→暂存→真启动验证→zip 一条龙
│  ├─ check_version_sync.py   # 三处版本漂移门禁
│  └─ consolidate_layout.py   # 一次性迁移脚本（已执行，留档可审计）
└─ dist/                      # 生成的 zip（Marguerite_Workbench_<ver>.zip）
```

## 四前端拓扑（不是冗余 —— 各司其职）

| 前端 | 位置 | 技术栈 | 托管方 | 端口 | 定位 |
|---|---|---|---|---|---|
| TUI 终端台 | `tui/` | Rust ratatui | 终端直接运行 | — | 键盘优先的本地驾驶舱 |
| 桌面端 | `gui/` | Tauri + React | 原生窗口 | — | 唯一原生桌面入口 |
| Web 控制台 | `webui/` | Vue3 + Pinia | 统一网关（回环） | 18790 | 引擎全能力控制台（模式/缓存/会话/插件） |
| **工作台 SPA** | `workbench/frontend/` | React + TS | FastAPI 同源托管 | 8011 | 文件智能 + LLM 实验 + Agent 循环的可视化实验台 |

共享契约：三端品牌令牌统一为「**蓝主 + 橙强**」（webui `--accent/--accent-secondary`
≙ workbench `--brand/--accent-2`；light `#1A56DB/#C2410C`，dark `#60A5FA/#F97316`，
出处 `webui/src/themes/{light,dark}/tokens.css`）。Python 侧车引擎经
`core_bridge` 物理桥接核心 store（CoreSqlite），与 `pyengine` 同一哲学。

## 常用命令

```bash
# 后端测试（71 例，离线确定性）
cd workbench/backend && python -m pytest marguerite_workbench/tests -q

# 前端构建 / 开发
cd workbench/frontend && npm run build        # tsc + vite，必须干净
cd workbench/frontend && npm run dev          # http://localhost:5173（代理 /api → :8011）

# 版本漂移检查 —— 现在是真门禁：由 verify.sh --workbench 调度
python workbench/packaging/check_version_sync.py

# 本子项目的完整门禁层（71 个离线用例 + 上面那条版本检查）
bash scripts/verify.sh --workbench

# 打包（构建→暂存→真启动验证→zip 到 workbench/dist/）
python workbench/packaging/package_workbench.py
```

发布产物即解即用：解压 zip 后运行 `start.bat`（或 `start.ps1` / `start.sh`）
→ http://127.0.0.1:8011 单端口获得完整工作台。

## 维护约定

1. **只认本 README 的布局**；历史路径 `Marguerite/marguerite_workbench`、
   `Marguerite/workbench-web` 已迁移，旧引用以本文件为准。
2. 升版本 = 改 `VERSION` + `backend api.py` + `frontend package.json` 三处一致，
   `check_version_sync.py` 不过不许打包。
   ⚠️ 本条此前**只是声明**：`grep -c workbench scripts/verify.sh` 实测为 **0**，
   即这 71 个用例与这条版本检查**没有任何门禁跑过** —— 而 `workbench/` 是随提交发布的
   （commit set 里 76 个文件）。R28 已把两者接进 `verify.sh --workbench` 并纳入 `--all`，
   声明与事实现在一致。改动本目录后请跑 `bash scripts/verify.sh --workbench`。
3. 根目录历史杂物已归档至 `../_archive/pre-consolidation-20260824/`（含清单
   `_manifest.txt`，仅移动未删除，可随时找回）。
