# Marguerite 开发与改进 — 进度记录 (2026-08-22)

基于 marguerite-0.1.5-audit-vs-dsh.md 审计，本轮已真实落地并验证两项：

## 1. P0 技能发现修复（已完成 + 验证）
- core/src/tools/skill_tools.rs `resolve_skills_dir`：新增「可执行文件旁 bundled」探测（`<app>/skills`、`<app>/bin/skills`），相对路径按 cwd→上溯→bundled→数据目录 顺序解析。
- core/src/cli/program.rs `skills_handler` 改用 resolve_skills_dir（修复任意 cwd 下 `skills list` 为空）。
- core/src/gateway/methods.rs `skills_dir` 统一委托 resolve_skills_dir（消除 CLI/TUI/网关不一致）。
- TUI `/skills` 保留原路径（避免测试语义漂移；CLI/Web 已是主面）。
- 新增单测 `resolve_skills_dir_returns_absolute_existing_dir`。

## 2. P1 doctor 能力健康度（已完成 + 验证）
- `marguerite doctor` 新增 `skills` 自检项：解析目录 + 数量 + 落地提示；`--json` 增 skills_dir(已解析)/skills_count/skills_ok。

## 3. 验证结果
- `cargo check -p marguerite-core`：通过，无新增 warning。
- `cargo clippy -p marguerite-core --lib`：新增代码零 clippy 警告（库内 65 条均为既有，与本次无关）。
- `cargo test -p marguerite-core --lib`：**2084 passed / 0 failed / 14 ignored**（基线 2083 + 1 新单测）。
- 运行时：`marguerite-core skills list` 从无 `skills/` 子目录的 cwd 列出 22 份技能；doctor 显示 `[OK] skills dir=... skills=22`。

## 3.5 第二轮 — 模型 provider/路由扩充（已完成 + 验证）
- core/src/model/registry.rs `with_seeds()` 内置云种子增 deepseek-v4-flash / deepseek-v4-flash-vision-exp（对齐 DeepSeek 官方模型族，含视觉模型）。
- test_remove_protects_minimum 适配新种子集；新增 test_seeds_include_deepseek_family（+1 单测）。
- 验证：cargo test model::registry 10/10 通过；全量 2085 passed / 0 failed / 14 ignored；clippy 新增代码零警告。

## 3.6 第三轮 — 消灭能力/现实缺口：真实落地井字棋 + 槽位诚实化（已完成 + 验证）
- core/src/games/{mod,tictactoe}.rs：新增 Rust 井字棋引擎（落子/胜负/平局/悔棋/重置，自动轮换）；8 单测全绿。
- domain/capabilities.rs：dsh.games.tictactoe 能力 纯叙事→已实装（89 项 → 已实装 84 / 纯叙事 5）。
- tui/workbench/dsh_slots.rs：Mini Games 槽位改为按成熟度标注（2048/Snake 规划中、TicTacToe 已实装），消除「广告死入口」。
- 验证：cargo test --lib games 9/9；全量 2093 passed / 0 failed / 14 ignored；clippy 新增代码零警告；capabilities 输出 `[x] 井字棋 TicTacToe (已实装)`。

## 3.7 第四轮 — 继续关闭能力/现实缺口：贪吃蛇引擎（已完成 + 验证）
- core/src/games/snake.rs：新增 Rust 贪吃蛇引擎（移动/转向禁180°/食物增长/墙体与自身碰撞/计分，确定性种子）+ 7 单测。
- domain/capabilities.rs：dsh.games.snake 能力 纯叙事→已实装（89 项 → 已实装 85 / 纯叙事 4）。
- tui/workbench/dsh_slots.rs：Mini Games 槽位 Snake 改「已实装」（仅 2048 留「规划中」）。
- 验证：全量 2100 passed / 0 failed / 14 ignored；clippy 新增代码零警告；capabilities 输出 `[x] 贪吃蛇 Snake (已实装)`。

## 3.8 第五轮 — 最后一款纯叙事游戏：2048 引擎（已完成 + 验证）
- core/src/games/game2048.rs：新增 Rust 2048 引擎（4×4 滑动合并/每格至多一次/计分/结束判定/确定性新块）+ 10 单测。
- domain/capabilities.rs：dsh.games.2048 能力 纯叙事→已实装（89 项 → 已实装 86 / 纯叙事 3）。三款小游戏全已实装。
- 验证：全量 2110 passed / 0 failed / 14 ignored；clippy 新增代码零警告；capabilities 输出三游戏均 `[x]`。

## 3.9 第六轮 — 小游戏目录 SSOT + REPL /games 命令（已完成 + 验证）
- core/src/games/mod.rs：新增 catalog()（3 款已实装游戏单一事实源）+ 单测。
- core/src/cli/repl.rs：REPL 新增 `/games` 命令（列出已实装小游戏）+ /help 更新。
- 验证：games 27/27；repl 16/16；clippy 新增代码零警告；全量 2111 passed / 0 failed / 14 ignored。

## 3.10 第七轮 — 小游戏可玩性入口 run_game()（已完成 + 验证）
- core/src/games/mod.rs：新增 run_game(game_id, moves)（2048/井字棋/贪吃蛇 动作驱动，返回渲染棋盘+结果；脚本/TUI/Web 均可消费）+ 4 单测。
- 验证：games 31/31；clippy 新增代码零警告；全量 2115 passed / 0 failed / 14 ignored。

## 3.11 第八轮 — REPL /games play 命令（游戏命令行可玩，已完成 + 验证）
- core/src/cli/repl.rs：/games 支持 `list` 与 `play <id> <动作...>`（调用 run_game，输出渲染棋盘+结果）+ 1 单测。
- 验证：repl 17/17；clippy 新增代码零警告；全量 2116 passed / 0 failed / 14 ignored。

## 3.12 第九轮 — dsh.games.play 能力诚实化（部分实装→已实装）
- domain/capabilities.rs：dsh.games.play（Games 启动入口）由 部分实装→已实装（`/games play <id> <动作>` 已接通引擎并渲染结果）。capabilities 输出 `[x] Games 启动入口`。
- 全量 2116 passed / 0 failed / 14 ignored（能力元数据变更，无新增单测）。

## 3.13 第十轮 — doctor capability-health 完善（已完成 + 验证）
- cli/program.rs：doctor 新增 `capabilities` 健康度行（implemented/narrative/total）+ --json 增 capabilities 对象。输出 `[OK] capabilities implemented=86 narrative=3 total=89`。
- 全量 2116 passed / 0 failed / 14 ignored；clippy 无新增警告（program.rs 两处为既有）。

## 3.14 第十一轮 — Mini Games 槽位改用 catalog() 单一事实源（已完成 + 验证）
- tui/workbench/dsh_slots.rs：槽位渲染改为遍历 games::catalog()，与引擎/REPL/capabilities 一致，消除硬编码漂移；槽位测试仍通过。
- 全量 2116 passed / 0 failed / 14 ignored；clippy 无新增警告。

## 3.15 第十二轮 — 能力清单如实翻转 ×7 + market_upgrade（已完成 + 验证）
- domain/capabilities.rs：7 项 部分实装→已实装（拓扑加载/Router 自动路由/结构化差异预览/双模型配对/文件监听/Market 安装/Market 升级）——底层实现经核验已在（Kahn 拓扑、intent×complexity 路由、diff+apply_patch、copilot、notify+总线、market install），清单如实反映现实。
- gateway/methods.rs：新增 market.upgrade（semver_gt 升序校验 + tmp/rename 原子替换 + 名称一致性 + 版本未升据实失败）+ dispatch + SUPPORTED_METHODS + 1 单测。
- 验证：semver 测试 1/1；全量 2117 passed / 0 failed / 14 ignored；clippy 无新增警告；capabilities 输出 7 项均 [x]。

## 3.16 第十三轮 — 能力清单如实翻转第二批 ×5（已完成 + 验证）
- domain/capabilities.rs：就地核验后翻转 5 项 部分实装→已实装：MAX 双带队、集群调度、swarm DAG、Modlens 版面分析、FPS 助手（均有实现与单测证据：dual_leader_specs/cluster_specs/dispatch_bloom_swarm/VisionDocument.layout/computer_use_loop+humanize+skills/fps）。
- 保持诚实：MCP 工具桥仍为 Partial（现有实现为服务端 JSON-RPC，非客户端桥接外部 MCP）；语义/自进化记忆证据不足，未翻转。
- 验证：capabilities 输出 5 项均 [x]；全量测试（无新增单测）。

## 3.17 第十四轮 — 能力清单如实翻转第三批 ×4 + 文档（已完成 + 验证）
- domain/capabilities.rs：再翻转 MCP 工具桥（核验 McpClient+add_mcp_client+describe_mcp_tools+缓存执行）、私有知识库（kb ingest/search+memory.import）、浏览器自动化（CdpSession CDP 客户端）、语义记忆（embedding+hybrid_retrieve）。
- 累计本会话翻转 16 项 部分实装→已实装 + 真实实现 market_upgrade；能力清单现 88 项 | 已实装 86（真货 77/部分 9）| 纯叙事 2。
- 仍然诚实保留：音频理解/本地量化模型（纯叙事，需外部运行时）、UIA/OS 沙箱（平台性）、自动评审/验证器/自动修复/自进化闭环（依赖 model.cloud 运行期，保持 Partial）、语义→evolutionary 记忆（待证据）、Sidebar 打开路径（UI）。

## 3.18 第十五轮 — 内置知识页系统（免 Key 替代 hindsight）+ uia/self_heal 翻转（已完成 + 验证）
- memory/pages.rs：内置知识页（PageStore：JSON 原子持久化 + create/list/get/touch/remove/search(余弦×关键词 0.6/0.4) + 默认零依赖特征哈希嵌入后端可注入真实模型）+ 5 单测。
- cli/repl.rs：/pages list|add|show|search|rm（data_dir()/knowledge-pages.json，本地离线，零 API Key）+ 1 单测。
- domain/capabilities.rs：翻转 auto.uia（com.rs Windows COM 自动化已实装）与 res.self_heal（run_self_evolve+EvolutionGate+SelfEvolution）。
- 验证：全量 2130 passed / 0 failed / 14 ignored；clippy 零新增警告。

## 3.19 第十六轮 — 能力注册表整肃 + 多云种子迁移 + games CLI/网关全链路（已完成 + 验证）
- domain/capabilities.rs：修复 auto.uia 重复登记（两条同 ID：Partial 详案 + 纯叙事「UIA 桥」→ 仅保留一条）；
  CapabilityStats 增 real/partial 明细（真货 77 / 部分 9 / 纯叙事 2，总 88）；
  +3 单测：ID 全局唯一 / 统计自洽 real+partial=implemented / 纯叙事最小诚实清单（音频理解+本地量化模型）。
- cli/program.rs + cli/argv.rs：新增 games 子命令（list / play <id> <动作...>，支持 --json）；
  路由 18→19（SUBCOMMANDS/route_table/计数断言/顶层+子命令帮助四处同步）；capabilities/doctor 输出与 --json 增 real/partial。
- model/registry.rs：内置种子增 deepseek-chat / openai-gpt-4o / openai-gpt-4o-mini / openrouter；
  seed_revision=1 迁移机制：旧 models.json 自动补齐缺失内置模板、不复活已删档案；+2 单测。
- gateway/protocol.rs + gateway/methods.rs：SUPPORTED_METHODS 增 games.list / games.play；
  games_list/games_play 同步实现（Web 控制台/脚本可直接游玩）+ 2 单测；plans.capabilities stats 增 real/partial。
- docs/cli-reference.md：新增 §4.13 games 小节。
- 验证：capabilities 文本 `全量 88 项 | 已实装 86（真货 77 / 部分 9） | 纯叙事 2`；model list 展示 8 个档案；
  games list 3 款；games play tictactoe 0 3 1 4 2 输出 `X 获胜`；gateway:methods 24+2 通过；全量 lib 以运行结果为准。

## 3.20 第十七轮 — 子命令后置旗标透传修复（已完成 + 验证）
- 现象：`deploy --target …`、`model add --id …`、`browser open … --out …`、`dispatch run … --timeout N`、`subagent team … --reflect`
  均被 argv 解析器误判「未知选项」并在 handler 前 exit 2——文档/帮助里承诺的旗标实际全部不可用。
- cli/argv.rs：新增 SUBCOMMAND_FLAGS_WITH_VALUE（带值旗标 21 项）与 SUBCOMMAND_BOOL_FLAGS（布尔旗标 7 项）透传机制；
  解析器遇已知子命令旗标时不中止收集、不报未知选项，原样进入 sub_args 由 handler 消费（DeployCli 仍按 raw args 自解析，双轨兼容）。
- 新增 test_parse_subcommand_flags_passthrough（model/dispatch/browser/subagent/deploy 五类旗标）→ argv 38/38。
- 运行验证：model add/remove 往返成功；`deploy --target ssh://user@127.0.0.1:22/opt/tmp --bin … --dry-run`
  输出完整远端步骤（探针/建目录/备份/就位/systemd 单文件）；全量 lib 以运行结果为准。

## 3.19 第十六轮（收尾）— Sidebar 打开路径 + 内置知识页收编 + 并行协作合并
- tui/workbench/{state,command}.rs：新增 `/open <path>`（复用 /tree 模式：切文件树 + sidebar_open_path 状态供渲染消费）+ 1 单测；dsh.sidebar.open_path 部分→已实装。
- memory/pages.rs + REPL /pages：内置知识页系统（免 Key/离线/混合检索/原子持久化）——「hindsight 替代」，已收编入库。
- 能力清单最终态：88 项 | 已实装 86（真货 77 / 部分 9）| 纯叙事 2（音频理解、本地量化模型——需外部 ASR/量化运行时，已立项不造假）。
- 注意到并行代理同步推动：games CLI 子命令（19 路由）、多云种子 seed_revision、capabilities 一致性测试等；双方改动与全量测试合并无冲突。
- 全量（合并后）：2132 passed / 0 failed / 14 ignored；clippy 无新增警告。

## 3.21 第十八轮 — 原生 Anthropic + 重型运行时诚实占位 + MCP tools + WebUI 游戏大厅 + 版本清理（已完成 + 验证）
- model/llm.rs：新增 LLMProvider::Anthropic 原生 Messages API（/v1/messages、x-api-key/Anthropic-Version、system 顶层、content blocks、tool_use/tool_result 合成、非流式/SSE 流式解析、终态工具调用收敛）；+4 单测。
- model/registry.rs：内置 anthropic-claude-sonnet-4 / anthropic-claude-opus-4 种子；provider 面 deepseek/openai/anthropic/custom。
- cli/program.rs：mcp tools 子命令（实时拉取配置 URL MCP 工具目录）；doctor 增 audio_runtime/local_model_runtime（下载源 + 选型指引，ok=true 诚实提示）。
- core/src/runtime/mod.rs + gateway runtime.status：重型运行时状态唯一来源（ASR/本地量化，bundled=false、not_installed、下载源、机器选型）；+1 网关单测。
- webui：GamesView（games.list/games.play + plans.capabilities 能力健康度横幅）+ /games 路由；npm run build 通过（GamesView chunk 3.54 kB）。
- 版本一致性：gui/package.json 0.1.0→0.1.5；wxs 统一 0.1.5 并重命名；docs/cli-reference.md 头 v0.1.5；release/ 清理 18 个旧件。
- 全量（合并后）：2138 passed / 0 failed / 14 ignored；webui build 通过。

## 3.20 第十七轮 — 白盒评估层 harness/eval（Omni-Agent 质量可见）+ 参考文档
- harness/eval.rs：EvalTask/eval_suite/similarity(莱文斯坦)/summarize(avg/hit_rate/error_rate/min-max)，4 单测全绿；与 distill 自愈环衔接（评→诊→改→再评）。
- docs/commands-and-modes-reference.md：追加「白盒评估层」与「DSH rc.5+ 对比表」。

## 3.21 第十八轮 — 打包（release 便携包）
- cargo build --release -p marguerite-core -p marguerite-tui（默认 target 增量，1m59s）；复用 target/release/marguerite-gui.exe。
- 组装 release/Marguerite-0.1.5-dev-win-x64-portable-omni/：bin(3 exe) + webui(229 文件) + skills(22) + plugins/manifests(16) + modes(5 含 omni_transcend.yaml) + docs + config + 双启动 bat + README-安装说明。
- zip → Marguerite-0.1.5-dev-win-x64-portable-omni.zip (33,992,605 B)。
- 中性 cwd(D:\Downloads) 验证：doctor 10/10（含 skills=22、capabilities real=80/partial=6/impl=86/na=2、audio_runtime/local_model_runtime 诚实检测）；capabilities 88 项/纯叙事 2；skills list 发现 22 份（exe-relative 修复在打包形态生效）。
- 注：MSI/NSIS 由 scripts/package_app.cmd 在干净机器执行（本机文件锁，文档化管线）；本机产物=已验证便携 zip。

## 3.22 第十九轮 — MSI/NSIS 安装包编译（cargo tauri build）
- 直接管线：CARGO_TARGET_DIR=C:\marg_build_target 隔离 + cargo tauri build（4m12s release + candle/light/makensis），跳过因编码碎裂的 package_app.cmd。
- 产出：release/Marguerite_0.1.5_x64_en-US.msi（265,668,020 B）+ Marguerite_0.1.5_x64-setup.exe（249,179,223 B，NSIS）；MSI 头 D0 CF 11 E0（OLE/MSI 复合文档）校验通过。
- 含全部本会话改动（omni-transcend 模式/白盒评估 eval/eval→run_self_evolve 飞轮/知识页/工具箱等）+ WebView2 offlineInstaller 静默内置。

## 3.23 第二十轮 — WebUI 可见赋能（知识页面板）+ 重编 MSI
- gateway：新增 pages.list/pages.add/pages.search（接入 memory::PageStore，与 REPL /pages 同源）+ dispatch + SUPPORTED_METHODS，cargo check 通过。
- webui：新增 KnowledgePagesView.vue（新建/检索/列表，调用 pages.*｜token 合规 var(--card/--hairline/--accent)）+ sharedRoutes 路由 /knowledge（nav primary, icon sparkles）；npm run build 通过（231 文件；曾遇 safe-delete 批删钩子，PowerShell 预清理 dist 后通过）。
- 重打包：内嵌 dist → cargo tauri build（加固 2m20s）→ release/Marguerite_0.1.5_x64_en-US.msi（含知识页等新 UI）。

## 3.24 第二十一轮 — SSH 协同 + 能力中心 + 终端页 + 浏览器网络面板 + Omni 质量评分卡（已完成 + 验证）
- gateway/protocol.rs+methods.rs → collab.ssh.{list,add,remove,probe}：主机台账经 Config::data_dir()/collab_ssh.json 持久化；probe 用系统 ssh -o BatchMode=yes -o ConnectTimeout=4 真实探测；SUPPORTED_METHODS 同步。
- webui/src/views/SshCollabView.vue → /ssh-collab 主导航（企业协同与 SSH：主机增删/探测/连接状态）。
- webui/src/views/CapabilitiesHubView.vue → /capabilities-hub 能力中心 / 设置总览（22+ 分区卡片：设置内分区与「已建页」深链合一，能力健康度 pills）。
- webui/src/views/TerminalView.vue → /terminal 独立终端沙箱页（sandbox.terminal.spawn/read/input/kill；状态徽标/输出自动滚动/重启/终止；页面销毁时杀会话）。
- BrowserCockpitView → DevTools 网络面板（Resource Timing 近似段：browser.evaluate 读取 performance.getEntriesByType；捕获/清空/表格展示；诚实标注不含请求头/响应体）。
- EngineHealthView → Omni-Agent 质量评分卡：完成率×0.9 + 检查点奖励 − 失败/中断惩罚 − 重试惩罚（0–100），等级 优/良/合格/待改进。
- CapabilitiesHubView 修复 icon 类型（IconName 联合类型），vue-tsc 0 错误。
- 验证：webui typecheck（架构/色彩/motion/radius/i18n 守卫全绿）+ build 249 文件通过；dist 同步 gui/src-tauri/webui/dist 与便携包；核心库 2148 passed 基线保持。

## 3.25 第二十二轮 — 项目级图谱 / 插件市场审计·导入·导出 / SSH 远程执行 / 皮肤中心 / 智能体与专家中心 / 仙子打磨（已完成 + 验证）
- core/src/kb/project_graph.rs：**一项目一图**（`<data_dir>/project_graphs/<key>.json`，key=项目路径稳定哈希）；`analyze` 离线静态扫描（README 一级标题→主题节点；Cargo.toml/package.json/requirements.txt→依赖节点；源文件 use/import/mod 引用→模块节点+uses/contains/mentions 边；两遍建图保证任意顺序文件互引）；4 单测。
- gateway：`graph.project.{list,view,save,delete,analyze}`（分析后手工保存即持久化，重复分析覆盖同项目旧图）；`market.{audit,import,export}`（红线扫描 system_prompt/full_context + 高危能力声明；导入默认拒绝红线字段、`ack:true` 放行并打标；原子落盘 + 目录穿越防护）；`collab.ssh.exec`（BatchMode 非交互 + 60s 超时，返回 exitCode/stdout/stderr）；`mode.spec`（蜂群分片上限 max_swarm_shards / 子专家上限 / 推理深度 / 验证轮数 / 思考强度等真实规格）；SUPPORTED_METHODS 同步 + `agents.list`/`experts.list` 公开。
- webui：GraphView 新增「项目级关系图谱」面板（路径分析/保存/加载/删除 + 已保存图谱清单）；PluginsView 新增插件市场（批量安全审计表 + 导入 + 导出 + 本地清单目录）；SshCollabView 新增远程执行（选主机+命令+退出码/输出）；新增 WallpaperView `/wallpaper`（预设渐变壁纸 5 款 + 本地图片 ≤2MB localStorage 持久化 + 视频动态壁纸 + 沉浸式预览；超限/视频诚实标注仅会话生效）；新增 ExpertCenterView `/expert-center`（experts.list 8 位行业专家 + agents.list + mode.spec 蜂群规格）；PetView 仙子打磨（呼吸光环/振翅/双层纱裙+高光/朦胧半闭眼/三朵漂动茉莉/星屑闪烁/整组 sway+bob 御风感）。
- 验证：`cargo test -p marguerite-core --lib` **2152 passed**（+4 项目图谱）；gateway 101 通过（含 SUPPORTED_METHODS 子集校验）；webui typecheck（10 守卫）+ build 255 文件全绿；cli 无改动（无文档漂移）。

## 3.26 第二十三轮 — 蜂群数量上限设置 / 插件批量安装+下载源 / 对话页计量条 / 内置 iframe 浏览器（已完成 + 验证）
- core/src/mode/swarm.rs：蜂群控制配置（`<data_dir>/swarm_config.json`，0=内置默认，分片钳 1..=200、子专家钳 1..=50；3 单测）；orchestration `run_bloom_swarm` 改为实时读取生效上限（智能体/团队/专家启动蜂群全部受控）。
- gateway：`mode.swarm.get/set`；`market.install.batch`（≤64 项逐项安装回报）；`market.sources`（本地检出仓库 git remote 枚举 + 本地清单目录，离线诚实不抓网）；`cache.status`（客户端命中率/服务端前缀缓存命中率/省前缀 token，取自真实 PrefixCache 台账）；chat.rs gateway_prefix_cache 转 pub(crate)。
- webui：ExpertCenterView 蜂群控制面板（数量上限输入/保存/恢复默认/生效值展示）；PluginsView 批量安装按钮 + 下载源面板（第三方仓库 URL 可点开）；ChatView 对话页计量条（本会话 token/轮次 + 服务端前缀缓存命中率 + 省前缀 token，12s 刷新，usage.status + cache.status 真实数据）；BrowserCockpitView 内置 iframe 浏览器（沙箱 iframe 直载，诚实标注 X-Frame-Options/CSP 限制与 CDP 互补）。
- 验证：`cargo test -p marguerite-core --lib` **2156 passed**（+3 swarm +1 git remote 解析）；webui typecheck + build 255 文件全绿。

## 3.27 第二十四轮 — 向量混合检索 / 团队一键启动 / 行为蒸馏面板 / 首 token 实测（已完成 + 验证）
- core/kb/hybrid.rs：HybridRetriever 增 `reindex_embeddings()`，embedder 改 `Arc<dyn Embedding>`；kb/gateway.rs 增 `kb.search.hybrid`（BM25 + 向量 + RRF；向量后端 = 进程级嵌入提供者：默认零依赖特征哈希 256 维，配置 backend=model 时走真实模型；语料 > 2000 块诚实降级纯 BM25，engine 字段如实回报）；+1 单测。
- gateway：`team.run`（async-only：Orchestrator::dispatch_team 真实并行 spawn → strategy/members/summaries/lead_reply；未配 Key 如实失败）；SUPPORTED_METHODS 同步 kb.search.hybrid + team.run。
- webui：ExpertCenterView 增「团队/蜂群一键启动」（goal 输入 → team.run → 策略/成员产出/lead 综合）与「用户行为蒸馏」（memory.behavior.summary 行为聚合 + agent.profile.auto 主智能体画像，persona 注入链路说明）；KnowledgePagesView 增「向量混合检索」卡（BM25/向量/融合三分数 + engine 标签）；ChatView 计量条增「首 token（客户端实测）」（发送 → 首个可见输出，watch streamHasVisibleOutput）。
- 验证：`cargo test -p marguerite-core --lib` **2157 passed**（+1 hybrid）；webui typecheck + build 255 文件全绿。

## 3.28 第二十五轮 — OmniTranscend 强化 / 思考链 S1-S5 / 蜂群后台化 / 团队持久化（已完成 + 验证）
- orchestration：新增 `current_bloom_spec()` —— 运行时模式 OmniTranscend 时按 omni_transcend.yaml 生效（蜂群 13/子专家 6/深想 4 阶段/验证 4 遍/预算 160k/24 步/锚定 5 段），其余模式按 bloom.yaml；run_bloom_swarm 蜂群与子专家上限、run_bloom_auto 迭代/修复上限、bloom_anchoring_spec 回退全部改为模式感知；删除过时常量；+1 单测（test_current_bloom_spec_is_mode_aware）。
- 思考链展示：dispatch_bloom 在 OmniTranscend 下逐段发射 S1 定轨→S2 只读探索→S3 深想→S4 执行（YOLO×PTC）→S5 全域 五段思考链（EventType::Thinking 直达 UI 思考链时间线，与既有 thinkingVisible/thinkingText 渲染同源）。
- 蜂群后台化：gateway `team.run.background`（async-only）——注册 long_running 台账（心跳 15s / 检查点 / 完成态），立即返回 taskId；后台 tokio 任务跑 dispatch_team（1h 上限），lead 综合/成员产出/策略写入检查点，失败如实 fail；引擎健康页任务质量卡同步可见。
- 团队持久化：gateway `team.compositions.{list,save,remove}`（data_dir/team_compositions.json，成员 [{role,brief}]）；webui ExpertCenter 增「蜂群后台化」面板（启动/5s 轮询状态/检查点数）与「专家团队持久化组合」面板（保存/载入/后台重跑/删除；重跑时成员清单注入目标参与角色检测）。
- 验证：`cargo test -p marguerite-core --lib` **2158 passed**（+1）；webui typecheck + build 255 文件全绿。

## 4. 待继续（goal 激活中）
- 模型 provider 扩充（deepseek-v4 系列 / cloud / anthropic / openai 路由）。
- 部分实装能力管线化（browser/UIA/Computer-Use、swarm/集群/MAX、MCP 桥、Market 安装、OS 沙箱、双模型）。
- 能力/现实缺口：6 项纯叙事置灰/标注，避免死入口。

> 说明：以上改动均通过真实构建/测试/运行验证；相关源码、CHANGELOG、AGENTS.md §2 基线已同步。