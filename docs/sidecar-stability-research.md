# 侧车（pyengine）长线稳定性 / 能力增强 —— 调研报告

> 日期：2026-10-20
> 范围：Rust 网关托管的 Python 侧车（`pyengine/`，OpenSquilla 系）在「长线复杂、长续航、长时间高负载」桌面任务下的稳定性现状、业界对照与改进项清单。
> 证据约定：标注 **[已验证]** 的结论来自本轮对仓库源码的直接阅读（附文件:行号）；标注 **[推断]** 的为基于代码的合理推断，未单独实测；外部案例全部附 URL 与资料类型。
> 运行形态（用户实测）：`C:\Python314\python.exe -m opensquilla.cli.main gateway run --bind 127.0.0.1 --port 59906`，由 Rust 网关（`core`）spawn 并守护，内部端口经 `core/src/gateway/sidecar_proxy.rs` 代理给控制台。

---

## 第一部分：本仓侧车现状（代码实证）

### 1.1 启动与拉起链

| 机制 | 现状 | 证据 |
|---|---|---|
| spawn 命令 | `<python> -m opensquilla.cli.main gateway run --bind 127.0.0.1 --port <port>`；环境注入 `PYTHONPATH=<pyengine/src>`、`OPENSQUILLA_HOME=<data_dir>/pyengine-home`、`OPENSQUILLA_GATEWAY_TOKEN`、`OPENSQUILLA_LOG_LEVEL` | **[已验证]** `core/src/pyengine/sidecar.rs:1288-1384`（`spawn_once`） |
| Python 探测 | 按 `py -3.14/3.13/3.12` → `python3.14` → `python` 顺序探测，要求 ≥3.12 | **[已验证]** `sidecar.rs:90-137` |
| 源码探测 | 打包布局 `<exe>/resources/pyengine/src` 优先，其次 cwd 相对候选，再 exe 上溯 6 层；`MARGUERITE_PYENGINE_SRC` 可显式覆盖 | **[已验证]** `sidecar.rs:162-212` |
| token SSOT | 启动前把共享 token 写入 `pyengine-home/gateway-token`（写失败有 5 次瞬态退避重试；沙箱拦截时 home 降级到系统临时目录） | **[已验证]** `core/src/pyengine/auth.rs:19-46`、`sidecar.rs:894-911` |
| 桌面端接入 | Tauri 命令 `sidecar_start`（懒启动：detect → `PyEngineSidecar::start`）、`sidecar_status`、`sidecar_stop`；`py_sidecar` 存于 AppState | **[已验证]** `gui/src-tauri/src/lib.rs:342-374` |
| 复用（attach） | 启动前探测可复用侧车：`<data_dir>/sidecar.json`（Rust 自记 pid/port）→ Python pidfile（`<home>/default/state/gateway.pid`）→ `netstat`(Win)/`lsof`/`ss`(Unix) 反查监听端口；健康就 `attach` 复用而非新拉 | **[已验证]** `sidecar.rs:228-2270` 区段（`candidate_sidecars`/`attach`/`listening_ports_of`） |
| 单实例约束 | Python 侧对同一 profile 是单实例（`OPENSQUILLA_PROFILE_IN_USE`，`gateway/pidlock.py`），第二次启动秒退 → 因此必须复用而非重复 spawn | **[已验证]** `sidecar.rs:229-239` 注释、`pyengine/src/opensquilla/gateway/pidlock.py` |

### 1.2 健康检查 / 心跳

| 机制 | 现状 | 证据 |
|---|---|---|
| 看门狗轮询 | 守护任务每 **2s**：`try_wait()` 查进程退出 + `GET /health`（3s 超时） | **[已验证]** `sidecar.rs:962-1061` |
| 判死阈值 | 进程在但探活失败 → 用 10s 宽超时复核一次；连续 **3 次失手**（`HEALTH_MISS_LIMIT`）才判死重启，避免把忙引擎误杀 | **[已验证]** `sidecar.rs:1005-1028,1386-1429` |
| 存活语义 | 任何 **HTTP <500**（含 429/404）都算活着 —— 限流回 429 曾被旧逻辑误判成死亡 | **[已验证]** `sidecar.rs:1405-1457` |
| 代理污染防护 | 探活 reqwest client 带 `.no_proxy()`，防本机 clash/代理把回环目标替回 502 | **[已验证]** `sidecar.rs:1440-1444` |
| Python 端点 | `GET /health` → **写死** `{"ok":true,"status":"live"}`；`GET /ready` → `{ready, status, uptime_ms}`，未就绪回 503；另有 `/healthz`、`/readyz` 别名 | **[已验证]** `pyengine/src/opensquilla/gateway/app.py:160-174,768-771` |
| **缺口** | Rust 看门狗**只用静态 `/health`**，从不消费已存在的 `/ready`；也没有"事件循环是否真在转"的深度自证 | **[已验证]** `sidecar.rs:1431-1458` 只请求 `/health` |
| WS 保活 | Rust→Python WS 客户端每 **25s** 发应用层 `{"type":"ping"}`，Python 回 `{"type":"pong"}`；另有裸 WS Ping/Pong 帧应答 | **[已验证]** `core/src/pyengine/client.rs:331,420-428`；`pyengine/.../websocket.py:1190-1195` |

### 1.3 失败处理、自动重启与崩溃取证

| 机制 | 现状 | 证据 |
|---|---|---|
| 自动重启 | 主路径：判死后 kill 旧进程 → 退避重启，退避 **500ms → 30s 指数封顶**；健康等待 90s（embedding 加载慢） | **[已验证]** `sidecar.rs:76-80,1039-1060` |
| **缺口①** | **崩溃循环无熔断**：退避封顶 30s 后无限重启；若侧车启动即崩（profile 锁损坏、缺依赖），会 30s+90s 一轮永远空转，UI 只看到 `failed`/时好时坏 | **[已验证]** 守护循环无任何失败计数窗口 / FATAL 状态（对照 supervisord `startretries`） |
| **缺口②** | **attach 复用路径不重启**：附着的孤儿侧车被杀后只记录证据、直接退出守护任务（`spawn_attach_guard` 取证即 return）——控制台此后永久降级为纯 Rust 模式，直到用户手动重启应用 | **[已验证]** `sidecar.rs:1218-1286`（注释明示"不负责重启"） |
| 崩溃现场 | 退出证据三件套：退出码 + 存活时长 + stderr 尾部（≤16KB，按 UTF-8 字符边界回退解码）；**双写**：进程级内存槽（`host.exit_evidence` RPC 可读）+ `<data_dir>/sidecar.json` 落盘（跨重启可读） | **[已验证]** `sidecar.rs:282-353,760-805` |
| 启动即退出快速失败 | spawn 后 12s 内子进程退出 → 不等 90s 健康超时，立即结构化报错并落证据（典型场景：`OPENSQUILLA_PROFILE_IN_USE`） | **[已验证]** `sidecar.rs:919-944,1180-1216` |
| Python 侧优雅关停 | POSIX：SIGINT/SIGTERM → asyncio handler → `server.close()` **排空在飞 turn 与后台 completion**；Windows：无真 SIGTERM，另开 HTTP shutdown 端点（`app.state.request_shutdown`） | **[已验证]** `pyengine/src/opensquilla/cli/gateway_cmd.py:39-57,236-281` |
| **缺口③** | **Rust 侧关停从不给 drain 窗口**：`stop()`/`Drop`/`kill_on_drop(true)` 直接 `kill()` —— Python 侧精心实现的排空在 Windows 桌面场景下永远拿不到执行机会；Rust 也没调用那个 HTTP shutdown 端点 | **[已验证]** `sidecar.rs:1096-1120,1354`（`.kill().await` / `kill_on_drop(true)`） |
| WS 断连自愈 | 客户端监督任务：断开后指数退避（500ms→30s）自动重连到同一 base_url；重连窗口内调用 fail-fast 立即回明确错误（不让调用方白等 60s）；pending 调用在断连时全部回"已断开"错误 | **[已验证]** `client.rs:170-258,431-436` |
| 调用超时 | 单调用默认 **60s**（`MARGUERITE_PYENGINE_CALL_TIMEOUT_MS` 可覆盖）；超时后 best-effort 发 `Cancel`，另有 25s tick 兜底清扫超期 pending（防 Cancel 丢失导致内存滞留） | **[已验证]** `client.rs:260-289,443-495` |

### 1.4 任务运行时与长任务

| 机制 | 现状 | 证据 |
|---|---|---|
| 任务队列 | Python `TaskRuntime`：per-session turn 队列 + 指标（队列深度 gauge、in-flight 计数、取消计数、队列满计数）；队列满回 `QUEUE_FULL`→HTTP 429 | **[已验证]** `pyengine/src/opensquilla/gateway/task_runtime.py:1-60`、`app.py:112-122` |
| 任务状态机 | `AgentTaskStatus{queued, running, succeeded, failed, cancelled, timeout, abandoned}`；重启后运行中任务落为 **abandoned/interrupted** 并如实广播 | **[已验证]** `pyengine/src/opensquilla/session/models.py:65-72`、`boot.py:1561-1593` |
| 重启恢复 | 启动时：过期未决审批全部失效（`approval_queue.expire_all_pending`）、清"重启孤儿会话键"（`take_restart_abandoned_session_keys`） | **[已验证]** `pyengine/src/opensquilla/gateway/boot.py:2601-2624` |
| **缺口④** | **无 checkpoint / 断点续跑**：在飞 turn 被重启打断即终止，没有"执行到哪一步、哪些副作用已生效"的持久化；也没有幂等键机制防止重启后重放造成副作用重复 | **[已验证]** 代码内无 run_id/checkpoint/journal 概念（对照 OpenHands/LangGraph 一节） |

### 1.5 日志与可观测

| 机制 | 现状 | 证据 |
|---|---|---|
| Rust 侧 stdout/stderr | 落盘 `%TEMP%/marguerite-sidecar/sidecar-<port>.{out,err}.log`，经 `observability::rotation::open_rotating`：**10MB/文件、200MB/目录总量封顶、删最旧**；`MARGUERITE_SIDECAR_LOG_DIR` 可覆盖 | **[已验证]** `sidecar.rs:1127-1158,1345-1351` |
| Python 侧 debug.log | `<OPENSQUILLA_HOME>/logs/debug.log`，`RotatingFileHandler`：**5MB/文件、保留 3 个备份**（默认 20MB 封顶）；`OPENSQUILLA_LOG_DIR`/`OPENSQUILLA_LOG_FILE_ENABLED` 可配 | **[已验证]** `pyengine/.../boot.py:1900-1909`、`config.py:2302-2305` |
| 诊断包 | `--export-diagnostics` 收集侧车日志目录 | **[已验证]** `sidecar.rs:1137-1138` 注释 |
| **缺口⑤** | **无资源指标采集**：Rust 不采集子进程 RSS/CPU/句柄数，UI 与诊断包看不到侧车内存趋势；长跑内存泄漏（Python 侧常见）只能等 OOM 或变慢才被发现 | **[已验证]** `sidecar.rs` 无任何进程资源读取代码 |

### 1.6 资源限制与并发围栏

| 机制 | 现状 |
|---|---|
| **缺口⑥** | **子进程无内存/CPU 围栏**：`spawn_once` 不设 Windows Job Object、不设 Linux rlimit —— 侧车可无限吃整机内存，拖垮整个桌面端（**[已验证]** `sidecar.rs:1294-1359` 无任何资源限制调用） |
| REST 代理 | `/api/{*path}` 通配转发给侧车：离线 501 契约、不可达 502 `sidecar_unreachable`（retryable）、同源校验、token 替换、流式 body |
| **缺口⑦** | **通配代理无请求超时**：共享 `sidecar_client()` 只设了 `.no_proxy()`，无任何 `.timeout()`（只有审批快照/裁决两个特例各自设了超时）——侧车 hang 住（TCP 在但事件循环堵死）时浏览器请求会永久挂起（**[已验证]** `core/src/gateway/approvals.rs:246-252`、`sidecar_proxy.rs:167-185`） |
| WS 事件扇出 | Python event 帧 → 客户端 channel（256 缓冲）→ event_bridge → Rust EventBus |
| **缺口⑧** | **事件 channel 有界但无背压策略**：`event_tx.send().await` 在驱动循环内 await，event_bridge 消费慢时会**反过来堵住整个 WS 驱动循环**（请求/响应被事件洪峰 head-of-line block）；Python 服务端本已有 lossy/control 分级丢弃，客户端没有 |

### 1.7 IPC 协议小结

- 协议形态：**HTTP REST（代理 + 直连）+ WebSocket 长连接（RPC v3：challenge→connect→req/res/event）**；无 stdio。
- 握手：challenge(10s 超时) → connect req(带空 auth，Python 对空 token 放行 guest) → HelloOk(15s 超时)。
- 单连接串行调用（1:N），一个 client 一个长连接；25s ping 保活；断连自动重连。

---

## 第二部分：外部多源调研

### 2.1 进程守护 / 看门狗（监督模式对照）

| 案例 | 来源 URL | 核心机制（一句话） | 可借鉴点 | 类型 |
|---|---|---|---|---|
| Supervisor（supervisord） | https://www.supervisord.org/subprocess.html ；http://supervisord.org/configuration.html | 子进程 STARTING→BACKOFF 状态机；`startsecs`（必须稳定运行 N 秒才算启动成功）+ `startretries`（连续失败超上限 → **FATAL 放弃重启**） | 本仓缺口①的直接参照：加"稳定运行时长"与"失败次数熔断 → FATAL/Degraded" | 官方文档 |
| supervisord 实战配置 | https://how2.sh/posts/how-to-build-agent-runtime-isolation-policies-for-incident-recovery-in-internal-platform-engineering/ | `autorestart=true / startretries=10 / startsecs=10` + 指数退避防 crash loop | 参数量级参考（10 次 / 10s 稳定窗口） | 实战博客 |
| DigitalOcean 教程 | https://www.digitalocean.com/community/tutorials/how-to-install-and-manage-supervisor-on-ubuntu-and-debian-vps | 频繁重启本身是信号而非常态；可调 `startsecs`/`startretries` | 熔断后要把"为什么一直重启"暴露给运维，而非静默 | 实战教程 |
| 硬件看门狗（嵌入式） | https://www.iesdouyin.com/share/video/7637784930021346570 （抖音，2825 赞 / 323 评论） | "死机可以，但必须能自己爬起来"：倒计时定时器，主循环卡死不喂狗 → 硬件复位 | 理念移植：**喂狗方必须是独立于被监控方的循环**（Rust 守护任务独立于 Python 事件循环，正是这个结构；但守护任务自己也需要"第二级保险"） | 视频 |
| Windows 进程监控软件 | https://www.iesdouyin.com/share/video/7441087451331906835 | Windows 上对 exe 卡死/关闭自动重新拉起 | 桌面端场景与本仓完全同构 | 视频 |
| ⚠️ 评论区说明 | 视频工具（video_search）只返回视频正文摘要，**不返回评论区原文**；上述点赞/评论数为工具返回的元数据。嵌入式看门狗视频的高评论量与"看门狗是否该用于中断子程序"的讨论，只能从正文片段推断，未能逐条摘录评论。 | | |

### 2.2 健康探针（liveness vs readiness）

| 案例 | 来源 URL | 核心机制 | 可借鉴点 | 类型 |
|---|---|---|---|---|
| Kubernetes 探针模型 | https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/ | liveness 抓死锁/死循环（失败即重启），且**不依赖外部系统**；startup 探针在慢启动期间抑制 liveness | 本仓已有 startup（wait_healthy 90s）与 liveness 雏形，但 `/ready`（readiness）写好了却没人用 | 官方文档 |
| FastAPI 健康检查分层 | https://vikulin-va.ru/observability/python/health-checks/ | `/health/live` 只看进程，`/health/ready` 才查依赖/事件循环 | Rust 看门狗应探 `/ready`，静态 `/health` 只证明 TCP 活着 | 实战博客 |
| 误导性健康检查事故 | https://dev.to/agenticstack/architectural-breakdown-my-health-check-watched-the-wrong-file-492b | "所有健康端点都回 200、pid 文件在、日志还在刷新，但 worker 已经死了 11 分钟" | 静态 200 端点=自欺；需要**事件循环自证新鲜度**（loop tick 时间戳） | 实战博客（事故复盘） |
| launchd Python 监控 | https://dev.to/ihazratummar/troubleshooting-misleading-health-checks-in-python-with-launchd-2n5m | 内存泄漏导致 swap/OOM 远早于 liveness 被发现；asyncio 死锁陷阱 | 探针之外要有内存趋势监控（对应缺口⑤⑥） | 实战博客 |

### 2.3 长任务检查点 / 断点续跑（durable execution）

| 案例 | 来源 URL | 核心机制 | 可借鉴点 | 类型 |
|---|---|---|---|---|
| LangGraph checkpointers | https://docs.langchain.com/oss/python/langgraph/checkpointers ；https://arxiv.org/pdf/2608.29381 | 每个 superstep 边界存 StateSnapshot（SQLite/PG）；同一 thread_id 重启后从最后快照恢复；三种 durability 模式权衡性能/一致性 | 本仓 AgentTaskStatus 已持久化，但 turn 内步骤无快照 | 官方文档 + 论文 |
| LangChain Delta Channels | https://www.langchain.com/blog/delta-channels-evolving-agent-runtime | 只存每步 delta + 每 K 步一次全量快照，恢复延迟与存储成本兼得 | 轻量 checkpoint 的工程做法 | 官方博客 |
| Temporal（Strada 案例） | https://temporal.io/resources/case-studies/strada | 工作流可跨天等待人工响应后原地恢复；限流等瞬态失败由 Retry Policy 自动消化 | "等审批"正是 agent 场景（本仓有审批队列） | 官方案例 |
| Restate | https://restate.dev/vs/temporal ；https://niteagent.com/blog/durable-execution-agents-2026/ | journal 回放：崩溃后重放执行函数，已完成步骤直接返回缓存结果**不重放**；`idempotency-key` 头去重 | 副作用防重放的关键：幂等键 | 官方 + 博客 |
| 概念对比表 | https://agdex.ai/blog/durable-execution-ai-agents-2026 | Temporal=append-only history、LangGraph=序列化快照 per node | 判断本仓该选哪种路线 | 博客 |
| OpenHands 持久化 | https://docs.openhands.dev/sdk/guides/convo-persistence ；https://docs.openhands.dev/sdk/guides/agent-server/conversation-goals | Conversation 序列化到 persistence_dir，换进程反序列化后 `run()` 继续；goal 只恢复非终态（complete/capped 不可恢复） | 桌面 agent 最接近的参照：conversation 级持久化 | 官方文档 |
| 断点续跑面试拆解 | https://www.iesdouyin.com/share/video/7666122075678421931 （抖音，385 赞 / 19 评论） | 生产级 checkpoint 四类信息：任务信息（run_id）、执行计划（已完成/下一步）、执行上下文（中间结果）、**已生效副作用清单**；防"从头重跑重复创建采购单" | checkpoint 必须含副作用账本 | 视频 |
| Agent 长任务管控四件套 | https://www.iesdouyin.com/share/video/7689040015665450278 （抖音） | 目标锚定 / 步骤校验 / 关键节点自省 / **进度熔断**（最大步数、最大工具调用、最大时长） | 本仓 TaskRuntime 缺"单 turn 最大预算"硬熔断 | 视频 |

### 2.4 流式连接重连与背压

| 案例 | 来源 URL | 核心机制 | 可借鉴点 | 类型 |
|---|---|---|---|---|
| WS 重连指南 | https://websocket.org/guides/reconnection/ | 指数退避+jitter；重连容易，**状态同步难**：序列号对账、补收丢失消息 | 本仓重连已具备，但重连后**丢的 256 缓冲内事件不补收**（event 无 seq 对账需求？Python 帧有 seq 字段，可利用） | 官方指南 |
| 重连雪崩事故 | https://invental.co/blog/websocket-backpressure-meltdown | 无退避无 jitter 的重连 + 每次重连重发 43.6 万消息 → 正反馈循环打垮 relay | 单客户端场景 jitter 不重要，但**断连窗口内事件洪峰**要丢弃而非堆积（对应缺口⑧） | 实战事故复盘 |
| 背压 meltdown 工程解 | https://dev.to/devopsdaily/websockets-are-the-easy-part-3l90 | 有界缓冲 + drop-oldest + 重连后对账 | 客户端事件 channel 改 lossy drop-oldest | 实战博客 |
| Claude Code `--bg` 后台会话 | https://www.iesdouyin.com/share/video/7639742380043783487 （抖音，1400 赞） | 后台会话由 **supervisor 进程托管**，关终端不影响；但**机器休眠/关机后 session 显示 failed，不跨休眠恢复** | 业界现状与本仓同级：长任务跨进程恢复仍是 open problem | 视频（实战经验） |
| Claude Code 上下文压缩 | https://www.iesdouyin.com/share/video/7671133902458735906 （抖音，4270 赞 / 88 评论） | snip/微压缩/全局压缩三级；压缩失败超 3 次放弃（**断路器**） | 本仓 auto-summarize 已有类似分层，但缺"压缩连续失败 N 次→降级策略" | 视频 |

### 2.5 资源围栏（Windows / Linux）

| 案例 | 来源 URL | 核心机制 | 可借鉴点 | 类型 |
|---|---|---|---|---|
| Windows Job Objects | https://learn.microsoft.com/zh-cn/Windows/win32/procthread/job-objects ；https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information | `CreateJobObject` + `AssignProcessToJobObject`；`JOB_OBJECT_LIMIT_JOB_MEMORY` 限制作业级提交内存总和，超限提交失败并通知完成端口 | 本缺口⑥的直接解法（Windows 为主力平台） | 官方文档 |
| Windows 容器资源控制 | https://learn.microsoft.com/en-us/virtualization/windowscontainers/manage-containers/resource-controls | 容器内存上限底层就是 Job Object | 佐证 Job Object 是 Windows 桌面应用进程隔离的标准件 | 官方文档 |

---

## 第三部分：差距分析与改进项清单

### 3.1 现状 vs 业界对照表

| 主题 | 业界成熟做法 | 本仓现状 | 差距 |
|---|---|---|---|
| 崩溃循环熔断 | supervisord startretries → FATAL | 30s 封顶无限重启 | **无熔断**（缺口①） |
| 复用进程守护 | 接管即守护到底 | attach 只取证不重启（缺口②） | 孤儿侧车=永久降级 |
| liveness/readiness | 探针抓事件循环死锁 | 静态 `/health` 200；`/ready` 闲置 | 探针不够深 |
| 优雅关停 | drain in-flight 后退出 | Python 有 drain，Rust 直接 kill | drain 被浪费（缺口③） |
| 长任务续跑 | checkpoint + 幂等键 + journal 回放 | turn 重启即 abandoned | 无续跑（缺口④，架构级） |
| 资源围栏 | Job Object / cgroup / rlimit | 无任何限制（缺口⑥） | 内存可吃光整机 |
| 资源趋势监控 | RSS/CPU 指标 + 预警 | 无采集（缺口⑤） | 泄漏只能后知后觉 |
| 代理超时 | 所有出站请求必须有超时 | 通配代理无超时（缺口⑦） | hang 时浏览器永久转圈 |
| 事件背压 | lossy/drop-oldest + seq 对账 | 256 有界 channel 无背压策略（缺口⑧） | 慢消费者堵驱动循环 |
| 日志轮转 | 轮转 + 总量封顶 | **已有**（Rust 10MB/200MB；Python 5MB×4） | 无差距，保持 |
| WS 重连 | 退避重连 + 状态对账 | 退避重连已有；事件不补收 | 小差距 |

### 3.2 改进项清单（按优先级）

---

#### P0-1 崩溃循环熔断（crash-loop circuit breaker）

- **问题/现状**：侧车若启动即崩（profile 锁坏、依赖缺失、配置损坏），守护任务 30s 一轮无限重启，每轮还叠加 90s 健康等待；UI 无明确感知，日志空转。对照 supervisord `startretries` → FATAL。
- **方案**：守护循环维护"重启窗口"——滑动窗口（建议 10 分钟）内重启 ≥5 次（或启动后 30s 内即退出的"秒退"≥3 次）→ 转入新状态 `Degraded`，**停止自动重启**；把 `last_exit_evidence`（已有）通过 `host.exit_evidence` RPC 暴露给 UI，由用户/显式 `sidecar_start` 命令触发人工恢复。正常稳定运行满 5 分钟则清零失败计数。
- **改动点**：
  - `core/src/pyengine/sidecar.rs`：守护任务循环加 `restart_events: Vec<Instant>` 窗口与 `classify_restart_loop()` 纯函数；`PyEngineStatus` 加 `Degraded`；`STATUS_*` 常量加一档。
  - `gui/src-tauri/src/lib.rs:312`：`py_status_to_str` 加 `"degraded"` 映射。
- **测试方式**：纯函数单测（窗口内 N 次 → Degraded / 稳定运行清零）；集成测试：mock 一个 spawn 即退的命令序列，断言第 5 次后守护任务停止重启并置 Degraded。
- **风险与兼容性**：低。状态机多一档，UI 旧文案按 unknown 处理即可；人工恢复路径（`sidecar_start` 命令重入）已存在。
- **是否架构级**：否。

#### P0-2 深度健康探针（readiness + 事件循环自证）

- **问题/现状**：看门狗只探静态 `/health`（写死 200），事件循环堵死/死锁时要么等 3s 超时×3 才判死（≈36s 空窗），要么（若 TCP 层还能应答）根本发现不了。
- **方案**：① Rust 看门狗把探测 URL 从 `/health` 换成已存在的 `/ready`（语义=能干活）；② Python 侧加"事件循环自证"：一个每 1s 更新时间戳的心跳任务，`/ready` 计算 `now - last_tick > 15s` 则回 503（让循环卡死自己暴露）；保留现有"活但忙"的 3 失手+宽超时复核逻辑不变。
- **改动点**：
  - `core/src/pyengine/sidecar.rs:1431-1458`：`check_health` 的 URL 改 `/ready`（或两阶段：`/health` 判进程、`/ready` 判就绪）。
  - `pyengine/src/opensquilla/gateway/app.py:166-174`：`ready()` 加 loop tick 新鲜度；在 `boot.py` 的 `start_gateway_server` 起心跳任务。
- **测试方式**：Python 侧单测（tick 新鲜度纯函数）+ 集成测试：人为 `await asyncio.sleep(30)` 堵循环，断言 `/ready` 回 503 且看门狗在阈值内判死；纯函数守卫防回退成静态 200。
- **风险与兼容性**：低-中。15s 阈值要与"忙引擎被误杀"的保护（3 失手）联调；阈值做成配置项 `MARGUERITE_SIDECAR_STALL_SECS`。
- **是否架构级**：否。

#### P0-3 通配 REST 代理加超时

- **问题/现状**：`sidecar_proxy.rs` 的 `/api/{*path}` 转发用共享 client，**无任何超时**；侧车 hang 时浏览器请求永久挂起（UI 转圈，不是报错）。
- **方案**：`forward()` 加分档超时——普通 JSON 端点 30s；流式上传/下载（`Content-Type: application/octet-stream` 或 `/api/v1/files/upload`、artifact download 类）放宽到 5 分钟或按 body stream 单独配置；超时统一回 502 `sidecar_unreachable`（retryable 契约已存在）。
- **改动点**：`core/src/gateway/sidecar_proxy.rs:167-185`（`forward`）；大文件路径白名单。
- **测试方式**：stub 侧车 `sleep(60)` 再回 → 断言 30s 后收到 502 且错误体为 `sidecar_unreachable`；流式大文件测试确认不被 30s 误杀。
- **风险与兼容性**：低。只会把"永久挂起"变成"快速失败"，与现有 502 契约一致。
- **是否架构级**：否。

#### P0-4 优雅关停窗口（drain before kill）

- **问题/现状**：Python 侧已实现 `server.close()` 排空在飞 turn 与后台 completion，还有 Windows 专用 HTTP shutdown 端点；但 Rust 侧 `stop()`/`Drop`/`kill_on_drop(true)` 从不给它机会——桌面端关闭时进行中的 agent turn 被硬杀。
- **方案**：`stop()` 改为三段：① 先尝试 POST 侧车 shutdown 端点（`app.state.request_shutdown` 已暴露）；② 等待 drain 完成或最多 N 秒（建议 10s，对齐 Python `gateway_shutdown_deadline`）；③ 超时再 `kill()`。`Drop` 路径同样优先优雅（fire-and-forget 等 2s 再 kill）。
- **改动点**：`core/src/pyengine/sidecar.rs:1096-1120`（stop/drop）；确认 Python 侧 shutdown 端点的路由已在 app.py 注册（**[推断]** 需实现阶段确认其 HTTP 路径；若未注册，则退化为"等待 SIGINT 模拟窗口"——Windows 上 `taskkill` 不带 `/F` 先发 WM_CLOSE 语义不可行，备选：spawn 时不加 `kill_on_drop`，由 stop 统一管理）。
- **测试方式**：集成测试：侧车跑一个 5s 的 turn，stop() 后断言 Python 侧日志出现 drain 完成、turn 以正常方式结束而非硬杀。
- **风险与兼容性**：低。仅影响关停路径；关停延长 ≤10s。
- **是否架构级**：否。

---

#### P1-5 子进程内存围栏（Windows Job Object）

- **问题/现状**：侧车可无限吃内存（embedding 缓存、泄漏），拖垮桌面端整机。
- **方案**：spawn 后用 `windows` crate 创建 Job Object，设 `JOB_OBJECT_LIMIT_JOB_MEMORY`（默认 **4GB**，`MARGUERITE_SIDECAR_MAX_MEM_MB` 可配，0=不设）；超限由系统杀进程——正好走已有崩溃取证路径。Linux 分支用 `setrlimit(RLIMIT_AS)` 等价实现。
- **改动点**：`core/src/pyengine/sidecar.rs` 的 `spawn_once`（Windows cfg 分支）；Cargo.toml 加 `windows` 依赖（或复用已有的 win32 调用方式）。
- **测试方式**：单测验证 Job 句柄创建与内存上限写入；冒烟：人为把上限设 200MB 跑侧车，断言系统杀后证据被记录。
- **风险与兼容性**：中。上限过小会杀正常大任务（embedding 索引/build 向量库）→ 默认 4GB 保守、可配置关闭；Windows 8+ 默认嵌套 Job，与桌面端进程共存 OK。
- **是否架构级**：否（但需要真实负载调参）。

#### P1-6 侧车资源指标采集与预警重启

- **问题/现状**：无 RSS/CPU 采集，内存泄漏只能后知后觉。
- **方案**：守护任务每 30s 读一次子进程 RSS（Windows `QueryInformationProcess`/性能计数器；Linux `/proc/<pid>/status`），写入指标并经 `host.runtime` RPC 暴露；趋势规则：连续 3 次采样 RSS > 上限 80% → 主动"健康重启"（drain 后重启，复用现有重启路径）。
- **改动点**：`core/src/pyengine/sidecar.rs`（采样循环）；`core/src/gateway/server.rs` 的 host.* RPC（若已有 runtime 端点则扩展字段）。
- **测试方式**：采样纯函数单测（从 `/proc` 文本解析）；指标暴露的 RPC 契约测试。
- **风险**：低。与 P1-5 配套使用，单独上也安全（只读）。
- **是否架构级**：否。

#### P1-7 复用路径（attach）看门狗自动接管重启

- **问题/现状**：孤儿侧车被杀后只取证、不重启（缺口②），控制台永久降级。
- **方案**：`spawn_attach_guard` 取证后，自动走 `spawn_once` 拉起新侧车（与主路径同环境）；**联动难点**：WS 客户端的 base_url 在 `PyGatewayClient::connect` 时写死，端口变化后需要让客户端重连到新端口——把 `py_client()` 单例改为每次取当前 sidecar 端口（或在重启时重建 client 句柄）。
- **改动点**：`sidecar.rs`（attach guard 加重启分支）；`client.rs`（base_url 动态化）；`gateway/server.rs` 的 `py_sidecar()` 注册逻辑。
- **风险**：中-高。涉及 client 单例生命周期（S7 曾专门消灭"跨代缓存句柄"），改动需谨慎；attach 复用的初衷是"用户可能手动启动了自己的实例"，自动重启前要确认没有用户手动实例的语义冲突。
- **是否架构级**：**是（部分）**——端口动态化是架构联动项，建议在 P0 全部落地后单独立项。

#### P1-8 WS 事件扇出背压（lossy drop-oldest）

- **问题/现状**：256 有界 channel + `send().await` 阻塞驱动循环，event_bridge 慢时请求/响应被事件洪峰 head-of-line block。
- **方案**：channel 改 `tokio::sync::mpsc` 的 try_send 模式或在驱动循环侧做"满则丢旧事件帧、保控制帧"——与 Python 服务端 `lossy/control` 分级对齐；事件类帧可丢（UI 会按新快照重拉），res/控制帧绝不丢。
- **改动点**：`core/src/pyengine/client.rs:64,364-365`；event_bridge.rs 消费侧保持不变。
- **测试方式**：单测：慢消费者（消费 10/s）+ 洪峰（发 1000 事件）→ 断言驱动循环的 req/res 不被阻塞（在洪峰中发一个 call 能在 1s 内返回）。
- **风险**：低。
- **是否架构级**：否。

---

#### P2-9 单 turn 最大预算硬熔断

- **问题/现状**：TaskRuntime 有队列与取消，但缺"单个 turn 最大步数/最大时长"硬熔断（业界"进度熔断"四件套之一）。
- **方案**：Python `TurnRunner` 加可配置 turn 预算（最大 LLM 调用次数、最大 wall-clock），超限按 timeout 正常结束——与现有 `TIMEOUT` 状态对齐。
- **改动点**：`pyengine/src/opensquilla/engine/runtime.py` / `turn_control.py`。
- **风险**：低（纯 Python 侧）；需默认关闭或宽松，避免打断合法长任务。
- **是否架构级**：否。

#### P2-10 WS 重连后事件对账（seq）

- **问题/现状**：Python event 帧已带 `seq`，但断连窗口内事件全丢、客户端不补收。
- **方案**：重连握手后向 Python 查询"最后收到的 seq"，缺号区间由 Python 侧推 snapshot 对齐（而非逐条重放）。
- **改动点**：`client.rs` 重连流程 + Python websocket 加一个 `events.resync` RPC。
- **风险**：中（新 RPC 契约）；收益取决于 UI 对事件丢失的敏感度——评估后可能不做。

#### P2-11 长任务 checkpoint/断点续跑（架构级，后续建议）

- **问题/现状**：在飞 turn 重启即 abandoned（缺口④）。对照 OpenHands conversation 持久化 / LangGraph snapshot / Temporal journal。
- **方案（仅出方案，本轮不实现）**：分两期——
  1. **轻量期**：turn 级进度落盘（`task_id + 已完成步骤 + 副作用清单`），重启后 UI 提示"N 个任务被中断"并提供"重放/放弃"；
  2. **完整期**：引入幂等键（Restate 式 `idempotency-key`）+ 步骤级快照，实现副作用不重放的断点续跑。
- **风险**：高。涉及 turn 执行模型重写与工具副作用账本；与本仓既有"compaction/auto-summary"体系有交集。
- **是否架构级**：**是**——仅记录方向。

### 3.3 推荐本轮进入实现的项（高价值、可测试、低风险）

| 项 | 为什么适合本轮 |
|---|---|
| **P0-1 崩溃循环熔断** | 纯 Rust 状态机改动；有 supervisord 成熟参数可抄；单测易写；直接消灭"空转重启"这一最差体验 |
| **P0-3 代理超时** | 一个函数级改动；把"永久挂起"变"快速失败"；测试只要一个 sleep-stub |
| **P0-4 优雅关停** | 复用 Python 已有的 drain 能力；只改 stop 路径；解决用户可感知的"关应用丢答案" |
| **P0-2 深度探针** | Python 侧小改（15s tick 新鲜度）+ Rust 换 URL；防"活死进程"；注意与既有忙引擎保护联调 |
| **P1-6 指标采集** | 纯只读、单独上安全；是 P1-5 围栏的配套 |

P1-5（Job Object）价值高但需真实负载调参，建议紧随 P0 之后；P1-7（attach 自动重启）涉及 client 单例架构，单列后续；P2-11（checkpoint）是架构级方向，本轮只记录。

---

## 附：诚实边界

1. 本报告"现状"部分均为代码阅读结论（**[已验证]**），未做运行时实测（未启动常驻网关、未跑重型测试，遵循任务约束）。
2. **[推断]** 项：P0-4 的 shutdown 端点 HTTP 路径尚未在本轮逐一核对 app.py 路由表，实现阶段需先确认；P0-2 的"事件循环 tick"任务在 boot.py 的确切挂载点需实现阶段定位。
3. 视频评论区未获取（video_search 工具不返回评论原文）；视频案例的"实战经验"属性来自正文片段与元数据。
4. 业界案例 URL 均为检索返回的公开链接；其中博客类（dev.to、invental.co 等）为第三方经验描述，非官方规范，落地时以官方文档为准。
5. 本报告不引用 `artifacts/` 目录（该目录不随仓库发布）。

---

## 附：侧车加固端到端集成冒烟（2026-10-20 运行时实测）

> 本节补做上文「诚实边界」第 1 条承认的缺口：**真正拉起独立 Rust 网关 + Python 侧车，逐项实测**。
> 环境：Windows，隔离目录 `D:\Downloads\Marguerite\.tmp\sidecar-smoke`，隔离网关 `marguerite-core.exe gateway --port 18899`，
> 同时设 `MARGUERITE_CONFIG_DIR` 与 `MARGUERITE_DATA_DIR`（只设前者无效——见下「隔离坑」）。

### 隔离坑（实测）
- `Config::data_dir()` 读的是 **`MARGUERITE_DATA_DIR`**，不是 `MARGUERITE_CONFIG_DIR`（config.rs:568）。
  只设 CONFIG_DIR 时，网关仍读真实用户 `data_dir` 下的 `sidecar.json`，于是**复用了桌面端正在跑的侧车**
  （实测连到 `ws://127.0.0.1:54024/ws`，pid 23148），导致 host.metrics 一直 `available:false`、且把别人的侧车搞成退出态。
- 两个 env 成对设置后，网关自起干净侧车：python pid 33196、port 55591（parent=网关 pid），pyengine-home 落在隔离目录内，
  health RPC 回 `data_dir` = 隔离路径。

### 逐项结果

| 项 | 结果 | 证据 |
|---|---|---|
| 侧车真实拉起 | **PASS** | 网关自起 python pid 33196 / port 55591（父=网关）；Uvicorn `Uvicorn running on http://127.0.0.1:55591` |
| 看门狗探活打 /ready | **PASS** | 源码 `check_health` 拼 `ready_probe_path()="/ready"`（sidecar.rs:1510/1552），单测 `watchdog_probe_targets_ready_not_static_health`；运行时 `GET /ready`→200 `{"ready":true,"status":"ready","stalled":false}`，`/health`→200 `{"ok":true}`。host.metrics 能采样到值，说明探活把侧车判成了 Healthy（该分类要求 HTTP 探活成功）。注：Uvicorn access log 不打印 GET 行（连手动 curl 也不记），故看不到 `/ready` 的访问行，证据为源码契约 + /ready 200 + 采样成功三者合证。 |
| host.metrics 真实 RSS | **PASS** | WS RPC `host.metrics` 回 `available:true, pid:33196, rss_bytes:400740352, rss_mb:382, uptime_ms:226684`；与手工读 `Get-Process 33196` WorkingSet≈382.2MB 一致。 |
| 通配代理转发 | **PASS** | 经网关 `GET /api/config?token=...`→200，回体内含 `"port":55591`（侧车自己的端口），证明是 Rust 通配 `/api/{*path}` 转发到 Python；`/api/usage`→200。 |
| 优雅关停（侧车 drain） | **PASS** | `POST /api/system/shutdown?token=...`→**202 `{"status":"accepted"}`，1.1s**；该路由 Rust 侧未注册，经通配代理落到 Python 侧车端点（sidecar.rs:1632 `SIDECAR_SHUTDOWN_PATH`），侧车 33196 drain 后退出。随后看门狗把它判为不健康并**自动拉起新侧车 pid 34092 同端口**——崩溃自愈重启链路联动正常。 |

### 边界与发现（不冒充通过）
1. **`/api/system/shutdown` 关的是侧车，不是 Rust 网关本身**：Rust 网关没有该路由，请求被通配代理转给 Python。
   所以 POST 后 Rust 网关进程继续存活是**设计如此**（它是网关关停时用来 drain 侧车的三段式第一步），不是泄漏。
   停止隔离网关用进程终止。
2. **首轮隔离失败根因**：只设 `MARGUERITE_CONFIG_DIR` 不隔离 `data_dir`，会复用他人侧车并污染其退出态。
   多实例/隔离测试必须 **CONFIG_DIR + DATA_DIR 成对设置**（与 config.rs:507 注释一致，本轮实测钉死）。
3. 崩溃循环熔断、事件循环背压、超时分层本轮**按要求未真触发**（有单测覆盖），未实测。

### 总体结论
**PASS**：独立网关真拉起了自有 Python 侧车；看门狗探活目标为 `/ready`；`host.metrics` 返回真实 RSS（pid 33196 / 382 MB）；
通配 `/api/*` 转发到侧车并回侧车自身端口；侧车优雅关停端点 202 accepted 且 drain 后退出、看门狗随即自愈重启。

### 附 2：release 桌面端最终闭环实测（2026-10-20，真实 Tauri 桌面网关 18790 / 0.2.13）

> 对象：已部署的 release 桌面端 `C:\Users\Administrator\AppData\Local\Marguerite\marguerite-gui.exe`（**非 debug**），网关 `http://localhost:18790/control/`，`health` 报 `version=0.2.13`、`data_dir=C:\Users\Administrator\AppData\Local\Marguerite`。
> 非侵入：未播放音乐、未触发熔断、未改任何安全配置；收尾保留桌面端运行。

| 项 | 结果 | 证据 |
|---|---|---|
| 桌面网关自起 Python 侧车 | **PASS** | 全新启动后，当前 GUI 进程（marguerite-gui pid 21772）直接 spawn 出 python 子进程 **pid 40284 / port 64717**，CIM 核对 `ParentProcessId=21772`（即本桌面网关亲生子进程）。 |
| host.metrics 真实 RSS | **PASS** | WS RPC `host.metrics` 回 `available:true, pid:40284, rss_bytes:330653696, rss_mb:315, uptime_ms:111105`。pid 与上面亲生子进程 40284 一致。 |
| 看门狗探活 /ready | **PASS** | 直连 `http://127.0.0.1:64717/ready`→**200 `{"ready":true,"status":"ready","uptime_ms":128293}`**。 |
| 通配代理转发 | **PASS** | 经网关 `GET /api/config?token=...`→**200**（body 13548B），证明 `/api/*` 转发到侧车。 |
| 控制台状态 | **PASS** | 浏览器截图 `desk-sidecar-closed.png`：Marguerite `/chat` 控制台，右上「就绪」徽标，无控制台报错。 |

**过程中的两个插曲（如实记录，不冒充一帆风顺）：**
1. 验证中途，桌面网关（原 21796）自行退出（非本 QA 发出任何停止/关停命令），侧车 40788 成孤儿（父进程已死）。此时 `host.metrics` 一度 `available:false`——**RSS 守卫只采样本网关亲生子进程**；复用孤儿侧车时拿不到有效 RSS。重启桌面端、由当前网关自起全新侧车后，`host.metrics` 立即恢复 `available:true`。这反而实测钉死了守卫的正确性：它不会对一个"不是自己孩子"的进程乱报 RSS。
2. 强制结束孤儿侧车 40788 时，桌面端随后也退出（看门狗未在本会话内自动拉起新侧车）。这是当前桌面版"侧车死→网关随之退出/不自动复活"的行为特征，与 debug 隔离网关的自愈重启表现不同，建议产品侧确认这是否为预期（是否应在桌面端侧车退出后自动 respawn）。

**结论：PASS**。release 桌面网关真实拉起了自己的 Python 侧车子进程，`host.metrics` 返回真实 RSS（pid 40284 / 315 MB），看门狗 `/ready` 200，通配代理 `/api/*` 200。桌面端最终保持运行。
