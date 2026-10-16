# K2.4 三仓整合迁移风险点清单（UnivAgent Migration Risk Register）

> 研究型产出（无代码改动）。基于 K2.2 重复/冲突/互补矩阵 + 本轮对 5 项「待核实」的实证复核。
> 路径与结论均来自 `ls`/`find`/`Grep`/`Read` 实证（与 K2.2 同口径）。
> 关联文档：K2.2 `bloom-mode-consolidation-matrix.md`、战略计划 v2.0 §3、K3 `docs/univagent-integration-doc-v1.md`、桌面迁移 `GUI_Agent/docs/desktop-migration-plan.md`（见 §〇 范围切割）。

---

## 〇、范围切割（避免与既有文档重复）

- **本清单范围**：三仓（archived + active + Marguerite）**跨仓资产整合的迁移风险**（K2 职责：D1–D11 / C1–C7 / M1–M11）。
- **不覆盖**：Tauri 桌面端重写迁移——该职责已由 `GUI_Agent/docs/desktop-migration-plan.md`（Phase A 方案文档，含 Tauri v2 / WebView2 / Sidecar / 离线模型 / 构建安装包）独立承担。本清单仅在 D6 / M8 处引用其结论，不重述桌面专项。
- **与 K1.4 关系**：C2 已被 K1.4 按修正表实现并 `tsc` 通过 → 本清单将其标记为「✅ 已消解，残留运行时验证」。

---

## 一、本轮实证更新（5 项待核实处置 + 3 项新发现）

### 1.1 待核实项处置结果

| 原待核实 | 实证手段 | 结论 | 对风险的影响 |
|----------|---------|------|-------------|
| ① v1 官网是否仍在线 | `ls` v1 目录 + `grep -rln "nexustar-website"` | v1 为 Vite 工程，`README-ARCHIVED.md` 标注归档；`nexustar-ui` 无 import；引用仅出现在 `.trae/specs/` 与 `.workbuddy/memory/*.md` 规划笔记 | **D1 降 🟢**：删 v1 `src/` 安全；残留=确认部署侧未静态托管其 `dist/` |
| ② hermes-skills schema 差异 | `ls gwwc/skills/builtin/hermes-skills/` | 该目录仅剩 `creative/meme-generation/`（单一子技能），原技能包已撤离 | **D2 降 🟢**：几乎无可合并内容；仅校验 `meme-generation` SKILL.md 与 `marketplaceStore.MarketplaceSkill` 兼容 |
| ③ archived cascade 与 nexustar-api 重迭 | `find` archived + `grep` nexustar-api | archived 确有 `gwwc/agent/adaptive_model_cascade.py` 与 `agent/model_mgmt/adaptive_model_cascade.py`；但 nexustar-api **已落地** `backend_switcher.py`（健康探测+心跳+故障转移+回切+`app.event_bus` 推送 `backend.switch`）+ `tests/test_backend_fallback_model.py` + `tests/test_backend_resilience_b1_qa.py` | **D8/D10 降 🟢🟡**：降级/切换已在线上后端就位，archived 仅参考，勿盲移植 |
| ④ TUI↔GUI bridge 文件 | `find *bridge*` + `grep bridge docs/` | 后端确有 `cross_project_bridge.py`（跨项目记忆共享，n-gram 余弦，敏感字段 REDACTED）+ `browser_bridge.py` + 测试 + `verify_bridges.py`；但 **Marguerite TUI 侧无 bridge 文件** | **M1 部分落地**：API 侧记忆桥已实现；TUI↔API 会话/技能契约仍待作者 |
| ⑤ swarmStore spawn 状态 | `grep` swarmStore.tsx | 已是 `SwarmRunStatus/SwarmStepView/SwarmLogEntry/SwarmStoreEvent/SWARM_EVENT_NAMES`（run/step/event 模型），`sourceKind:'sse'|'poll'|'none'`，对齐 `orchestrator.py` 的 `swarm_*` 事件 | **C4/D7 已基本落地**：数据模型已 spawn 兼容，J3 固定实体卡转换工作量极小 |

### 1.2 新发现（影响门禁）

- **F1 — 后端已有 `app.event_bus`**：`backend_switcher.py` 经 `app.event_bus` 推送 `backend.switch`，前端经 SSE 消费。说明 **EventBus 存在于后端、不存在于前端**（C2 仅指前端）。含义：未来前端跨模块广播应复用 `api/sse.ts` 或 store，而非误以为后端 event_bus 可直连前端。
- **F2 — `cross_project_bridge.py` 是记忆共享桥，非会话/技能契约**：它读 `~/.trae-cn/memory/projects/` 的 `project_memory.md`/`topics.md`，与 M1「会话/记忆/技能数据契约」只覆盖记忆层；**会话与技能契约仍缺两端实现**。
- **F3 — `docs/desktop-migration-plan.md` 已存在且完整**：Phase A 方案文档覆盖 Tauri 迁移全链路。本清单与其**互补不重叠**，桌面专项迁移风险以该文档为准。

---

## 二、迁移风险总表（按严重度，含实证证据与处置）

> 每行：风险ID / 当前严重度 / 触发条件 / 触发后果 / 迁移动作 / 验证门禁 / 回滚 / 关联 Phase。
> 符号：🔴 高　🟡 中　🟢 低　✅ 已消解。

### 🔴 高严重度（2 项遗留 + 1 项已消解）

| ID | 严重度 | 触发 | 后果 | 迁移动作 | 验证门禁 | 回滚 | Phase |
|----|--------|------|------|----------|----------|------|-------|
| D3/C5 | 🔴 | 三套渲染范式（R3F `VoxelWorld` / Canvas2D `useSplashParticles`+`useAudioParticles` / 计划 Bloom）并存 | 包体积膨胀、FPS 不达标（Bloom 标准 §性能）、视觉不一致 | K1.4 `BloomBackground` 已立为唯一持久型 R3F 基座；Splash 保留一次性 Canvas2D；`useAudioParticles` 复用 Bloom `useAudioAnalyzer`+双核 FFT（M6）；`VoxelWorld` 评估弃用或并入 | `useFpsMonitor`：FPS<30 减半、<20 暂停；三处视觉统一到同一 rAF+three 管线；`tsc` 通过 | 抽离 Bloom 为独立组件，旧视觉回退 Canvas2D | 0 末 / K1.4 已启 |
| D5 | 🔴 | ≥4 处 DAG/编排视图（`OrchestrationCanvas`+`Dashboard` / `AgentFlowCanvas` / `swarm/SwarmDAGView` / `gome/CockpitPanel`）节点/边数据模型各异 | 跨屏编排状态不可互通，swarm/gome 成数据孤岛 | 收敛到单一 `OrchestrationCanvas`，swarm/gome 作该画布「模式」；统一节点/边 schema；`SwarmDAGView` 引用同一 store | 跨屏打开同一编排任务状态一致；节点增删在任一视图同步 | 旧画布保留为兼容壳，引用点逐步切换 | 1 |
| C2 | ✅（残留🟡） | 原规划 Bloom 订阅不存在的 EventBus | 接错则特效永不触发 | K1.4 已改为 `useModeStore`+`api/sse.ts` 订阅（C2 修正表）；零新增全局机制 | `tsc --noEmit` EXIT=0；浏览器运行时确认 SOLID_APPEAR/DISSOLVE/PARTICLE_BLOOM 按消息生命周期触发 | 无（仅文档对齐） | 0 末 |

### 🟡 中严重度（含 1 项需用户拍板的阻塞）

| ID | 严重度 | 触发 | 后果 | 迁移动作 | 验证门禁 | 回滚 | Phase |
|----|--------|------|------|----------|----------|------|-------|
| C1 | 🟡 | 5 套状态机制（core/store / core/shu / core/instances / Context / useReducer）并存，特征态易落错层 | 难测试/迁移；新组件误用遗留层 | 立规：特征态一律 `core/store.ts create()`（bloomStore 已示范）；冻结 `instances.ts`/`shu.ts` 仅作遗留/编排 | lint 禁止业务态 import `instances.ts`/`shu.ts`；新增 store 100% 走 `create` | 无（增量约束） | 0 |
| C3/D9 | 🟡 | 主题变量体系（`theme/tokens.css`）与 6+ 散落根 CSS 硬编码色并存 | 切主题部分界面不跟随，选择器优先级冲突 | 新组件读 `--nx-color-*` 变量；散落 CSS 逐步并入设计系统（仿 `useSplashParticles::readBrandTokens`）；硬编码色清理 | 四主题切换无「半主题化」残留；组件 CSS 无 `rgb()/hex` 硬编码 | 旧 CSS 留兼容壳 | 持续 |
| C6 | 🟡（阻塞 G3） | PRD P0-3 本地 SQLite `user_id` 分区 vs P1-4 Java `admin-backend` 双写 | 用户态双写不一致，破坏 G3 数据隔离 | **需用户拍板单事实源**：先 Python 本地（P0-3），Java 上线后仅同步禁双写 | 单一写入路径；跨端用户态一致性测试 | 冻结 Java 写，回退 Python | 3（需先决策） |
| M1 | 🟡 | TUI↔GUI 共享「会话/记忆/技能」契约未落地；仅 API 侧记忆桥存在 | 双形态数据不互通，TUI 与 GUI 各说各话 | 以 `cross_project_bridge.py` 为记忆层基础；补会话/技能契约（OpenAPI 复用 + 统一类型）；Marguerite TUI 侧 bridge 文件待作者 | TUI 与 GUI 对同一会话/技能读写一致；契约有测试 | 契约未定前 TUI/GUI 各自独立，不强行共享 | 持续 / Phase E |
| D4 | 🟡 | 聊天旧壳 `ChatSection` 与 `unified` 壳并存 | section-convergence churn，新卡片逻辑漏接 | 统一 `unified` 壳；`AgentScreen`/`SwarmScreen` 重构后删旧 `ChatSection` | 新旧壳下消息流行为一致；无重复渲染 | 旧壳保留兼容壳 | 1 |
| D11 | 🟡 | MAX 双带队 UI 分散（MaxDualPanel/DualLeaderPanel/MaxModeBanner + workbuddy/* 沉浸态） | PRD①「零冗余」不达标，双带队行为与沉浸态不一致、难统一测试 | 收敛到统一 MAX 子系统（`chat/MaxDualPanel` 卡片核心，`workbuddy/*` 沉浸态）；去重 `DualLeaderPanel`/`MaxModeBanner` | 双带队卡片与沉浸态行为一致；单一测试入口 | 旧组件保留兼容壳 | 1（PRD①最高优先） |

### 🟢 低严重度（实证已降级，纯清理 / 已落地）

| ID | 严重度 | 触发 | 后果 | 迁移动作 | 验证门禁 | 回滚 | Phase |
|----|--------|------|------|----------|----------|------|-------|
| D1 | 🟢 | v1 官网 `src/` 与 v2 `nexustar-website-next` 双版本漂移 | PRD P1-3 仅指向 v2，v1 不应接需求 | 删 v1 `src/`（已证实无活跃 import）；保留 `README-ARCHIVED.md`；**先确认部署侧未托管 v1 `dist/`** | 无构建引用 v1；站点仅 v2 | 从 git 恢复 v1 `src/` | 3 |
| D2 | 🟢 | hermes-skills 与 marketplaceStore 技能隔离 | 桌面端能用、官网市场看不见 | hermes-skills 已仅剩 `creative/meme-generation`；校验其 SKILL.md 与 `MarketplaceSkill` 兼容；`gwwc/skills` 整体弃用 | `meme-generation` 在市场中可见可用；无技能加载路径断裂 | 无 | 持续 |
| D6 | 🟢 | 双 admin（`admin-backend` vs `_deprecated_admin_*` vs archived `gwwc/admin`） | 维护负担 | 保留 `admin-backend`；删两类遗留 admin（已 `_deprecated`） | 无引用指向遗留 admin | git 恢复 | 持续 |
| D7 | 🟢（已 spawn 兼容） | 固定 swarm 实体 vs spawn 模型 | J3 实体卡设计冲突 | swarmStore 已是 run/step/event 模型，已基本 spawn 兼容；J3 改为 spawn 卡（D7）；等 Phase E swarm 子系统 | swarm 事件经 `SWARM_EVENT_NAMES` 对齐 `orchestrator.py` | 无 | Phase E |
| D8/D10 | 🟢🟡 | archived 降级逻辑移植 | 双写/漂移 | nexustar-api 已有 `backend_switcher`+fallback+resilience 测试；archived cascade 仅参考，不盲移植；差异点 diff 后择优 | 后端切换/回切测试通过；`backend.switch` 事件前端消费 | 无（线上实现为准） | 0 |
| M9/M10 | 🟢 | 运维/文档双源 | 脚本分散、结论双写 | 保留 `nexusops` 为运维权威入口；`GUI_Agent/docs` 为文档唯一真相源；archived 仅追溯 | 新运维/文档不写回 archived | git 恢复 | 持续 |

---

## 三、迁移时序与门禁（consolidation-first）

> 原则：先立规范/地基（C2/C1/D8/D10 已落地）→ 再收敛视觉（D3/C5/M6）→ 后清遗留（D1/D6）→ 跨屏整合（D4/D5/D11/C4）→ 长期（M1/M3/M4/C6）。每阶段以门禁通过为进入下一阶段前提。

| 阶段 | 关联风险 | 进入下一阶段门禁 | 状态 |
|------|---------|----------------|------|
| Phase 0 地基/可靠性 | C2✅、C1、D8/D10✅、M7 | `tsc` 通过；状态规范 lint 通过；后端切换/回切测试通过 | 进行中（C2/K1.4 已消） |
| Phase 0 末 / K1.4 启动 | D3/C5/M6 | Bloom 为唯一持久视觉基座；FPS 纪律生效；音频复用同一 FFT | BloomBackground 已实现，待运行时验证 |
| Phase 1 MAX 体验 | D4、D5、D11、C4/D7✅ | 跨屏编排状态互通；MAX 子系统行为一致；单一测试入口 | 待启动 |
| Phase 2 执行链路 | M3、M4 | LSP 诊断 / 市场治理进 P1 路线图 | 待启动 |
| Phase 3 官网/账号 | D1、C6 | 单官网 v2；账号单事实源（需用户拍板 C6） | 待启动 |
| 持续治理 | C3/D9、M9/M10、D6、D2 | 样式变量化；运维/文档权威源；纯清理 | 随日常 PR 进行 |

---

## 四、已落地能力盘点（正面信号 — 不必重做）

避免重复造轮子，以下能力**已被现有代码消解**，迁移时直接复用而非重建：

1. **HTTP 客户端 SSOT**：`api/client.ts` 已收敛，`http.ts`/`client-root.ts` 退为兼容壳（K2.2 〇基线）。
2. **后端降级/切换**：`backend_switcher.py` + fallback_model + resilience 测试已在线（F1 / D8 / D10 已落地）。
3. **跨项目记忆桥**：`cross_project_bridge.py` + `browser_bridge.py` 已实现记忆层共享（M1 部分落地）。
4. **swarm 运行模型**：`swarmStore.tsx` 已 run/step/event，对齐 `orchestrator.py`（C4 / D7 已落地）。
5. **状态规范范本**：`bloomStore.ts` 已示范 `core/store create()`（C1 范本）。
6. **桌面迁移方案**：`docs/desktop-migration-plan.md` Phase A 已完整（D6 / M8 桌面专项以该文档为准）。

---

## 五、残留不确定性（需用户拍板 / 待确认）

1. **C6 账号单事实源（阻塞 G3）**：Python 本地优先 vs Java admin-backend 同步策略未定，需用户决策后方可进 Phase 3。
2. **v1 官网部署侧托管**：需确认无静态托管服务指向 v1 `dist/`，方可删 `src/`（D1）。
3. **Marguerite TUI 侧 bridge 契约**：`cross_project_bridge.py` 仅覆盖记忆层；会话/技能契约需在 Marguerite 与 nexustar-ui 两端作者（M1）。
4. **archived cascade 与 nexustar-api 差异**：需 diff `gwwc/agent/adaptive_model_cascade.py` 与 `backend_switcher.py`，确认线上实现无功能缺口（D8 / D10 收尾）。
5. **`creative/meme-generation` 技能兼容**：需校验其 SKILL.md 元数据与 `marketplaceStore.MarketplaceSkill` 类型对齐（D2 收尾）。

---

## 六、方法论与证据来源

- 实证手段：`ls`/`find` 列目录；`Grep`/`grep -rln` 搜引用；`Read` 读 `backend_switcher.py` / `cross_project_bridge.py` / `swarmStore.tsx` / `desktop-migration-plan.md` 头部。
- 不臆造：所有结论来自本轮命令实证；原 K2.2 标「待核实」者本轮已逐条处置（§1.1），无残留「待核实」。
- 与既有交付物：本清单是 K2.4（战略计划 §3.5 ⏳）；结论直接继承 K2.2 §六风险矩阵，并按实证重评严重度。

---

## 七、一句话结论

UnivAgent 三仓整合的**真实高风险只剩 2 项**（D3/C5 视觉引擎分裂、D5 编排画布孤岛）——其余 🔴 项（C2）已随 K1.4 消解，多项中风险因**现有代码已落地**（backend_switcher / cross_project_bridge / swarmStore run 模型 / hermes-skills 撤离）而**实质降级**。最大外部阻塞是 **C6 账号单事实源需用户拍板**。迁移应按「先地基→再视觉→后清理→跨屏整合」的 consolidation-first 时序，并以 §三 门禁逐阶段把关。

*文档结束。K2.4 基于 K2.2 + 本轮 5 项待核实实证复核合成；路径与结论来自 `ls`/`find`/`Grep`/`Read` 实证，未做任何代码修改。*
