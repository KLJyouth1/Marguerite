# AGENTS.md — Marguerite 仓库指南（供 AI 智能体与贡献者）

> Marguerite 是终端优先的智能体工作台：单 Rust 引擎 + 四入口（CLI / ratatui TUI / **Tauri 桌面端** / **Vue3 Web 控制台**）。
> CLI 与 TUI 各自是独立 bin；桌面端是加载网关地址的 Tauri 壳；Web 端仅由回环网关（127.0.0.1）托管，**不开放公网**。
> Python 引擎（OpenSquilla 系）以侧车方式共存，能力经统一网关触达。

## 0. 先读这一段（本文件已超长，尾部可能被截断）

> ⚠️ 本文件约 **66 KB**，超过智能体指令预算（65536 B）时**尾部会被截断** —— 而铁律恰好在 §8。
> 所以把最要紧的三件事提到最前，其余细节按需检索：

1. **无新鲜验证证据，不得声称完成。** `bash scripts/verify.sh --clean --all` 全绿（每条 `[PASS]` + exit 0）才算数；
   `[SKIP]` **永远不算通过**。见 §8（若被截断，按此句执行）。
2. **当前头号发布阻塞**：成果只在工作区，仓库里没有。`git ls-files LICENSE .github` = **0** → clone 拿不到
   LICENSE、CI、以及 **131 个 Rust 源文件**。解除只有 `git add -A`（暂存≠提交，`git reset` 可退）。
3. **动手前先看 §2（权威命令与基线）与 §4（已知坑）**：那里每一条都是踩过的坑，含「GBK 乱码伪装成合法 UTF-8」、
   「作者本机路径会随代码发布」、「`sk-` 既是密钥前缀也是本仓 CSS 类名前缀」等。

## 1. 仓库结构（权威）

- `Cargo.toml`：`[workspace] members = ["core","gui/src-tauri","tui"]`，`resolver = "2"`。
- `core/`：核心引擎 + CLI（`marguerite-core` bin）+ TUI 引擎（`marguerite-tui` 由 `tui/` 构建）+ ACP 薄壳（`serve/acp/`）。
- `tui/`：ratatui 终端工作台 bin。
- `gui/`：Tauri 2 桌面壳，**唯一原生桌面入口**（bin: `marguerite-gui`，Rust 壳在 `gui/src-tauri`）。
  ⚠️ **主窗口加载的是网关地址 `http://localhost:18790/control/`**（即 `webui/` 的 Vue 控制台），
  不是 `gui/src` 下的 React 应用——该 React 前端会被 CI 构建（`verify.sh --gui`）但**在产品界面中不可达**，
  待接线或移除。发布桌面端时 `scripts/package_app.cmd` 只复制 `webui/dist`。原 egui 桌面 `desktop/` 已删除。
- `webui/`：Vue3 控制台（Vite + Pinia + vue-i18n，原 opensquilla-webui 全量迁入并品牌化为 Marguerite），构建产物出 `webui/dist`，由统一网关回环托管。
- `pyengine/`：Python 引擎侧车（OpenSquilla Python 微内核迁入，含 migrations/tests），经网关代理方法组，UI 无需感知双引擎。
- `workbench/`：**独立子项目**（R28 实测补录）—— 文件智能 + LLM 实验台 + 智能体循环：
  Python 后端 `backend/marguerite_workbench/`（engine · api · cli · llmlab · ttc · agentloop，**71 个离线用例**）、
  由 FastAPI 在 `:8011` 同源托管的 React SPA `frontend/`、打包流水线 `packaging/`（`package_workbench.py` /
  `check_version_sync.py`）、产物 `dist/`。
  ⚠️ **三条容易误判的事实**：① 它**不是** Rust 引擎的第五个前端 —— `marguerite_workbench` 在 core/tui/webui/gui 里
  **0 引用**，它经 `core_bridge` 桥接核心 store，思路同 `pyengine/`；② 它有**自己的版本线**
  （`workbench/VERSION`，实测 `1.1.0`），与引擎的 `0.2.7` 相互独立，`check-version-sync.py` 那 9 处**不含它**；
  ③ 它**会随提交发布**（`git ls-files -co` 里 76 个文件），所以在 R28 之前「无门禁跑过它的 71 个用例」
  属于真实缺口，现已由 `verify.sh --workbench` 覆盖并纳入 `--all`。
- `docs/`：文档（`cli-reference.md` 为 CLI 权威手册；`mcp.md` / `development.md` 为技术指南）。
- `artifacts/`：**内部工作记录**（发布就绪状态与交接 `RELEASE-READINESS-STATUS.md` 等）。
  ⚠️ 它被 `.gitignore:127` 的 **`/artifacts/` 整目录忽略**（实测 `git ls-files artifacts` = **0**），
  **不随仓库发布**，`git add -A` 也带不进去 —— 所以**绝不要在被发布的文档（README / CONTRIBUTING / `docs/*.md`）里引用它**，
  否则就是一个新人永远点不开的死链（实测当前 0 处引用，保持这样）。它是给本机会话用的，不是给 clone 用的。

## 2. 构建与测试（权威命令）

```bash
cargo build -p marguerite-core --bin marguerite-core   # 构建 CLI 二进制
cargo test -p marguerite-core --lib                     # 单元测试
cargo build --workspace                                 # L1 构建门禁
cargo clippy --workspace --all-targets                  # L3 零 warning
python3 scripts/check-doc-cli-drift.py                 # 文档/代码 CLI 漂移门禁
```

Web UI 门禁（`webui/` 下执行）：

```bash
npm run check:architecture   # 10 项守卫（colors/motion/radius/theme-contract/…）
npm run typecheck            # check:architecture + vue-tsc --noEmit
npm run test:unit            # Vitest 单测（当前 3509 passed / 0 failed / 331 files）
npm run build                # typecheck + vite build + 产物校验（dist/）
```

> **统一闸门（推荐）**：上述命令已收口为 `bash scripts/verify.sh`（分层：`--core` / `--webui` / `--gui` / `--pyengine` / `--workbench` / `--all` / `--release` / `--fast`）。任何「完成」声明前运行 `bash scripts/verify.sh --clean --all`（发版加 `--release`），全绿方可声称。详见 §8。

- **测试基线（2026-09-25 R61 后）**：`marguerite-core` lib **2704 passed + 0 failed + 17 ignored**（⚠️ 此前写 2538 已过期；R44–R61 累计 +166）；`webui` **3509 passed + 0 failed（331 files，R59 复测；此前文档里的 3216/311 已过期两轮）**；`pyengine` 三层 —— `tests/unit` **768 passed / 0 failed / 6 skipped**（R61 复测）、`tests/test_gateway` **2929 passed / 0 failed / 11 skipped**（R58 新增该层门禁）、**契约族** `tests/test_{ci,migrations,sandbox,onboarding,tools,recovery}` **4442 passed / 0 failed / 390 skipped**（R60 新增，拆开为 35+1s / 59 / 1746+349s / 956+8s / 1202+23s / 444+9s；R61 复测一致）。任何改动后不得回退；若基线变化，同步更新本文件 §2 与 `README.md`／`README.zh-Hans.md` 的「Quality gates」表（`scripts/check-readme-claims.py` 会比对中英两份的硬事实与闸门总数，但**不会**核对数字是否与现实一致 —— 所以「过期基线」只能靠人复测）。门禁运行前若 shell 带 `NO_COLOR=1` 需先移除（否则 2 个既有主题测试走单色回退失败，非代码缺陷）。
- **Python 侧车（`pyengine/`）的门禁与两条必修配置**：`verify.sh --pyengine` 跑三层 —— `tests/unit`（按 `pyproject.toml` 的 marker 排除需要真实外部世界的用例：网络 / 模型 API / Playwright / 真实终端）、`tests/test_gateway`（无此类 marker，整层全跑）与**契约族**（`tests/test_ci` / `test_migrations` / `test_sandbox` / `test_onboarding` / `test_tools` / `test_recovery`，R60 新增，见本节 R60 条目）。`tests/unit` 这一层有两个**载荷性**配置，删掉任何一个都会让整套 Python 测试看起来「大面积损坏」：
  1. `pyproject.toml` 的 `[tool.pytest.ini_options] pythonpath = ["src"]` —— 包是 src-layout（`src/opensquilla`），没有这一行时从源码树裸跑 `pytest` **根本 import 不到 `opensquilla`**。实测差值：**1553 tests collected / 1146 errors → 20539 / 22 errors**。
  2. `tests/__init__.py` —— 多处用例跨文件 import（`from tests.unit...mport FakeOpenTuiBridge`）。默认 `prepend` 导入模式只把用例自身目录塞进 `sys.path`，`tests.*` 不是可导入包 → `ModuleNotFoundError: No module named 'tests.<x>'`。实测：**20539/22 → 20588/21**，并让一个硬失败转绿。
  ⚠️ 两者都「看起来可以删」——**它们是载荷性的，不要清理**。
- **45 个用例模块被显式拒绝收集 —— 是跳过，不是静默忽略**（R22 建立，R24 扩展为两类，R60/R62 补齐剩余）。`pyengine/tests/` 里有模块**要么 import 不进来、要么跑起来必红**，根因是 `pyengine/` 从上游项目整体吸收过来时**只带了测试、没带被测的脚手架**。`pyengine/tests/conftest.py` 用两张表把它们全部具名列出（`_SCAFFOLDING_GROUPS` 41 个 + `_INHERITED_GROUPS` 4 个），终端汇总会打印「缺什么」，并明确标注 `SKIPPED (not verified)`（呼应 §8「跳过 ≠ 通过」）。
  - **`absent` 类（41 个，自愈）**：前置确实不在仓库里 —— `pyengine/scripts/*` 与 `pyengine/.github/*` 两棵子树整体不存在（R60 又补了 12 个 `test_scripts` 模块 + 3 个 Electron 桌面模块 + 1 个 TUI lab 模块 + 4 个根级模块；R62 再补 1 个 meta-skill 文档模块）+ 3 个**声明在 `[dependency-groups]`/`[optional-dependencies]` 里但没装**的开发依赖（`syrupy` / `laboratory` / `mypy`）。实测 `git ls-files pyengine/scripts pyengine/.github` = **0**，两个目录**磁盘上也不存在**。前置一出现即自动放行，**不需要改 conftest**。
  - ⚠️ R60/R62 的取舍标准：**只登记「整个模块被同一个缺失前置 100% 挡死」的**（逐模块实测 `pytest <module> --tb=no`）。混合模块（部分用例通过、部分读缺失上游文件，如 `test_packaging/*`、`test_public_release_hygiene.py`、`test_skills/test_high_value_meta_skill_contracts.py`、`test_skills/test_managed_toolchains.py`）**刻意不登记** —— 忽略它们会把真实绿灯一起丢掉，也会把「有真失败」悄悄标成「不可运行」。它们保持可见，已记入项目记忆的「已知缺陷」。
  - **`inherited` 类（4 个，不自愈）**：它们断言的是**另一个仓库**的 CI/发布契约 —— `test_ci/test_workflows.py`（84 个失败全在它一个文件里，因为它按 CWD 读 `.github/workflows/ci.yml`，即 `pyengine/.github/...`）、`test_pr_body_lint.py`（要 `.github/scripts/validate_pr_body.py`）、`test_router_artifact_manifest.py`（要 `scripts/update_router_artifact_manifest.py`）、`test_bgm_asset_hygiene.py`（断言上游控制台目录 `opensquilla-webui/`，本仓是 `webui/`）。⚠️ **把路径补回去也不够**：断言里点名的是本仓**故意没有**的 job 与产物，所以它们**不会**自动变绿，汇总里逐条标注 `inherited, will NOT self-heal` —— 列出来是为了不让 93 行红线冒充「产品坏了」，**不是**为了让套件看起来全绿。
  - **实测（R24，`PYTHONUTF8=1`，排除 Windows locale 干扰）**：`pytest tests/test_ci` 由 **94 failed / 41 passed** 变为 **0 failed / 35 passed / 1 skipped**。顺带发现那 6 个 `test_migration_v0*` 失败是**顺序污染**（单跑全过），而污染源正是被跳过的那些文件 —— 跳掉后一并消失。
  - ⚠️ **但「收集干净」≠「套件全绿」**：R58 之前 `tests/unit` 之外**没有任何门禁执行**；R58 接进 `tests/test_gateway`，R60 再接进契约族六个目录（`test_ci` / `test_migrations` / `test_sandbox` / `test_onboarding` / `test_tools` / `test_recovery`），三层之外的目录（`test_scripts` / `test_desktop` / `test_packaging` / `test_provider` / `test_skills` …）**仍然无人跑**。跑全树 `pytest tests` 实测：**22 分钟墙钟只消耗 8% CPU**（1322s wall / 107s CPU）—— 即**绝大多数时间阻塞在外部 I/O**，这正是门禁刻意把自己限定在能跑完的层的原因。**未进门禁的那些目录的执行结果未测（不可测），不要声称它们绿。**
  **守卫**：`python scripts/check-pyengine-collection.py`（纯标准库；`0` = 收集干净、`1` = 有 collection error 并逐条打印原因、`2` = 无法验证 —— 绝不冒充通过）。
  ⚠️ 它**故意没有**接进 `verify.sh`：整套 `--collect-only` 要 import **每一个**测试模块，而门禁刻意把自己限定在能确定性跑完的那两层（不碰网络 / 模型 API / 浏览器 / 真实终端）。
  三条实测：
  1. **修复是正向的**：`tests/test_tools/{test_dispatch_legacy_coverage,test_dispatch_properties}.py` 原写 `from test_tools.dispatch_corpus import ...`，但 `tests/test_tools/` 没有 `__init__.py` → `test_tools` 不是可导入顶层名（**`dispatch_corpus/__init__.py` 本身存在且完好**，缺的只是导入路径）。按本仓既有惯例改成 `from tests.test_tools.dispatch_corpus import ...`（同款 `from tests.<子包>` 写法仓内已有 6 处在用）后 **80 passed**；裸 `pytest` 收集数 20588 → **20668**、错误 21 → **0**。
  2. **表是自愈的，不是永久忽略名单**：临时造桩 `pyengine/scripts/live_harness_security.py` → 跳过数 **19 → 16**，那 3 个模块**重新被尝试收集**（因桩不完整而报错）；删桩后**回到 19**。
  3. **报告按「本次收集范围」收窄（首版这里是错的，已修）**：pytest 下降到参数目录时会**评估其父目录 `tests/` 的顶层条目**，于是 `pytest tests/unit` 也会把这 8 个直接躺在 `tests/` 下的模块交给 hook（实测：**换成单个文件参数同样触发**，且与 `testpaths` 无关 —— `-o testpaths=` 无效）。首版因此把 **8 行无关跳过**打进了 `--pyengine` 的汇总（**门禁覆盖未受影响**：仍是 `758 passed, 6 skipped`，与基线一致，因为那 8 个本来就不在收集范围内）。已加 `_in_requested_scope()`：**依然 `return True`（该忽略就忽略），但只把落在本次参数路径内的模块计入汇总**。四档实测：`tests/unit` → **0** 行、`tests/unit/test_env.py` → **0** 行、`tests` → **19** 行、裸 `pytest` → **19** 行。
  ⚠️ **已知边界（pytest 设计使然，本机制无法覆盖）**：**命令行里直接点名的路径是 initial path**，pytest 对它们**不调用** `pytest_ignore_collect`（`Session._collectfile` 里 `isinitpath` 判定）。因此 `pytest tests/test_live_mixed_provider_gateway.py` 仍会以 import error 结束（已实测钉住）。对门禁无影响（门禁传的是目录 `tests/unit`），且此时用户本就是要跑那一个模块 —— 缺前置正是他该看到的错误。
  ⚠️ 所以**不要**把这 45 个模块当「可删的死测试」清理：`absent` 类那批是工具链的规格说明（工具链一旦回迁，前置出现即自动放行）；`inherited` 类那 4 个则是**继承来的、本仓不该有的契约** —— 若将来决定清理，删的是它们，而不是改 conftest 去假装通过。
  ⚠️ 另一个坑：`pytest_ignore_collect` 是 `@hookspec(firstresult=True)`，**只能返回 `True` 或 `None`** —— 返回 `False` 会强行否决其它插件的忽略决定。
- **⚠️ `pyengine/` 在本仓库里装不上、也构建不出 wheel/sdist**（R24 实测）。它的构建后端**故意 fail-closed**：`hatch_build.py` 的 `initialize()` 要求 `src/opensquilla/gateway/static/dist` 存在**且通过** `verify_dist()` 校验，否则直接 `raise RuntimeError`；sdist 分支还会去读 `opensquilla-webui/` —— 那是**上游**控制台目录，本仓是 `webui/`，迁移时根本没带过来。实测三连：该 dist 目录**不存在**、`pyengine/opensquilla-webui/` **不存在**、`pip show opensquilla` **为空**。**所以不要写 `pip install -e pyengine`**（本地与 CI 都会失败）。
  ✅ **测试并不需要安装这个包**：`[tool.pytest.ini_options] pythonpath = ["src"]` 已足够 —— 这正是「本机没装包、`tests/unit` 却 758/758 通过」的原因。CI 的 pyengine job 因此改成**先从 `pyproject.toml` 读出依赖集（`dependencies` + `optional-dependencies.dev`，实测 45 项）再 `pip install`**，完全不碰构建后端。
- **CI 现在有第 5 个 job：`pyengine`**（R24 新增）。此前 `ci.yml` 只有 `docs`/`rust`/`webui`/`gui` —— **全是 Rust 或 Node**，于是侧车那 758 个测试**在 CI 里一个都不跑**，而 README 却在宣传 `--pyengine` 门禁：一个对 1/3 代码库沉默的绿徽章。
  该 job 调用的就是 `bash scripts/verify.sh --pyengine`（**刻意不复制 pytest 命令行**，marker 排除表因此只有一处定义、不会漂移；R58 起该层同时还跑 `tests/test_gateway`，见上文 R58 条目），并在跑之前先断言 `pytest` 可导入、`tests/unit` 存在 —— 因为 `verify.sh` 对缺前置只记 `[SKIP]` 且**仍然 exit 0**（§8「跳过 ≠ 通过」）。它还会跑一次 `scripts/check-pyengine-collection.py`（约 10s，0 collection error）。
  ⚠️ **本轮只做了静态验证**（诚实上限）：YAML 可解析、共 5 个 job、每个 `run:` 脚本过 `bash -n` 零语法错、heredoc 体过 `ast.parse`、依赖集解析出 45 项、`--pyengine` 当时只调度 `check_pyengine_tests`（R58 起为两项）。**GitHub Actions 无法在本机运行**，所以「这个 job 真的会绿」尚未被证实 —— 首次 push 时它是第一个该盯的地方（Python 固定在声明的下限 3.12）。
- **`path.read_text()` 不写 encoding 在本机必然炸**：Python 的 `read_text()`/`write_text()` 缺省用 **locale 编码**，本机中文 Windows 是 **GBK（cp936）**，读 UTF-8 源码文件即 `UnicodeDecodeError: 'gbk' codec can't decode byte 0x94`。实测 `pyengine/tests/unit` 里 17 个失败中有 **14 个**是这一个根因（`test_turn_stream_boundaries.py` 用 `ast.parse(path.read_text())` 读 `chat_cmd.py`）。已给门禁范围内的 12 处补上 `encoding="utf-8"`。
  ⚠️ 全仓仍有 **395 处**裸 `read_text()` 分布在 **81 个** `pyengine/tests/**` 文件里（**产品代码 `src/` 为 0**，故纯属测试脆弱性）。在 UTF-8 locale 的 Linux/CI 上它们不会暴露，属于「Windows 开发机上红、CI 上绿」的隐性缺陷。**不要用脚本一次性全改**：仓内有 **32 处**用例是**故意**指定 `gbk`/`latin-1`/`cp1252` 等编码的，无差别替换会破坏它们。
- **fresh clone 的构建顺序**：`webui/dist` 是被 `.gitignore` 的构建产物，却由 `gui/src-tauri` 的 `bundle.resources` 声明为 Tauri 资源；Tauri 2 在资源路径缺失时**直接中止 build script**（`cargo build --workspace` → exit 101 `resource path 'webui\dist' doesn't exist`）。任何会编译 `marguerite-gui` 的检查（clippy `--all-targets`、workspace 构建、`cargo tauri build`）都依赖它。`scripts/verify.sh` 已内置前置（`ensure_webui_dist`）自动补建；手工操作时先 `cd webui && npm run build`。
- **版本号单一事实源**：`core/Cargo.toml` 的 `version`。其余 8 处（tui / gui / tauri.conf / webui / gui 的 package.json，以及**两个 npm lockfile 各自的两处**：顶层 `version` 与 `packages[""].version`）由 `python scripts/check-version-sync.py` 强制对齐，已接入 `verify.sh`。改版本时用 `--fix`，不要手改单点。
  ⚠️ npm lockfile 里的版本号出现**两次**，只改正则会留下 `packages[""]` 漂移（实测残留过 `webui 0.2.1` / `gui 0.1.5`），而 `npm ci` 会校验这份一致性 —— 门禁现已覆盖这两处。
- **构建环境坑**：后台 shell 设置 `CARGO_TARGET_DIR` 可能不被采纳，二进制会落到默认 `target/debug/`；冒烟测试前请用默认 target 重新 `cargo build`。D 盘空间不足会导致 `ENOSPC` 写入失败，注意 target / 临时目录位置。
- **并发跑 cargo 会互相污染**：多个 `cargo test` 同时跑会争 `target/` 锁（实测 `LNK1104: 无法打开文件 ...marguerite_core-<hash>.exe`），且历史上有测试因共享临时 DB 而假失败（已修 `core/src/events/product.rs`）。判断「是不是并发导致的」之前，先确认没有其它 cargo 在跑。
  ⚠️ **同理，两份 `verify.sh --all` 不能同时跑**（同一工作区被两个会话/两个终端共用时）：`check_webui_e2e` 默认
  固定占用 `18877`（可用 `WEBUI_E2E_PORT` 覆盖），且收尾时按端口 `taskkill /T` —— 后跑的那个会把先跑的隔离网关杀掉。
  实测（2026-09-25）：并发时先跑的那次拿到「**92 passed < 基线 95**」，而同一份代码在干净环境下是
  **95 passed / 2 skipped**（那 2 例是 `approval-card` 的 live 用例，`OPENSQUILLA_E2E_LIVE` 未开时会跳过）；
  并发那次多出的 3 例是**被 skip**（本机只在拿不到 WebGL / 无日志夹具时才 skip）——「跳过」与「通过」都算进 97 例总数，
  但棘轮只数 passed，于是表现为「通过数掉了」而不是「失败了」。
  同时两个会话还会争抢 `target/debug/marguerite-core.exe`（daemon / e2e 网关持有它就 `cargo build` 失败）。
  ⇒ 判「红」之前先看**进程表与文件 mtime**，别默认是自己的改动坏了。

## 3. 架构不变量（invariants — 违反即缺陷）

1. **Web UI 仅回环托管**：`webui/`（Vue3 控制台）与统一网关只监听 `127.0.0.1`（默认 18790），**禁止** Tauri 前端与公网监听；三端（TUI/桌面/Web）共享蓝橙语义令牌。
2. **ACP 仅回环**：`serve` 永远只监听 `127.0.0.1`，绝不 `0.0.0.0` / 公网。
3. **离线可运行**：编排与 `agent.invoke` 走 `Orchestrator::analyze`（不调 LLM），无 API Key 降级为离线回显。
4. **内部可变性范式**：跨 `await` 共享状态用 `Arc<Mutex<T>>`；克隆锁再跨 await，避免 `MutexGuard` 非 Send。
5. **治理双闭环**：执行前 `PlanGate`（宪法 C1–C4）、产出后 `CompletionGate`（四检查 + 自报文件落盘核验）。
6. **CLI 输出契约**：结构化错误走 stderr + 退出码；`--json` 输出 `{ok,data,meta}`；尊重 `NO_COLOR`；文本标记用 ASCII `[OK]/[FAIL]`，**禁止 emoji 图标**。
7. **路由三处同步**：`cli/argv.rs` 的 `SUBCOMMANDS`、`program.rs` 的 `route_table.register(...)`、`test_program_*` 的路由计数断言必须一致。当前 **19 个路由**：`doctor` / `model` / `mcp` / `skills` / `session` / `exec` / `deploy` / `serve` / `sandbox` / `subagent` / `team` / `browser` / `chat` / `gateway` / `goal` / `capabilities` / `cache` / `dispatch` / `games`。
8. **缓存三层诚实性（Task #5）**：`MultiTierCache` 的 Tier 0 必须与网关 `gateway_prefix_cache()` 共享同一 `Arc<PrefixCache>`（`Arc::ptr_eq` 为真），确保精确命中由共享方统一计量、零重复计数；Tier 2 语义近似默认关闭（`semantic_enabled=false`），且工具路径（`chat_complete_with_tools_cached`）永远走 `ExactOnly` 策略——近似命中会静默丢弃工具调用，是正确性缺陷；`cache.status` 的 `tiers.hit_rate` 由真实计数相除（夹紧 [0,100]），**绝不冒充 100%**。
9. **WebUI 导航分组契约（Task #6/#7）**：侧栏由 `getWorkNavigationSection()`（平铺核心 rail，仅 group:Work 的 nav:primary 项）与 `getGroupedNavigation()`（可折叠 Work/Operate/Observe/Settings 四组，收纳所有深链能力）共同渲染；新增深链能力必须设 `meta.group`，否则不可见于导航；`/control/control-center`（智能中枢，控制台 base 为 `/control`，故浏览器 URL 需带前缀）为模式/BloomSpec/缓存/能力的统一仪表，数据源仅限既有 RPC（mode.get/switch/spec、cache.status、plans.capabilities），不得新增后端契约。

### 3.1 Bloom 模式与插件系统的设计红线（anti-patterns；策划 §三 + bloom.yaml）

> 以下两条来源于 dsh-forge 的反模式警告，已在 `core/src/mode/modes/bloom.yaml` 的 `anti_patterns` 字段声明，同时由代码侧**强制守卫**；任何绕过守卫的实现都视作缺陷。

8. **红线 #1：禁止全上下文重注入**（dsh-forge："禁止全上下文重注入"）
   - 会话内只允许**增量追加**消息；禁止替换 system 提示、禁止回滚或重置整段历史。
   - 插件侧实现：`plugins/di.rs::PluginLoader::discover` 会扫描 `plugin.json` 是否包含 `system_prompt` / `full_context` 字段，并把插件名加入 `context_leak_suspects()`；上层启动时必须拒绝启用该列表中的插件（或至少显式告警 + require-ack）。
   - 持久注册表 `PluginRegistryFile::entries` 的 schema 中**不得**出现以上字段；新增序列化字段时代码评审要对照本 invariant。

9. **红线 #2：避免中途替换工具面**（dsh-forge："避免中途替换工具面"）
   - 会话内工具/服务集合必须**单调扩展**；Bloom 三段锚定（`core/src/mode/anchoring.rs::AnchorStage::{Anchored→Ptc→Full}`）只加工具不撤回；PTC 阶段不得回退到锚定工具集；不得随机重置已提供的服务。
   - 插件侧实现：`plugins/di.rs::PluginLoader::guarded_provide::<T>` 在 `T` 的 `TypeId` 已出现于 `applied_service_types` 时**返回 Err 并指明红线违规**；只有 `reload_plugin()` 作为显式管理动作会清空该集合并允许替换。
   - 任何直接 `ServiceContainer::provide`（无 guard 包装）替换了同一 TypeId 的代码，在 CI lint 中应被标注为「潜在红线违规」。

## 4. 已知坑（gotchas）

- **🧪 「隔离网关」并没有隔离 —— 改 `HOME`/`USERPROFILE` 在 Windows 上无效**（R52 实测，同时解释了一类红用例）。
  `Config::config_dir()` 用 `dirs::home_dir()`，而它在 Windows 上走 **Known Folder API**，**不认 `USERPROFILE`/`HOME`**
  （`LOCALAPPDATA` 是认的，所以 `AppData/Local` 那半是真的隔离）。实测：以 `USERPROFILE=$tmp HOME=$tmp LOCALAPPDATA=$tmp/AppData/Local`
  起的「隔离网关」，跑完一轮后**真实 `~/.marguerite/config.yaml` 的 mtime 变了**（12:22:12 → 12:23:35），
  而 `$tmp` 下**根本没有 `.marguerite` 目录**。两个后果都是真缺陷：① e2e/冒烟/探针会**写用户的真实配置**
  （`settings-modal.spec` 正好在翻 privacy 开关）；② 隔离网关与常驻 daemon **争抢同一份 config.yaml** —— 就是
  「并发写丢更新」的真实现场：两条并行 `config.patch.safe` **都回 `ok: true`**，而**只落一条**。
  ✅ 已修：`Config::config_dir()` 新增 **`MARGUERITE_CONFIG_DIR`** 覆盖（空串按未设置，否则配置目录会被解析成 cwd，
  配置静默丢失），`verify.sh` 与 `smoke_orchestrate.sh` 的隔离启动都带上它；解析逻辑抽成纯函数
  `resolve_config_dir()` 以便单测（**不要在单测里改全局 env**：Rust 测试并行跑，会串扰其它用例）。
  写路径另加 Rust 侧 `CONFIG_WRITE_LOCK`（`gateway/config.rs`）把「load → 改 → save」整段串行化；
  pyengine 侧**没有锁**（它的读改写窗口内不存在 `await`，asyncio 无从交错 —— 实测 `gather` 双写都落盘），
  但该不变量写进了 `rpc_config.py::_handle_config_patch` 上方注释：**在持久化前插入 await 必须先补串行锁**。
  **判据**：隔离是否真的生效，看**真实配置 mtime 前后是否相等**（不能只看「测试通过」）；并发写则看
  `scripts` 外的一次性探针：两条并行 patch **两边的键都要在 `config.get` 里可见**。
  回归守卫：`test_resolve_config_dir_prefers_env_override`、`test_concurrent_config_patch_keeps_both_writes`
  （Rust；把锁去掉后它**第 0 轮**就报「两边都回 `ok:true` 但只落一条」—— 有牙）、
  `test_concurrent_safe_patches_keep_both_updates`（pyengine；钉住「并发双写都落盘」这个用户可见契约）。
  ⚠️ 诚实边界：写锁是**进程内**的（覆盖了实测到的现场：同一网关的并发请求、以及 e2e 与常驻 daemon 抢同一文件 ——
  后者已靠 `MARGUERITE_CONFIG_DIR` 从根上消除）。**跨进程**同写（例如 TUI 与网关同时改 `~/.marguerite/config.yaml`）
  仍可能互相整份覆盖，文件级锁尚未实现。

- **🛡️ 同一份控制台，两个引擎，只有一边下发安全响应头**（R53 实测）。`/control` 由 **Rust** 网关托管
  （真机 18801），但 CSP / `X-Frame-Options` / `nosniff` / `Referrer-Policy` **只在 pyengine** 的
  `SecurityHeadersMiddleware` 里下发 —— Rust 侧一个都没有。于是 `webui/e2e/preview-origin-csp.spec.ts`
  拿到的是**空 CSP**（`Received string: ""`），而它钉的正是「控制台允许隔离的 localhost 预览源、
  其余一律收紧」这条安全边界。**判据**：别搜关键词就下「断言过期」的结论 —— 该文案在
  `pyengine/src/opensquilla/gateway/middleware.py` 里一直存在（我第一遍只搜了 `webui/`，差点误判成
  「文案全仓消失 = 断言过期」）。✅ 已修：`core/src/gateway/server.rs` 新增 `CONTROL_UI_CSP` +
  `control_ui_security_headers_layer`（挂在 router 最外层，覆盖静态托管与 SPA fallback 两条产出路径），
  判定口径逐条对齐 pyengine `_is_control_ui_request`（GET/HEAD、at-or-below `/control`、排除
  `/api`/`/ws`/`/health*`）；`control_ui.enabled` 在 Rust 侧没有该字段，按 `enabled = true` 处理。
  跨实现守卫 `test_control_ui_security_headers_match_pyengine_middleware`（`include_str!` 解析 Python
  源码里的 CSP 拼接 + 三个头，逐字比对；**变异实测**：把 Rust 的 `frame-src` 改成 `'none'` → 判红并
  同时打印两侧全文）。真机复核：`/control/` 四个头齐备、`/api/health` 一个都没有；headless 探针
  （`%TEMP%\csp_violation_probe.cjs`）在 `/control/`、`/control/settings`、`/control/chat` 三个路由上
  **securitypolicyviolation 事件为 0**（页面照常渲染 2000+ 节点）→ CSP 没有把控制台打坏。
  ⚠️ 同一现象的另一面（**已由 R54 处理**）：控制台加载即打 13 个 501。其中审批那 8 条已修（见下一条——
  它不是噪声，是**功能坏掉**）；剩 `GET /api/system/update`（3）与 `POST /api/elevated-mode`（2）待前端按能力降级。

- **🚨 控制台点「允许」曾经永远不生效 —— 审批的 REST 面只有 pyengine 有**（R54 实测定位 + 已修）。
  `/control` 由 **Rust** 网关托管，抬升审批的 `chat.send` / 链式执行器也是 **Rust**（挂起项注册进
  `guard::approval_registry`），但 WebUI 的审批卡片**只通过 REST** 做快照与裁决
  （`useChatApprovals.ts` 的 `fetchSnapshot` → `GET /api/approvals`、`resolveApproval` →
  `POST /api/approvals/resolve`；推送才走 WS `session.event.approval`）。而 Rust 网关对
  `/api/approvals*` 一律回 **501**（只有 pyengine `app.py` 实现了这几条）—— 于是**「允许」按钮
  打不通**，链式执行器只能等审批超时按 fail-closed 判失败，功能等于坏掉（`approval_registry.rs`
  头注释还把 `/api/approvals` 列为 WebUI 的三条裁决链路之一 = 文档承诺了、代码没兑现）。
  ✅ 已修：新增 `core/src/gateway/approvals.rs`（`GET /api/approvals` 快照 + `POST /api/approvals/resolve`），
  契约按 pyengine `api_approvals / api_approvals_resolve` 对齐；写接口照 pyengine `_same_origin` 做**同源校验**
  （带 `Origin` 必须与请求 authority 同源，且 authority 必须是回环 —— 第二条挡 DNS rebinding；无 `Origin`
  的 curl/桌面客户端放行），拒绝时回 pyengine 同款 `403 {"code":"FORBIDDEN_ORIGIN"}`。
  注册表新增只读 `pending_snapshots()` / 门控新增 `live_pending()`（**不能**用消费式的 `outcome()` 做列表查询，
  那会把待裁决项判掉）。
  **证据**：真机 18801 → `GET` 200 `{"pending":[],"mode":"prompt",…}`（此前 501）、未知 id 404、跨源 403、
  同源 Origin 放行、缺 id 400；`chain/executor.rs::approval_survives_console_rest_round_trip` 跑**真实 HTTP
  往返**——执行器抬升审批 → 后台任务拉快照拿 id → POST 裁决 → 断言步骤**真的执行成功**。
  **变异实测**：摘掉路由 → 该用例 `审批挂起后在 6s 内未被快照暴露`、HTTP 用例 `快照应 200: HTTP/1.1 501`（有牙）。
  ⚠️ 诚实边界：快照只覆盖**本进程**（Rust 引擎）的队列；侧车自己的审批队列归 pyengine 的 `/api/approvals`，
  双引擎同在时两侧互不可见，跨队列合并/未命中转发**尚未实现**。
  ⚠️ 仍未修的两个 501（同一现象的剩余部分）：`GET /api/system/update`（更新横幅）、`POST /api/elevated-mode`
  （pyengine 的按会话提权开关，Rust 引擎没有这个概念）—— 这两条该由**前端**按能力降级，而不是在 Rust 里造语义。
  ✅ **R57 更正**：这两个死调用前端**已经**在正确降级（`useChatElevatedMode` 把 403/404/405/501 一律当「本部署不支持」
  并永久闩锁、同时清掉 localStorage 里的假开关；`UpdateBanner` 对 `!response.ok` 直接 return，真通道是注入的
  `#Marguerite-data[data-update]`）。它们是**噪声**，不是缺陷 —— 不要再按缺陷去「修」。

- **🔗 两侧审批队列各说各话：侧车抬起的审批「卡片推得到、点『允许』什么也不发生」**（R57 实测定位 + 已修）。
  双引擎下 Rust 引擎与侧车**各有一份审批队列**（Rust 在 `guard::approval_registry`，侧车在它的 `ApprovalQueue`），
  而控制台只连 Rust 网关。实测（隔离双引擎网关，**等侧车就绪后**）：经 `exec.approval.request` 造出一条真实的
  **侧车侧**挂起审批 → `GET /api/approvals` 回 **`pending: []`** → `POST /api/approvals/resolve` 回 **404**
  「approval is missing or does not match」→ 复查侧车队列仍是 `pending: true, resolved: false`：
  **用户点了「允许」，审批原地不动**。
  ✅ 已修：`gateway/approvals.rs` 快照**合并**两侧（`merge_pending` 按 id 去重，本进程在前），裁决时本进程未命中就
  **转发**给侧车 `/api/approvals/resolve`（带共享 token；2s 拉快照 / 5s 转发，侧车不可用则回自己的 404 ——
  绝不假装成功，也不把界面挂住）。
  **证据**：同一探针（`%TEMP%\cross_queue_approval_probe.cjs`）修后 —— `rustSnapshotPending: 1`、`resolveStatus: 200`、
  侧车回包 `{"approved":true,"resolved":true,"resolution":"approved"}`、复查侧车队列 `pending:false, resolved:true`
  （两个不同 approval id 各复现一次）。守卫：`merge_pending_keeps_both_queues_without_duplicating_ids`、
  `forward_resolve_returns_none_when_sidecar_is_unreachable`。
  ⚠️ 探针坑：侧车 boot 可长达 ~90s，**不等它「不再回 Rust 的 NOT_IMPLEMENTED」就测，测到的是纯 Rust 模式**。

- **🧪 那个「偶发」的 `settings-modal` 用例不是超时抖动，是断言过窄**（R57 定根因 + 已修）。
  失败时 Playwright 的 ARIA 快照显示 Model Routing 面板渲染的是**空状态**（`Add a model provider to start`
  → `data-testid="model-strategy-provider-first"`）；生效条件是 `panel.hasSavedProvider`
  （= `primaryProviderIsConfigured`，看 `effective.fields['llm.provider'].source` 与 `onboardingStatus.llmConfigured`）。
  **隔离网关用的是空配置**，所以「没有服务商」才是常态；旧断言无条件等策略卡 radiogroup → 时红时绿。
  修法（不弱化）：接受两种**都合法**的渲染，并加自证 —— 两者**恰好**出现其一
  （`count() + count() === 1`，面板没渲染时是 0/0 而先失败）。
  实测：修前两次全量门禁各红一次；修后连跑 **5 次全量 `--e2e`（3+2）全绿 95/95**。
  ⚠️ 教训：`serial` 模式把一例失败放大成 ~20 例 skip（表现为 `75 passed + 1 failed`），别被「通过数骤降」
  误导成「大面积回归」。

- **🔌 会话键请求头改了一半 —— 每次按会话取附件/产物都 404**（R55 实测定位 + 已修）。
  控制台把会话键放在**请求头**里：`webui/src/utils/chat/attachmentAccess.ts` 在发请求前会删掉所有含
  `token`/`session` 的查询参数（`CREDENTIAL_QUERY_KEYS`），所以 URL 携带的键必然被丢弃。品牌改名把
  src/ 的头改成了 `x-Marguerite-session-key`（15+ 处），而 **pyengine 的四个读取点只认旧名
  `x-opensquilla-session-key`**（`attachments.py` / `artifacts.py`×2 / `artifact_preview.py`）→
  每个按会话鉴权的下载都解析成「没有会话」并回 **404**。e2e 侧同时有 4 处断言还在用旧名
  （`attachment-drag-upload` / `artifact-card`×3），即**两边都以为对方还叫旧名**。
  ✅ 已修：新增 `pyengine/src/opensquilla/gateway/session_key.py::request_session_key()` 作为唯一解析入口
  （品牌名头 → 旧名头 → 查询参数；头优先是刻意的——URL 里的凭据会进日志/Referer），四个读取点全部改用它；
  e2e 的 4 处断言改成品牌名。
  **判据**：`pyengine/tests/unit/gateway/test_session_key_header_contract.py` 三件事——①真机路由用**品牌名头**
  能 200 下载（修复前 404，实测红）②解析优先级真值表 ③**跨实现源码守卫**：控制台 HEAD 发的头名必须落在网关
  接受清单里（前端再改名就会在这里红）。e2e 侧 `e2e/brand-keys.guard.spec.ts` 新增同向守卫：扫 `src/` 得出
  实际发送的头名，任何 spec 断言了别的拼写就精确报出行号。**变异实测**：把 spec 改回旧名 →
  `attachment-drag-upload.spec.ts:439 … → src/ sends x-marguerite-session-key`。
  ⚠️ 教训：`brand-keys.guard.spec.ts` 里原本把 `x-opensquilla-session-key` 写在「仍然正确、别乱改」的
  白名单注释里（写着「15 uses in src/」），**那条注释已过期**，反而护着群里最旧的用法 —— 白名单注释也会腐烂。

- **🧾 `pyengine/tests/test_gateway` 曾是没人跑的死树 —— R58 已接入门禁**（R55 发现，R58 修复）。
  `verify.sh --pyengine` 只跑 `pyengine/tests/unit`（761 例），CI 也只跑同一层外加一个**只查收集**
  的检查（`check-pyengine-collection.py`）。于是 `tests/test_gateway` 2900+ 个通过用例**没有任何门禁在跑**，
  失败长期无人看见（R58 复测基线：**2901 passed / 29 failed / 162s**）。
  ⚠️ 这与 R24（pyengine 未进门禁）、R28（workbench 未进门禁）是**同一类缺陷的第三次**：一个目录改名
  就能让一整棵 2900+ 用例的测试树静默失效。R58 把 29 例逐条定性并修复后，接入
  `check_pyengine_gateway_tests`（`--pyengine` 与 `--all` 都调度，README 闸门总数 23 → **24**）：
  1. **13 处静态源断言指向从未迁移的上游控制台目录 `opensquilla-webui/`**（本仓控制台是 `webui/`）：
     `test_chat_view_static.py`、`test_chat_meta_{command,ribbon}_static.py`、`test_health_view_static.py`、
     `test_static_onboarding_views.py`、`test_webui_sources_row_static.py`。已抽到**单一模块**
     `tests/test_gateway/spa_source.py`（`SPA_ROOT` / `spa()`），不再逐文件写死路径。
     ✅ 断言字面量在 `webui/` 里**逐条命中**（70+ 条探针实测），即 `webui/` 就是那个 SPA 的迁移体 ——
     缺的只是路径，所以修复=换路径，不是换契约。
     ⚠️ 其中两条是**断言本身有缺陷**，与目录名无关：①`sharedRoutes.ts` 的
     `assert "HealthView.vue" not in routes` 被现役 `@/views/EngineHealthView.vue`（`/engine` 的仪表盘）
     **子串误伤** → 改为断言导入路径 `"/views/HealthView.vue" not in routes`；②`ChatView.vue` 断言
     `const pendingForkBeforeMessageId = ref(...)` 在视图里，而该 ref 已抽到
     `views/chat/useChatViewState.ts` → 断言改到新家，其余接线断言（传入两个 composable、会话切换时清零）
     原样保留。
  2. **14 例 auto-summarize 的根因是夹具 `budget=10` 低于「压缩后载荷」的估算地板** ——
     R55 记的「行为漂移、实现已变」**定性错了**，此处更正。`AUTO_SUMMARIZE` 只有「压缩后估算 ≤ budget」
     才走成功分支；估算器有**两档**：装了 `tiktoken`（`pyproject` 的 `memory`/`recommended` **可选** extra）
     时 ≈ chars/4，**未装**时回退保守的 utf8 bytes/2（约翻倍）。两档下「kept entry + 回放 summary +
     新消息」的地板是 **11 / ~7 tokens**，所以 `budget=10` 只在装了 tiktoken 的机器上能成功。
     **实测三连**：把估算器钉成 chars/4 跑该文件 → 41 passed；本机默认档 → 14 failed；
     改成两档都成立的 `_AUTO_BUDGET = 20` → **两档都 2929 passed / 0 failed**。
     ⚠️ 教训：夹具被校准到「某个可选依赖存在时的估算语义」，会让 CI 与开发机红绿不一 —— 而这正是
     门禁层最容易长期无人发现的那类缺陷。
  3. **1 例断言源码树没有 dist 元数据时的行为**：`test_status_version_provider.py` 断言
     `opensquilla.__version__ == importlib.metadata.version("opensquilla")`，而本仓**从不安装这个包**
     （`pythonpath = ["src"]`，见下条）→ 它在任何源码树上必红。改为：有 dist 元数据时比对元数据；
     没有时**仍咬住真契约**（`__init__.py` 的哨兵必须是 `0.0.0+unknown`，不得退化成硬编码 semver ——
     那正是该模块存在的理由）。
  4. **1 例断言从未迁移的上游文档章节**（`docs/troubleshooting.md` 的「Control UI Assets Are Unavailable」）
     → 显式 `pytest.mark.skip` 并写明原因（与 `_INHERITED_GROUPS` 同类：具名列出，不是假装通过）。
     ⚠️ 该层门禁因此带 `-rs`，让跳过原因出现在证据里（§8「跳过 ≠ 通过」）。
  与 `tests/unit` 拆成两条检查是刻意的：**162s** 与「纯单测 11s」不同量级，混进同一条 `[PASS]` 会掩盖慢测。
  现状：**2929 passed / 0 failed / 11 skipped / 162s**（该目录**不含**任何 llm/live/browser marker，
  实测 0 处 —— 所以它不需要 marker 排除表，也就不存在第二份会漂移的清单）。
  ⚠️ 其余目录（`test_scripts` / `test_desktop` / `test_packaging` / `test_provider` /
  `test_skills` / `test_public_release_hygiene.py` …）**仍无门禁执行**；全树**执行**结果未测（不可测），
  不要声称它绿。⚠️ 另：`hatch_build.py`、`THIRD_PARTY_NOTICES.md`、`test_compose_yaml_shape.py` 里
  仍写着 `opensquilla-webui/`（各自有独立理由，本轮未动）。

- **🧾 契约族六个目录曾是没人跑的死树 —— R60 已接入门禁**（R59 发现，R60 修复）。
  与 R58 的 `tests/test_gateway`、R24 的 pyengine、R28 的 workbench 是**同一类缺陷的第四次**：
  `verify.sh --pyengine` 只跑 `tests/unit` + `tests/test_gateway`，于是 `test_ci` / `test_migrations` /
  `test_sandbox` / `test_onboarding` / `test_tools` / `test_recovery` 里 **30 个失败长期无人可见**。
  R60 新增 `check_pyengine_contract_tests`（`--pyengine` 与 `--all` 都调度，**一次检查内逐目录登记证据**，
  闸门总数 24 → **25**）。R59 基线 → R60 现状：
  | 目录 | R59（未接门禁） | R60（已接门禁） |
  |---|---|---|
  | `test_ci` | 35p / 0f / 1s（48s） | 35p / 0f / 1s（35s） |
  | `test_migrations` | 59p / 0f（23s） | 59p / 0f（23s） |
  | `test_sandbox` | 1738p / **11f** / 346s（94s） | 1746p / **0f** / 349s（89s） |
  | `test_onboarding` | 955p / **3f** / 6s（44s） | 956p / **0f** / 8s（41s） |
  | `test_tools` | 1200p / **2f** / 23s（109s） | 1202p / **0f** / 23s（92s） |
  | `test_recovery` | 431p / **14f** / 8s（121s） | 444p / **0f** / 9s（100s） |
  ⚠️ **R63 给该检查补了「无结论行」的证据兜底**：它原先对每个目录只 grep `^(FAILED|ERROR)` 与汇总行，
  而实测出现过「目录红了、却既无 FAILED 行也无汇总行」（进程被打断 / 输出里没有 pytest 结论行），
  此时原写法**一行都不打印**，末尾只剩一句「1/6 个目录有失败」—— 违反「红必须带证据」。
  现改为：没有汇总行时退回**原始输出尾部 20 行**，至少留下现场。
  **30 个失败逐条定性：25 条真修，5 条具名 skip**（跳过 ≠ 通过）。真修的部分：
  1. **`recovery/atomic.py::_chmod_open_file` 的能力探测是过期的（7 例）** —— 它只探 `os.fchmod` 是否存在，
     而 CPython **3.13 起 Windows 也有了 `os.fchmod`**（本机 3.14 实测 `hasattr` = True），于是那句
     「POSIX 加固是额外能力、不是可移植前置」的注释**变成了硬失败**：`fchmod(fd, 0o600)` 对
     **只读打开的 fd** 抛 `PermissionError [WinError 5]`，而每个调用点都是先 `O_RDONLY` 打开比对身份再加固
     → `AtomicStateUnknownError: workspace config backup could not be restricted to its owner`。
     实测 `os.chmod(0o600)` 在 NTFS 上**永远回报 `0o666`**（stat 只有只读位），所以「仅属主」在 Windows
     本来就不该由 mode 表达。修法：新增 `can_harden_posix_mode()`（`os.name != "nt" and callable(fchmod)`），
     `settings_transaction._restrict_existing_credential_to_owner` 也改用它 —— 那个 `can_harden_posix_mode`
     变量此前还会在 Windows 上**对只读 fd 调 `os.fsync`**（实测 `OSError [Errno 9]`）并断言 `0o600`。
  2. **`gateway/rpc_sandbox._path_list_start` 的符号链接环在 Windows 上换了错误类型（2 例）** ——
     POSIX 的 `pathlib` 抛 `RuntimeError("Symlink loop from …")`，Windows 走 `ntpath.realpath`，
     实测抛 `OSError [WinError 1921]`（`ERROR_CANT_RESOLVE_FILENAME`；缺文件仍是 `FileNotFoundError`，故可区分）。
     于是「工作区不可解析 → 回落主目录」的 `except (FileNotFoundError, NotADirectoryError, RuntimeError)`
     **在 Windows 上不生效**，而「显式请求的坏路径必须保持严格」也变成了另一个异常类。修法：新增
     `_resolve_strict()` 把 1921 归一成与 POSIX 同文案的 `RuntimeError`，两处调用点共用。
     ⚠️ 同一常量 `1921` 在 `sandbox/directory_listing.py` 早已被当作「未解析的符号链接目标」处理 —— 本仓的既有惯例。
  3. **六处静态源断言指向从未迁移的上游控制台目录 `opensquilla-webui/`（这 30 条里的 6 例）** —— 与 R58 同一根因、
     同一结论：`webui/` 就是那个 SPA 的迁移体，缺的只是路径。涉及
     `test_sandbox/test_frontend_sandbox_assets.py`（4 例）与 `test_sandbox/test_release_contract.py`（2 例）。
     同一次排查在门禁之外还修了两处同源路径（`test_skills_community_copy_contract.py` 转为全绿；
     `test_skills_third_party_notices.py` 顺带暴露真正的合规缺口：Vue 控制台的直接依赖 **`three`**
     没有出现在 `THIRD_PARTY_NOTICES.md` —— 已补一行 MIT 署名）。
  4. **子进程类用例 import 不到 `opensquilla`（6 例）** —— `pyproject.toml` 的 `pythonpath = ["src"]`
     只补**本解释器**的 `sys.path`，而恢复事务的契约用例要 spawn 新解释器
     （`subprocess.run([sys.executable, "-m", "opensquilla.cli.main", …])`），侧车按设计**从不安装**
     → 子进程一律 `ModuleNotFoundError: No module named 'opensquilla'`。更坏的是 Windows 的次生效应：
     父进程往**已经死掉**的子进程 stdin 写/flush 时报裸 `OSError [Errno 22] Invalid argument`，
     把「import 失败」伪装成「管道坏了」。修法：`tests/conftest.py` 把 `pyengine/src` 前置进 `PYTHONPATH`
     （子进程继承），实现「PACKAGED」语义的用例本来就会自己 `pop("PYTHONPATH")`，语义不受影响。实测
     `tests/unit` 761p/6s、`tests/test_gateway` 2929p/11s **不变**（无外溢）。
  5. **两例硬编码 `python3`（2 例）** —— 全新 Windows 只有 `python`，实测 `python3 -c "print(1)"` 是
     **exit 1 且 stderr 为空**。改用运行中的 `sys.executable`（**不加引号**：Windows 侧工具会把
     「含分隔符的 python 路径」重写成 `Invoke-OpenSquillaPythonProcess -FilePath '…'`，手写引号反而
     触发 PowerShell `Unexpected token`）。同模块里其余 `"python3 …"` 字面量是**只被解析、不被执行**的
     分析器输入，**故意保留**。
  6. **`test_flow.py` 的飞书依赖提示断言写死了发布版的形态（1 例）** —— 它断言输出里有
     `releases/download/` 与 `v0.5.2`，而源码树的 `__version__` 是哨兵 `0.0.0+unknown`
     → 产品**故意**走「不猜 wheel 地址、指向 latest release 页」那一支（该分支的 docstring 明写）。
     改为按构建类型断言两条互斥契约：发布版必须报**运行中版本**的 wheel 名（比写死 `0.5.2` 更强），
     哨兵版必须报 `<wheel-url>` + `releases/latest`。
  7. **`test_windows_native_move_refuses_real_cross_volume_move` 的 CI 判定过宽（1 例）** ——
     它用通用 `CI` 变量判「是不是 CI」，而**本机工具 shell 就设了 `CI=true`**（实测）→ 把一次合法的
     「缺两个合成卷」skip 变成硬失败。改用 `GITHUB_ACTIONS`（本仓 pyengine job 是 ubuntu，该用例在那里本就 skip；
     Windows runner 上仍 fail-closed）。
  5 条具名 skip：`test_file_policy` 那条把 `platform="linux"` 与**原生 Windows home 路径**混用
  （POSIX 图案保留反斜杠、宿主解析器却给出带盘符路径 → 永不匹配，与同文件既有 skip 同类）；
  `test_release_contract` 两条断言上游 Electron 运行时清单与上游打包校验器；`test_onboarding` 两条
  断言从未迁移的上游产品文档与 Electron 桌面壳。

- **⏱️ `settings-modal.spec.ts` 有偶发**（R55 实测：同一批改动下，连续两次全量 `--e2e` 一次红一次绿；
  更早的 R53 两次都绿，故是低频偶发而非稳定红）：`rail switches sections…` 在 5s 内等不到
  `radiogroup 'Choose how models are used'`。**放大效应**：该文件是 `mode: 'serial'`，一例失败会让
  **后续 ~20 例全部 skip**（表现为「75 passed + 1 failed」而不是「94 passed + 1 failed」），闸门直接红。
  已记录，未定位到根因；下次复现时应查该面板的 `radiogroup` 是否被某个异步 profile 加载门控，
  而不是简单加超时（加超时只会把概率往后推）。

- **📦 产物下载在真机控制台上必然 501 —— 取字节只有 REST 一条路**（R56 实测定位 + 已修）。
  `webui/src/utils/chat/artifacts.ts::artifactDownloadUrl` 对「有 `id`、没有 `download_url`」的产物**直接拼**
  `/api/v1/artifacts/{id}`，而 Rust 台账里的产物正是这个形状（`id = artifact:<session>:<path>`）→ 点下载
  必然拿到 501，前端弹「下载失败（HTTP 501）」。同一条 URL 还承担**内联文本编辑的读取**
  （`useArtifactActions::loadTextContent`）。对照：`open_in_system` / `edit_inline` 是「RPC 优先、REST 兜底」，
  所以只有取字节这条在纯 Rust 托管下是死的（**这是 R54 同族的第三个实例**：pyengine 有、Rust 没有、
  而控制台只用 REST）。
  ✅ 已修：`gateway/artifacts.rs` 新增 `artifact_download`（`GET /api/v1/artifacts/{id}`）——**流式**回文件本体，
  如实给 `Content-Type`/`Content-Length`/`Content-Disposition`（文件名过 ASCII 白名单，防响应头注入）+ `no-store`。
  **安全要点**：只接受 **id** 且必须命中 Rust 台账（`file_change::get_artifact`）——绝不接受路径，
  否则就是任意文件读取；未登记 / 已删除 / 目录类一律 404。
  **证据**：真机 18801 —— 经 `artifacts.register` 登记真实文件后 `GET /api/v1/artifacts/<id>` → **200**、
  `content-type: text/plain`、`content-length: 22`、`content-disposition: attachment; filename="probe-report.txt"`、
  本体逐字节正确（修复前同一请求 501）；未知 id → 404 `{"code":"NOT_FOUND","error":"Artifact not found"}`。
  守卫 `test_artifact_download_serves_registered_file_only`（**变异实测**：摘掉路由 → `已登记产物应 200: HTTP/1.1 501 Not Implemented`），
  其中含关键否定面：**同目录下未登记的文件不能靠猜 id 读到**。
  ⚠️ 已知差距：Rust 无缩略图存储，`?variant=thumb` 被忽略并回原图（缩略图网格会拉整图）；
  `/api/v1/artifacts/{id}/open|content` 仍是 501，但前端只在这两条 RPC 缺席时才走它们，故不影响。

- **🌉 侧车 REST 面在 Rust 托管下整体不可达 —— 改成「按面兜底」的通用代理（R61 实测定位 + 已修）**。
  与 R54（审批）/ R56（产物取字节）同族，但这次不再是第四个实例：**UI 只有 REST 一条路的功能有一整片**，
  而 Rust 网关对它们一律回 501 —— 最典型的是**附件上传**（`useChatAttachments.ts` → `POST /api/v1/files/upload`，
  拖文件进输入框必然失败、`chat.send` 拿不到 `file_uuid`），另有支持诊断包（`/api/v1/diagnostics/bundle`）、
  Workbench 预览租约（`/api/v1/artifact-preview-leases/*`）、附件下载（`/api/v1/attachments/*`）、
  语音转写（`/api/audio/transcribe`）。**逐端点补必然继续漏**（每新增一个侧车端点就再坏一次），
  所以改成按面兜底：`core/src/gateway/sidecar_proxy.rs` 接管通配 `/api/{*path}` —— 侧车在线则原样转发，
  离线则沿用原来的 **501 JSON**（既有的「本部署不支持」契约不变）；更具体的路由（`/api/health`、
  `/api/approvals*`、`/api/v1/artifacts/{id}`）在 axum 里优先匹配，仍由 Rust 直接服务。
  实现要点：**双向流式**（大附件不整块进内存）、逐跳头两个方向都不转发、业务头（`x-Marguerite-session-key`
  等）原样透传、**客户端 `authorization` 不转发**（改带网关共享 token）、同源校验直接复用
  `approvals::origin_allowed`（口径不许漂移）、失败如实回 `502 sidecar_unreachable`（绝不假装成功）。
  `artifact_download` 在 Rust 台账未命中时**转问侧车**（Python 引擎产出的产物此前点下载必 404）。
  **证据（真机 18801 双引擎）**：`POST /api/v1/files/upload` → **200** + `{"file_uuid":"u-…"}`（修复前 501）、
  `/api/config|/api/sessions|/api/usage` → 200（侧车载荷）、未知路径 → 404、`/api/health` → 200（未被通配遮蔽）；
  路由层真实 HTTP 用例钉住**离线时仍是 501 + `not_implemented`**，跨源仍 403 `FORBIDDEN_ORIGIN`。
  ⚠️ **`no_proxy()` 是必需项，不是优化**：本机 WinINET 代理（`ProxyServer=127.0.0.1:7890`）会**替回环目标回 502**，
  于是「侧车离线」与「代理拒绝转发」在响应上无法区分 —— 摘掉 `.no_proxy()` 后
  `forward_reports_unreachable_sidecar_as_502` 立刻判红（实测：响应体变成空串而不是本模块的 JSON）。

- **⏱️ 侧车的 `/health` 被自己的限流回 429 → 看门狗把「活着」读成「死了」**（R61 实测定位 + 已修；
  这是「e2e 偶发变红」的真根因）。侧车 `RateLimitMiddleware` 当时只豁免了 Control UI、`GET /api/approvals`
  与预览 capability 路径，**没豁免存活探测**；而 Rust 侧车守护任务用 `status.is_success()` 判定健康 →
  负载下（全量 e2e、4 worker）探针实测 **21/90 次拿到 429** → 看门狗判定
  `alive=true http_ok=false` 并**杀掉健康引擎**（实测重启耗时 21s，期间控制台所有 RPC 失败、
  `settings-modal` 的 Discard 无法完成重载 → 该用例红 + serial 连带 15 例 skip）。
  ✅ 两处都修：①pyengine `middleware.py` 新增 `_LIVENESS_PATHS`（`/health`/`/healthz`/`/ready`/`/readyz`、
  `/api/health`）永久豁免限流；②Rust `health_status_is_alive(status) = status < 500`（**有回答即存活证据**，
  只有 5xx 与传输失败才算不健康），并把「单次失手就杀」改成「连续 3 次失手 + 一次 10s 长超时复核」
  （`classify_health` 纯函数；`HEALTH_MISS_LIMIT`/`HEALTH_PROBE_TIMEOUT`/`HEALTH_REPROBE_TIMEOUT`）。
  守卫：Rust `rate_limited_probe_still_proves_liveness`、`alive_but_slow_probe_is_not_dead`、
  `exited_process_is_dead_immediately`、`reprobe_success_counts_as_healthy`；
  pyengine `tests/unit/gateway/test_rate_limit_liveness_exemption.py`（真实 app + 真实中间件，桶限 1：
  `/health` 5 次全 200、`/api/config` 第 2 次 429 —— 反向自证豁免没把限流整体关掉）。
  **变异实测**：把 pyengine 侧豁免条件改成 `if False:` → pyengine 2 例红（429）；摘掉 `.no_proxy()` →
  上述 502 用例红。**e2e 连跑 3 次 95 passed ≥ 基线 95 全绿**（修复前每次都在 Discard 那一步红）。

- **📦 安装版里打包进来的 Python 引擎永远用不上（探测不看 `resources/`）**（R61 实测定位 + 已修）。
  `package_full.cmd` 会把引擎源码放进 `gui/src-tauri/resources/pyengine/src` （安装后 =
  `<install>/resources/pyengine/src`），但 `detect_pyengine_src()` 只找 `pyengine/src` / `../pyengine/src` /
  `marguerite/pyengine/src` 与 **exe 上溯** —— 安装目录里这三条都不存在 → `SidecarConfig::detect` 失败 →
  桌面端**静默降级成纯 Rust 模式**（那一片 REST 能力直接变 501，见上一条）。
  ✅ 已修：候选列表抽成纯函数 `pyengine_src_candidates(cwd, exe)`，**打包资源布局排在最前**
  （`<exe dir>/resources/pyengine/src`，每层上溯也都看一次），`detect_pyengine_src` 只做「取第一个合法候选」
  （合法 = 含 `opensquilla/cli/main.py`）。守卫：`packaged_resource_layout_is_searched_and_ranks_first`、
  `first_valid_src_picks_the_packaged_tree_and_skips_invalid_candidates`（真实目录夹具 + 干扰项）。
  **变异实测**：把该候选改成不存在的路径 → 两条同时判红。
  ⚠️ 打包纪律：`scripts/package_full.cmd` 会先同步 webui/dist、skills、resources/*（含 pyengine src）再构建；
  **直接 `cargo tauri build` 会用上一次的 resources 快照**（本次实测：装出来的 pyengine 是 9/24 的副本）——
  发版必须走 `package_full.cmd`，或自行执行它的步骤 3–4 后再构建。
  ⚠️ **同一个坑的第二段（本轮实测）**：修好探测后，安装版引擎**能启动**却立刻退出 ——
  `RuntimeError: opensquilla migrations directory not found`。因为 `resolve_migrations_dir()`
  （`pyengine/src/opensquilla/persistence/migrator.py`）的仓库候选是 `Path(__file__).parents[3]/"migrations"`
  = 安装后的 `<install>/resources/pyengine/migrations`，而 `package_full.cmd` 当时**只复制了 `pyengine/src`**。
  ✅ 已修：脚本新增 `pyengine\migrations` 的复制（缺失时**直接 exit 1**，不再产出「装得上但引擎起不来」的包）。
  守卫：`pyengine/tests/unit/gateway/test_desktop_bundle_migrations.py` 四条 —— 仓库 migrations 存在且含 `V*.py`、
  `migrator.py` 的候选路径**等于**打包位置（证明两者不会各走各的）、打包脚本含 `pyengine\migrations`（含 `src` 的反向自证）、
  轻量脚本 `package_app.cmd` 必须指明资源来自全量脚本。
  **判据（安装版是否真的双引擎）**：管理员解包 MSI（`msiexec /a <msi> /qn TARGETDIR=<tmp>`）→ 以解包目录为
  `WorkingDirectory` 启动 `marguerite-gui.exe`（`MARGUERITE_CONFIG_DIR` + `LOCALAPPDATA` 都指向临时目录 = 干净机器等价）
  → 探 `http://127.0.0.1:18790/api/config`：**200 = 引擎在线**，501 = 纯 Rust 降级（本轮修复前实测就是 501）。

- **🧨 PowerShell 的 `Set-Content` 会写 BOM → Python 源码直接 `SyntaxError`**（R61 踩到，代价 4 个用例变红）。
  用 `Set-Content -NoNewline` 就地改文件（我用来做变异测试/批量替换）时，**Windows PowerShell 5.1 默认写 UTF-8 **带 BOM****，
  于是 `pyengine/src/opensquilla/gateway/middleware.py` 首行变成 `\uFEFF"""Middleware pipeline…`，
  Python 报 `SyntaxError: invalid non-printable character U+FEFF`，而 `tests/test_ci` 有 4 个「把全树源码
  喂给 ast/compile」的架构守卫因此变红（现象像「我的改动把架构搞坏了」，其实是编码）。
  ✅ 改文件的正确姿势：`[System.IO.File]::WriteAllText($p, $text, (New-Object System.Text.UTF8Encoding($false)))`
  （读用 `ReadAllText` 会自动丢掉既有 BOM）；改完用 `[System.IO.File]::ReadAllBytes($p)[0..2]` 确认不是 `239,187,191`。

- **🔴 真钥匙进过仓库 —— 工作区已干净，但历史就是发布面**（R27 核实；这条**需要你动手**）。实测三段：
  1. **工作区干净**：对**将随提交落地**的 8977 个文件扫 `sk-…` 形态，命中的 92 个**全部自我标注是夹具**
     （`sk-FAKE…`、`sk-fake--fixture-…`、`sk-SEN71NEL-do-not-leak-…`）或 CSS 类名（`sk-` 也是本仓类名前缀，
     所以朴素正则 `sk-[A-Za-z0-9_-]{10,}` 会刷出几千条假阳性，必须收紧成「无 `-`/`_` 分隔的长串」）。
     被报告的 `sk-bfd…` 真钥匙在**工作区 0 命中** → **`git add -A` 不会把它带回来**。
  2. **但它在历史里**：`git grep -l 'sk-bfd' HEAD` → **`HEAD:smoke_orchestrate.sh` 第 21 行**
     （`API_KEY="${API_KEY:-sk-bfd…}"`，一个兜底默认值），最早出现在**首个提交** `9362b1f`，共 29 个提交。
  3. ⚠️ **结论：这不是工作区能修的问题**。必须**吊销该密钥**（已在 29 个提交里，按已泄露处理），
     再决定**重写历史**（`git filter-repo`）还是**接受披露**。重写历史是不可逆的破坏性操作，
     **没有明确指示我不会替你执行**。
  ✅ 本轮补上的是**防再犯**：`scripts/check-secrets.py` 成为门禁的常驻检查（`--fast` / `--core` / `--all` 三层都跑）。
  ⚠️ 但接线时**只接进了 `--fast` 与 `--core`，漏了 `--all`** —— 于是 `--all` 实际只跑 **17** 项，而文档写着 18。
  这不是靠读代码发现的，是靠新加的 `scripts/check-readme-claims.py`（它会读 `verify.sh` 的 `--all` 块来数）
  当场报出来的：**「文档里的闸门总数」与「脚本真正调度的检查数」必须相等**。
  它只扫 `git add -A` 集合（被忽略的草稿里有密钥不是发布风险），**放行可识别的占位符、其余一律 FAIL**。
  夹具因此必须**自我标注** —— 该规则当场抓到一例：`skills/codebuddy/capability-evolver/test/sanitize.test.js`
  里一个 `sk-proj-…` 值长得像真 OpenAI 项目键（同文件其它夹具都自带 `EXAMPLE`/`abcdef`/`123456`），已改为 `sk-proj-FAKE…`。
  ⚠️ 同一轮还发现：**这个守卫此前只在本机跑**。`ci.yml` 的 `docs` job 只跑 `check-doc-cli-drift.py`，
  所以（a）新密钥扫描在 PR 路径上缺席，（b）`check-version-sync.py` **从来没在 CI 里跑过** ——
  而 `README.md` 明写它「in CI」强制对齐版本号：**一句没有实现的宣传**。已在 `docs` job 补上这两条
  （都只需 `python3`，无需 setup-python；实测 `bash -n` 零语法错、YAML 5 个 job 解析正常、docs job 由 2 步变 6 步 ——
  后来又把 `check-doc-links.py` 与 `check-readme-claims.py` 一并接了进去）。
  ⚠️ 诚实边界：`actions/checkout` 之后 `git ls-files -co` 只剩已跟踪文件，**只扫得到提交树、扫不到历史** ——
  所以它挡的是「新泄漏」，**不会**发现 `smoke_orchestrate.sh` 历史里那把钥匙；那把只能靠吊销。
  另一处必须说清：**当前 HEAD 的 `smoke_orchestrate.sh` 第 21 行仍是那把真钥匙**（工作区那份已修好但未提交），
  所以这个新 CI 步骤在**提交工作区之前必然红** —— 那是守卫在正确报警，不是回归。

- **📄 两份 README 会各自漂移，而没有任何门禁比对它们**（R27 实测）。`README.zh-Hans.md` 落后于
  `README.md` 一整代：写着整道闸门 **15/15 全绿（2026-09-22）**，而真实是 **18/18**；基线写 **2533 / 3212（310 文件）**，
  真实是 **2538 / 3216（311 文件）**；表里**完全没有** `pyengine` 那一层（758 个测试）与密钥扫描行；
  i18n 写 **3834 键**，实测 **3862 键**（脚本逐叶子统计，6 个 locale 全部 3862）；
  还声称 `PHASE_*.md` / `Security_Audit_*.md` 躺在仓库里 —— 它们早已隔离到 `docs/internal/`（实测根目录只剩
  7 个 `.md`，全是入口与社区健康文件）。**一个中文读者看到的是更旧、更小的项目**，而两页互相矛盾。
  **判据（可复用）**：语言差异不是差异 —— 只比对**与语言无关的事实**（`NNN passed / N failed` 基线、
  `NNN ignored/skipped/files`、`N/N` 闸门总数、locale 键数、版本号），并把「表格里出现的脚本名」求交集；
  行标题（`webui unit tests` vs `webui 单测`）不参与比对，否则全是假阳性。
- **🔗 相对链接会悄悄指到仓库外面**（R27 实测）。`README.zh-Hans.md` 写的是 `[SECURITY.md](../SECURITY.md)`
  与 `[AGENTS.md](../AGENTS.md)` —— 该文件就在**仓库根**，`../` 于是指到仓库之外，在 GitHub 上是**两个死链**
  （英文版写的是 `SECURITY.md`，正确）。同类还有 `docs/archive/architecture.md` 的 `../config.yaml`（正确是 `../../config.yaml`）。
  **实测口径**：把 `git ls-files -co --exclude-standard` 里的 **2096 个 markdown** 全量扫一遍，
  共 **1446 条相对链接**，其中**只有 3 条**落在本仓自己的文档里 —— 另外 **195 条全在 vendored 技能包里**
  （`skills/codebuddy/**`、`skills/trae/**`、`pyengine/src/opensquilla/skills/bundled/**`），
  且混着 `url` / `{videoUrl}` / `result|data|output` 这类**正则把散文当链接**的噪声（表格竖线、占位符）。
  ⚠️ **所以不要做一个「全仓相对链接必须可解析」的门禁** —— 它会立刻要求你修 195 条上游内容，
  或者写一大堆豁免。要做就**把范围限定在本仓自己的文档**（排除 vendored 技能包），那样基数是 0、直接可用。

- **🧹 机器状态残留有两种形态：目录和文件 —— R18 只堵了目录那种**（R28 实测）。
  R18 加的规则是 **`*.tmpdir/`**（目录形态），于是 R28 用「按文件名逐条扫 `git ls-files -co` 集合」时
  仍然抓到 **1 个会随提交发布的机器状态**：
  `workbench/frontend/_tmp_32456_50033095d6122e050fba89244b7152f9`（**0 字节**）——
  命名是 `_tmp_<pid>_<32位hex>`，是原子写在目标目录留下的中间文件：写成功会被 rename 掉，
  中断就永久留在那里。**判据**：`git check-ignore` 对**不存在**的路径可能不匹配，
  所以要先证明规则命中（`git check-ignore -v <真实文件>` = exit 0）、再确认它掉出 `git ls-files -co`、
  再删；并且**造一个同形态的合成文件再验一次**，否则你只是删了一个实例、没修掉那一类。
  已加 **`**/_tmp_[0-9]*_[0-9a-f][0-9a-f]*`**。修后实测：`git ls-files -co` 里**机器状态残留 = 0**
  （只剩 4 个**正当**的 `package-lock.json`），commit set 8979 → **8978**。
  ⚠️ 复扫口径：把 commit set 按可疑文件名形态分组（`.pytest_cache/`、`pytest-cache-files-*/`、`__pycache__/`、
  `*.py[co]`、`*.orig/.rej/.bak/.swp`、`.DS_Store`、`*.log`、`*.db`、`.vite/`、`*.tsbuildinfo`），
  并且**只算磁盘上真实存在的文件** —— commit set 里含 504 条「已跟踪但工作区已删」，
  它们提交后是删除，不是被发布的内容。

- **🕳️ 一个整子项目可以既随仓库发布、又不被任何门禁跑、也不出现在主 README 里**（R28 实测）。
  `workbench/` 是**第五个入口**：Python 后端（engine · api · cli · llmlab · ttc · agentloop）+
  FastAPI 在 `:8011` 同源托管的 React SPA + 独立打包流水线，`git ls-files -co` 里 **76 个文件**会随提交落地。
  三条实测把它钉死：① `python -m pytest marguerite_workbench/tests` → **71 passed, exit 0**（2.3s，离线无需 key）；
  ② `workbench/packaging/check_version_sync.py` → **PASS**（backend/frontend/VERSION 三处都是 1.1.0）；
  ③ **`grep -c workbench scripts/verify.sh` = `0`** —— 即上面两条**从来没有被任何门禁跑过**，
  而 `workbench/README.md` 却把它写成「CI 门禁」。与 R24 的 pyengine 是**同一类缺陷**：
  一个对部分代码库沉默的绿徽章（当年的原话是「文档宣传了一个不存在的门禁」）。
  ⚠️ **三个容易误判的点**：① 它**不是** Rust 引擎的第五个前端 —— `marguerite_workbench` 在
  core/tui/webui/gui 里 **0 引用**（`core/src` 里那 27 处 `workbench` 命中是 **TUI 的 workbench 模块**，
  同名不同物；**求证时必须用 `marguerite_workbench` 这个包名，不能用 `workbench` 这个目录名**）；
  ② 它有**自己的版本线**（`workbench/VERSION` = **1.1.0**，与引擎的 **0.2.7** 相互独立，
  `check-version-sync.py` 那 9 处**不含它**），所以「版本号单一事实源」这句话**只对引擎那条线成立**；
  ③ 它的 `frontend/` **没有测试脚本**（只有 `dev/build/preview/typecheck`），所以这一层目前只能守后端 + 版本。
  已修：`verify.sh --workbench`（门禁第 21/22 项）纳入 `--all`，`workbench/README.md` 的声明改成事实，
  主 README 的 Layout 与 Status 补上它。

- **🧹 「作者本机路径」会随代码一起发布 —— 而且没人会告诉你**（R25 实测）。用一条正则扫全部**将随提交落地**的
  `.rs/.py/.ts/.vue/.js/.mjs/.json/.toml/.yml/.yaml/.cmd/.sh`，一开始扫出 **30+ 个文件**带着
  `D:\Downloads\Marguerite`。绝大多数是**正当的测试夹具**（`/home/u/proj`、`C:\Windows` 给安全扫描器用），
  但确实混着真缺陷，挑出来这几个都是**用户可见**或**完全跑不起来**的：
  | 文件 | 问题 | 处理 |
  |---|---|---|
  | `webui/src/views/GraphView.vue` | 输入框 `placeholder` 直接写着作者的项目路径（**每个用户都看得到**） | 换成平台中立的 `/path/to/your/project` |
  | `scripts/mcp_btpanel_smoke.py`（**已跟踪**） | 硬编码 `d:/…/marguerite/btpanel-mcp` —— **本仓库里并不存在的嵌套路径**，外加 Windows 专有的 `py -3.14` | 默认改用**仓库内** `btpanel-mcp/`，`BTPANEL_MCP_PATH` 可覆盖，命令用 `sys.executable` |
  | `btpanel-mcp/README.md` | 安装/自测步骤让人 `cd d:\Downloads\…`（对所有人都错） | 改成仓库相对路径 |
  | `pyengine/.../bundled/{audit,explore}-third-party/*.sh` | 默认目录写死作者绝对路径；而 `third_party/` **被 gitignore、clone 里根本没有** | 默认 `./third_party` + 目录不存在时**明确报错退出 2** |
  | `smoke_orchestrate.sh` | 钉在 `D:/tp_build_fresh1/...` 与作者检出 | 全部改为从 `$0` 推导，环境变量可覆盖 |
  | `core/src/gateway/refactor_methods.py` | **Rust 源码树里唯一的 .py**，零引用的一次性脚本 | 移到 `scripts/dev/` 并去掉写死路径 |
  | `scripts/{audit_rpc_surface2.py,dev/audit_rpc_surface.py}` | `ROOT` 写死本机 | 改为从 `__file__` 推导 |
  **判据（可复用）**：把 `git ls-files -co --exclude-standard` 的结果按扩展名过滤后逐文件正则扫；
  ⚠️ **只扫将要落地的文件**（`-co` 集合），且**接受测试夹具**——否则会把 30 个正当夹具当成缺陷。
  收尾后只剩 `scripts/dev/` 里 2 个（该目录被 `scripts/README.md` 明确标注为「历史一次性诊断，不维护、不进门禁」）。
- **⚠️ `git grep` 只搜「已跟踪」文件 —— 本仓库有约 1900 个未跟踪文件，所以它会静默漏掉真相**（R23 一次踩中两回）。
  实测：查 `modeHintBar` / `gotoTopbarBadge` 的消费者时，`git grep` 只返回 6 个 locale 文件，于是得出
  「这个键没有任何源码引用」甚至「提示条根本不渲染」的结论；**两条都是错的** —— 真正的消费者
  `webui/src/components/ModeHintBar.vue` **未被跟踪**，裸 `git grep` 看不见它，而截图里该提示条明明在渲染。
  **判据**：在本仓库问「谁引用了 X」必须用**文件系统级**搜索（`grep`/`rg` 工具，或 `git grep --untracked`），
  **不要用裸 `git grep`**。同族陷阱：`git ls-files <dir>` = 0 **不代表目录不存在**（可能只是未跟踪）。
- **📸 `docs/assets/*.png` 会随代码过期，而没有任何门禁会告诉你**（R23 实测）。README 的两张主图是
  **2026-09-22 14:16** 生成的，而 `classifier.rs` 是 **09-23 12:30**、locale 是 **09-23 18:27** 改的 ——
  于是**主图里展示的正是 R16 已经修掉的那个缺陷**：计划卡写着「决策理由：单一意图（**Unknown**）」，
  而 README 正文在同一页吹这个缺陷已修（现在是 `Build`）。更早那版还在英文界面里显示中文提示
  （`modeHintBar` 当时尚未本地化）—— 两张图都在**低报产品**，且与自家文档自相矛盾。
  **判据**：① 改完 core/webui 的**用户可见**行为后，比对 `docs/assets/*.png` 与 `core/src`、`webui/src` 的 mtime；
  ② 重新生成是**一条命令、不需要真 API Key**：
  ```bash
  python scripts/demo/mock_llm.py 18999 &                    # 本地 mock 模型
  MARGUERITE_API_KEY=mock MARGUERITE_BASE_URL=http://127.0.0.1:18999/v1 \
    MARGUERITE_MODEL=mock-model target/debug/marguerite-core.exe gateway &
  python scripts/demo/capture_console.py --locale zh-Hans \
    --prompt "帮我做一个完善的 FPS 第一人称射击游戏"
  ```
  ③ ⚠️ **改完 core 必须先重编 `marguerite-core` 再截图**：截图经网关渲染，跑的是**二进制**，
  与「桌面冒烟必查 stale core」是同一类问题（旧二进制 = 旧截图）。
  ④ `--locale en --suffix -en` 可出英文界面版本；但引擎生成的计划步骤/标题**目前仍是中文**，
  英文壳套中文内容会显得半翻译 —— 所以 README 主图刻意用中文界面（见 README「Known limitations」）。
  ⑤ **TUI 截图不需要 pty**（R26 打通）：`core/examples/tui_frame.rs` 用 ratatui 的 `TestBackend`
  渲染**真实一帧**（与 `core/src/tui/**` 测试同一路径），把每个单元格连同 fg/bg/modifier 重新发成
  ANSI；`python scripts/demo/render_tui.py` 再把 ANSI 转成 `docs/assets/tui-workbench.png`。整条链路
  **无 pty、无真终端、无交互会话**，所以 Windows 与 CI 都能复现 —— 这解决了「终端优先的产品却拿不出
  终端截图」这个长期缺口（此前因为 Windows 没有 `pty`/`termios` 一直没做）。
  ⚠️ **两个坑**：① ratatui 渲染宽字符时把后续单元格**清成空格**（不是标记 `skip`），所以按单元格逐个
  输出会在每个 CJK 串后多插一列，必须按**字形显示宽度**前进（为此给 `core` 加了 dev-dependency
  `unicode-width`，它本来就在 lockfile 里）；② `render_tui.py` 会校验**每行宽度恰好等于终端宽度**，
  这同时交叉验证了 Rust 与 Python 两套宽度表 —— 不一致会直接 exit 1，不会静默出一张错图。
- **🧨 GBK 乱码会以「合法 UTF-8」的形态混进仓库**（R26 实测）。`core/Cargo.toml` 里 **19 处**中文注释
  是「UTF-8 字节被当成 GBK 解码后再写成 UTF-8」的产物（`# OS 绾ф矙绠憋細Linux Landlock 浠呭湪 linux target …`）——
  **`utf8` 解码完全成功、`U+FFFD` 计数为 0**，所以任何「编码检查」都发现不了它，肉眼看却是彻底的乱码。
  **判据（可复用的精确检测，不是字符黑名单）**：把可疑的非 ASCII 片段 `encode("gbk")` 再 `decode("utf-8")`，
  能还原出通顺中文的才是乱码。⚠️ 我第一版用「乱码高频字符」黑名单，**误报 1336 个文件** —— 因为名单里
  混进了普通句号 `。`，于是每个中文文件都被判成乱码。精确法实测全仓**只有 2 个**命中：`core/Cargo.toml`
  （真缺陷，已修）与 `AGENTS.md`（**是刻意引用的示例**，本身不是缺陷）。
  ⚠️ 修法上要说清：这些注释的原始字节**不可还原**（多处混着丢失的 `?`），所以我**重写了**这些注释
  （并顺手去掉一条「本机已确认装有 MSVC BuildTools」这类**只对作者成立**的断言），而不是假装「恢复原文」。
  改完用 `cargo metadata` 核对：**41 个 normal 依赖 + 2 个 dev 依赖逐一不变**，只有注释变了。
- **⚠️ 不要用 shell 管道输出判断编码 —— 本会话被它骗了三次**：本机控制台代码页是 **cp936(GBK)**，PowerShell 与 `python -c "print(...)"` 的 **stdout** 会把 UTF-8 中文重新编码，于是**完好的文件/响应看起来像乱码**。实测三次假警报：
  1. 以为 `.gitignore` / `config.yaml` 的中文注释「已损坏」（`# 瀵嗛挜涓庣幆澧僠`）——实际用 `read` 工具一看完全正常，Python 逐字节复核 **`utf8=YES` 且 `U+FFFD=0`**；
  2. `pytest` 输出被 PowerShell 重定向写成 **UTF-16LE**，按 UTF-8 读全是 `\x00` 交错；
  3. 网关 `/api/approvals` 的 501 JSON 中文看着像乱码 —— 存成文件后 **hex 是 `e8afa5`(=该) `e7abaf`(=端)**，**是合法 UTF-8**。
  **判据**：用 `read` 工具读文件、或 `open(p,'rb')` 看 hex / `decode('utf-8')`，**绝不用终端里「看起来像乱码」下结论**。
  （另注：`verify.sh` 里 `check-version-sync.py` 的中文输出在终端同样显示为乱码，那是**显示**问题，不是脚本问题。）
- **系统代理劫持 loopback（测试隐雷）**：本机常开 Windows 系统代理（Clash 127.0.0.1:7890），reqwest 会把部分 127.0.0.1 请求也走代理——「关闭端口必被拒」等网络假设会失效（被代答 502，而 502 是可重试错误）。测试一律用「本地 mock 监听器回明确状态码」（400 不可重试 / 404 非可达），禁止用连接拒绝作为确定性失败源；单次 accept 的 mock 必须循环应答（对话流程含经验抽取等第 2 次请求，单发 mock 会让流程挂死）。公共基建见 `core/src/tests/mock_http.rs`（完整读请求后再应答，防分段读半截）。
  PowerShell 探测回环请用 `curl.exe --noproxy "*"`；本机 `Invoke-WebRequest` **没有** `-SkipHttpErrorCheck`（是 Windows PowerShell 5.1）。
- **网关闭环探测的实测基线（R15 亲测，可直接复用）**：`GET /api/health` → **200 `{"engine":"rust","status":"ok",...}`**；`GET /control/` → **200 `text/html`**（Vue 控制台）；`GET /api/<未实现>` → **501 JSON**（`{"error":"not_implemented",...,"transport":"/ws"}`，**不是** SPA HTML）；`GET /ws` → **400 `Connection header did not include 'upgrade'`**（正确，它不是普通 GET 端点）。
  ⚠️ **强杀网关会遗留 python 侧车**（`opensquilla.cli.main gateway run --bind 127.0.0.1 --port <随机>`，脱离进程树）。`Stop-Process` 网关后必须再查一次并清理 `python`，否则它会一直占着端口与文件。
- **⚠️ 用 `getBoundingClientRect()` 两两求交做「元素重叠」检测会产生假阳性**：rect 相交**不考虑祖先裁剪**。`.sidebar-core` 是 `overflow-y: auto` 的滚动容器，被 flex 压到 14px 后，其子行的 layout rect 仍在 y=105/150/195，于是「两个 `.sidebar-fn-label` 在 y=115 重叠 38×20」这种结论**在 DOM 里成立、在屏幕上不成立**（截图里侧栏完全干净）。
  **判据**：① 用**截图**确认（`scripts/demo/capture_console.py` 是可复现管线，或直接 Playwright 截 `.sidebar`）；② 判定可见性要与**祖先滚动盒**求交（`el` 的 rect ∩ `scroller` 的 rect），不要元素两两求交。上一次「侧栏文字重叠」的诊断就是这类假阳性，真实缺陷是另一件事（见下条）。
- **侧栏纵向布局契约（勿回退）**：默认展开时侧栏内容约需 1110px，而 `.sidebar` 是 `position: fixed` 撑满视口。900px 视口下的**真实**缺陷（R16 实测）：`.sidebar-core` 被 flex 压成 `clientHeight 14px`（`scrollHeight 191px`）→ 行文字被裁掉，只剩一条**无文字的橙色条**，三个目的地（技能·渠道 / 定时任务 / 查看用量）不可达；`.sidebar-foot` 落在 **y=1062** → **设置根本点不到**。
  **根因**是 flexbox 的自动最小尺寸规则：flex item 只要 `overflow` 非 `visible`，其 `min-height: auto` 就解析为 **0** —— 于是它成了唯一能吸收缺口的项并吸收了全部；而 `.sidebar-history-list` 是 `flex: 1`（`flex-basis: 0%`），在收缩分配里贡献恒为 0。
  **修法三条缺一不可**：`.sidebar-core { min-height: 96px }`（两整行）、`.sidebar-history-list { min-height: 160px }`、`.sidebar { overflow-y: auto }`（原先 `hidden` 让溢出**不可达**）。
  已由 `webui/src/assets/sidebar-layout.contract.test.ts` 钉住（4 条断言，做过反向验证：`min-height` 改回 0 → 测试变红）。修后实测：rail clientHeight **14 → 96**、recents 仍保 2 行可见、`footReachable: true`、`.sidebar` scrollHeight 1126 / clientHeight 900。
  ⚠️ 该测试从磁盘读 `base.css` 而**不是** `import 'base.css?raw'` —— 后者在本仓库解析出来是断言看不见的内容（4 条断言全拿到空规则体）。测试还必须**先剥掉 CSS 注释**：这些规则的长注释里引用了被守卫的声明原文（如散文里的 `overflow: hidden`），否则会因为一句注释而误红（实测踩过）。
- **🔴 已跟踪状态（index/HEAD）严重落后于工作区 —— 这是当前头号发布阻塞**（R17 实测）：
  用户自 Round 10 起要求「不提交」，于是历次冲刺的成果**全部只存在于工作区**，`git status` 有 **1683** 条脏记录。
  用「忠实导出索引」的方式模拟了一次真实 clone（`git checkout-index -a -f --prefix=…`，导出 9015 个文件），
  一个新人 clone 下来会**完全拿不到**这些已完成的修复：
  | 检查项（在 clone 导出里 grep） | 结果 |
  |---|---|
  | core 的 `Build` 意图（R16） | **不存在** |
  | 网关 501（R13） | **不存在** |
  | webui 侧栏 floor（R16） | **不存在** |
  | `webui/src/stores/activeMode.ts` | **文件缺失** |
  | `pyengine` 的 pytest `pythonpath`（R14） | **不存在** |
  | `pyengine/tests/__init__.py`（R14） | **文件缺失** |
  | `scripts/check-version-sync.py` | **文件缺失** |
  | `docs/internal/README.md`（R14） | **文件缺失** |
  | `design/tokens.json`（tokens 门禁要读它） | **文件缺失** |
  | **整个 `.github/`（`git ls-files .github` = 0）** | **全部缺失 → 公开仓库上 GitHub 根本没有 CI** |
  | **根目录全套社区健康文件**：`LICENSE` / `CONTRIBUTING.md` / `SECURITY.md` / `CODE_OF_CONDUCT.md` / `README.zh-Hans.md` / `.gitattributes` | **全部 untracked** |
  ⚠️ 也就是说 objective 第 2 项（LICENSE、CI/CD、Release 流程、社区健康文件）**只存在于工作区，不在仓库里**：
  一个 clone 下来既没有 LICENSE，也没有 workflow，`design/tokens.json` 缺失还会让 tokens 门禁直接红。
  另外 `.gitattributes` 未跟踪意味着它的行尾归一化**从未生效**；一旦真的提交，Git 会对约 60 个文件
  做 CRLF→LF 归一化（`git status` 的 warning 已列出清单）——这是预期行为，但要心里有数。
  后果：`verify.sh --all` 在真 clone 上必然红（连 `check-version-sync.py` 都没有；`gui/package.json` 的已跟踪版本是 **0.1.0** 且**没有** `typecheck`/`lint`/`test:unit` 脚本 → `--gui` 四步会以 `Missing script` 直接失败）。
  **解除方法只有一条**：`git add -A`（**暂存不是提交**；`git reset` 可完全回退）。暂存之后再用同样的
  `checkout-index` 导出跑一遍门禁，objective 里的「新人 clone→跑起来」才第一次**可验证**。
  ⚠️ **不要只暂存一部分**（例如只 `git add gui`）：这会让索引内部自相矛盾（暂存的新 `gui/package.json`
  配着过期的 `core/Cargo.toml` 版本号），比「整体落后」更糟。
- **构建快照目录绝不能入库**（R17 修复）：索引里曾躺着**三份过期的整包构建产物**，共 **594 个文件 /
  24 MB**（2026-08-11 快照）：`gui/src-tauri/webui/dist_old_202608110032`、`dist_old_202608110146`、
  `dist_opensquilla_upstream_202608110033`（含多份重复的 KaTeX 字体等二进制）。
  它们在工作区里**早已被删除**，只在索引里残存。根因是 `.gitignore` 只写了 `gui/src-tauri/webui/dist`，
  匹配不到 `dist_old_*` / `dist_opensquilla_upstream_*`。已改为 **`gui/src-tauri/webui/dist*/`** 并用
  `git rm -r --cached` 移出索引（`--cached` **不删磁盘文件**；实测 `gui/src-tauri/webui` 前后都是
  285 文件 / 24.13 MB，那 285 个是当前构建产物 `dist/`）。
  ⚠️ 判据小坑：`git check-ignore -v <目录>` 对**磁盘上不存在**的目录可能不返回匹配（git 无法确认它是目录），
  要测就测目录**里面**的文件路径。
- **`git add -A` 之前必须先把机器状态挡掉**（R18 审计 + 修复）。审计方法是
  `git status --porcelain -z -uall`（**必须 `-z`**：非 ASCII 路径经 PowerShell 管道会被 C-quote 成
  `"\345\274\225.md"` 之类，`Test-Path` 直接报 Illegal characters）。审计发现 **597 个未跟踪文件里有 41 个
  是机器状态，会被一并提交**，已补 4 处规则 + 1 处 untrack：
  1. **`*.tmpdir/`** —— 原有的 `**/.dsh-*.tmpdir` **匹配不到真实命名**：实测残留是
     `.<文件名>.<pid>.<uuid>.tmpdir/`（如 `core/src/kb/.project_graph.rs.30984.<uuid>.tmpdir/`），
     `git check-ignore` 返回 **exit 1**，于是 9 个残留长期是「未跟踪且未忽略」。
  2. **`*.db` / `*.sqlite` / `*.sqlite3`** —— 运行期桥接库（`workbench/**/core_bridge.db` 等）。
     索引里原本有 **0 个** `*.db`，所以整类忽略不会误伤已跟踪文件。
  3. **`/scripts/qa/`、`/scripts/launch/`** —— 一次性 QA 探针（25 个 + 截图 + 结果 JSON）与中文名个人启动器；
     `scripts/README.md` 早已声明它们不属于仓库，这里只是把该声明落实成规则。
  4. **被跟踪的运行期日志 `core/.marguerite/security-audit/skill_block.jsonl`**（51 KB）：
     `core/src/security/skill_scan.rs::log_skill_block` 是「尽力而为地追加一行 JSON」，
     **每跑一次测试就追加一行**，于是仓库每次测试后都变脏。代码里没有任何地方读取它 →
     `git rm --cached` 移出索引（文件保留在磁盘）。同时把 `/.marguerite/` 改成 **`**/.marguerite/`**：
     原规则锚定仓库根，**漏掉了 `core/.marguerite/` 这份嵌套的**。
  修后：未跟踪 597 → **556**，且**机器状态残留为 0**；所有正当源码（`LICENSE`、`.github/**`、
     `gui/src/**`、`pyengine/tests/__init__.py`、`design/tokens.json`、`workbench/**` 等）仍在 add 集合里。
  **对账口径**（`-z -uall`）：1942 条 = `??` 556 + ` M` 266 + ` D` 504 + `D ` 594（本轮暂存的产物删除）
    + `R ` 9（R14 的 `git mv`）；`git diff --name-only` 的 772 = 504 + 266 + 2，与 porcelain 完全一致。
  ⚠️ 解析 `-z` 输出时注意：**重命名条目会多带一个 NUL 字段（原路径）**，naive `split('\0')` 会把它当成
  新条目，表现为 `sc` / `PH` / `Se` 这类不存在的 XY 码（分别是 `scripts/`、`PHASE_`、`Security_Audit_` 的头两字符）。
- **⚠️ `gateway::methods` 的桌宠往返用例曾偶发假失败（R18 已修）**：现象是整套**并行**跑时
  `local_hub_contract_tests::pet_trigger_pet_roundtrip_increments_affection` 断言
  「pet.status 应读到同一持久化好感度」失败（2535 passed / 1 failed），而 `--exact` 单独跑**必过**（0.00s）。
  根因是**共享可变状态**：该模块的 `ctx()` 原先用 `OnceLock<Mutex<PathBuf>>` 让**同进程所有用例共用同一个
  临时目录**（按 pid 命名），而每次 `ctx()` 都会先删掉目录里的 `pet_state.json`。于是「+1 之后重新读」
  那一步可能刚好被兄弟用例的删除插进来 → `load_store()` 回落到默认值 0 → 断言失败。
  修法：`ctx()` 改为按调用递增的**唯一**目录（`AtomicUsize` + pid），用例之间不再共享持久化状态。
  验证：修后连跑 3 次整套，**2536 passed / 0 failed**（3/3）。
  教训（与 `events/product.rs` 同类）：**任何「写文件再读回来」的用例都必须有自己的路径**；
  `set_config_dir_override` 是**线程局部**的，所以覆盖必须在每个用例线程内设置。
- **🧾 单元测试曾把夹具事件写进用户的 `~/.marguerite/usage.db`（R64 定位并修；前端因此显示假指标）**：
  `usage::init()` 是**进程级** `OnceLock`（`LEDGER`），被 chat / engine / gateway / TUI 多处入口调用 ——
  只要同进程里有一个用例走到那些入口，台账就被点亮，之后**所有** `record()` 都写进**真实**用户库，
  包括 `record_is_noop_without_init` 这个本意是「未初始化就该 no-op」的夹具。
  真机库实测（2026-09-28）：`never-initialized-model` **567 行**、`session=''` 的空归属事件 **6930 行 / 680677 token**。
  而前端在**没有会话键**时仍拿空串去问 `usage.status`，网关按「一次合法过滤」处理 → 返回那一桶，
  于是聊天页脚注把网关级垃圾渲染成「本会话 token 680677 · 轮次 6930」（全新会话不可能有 6930 轮）。
  ✅ 修法：`ledger_path()` 在 `cfg(test)` 下落到 `%TEMP%\marguerite-usage-test-<pid>.db`（用户目录永不打开）；
  前端在 `sessionKey` 为空时**不查**该 RPC，且脚注只在**本会话真有用量**（或本轮实测到首 token）时出现。
  证据：全量核心单测 2784 passed / 0 failed 之后，真实库 **rows=9915 / max_id=9915 与跑前完全一致**，
  同时 `%TEMP%` 出现 `marguerite-usage-test-<pid>.db`；守卫 `ledger_path_is_never_the_real_user_database_under_test`
  在变异（路径改回真实目录）后精确报红。
- **🎨 `MeshToonMaterial` 的渐变贴图有一半是死区（R59 定位并修；「桌宠发灰/发脏」的真根因）**：
  three r185 的 `getGradientIrradiance` 取样坐标是 `coord = vec2(dotNL * 0.5 + 0.5, 0)`，而 `dotNL`
  在光照循环里**已被 `saturate`** —— 于是**受光面（dotNL ∈ [0,1]）只可能落在 coord.x ∈ [0.5, 1]**，
  贴图前半段永远采不到。旧贴图是 **3 texel**，受光面只能取到 texel#1/#2（`0xee` 与 `0xff`）：
  **LOW 档（`0xc6`）对受光面是死代码，可见明暗差只有 6.7%** → 白色部件平坦、无体积感。
  这也解释了历次排查里那个费解结果：**「改 LOW 档位 → 采样像素 Δ=0」**（受光面根本采不到那一格）。
  另一条同样关键的着色器事实：`RE_Direct_Toon` 里**没有独立的 N·L 因子** —— 渐变取样值**替代**了 N·L，
  所以曝光估算必须写成「档位(dotNL) × 强度 / π」，按 `dotNL × 强度 / π` 算会算不准（旧守卫就是这么错的）。
  ✅ 修法：贴图宽度取 **6**（偶数）、可达三档放**后半段**（`PET_TOON_RAMP_WIDTH` / `toonRampBytes`），
  可达区间由 0.067 变成 `0xc6→0xff` = 0.224（LOW 仍保持仓库既有的 0xc6「不显脏」结论）；
  兽形 key 由 3.30 降到 1.90（旧值在卡通口径下让主光正对面 = **1.371**，几乎整个可见半球裁平）。
  **实测（隔离网关 + Playwright；用「隐藏画布前后差集」隔离宠物像素，两次独立试验 IoU ≥ 0.93 才采信，
  并带 `lights=none → 全黑` 的有效性校验）**：
  | 皮肤 | 修前 中位 sRGB / ≥235 占比 / 线性中位 | 修后 |
  |---|---|---|
  | 橘猫小工（beast） | 255 / **75.4%** / 1.000 | **221 / 30.9% / 0.72** |
  | 雏菊精灵 | 229 / 42.4% / 0.788 | 220 / 37.3% / 0.72 |
  | 樱花精灵 | 221 / 35.9% / 0.727 | 213 / 27.6% / 0.669 |
  | 薰衣草精灵 | 202 / 34.3% / 0.596 | 195 / 24.9% / 0.55 |
  | 四叶草精灵 | 228 / 34.6% / 0.781 | 217 / 28.6% / 0.702 |
  即**橘猫从「整只裁平的白团」回到与 4 款精灵同一档**，中间带（100–200）占比全面上升（猫 20.7% → 28%）。
  逐灯分解（同一隔离法，只让一盏灯进场景）：`hemi 0.079 / key 0.914 / fill 0.162 / rim 0.009 /
  bottom 0.045 / none 0.000`（线性中位）——「none 全黑」即遮罩有效性的自证。
  守卫（都在 `webui/src/components/pet/petSpec.test.ts`，均实测过有牙）：
  ①「受光面可达档位覆盖 LOW→MID→HIGH」（旧 3 texel 布局必红）②「单盏主光 ≤ 0.95」（3.30 必红）
  ③「兽形可见半球裁切 ≤ 15%」（旧 28.2% 必红）④「受光面明暗差 ≥ 0.20」。
  ⚠️ **旧守卫漏掉它的原因（可复用的教训）**：它们只在 `[0,0,1]` **一个法线**上取值，且用**错误的曝光公式**
  （按 dotNL 而非按档位）——于是「主光正面被裁平」这种大面积缺陷完全落在盲区之外；
  而且两条旧阈值（`LOW ≥ 0.6` 与「不得过曝」）在兽形上**互相矛盾、无解**（扫描实测：要 LOW≥0.6 需 key≈3.10，
  那时裁切已 24.1%）。**单点采样 + 形式错误的模型 = 恒绿的守卫。**
  ⚠️ 已知差距（未做）：曝光模型是**白反照率 + 白灯的上界**，对以彩发/彩翼为主的人形 4 款皮肤偏松
  （模型裁切 49%，实测「恰为 255」仅 3.7~15%）——故紧界只设在兽形，人形只设硬天花板（<1.25）。
- **🔑 「全新 clone 能不能跑」要用「提交集快照」验证，而不是等用户提交**（R19 方法，可复用）：
  `git ls-files -co --exclude-standard -z` 就是 `git add -A` 会暂存的集合（已跟踪 + 未跟踪但未被忽略）。
  把它**拷到一个临时目录**（NUL 分隔读取，否则非 ASCII 路径会被 PowerShell 破坏），就得到「提交后的 clone 长什么样」，
  **完全不需要碰索引**。然后在那个目录里 `npm ci` + 跑 `verify.sh --webui --gui --pyengine`。
  本轮实测：`D:\commit-sim` = **8472 个文件 / 105.7 MB**，58 项门禁必需输入**全部存在**、17 个顶层目录计数**逐一相符**。
  这个方法一次就找出两个**本机门禁永远看不见**的真实缺陷（见下面两条）——因为本机 `node_modules` 早已装好，
  「能不能从干净状态装起来」从来没被验证过。
  ⚠️ 一个自检陷阱：`commit-set` 计数**包含**已跟踪但工作区已删除的文件（它们会作为删除提交），
  比对目录计数时必须只算磁盘上真实存在的文件，否则每个目录都会「少」掉自己的删除数（本轮差点误判成 4 处不一致）。
  **R25 复测 + 加强（这是 objective 第 4 项「clone→跑起来」的当前证据）**：
  | 量 | 实测值 |
  |---|---|
  | 索引（今天 clone 到的） | **8417** 个文件 |
  | `git add -A` 会写入的条目 | **8973**（其中 **504** 是「已跟踪但工作区已删」，提交后是**删除**） |
  | 提交后真正落地的文件 | **8469** |
  | 索引里的 `.rs` | **312** / 工作区 **443** —— 即今天的 clone **落后 131 个 Rust 源文件** |
  三条结论（都是本轮实测，不是推断）：
  1. **索引的模块图是闭合的**：把索引 `git checkout-index -a -f --prefix=…` 导出后逐文件解析 `mod X;`（含 `#[path]`、并**必须跟踪 inline `mod tests { … }` 的嵌套**），**0 个无法解析**。所以今天的 clone 是**陈旧**，不是**编译坏了**。
  2. **提交集与「能编译的工作区」源码完全一致**：`core` 438 / `tui` 1 / `gui/src-tauri` 3 个工作区 `.rs` 与提交集**逐一相等**；9 个 `include_str!`/`include_bytes!` 目标全部存在且在提交集内；17 项编译/发布必需件（含 `Cargo.lock`、`rust-toolchain.toml`、`build.rs`、两个 lockfile、`LICENSE`、`.gitattributes`、**两个 workflow**）**全部在提交集内**。→ 门禁那 2538 个通过的测试跑的就是这同一批源码，所以「提交后能编译」有据可依，无需再付一次 20 分钟的全新构建。
  3. ⚠️ **解析 `mod` 时不要相信「按文件目录解析」的朴素实现** —— 本轮我的第一版脚本就是这么写的，于是把 `core/src/lib.rs` 里 `#[cfg(test)] pub mod tests { pub mod mock_http; }` 判成「缺 `core/src/mock_http.rs`」，得出**「今天的 clone 编译不了」的错误结论**。真实文件在 `core/src/tests/mock_http.rs`（inline 模块改变了基准目录）。**是打开源码核对才发现的**；同理，统计大括号前必须先剥掉注释与字符串字面量（`format!("{}")` 之类会把深度算歪）。
- **`gui/package-lock.json` 曾与 `package.json` 严重脱节**（R19 修复，且是**门禁盲区**）：
  实测 `npm ci` 在 `gui/` 直接失败 —— `lock file's @eslint/js@10.0.1 does not satisfy @eslint/js@9.39.5`、
  `Missing: eslint@9.39.5 from lock file`…… 共 **85 条 Missing**，整棵 eslint 依赖树都不在 lockfile 里。
  **为什么长期没人发现**：`verify.sh` 的 lockfile 自洽检查**只覆盖 webui**（`cd webui && npm run deps:lockfile-sync`），
  `--gui` 那一层根本没有对应检查；而本机 `gui/node_modules` 是早就装好的，四项 gui 检查因此**从不触发 `npm ci`** ——
  于是**本机全绿、全新 clone 上四项 gui 检查连启动都做不到**（`tsc is not recognized`）。
  修法：`npm install --package-lock-only` 重建 lockfile（221 → 306 个 packages），给 `gui/package.json` 补
  `deps:lockfile-sync` 脚本，并在 `verify.sh` 里把该检查**拆成 webui/gui 两个独立项**（`--all` 16 → **17 项**；R27 加入密钥扫描、文档链接校验、README 事实校验后为 **20 项**；R28 加入 workbench 两项后为 **22 项**）。
  已验证负向：往 `gui/package.json` 注入 `left-pad` → 新检查 **FAIL**、闸门 exit 1。
- **`axe-core` 被 import 但从未声明**（R19 修复，同一类盲区）：`gui/src/__tests__/a11y.axe.test.tsx` 用了
  `axe-core`，而 `gui/package.json` 里没有它 —— 本机因为 `node_modules` 里残留着 4.13.0 所以 typecheck 过，
  全新 clone 上则 `error TS2307: Cannot find module 'axe-core'`。已补为 devDependency `^4.13.0`。
  **系统性排查手段**（值得固化成脚本）：扫 `src/**` 里所有裸模块说明符，与 `package.json` 的
  `dependencies/devDependencies/peer/optional` 求差集。实测结果：**gui 1 个（axe-core）、webui 0 个**。
  ⚠️ 扫描器必须把 `@/xxx` 这类 **tsconfig/vite 路径别名**排除，否则 webui 会报出 15 个假 "undeclared"
  （`@/components`、`@/stores`… 全是别名）。
- **⚠️ i18n 有两个盲区，其中一个已在本轮封堵**（`webui/scripts/check-i18n.mjs`）：
  1. **已封堵：源码引用了「所有字典都没有」的键**。原来该脚本**只做 locale 互比**，所以「6 个语言都没有这个键」
     是它结构上看不见的 —— 而 vue-i18n 解析不到键时会**渲染原始键字符串**。实测因此有 **24 个键**在界面上直接显示成
     `settings.ext.collabSsh.step1` 这类文本（其中 15 个是整块「企业协同 · SSH」指南，另有
     `chat.expand` / `chat.collapse`、`chrome.modeBadge` / `chrome.modeMenu`、`shared.cmdp.actionModeSwitch`、
     `chat.promptCacheKeepalive.cacheHits(.Hint)`、`chat.planRun.parallelSimulated`）。
     已加**源码扫描**：抓 `t('字面量')` / `$t(...)` / `te(...)` 与 en 字典求差集，**有未定义键即 FAIL**（并打印键名与文件）。
     实测 **2651 个字面量键、0 个未定义**；反向验证：注入 `t('definitely.not.a.key')` → 立刻 FAIL/exit 1。
     ⚠️ 只抓**整体字面量参数**（`t('k')` / `t('k', {...})`）；拼接、模板字符串、动态键一律跳过，否则全是假阳性。
     ⚠️ 反方向（en 里有、源码没用）**只统计不失败** —— 实测 1211 个键是动态拼出来的（`channelStatus.${x}`、nav 标签），
     把它们当缺陷会立刻变成噪音。
  2. **未封堵：模板里硬编码的中文**。实测 `<template>` 中**未走 `t()` 的 CJK 文案共 538 行、分布在 37 个 `.vue`**，
     例如 `SidebarConversations.vue`（`运行中`）、`ArtifactInlineEditor.vue`（13 行）、`RightSidebar.vue`（24 行）、
     `BudgetPanel.vue`（15 行）、`DesktopPet.vue`（9 行）。**切到 ja/fr/de/es 时这些位置仍然是中文**，
     而门禁只会报「full parity, no English leakage」。
     本轮只顺手把紧邻的 `ModeBadge.vue` 菜单三处（`运行时模式` / `当前：` / `打开智能中枢（完整模式管理）`）改成 `t()`，
     **其余 535 行未动** —— 那是一次独立的、需要逐条翻译的工程。
     建议下一步用**棘轮（ratchet）**方式防新增：把当前的（文件 → 行数）基线写进守卫，只允许下降不允许上升。
- **tokio::test TCP 端到端**：客户端必须用 `tokio::net::TcpStream`（异步），**绝不**用同步 `std::net::TcpStream`——后者阻塞 runtime worker，单核环境致 accept 饿死、连接超时（错误码 10060）。
- **`rm` 被 safe-delete 钩子拦截且 fail-closed**：本环境 `rm` 经 genie-trash 包装，相对路径会被拒、且失败时**拒绝删除（文件保留）**。确需删除请用绝对路径，或直接用托管 Python `os.remove()` 真删；脚本/CI 删除不可用裸 `rm`。
- **`std::thread::JoinHandle` 无 `join_timeout`**：有界网络探测用 `tokio::time::timeout` + `tokio::net::TcpStream::connect`，不要用 `std::thread::spawn().join_timeout()`（std 没有此方法）。
- **子命令后 `--flag` 已被透传**：`cli/argv.rs` 对子命令 handler 消费的旗标（deploy/model add/browser/dispatch/subagent 的 `--target`/`--id`/`--out`/`--timeout`/`--reflect` 等）做白名单透传进 `sub_args`；真正未知的 `--flag` 仍会停止收集并报「未知选项」。
- **`serve --help` 误启动 listener**：旧二进制若不含 Phase 5 代码会把 `--help` 当启动。确认二进制已重建（`argv.rs` 正确设置 `subcommand_help` 后 `run()` 会先拦截帮助再路由）。
- **桌面冒烟必查「stale core」**：`marguerite-gui` 静态链接 core。改 core 后只重编 `marguerite-core` 不会进入桌面进程——必须先 `cargo build -p marguerite-gui`（重链接新 core）再**重启已运行的桌面进程**，否则桌面网关仍是旧行为（实测：pet.triggerAction 缺 `ok:true` 的修复未进 GUI 二进制，桌面 E2E「顺毛抚摸」静默失败；重链接重启后 7/7 全过）。冒烟前确认 `target/debug/marguerite-gui.exe` 的 LastWriteTime 晚于 core 改动。
- **`cargo tauri dev` 死循环（dist 监视打架）**：本环境 `tauri.conf.json` 的 `frontendDist = "webui/dist"`（解析为 `gui/src-tauri/webui/dist`），`cargo tauri dev` 时 tauri-cli 与 GUI 侧 vite 同时监视该目录，任一变更触发对方重建 → 无限「Rebuilding application...」循环，永远到不了 app 启动。桌面冒烟请直接用 `cargo build -p marguerite-gui` + 运行 `target/debug/marguerite-gui.exe`（真机验收路径，见 docs/development.md 冒烟清单）。
- **Windows 上运行中的 GUI 二进制被文件锁**：网关/桌面进程占用的 `marguerite-core.exe` / `marguerite-gui.exe` 无法原地重编（`拒绝访问 os error 5`）。先停进程再 `cargo build`。
- **⚠️ `Get-Process name1,name2 | Measure-Object` 会漏报**（本环境实测踩过两次，代价是两轮「门禁变红」的误判）：
  用它判断「还有没有残留进程」**不可靠**，会得出 `0` 却仍有进程在跑。请改用显式逐个查询：
  ```powershell
  Get-Process -Name marguerite-core,marguerite-gui,marguerite-tui -ErrorAction SilentlyContinue |
    Select-Object Id,ProcessName,StartTime,Path
  ```
  残留的 `marguerite-core` 会锁住 `target/debug/marguerite-core.exe` 与
  `target/debug/deps/marguerite_core.exe`，使 `cargo build --workspace` 报
  `failed to remove file ... 拒绝访问 (os error 5)` / `LNK1104: 无法打开文件` ——
  **这是环境问题，不是代码缺陷**。跑闸门前务必先确认没有残留进程；
  注意 `cargo test` 自身可能留下子进程，所以测试步跑完也要复查一次。
  另注：`Get-Process -Name X | Stop-Process -Force` 有时会静默失败，必要时再查一遍确认。
  ⚠️ **最常见的持锁者就是常驻 daemon 本身**（R60 实测：`target/debug/marguerite-core.exe gateway --port 18801` 一直在跑，
  `--all` 的 24 项全绿、只有最后一条 `cargo build --workspace` 报 `failed to remove file … 拒绝访问 (os error 5)`）。
  所以**跑全量闸门之前要先停 daemon，跑完再把它起回来** —— 这与「daemon 常驻」不冲突，是同一件事的两个阶段。
  `cargo build -p marguerite-core --bin …` 与 `cargo test` 可能**恰好**不改写那个 exe 而侥幸通过，
  只有真正触发 relink 的 `--workspace` 才会红 —— 别把「前几条过了」当成没被锁。
- **依赖安全审计 + lockfile 自洽（两条新增本地门禁）**：`webui/package.json` 新增两个脚本，
  已接入 `verify.sh` 的 `--webui` / `--gui` 与 `--all`（`--all` 现为 **22 项**检查）：
  - `security:audit`（`npm audit --audit-level=moderate`）—— console 会渲染模型输出，所以消毒器
    （`dompurify`）与 id 生成器（`nanoid`）的公告属**真实产品风险**。本仓库此前**没有任何审计步骤**，
    因此静态携带 4 条公告无人可见（3 moderate + 1 high：`dompurify` XSS、`nanoid` 死循环、
    `vitest`/`@vitest/mocker` 路径穿越）。已 `npm audit fix` 清零，并实测 `npm ci` 与全量套件
    在升级后的版本上仍全绿。
  - `deps:lockfile-sync`（`npm ci --dry-run`）—— `npm ci` 要求 `package.json` 与 lockfile 完全自洽，
    否则 **CI 第一步就报错**（`can only install packages when … in sync` / `Missing: <pkg>@<ver> from lock file`）。
    **`npm audit` 看不见这类漂移**（它审的是 lockfile 解析出的依赖树，不是「声明 vs lock 是否一致」），
    所以两者互补、都要跑。实测负向验证：声明 `left-pad` 但不进 lockfile 时，audit PASS 而 lockfile-sync FAIL，
    闸门整体 exit 1。
  ⚠️ **`gui/` 仍有 4 条公告未处理**：修它们需要 `npm audit fix --force`（会把 `vite` 升到 8.3.0，属破坏性变更），
  而 `gui/` 的 React 前端**在产品界面中不可达**（见 §1），故未强行升级 —— 这条待「接线或移除 `gui/src`」的决策一并处理。
- **npm lockfile 的版本号一致性**：`npm ci` 会校验 lockfile 与 `package.json` 是否自洽。
  本仓库实测出现过「`node_modules` 与实际 lockfile 不一致」（安装的 `dompurify 3.4.12` vs lockfile 声明 `3.4.15`），
  所以**新增或升级依赖后请跑一次 `npm ci` 而不是只 `npm install`**，两者对 lockfile 的处理不同。
- **⚠️ `python3` 可能是「没有依赖」的隔离解释器（门禁解释器解析已改为能力感知）**：本机实测
  `which python3` → `/c/Users/Administrator/.workbuddy/binaries/python/versions/3.13.12/python3`
  （WorkBuddy 托管的隔离解释器，实测**只有 `pytest`**，缺 `anyio`/`httpx`/`structlog`/`typer`/`rich`/
  `pydantic`/`websockets`/`tomli_w`/`openpyxl`/`reportlab`/`yoyo`），而 pyengine 的依赖装在系统
  `C:\Python314\python.exe`（实测上述 11 个模块 + `pytest` 全部可 import）。
  ⚠️ 旧版 `verify.sh` 只做 `command -v python3 → python`，于是**会挑到隔离解释器**：pyengine 三层在
  import 阶段整层 collection error（实测 pyengine 单测 39 个、`test_gateway` 187 个 ERROR），看起来像
  产品缺陷，其实只是解释器选错 —— **同一条门禁两次跑出不同结果**（R1 红 / R2 绿）就是这个症状。
  `verify.sh` 现已改为**能力感知**：候选按 `$MARGUERITE_PYTHON` > PATH `python3` / `python` >
  `C:/Python314|313|312` > `%LOCALAPPDATA%/Programs/Python/Python3*` 枚举，只有能**一次** `import`
  全部所需模块的才算合格，并把**选中的解释器**打进输出（`-- 选中的 Python 解释器: …`）。
  `$MARGUERITE_PYTHON` 被显式设置却不合格时**响亮 `[FAIL]`**（绝不静默回落）；全都不可用时记
  `[SKIP]` 并**逐候选列出缺哪些模块**（§8：跳过 ≠ 通过）。
  手工跑 pyengine 层请用 `MARGUERITE_PYTHON` 钉死：
  ```bash
  MARGUERITE_PYTHON="C:/Python314/python.exe" bash scripts/verify.sh --pyengine
  ```
- **⚠️ WorkBuddy 的 Python 启动链注入了 `safe-delete` 垫片，会污染「删除」语义**：本机 Python
  启动时加载 `sitecustomize`
  （`…\WorkBuddy\resources\app.asar.unpacked\cli\vendor\shim\sitecustomize.py`），它拦截所有删除并
  改投回收站。有两条副作用会**让被测代码看到错误的删除结果（不是产品缺陷）**：
  1. **批量删除 >5000 个文件要审批** → 日志出现 `[safe-delete][SAFE_DELETE_BULK_CONFIRM_REQUIRED]`
     （实测 `count=6031` / `7843`），pytest 的汇总行被吃掉、进程 `rc=1` → 门禁报「无 pytest 汇总行」。
     受影响：`test_sandbox`（6031 文件）、`test_ci`（7843）。
  2. **单文件删除 fail-closed** → 日志出现 `[safe-delete][SAFE_DELETE_FAIL_CLOSED]`
     （`{"reason":"trash-failed","detail":"[Errno 53] 找不到网络路径。: '\\?\C:\…\.opensquilla.profile-consolidation.<uuid>.staging'"}`）
     → 被测的原子/恢复代码把本该 `consolidated` 的结果判成 `blocked` / `profile_consolidation_failed`
     → 实测 `test_recovery` **8 个失败**。
  ✅ 关闭它（**仅本次运行**）后实测归零：`CODEBUDDY_SAFE_DELETE_ENABLED=0` + `C:\Python314` →
  `test_recovery` **rc=0, 444 passed / 9 skipped**、`test_sandbox` **rc=0, 1746 passed / 349 skipped**，
  两次 `safe-delete` 干预计数都为 **0**。跑 pyengine 层前：
  ```bash
  export CODEBUDDY_SAFE_DELETE_ENABLED=0   # 仅本机 WorkBuddy 终端需要
  ```
  ⚠️ 该垫片**只在本机 WorkBuddy 会话里存在**，普通终端与 CI 都没有它 —— 所以**不要**把它写进
  `scripts/verify.sh`，只作为**运行期环境变量**（及本条文档条目）。


## 5. 贡献 workflow

1. 改动前先读 `docs/cli-reference.md`（CLI 契约）与 `docs/mcp.md` / `docs/development.md`，确保理解现状。
2. 改动 CLI 路由 / 子命令 → 同步更新 `argv.rs` 的 `SUBCOMMANDS`、`program.rs` 的路由注册与测试断言，并补 `docs/cli-reference.md` 对应小节。
3. 改动后运行 `bash scripts/verify.sh --clean --all`（发版加 `--release`），保证全绿、§2 的基线不回退；必要时更新本文件 §2 与 `README.md` 的基线数字。
4. **文档与代码冲突时以代码为权威，回改文档**——不要为了文档去虚构 CLI 动词（如 `mcp connect` 未实现就不应写进手册）；也不要为“凑齐文档示例”去新增未经验证的实现。
5. 改动 CLI 路由 / 文档示例后，运行 `python3 scripts/check-doc-cli-drift.py`，确保手册不再漂离代码（详见 §6）。

## 6. 文档/代码 CLI 漂移校验（CI 门禁）

`scripts/check-doc-cli-drift.py` 自动校验 `docs/*.md` 里出现的 `marguerite <sub>` / `marguerite <sub> <verb>` 是否都有真实 CLI 实现，防止手册漂离代码（本次优化即修复了 `mcp connect` 虚构、「`gui/` 是成员」两处漂移）。

- **Tier 1（FAIL）**：文档里的顶层子命令必须在 `argv.rs::SUBCOMMANDS` 或 `program.rs::route_table.register(...)` 中注册。
- **Tier 2（FAIL）**：对会派发嵌套动词的 handler（如 `mcp` 读 `sub_args`），文档里的嵌套动词必须在该 handler 体内作为字符串分支出现。

```bash
python3 scripts/check-doc-cli-drift.py   # exit 0 = 无漂移；exit 1 = 漂移（打印明细）
```

CI 接入（`docs` job，checkout 之后的**五个**纯标准库检查 —— 都只需要 `python3`，不需要 setup-python）：

```yaml
- name: Doc/Code CLI drift check
  run: python3 scripts/check-doc-cli-drift.py
- name: Version consistency across the four front ends
  run: python3 scripts/check-version-sync.py
- name: Secret scan of the shipping file set
  run: python3 scripts/check-secrets.py
- name: Doc link check (first-party docs)
  run: python3 scripts/check-doc-links.py
- name: README claims vs verify.sh
  run: python3 scripts/check-readme-claims.py
```

实现要点：脚本只扫描 fenced code block 与 inline `code`（忽略散文）；flag（`--health` 等）与非派发参数（session id / 路径）不检查，避免误报。根因是 CLAUDE/AGENTS 类文档与代码易漂移，本脚本把“对账”固化成可重复的门禁。

## 7. 第三方集成层（third_party/）

> 🔴 **`/third_party/` 被 `.gitignore` 整个忽略（实测 `git ls-files third_party` = 0），所以它不在任何 clone 里。**
> 本机约 **121 GB**：`external/` 101.6 GB / 230,560 文件、`local/` 18.9 GB、`harness/` 705 MB、`computer_use/` 461 MB。
> 忽略规则本身是对的 —— 那是上游仓库快照，不是本项目源码。
> **但 §7.1 / §7.2 描述的那些「本项目产出」的小文件（17 个清单 + 5 个技能 + 校验脚本）也埋在同一个目录里**，
> 于是**本节列出的产物默认不随仓库发布**：新人 clone 下来一个都看不到。
> 是否把它们移出 `third_party/`（例如把 5 个技能放进**已发布**的根 `skills/`）是**待决策项**，本轮未做 ——
> `.gitignore` 取反规则很容易连带把 GB 级快照放进来，风险高于收益。

`third_party/` 是**参考 + 适配层**，不编译进 cargo workspace。`third_party/manifest.json` 记录 **28 个**二开仓库
（**18 harness + 10 computer_use**，全部 `status: cloned`；实测 `counts` 与 `repos` 列表自洽，均为 28/18/10。
⚠️ 本节此前写「27 个（18 + 9）」—— 错的，已按实测更正）。价值通过两条路径进入 Marguerite：

1. **模式 → Rust 实现**：`core/src/tools/computer_use.rs`（`ScreenPerception` 截图 / `InputControl` 键鼠 / `GuiGrounding` 视觉闭环）。**这一项确实随仓库发布**（在 `core/` 内且已被跟踪）。
2. **能力 → 清单 / 适配器**：harness 类项目以声明式 `PluginManifest` 接入既有插件系统（`core/src/plugins/manifest.rs`），computer use 类以外部 mcp/视觉适配器接入。**这一项的清单文件不随仓库发布**（见上面的红框）。

### 7.1 插件清单（harness → PluginManifest）

- 位置：`third_party/plugin-examples/*.plugin.json`，**实测 17 个**（⚠️ 本节此前写 18 —— 错的）。
  17 个**全部可解析且满足必填字段**（`name`/`version`/`description`）。
- ⚠️ 覆盖缺口：`manifest.json` 声明 **18** 个 harness 仓，而清单只有 **17** 个 → **有 1 个 harness 仓没有对应清单**。
  这属于「完整性缺口」而不是自相矛盾，校验脚本把它列在 REPORT 段（见 §7.3），不会误报成 FAIL。
- Schema：`core/src/plugins/manifest.rs::PluginManifest`（必填 `name`/`version`/`description`；可选 `capabilities`/`hooks`/`slash_commands`/`subagents`/`skills`/`dependencies`）。
- 接线：`PluginManifest::from_json(&text)` → `register_manifests(&mut PluginRegistry, &[manifest])`；`build_hook_registry(...)` 把 `hooks[].handler` 注入 L1/L2/L3 护栏。
- 索引与逐仓映射：`third_party/harness-manifests-index.md` + `third_party/harness/HARNESS_BRIEF.md`（同样不随仓库发布）。

### 7.2 JIT 工程纪律技能包（Skills）

- 位置：`third_party/skills/*.yaml`，**5 个**：`engineering-discipline` / `tdd` / `systematic-debugging` / `verification-before-completion` / `write-plan-first`。
- ⚠️ **R20 之前，其中 4 个是 0 字节空文件**（只有 `write-plan-first.yaml` 有内容），
  而 `write-plan-first` 又声明依赖那个空的 `engineering-discipline` → **依赖不可解析**。
  本轮按同一 schema 补齐了 4 个，现在 **5/5 结构合法**且依赖链闭合（校验脚本已验证）。
- ⚠️ **这 5 个技能在产品的技能加载路径上取不到**：`SkillLoader::new(skills_dir).load_all()` 只扫描**顶层** `skills_dir`
  （`doctor` 实测解析到 `…\AppData\Local\Marguerite\skills`，24 个技能），而**已发布**的根 `skills/` 里
  **没有**这 5 个名字 —— 它们只存在于被忽略的 `third_party/skills/`。
- Schema：`core/src/skills/registry.rs::Skill`（必填 `name`/`description`/`version`/`prompt_template`；可选 `author`/`intent`/`tools`/`dependencies`）。
- 来源：HARNESS_BRIEF 综合模式 #7（LeAgent Agent Skills v1.0 + superpowers-reasonix）。

### 7.3 校验（**本地检查 —— 不是 CI 门禁**）

```bash
python third_party/validate_integration.py   # 0 = 无不变式违例；1 = 有（REPORT 段仍必须阅读）
```

> ⚠️ **两点必须说清，因为本节此前两点都写错了**：
> 1. **它曾经是一个 0 字节的空文件**，而本节把它描述成「CI 门禁」并罗列了它会检查什么。
>    空脚本 `exit 0`，于是「永远绿、什么都没查」—— 正是 §8 明令禁止的**假门禁**。R20 已实现为真。
> 2. **它不可能成为 CI 门禁**：`/third_party/` 被忽略，脚本与它读的数据都不随仓库发布。
>    因此它**没有**接入 `verify.sh`，只是本机自检工具。

实现后的两级严重度（刻意分开，避免把「绿」误读成「完整」）：
- **FAIL（exit 1）**：不变式被破坏 —— `manifest.json` 的 `counts` 与自身 `repos` 不一致；
  清单不可解析或缺必填字段；**非空**技能文件不可解析或缺必填字段；技能声明的依赖无人提供。
- **REPORT（不影响 exit code）**：完整性缺口 —— 0 字节技能占位文件；声明了 harness 仓却没有清单。

**R20 实测**：`PASS`（0 条 FAIL）+ **1 条 REPORT**（18 个 harness 仓 vs 17 个清单）。
**已做反向验证**：删掉清单必填字段 → FAIL/exit 1；清空一个技能 → REPORT 增加而 exit 仍 0；
把依赖指向不存在的技能 → FAIL/exit 1；三次都正确复原。

⚠️ 本节此前写「当前环境未装 PyYAML，技能 YAML 用启发式校验」—— **已过时**：本机实测 PyYAML **6.0.2** 可用，脚本走严格解析。

### 7.4 升级路线（优先级，详见 INTEGRATION.md §5.4）

- **P0（已落地）**：JIT 工程纪律技能包 + harness 清单 + computer_use 三层重构 + 校验脚本。
  ⚠️ 「已落地」需要限定：**只有 computer_use 三层重构真的随仓库发布**（在 `core/` 内且已被跟踪）；
  技能包与清单都在被忽略的 `third_party/` 里 —— clone 拿不到，技能也加载不进产品（见 §7.2）。
- **P1**：扩展 `PluginManifest`（dependencies / 能力级 model）；自演化闭环（`harness/distill.rs` + `automation/cron.rs`）；fleet 模型路由（`multi_strategy.rs`）；P2P 团队（`agent/team.rs`）。
- **P2**：memory + distill 自动蒸馏；`tool_cycle` + `stream_loop` 的 DAG 并发 + 断点续跑；`events` + `bus` + tui/workbench 结构化 model↔UI 协议。

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
| `--all`（默认） | `--core` + `--webui` + `--gui` + `--pyengine` + `--workbench` + `cargo build --workspace` + clippy(workspace) = **22 项检查** | 日常全量 |
| `--clean --all` | 上述 + 先 clean core | **完成声明用此档** |
| `--release` | `--all` + `cargo tauri build` | 发版门禁 |

**跳过 ≠ 通过**：`verify.sh` 把跳过的检查记进独立的 `SKIPPED` 列表并在汇总里显式列为「未验证，不计入通过」。因工具缺失而静默变绿是本仓库明令禁止的伪完成，所以 `[SKIP]` 永远不会被计成 `[PASS]`。

**clippy 范围说明**：`--all` / `--fast` 用 `cargo clippy --workspace --all-targets -- -D warnings`（L3 零警告，含 tui）；`--core` 单独运行则用 `-p marguerite-core -p marguerite-gui` 以加速。若 `--workspace` clippy 因 tui 遗留警告变红，那是真实待办，须修复而非绕过；临时排错可用 `--core`。

**证据格式**：闸门输出含每条 `[PASS]/[FAIL]`、耗时、以及测试计数 / 退出码；声称完成时必须附该次运行输出或明确引用本次运行（与 §6 文档漂移门禁同理，把「对账」固化成可重复机械闸门）。
