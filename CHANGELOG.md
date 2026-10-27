# Changelog

本文件记录 Marguerite 的所有重要变更。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

> **版本号单一事实源**：`core/Cargo.toml` 的 `version`（`marguerite-core --version` 打印的值）。
> 四端（core / tui / gui / webui）的版本号由 `python scripts/check-version-sync.py` 强制对齐，
> 并已接入 `bash scripts/verify.sh --all` 门禁。

## [Unreleased]

按 `docs/plans/2026-10-26-capability-cache-computer-use-remediation.md` 推进 W0–W12：门禁复位、双引擎事件契约、占位清理、安全边界、能力路由与缓存口径。

### Added

- **能力卡与能力自述注入**（`core/src/agent/capability_cards.rs`、`capability.rs::brief_from`）：按「意图 → 能力族 → 真实工具」生成确定性文本，进入系统提示稳定半区；不可用能力保留一行说明，团队编排明确「本轮模型无工具」。
- **反脚本守卫**（`core/src/agent/capability_guard.rs`）：当模型用 `shell_exec`/`code_exec` 模拟已可用的桌面/浏览器/文件写能力时，返回一次 `CAPABILITY_MISROUTE` 纠偏（同 turn 每能力一次，`MARGUERITE_CAPABILITY_GUARD=off` 可关）。
- **固定高频工具面扩到 18 项**（`tool_schema.rs::FIXED_FACE`）：文件/浏览器/记忆/知识/技能/桌面/脚本七族首轮可见；首轮不再按请求词裁剪，工具面不随用户文本变化。
- **工作区最近列表**（`workspaces.rs` + 选择器）：`last_opened_at` 排序、置顶优先、单选默认选中最近一条，不再强制弹原生文件夹选择器；列表为空才自动弹一次。
- **i18n 中文硬编码棘轮**（`webui/scripts/check-i18n.mjs` + 基线 18 文件/408 行）：只许下降不许上升，`--update-baseline` 拒绝抬高。
- **core 集成测试入闸门**（`verify.sh` 第 25 项）：`e2e`/`plugin_manifest_gate`/`plugin_patch_gate`/`tui_model_panel` 离线必跑；`third_party/` 缺失时 `third_party_integration` 记 `[SKIP]`（跳过永不计入通过）。
- **诚实缓存指标**：`cache.status` 新增 `server_cache_coverage_pct`、`server_cache_request_hit_rate_pct`、`eligible/hit_requests`；`server_cache_hit_rate` 改为 coverage 别名，旧的 `read/(read+creation)` 仅作诊断。
- **Anthropic 前缀缓存断点**（`llm.rs`）：system / 最后一个 tool / 最后一条 user 三处 `cache_control: ephemeral`。
- **电脑操控结构化矩阵（W6）**：UIA 新增 find/invoke/set_value/type/toggle/select/expand/collapse/scroll/focus/wait/window_activate 与 7 类 pattern 分发（10s 超时、节点上限）；`ActionReceipt` 增加 before/after/verified/effect（observe→act→verify）；`desktop_action` 的 handle 可省略并按 window 附着，新增 `desktop_attach` 工具；网关新增 `desktop.attach` / `desktop.windows` / `desktop.a11y`。
- **能力评测集（W4.7）**：`core/src/agent/capability_bench.rs` 内置 60 条离线路由用例（11 个能力族、55 条「不得脚本替代」负例），输出 JSON / Markdown 报告；真实模型端到端基准仍需 provider key，已在 `scripts/bench/capability/README.md` 注明。
- **命令面板补齐（W8.2）**：`workspace:` / `model:` / `team:` / `agent:` provider 与 `ws`/`工作区`/`模型`/`团队` 别名；只列出网关声明支持的方法。
- **会话级稳定工具面（W4.2）**：`SessionToolFace` 会话内只增不减、按名排序；发现只追加，会话切换重建；首轮面不再随用户文本变化。
- **漂移分域（W5.3）**：`stable_prefix_drift_by_scope` 区分 `agent.stable_half` 与 `compress.tool_results`。

### Changed

- **pyengine 事件透传**（W1）：`session.event.state_change`/`cron_result`/`task_group.*`/`run_heartbeat` 原样到达前端，不再统一拍平成 `status`。
- **工具指引去掉「本轮可调用工具共 N 个」**：该行随发现增长逐轮变化，是 KV 前缀的头号抖动源；工具清单只经 function schema 与能力自述呈现。

### Fixed

- **跨会话事件泄漏**（`event_bridge.rs`）：pyengine payload 不带会话键，转发时注入 `session_key`；否则别的会话的 `state_change` 会把当前聊天凭空拉成 Working/Sending 并冲掉审批卡。
- **假成功占位清理**：`sandbox.resume` 与 `skills.deps.install` 删除 Rust 假实现（在线代理侧车，离线如实 `NOT_IMPLEMENTED`）；`sessions.messages.subscribe/hydrate/snapshot` 不再谎报「已完整水合」，在线代理真实快照、离线返回 `degraded`。
- **远程 SSH 执行需审批**（`collab.ssh.exec`）：复用工具回路的审批链路，批准前绝不 spawn ssh。
- **系统打开加固**（`gateway/open_path.rs`）：工作区边界 + 可执行扩展名/控制字符拦截 + `ShellExecuteW`（不再 `cmd /c start`）。
- **`poker.auto` 座位越界 panic**：空房/损坏座位返回 `SEAT_OUT_OF_RANGE`，不再带走常驻网关（变异测试实测有牙）。
- **限流接线**（`gateway/rate_limit.rs` → 中间件）：只限 `/api/*`，`/api/health` 与 WS/静态资源豁免；超限 429 + `RATE_LIMITED` JSON-RPC 错误体。
- **注入扫描语义化**：只扫命令面字段；「比较 cmd 与 powershell」不再误拦，点击「删除」等危险目标改为二次确认（词边界避免「发送者信息」误伤）。
- **cron wire shape**：在线代理侧车；离线返回数组（`next_run` ISO），修复 App.vue 按数组读导致的「受影响定时任务数恒 0」。
- **workbench 健康检查**：`getHealth` 由 `/api../health`（404）改为 `/api/health`；`/harness/capabilities` 按服务源解析。
- **桌宠 ACL**：`pet_overlay_show/toggle` 补进构建清单与主窗口权限；新增 `pet-overlay` 窗口能力（含回环 remote、拖动/隐藏）。
- **e2e 门禁确定性**：首启 BootOrchestra 由 storageState 预置跳过；deliverables 助手修复水合竞态与 `D:\Downloads\…` 子串误匹配；`verify.sh` 等侧车真正接管 `/api/config` 后再启动 Playwright。

## [0.2.17] - 2026-10-26

补齐「插件 / MCP / 工具 / 技能」四页的独立管理 UI（增删改查 + 启停开关），并把 DSH 来源插件的适配情况变成可审计证据。

### Added

- **`/tools` 工具管理页**（`webui/src/views/ToolsView.vue`）：工具总表 + 来源（内置 / 仅 MCP）与启停状态筛选、
  逐项启停开关、详情抽屉（补丁文件路径、生效边界说明）。开关写入 `tools.patch.set`，`appliesTo: next_turn`。
- **插件管理 tab**（`PluginsView.vue` manage）：17 个内置清单的统计、7 类筛选、详情抽屉（清单原文、诊断、
  声明的能力、可调用的原生 RPC、对应页面）、新建（manifest / URL 安装）、启停、重载、删除（带依赖检查）。
- **`core/src/plugins/adaptation.rs`**：DSH 清单 → 真实 RPC / 页面 的适配映射单一事实源，状态分
  `native` / `partial` / `declarative`，随 `plugins.coverage` 与 `plugins.detail` 一并下发；单测校验
  「映射的 RPC 必须在 `SUPPORTED_METHODS` 中声明」「映射的页面必须存在于 `sharedRoutes.ts`」。
- **网关新增方法**：`plugins.detail`、`plugins.delete`、`mcp.servers.setEnabled`、`skills.update`、
  `skills.setEnabled`（`SUPPORTED_METHODS` 447 条）。
- **技能停用落盘语义**：YAML 与目录型技能改名为 `.disabled` 并写 `.disabled-index.json` 索引；
  扫描器与 `load()` 均跳过停用项，`skills.list` 另返回 `disabledSkills`。

### Changed

- **MCP 服务器新增 `enabled` 字段**（`config.yaml`，缺省 `true`）：运行期连接、工具注册、
  批量探测、能力清单均按启停过滤；`mcp.tools.list` 回传被跳过的服务器名单。
- **插件删除具备可回滚性**：先校验依赖（被依赖时返回 `PLUGIN_IN_USE`），删清单与注册表条目失败时
  恢复原文件并重跑 DI。

### Fixed

- **能力中枢（CapabilitiesHub）跳转整页刷新**：裸 `<a href="/control/...">` 改为 `<RouterLink>`，
  并新增全局 `installInternalLinkGuard` 捕获阶段拦截，任何遗留裸锚点都不再脱离 SPA（不再新开标签 / 整页重载）。
- **安装版 MCP 页误报「网关未声明所需方法」**：视图挂载早于 WebSocket 握手，`supportsMethod` 读到空表；
  三个加载函数改为先 `await rpc.waitForConnection()`。
- **工具页来源列被误标为「扩展」**：改用 `server` 字段区分内置 / MCP。

## [0.2.13] - 2026-10-16

岛屿从「可视化」升级为**可玩的 3D 开放世界**，并补齐资产热加载闭环、会话级成本治理与三处真缺陷修复。

### Added

- **3D 开放世界岛屿**（`webui/src/components/island/world3d/`）：程序化噪声地形 + 水面 + 程序化道具 +
  WASD/PointerLock 漫游；四档模型档位（旗舰推理 / 高效推理 / 极速产出 / 全模态帕累托）映射为粒子预算、
  生成节律、NPC 对话深度、极光强度与时间流速；游戏 ↔ Agent 双向绑定（拾取道具 → 确定性委托提示词入
  outbox，断线积压不假装送达；运行时事件 → 场景脉冲可视化）。`/island-game` 增加
  「3D 开放世界 ↔ 经典岛屿」切换，默认 3D，两种模式互斥持有 WebGL 上下文（切入才建、切出即 dispose）。
- **GLB 资产热加载闭环**：资产规格 → Blender（bpy headless）导出 GLB → 运行时校验（含 Mesh /
  非退化包围盒 / 三角形预算 ≤ 50k）→ 原子替换 + revision 递增；失败或缺失一律回退程序化几何，
  HUD 如实显示「程序化几何 / 已热加载 N 件（rev K）/ 校验失败」。共享几何/材质带实例标记，
  清理时绝不 dispose 共享资源。
- **AI 自主设计契约层**（`world3d/gdd/`）：GDD 机器可读契约 + 手写校验器（枚举收口 / id 唯一 /
  数值边界 / 悬空引用；未知字段仅 warning 前向兼容），整份通过才热加载、失败保持旧配置。
- **会话级思考强度 + 上下文长度持久化**：`sessions` 表新增 `reasoning_effort` / `context_window`
  （幂等迁移），`sessions.patch` 三态语义（缺省不动 / null 清除 / 值设置），会话抽屉内即改即存。
- **git worktree 并行任务编排**：`WorktreeManager` 为子代理分配隔离工作树（RAII 自动回收、
  路径穿越防护、fail-closed 白名单），复用既有并发闸门与预算。

### Fixed

- `gateway.status` 的 doctor 项数断言写死 8，而 `doctor::collect_items_async` 早已新增第 9 项
  `check_tier_caps` → 断言与注释同步为 9，测试改名以免再次误读。
- `gui` 侧 `ModelProfile` 构造缺必需字段 `tier_caps_declared`，导致 `cargo clippy --workspace
  --all-targets` 直接编译失败（E0063）→ 补 `false`（手工添加的档案从未声明档位能力）。
- worktree 的 git 调用只用 `current_dir` 指定仓库、未清 `GIT_DIR` / `GIT_INDEX_FILE` 等继承变量：
  在 git 钩子（pre-commit）里调用时，嵌套 git 会作用到**外层仓库**（实测触发 `reset: moving to HEAD`
  与 `index.lock` 冲突）→ 每个子进程显式清 7 个覆盖变量。

## [0.2.11] - 2026-09-29

修掉**「桌面端默认路径根本没有工具」**这一条最致命的功能缺口，并让完成门禁与退出码开始说真话。

真机取证（隔离网关 + CLI，均为可复现命令）：让助手「在当前目录创建 `ui_probe.txt` 并写入 hello」时，
**事件流里零个工具事件、工作目录零产物**，但助手答「已创建 `ui_probe.txt` 并写入内容 `hello`」，
且 `session.event.done` 的完成门禁给出 `allowed: true / completed` —— 即**既没做事、又报成功**。
根因是 `Orchestrator` 的 Solo 分支默认 `solo_with_tools = false`，走的是无工具的 `lead.process_input`。

### Fixed

- **桌面 UI 默认路径接入工具回路**：`tui/workbench` 的 `execute_turn` 现按运行时模式设置
  `set_solo_with_tools`，仅 **Plan（只读探索）保持关闭**以免绕过「先计划后执行」语义；
  `Orchestrator` 的非计划子步骤走既有 `run_solo_tool_loop` 真实调用 `write_file` 等工具，
  并在其后**补发 `EventType::Done`**（非流式工具回路自身不产事件，否则依赖它收尾的消费方永远等不到完成）。
  修复后同一任务：产物真实落盘、门禁 `allowed: true` 且**确有其事**。
- **子代理工具回路缺 `reasoning_content` 回传（专家团整条线不可用）**：`agent/subagent.rs` 的两处回路
  在回灌 `Message::Tool` 前**没有落承载 `thinking` 的空 assistant 轮**，导致 DeepSeek thinking 模式自
  第二次请求起报 `The reasoning_content in the thinking mode must be passed back to the API`（HTTP 400）。
  现按 `runtime.rs::run_tool_loop` 的既有做法补齐（正常回灌与批次失败回灌**两条路径**都补）。
  修复后 `team` 的子专家由「400 失败 + 首席答未完成」变为**全部完成**。
- **子代理产物记错会话**：`SubAgentManager` 新增会话传递，两处 `tokio::spawn` 内以 `SESSION.scope(...)`
  包裹执行体（task-local 不跨任务继承），编排器 `set_event_bus` 同时接线。此前子代理内文件工具登记产物时
  `current_session()` 为空，产物被记到默认会话，于是「本次会话产物列表」恒空（真机告警
  `record_artifact: 无可用 SESSION`）。
- **`team` 失败仍退出 0**：`team_handler` 现读取 `out.completion_verdict`，未放行时以既有分层退出码
  `Conflict(9)` 非零退出（此前只打印、随后静默 `Ok(())`，使「跑完了但没做成」与「真做完了」无法区分）。

### Tests

- 新增 `subagent_tool_loop_passes_reasoning_content_on_followup_request`：假 provider 捕获**第二轮**请求体，
  断言含 `reasoning_content`（变异自检：移除插入即变红）。
- 新增 `subagent_artifact_lands_in_caller_session`：断言子代理产出登记在**调用方会话**下
  （变异自检：去掉接线即变红，实际条目为空）。
- 新增 `test_team_failure_exit_code_reflects_completion_verdict`：锁定放行 → 常规退出、未放行 → 9。

### Verified

- `cargo test -p marguerite-core --lib`：2911 passed / 0 failed。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告。
- `bash scripts/verify.sh --all`：**全绿 25/25**（含 `webui e2e 95 passed ≥ 基线 95`）。
  （首轮 e2e 曾报 94 < 95；用同一二进制同参数独立复跑得 95，判定为偶发。）

### Known

- 完成门禁的 `PlacationDetected` 仍可能误判：`extract_claimed_artifacts` 只收含 `/` 或 `\` 的路径，
  裸文件名（如 `e2e_probe.txt`）被过滤 → 判据为空 → 有产物也可能被判「无产出」。未改（该判据同时用于拦截
  「声称却没落盘」，属产品取舍）。
- UI Solo 路径为避开流式回路对非 SSE 响应的空解析，仍走非流式，故暂无实时工具卡/正文增量。
- CLI 的 `-p` / `chat` 单发仍无工具（本轮只修桌面端主路径）。

## [0.2.10] - 2026-09-27

桌宠**从 WebGL 渲染器整体重做为「2D SVG + 伪 3D 光照」**，五款皮肤形象全部重新设计。

同时修掉**阻断任务执行的一整条链路**（真机取证：会话里模型手上只有 1 件工具、联网检索结构性不可执行，
且同参调用 3 次即中止整轮），并落地架构评审的 16 项（就绪单一事实源、persona 全链路、doctor 探测档位、
补订事件、结构化步骤错误、模式中文名、三档预设、计划复用、先问再做、runStatus 词表、Config 缓存、
TIMEOUT 分类、历史工具耗时、等待指示器）。

### Changed

- **渲染器换代**：`Pet3D.ts`（Three.js/WebGL，1848 行）与其 WebGL 粒子池退场，取而代之的是
  `petSvg.ts`（矢量分层 + 伪 3D 光照）与 `petSvgColor.ts`（材质明暗派生）。三处宿主
  （`DesktopPet` / `PetView` / `PetOverlayView`）统一切换，公开 API 与旧渲染器同名同义。
  **动画数学一行未改**：逐帧位姿仍由 `petPose.computePose` 这个纯函数引擎产出（呼吸/漂浮/
  眨眼/耳抽动/尾摆/翼振/光环/蹦跳/揉眼/部位点击衰减/悬停聚焦），渲染层只做「值 → SVG 变换」。
- **立体感来自四条可核查的光照约定**（而非"画得像"）：① 全局同向主光（左上受光、右下暗面、
  取主题色作背光轮廓光），每个材质由 `rampOf` 派生 light/base/shade/deep/rim/line 六层；
  ② 环境遮蔽（眼窝、刘海投影、爪下、身体贴地）；③ 接触投影（随跳跃高度缩放淡出）；
  ④ **伪 3D 视差**：头部 yaw 不只是转角——五官整体横移、远侧耳与后发反向偏移、头部横向压缩；
  pitch 压缩纵向并下移五官。
- **皮肤形象重设计**：兽形（茉莉 · 云绒猫）圆头尖耳/暖粉内耳/绒球尾/项圈铃铛；人形（四款精灵
  花嫁）长波浪发（暗部大形→主色→发梢→天使环高光带）、三层花瓣嫁裙 + 薄纱拖尾、四片蝶翼
  （带尖角与翅脉，上下分片留有缺口）、精灵尖耳、花冠（每皮肤花型不同）、捧花。冷暖对比由
  暖粉内耳/腮红与冷调毛色构成。
- **特效与交互**：`burst()` 用 Web Animations API 播完即移除（星/心/花三形态，预算按渲染档位
  缩减，不常驻 DOM）；`data-mood` / `data-behaviour` 落到 SVG 根上，CSS 与测试可直接观察；
  指针视线跟随（`setPointer` / `lookAt`）与悬停聚焦幅度沿用既有系数。
- **诚实边界**：`PET_CAPABILITIES` 不再宣称 WebGL 3D / toon 着色（如实改为 SVG / 伪 3D）；
  `PET_QUALITY` 档位语义改为「只调特效预算（模糊半径、粒子数），不影响形象完整度」；
  无 WebGL 环境也能完整渲染（不再有「精简模式」降级分支）。顺带清掉随渲染器退场而失去
  执行路径的常量（粒子池观感基线、WebGL 贴图色）。

### Fixed

- **下半身整体跑到画布外**（截图实证）：身体组的变换写成 `translate(a) scale(s) translate(-a)`，
  那是「绕 a 缩放」的写法，只适用于子节点已画在父级绝对坐标里的情形；身体子节点画的是组内
  局部坐标，于是 s≈1 时整段变换退化成恒等 —— 猫的身体/爪、精灵的裙/腿/鞋/捧花全被画到画布
  左上角（表现为「头浮着、身子不见了」）。现改为保留 translate 基位。
- **双耳被头部轮廓盖住**（截图实证）：耳锚点落在头圆内部且画在头圆之前，剪影上等于没有耳朵。
  现锚点移到头轮廓外侧上方并把耳叠在头圆之上。
- **SVG 尺寸溢出**：只给 `viewBox` 时浏览器会退化成默认尺寸（实测把 200 单位图撑成 806px 高、
  下半身被面板盖住）。现显式给 `width/height=100%` 与 `preserveAspectRatio="xMidYMax meet"`
  （脚底/地面恒贴容器下沿，桌宠是"站在"格子里而不是浮空）。
- **内耳/腮红用错色**：从冷调 `furAccent` 派生，内耳呈灰蓝（应是暖粉）。现改用暖粉色基。

### Fixed（执行链路：本轮真机取证后治本）

- **锚定工具面被幽灵别名锁成 1 件工具**。真机会话里模型自述「实际可调用的工具只有文件编辑器，
  `web_search` / `web_fetch` / `web_research` / `browser` 均未挂载」——根因是锚定最小工具集与 PTC 别名
  指向了**注册表里不存在的名字**（`bash_persistent` → 真名 `shell_exec`，`run_code` / `code` → 真名 `code_exec`），
  收窄后只剩 `str_replace_editor`。现别名一律映射到真实注册名；新增测试**以真实注册表为数据源**校验锚定面
  （旧的伪造 `ToolDef` 用例正是掩盖此缺陷的原因）。
- **`Full` 阶段仍只给 3 件工具**。`bloom_anchor_tools` 原注释写着「完整面由调用方追加」，而三个调用方
  （`dispatch_bloom` / auto / swarm）都只是直接赋值 —— 阶段升到 Full 也永远只见 `minimal ∪ {ptc_tool}`。
  现 `Full` 阶段**原样返回全工具面**。
- **`expand_to_full` 在生产路径没有任何调用点**（只有测试调用它），锚定态永久停在 PTC。现补两个生产触发点：
  ① PTC 越过宽容轮数即扩展；② 任务声明联网检索而当前锚定面**没有网络工具**时立即扩展（携带交付物的检索类任务
  已被 web 快路拒之门外、必须走工具回路产出真实产物，此时缺网络工具只能退化成「模型自述无法联网」）。
  红线不变：工具面**单调扩展、只加不撤**。
- **同参重复 3 次即中止整轮**（真机真现场「本轮失败：工具 `str_replace_editor` 连续调用 3 次，疑似死循环，已中断」）。
  改为两档：3 次**注入纠正提示但不中止**（纠正提示写进工具结果，模型下一轮必然读到），5 次才判定卡死中止。
  单工具锚定面下「同一编辑工具连续调用 3 次」是正常轨迹，不该炸掉整轮并降级为纯文本。
- **工具卡堆叠十几行、秒数一直涨**。后端 `tool_call` / `tool_result` 帧不带调用 id，前端拿空串匹配必然落空 →
  每次结果都新建一条，而原来那条 running 记录永不收敛。现后端把**模型给出的调用 id** 透传到 WS 帧
  （`AgentEvent::ToolCall` / `ToolResult` 新增 `id`），前端按「显式 id → 同名最早未定案调用（FIFO，与后端顺序执行一致）
  → 才新建」配对；兜底身份改用**单调计数**，不再用时钟当身份（时钟有两个反向病：吞并同毫秒同名调用、跨毫秒又把结果另起一条）。
- **计划失败文案插值错位**：`已完成 {} 个步骤，第 {} 步失败后停止` 的第二个占位收到的是
  `step-3 [演示文稿] 生成 PPT（presentation.pptx）。` 这类长标签，被拼成病句。
- **状态显示矛盾的四处**（并发会话真机截图实证）：「正在工作 · 0s」与「思考中 · 63 秒」并存（双计时源）、
  状态栏「已用 1m 16s」长时间冻结、右侧产物面板常驻空态（磁盘上 `output.html` 与 `ai_news/step1_检索方案.md` 确实落盘）、
  长时间等待无指示器。

### Added（架构评审 16 项）

- **就绪状态单一事实源**：`provide/inject` 字符串 key，对 per-chunk 模块拷贝免疫 ——
  这是与已失败的「Pinia」「模块单例」两条路线的本质区别（换容器不解决双实例）。
- **persona 全链路**：专家人设/徽标在会话头与思考链上可见。
- **`doctor.status` 暴露真实鉴权探测档位**：复用 CLI 既有的 `GET /models`（401 = 密钥被拒）探测下沉复用，
  不再出现「假绿」；对 `gateway/methods.rs` 的触碰压到最小（扩展参数，不新增 RPC 方法，不动 protocol / rbac）。
- **补订三类此前被丢弃的事件**：记忆召回 / 步骤迭代 / 流水线进度。
- **结构化步骤错误**（`core/src/errors.rs`）：治理门 / 宪法 / 反思门禁按**类型**裁决（可 downcast 的 `StepError`），
  替代「中文子串嗅探」；provider 归类优先于文本子串 —— 修掉「恰好含『治理门』字样的瞬时错误被误判不可重试」，
  同时保留子串兜底与迁移告警（未迁移的确定性错误行为不变）。
- **模式中文名**（安全探索 / 日常协作 / 多候选融合 / 深度打磨 / 全知超验；技术名进 tooltip）；
  引擎侧计划文案本地化不随本版。
- **三档预设**：简单 = plan + 只读沙箱 + 关路由｜标准 = agent + 工作区写 + 融合路由｜专家 = bloom + 网络 + 蜂群（集中单文件，可随时改）。
- **计划复用**：16 位归一化哈希指纹去重，命中**候选**随 `plan_revision` 事件下发、由用户点「继续上次计划」触发，不静默复用。
- **先问再做**：平台 / 技术栈 / 交付形态三项触发，≤3 问，每问带可跳过的默认值。
- **runStatus 词表收口**与 **Config 缓存**（计划路径）。
- **TIMEOUT 归入结构化错误分类**（`error_taxonomy`），前端错误文案随之统一。
- **历史工具耗时**：core 全链落地（`elapsed_ms` 列 + `PRAGMA` 幂等迁移 + `chat.history` 下发，多工具批不臆造归因）
  —— **前端历史工具卡的耗时消费端未随本版**（历史工具行目前仍是文本行，见本版交付报告的已知缺口）。
- **等待指示器**（长时等待的心跳提示）。

## [0.2.9] - 2026-09-26

新增**元智能体自迭代闭环**：工作台不再只执行任务，还能「感知失败 → 定位根因 → 提出改进 →
沙箱裁决 → 版本化落地/回滚」。移植自外部方案的五层架构（监控 / 反思诊断 / 改进规划 /
安全沙箱执行 / 评估闭环），并把该方案的三条避坑点固化为**类型与门禁**，而非文档承诺。

### Added

- **元智能体五层闭环**（`core/src/harness/meta.rs`）：从真实行为台账抽取**带类型的失败信号**
  （`agent_self` / `tool_fault` / `tool_insufficient` / `capability_gap`）→ 四类根因裁决
  （规划逻辑弱 / 工具参数错 / 工具脚本缺陷 / 能力缺口，带证据与置信度）→ 白名单化改进规划
  → 回放对照四闸裁决 → 版本快照 + 灰度 + 回滚。无信号时**不产出提案**，不编造。
- **`selfevolve.*` RPC 族**（`core/src/gateway/selfevolve.rs`，9 个方法）：
  `status` / `signals` / `analyze`（只读预演）/ `history` / `sweep`（登记提案）/
  `cycle`（带真实回放数据的完整一轮）/ `approve` / `reject` / `promote`（灰度→全量）/ `rollback`。
  全部为 Rust 原生确定性实现，侧车离线时同样可用。
- **自进化操控台**：`/evolve` 页面新增元智能体面板（策略与阈值、生效版本、待审提案的
  批准/否决、灰度与转全量、回滚、失败信号、版本史），并入主导航「观测」分组；原有
  prompt 自进化历史保留在同页。
- **能力登记**：`res.meta_agent`（`core/src/domain/capabilities.rs`）。

### Safety（方案里的「避坑点」用代码固化）

- **不许碰核心源码**：`ChangeTarget` 只枚举 system prompt / 任务规则 / 工具默认参数 /
  沙箱内脚本 / 知识库 / few-shot 六类，**结构上不存在**「改 core 源码」这一变体
  （护栏测试 `core_source_is_not_a_change_target` 断言）。
- **默认不自动上线**：`MetaPolicy::auto_apply` 默认 `false`（方案中的「路线 A」），
  提案只登记、进待审队列；人工批准后才可灰度。`selfevolve.*` **无任何修改真实配置的入口**。
- **成本进闸**：采纳不只看「跑得通」，还看 token 与时延相对基线的涨幅
  （默认 +20% / +25%），并有「无回退」硬闸（通过的用例数不得少于基线）。
- **「未测量」≠「测得 0」**：`VersionSnapshot::baseline` 为 `Option`，提案阶段如实为 `None`；
  有候选却无基线时**拒绝裁决**（`candidate_without_baseline_is_refused`），
  不拿默认值凑一个「看起来通过」的结论。
- **回滚有据**：阶段推进沿用同一版本号（版本是改动的身份，推进不是新改动），
  台账只追加不修改；回滚后可审计地落回更早的全量版本，从未全量过则如实报错。

### Fixed

- **长回合被前端误杀（Rust 侧从不发保活心跳）**：`session.event.run_heartbeat` 此前
  **只有 Python 侧车发**，而前端的硬空闲超时（630s 收不到任何帧即判死并掐断本轮）与该事件
  是配套设计。侧车离线/未安装时由 Rust 驱动回合，长工具执行或长 provider 等待期间一个帧都
  不发 → 健康的回合在 10.5 分钟后被掐断。现在 Rust 网关在回合期间按 20s 周期发
  `phase: "turn"` 的保活帧（相位无关的 keepalive，不覆盖活动文案），并由 `Drop` 守卫保证
  心跳**绝不活得比回合久**（回合结束/中止/panic 展开都会停）。回归护栏驱动真实 `chat.send`：
  用一个「接了但永不回复」的 LLM mock 让回合停在 provider 等待上，断言 ≥3 帧保活、
  载荷携带真实 session/task_id/phase，且中止后不再有帧。
- **`onboarding.llmProfile.active.remove` 是假移除**：它清空内存里的 key 再 `save()`，但
  `api_key` 带 `skip_serializing`、YAML 结构性不落钥，下次 `Config::load()` 会从 `.env`
  原样读回来 —— 返回 `removed: true` 而引擎照旧用旧钥鉴权。新增 `Config::remove_env_key()`
  真正删除 `.env` 中该行（保留其余键与注释），并在响应里如实回报 `keyRemoved`。
- **失败轮次被一条假超时盖住**（真机截图：401 之后又出现
  `Response timed out -- no events received for 630s`）。一轮以 `session.event.error`
  收尾后流已结束，但随后一条迟到的 `state_change` / `ensemble_progress` / `run_heartbeat`
  帧会把渲染重新拉起并重新武装空闲看门狗（630s）——而后端那一轮已经死了，没有任何生产者
  再发事件，用户只能干等满 10.5 分钟。现在已结算的回合（`FINISHED`/`STOPPED` 哨兵）
  **不再被迟到帧复活**，迟到帧仍会登记 seq 与运行态（不丢信息）。变异自检：把守卫改成恒假
  → 2 条护栏变红。
- **`run_status: "error"` 被静默归一成 `idle`**：网关 `sessions.changed` 文档声明取值为
  `running`/`idle`/`error`，而两份 `normalizeRunStatus`（`composables/useSessions.ts` 与
  `composables/chat/useChatRunStatus.ts`，同一份词表分叉成两处）都漏了 `error`，落到兜底
  `idle`——失败轮次在列表里显示「空闲」，`sessionChangeIsTerminal()` 也判不出终态。现如实
  映射为 `failed`（有本地化文案 `chat.status.failed`）。
- 版本号漂移：`README.md` / `README.zh-Hans.md` 正文的引擎版本声明统一到 `0.2.9`；
  `scripts/check-version-sync.py` 新增对 README 正文版本声明的校验（此前门禁全绿也没抓到
  这次漂移）。

### Notes

- **发布关系（如实）**：上面 `[0.2.8]` 条目记录的 5 个 P0 + 4 个 P1 修复是在 0.2.8 安装包
  **已产出之后**才合入工作区的，因此**未随已发布的 0.2.8 安装包发货**。`0.2.9` 是第一个
  同时包含「元智能体自迭代闭环」与那些修复的安装包。已装 0.2.8 的用户看到的仍是修复前的
  行为（鉴权失败不降级、专家全禁用、Fleet 连接竞态等）。
- **仍未验证（不拿单测冒充）**：元智能体闭环的端到端表现需真实运行数据积累——首次
  `selfevolve.sweep` 的输入是既有行为台账，无信号时不会产出提案；「沙箱回放」的基线/候选
  数据须由调用方真实回放产生，本版本不自动生成。

## [0.2.8] - 2026-09-26

修复外部对已装 0.2.7 深度测试报告中**已逐条核实**的缺陷（5 个 P0 + 4 个代码级 P1）。

### Fixed

- **鉴权失败不再抛上游原始正文**：上游 provider 失败按类别归类为稳定错误码
  （`AUTH_FAILED` / `RATE_LIMITED` / `NETWORK` / `SERVER` / `OTHER`，见
  `core/src/model/error_taxonomy.rs`）。此前 `chat_complete` 把上游响应正文原样拼进错误
  （DeepSeek 鉴权失败回显形如 `Your api key: ****gEni is invalid`）并透传到前端；现用户
  只看到本地化文案，原始详情仅入日志与可折叠「技术细节」。故障转移链在**全部候选同因**
  失败（如全部 401/403）时返回该稳定错误码，不再降级成一坨泛化文案。
- **鉴权失败触发离线降级**：CLI 单指令模式的离线回显（`scripts`：`needs_offline_echo`）
  触发条件从「未配置密钥」扩展到「密钥被上游拒绝（401/403）」，并以专用提示行如实标注
  「鉴权失败」，不谎称「未配置」。
- **前端错误本地化**：`webui/src/utils/chat/errors.ts` 按错误码给出本地化文案；鉴权失败
  的对话错误卡提供「前往设置」+「重试」两个操作。新增文案 6 locale 齐全。
- **Fleet 页连接失败**：`FleetView` 在 WS 仍处于 `connecting` 时显示骨架并等待连接就绪
  （不再把连接竞态误报为 `Cannot call fleet.list: not connected`），就绪后成功加载；
  真失败仍可见（不吞错）。
- **版本号漂移**：`README.md` / `README.zh-Hans.md` 正文的引擎版本声明统一到 `0.2.8`；
  `scripts/check-version-sync.py` 新增对 README 正文版本声明的校验（此前门禁全绿也没抓到
  这次漂移）。
- **12 个内置专家默认禁用**：`AgentRecord::Default` 让 `enabled` 落为 `false`，导致
  `personality_for()` 对内置专家直接返回 `None`、人格永不注入。现显式启用 main +
  code-engineer / researcher / data-analyst / code-reviewer，其余保持禁用。
- **计划子步骤输出格式无约束**：计划子步骤注入固定输出结构约束（固定小节标题
  `## 结论 / ## 依据 / ## 产出 / ## 风险与假设` + 固定表格列名），减少同类任务在不同会话
  里的格式漂移（正文由 LLM 生成，属提示约束而非强保证）。
- **复杂任务全路由 General/T0**：规则路由此前对「无 persona 命中」的输入一律降级到
  General/T0（日志实测 2140 条中 2138 条为 General）。现按可测的复杂度信号（长度 / 多步骤
  结构 / 复杂度关键词）把复杂任务提升到更高档 + T1，简单闲聊保持 General/T0。

### Notes

- **产品取舍类未改动**：`bloom.parallel_enabled` 默认仍为 `false`（蜂群并行是 Bloom 模式的
  卖点但默认关闭，属成本/产品决策）；改为在 UI 如实标注「蜂群并行当前关闭」，不名不副实。

## [0.2.7] - 2026-09-24

修复三个用户可见故障（装在真机的 0.2.6 首启即出现：会话历史恢复失败横幅、
侧栏「无法加载任务」、以及助手只回一句 `计划执行结束 (not_armed)。`）。

### Fixed

- **交互式请求被残留目标截断为 `not_armed`**：`GoalRunnerGuard` 此前只要注册表里存在任何
  目标记录就接管护栏（`from_registry` 无条件取 revision），而按语义「armed 为会话级、不落库」——
  进程重启/会话恢复后目标恒为 disarmed，于是**每一次普通对话**都在第一步之前被
  `GoalStopReason::NotArmed` 掐死。现改为只在目标**已激活**（armed）时才接管；
  护栏一旦授予，运行中的人工解除激活仍会即刻拦停（授权语义未削弱）。
- **收尾语泄露内部枚举**：面向用户的回复不再出现 `not_armed` / `stale_revision` 等实现细节，
  改由 `plan::runner::stopped_reason_text` 给出可读中文原因。
- **单测污染真实用户目标库**：`goal` / `runner_guard` / `gateway::goal` 三处测试此前直写
  `~/.marguerite/goal.db`（累计 900+ 行夹具行，含进程级环境变量隔离在多线程下的竞态漏写）。
  进程重启后 `restore_from_sqlite` 取最新一行，正好是夹具 `objective`——这正是上面那条
  `not_armed` 的源头。现全部改走 `Config::set_config_dir_override` 的线程局部临时目录。
- **控制台读失败不再永久卡死**：会话列表与聊天历史的初次读失败（首启瞬时不可用、请求超时）
  现在具备有界自愈——会话列表按 1s/3s/8s 退避重试，历史按 4s/15s 重试；预算耗尽后
  仍保留原有的手动重试按钮，不隐瞒失败。

## [0.2.6] - 2026-09-22

发布就绪冲刺：把仓库从「内部开发态」提升到「陌生人能 clone、能构建、敢用」的状态。

### Added

- **开源发布要件**：`LICENSE`（MIT）、`.gitattributes`、`CONTRIBUTING.md`、
  `SECURITY.md`、`CODE_OF_CONDUCT.md`、PR 模板、3 个 issue 表单。
- **CI/CD**：`.github/workflows/ci.yml`（docs 漂移 / webui 构建 / Rust 测试与 clippy 三 job）
  与 `.github/workflows/release.yml`（tag 触发，三平台 matrix 产出二进制并发布 Release）。
- **版本一致性门禁**：`scripts/check-version-sync.py`，把 7 处版本号钉到 `core/Cargo.toml`，
  支持 `--print` / `--fix`，并接入 `verify.sh` 的 `--core` 与 `--all` 两条路径。
- **双语文档**：英文主 `README.md` + 中文 `README.zh-Hans.md`（仓库根，即 GitHub 约定的
  `README.<locale>.md` 位置），并新增 `docs/README.md` 作为文档索引。
- **`--pipe` 的有界读取内核**：新增 `core/src/cli/pipe_input.rs` 的 `read_bounded()`，
  多行输入完整读取，空输入快速失败而非挂起。
- **桌面端 WebUI 产物缺失诊断页**：`/control/` 在产品缺失时返回 503 + 自包含诊断页
  （`x-marguerite-webui: missing`），并列出已探测路径与三条修复动作。
- **`doctor --probe`**：显式的联网鉴权探测项（默认离线、不联网）。

### Changed

- **`--help` 补全子命令清单**：此前只列 14 项而实际注册 19 项，
  `sandbox` / `subagent` / `team` / `browser` / `capabilities` 用户无从发现。
  新增两条对账测试，防止再次漂移。
- **内置 baota 工具名规范化**：`baota.system_info` → `baota_system_info`（14 个）。
  点号名在 MCP 注册表里属非法字符，此前每次注册都会触发「自动修正」并刷 WARN；
  `docs/mcp.md` 本来就写的是下划线形式，属代码漂移。旧点号名保留为可派发别名。
- **`doctor` 的 `api_key` 项诚实化**：由「非空即 OK」改为「已配置（仅存在性检查，未校验有效性）」，
  不再把「配了一个无效 key」报成通过。
- **桌面端就绪判定**：由「TCP connect 成功即就绪」改为真实 `GET /control/` 并要求 200，
  不再出现「日志说已就绪、界面其实 404 空白」。
- **`verify.sh` 的 fresh-clone 前置**：`webui/dist` 是 Tauri 声明的打包资源且被 gitignore，
  全新 clone 直接 `cargo build --workspace` 必然失败（exit 101 `resource path 'webui\dist' doesn't exist`）。
  现在任何会编译 `marguerite-gui` 的检查（clippy `--all-targets`、workspace 构建）都会先自动补齐该产物。

### Fixed

- **`--pipe` 永不返回**：根因不在 stdin 读取，而在 `core/src/cli/argv.rs` —— `--pipe` 只置位
  `args.pipe`，未把 `mode` 从默认 `CliMode::Interactive` 提升为 `CliMode::Direct`，
  于是走进了 TUI 分支、`handle_direct()` 根本没被调用（真文件重定向亦同样挂起）。
- **CLI 无密钥时的离线回显**：`--help` 早已承诺「未配置 API Key 时自动降级为离线回显」，
  实际却直接报错退出。现已真实落地：本地规则编排判定 + 输入回显，退出码 0，且不伪造模型回答。
- **调试探针泄漏到用户输出**：`[PROBE-cache]` / `[CACHE-OBS]` / `[PROBE-cc]` 默认静默，
  仅在 `MARGUERITE_DEBUG_PROBE=1` 时输出。
- **首次运行 stderr 噪音**：14 行 MCP 工具名「非法字符已自动修正」WARN 不再出现（根因消除，非仅降噪）。
- **Web 控制台 17 个失败单测**：真实根因是 `en.json` 落后其余 5 个语言 **101 个键**，
  组件解析不到键时会渲染原始键字符串，表现为按钮 `aria-label` 变成键名、`querySelector` 返回 null，
  看起来像 popover 组件损坏。已补齐 6 个语言的真实翻译（各 3834 键全等）。
- **WebUI 颜色/圆角架构守卫**：`EscalationCard.vue` 的 14 处裸色值与 2 处硬编码圆角改为语义令牌。
- **core 测试非确定性**：`events::product` 的用例使用**固定路径**的跨进程共享临时 DB，
  且 `LOG_PATH` 只能注入一次；并发 `cargo test` 时 `remove_file` 在 Windows 上静默失败、
  残留行污染断言。现改为每用例唯一路径（pid + 序号）+ 增量断言，生产路径行为零变化。
- **`gateway::chat` 复杂任务用例负载敏感失败**：等待窗口由 120s 提高为 300s
  （可用 `MARGUERITE_TEST_PLAN_WAIT_SECS` 覆盖）。**未放宽任何断言**——
  三条终态断言与落库断言一字未改；该用例独立运行 36s 即可确定性通过。

### Security

- **移除硬编码真实凭据**：`smoke_orchestrate.sh` 内嵌的真实 API key 改为从环境变量读取，
  缺失即拒绝启动（exit 5），不再存在内置默认密钥。
- **删除文档中的 AIGC 生成内容标识 frontmatter**（5 个 `docs/*.md`），正文逐字节未变。
- 确认直接依赖（tokio / reqwest / axum / ratatui / tree-sitter 等）均为 MIT / Apache-2.0 / BSD 类
  宽松许可，无 GPL/AGPL，故采用 MIT 发布无兼容性问题。

> ⚠️ **发布前必须处理**：上述真实 key 仍存在于 git 历史（提交 `9362b1f7`）。
> 仓库当前无 remote、未外泄，但公开前需重写历史（`git filter-repo`）并轮换该密钥。

---

## 历史内部开发日志

> 以下为项目公开前的内部迭代记录（v1–v32 等编号为当时的过程性编号，与语义化版本无关）。
> **内容按原样保留**，其中标题行可能残留早期工具写入时产生的编码损坏（`·` 显示为 `??`），
> 属历史记录保真，不做改写。权威变更以本文档上方的 SemVer 条目为准。

Version v0.1.5 · Build 2026-08-23

## v32 · 2026-08-23 — 阶段 E + 模式→UI 深化
- **全局模式徽标（顶栏）**：新增 `ModeBadge.vue`，订阅共享 `useActiveMode`，在顶栏实时显示当前运行时模式（带彩色状态点 + 模式标签）；点击跳转 `/control-center` 切换，右键循环切换模式。与 ChatView/EngineHealth/CommandPalette 同源刷新（经 `session.event.mode.changed`）。
- **E 设置与导航去重**：`CapabilitiesHubView` 的 23 张重复卡片收敛为 11 张——9 项「设置内已实现」入口（网关连接/AI 配置/偏好/行为/隐私/沙箱/记忆/键盘/高级）合并为单一「设置」卡片深链 `/settings`；保留与权威页一一对应的独立页面（engine/plugins/expert-center/wallpaper/pet/graph/collab/terminal）；移除 `window.alert` 兜底，改用 `<a>` 直链 + 路由跳转。
- **i18n**：补充 `chrome.modeBadge` 六语言键（`模式：{mode}`）。
- **验证**：vue-tsc 零错 · vite build 成功 · vitest **3113 passed**（ModeBadge 3 新增）。

## v31 · 2026-08-23 — 阶段 C：缓存语义层默认策略可配置
- `core/src/cache/semantic.rs`：语义层默认开启策略改为 env 驱动（阶段 C）。
  - 新增 `parse_semantic_env_policy(raw)` 纯函数 + `semantic_default_enabled()`；读 `MARGUERITE_SEMANTIC_CACHE`：`off`→关闭，`on`/`auto`/未设→开启（默认开启保持契约）。
  - `with_embedder`/`new` 改用 env 策略；`with_shared_exact`（网关）仍硬 `false`（网关精确优先契约独立成立）。
  - 补 5 Rust 测试：env 四态（off/on/unset/auto）+ 语义开启下近似 prompt 命中且 ExactOnly 不被污染（诚实性）。
  - 既有 `shared_exact_semantic_off_by_default` 测试改用 `.with_semantic_enabled(true)` 锁定，避免依赖进程 env（并行竞态）。
- **不变量**：缓存命中率仍诚实计量（不冒充 100%）；语义近似仅在 `TierPolicy::Full` 且显式开启时参与，工具路径 `ExactOnly` 永不被污染。
- **验证（待全量回归确认）**：semantic 模块 20 passed；全量 `cargo test -p marguerite-core --lib`（target-dir D:/tp_build_fresh1）基线 2177 不回退。

## v30 · 2026-08-23 — 阶段 D：模式能力→UI 实时映射
- **新增 `ModeHintBar.vue`**：模式→UI 映射的单一来源（5 真实模式 plan/agent/ensemble/bloom/omni_transcend 的标签+能力提示），订阅共享 `useActiveMode`，切换即时变。
- **ChatView 接入**：thread 顶部插入 `ModeHintBar`，对话页现在显示当前运行时模式 + 一键切换（此前完全不感知模式，是信息孤岛）。
- **EngineHealthView 去本地轮询**：`mode` → 共享 `activeMode`，`switchMode` 走 `switchActiveMode`；移除本地 `mode.get` 水合（App 统一 `initActiveMode` 水合），消除与实时广播的竞态。
- **CommandPalette 加 Modes 组**：5 个模式切换命令，当前模式标记 `modeActive` 副标题；补 `shared.cmdp.groupModes`/`actionModeSwitch`/`modeActive` 六语言键。
- **不变量**：消除"只有 ControlCenter 知道当前模式"的信息孤岛；模式切换经 `session.event.mode.changed` 全局即时联动（ChatView/EngineHealth/CommandPalette/ControlCenter 同源刷新）。
- **验证**：vue-tsc 零错 · vite build 成功 · vitest 3110 passed（ModeHintBar 4 新增）。

[0.1.5-dev] - 2026-08-23 · 全景总纲 + 图标系统统一 + 模式实时联动（v29）

Added
  deliverables/upgrade-masterplan.md → 全景分析 + 升级总纲设计文档（单一事实源）：现状拓扑 / 能力矩阵 / 已落地项(v22-v28) / 待升级优先级 / 逐功能路线图 / 不变量守关门
  utils/icons.ts → 追加 8 个语义专名图标（mode/model/plugin/capabilities/knowledge/pet/collab/swarm），消除 gauge/sparkles/agents/blocks 的跨概念混用
  utils/icons.test.ts → 图标注册表守卫：每个 IconName 有非空 path + star 已移除 + 专名图标互不形同
  composables/useActiveMode.ts → 全局共享运行时模式 store：init() 水合 mode.get + 订阅 session.event.mode.changed（核心已广播，前端此前零订阅）；switchActiveMode() 乐观更新
  composables/useActiveMode.test.ts → 4 守卫：水合订阅 / 事件即时更新 / 乐观切换 / 退订
Changed
  WebUI 图标语义统一（总纲 §4.2）：settingsSections + sharedRoutes + CapabilitiesHub/Pet/Knowledge 内联图标按映射表更新专名
  ControlCenterView → 改消费共享 activeMode（模式切换全局即时生效）；12s 轮询降级为 30s 保底（仅刷 spec/cache/caps）
  App.vue → 挂载 initActiveMode / 卸载 disposeActiveMode（顶层订阅入口）
  zh-Hans.json → 补全 3 处遗留英文同形键（ssh-collab→SSH 协同 / knowledge→知识库 / wallpaper→皮肤中心），消除 i18n leaked-key 失败
Fixed
  模式切换实时联动缺口：此前仅 ControlCenter 轮询，顶栏/侧栏/打开页不感知切换 → 现由 session.event.mode.changed 事件驱动全局即时刷新
Verified
  vue-tsc --noEmit 零错误；vite build 成功；icons 6 passed + useActiveMode 4 passed + i18n 27 passed；全量 vitest 3106 passed（0 fail）

[0.1.5-dev] - 2026-08-23 · WebUI 分组导航 + 智能中枢面板（v28）

Added
  router/nav.ts → getGroupedNavigation()：按语义分组（Work/Operate/Observe/Settings）返回可折叠区块；深链能力（engine/editor/terminal/browser-cockpit/ssh-collab/computer-use/fleet/agents/graph/expert-center/capabilities-hub/knowledge/pet/wallpaper）从不可见变为分组可达；去重逻辑优先取带 group 的规范路由定义
  router/sharedRoutes.ts → 校正 23 条路由的 meta.group 语义归属（Operate=执行工具 / Observe=观测诊断 / Settings=系统能力）；/control-center 接入（group:Settings, order:93, 懒加载）
  router/nav.test.ts → 4 个分组守卫：四组 bucket 正确 / navOrder 组内排序 / 无空组 / primary rail 项不重复进组
  app/useNavigation.ts → 暴露 groupedNav + collapsedGroups + toggleGroup（分组折叠态会话内持久）
  App.vue → 侧栏平铺 rail 后渲染可折叠分组区块（chevron 旋转 + 嵌套缩进 + 计数徽标）；暗色主题匹配既有 sidebar 节奏
  views/ControlCenterView.vue → 智能中枢面板：5 模式切换（mode.get/mode.switch）+ BloomSpec 全字段（mode.spec：声明深度/验证遍数/温度/蜂群上限/锚定等 + effective_* 执行层实值高亮）+ 三层缓存仪表（cache.status.tiers：T0 精确/T1 规范化/T2 语义 命中与节省 Token，honest hit_rate 不冒充 100%）+ 能力矩阵（plans.capabilities.stats：总/真实/部分/已实现/叙事）
  locales/*.json → 补 nav 分组键（groupWork/Operate/Observe/Settings）+ control-center + ssh-collab/knowledge/wallpaper 等，所有 locale 与 en 基准对齐
Changed
  导航从「全 Work 平铺」重构为「Work 核心 rail + 三可折叠分组」，深链能力可发现性从 0 提升到完整
  ControlCenter 每 12s 自动刷新（mode/cache/caps 三源并行），与 ChatView 计量条同源 cache.status
Fixed
  既有 nav.test 契约漂移（/games 本就 nav:'primary'+group:Work 却未入断言）→ 修正为 7 项；getNavigationItems('primary') 软断言（agents/settings 不出现）
Verified
  vue-tsc --noEmit 零错误；vite build 成功（ControlCenterView 懒加载 chunk）；router/app 测试 35 passed
  webui 全量 typecheck + build 通过（Task #8 收尾）

[0.1.5-dev] - 2026-08-23 · 思考深度接线 + 三层语义缓存接入（v27）

Added
  model/thinking.rs → ThinkingEffort::from_spec_str：BloomSpec 字符串（max/medium/low/off）→ 枚举解析入口（历史缺失）
  model/ensemble_deep.rs → DeepPlan：模式声明的 thinking_depth/verification_passes 映射到打磨轮数/验证遍数（Bloom 保历史 1+1=4 调用，OmniTranscend 加深到 2+2=7）
  model/llm.rs → LLMClient::with_temperature（钳 [0,2]，NaN 保护）；温度进请求体
  orchestration run_bloom_text → 读取 current_bloom_spec() 接线 thinking_effort + temperature + DeepPlan（消除空转字段）
  gateway/methods.rs mode_spec → 回报 effective_refine_rounds / effective_verify_rounds / effective_max_llm_calls / effective_thinking_effort（声明深度 vs 实际轮数并列）
  cache/semantic.rs → MultiTierCache 接驳网关：Tier 0 精确（共享网关 PrefixCache，同源零重复计数）+ Tier 1 规范化精确 + Tier 2 语义近似（默认关闭，需显式开启）；TierPolicy 策略（ExactOnly/NormalizedOnly/Full）保证工具路径永不近似命中丢调用
  cache/mod.rs → install_semantic_layer / semantic_layer 进程级接驳位
  gateway/chat.rs → gateway_semantic_cache()：Tier 0 复用 gateway_prefix_cache，Tier 1/2 从 llm-semantic-cache.json 冷启动恢复；每轮落盘
  gateway/methods.rs cache.status → 扩展三层命中分解（exact/normalized/semantic、各自 hits、lookups、honest hit_rate、semantic_enabled 标志），绝不冒充 100%
Changed
  历史「思考深度/验证遍数/推理档/采样温度」仅为展示字段（执行层零消费）→ 现在真正驱动打磨轮数/验证遍数/思考档位/采样温度
  MultiTierCache 由死代码（模块外零调用）→ 真实接入 LLM 对话路径（chat_complete_cached 精确未命中后逐层短路补命中）
Fixed
  工具路径缓存近似命中的正确性风险（TierPolicy::ExactOnly 永远不查语义层）
Verified
  cargo test --lib 2177 passed（+19 vs 2158 基线）；webui 待 Task #6/#7 完成后统一 typecheck + build
  cache 模块 66 passed；model::llm 48 passed（含三层接驳短路守卫 + 共享 exact 同源守卫 + 诚实命中率守卫）

[0.1.5-dev] - 2026-08-22 · OmniTranscend 强化 + 思考链 S1-S5 + 蜂群后台化 + 团队持久化（v26）

[0.1.5-dev] - 2026-08-22 · OmniTranscend 强化 + 思考链 S1-S5 + 蜂群后台化 + 团队持久化（v26）

Added
  orchestration → current_bloom_spec()：OmniTranscend 模式按 omni_transcend.yaml 全量生效（蜂群 13/子专家 6/深想 4/验证 4/160k/24 步/5 段锚定），蜂群·自主闭环·锚定全部模式感知
  orchestration → OmniTranscend 思考链：S1 定轨→S2 只读探索→S3 深想→S4 执行（YOLO×PTC）→S5 全域 逐段发射到 UI
  gateway → team.run.background：蜂群后台化（long_running 台账心跳 15s/检查点/完成态，立即返回 taskId）
  gateway → team.compositions.list / save / remove（团队持久化组合，data_dir/team_compositions.json）
  webui ExpertCenterView → 蜂群后台化面板（启动/状态轮询）+ 团队持久化组合面板（保存/载入/重跑/删除）
Verified
  cargo test --lib 2158 passed（+1）；webui typecheck（10 守卫）+ build 255 文件

[0.1.5-dev] - 2026-08-22 · 向量混合检索 + 团队一键启动 + 行为蒸馏 + 首 token 实测（v25）

Added
  core/kb → kb.search.hybrid：BM25 + 向量 + RRF 融合（零依赖特征哈希 256 维默认；model 后端可配；超限诚实降级，engine 字段如实回报）
  core/kb/hybrid.rs → reindex_embeddings + Arc<dyn Embedding>
  gateway → team.run（专家团队真实并行 spawn：strategy/members/summaries/lead_reply）
  webui ExpertCenterView → 团队/蜂群一键启动 + 用户行为蒸馏面板（behavior.summary + agent.profile.auto）
  webui KnowledgePagesView → 向量混合检索卡（BM25/向量/融合三分数 + 引擎标签）
  webui ChatView → 计量条「首 token（客户端实测）」
Verified
  cargo test --lib 2157 passed（+1）；webui typecheck（10 守卫）+ build 255 文件

[0.1.5-dev] - 2026-08-22 · 蜂群上限设置 + 插件批量安装/下载源 + 对话计量条 + 内置浏览器（v24）

Added
  core/src/mode/swarm.rs → 蜂群控制：数量上限可配置（swarm_config.json，硬边界钳制）
  orchestration run_bloom_swarm → 实时读取生效上限（智能体/团队/专家蜂群全部受控）
  gateway → mode.swarm.get / mode.swarm.set
  gateway → market.install.batch（逐项安装回报）+ market.sources（第三方仓库 git remote 枚举 + 清单目录）
  gateway → cache.status（客户端命中率 / 服务端前缀缓存命中率 / 省前缀 token）
  webui ExpertCenterView → 蜂群控制面板（上限输入/保存/恢复默认）
  webui PluginsView → 批量安装全部插件 + 下载源面板
  webui ChatView → 对话页计量条（本会话 token/轮次 + 缓存命中率 + 省前缀 token，12s 刷新）
  webui BrowserCockpitView → 内置 iframe 浏览器（沙箱直载，诚实标注站点嵌套限制）
Verified
  cargo test --lib 2156 passed（+4）；webui typecheck（10 守卫）+ build 255 文件

[0.1.5-dev] - 2026-08-22 · 项目级图谱 + 插件市场审计/导入/导出 + SSH 执行 + 皮肤中心 + 专家中心（v23）

Added
  core/src/kb/project_graph.rs → 项目级关系图谱（一项目一图：项目文件夹唯一图谱）
    - analyze 离线静态扫描：README 主题 / 依赖清单 / 源文件 import·use·mod 引用
    - 持久化 <data_dir>/project_graphs/，原子写，4 单测
  gateway → graph.project.list / view / save / delete / analyze
  gateway → market.audit（红线 system_prompt·full_context + 高危能力扫描，批量）/
    market.import（默认拒绝红线字段，ack 放行打标，目录穿越防护）/ market.export
  gateway → collab.ssh.exec（BatchMode + 60s 超时，exitCode/stdout/stderr）
  gateway → mode.spec（蜂群分片上限 / 子专家上限 / 推理深度 / 验证轮数 / 思考强度）
  gateway → SUPPORTED_METHODS 增 agents.list / experts.list 公开
  webui GraphView → 项目级图谱面板（分析/保存/加载/删除 + 已保存清单）
  webui PluginsView → 插件市场：批量安全审计 / 导入 / 导出 / 本地清单目录
  webui SshCollabView → 远程执行面板
  webui WallpaperView → /wallpaper 皮肤中心：预设渐变 5 款 + 本地图片（≤2MB 持久化）+ 视频动态壁纸 + 沉浸式预览
  webui ExpertCenterView → /expert-center 智能体与专家中心：experts.list / agents.list / mode.spec 蜂群
  webui PetView → 茉莉仙子打磨：呼吸光环 / 振翅 / 双层纱裙 / 朦胧半闭眼 / 三朵茉莉 / 星屑 / 御风 sway+bob
Verified
  cargo test --lib 2152 passed（+4）；webui typecheck（10 守卫）+ build 255 文件

[0.1.5-dev] - 2026-08-22 · 能力中心 + SSH 协同 + 终端页 + 网络面板 + 质量评分卡（v22）

Added
  gateway/protocol.rs+methods.rs → collab.ssh.list / add / remove / probe（企业协同 SSH 主机台账 + 真实探测）
  webui/src/views/SshCollabView.vue → /ssh-collab 主导航（主机增删/探测/状态）
  webui/src/views/CapabilitiesHubView.vue → /capabilities-hub 能力中心/设置总览（22+ 分区卡片 + 能力健康度）
  webui/src/views/TerminalView.vue → /terminal 独立终端沙箱（sandbox.terminal.*）
  BrowserCockpitView → DevTools 网络面板（Resource Timing 近似段：捕获/清空/表格；不含请求头/响应体，诚实标注）
  EngineHealthView → Omni-Agent 质量评分卡（long_running.list 派生 0–100 分 + 优/良/合格/待改进）
  CapabilitiesHubView icon 类型修正（IconName 联合类型）
Verified
  webui typecheck 通过（10 项守卫）；build 249 文件；dist 已同步 gui/src-tauri/webui/dist 与便携包

[0.1.5-dev] - 2026-08-22 · 浏览器驾驶舱 + 茉莉仙子粒子宠物 + 意图置信度可调（v21）

Added
  webui/src/views/BrowserCockpitView.vue → /browser-cockpit（浏览器驾驶舱）：
    - browser.status 检测（本机 Chromium/CDP）
    - browser.open / browser.navigate（地址栏）
    - JS 控制台（browser.evaluate）
    - 截图显示 + 点击截图=真实鼠标点击（browser.screenshot / browser.click）
    - DOM 快照 + 无障碍树（browser.a11y）
    - 会话关闭（browser.close）
  webui/src/views/PetView.vue → 茉莉仙子：
    - canvas 粒子艺术（ASCII 字符 ✿❀✽·~⊹ﾟ 流动 + 光晕）
    - 仙子剪影（裙摆/长发/翅膀/手拿雏菊/脚踩茉莉）
    - 保留 pet.status/action 与能量/好感度逻辑
  core/src/intent/classifier.rs → MARGUERITE_INTENT_CONFIDENCE_THRESHOLD 环境变量（0..1，默认 0.6，非法回退）；
    调低可减少「低置信度强制 LLM 消歧」，任务计划更顺畅执行
  router sharedRoutes → /browser-cockpit 主导航（Browser）
Docs: webui-coverage-audit 更新（浏览器驾驶舱/宠物/意图置信度）

[0.1.5-dev] - 2026-08-22 · 内置多文件工作区编辑器（v20）

Added
  gateway/protocol.rs+methods.rs → workspace.file.read / workspace.file.write（工作区根越界防护、父目录自动创建、字节数回传）
  webui/src/views/WorkspaceEditorView.vue → /editor 导航页：
    - 左侧文件树（sandbox.path.list：目录导航/上级/刷新）
    - 右侧多标签文本编辑（打开/切换/关闭/脏标记/保存/保存全部）
    - 状态栏：路径/行数/字符数
  webui/src/router/sharedRoutes.ts → /editor 主导航（fileCode 图标）
Verified
  webui typecheck 通过；build 237 文件（WorkspaceEditorView chunk）；gateway 方法子集校验通过

[0.1.5-dev] - 2026-08-22 · WebUI 任务执行质量卡（v19）

Added
  EngineHealthView → 任务执行质量卡（long_running.list）：
    - 总数 / 进行中 / 完成(完成率) / 失败 / 暂停 / 中断 / 检查点 / 重试 汇总
    - 最近 8 条任务列表（goal/status/retry/checkpoints）
    - 修复：loadAll 现同载 mcp.servers.list（此前 MCP 卡首次进入为空）
  docs/webui-coverage-audit → 更新质量可视化状态
Verified
  webui typecheck 通过；构建 234 文件（EngineHealthView chunk 更新）

[0.1.5-dev] - 2026-08-22 · MCP stdio 桥落地（v18）

Added
  core/src/mcp/client.rs → McpStdioSpec + McpClient::new_stdio（子进程 JSON-RPC over stdin/stdout）：
    - initialize(id=0) + notifications/initialized 握手
    - tools/list / tools/call（每次请求独立进程，kill_on_drop 兜底）
    - parse_mcp_result（HTTP/stdio 共用结果解析）
  orchestration/mod.rs → connect_configured_mcp_clients 支持 command/args/env 型 stdio 服务器（与 URL 型并存）
  gateway methods.rs → mcp.servers.probe 支持 command 型 stdio 服务器（返回工具名/描述）
  EngineHealthView → MCP 新增表单支持 command（npx -y @modelcontextprotocol/server-* 等）
Tests: mcp::client 20 passed（+2 stdio 构造/结果解析）；gateway::methods 28 passed

[0.1.5-dev] - 2026-08-22 · WebUI MCP 管理 + 模型档案增删（v17）

Added
  gateway/protocol.rs+methods.rs → models.profiles.add / remove、mcp.servers.list / add / remove / probe
    - models.*（Rust-first，models.json 持久化；probe 为 async-only）
    - mcp.servers.*（config.yaml 持久化；probe 真实连接 URL MCP 拉取工具目录）
  EngineHealthView → MCP 服务器管理卡（新增/删除/探测 + 探测结果）与模型档案新增表单/删除按钮
  docs/webui-coverage-audit-v0.1.5.md → 更新 MCP/模型档案 WebUI 覆盖状态
Verified
  gateway::methods 28 passed（含 SUPPORTED_METHODS 子集校验）；webui typecheck 通过

[0.1.5-dev] - 2026-08-22 · WebUI 引擎健康/模型档案/模式面板 + 覆盖审计（v16）

Added
  webui/src/views/EngineHealthView.vue → /engine 导航页：
    - 能力健康度（plans.capabilities）
    - Rust 模型档案（models.profiles.list / set_active / set_copilot）
    - 重型运行时（runtime.status 下载源/选型）
    - 运行时模式（mode.get / mode.switch：plan/agent/ensemble/bloom/omni-transcend）
  core/src/gateway/* → 新增 models.profiles.list / set_active / set_copilot（Rust-first，侧车离线可用）
  docs/webui-coverage-audit-v0.1.5.md → 跨会话改进→WebUI 覆盖矩阵
Tests: gateway games_tests 4/4（含 models_profiles_list_contract）；webui typecheck + build 通过

[0.1.5-dev] - 2026-08-22 · 发布包重建（v15）

Changed
  重新执行 cargo tauri build（MSI + NSIS）：
    - 嵌入最新 webui/dist（含 GamesView 游戏大厅 + plans.capabilities 能力健康度横幅）
    - 同步最新 skills / resources / modes / pyengine / btpanel-mcp / docs
    - 使用包含 Anthropic provider + games + runtime.status + mcp tools 的 marguerite-core
  release 产物：Marguerite_0.1.5_x64-setup.exe (255,745,965 B, 04:46) / Marguerite_0.1.5_x64_en-US.msi (277,618,387 B, 04:41)
  便携包：Marguerite-0.1.5-dev-win-x64-portable（bin 三件套 + 最新 webui/docs）并重新压缩 zip
  Verified：便携 marguerite-core 已含 games list / doctor heavy-runtime / anthropic 模型档

[0.1.5-dev] - 2026-08-22 · 能力诚实化/多云种子/games 全链路（v12）

Added
  domain/capabilities.rs   → 修复 auto.uia 重复登记（仅保留一条 UIA 跨应用桥）；CapabilityStats 增 real/partial；
                                新增注册表一致性单测（ID 唯一/统计自洽/纯叙事最小清单=音频+本地模型）
  cli/program.rs            → capabilities/doctor 输出与 --json 增 real/partial；新增 games 子命令（list/play）
  cli/argv.rs               → SUBCOMMANDS 增 games（路由 18→19，帮助同步）
  model/registry.rs         → 内置种子增 deepseek-chat/openai-gpt-4o/openai-gpt-4o-mini/openrouter；
                                seed_revision 机制：旧 models.json 自动补齐缺失内置模板（不复活已删）
  gateway/protocol.rs       → SUPPORTED_METHODS 增 games.list/games.play
  gateway/methods.rs        → games_list/games_play 同步实现 + 2 单测；plans.capabilities stats 增 real/partial
  docs/cli-reference.md     → 新增 §4.13 games 小节
Tests: 以全量运行结果为准（见 AGENTS.md §2）

[0.1.5-dev] - 2026-08-22 · 原生 Anthropic + 重型运行时诚实占位 + MCP tools + WebUI 游戏大厅（v14）

Added
  model/llm.rs               → LLMProvider::Anthropic 原生 Messages API：
                                /v1/messages、x-api-key/Anthropic-Version 头、system+content blocks 消息转换、
                                tool_use/tool_result 合成、非流式 content blocks 解析、SSE 流式（text/thinking/input_json delta）
                                + 终态 tool_accum 收敛；chat_complete/chat_complete_with_tools/两条流式路径全分支；+4 单测
  model/registry.rs          → 新增 anthropic-claude-sonnet-4 / anthropic-claude-opus-4 内置云种子；provider 面扩为：deepseek/openai/anthropic/custom
  cli/program.rs             → mcp tools 子命令：实时连接配置中的 HTTP MCP 服务器并拉取工具目录（与编排器桥接同源）；
                                doctor 增 audio_runtime / local_model_runtime 两项诚实占位（下载源 + 按机器选型指引）
  gateway/protocol.rs        → SUPPORTED_METHODS 增 runtime.status
  gateway/methods.rs         → runtime_status 实现 + 1 单测（bundled=false / not_installed / 下载源指引契约）
  core/src/runtime/mod.rs    → 重型运行时状态唯一来源（ASR + 本地量化模型；不预下载，用户按配置自行下载后翻转能力）
  webui                      → GamesView（games.list/games.play + plans.capabilities 能力健康度横幅）+ /games 路由（导航 Games）
  docs/heavy-runtime-guide.md→ 下载源 + CPU/内存/显存选型表 + 启用步骤
Changed
  版本一致性：gui/package.json 0.1.0→0.1.5；wxs 统一 0.1.5 并重命名（根 + marguerite 内）；
  docs/cli-reference.md 版本头 v0.1.0→v0.1.5；release/ 清理 18 个旧件（v25-v27、0.1.4、v17-v26 二进制、旧截图）
Tests: 以全量运行结果为准（见 AGENTS.md §2）

[0.1.5-dev] - 2026-08-22 · 子命令旗标透传修复（v13）

Fixed
  cli/argv.rs → 新增 SUBCOMMAND_FLAGS_WITH_VALUE / SUBCOMMAND_BOOL_FLAGS 透传机制：
    deploy（--target/--bin/--service/--health-url/--user/--identity/--dir/--site/--endpoint/--api-key/--dry-run/--no-systemd/--no-ssl/--add-panel）、
    model add（--id/--base-url/--model/--provider/--label/--api-key/--kind）、
    browser（--out）、dispatch（--timeout/--parallel/--targets/--env）、subagent（--reflect/--trellis）
    后置旗标不再被误判「未知选项」exit 2，而是原样透传到 sub_args 由对应 handler 消费。
  cli/argv.rs → +1 单测 test_parse_subcommand_flags_passthrough（model/dispatch/browser/subagent/deploy 五类旗标覆盖）。
Verified
  model add/remove 往返成功；deploy --target ssh://… --dry-run 演练输出完整远端步骤；argv 38/38；
  全量 lib 以运行结果为准（见 AGENTS.md §2）。

[0.1.5-dev] - 2026-08-22 · 技能发现与能力健康度改进 (v11)

Added
  tools/skill_tools.rs    → resolve_skills_dir 增加「可执行文件旁 bundled」探测（`<app>/skills`、`<app>/bin/skills`）；
                               相对路径按 cwd→上溯→bundled→数据目录 顺序解析
  cli/program.rs          → skills_handler 改用 resolve_skills_dir（修复任意 cwd 下 `skills list` 为空）；
                               doctor 新增 `skills` 自检项（解析目录 + 数量 + 落地提示）与 `capabilities` 健康度行（implemented/narrative/total），--json 增 skills_dir/skills_count/skills_ok/capabilities
  gateway/methods.rs      → skills_dir 统一委托 resolve_skills_dir，消除 CLI/TUI/网关不一致
  model/registry.rs       → 内置云种子增 deepseek-v4-flash / deepseek-v4-flash-vision-exp（对齐 DeepSeek 官方模型族）；
                               test_remove_protects_minimum 适配新种子集；新增 test_seeds_include_deepseek_family（+1）
  games/tictactoe.rs       → 新增 Rust 井字棋引擎（落子/胜负/平局/悔棋/重置）+ 8 单测；dsh.games.tictactoe 能力 纯叙事→已实装；
  games/snake.rs           → 新增 Rust 贪吃蛇引擎（移动/转向禁180°/食物增长/碰撞/计分）+ 7 单测；dsh.games.snake 能力 纯叙事→已实装；
  games/game2048.rs        → 新增 Rust 2048 引擎（4×4 滑动合并/每格至多一次/计分/结束判定/确定性生成）+ 10 单测；dsh.games.2048 能力 纯叙事→已实装；
  games/mod.rs             → 新增 catalog() 单一事实源（3 款已实装游戏）+ run_game() 可玩性入口（动作驱动，返回棋盘+结果）+
                               REPL `/games` 命令（list 与 play <id> <动作...>）；Mini Games 槽位：三款小游戏引擎均已实装；
                               dsh.games.play 能力 部分实装→已实装（`/games play` 已接通引擎）；
                               Mini Games 槽位改为渲染 games::catalog() 单一事实源（杜绝与引擎/REPL/capabilities 漂移）
  gateway/methods.rs       → 新增 market.upgrade（semver_gt 升序校验 + tmp/rename 原子替换 + 名称一致性 + 据实失败）+ dispatch 接线 + SUPPORTED_METHODS + 1 单测
  memory/pages.rs           → 新增内置知识页系统（免 Key/离线：JSON 原子持久化 + 语义×关键词混合检索(EmbeddingProvider 可插拔) + 引用计数）
                               + 5 单测；REPL 新增 /pages（list/add/show/search/rm）+ 1 单测；data_dir()/knowledge-pages.json
  domain/capabilities.rs   → 再翻转 auto.uia（Windows COM 自动化桥 com.rs 已实装）与 res.self_heal（harness/distill.rs run_self_evolve+EvolutionGate + memory SelfEvolution）
  domain/capabilities.rs   → 能力清单如实翻转 16 项 部分实装→已实装：Bundle 拓扑加载(Rust Kahn)、Router 自动路由(intent×complexity)、
                               结构化差异预览(diff+apply_patch)、双模型配对(copilot)、文件监听(notify+总线)、Market 安装/升级、
                               MAX 双带队(dual_leader_specs)、集群调度(cluster_specs)、swarm DAG(dispatch_bloom_swarm)、
                               Modlens 版面分析(VisionDocument.layout)、FPS 助手(computer_use_loop+humanize+skills/fps)、
                               MCP 工具桥(McpClient+ToolRegistry::add_mcp_client+describe_mcp_tools+缓存执行)、
                               私有知识库(kb ingest/search+memory.import)、浏览器自动化(CdpSession CDP)、语义记忆(embedding+hybrid_retrieve)
  Tests: 2121 passed（能力注册表驱动测试随翻转生效）
  tests                   → resolve_skills_dir_returns_absolute_existing_dir（+1）
Changed
  AGENTS.md §2 测试基线 2083 → 2117（+34 新单测）
Tests: 2117 passed / 0 failed / 14 ignored（NO_COLOR 移除后）

[0.1.5-dev] - 2026-08-21 · 五项深度强化 (v10)

Added
  model/prefetch.rs           → PrefetchDedup 会话级去重（warmup_once_for_session 无 TTL、
                                SESSION_DEDUP_CAP 有界）+ agent/team 会话注入
  model/router.rs             → IntentType 增 WebSearch/Vision/LongContext + 规则补全 +
                                平局确定性（同 priority 取表首）+ primary_with_thinking/cap 抽取
  harness/routing.rs          → 三模式策略边界测试与修复
  agent/multi_strategy.rs     → resolve_role_model（角色→模型表：architect/reviewer/researcher→pro 等）
  tests/joint_call.rs         → 离线闭环集成测试（模型路由→子代理→工具执行，mock LLM 循环应答）
  deploy/ssh.rs               → snip_output / is_idempotent_command / idempotency_warning /
                                exec_idempotent（trait 默认实现）/ exec_batch / HostSpec::parse +
                                退出码保真；14 项离线单测
  tools/builtin.rs            → str_replace_editor 补 insert/undo_edit + CRLF·空文件·多次替换
                                边界硬化（子代理交付）
  docs/five-hardening-round-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2055 → 2083（+28 新单测）
Tests : 2083 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · 完成性/缓存 100% 补强 (v9)

Added
  cache/mod.rs                → PrefixCache 脏标记 + absorb 冷上下文并入 + save_if_dirty 按需落盘
                                （无写入零 IO）；3 项单测（含跨重启命中 hits=1/misses=0 证明）
  model/{llm,router}.rs       → LLMClient::load_cache + ModelRouter::new_with_cache /
                                new_with_credentials_and_cache / load_cache_from（共享缓存构造链）
  orchestration/mod.rs        → new_with_credentials_and_cache / new_with_workspace_credentials_cache
                                （原构造器委托 None）；REFLECT_GATE_MARKER 改 pub(crate)
  tui/workbench RealEngine     → with_shared_cache + execute_turn/run_turn 透传
  gateway/chat.rs             → gateway_prefix_cache（OnceLock 冷启动 open 恢复 + 跨轮共享）+
                                persist_gateway_cache 三条路径每轮按需落盘 + Bloom/RealEngine/长任务接线
  plan/runner.rs              → 步骤瞬时错误退避重试至 3 次（治理门/宪法/反思门禁确定性拒绝绝不重试，
                                is_retryable_step_error 分类 + 单测）；失败终态先存检查点；
                                完成门禁真实执行（移除硬编码 allowed()，未放行如实回传，永不假成功）
  agent/runtime.rs            → 稳定前缀逐字节一致守卫单测（KV-cache 命中前提锁定）
  docs/completion-cache-100pct-report-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2050 → 2055（+5 新单测）
Tests : 2055 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v8 (剩余四项一次性收尾)

Added
  workflow/mod.rs             → 声明式工作流引擎（WorkflowSpec 阶段内 fan-out + 阶段间屏障 +
                                单条失败→None 不拖垮整轮；fanout_plan 纯函数；离线单测）
  workflow/ralph.rs           → Ralph 全新子代理循环（next_prompt/parse_report 纯函数 +
                                run_with 注入式 spawn + 工作区记忆文件 + clamp[1,16]；8 项单测）
  gateway/chat.rs             → 会话投影消费方：每轮折叠 turnCount/hostAudit 进 session-projections.db，
                                projection.snapshot 事件下发（+1 单测）
  schedule/mod.rs             → execute_slash 执行面（/schedule list|after|at|every|cancel 落地台账，
                                +2 单测）；cli/repl.rs /schedule 分支 + /help 更新
  webui/public/wallpapers/    → 主题/个性化资产包：4 款 SVG（水墨/深空蓝橙/夜樱/鲸海）+
                                manifest.json 版本化清单 + README
  docs/dsh-fusion-v8-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2035 → 2050（+15 新单测）
Tests : 2050 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）
WebUI : npm run typecheck 通过（check:architecture 10 项守卫 + vue-tsc）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v7 (schedule 会话提醒)

Added
  schedule/mod.rs             → 上游 dsh-schedule 移植：ReminderTrigger{after/at/every} +
                                MIN_EVERY_INTERVAL_SECONDS=300 + ScheduleError 错误码族 +
                                SQLite 台账（schedule.db）+ due 纯查询 + every 锚点重挂载
                                （不枚举积压）+ cancel；9 项单测
  gateway/chat.rs             → emit_due_schedule_reminders：三条对话路径每轮后触发到期提醒，
                                schedule.due 事件下发；schedule_store 进程级缓存 + 失败退内存
  cli/dslash.rs               → 第 8 条斜杠命令 /schedule（list/after/at/every/cancel + 错误路径）
  plugins/manifests/dsh-schedule.plugin.json → 0.1.1-mgr1（+list/cancel 能力 + slash_command）
  docs/dsh-fusion-v7-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2026 → 2035（+9 新单测）
Tests : 2035 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v6 (goal race-fence + persona)

Added
  goal/mod.rs                 → begin_round_at（race-fenced 预留：stale 修订号不耗轮）+ revision()/is_stale()
                                （预留成功 rounds_started+1 且 revision+1，双驱动不双耗）
  persona/mod.rs              → PersonaSection 组合渲染 + complete 阻断后续段 + {{model}}/{{cwd}} 模板解析
  agent/store.rs              → personality_for 改段落化渲染（<!--persona-complete--> 标记剥除并阻断技能/工具段）
  gateway/chat.rs             → agent_personality_hint(session, model, cwd) 三路径接入模板解析
  docs/dsh-fusion-v6-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2015 → 2026（+11 新单测）
Tests : 2026 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v5 (凭证缝/投影接线)

Added
  credentials/mod.rs          → resolve_llm_key（provider 路由键 → 常见引用 → config 回退）
  model/router.rs             → ModelRouter::new_with_credentials（台账覆盖 api_key，全客户端继承）
  orchestration/mod.rs        → new_with_credentials / new_with_workspace_credentials（原构造器委托 None）
  tui/workbench RealEngine     → with_credentials + execute_turn/run_turn 透传
  gateway/chat.rs             → gateway_credentials（OnceLock 进程级缓存）+ Bloom/RealEngine 双路径接入
  projection/mod.rs           → TokenUsageUnit（上游 tokenUsage fold：同一步替换 addReplacing、跨步累加、totals 视图）
  harness/dsh_bridge.rs       → install_usage_hook_with_projection / install_dsh_bridges_with_projection
                                （usage 样本折叠进会话 tokenUsage 投影，best-effort 不阻断记账）
  docs/dsh-fusion-v5-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 2007 → 2015（+8 新单测）
Tests : 2015 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v4 (P1 机制移植)

Added
  credentials/mod.rs          → CredentialKey `<scope>/<id>` + CredentialRef 双名义键空间；
                                 CredentialRecord（api-key | grant tagged）+ CredentialsStore
                                 （data_dir/credentials.json 原子落盘 + ref 解析 + 损坏回退）
  credentials/authorization.rs→ AuthorizationRegistry 授权流注册表（method/notice/prompt/status/
                                 settlement/outcome/entry wire-safe 类型 + begin/settle/last_settlement）
  projection/mod.rs           → ProjectionUnit 纯 init/apply fold + 可选 wire.view（host-only 无视图）+
                                 ProjectionStore（SQLite session_projection_state 版本化状态表、
                                 state_of/view_of/replay）+ TurnCounterUnit/HostAuditUnit 示例单元
  guard/gateway.rs            → ApprovalOrigin（default|selection|inferred）+ PendingApproval.origin +
                                 request_with_origin（request = Default，向后兼容）
  mode/mod.rs                 → ModeManager.spec_origins + spec_origin（内置=Default、目录覆盖=Selection）
  events/envelope.rs          → Envelope::mode_changed_with_origin（permission_origin 随 mode.changed 下发）
  docs/dsh-fusion-v4-2026-08-21.md
Changed
  AGENTS.md §2 测试基线 1985 → 2007（+22 新单测）
  skills/dsh-market-list.yaml → dsh-session-projection 行更新（fold 契约已移植）
Tests : 2007 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.5-dev] - 2026-08-21 · DSH 0.1.1-rc.1 Fusion v3 (锚定接线收尾)

Added
  mode/anchoring.rs        → AnchorState 轮次计步 + deferred_allows（deferred_sources × deferred_grace_steps 语义）
  agent/runtime.rs         → bootstrap_max_tokens 一次性请求预算（首请求注入后剥离）+ defer_skill_catalog + mode_section 装配通道
  model/{llm,router}.rs    → with_max_tokens / chat_for_capped / chat_with_tools_capped（锚定 bootstrap 请求级上限）
  orchestration/mod.rs     → anchoring_spec/anchor/message_source 字段 + set_anchoring_spec/set_anchor/take_anchor/
                             set_message_source + anchor_for_turn/anchor_for_delegation；三处 dispatch 改走会话锚定态
  gateway/chat.rs          → SessionAnchors 会话级锚定台账（跨轮持久，晋升一次不重锚）+ 人设 deferred 门控
  tui/workbench RealEngine  → with_mode_section + 按模式自动注入 plan prompt_section（v2 §4 收尾）
  plugins/manifests/dsh-presets.plugin.json → 0.1.1-mgr1（session_persist/deferred_inject 能力 + 新 hook params）
  docs/dsh-fusion-v3-2026-08-21.md + docs/research/{dsh-rc8-to-011rc1,ecosystem-025-to-027}-delta.md
Changed
  AGENTS.md §2 测试基线 1976 → 1985（+9 新单测；NO_COLOR 环境注记）
  skills/dsh-market-list.yaml → dsh-presets/dsh-plan-mode/dsh-session-projection 行更新
Tests : 1985 passed / 0 failed / 14 ignored（NO_COLOR 已移除环境下）

[0.1.4] - 2026-08-20 · DSH 12 Plugin Deep Fusion

Added
  Capabilities SSOT unified registry  → 35 dsh.* capabilities  (12 manifests × 8 manifests SSOT)
  PetWhaleSlot V2 real integration    → WorkbenchState.pet_v2 = Some(PetV2::default())
  12 DSH plugins manifests + skills mappings aligned; 一键启动脚本
  Clippy Advisory-A (7/7) 全部修复
Changed
  core 0.1.3 → 0.1.4   |   tui 0.1.0 → 0.1.4   |   gui 0.1.3 → 0.1.4
  PetV2 6-frame ASCII sprite rendering 真读真写
Fixed
  Deprecated: Buffer::get → buf[(x,y)]
  Deprecated: NaiveDateTime::from_timestamp_opt → DateTime::<Utc>::from_timestamp().naive_utc()
  Unused: ratatui::text::Text import removal
Tests : 43 DSH modules → 43 passed / 0 failed / 0 ignored
Caps  : 35/35 (0 missing, 0 extra)  SSOT
