<!-- 本文件原为 AGENTS.md §2，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
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
npm run check:architecture   # 13 项守卫（architecture/chat-security/colors/motion/radius/theme-contract/theme-scope/cross-surface-tokens/i18n/channel-catalog-i18n/rpc-contract/index-injection/splash-media）
npm run typecheck            # check:architecture + vue-tsc --noEmit
npm run test:unit            # Vitest 单测（当前 4902 passed / 0 failed / 496 files，2026-10-29 全量闸门复测）
npm run build                # typecheck + vite build + 产物校验（dist/）
```

> **统一闸门（推荐）**：上述命令已收口为 `bash scripts/verify.sh`（分层：`--core` / `--webui` / `--gui` / `--pyengine` / `--workbench` / `--all` / `--release` / `--fast`）。任何「完成」声明前运行 `bash scripts/verify.sh --clean --all`（发版加 `--release`），全绿方可声称。详见 §8。

- **测试基线（全部为 2026-10-29 `verify.sh --clean --all` 全绿（25/25，exit 0，汇总无 `[SKIP]`）的实测口径）**：`marguerite-core` lib **4259 passed + 0 failed + 24 ignored**（62.50s，`running 4283 tests`）；`webui` **4902 passed + 0 failed（496 files）**（此前 4230/4898、4226/4897、4868/490、4816/484、3509/331 均已过期）；`pyengine` 三层 —— `tests/unit` **823 passed / 0 failed / 7 skipped**（13s）、`tests/test_gateway` **2956 passed / 0 failed / 11 skipped**（180s）、**契约族** `tests/test_{ci,migrations,sandbox,onboarding,tools,recovery}` **4446 passed / 0 failed / 390 skipped**（294s，拆开为 `test_ci` 35+1s / `test_migrations` 59 / `test_sandbox` 1746+349s / `test_onboarding` 956+8s / `test_tools` **1205**+23s / `test_recovery` **445**+9s）；web e2e 本轮 **PASS**（棘轮基线 96，`webui/test-results/.last-run.json` = `status: passed`；确切通过数落在被 `tail -80` 截掉的输出里，未作新证据引用）。⚠️ **相对 2026-10-28 的三处增量都不是记忆检索整改带来的**：core +29 里有 25 条是三个新模块自带（`scoring.rs` 6 / `lexical.rs` 6 / `recall_probe.rs` 13），而 webui +4 与 `test_recovery` +1 来自**更早落在工作区**的改动（按文件 mtime 核对过），只是这次复测才第一次被计进基线 —— 别把它们当成「我这轮写绿的」。任何改动后不得回退；若基线变化，同步更新本文件 §2 与 `README.md`／`README.zh-Hans.md` 的「Quality gates」表（`scripts/check-readme-claims.py` 会比对中英两份的硬事实与闸门总数，但**不会**核对数字是否与现实一致 —— 所以「过期基线」只能靠人复测）。门禁运行前若 shell 带 `NO_COLOR=1` 需先移除（否则 2 个既有主题测试走单色回退失败，非代码缺陷）—— ⚠️ **2026-10-27 又踩一次**：给验证命令「顺手」加上 `NO_COLOR=1`，跑出 `4143 passed / 2 failed` 的**假红**，`env -u NO_COLOR` 重跑即 `4162 / 0 failed`；这两条失败从来不是缺陷。
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
- **CI 的 Python 侧车 job 是第 5 个（R24 新增），第 6 个是 workbench（2026-10-28 新增）**。此前 `ci.yml` 只有 `docs`/`rust`/`webui`/`gui` —— **全是 Rust 或 Node**，于是侧车那 758 个测试**在 CI 里一个都不跑**，而 README 却在宣传 `--pyengine` 门禁：一个对 1/3 代码库沉默的绿徽章。workbench 那条是同一类缺口的第二例：README 写着「除 web e2e 外每一层都在 CI 里跑」，而 workbench 的 71 个离线用例 + 版本对齐在 `ci.yml` 里 **0 引用**；补上后 `--workbench` 的 3 项（单测 / 版本 / 专家定义自洽）随 job 一起跑。
  该 job 调用的就是 `bash scripts/verify.sh --pyengine`（**刻意不复制 pytest 命令行**，marker 排除表因此只有一处定义、不会漂移；R58 起该层同时还跑 `tests/test_gateway`，见上文 R58 条目），并在跑之前先断言 `pytest` 可导入、`tests/unit` 存在 —— 因为 `verify.sh` 对缺前置只记 `[SKIP]` 且**仍然 exit 0**（§8「跳过 ≠ 通过」）。它还会跑一次 `scripts/check-pyengine-collection.py`（约 10s，0 collection error）。
  ⚠️ **workbench job 能在「没装 Rust」的 runner 上成立，靠的是 verify.sh 的一处修正**：脚本原先在解析参数之前就硬性要求 `cargo`（找不到即 `exit 2`），于是纯 Python 层也跑不了。现改为**模式感知**——只有参数里出现 `--core`/`--gui`/`--e2e`/`--all`/`--fast`/`--release`/裸 `--clean` 或无参数时才强制 `cargo`；`--workbench` / `--pyengine` / `--webui`（可带 `--clean`）允许缺失。判定真值表实测：`--clean --workbench`→可选、`--core --workbench`→必需、裸 `--clean`→必需。
  ⚠️ **R24 那次只做了静态验证**（诚实上限）：YAML 可解析、当时共 5 个 job、每个 `run:` 脚本过 `bash -n` 零语法错、heredoc 体过 `ast.parse`、依赖集解析出 45 项、`--pyengine` 当时只调度 `check_pyengine_tests`（R58 起为两项）。**GitHub Actions 无法在本机运行**，所以「这个 job 真的会绿」尚未被证实 —— 首次 push 时它是第一个该盯的地方（Python 固定在声明的下限 3.12）。workbench job 同样只有静态证据 + 本机 `verify.sh --workbench` 3/3 全绿（含在 `PATH` 里剔除 cargo 的模拟），**未在 GitHub runner 上跑过**。
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
  （**上面是 2026-09-25 的现场记录**：当时 97 例总数 = 95 通过 + 2 跳过。**当前口径是 99 例总数 = 97 通过 + 2 跳过**，
  棘轮基线 **96**，见 §4 R65 补记 —— 别把这里的 97 当成今天的通过数。）
  同时两个会话还会争抢 `target/debug/marguerite-core.exe`（daemon / e2e 网关持有它就 `cargo build` 失败）。
  ⇒ 判「红」之前先看**进程表与文件 mtime**，别默认是自己的改动坏了。

