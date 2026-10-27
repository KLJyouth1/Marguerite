# Marguerite 能力路由 · 缓存命中 · 电脑操控 · 全端缺陷 修复与补强施工计划

| 项 | 值 |
|---|---|
| 文档日期 | 2026-10-26 |
| 适用版本 | 0.2.17（`core/Cargo.toml`，已用 `check-version-sync.py` 核实 13 处一致） |
| 文档状态 | 待批准；**未开始任何代码实施** |
| 事实基线 | 2026-10-26 工作区实测（每条结论都标注了文件与行号，见附录 A） |
| 读者 | 主代理、实施工程师、QA、发版负责人 |
| 关联文档 | `AGENTS.md`、`README.md`、`README.zh-Hans.md`、`docs/cli-reference.md`、`docs/development.md`、`docs/architecture.md`、`docs/PRODUCT.md`、`docs/plans/2026-10-17-feasibility-and-roadmap.md` |

> 本文档的目标不是"列出想做的事"，而是**把每一处改动落到文件、函数、契约、测试、验收和回滚**，使另一个工程师在不询问作者的情况下可以逐条施工。
>
> 本文档同时是一份**约束文件**：施工期间新增的决定若与 §2/§3 冲突，必须先改本文档并说明理由。

---

## 0. 怎么读这份文档

三条主线，互相有依赖，不要打乱顺序：

1. **先让门禁说真话（W0、W1、W2、W9）**：今天的 `verify.sh --all` 并不绿——`check_gui_shell_config.py` 实测 `exit 1`；CI 的 gui job 调用了 `gui/package.json` 里**不存在**的 4 个脚本；双引擎事件被拍平成 `session.event.status`；三个 Rust 占位 handler 对前端返回假成功。**在门禁失真时做任何"优化"，都无法证明没有回归。**
2. **再做能力与缓存（W3、W4、W5）**：用户提出的"模型只会写脚本盲控""缓存命中率低"是同一组根因的两面——工具面每轮变化、提示词把可变内容放进稳定前缀、固定高频工具只有 shell/code、模型不知道 memory/kb/browser/desktop/MCP 的真实入口。
3. **最后做电脑操控与体验（W6、W7、W8）**：结构化 UIA 双轨、observe→act→verify、ACL 补齐、工作区最近列表与命令面板。

阅读顺序建议：

- 想先看"到底坏在哪"：§4 缺陷总账。
- 想直接施工：§5 工作包 + 附录 A 行号索引。
- 想审方案：§6（能力路由）、§7（缓存）、§8（电脑操控）。
- 想知道"怎么算完成"：§3 门禁 + §13 DoD。

证据等级（全文统一）：

| 等级 | 含义 |
|---|---|
| **实测** | 本次会话真实运行命令或读取当前工作区文件确认 |
| **代码核实** | 已读到具体文件与行号的当前内容（可能未运行） |
| **推断** | 由代码结构推出，尚未用运行证据确认；施工前必须补测 |

---

## 1. 范围与非目标

### 1.1 范围内

1. **门禁复位**：让 `bash scripts/verify.sh --clean --all` 在当前工作区真实全绿，并让 CI 的 5 个 job 与本地门禁同源。
2. **双引擎契约**：事件名透传、占位实现清理、cron 契约对齐。
3. **安全边界**：远程命令审批、系统打开路径、扑克越界、限流接线、注入扫描语义化。
4. **能力路由**：模型能稳定知道"有哪些能力、何时用哪个、不要用脚本替代"；可观测、可评测。
5. **缓存命中**：稳定前缀纪律、工具面稳定、Anthropic 断点、诚实的四层指标、受控基准。
6. **电脑操控**：结构化 UIA 优先、像素兜底；observe→act→verify；审批与审计；每步可回执。
7. **前端体验**：工作区"最近打开"、命令面板补工作区/模型/团队、静默失败清理。
8. **发布与文档**：README 占位符、根级第三方声明、ACL、安装版验收。

### 1.2 非目标（明确不做）

- 不重写引擎、不替换 Tauri/Vue/FastAPI 技术栈、不合并 `pyengine` 与 Rust 引擎。
- 不新增公网监听；不改「Web UI 仅回环」不变量。
- 不为了"看起来更快"引入未经测量的缓存或预填充；不做伪预热。
- 不删除 `workbench/`、`third_party/`、`gui/src` 归档。
- 不在本计划内完成 538 行硬编码中文的 i18n 全量翻译（只做棘轮防新增，见 W12.3）。
- 不重写历史（`git filter-repo`）；已泄露密钥只能吊销，不在本计划自动执行。

### 1.3 合规红线（"破甲"的唯一允许解释）

用户提出的"破甲能力"在本计划中**只允许**以下三类，且必须可审计：

1. **能力评测**：用公开基准与自建评测集测量模型在工具选择、长上下文、指令遵循上的真实上限。
2. **提示与上下文优化**：更清晰的能力卡、更稳定的前缀、更好的工具描述、更少的歧义——即"让模型发挥它本来就有的能力"。
3. **失败可观测**：记录模型为何选错工具、在哪一步降级，用数据驱动下一轮提示词改进。

**明确禁止**（出现即视为方案缺陷，必须打回）：

- 绕过或削弱模型/产品的安全分类器、审批、沙箱、审计。
- 提取隐藏思维链、系统提示或供应商私有参数。
- 对第三方服务做提示注入、越权探测、速率攻击。
- 以"压榨模型潜能"为名关闭 `PlanGate` / `CompletionGate` / 审批 / 限流。
- 把"100% 缓存命中"写成营销数字；见 §1.4。

### 1.4 诚实性红线（缓存指标）

"DeepSeek Harness 100% 缓存命中"在本计划中被严格定义为：

> **同一会话、工具面不变、无压缩的稳态请求中，可缓存前缀的请求命中率 = 100%。**

它**不是**"所有 token 都命中"，也**不是**"首轮就命中"。任何对外文案必须同时给出分母与口径，且数字来自真实计数（`cache_read_input_tokens` / `prompt_tokens`），不得由 `read / (read + creation)` 推导后冒充总体命中率——后者在 DeepSeek 上恒等于 100%（因为 `cache_creation` 通常为 0），是当前"看起来 100%"的根因（见 D-30）。

---

## 2. 规章制度（施工期强制）

| ID | 规则 | 为什么（证据） | 怎么执行 |
|---|---|---|---|
| R-01 | **无新鲜验证证据，不得声称完成** | `AGENTS.md` §8 铁律；本仓库历史上出现过"调了工具就宣称完成" | 每个工作包结束必须贴当次 `verify.sh` 输出；禁止"应该/大概/看起来" |
| R-02 | **`[SKIP]` 永远不算通过** | `verify.sh` 的 `skip_check()` 设计（L105-L116） | 新增跳过必须走 `skip_check`；汇总里必须出现；不得用 `echo [SKIP]` 绕过 |
| R-03 | **不得并发跑两份 `verify.sh --all`，不得并发跑 cargo** | `AGENTS.md` §2：`target/` 锁与 e2e 端口 18877 会互相污染，实测通过数从 95 掉到 92 | 施工前 `Get-Process marguerite-*` 查残留；同一时间只允许一个全量门禁 |
| R-04 | **改 core 后桌面端必须重链接并重启** | `marguerite-gui` 静态链接 core；只重编 CLI 不会进入桌面进程 | 桌面验收前 `cargo build -p marguerite-gui`；确认 exe mtime 晚于 core 改动 |
| R-05 | **不得用终端乱码判断编码** | 本机控制台 cp936；`AGENTS.md` §4 记录了三次假警报 | 用 `Read` 工具或 `open(...,'rb')`；只看 `[PASS]/[FAIL]` 与退出码 |
| R-06 | **版本号单一事实源** | `core/Cargo.toml` 的 `version`，13 处同步 | 改版本只走 `python scripts/check-version-sync.py --fix` |
| R-07 | **路由三处同步** | `argv.rs::SUBCOMMANDS`、`program.rs::route_table.register`、路由计数断言 | 改 CLI 路由必须三处一起改，并跑 `check-doc-cli-drift.py` |
| R-08 | **稳定前缀纪律（新）** | §7：任何进入 `build_model_messages` 稳定半区的字符串，不得包含轮次、数量、时间、随机值、当前工具清单变化 | 新增提示词必须写 `stable:` 或 `turn:` 注释；`stable_prefix_drift` 必须为 0 |
| R-09 | **工具面单调扩展** | `AGENTS.md` §3.1 红线 #2；Bloom 锚定只加不减 | `SessionToolFace` 只允许 `extend`，不允许 `retain/clear`；单测钉住 |
| R-10 | **禁止假成功 handler** | 本计划 D-10/D-11/D-12：`{ok:true}` 但不做事比报错更坏 | 任何 RPC 必须满足"返回成功 ⇒ 副作用已发生"；否则回 `NOT_IMPLEMENTED` 或 `degraded:true` |
| R-11 | **不得用"修复"绕过安全控制** | 本计划 §1.3 | 评审清单逐条对照；审批/沙箱/限流只能加强或显式配置化，不能默认关闭 |
| R-12 | **能力路由必须确定、可测** | 不能引入第二次 LLM 调用来"决定用哪个工具"（成本与不确定性） | `route()` 是纯函数；固定输入必须固定输出；新增规则必须有表驱动测试 |
| R-13 | **任何新指标必须定义分子/分母** | D-30 的教训：`read/(read+creation)` 在 DeepSeek 上恒 100% | 指标结构体字段必须带 `numerator`/`denominator`/`scope` 文档；前端标签不得省略口径 |
| R-14 | **新增守卫必须有变异验证** | 本仓库多次出现"守卫恒绿"（如旧 toon 光照守卫） | 每条新守卫至少做一次"故意改坏 → 必须红 → 恢复 → 必须绿" |
| R-15 | **提交粒度与暂存纪律** | `AGENTS.md` §4：部分暂存会让索引自相矛盾 | 每包一个提交；提交前 `git status` 逐项看；不提交机器状态（`*.db`、`.tmpdir/`、日志） |
| R-16 | **密钥与作者路径** | `check-secrets.py` 常驻；历史密钥仍需吊销 | 新代码不得出现 `sk-` 真值、`D:\Downloads\Marguerite` 等本机路径 |

---

## 3. 门禁要求

### 3.1 当前基线（实测，2026-10-26）

| 项 | 实测结果 | 说明 |
|---|---|---|
| `python scripts/check_gui_shell_config.py` | **exit 1** | 主窗口 `url = "splash.html"`，守卫要求包含 `/control/`。这是**当前 `--all` 不绿的第一根因** |
| `python scripts/check-readme-claims.py` | exit 0，`24/24` | README 声称的总数与 `--all` 实际调度数一致；但 README 声称的"全绿（2026-09-25）"已过期 |
| `python scripts/check-version-sync.py` | exit 0，13 处一致 | 版本 0.2.17 |
| `core/tests/` | 7 个文件、32 个测试、1 个 `#[ignore]` | **没有任何门禁执行**（`grep core/tests scripts/verify.sh .github/workflows/ci.yml` = 0） |
| CI `gui` job | 必红 | 调用 `npm run typecheck/lint/test:unit/build`，而 `gui/package.json` 只有 `tauri/deps:lockfile-sync/shell:check/shell:clippy/verify:no-frontend` |
| CI `rust` job | 结构正确 | 已安装 WebKitGTK/GTK 依赖并构建 workspace（含 Tauri） |
| `verify.sh --all` 调度数 | 24 | 新增 `check_core_integration_tests` 后变 25，两份 README 必须同步 |

> **未做的事**：本轮没有运行完整 `verify.sh --clean --all`（约 25 分钟，且会与当前可能存在的 daemon 争锁）。本计划的第一个动作就是 W0.1，在干净进程状态下跑一次完整门禁，记录真实基线。

### 3.2 门禁分层（保持既有，新增 1 项）

| 模式 | 用途 | 本计划的要求 |
|---|---|---|
| `--fast` | 提交前快闸 | 每次提交前必跑 |
| `--core` | Rust 栈 | W1/W2/W3/W4/W5/W6 每包必跑 |
| `--webui` | 前端栈 | W5/W8 每包必跑 |
| `--gui` | 桌面壳 | W0/W7 必跑 |
| `--pyengine` | 侧车三层 | W1/W2/W9 必跑 |
| `--workbench` | 第五入口 | W11 必跑 |
| `--all` | 日常全量 | 每个工作包结束必跑 |
| `--release` | 发版 | W12 收口时必跑 |

新增：

- **`check_core_integration_tests`**：`cargo test -p marguerite-core --tests -- --skip perf_call_chain_under_50ms`，只跑确定性集成测试；`third_party_integration` 在 `third_party/plugin-examples` 不存在时用 `skip_check` 明确跳过（见 W10）。
- **不新增**缓存基准与能力评测为门禁：它们依赖真实模型或较长运行时间，属于 `scripts/bench/` 下的**手动基准**，结果写入 `artifacts/`（不随仓库发布）。门禁只跑其**纯函数部分**（指标公式、路由决策、序列化结构）。

### 3.3 每个工作包的必跑门禁

| 工作包 | 必跑 | 额外证据 |
|---|---|---|
| W0 | `bash scripts/verify.sh --clean --all` | 完整输出 + exit 0 + **24/24**（W10 引入第 25 项后改为 25/25） |
| W1 | `--core` + `--pyengine` | 事件名单测 + WS 往返测试 |
| W2 | `--core` + `--pyengine` + `--webui` | 每个占位 handler 的"在线代理/离线诚实"双测 |
| W3 | `--core` | 安全用例（越权/审批/越界/限流）+ 变异 |
| W4 | `--core` + `--webui` | 路由表驱动测试 + 能力评测（手动） |
| W5 | `--core` + `--webui` | 指标公式测试 + 缓存基准（手动） |
| W6 | `--core` + 真机 | Windows 真机场景 8 条 + 审计快照 |
| W7 | `--gui` + 安装版 | release 下 invoke 成功 |
| W8 | `--webui` | Playwright e2e + 截图 |
| W9 | `--core` + `--webui` + `--pyengine` | 三种 cron 形状 |
| W10 | `--core` | 32 例真实执行/明确跳过 |
| W11 | `--workbench` | `/api/health` 200 |
| W12 | `--clean --release` | 安装包 + 安装版验收 |

### 3.4 变异验证（每条新守卫必须做）

模板：

1. 记录绿态。
2. 故意注入缺陷（改一行）。
3. 运行守卫，确认**精确报红**且错误信息指向该行。
4. 恢复，确认绿。
5. 把"注入 → 红"的命令与输出写进 PR 描述。

本计划要求做变异的条目：W0.1（守卫接受 splash 的条件）、W0.2（SKIP 登记）、W1（事件名透传）、W2（占位不再假成功）、W3（审批/边界）、W4（反脚本守卫）、W5（指标公式与 Anthropic 断点）、W6（UIA action 与审计快照）、W8（最近工作区）、W10（集成测试门禁）。

### 3.5 发布门禁

发版必须：

1. `bash scripts/verify.sh --clean --release` 全绿（含 `cargo tauri build`）。
2. 走 `scripts/package_full.cmd`，不得直接 `cargo tauri build`（否则 `resources/pyengine` 可能是旧快照；`AGENTS.md` §4 已实测）。
3. 安装版验收：解包 MSI → 以解包目录为工作目录启动 → `GET /api/config` 必须 200（双引擎在线），不是 501。
4. 更新 `CHANGELOG.md`、两份 README 的基线与闸门数、`AGENTS.md` §2 的测试基线。

### 3.6 门禁禁止事项

- 禁止 `--no-verify`、跳过 pre-commit。
- 禁止把 `[SKIP]` 计入通过数。
- 禁止在 `verify.sh` 里复制 pytest 命令行（marker 排除表只能有一处）。
- 禁止为了变绿而删测试、放宽断言、加超时。若确属环境问题，写进 `KNOWN-LIMITATIONS` 并用 `skip_check` 具名登记。

---

## 4. 缺陷总账

### 4.1 统计

| 级别 | 数量 | 定义 |
|---|---|---|
| P0 | 12 | 阻断发布、门禁失真、用户可见功能坏掉、安全边界缺失 |
| P1 | 21 | 契约不一致、能力不可用、显著体验缺陷 |
| P2 | 12 | 可观测性、文档漂移、次要体验 |
| P3 | 5 | 清理与长期改进 |

> 编号 `D-xx` 全文引用。每条都给出"当前事实"（文件:行）与"计划动作"（工作包）。

### 4.2 P0

| ID | 缺陷 | 当前事实（证据） | 计划动作 |
|---|---|---|---|
| D-01 | GUI 壳守卫与真实启动流程冲突，`--all` 必红 | `scripts/check_gui_shell_config.py:59-65` 要求 `url` 含 `/control/`；`gui/src-tauri/tauri.conf.json:24` 是 `splash.html`；`lib.rs:3319-3369` 在网关就绪后 `navigate(control_url())`。实测 exit 1 | W0.1 |
| D-02 | CI `gui` job 必红 | `.github/workflows/ci.yml:257-293` 调 `npm run typecheck/lint/test:unit/build`；`gui/package.json:6-13` 只有 `tauri/deps:lockfile-sync/shell:check/shell:clippy/verify:no-frontend` | W0.3 |
| D-03 | `verify.sh` 3 个检查的 4 处 SKIP 未登记进汇总 | `scripts/verify.sh:424-451` 的依赖审计（2 处）/typecheck/单测只 `echo [SKIP]` + `return 0`，不调用 `skip_check`、也不 `SKIPPED+=` → 汇总里消失（同文件 L329/370/876 的 SKIP **是**正确登记的，不要误改） | W0.2 |
| D-04 | `core/tests` 32 例是死测试树 | 7 个文件存在；`grep core/tests scripts/verify.sh .github/workflows/ci.yml` = 0 | W10 |
| D-05 | 所有 pyengine 事件被拍成 `session.event.status` | `core/src/pyengine/event_bridge.rs:56` 固定 `EventType::Status`；`core/src/gateway/events.rs:17-33` 只映射 4 个特例，其余 `session.event.{other}` → `status` | W1 |
| D-06 | `sessions.messages.snapshot/hydrate/subscribe` 假成功 | `core/src/gateway/methods.rs:1664-1680` 固定回 `events:[]`、`hydration_complete:true`、`current_stream_seq:0`；且在 `RUST_FIRST_METHODS:398-399`，侧车在线也永不咨询 | W2 |
| D-07 | `skills.deps.install` 不安装却回成功 | `core/src/gateway/methods.rs:1874-1878` 回 `{ok:true,deps_installed:true}`；pyengine `rpc_skills.py:688` 有真实实现 | W2 |
| D-08 | `sandbox.resume` 不恢复却回成功 | `core/src/gateway/methods.rs:1870-1873` 回 `{ok:true,status:"resumed"}`；pyengine `rpc_sandbox.py:913` 有真实实现（清除 denial-pause） | W2 |
| D-09 | 固定高频工具面只有文件/shell/code/search，模型因此偏好脚本 | `core/src/agent/tool_schema.rs:108-116` 固定 7 个：`read_file/list_dir/str_replace_editor/write_file/web_search/shell_exec/code_exec`；`memory_search/browser/desktop_*/kb_search/list_skills/use_skill` 不在默认面 | W4 |
| D-10 | 能力自述存在但从未注入 | `core/src/agent/capability.rs:238` `brief()` 全文 0 调用（只有定义）；文件头 L11 自认"本轮不接注入点" | W4 |
| D-11 | 缓存指标 `read/(read+creation)` 在 DeepSeek 上恒 100%，是"假 100%"根因；且总量口径因 provider 而异（Anthropic `input_tokens` 不含缓存读写） | `core/src/cache/mod.rs:578-587`；`core/src/gateway/cache.rs:100` 直接乘 100 当前端主指标；`core/src/model/llm.rs:2291-2295` 用裸 `input_tokens` 上报 | W5 |
| D-12 | Anthropic 路径没有 `cache_control`，前缀缓存结构性为 0 | `core/src/model/llm.rs:824-838` 把多个 System 拼成一个字符串；`983-1007` 只输出 `system` 字符串与裸 `tools`，无断点 | W5 |

### 4.3 P1

| ID | 缺陷 | 当前事实 | 计划动作 |
|---|---|---|---|
| D-13 | 工具面每轮按请求文本重算 → 工具数组每轮变化 → 服务端前缀从最前失配 | `runtime.rs:785`（非流式）、`1101`（流式）每 turn `ToolDisclosure::new(...)`；`tool_schema.rs:159-190` 按 `relevance_score` 排序后截断 | W5.1（依赖 W4.2 稳定面） |
| D-14 | `tool_guide_text` 把可变数量/名单嵌入稳定前缀 | `core/src/agent/runtime.rs:166` `"本轮可调用工具共 {n} 个：{names}。"`；`495-507` 该 guide 位于 history 之前 | W5.2 |
| D-15 | `catalog_text` 的 B 段随工具面变化 | `core/src/agent/tool_catalog.rs:317-384`；B 段"本轮可见工具（按用途分组）"每次重建 | W5.2 |
| D-16 | 稳定前缀漂移只在压缩器上观测，不是完整前缀 | `core/src/agent/tool_compress.rs:217` 是 `observe_stable_prefix` 唯一生产调用点 | W5.3 |
| D-17 | 前端把本地命中率与服务端命中率混为一个数 | `webui/src/components/island/game/useEngineBridge.ts:133-136` `Math.max(client, server)` | W5.6 |
| D-18 | cron 契约三重不一致：Rust 返回 `{items}`、前端只认 `{jobs}`/数组；Rust 字段是 `next_fire_ms/created_at_ms`、前端读 `next_run/last_run`；pyengine 返回数组 | `methods.rs:1723-1748`；`useCronJobs.ts:10-35`；`pyengine/src/opensquilla/gateway/rpc_cron.py:562`；`webui/src/types/cron.ts:1-30` | W9 |
| D-19 | 远程 SSH 执行任意命令，无审批 | `core/src/gateway/collab.rs:118-175` `collab_ssh_exec` 直接 `ssh ... command`；全文件 0 处 `approval`/`Policy` | W3.1 |
| D-20 | `open_in_system` 任意路径 + `cmd /c start` 元字符面 + 可执行扩展名 | `core/src/gateway/_helpers.rs:70-88`；调用方 `artifacts.rs:132-155` 无 id 时接受任意 `path` | W3.2 |
| D-21 | `poker.auto` 空房/越界 panic 会毒化全局锁 | `core/src/gateway/native.rs:111-112` `let idx = room.current_seat as usize; let p = &room.players[idx];` 无边界检查 | W3.3 |
| D-22 | 限流是死代码 | `core/src/gateway/rate_limit.rs` 完整；`core/src/gateway/mod.rs:61` 导出；`server.rs` 0 引用 | W3.4 |
| D-23 | `computer_use` 只有像素动作，无结构化选择 | `core/src/tools/computer_use.rs:1917` 动作枚举 24 项，无 `a11y`/`invoke`/`set_value`/`toggle`/`select` | W6 |
| D-24 | UIA 只暴露 snapshot/click，其余能力存在但未接线 | `core/src/desktop_automation/uia_driver.rs:165-187`；`windows_uia.rs:40-56,279-295` 已有 Invoke/Toggle/SelectionItem pattern 与 10s 超时 | W6.1 |
| D-25 | 桌面动作只能用 `desktop_launch` 返回的 handle，不能附着已有前台应用 | `computer_use.rs:1804-1880` 的 `desktop_action` 要求 `handle`；`desktop_launch` 才产生句柄 | W6.2 |
| D-26 | 动作没有 observe→act→verify，回执无前后状态 | `desktop_automation/driver.rs:127-135` `ActionReceipt` 只有 ok/detail/snapshot_path；`uia_driver.rs:179` 前后快照都是 `None` | W6.3 |
| D-27 | Policy 按 kind 字符串判断，拦不住"点删除按钮"，却误拦含 `|`/`cmd` 的合法文本 | `desktop_automation/policy.rs:33-45` 只看 kind；`50-72` 对 `action.params.to_string()` 全文子串匹配 | W6.4 |
| D-28 | 桌宠 ACL 缺 `show/toggle`，`pet-overlay` 窗口不在 capability | `gui/src-tauri/build.rs:278-290` 命令清单只有 move/hide；`capabilities/default.json` windows 只有 `main`，权限只有 move/hide；前端真实 invoke 三条（`webui/src/platform/desktopBridge.ts:177-193`） | W7 |
| D-29 | 单选工作区总是弹原生目录选择器，即使已有工作区 | `webui/src/components/ProjectWorkspacePickerDialog.vue:530-542` 单选直接 `runNativePicker`；多选才 `web-ready` | W8.1 |
| D-30 | 工作区没有"最近打开"排序/时间 | `core/src/gateway/workspaces.rs:20-28` `WorkspaceEntry` 只有 `created_at`；`list()` 无排序 | W8.1 |
| D-31 | 命令面板缺工作区/模型/团队切换 | `webui/src/components/CommandPalette.vue` 已有 nav/session/message/recent/mode，无 workspace/model/team | W8.2 |
| D-32 | `core/tests/third_party_integration.rs` 依赖被 gitignore 的 `third_party/`，接入门禁会红 | 文件 `third_party_dir()` 断言目录存在；`third_party/` 实测不在 clone 中 | W10 |
| D-33 | `workbench` 健康检查路径错误，默认 404 | `workbench/frontend/src/api/client.ts:35,197` `../health` → `http://localhost:8011/health`；后端只有 `api.py:353 @app.get("/api/health")` | W11 |

### 4.4 P2 / P3

| ID | 级别 | 缺陷 | 当前事实 | 计划动作 |
|---|---|---|---|---|
| D-34 | P2 | 未登记路径的产物可直接按 `path` 打开 | `core/src/gateway/artifacts.rs:132-155` | W3.2 |
| D-35 | P2 | `session.event.status` 只处理 steer，无其他消费者；但拍平导致 state_change/cron_result/task_group.* 丢失 | `useChatRpcEventHandlers.ts:1460-1475`；前端监听见 `useChatRpcSubscriptions.ts:74-88` | W1 |
| D-36 | P2 | `a11y_tree.rs` 文件头仍写"真实 OS 接入未做" | `core/src/desktop_automation/a11y_tree.rs:1-27`，而 `windows_uia.rs` 已实现 | W6.6 |
| D-37 | P2 | 没有 `desktop.a11y` RPC，浏览器有 `browser.a11y` | `core/src/browser/gateway.rs:236`；桌面侧无对应 | W6.5 |
| D-38 | P2 | 8 处 `catch → return []` 静默失败 | `webui/src/App.vue:1068`、`webui/src/components/workbench/terminalWorkbenchProvider.ts:316/343`、`webui/src/composables/chat/useOrchestrationRuns.ts:109`、`webui/src/composables/usePendingRestart.ts:46`、`webui/src/utils/chat/metaDiscardOutbox.ts:54`、`webui/src/utils/chat/sessionNavigationDiag.ts:49`、`webui/src/views/TerminalView.vue:163` | W8.3 |
| D-39 | P2 | README 的 `OWNER/REPO` 占位符未替换 | `README.md:11,111`、`README.zh-Hans.md:10,104`；真实 remote = `KLJyouth1/Marguerite` | W0.4 |
| D-40 | P2 | 根级 `THIRD_PARTY_NOTICES.md` 不存在 | 只有 `pyengine/THIRD_PARTY_NOTICES.md` | W0.5 |
| D-41 | P2 | README 基线过期（2704 tests / 24/24 "green"） | `README.md:15,239,246`；`README.zh-Hans.md:14,216,223` | W12.1 |
| D-42 | P2 | Tauri 前端依赖仍保留 React/ESLint/Vitest，审计噪声与安装体积 | `gui/package.json:13-38` | W0.6（可选） |
| D-43 | P2 | `useEngineBridge` 的 `cacheHitRate` 无法解释 | 同 D-17 | W5.6 |
| D-44 | P2 | 能力评测没有基线，无法判断提示词改动是否有效 | 无评测集 | W4.7 |
| D-45 | P2 | `tools.patch` / MCP 热插拔会改变工具面，可能造成前缀漂移但无告警 | `methods.rs` tools.patch 分支；无 drift 关联 | W5.3 |
| D-46 | P3 | `docs/assets/*.png` 可能过期 | AGENTS §4 已记录 | W12.2 |
| D-47 | P3 | 538 行模板硬编码中文 | AGENTS §4 记录 | W12.3（只加棘轮） |
| D-48 | P3 | `gui/` 4 条 npm 公告未处理 | 需破坏性升级 vite 8 | 保持现状，W0.6 后重新评估 |
| D-49 | P3 | `third_party/` 产物不随仓库发布 | `AGENTS.md` §7 红框 | 不在本计划处理，仅记录 |
| D-50 | P3 | `docs/plans/` 旧计划未跟踪 | `2026-10-17-feasibility-and-roadmap.md` untracked | 本计划落盘时一并说明 |

### 4.5 已核实为"非缺陷"的历史结论（禁止重复施工）

| 历史结论 | 当前事实 | 结论 |
|---|---|---|
| "工具目录没有注入提示词" | `core/src/agent/mod.rs:20` 引入；`runtime.rs:220-227 compose_guide`；流式/非流式/子代理共 6 处注入 | 已实现，不再列为待办 |
| "推理强度没有接线" | `core/src/model/llm.rs` 的 `build_body` 已按 supported 写 `reasoning.effort`；`useEngineBridge.ts:125-128` 读取 `thinking_effort` | 已实现 |
| "Windows UIA 是空壳" | `core/src/desktop_automation/windows_uia.rs` 有真实 COM、pattern、超时、节点上限 | 已实现，缺的是**动作暴露**（D-24） |
| "浏览器没有 a11y" | `core/src/browser/cdp.rs:119-220` 有 `Accessibility.getFullAXTree`、`DOM.getBoxModel`；`browser/gateway.rs:236` 有 `browser.a11y` | 已实现，桌面侧需对齐（D-37） |
| "`state_change` 是幻影事件" | pyengine `engine/types.py:145-147` 定义并构造；前端 `webui/src/components/island/islandRuntime.ts:25`、`webui/src/composables/chat/useChatRpcSubscriptions.ts:74` 监听 | 真实事件；缺陷是桥接拍平（D-05） |
| "命令面板只有 7 个 action" | 已有 nav/mode/session/message/recent 等 10+ 类 | 已实现，缺 workspace/model/team（D-31） |

---

## 5. 工作包（施工细则）

> 每个工作包按"目标 → 依赖 → 改动清单 → 契约 → 测试 → 验收 → 回滚"组织。
> 行号以 2026-10-26 工作区为准；若施工时漂移，以函数名/符号为准并在 PR 里更新附录 A。

### W0 门禁复位（P0，预计 1 天）

**目标**：`verify.sh --clean --all` 真实全绿；CI 与本地同源；README 不再有占位符。

**依赖**：无。**必须先做**，否则后续所有"绿"都不可信。

#### W0.1 让 GUI 壳守卫理解 splash → navigate 的真实启动流程

- 文件：`scripts/check_gui_shell_config.py`
- 位置：`main()` L42-L124；常量区 L36-L39。
- 现状：L59-L65 只接受 `url` 含 `/control/`；实际 `tauri.conf.json:24` 是 `splash.html`，`lib.rs:3319-3369` 在后台探测网关就绪后 `win.navigate(control_url())`，且 `MIN_SPLASH_MS = 6000`（L3355）。
- 改法（逐条）：

  1. 新增常量：
     ```python
     SPLASH_URL = "splash.html"
     LIB_RS = ROOT / "gui" / "src-tauri" / "src" / "lib.rs"
     ```
  2. 把"主窗口 URL"判定抽成纯函数，便于自检：
     ```python
     def check_main_window_url(url: str, lib_src: str) -> list[str]:
         if GATEWAY_CONTROL_PATH in url:
             return []
         if url == SPLASH_URL:
             failures = []
             if "fn control_url()" not in lib_src:
                 failures.append("splash 启动模式要求 lib.rs 定义 control_url()")
             if "win.navigate(" not in lib_src:
                 failures.append("splash 启动模式要求 lib.rs 在就绪后调用 win.navigate(...)")
             if GATEWAY_CONTROL_PATH not in lib_src:
                 failures.append("lib.rs 的 control_url() 必须指向 /control/")
             if "MIN_SPLASH_MS" not in lib_src:
                 failures.append("splash 启动模式要求声明 MIN_SPLASH_MS")
             return failures
         return [f"主窗口 url 必须是 {GATEWAY_CONTROL_PATH!r} 或 {SPLASH_URL!r}，实际 {url!r}"]
     ```
  3. `main()` 里读取 `LIB_RS`（不存在则 FAIL），调用该函数。
  4. 新增 `--self-check` 模式：内置 4 个夹具（`/control/` 通过、`splash.html` + 完整 lib 通过、`splash.html` + 缺 `navigate` 失败、`http://localhost:1420` 失败），任一不符合预期则 exit 1。
  5. 在 `verify.sh:634-637` 的 `check_gui_shell_config()` 里先跑 `python scripts/check_gui_shell_config.py --self-check`，再跑真实检查（两个 `run_check` 或一个 `bash -c` 串联；保持"一条 [PASS]"）。
- 契约：`tauri.conf.json` 允许两种合法形态：
  - 直接 `url` 含 `/control/`；
  - `url = splash.html` 且 `lib.rs` 证明会导航到 `/control/`。
- 测试：`--self-check` 4 例；真实仓库 1 例。
- 验收：`python scripts/check_gui_shell_config.py` exit 0；`--self-check` exit 0；变异（删 `win.navigate`）→ 精确报红。
- 回滚：只改脚本；`git checkout scripts/check_gui_shell_config.py scripts/verify.sh`。

#### W0.2 `verify.sh` 4 处 SKIP 登记

- 文件：`scripts/verify.sh`
- 位置：L423-L452。
- 现状：`check_dependency_audit`（L425/L429 两个分支）、`check_webui_typecheck`（L441）、`check_webui_test`（L450）只 `echo [SKIP]` + `return 0`，不调用 `skip_check`，也不 `SKIPPED+=`；汇总里看不到。
- 改法：
  - L424-L426：
    ```bash
    if ! command -v npm >/dev/null 2>&1; then
      skip_check "webui 依赖审计 (npm audit --audit-level=moderate)" "npm 不可用"
      return 0
    fi
    ```
  - L428-L430 同理，原因写"webui/node_modules 不存在（未安装依赖）"。
  - L440-L442、L449-L451 同理。
- 契约：任何 `[SKIP]` 必须同时进入 `SKIPPED` 数组；汇总 L976-L989 会打印。
- 测试：临时把 `npm` 从 PATH 移除（或 `PATH=/nonexistent`）跑 `--webui`，断言汇总出现 5 个跳过行且不伪装成 PASS（4 项 npm 检查 + `check_tokens_sync` 的 node 缺失）。
- 连带修复（同一条诚实性纪律）：汇总末行原写「全绿 (0 项通过，5 项跳过)。可据此声称完成」——0 项通过时不得叫全绿。现改为「无有效结论 (0 项通过，N 项跳过) —— 跳过项未获验证，不得声称完成」，退出码仍为 0（工具缺失不等于产品失败）。有通过项时才保留「全绿」措辞。
- 验收（可机械判定）：对 `scripts/verify.sh` 逐行扫描，每一个含 `[SKIP]` 的 `echo` 要么位于 `skip_check()` 函数体内（L113），要么其后的 `SKIPPED+=(` 出现在同一 if 分支内；以下 awk 片段输出行号与判定结果：
  ```bash
  awk '/echo .*\[SKIP\]/{ln=NR;body=$0;getline; if ($0 !~ /SKIPPED\+=\(/ && ln!=113) print "UNREGISTERED", ln": "body}' scripts/verify.sh
  ```
  期望输出为空；L329/L370/L876 已各自 `SKIPPED+=` 登记，属正确实现，不得为了让 awk「更干净」而删掉它们。
- 回滚：单文件。

#### W0.3 修 CI `gui` job（改为与 `verify.sh --gui` 同源）

- 文件：`.github/workflows/ci.yml`
- 位置：L257-L293。
- 现状：`Typecheck/Lint/Unit tests/Production bundle` 四步在 `gui/` 下没有对应脚本。
- 改法：
  - 删除 L279-L293 的四步。
  - `working-directory: gui` 保留；新增：
    ```yaml
    - name: Install dependencies
      run: npm ci
    - name: Guard: no frontend source in gui
      run: npm run --silent verify:no-frontend
    - name: Guard: shell architecture invariants
      run: python ../scripts/check_gui_shell_config.py
    ```
  - `Lockfile self-consistency` 保留在最前（不装依赖也能跑）。
  - job `name` 改为 `gui / lockfile + shell guards`。
  - 更新 L240-L256 的注释：说明 React 前端已归档，本 job 只守"纯壳"与 lockfile，Rust 壳编译由 `rust` job 的 workspace build 覆盖。
- 契约：CI 的 gui job 必须与 `verify.sh --gui` 的检查集合一致：no-frontend、shell config、lockfile（shell check/clippy 由 rust job 覆盖，注释里写明）。
- 测试：本地用 `act` 不可行；至少 `python -c "import yaml; yaml.safe_load(...)"` 解析；逐步 `bash -n`；在 PR 里贴出"5 个 job 解析成功"。
- 验收：首次 push 后 gui job 绿；不得再出现 `Missing script`。
- 回滚：单文件。

#### W0.4 README 占位符替换

- 文件：`README.md:11,111`、`README.zh-Hans.md:10,104`
- 改法：`OWNER/REPO` → `KLJyouth1/Marguerite`（已核实 `git remote -v`）。
- 测试：`python scripts/check-readme-claims.py`；新增一条 grep 断言（可由 W0.6 的守卫脚本承担）。
- 验收：`grep -n "OWNER/REPO" README*.md` = 0。

#### W0.5 根级第三方声明

- 新文件：`THIRD_PARTY_NOTICES.md`（仓库根）
- 内容（最小但真实）：
  - 项目自身许可证（读 `LICENSE` 的标题与年份，逐字引用）。
  - 一段"本文件是聚合入口"：`pyengine/THIRD_PARTY_NOTICES.md` 是侧车的完整清单，具有同等效力。
  - Web 控制台直接依赖表：从 `webui/package.json` 的 `dependencies` 逐项列出（名称、版本范围、许可证）。必须人工核对每个包的 LICENSE 字段，不得凭记忆写。至少包含 `vue`、`pinia`、`vue-i18n`、`vue-router`、`three`、`dompurify`、`nanoid`（以实际 `package.json` 为准）。
  - 一段"构建工具不随产物分发"的说明（vite/vitest 等 devDependencies）。
- 测试：`pyengine/tests/test_skills_third_party_notices.py` 不受影响；新增 `scripts/check-third-party-notices.py`（可选，若做则必须有变异验证）。
- 验收：根文件存在且非空；`three` 等直接依赖在表内。
- 回滚：删文件。

#### W0.6 （可选）清理 gui 的无用前端依赖

- 文件：`gui/package.json:13-38`、`gui/package-lock.json`
- 改法：删除 `react`、`react-dom` 及所有 React/ESLint/Vitest/Testing-Library devDependencies；保留 `@tauri-apps/api`、`@tauri-apps/plugin-notification`、`@tauri-apps/cli`。
- 风险：`npm audit` 公告数会变化；`check_gui_lockfile_sync` 必须同步重建 lockfile（`npm install --package-lock-only`）。
- 验收：`cd gui && npm ci --dry-run` 通过；`verify.sh --gui` 四项全绿。
- 回滚：`git checkout gui/package.json gui/package-lock.json`。
- 备注：此项不阻塞发布；若时间紧，推迟到 W12 之后。

#### W0.8 `tauri.conf.json` 的 GBK 乱码清理（执行期补入）

- 文件：`gui/src-tauri/tauri.conf.json` 的 `build.beforeBuildCommand`
- 现状（实测）：值是「UTF-8 字节被当 GBK 解码再写成 UTF-8」的产物，含 U+E06C 等私用区字符与替换符，终端与 CI 日志都会显示为乱码；同一文件其余键均正常。
- 改法：替换为 ASCII 提示 `echo front-end is prebuilt into webui/dist by scripts/package_app.cmd`。不做「还原原文」——原始字节已不可逆。
- 验收：`json.loads` 成功；文件非 ASCII 字符数 = 0；`python scripts/check_gui_shell_config.py` 仍 exit 0。

#### W0.7 闸门总数同步

- 文件：`README.md:239`、`README.zh-Hans.md:216`
- 改法：W10 新增 `check_core_integration_tests` 后，`--all` 从 24 → 25；两份 README 同时改 `24/24` → `25/25`（或实际数）。
- 测试：`python scripts/check-readme-claims.py` 必须 exit 0。
- 验收：脚本输出的 `matches the N checks` 与 README 一致。

**W0 完成条件（全部满足才算完）**：

1. `python scripts/check_gui_shell_config.py` exit 0，`--self-check` exit 0。
2. `verify.sh --gui` 全绿；`--all` 全绿（在 W10 之前是 24 项，W10 之后 25 项）。
3. `grep -n "OWNER/REPO" README*.md` = 0。
4. NPM 缺失场景下 `--webui` 汇总出现 **5** 条跳过（4 项 npm + 令牌同步的 node 缺失），且汇总不得写成「全绿/可据此声称完成」。
5. CI YAML 可解析，gui job 步骤与 `--gui` 对齐。
6. `gui/src-tauri/tauri.conf.json` JSON 合法、无非 ASCII 字符，`check_gui_shell_config.py` 仍 exit 0。

---

### W1 双引擎事件契约修复（P0，预计 1 天）

**目标**：pyengine 的 `session.event.state_change`、`session.event.cron_result`、`session.event.task_group.*` 等原样到达前端 WS，不再被拍平成 `status`。

**依赖**：W0（门禁可信）。

#### W1.1 事件名透传

- 文件：`core/src/gateway/events.rs`
- 位置：`envelope_to_frame` L17-L38。
- 现状：`match env.r#type.as_str()` 只识别 `tool_call/sessions_changed/artifact_updated/desktop_action_done`，其余一律 `session.event.{other}`；pyengine 事件类型是 `status`，于是全部变成 `session.event.status`。
- 改法：

  1. 新增纯函数：
     ```rust
     /// 只接受 pyengine 会话（session 以 "pyengine." 开头）且带 py_event 的信封。
     /// 合法名只有两类：`session.event.<[a-z0-9_.]>` 与顶层 `sessions.changed`。
     /// 任何不合法字符一律拒绝，防止侧车注入任意 WS 事件名。
     ///
     /// 为什么不放行 artifact.updated / desktop.action_done：grep 实测 pyengine
     /// 的发射点里没有这两个名字（它们是 Rust 引擎自己的事件），放行只会扩大
     /// 侧车能伪造的事件面。
     fn pyengine_event_name(env: &Envelope) -> Option<String> {
         if !env.session.starts_with("pyengine.") { return None; }
         let raw = env.payload.get("py_event")?.as_str()?;
         let ok = raw.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()
             || c == '_' || c == '.');
         if !ok { return None; }
         if raw.starts_with("session.event.") { return Some(raw.to_string()); }
         if raw == "sessions.changed" { return Some(raw.to_string()); }
         None
     }
     ```
     ⚠️ 边界：`event_bridge.rs:44-51` 只在 payload 是对象时注入 `py_event`。若侧车发来非对象 payload，本函数返回 `None`，事件仍落到 `session.event.status`（保持现状，不 panic）。
  2. 在 `envelope_to_frame` 的 `let event = match ...` 之前插入：
     ```rust
     if let Some(name) = pyengine_event_name(env) {
         return json!({ ... "event": name, ... });
     }
     ```
     为避免重复构造 JSON，改为先求 `let event = pyengine_event_name(env).unwrap_or_else(|| match ...);`。
  3. 顶层 `sessions_changed` 的现有分支保留（Rust 引擎自身事件）。
- 契约：
  - `py_event` 是 pyengine 网关的原始事件名（`event_bridge.rs:39-52` 已注入）。
  - pyengine 侧已发出带前缀名（`pyengine/src/opensquilla/gateway/event_bridge.py:34`、`boot.py:4118`（`sessions.changed`）、`channel_dispatch.py:1903`（`session.event.run_heartbeat`））。
  - 前端订阅名不变：`webui/src/composables/chat/useChatRpcSubscriptions.ts:74-88`。
- 测试（Rust 单测，放在 `events.rs` 的 `mod tests`）：
  1. `state_change` 信封 → frame.event == `session.event.state_change`。
  2. `cron_result` → `session.event.cron_result`。
  3. `task_group.done` → `session.event.task_group.done`。
  4. 非法名 `session.event.x;drop` → 回退 `session.event.status`。
  5. 非 pyengine 会话 → 不受影响。
  6. `tool_call` 仍映射 `tool_use_start`（现有回归）。
- 变异：把 `pyengine_event_name` 恒返回 `None` → 测试 1-3 必红。
- 验收：`cargo test -p marguerite-core --lib gateway::events` 全绿；真机双引擎下，pyengine 跑一轮对话，WS 观察者能收到 `session.event.state_change`。
- 回滚：删函数与前置分支。

#### W1.2 事件名契约守卫（双侧）

- 新文件：`core/src/gateway/events_contract.rs`（仅测试用）或放在 `events.rs` 的测试模块。
- 内容：读取 `pyengine/src/opensquilla/gateway/event_bridge.py` 与 `boot.py`，提取 `session.event.<name>` 字面量，与 Rust 透传白名单求交集；断言前端 `useChatRpcSubscriptions.ts` 订阅的每个 `session.event.*` 都在 pyengine 的发射集合或 Rust 引擎的发射集合中出现。
- 实现方式：`include_str!` + 正则，不用网络。
- 注意：前端订阅名包含 Rust 原生事件（`steer_accepted` 不是独立事件，它在 status payload 里）；守卫只对"跨引擎必须一致"的集合断言，避免假阳性。建议先在测试里硬编码 6 个关键名（state_change、cron_result、task_group.waiting/synthesizing/done/failed），再逐步扩展。
- 验收：守卫有牙（删 pyengine 的 `state_change` 发射 → 红）。

---

### W2 占位实现清理（P0，预计 2 天）

**目标**：Rust 不再对前端谎报成功；在线时让真实实现（Rust 或 pyengine）接管，离线时诚实降级。

**依赖**：W1（事件契约稳定后再改会话水合）。

#### W2.1 `sessions.messages.snapshot/hydrate/subscribe` 真实化

- 文件：`core/src/gateway/methods.rs`
- 位置：L1664-L1680（三个 handler）；L398-L399（`RUST_FIRST_METHODS`）。
- 现状：固定回空事件/已水合；`RUST_FIRST_METHODS` 保证侧车永不参与。
- 改法：

  1. 在 `core/src/gateway/sessions.rs` 新增：
     ```rust
     /// 从 Rust 会话库读消息并转成前端水合事件。
     /// 返回 (events, current_stream_seq, has_more, source)。
     pub fn message_snapshot(session: &str, limit: usize) -> Value
     ```
     复用 `chat_history` 的 `SessionManager::load_snapshot`（L221-L225）与 `context_entries` 回退。事件形如：
     ```json
     {"seq": 1, "event": "session.event.message", "payload": {"role": "user", "text": "...", "id": "..."}}
     ```
     **必须与 pyengine 的 hydration 事件形状一致**；施工前先用 `pyengine/src/opensquilla/gateway/rpc_sessions.py` 的 snapshot 实现核对字段（禁止自创）。
  2. `sessions.messages.snapshot`：
     - 先取 Rust 快照；`events` 非空 → 返回 `source:"rust"`、真实 `current_stream_seq`（从流状态读取；无流则为 0）。
     - Rust 为空且侧车在线 → 调用 pyengine 的 `sessions.messages.snapshot`（**从 `RUST_FIRST_METHODS` 移除该项**，或在 handler 内显式 `py_call`）。优先"移除 RUST_FIRST + 让通用代理接管"，但那样在线时会完全走侧车；为了双引擎会话合并，建议显式合并：Rust 有则返回 Rust，否则代理。
     - 都不可用 → 回 `{"events": [], "degraded": true, "reason": "no_session_store"}`，**不得**再声称 `canonical_complete: true`。
  3. `sessions.messages.hydrate`：同上；`hydration_complete` 只在真实快照成功时为 true。
  4. `sessions.messages.subscribe`：`current_stream_seq` 从真实流状态读取；无流状态时回 `degraded:true, reason:"rust_stream_state_unavailable"`，不再硬编码 0 并声称 `subscribed:true`。
  5. 从 `RUST_FIRST_METHODS` 移除 `sessions.messages.snapshot`/`hydrate`（L398-L399），改为 `RUST_OWNED_METHODS` 或保留在 dispatch 内但允许代理。**注意**：`RUST_OWNED_METHODS` 会让侧车永不接管，不适合；应放在普通 dispatch，内部按"先本地、空则代理"。
- 契约：前端 `useChatRpcSubscriptions` 期望 `hydration_complete`、`events`、`current_stream_seq`；不得新增必填字段。
- 测试：
  1. Rust 有 3 条消息 → snapshot 返回 3 个事件、`hydration_complete:true`、`source:"rust"`。
  2. Rust 空 + mock pyengine 返回 2 条 → 合并/代理返回 2 条。
  3. 两者都空 → `degraded:true`，`events:[]`，`hydration_complete:false`。
  4. 变异：删掉"空则代理"分支 → 测试 2 红。
- 验收：真机双引擎，pyengine 会话在控制台刷新后能看到历史；Rust 会话也能看到。
- 回滚：保留旧 handler 分支，用配置 `MARGUERITE_RUST_HYDRATION=0` 回退（施工期临时开关，发布前移除）。

#### W2.2 `skills.deps.install`：删除假实现，改为代理/诚实

- 文件：`core/src/gateway/methods.rs:1874-1878`
- 改法：
  - 删除该 handler。
  - 若 `skills.deps.install` 不在 `PY_ENGINE_PREFIXES`，加入（前缀 `skills.` 可能未覆盖，需核实 `is_python_method`）。或直接在 handler 内 `py_call`。
  - 侧车离线 → `RpcError::new("NOT_IMPLEMENTED", "技能依赖安装需要 Python 侧车在线（离线模式下不安装）")`，不得 `{ok:true}`。
- 测试：mock pyengine 在线返回真实结果；离线返回 NOT_IMPLEMENTED；变异：恢复假 handler → 离线测试红。
- 验收：前端点击安装时，在线会真实安装；离线会看到明确错误，而不是"已安装"。

#### W2.3 `sandbox.resume`：同上

- 文件：`core/src/gateway/methods.rs:1870-1873`
- 改法：删除假实现；代理到 pyengine `sandbox.resume`（`rpc_sandbox.py:913` 会清除 denial-pause）；离线回 `NOT_IMPLEMENTED`。
- 测试：在线真实恢复；离线诚实；变异。
- 注意：`sandbox.resume` 在 pyengine 是 `operator.write` + owner-only，Rust 侧不得绕过作用域检查（如果 Rust 的鉴权层没有 owner 概念，如实转发并让侧车裁决）。

#### W2.4 清理 `{ok:true}` 伪装清单

- 全仓扫描 `json!({ "ok": true` 并逐条确认副作用：
  ```bash
  grep -rn '"ok": true' core/src/gateway --include=*.rs
  ```
- 对每个"只返回 ok 不做事"的 handler：删除、代理、或改 `degraded:true` + `reason`。
- 产出一张表（PR 描述）：handler → 副作用 → 处理方式。
- 已知需检查：`cron.*`、`workspaces.*`、`plugins.*`、`agent.*`、`sandbox.*`、`skills.*`。
- 验收：表中无"返回成功但无副作用"的项。

---

### W3 安全边界（P0，预计 2 天）

**目标**：远程命令、系统打开、越界 panic、限流、注入扫描五处达到"默认拒绝/可审计"。

**依赖**：W2（占位清理后，错误语义统一）。

#### W3.1 `collab_ssh_exec` 审批与审计

- 文件：`core/src/gateway/collab.rs:118-175`
- 现状：直接执行任意远程命令；无审批、无策略、无审计条目。
- 改法：

  1. **不要新造审批 API**：`agent/tool_cycle.rs:611` 的 `suspend_for_high_risk(gate, host_present, name, args)` 已经完整实现了「无宿主 fail-closed → `gate.request` → 全局注册表 → `EventType::Approval` → 轮询裁决 → 拒绝/超时错误」这条链路（`tool_cycle.rs:634-685`）。把它的可见性改成 `pub(crate)`（或抽到 `guard/approval_flow.rs` 并让两处共用），`collab.rs` 调用同一个函数：
     ```rust
     let gate = crate::guard::gateway::agent_gate(); // gateway.rs:291，进程级单门
     let host_present = crate::guard::gateway::agent_approval_host_present();
     let args = json!({"host": entry.host, "user": entry.user, "command": command});
     if let Some(blocked) = suspend_for_high_risk(gate, host_present, "collab.ssh.exec", &args).await {
         return Ok(json!({"ok": false, "code": "APPROVAL_DENIED", "error": blocked_message(blocked)}));
     }
     ```
     ⚠️ `suspend_for_high_risk` 的 `args` 是 `HashMap<String,String>`，与 `collab_ssh_exec` 的 `Value` 参数不同 —— 施工时先做一层显式转换，**不要把整段 JSON 塞进一个字段**（审批卡片要逐字段展示）。
  2. `session_key` 从 params 读取；缺失时**不生成假键**，按现有工具循环的语义走"无会话"分支并写审计（避免两条命令共用同一个伪会话）。
  3. 审计：复用 `Recorder` 或 `file_change` 台账，记录主机、命令摘要（**不含密钥**）、退出码、耗时。
  4. 策略：默认所有 `collab.ssh.exec` 都需要审批；提供 `config.collab.ssh.auto_approve`（默认 false）与只读命令白名单（可选，P2）。
- 契约：审批超时/拒绝 → 不执行；返回 `{ok:false, code:"APPROVAL_DENIED"|"APPROVAL_TIMEOUT"}`。
- 测试：
  1. 未批准 → 不执行（mock ssh 不存在也不会 spawn）。
  2. 批准 → 执行。
  3. 审批超时 → fail-closed。
  4. 变异：删审批 → 测试 1 红。
- 验收：真机 SSH 到测试主机，控制台出现审批卡片；拒绝后无连接。

#### W3.2 `open_in_system` 与 `artifacts.open_in_system` 加固

- 文件：`core/src/gateway/_helpers.rs:70-88`、`core/src/gateway/artifacts.rs:132-155`
- 现状：任意路径 + `cmd /c start` + 无扩展名限制；产物无 id 时可传任意 `path`。
- 改法：

  1. 新增 `core/src/gateway/open_path.rs`：
     ```rust
     pub fn open_path_checked(path: &str, roots: &[PathBuf], allow_outside: bool) -> Result<(), OpenError>
     ```
     - `std::fs::canonicalize`；不存在 → `NOT_FOUND`。
     - `crate::tools::fs_path::safe_resolve(root, path)` 对每个 root 检查；全部不通过且 `!allow_outside` → `PATH_ESCAPES_WORKSPACE`。
     - 扩展名黑名单（默认拒绝）：`exe, bat, cmd, com, scr, ps1, vbs, js, jse, wsf, msi, lnk, url, reg`。返回 `EXECUTABLE_BLOCKED`。
     - 路径含控制字符（`\0-\x1f`）→ `INVALID_PATH`。
  2. Windows 实现改用 `ShellExecuteW`（`windows` crate 已在依赖中；若未启用 feature，加 `Win32_UI_Shell`），**不再** `cmd /c start`；非 Windows 保持 `open`/`xdg-open`，但先走同一检查。
  3. `artifacts_open_in_system`：
     - 有 `id` → 台账路径，允许（仍检查存在）。
     - 无 `id` → 只接受"已登记产物路径"或"工作区根内路径"；否则 403 `PATH_NOT_ALLOWED_FOR_OPEN`。
  4. `_helpers::open_in_system` 的调用点全部改为 `open_path_checked`；搜索 `open_in_system(` 确认无遗漏。
- 契约：错误码固定，前端可识别；不得把路径拼进 shell。
- 测试：
  1. 工作区内 `.txt` → 允许（用 mock 替换实际启动，断言参数）。
  2. 工作区外 → 拒绝。
  3. `.exe`/`.bat` → 拒绝。
  4. 未登记 path → 拒绝。
  5. 含 `&`/`|` 的合法文件名 → 不被误判为注入（只拒绝控制字符）。
  6. 变异：去掉扩展名黑名单 → 测试 3 红。
- 验收：前端"在系统中打开"产物仍工作；任意路径探测失败。

#### W3.3 `poker.auto` 边界

- 文件：`core/src/gateway/native.rs:98-135`
- 现状：`let idx = room.current_seat as usize; let p = &room.players[idx];`
- 改法：
  ```rust
  let Some(p) = room.players.get(idx) else {
      return json!({"ok": false, "error": "当前座位越界"});
  };
  ```
  并在循环开始检查 `room.players.is_empty()`；`current_seat` 超出 `u32` 范围时同样返回错误。
- 测试：空房、`current_seat = 99`、正常房；断言不 panic 且锁仍可用（后续调用成功）。
- 变异：恢复索引 → 测试 panic（用 `catch_unwind` 或直接红）。
- 验收：`cargo test` 无 panic；网关进程存活。

#### W3.4 接线限流

- 文件：`core/src/gateway/server.rs:248-296`、`core/src/gateway/rate_limit.rs`
- 现状：模块完整但零引用。
- 改法：

  1. 在 `GatewayState` 里持有 `Arc<RateLimiter>`（或在 server.rs 用 `OnceLock`）。
  2. 在 `build_router` 的 `/api/{*path}` 与 `/ws` 路由上挂 `axum::middleware::from_fn_with_state` 或自定义 `tower::Layer`：
     - 键：客户端 IP（回环场景下用 `ConnectInfo` 或固定 `"loopback"`）+ 方法名（从 JSON-RPC body 读取太重；先按 IP 全局限流）。
     - 生产默认：`capacity=120, refill=20/s`（宽松，不伤正常 UI）；`/api/health`、`/control/*` 静态资源、`/ws` 握手豁免。
     - 超限回 HTTP 429 + JSON-RPC 错误 `RATE_LIMITED`（`rate_limit.rs:41-49` 已有 `json_rpc_error`）。
  3. 配置项 `gateway.rate_limit.enabled`（默认 true）、`capacity`、`refill`；关闭需显式配置并写审计。
- 契约：429 body 与 `RateLimitError::json_rpc_error` 一致；不得影响 liveness（`/api/health` 永远 200）。
- 测试：
  1. 桶限 1：第二次请求 429；`/api/health` 连续 5 次 200。
  2. 限流关闭时不 429。
  3. 变异：摘掉层 → 测试 1 红。
- 验收：真机压测 100 req/s，正常 UI 不受影响；健康检查不受影响。

#### W3.5 注入扫描语义化

- 文件：`core/src/desktop_automation/policy.rs:50-72`
- 现状：对 `action.params.to_string()` 全文匹配 `; && || | $( ` ..\ ../ cmd powershell regsvr32`。
- 问题：① 合法文本（如"比较 cmd 与 powershell 的区别"）被拦；② "点击名为删除的按钮"不拦；③ JSON 转义导致误判。
- 改法：

  1. 只扫描**将被送入命令/终端的字段**：`UiAction.kind == "type"` 且目标控件是终端/地址栏，或 `kind == "run_command"`；其他动作不扫描。
  2. `params` 逐字段检查，不 `to_string()`；对 `text`/`command`/`argv` 做检查。
  3. 新增语义敏感判定：`click` 的目标节点 `name` 命中危险词表（中英）→ 需要确认：
     ```text
     删除, 移除, 卸载, 格式化, 清空, 支付, 转账, 发送, 提交订单, 注销,
     delete, remove, uninstall, format, erase, pay, transfer, send, submit, sign out
     ```
  4. 危险词表放 `policy.rs` 常量并写测试；大小写不敏感、按词边界匹配（避免"发送者"误伤 → 用词表精确短语）。
  5. 保留 `scan_injection` 作为"命令字段"检查，改名 `scan_command_text`，避免语义混淆。
- 测试：
  1. `type` 到普通输入框，文本含 `|` → 不拦。
  2. `type` 到终端，文本含 `rm -rf` → 拦。
  3. `click` 目标 name="删除" → 需要确认。
  4. `click` 目标 name="发送者信息" → 不拦（词边界）。
  5. 变异：删危险词表 → 测试 3 红。
- 验收：真机点击"删除"按钮出现确认；文本编辑不被误拦。

---

### W4 能力路由（P0，预计 4 天）

**目标**：模型在每一轮都能看到"真实能力面 + 何时用哪个 + 禁止用什么替代"，且这套信息**稳定、可测、可观测**。

**依赖**：W3（Policy 语义化后，桌面能力的确认语义才正确）。

#### W4.1 能力卡数据模型

- 新文件：`core/src/agent/capability_cards.rs`
- 设计：

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum CapabilityFamily {
      File, Shell, Browser, DesktopUia, DesktopPixel, Memory, Knowledge, Skill, Mcp, Cron, Team,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum CostClass { Cheap, Medium, Expensive, Interactive }

  pub struct CapabilityCard {
      pub id: &'static str,                 // "desktop.uia"
      pub family: CapabilityFamily,
      pub tools: &'static [&'static str],   // 模型可直接调用的真实工具名（空 = 本轮模型不可调用）
      pub entries: &'static [&'static str], // UI/编排 RPC 入口，仅供说明，不参与路由匹配
      pub triggers: &'static [&'static str],
      pub prerequisites: &'static [&'static str],
      pub fallback: &'static [&'static str],
      pub forbidden_shortcuts: &'static [&'static str],
      pub cost: CostClass,
  }

  pub fn cards() -> &'static [CapabilityCard];
  pub fn render_stable_cards(available: &[ToolDef]) -> String;
  pub fn route(intent: &str, available: &[ToolDef]) -> RouteDecision;
  ```

- 首批卡片（工具名以当前注册表为准，已核实存在）：

  | id | 工具 | 触发 | 前置 | 回退 | 禁止 |
  |---|---|---|---|---|---|
  | `file.edit` | `read_file` `write_file` `edit_file` `apply_patch` `list_dir` `grep` | 读写项目文件、改代码 | 工作区根 | `shell_exec`（仅当无文件工具） | 用 python 脚本读写文件 |
  | `shell.oneshot` | `shell_exec` | 一次性命令、构建、测试 | 工作区 cwd | `code_exec` | 用 shell 启动 GUI 模拟点击 |
  | `browser.a11y` | `browser` | 网页导航、表单、登录、抓取 | 浏览器后端在线 | `web_fetch`（只读） | 用 curl/requests 替代浏览器交互 |
  | `desktop.uia` | `desktop_action` `desktop_launch` | 操作本机应用、窗口、控件 | Windows UIA | `computer_use` 像素 | 用 pyautogui/uiautomation 脚本 |
  | `desktop.pixel` | `computer_use` | UIA 不可用、游戏/画布/远程桌面 | 屏幕权限 | 无 | 无 |
  | `memory` | `memory_search` `memory_save` | 用户偏好、上次约定、跨会话事实 | 记忆已初始化 | 直接问用户 | 把记忆写进代码/脚本 |
  | `knowledge` | `kb_search` `kb_ingest` | 项目文档、知识库资料 | KB 索引可用 | `grep` 文档目录 | 用脚本遍历全盘找资料 |
  | `skill` | `list_skills` `use_skill` | 特定领域流程、可复用方法 | 技能目录 | 直接执行 | 无视技能自己造流程 |
  | `mcp` | 已连接 MCP 工具（`sanitize_mcp_tool_name` 后的动态名，真实存在于 `tools` 数组） | 外部系统集成 | MCP server 在线 | 无 | 用 HTTP 脚本绕过 MCP（`plugins.mcp.call` 是 UI/管理 RPC，**不是模型工具**） |
  | `cron` | `automation`（`action=list\|register\|run\|enable\|disable`） | 定时/重复任务 | 调度器在线 | 提醒用户手动 | 用 sleep 脚本轮询（`cron.*` 是 UI RPC，模型侧用 `automation`） |
  | `team` | **模型侧无工具**（入口是 RPC `team.run` / `subagent`，由编排层或用户触发） | 并行、多专家 | 团队启用 | 单代理 | 串行脚本模拟并行；**禁止在提示词里假装存在 `team` 工具** |

- 规则：卡片只列**当前真实可用**的工具；不可用的卡片以一行"不可用（原因）"呈现，不隐藏（让模型知道"存在但离线"）。

#### W4.2 会话级稳定工具面

- 文件：`core/src/agent/tool_schema.rs`、`core/src/agent/runtime.rs`
- 现状：`ToolDisclosure::new` 每 turn 按请求文本重选（`runtime.rs:785,1101`），`active_tools()` 每次返回不同集合与顺序。
- 改法：

  1. 新增：
     ```rust
     pub struct SessionToolFace {
         available: Vec<ToolDef>,   // 全会话可用（按名字排序）
         visible: HashSet<String>,  // 已暴露
         discovery_enabled: bool,
     }
     impl SessionToolFace {
         pub fn bootstrap(available: Vec<ToolDef>, mode: RuntimeMode) -> Self;
         pub fn active_tools(&self) -> Vec<ToolDef>; // 按名字排序，确定性
         pub fn extend(&mut self, names: &[String]);  // 只增不减
     }
     ```
  2. `bootstrap` 的初始面 = 固定高频面（见 W4.3）+ 与模式无关的安全只读工具；**不再**按本轮 request_text 裁剪。
  3. `ToolDisclosure` 保留用于 `discover()` 查询与 `permits()` 鉴权，但 `active_tools()` 委托给 `SessionToolFace`。
  4. `AgentRuntime` 持有 `Option<SessionToolFace>`（按会话键），首次 turn 创建，后续复用；会话切换时重建。
  5. `is_direct_answer_request` 不再改变工具数组；改为在**动态尾部**插入一句"本轮建议直接作答，除非需要副作用"（见 W5.2）。如果实测发现直答优化对工具调用率影响大，再评估"首轮无工具面"的会话级策略（仍不逐轮变化）。
  6. 顺序：`active_tools()` 返回前按 `name` 排序（`runtime.rs:158` 注释已指出 HashMap 无序会破坏前缀；这里统一到工具数组）。
- 契约：同一会话内，工具数组只允许追加；追加时保持既有顺序，新工具按名字插入到排序位置（会导致一次前缀变化，之后稳定）。
- 测试：
  1. 同一会话连续 5 轮不同用户文本 → 工具名数组逐字节相同。
  2. `discover("web")` 后只追加，不删除。
  3. 会话切换 → 重建。
  4. 变异：恢复"每轮 `ToolDisclosure::new` 后取 active" → 测试 1 红。
- 验收：缓存基准中 `stable_prefix_drift == 0`（W5）。

#### W4.3 固定高频面扩展

- 文件：`core/src/agent/tool_schema.rs:108-116`
- 现状：`read_file, list_dir, str_replace_editor, write_file, web_search, shell_exec, code_exec`。
- 改法（按"用户最常要求、最能减少脚本"排序）：

  ```text
  read_file, list_dir, str_replace_editor, write_file, apply_patch,
  grep, web_search, web_fetch, browser,
  memory_search, kb_search, list_skills, use_skill,
  desktop_launch, desktop_action, computer_use,
  shell_exec, code_exec
  ```
- 注意：
  - 共 **18 项**：`MAX_INITIAL_TOOLS = 12`（`tool_schema.rs:20`）需要提高到 **18**，并加断言 `MAX_INITIAL_TOOLS == FIXED_FACE.len()`，避免两处漂移；同时 `TOOL_SCHEMA_BUDGET_FRACTION`（L18）要重新评估，确保 schema 不超预算。
  - `desktop_launch` 必须与 `desktop_action` 成对出现：在 W6.2 之前，动作仍需要 `desktop_launch` 返回的 handle；只给 `desktop_action` 会让模型拿到一个无法使用的工具。
  - 只加**已注册且模式允许**的工具；不在模式内的仍被过滤。
  - 必须用真实注册表测试：`agent.selfcheck` 的 `tools.names` 包含上述名称（`capability.rs:285-300` 已有类似断言，扩展它）。
- 测试：初始面包含 6 个能力族；`shell_exec` 仍在（不能因反脚本而砍掉正当用途）。
- 验收：能力评测中"该用 memory/kb/browser/desktop 的任务"首轮工具可见率 ≥ 95%。

#### W4.4 反脚本守卫（软门控）

- 新文件：`core/src/agent/capability_guard.rs`
- 目的：当模型用 `shell_exec`/`code_exec` 去模拟一个**已可用**的结构化能力时，返回一次可识别的纠偏错误，而不是静默让它跑。
- 规则（首版，保守）：
  - `shell_exec`/`code_exec` 的命令文本命中：
    - GUI 自动化库：`pyautogui`、`uiautomation`、`pywinauto`、`autoit`、`ahk`、`xdotool`；
    - 浏览器自动化库：`selenium`、`playwright`、`puppeteer`（且 `browser` 工具可用）；
    - 文件读写：`open(`/`Path(`/`cat ` + 写操作（且 `read_file`/`write_file` 可用）。
  - **仅当**对应结构化工具在 `SessionToolFace` 中可见时触发。
  - 动作：返回 `ToolResult::Error { code: "CAPABILITY_MISROUTE", message: "检测到你用脚本替代 <capability>；请改用 <tool>。若确需脚本，请在下一轮明确说明理由并重试。" }`。
  - 同一 turn 内每个 capability 只纠偏一次，避免死循环；第二次放行并记录 `misfire` 指标。
- 配置：`capability_guard = "warn" | "block" | "off"`，默认 `warn`（返回错误但允许模型下一轮重试）；首版不建议 `block` 硬拒。
- 测试：
  1. `shell_exec("python -c 'import pyautogui; ...'")` + `desktop_action` 可见 → 返回 `CAPABILITY_MISROUTE`。
  2. 同样的脚本但 `desktop_action` 不可见 → 放行。
  3. 正当脚本（`pytest`、`cargo test`）→ 放行。
  4. 同一 turn 第二次 → 放行 + 计数。
  5. 变异：删规则 → 测试 1 红。
- 验收：真机跑"帮我点一下记事本的保存" → 模型首选 `desktop_action`；若先写脚本，收到一次纠偏后改用工具。

#### W4.5 能力自述注入（把 `brief()` 接上）

- 文件：`core/src/agent/capability.rs:236-256`、`core/src/agent/runtime.rs:461-565`
- 现状：`brief()` 从未调用。
- 改法：
  1. `brief()` 改为纯函数输入：`brief_from(names: &[String], skills: usize, mcp: usize) -> String`；`brief()` 仅作为方便包装。
  2. 在 `SessionToolFace` 创建时生成一次 `capability_brief`，作为稳定半区的一部分注入（见 W5.2 的布局）。
  3. 内容必须**确定性**：工具名排序、计数来自真实注册表/配置；同一会话不重复计算。
  4. 不再写"不确定时调 `agent.selfcheck`"（自检是 RPC，模型在对话中不一定能调）；改为"以下是你此刻真实可用的能力"。
- 测试：两次生成逐字节相同；工具增减时只在该会话下一次创建时变化。
- 验收：问"你能操控浏览器/桌面/记忆吗"时，模型能准确回答并给出工具名。

#### W4.6 能力探测 RPC（供 UI 与模型）

- 文件：`core/src/gateway/methods.rs` 新增 `capabilities.snapshot`（或复用 `agent.selfcheck`）。
- 内容：`{families:[{id,available,tools,prerequisites,reason}], generated_at}`。
- 注意：`generated_at` 不得进入提示词（破坏前缀）；只用于 UI。
- 测试：离线时 `browser` 不可用、`desktop.uia` 在非 Windows 不可用；不得虚报。
- 验收：控制台"能力中枢"显示真实状态。

#### W4.7 能力评测集（Capability Bench 60）

- 新目录：`scripts/bench/capability/`
  - `tasks.jsonl`：60 条，字段 `{id, prompt, expected_family, allowed_tools, forbidden_tools, workspace_fixture}`。
  - 覆盖：文件 10、shell 5、browser 10、desktop 10、memory 5、kb 5、skill 5、mcp 5、cron 3、team 2。
  - 每条必须有"期望首选能力族"与"禁止的脚本替代"。
- 运行方式：
  - **离线部分**：`cargo test -p marguerite-core capability_bench_routing` 只测 `route()` 的确定性（60 条全部命中期望族）。
  - **在线部分**（手动，不进 CI）：用真实/本地 mock 模型跑 60 条，记录首选工具、是否触发纠偏、是否完成；输出 `artifacts/capability-bench-<date>.json`。
- 验收（首版目标，不承诺一步到位）：
  - 路由纯函数：60/60 命中期望族。
  - 在线：结构化能力首选率 ≥ 80%（baseline 未知，先测再定）；脚本替代率 ≤ 10%。
- 说明：这不是"模型能力上限"的绝对证明，只是回归基线；结果必须如实记录失败项。

---

### W5 缓存命中（P0，预计 3 天）

**目标**：先把指标改诚实，再把可缓存前缀做稳定，最后用受控基准证明"稳态请求命中率接近 100%"。

**依赖**：W4.2（会话级稳定工具面）是前提；没有它，任何缓存优化都会被每轮变化的 `tools` 数组抵消。

#### W5.1 指标口径重定义

- 文件：`core/src/cache/mod.rs:528-595`、`core/src/gateway/cache.rs:73-110`、`core/src/model/llm.rs:1625/1892/2291/2404`
- 现状（两处口径错误，第二处更隐蔽）：
  1. `hit_rate = read/(read+creation)`；DeepSeek 的 `creation` 通常为 0 → 有命中即 100%。
  2. `prompt_tokens_total` 的口径**因 provider 而异**：OpenAI/DeepSeek 的 `prompt_tokens` 已含缓存命中（`cached_tokens` 是子集），而 Anthropic 的 `input_tokens` **不含** `cache_read_input_tokens` / `cache_creation_input_tokens`（`llm.rs:2291-2295` 直接 `prompt_tokens: u.input_tokens`）。若新指标直接拿 `prompt_tokens_total` 做分母，Anthropic 的覆盖率会被算大（分母偏小）。

- **W5.1a 先修总量口径**（不做这一步，后面的百分比全是错的）：
  1. 在 `core/src/model/llm.rs` 新增纯函数（放 `TokenUsage` 附近，便于单测）：
     ```rust
     fn total_input_tokens(prompt_tokens: u64, cache_read: u64, cache_creation: u64, anthropic: bool) -> u64 {
         if anthropic { prompt_tokens + cache_read + cache_creation } else { prompt_tokens }
     }
     ```
  2. 四个 `report_prefix_cache_usage(...)` 调用点（`llm.rs:1625`、`1892`、`2291`、`2404`）的第一个参数改为 `total_input_tokens(...)`；两个流式点（1625/1892）用外层的 `anthropic` 布尔（与 `parse_anthropic_stream_chunk` 分支同一个变量），两个非流式点按各自 provider 分支传常量 `true`/`false`。
  3. `cache/mod.rs:503` 的字段注释改为"provider 归一后的总输入 token（Anthropic = input + read + creation；OpenAI/DeepSeek = prompt_tokens）"。
  4. 同步检查 `TokenUsage::prompt_tokens` 的消费者（`usage` 台账、`gateway/cache.rs` 的 `prompt_tokens_total`）——**不要**改 `TokenUsage` 本身的语义（它是 provider 原始值），只改追踪器的入参。

- 新指标（全部来自真实计数）：

  | 指标 | 公式 | 含义 |
  |---|---|---|
  | `cache_coverage_pct` | `cache_read_total / total_input_tokens * 100` | 所有输入 token 中，有多少被服务端缓存复用 |
  | `request_hit_rate_pct` | `hit_requests / eligible_requests * 100` | 合格请求中，有多少至少命中一次 |
  | `write_adjusted_hit_rate_pct` | `read / (read + creation) * 100` | 旧指标，保留为诊断，不再作为 UI 主指标 |

  - 不再设 `saved_token_ratio_pct`：它在语义上等于 coverage，而原拟分母 `prompt+read` 在 OpenAI/DeepSeek 上会重复计入命中 token —— **宁可少一个指标，也不要一个会骗人的指标**。成本视角若要做，应单独按 provider 价格表计算，不属于本轮。
  - `eligible_requests`：`total_input_tokens >= provider_min_cache_tokens`（DeepSeek 1024；Anthropic 1024/2048 视模型；OpenAI 1024）。阈值放 `cache/mod.rs` 常量，按 provider 选择；判定用归一后的总量。
  - 新增 `hit_requests` 计数：`report()` 时 `if cache_read > 0 { hit_requests += 1 }`；`eligible` 同理按阈值判定。
  - `prompt_tokens_total` 已在 `PrefixCacheTrackerInner` 累计（L534），只是没暴露；改名或加别名 `total_input_tokens` 均可，但**字段语义必须在注释里写死**。
- 改法：
  1. `PrefixCacheTrackerInner` 加 `hit_requests`、`eligible_requests`；`report()` 增加 `eligible: bool` 参数（或传 `provider_min` 在内部判定）。
  2. `PrefixCacheTrackerStats` 加字段。
  3. `cache.status` 返回：`total_input_tokens`、`cache_read_tokens`、`cache_creation_tokens`、`cache_coverage_pct`、`eligible_requests`、`hit_requests`、`request_hit_rate_pct`、`write_adjusted_hit_rate_pct`。
  4. `server_cache_hit_rate` 保留为 `cache_coverage_pct` 的别名（兼容前端），并在注释里写明"不是 read/(read+creation)"。
- 测试：
  1. OpenAI/DeepSeek：`prompt=1000（含 900 cached）, read=900, creation=0` → total=1000，coverage 90，write-adjusted 100（证明旧指标会骗人）。
  2. Anthropic：`input=100, read=900, creation=0` → total=1000，coverage 90（若漏加 read+creation 会得 900%，必须红）。
  3. Anthropic 冷启动：`input=100, read=0, creation=900` → total=1000，coverage 0。
  4. `eligible=10, hit=9` → 90%；`read=0` 的请求不计入 `hit_requests`。
  5. 变异：① 把 coverage 改回 `read/(read+creation)` → 测试 1 的 coverage 断言红；② 删掉 Anthropic 的 `+ read + creation` → 测试 2 红。
- 验收：控制台不再出现"命中率 100%"而覆盖率很低的矛盾；Anthropic 与 DeepSeek 的 coverage 可在同一把尺子上比较。

#### W5.2 提示词布局：稳定半区 / 动态尾部

- 文件：`core/src/agent/runtime.rs:461-565`（`build_model_messages`）、`132-227`（`tool_guide_text`/`compose_guide`）、`core/src/agent/tool_catalog.rs:317-384`
- 现状：
  - 稳定半区：mode → reasoning → workspace → 用户规则 → **tool_guide（含 `{n}` 与 names）** → 插话 → history。
  - 动态尾部：`stable_prefix` → `experience_hint` → `route_decision`（在 history 之后）。
- 目标布局：

  ```text
  [稳定]
  1. mode section
  2. reasoning hint
  3. workspace guidance
  4. 用户规则
  5. 能力卡（静态文本，来自 SessionToolFace 的首次快照）
  6. 工具指引（静态：只讲规则，不讲数量/名单）
  7. 技能目录（按目录快照，会话内不变；变化时算 drift）
  8. history（只追加）

  [动态，history 之后]
  9. 本轮增量（新增工具名、直答建议、纠偏提示）
 10. experience_hint
 11. route_decision（能力路由的决定与理由）
  ```

- 改法：
  1. `tool_guide_text`（runtime.rs:132-213）删除 `"本轮可调用工具共 {n} 个：{names}。"`；改为静态规则文本，工具清单只通过 function schema 呈现。
  2. 新增 `compose_turn_delta(turn, newly_visible: &[String], direct_answer: bool) -> Option<String>`，把"新增工具/直答建议"放动态尾部。
  3. `compose_guide` 拆成 `compose_stable_guide(cards, catalog)` 与 `compose_turn_delta(...)`。
  4. `tool_catalog::catalog_text` 的 B 段改为**会话级快照**（`SessionToolFace` 的初始面 + 已发现面），而不是每轮 `active_tools`；如果工具面未变，输出逐字节相同。
  5. `build_model_messages` 参数从 `tool_guide: Option<&str>` 改为 `GuideParts { stable: Option<&str>, turn: Option<&str> }`；动态部分在 history 之后追加。
  6. `compact_window` 仍然只在超窗时生效（`runtime.rs:560-563`），不改变稳定半区。
  7. 所有新增字符串在代码里用注释标注 `// stable:` 或 `// turn:`（R-08）。
- 测试：
  1. 同一会话两轮（工具面不变）→ 稳定半区逐字节相同；`turn_delta` 可以不同。
  2. 新增一个工具 → 稳定半区变化一次（drift+1），下一轮恢复稳定。
  3. 不同用户文本 → 稳定半区不变。
  4. 变异：把 `{n}` 加回 → 测试 1 红。
- 验收：W5.4 的 drift == 0（稳态）。

#### W5.3 完整前缀指纹与漂移监测

- 文件：`core/src/agent/runtime.rs`（`build_model_messages` 末尾）、`core/src/cache/mod.rs:528-595`、`core/src/agent/tool_compress.rs:217`
- 改法：
  1. 在 `build_model_messages` 组装稳定半区后，序列化稳定半区（`serde_json::to_string` 或逐个 content 拼接），调用：
     ```rust
     crate::cache::prefix_cache_tracker().observe_stable_prefix_scoped("agent.stable_half", &text);
     ```
  2. `observe_stable_prefix` 增加 `scope` 参数（`agent.stable_half`、`compress.tool_results`），分别计数；`stable_prefix_drift` 返回总和，`drift_by_scope` 返回明细。
  3. `cache.status` 暴露 `stable_prefix_drift_by_scope`。
  4. `tools.patch` / MCP 热插拔 / 技能目录变化时，显式记录一个 `face_changed` 事件（可只写日志），让漂移有归因。
- 测试：
  1. 稳态 5 次 → drift 0。
  2. 改一个工具名 → drift 1。
  3. `compress` 与 `agent` 两个 scope 独立计数。
- 验收：控制台能看到"漂移发生在哪一层"。

#### W5.4 Anthropic `cache_control` 断点

- 文件：`core/src/model/llm.rs:824-838`（`messages_to_anthropic`）、`983-1007`（body 构造）
- 现状：`system` 是单个字符串；`tools` 无 `cache_control`；Anthropic 前缀缓存结构性不可用。
- 改法：

  1. `messages_to_anthropic` 返回：
     ```rust
     fn messages_to_anthropic(&self, messages: &[Message]) -> (Vec<serde_json::Value>, Vec<serde_json::Value>)
     ```
     System 消息合并为 **一个 block array**（多个 System 用 `\n` 合并为一个 text block，避免多个 block 的语义差异）：
     ```json
     [{"type":"text","text":"...","cache_control":{"type":"ephemeral"}}]
     ```
  2. body：
     ```json
     "system": [{"type":"text","text":"...","cache_control":{"type":"ephemeral"}}]
     ```
  3. `tools`：最后一个工具加 `"cache_control": {"type": "ephemeral"}`（Anthropic 的断点覆盖到该位置）。
  4. 最后一条 user 消息的 content 改为 block array，并在最后一个 text block 加 `cache_control`（第 3 个断点；Anthropic 上限 4）。
  5. 仅在 `self.is_anthropic()` 分支做；OpenAI/DeepSeek 保持不变（自动前缀缓存）。
  6. 如果 `messages` 过长（超过模型上下文），断点仍放在最后一条 user；不要放在 history 中间。
- 测试：
  1. 单测：Anthropic body 的 `system[0].cache_control.type == "ephemeral"`。
  2. 单测：`tools.last().cache_control` 存在。
  3. 单测：最后一条 user 的 content 是数组且末块有断点。
  4. 回归：非 Anthropic provider 的 body 不含 `cache_control`。
  5. 变异：删断点 → 测试 1 红。
- 验收：Anthropic 真实调用第 2 轮 `cache_read_input_tokens > 0`（需要 API key；无 key 时只做单测，并在 PR 里注明未做真机）。

#### W5.5 受控基准（Cache Bench）

- 新文件：`scripts/bench/cache/bench_prefix_cache.py`
- 目的：把"100% 缓存命中"变成可复现的实验，而不是口号。
- 设计：
  - 启动隔离网关（`MARGUERITE_CONFIG_DIR` + `LOCALAPPDATA` 指向临时目录；注意 Windows 上 `USERPROFILE` 无效，见 AGENTS §4）。
  - 固定：同一模型、同一会话、同一工具面、无压缩、无 MCP 变更。
  - 12 轮固定对话（每轮一句短问），每轮读 `cache.status`：
    - `total_input_tokens`（provider 归一）、`cache_read_tokens`、`cache_creation_tokens`、`cache_coverage_pct`、`request_hit_rate_pct`、`stable_prefix_drift`。
  - 输出 JSON + Markdown 表。
- 目标（DeepSeek，12 轮）：
  - 第 1 轮：`cache_read=0`（冷启动，允许）。
  - 第 2-12 轮：每轮 `cache_read > 0`；`request_hit_rate_pct == 100%`（11/11）。
  - `cache_coverage_pct` ≥ 85%（保守；实际应更高，因为历史前缀被复用；以首测为准再定）。
  - `stable_prefix_drift == 0`。
- 目标（Anthropic）：
  - 第 2 轮起 `cache_read_input_tokens > 0`；`cache_creation` 在第 1 轮 >0。
- 失败判定：
  - 任一轮 `cache_read=0`（非首轮）→ 打印该轮的 stable half 哈希与工具名列表，定位漂移。
  - `drift > 0` → 打印 scope 明细。
- 说明：该脚本**不进 `verify.sh`**（需要真实 API key 与费用）；它是发布前的必跑证据，结果存 `artifacts/cache-bench-<date>.{json,md}`。

#### W5.6 前端展示

- 文件：
  - `webui/src/components/island/game/useEngineBridge.ts:133-136`
  - `webui/src/components/chat/...CacheStrip`（施工前用 `grep -rn "server_cache_hit_rate" webui/src` 定位）
- 改法：
  1. `next.cacheHitRate` 拆成：
     ```ts
     next.cacheCoveragePct = cache?.cache_coverage_pct ?? null
     next.cacheRequestHitRatePct = cache?.request_hit_rate_pct ?? null
     next.clientHitRatePct = cache?.client_hit_rate ?? null
     ```
  2. 删除 `Math.max(...)`。
  3. CacheStrip 显示两个标签：
     - "服务端前缀覆盖"（coverage）
     - "合格请求命中"（request hit rate）
     - tooltip 写明分子/分母。
  4. 若后端旧版本没有新字段，回退到 `server_cache_hit_rate` 并加"旧口径"标记。
- 测试：`useEngineBridge` 单测（新增）覆盖三种情况：新字段、旧字段、全空。
- 验收：控制台不再把本地与服务端混成一个数；数字与 `cache.status` 一致。

---

### W6 电脑操控（P0，预计 5 天）

**目标**：默认走结构化 UIA，像素作为兜底；每次动作都有 observe→act→verify 回执；审批按语义而非字符串。

**依赖**：W3.5（Policy 语义化）、W4（能力卡与稳定工具面）。

#### W6.1 扩展 UIA 动作矩阵

- 文件：`core/src/desktop_automation/uia_driver.rs:160-187`、`core/src/desktop_automation/windows_uia.rs`
- 现状：只支持 `snapshot`、`click`；`windows_uia.rs` 已有 Invoke/Toggle/SelectionItem pattern 与超时。
- 新增动作（`UiAction.kind`）：

  | kind | params | UIA pattern / 实现 | 回执 |
  |---|---|---|---|
  | `find` | `role?`, `name?`, `automation_id?`, `timeout_ms?` | 遍历树，返回匹配节点（最多 20） | nodes |
  | `invoke` | `ref_id` 或 `name` | InvokePattern | 节点摘要 |
  | `set_value` | `ref_id`/`name`, `value` | ValuePattern.SetValue | 前后 value |
  | `type` | `ref_id`/`name`, `text`, `clear?` | ValuePattern 或 SendKeys 到焦点 | 前后 value |
  | `toggle` | `ref_id`/`name` | TogglePattern.Toggle | 前后 state |
  | `select` | `ref_id`/`name` | SelectionItemPattern.Select | 选中项 |
  | `expand`/`collapse` | `ref_id`/`name` | ExpandCollapsePattern | 前后 state |
  | `scroll` | `ref_id`/`name`, `horizontal?`, `vertical?`, `amount` | ScrollPattern | 前后 scroll percent |
  | `focus` | `ref_id`/`name` | SetFocus | 焦点节点 |
  | `wait` | `ref_id`/`name`, `state`, `timeout_ms` | 轮询属性 | 命中/超时 |
  | `window_activate` | `title` 或 `handle` | SetForegroundWindow | 窗口句柄 |
  | `snapshot` | `window?`, `max_depth?` | 现有 | nodes |

- 实现要点：
  - `windows_uia.rs` 需要一个 `pattern` 分发函数；`GetCurrentPatternAs` 在 L279-295 已有示例。
  - 每个动作都必须有 10s 超时（`UIA_CALL_TIMEOUT`）与节点上限（`MAX_NODES`），不得无限遍历。
  - 找不到节点 → `DesktopError::NodeNotFound`，不 panic。
  - 写操作（`set_value`/`type`/`invoke`）必须返回前后值，作为 verify 依据。
- 测试：
  - 单元：用 mock `A11ySource` 实现 pattern 分发。
  - 真机：记事本（输入、保存对话框、取消）、计算器（按钮 invoke）、浏览器地址栏（set_value）。
- 验收：真机 6 个动作全部成功；失败返回明确错误。

#### W6.2 附着已有前台应用（不再必须 `desktop_launch`）

- 文件：`core/src/tools/computer_use.rs:1804-1880`（`desktop_action` 工具）、`core/src/desktop_automation/`（AppRegistry）
- 现状：`desktop_action` 必须传 `handle`；`desktop_launch` 才产生句柄。
- 改法：
  1. `desktop_action` 的 `handle` 改为可选；缺省时：
     - 若 params 带 `window`（标题/进程名）→ 附着该窗口；
     - 否则附着当前前台窗口。
  2. 新增 `desktop_attach` 工具（或 `desktop_action` 的 `action.kind="attach"`），返回可复用 handle。
  3. `desktop_launch` 仍保留，用于启动新应用。
  4. 工具描述明确："操作已打开的应用用 `desktop_action` + `window`；启动新应用用 `desktop_launch`"。
- 测试：不传 handle、传 window 标题 → 成功；空标题 → `WINDOW_NOT_FOUND`。
- 验收：用户说"点一下我正开着的记事本"时，模型不需要先 launch。

#### W6.3 observe→act→verify 回执

- 文件：`core/src/desktop_automation/driver.rs:127-135`（`ActionReceipt`）、`uia_driver.rs:165-187`、`recorder.rs:16-50`
- 改法：

  1. `ActionReceipt` 增加：
     ```rust
     #[serde(skip_serializing_if = "Option::is_none")]
     pub before: Option<Value>,   // {node_digest, value, enabled, focused}
     #[serde(skip_serializing_if = "Option::is_none")]
     pub after: Option<Value>,
     pub verified: bool,
     #[serde(skip_serializing_if = "Option::is_none")]
     pub effect: Option<String>,  // "value_changed" | "node_removed" | "state_changed" | "no_observable_change"
     ```
  2. 每个写动作：
     - act 前抓目标节点（或窗口）摘要 → `before`。
     - act 后重新抓 → `after`；比较生成 `effect` 与 `verified`。
     - `verified=false` 时不改变 `ok`（动作可能成功但无可观测变化），但回执明确写出。
  3. `Recorder::record_step` 接收真实的 before/after JSON，写入审计快照（`recorder.rs` 当前是 None）。
  4. `snapshot` 类只读动作仍不落审计快照（避免刷满目录），但 `before/after` 可为 None。
- 测试：
  - mock source：set_value 后值变化 → `effect=value_changed, verified=true`。
  - mock source：invoke 后节点消失 → `node_removed`。
  - mock source：无变化 → `no_observable_change`。
  - 审计文件存在且包含 before/after。
- 验收：真机每次点击/输入后，回执能说明"发生了什么"。

#### W6.4 Policy 语义化（与 W3.5 同一文件，施工时合并）

见 W3.5。额外：`desktop_action` 的 `approved` 参数保留，但只在 `Policy::require_confirmation` 返回 true 时要求；前端审批卡片走 `guard::approval_registry`（与 W3.1 同一条链路），不要另造一套。

#### W6.5 桌面侧 RPC 对齐浏览器

- 文件：`core/src/gateway/methods.rs`（新增 `desktop.a11y`、`desktop.action`）
- 契约（参考 `browser.a11y`）：
  - `desktop.a11y {window?, maxDepth?}` → `{window, nodes:[{ref_id, role, name, automation_id, bounds, enabled, patterns}]}`
  - `desktop.action {kind, params}` → `ActionReceipt`
  - `desktop.windows` → 窗口列表（现有 window_list 可复用）
- 作用：前端"电脑操控"面板可以展示可点击控件，用户可手动驱动；模型也能通过 RPC 做同样的事。
- 测试：RPC 单测 + 真机。
- 验收：控制台能列出前台窗口控件并点击。

#### W6.6 文档与陈旧注释

- `core/src/desktop_automation/a11y_tree.rs:1-27` 文件头改为"UIA 实现在 windows_uia.rs；本文件只定义跨平台抽象"。
- `core/src/tools/computer_use.rs` 的工具描述加一句"结构化优先：能用 desktop_action 就不要用像素坐标"。

#### W6.7 真机验收场景（8 条，必须逐条录像/截图）

1. 打开记事本 → 输入一段中文 → 保存到工作区 → 读回文件内容一致。
2. 打开计算器 → 点击 1+2= → 读结果 3。
3. 浏览器地址栏输入 URL → 回车 → 页面标题读取。
4. 文件资源管理器 → 选中文件 → 重命名（确认弹窗走审批）。
5. WPS/Office（若安装）→ 生成演示文稿（现有 `generate_presentation` 路径）。
6. 打开设置 → 切换一个开关 → 回执 `state_changed`。
7. 点击"删除"按钮 → 出现审批 → 拒绝 → 无副作用。
8. UIA 不可用的画布应用 → 自动降级 `computer_use` 像素 → 成功点击。

每条记录：动作序列、回执 JSON、审计快照路径、耗时、失败原因（如有）。

---

### W7 ACL 补齐（P1，预计 0.5 天）

**目标**：release 安装版里桌宠的 show/hide/toggle 与 pet-overlay 窗口真的可用。

**依赖**：无。

- 文件：
  - `gui/src-tauri/build.rs:278-290`
  - `gui/src-tauri/capabilities/default.json`
- 改法：
  1. `AppManifest::commands` 增加 `"pet_overlay_show"`、`"pet_overlay_toggle"`（`hide` 已有）。
  2. `capabilities/default.json`：
     - `windows` 增加 `"pet-overlay"`（窗口 label 必须与 `lib.rs` 创建时一致；施工前 `grep -n "pet-overlay" gui/src-tauri/src/lib.rs` 核实）。
     - `permissions` 增加 `"allow-pet-overlay-show"`、`"allow-pet-overlay-toggle"`。
  3. 保持 `remote.urls` 只含回环。
- 测试：
  - `cargo build -p marguerite-gui` 通过（build.rs 会校验 ACL）。
  - 安装版：Tauri 前端调用 `invoke('pet_overlay_show')` 成功（控制台无 ACL 拒绝）。
- 验收：release 模式下桌宠显示/隐藏/切换三条命令都成功；`capabilities` 无多余权限。
- 回滚：两文件。

---

### W8 前端体验（P1，预计 2 天）

**目标**：不再"选已打开工作区还要弹文件夹"；命令面板能切换工作区/模型/团队；静默失败可见。

**依赖**：W9（cron 契约）不阻塞本包，但 W8.4 的 "最近" 会复用 `workspaces.list` 的新字段（W8.1 后端）。

#### W8.1 工作区最近列表

- 后端：`core/src/gateway/workspaces.rs`
  - `WorkspaceEntry` 增加 `#[serde(default)] pub last_opened_at: String`（RFC3339）。
  - `open()`（L145-L175）：每次打开更新 `last_opened_at = Utc::now().to_rfc3339()`；幂等匹配时也更新。
  - `list()`（L136-L140）：排序规则 `pinned desc, last_opened_at desc, created_at desc`；返回前 50 条。
  - `view()` 增加 `lastOpenedAt`/`last_opened_at` 两个别名（前端两种命名都在用，见 `useProjectWorkspaces.ts:66-70`）。
  - 迁移：旧 `workspaces.json` 没有该字段 → `serde(default)` 为空；排序时空值排最后。
- 前端：`webui/src/components/ProjectWorkspacePickerDialog.vue:530-542`
  - 单选模式不再直接 `runNativePicker`；改为：
    1. 打开即 `phase='web-ready'`，加载 `workspaces.list`。
    2. 顶部显示"最近打开"（按 `lastOpenedAt`），默认选中第一条；用户回车即确认。
    3. 列表底部有"浏览文件夹…"按钮 → `runNativePicker`。
    4. 仅当列表为空（首次使用）时，自动弹原生选择器一次（保留首次体验）。
  - 多选模式保持不变。
  - 新增 `recent` 标记与置顶星标（复用 `pinned`）。
- 测试：
  - Rust：打开两次不同工作区 → list 顺序正确；pinned 永远在前。
  - Vue：单测 mock `workspaces.list` 返回 3 条 → 不调用 `runNativePicker`；空列表 → 调用一次。
  - e2e：打开对话框 → 直接选中最近工作区 → 确认。
- 验收：用户不再需要重复浏览文件夹；首次使用仍友好。
- 回滚：前后端各一个提交。

#### W8.2 命令面板补齐

- 文件：`webui/src/components/CommandPalette.vue`
- 现状：已有 `nav:`、`session:`、`message:`、`recent:`、`mode:`、`action:`。
- 新增：
  1. **工作区** `workspace:<id>`：列出 `workspaces.list`，动作 `workspaces.open` + 切换到该工作区（复用现有 workspace store/composable）。
  2. **模型/服务商** `model:<provider>:<model>`：数据源用现有 `models.list` / `model.status` RPC（施工前核实真实方法名），动作走现有切换 RPC。
  3. **团队/专家** `team:<id>` / `agent:<id>`：数据源 `team.list` / `agents.list`（施工前核实），动作复用现有选择逻辑。
  4. 搜索别名：`ws`、`工作区`、`model`、`模型`、`team`、`团队`。
  5. 键盘：`↑/↓` 选择、`Enter` 执行、`Esc` 关闭；与现有实现一致。
- 测试：新增 `CommandPalette` 单测：每类条目能被搜到并触发对应动作（mock RPC）。
- 验收：用户在任意页面按快捷键即可切换工作区/模型/团队，不需要先导航到设置页。

#### W8.3 静默失败清理（8 处）

- 文件与位置（已核实，路径逐一 `test -f` 通过）：
  - `webui/src/App.vue:1068`
  - `webui/src/components/workbench/terminalWorkbenchProvider.ts:316,343`
  - `webui/src/composables/chat/useOrchestrationRuns.ts:109`
  - `webui/src/composables/usePendingRestart.ts:46`
  - `webui/src/utils/chat/metaDiscardOutbox.ts:54`
  - `webui/src/utils/chat/sessionNavigationDiag.ts:49`
  - `webui/src/views/TerminalView.vue:163`
- 改法：
  - 能恢复的：`catch (e) { pushToast({ kind: 'warn', text: ... }); return [] }`。
  - 诊断类（`sessionNavigationDiag`）：保留静默，但 `console.warn` + 采样上报（避免刷屏）。
  - 终端类：必须提示"终端状态读取失败，已重试/请手动刷新"。
- 测试：每处一个单测，断言错误时 toast 被调用（诊断类断言 console.warn）。
- 验收：断网/网关离线时，用户能看到原因，而不是空白列表。

#### W8.4 工作区选择器与命令面板的联动

- 在 CommandPalette 的 `workspace:` 项里显示最近 5 个；选中后直接切换，不打开对话框。
- 在 Chat 页的空状态增加"最近工作区"快捷入口（复用同一数据源）。

---

### W9 cron 契约对齐（P1，预计 1 天）

**目标**：控制台的定时任务列表在 Rust 托管、pyengine 托管、离线三种情况下都能正确显示。

**依赖**：W2（占位清理）。

#### W9.1 统一 wire shape

- 权威：pyengine 的 `_job_to_wire`（`pyengine/src/opensquilla/gateway/rpc_cron.py:562`，返回数组）。
- Rust 改法（`core/src/gateway/methods.rs:1723-1748`）：
  1. **优先代理**：侧车在线时，`cron.*` 直接转发 pyengine（加入 `PY_ENGINE_PREFIXES` 的 `cron.` 前缀，或 handler 内 `py_call`）。Rust 的 `ScheduleStore` 只作为离线兜底。
  2. 离线兜底返回**数组**，每条映射为前端可读的 wire：
     ```json
     {
       "id": "...",
       "name": "...",
       "enabled": true,
       "next_run": "2026-10-26T12:00:00Z",
       "last_run": null,
       "payloadKind": "reminder",
       "message": "...",
       "scheduleKind": "every",
       "scheduleRaw": "3600",
       "source": "rust_schedule_store"
     }
     ```
     - `next_fire_ms` → RFC3339 `next_run`；`created_at_ms` → `created_at`（可选）。
     - 保留原始 ms 字段（兼容旧前端），但主字段必须是 ISO。
  3. `cron.create/update/delete/pause/resume`：同样优先代理；离线时若 Rust 的 `ScheduleStore` 能表达（reminder/after/at/every），执行并返回 wire；不能表达则 `NOT_IMPLEMENTED`。
- 前端改法（`webui/src/composables/cron/useCronJobs.ts:10-35`）：
  - `jobs` 归一化接受三种形状：数组、`{jobs}`、`{items}`。
  - 时间归一化：`next_run` 优先；缺失时用 `next_fire_ms` 转 ISO。
  - `payloadKind`/`payload_kind` 都接受（已有）。
- 测试：
  1. Rust 离线返回数组 → 前端显示 2 条。
  2. pyengine 在线数组 → 显示同样 2 条。
  3. 混合形状 → 不丢。
  4. 变异：把 Rust 改回 `{items}` 且前端只认 `{jobs}` → 测试 1 红。
- 验收：真机三种模式下列表一致；新建任务后下一轮刷新可见。

#### W9.2 `cron.result` 事件

- W1 修好后，pyengine 的 `session.event.cron_result` 能到前端；确认 `useChatRpcSubscriptions.ts:79` 的 handler 被触发（现有 `handleRpcCronResult`）。
- 测试：mock WS 帧 → handler 被调用。
- 验收：定时任务完成后，聊天页出现结果卡片。

---

### W10 `core/tests` 接入门禁（P1，预计 0.5 天）

**目标**：32 个集成测试不再无人执行；依赖 `third_party/` 的部分诚实跳过。

**依赖**：W0。

#### W10.1 明确测试选择

- 文件：`scripts/verify.sh`
- 新增：
  ```bash
  check_core_integration_tests() {
    local tests="e2e plugin_manifest_gate plugin_patch_gate tui_model_panel"
    if [ -d third_party/plugin-examples ]; then
      tests="$tests third_party_integration"
    else
      skip_check "core 集成测试 (third_party_integration)" \
        "third_party/plugin-examples 不随仓库发布（AGENTS.md §7）"
    fi
    run_check "core 集成测试 (${tests// /, })" \
      bash -c "cargo test -p marguerite-core $(for t in $tests; do printf -- '--test %s ' \"$t\"; done)"
  }
  ```
- 说明：
  - `macro_live` 默认 `#[ignore]`，不选。
  - `perf_call_chain_under_50ms` 是性能基准（50ms 预算，CI 2 核可能抖动），**不进门禁**；在 `scripts/bench/perf/README.md` 里记录手动运行方法。若团队坚持要进，则放宽到 150ms 并标注"仅趋势"。
  - `third_party_integration` 在本地有 `third_party/` 时才跑；跳过必须出现在汇总。
- 接线：`--core` 与 `--all` 都调度；`--all` 从 24 → 25 项。
- 测试：
  - 有 `third_party/`：5 个二进制全跑。
  - 无 `third_party/`：4 个跑，1 条 `[SKIP]` 出现在汇总。
  - 变异：删掉一个测试断言 → 门禁红。
- 验收：`cargo test -p marguerite-core --tests` 的真实通过数写入 README 基线。

#### W10.2 更新 README 与 AGENTS 基线

- README：新增一行 `core integration tests | N passed / 0 failed / 1 ignored`。
- AGENTS §2：同步。
- `check-readme-claims` 只比对总数，不比对数字，所以需人工确保两份 README 一致。

---

### W11 workbench 健康检查（P2，预计 0.5 天）

**目标**：默认配置下 `getHealth()` 返回 200，而不是 404。

**依赖**：无。

- 文件：`workbench/frontend/src/api/client.ts:33-37,197`
- 改法（二选一，推荐 A）：
  - A. 前端改 `getHealth: () => http('health')`（相对 `/api`）→ `http://localhost:8011/api/health`。
  - B. 后端同时暴露 `@app.get("/health")` 别名（`api.py:353` 旁加一个路由）。
- 推荐 A，因为后端契约已经是 `/api/*`；同时把注释"health endpoint lives OUTSIDE the /api prefix"改正（它是错的）。
- 同类检查：`getCapabilities` 用 `../harness/capabilities`，后端是 `@app.get("/harness/capabilities")`（L323）→ **这个是正确的**，不要一起改。
- 测试：`workbench/backend/marguerite_workbench/tests` 或前端单测 mock fetch，断言 URL 为 `/api/health`；手动 `curl http://localhost:8011/api/health`。
- 验收：Overview 页不再显示健康检查失败。

---

### W12 文档、基线与收口（P2，预计 1 天）

#### W12.1 基线同步

- 文件：`README.md`、`README.zh-Hans.md`、`AGENTS.md` §2
- 内容：
  - core 单测基线（W0 跑完后实测值；当前文档写 2704，`AGENTS` 写 2784，需以实测为准）。
  - webui 单测文件数与通过数。
  - pyengine 三层数字。
  - `--all` 闸门总数（W10 后 25）。
  - 日期统一为本次实测日期。
- 测试：`check-readme-claims.py`。

#### W12.2 截图重生成

- `docs/assets/*.png` 若与 `core/src`、`webui/src` 的 mtime 落后，按 AGENTS §4 的 `scripts/demo/capture_console.py` 重生成。
- 需要：mock LLM + 重建 `marguerite-core` + 隔离网关。
- 验收：截图不展示已修复的缺陷（例如旧的 Unknown 意图、中文提示泄漏）。

#### W12.3 i18n 棘轮（只防新增）

- 文件：`webui/scripts/check-i18n.mjs`
- 现状：只检查键缺失，不检查模板硬编码中文（538 行）。
- 改法：
  1. 扫描 `webui/src/**/*.vue` 的 `<template>` 中未走 `t()` 的 CJK 行。
  2. 把当前"文件 → 行数"写入基线 JSON（`webui/scripts/i18n-cjk-baseline.json`）。
  3. 守卫：任一文件行数 > 基线 → FAIL；减少 → 更新基线（允许）。
- 测试：变异（新增一行硬编码中文）→ 红。
- 验收：不再新增硬编码中文；存量翻译作为后续独立工程。

#### W12.4 最终验收

- `bash scripts/verify.sh --clean --release` 全绿（记录 exit 0、每项 [PASS]、总数）。
- `scripts/package_full.cmd` 产出 MSI + NSIS。
- 安装版双引擎验收（`GET /api/config` 200）。
- 电脑操控 8 条真机场景通过。
- 缓存基准报告（DeepSeek + Anthropic 各一份）落盘。
- 能力评测报告（60 条）落盘。
- 更新 `CHANGELOG.md`，写明"缓存指标口径变更"是 breaking-ish 的观测变更（前端已兼容）。

---

## 6. 能力路由：为什么这样设计

### 6.1 根因链（从用户抱怨到代码事实）

用户观察："它不懂智能调用工具/技能/MCP/记忆/知识库，都是写脚本来偏盲控电脑。"

代码事实：

1. **默认工具面只有 7 个**，其中 2 个是 `shell_exec`/`code_exec`（`tool_schema.rs:108-116`）——模型的"第一反应"必然是脚本。
2. **能力自述没有注入**（`capability.rs:238` 的 `brief()` 零调用）——模型不知道 `memory_search`/`kb_search`/`browser`/`desktop_action` 存在。
3. **工具面每轮变化**（`runtime.rs:785,1101`）——模型无法形成稳定的"我有哪些工具"的心智模型；刚学会用 A，下一轮 A 可能不在。
4. **提示词只说"优先用工具"**（`runtime.rs:166-213`），没有"意图 → 能力族 → 工具"的映射，也没有"禁止用脚本替代"的负规则。
5. **`desktop_action` 需要先 `desktop_launch`**（D-25）——操作已打开的应用这个最常见场景，模型的路径是"先 launch 再说"，很容易失败后转向脚本。
6. **Policy 误拦**（D-27）——模型尝试结构化点击时被 `|`/`cmd` 子串误判，失败后学会"还是写脚本吧"。

这六条合起来，就是"脚本偏盲控"的完整因果链。W4 逐条对治：扩展默认面（1）、注入能力卡（2）、会话稳定面（3）、路由规则（4）、附着已有窗口（5）、语义 Policy（6）。

### 6.2 路由算法（纯函数）

```rust
pub struct RouteDecision {
    pub primary: Option<&'static str>,   // card id
    pub secondary: Vec<&'static str>,
    pub reason: String,
    pub confidence: u8,                  // 0-100，仅用于排序与观测
}

pub fn route(intent: &str, available: &[ToolDef]) -> RouteDecision
```

规则顺序（先硬后软）：

1. **显式工具名**：用户说"用 browser 打开" → 直接命中。
2. **动作词 + 对象**：
   - "打开/点击/输入/窗口/应用/桌面" → `desktop.uia`（若可用）→ `desktop.pixel`。
   - "网页/链接/登录/表单/抓取" → `browser.a11y` → `web_fetch`。
   - "记住/上次/偏好/以后都" → `memory`。
   - "文档/知识库/资料/规范" → `knowledge`（先 `kb_search`，再 `grep`）。
   - "技能/流程/SOP" → `skill`。
   - "每天/定时/提醒" → `cron`。
   - "并行/团队/多个专家" → `team`。
3. **文件/代码类**：默认 `file.edit`；需要执行 → `shell.oneshot`。
4. **无匹配**：`primary=None`，不注入路由片段（避免噪声）。

约束：

- 纯函数，不读盘、不调模型、不随时间变化。
- 同一 `(intent, available)` 必须返回同一结果。
- `confidence < 50` 时不写"必须用 X"，只写"可考虑 X"。
- 路由结果放在**动态尾部**（`route_decision`），不进稳定前缀。

### 6.3 能力卡渲染（稳定文本，控制在 800 字内）

```text
【能力路由】你有以下结构化能力。除非它们不可用或失败，否则不要用 shell/python 脚本替代：
- 文件编辑：read_file / write_file / apply_patch / grep（改代码、读项目）
- 浏览器：browser（导航/表单/登录/抓取；不要用 curl/requests）
- 桌面应用：desktop_action + window（操作已打开的应用；不要用 pyautogui/uiautomation）
- 像素兜底：computer_use（仅当 UIA 不可用、游戏/画布/远程桌面）
- 记忆：memory_search / memory_save（用户偏好、跨会话约定）
- 知识库：kb_search / kb_ingest（项目文档、规范）
- 技能：list_skills / use_skill（已有 SOP，先查再用）
- 定时：cron.*（不要用 sleep 脚本轮询）
- MCP：plugins.mcp.call（外部系统集成）
- 团队：team.run / subagent（并行、多专家）
若某个能力不可用，会在下方标注原因；不要假装它可用。
```

这段文本**只依赖会话首次快照**，同一会话逐字节相同。

### 6.4 反脚本守卫的边界（避免变成"禁脚本"）

- 只在"结构化工具**确实可见**"时纠偏。
- 只纠偏一次；第二次放行并记录。
- 只匹配明确的库名/模式，不做宽泛的"含有 python 就拦"。
- 配置可关；默认 `warn`。
- 所有纠偏事件计入 `capability.metrics`，用于评估误伤率。

### 6.5 观测指标

| 指标 | 定义 | 目标 |
|---|---|---|
| `capability.first_choice_structured_pct` | 任务首轮选择结构化能力的比例 | ≥ 80% |
| `capability.script_substitution_pct` | 触发反脚本纠偏的任务比例 | ≤ 10% |
| `capability.misfire_pct` | 纠偏后模型仍坚持脚本的比例 | ≤ 30% |
| `capability.tool_not_visible_pct` | 期望能力不在可见面的比例 | 0% |
| `capability.fallback_pct` | UIA 失败降级到像素的比例 | 观测，不设目标 |

---

## 7. 缓存：为什么当前"100%"是假的，以及怎么做到真高

### 7.1 三个"命中率"不是一回事

| 名称 | 当前实现 | 真实含义 | 问题 |
|---|---|---|---|
| 客户端精确缓存 | `client_hit_rate = hits/lookups`（`gateway/cache.rs:76-81`） | 本地相同请求复用 | 与上游缓存无关 |
| 服务端前缀缓存 | `server_cache_hit_rate = read/(read+creation)`（`cache/mod.rs:578`） | 上游 KV 复用比例 | DeepSeek `creation=0` → 有命中即 100% |
| 请求命中率 | 不存在 | 多少请求至少命中一次 | 未计数 |

所以"缓存命中率非常低"和"我们已经 100%"可以同时出现在同一份 UI 上——因为量纲不同。W5.1 先修这个。

### 7.2 稳定前缀的五个杀手（按影响排序）

1. **工具数组每轮变化**（D-13）：DeepSeek/OpenAI 的前缀从请求体最前开始匹配，`tools` 变化会让整个前缀失效。**这是最大杀手。**
2. **系统提示里的可变数量/名单**（D-14）：`本轮可调用工具共 N 个：...` 每轮可能变。
3. **技能目录变化**（D-15）：新增/删除技能、MCP 工具热插拔。
4. **压缩**（`compact_window`）：超过阈值后重写历史，前缀必然失效；这是可接受的，但应被计量。
5. **Anthropic 没有断点**（D-12）：完全不用缓存。

### 7.3 稳定前缀纪律（可验收）

- 稳定半区只允许由以下来源构成：模式、推理档位、工作区、用户规则、能力卡、静态工具指引、技能目录快照。
- 禁止：当前时间、轮次序号、随机 ID、本轮工具数、本轮用户文本、经验提示、路由决定。
- 每次组装后计算指纹；稳态 `drift == 0`。
- 任何"为了让模型知道 X"而想往稳定半区加内容的需求，先问："X 会变吗？"会变 → 放动态尾部。

### 7.4 受控基准的口径

- 单会话、固定工具面、固定技能目录、无压缩、12 轮。
- 第 1 轮冷启动允许 `cache_read=0`。
- 第 2-12 轮每轮必须有 `cache_read > 0`。
- `request_hit_rate_pct` 目标 100%（11/11）。
- `cache_coverage_pct` 目标 ≥ 85%（首测后校准）。
- `stable_prefix_drift == 0`。
- 任何一条不满足，基准报告必须打印该轮的稳定半区哈希、工具名列表、技能数，禁止只给一个总数。

### 7.5 为什么不是"100% token 命中"

即使前缀完全稳定，每一轮新增的用户消息、模型输出、工具结果都是新的 token，不可能被缓存。所以"100%"只能指"合格请求的请求命中率"，或"可缓存前缀的 token 命中率"。对外文案必须写清分母。

---

## 8. 电脑操控：双轨模型

### 8.1 两条轨道

| 轨道 | 适用 | 优点 | 缺点 | 默认 |
|---|---|---|---|---|
| 结构化（UIA/DOM） | 原生 Windows 控件、浏览器页面 | 稳定、可定位、可验证、无需坐标 | 部分应用不暴露 UIA | 优先 |
| 像素（截图 + 坐标） | 游戏、画布、远程桌面、UIA 空白 | 万能 | 脆弱、依赖分辨率、验证难 | 兜底 |

决策顺序：

```text
UIA 可用且找到目标 → 结构化动作
UIA 可用但动作不支持 → 像素（记录 fallback）
UIA 不可用 → 像素
像素失败 → 返回明确错误，不无限重试
```

### 8.2 observe → act → verify

每个写动作三步：

1. **observe**：抓目标节点（或窗口）摘要：`{role, name, value, enabled, focused, bounds}`。
2. **act**：调用 UIA pattern 或像素动作。
3. **verify**：
   - 重新抓同一节点/窗口；
   - 比较 `value/enabled/focused/exists`；
   - 生成 `effect` 与 `verified`；
   - 审计落盘。

失败处理：

- `verified=false` 不一定是失败（例如点击打开了新窗口），但回执必须写明 `no_observable_change`。
- `NodeNotFound` → 不重试超过 2 次；返回候选节点（同 role/近似 name），让模型决策。
- UIA 超时 → 降级像素；记录 `fallback`。

### 8.3 审批

- 敏感动作在 `Policy::require_confirmation` 返回 true 时，走 `guard::approval_registry`（与 W3.1 同一条链路）。
- 审批卡片展示：应用名、窗口标题、控件 role+name、动作、参数摘要。
- 拒绝 → 不执行；超时 → fail-closed。
- 审计：每次动作（含只读 snapshot 的摘要）记录到 `Recorder`；敏感动作额外记录审批 ID。

### 8.4 与浏览器侧对齐

| 浏览器 | 桌面（新增） |
|---|---|
| `browser.a11y` | `desktop.a11y` |
| `click_ref` | `desktop.action {kind:"invoke"/"click", ref_id}` |
| `evaluate` | `desktop.action {kind:"set_value"/"type"}` |
| `screenshot` | `computer_use.action=screenshot` |
| `open` | `desktop_launch` / `window_activate` |

同一套 `ref_id` 语义：从 snapshot 获得，在下一次 snapshot 前有效；失效则返回 `REF_EXPIRED`。

### 8.5 跨平台

- Windows：UIA（现有）。
- macOS：AXUIElement（未实现；返回 `UNSUPPORTED_PLATFORM`，不要假装支持）。
- Linux：AT-SPI（未实现；同上）。
- `desktop.a11y` 在非 Windows 上返回 `available:false, reason:"uia_windows_only"`。

---

## 9. 双引擎事件契约（补充）

### 9.1 映射表（W1 的实现依据）

| pyengine 原始事件 | Rust WS 帧事件 | 前端订阅 | 现状 |
|---|---|---|---|
| `session.event.state_change` | 同名 | `useChatRpcSubscriptions.ts:74` | 丢失（变 status） |
| `session.event.cron_result` | 同名 | L79 | 丢失 |
| `session.event.task_group.waiting` | 同名 | L85 | 丢失 |
| `session.event.task_group.synthesizing` | 同名 | L86 | 丢失 |
| `session.event.task_group.done` | 同名 | L87 | 丢失 |
| `session.event.task_group.failed` | 同名 | L88 | 丢失 |
| `session.event.tool_use_start` | 同名 | 已有 | 丢失（pyengine 侧的会被拍平） |
| `session.event.tool_result` | 同名 | 已有 | 丢失 |
| `session.event.text_delta` | 同名 | 已有 | 丢失 |
| `session.event.done` | 同名 | 已有 | 丢失 |
| `sessions.changed` | 同名（顶层） | 已有 | 可能正常（取决于 py_event） |
| `artifact.updated` | 同名（顶层） | 已有 | 可能正常 |

> W1 修复后，前 10 行全部按原名到达。第 11-12 行需在真机复核。

### 9.2 安全约束

- 只接受 `pyengine.*` 会话。
- 事件名只允许 `[a-z0-9_.]`，且必须以 `session.event.` 开头或命中三个顶层白名单。
- 长度上限 128 字符。
- 非法 → 回退 `session.event.status`，并 `warn!` 记录一次（限流日志，避免刷屏）。

### 9.3 双侧守卫

- Rust 单测：6 个关键事件名。
- pyengine 单测：`tests/test_gateway` 新增"网关发出的每个 `session.event.*` 都能被 Rust 映射"的契约（读取 Rust 源码白名单？跨语言；用共享 JSON 清单 `docs/contracts/session-events.json`，两侧都读它）。
- 新文件：`docs/contracts/session-events.json`，字段 `{name, direction, consumers}`；Rust 与 pyengine 的测试都 `include_str!`/读文件，断言自己的发射/消费集合是它的子集。

---

## 10. 测试矩阵

### 10.1 单元测试

| 包 | 文件 | 关键断言 |
|---|---|---|
| W0 | `scripts/check_gui_shell_config.py --self-check` | 4 个夹具 |
| W1 | `core/src/gateway/events.rs` tests | 6 个事件名 + 非法名 |
| W2 | `core/src/gateway/sessions.rs` tests | 3 种 snapshot 场景 |
| W3 | `core/src/gateway/{collab,open_path,native,rate_limit}.rs` | 审批/越界/限流 |
| W4 | `core/src/agent/{capability_cards,capability_guard}.rs` | 路由表驱动 + 纠偏 |
| W5 | `core/src/cache/mod.rs`、`core/src/model/llm.rs` | 4 个指标 + Anthropic body |
| W6 | `core/src/desktop_automation/*` | pattern 分发 + 回执 |
| W8 | `webui/src/**/*.spec.ts` | 最近工作区、命令面板、toast |
| W9 | `webui/src/composables/cron` | 三种形状 |

### 10.2 契约测试

- `docs/contracts/session-events.json`：双侧读取。
- `docs/contracts/capability-cards.json`：卡片里的工具名必须存在于真实注册表（Rust 测试）。
- `docs/contracts/cache-metrics.json`：字段名与公式说明；前端类型生成可选。

### 10.3 变异测试（每条守卫至少一次）

见 §3.4。结果写入 PR 描述，不写进代码注释。

### 10.4 e2e（Playwright）

- 工作区选择：最近列表、浏览文件夹、首次自动弹。
- 命令面板：workspace/model/team 切换。
- 缓存面板：两个指标标签存在且数值来自 `cache.status`。
- 定时任务：三种形状渲染。
- 事件：mock WS 发送 `state_change` → 岛状态变化。

### 10.5 真机 / 安装版

- 电脑操控 8 条（W6.7）。
- 双引擎事件 6 条（W1）。
- ACL 3 条（W7）。
- 安装版：解包 MSI → 启动 → `/api/config` 200 → 桌宠 show/toggle。
- 缓存基准：DeepSeek + Anthropic。
- 能力评测：60 条。

### 10.6 非门禁基准

| 基准 | 位置 | 频率 |
|---|---|---|
| `perf_call_chain_under_50ms` | `core/tests/perf.rs` | 手动，发版前 |
| 缓存基准 | `scripts/bench/cache/` | 每次改提示词/工具面后 |
| 能力评测 | `scripts/bench/capability/` | 每次改能力卡/工具面后 |

---

## 11. 提交顺序与并行

### 11.1 批次

| 批次 | 包含 | 前置 | 可并行 |
|---|---|---|---|
| B0 | W0 | 无 | 否（先做） |
| B1 | W1 + W10 | W0 | W10 可与 W1 并行（不同文件） |
| B2 | W2 + W9 | B1 | W2 与 W9 有重叠（methods.rs），串行 |
| B3 | W3 | B2 | 可与 B2 部分并行（不同文件），但 `methods.rs` 冲突需协调 |
| B4 | W4 | B3 | 否（核心改动） |
| B5 | W5 | B4 | 否（依赖稳定面） |
| B6 | W6 | B4 | 可与 W5 并行（不同模块） |
| B7 | W7 + W11 | B6 | 可并行 |
| B8 | W8 | W9 | 可与 W6 并行 |
| B9 | W12 | 全部 | 否 |

### 11.2 文件冲突面（同一时间只允许一人改）

- `core/src/gateway/methods.rs`：W2、W3、W9、W6.5。
- `core/src/agent/runtime.rs`：W4、W5。
- `core/src/model/llm.rs`：W5。
- `webui/src/components/CommandPalette.vue`：W8。
- `scripts/verify.sh`：W0、W10。
- `.github/workflows/ci.yml`：W0。

### 11.3 每包提交信息模板

```text
<type>(<scope>): <一句话为什么>

- 变更点 1
- 变更点 2

验证：bash scripts/verify.sh --<层>（exit 0，N/N）
变异：<改坏什么> → <哪条测试红>（已恢复）
```

---

## 12. 风险登记

| ID | 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|---|
| K-01 | 稳定工具面改变模型行为，任务完成率下降 | 中 | 高 | 先做离线评测；保留 `MARGUERITE_STABLE_TOOL_FACE=0` 回退开关（发布前移除） |
| K-02 | Anthropic 断点位置不当导致缓存不命中 | 中 | 中 | 只放 3 个断点（system/tools/last user）；真机验证 |
| K-03 | 反脚本守卫误伤正当脚本 | 中 | 高 | 默认 `warn`、一次纠偏、可关、记录误伤率 |
| K-04 | UIA 在不同应用上行为差异大 | 高 | 中 | 8 条真机场景 + 明确降级；不承诺"所有应用" |
| K-05 | 审批接入阻塞自动化流程 | 中 | 中 | 审批超时 fail-closed；提供只读白名单（后续） |
| K-06 | 限流误伤 UI 轮询 | 低 | 高 | 健康检查豁免；默认宽松；可配置 |
| K-07 | 事件透传导致前端重复处理（status + 具体事件） | 中 | 中 | 只发具体事件，不再发 status（除非 pyengine）；前端回归 |
| K-08 | cron 双系统语义冲突 | 高 | 中 | 在线以 pyengine 为准；离线 Rust 只做 reminder；文档写清 |
| K-09 | `core/tests` 含不稳定测试 | 中 | 中 | 先本地跑 3 次；不稳定者 `#[ignore]` 并具名 |
| K-10 | 安装版验收依赖真实 Windows 桌面 | 低 | 高 | 预留真机时间；CI 只做构建 |

---

## 13. 完成定义（DoD）

一个工作包"完成"必须同时满足：

1. 代码已合并到工作区（未提交也可，但必须是当前分支的最终态）。
2. 该包的必跑门禁全绿（§3.3），输出已附。
3. 新增守卫做过变异验证（§3.4）。
4. 测试覆盖了"正常/边界/失败"三类；失败路径有明确错误码。
5. 文档（`AGENTS.md`/README/契约 JSON）已同步，或明确写"本包不改文档，原因 X"。
6. 回滚方案已写明并演练过一次（至少 dry-run）。
7. 没有遗留 `TODO`/`FIXME`/`unimplemented!` 与本包相关。
8. 没有新增未登记的 `[SKIP]`。

整个计划"完成"额外要求：

1. `bash scripts/verify.sh --clean --release` exit 0。
2. 安装版双引擎在线（`/api/config` 200）。
3. 电脑操控 8 条真机通过。
4. 缓存基准（**受控口径**：单会话、固定工具面、固定技能目录、无压缩、12 轮）：DeepSeek 第 2-12 轮请求命中率 100%，coverage ≥ 85%，drift 0；Anthropic 第 2 轮起 `cache_read > 0`。不得把该结果写成"所有场景 100% 缓存命中"。
5. 能力评测：路由纯函数 60/60；在线结构化首选率 ≥ 80%（首测后校准）。
6. 两份 README 的基线、闸门数、日期一致，且 `check-readme-claims.py` 绿。
7. 所有 P0/P1 关闭；P2 有明确归属（本计划或后续）。

---

## 附录 A：已核实行号索引（2026-10-26）

> 施工时若行号漂移，以符号名为准；PR 里更新本表。

| 文件 | 符号/位置 | 用途 |
|---|---|---|
| `core/src/pyengine/event_bridge.rs` | L39-56 | py_event 注入与 Status 拍平 |
| `core/src/gateway/events.rs` | L17-38 | envelope→frame 映射 |
| `core/src/gateway/methods.rs` | L1664-1680 | 会话水合占位 |
| `core/src/gateway/methods.rs` | L1723-1748 | cron.list Rust |
| `core/src/gateway/methods.rs` | L1870-1878 | sandbox.resume / skills.deps.install 占位 |
| `core/src/gateway/methods.rs` | L398-399 | RUST_FIRST sessions.messages |
| `core/src/gateway/sessions.rs` | L194-310 | chat_history 真实存储 |
| `core/src/gateway/collab.rs` | L118-175 | ssh_exec 无审批 |
| `core/src/gateway/_helpers.rs` | L70-88 | open_in_system |
| `core/src/gateway/artifacts.rs` | L132-155 | artifacts open |
| `core/src/gateway/native.rs` | L98-135 | poker.auto 越界 |
| `core/src/gateway/rate_limit.rs` | 全文件 | 死限流 |
| `core/src/gateway/server.rs` | L248-296 | 路由与层 |
| `core/src/gateway/workspaces.rs` | L20-28, L136-140, L145-175 | WorkspaceEntry/list/open |
| `core/src/agent/tool_schema.rs` | L14-20, L82-201, L225-234 | ToolDisclosure |
| `core/src/agent/runtime.rs` | L132-227 | tool_guide/compose_guide |
| `core/src/agent/runtime.rs` | L461-565 | build_model_messages |
| `core/src/agent/runtime.rs` | L785, L1101 | 每 turn ToolDisclosure |
| `core/src/agent/tool_catalog.rs` | L30-50, L317-384 | 工具分组与目录文本 |
| `core/src/agent/capability.rs` | L236-256 | brief() 未注入 |
| `core/src/agent/tool_compress.rs` | L217 | 唯一漂移观测点 |
| `core/src/cache/mod.rs` | L528-595 | tracker 与指标 |
| `core/src/gateway/cache.rs` | L73-110 | cache.status |
| `core/src/model/llm.rs` | L824-838, L983-1007 | Anthropic 序列化 |
| `core/src/tools/computer_use.rs` | L1791-1862, L1900-1930, L1917 | desktop_launch/desktop_action schema、computer_use 参数、动作枚举 |
| `core/src/desktop_automation/uia_driver.rs` | L160-187 | 动作分发 |
| `core/src/desktop_automation/windows_uia.rs` | L40-56, L279-295 | UIA pattern |
| `core/src/desktop_automation/policy.rs` | L28-80 | 敏感判定/注入扫描 |
| `core/src/desktop_automation/driver.rs` | L103-135 | UiAction/ActionReceipt |
| `core/src/desktop_automation/recorder.rs` | L16-50 | 审计快照 |
| `gui/src-tauri/build.rs` | L278-290 | ACL 命令清单 |
| `gui/src-tauri/capabilities/default.json` | 全文件 | 窗口/权限 |
| `gui/src-tauri/tauri.conf.json` | L24 | splash.html |
| `gui/src-tauri/src/lib.rs` | L587, L633, L643, L3319-3369 | pet_overlay_* / navigate |
| `scripts/check_gui_shell_config.py` | L59-65 | /control/ 断言 |
| `scripts/verify.sh` | L105-116, L421-453, L634-637, L912, L932-961 | skip_check / webui 层 / gui 守卫 / `--all` 调度块（24 项） |
| `webui/src/composables/cron/useCronJobs.ts` | L10-35 | cron 形状 |
| `webui/src/types/cron.ts` | L1-30 | CronJob 字段 |
| `webui/src/components/island/game/useEngineBridge.ts` | L133-136 | Math.max |
| `webui/src/components/ProjectWorkspacePickerDialog.vue` | L530-542 | 单选原生选择器 |
| `webui/src/platform/desktopBridge.ts` | L177-193 | pet_overlay invoke |
| `webui/src/composables/chat/useChatRpcSubscriptions.ts` | L74-88 | 事件订阅 |
| `workbench/frontend/src/api/client.ts` | L35, L197 | health URL |
| `pyengine/src/opensquilla/gateway/rpc_cron.py` | L562 | cron 返回数组 |
| `pyengine/src/opensquilla/gateway/rpc_skills.py` | L688 | deps.install 真实现 |
| `pyengine/src/opensquilla/gateway/rpc_sandbox.py` | L913 | sandbox.resume 真实现 |
| `pyengine/src/opensquilla/gateway/event_bridge.py` | L34 | session.event.{kind} |
| `.github/workflows/ci.yml` | L92-180, L257-293 | rust/gui job |

## 附录 B：新增文件清单

| 文件 | 工作包 | 用途 |
|---|---|---|
| `core/src/agent/capability_cards.rs` | W4 | 能力卡与路由 |
| `core/src/agent/capability_guard.rs` | W4 | 反脚本软门控 |
| `core/src/gateway/open_path.rs` | W3 | 受控系统打开 |
| `core/src/gateway/events_contract.rs`（或测试模块） | W1 | 跨引擎事件守卫 |
| `docs/contracts/session-events.json` | W1 | 事件契约单一事实源 |
| `docs/contracts/capability-cards.json` | W4 | 能力卡冻结清单（供测试） |
| `docs/contracts/cache-metrics.json` | W5 | 缓存指标字段名与公式说明（前端类型生成可选） |
| `scripts/bench/capability/tasks.jsonl` | W4 | 60 条评测 |
| `scripts/bench/capability/run.py` | W4 | 在线评测 |
| `scripts/bench/cache/bench_prefix_cache.py` | W5 | 缓存基准 |
| `scripts/bench/perf/README.md` | W10 | 性能基准手动说明 |
| `THIRD_PARTY_NOTICES.md` | W0 | 根级合规入口 |
| `scripts/check-third-party-notices.py` | W0（可选） | 根级声明与 `pyengine/THIRD_PARTY_NOTICES.md` 的一致性守卫；做则必须含变异验证 |
| `webui/scripts/i18n-cjk-baseline.json` | W12 | 中文硬编码棘轮 |

## 附录 C：本次未验证、施工前必须补测的项目

| 项 | 原因 | 补测方法 |
|---|---|---|
| `verify.sh --clean --all` 当前真实全绿/红 | 未运行完整门禁（成本与进程锁） | W0 第一件事；先停 daemon，跑完再起 |
| `core/tests` 32 例是否全稳定 | 未执行 | `cargo test -p marguerite-core --tests` 连跑 3 次 |
| pyengine snapshot 事件的精确字段 | 未读 `rpc_sessions.py` 的实现细节 | 施工前读；以它为准 |
| `windows_uia.rs` 各 pattern 的实际可用性 | 未在真机逐个验证 | W6.1 真机 6 动作 |
| DeepSeek 真实缓存命中 | 无 API key/未跑基准 | W5.5 |
| Anthropic 真实缓存命中 | 无 API key/未跑基准 | W5.4 真机 |
| 安装版双引擎在线 | 未解包当前安装包 | W12.4 |
| `settings-modal` e2e 偶发 | 历史已知，未定位 | W12 前连跑 5 次 |
| `gui` 的 `pet-overlay` 窗口 label | 未读创建代码 | W7 前 grep |
| 控制台 CacheStrip 的具体组件路径 | 未定位 | W5.6 前 grep `server_cache_hit_rate` |

---

## 附录 D：一页纸执行摘要（给决策者）

| 问题 | 根因 | 方案 | 验收 |
|---|---|---|---|
| 门禁"绿"不可信 | GUI 守卫与 splash 设计冲突；CI gui job 调不存在的脚本；3 处静默 SKIP；core/tests 无门禁 | W0 + W10 | `--all` 25/25，变异有牙 |
| 模型只会写脚本 | 默认工具面只有 shell/code；能力自述未注入；工具面每轮变；无路由规则；Policy 误拦 | W4 + W6 | 结构化首选率 ≥80%，脚本替代 ≤10% |
| 缓存命中低/假 100% | 工具数组每轮变；可变文本在稳定前缀；Anthropic 无断点；指标量纲错 | W5 | 第 2-12 轮请求命中 100%，coverage ≥85%，drift 0 |
| 电脑操控盲 | 只有像素；UIA 只暴露 click；无 observe/verify；无附着已有窗口 | W6 | 8 条真机场景 |
| 双引擎事件丢 | 所有 pyengine 事件拍平成 status | W1 | 6 个事件原名到达 |
| Rust 假成功 | 3 个占位 handler + cron 契约不一致 | W2 + W9 | 在线真实/离线诚实 |
| 安全边界 | ssh 无审批；任意路径打开；poker 越界；限流死代码 | W3 | 审批/边界/限流用例 |
| 体验摩擦 | 工作区总弹目录；命令面板缺切换；静默失败 | W8 | e2e |
| 安装版桌宠不可用 | ACL 缺 show/toggle 与窗口 | W7 | release invoke 成功 |
| workbench 健康 404 | 路径 `../health` | W11 | `/api/health` 200 |

---

## 7. 施工状态（2026-10-26 本轮）

### 已完成（均带实测证据）

- **W0 门禁复位**：`verify.sh --clean --all` 从 24 项扩到 **25 项**（新增 core 集成测试），逐项 `[PASS]` + exit 0；e2e 棘轮稳定在基线 95（两次 95/2 与一次 94/3 的波动已定位为侧车冷启动，`verify.sh` 改为带 token 等 `/api/config` 200 后才启动 Playwright）；deliverables 助手修掉水合竞态与真实侧栏路径子串误匹配；GUI 壳守卫接上 `--self-check`；README 占位符与闸门总数同步（中英一致）。
- **W1 双引擎事件契约**：`pyengine_event_name` 透传 `state_change`/`cron_result`/`task_group.*`/`run_heartbeat`；跨语言守卫（include_str! 解析 pyengine + 前端订阅）有变异证据。**额外发现并修复**：pyengine payload 无会话键 → 转发时注入 `session_key`，否则别的会话的事件会污染当前聊天（e2e 实测复现）。
- **W2 占位清理**：`sandbox.resume` / `skills.deps.install` 的 `{ok:true}` 假实现删除；`sessions.messages.subscribe/hydrate/snapshot` 在线代理侧车真实数据、离线 `degraded`。真机双引擎探针：在线 hydrate 返回侧车完整元数据；杀侧车后 snapshot/hydrate `degraded:true`、resume/deps 回 `INTERNAL_ERROR`（重连中，不再假成功）。
- **W3 安全边界**：ssh 执行审批（批准前不 spawn）、`open_path` 工作区/可执行/控制字符拦截 + ShellExecuteW、poker 越界 panic（变异实测 panic 复现）、限流接线（真机 500 并发：247 通过 / 253 个 429；health 5×200；`/control/` 20×200）、注入扫描按字段语义化（含「发送者信息」反例）。
- **W4 能力路由**：能力卡 + 能力自述注入（稳定半区、逐字节可复现）、固定高频面 7→18（不随请求文本变化）、反脚本守卫（每 turn 每能力一次纠偏）、`brief_from` 纯函数化。
- **W5 缓存**：coverage / request-hit-rate / eligible 计数落库并进 `cache.status`；Anthropic 输入总量归一（input+read+creation）；Anthropic `cache_control` 三断点；工具指引删除逐轮变化的「共 N 个」行。
- **W7 桌宠 ACL**：show/toggle 进构建清单与主窗口权限；新增 pet-overlay 窗口能力。
- **W8.1 工作区最近列表**：`last_opened_at` 排序/置顶/幂等刷新；单选默认最近一条，列表为空才弹原生选择器；Rust 8/8 + 前端 52 例 + typecheck。
- **W9.1 cron 契约**：在线代理侧车、离线返回数组（`next_run` ISO）；前端接受数组/`{jobs}`/`{items}` 三形状。
- **W10 集成测试门禁**：4 个离线二进制进 `--core`/`--all`（实测 5+9+9+5 全过）；`third_party_integration` 缺失时诚实 `[SKIP]`。
- **W11 workbench 健康**：真机 `/api/health` 200、`/harness/capabilities` 200。
- **W12.3 i18n 棘轮**：基线 18 文件/408 行，变异即红，`--update-baseline` 拒绝抬高。
- **W6 电脑操控（后端）**：UIA 动作矩阵 13 种 + 7 类 pattern 分发（10s 超时/节点上限）；observe→act→verify 回执（before/after/verified/effect）与真实审计快照；`desktop_action` 可省略 handle 按 window 附着、新增 `desktop_attach`；`desktop.attach/windows/a11y` RPC 入 SUPPORTED_METHODS（spawn_blocking 异步分派）。单测 80 例。**未做**：真机 8 场景录像、`type` 的 SendKeys 兜底。
- **W4.2 会话级稳定工具面**：`SessionToolFace` 会话内只增不减、按名排序；发现只追加；会话切换重建；首轮面不随请求文本变化。
- **W4.7 能力评测集**：60 条离线路由用例（11 族、55 条反脚本负例），JSON/Markdown 报告；真实 provider 端到端基准待 key。
- **W5.3 漂移分域**：`observe_stable_prefix_scoped` + `drift_by_scope`，runtime 已按 `agent.stable_half` 接线，压缩层按 `compress.tool_results`；`cache.status` 暴露明细。
- **W8.2/W8.3**：命令面板补 workspace/model/team/agent + 别名（只列网关声明的方法）；静默失败 7/8 处落地（App.vue 的 4 处经核实为正当降级），新增统一的采样提示与单测。

### 未完成 / 带到下一轮（不声称完成）

- **W6.7 真机 8 场景**：未执行（会操作当前桌面），需人工录像/截图验收；`type` 暂无 SendKeys 兜底。
- **W4.7 真实模型基准报告** / **W5.4 缓存基准报告**：需要真实 provider key；当前只有离线评测与单元证据。
- **W8.4 选择器与命令面板联动**（W8.1 已覆盖「最近工作区」主路径，空状态快捷入口未做）。
- **W12.1 基线数字**（核心 3999/0/24、webui 等以最终门禁实测为准）、**W12.2 截图重生成**、**W12.4 `--release` 打包 + 安装版双引擎 + 8 场景验收**。
- 已知观测口径变更：`server_cache_hit_rate` 语义已改为 coverage（前端旧字段兼容），发布说明需提醒。

---

（完）
