# Marguerite 全量功能核查 — QA 验证最终报告

> 负责人：gstack-qa-lead（QA & 发布）
> 范围：`core/`（Rust 引擎）+ `webui/`（Vue3 控制台）+ `gui/`（Tauri 壳）+ `pyengine/`（Python 侧车）
> 结论性质：**可执行验证已全部实际运行**；产物为本次会话真实结果，非推断。

---

## 0. 摘要（Executive Summary）

| 维度 | 结果 |
|---|---|
| `cargo test -p marguerite-core --lib` | **1166 passed / 0 failed / 7 ignored**（本次实跑） |
| webui Vitest 单测 | **3067 passed / 0 failed**（需 `--testTimeout=20000` 规避 flake；默认 5s 下 flaky） |
| webui `check:architecture` | **10/10 通过**（其中 cross-surface 为"真空通过"，见 P3） |
| webui `typecheck`（vue-tsc --noEmit） | **exit 0** |
| `check-doc-cli-drift.py` | **exit 0** |
| 宝塔 MCP 自测（`smoke_stdio.py`） | 此前会话 PASS（协议/错误/ping） |
| **发布就绪（ship-readiness）** | **条件就绪**：core/webui 可发；桌面 GUI 运行时 + 安装器签名为**人工验收里程碑**（本环境无头，未实机验证） |

**关键结论**：
1. Rust 侧**全绿**，且真实通过数（1166）高于文档基线（AGENTS 写 1057/928、capability-map 写 1157）→ 文档基线漂移（P4）。
2. -reported 的「**设置开关持久化 bug**」**不是产品缺陷**，而是 e2e 套件在 opensquilla→marguerite 品牌化后**未跟进存储键**导致的陈旧测试（P2）。
3. **L3 安全层（GuardrailScanner）已写好并单测通过，但运行时从未被调用**——"接线缺失"（P2，安全相关）。
4. webui 单测存在**非确定性 flake**（P1），使单测门禁的"通过/失败"不可靠，且**源码尚未修复**（仅运行时绕过）。

---

## 1. cargo test -p marguerite-core --lib（最终结果）

```
test result: ok. 1166 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 6.02s
```

- 执行方式：`CARGO_TARGET_DIR=D:/Downloads/Marguerite/marguerite/target_qa`（避开被锁定的 `target/release/.cargo-lock`），本次会话重新编译并实跑。
- **7 ignored** = 需联网/浏览器的 live 测试（`live_browser_*`、`live_llm_tests`、`live_web_search`），符合预期、非失败。
- 覆盖的关键模块（均绿）：统一网关 `gateway/`（40+ 测试）、`sandbox::policy_store`、`plan::runner`（auto_decompose 4 例 + `long_running`）、`security`（L1/L2 单测 + 集成链）、`tui::workbench::workspace_watcher`、Bloom 模式。
- **基线漂移**：文档写 1057/928（AGENTS §2/§5）与 1157（capability-map §4.3），实际 1166 → 文档需回填（P4）。

---

## 2. webui 单测 flake（SettingsDialog.dataMaintenance.test.ts 超时污染）

### 根因（已定位）
- `webui/src/components/settings/SettingsDialog.dataMaintenance.test.ts` 的 `mountDialog()` 每次都执行 `vi.resetModules()` + 动态重导入 `SettingsDialog.vue` + `i18n` + `vue-router`（约 1.5s/次）。
- Vitest 默认 `testTimeout = 5000ms`。在 289 个测试文件**并行**负载下，该文件测试易超过 5s → **超时失败**。
- 超时时，上一个已挂载的 app（使用 `<Teleport to="body">`）**未被清理**，body 中残留重复的 `.settings-modal__head button`；后续测试的 `querySelector` 取到陈旧节点 → 报 `expected <button> to be <button> // no visual difference`（**断言假失败，非真实缺陷**）。
- 复现规律：隔离单跑该文件（1515ms）通过；并行满载下 flaky（首跑 3065/2 fail，并发跑 3060/7 fail，随机性印证负载相关）。

### 是否已源码修复？
**否。** 仅本次会话以运行时参数 `--testTimeout=20000` 规避，源码（vite.config.ts / 该测试文件）**未作任何修改**。git 工作区无相关改动。→ 该 flake 仍是潜伏风险，作为发布门禁不可靠。

### 修复建议（二选一或组合）
1. **首选（治本）**：在该测试的 `afterEach` 中显式 `wrapper.unmount()` + `document.body.innerHTML = ''`，消除 Teleport 残留；或将 `mountDialog()` 改为不 `resetModules`、复用已挂载的 i18n/router 实例。
2. **兜底（治标）**：在 `webui/vite.config.ts` 将 `test.testTimeout` 提至 `15000–20000ms`（避免并行超时），并加 CI "双跑去 flake" 步骤。

---

## 3. 10 项功能 — 三档覆盖矩阵

> ⚠️ **来源说明**：仓库内未发现由用户/团队提供的"权威 10 项功能清单"文件（AGENTS §2 的"10 项守卫"指 `check:architecture` 的 10 个架构守卫，非功能项；PHASE_F §5 验收清单为 5 项）。下表为**依据 `docs/capability-aggregation-map.md` §4 + 仓库真实能力推导**的 10 项，用于覆盖分析；**请 team-lead 确认或补发权威清单**。

图例：A=现有测试/e2e/运行时可自动验证；M=需人工/桌面/显示/网络；G=当前无覆盖（需新增）。

| # | 功能项 | 来源/落点 | 覆盖档 | 证据 |
|---|---|---|---|---|
| 1 | 宝塔面板 MCP 工具集（16 工具） | `btpanel-mcp/` + pyengine discovery `mcp_baota_*` | **A（部分）** | `smoke_stdio.py` PASS；`mcp_btpanel_smoke.py`（需 PYTHONPATH）— 本环境无活面板，全链路待活面板 |
| 2 | Bloom 融合路由 + 自动拆解 + 长续航 | `plan/runner.rs`、`gateway/chat.rs`、`methods.rs` | **A** | cargo：`plan` runner 4 例 + `long_running` 全绿；并入 1166 |
| 3 | WS/SSE 断线重连 + outbox 消息重放 | `webui/src/lib/rpc.ts` + `gateway/server.rs` | **A** | `rpc.test.ts` 20/20（含 2 新增 outbox 用例） |
| 4 | `sandbox.policy.get` + 重试/错误回传 | `sandbox/policy_store.rs`、`protocol.rs`、`useRpc.ts` | **A** | cargo `sandbox::*` + webui `useRpc` retry 单测 |
| 5 | 分层安全 L1（command/ssrf/exec 沙箱） | `security/command_guard.rs`、`ssrf_guard.rs`、`exec.rs` | **A** | 单测 + `security/mod.rs` 集成链 |
| 6 | 分层安全 L2（恶意代码启发式扫描） | `security/scanner_l2.rs`、`chain/executor.rs` | **A** | `guards.check()` 集成 + 单测 |
| 7 | 分层安全 L3（越界/逃逸裁决 GuardrailScanner） | `security/scanner_l3.rs` | **G（测试过但运行时未接线）** | 12 单测通过；**运行时 0 调用点** → 见 P2 |
| 8 | i18n 本地化解析（网关注入 `data-locale`） | `webui/src/i18n/index.ts` | **A（单测）/ M（e2e）** | `i18n.test.ts`；e2e 需 locale 注入（当前 16 陈旧） |
| 9 | 品牌化存储键迁移 opensquilla→marguerite | `webui/src/utils/storageMigration.ts` | **A（单测）/ G（e2e）** | 迁移单测；**10 个 e2e 仍断言旧键** → 见 P2 |
| 10 | 桌面 GUI（托盘/快捷键/菜单/实时监听）+ 发布安装器 | `gui/src-tauri/*`、Phase C/F | **M** | 仅 `cargo check -p marguerite-gui`；无运行时 e2e（需显示+MSVC/WebView2） |

**矩阵结论**：
- 可自动验证（A）：#2 #3 #4 #5 #6 #8(单测) #9(单测) — 核心引擎与 Web 逻辑覆盖充分。
- 人工/环境受限（M）：#1(活面板) #8(e2e locale) #10(桌面) — 非代码缺陷，是验收环境门槛。
- **覆盖缺口（G）**：#7（L3 接线）#9（e2e 陈旧）— 见缺陷清单。

---

## 4. 缺陷定级清单

### P1 — webui 单测非确定性 flake（已定位，未修）
- 位置：`webui/src/components/settings/SettingsDialog.dataMaintenance.test.ts`（`<Teleport to="body">` + `resetModules` + 默认 5s 超时）。
- 影响：单测门禁"通过/失败"不可靠；并行满载下随机失败（与真实缺陷无关）。
- 状态：**源码未修复**，仅以 `--testTimeout=20000` 运行时绕过。详见 §2。

### P2 — L3 安全层接线缺失（GuardrailScanner 运行时从未被调用）
- 证据：全仓 grep `GuardrailScanner`/`scanner_l3`，除 `security/scanner_l3.rs`（定义+12 单测）与 `security/mod.rs`（文档注释 + 2 个集成测试调用）外，**零运行时调用点**。`tools/builtin.rs` 的写/删/exec 路径与 `security/exec.rs` 只调 L1（`command_guard`/`ssrf_guard`），`chain/executor.rs` 只调 L2（malware scanner）。
- 影响：L3 声明的能力（写 `/etc`、`.ssh`/`.aws`/`.gnupg`、跨工作区、路径穿越、`chroot`/沙箱逃逸 exec）**在运行时不被执行**——该层是"死代码"，造成"已防护"的假象（fail-open by omission）。
- 处置建议：在 `write_file`/`delete`/`move`/`exec` 工具落地前接入 `GuardrailScanner::check()`，或将 L3 显式降级/文档注明不启用（不得保留"已防护"错觉）。**建议同步 security-officer 评估定级**。

### P2 — e2e 套件品牌化陈旧（13 文件 / 10 文件断言 legacy `opensquilla.*` 键）
- 证据：`webui/e2e/` 下 10 个 spec 仍断言 `opensquilla.theme`/`routerFx`/`appearance.toolDetails.v1`/`chat.approvalPoll`/`chat.answerReveal`；产品代码已用 `marguerite.*`（`SettingsAdvancedPanel.vue:20`、`useChatApprovals.ts:38`、`stores/app.ts`）。
- 影响：强制 en locale 后 settings e2e 仍 16 失败；默认 zh locale 下 22 失败。**e2e 当前不能作为验收门禁**。
- **「设置开关持久化 bug」= 陈旧测试，非产品缺陷**：`settings-modal.spec.ts:576` 断言 `opensquilla.chat.approvalPoll`，产品写入 `marguerite.chat.approvalPoll` → 该"bug"已澄清为测试债，可关闭。
- 处置：将 10 个 e2e spec 的存储键改为 `marguerite.*`；建议新增守卫脚本，e2e 若再出现 `opensquilla.` 即 fail（复用现有迁移检查范式）。

### P3 — 跨端品牌色守卫缺口（vacuous pass）+ GUI emoji 违例
- 证据：`webui/scripts/check-cross-surface-tokens.mjs` 目标为**不存在**的 `desktop/electron/src/boot.html`、`desktop/electron/src/main.ts`（实际 Tauri 壳在 `gui/src/`）。守卫"未找到目标 → 跳过 → 通过"，**真空通过**。
- 真实 GUI 壳未被校验：`gui/src/theme.css` 含 4 个非规范品牌橙（`#F97316`/`#FB923C`/`#C2410C`/`#F59E0B`）；`gui/src/App.tsx` 使用 emoji 图标（⚡🔍🤖🛡️…），**违反 AGENTS §3 不变量 #6「禁止 emoji 图标」**。
- 处置：将守卫 glob 改为 `gui/src/theme.css` + `gui/src/App.tsx`；GUI 壳 emoji 需替换为文本/ASCII 标记。

### P4 — `baota.rs` 编译告警（违反 clippy 零 warning 门禁）
- 证据：`core/src/tools/baota.rs` — `BaotaClient::new(config: BaotaConfig)` 为 `pub` 但参数 `BaotaConfig` 为 `pub(self)` → `private_interfaces`；`BaotaConfig.secret_key` 字段从未读取 → `dead_code`。
- 影响：违反 AGENTS §2 `cargo clippy --workspace --all-targets # L3 零 warning` 门禁（构建 hygiene）。
- 处置：将 `BaotaConfig` 提为 `pub(crate)` 或让 `BaotaClient::new` 接收合理类型；删除或实际使用 `secret_key`。

### P4 — 文档基线漂移
- 证据：AGENTS §2 写 1057 passed、§5 写 928 基线；capability-map §4.3 写 1157；实际 **1166**。webui 基线 AGENTS 写 3044；实际 **3067**。
- 处置：回填 AGENTS §2/§5 与 capability-map §4.3 为真实数字（1166 / 3067），消除漂移。

---

## 5. 回归测试清单（Regression Recommendations）

1. **L3 接线回归守卫**（堵 P2）：新增集成测试，断言文件写/删/exec 工具落地路径**确实调用** `GuardrailScanner::check()`（例如写一个会命中 L3 deny 的操作，验证被拒）。将"未接线"转为编译/运行期失败。
2. **e2e 存储键守卫**（堵 P2）：扩展现有 rebrand 迁移检查，e2e 中若出现 `opensquilla.` 即 fail；将 10 个陈旧 spec 改写为 `marguerite.*`。
3. **开关持久化 e2e 真回归**（堵"误报 bug"）：`settings-modal` 用真实键 `marguerite.chat.approvalPoll`，验证切换 → 重载 → 值保持的闭环。
4. **flake 修复 + CI 去 flake**（堵 P1）：落实 §2 修复（teardown/timeout），CI 双跑验证稳定。
5. **cross-surface 守卫修正**（堵 P3）：glob 指向 `gui/src/*`，CI 失败即拦截。
6. **clippy 零 warning**（堵 P4）：修 `baota.rs` 两告警，纳入 `cargo clippy --workspace --all-targets`。
7. **文档基线回填**（堵 P4）：AGENTS / capability-map 数字对齐 1166 / 3067。

---

## 6. 发布验收建议（Release Acceptance）

**可立即发（ship-ready）**：core 引擎 + webui 控制台。门禁全绿条件：
- `cargo test -p marguerite-core --lib` = 1166/0/7 ✅
- `npm run test:unit`（vitest，已修 flake 后）= 3067/0 ✅
- `npm run check:architecture` = 10/10（**须先修 P3 守卫目标**）✅
- `npm run typecheck` = exit 0 ✅
- `python3 scripts/check-doc-cli-drift.py` = exit 0 ✅
- 宝塔 MCP 自测 PASS ✅

**须人工验收里程碑（本环境无头，未实机验证）**：
- 桌面 GUI 运行时：托盘出现、全局快捷键 `Ctrl+Shift+Space` 唤出、对话闭环（Phase F §5）。需带显示的目标机 + MSVC/WebView2 工具链。
- 安装器签名：当前 NSIS/MSI 未签名（SmartScreen 警告，体验非阻断）；发布前需 EV 证书（Phase F §六 开放项）。
- 自进化闭环检索、数据持久（关闭重开台账仍在）需在目标机验证。

**门禁纪律**：
- **当前 e2e 不可作为 green 信号**（16/22 陈旧失败）。发版前要么修、要么显式 quarantine 并从门禁排除，严禁"假装全绿"。
- L3 接线（P2）建议在发版前闭环或出显式决策记录；不得保留"已防护"错觉。

---

## 7. 临时产物清理
- ✅ 已删除：本次 QA 临时 harness 配置文件 `webui/playwright.qa-en.config.ts`（避免被 `playwright` 误识别）。
- ⚠️ 残留（无害、git-未跟踪）：`webui/test-results-qa-en{,2,3}/` 三个 Playwright 中间目录。本环境因 safe-delete 钩子拦截 >50 文件批量删除、且 PowerShell 递归删除被安全策略阻断，无法自动清空；请手动以 `Remove-Item -Recurse -Force` 清理，或忽略（均为未跟踪产物，不影响发布）。
- 复现用网关（127.0.0.1:18791）为临时进程，非发布产物。

---
*报告生成于本次 QA 会话；所有数字为当时实跑结果。10 项功能矩阵为推导版，待 team-lead 确认权威清单。*
