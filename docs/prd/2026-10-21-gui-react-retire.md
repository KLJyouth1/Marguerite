# PRD：gui/src 双前端收敛（S 档）

- 日期：2026-10-21
- 状态：**已决策，执行中**
- 决策人：本会话（用户授权"你决定并拍板执行"）

## 1. 问题

仓库同时维护**两套 Web 前端**：

| | `gui/src`（React） | `webui/`（Vue 3） |
|---|---|---|
| 产物 | `gui/dist` | `webui/dist` |
| 是否进安装包 | **否**（`bundle.resources` 不含 `gui/dist`） | **是** |
| 主窗口是否加载 | **否**（`url = http://localhost:18790/control/`） | **是** |
| 视图/面板 | 20 个面板 | 40 个视图、40 条路由 |
| 测试 | 70 | 3509 |
| 设计 token 守卫 | 1 套（本会话新增） | 10 项 `check:architecture` |
| i18n | 无 | 6 语言 |

`gui/src` 会被 `npm run build` 与 CI 构建，**但用户永远看不到**。这是纯粹的维护负债：
两套 token 纪律、两套测试、两套导航分类法，而其中一套不产生任何用户价值。

## 2. 竞品事实（本机实地勘察，2026-10-21）

| 产品 | 内核 | 前端组织 | 是否双框架 |
|---|---|---|---|
| WorkBuddyAI | Electron（`LICENSE.electron.txt` + `chrome_*.pak`） | 单 `renderer/`，Vite code-split（`AccountPanel-<hash>.js`、`index.esm-<hash>.js`） | 否 |
| DuMate | Electron | 单 `app.asar` + `build-ts-electron` | 否 |
| Trae CN | Electron（VSCode fork：`out/vs/workbench`、`nls.*.messages.json`） | 单 workbench renderer | 否 |
| TRAE SOLO CN | Electron（同上，`aha_*` 壳） | 单 workbench renderer | 否 |
| ZCode | Electron | 单 `app.asar` | 否 |

**5/5 全部是「Chromium + 单个 Web renderer」。没有一家同时维护 React 与 Vue 两套界面。**

行业做法是：**一个壳，一套 Web UI，能力靠"路由/面板按需 code-split"扩展**。

## 3. 目标

- **G1**：前端收敛到一套（`webui/`），删除 `gui/src` 的维护面，同时**不减少任何用户可见能力**。
- **G2**：`gui` 退化为**纯 Tauri 壳**——只保留 Rust 侧（commands / 插件 / 窗口 / 打包），
  其职责与竞品一致：启动网关、托管 `webui/dist`、提供原生能力（通知 / 窗口 / 快捷键）。

## 4. 非目标（明确不做）

- **NG1**：不做"把 webui 迁到 React"的等价重构。webui 有 3509 测试与 10 项架构守卫，
  迁移动机为零，成本极高。
- **NG2**：不删除 `gui/src-tauri`。Rust 壳是真生效层（`cron_*`、`notify_turn_ended`、
  桌宠窗、单实例、全局快捷键），删了产品就没了。
- **NG3**：不硬删 `gui/src` 文件。**先归档**（`_archive/gui-react-legacy/`），
  保留可回溯性；真正删除留到下一次发布后。
- **NG4**：不改 `webui` 的任何行为。本次不产生任何用户可见变化。

## 5. 决策

**保留 `webui/`（Vue），归档移除 `gui/src`（React），`gui` 降级为纯 Tauri 壳。**

### 5.1 为什么是删 React 而不是删 Vue（五条判据）

1. **可达性**：webui 是唯一被加载的界面（`tauri.conf.json` 硬证据）；React 不可达。
   收敛方向只能是"保留生效的、移除不生效的"。
2. **成熟度**：3509 vs 70 测试，40 vs 20 视图，6 语言 i18n vs 无，10 项架构守卫 vs 1 项。
3. **能力重叠 100%**：逐面板对照 20/20 全部在 webui 有对应视图
   （含 `ModlensPanel` → `webui/src/composables/useModlens.ts` + `modlens.analyze` RPC）。
   **删它不损失任何能力**——这是删除的充分条件。
4. **维护成本**：两套导航分类法已经在本会话造成一次事故
   （我把 gui 拆成 5 层，与 webui 的三层契约分叉，被迫回滚）。这类事故会持续发生。
5. **竞品一致**：5/5 单 renderer。双框架不是"更灵活"，是"两倍表面积"。

### 5.2 反面理由与代价（诚实记录）

- **损失**：`gui/src` 里 20 个面板的 React 实现（5597 行 + 70 测试）不再可用。
  若将来要迁回 React，只能重写。**判断：值得**——因为它们在两年内一次也没被用户看到过。
  归档保留，随时可取。
- **损失**：本会话上一批刚写的 `AutomationPanel.tsx` + `automation.test.tsx` 一并归档。
  其能力（定时任务）在 webui 由 `CronView.vue` + `cron.*` RPC 提供，且**真正可被用户使用**。
- **风险**：`scripts/verify.sh --gui` 若仍期待 `gui/src`，删后会红。**必须同步修改**，
  否则就是"半修"（删了源码留了期待它的门禁）。

## 6. 验收口径（可命令验证）

| 编号 | 判据 |
|---|---|
| AC1 | `gui/src` 不存在；`_archive/gui-react-legacy/` 含全部原文件 |
| AC2 | `gui/package.json` 不再有引用 `src` 的 build/typecheck/test 脚本 |
| AC3 | `gui/src-tauri` `cargo check` 干净（0 error） |
| AC4 | `webui` `npm run typecheck` exit 0；`npm run test:unit` 全绿 |
| AC5 | `scripts/verify.sh --gui` 不再因缺少 `gui/src` 而失败 |
| AC6 | 无残留引用：全仓 `grep` 不到指向 `gui/src` 的活代码路径 |
| AC7 | `AGENTS.md` §1 的"待接线或移除"更新为"已移除（归档）" |

## 7. 影响面

- 消费方：`scripts/verify.sh`（--gui 层）、`scripts/package_app.cmd`（打包）、`AGENTS.md`（文档）、
  `gui/package.json` / `gui/vite.config.ts` / `gui/tsconfig.json` / `gui/index.html`（构建链）
- **不受影响**：`webui/`、`core/`、`gui/src-tauri/`（Rust）、安装包内容

## 8. 回滚

`git mv` 归档 → 回滚为 `git mv _archive/gui-react-legacy gui/src` + 还原
`gui/package.json`、`vite.config.ts`、`tsconfig.json`、`index.html`、`scripts/verify.sh`。
归档期间文件完整保留，无信息损失。
