# Marguerite 功能实现核查报告

> 核查日期：2026-08-11
> 核查对象：D:\Downloads\Marguerite\marguerite（core/ Rust 引擎 + webui/ Vue3 + gui/ Tauri + tui/）
> 核查方式：并行只读代码审计（4 路 Agent）+ 直接证据复核 + 门禁实测
> 门禁基线：`cargo test -p marguerite-core --lib` → 1166 passed / 0 failed / 7 ignored；clippy 零新增警告；webui 受影响测试 94 passed；vue-tsc 零错误

---

## 零、Bug 修复（优先处理）

### Bug：设置页部分开关打开后会自动关闭（状态未持久化）

**根因（已定位，三层证据）**：
1. **写链路**：设置页保存走 `config.patch.safe` RPC → Rust 网关 dispatch 未实现该方法 → 自动代理到 Python 侧车（`pyengine/src/opensquilla/gateway/rpc_config.py:1018 _handle_config_patch_safe`），写入 opensquilla 完整配置。
2. **读链路**：保存后 `loadData()` 调 `config.get` → **Rust 精简实现拦截**（`core/src/gateway/methods.rs:400 config_get` 只返回 model/mcp/memory/skills/log），**不代理 Python**（`config.get` 命中 dispatch_rust，未落入 `PY_ENGINE_PREFIXES` 或自动代理分支）。
3. **回弹**：webui `useSetupCatalog.ts:775-793` 用精简配置重建全部表单 → `privacy`、`naming`、`memory.auto_capture_enabled`、`squilla_router` 等字段缺失 → 开关被默认值覆盖 → 视觉上"打开后自动关闭"。

**修复（已落地 + 门禁通过）**：
- `core/src/gateway/methods.rs`：新增 `PY_FIRST_METHODS = ["config.get","config.effective","config.patch","config.patch.safe"]`，`dispatch()` 中侧车在线时优先代理 Python（读写同源）；侧车离线回退 Rust 精简实现（行为不恶化）。
- 新增回归测试 `test_config_rpc_read_write_symmetry_listing`（覆盖 4 个方法必须进入 PY_FIRST_METHODS）。
- 门禁：1166 passed（无回归），clippy 仅 4 条预存警告（tools/baota.rs，非本次引入）。

**修复建议（后续）**：若需"纯 Rust 离线模式"也能持久化设置，需为 Rust Config 增加 privacy/naming 等字段并实现 `config.patch.safe` 本地落盘 —— 当前修复保证双引擎模式（用户实际场景）读写对称。

---

## 一、十项功能核查结论

| # | 核查项 | 结论 | 证据要点 |
|---|--------|------|----------|
| 1 | 7 层记忆/上下文/跨项目/压缩/知识蒸馏 | **部分实现** | 分层记忆真实（memory/mod.rs:7-13 七模块）；context.compact 真实（context/session.rs:539 + 测试）；KB BM25 本地库真实（kb/store.rs:107）但 **未暴露 kb.* RPC**；多模态蒸馏（视频/文章/网页/文件）在 Python 侧车 `memory.import.*`（rpc_memory_import.py:1165-1390），**侧车离线即 NOT_IMPLEMENTED** |
| 2 | RAG / 全程联网 / computer use | **部分实现** | web_search（builtin.rs:1619）、web_fetch（:726）、browser CDP（:1129）、system_control（:1432）真实注册并经 ToolRegistry 执行；**网关无独立 web_search./browser.* RPC 直通**，仅经 chat 工具链路 |
| 3 | L3 级别防护 | **部分实现** | guard/mod.rs:25-34 四层 GuardLayer（Syntax/Semantic/Fact/Contract）+ GuardGateway.check 真实；但**无 L0-L3 分级枚举、未注册网关 RPC**（仅引擎内代码审计型护栏） |
| 4 | 宝塔上传前语法/质量校验 | **部分实现** | 仅 `_deploy_*.py` 脚本有 PHP 上传前预检（include+`catch(\ParseError)`，_deploy_d33.py:46-64）；通用链路上传 zip 直接覆盖（deploy_to_baota.py:57-79）**无校验**；`_push.py:48-69` 先推后检无回滚；dry-run/confirm 护栏可被脚本级绕过（40+ 脚本强设 false） |
| 5 | 生成代码遵循开发规范 | **部分实现** | 分层/安全/Token 化大体符合（security/mod.rs:11-15、byok.rs:19,41、capabilities.rs:77-80）；**但**：ChatView.vue 4801 行 / CronView.vue 2578 行 / useSetupCatalog.ts 3810 行超长；emoji 图标违规（本次已修 metaRibbon/PendingQueue/ChannelComposeSurface/SessionsAttentionStrip + 6 locale）；orchestration/mod.rs 5 处 `lock().unwrap()`；"单文件≤300行"条款实际不在 4 份规范中（仅 cli-integration-plan.md） |
| 6 | 长时长/高难度复杂任务 | **部分实现** | plan 拆解/检查点/心跳真实（plan/mod.rs:452-486、runner.rs:87-238、chat.rs:251-269 + e2e 测试）；但 long_running.rs:10 头注释自认"内存态、SQLite 持久化后续接入"、`restore_on_startup` 未接引擎启动、`parallel_run` 为确定性扇出伪并行 |
| 7 | GUI_Agent skill/tool 配置与对话体现 | **未实现** | Marguerite 对 GUI_Agent 仅注释引用（mode/modes/bloom.yaml:2、model/bloom.rs:3）；SkillLoader 只扫自有 skills_dir（6 个 YAML）；SkillEngine 只在 chain/executor.rs、chain/cli.rs 构造，**不在对话路径**；webui 技能页展示的是 Marguerite 自身技能 |
| 8 | Bloom 模型并行对话体现 | **部分实现** | 融合路由决策真实（orchestration/mod.rs:502-583 MAX 两阶段、router.decision、webui RouterFxStrip.vue:28 徽标）；但真正多模型并行 `BloomOrchestrator`（Captain+Copilot `tokio::join!`，model/bloom.rs:74-135）**仅单测引用未接对话**；前端 ensemble 徽章依赖后端从不发送的 usage 事件 → 占位 UI |
| 9 | 会话中文件更改与折叠 | **部分实现** | file.write/edit 工具真实（builtin.rs:443,225,252）、ToolCallTimeline 展示写入字节；但**无 diff 卡片、无代码块折叠 UI**（TextPart.vue:97-134 只有复制按钮）、`apply_patch` 未实现、无独立 file_edit 事件类型（events/envelope.rs:41-70 仅通用 tool_call/tool_result） |
| 10 | 文件/项目预览、沙箱终端 | **部分实现** | artifact 预览真实（html/pdf/markdown/image，artifactPreview.ts:44-57）；**无项目结构树/文件浏览器组件**（仅工作区选择器）；webui 无 xterm 终端；`sandbox.exec`/`terminal.*` **未注册网关 WS 方法**（protocol.rs SUPPORTED_METHODS 无）；capability.status 固定 `runtime_unavailable`（methods.rs:241-253） |

---

## 二、逐项详细证据与修复建议

### 1. 记忆架构（部分实现）
- **已实现**：`core/src/memory/` 七模块（sensory/short_term/long_term/experience/recall/cross_project/behavior），RPC 已注册（methods.rs:309-311 memory.search/save/list、:333-337 cross_project/behavior、:326 context.status、:158-161 context.compact）；压缩 LLM 摘要 + 确定性回退（context/session.rs:539，测试 :664-782）。
- **未实现**：知识库（kb/store.rs BM25）仅被 memory/recall.rs、research/mod.rs 内部使用，**未暴露 kb.* RPC**；多模态知识蒸馏（视频/文章/网页/文件导入）依赖 Python 侧车 memory.import.*，侧车离线即 NOT_IMPLEMENTED（Rust 仅前缀代理 methods.rs:45）。
- **建议**：① 暴露 kb.search RPC（Rust 侧 BM25 已有实现，加一个方法即可）；② 蒸馏能力离线降级为"Rust 侧纯文本 ingest + 摘要"，不依赖侧车。

### 2. RAG / 联网 / computer use（部分实现）
- web_search（DDG+Bing 回退）、web_fetch（SSRF 防护）、browser（CDP 双通道）、system_control（Windows 鼠标键盘）均为真实注册工具且有测试（builtin.rs:2489-2637）。
- **缺口**：这些工具只能经 agent 对话链路（tool_cycle.rs:294）触发，网关无独立 `web_search.*` / `browser.*` RPC 直通方法，外部客户端无法直接调用。
- **建议**：为 web_search / web_fetch 增加网关直通方法（复用现有 ToolExecutor，约 30 行）。

### 3. L3 防护（部分实现）
- guard 四层（Syntax/Semantic/Fact/Contract）真实、GuardGateway.check 语法短路真实。
- **缺口**：无 L0-L3 分级枚举；未注册网关 RPC → webui/外部无法查询或配置防护等级。
- **建议**：将 guard 状态暴露为 `guard.status` RPC，并实现分级配置（guard.mode L0-L3）。

### 4. 宝塔上传校验（部分实现，风险最高）
- **有**：`_deploy_d33.py:46-64` PHP 上传前预检（写 `_lintsrc.txt` 不可执行 → `_lint.php` include+ParseError 判定 → 不通过即清理终止）；`_deploy_unified_fix.py:152-177` 推送前全量 PHP 预检。
- **无**：Python/JS/TS 无任何语法与质量校验；通用链路（deploy_to_baota.py:57-79）上传 zip 直接覆盖；`_push.py:48-69` 先覆盖线上再校验，失败无回滚；dry-run/confirm 可被脚本顶部强设 false 绕过。
- **建议**：① 统一 `_deploy_*` 的预检逻辑为共享模块；② 上传前强制 `python -m py_compile` + `node --check`；③ 先校验后覆盖 + 自动回滚备份；④ 移除脚本级绕过 dry-run 的开关（或改为显式 `--force` 参数）。

### 5. 规范符合度（部分实现）
- 符合：分层（security/mod.rs）、密钥 Aes256Gcm（byok.rs:19,41）、能力面不硬编码（rpc.test.ts:169）、无紫粉渐变硬编码。
- 不符合：超大单文件（ChatView 4801 行等，超"单文件≤300 行"工程纪律）；emoji 图标（**本次已修复**）；生产路径 unwrap（orchestration/mod.rs:417-468）；v1.0"零 Web"与仓库内完整 Vue3 webui 的定位冲突（v1.1 已放开，需明确基线版本）。
- **建议**：超大文件拆分为 composable/组件（建议优先 ChatView）；新增 git hook 阻止 emoji 图标提交（正则见团队 P0-1）；unwrap 改为 expect + 上下文或锁中毒处理。

### 6. 长任务（部分实现）
- 真实：自动拆解（plan/mod.rs:452-486）、run_autonomous 完整循环（runner.rs:87-238，心跳+checkpoint+advance）、e2e 测试（chat.rs:533-589）。
- 缺口：跨会话续跑不生效（SQLite 持久化未接、restore_on_startup 未接入引擎启动）；resume 只翻状态不恢复执行；parallel_run 伪并行。
- **建议**：接入 SQLite 持久化（plan 表 + long_running 表），引擎启动时调用 restore；parallel_run 升级为 tokio 并发子任务。

### 7. GUI_Agent 能力（未实现）
- 无任何 GUI_Agent skill/tool 被 Marguerite 加载；对话中只有工具调用时间线，无 skill 身份标识。
- **建议**：若需要，把 GUI_Agent 的 skills 目录挂载到 Marguerite skills_dir（或实现 skill 目录多源扫描），并在对话事件中增加 skill 元数据。

### 8. Bloom 并行（部分实现）
- 路由决策真实、UI 徽标真实；但多模型并行引擎未接入对话（仅测试），ensemble 数据源缺失。
- **建议**：把 BloomOrchestrator 接入 chat 主循环（captain/copilot 并行后聚合）；后端发送 model_usage_breakdown/ensemble_progress 事件（前端组件已就绪）。

### 9. 文件更改与折叠（部分实现）
- **建议**：① 实现 apply_patch（可复用 edit_file 的 diff 逻辑）；② 新增 file_change 事件类型并在事件总线发布；③ TextPart 增加代码块折叠（可折叠 >60 行代码块）。

### 10. 预览与沙箱终端（部分实现）
- **建议**：① webui 增加项目文件树（复用 sandbox.path.list）；② 将 sandbox.exec / terminal.session 注册为网关 WS 方法（core/src/sandbox/rpc.rs:34,40 已有子进程执行实现，缺注册）；③ webui 集成 xterm 终端面板。

---

## 三、本次已修复清单

| 文件 | 修复内容 | 验证 |
|------|----------|------|
| core/src/gateway/methods.rs | config.* 读写对称代理（PY_FIRST_METHODS）+ 回归测试 | cargo test 1166 passed；clippy 零新增 |
| webui/src/utils/chat/metaRibbon.ts | STATE_GLYPH ⚙✓✗→●√×；failedSummary ✗→× | metaRibbon.test 2 passed |
| webui/src/components/chat/PendingQueue.vue | 📎 → Icon paperclip | PendingQueue.test 11 passed |
| webui/src/components/channels/ChannelComposeSurface.vue | ✓ → Icon check | vue-tsc 通过 |
| webui/src/components/sessions/SessionsAttentionStrip.vue | ✓ → Icon check | test 2 passed |
| webui/src/locales/{en,zh-Hans,de,es,fr,ja}.json | 剥离 5 处 ✓/✗ 前缀 | JSON 全部合法；94 tests passed |
| webui/src/components/setup/SetupProviderPanel.test.ts | 4 处断言去 emoji | 81 tests passed |

## 四、遗留事项（建议后续推进）

1. **纯 Rust 离线模式设置持久化**：为 Rust Config 增加 privacy/naming 字段 + 本地 config.patch.safe。
2. **kb.search RPC 暴露**（Rust BM25 已有，补方法）。
3. **guard.status / L0-L3 分级 RPC**。
4. **宝塔上传预检统一化**（共享模块 + py_compile/node --check + 先检后推 + 回滚）。
5. **超大文件拆分**：ChatView.vue（4801 行）、useSetupCatalog.ts（3810 行）。
6. **长任务 SQLite 持久化 + 启动恢复**。
7. **BloomOrchestrator 接入对话 + ensemble 事件**。
8. **sandbox.exec / terminal 网关注册 + webui 终端面板 + 项目文件树**。
9. **GUI_Agent skills 目录挂载**（按需）。
