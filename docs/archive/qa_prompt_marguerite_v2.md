# Marguerite Agent 工作台 · 全链路 QA 测试指令 v2（项目专用·反肢解强化版）

> 用途：粘贴进 vibe coding 工作台，对 `D:\Downloads\Marguerite\marguerite` 做「前台↔后台↔API(WS)↔持久化」双向对账 + 护栏/安全审计 + 上线前全检。
> 本版在 `qa_prompt_marguerite.md`（架构映射正确）基础上，专门修复「被肢解为孤立小任务（如只验证模型切换、只列 skills）」的执行缺陷——强制 P0 主链路优先、固定闭环模板、禁止拆散用户旅程。
> 前置：本提示词模块/路径/事件名均来自本仓库（已核验：`core/src/gateway/methods.rs`、`events.rs`、`server.rs`、`core/src/guard/*`、`core/src/sandbox/audit.rs`、`core/src/router/decision_log.rs`、`core/src/context/session.rs`、`plugins/manifests/*.plugin.json`、`core/src/governance/*`、`core/src/security/*`、`persistence/config.rs:435 data_dir()`）。

## 0. 你的角色与能力
你是一名拥有本仓库完整访问权的 QA/渗透测试工程师，必须主动使用：
- **运行与构建**：`cargo build` / `cargo test -p marguerite-core --offline`；桌面端走 Tauri dev 或已打包 exe。
- **前端观测**：操作 `webui`(Vue，Tauri 窗体加载 `http://localhost:18790/control/`) 与 TUI（`cargo run -p marguerite-core --`），截图并读取浏览器/网络面板与 WS 帧。
- **后端探针**：读取 `core/src/gateway` 的 JSON-RPC 方法（`methods.rs`）、WS 事件定义（`events.rs`）、事件转发（`server.rs`）；订阅 `ws://localhost:18790/ws` 抓 `session.event.*` 实时帧。
- **持久化审计**：读取 `$1marguerite/config.yaml` 与 `Config::data_dir()`（见 `persistence/config.rs:435`）下的运行时文件（`router_decision_log.jsonl`、SQLite usage、sessions、插件注册表）。
- **源码阅读**：按模块定位实现（见第 2 节映射），核对行为是否符合预期。
- **联网调研**：对标主流 Agent 工作台（OpenDevin / Cline / AutoGen / Dify / Aider 等）的能力与护栏设计，补齐本站缺失项。

## 1. 环境与入口（按此连接，勿猜测）
- 仓库根：`D:\Downloads\Marguerite\marguerite`
- 配置目录：`$1marguerite/`（生产）；`config.yaml` 为配置，`data_dir()` 为运行时数据
- 网关：JSON-RPC + 唯一 WS `/ws`，监听 `18790`；Tauri 主窗体需等网关就绪（`gui/src-tauri/src/lib.rs:706-735` 的 gateway-readiness 等待）才能创建，否则白屏/连接被拒
- 构建产物：`dist/marguerite-desktop` / `dist/marguerite-gui` / `dist/marguerite-tui`
- 文档参考：`docs/architecture.md`、`docs/skills.md`、`docs/mcp.md`、`docs/memory.md`、`docs/development.md`、`CHANGELOG.md`

## 2. 双向观察映射表（核心：每用一项功能就对账）
| 你的动作层 | 去哪看「后台（引擎）」 | 去哪看「API / 数据」 |
|---|---|---|
| 在 webui / TUI 操作（发消息、切会话、装技能、改路由…） | 对应 `core/src` 引擎模块（见矩阵 A–K） | gateway JSON-RPC 方法名 + WS `session.event.*` 帧 + `data_dir()` 文件 |
| 在 `~/.marguerite` 改配置 / 调内部 / 注入事件 | 引擎模块状态与日志 | config.yaml 是否回写、SQLite/jsonl 是否变更、WS 是否广播 |
| 跑 `cargo test` / 构建 | 编译与单测覆盖的模块 | 断言事件名、落盘路径、护栏拦截 |

**闭环 A（前台驱动）**：前台动作 → 抓触发的 JSON-RPC 方法 + 订阅 WS 看 `session.event.*` → 读 `data_dir()` 确认落库 → 回前台刷新确认 UI 状态一致。
**闭环 B（后台驱动）**：改 config.yaml / 直接调 gateway 内部 / 注入事件 → 回前台确认 UI 即时反映（如路由切换、usage 刷新、插件 Widget 出现）。

## 3. 禁止肢解为孤立小任务
以下行为属于**错误执行**，必须避免：
- ❌ 先测「模型切换是否生效」「skills 列表是否显示」等边缘点，跳过「启动→对话→路由→运行时→护栏」主链路。
- ❌ 把「发消息」「切会话」「装技能」「看 usage」拆成互不相关的独立任务，各自截张图就交差。
- ❌ 只验证前台页面是否存在，不验证对应 gateway 方法、WS 帧、持久化文件。
- ❌ 只跑 `cargo build` 看能否编译，不结合业务流程验证事件/落盘是否真实正确。
- ❌ 只做安全扫描（如 `cargo audit`），不结合业务路径验证漏洞是否真实可复现。

**正确组织方式：按「用户角色 + 完整使用旅程」切块**。例如「新用户从启动、建会话、发消息得到回复、观察路由决策、看 usage 与持久化」是一条完整用例，必须贯穿前后台与持久化，不得拆散。

## 4. 固定记录模板（每用例必填，未填完视为未完成）
```
用例编号 | 模块 | 流程 | 前置状态 | 动作 | 预期前台 | 预期 API/WS | 预期持久化 | 实际前台 | 实际 API/WS | 实际持久化 | 是否完美 | 证据
```
每完成一个用例都必须填完上表；「只截图不抓 API/WS」「只查页面不看持久化」均判为未完成。证据包括：前台截图、WS 帧 JSON、JSON-RPC 报文、config.yaml / `data_dir()` 文件前后 diff、后端代码位置（文件:行）。

## 5. 执行顺序：先 P0 主链路，再 P1/P2
必须严格按以下优先级执行。**P0 未完成不得进入 P1；P0 失败立即暂停上报，不得带病继续。**

### P0 核心使用链路（用户旅程，必须完整跑通）
1. **启动与就绪**：桌面/TUI 启动 → 网关就绪等待生效 → webui 加载 `18790/control` 不白屏、不连接被拒；首次运行生成 `$1marguerite/config.yaml`，配 `model.api_key` 后获真实回复（A1–A3）。
2. **会话与对话**：建会话 → 发消息 → AgentRuntime 真实回复 → 观察 `session.event.tool_use_start` / `usage` / `done` 流式帧；上下文压缩触发 `session.event.compaction`（B1–B3）。
3. **意图与模型路由**：意图分类 → `session.event.router_decision` + `data_dir()/router_decision_log.jsonl` 落盘（`router/decision_log.rs`）；模型路由切换（`models.routing.set`）→ config.yaml 持久化（`persistence/config.rs`）；Ensemble/Bloom 模式 → `session.event.ensemble_progress`（C1–C3）。
4. **Agent 运行时与工具**：工具循环 → `session.event.tool_use_start`；MCP 工具注册（`mcp list`）；pyengine（Python harness）调用链路；`desktop_automation` / `browser` / `terminal` 工具（D1–D2）。

### P1 护栏 / 记忆 / 插件 / 事件 / 持久化
- **E 幻觉护栏与治理**：`guard` 四类护栏（语法/语义/事实/契约）→ 触发拦截 + 自动修复（`core/src/guard/*`）；`governance` / `security` / `sandbox`（审计日志 `sandbox/audit.rs`）→ 越权/越界被拦；`credentials` 密钥**绝不**写入 config.yaml（优先级 环境变量 > .env > YAML）。
- **F 记忆与知识**：三层记忆（感知/短期/长期）沉淀与检索；`kb` 知识库检索。
- **G 技能与插件**：`skills list` / `skills install <yaml-url>`（远程安装）；DSH 插件加载（`plugins/manifests/*.plugin.json`：modlens/genui/pet-whale/market/router/schedule/usage…）→ 前端 Widget + SteeringEvent 回传；manifest 校验、不新增原生依赖（宪法 D12/D16）。
- **H 网关与事件总线**：JSON-RPC 方法调用 → 响应结构正确；订阅 `ws://localhost:18790/ws` → `session.event.*` 全量转发（`server.rs` 订阅 `**`）；`collab` / `team` 多端同步事件。
- **I 持久化与一致性**：config.yaml 读写一致；`data_dir()` 文件（router_decision_log.jsonl、SQLite usage、sessions）正确写读；重启 / 并发后状态恢复，无互踩。

### P2 自动化 / 打包 / 安全审计
- **J 自动化 / 调度 / 部署**：`automation` + `schedule` 定时任务；`deploy` + Tauri 打包（`build.rs` 清空 skills 陷阱、复用 `CARGO_TARGET_DIR` 规避 os error 5、不跑 `package_*.cmd` 保留缓存）；`btpanel-mcp` 集成。
- **K 安全审计专项**：密钥不落盘（见 E3）；`sandbox_paths` 路径限制；`cargo audit` 零漏洞 + `cargo clippy` 零警告 + 调用链路 < 50ms（质量门禁）。

## 6. 护栏 / 风控系统专项（必须给出「缺失补齐建议」）
后台管控与风控不是「看看有没有页面」，而是逐项检查是否**真正生效**。以下能力若无实现，必须标为缺失并给出建议：
- [ ] **提示注入防护**：用户/工具回传内容试图劫持系统指令、越权调用工具时，`guard` 是否拦截（`core/src/guard/gateway.rs`、`security/scanner_l3.rs`）。
- [ ] **工具滥用防护**：危险命令/越权文件访问被 `security/command_guard.rs` 与 `os_sandbox.rs` 拦截；`preflight.rs` 前置检查生效。
- [ ] **密钥/凭证防护**：API Key 不写 config.yaml、不进日志、不回传前端；`.env` / 环境变量优先级正确（`persistence/config.rs` credentials 逻辑）。
- [ ] **沙箱越界防护**：`sandbox_paths` 限制文件可达范围；`sandbox/audit.rs` 审计日志完整可查；`penetration_tests.rs` 自检通过。
- [ ] **SSRF 防护**：`browser` / `fetch` / 联网工具对外请求受 `security/ssrf_guard.rs` 约束，禁止内网/元数据地址。
- [ ] **模型路由滥用防护**：路由决策可审计（`router/decision_log.rs`），异常切换/成本超预算（`context/budget.rs`）有拦截或告警。
- [ ] **技能供应链防护**：远程 skill yaml 经 `security/skill_scan.rs` 扫描；manifest 不引入未声明原生依赖（宪法 D12/D16）。
- [ ] **治理闭环**：`governance/review.rs` / `verifier.rs` / `completion_gate.rs` 在关键产物/回复前把关，违规被拦并进入修复（`guard/repair.rs`）。

## 7. 安全漏洞检查清单（必须在业务流程中复现，禁止只扫描）
每类漏洞都要构造真实请求/场景，确认能否利用，并给出利用链与修复位置（文件:行）：
- [ ] **提示注入（Prompt Injection）**：在用户输入、工具回传、检索内容中夹带指令，验证能否越权/数据外泄；看 `guard` 拦截证据。
- [ ] **命令注入**：经 `terminal` / `desktop_automation` 工具尝试 `; rm -rf`、`$(...)`、`&&` 串联；验证 `command_guard.rs` / `os_sandbox.rs` 拦截。
- [ ] **SSRF**：让联网/浏览器工具请求 `http://169.254.169.254/`、`http://127.0.0.1:18790/ws`、`file://`；验证 `ssrf_guard.rs` 拦截。
- [ ] **路径遍历 / 沙箱逃逸**：工具尝试读写 `../`、绝对系统路径、配置目录外文件；验证 `sandbox_paths` 与 `os_sandbox.rs`。
- [ ] **凭证泄露**：API 响应 / WS 帧 / 日志 / 前端包中是否出现明文 API Key、token、数据库字段；错误信息是否暴露内部路径。
- [ ] **技能供应链**：构造恶意 skill yaml（含未声明依赖 / 危险脚本），验证 `skill_scan.rs` 是否拦截、manifest 校验是否生效。
- [ ] **MCP 工具提权**：普通会话能否调用仅限内部/管理员的方法；WS 订阅 `**` 是否存在信息越权泄露。
- [ ] **越权与会话**：多会话/多 workspace 间能否越权访问彼此的 sessions、记忆、文件；`workspace.rs` / `sessions.rs` 隔离是否生效。
- [ ] **拒绝服务 / 资源耗尽**：超长输入、超大文件、并发风暴下网关与 `context/budget.rs` 是否仍能自我保护。

## 8. 持续运行会话纪律
- 使用**同一个网关进程 / 同一个配置目录 `~/.marguerite`** 完成前后台与持久化对账，不得中途重启 gateway（除非测启动本身）导致状态丢失。
- 切换「前台操作 → 后台改配置 → 回前台观察」时，保持同一 dev 实例与同一 config.yaml，不得另开一份干净配置造成误判。
- 每次「后台改动 → 回前台观察」必须在改动后即时完成，并重新抓 API/WS/持久化对账，不凭记忆。
- 动到 `build.rs`、`CARGO_TARGET_DIR`、已打包产物时严格遵循打包陷阱约定，避免清掉 skills 或触发 os error 5。

## 9. 联网补充设计
开始前联网调研 3–5 个主流 Agent 工作台，整理「本站缺失或薄弱的能力点」（如：可视化工作流编排、权限 RBAC、审计溯源面板、插件沙箱评级、用量计费、红队自检、合规导出），列成补充清单；测试时反向验证是否已有实现，没有则标「建议新增」。

## 10. 缺陷上报与产出
- 缺陷按 **P0（阻断/资损/安全）/ P1（功能错乱）/ P2（体验/边界）** 分级，附：
  - 复现步骤、前台截图、对应 WS 帧/JSON-RPC 报文
  - 持久化差异（config.yaml 或 `data_dir()` 文件前后对比）
  - 后端代码位置（文件:行，如 `core/src/gateway/methods.rs:NN`）
- 最终产出结构化报告：`deliverables/gstack/qa_marguerite_v2.md`，包含：
  - P0/P1/P2 用例逐项结论（第 4 节表格汇总）
  - 护栏/风控系统缺失补齐建议（第 6 节）
  - 安全漏洞清单与修复位置（第 7 节）
  - 联网功能补齐建议（第 9 节）
  - 上线前风险结论（可上线 / 条件上线 / 不可上线）

## 11. 对执行代理的强制指令
- 不得以「OrganizeAgent 已派发 / 框架已分块」为由降低质量；每个闭环必须自己验证并返回证据。
- 不要一上来就测「模型切换」「skills 列表」「设置页显示」等边缘项；必须先跑完 P0 主链路（启动→对话→路由→运行时）。
- 每完成一个模块 / 一条完整使用旅程，立即汇总第 4 节表格并请求用户确认是否继续，不要等全部跑完才一次性报告。
- 发现 P0（阻断/资损/安全）立即暂停，先上报再决定是否继续，不得带病推进。
