# AGENTS.md — Marguerite 仓库指南（供 AI 智能体与贡献者）

> Marguerite 是终端优先的智能体工作台：单 Rust 引擎 + 四入口（CLI / ratatui TUI / **Tauri 桌面端** / **Vue3 Web 控制台**）。
> CLI 与 TUI 各自是独立 bin；桌面端是加载网关地址的 Tauri 壳；Web 端仅由回环网关（127.0.0.1）托管，**不开放公网**。
> Python 引擎（OpenSquilla 系）以侧车方式共存，能力经统一网关触达。

## 0. 先读这一段

> **【2026-10-29 · 文档瘦身】** 本文件原为 **159 KB** 单文件，超过智能体指令预算后在注入时会被**截断**，
> 而「完成前验证铁律」恰在 §8 —— 等于把最要紧的规矩放在会被切掉的位置。现已拆为「瘦入口 + `docs/agents/` 分册」：
> 入口只保留铁律、仓库结构与验证章节；其余按 § 分册，**内容逐字节搬运、无删改**（拆分脚本自带行覆盖校验）。
> 按需 `Read` 对应分册即可，不要再往本文件里堆细节。

### 三条铁律（先记这三条，其余按需检索）

1. **无新鲜验证证据，不得声称完成。** `bash scripts/verify.sh --clean --all` 全绿（每条 `[PASS]` + exit 0）才算数；
   `[SKIP]` **永远不算通过**。详见 §8（已保留在本文件内）。
2. **动手前先看 §2（构建/测试）与 §4（已知坑）。** §4 里每一条都是踩过的坑：GBK 乱码伪装成合法 UTF-8、
   作者本机路径随代码发布、`sk-` 既是密钥前缀也是本仓 CSS 类名前缀……
3. **工作区长期有未提交改动（用户明确要求不 commit）** → `git checkout/restore/stash` 一律禁止；
   撤销只能用编辑法或 `git show HEAD:<path>` 手工比对。

### 分册导航（原 §N ↔ 文件）

| 原章节 | 内容 | 位置 |
|---|---|---|
| §0 | 0. 先读这一段（本文件已超长，尾部可能被截断） | 本文件内（下方） |
| §1 | 1. 仓库结构（权威） | 本文件内（下方） |
| §8 | 8. 完成前验证铁律（Verification Before Completion） | 本文件内（下方） |
| §2 | 构建与测试（权威命令） | [`docs/agents/02-build-and-test.md`](docs/agents/02-build-and-test.md) |
| §3 | 架构不变量（违反即缺陷） | [`docs/agents/03-invariants.md`](docs/agents/03-invariants.md) |
| §4 | 已知坑（gotchas） | [`docs/agents/04-gotchas.md`](docs/agents/04-gotchas.md) |
| §5 | 贡献 workflow | [`docs/agents/05-contributing-workflow.md`](docs/agents/05-contributing-workflow.md) |
| §6 | 文档/代码 CLI 漂移校验（CI 门禁） | [`docs/agents/06-doc-cli-drift.md`](docs/agents/06-doc-cli-drift.md) |
| §7 | 第三方集成层（third_party/） | [`docs/agents/07-third-party.md`](docs/agents/07-third-party.md) |

> 维护约定：**新增「坑」写进 §4 分册**，新增门禁/命令写进 §2 分册，架构不变量写进 §3 分册；
> 入口只改导航与铁律。任何一条分册不应超过 32 KiB（超了继续按 `###` 再拆）。

## 0. 先读这一段（本文件已超长，尾部可能被截断）

> ⚠️ 本文件约 **160 KB**（2026-10-29 实测；精确字节数写不得 —— 这句话自己就有 30 字节，改它必然让下一个数字过期），超过智能体指令预算（65536 B）时**尾部会被截断** —— 而铁律恰好在 §8。
> 所以把最要紧的三件事提到最前，其余细节按需检索：

1. **无新鲜验证证据，不得声称完成。** `bash scripts/verify.sh --clean --all` 全绿（每条 `[PASS]` + exit 0）才算数；
   `[SKIP]` **永远不算通过**。见 §8（若被截断，按此句执行）。
2. **旧的「头号发布阻塞」已解除**（2026-10-28 复测）：`git ls-files LICENSE .github` = **8**（含**三个** workflow：
   ci / pages / release）、`git ls-files '*.rs'` = **554**、HEAD == `origin/master` —— clone 拿得到 LICENSE、CI 与全部 Rust 源文件。
   现在唯一挡在「可发布」前面的不是提交树，而是 §8 那次新鲜的全绿门禁。
3. **动手前先看 §2（权威命令与基线）与 §4（已知坑）**：那里每一条都是踩过的坑，含「GBK 乱码伪装成合法 UTF-8」、
   「作者本机路径会随代码发布」、「`sk-` 既是密钥前缀也是本仓 CSS 类名前缀」等。


## 1. 仓库结构（权威）

- `Cargo.toml`：`[workspace] members = ["core","gui/src-tauri","tui"]`，`resolver = "2"`。
- `core/`：核心引擎 + CLI（`marguerite-core` bin）+ TUI 引擎（`marguerite-tui` 由 `tui/` 构建）+ ACP 薄壳（`serve/acp/`）。
- `tui/`：ratatui 终端工作台 bin。
- `gui/`：Tauri 2 桌面壳，**唯一原生桌面入口**（bin: `marguerite-gui`，Rust 壳在 `gui/src-tauri`）。
  ⚠️ **主窗口加载的是网关地址 `http://localhost:18790/control/`**（即 `webui/` 的 Vue 控制台）。
  **`gui/src` 的 React 前端已于 2026-10-21 归档移除**（`docs/legacy/gui-react-2026-10-21/`，含 PRD
  `docs/prd/2026-10-21-gui-react-retire.md`）：它从未进入安装包（`bundle.resources` 不含 `gui/dist`），
  20 个面板与 webui 的 40 个视图 100% 能力重叠，留着只会带来两套 token 纪律 / 两套测试 /
  两套导航分类法。**前端只在 `webui/`（Vue）一处维护。**
  `gui/` 现在是**纯壳**：`package.json` 只剩 `tauri` / `shell:check` / `shell:clippy` /
  `verify:no-frontend`（守卫：`gui/src` 不得重现）。发布桌面端时 `scripts/package_app.cmd`
  只复制 `webui/dist`。原 egui 桌面 `desktop/` 已删除。
- `webui/`：Vue3 控制台（Vite + Pinia + vue-i18n，原 opensquilla-webui 全量迁入并品牌化为 Marguerite），构建产物出 `webui/dist`，由统一网关回环托管。
- `pyengine/`：Python 引擎侧车（OpenSquilla Python 微内核迁入，含 migrations/tests），经网关代理方法组，UI 无需感知双引擎。
- `workbench/`：**独立子项目**（R28 实测补录）—— 文件智能 + LLM 实验台 + 智能体循环：
  Python 后端 `backend/marguerite_workbench/`（engine · api · cli · llmlab · ttc · agentloop，**71 个离线用例**）、
  由 FastAPI 在 `:8011` 同源托管的 React SPA `frontend/`、打包流水线 `packaging/`（`package_workbench.py` /
  `check_version_sync.py`）、产物 `dist/`。
  ⚠️ **三条容易误判的事实**：① 它**不是** Rust 引擎的第五个前端 —— `marguerite_workbench` 在 core/tui/webui/gui 里
  **0 引用**，它经 `core_bridge` 桥接核心 store，思路同 `pyengine/`；② 它有**自己的版本线**
  （`workbench/VERSION`，实测 `1.1.0`），与引擎的 `0.2.17` 相互独立，`check-version-sync.py` 那 9 处**不含它**；
  ③ 它**会随提交发布**（`git ls-files -co` 里 76 个文件），所以在 R28 之前「无门禁跑过它的 71 个用例」
  属于真实缺口，现已由 `verify.sh --workbench` 覆盖并纳入 `--all`。
- `docs/`：文档（`cli-reference.md` 为 CLI 权威手册；`mcp.md` / `development.md` 为技术指南）。
- `artifacts/`：**内部工作记录**（发布就绪状态与交接 `RELEASE-READINESS-STATUS.md` 等）。
  ⚠️ 它被 `.gitignore:127` 的 **`/artifacts/` 整目录忽略**（实测 `git ls-files artifacts` = **0**），
  **不随仓库发布**，`git add -A` 也带不进去 —— 所以**绝不要在被发布的文档（README / CONTRIBUTING / `docs/*.md`）里引用它**，
  否则就是一个新人永远点不开的死链（实测当前 0 处引用，保持这样）。它是给本机会话用的，不是给 clone 用的。


## 8. 完成前验证铁律（Verification Before Completion）

> 源自 superpowers `verification-before-completion` 技能。本项目曾出现「伪完成」：调用了搜索/构建工具即声称任务完成，却未校验结果含真实内容、未校验产物落盘（例如声称产出 HTML/PPT，实则只调了工具、停在输入框）。本仓库因此固化以下**强制门禁**，任何完成声明都须先过此闸。

**铁律（Iron Law）**
```
无新鲜验证证据，不得声称完成 / 通过 / 已修复 / 已构建。
NO COMPLETION CLAIMS WITHOUT FRESH VERIFICATION EVIDENCE
```

**任何「完成 / 通过 / 已修复 / 已构建」声明之前，必须：**
1. 运行统一闸门 `bash scripts/verify.sh --clean --all`（发版加 `--release`），取得「全绿」证据：每条 `[PASS]` + 最终退出码 `0`。任一 `[FAIL]` 即禁止声称完成。
2. 若产出文件（HTML / PPT / 报告 / 构建产物等），先用 `bash scripts/assert-deliverables.sh <路径...>` 确认文件存在且非空，再 `present_files` 交付。
3. **破除增量缓存遮蔽**：权威完成验证须带 `--clean`（先 `cargo clean -p marguerite-core` 再全测）。增量编译缓存会掩盖库级编译错误，使「上次绿」误导本次声明——不得用「之前跑过」替代本次全测。

**禁止用语（出现即视为未验证）**
- 「应该没问题 / 大概可以 / 看起来对了 / 似乎通过了」
- 在任何验证输出之前说「Great! / Perfect! / 完成了！ / Done!」
- 仅因「linter 过了」就声称「构建通过」（linter ≠ 编译器）
- 仅因「agent 报告成功」就声称「任务完成」（须独立核验 VCS diff 与产物落盘）

**闸门分层（`scripts/verify.sh`）**
| 模式 | 范围 | 用途 |
|---|---|---|
| `--fast` | core 测试 + clippy(workspace) + webui typecheck | git pre-commit 快闸 |
| `--core` | core 测试 + core bin 构建 + clippy(core+gui) + 文档漂移 | Rust 栈隔离 |
| `--webui` | webui typecheck + webui 单测 + **lockfile 自洽** + **依赖审计** | 前端栈隔离 |
| `--gui` | gui typecheck + lint + 单测 + 构建 + **lockfile 自洽** | 桌面壳 React 栈隔离 |
| `--pyengine` | Python 侧车 `tests/unit`（按 marker 排除网络/LLM/浏览器/真实终端） | Python 栈隔离 |
| `--all`（默认） | `--core` + `--webui` + `--gui` + `--pyengine` + `--workbench` + `cargo build --workspace` + clippy(workspace) = **26 项检查** | 日常全量 |
| `--clean --all` | 上述 + 先 clean core | **完成声明用此档** |
| `--release` | `--all` + `cargo tauri build` + 资源卫生（staged/物化两遍）+ **侧车启动冒烟**（`check-sidecar-boot.py`：真启动打包侧车，打 `/health`、`/api/config`、`/ws`） | 发版门禁 |

**跳过 ≠ 通过**：`verify.sh` 把跳过的检查记进独立的 `SKIPPED` 列表并在汇总里显式列为「未验证，不计入通过」。因工具缺失而静默变绿是本仓库明令禁止的伪完成，所以 `[SKIP]` 永远不会被计成 `[PASS]`。

**clippy 范围说明**：`--all` / `--fast` 用 `cargo clippy --workspace --all-targets -- -D warnings`（L3 零警告，含 tui）；`--core` 单独运行则用 `-p marguerite-core -p marguerite-gui` 以加速。若 `--workspace` clippy 因 tui 遗留警告变红，那是真实待办，须修复而非绕过；临时排错可用 `--core`。

**证据格式**：闸门输出含每条 `[PASS]/[FAIL]`、耗时、以及测试计数 / 退出码；声称完成时必须附该次运行输出或明确引用本次运行（与 §6 文档漂移门禁同理，把「对账」固化成可重复机械闸门）。

