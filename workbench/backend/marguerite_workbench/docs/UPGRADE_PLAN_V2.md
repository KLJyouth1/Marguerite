# Marguerite Workbench 升级方案 V2 —— 可观测的 LLM 计算 × 智能体驾驭实验台

> 状态: **已批准实施** · 日期: 2026-08-24 · 影响面: `marguerite_workbench/*` + `workbench-web/src/*`
> 决策记录: 本文即 ADR 集合（每节含 背景/备选/决策/后果）。基线: 后端 38 passed / web tsc+vite 干净构建。

## 0. 研究输入（证据链）

| 来源 | 关键事实 | 对本方案的约束 |
|---|---|---|
| DeepSeek 官方 Thinking Mode 文档 | 思考模型的思维链经独立字段 `reasoning_content` 下发、不回传进多轮历史；思考型模型对 temperature/top_p 等采样参数有约束 | TTC 层的采样预设必须按「任务类型」而非全局常量；LLM 客户端需双格式兼容 |
| OpenAI Codex CLI 内部机制系列 + ZenML 架构分析 | turn-based agent loop；rollout 以 message/function_call/output 事件流记录；接近窗口上限触发 compaction | Agent 循环必须事件化落盘（append-only），步数硬上限，工具失败进修复环 |
| DSH rc.8 本机包（60+ Cordis 插件） | compaction-basic 与 compaction-tool-result-pruner 分层压缩、token-meter 计量、session-projection 投影、全插件内核 | 观测层按「事件总线 + 会话日志」最小闭环实现，不做全量复刻（匹配度优先） |
| karpathy/minbpe | 字节级 BPE：256 基础词表 + 迭代合并最高频相邻对 | 分词器实验台按 minbpe 语义实现并附合并轨迹 |
| karpathy/build-nanogpt | GPT-2 复刻前向链路: wte+wpe → N×(LN→causal-MHA→res, LN→GELU-MLP→res) → LN → tied LM head | 模型实验室逐层真实计算并导出注意力矩阵供可视化 |

## 1. 五维决策评分

| 维度 | 权重 | 评分 | 理由 |
|---|---|---|---|
| 匹配度 | 30% | 5/5 | 补齐侧车工作台的三大缺口（LLM 原理/TTC/Agent 循环），与 Rust 核心 harness/orchestration 互补而非重复 |
| 功能完整性 | 25% | 4/5 | 三支柱均含 API+CLI+WebUI+测试；流式 SSE 与分布式执行显式标注 NOT IMPLEMENTED |
| 可维护性 | 20% | 5/5 | 纯函数数学核、无新依赖、离线确定性测试 |
| 可扩展性 | 15% | 5/5 | 工具注册表/采样预设/验证器均为表驱动注册制 |
| 先进性 | 10% | 4/5 | 对齐 2025-2026 主流 harness 设计（thinking 字段、TTC 预算、事件化循环） |

## 2. 三大能力支柱（E-T-C-L-O-V-G 映射）

### 支柱 A — LLMLab：第一性原理实验室（C 层）
- `llmlab/tokenizer.py` — 字节级 BPE（train/encode/decode + 合并轨迹）
- `llmlab/transformer.py` — 纯 Python TinyGPT 前向：embedding → N×(LN→多头因果 SDPA→残差, LN→GELU-MLP→残差) → LN → 权重绑定 LM head → 交叉熵；导出逐头注意力矩阵
- `llmlab/sampler.py` — greedy/temperature/top-k/top-p/frequency+presence penalty，含逐步选择轨迹
- **ADR-A1**: 选纯 Python 而非 numpy——零新增依赖（离线门禁）、教学透明；后果：维度上限 dim≤64/T≤128，性能非目标。
- 接口: `POST /api/llmlab/{tokenize,forward,sample}`；CLI `llmlab *`；WebUI Model Lab 面板（注意力热力图渲染真实数值）

### 支柱 B — TTC：推理时计算一等公民（L 层）
- `ttc.py` — SelfConsistency（N 采样+归一化多数投票）、BestOfN（可插拔验证器）、AdaptiveBudget（难度启发式→采样/轮次预算预设，对齐 thinking-mode 约束）
- **ADR-B1**: 无 key 时回退到本地 KB 检索作为候选生成器（诚实标注 `engine=fallback`），保持仓库不变量「离线可运行」；后果：投票语义在回退模式下退化为检索一致性，文档明示。

### 支柱 C — Agent Loop：Codex 范式状态机（L+O 层）
- `events.py` — EventBus（订阅/环形缓冲）+ SessionLog（append-only JSONL）
- `agentloop.py` — PLAN→ACT→OBSERVE→REPAIR→FINALIZE；工具注册表（read_file/list_dir/kb_search/memory_query，全部过 `safe_resolve`）；`max_rounds`/`max_repairs` 硬上限；失败如实上报
- 接口: `POST /api/agent/run`、`GET /api/events/recent`；CLI `agent run/log`；WebUI Orchestrate 面板（步骤时间线）

### 显式 NOT IMPLEMENTED（零模拟红线下的诚实边界）
SSE 流式推送、分布式子代理扇出、ToT 树搜索持久化、GPU 训练循环 —— 各给出实现路径，不用假数据伪装。

## 3. 质量门禁承诺
L1: 每模块同 PR 附 pytest（形状/概率/因果掩码/BPE 回环/状态机修复环）。L2: 无新依赖引入。L3: CLI/Web 门禁命令照跑。L4: 本文五维+ADR。L5: README 同步 + 回滚 = git revert 单提交序列。
