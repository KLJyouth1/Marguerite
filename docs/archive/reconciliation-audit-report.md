# Marguerite 文档与记忆对账报告（Reconciliation Audit）

> 核查日期：2026-08-07
> 核查人：项目总监（大湾区靓仔）
> 方法：逐条读取指定文档/记忆 → 与 `marguerite/` 实际源码、配置、构建产物、`GUI_Agent/` 冻结状态、`cargo test` 实测比对
> 实测基线：`cargo test -p marguerite-core --lib -- --list` = **911**（909 通过 + 2 ignored）；`cargo build -p marguerite-tui` = exit 0；`dist/marguerite-desktop.exe` = 16,311,808 字节（GUI 子系统）

图例：
- ✅ **已落实且一致**：文档/记忆描述与代码现状吻合
- 🟡 **已落实但存在偏差**：方向正确，但具体状态/数字/范围与现状有出入（已说明偏差）
- ⚪ **未落实 / 已过时**：描述对象已变更或从未以所述形态存在

---

## A. `docs/capability-reconciliation.md`（生成 2026-08-07）

| # | 条目 | 状态 | 证据 / 偏差说明 |
|---|------|------|----------------|
| A1 | §0 E 期四残留：research/vision/automation/serve(acp) | ✅ | `core/src/{research,vision,automation,serve/acp}` 均存在 |
| A2 | §0 KB 2.0：`kb/{graph,denoiser,integrity}.rs` + mod.rs 重导出 | ✅ | `core/src/kb/mod.rs:11-18` 显式 `pub mod graph/denoiser/integrity` + re-export |
| A3 | §0 Security L2/L3：`security/scanner_l2.rs`/`scanner_l3.rs` | ✅ | 文件存在；`security/mod.rs:14-15,21-22` re-export |
| A4 | §1 前提"注册表只有 tier，**没有 implemented 落地标志**" | 🟡 | `capabilities.rs:102-109` 的 `Capability` 确实只有 `tier`/`maturity` 字段（无 `implemented:bool`），但 `:77,95` 已有 `is_implemented() = maturity != Narrative`，`:240,246` 已有 `CapabilityStats.implemented`。文档 §5.2 建议"加 implemented:bool"已被派生机制部分满足 |
| A5 | §1 能力表 `sec.os_sandbox` 标 ⚪（"需系统调用隔离，未做"） | 🟡 | `core/src/security/os_sandbox.rs`（08-07 02:21）**真实实现** Landlock(Linux)/Seatbelt(macOS)，Windows 诚实返回 `Unsupported`。应为 🟡（部分实现），非 ⚪ |
| A6 | §1 能力表 `tool.git` 标 ⚪ | 🟡 | `core/src/tools/git.rs` 存在（实现文件在盘） |
| A7 | §1 能力表 `tool.mcp_bridge` 标 ⚪ | 🟡 | `core/src/mcp/{client,mod,registry,server}.rs` 共 4 文件存在 |
| A8 | §1 能力表 `tool.browser`/§2 `browser_bridge` 标 OPEN/⚪ | 🟡 | `core/src/browser/{mod,cdp}.rs`（08-07 02:32/02:40）CDP 脚手架在盘；OPEN 仅指"未接入生产 UI/UIA 重桥" |
| A9 | §2 OPEN-DECISIONS 桶：A RESOLVED / B·C OPEN | ✅ | 与 `OPEN-DECISIONS.md` 完全一致 |
| A10 | §3 诚实差距（重型原生桥/LLM 后端未做） | 🟡 | 方向基本成立，但 os_sandbox/git/mcp/browser 已有脚手架，与"零实现"表述略有出入 |
| A11 | §4 差异化内核表（EngineInvoker/离线 analyze/KB2.0/L1-L3/PlanGate/快照+pitfalls） | ✅ | 对应模块均存在 |
| A12 | §5.1 "`marguerite/` 非 git 仓库" | ✅ | 实测 `marguerite/.git` 不存在 |
| A13 | §5.1 "lib 893 passed + e2e 5 + perf 1 = 899 用例" | 🟡 | **实测 lib = 911 listed（909 通过 + 2 ignored）**；893 少计 16。文档数字与 2026-08-06 memory 的 909 及实测均不符 |
| A14 | §5.4 "前端双原生 `cargo build -p marguerite-tui -p marguerite-desktop` 门禁通过" | ✅ | `desktop` 产物 `dist/marguerite-desktop.exe` 存在（16MB）；`tui` 本轮实测 `cargo build -p marguerite-tui` exit 0 |

---

## B. `docs/nexustar-api-capability-review.md`（生成 2026-08-06）

| # | 条目 | 状态 | 证据 / 偏差说明 |
|---|------|------|----------------|
| B1 | 审查对象"`D:\Downloads\GUI_Agent\nexustar-api` — **280 个 .py**" | ⚪ | 实测 `find nexustar-api -name '*.py'` = **422**（差 142）。严重少计，应更正为 422 |
| B2 | §1 "`auth.py`/`crypto.py` → `auth/`（本地优先账号下沉）已落地" | ⚪ | `core/src/` 下**无 `auth/` 目录**（33 个子目录均无 auth）。该声明与现状不符，需核实 auth 是否以其他命名/并入其他模块，否则删除或标注"规划中" |
| B3 | §1 "`kb_index.py` → `kb/`（BM25 RAG）" | ✅ | `core/src/kb/store.rs` 存在（BM25） |
| B4 | §1 "`tools/`（46 原生工具）" | ✅ | `core/src/tools/` 存在（`git.rs` 等） |
| B5 | §2 差异吸收：ACP/research/vision/long_running → `serve/acp`/`research`/`vision`/`automation` | ✅ | 对应目录均存在 |
| B6 | §2 暂列待办 knowledge_graph/rag_denoiser/rag_integrity + security L2/L3 | 🟡 | KB 三件套 + L2/L3 现已实装（见 A2/A3），仅 auto_fix/auto_review/verifier 仍 OPEN；文档"待办"中已做的应标 DONE |
| B7 | §3 明确不吸收 browser_bridge/desktop/nexustar-ui（冻结） | ✅ | 与 `g-phase-web-freeze.md` 一致（但文档未提 marguerite 内已有 `browser/cdp.rs` 脚手架） |
| B8 | §4 结论"吸收 4 项 ACP/研究/视觉/长时调度" | ✅ | 已落地 |

---

## C. `docs/g-phase-web-freeze.md`（生成 2026-08-06）

| # | 条目 | 状态 | 证据 / 偏差说明 |
|---|------|------|----------------|
| C1 | 冻结根目录 `D:\Downloads\GUI_Agent` | ✅ | 目录存在（含 `.git`） |
| C2 | 6 个 `.frozen-20260806` 目录（admin-backend/desktop/gwwc/nexustar-ui/nexustar-website/nexustar-website-next） | ✅ | 实测 6 个 frozen 目录全部存在 |
| C3 | 保留未冻结：nexustar-api / nexusops / nexustar-qa | ✅ | 三者均存在且无 `.frozen` 版本 |
| C4 | 还原方法（可逆重命名脚本） | ✅ | 未执行删除，与"保留可还原"一致 |
| C5 | 门禁结果"冻结前 784 passed，冻结后复测同" | 🟡 | 实测当前 **909 passed**（911 listed）。784 为更早历史快照，建议注明"截至冻结时" |
| C6 | "web 端彻底移除，收敛到 Rust 单引擎 + 双原生前端" | ✅ | 与最终决策一致；但文档未提及 `gui/`（Tauri+React 孤儿目录，见 F） |

---

## D. `docs/univagent-integration-doc-v1.md`（生成 2026-08-05，v1.0 草案）

> 总体判定：本文件是 08-05 的"活文档"快照；自 08-06 web 冻结 + 08-07 零 Web 决策后，其 web 端状态描述已**过时**，但 Marguerite 核心架构概念仍准确。

| # | 条目 | 状态 | 证据 / 偏差说明 |
|---|------|------|----------------|
| D1 | 卷一 三仓整合表：Marguerite "Git: 无 git" | ✅ | 实测 Marguerite 项目根的 `marguerite/` 子目录不存在（原"三仓之一"未建仓），代码现统一于根目录 `D:\Downloads\Marguerite` |
| D2 | 卷一 三仓整合表：GUI_Agent "✅ 有 git" | ✅ | 实测 `GUI_Agent/.git` 存在 |
| D3 | 卷二 §2.3 技术栈：桌面壳 Tauri 2.x / 前端 React+Vite / Marguerite "Rust+ratatui" | 🟡 | 所列 web 技术栈现已冻结（g-phase）；Marguerite **最终 GUI = `gui/`（Tauri+React 桌面壳）**，原 egui `desktop/` 已从 Cargo workspace 移除 |
| D4 | 卷二 §2.2 36 槽 SlotId / WorkbenchState / EventBus | ✅ | 概念存在于 `core/src/tui/workbench/` + `core/src/events/`，与 08-05 memory Phase J（J5 验证）一致 |
| D5 | 卷三 §3.1 TUI Workbench 各模块路径（layout/slot/state/...） | ✅ | `core/src/tui/workbench/` 下模块存在 |
| D6 | 卷三 §3.1 "79 测试通过（Phase J）" | ✅ | workbench 子集测试，属 911 总测试的合理子集 |
| D7 | 卷三 §3.2 Desktop GUI(nexustar-ui+Tauri) 标 ⚠️ 开发中 | ⚪ | 该端已于 08-06 冻结，状态应改为 frozen |
| D8 | 卷三 §3.3 API 后端 "421 个 .py" | 🟡 | 实测 422（±1，轻微） |
| D9 | 卷三 §3.3 模块表（auth/backend_switch/sessions/max/orchestrator/...） | 🟡 | 对象在 frozen `nexustar-api` 中存在，但状态应标 frozen |
| D10 | 卷七 §7.4 Tauri IPC 契约 `desktop/src-tauri` | ⚪ | 该路径现已 `desktop.frozen-20260806/src-tauri` |
| D11 | 卷八 部署运维（各组件启动方式） | ⚪ | 描述对象多数已冻结 |
| D12 | 附录 A 归档仓 `D:\Downloads\_GUI_Agent_archived_20260713` | 🟡 | 未专项核验该归档仓存在性（参考类，建议补一句存在性确认） |
| D13 | 卷四/五/六 竞品矩阵/Bloom 粒子/工具路线图 | — | 信息/规划类，非代码状态，不纳入机器对账；但 Bloom/K1.4/K1.5 所依赖的 `nexustar-ui` 现已冻结 |

---

## E. `docs/decisions/OPEN-DECISIONS.md`（2026-08-07，含 08-05 行）

| # | 条目 | 状态 | 证据 / 偏差说明 |
|---|------|------|----------------|
| E1 | 表 8 行（4×08-05 RESOLVED + 4×08-07） | ✅ | 与 `MEMORY.md` 及 `2026-08-05.md`/`2026-08-07.md` 完全对应 |
| E2 | RESOLVED 决议 D-2026-08-05-01~04 | ✅ | 与 08-05 memory Phase H/G 落地（collab/deploy/registry/bloom）一致 |
| E3 | D-2026-08-07-01 KB 2.0 / D-2026-08-07-02 L2/L3 | ✅ | 与 A2/A3 + `kb/mod.rs`/`security/mod.rs` 一致 |
| E4 | 关联设计稿 `.trae/specs/cli-workbench/bloom-mode-and-platform.md` | ✅ | 文件存在，含 4 项决策（行 332-334：轻量协作中心/宝塔 MCP/本地模型/并行预取）+ Phase G/H/I/J 引用；交叉引用有效 |
| E5 | 状态桶 A RESOLVED / B·C OPEN | ✅ | 与 A9 一致 |

---

## F. `.workbuddy` 记忆

### F1. `2026-08-04.md`
| 条目 | 状态 | 说明 |
|------|------|------|
| A4.4 引擎埋点（EventBus/AgentRuntime/RealEngine） | ✅ | `events/bus.rs`/`agent/runtime.rs`/`tui/workbench/mod.rs` 存在 |
| Phase C workspace_watcher.rs | ✅ | 文件存在 |
| 约束"不可修改 Cargo.toml" | 🟡 | 08-05 用户决策放开（选项 B，加 notify/tokio），属历史约束后被放宽 |
| "全程未 git 提交" | ✅ | 至今 `marguerite` 无 `.git` |

### F2. `2026-08-05.md`（509 行，最厚）
| 条目 | 状态 | 说明 |
|------|------|------|
| Phase C/D 构建 `gui/`（marguerite-gui Tauri+React） | ✅ | `gui/` 实际存在（package.json/tauri.conf.json/src-tauri） |
| **Phase J "gui/ 目录不存在…Tauri+React GUI 从未落地"** | ⚪ | **与磁盘事实相反**——`gui/` 真实存在；仅"非 workspace 成员、被 egui desktop 取代"为真。memory 内部 Phase C/D 与 Phase J 自相矛盾 |
| 测试数递进 615→…→702 | 🟡 | 历史快照，已被 909/911 超越；作为"当前状态"引用会误导 |
| Phase H registry/bloom/selector/byok | ✅ | `model/{registry,bloom,selector,byok}.rs` 存在 |
| Phase I collab/deploy(ssh/baota) | ✅ | `collab/` + `deploy/{ssh,baota}.rs` 存在 |
| Phase J 36 槽 / J5 像素复核 | ✅ | 与 D4 一致 |
| K1-K4 / Bloom 标准 v2.0 / 整合文档 | ✅ | 产出文件均存在（根目录 `bloom-mode-*.md`、`docs/univagent-integration-doc-v1.md`） |
| K1.4/K1.5 修改 nexustar-ui 的 BloomBackground 等 | 🟡 | 修改发生在 08-05，对象已于 08-06 冻结；记录的是当时动作，可接受 |

### F3. `2026-08-06.md`
| 条目 | 状态 | 说明 |
|------|------|------|
| E 期四残留落地 | ✅ | 模块存在 |
| 磁盘满事故 | — | 历史事件，08-07 已腾出 55GB，现磁盘已释放 |
| "重型桥 browser_bridge/cdp、sec.os_sandbox、tool.git 已在盘并通过构建/测试门禁" | ✅ | 与本次实测吻合（`browser/cdp.rs`/`os_sandbox.rs`/`tools/git.rs` 均在盘）——**但此条与 A5/A6/A8 的 ⚪/OPEN 标注冲突** |
| "909 passed / 0 failed / 2 ignored" | ✅ | 与本轮实测 911 listed / 909 pass 一致 |

### F4. `2026-08-07.md`
| 条目 | 状态 | 说明 |
|------|------|------|
| GUI exe 打包 | ✅ | `dist/marguerite-desktop.exe` 存在 |
| CJK 字体修复 / GUI 卡片式重构 | ✅ | `desktop/src/main.rs` 重写、编译通过 |

### F5. `MEMORY.md`
| 条目 | 状态 | 说明 |
|------|------|------|
| 目标（Rust 单引擎+双原生前端，放弃 Web） | ✅ | 与最终决策一致 |
| 已落地（E期+KB2.0+安全L1/L2/L3+双前端） | ✅ | 模块存在 |
| 收尾对账 / 非 git / OPEN-DECISIONS 桶 | ✅ | 与 A/E 一致 |
| 架构约束（Bloom 禁 emoji/紫粉/硬编码；ACP 仅 127.0.0.1 等） | ✅ | 方向一致（策略层，未逐条跑测试） |
| 环境坑（D盘写满/tokio::test TCP） | — | 历史/环境，可接受 |

### F6. `pitfalls.jsonl`
| 条目 | 状态 | 说明 |
|------|------|------|
| governance/substring-keyword-false-positive | ✅ | 与 2026-08-06 memory 的 substring bug 修复（secret_sig/env、fanout/所有）一致，`validated=true` |

---

## G. `.trae`（设计稿，被 OPEN-DECISIONS 引用）

| 条目 | 状态 | 说明 |
|------|------|------|
| `.trae/specs/cli-workbench/` 含 8 文件（bloom-mode-and-platform.md / spec.md / tasks-v2.md / design-workbench-v2.md 等） | ✅ | 目录存在 |
| `bloom-mode-and-platform.md` 被 OPEN-DECISIONS 引用 | ✅ | 文件存在且含 4 项决策（行 332-334）+ Phase G/H/I/J，交叉引用有效 |
| 其余 spec 内容（Phase A-J / Bloom 规划） | 🟡 | 与 Marguerite 实际 TUI/egui 实现方向一致（36槽/J5 已落地），属设计文档未逐一比对，但无矛盾证据 |

---

## H. 跨文档 / 文档与记忆冲突清单

| 冲突 | 涉及方 | 说明 | 建议 |
|------|--------|------|------|
| **H1** | `capability-reconciliation`(A5/A8) vs `2026-08-06.md`(F3) | 前者将 sec.os_sandbox / browser_bridge 标 ⚪/OPEN（"未做"），后者明确"已在盘并通过构建/测试门禁"，实测 `os_sandbox.rs`/`browser/cdp.rs` 均为真实实现 | reconciliation 改 🟡 并注明"脚手架在盘，未接入生产链路" |
| **H2** | `capability-reconciliation`(A6/A7) vs 实测 | tool.git / tool.mcp_bridge 标 ⚪，但 `tools/git.rs` / `mcp/` 4 文件存在 | 注明实现文件在盘 |
| **H3** | `2026-08-05.md` Phase C/D vs Phase J（memory 自冲突） | 同文件内一处详述构建 `gui/`，另一处称"`gui/` 不存在、从未落地"；实测 `gui/` 存在 | 修正 Phase J 表述 |
| **H4** | 测试数：reconciliation 893 vs 08-06 memory 909 vs 实测 911 | 三处数字互不一致 | 统一为 **909 passed（911 total, 2 ignored）** |
| **H5** | `univagent-integration-doc-v1`(D3/D7/D10) vs `g-phase-web-freeze` + `capability-reconciliation`/`MEMORY` | 集成文档描述 web 重架构（nexustar-ui active / Tauri 桌面），与 08-06 冻结 + 08-07 零 Web 决策冲突 | 集成文档顶部加状态更新横幅 |
| **H6** | `nexustar-api-capability-review`(B2) vs 实测 | 称 Marguerite 有 `auth/` 模块，实测无 | 核实 auth 形态或删除/标注"规划中" |
| **H7** | `nexustar-api-capability-review`(B1) vs 实测 | ".py 数 280" vs 实际 422 | 更正为 422 |

---

## I. 优先修复建议与修改方案

### P0（直接影响决策准确性）
1. **修正测试数**：`capability-reconciliation.md` §5.1 的 "893 passed" → **"909 passed（lib 911 total，2 ignored）"**；同步将 08-05/08-06 memory 中作为"当前状态"引用的测试数注明时间戳。
2. **修正 capability-reconciliation §1/§3 能力状态**：
   - `sec.os_sandbox` ⚪ → 🟡（Linux Landlock / macOS Seatbelt 真实实现，Windows `Unsupported` 诚实返回；证据 `os_sandbox.rs`）。
   - `tool.git`/`tool.mcp_bridge`/`tool.browser` 说明补 "实现文件已在盘（`tools/git.rs`/`mcp/`/`browser/cdp.rs`），OPEN 仅指未接入生产工具链/UIA 重桥"。
3. **修正 nexustar-api-capability-review**：首段 "280 个 .py" → **"422 个 .py"**；§1 `auth/` 条目核实后修正或标"规划中"（marguerite 无 `auth/` 目录）。

### P1
4. **修正 2026-08-05.md Phase J**：将"`gui/` 目录不存在…从未落地"改为 erratum 注释——"`gui/` Tauri+React 脚手架曾搭建（package.json/tauri.conf.json/src-tauri 在盘），但未被选为活跃 GUI（非 workspace 成员），最终由 egui desktop crate 取代"。保留原始时序记录，加明显标注。
5. **univagent-integration-doc-v1.md** 顶部加状态横幅：说明 web 端已于 08-06 冻结、最终决策零 Web、Marguerite 现有 `egui` desktop crate；卷三 §3.2/§3.3/§7.4/§8 状态标注 frozen。

### P2
6. **g-phase-web-freeze.md** 门禁结果 "784 passed" 注明"截至冻结时"。
7. **capability-reconciliation §5.2** 建议措辞调整：`is_implemented()`（maturity≠Narrative）+ `CapabilityStats.implemented` 已提供机器可校验信号，建议改为"扩展 is_implemented 覆盖更多维度（如测试守护/生产接入）"。

### 修改落地方式
- 上述 P0/P1 均为**文档与记忆的文字修正**，不改代码（代码现状已验证正确）。
- **执行记录（2026-08-07）**：已按 P0/P1/P2 全量落地，Edit 五处：
  - `capability-reconciliation.md`：测试数 893→909/911、§1 前提补 `is_implemented()`/`CapabilityStats.implemented` 派生信号、tool.git/tool.mcp_bridge/tool.browser/sec.os_sandbox ⚪→🟡（注明在盘实现）、§3 os_sandbox 改为「部分真实实现」、§5.2 措辞调整。
  - `nexustar-api-capability-review.md`：280→**422** 个 .py；`auth/` 标「规划中」（core/src 无该目录）。
  - `2026-08-05.md`：顶部加测试快照 erratum；Phase J 行 ~334 加 gui/ 自相矛盾 ERRATUM。
  - `univagent-integration-doc-v1.md`：顶部加「web 端已过时」状态横幅。
  - `g-phase-web-freeze.md`：门禁 784 passed 注明「截至冻结时」。
  - 代码层零改动；所有修正均可经 `git diff`（若启用）或文件 mtime 回滚。

---

## J. 核查结论

- **代码现状总体可信**：E 期四残留、KB 2.0、安全 L1/L2/L3、双原生前端（tui 编译通过 / desktop exe 存在）、KB 模块 re-export、OPEN-DECISIONS 决议与 `.trae` 引用——均与文档/记忆**一致**。
- **主要问题集中在文档自贬与过时**：`capability-reconciliation` 系统性低估了已在盘的实现（os_sandbox/git/mcp/browser 脚手架），测试数少计；`nexustar-api-capability-review` 的 .py 计数与 `auth/` 声明失准；`univagent-integration-doc-v1` 因 08-06 冻结+08-07 零 Web 决策而过时；`2026-08-05.md` 存在 gui/ 自相矛盾。
- **无代码层"宣称未落地"的实际缺失**（除重型原生桥/LLM 后端这类本就 OPEN 的项外）；记录的"未做"项中，脚手架其实已在盘，需文档如实反映。
