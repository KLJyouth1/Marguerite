# Marguerite Agent 工作台 · 全链路 QA 测试指令（项目专用版）

> 用途：粘贴进 vibe coding 工作台，对 `D:\Downloads\Marguerite\marguerite` 做「前台↔后台↔API↔持久化」双向对账测试，达到运营级质量。
> 前置：本提示词是针对本项目真实架构定制的；通用模板见 `qa_prompt_enhanced.md`，二者方法论一致，但本版模块/路径/事件名均来自本仓库。

## 0. 你的角色与能力
你是一名拥有本仓库完整访问权的 QA 工程师，必须主动使用：
- **运行与构建**：`cargo build` / `cargo test -p marguerite-core --offline`；桌面端走 Tauri dev 或已打包 exe。
- **前端观测**：操作 `webui`(Vue，Tauri 窗体加载 `http://localhost:18790/control/`) 与 TUI（`cargo run -p marguerite-core --`），截图并读取浏览器/网络面板。
- **后端探针**：读取 `core/src/gateway` 的 JSON-RPC 方法（`methods.rs`）、WS 事件定义（`events.rs`）、事件转发（`server.rs`）；订阅 `ws://localhost:18790/ws` 抓 `session.event.*` 实时帧。
- **持久化审计**：读取 `$1marguerite/config.yaml` 与 `Config::data_dir()` 下的运行时文件（`router_decision_log.jsonl`、SQLite usage、sessions、插件注册表）。
- **源码阅读**：按模块定位实现（见第 2 节映射），核对行为是否符合预期。
- **联网调研**：对标主流 Agent 工作台（OpenDevin / Cline / AutoGen / Dify 等）的能力与护栏设计，补齐本站缺失项。

## 1. 环境与入口（按此连接，勿猜测）
- 仓库根：`D:\Downloads\Marguerite\marguerite`
- 配置目录：`$1marguerite/`（生产）；`config.yaml` 为配置，`data_dir()` 为运行时数据
- 网关：JSON-RPC + 唯一 WS `/ws`，监听 `18790`；Tauri 主窗体需等网关就绪（`gui/src-tauri/src/lib.rs:706-735` 的 gateway-readiness 等待）才能创建，否则白屏
- 构建产物：`dist/marguerite-desktop` / `dist/marguerite-gui` / `dist/marguerite-tui`
- 文档参考：`docs/architecture.md`、`docs/skills.md`、`docs/mcp.md`、`docs/memory.md`、`docs/development.md`、`CHANGELOG.md`

## 2. 双向观察映射表（核心：每用一项功能就对账）
| 你的动作层 | 去哪看「后台」 | 去哪看「API/数据」 |
|---|---|---|
| 在 webui / TUI 操作（发消息、切会话、装技能…） | 对应 `core/src` 引擎模块（见矩阵） | gateway JSON-RPC 方法名 + WS `session.event.*` 帧 + `data_dir()` 文件 |
| 在 `~/.marguerite` 改配置 / 调内部 / 注入事件 | 引擎模块状态与日志 | config.yaml 是否回写、SQLite/jsonl 是否变更、WS 是否广播 |
| 跑 `cargo test` / 构建 | 编译与单测覆盖的模块 | 断言事件名、落盘路径、护栏拦截 |

**闭环 A（前台驱动）**：前台动作 → 抓触发的 JSON-RPC 方法 + 订阅 WS 看 `session.event.*` → 读 `data_dir()` 确认落库 → 回前台刷新确认 UI 状态一致。
**闭环 B（后台驱动）**：改 config.yaml / 直接调 gateway 内部 / 注入事件 → 回前台确认 UI 即时反映（如路由切换、usage 刷新、插件 Widget 出现）。

## 3. 测试矩阵（按优先级执行，每格走完闭环）
- **A 启动与就绪**（最先做，决定能否测）
  - A1 桌面启动 → 网关就绪等待生效 → webui 加载 `18790/control` 不白屏、不连接被拒
  - A2 TUI 启动（`cargo run -p marguerite-core --`）→ `/help /new /clear /model /skills /status /compress /exit` 可用
  - A3 首次运行生成 `$1marguerite/config.yaml`；配置 `model.api_key` 后 `-p`/TUI 获真实回复
- **B 会话与对话**（优先级 #25）
  - B1 会话增删切换（`session list/delete`、`/new`、`/clear`）
  - B2 发消息 → AgentRuntime 真实回复 → 观察 `session.event.tool_use_start` / `usage` / `done` 流式帧
  - B3 上下文压缩 → `session.event.compaction` / `contextPressure` 契约对齐（`context/session.rs`）
- **C 意图与模型路由**（优先级 #24）
  - C1 意图分类 → `session.event.router_decision` + `data_dir()/router_decision_log.jsonl` 落盘（`router/decision_log.rs`）
  - C2 模型路由切换（`models.routing.set`）→ config.yaml 持久化（`persistence/config.rs:322`）
  - C3 Ensemble/Bloom 模式 → `session.event.ensemble_progress`（Captain+Copilot / 专家 DAG，`orchestration/mod.rs`、`model/ensemble.rs`）
- **D Agent 运行时与工具**
  - D1 工具循环 → `session.event.tool_use_start`；MCP 工具注册（`mcp list`）
  - D2 pyengine（Python harness）调用链路；`desktop_automation` / `browser` / `terminal` 工具
- **E 幻觉护栏与治理**（对应「风控系统」）
  - E1 `guard` 四类护栏（语法/语义/事实/契约）→ 触发拦截 + 自动修复
  - E2 `governance` / `security` / `sandbox`（审计日志 `sandbox/audit.rs`）→ 越权/越界被拦
  - E3 `credentials` 密钥管理：密钥**绝不**写入 config.yaml（优先级 环境变量 > .env > YAML，`persistence/config.rs:462`）
- **F 记忆与知识**
  - F1 三层记忆（感知/短期/长期）沉淀与检索；F2 `kb` 知识库检索
- **G 技能与插件**（DSH 集成已落地，验证完整性）
  - G1 `skills list` / `skills install <yaml-url>`（远程安装）
  - G2 DSH 插件加载（`plugins/manifests/`：modlens/genui/pet-whale/market/router/schedule/usage…）→ 前端 Widget + SteeringEvent 回传
  - G3 插件 manifest 校验、不新增原生依赖（宪法 D12/D16）
- **H 网关与事件总线**
  - H1 JSON-RPC 方法调用 → 响应结构正确；H2 订阅 `ws://localhost:18790/ws` → `session.event.*` 全量转发（`server.rs` 订阅 `**`）
  - H3 `collab` / `team` 多端同步事件
- **I 持久化与一致性**
  - I1 config.yaml 读写一致；I2 `data_dir()` 文件（router_decision_log.jsonl、SQLite usage、sessions）正确写读
  - I3 重启 / 并发后状态恢复，无互踩
- **J 自动化 / 调度 / 部署**
  - J1 `automation` + `schedule` 定时任务；J2 `deploy` + Tauri 打包（`build.rs` 清空 skills 陷阱、复用 `CARGO_TARGET_DIR` 规避 os error 5、不跑 `package_*.cmd` 保留缓存）
  - J3 `btpanel-mcp` 集成
- **K 安全审计专项**
  - K1 密钥不落盘（见 E3）；K2 `sandbox_paths` 路径限制；K3 `cargo audit` 零漏洞 + `cargo clippy` 零警告 + 调用链路 < 50ms（质量门禁）

## 4. 「完美」判定清单（每条用例逐项勾选）
- [ ] 前台状态正确（文案/按钮/Widget/徽章与预期一致，无白屏/连接拒）
- [ ] 跳转/响应正确（URL、WebSocket 连接、无死链/超时）
- [ ] API 正确（JSON-RPC 方法存在、参数合法、响应码/报文结构正确；WS 帧名与 `events.rs` 契约一致）
- [ ] 持久化正确（config.yaml / `data_dir()` 文件写读正确，无脏数据、无并发互踩）
- [ ] 前后台一致（状态同步无延迟、无错乱；后台改动前台即时反映）
- [ ] 护栏有效（guard/governance/sandbox 对越权/越界/幻觉正确拦截，密钥不落盘）
- [ ] 无报错（前端 console、Rust 编译/单测、后端 error log 干净；clippy/audit 通过）
- [ ] 边界/幂等（重复提交、空值、超长输入、并发不崩）

## 5. 联网补充设计
开始前联网调研 3-5 个主流 Agent 工作台，整理「本站缺失或薄弱的能力点」（如：可视化工作流编排、权限 RBAC、审计溯源面板、插件沙箱评级、用量计费），列成补充清单；测试时反向验证是否已有实现，没有则标「建议新增」。

## 6. 缺陷上报与产出
- 缺陷按 **P0（阻断/资损/安全）/ P1（功能错乱）/ P2（体验/边界）** 分级，附：
  - 复现步骤、前台截图、对应 WS 帧/JSON-RPC 报文
  - 持久化差异（config.yaml 或 `data_dir()` 文件前后对比）
  - 后端代码位置（文件:行，如 `core/src/gateway/methods.rs:NN`）
- 最终产出结构化报告：`deliverables/qa_marguerite.md`（矩阵逐项结论 + 缺陷清单 + 补充设计建议 + 上线前风险结论）。

## 7. 执行纪律
- 不跳过任何闭环；宁可慢，不可漏。
- 每次前后台切换都重新抓 API/WS/持久化对账，不凭记忆。
- 全部走完再下「可上线 / 不可上线」结论；发现 P0 立即暂停上报。
- 动到 `build.rs`、`CARGO_TARGET_DIR`、已打包产物时严格遵循打包陷阱约定，避免清掉 skills 或触发 os error 5。
