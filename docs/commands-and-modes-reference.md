# Marguerite 命令与模式参考（2026-08-22 增补）

> 说明：`docs/cli-reference.md` 由并行会话重写中；本文件为新功能/新模式的权威速查，待其落地后可合并。

## 模式（/mode <name> 或 TUI 工作台切换）
| 模式 | 别名 | 语义 |
|---|---|---|
| plan | — | 只读探索，写操作一律拒绝 |
| agent | — | 写操作请求确认（交互审批） |
| ensemble | — | 受控强编排（原 bloom 升级） |
| bloom | yolo/max/auto/swarm/deep/ptc | 统一编排：锚定/PTC/YOLO/MAX/AUTO/SWARM/DEEP 内化 |
| **omni-transcend** | omni/omnix/ostar | **超星 x·全谱超模**：S1 梁神锚定(2 工具+1024) → S2 Plan 只读探索 → S3 MAX 四阶段深想 → S4 YOLO×PTC 融合执行 → S5 DEEP/SWARM/AUTO 全域；工具面单调扩展，顶栏显示 Omni |

### Omni-Transcend 模式流转（原理）
| 阶段 | 模式组合 | 核心动作 | 预期效果 |
|---|---|---|---|
| S1 定轨 | 梁神锚定 | 2 最小工具 + 单行 persona + 1024 预算，前缀门控晋升 | 定轨防偏、压低首 Token |
| S2 探索 | Plan 段 | PTC 单 run_code 只读摸库、识别依赖 | 零损摸清（无写面） |
| S3 深想 | MAX | thinking_depth 4 + 验证 4 遍 + effort max，生成→批判→改进→验证 | 推理峰值 |
| S4 执行 | YOLO×PTC | 程序化工具调用驱动 + 免确认自动循环 + 快照/4 遍验证/CompletionGate | 高速落地不失控 |
| S5 全域 | 工作台 | DEEP 双模型 + SWARM 13 分片 + AUTO 160k/24 步 + 视觉/LSP/成本 | 复杂任务续航与可控 |

## REPL 斜杠命令（`marguerite` 交互 REPL）
- `/pages`：本地知识页管理（免 Key/离线）—— `list | add <标题> | <标签,逗号> | <内容> | show <id> | search <q> | rm <id>`；数据 `%AppData%/Marguerite/knowledge-pages.json`（原子写 + 语义×关键词混合检索）。
- `/games`：小游戏—— `list` 列出已实装（2048/井字棋/贪吃蛇），`play <id> <动作...>`（如 `/games play tictactoe 0 3 1 4 2`）运行引擎并渲染棋盘+结果。

## 工作台斜杠命令（TUI workbench）
- `/open <path>`：侧边栏聚焦并展开指定路径节点（切文件树 + sidebar_open_path 状态，供树渲染消费）；无参→仅重绘。（BuiltinCommand::Open，`/help` 已列出）
- `/tree`：打开文件树活动栏。
- `/games`（DSH 路径）：与 REPL 同语义的 games 命令面。

## CLI 子命令（`marguerite <sub>`）
- `marguerite games list|play <id> <动作...>`：同小游戏引擎（并行会话已实现并登记 19 路由）。
- `marguerite capabilities` / `doctor`：能力健康度（含 real/partial 分列与纯叙事最小集）。
- 模式切换走 `/mode` 别名（omni-transcend/omni/omnix/ostar）；CLI 暂无独立 mode 子命令（网关提供 mode.switch / mode.get）。

## 白盒评估层（harness::eval）——「质量可见」
- `EvalTask{id,prompt,expected}`（预期输出 = 验收基准）；`eval_suite(tasks, actuals)` 按 id 关联实际输出；
- `levenshtein`（UTF-8 字符级）+ `similarity = 1 - dist/max(len)`（0..1）；
- `summarize` → `{total, avg_score, exact_count, hit_rate(>=0.8 命中), error_rate, min/max}`——量化仪表盘数据；
- 与自愈环衔接：score/hit_rate 喂给 `harness::distill::{diagnose, mutate_prompt, EvolutionGate}`，形成「评估→诊断→改进→再评估」飞轮。

## 与 DSH rc.5+ 对比（超维设计要点）
| 维度 | DSH rc.5+ | Marguerite 现状/拟建 |
|---|---|---|
| 插件回滚 | Cordis 完整撤销 | plugins/di guarded_provide + reload；工具面单调扩展红线 |
| 评估 | 靠人工验收 | **harness::eval 白盒量化**（本轮落地）+ automation/audit 多维 |
| 代理协作 | 子代理委派 | 分层：专家团/双带队/集群/swarm DAG/子代理 trellis + 隔离预算/sidechain |
| 命中率 | 架构 99% 目标 | model/prefetch 去重 + PrefixCache 脏标记/落盘 + 投影折叠 + doctor 可见 |
| 自进化 | 无 | distill run_self_evolve + EvolutionGate + memory SelfEvolution/ExperienceLedger |
| 视觉/工具 | 多模态图文命令 | Modlens/Computer-Use/CdpSession/baota/MCP 桥/LSP(codeintel) |