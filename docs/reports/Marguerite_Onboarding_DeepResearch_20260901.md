# Marguerite 桌面端首次启动引导体验深度研究与改进方案

**摘要**

Marguerite 桌面端（Tauri + Vue3 + Rust 内嵌网关）当前首启动体验存在"零配置即崩溃"的致命缺陷：API Key 未配置时直接触发 401 致命错误、连接状态语义矛盾（WebSocket"已连接"与业务"未就绪"混为一谈）、零值状态栏显示无意义数据。本研究基于对 Cursor、Claude Desktop、Trae IDE、Ollama 四款竞品及 Material Design/Carbon Design System/IBM 官方设计系统的深度对标，提出"四层连接状态模型 + Self-Select/Quickstart 混合 Onboarding 范式"的综合改进方案。实施后用户从安装到首次成功对话的旅程将不再出现红色错误堆栈，而是通过结构化引导卡片在 60 秒内完成配置并进入对话。关键出人意料的发现：Tauri 社区多个独立项目（Thuki/Citadel/MouseClaw）不约而同采用"后端检测 -> 前端引导"模式，形成明确设计范式；Microsoft WebView2 官方明确建议"不要用 WebView2 渲染初始 UI"，与 Tauri 默认全 WebView2 渲染存在潜在性能冲突。

---

## 目录

- [一、问题诊断与现状分析](#一问题诊断与现状分析)
  - [1.1 首启动崩溃路径复盘](#11-首启动崩溃路径复盘)
  - [1.2 连接状态语义矛盾](#12-连接状态语义矛盾)
  - [1.3 零值状态栏与视觉噪音](#13-零值状态栏与视觉噪音)
- [二、竞品对标与最佳实践](#二竞品对标与最佳实践)
  - [2.1 竞品 Onboarding 三大范式](#21-竞品-onboarding-三大范式)
  - [2.2 行业最佳实践演进](#22-行业最佳实践演进)
- [三、连接状态与错误处理体系重构](#三连接状态与错误处理体系重构)
  - [3.1 四层连接状态模型](#31-四层连接状态模型)
  - [3.2 错误消息结构化设计](#32-错误消息结构化设计)
- [四、首次启动引导 UI 实现方案](#四首次启动引导-ui-实现方案)
  - [4.1 Self-Select + Quickstart 混合范式](#41-selfselect--quickstart-混合范式)
  - [4.2 OnboardingOverlay 组件设计草案](#42-onboardingoverlay-组件设计草案)
- [五、零值状态与视觉噪音治理](#五零值状态与视觉噪音治理)
  - [5.1 空状态设计模式](#51-空状态设计模式)
  - [5.2 构建产物自动清理机制](#52-构建产物自动清理机制)
- [六、实施路线图与验证计划](#六实施路线图与验证计划)
  - [6.1 分阶段实施计划](#61-分阶段实施计划)
  - [6.2 验证基准与门禁](#62-验证基准与门禁)
- [结论](#结论)
- [参考文献](#参考文献)

---

## 一、问题诊断与现状分析

### 1.1 首启动崩溃路径复盘

Marguerite 桌面端的首启动崩溃路径经过系统性复现，可精确还原如下：用户安装 MSI 后首次运行 -> 前端 Vue3 应用加载 -> WebSocket 连接到 Rust 内嵌网关 -> 网关报告 WebSocket "已连接" -> 用户看到顶栏绿色 "已连接" 状态 -> 用户在对话区输入任意消息 -> 前端发送 `run_turn` Tauri 命令 -> Rust 后端调用 `run_turn` 函数 -> 函数内检查 `config.model.api_key` -> 发现为空字符串 -> 仍然构造 HTTP 请求发送至 DeepSeek API -> DeepSeek 返回 401 Unauthorized -> Rust 端 panic/返回原始错误 -> 前端渲染红色致命错误堆栈 -> 用户首次体验以崩溃结束。

这一崩溃路径的核心问题在于：系统未在业务层设置"配置完整性校验"（Configuration Sanity Check）闸口。参照 Carbon Design System 对错误管理空状态的定义，当"配置需要"时，系统应"解释问题并提供第一步操作指引"[^1]，而非直接暴露底层 HTTP 错误码。Jakob Nielsen 第九可用性启发法明确指出："帮助用户识别、诊断和从错误中恢复。错误消息应使用平实语言（不用代码），精确指出问题，建设性地建议解决方案。"[^1]

竞品对标显示，Cursor 采用"先提供免费试用价值，再要求配置 API Key"路线——安装后直接可用内置额度，自定义 Key 需手动进 Settings 面板配置，无强制引导向导[^2]。Claude Desktop 采用"账号登录即就绪 + 渐进式功能发现"——首次打开是登录界面，登录后进入与 Web版相似的主界面，左下角输入框有"试试对我说你好"引导文案[^3]。Trae IDE 用"状态栏就绪指示 + 双模式渐进探索"——底部状态栏显示"已登录"作为就绪指示，IDE/Work 双模式可自由切换[^4]。Ollama 走极简命令行路线——安装 -> 验证版本 -> 拉取模型 -> 对话，无 onboarding 界面[^5]。

对比可见，Marguerite 的问题不在于缺少试用额度（这是商业决策），而在于未配置 Key 时的"引导缺失"——用户面对的不是"需要做什么"的引导，而是"出了什么问题"的红色错误。

### 1.2 连接状态语义矛盾

当前 Marguerite 的连接状态存在严重的语义矛盾。前端有两个独立的状态指示器：① 顶栏 conn-pill 显示 WebSocket 传输层状态（disconnected/connecting/connected）；② ModeHintBar 显示业务层状态（activeMode 为空时显示"运行时模式未知，请在顶栏模式菜单确认"）。

问题的根源在于：WebSocket "已连接"（传输层握手完成）并不等价于"业务就绪"（网关加载完配置、模型、API Key 验证通过）。Beekeeper Studio 的连接状态机模式提供了正确示范：采用 Vuex Store 管理连接状态，核心属性包含 `connected`(boolean)、`connError`(string)，连接建立遵循严格异步操作模式——创建连接 -> 获取元数据 -> 更新就绪状态[^6]。Marguerite 当前将这两层混为一谈，导致用户看到"已连接"却被告知"运行时模式未知"，产生强烈的认知冲突。

XState 状态机完整生命周期模型进一步揭示了正确的状态设计：`disconnected -> connecting -> connected -> idle -> processing -> error`，其中 `connecting` 状态有超时机制，`error` 是独立子状态可通过恢复事件复原[^7]。这意味着连接状态应该是有限状态机，有明确转换条件、超时机制和恢复路径，而非简单的布尔值。

### 1.3 零值状态栏与视觉噪音

Marguerite 底部状态栏在首启动时显示大量无意义的零值：`total_tokens: 0`、`runs: 0`、`server_cache_hit_rate: 0%`、`saved_prompt_tokens: 0`、`prefix_hits: 0`。这些零值不仅不提供有用信息，反而传递"功能可能损坏"的负面信号——用户会疑惑"为什么缓存命中率是 0%，是不是哪里出了问题？"

Carbon Design System 将空状态分为三大类型：No data（首次使用/无数据 -> 告诉用户填充后会有什么 + 如何添加）、User action（操作反馈）、Error management（权限/系统/配置问题 -> 直接语言 + 可操作步骤）[^1]。对于 Marguerite 的零值状态栏，正确的处理方式是：当所有指标均为零时，完全隐藏状态栏或显示空状态卡片（"发送第一条消息后这里将显示 token 用量统计"），而非展示无意义的零值。

---

## 二、竞品对标与最佳实践

### 2.1 竞品 Onboarding 三大范式

通过对 Cursor、Claude Desktop、Trae IDE、Ollama 四款主流 AI 桌面应用的首次启动引导进行系统性对标，可归纳出三种截然不同的 Onboarding 范式：

| 应用 | 首启动体验 | 配置触发时机 | 引导强度 | 空状态处理 |
|------|-----------|------------|---------|----------|
| Cursor | 安装后直接可用（内置额度），自定义 Key 手动进 Settings | 首次使用后按需 | 弱（无强制向导） | 空项目列表 + 新建按钮 |
| Claude Desktop | 登录界面 -> 登录后进入主界面，输入框有"试试对我说你好" | 登录即就绪 | 中（渐进式发现） | 空对话区 + 示例文案 |
| Trae IDE | 底部状态栏"已登录"指示，IDE/Work 双模式自由切换 | 首次启动即需登录 | 中（状态指示为主） | 空编辑器 + AI 面板 |
| Ollama | 命令行：安装 -> 验证版本 -> 拉取模型 -> 对话 | 首次使用时命令行配置 | 弱（纯文档驱动） | 终端空提示符 |

上表揭示了关键差异：Cursor 和 Ollama 将配置责任完全交给用户（弱引导），Claude Desktop 和 Trae IDE 在首启动时即建立明确的就绪信号（中引导）。Marguerite 的当前状态介于两者之间——它有一个前端界面（比 Ollama 强），但没有配置引导（比 Claude Desktop 弱），导致未配置 Key 时直接暴露底层错误。

### 2.2 行业最佳实践演进

Userpilot 在测试 30+ SaaS 工具后总结了 Onboarding 的三大演进阶段：1.0 静态导览（tooltip + 欢迎弹窗）-> 2.0 用户分群（按角色/用例提供个性化起点 + 空状态 + checklist）-> 3.0 AI 驱动结果导向（AI 帮用户生成可用产出，60 秒到价值）[^8]。核心原则四条：① 先到达第一个有用结果（首条消息/文件/草稿），不要让用户做完教程才看到价值；② 让下一步显而易见——空状态/提示/模板/checklist 指向单一明确动作；③ 移除延迟激活的设置工作——跳过不必要配置或解释为何需要；④ 适应用户目标——按角色/用例/首个 prompt 调整。

Material Design 官方文档将 Onboarding 分为三种模型：① Self-Select——用户自定义首屏体验，选择必须"有意义且显著 + 提供新信息 + 简短"（单屏 < 10 个），适用于 UI 可定制 + 有设置要求的场景；② Quickstart——用户直接进入应用，不显示任何 onboarding，优先第一个关键动作，如果 90% 首次体验后用户仍不清楚怎么用才显示学习提示；③ Top User Benefits——最多三个受益点自动轮播，适用于新产品/新功能。组合规则：Self-Select 不与 Top User Benefits 组合；可在 FRE 之前或之后展示设置[^9]。

对 Marguerite 而言，最佳选择是 **Self-Select + Quickstart 混合模式**：首启动检测到无 API Key 时进入 Self-Select（引导用户选择提供商并配置 Key），配置完成后进入 Quickstart（直接进入对话界面，预填示例 prompt 避免空白页）。这符合 Userpilot 的"60 秒到价值"基准——用户从安装到首次成功对话应在 60 秒内完成（不含 API Key 获取时间）。

---

## 三、连接状态与错误处理体系重构

### 3.1 四层连接状态模型

基于 XState 有限状态机理论和 Beekeeper Studio 的连接状态机实践[^6][^7]，Marguerite 应建立四层连接状态模型：

| 层级 | 状态名称 | 含义 | 当前问题 | 改进后 |
|------|---------|------|---------|--------|
| L1 传输层 | disconnected/connecting/connected | WebSocket 握手状态 | 与业务层混为一谈 | conn-pill 仅反映此层 |
| L2 业务就绪层 | not_ready/ready | 网关是否加载完配置 | 无此层，导致矛盾 | 新增 readiness 查询 |
| L3 功能可用层 | api_key_missing/model_unavailable | 特定功能是否可用 | 无此层，错误直接暴露 | 新增功能级校验 |
| L4 错误层 | error{kind,message,recoverable} | 错误的分类与恢复路径 | 错误无结构化 | 统一错误信封 |

具体实现上，现有 `runtime_status` 命令已返回 `api_key_configured`/`model_provider`/`model_name`，需扩展为完整 readiness 查询：

```typescript
interface ReadinessState {
  transport: "disconnected" | "connecting" | "connected";  // L1
  gateway_ready: boolean;  // L2
  api_key_configured: boolean;  // L3
  model_provider: string;  // L3
  model_name: string;  // L3
  first_run: boolean;  // 是否首次启动
}
```

前端在 `App.vue` mounted 时调用 `invoke("runtime_status")`，根据 `api_key_configured` 决定是否展示 Onboarding 覆盖层。ModeHintBar 根据四层状态显示不同文案：disconnected -> "正在连接网关…"；connecting -> "正在连接网关…"；connected + gateway_not_ready -> "网关正在加载运行时配置，请稍候…"；connected + gateway_ready + no_api_key -> "API Key 未配置，请进入设置配置"；connected + gateway_ready + has_api_key -> 隐藏提示栏（正常状态）。

### 3.2 错误消息结构化设计

Carbon Design System 对错误管理空状态的设计原则要求："错误消息应使用平实语言（不用代码），精确指出问题，建设性地建议解决方案"[^1]。Marguerite 当前在 `lib.rs:340` 的 `run_turn` 入口已增加 `config.model.api_key.is_empty()` 前置校验，未配置时 emit `api_key_missing` 事件并返回 `Ok(())`，这是正确的方向，但需进一步结构化：

```rust
enum AppError {
    ApiKeyMissing {
        message: String,       // "API Key 未配置"
        action: String,        // "请先进入「设置 -> 提供商」配置 API Key 后再发送消息。"
        cta_label: String,     // "去配置"
        cta_target: String,    // "settings/providers"
    },
    ConnectionFailed { ... },
    ModelUnavailable { ... },
}
```

前端收到 `api_key_missing` 事件后，不应在对话区渲染红色错误堆栈，而应展示结构化引导卡片：包含错误说明（平实语言）、解决步骤（可操作）、CTA 按钮（一键跳转设置）。这符合 Nielsen 第九启发法和 Carbon Design System 的错误管理原则。

---

## 四、首次启动引导 UI 实现方案

### 4.1 Self-Select + Quickstart 混合范式

根据 Material Design 三模型分析，Marguerite 应采用 Self-Select + Quickstart 混合模式[^9]。具体流程如下：

1. **检测阶段**：`App.vue` mounted 时调用 `runtime_status`，检查 `api_key_configured`
2. **Self-Select 阶段**（无 API Key）：全屏 Onboarding 覆盖层，三步向导
   - 步骤 1：欢迎 + 功能介绍（Top User Benefits 模式，3 个受益点轮播）
   - 步骤 2：提供商选择（DeepSeek/OpenAI/Anthropic/Ollama，Self-Select 模式）
   - 步骤 3：API Key 输入 + 验证（实时验证按钮，验证通过后才允许完成）
3. **Quickstart 阶段**（配置完成）：自动进入对话界面，预填 3 个示例 prompt（Starter content 模式）
4. **跳过机制**：提供"稍后配置"按钮，但发送消息时再次拦截引导

这一流程符合 Userpilot 的"60 秒到价值"原则——用户从看到 Onboarding 到进入对话界面的时间应控制在 60 秒内（不含 API Key 获取时间）[^8]。

### 4.2 OnboardingOverlay 组件设计草案

新增 `OnboardingOverlay.vue` 组件，设计要点如下：

- **视觉层**：全屏覆盖层，蓝橙语义令牌配色（与现有设计系统一致），半透明 backdrop-blur 背景
- **内容层**：
  - 顶部：Marguerite Logo + 版本号
  - 中部：三步向导（步骤指示器 + 动态内容区）
  - 底部："稍后配置"跳过按钮 + 隐私声明文案
- **交互层**：
  - 步骤 1 自动轮播 3 个受益点（3 秒/页，可手动切换）
  - 步骤 2 提供商选择使用卡片式网格布局，选中状态用蓝橙令牌高亮
  - 步骤 3 API Key 输入框支持粘贴、显示/隐藏切换，验证按钮调用后端 `validate_api_key` 命令
- **动画层**：使用 Vue3 `<Transition>` 组件实现覆盖层淡入淡出（300ms ease-out），步骤切换使用 slide-in 动画（200ms）

预填示例 prompt（Starter content 模式）设计：
1. "你好，请介绍一下你能帮我做什么？"（基础功能发现）
2. "帮我写一段 Python 代码，实现斐波那契数列的递归算法"（编程助手核心卖点）
3. "总结一下这篇文章的核心观点：[粘贴一段文本]"（文本处理能力展示）

---

## 五、零值状态与视觉噪音治理

### 5.1 空状态设计模式

Carbon Design System 将空状态分为三大类型[^1]，Marguerite 需针对三种场景分别处理：

| 场景 | 类型 | 当前处理 | 改进方案 |
|------|------|---------|---------|
| 首启动无对话 | No data empty state | 空白对话区 | 显示示例 prompt 卡片（Starter content） |
| 首启动零值状态栏 | No data empty state | 显示全零数值 | 完全隐藏状态栏或显示空状态提示 |
| API Key 未配置 | Error management | 红色错误堆栈 | 结构化引导卡片（配置缺失型空状态） |

IBM 的进阶空状态替代方案中，"Starter content"（预建内容让用户直接交互学习）最适合 Marguerite 的首启动空对话区——"当用户感觉可以探索界面而不会产生严重后果时，她会学到更多，也感觉更积极"[^1]。

零值状态栏治理的具体规则：① 只有在至少一个指标有真实非零值时才渲染状态栏；② 全零时用空状态卡片替代（"尚未开始对话，发送第一条消息后这里将显示 token 用量统计"）；③ 避免在零值状态下显示百分比/比率（0% 缓存命中率误导用户以为功能损坏）。

### 5.2 构建产物自动清理机制

旧 dist 目录残留（`dist_old_*/dist_opensquilla_*`）已在之前修复中删除，但需建立自动化机制防止复发。Tauri 的 `beforeBuildCommand` 可添加清理逻辑：

```json
{
  "build": {
    "beforeBuildCommand": "pnpm run build && rm -rf dist_old_* dist_opensquilla_*"
  }
}
```

同时，`.gitignore` 应加入 `gui/src-tauri/webui/dist_old_*` 和 `gui/src-tauri/webui/dist_opensquilla_*` 模式，确保这些临时目录不会被误提交到版本控制。

---

## 六、实施路线图与验证计划

### 6.1 分阶段实施计划

| 阶段 | 任务 | 依赖 | 预计工时 | 验证方式 |
|------|------|------|---------|---------|
| 1 | 后端 readiness 状态查询增强：扩展 `runtime_status` 返回完整四层状态 | 无 | 2h | 单元测试：`runtime_status` 在无配置/有配置/首次启动三种场景下返回正确状态 |
| 2 | 前端 OnboardingOverlay.vue 组件：三步向导 + 提供商选择 + Key 验证 | 阶段 1 | 8h | 组件测试：向导步骤切换、Key 验证流程、跳过机制 |
| 3 | 连接状态四层模型 + ModeHintBar 统一：新增 `useConnectionState` composable | 阶段 1 | 4h | 集成测试：WebSocket 断开/重连/就绪/未配置四种状态下的 UI 一致性 |
| 4 | 零值状态栏空状态卡片：ChatView usageStrip 全零时显示引导文案 | 无 | 2h | 视觉测试：首启动状态栏隐藏、有对话后正常显示 |
| 5 | 构建脚本自动清理：`beforeBuildCommand` 添加 `rm -rf dist_old_*` | 无 | 1h | 构建测试：构建后确认无旧 dist 残留 |
| 6 | 全量验证：verify.sh 7/7 门禁 | 1-5 | 2h | core 2412 passed + webui 3148 passed + clippy L3 零警告 |
| 7 | MSI 重新打包：cargo tauri build | 6 | 3h | 文件校验：Marguerite_0.2.0_x64_en-US.msi 存在且大小合理 |

总计预计工时：22 小时（不含 API Key 获取时间）。

### 6.2 验证基准与门禁

验证基准继承自历史修复的成熟标准：

- **核心库测试**：`cargo test` 2412 passed / 0 failed / 17 ignored
- **前端测试**：`vitest run` 3148 passed / 305 files
- **静态检查**：`cargo clippy --all-targets --all-features -D warnings` L3 零警告
- **构建验证**：`cargo tauri build` 成功产出 MSI
- **安装验证**：MSI 安装后首次启动无红色错误堆栈
- **Onboarding 验证**：未配置 API Key 时显示引导覆盖层，配置完成后进入对话界面
- **状态一致性验证**：WebSocket 断开/重连时 conn-pill 与 ModeHintBar 状态一致

---

## 结论

Marguerite 桌面端的首启动引导体验改进应遵循"四层连接状态模型 + Self-Select/Quickstart 混合 Onboarding 范式"的综合方案。核心判断如下：

1. **必须立即修复的 P0 缺陷**：API Key 未配置时的 401 致命错误已通过 `lib.rs:340` 的前置校验拦截，但需进一步将错误响应结构化为引导卡片（而非仅 emit 事件）。
2. **连接状态语义矛盾是设计层面问题**：当前将 WebSocket 传输层状态与业务就绪态混为一谈，必须引入四层模型（传输层/业务就绪层/功能可用层/错误层）才能根治。
3. **Onboarding 范式选择**：Self-Select + Quickstart 混合模式最适合 Marguerite——首启动无 Key 时引导配置（Self-Select），配置完成后通过示例 prompt 直接进入价值（Quickstart）。
4. **Tauri 社区已形成明确设计范式**：Thuki/Citadel/MouseClaw 等多个独立项目均采用"后端检测 -> 前端引导"模式，Marguerite 应遵循这一社区共识。
5. **性能层面需注意 WebView2 启动优化**：Microsoft 官方建议"不要用 WebView2 渲染初始 UI"[^10]，Marguerite 当前全 WebView2 渲染在冷启动场景下存在优化空间，未来可考虑用轻量原生 UI 作为启动画面。

实施路线图分为 7 个阶段，总计 22 工时，验证门禁为 core 2412 + webui 3148 + clippy L3 零警告 + MSI 成功构建。按此方案实施后，用户从安装 Marguerite MSI 到完成首次成功对话的旅程将不再出现红色错误堆栈，而是通过结构化引导在 60 秒内完成配置并进入对话。

---

## 参考文献

[^1]: Carbon Design System — Empty States Pattern — https://carbondesignsystem.com/patterns/empty-states-pattern/
[^2]: Cursor 新员工入职项目指南 — https://zhuanlan.zhihu.com/p/1959306601128260721
[^3]: Claude Desktop 首次登录与界面导览 — https://blog.csdn.net/sg_knight/article/details/162859039
[^4]: Trae 完全上手指南 — https://blog.csdn.net/u012723673/article/details/158618750
[^5]: Ollama 本地部署全流程指南 — https://cloud.baidu.com/article/5682577
[^6]: Beekeeper Studio 连接状态管理 — https://blog.csdn.net/gitblog_01072/article/details/151133133
[^7]: UML 状态机图核心概念系统性解读 — https://programb.blog.csdn.net/article/details/157583486
[^8]: Userpilot — 10 Onboarding UX Examples and How AI Changes First User Experiences — https://userpilot.com/blog/how-to-onboard-new-users/
[^9]: Material Design — Onboarding — https://m1.material.io/growth-communications/onboarding.html
[^10]: Microsoft — Performance best practices for WebView2 apps — https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance
