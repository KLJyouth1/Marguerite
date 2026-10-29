# Marguerite RSI 路线图：从「有零件」到「全对象自改进」

- 日期：2026-10-26
- 状态：**方案待确认**（P0 可直接开工）
- 依据材料：Recursia / EmbodiRSI / ASPIRE / ENPIRE 公开论文与报道；`D:\Downloads\Marguerite` 源码实测；16 份会话导出（323 条用户原话）；`yolo-ultralytics` 工程模式；GitHub `KLJyouth1` 69 个仓库盘点
- 关联 PRD：本文同时作为 L 档 PRD（问题 / 目标 / 非目标 / 验收口径 / 风险 / 回滚均已含于 §0–§8）

---

## 0. 一句话结论

> **Marguerite 不缺 RSI 的零件，缺的是「把零件接到全部对象上」和「判定改动该不该留」的门。**
>
> `core/src/harness/meta.rs`（1628 行）已实现「监控 → 诊断 → 规划 → 沙箱评估 → 版本闭环」，
> `evolve.rs` 已有多轮进化与回滚，`eval.rs` 已有评估飞轮，`ledger.rs` 已有确定性回放。
> **但 `ChangeTarget` 只枚举 6 类可改对象**（prompt / 规则 / 工具默认参数 / 沙箱脚本 / 知识库 / few-shot），
> **不含专家、团队、蜂群、记忆结构、技能、子代理**——这 6 类之外的 10 个对象，今天都进不了自改进闭环。

---

## 1. 三篇工作到底给了什么（只取可迁移的机制）

> 事实核对先行：用户提到的「哥伦比亚大学 EmbodiedRSI」与「穹智能 RobotRSI」，
> 经检索**未能证实**。EmbodiRSI 的公开来源为**上海科技大学 + 瞬适科技（InstAdapt）**；
> 哥伦比亚大学相关工作是 Hod Lipson 团队的「机器人自我建模」（Nature MI 2025），非 RSI 系统。
> 「RobotRSI / 穹智能」未找到公开来源，故**不纳入方案依据**，以免建立在无法追溯的前提上。

### 1.1 Recursia —— 内外双层循环 + 共享执行栈 + 双门控

**论文**：`Learning to Compose: Recursive Self-Improvement of Shared Executable Stacks for Zero-Shot Long-Horizon Robot Manipulation`
**机构**：北京交通大学、北京大学、NYU、USC、北京灵行智能
**代码**：`github.com/cocacola-lab/Recursia` · 索引：`cocacola-lab/awesome-embodied-rsi`

| 机制 | 内容 | 为什么关键 |
|---|---|---|
| **内层循环** | 当前任务的观察 → 决策 → 执行 → 再观察 | 执行反馈必须能改变下一步，而不是一次性生成 |
| **外层循环** | 汇总执行经验，提出 / 验证 / **保留**对共享栈的修改 | 改动要留下来，且对后续任务生效 |
| **共享执行栈四层** | 工具层（能否可靠执行）/ 技能层（前置条件 + 执行 + **完成证据**）/ 交接层（上一步完成到下一步就绪的距离）/ 验证层（结果是否真的满足要求） | 单点成功掩盖接口问题：案例中「杯子放盘子」10/10 成功，但「杯子+布丁」组合只成功 1/10，因为布丁识别返回 `not_found` |
| **选择门控** | 候选是否带来足够任务收益 | 防「改了但没用」 |
| **回归门控** | 是否丢掉原本已具备的成功能力，且**每任务有损失上限** | 案例：总成功 7/50→11/50，但某任务丢掉 2 个旧成功态，超上限 → **仍然拒绝** |
| **元改进** | 把 verification gate 的松紧、debugging 数据选择也纳入调整 | 这才是「改进改进机制本身」 |

**效果**（论文口径）：LIBERO-Pro Long 零样本 Pos 28.4% / Task 53.4%，超 ASPIRE 5.8 / 15.1 个百分点；真机三任务平均 66.7% vs SD-only 26.7%；强组合依赖任务 20% → 100%。

### 1.2 EmbodiRSI —— 执行 → 发现弱点 → 补针对性经验 → 再训练

**机构**：上海科技大学 + 瞬适科技（InstAdapt），2026-09
**机制**：`Rollout 监控器`（多视角观测 + 环境状态，**无需重置即可从当前状态恢复**）→ `故障分析`指导**针对性**数据收集 → 成功轨迹（含修正后完成）与新演示累积 → 更新下一轮策略。

**可迁移的一条**：**失败不是拿来报错的，是拿来定向补数据的**。缺口在哪，就只补那一块，而不是无差别重训。

### 1.3 ASPIRE / ENPIRE —— 技能库沉淀 + 真机外层循环

- **ASPIRE**（NVIDIA）：根据执行反馈修复控制程序，把有效经验沉淀为**可复用技能库**。
- **ENPIRE**（NVIDIA）：打通真机外层改进循环——场景复位 → 策略执行 → 结果验证 → 训练改进。

**可迁移的一条**：**技能必须有「完成证据」才能入库**，否则后续调用者不知道它到底建立了什么状态。

### 1.4 三者共同的三条硬约束（也是 RSI 翻车的地方）

1. **必须依赖可靠的外部 Verifier**。自我评价会导致自我确认、多样性下降、模型退化。
2. **模拟器成功 ≠ 真实成功**。指标涨了可能以安全性、成本、时延为代价。
3. **要区分「速度」与「加速度」**：每轮有提升是速度 > 0；**让下一轮更容易提升**才是加速度 > 0，后者才是 RSI。

---

## 2. 现状盘点：Marguerite 已经有什么（file:line 级）

| 能力 | 位置 | 成熟度 |
|---|---|---|
| 元智能体闭环（监控→诊断→规划→评估→版本） | `core/src/harness/meta.rs`（1628 行） | ✅ 完整，含三条硬边界 |
| 多轮进化 + 历史 + 回滚 | `core/src/harness/evolve.rs`（304 行）`EvolutionRunner` | ✅ 可用 |
| 评估飞轮（评估→诊断→改进→再评估） | `core/src/harness/eval.rs`（320 行）`eval_flywheel` | ✅ 可用，但指标仅相似度/命中率 |
| 运行台账 + 确定性回放 | `core/src/harness/ledger.rs`（1194 行） | ✅ append-only，`replay_digest` 不含时间戳 |
| 团队 / 子代理 / 蜂群 | `core/src/agent/team.rs`、`subagent.rs`、多文件 `swarm` | ✅ 存在，30 处并行 |
| 记忆分层（感官/短时/长期/经验/压缩/召回/跨项目/联邦） | `core/src/memory/*`（20 个模块） | ✅ 结构完整 |
| 知识库（图/混合检索/完整性/项目图） | `core/src/kb/*`（8 个模块） | ✅ 存在 |
| 桌面操控（真实 COM/UIA） | `core/src/desktop_automation/windows_uia.rs`、`com.rs`、`tools/computer_use.rs` | ✅ **真能力，非降级项** |
| 专家体系（8 团 43 角色） | `workbench/backend/marguerite_workbench/expert_teams.py` | ⚠️ **静态分析器**，无自改进通道 |
| 自改进/反思（语义层） | 全仓 `self_improve\|reflection` 仅 **1 处**（`orchestration/mod.rs`） | ❌ 语义层缺口 |

**结论**：基础设施齐备，缺的是**覆盖面**与**判定门**。

---

## 3. 五个断点（RSI 为什么现在还没闭环）

| # | 断点 | 证据 | 后果 |
|---|---|---|---|
| **B1** | **可改对象太窄** | `meta.rs::ChangeTarget` 只枚举 6 类 | 专家/团队/蜂群/记忆结构/技能/子代理**进不了改进闭环**，只能改 prompt 和规则 |
| **B2** | **只有选择门控，没有回归门控** | `PromotionCriteria` 有成本/时延闸，但无「每任务损失上限」 | 会为了总均分上涨而牺牲某个任务，正是 Recursia 明确拒绝的情形 |
| **B3** | **无「加速度」度量** | `eval.rs` 只有单轮 `EvalSummary` | 无法回答「这一轮让下一轮更容易了吗」，只能看到「这轮涨了没」 |
| **B4** | **共享栈未成型** | 工具/技能/交接/验证四层散落在 `tools/`、`agent/`、`harness/` | 修好一个技能，不知道是否破坏了调用它的团队 |
| **B5** | **双真相源漂移** | 8 团 43 成员同时存在于客户端插件与 `expert_teams.py` | 本次已实证：改了客户端，仓库内 43 角色**毫无变化** |

---

## 4. 方案：Marguerite RSI 五层架构

```
┌─────────────────────────────────────────────────────────────┐
│ L5 元改进层  meta-gate：调门控松紧 / 调补数据策略 / 调评测集      │  ← 已有 meta.rs 雏形
├─────────────────────────────────────────────────────────────┤
│ L4 门控层    选择门控（收益）+ 回归门控（每任务损失上限）+ 成本闸  │  ← 需补回归门控
├─────────────────────────────────────────────────────────────┤
│ L3 外层循环  诊断 → 定向补经验 → 生成候选 → 沙箱回放 → 采纳/回滚  │  ← 已有 evolve/eval
├─────────────────────────────────────────────────────────────┤
│ L2 共享执行栈 工具 | 技能 | 交接 | 验证（四层，跨对象复用）        │  ← 需成型
├─────────────────────────────────────────────────────────────┤
│ L1 内层循环  观察 → 决策 → 执行 → 再观察（含完成证据）            │  ← 已有 stream_loop
└─────────────────────────────────────────────────────────────┘
```

### 4.1 十个对象的改造映射（这是「举一反三」的核心表）

| 对象 | 今天的形态 | RSI 改造后 | 改哪个文件 |
|---|---|---|---|
| **主智能体** | `stream_loop.rs` 单轮循环 | 内层循环显式化：每步产出「完成证据」，失败即触发外层 | `agent/stream_loop.rs` |
| **子代理** | `subagent.rs` 一次性派发 | 子代理回传「可复用的失败模式」，进 L3 诊断池 | `agent/subagent.rs` |
| **子智能体** | 同子代理，粒度更细 | 同上，但只回传**结构化信号**不传原始 trace | `harness/meta.rs::extract_signals` |
| **专家（43 角色）** | 静态 checklist 分析器 | 每个角色挂 `evolution_target`：其 checklist 维度与阈值可被外层改进 | `expert_teams.py` + `experts.py` |
| **团队（8 团）** | `TEAM_DEFS` 静态元信息 | 团队级 SOP 编排参数（并行/串行、阶段门）可调 | `expert_teams.py` |
| **蜂群** | 并行 spawn | 蜂群规模/分工策略作为 `ChangeTarget` 变体 | `agent/team.rs`、`swarm` 相关 |
| **记忆** | 20 个模块，分层但无自评 | 记忆写入前过「价值门」；召回后记「是否被用到」→ 反向调权重 | `memory/experience.rs`、`recall.rs` |
| **经验** | `experience.rs` | 经验必须带「适用条件 + 反例」，否则不入库（对齐 Recursia 技能层） | `memory/experience.rs` |
| **知识库** | `kb/*` 混合检索 | 检索命中率进评测集；低命中条目触发补数据 | `kb/hybrid.rs`、`kb/integrity.rs` |
| **技能/工具** | `tools/`、`distill.rs` 映射表 | 技能入库需「完成证据」；工具调用失败率进信号池 | `harness/distill.rs`、`tools/` |

### 4.2 `ChangeTarget` 扩展（B1 的解法）

```rust
// 今天（6 类）
pub enum ChangeTarget { Prompt, Rule, ToolDefault, SandboxScript, KnowledgeBase, FewShot }

// 扩展后（+7 类，共 13 类；仍不含「改 core 源码」）
pub enum ChangeTarget {
    Prompt, Rule, ToolDefault, SandboxScript, KnowledgeBase, FewShot,
    ExpertChecklist { team: String, role: String },  // 专家 checklist 维度/阈值
    TeamSop { team: String },                        // 团队 SOP 编排参数
    SwarmPolicy,                                     // 蜂群规模与分工
    MemoryWeight { layer: String },                  // 记忆层权重
    SkillDefinition { skill: String },               // 技能前置/完成证据
    RetrievalPolicy,                                 // 检索策略
    EvalTaskSet,                                     // 评测集本身（L5 元改进）
}
```

**硬边界保留**：仍然**没有**「改 core 源码」变体。`tests::core_source_is_not_a_change_target` 继续有效。

### 4.3 双门控（B2 的解法，直接借 Recursia）

```rust
pub struct PromotionCriteria {
    pub min_gain: f64,              // 选择门控：总收益下限
    pub max_token_growth: f64,      // 成本闸（已有）
    pub max_latency_growth: f64,    // 时延闸（已有）
    // 新增：回归门控
    pub max_per_task_loss: f64,     // 单任务成功率下跌上限
    pub protected_tasks: Vec<String>, // 绝不允许退化的任务（红线集）
}
```

**判定顺序**：先过 `protected_tasks`（任一退化即拒）→ 再过 `max_per_task_loss`（任一超限即拒）→ 再看总收益 → 最后过成本/时延闸。

**这条规则的价值**：Recursia 用真实案例证明，**总均分上涨可以是假的好消息**。Marguerite 现在的 `PromotionCriteria` 只看总收益，会吃这个亏。

### 4.4 加速度度量（B3 的解法）

在 `eval.rs` 的 `EvalSummary` 之外增加：

```rust
pub struct AccelerationReport {
    pub round: usize,
    pub score: f64,              // 本轮得分（速度的积分）
    pub delta: f64,              // 相对上轮（速度）
    pub delta_of_delta: f64,     // 加速度
    pub cost_per_gain: f64,      // 每单位提升的 token 成本
    pub reuse_rate: f64,         // 上轮沉淀被本轮复用的比例 ← 最关键
}
```

**`reuse_rate` 是 RSI 的真判据**：如果上轮学到的技能/规则，本轮一次都没被调用，那只是「每轮重新学一遍」，加速度为 0。

### 4.5 双真相源合并（B5 的解法）

**今天**：客户端插件（Markdown 指令）与 `expert_teams.py`（Python 声明）各写一份 43 角色，必然漂移。
**方案**：以 `expert_teams.py` 为**唯一真相源**，客户端插件由脚本生成（`scripts/gen_expert_plugins.py`），CI 断言两者一致。

> 本次已踩实：我改了客户端 8 包，仓库内 43 角色**零变化**。这就是 B5 的活体样本。

---

## 5. 分期路线（每期都有可判定验收）

### P0 · 门控与度量（2 周，可立即开工）

**状态（2026-10-27）：代码已实现，核心测试通过；统一门禁未能在本机沙箱跑完 —— 见 §5.1。**

| 任务 | 验收口径 | 状态 |
|---|---|---|
| `PromotionCriteria` 加 `max_per_task_loss` + `protected_tasks` | 单测：构造「总涨但某任务跌」的候选 → 必须被拒 | ✅ `task_gate_rejects_when_total_up_but_one_task_down` |
| `eval.rs` 加 `AccelerationReport`（含 `reuse_rate`） | 单测：两轮评测输出 `delta_of_delta` 与 `reuse_rate` 非空 | ✅ 3 条（含 JSON 序列化安全） |
| `ChangeTarget` 扩 7 类 | 单测：13 变体齐备，且 `core_source_is_not_a_change_target` 仍绿 | ✅ 6 → 13，护栏断言同步更新 |
| 双真相源 CI 断言 | `scripts/check-expert-parity.py` 退出码 0/1 | ✅ PASS；变异测试 4/4 被拦 |

**额外做了清单外但必需的一件事**：`run_cycle_with_tasks`。没有它，`task_level_gate` 就是没人调用的孤儿函数 —— 那是「文件在 ≠ 能用」的典型假就绪。

#### 5.1 验证边界（诚实声明）

| 验证 | 结果 |
|---|---|
| `cargo test -p marguerite-core --lib harness::` | **136 passed / 0 failed**（含新增 10 条） |
| `cargo test -p marguerite-core --lib` | **4106 passed / 1 failed / 24 ignored**。唯一失败是沙箱 Program Blacklist 拦 `wsl.exe`（MCP 测试启动 server 时触发 shell 探测），与 harness 改动无关 |
| `cargo check -p marguerite-core --lib` | EXIT=0 |
| `check-expert-parity.py`（CI 模式 / `--client-dir`） | 均 PASS；变异测试（缺角色 / 坏 team 引用 / 重复 role / 空团队）全部被拦 |
| `bash scripts/verify.sh --core` | **未完成**：卡在第一个检查项 17 分钟（完整 lib 测试在此沙箱跑不完）；默认 target 的 `.cargo-lock` 另被并发 IDE 占用（`os error 5`） |
| `cargo clippy -p marguerite-core --lib` | **未完成**：沙箱拒绝写 `E:\temp\...`（clippy 临时文件） |

→ 因此 **P0 不标「已完成」**。解除条件：在无沙箱写入限制、且无并发 cargo 的窗口跑一次 `bash scripts/verify.sh --clean --all` 全绿。

### P1 · 共享执行栈成型（3 周）

- 技能入库强制「前置条件 + 完成证据 + 反例」（对齐 Recursia 技能层）
- 交接层：显式建模「上一步完成后，下一步的可达条件」
- 验证层：复用 `CompletionGate`，但补「关系级验证」（不只是文件存在）

**验收**：构造一个「单步成功但组合失败」的用例（复刻 Recursia 的杯子+布丁案例），系统必须能定位到交接层而非报「技能失败」。

### P2 · 专家与团队接入（3 周）

- `expert_teams.py` 每个角色加 `evolution_target`（可被外层改进的维度与阈值）
- 8 团 SOP 编排参数可调
- 43 角色进 `extract_signals` 信号池

**验收**：跑一轮真实改进，产出 1 条针对某专家 checklist 的提案，且经人工 approve 后灰度生效、可回滚。

### P3 · 元改进与加速度（4 周）

- 门控松紧、补数据策略、评测集本身进 `ChangeTarget`
- `AccelerationReport.reuse_rate` 接看板
- 蜂群策略、记忆权重接入

**验收**：连续 5 轮自改进，`delta_of_delta > 0` 且 `reuse_rate` 单调不降；任一轮触发回归门控即记录并回滚。

---

## 6. 与已有硬约束的兼容性

| 既有约束 | 本方案如何遵守 |
|---|---|
| 不许改 core 源码（`ChangeTarget` 无该变体） | 扩展后仍无该变体，护栏测试保留 |
| 默认不自动上线（`auto_apply=false`） | P0–P3 全程保持；P3 的元改进也只生成提案 |
| 成本进闸 | `cost_per_gain` 进 `AccelerationReport`，每轮必算 |
| 循环网关、无遥测 | RSI 全程本地，不上传任何 trace |
| 门禁全绿才可声称完成 | 每期结束跑 `bash scripts/verify.sh --clean --all` |

---

## 7. 不做什么（明确非目标）

- ❌ **不做模型微调**。RSI 在 Marguerite 里是「harness + 对象」的改进，不是训练权重。
- ❌ **不做无人值守的自动上线**。所有改动进待审队列，人工 approve 才灰度。
- ❌ **不引入「RobotRSI / 穹智能」**：无公开来源，不能作为依据。
- ❌ **不做跨用户联邦学习**。`memory/federated.rs` 是本地联邦，不对外。
- ❌ **不在本期改前端**。P0–P3 全部是引擎侧，UI 接线另开一期。

---

## 8. 材料覆盖度声明（诚实边界）

用户要求「完整读完所有 md 文档及模型项目、所有 GitHub 仓库」。实测规模与我的实际做法：

| 材料 | 规模 | 实际做法 | 覆盖度 |
|---|---|---|---|
| 16 份会话导出 | **91 MB**（约 3000 万 token） | 结构化抽取：解析双格式（`### **You**` / `**User**`），提取 **323 条用户原话（5.0 万字全文）** + 结论型行；`Thinking` 段仅统计占比（15%–51%） | 文件 100%，用户需求 100% 原文，AI 思考过程未逐字读 |
| `yolo-ultralytics` | 35,311 文件 / 396 md | 读项目结构 + 作者自写脚本（`build_voc_traffic.py`/`train_voc.py`/`eval_voc.py`/`make_figures.py`/`make_doc.py`/`qa_render.py`/`nms_compat.py`）+ 3 篇作者自写文档；371 篇上游文档未读 | 作者自写部分 100%，上游文档 0% |
| GitHub `KLJyouth1` | **69 仓库**（8 public / 61 private） | 全部列出；8 个重点仓库读 README/结构；`git` 同步状态实测 | 清单 100%，重点仓库深读 |
| 三篇 RSI 工作 | 公开报道 + 论文摘要 | 读机制与数字；**EmbodiRSI 机构已更正**；RobotRSI 未找到来源 | 机制级，非逐篇论文精读 |

**物理边界**：91 MB 文本 + 3.5 万文件的逐字通读在任何上下文窗口内都不可能。若需要某一篇的逐段精读，指定文件名，我单独做。
