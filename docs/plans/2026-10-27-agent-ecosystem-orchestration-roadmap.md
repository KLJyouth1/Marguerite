# 智能体生态与协调联动路线（2026-10-27）

> 输入：本轮用户九项指控 + 4 份最新会话转写（`D:\Downloads` 导出）+ 用户 69 个 GitHub 仓 + 本地模型项目
> `D:\Downloads\yolo-ultralytics` + 2 份设计笔记 + RSI 文献。
> 方法：**每条主张都在本树复核到 file:line 才写结论**；与主张不符的地方如实标注，不为凑需求编造缺口。
> 状态：**方案已定稿，代码未全部落地**。第 4 节的 P0 前 3 项本轮已完成并附测试证据，其余为待施工图。

---

## 1. 取证口径与诚实边界

**读到并用于决策的**：
- `项目设计深度查漏`（17 MB）、`竞品分析与优化研究`（15 MB）、`完善 WebUI 各项功能与智能体生态`（4.6 MB）、
  `编写规格书、竞品分析与路线图规划`（230 KB）四份转写，由子代理蒸馏，关键结论回原文件逐条核对。
- 本树源码现状（所有 file:line 均为本轮实测）。
- `gh api` 拿到的 69 个仓库元数据 + README/文件清单；`yolo-ultralytics` 的 `args.yaml`/`metrics_*.json`（**转抄，未复跑推理**）。

**拿不到 / 未证实（不得据此下结论）**：
- 17 MB / 15 MB 转写**不可能逐字全读**，采用分片蒸馏 + 关键点回查；引用到的 `omnipilot-architecture-roadmap.md`
  与 `omnipilot16-eval-report.json` 在 `D:\Downloads` 三层内**不存在** → 该两份交付物已丢或在别处。
- 私有仓 `gui-agent` / `tmp-t1` / `archived-agent-s` / `3d_game_framework` **只读到元数据与文件清单，未本地检出**，
  可运行性未证；`caisiqi`、`liyuan-topic5` 的 tags/releases API 一次网络超时未复查。
- 笔记里引用的训练指标全部**转抄**，本轮未跑任何训练/推理/构建（按要求）。
- RSI 论文的 arXiv 编号系检索代理所得，**未逐条回链核验**；主体归属已按检索结果更正（见 §5.3）。

---

## 2. 九项主张 × 实测：把「已修 / 部分修 / 真缺 / 与实测不符」分开

这一节是路线图的唯一事实基座。**四类结论的成本差一个数量级**，混为一谈就会重复施工或漏修真缺陷。

| # | 用户主张 | 实测结论 | 证据（本树 file:line） | 分类 |
|---|---|---|---|---|
| 1 | 智能体不知道如何操控工作台内置应用（浏览器/PowerShell/cmd/CLI/TUI/GUI/编辑器） | 内置能力**目录存在但子代理看不见**：`BUILTIN_SURFACES` 只注入主面提示文本，`compact_catalog_text` 不含该段 | `core/src/agent/tool_catalog.rs:139-178`（目录）、`:321-325`（注入点）、`:384-394`（子代理用的 compact 文本，**无该段**） | **真缺（半截接线）** |
| 1b | …也没有工具能驱动工作台自身 | 面板/TUI/产物中心/团队看板**均无 `ToolExecutor`**；`tool_catalog.rs:65` 写了 `terminal` 但从未注册；`terminal.backend.exec` 只是 RPC | `core/src/tools/terminal.rs`（无 executor）、`core/src/gateway/methods.rs:2473-2475` | **真缺** |
| 2 | 不知道 MCP / Tools / skills 的存在 | 技能**有**权威枚举注入（上限 60、如实标注截断）；MCP **只有计数、没有枚举**，能力卡的 `tools` 字段是空数组 | `tool_catalog.rs:338-355`（技能）vs `core/src/agent/capability_cards.rs:225-227`（MCP：`tools: &[]`） | **真缺（MCP 半边）** |
| 3 | 怀疑「有没有真的读取智能体的定义」 | **确实读取了**，但只进系统提示：`AgentRecord` 经 `personality_for`（带 enabled 门）注入 `chat.rs:1212-1230`；**工具面未接线**（源码注释自承「工具段只声明偏好」）、**`AgentRecord.model` 零消费** | `core/src/agent/store.rs:20-45`（定义体）、`:506-512`、`:528`（注释）、`core/src/agent/profile.rs:93-113`（历史自动推断的 Persona，与定义无关） | **部分修 ——「没读」是不成立的，「读了但没生效」成立** |
| 4 | 桌宠无法移动出窗体 | OS 浮窗**已实现且权限齐备**：`pet-overlay` 窗口运行时创建、`usePetOverlay.ts`/`desktopBridge.ts` 桥接、`capabilities/pet-overlay.json` 授权 | `gui/src-tauri/src/lib.rs:587-628`、`webui/src/composables/usePetOverlay.ts`、`gui/src-tauri/capabilities/pet-overlay.json` | **代码已修 → 未装到用户机器上**（见 §3 结论） |
| 4b | 右键弹的是浏览器菜单 | 主窗口级全局抑制器 **已存在**（`AppContextMenu.vue`，2026-10-25 新建、**已暂存未提交**），尊重 `defaultPrevented`；宠物热区实测无缺口（168×186 事件盒 ⊇ 可见轮廓） | `webui/src/components/AppContextMenu.vue:84`、`:137-140`；`webui/src/components/pet/DesktopPet.vue:20`、`:525-534`；浮窗另有 `PetOverlayView.vue:528-530` | **代码已修 → 旧构建产物** |
| 5 | 任务结束后历史会话排版错乱、仍然居中（「这个问题很大」） | **真实缺陷，本轮已修**。根因不是「一条根规则」：`ChatView.vue:5008` 用 `<style scoped src="../styles/chat-view.css">` 挂载 → 编译成 `.msg-ai[data-v-chatview]`，**跨不过子组件边界**；历史行的 `.msg-ai` 由子组件各自 scoped 决定，同特异度按打包顺序决胜 → 「有时像修过、有时又居中」 | 修法统一为 `width:100%` + `max-width:var(--chat-col-start)` + `margin-inline:var(--chat-gutter) auto` + `text-align:start`；覆盖 `chat-view.css`、`AssistantMessage/UserMessage/CompactionEvent/TurnSuggestions/PlanCard/ApprovalCard/ClarifyCard/ChatStallNotice/HistoryLoadSentinel/MetaSkillSetupCard/OrchestrationRunCard`；删除 `ChatMessageList.vue` 的 `>` 脆弱覆盖 | **✅ 已完成**（25 例契约测试 + 变异实测有牙） |
| 6 | 团队没有独立成员卡片看每个成员动向 | 后端事件与 `team.members`/`/team-board` **早已存在**，前端**此前零消费者**；本轮补齐 | 新：`webui/src/components/team/TeamMemberCard.vue`、`TeamMemberActivity.vue`、`TeamBoardRunBar.vue`；`useTeamBoard.ts` 的 `reduceProgressFrame`/`resolveMemberStatus`（新增 `thinking` 态）；73 例测试绿 | **✅ 已完成（前端段）**；后端遗留见 §4-P1-3 |
| 7 | 开关缺/多/全开全派都不能影响协调联动，且要保证产出质量 | **没有任何组合守卫**。唯一守卫是「权限×自主性」矩阵，与子代理/MCP/记忆等开关无关；已确认 4 处**静默降级** | `core/src/gateway/orchestrate.rs:136-159`（`gate_form`）、`:407-412`（`agents` 空回落单 orchestrator，注释自承「与团队形态的静默降级同源」）、`agent/subagent.rs:912`/`:1090`、`agent/multi_strategy.rs:225`（无工具→退化纯对话，不报错）；`mode/swarm.rs:52-58`（0 收敛为 1，且模块头声明不设硬上限） | **真缺（本轮头号工程项）** |
| 8 | 推理档位与记忆要对它们全部有用 | 档位：**主回合/Bloom/编排已生效**（`TURN_EFFORT` task-local → `runtime.rs:675` → `model/llm.rs:589 with_thinking`；`reasoning_system_prompt()` 死代码已修）；但 **subagent 记忆读取未接线、multi_strategy/蜂群/专家/团队/chain 档位未接线**。`tokio::spawn` 丢 task-local 是机制根因 | `core/src/model/thinking.rs:266-295`（四档已接通，含回归测试 `:327-356`）、`gateway/chat.rs:2425`、`gateway/team.rs:723/766`（只用 `auto_profile(&history)`，无记忆读）、`chain/executor.rs:166/248-249/660`（记忆字段存在但**无 setter 调用方**）、`multi_strategy.rs:310`（`chat_with_tools` 无档位解析） | **部分修 → 系统性接线工程** |
| 9 | 没有文生图、视频、3D 建模等工程工具内置 | **主张与实测不符**：生图/生视频/Manim/代码视频/网页视频/音频转写合成/录放宏**全部已注册**；Blender 为**可选注册**（且注释记录了历史事故：当年用 `?` 会让 Blender 缺失导致**一个工具都没注册**）。**唯一真空缺是 text-to-3D** | `core/src/tools/builtin.rs:180-212`（注册表 + Blender 降级分支）、`tools/image_gen.rs:197`→`image_generate`、`video_gen.rs:565`→`video_generate`、`agent/tool_catalog.rs:94,95,98`（已编目）、前端 `VideoArtifactCard.vue`/`settings/MultimodalCapabilitiesPanel.vue`；3D 侧 `three` **只做渲染**（`pet/pet3d/*`、`island/world3d/*`、`device-twin/scene/*`），无生成工具 | **半误报**：真缺口是**可发现性/参数面/素材源/编排**，不是「没有工具」 |

### 2.1 顺带核出的两条过期文档（未在本轮改，属 §4-P2）

- `AGENTS.md` §0 第 2 条「仓库里没有 LICENSE / `.github` / 131 个 Rust 源文件」**已过期**：本树
  `git ls-files` 有 `LICENSE` 与 `.github/workflows/ci.yml`，`.rs` 文件 **487** 个，`HEAD == origin/master`。
- 版本线已推进到 **0.2.17**（`core/Cargo.toml`），而 `Marguerite-Releases` 最新 tag 仍是 **0.2.13** ——
  唯一滞后项即「未重新打包」。这**同时解释了主张 4/4b**：代码修好了，用户手上的安装包里没有。

---

## 3. 环境阻塞：本轮拿不到干净的 25/25，原因已定位而非猜测

**现象**：日志 `verify-clean-all-2026-10-27.log` 有 5 条 `[FAIL]`。
**根因**：该时段本机同时跑着 **3 份 `verify.sh --clean --all` + 2 份 `--fast`**（实测进程表：4 个 cargo + 2 个 rustc），
这正是本项目计划明文禁止的 **R-03（不得并发跑两份 `--all`；不得并发跑 cargo）**：e2e 固定端口 `18877` 与 `target/` 锁互杀、
共享文件被撕裂读。**不是代码回归。**逐条定性证据：

| FAIL | 定性 | 取证方式 |
|---|---|---|
| `check-readme-claims.py` (rc=1, **0s**) | **撕裂读伪失败**：README 正被并发会话写入（mtime 10:02） | 本地单独复跑 → `[PASS] both READMEs agree … gate total 25/25`，**RC=0** |
| `webui 单测`（1 failed / 4655） | **满载饥饿超时**：`SettingsDialog.dataMaintenance.test.ts` 报 `Test timed out in 15000ms`；该文件与本轮改动无关 | 隔离重跑 → **2 passed**（4.5s，`transform 3.82s` 对比满载时的 `717.68s`） |
| `webui e2e`（73 passed < 基线 95，2 failed，20 did not run） | 满载 + 既有偶发：`computer-use:38 cursor_pos`（原生动作走 PowerShell 5.1 + Add-Type，满载时超网关 30s 内部超时）与 `settings-modal:75`（`AGENTS.md` §4 已记录的偶发）；`serial` 模式把 1 例失败放大成 20 例 skip | 日志原文 + 项目既有记录 |
| `pyengine 契约族`（`test_recovery`: **1 failed** / 443 passed / 9 skipped） | **未定性** —— 唯一需要当真查的一条。历史同类根因是本机 WorkBuddy `safe-delete` 垫片（`SAFE_DELETE_FAIL_CLOSED` → `test_recovery` 曾 8 失败）；日志里**没有 `FAILED` 行**（并发打断），故不能排除真实回归 | 待冻结树复跑：`CODEBUDDY_SAFE_DELETE_ENABLED=0 MARGUERITE_PYTHON=C:/Python314/python.exe bash scripts/verify.sh --pyengine` |
| （`cargo build --workspace` 未见结论） | 被并发 `target/` 锁阻塞风险最高 | 同上 |

**结论与纪律**：
1. **完成声明仍然欠着**——按 `AGENTS.md` §8 铁律，必须是**冻结树上单份** `bash scripts/verify.sh --clean --all` 全绿（25/25 + exit 0）。
2. **不得为了拿绿去杀别的会话的进程**（用户此前明确投诉过），也不得加第 4 份门禁。正确做法是等并发结束后独占复跑。
3. `[SKIP]` 永不算通过；本轮 5 条 FAIL 的定性证据必须与绿灯一起交付，不能用「环境问题」一句话带过。

---

## 4. 缺口分层：P0 / P1 / P2

每项给**收益 / 成本 / 验收判据**。验收判据一律写成可机检的形式（本仓库的传统：守卫必须**变异实测有牙**，
且指标必须定义分子分母 —— 明文 R-13）。

### P0-1 主智能体自认知：让每一层执行体都知道自己能用什么 —— **待做**
| 子项 | 做法 | 验收判据 |
|---|---|---|
| a. 内置应用面进子代理 | 把 `BUILTIN_SURFACES`（`tool_catalog.rs:139-178`）纳入 `compact_catalog_text`（`:384-394`） | 新增单测：子代理提示文本里能逐条命中「内置浏览器 / PTY 终端 / 编辑器工作台 / 产物中心 / 团队看板」；反向断言「主面有、compact 面没有」的旧缺口不再成立 |
| b. 工作台自身可控工具 | 给 `terminal`（`tools/terminal.rs` 已有能力、`:65` 声明过却未注册）与产物中心/团队看板各补一个 `ToolExecutor`；**只包既有 RPC，不新增后端契约** | 真实往返用例：工具调用 → 断言副作用真的发生（R-10：成功⇒副作用已发生，否则 `NOT_IMPLEMENTED`/`degraded:true`） |
| c. MCP 权威枚举 | `capability_cards.rs:225-227` 从「只有计数」改为**枚举 + 每 server 的工具名**（入口已有 `plugins.mcp.list`，属读取而非新契约） | 用例：MCP 配了 2 个 server 时，能力卡 `tools` 非空且与 `plugins.mcp.list` 返回一一对应；未配时**如实为空并标注原因** |
| d. 意图→能力族→工具映射 | 提示词补「禁止用脚本偏盲控电脑」负规则 + 映射表（`docs/plans/2026-10-26-…remediation.md` W4 已设计 `File Shell Browser DesktopUia DesktopPixel Memory Knowledge Skill Mcp Cron Team` + `forbidden_shortcuts`，**未全落地**） | 离线可测：给定意图，brief 文本含正确能力族且不含被禁捷径；`check-doc-cli-drift` 不受影响 |

**收益**：直接对应主张 1/1b/2 ——「智能体不懂调用什么工具、技能、mcp、记忆、知识库，都是写脚本偏盲控电脑」。
**成本**：中（全部在 `core/src/agent/`，无新契约）。**风险**：`FIXED_FACE` 有 19 个上限（`tool_schema.rs:39/:46-66`），
新增工具**必须走发现路径**（`_marguerite_discover_tools` 每次最多补 8 个 schema），否则首轮工具面膨胀会伤缓存命中。

### P0-2 协调联动健壮性：任意开关组合都要能把任务做完 —— **待做（本轮头号）**
先建**开关清单**（本轮实测，勿再凭印象）：运行时模式 / 编排形态（single·subagent·team·expert·swarm）/ 蜂群总开关 + 规模 /
子专家数 / Bloom 并行 / 团队名册 / 子代理角色多选 / MCP / Skills / 记忆族（`auto_capture`·`synapse`·`tiered`·`dream`·`squilla_router`）/
桌面与浏览器自动化审批档 / self-evolve / 推理档位（3×3 `UserMode` × `reasoning_effort` × `thinking_depth`）。
读取点与前端设置点逐一对应（`gateway/orchestrate.rs:237/264/271/299/305`、`mode/spec.rs:210-246`、`mode/swarm.rs:60/64`、
`persistence/config.rs:133-192`、`gateway/mode.rs:21/187/207`、前端 `ChatComposer.vue:906/916`、`ModeMatrix.vue`、
`SettingsMemoryPanel.vue`、`SettingsAdvancedPanel.vue`、`RunModePanel.vue`）。

工程内容：
1. **组合语义表**（现在完全没有）：每种「缺失/冲突」组合给出**明确结局**——要么按声明的降级路径继续、要么响亮报错。
   现有 4 处静默降级（`orchestrate.rs:407-412`、`subagent.rs:912`/`:1090`、`multi_strategy.rs:225`）统一改为
   **降级 + 在产出里如实标注**（`degraded:true` + 原因），绝不静默把「没派子代理」伪装成「派了」。
2. **矩阵测试**：`{全开, 全关, 只开子代理, 只开团队, 只开蜂群, 只开专家, 记忆关+其余全开, MCP 关, 工具面为空}`
   ≥ 9 个组合的**离线确定性**用例，每个都断言：任务**产出非空** + 质量门（`CompletionGate` 四检查）通过 + 降级原因可见。
   这是该计划里**明文遗留未补**的一项。
3. **产出质量守恒**：用户要求「不管怎么开关，特别是产出任务的质量」。判据写成可机检：
   `CompletionGate` 的自报文件落盘核验**不得因降级被跳过**；任何降级路径都要给出「少了什么能力、因此少做了什么」。

**成本**：中高（`gateway/orchestrate.rs` + `orchestration/mod.rs`）。⚠️ 这两个文件**正被并发会话写入**，施工前必须确认冻结。

### P0-3 重新打包并核验安装版（把已修的东西送到用户机器上）—— **待做**
主张 4/4b 的真正解法。步骤按既有 skill/文档纪律：
1. **必须走 `scripts/package_full.cmd`**（直接 `cargo tauri build` 会用上一次的 `resources/` 快照 —— 实测过装出来的 pyengine 是 9/24 的副本）。
2. 核验三件事：①解包 MSI（`msiexec /a`）后以解包目录为工作目录启动，探 `http://127.0.0.1:18790/api/config` → **200 = 双引擎在线**（501 = 纯 Rust 降级）；
   ②真机验四项**从未验过**的：桌宠拖出窗体、面板收缩、diff 颜色、团队成员卡片；③`webui/dist` 里确有 `app-context-menu`（右键缺陷的判定物）。
3. ⚠️ 新暴露风险：**安装目录＝数据目录**的覆盖安装场景，须一并核验（`MARGUERITE_CONFIG_DIR` 只解决配置目录，不解决资源覆盖）。
4. 版本线滞后：`0.2.17`（引擎）vs `0.2.13`（releases tag）→ 本次要一次追平。

### P0-4 交付前澄清问答流（`deck_craft` PPT 团队首例）—— 详见 §5.1

### P1 清单
1. **推理档位与记忆全路径贯通**（§5.2 矩阵）。含机制根因修复：`tokio::spawn` 丢 task-local → 显式 `TURN_EFFORT.scope` 传递。
2. **多模态可发现性 + 参数面 + 素材源 + 编排 + 运行环境**（不是「补工具」）：
   - 把 `image_generate`/`video_generate` 等从「目录里有」变成「主智能体在相关意图下**会主动用**」；
   - 参数面（尺寸/时长/种子/风格/负面词）在 UI 与工具 schema 两侧对齐；
   - **素材源**四选一（网络 / 自动生成 / 指定目录 / 用户自填）—— 这正是用户对 PPT 团队提的同一套语义，可复用为一个组件；
   - **text-to-3D 是唯一真空缺**：`blender/`（远端仓已有：spec JSON→bpy→glTF/GLB→webui 热加载 + `validate_assets.py` 无 Blender 也能 CI 校验）
     是最短路径；⚠️ 依赖本机 Blender，产物体积不得进 MSI。
3. **团队成员卡片的后端遗留**：`solo` 恒空是**设计**（`member_registry.rs:20`；`orchestration/mod.rs:1171-1199` 不调 `begin_orchestration:2633`）
   → 单聊直答时看板永远空态，需要产品决策（补投影 or 明示「 solo 不入看板」）；
   观察事件**只在 ProgressReporter 路径发出**（唯一调用点 `agent/subagent.rs:187`；三处 `run_failed` 兜底绕过投影）
   → lead 在快照里永远 running，前端已用事件流收口，后端应补投影侧终态。
   ⚠️ 交接文档里的 `member_progress/expression/estimate/joined` **不是编排事件名**（分别是 tree-sitter 节点名
   `guard/semantic.rs:163`、Rust 参数名 `mod.rs:2637`、协作事件 `collab/session.rs:43`）—— 以此为据的后续施工会走空。
4. **`sessions.fileChanges` 是进程内内存台账，daemon 重启即空**（真实用户可见缺陷，已记录未修）。
5. **同类居中残留**：`ChatView.vue` scoped 块内的 `.chat-orch-runs`（本轮已确认**不需要改**，容器无居中；
   若将来统一口径必须与 `OrchestrationRunCard.vue` 的 `margin-inline` 联动，否则 64px 叠两次）。

### P2 清单
1. **i18n 硬编码中文棘轮（ratchet）**：`AGENTS.md` 建议的「基线只降不升」仍未做（本轮实测基线 **408 行**，已由守卫记录为不增长）。
2. **文档过期修正**：`AGENTS.md` §0/§2（LICENSE/.github 阻塞已解除、基线数字、`--all` 项数）、两份 README 的 Quality gates 表、
   `webui/dist` 与安装包滞后声明。
3. **`docs/assets/*.png` 重生成**（截图会随代码过期且无门禁告知，`AGENTS.md` §4 有完整可复现管线）。
4. **两个已知 501**（`GET /api/system/update`、`POST /api/elevated-mode`）：前端**已在正确降级**，是噪声不是缺陷 ——
   **不要按缺陷去修**（R57 已更正过一次，别再回退）。
5. **`computer_use.approval` 字段**：缺字段时省略徽章、不伪造（现状符合 R-10，保持）。
6. **W6.7 真机场景**、**W5.5/W4.7 在线基准**（需 provider key）：本轮环境不具备，不得声称完成。

---

## 5. 三个专题方案

### 5.1 交付前澄清问答流（`deck_craft` 首例）

**用户要的流程**（PPT 团队，制作前先弹问答窗）：
- **A 操控电脑** → 追问用哪个软件（WPS 等）→ 观察该软件**有无会员**、能否用会员素材库/模板/字体 → 确认**大纲**与**图片来源**
  （A 网络 / B 自动生成 / C 指定目录 / D 其他=用户自填）；
- **B 脚本生成** → 同样确认大纲与图片来源（A/B/C/D）；
- **C 其他** → 用户自填。
- 并要求 PPT 团队去抖音/小红书等媒体平台与各大 PPT 素材/模板库「学习改进」，其他团队举一反三。

**现状核对（关键，省掉一次「从零建」的误判）**：
| 需要的东西 | 现状 | 结论 |
|---|---|---|
| PPT 团队本体 | **已存在**：`deck_craft` 名册（`core/src/gateway/team.rs:64`，测试 `:801`），前端已有标签 | 不是从零建 |
| 前端问答卡 | **已存在**且**已含「其他」自定义输入**：`webui/src/components/chat/ClarifyCard.vue:113` | 不缺 UI |
| 提交链路 | **已存在**：`useChatApprovals.ts:1025` → `rpc.call('chat.clarify_submit')`（`protocol.rs:556`） | 不缺 |
| 题库/队列数据结构 | **已存在但是死代码**：`core/src/mode/ask.rs:12-93`（`Question/QuestionOption/Answer/QuestionQueue{ask_many,answer,pending_count}`）**全仓零消费者** | **真缺口在这里** |
| Rust 侧澄清 RPC | `chat.clarify` 落在 **`PY_ENGINE_PREFIXES`**（`gateway/methods.rs:125`）→ 纯 Rust 模式下澄清链**断裂** | 需去 PY-first 依赖 |

**施工**：① 给 `QuestionQueue` 接上消费者（Rust 侧 `chat.clarify` 直接实现，不再前缀转发；侧车在线时保持兼容）；
② 在 `deck_craft` 团队执行前插入「形态选择 → 条件追问 → 大纲/素材源确认」两到三步队列（`ask_many` 天然是队列，够用）；
③ 素材源四选一（网络/自动/指定目录/自填）**做成可复用组件**，与 P1-2 的多模态素材源共用一套语义；
④ 「会员/模板库可用性」属**观察结果**，必须走既有 computer-use 感知层如实回报（有则有、无则无），
   **不得**在提示词里假定「WPS 有会员」；⑤ 平台学习（抖音/小红书/模板库）应实现为**技能 + 素材清单**而非运行时抓取承诺 ——
   抓取涉及外部站点与登录态，属 `AGENTS.md`/本轮 Non-goals 的敏感区，**先做人工整理的模板库索引 + 技能，不做自动爬取**。

**验收判据**：`deck_craft` 团队在**纯 Rust 模式（无侧车）**下能产出一次真实问答往返，用户选「A 网络」时产物真的引用网络素材源、
选「C 指定目录」时只用该目录；`QuestionQueue` 的零消费者状态被单测反向钉死（不允许再退回死代码）。

### 5.2 推理档位 × 记忆 全路径矩阵（本轮实测，作为施工 checklist）

| 执行路径 | 推理档位 | 记忆读 | 记忆写 | 关键证据 |
|---|---|---|---|---|
| 主回合 chat | ✅ | ✅ | ✅ | `gateway/chat.rs:1567-1568/1200/1585/1627-1635`、`:1151 remember` |
| subagent | ✅ 继承 | ❌ | ⚠️ 仅靠 `memory_save`（**不在 `FIXED_FACE`**） | `tools/builtin/memory_tools.rs:53` |
| multi_strategy / 蜂群 | ❌ | ❌ | ❌ | `multi_strategy.rs:310`（`chat_with_tools` 无档位解析） |
| Bloom | ✅ | ⚠️ 部分 | ⚠️ | `mode/spec.rs:149/491` → `orchestration/mod.rs:2245-2258 resolve_effort` |
| 专家 | ❌ | ❌ | ❌ | 只消费 `expert_prompt`（`orchestration/mod.rs:3536/3706-3708`） |
| 团队 | ❌ | ❌ | ⚠️ | `gateway/team.rs:723/766` 只用 `auto_profile(&history)` |
| 编排 | ✅ | ⚠️ 部分 | ⚠️ | `chat.rs:2688 set_experience_hint` |
| chain | ❌ | ❌ | ❌ | `chain/executor.rs:166/248-249/660` 有字段但**无 setter 调用方** |

**结构性缺失**（这是「档位和记忆对它们都有用」做不到底的根因）：
**没有任何携带 per-role 档位的结构**。`ExpertProfile`（`agent/expertise.rs:22-38`）与 `Persona`（`profile.rs:93`）
全是 `&'static str` 文本字段，**没有 tier/model/effort**；档位只存在于模式级 `BloomSpec`（`mode/spec.rs:210-246`）与
`ModelParams`（`:143`）；角色→模型映射只有 `multi_strategy.rs:396-417 resolve_role_model`（未命中回落 `default`）。
→ 第一步是**给角色加可选 `{effort, model}` 覆盖位**（缺省继承会话档位，保持向后兼容），再把解析点从「模式级」
扩到「角色级」，并**显式跨 `tokio::spawn` 传** `TURN_EFFORT`（task-local 不自动继承，这是实测根因）。

**边界**：记忆写入仍必须走 `memory_save` 的既有治理路径，**不得**为「让子代理有记忆」开第二条写库通道；
`AgentRecord.model` 一旦接线就构成真实计费行为，需要 UI 明示与审批（现状是零消费，等于「选了模型不生效」）。

### 5.3 RSI（递归自我改进）移植 —— 主体更正 + 可落地机制

**先更正两处引用（用户原话有偏差，按检索结果修正）**：
- 「哥伦比亚大学的 EmbodiedRSI」→ 检索到的是 **EmbodiRSI**，归属**上科大 + 瞬适**（arXiv 2609.38905）；
  **「哥伦比亚大学」未能一手证实**，不要把该归属写进对外文档。
- 「穹智能 10 月的 RobotRSI」→ **RoboRSI**，归属 **Noematrix（穹彻智能）**（arXiv 2610.12424）；
  另有 **SenseNova-RoboRSI（商汤）** 是**不同主体**，引用时必须区分。
- ⚠️ 上述编号系子代理检索所得，**本会话未逐条回链核验**；对外发布前需回原文确认（本文件的纪律：未核验不写成事实）。

**本项目已有的 RSI 骨架（不是从零）**：`core/src/harness/{meta,evolve,eval,ledger,distill,checkpoint,routing,subagent}.rs`，
且**三条类型强制边界是真实存在的代码**：`ChangeTarget` 枚举（`meta.rs:350`）**没有 core 源变体**；
`MetaPolicy::auto_apply` 默认 `false`（`meta.rs:456/476`）；`meta.rs:930` 注释明确「无论 `auto_apply` 与否都**不会改任何真实文件**」。
→ 与 `AGENTS.md` 两条红线（上下文 append-only、工具面单调扩展）一致，**这是资产，不是负债**。

**值得移植的三条机制**（按性价比排序，全部要求「外生验证」）：
1. **Trust 锁 + git 留痕**（RoboRSI）：任何自我改进提案落盘**必须**留下可 diff 的变更集与回滚点；
   本项目已有 `checkpoint.rs`/`ledger.rs`，缺的是「**提案 → 变更集 → 外生验证 → 才允许合并**」这条强制链。
2. **三层技能树 + Manager/Planner/Engineer/Reviewer 分工**（RoboRSI 的 TSR）：与本项目的
   主智能体/子代理/专家/团队天然同构，落到 §5.2 的 per-role 档位上最省事 —— **Reviewer 必须用外生指标**，
   不用自评（文献里「自评门失效」是反复出现的结论；本项目 `CompletionGate` 的四检查 + 自报文件落盘核验已是外生门，别绕过它）。
3. **编辑预算 / 改动半径上限**（DGM 系）：给每轮自改进设**可改动文件数上限**，与 `ChangeTarget` 的窄枚举叠加，
   防止「智能体改智能体提示词」悄悄扩大。

**红线（不可协商，写进实现就不许回退）**：
- 不修改 `core/` 自身源码作为改进手段（`ChangeTarget` 已无该变体，**不要为 RSI 加这个变体**）；
- **不以「压榨潜能」为名关闭 `PlanGate`/`CompletionGate`/审批/限流**（2026-10-26 计划明文；「破甲」只允许三类：
  能力评测、提示与上下文优化、失败可观测）；
- 不提取隐藏思维链或系统提示；
- 新增指标必须定义分子分母（R-13：`read/(read+creation)` 在 DeepSeek 上恒 100% 就是「假指标」的现行案例）。

⚠️ **正面冲突，需用户复议**：转写里用户曾要求「取消置信度阻拦任务计划执行」，而上述红线与 `AGENTS.md` §3.5
要求保留 `PlanGate`。二者不可同时成立。本文件**默认保留治理门**，只把「置信度不足时」的行为从「拦住」改成
「**照做但显式标注不确定性 + 事后 `CompletionGate` 兜底**」——这是唯一同时满足两个诉求的解法，需用户确认。

---

## 6. 可复用资产（用户 GitHub 69 仓 + 本地模型项目）—— 逐条标可用/不可用

| 资产 | 能补哪个缺口 | 结论 |
|---|---|---|
| `Marguerite` 远端仓已含 `blender/`（spec JSON→bpy→glTF/GLB→webui 热加载 + 无 Blender 也能跑的 `validate_assets.py`） | P1-2 **text-to-3D 真空缺** | ✅ **最短路径**；需本机 Blender，产物不进 MSI |
| `KLJyouth-office-Modle`（Qwen2.5-1.5B + 4-bit QLoRA，4327 样本，RTX 3050Ti 4GB；GGUF `q4_K_M` 940 MB） | 无 API Key 时的意图/工具域分类 | ⚠️ **只能做候选排序提示，不可做决策权威**：宏平均 0.3936、`directed_routing` 0.178 ≈ 随机 0.167、`nli_*` **低于随机**，且标签掩码 off-by-one 修复后**未重训**；940 MB **绝不能进安装包**（现 252–264 MB）。`local_models.rs`/`registry.rs` 有通用 GGUF 档位支持，但**未搜到该权重的登记项** → 「已被引擎自动发现」不成立 |
| `yolo-ultralytics` 的 `best.pt`（v8n 6.2 MB / v8s 22.5 MB；@640 test mAP50 **0.8587** / mAP50-95 **0.6017**，转抄） | `ScreenPerception` 的目标定位 | ❌ **不适用 GUI grounding**：权重是 **VOC 交通 6 类**（4775/597/597），不含按钮/输入框，需换 UI 元素集重训；且 ultralytics 是 **AGPL-3.0**，链路分发有许可风险。项目**非 git 仓库**，源码树是否被本地改动未验证 |
| `yolo-ultralytics/make_doc.py` + `make_figures.py`（python-docx，全部数字从 `results.csv`/`metrics_*.json` 真读） | 成果物生成（docx/pdf） | ✅ 可作 workbench `artifact.docx.build` 的范式（**真实数据驱动、不编数字**，与 R-13 同精神）；依赖 Windows 中文字体 |
| `nms_compat.py`（CPU 构建无 CUDA NMS 的降级补丁，~20 行） | 侧车降级即崩 | ✅ 低成本 |
| `archived-agent-s`（Agent S/S3，OSWorld 72.6%）+ `awesome-computer-use` | computer-use **客观回归集**（当前只有 e2e 冒烟，无基准） | ✅ 只取**任务规格**做离线夹具；⚠️ 私有仓未本地检出，可运行性未证 |
| `tmp-t1`（Codewhale：按角色钉模型/fleet/自写宪法） | §5.2 的 **per-role 档位/模型** | ✅ 设计参照（不是代码来源）；上游镜像非自有 |
| `gui-agent`（UnivAgent PRD + nexustar 双模型带队） | MAX 双模型/蜂群编排规范 | 📄 只当文档复用；停更 2 个月，未本地检出 |
| `3d_game_framework`（MetaverseGame/NativeEngine 643 MB） | 《3D WebGL Agent Workspace》笔记里的岛屿工作台 | ⚠️ 未接、未证可运行 |
| `Marguerite-Releases` | P0-3 打包 | ✅ 唯一滞后项（tag 停在 0.2.13） |

**两份设计笔记**：《3D WebGL Agent Workspace》的**铁律「没有数据就不建建筑，绝不虚构运行状态」**与无 WebGL 时
明确降级 2D 而非假装 3D，**正是本项目 R-10/R-13 的同一精神**，应作为岛屿工作台实现的验收判据；
《多模态模型训练改进与优化》记的实测硬件结论（**PyTorch 2.11+cpu，训练根本没用上 GPU** 是真根因）
与三档 Flagship/Balanced/Turbo 的 `accepts()` **只校验 4/6 项**（Turbo 漏 `streaming_stable`）是可直接修的既有缺陷。

---

## 7. 里程碑

| 里程碑 | 内容 | 完成判据（可机检） |
|---|---|---|
| **M1（本轮已完成部分）** | 居中排版全量收口、团队成员卡片前端、桌宠右键契约钉死 | `chat-reading-column.contract.test.ts` 25 例 + 受影响 128 例绿；`TeamBoardView`/`useTeamBoard`/`components/team` 73 例绿；`AppContextMenu.suppression.test.ts` 6 例绿；`vue-tsc --noEmit` exit 0；13 项架构守卫绿 |
| **M2** | P0-1 自认知（a–d）+ P0-2 开关矩阵 | 子代理提示含内置应用面（单测反向钉死旧缺口）；≥9 组合矩阵用例全绿且产出非空 + `CompletionGate` 通过 + 降级原因可见 |
| **M3** | P0-4 澄清问答流（`deck_craft`）+ §5.2 档位/记忆矩阵 8 条路径 | 纯 Rust 模式下真实问答往返；矩阵表 8 行全部 ✅ 或有单测说明为何不适用；`QuestionQueue` 不再零消费者 |
| **M4** | P0-3 打包 + 安装版四项真机核验 + §5.3 RSI 三步机制（带 Trust 锁） | 解包后 `/api/config` **200**（双引擎）；`verify.sh --clean --release` 全绿 |

**每个里程碑的统一验收**：`bash scripts/verify.sh --clean --all` **25/25 + exit 0**（发版加 `--release`），
且 `[SKIP]` 不计入通过；本轮 §3 的 5 条 FAIL 必须在冻结树上逐条复跑消除，**不能靠并发环境「碰巧绿」**。

---

## 8. Non-goals（沿用已拍板，本轮不扩大）

不自研模型权重；不锁厂商默认；**不云端托管执行（数据不出端）**；不做 Electron；不做完整企业 IAM/SSO；
不做跨端移动 App；不重写引擎；不换 Tauri/Vue/FastAPI；**不合并 pyengine 与 Rust**；不新增公网监听、
不改「Web UI 仅回环」；不做伪预热；不删 `workbench`/`third_party`/`gui/src` 归档；不在本路线做 535 行硬编码中文全量翻译（只做棘轮）。

⚠️ **两条新引入的待复议项**：
1. 多模态与素材源若接**外部生成 API**，与「数据不出端/本地优先」正面冲突 → 需用户决定是「仅可选、默认关、明示外发」还是「只做本地链路」。
2. §5.3 的「置信度阻拦」与「保留 `PlanGate`」二选一（本文件默认后者 + 不确定性显式标注）。

---

## 9. 本轮已落地（截至 2026-10-27 12:00，**验证状态逐条标注**）

> 纪律：`[已跑绿]` = 本会话亲跑并贴了数字；`[待门禁]` = 代码在树里、`cargo check` 过但**未跑全量测试与 clippy**；
> `[未开始]` = 仅设计。**`[待门禁]` 与 `[未开始]` 都不构成完成声明。**

| 项 | 落地内容 | 验证状态 |
|---|---|---|
| P0-1a 内置应用面进子代理 | `core/src/agent/tool_catalog.rs:180-192` 新增 `BUILTIN_SURFACES_COMPACT`（名/一句话/工具名三元组），`:399-411` 在 `compact_catalog_text` 注入；两表名称由 `assert_eq!` 咬合，另有「不得把主面长文本整段复制」负断言与既有 `compact < full/2` 尺寸约束 | `[待门禁]`（`cargo check --lib` **exit 0 零警告**） |
| P0-1c MCP 权威枚举 | `core/src/agent/capability_cards.rs:207-266` `collect_mcp_facts()`（数据源全部既有只读点：`selfcheck` 的 `mcp.configuredServers` + `plugins::mcp_launch::list()` 内存快照，**渲染路径刻意不碰带网络副作用的 `mcp.tools.list`**）、`:272-333` 纯渲染 `render_mcp_line()`（上限 12 服务器 / 8 工具，截断标 `另有 K 个未列出`，零来源如实标不可用）；旧公开签名不变、调用方零改动 | `[待门禁]` |
| P0-4 澄清问答流（`deck_craft` 首例） | 新模块 `core/src/gateway/clarify.rs`（736 行，照 `approvals.rs` 的 forward-on-miss 形状）；`gateway/mod.rs:21` 注册；`gateway/methods.rs:1012-1018` **7 行最小拦截**（必须在 `is_python_method` 之前，否则纯 Rust 模式整条链只剩 `NOT_IMPLEMENTED`）；`mode/ask.rs` 给死代码 `QuestionQueue` 接上第一个消费者并新增分支能力 `QuestionDependency` / `allow_other` / `ready_batch()`；`gateway/team.rs:168-334` 定义问卷（形态 A/B/C → 条件追问 → 大纲 → 图片来源四路） | `[待门禁]`（14 条新测试未跑） |
| — 会员/素材库探测 | `MembershipProbe::{Available,Unavailable,NotProbed}` + `current_membership_probe()` **恒返 `NotProbed`**，三处文案（intro / 题头 / 题干）都写「未探测」并有断言咬住 | **诚实状态：未探测**（未新写屏幕抓取、未假定「WPS 有会员」） |
| — `PY_ENGINE_PREFIXES` 处置 | `methods.rs:125` 的 `"chat.clarify"` **保留**：它只影响 `is_python_method`，而新拦截在其之前返回；语义降级为「归属提示」，本地无队列时仍转发侧车，`dispatch_sync` 口径不变 | 设计决策，随代码 |
| P1-2 多模态参数面 | `webui/src/components/multimodal/{materialSource,generateParams}.ts` + `MaterialSourcePicker.vue` + 两个测试文件；**修了遗留两条真缺陷**：A（网络）的选填 URL 被抹成空串导致 `web → video.image` 联动恒不生效；`seed` 走 `parse::<i64>`（`video_gen.rs:443`）故负种子合法、原 `integer` 一刀切拒负数是误伤。暴露参数严格对齐后端 schema：image 7 项（`image_gen.rs:208-218`）、video 10 项（`video_gen.rs:578-589`）；`response_format` 与 4 个轮询键**故意不暴露**并由闭合世界断言钉死 | `[已跑绿]` `vitest src/components/multimodal/` **60 passed / 0 failed**；`check-i18n` exit 0（6 语 **6241** 键全等）；`check:architecture` **13/13**；`vue-tsc --noEmit` **exit 0** |
| — 组件挂载点 | **未挂载**（`MultimodalCapabilitiesPanel.vue` 有并发编辑风险）。接线点：`:81` 之前插 `<MaterialSourcePicker />`、`:107` 后加 import | **阻塞项见下一行** |
| 🔴 **通道缺口（新发现，需决策）** | 网关**没有通用 `tools.invoke` RPC** —— 只有只读 `tools.catalog`（`methods.rs:1799`）与三个固定 `tools.web_*`（`:879-881`）。所以「从 UI 一键发起一次真实生成」在当前契约面上**做不到**，只能由宿主经既有 `chat.send` 下发载荷。这既解释了用户「没有文生图/视频工具」的体感（工具有、**入口没有**），也决定了挂载方案二选一：A) 补 `tools.invoke`（新增后端契约，需评估）；B) 组件 `launch` 事件 → 组装 `chat.send` 载荷交主智能体执行（不加契约） | `[未开始]` 待用户拍板 |
| — 文案与代码漂移 | `MultimodalCapabilitiesPanel.vue:168` 原文案对用户宣称「视频生成工具**尚未内置**」，而 `video_generate` 早已注册（`builtin.rs:180-212`）→ 改为「工具 `video_generate` 已内置，缺的是端点配置」（行数不变，CJK 棘轮 408 不受影响） | `[已改]` |
| P2-2 文档修正 | 本 §2.1 记录了 `AGENTS.md` §0/§2 过期项；**未动 `AGENTS.md`/README**（并发写入中，见 §3） | `[未开始]` |

**尚未开始的 P0/P1**：P0-1b（工作台自身可控 `ToolExecutor`）、P0-1d（意图→能力族→工具映射 + 负规则）、
P0-2（开关组合守卫与矩阵用例）、P0-3（重新打包）、P1-1（§5.2 八条路径的档位与记忆贯通）、P1-3（团队看板后端遗留）。

---

## 10. 收发窗口（2026-10-27 13:00–14:20）落地与验证状态

> 纪律不变：`[已跑绿]` 只用在真跑过并留下数字的项上。**本窗口按用户明确指示「测试先全部跳过，先收尾打包」，
> 所以绝大多数条目只有编译证据，没有测试证据** —— 这里如实写成 `[仅编译]`，不得当完成。

| 项 | 内容 | 状态 |
|---|---|---|
| 桌宠可移出窗体 + 自有右键菜单 | `AppContextMenu` 抑制用例、桌宠右键契约 | `[仅编译]`（前端构建通过，未跑 vitest） |
| 会话历史居中/排版收口 | `chat-reading-column.contract.test.ts`、`ChatMessageList`/`messageIdentity` | `[仅编译]`（构建通过，未跑该 25 例） |
| 团队成员卡片 | `components/team/*`、`useTeamBoard.reduceProgressFrame` | `[仅编译]` |
| 澄清问答流（`deck_craft`） | `gateway/clarify.rs`（736 行）+ `mode/ask.rs` `ready_batch()` + `team.rs` 问卷 | `[仅编译]`（14 条新测试**未跑**） |
| RSI 机制（Trust 锁 + 编辑预算） | `harness/trust.rs`（`trust_gate` / `EditBudget` / `budget_gate`）、`harness/meta.rs` +258 行 | `[仅编译]`（文件内测试**未跑**） |
| 工作台 ToolExecutor（P0-1b 部分） | `tools/workbench.rs`（产物中心 / 团队看板，只包既有 RPC）、`tools/builtin.rs` 注册 | `[仅编译]`；本轮修掉 6 处 `Ok(...)` 类型错后才编译通过 |
| 素材源 / 多模态（P1-2 部分） | `tools/media.rs` 素材扩展名与目录枚举、`webui/components/multimodal/*` | `[仅编译]`；`media.rs` 仍有 5 处 dead-code 警告（对应未接线的 slideshow/concat 分支），**属未完成功能，不是已交付** |
| 档位 × 记忆全路径（P1-1） | `agent/role_context.rs` 等**半成品**：三个并行子代理在写入中途被限额中断 | ❌ **未完成**：只有类型骨架，四条路径未接线、无测试；不要把它当已修 |
| 打包 | 见下 | 进行中 |

**已知未完成（必须在下一窗口接手）**：P1-1 档位/记忆四路径、P0-1d 能力族负规则、P0-2 开关组合矩阵、
P0-3 安装版四项真机核验、`media.rs` 5 处 dead-code、`tools/workbench.rs` 的测试、
以及 `docs/assets/*.png` 重生成。**`verify.sh --clean --all` 本窗口未跑 —— 所以本轮不构成「全绿」声明。**

