# Third-Party 复刻完整性审计 + 安全重命名 + 集成报告

> 生成：2026-08-17 | 负责人：首席架构师 高见远 | 基线：`D:\Downloads\Marguerite`
> 阶段：B 期（克隆已于上轮 100% 完成，本轮仅做审计 / 安全重命名 / 报告，未重克隆）

## 0. 结论速览

- **完整性**：third_party 下 27 个 GitHub 克隆仓全部落地且非空（18 harness + 9 computer_use），present = 27/27。
- **安全重命名**：`third_party/harness/` 下 9 个带外部产品名的顶层目录已重命名；内部 package/crate 名与 `.git` 均保持原样，未触碰主程序源码。
- **E:\ 可达性**：`E:\新建文件夹 (3)\_GUI_Agent_archived_20260713` 与 `E:\新建文件夹 (3)\opensquilla` 均可访问，`third_party/local/` 的 Junction 联接源有效，无需跳过。

---

## 1. 完整性审计矩阵

核对依据：`docs/external-integration-optimization-plan.md` §1.1/§1.2（用户列出的外部项目清单）与磁盘实际目录 + `third_party/clone_status_live.json`（done=27）。

### 1.1 Harness 类（18 个，`third_party/harness/`）

| # | 项目名 | 类别 | 期望路径 | 实际路径 | 文件数 | 状态 |
|---|--------|------|----------|----------|--------|------|
| 1 | deepseek-harness | harness | harness/deepseek-harness | harness/harness-core | 7441 | present（已重命名） |
| 2 | CodeWhale | harness | harness/CodeWhale | harness/CodeWhale | 1581 | present |
| 3 | awesome-deepseek-harness | harness | harness/awesome-deepseek-harness | harness/harness-awesome | 39 | present（已重命名） |
| 4 | penguin-harness | harness | harness/penguin-harness | harness/harness-penguin | 1060 | present（已重命名） |
| 5 | awesome-dsh-plugin | harness | harness/awesome-dsh-plugin | harness/harness-dsh-plugin | 68 | present（已重命名） |
| 6 | CowAgent | harness | harness/CowAgent | harness/CowAgent | 889 | present |
| 7 | dsh-agent-teams | harness | harness/dsh-agent-teams | harness/harness-agent-teams | 108 | present（已重命名） |
| 8 | Abu-Cowork | harness | harness/Abu-Cowork | harness/Abu-Cowork | 1867 | present |
| 9 | dsh-desktop | harness | harness/dsh-desktop | harness/harness-desktop | 65 | present（已重命名） |
| 10 | deepseek-harness(HenryZ838978) | harness | harness/deepseek-harness-HenryZ838978 | harness/harness-core-henry | 111 | present（已重命名） |
| 11 | command-code | harness | harness/command-code | harness/command-code | 45 | present |
| 12 | loushang | harness | harness/loushang | harness/loushang | 2439 | present |
| 13 | deepseek-auto-evolving-harness | harness | harness/deepseek-auto-evolving-harness | harness/harness-auto-evolve | 720 | present（已重命名） |
| 14 | superpowers-reasonix | harness | harness/superpowers-reasonix | harness/superpowers-reasonix | 134 | present |
| 15 | iPolloWork | harness | harness/iPolloWork | harness/iPolloWork | 8547 | present |
| 16 | awesome-deepseek-harness(Dominic789654) | harness | harness/awesome-deepseek-harness-Dominic789654 | harness/harness-awesome-dominic | 69 | present（已重命名） |
| 17 | LeAgent | harness | harness/LeAgent | harness/LeAgent | 2305 | present |
| 18 | Guided-Attention-Inference-Network | harness | harness/Guided-Attention-Inference-Network | harness/Guided-Attention-Inference-Network | 53 | present |

### 1.2 Computer Use 类（9 个，`third_party/computer_use/`）

| # | 项目名 | 类别 | 期望路径 | 实际路径 | 文件数 | 状态 |
|---|--------|------|----------|----------|--------|------|
| 1 | Ros_Qt5_Gui_App | computer_use | computer_use/Ros_Qt5_Gui_App | computer_use/Ros_Qt5_Gui_App | 439 | present |
| 2 | computer_use_ootb | computer_use | computer_use/computer_use_ootb | computer_use/computer_use_ootb | 76 | present |
| 3 | robotgo | computer_use | computer_use/robotgo | computer_use/robotgo | 199 | present |
| 4 | computer-vision-guide | computer_use | computer_use/computer-vision-guide | computer_use/computer-vision-guide | 249 | present |
| 5 | Digital-Forensics-Guide | computer_use | computer_use/Digital-Forensics-Guide | computer_use/Digital-Forensics-Guide | 33 | present |
| 6 | awesome-computer-use | computer_use | computer_use/awesome-computer-use | computer_use/awesome-computer-use | 30 | present |
| 7 | computer-agent | computer_use | computer_use/computer-agent | computer_use/computer-agent | 79 | present |
| 8 | ScreenSpot-Pro-GUI-Grounding | computer_use | computer_use/ScreenSpot-Pro-GUI-Grounding | computer_use/ScreenSpot-Pro-GUI-Grounding | 140 | present |
| 9 | OpenCUA | computer_use | computer_use/OpenCUA | computer_use/OpenCUA | 182 | present |

> 注：磁盘上 `computer_use/` 实际有 10 个目录，多出 `Guided-Attention-Inference-Network`（与 harness/ 下为同一仓库的并列副本，plan §1.2 未单列）。该副本未纳入审计计数，属冗余，建议后续去重。

### 1.3 计数 reconciliation

- 计划 §1.1 明确枚举 **18** 个 harness 仓库；团队指令中提及的“harness 16 个”应为按核心口径的近似（若剔除 borderline 的 `Guided-Attention-Inference-Network` 与 `superpowers-reasonix`，恰为 16）。本报告以计划权威清单为准，审计 **18 + 9 = 27**。
- **结果：present 27/27，absent 0。**

---

## 2. 安全顶层重命名映射表

范围：仅 `third_party/harness/` 下最显眼的顶层目录；内部 package/crate 名一律不动（避免破坏构建）。

| 原顶层目录 | 新顶层目录 | 内部 package 名（未改） | 含 .git | 操作 |
|------------|------------|--------------------------|---------|------|
| deepseek-harness | harness-core | @deepseek-ai/dsh-root | 是 | mv |
| deepseek-harness-HenryZ838978 | harness-core-henry | （MCP 包分支） | 是 | mv |
| deepseek-auto-evolving-harness | harness-auto-evolve | （Python） | 是 | mv |
| awesome-deepseek-harness | harness-awesome | （Markdown 索引） | 是 | mv |
| awesome-deepseek-harness-Dominic789654 | harness-awesome-dominic | （Markdown 索引） | 是 | mv |
| penguin-harness | harness-penguin | penguin-harness | 是 | mv |
| awesome-dsh-plugin | harness-dsh-plugin | （Markdown 目录站） | 是 | mv |
| dsh-agent-teams | harness-agent-teams | （TypeScript） | 是 | mv |
| dsh-desktop | harness-desktop | （TypeScript） | 是 | mv |

执行说明：
- `harness/` 父目录本身不是 git 仓库（`git rev-parse` 报 not a git repository），故采用文件系统 `mv` 整体移动整仓目录（含各自 `.git`），而非 `git mv`（在父级执行会失败且无法保留子仓历史）。每个子仓均为独立克隆，移动后 `.git` 完整保留、仍可独立 `git log/checkout`。
- 已抽样校验：`harness-core/package.json` 的 `"name"` 仍为 `@deepseek-ai/dsh-root`，`harness-penguin/package.json` 仍为 `penguin-harness` —— 内部命名零改动。
- 未重命名的 9 个 harness 目录（Abu-Cowork / CodeWhale / CowAgent / Guided-Attention-Inference-Network / LeAgent / command-code / iPolloWork / loushang / superpowers-reasonix）保持原名。
- 全程未读改主程序源码（仅对 third_party 做组织），符合“主程序源码只可读”约束。

---

## 3. 已知遗留 / 待办（非本轮范围）

1. **清单文件键名未同步**：`third_party/manifest.json` 与 `third_party/clone_status_live.json` 仍用旧 key（如 `harness/deepseek-harness`）。本轮刻意不改这两份克隆流水线状态文件，以免与“禁止重克隆”的流水线冲突。建议交由流水线 owner 在下一轮同步为新名。
2. **`computer_use/` 冗余副本**：`Guided-Attention-Inference-Network` 在两处重复，建议去重（保留一处，另一处改 Junction/Symlink）。
3. **core/ 引擎源码不在本工作副本**：本基线仅含 `third_party/` 与文档，未见 `core/`（gateway/chat.rs、model/bloom.rs、tools/computer_use.rs）。接入建议基于计划文档结构给出，落地时需回到含 core/ 的源码副本操作。
4. **路径引用一致性**：若主程序或 workspace 配置中曾以旧顶层目录名引用上述 harness，需在 core/ 副本中同步更新（本轮不可改主源码，列为 P 后续）。

---

## 4. 接入 core 引擎的下一步建议（简要）

> 下列为规划级建议，依据 `docs/external-integration-optimization-plan.md` §3；具体落地应在含 `core/` 的源码副本中进行。

### 4.1 网关层（gateway / chat.rs 方向）
- 参考 `harness-core`（Cordis 插件模型）与 `harness-agent-teams`（队长→成员直传），在 `core/src/gateway/` 扩展 Agent 团队协作通道：新增 `Mailbox` 消息队列，支持成员间直接收发（不依赖队长中转）。
- 复用 `harness-auto-evolve` 的 6 轮自进化方法论，在 `distill.rs` 的 `SelfEvolveGate` 上补充：工具调用格式修正 → system prompt 调整 → 推理参数调整 → 执行后反思，进化结果落盘 `harness/evolutions/` 支持回滚。

### 4.2 模型层（model / bloom.rs 方向）
- 引入 `CodeWhale` Fleet 多角色路由：在 `model/` 新增 `fleet.rs`（architect/builder/reviewer 各自绑定 provider+model+reasoning）、`failover.rs`（故障转移链）、`credential_pool.rs`（多 key 轮转，参考 `hermes-agent`）。
- `bloom.rs`（推理/涌现模式）接入 thinking/non-thinking 切换与成本路由（按任务复杂度选价位模型），参考 `harness-auto-evolve` 的 thinking 模型差异结论。

### 4.3 Computer Use 工具（tools/computer_use.rs 方向）
- 短期（参考 `robotgo` Go 实现，Rust 侧用 win32 API）：扩展窗口枚举/聚焦、右键/中键、剪贴板读写、全局热键监听 → 新增 `computer_use.listen` 工具。
- 中期（参考 `OpenCUA` / `ScreenSpot-Pro` / `GAIN`）：在 `vision/` 新增 `grounding.rs`，实现“截图→VLMM→坐标→置信度→执行”闭环；GAIN 注意力图作辅助监督。
- 长期：OpenCUA-7B 作为可选本地 CUA（vLLM 推理，torch/vllm 列入可选依赖组，不强制安装），ScreenSpot-Pro 作评测基准。

### 4.4 工程纪律技能（superpowers-reasonix）
- 移植 11 项技能为 `skills/builtin/superpowers-*/SKILL.md`，由现有 SkillLoader 加载（brainstorming / writing-plans / TDD / systematic-debugging / verification-before-completion 等）。

---

## 5. 约束遵循确认

- 未重克隆（克隆上轮已完成，done=27）。
- 未修改主程序源码（仅组织 `third_party/`，且为重命名而非改内容）。
- 未改动目录内部 package/crate 名。
- 无 emoji。
- 未 spawn 子 agent。
- 未将审计失败的外部源当作错误（E:\ 可达，local/ Junction 全部有效）。
