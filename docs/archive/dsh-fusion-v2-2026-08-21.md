# DeepSeek Harness × Marguerite 融合总结 v2（2026-08-21）

> 依据：DSH 0.1.0-rc.8 checkout（node_modules/@deepseek-ai/dsh，66 个官方包）、本机安装面（~/.dsh、profiles/web：modlens / genui / openviking / hindsight / agent-teams / router-standard / usage-stats / dshmarket / dsh-web-ui-all 0.2.5）、梁神锚定插件 @linxin666/dsh-liangshen 0.2.5 预置文件、Marguerite 既有 DSH v1 集成（8 份 PluginManifest + 63 单测 + 3 原生模块）。

## 1. DSH 最新版架构要点（rc.8）

- **Profile = 有序补丁层栈**：`dsh.profile.bundles` 依序叠加 cordis 补丁 → profile `cordis.patch.yml` → 家目录 `cordis.patch.yml` → `--patch`。启动器只解析自己的参数（dsh-cmdline 不可变交接）。
- **Agent preset 四档**（config/agent-presets/）：standard（全量）/ code（PTC：单 run_code 呈现全工具）/ minimal（双工具 + 一行 persona + 无运行时上下文 + 无压缩）/ cordis（创造模式，运行时自检）。每档 = `preset.yml` + `agent.cordis.yml`（AGENT-PLANE 组合，一次挂载、会话按 scope 加入）。
- **关键机制**：token-meter（单一启发式 4 字符/token + 结构开销，replay-aware，O(surface) 快照）→ compaction-basic（按路由容量定预算 + LLM 摘要复用前缀 KV cache）+ tool-result-pruner（8192/4096/1024 头中尾修剪，保留原始事件日志可回放）；goal-round-driver（race-fenced 预留 roundsStarted+1，stale 不耗轮）；session-projection（纯计算投影单元注册表 + asOfSeq 一致性快照）；plan-mode（log-only 状态，resume/fork/compaction 可恢复；目录跨模式不变保证请求缓存稳定）；schedule（事件日志持有状态，定时器是可弃投影，≥5 分钟间隔）；skill registry（全局层+预设层就近覆盖）；workflow（worker-thread 执行模型脚本，pipeline 无阶段屏障）+ ralph（每轮全新子代理，工作区共享为记忆，只传有界结构化报告）；subagent providers（spawn/fork/codex/claude-code，后两者默认禁用）；persona（组合式部署段落，`complete` 阻断后续 prompt 追加，`includeRuntimeContext` 控制运行时上下文，`{{model}}`/`{{cwd}}` 模板解析）。
- **梁神锚定（liangshen 0.2.5 预置，两阶段锚定标准预设）**：phase-1 = 官方 Minimal 精确双工具（持久 bash + str_replace_editor）+ 一行 persona（appended cwd），无运行时上下文、只放行 user/goal 消息源；anchorGate 首块门控（minimal-like 推理块 + 4 步兜底）、promoteAfterFirstResponse（无工具首轮后晋升）、bootstrapMaxTokens=1024（首请求输出预算，晋升后剥离）、deferredSources=[agent-instructions, skill-catalog] + deferredGraceSteps=1（晋升后延迟一步注入）、promotedPresentation=code（PTC）；压缩后回退到「受控阶段」（双工具 + compactionTools 核心集）直到越过边界的新晋升信号；turn/end 释放门控会话。

## 2. 融合映射表（DSH → Marguerite）

| DSH（rc.8 / web-ui-all 0.2.5） | Marguerite 对应 | 状态 |
| --- | --- | --- |
| dsh-base / cordis patch layers | plugins/di.rs ServiceContainer + PluginManifest 桥接 | 已融合（红线 #1/#2 守卫） |
| agent presets minimal/standard/code/cordis | mode/anchoring.rs AnchorStage{Anchored→Ptc→Full} + modes/*.yaml | 部分：三阶段已对齐 minimal→code(PTC)→standard(Full)；cordis 创造模式与 preset 列表缺口 |
| liangshen 锚定参数族 | anchoring.rs 前缀门控 + 4 步兜底 + 无工具首轮晋升 | 部分：缺 bootstrapMaxTokens、messageSources 白名单、deferredGraceSteps、压缩后回退重晋升 |
| persona 组合（complete/includeRuntimeContext/{{model}}） | agent/catalog persona + stable_prefix/experience_hint 注入 | 部分：缺 complete 阻断与模板解析 |
| compaction-basic + tool-result-pruner | context/session.rs 四层渐进压缩 + snip_long 折叠 | 已融合（阈值策略不同：本仓 85%/80% 预算比例 + 头 400/尾 200 字符；DSH 为 8192/4096/1024） |
| token-meter | context/budget.rs + model/llm.rs token_estimate（CJK/ASCII 加权） | 已融合且更精细（DSH 固定 4 字符/token） |
| goal-round-driver | goal/mod.rs + 自动化延续轮 | 部分：缺 race-fenced 预留与 stale 判定 |
| plan-mode + exit_plan_mode | mode/plan.yaml 只读模式 + plan/ 计划器 + PlanGate | 已融合（工具目录稳定等价「目录跨模式不变」；DSH 规则段全文见 §4 待注入） |
| schedule after/at/every | automation/cron.rs（5 段 cron）+ task-board ledger-v2 | 部分：缺会话级 after/at 语义（现为全局 cron） |
| session-projection | events/bus.rs 有界订阅 + gateway context_status 台账 | 部分：缺投影单元注册表与 asOfSeq 快照语义 |
| skill registry + skill-filesystem + tool-skill | skills/engine.rs + loader + 5 JIT 技能 + /skills | 已融合（缺分层就近覆盖语义） |
| workflow worker-thread + ralph | orchestration 扇出 + subagent trellis 事件驱动 + agent/benchmarks | 部分：无模型编写脚本的 workflow 引擎；ralph 语义可由 fresh subagent 循环对译 |
| subagent providers spawn/fork | agent/subagent.rs + harness/subagent.rs | 已融合（codex/claude-code provider 缺口，登记为可选） |
| mcp-client | mcp/registry + client/server | 已融合 |
| tool-ask-user / tool-todo / tool-web | 同款工具 | 已融合 |
| tool-cordis（自指运行时检查） | plugins/di.rs guarded_provide 自检 | 部分：无模型面自检工具 |
| agent-instructions（AGENTS.md 加载） | 宿主注入 + governance 护栏 | 已融合 |
| time-context / tmux-context | — | 缺口（低优先级） |
| modlens / genui / usage / pet / market / sidebar / router（web-ui 全家桶 v1 已二创 10 插件） | vision/modlens + genui/* + usage/query_service + workbench/pet_v2 + market 技能 + tui slots + router 3 模式 | 已融合（v1 交付，63 单测） |
| dsh-ssh | deploy/ssh.rs | 已融合（能力等价；无 Web 终端隧道 UI） |
| task-board | automation/host_board + cron + ledger-v2 | 已融合（Host 权威账本同构） |
| desktop-launcher | desktop_automation/ | 已融合 |
| chat-recovery（fork 编辑最后一条消息） | session/rollback.rs | 已融合 |
| skill-explorer | skills/engine + /skills | 已融合 |
| skin-center / skins / 主题 | webui 水墨 skin token（零 hex）+ 蓝橙语义令牌 + gui-visual-audit | 已融合（缺 wallpaper 素材资产包） |
| plugin-manager / community-plugins | plugins/manifest + market 技能 | 部分 |
| remote-web-ui / aionui-panel | gateway serve 回环 + genui | 已融合 |

## 3. 本轮落地（数据层 8+1 + Rust 锚定/模式逻辑层）

新增 `plugins/manifests/` 8 份 PluginManifest（JSON 已进程内解析校验；hooks 用 HookSpec{event,handler,params} 形状，与 Rust 解析器一致）：

| 文件 | 对译 DSH 包 | 桥接目标 |
| --- | --- | --- |
| dsh-presets.plugin.json | agent presets + dsh-liangshen | mode/anchoring.rs（含锚定参数 Hook params） |
| dsh-compaction.plugin.json | compaction-basic + tool-result-pruner | context/session.rs（8192/4096/1024 入 params） |
| dsh-goal-round-driver.plugin.json | goal + goal-round-driver | goal/ 状态机 |
| dsh-plan-mode.plugin.json | plan-mode | mode/plan.yaml + plan/ |
| dsh-schedule.plugin.json | schedule | automation/cron.rs |
| dsh-workflow-ralph.plugin.json | tool-workflow + tool-ralph | orchestration + subagent trellis |
| dsh-session-projection.plugin.json | session-projection | events/bus.rs |
| dsh-skill-registry.plugin.json | skill + skill-filesystem + tool-skill | skills/engine.rs（引用 5 JIT 技能） |

同步 `skills/dsh-market-list.yaml` 清单 8 → 16 行。

## 4. 计划模式规则段（DSH 原文，待注入 Marguerite plan 模式装配）

DSH standard 预设的 plan-mode section 是社区验证的完整规则文本（探索优先、目录跨模式不变、exit_plan_mode 必须为唯一且最后的工具调用、拒绝后回改再提交等）。融合已落地数据层：`ModeSpec` 新增 `#[serde(default)] prompt_section: Option<String>`，`plan.yaml` 已写入适配本仓工具面的规则段（exit_plan_mode 语义由 PlanGate 审批落地）；提示装配层接线待 shell 恢复后随 §5 一并提交。原文见 DSH checkout `config/agent-presets/standard/agent.cordis.yml` 的 plan-mode 段。

## 5. 锚定规格缺口（Rust 侧实施计划，待宿主 shell 恢复后落地 + 门禁）

`BloomAnchoringSpec`（mode/spec.rs）扩展如下字段并接入 `anchoring.rs` 消费（全部带默认值，向后兼容）：

- `bootstrap_max_tokens: Option<u32>`（默认 None=不设限；梁神用 1024）：首轮请求输出预算，晋升后剥离——防止「We need」触发窗口期烧掉长输出。**【数据层已落地】**
- `message_sources: Vec<String>`（默认 ["user","goal"]）：锚定期消息源白名单，非白名单（如 auto-round 续跑）不进模型——对齐 liangshen issue #578 的过滤语义。**【已落地：`AnchorState::source_allowed` + 单测】**
- `deferred_sources: Vec<String>` + `deferred_grace_steps: u32`（默认 ["agent-instructions","skill-catalog"] / 1）：晋升后延迟一步再注入工作区指令与技能目录。**【数据层已落地，装配接线待提交】**
- `promote_after_first_response: bool`（默认 true）：无工具首轮自动晋升，已参数化进 `note_no_tool_round`（置 false = 严格锚定，对齐梁神 `promoteAfterFirstResponse: false`）。**【已落地 + 单测】**
- turn/end 释放：首轮结束即解除锚定门控会话（防「双工具会话」永久化）。【待装配层接线】

**接线点（已锁定，供 shell 恢复后实现）**：

- `orchestration/mod.rs:768`（dispatch_bloom）/ `:989`（swarm）/ `:1039`（auto 闭环）三处均以 `AnchorState::new()` 构造锚定态——改为从模式 `BloomSpec.anchoring` 取规格（`with_spec`），使 bloom.yaml 中的新旋钮真正生效；`bloom_anchor_tools`（文件尾部）已按 `active_tools()` 收窄，无需改动。
- 首请求预算：`bootstrap_max_tokens` 在锚定期第一轮经 AgentRuntime → LLMClient 请求 `max_tokens` 注入，晋升后不再注入（参考 `model/llm.rs` 的 with_adaptive_profile / with_thinking 链式配置风格）。
- 消息源白名单：`source_allowed` 在目标自动轮驱动（goal 延续）与网关重放路径入口处过滤非白名单轮次。
- 延迟注入：`deferred_sources` + `deferred_grace_steps` 控制晋升后第 N 步才注入 agent-instructions / skill-catalog（装配点 = `build_model_messages` 的注入顺序，见 agent/runtime.rs）。

**有意的分歧（宪法优先，不移植）**：liangshen 的「压缩后回退受控阶段（compactionTools）」会缩小已扩展的工具面，与 AGENTS.md 红线 #2（会话内工具面只允许单调扩展、禁止中途替换/回退）冲突。Marguerite 保留单调扩展不变，压缩一律走 context/session 台账（不缩工具面），因此 `compaction_tools` 字段不落地，仅在 `dsh-presets.plugin.json` 的 Hook params 中登记为参考语义。

## 6. 验证状态（2026-08-21 最终）

- **cargo test -p marguerite-core --lib：1976 passed + 14 ignored，0 failed**（含 anchoring 4 新单测）。
- **cargo check --workspace：0 warning / 0 error**；clippy --all-targets：0 error（85 个 style 类 warning 均为工作树既有 WIP 引入，本次未新增）。
- **python3 scripts/check-doc-cli-drift.py：OK**（18 路由/嵌套动词全对齐）。`third_party/validate_integration.py` 与 `third_party/manifest.json` 在磁盘上不存在（AGENTS §7 的既有文档漂移，另案处理）。
- 数据层：16 份 PluginManifest JSON 经进程内解析 + HookSpec/SlashCommand/SkillRef 形状校验；`skills/dsh-market-list.yaml` 16 行。
- **测试基建加固（本轮新增）**：`core/src/tests/mock_http.rs` 共享基建（完整读请求后再应答）；12 处单发 read mock 全部改为循环应答/耗尽复用末条；捕获型 mock 只捕获首请求。修复宿主重启后 mock 分段读导致的 14 项随机失败。
- **环境适配修复**：宿主 Windows 系统代理（Clash 127.0.0.1:7890）会把部分 loopback 请求代答 502（可重试错误）——`mcp connect 无效主机` 测试改用 404 监听器、`gateway e2e` 改用本地 400 mock（不可重试），错误路径确定性快速完成；坑已写入 AGENTS.md §4。
- 全部门禁通过后，连同工作树既有 WIP（Track G / v1 集成 / goal / poker 等）一次性快照提交（见 §7 提交清单扩展说明）。

## 7. 提交清单（shell 恢复后）

```text
core/src/mode/spec.rs          BloomAnchoringSpec 5 字段 + ModeSpec.prompt_section
core/src/mode/anchoring.rs     新 getter + source_allowed + note_no_tool_round 参数化 + 4 单测
core/src/mode/modes/bloom.yaml anchoring 段显式写入 message_sources/bootstrap_max_tokens/deferred_sources/deferred_grace_steps/promote_after_first_response
core/src/mode/modes/plan.yaml  prompt_section 规则段
plugins/manifests/*.json       8 份新清单（dsh-presets/compaction/goal-round-driver/plan-mode/schedule/workflow-ralph/session-projection/skill-registry）
skills/dsh-market-list.yaml    清单 8 → 16 行
docs/dsh-fusion-v2-2026-08-21.md + docs/dsh-integration-v1.md（v2 指引行）
```

门禁顺序：`cargo test -p marguerite-core --lib`（含 anchoring 4 新单测与 builtin YAML 解析）→ `cargo check --workspace` → `python3 third_party/validate_integration.py` → `python3 scripts/check-doc-cli-drift.py` → git add 上述文件并提交。

## 8. 附录：DSH rc.8 官方 66 包逐包映射

| 包 | Marguerite 对应 | 状态 | DSH 官方描述（截断） |
| --- | --- | --- | --- |
| dsh-agent-tool-presentation | model/persona.rs + genui/contract.rs | 部分 | Agent-plane presentation selector: composes one agent's tool |
| dsh-app-boot | cli/argv.rs + program.rs 启动装配 | 已融合 | Shared boot glue for the app bins: .env loading, fail-loud L |
| dsh-base | lib.rs 模块骨架 + plugins/di.rs | 已融合 | The shared dsh core as a profile bundle: every profile's fir |
| dsh-cordis-client-runner | — | 缺口(浏览器侧) | Browser half of dynamic dual-half plugin packages: event sub |
| dsh-client-ui-agent-preset | mode/anchoring.rs 三阶段 + dsh-presets 清单 | 已融合(数据层) | Agent-preset surfaces: the default for later sessions, this  |
| dsh-command-compact | cli/dslash.rs /compact → context_compact | 已融合 | Human-facing slash command for explicit session compaction |
| dsh-command-goal | cli/dslash.rs /goal → goal/ | 已融合 | Human-facing slash command for persisted same-session goals |
| dsh-client-ui-cordis | — | 缺口 | Cordis dynamic-plugin definition card: the keyed cordis_defi |
| dsh-compaction-basic | context/session.rs 四层渐进压缩 | 已融合 | Token-meter-driven compaction policy and LLM summarization b |
| dsh-compaction-tool-result-pruner | context/session.rs snip_long 折叠 | 已融合 | Replay-safe model-free head/middle/tail pruning for tool-res |
| dsh-goal | goal/mod.rs | 部分 | Event-sourced same-session goal state and lifecycle service  |
| dsh-goal-round-driver | goal/mod.rs 延续轮 | 部分 | Race-fenced same-session goal-round driver |
| dsh-cmdline | cli/argv.rs 不可变参数交接 | 已融合 | Immutable command-line handoff from a dsh launcher to any ap |
| dsh-headless | cli 单指令 -p/--pipe 模式 | 已融合 | The dsh one-shot bundle: a direct core Agent/Session runner  |
| dsh-mcp-client | mcp/client.rs + registry | 已融合 | MCP client bridge: connects to MCP servers and registers the |
| dsh-home-paths | persistence/config.rs data_dir | 已融合 | Shared filesystem path helpers for the DeepSeek Harness |
| dsh-persona | model/persona.rs + agent/catalog + router/persona.rs | 部分(缺 complete/includeRuntimeContext) | Composition-authored deployment persona section for the Deep |
| dsh-plan-mode | mode/plan.yaml + plan/approval.rs + PlanGate | 已融合(数据层 prompt_section 本轮) | Logged per-agent plan mode with deployment guidance, a direc |
| dsh-terminal | tools/terminal.rs 持久 PTY | 已融合 | Persistent PTY session seam for the DeepSeek Harness — owner |
| dsh-terminal-bash | tools/terminal.rs + security/exec.rs | 已融合 | Persistent shell PTY backend over the DeepSeek Harness subpr |
| dsh-pwsh-local | security/exec.rs pwsh | 已融合 | Local PowerShell implementation of the DeepSeek Harness bash |
| dsh-launch-environment | — | 缺口(低优先级) | Immutable DeepSeek Harness launch environment that records w |
| dsh-pwsh-sandbox | security/exec.rs + sandbox/policy.rs | 已融合 | Sandbox-consuming implementation of the DeepSeek Harness Pow |
| dsh-fs-local | tools/fs 系列 + sandbox/policy.rs | 已融合 | Local-filesystem implementation of the DeepSeek Harness file |
| dsh-session-projection | events/bus.rs + gateway context_status | 部分 | Session-projection seam: the merge-extensible projection typ |
| dsh-session-reference | context/session.rs 台账 + session/rollback.rs | 部分 | Cross-session snapshot references and durable untrusted mode |
| dsh-time-context | — | 缺口(低优先级) | Opt-in durable per-step context with the current time and el |
| dsh-skill | skills/engine.rs + registry | 已融合 | Agent skill provider registry for the DeepSeek Harness |
| dsh-skill-filesystem | skills/loader.rs 扫描 | 已融合 | Local filesystem skill provider for the DeepSeek Harness |
| dsh-tmux-context | — | 缺口(不适用) | Opt-in durable per-step context with this agent's tmux pane  |
| dsh-token-meter | context/budget.rs + model/llm.rs token_estimate | 已融合且更精细 | Replay-aware token measurement service (ctx.tokenMeter) for  |
| dsh-tool-ask-user | mode/ask.rs + genui 交互 | 已融合 | Model-facing ask_user_question tool over the ctx.userQuestio |
| dsh-tool-bash | tools/builtin.rs bash + 沙箱 | 已融合 | Model-facing bash tool with optional generic background-job  |
| dsh-tool-bash-persistent | tools/terminal.rs 持久会话 | 已融合 | Model-facing owner-scoped persistent Bash tool backed by the |
| dsh-tool-pwsh-persistent | tools/terminal.rs pwsh | 已融合 | Model-facing owner-scoped persistent PowerShell tool backed  |
| dsh-tool-cordis | plugins/di.rs guarded_provide 自检 | 部分(无模型面工具) | Self-referential cordis toolset: inspect the live runtime, m |
| dsh-jobs-local | automation/long_running.rs + 后台任务 | 已融合 | Process-local implementation of the DeepSeek Harness backgro |
| dsh-tool-fs | tools/builtin.rs read/write/edit | 已融合 | Model-facing filesystem tools (read, write, edit) over the D |
| dsh-tool-pwsh | tools/builtin.rs pwsh | 已融合 | Model-facing pwsh tool over the bash executor seam |
| dsh-tool-fs-search | tools/builtin.rs glob/grep | 已融合 | Model-facing filesystem discovery tools (glob, grep) backed  |
| dsh-tool-ralph | agent/subagent.rs fresh 子代理循环(待工具化) | 部分 | Model-facing fresh-agent Ralph loop over the workflow and su |
| dsh-tool-skill | skills/engine.rs use_skill | 已融合 | Model-facing skill loading tool for the DeepSeek Harness |
| dsh-schedule | automation/cron.rs + host_board | 部分 | Agent-scoped durable after, at, and fixed-rate reminders ove |
| dsh-tool-str-replace-editor | tools/builtin.rs str_replace_editor | 已融合 | Model-facing view, create, literal replace, and line insert  |
| dsh-tool-subagent-control | agent/trellis.rs send_message/interrupt/list | 已融合 | Globally named send_message, interrupt_agent, and list_agent |
| dsh-tool-jobs | automation/long_running.rs 查询/终止 | 已融合 | Model-facing background job control tools (job_output, job_l |
| dsh-tool-goal | goal/mod.rs 工具面 | 部分 | Model-facing same-session goal tools with execution-time aut |
| dsh-tool-todo | todo_write 等价 + plan 步骤 | 已融合 | Model-facing todo_write tool over the DeepSeek Harness event |
| dsh-tool-web | tools/builtin.rs web_search | 已融合 | Model-facing web tools (web_search, web_fetch) over the Deep |
| dsh-tool-subagent | agent/subagent.rs spawn/fork | 已融合 | Model-facing subagent delegation tool over the ctx.subagents |
| dsh-web-app | gateway/ + webui/ 回环托管 | 已融合 | The dsh browser-surface bundle: the web patch layer over dsh |
| dsh-agent-instructions | governance/ + AGENTS.md 宿主注入 | 已融合 | Workspace context loader for AGENTS.md/CLAUDE.md instruction |
| dsh-tool-workflow | orchestration/dag.rs 扇出(缺脚本引擎) | 部分 | Model-facing workflow tool: run a JavaScript orchestration s |
| dsh-workflow-worker-thread | orchestration/dag.rs worker | 部分 | worker-thread workflow engine: executes model-written orches |
| dsh-agent | agent/runtime.rs + stream_loop | 已融合 | Agent interface, registry, initiator scope, and event vocabu |
| dsh-host-frontend-static | webui/dist 静态托管(gateway) | 已融合 | SPA dist server for the Web shell: owns the webserver fallba |
| dsh-host-apiproxy | gateway/protocol.rs + methods.rs | 已融合 | API gateway: the ApiProxy contract (api/), the fetch carrier |
| dsh-host-webserver | gateway/server.rs 回环 | 已融合 | Web route-registration plugin: HTTP and upgrade routes, inde |
| dsh-llm | model/llm.rs 供应商中立接口 | 已融合 | Provider-neutral LLM service interface for the DeepSeek Harn |
| dsh-llm-mock-server | tests mock_llm_server | 已融合 | ? |
| dsh-loader-smoke | — | 缺口(CI 专用) | ? |
| dsh-session | session/ + persistence/session.rs 事件日志 | 已融合 | Event-sourced session store for the DeepSeek Harness |
| dsh-settings | persistence/config.rs + $1marguerite/config.yaml | 已融合 | Abstract user-settings seam (ctx.settings) for the DeepSeek  |
| dsh-subagent | agent/subagent.rs 注册表 | 已融合 | Abstract subagent seam (ctx.subagents): named-provider regis |
| dsh-system-prompt | router/persona.rs + stable_prefix 装配 | 已融合 | System prompt assembly registry for the DeepSeek Harness |
| dsh-tools | tools/ 注册表 + mcp/registry | 已融合 | Tool registry and execution pipeline for the DeepSeek Harnes |

## 9. 参考

- DSH checkout：node_modules/@deepseek-ai/dsh（66 包，rc.8）；presets：config/agent-presets/{minimal,standard,code,cordis}
- 梁神：~/.dsh/profiles/web/node_modules/@linxin666/dsh-liangshen/presets/liangshen/{preset.yml,agent.cordis.yml}
- web-ui 全家桶：@linxin666/dsh-web-ui-all 0.2.5（16 子插件清单见其 package.json）
- Marguerite v1 集成：docs/dsh-integration-v1.md（8 清单 + 7 斜杠 + 6 槽位 + 63 单测）
