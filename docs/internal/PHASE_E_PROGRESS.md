# Phase E — 自进化（记忆台账 / 经验沉淀）进展

> 状态：经验台账模块（SQLite + 语义检索）已实现并通过 6 个单元测试；Tauri 暴露 `record_experience` / `search_experience` 命令；React 前端新增「经验台账」面板（检索 + 沉淀）。全链路验证通过。
> 全程未做任何 git 提交（用户要求）。
> 前置：A4.4（真实引擎埋点）✅ / Phase C（托盘+快捷键+菜单+实时文件监听）✅ / Phase D（GUI 对话闭环）✅。

## 一、设计原则

建立在既有 `core/src/memory/` 三层记忆底座之上，**不另起炉灶**：

| 层 | 既有模块 | 职责 |
|----|----------|------|
| 感知 | `SensoryMemory` | 当前会话上下文（内存 + SQLite） |
| 短期 | `ShortTermMemory` | 跨会话 FTS5 全文检索 |
| 长期 | `LongTermMemory` + `LocalEmbedding` | 特征哈希向量 + 余弦相似度语义检索 |

Phase E 在其上叠加**结构化经验沉淀层**，复用 `LocalEmbedding::embed/cosine/dim`（零新增依赖）。

## 二、新增内容

### 1. `core/src/memory/experience.rs`（新模块）

- `ExperienceKind`：`Pitfall`（踩坑）/ `Decision`（ADR 式）/ `Pattern`（可复用解法）。
- `Experience`：结构化条目（id / kind / title / body / tags / signature / validated / created_at）。
- `ExperienceLedger`：SQLite 持久化 + 语义检索。
  - `record(...)`：**幂等**（同 `signature` 更新而非重复插入，防重）。
  - `search(query, limit)`：复用 `LocalEmbedding` 余弦相似度，阈值 0.25 过滤无关。
  - `list_by_kind` / `mark_validated`（踩坑闭环：修法被真实运行验证）/ `count`。
- `SelfEvolution` 编排器（自进化闭环）：
  - `consolidate_session(sensory)`：会话有价值内容 → 长期记忆 + 经验台账双重固化。
  - `recall(query, limit)`：执行前召回相关经验，让 Agent 越用越聪明。

### 2. `core/src/memory/mod.rs`

注册 `pub mod experience` 并 re-export `Experience / ExperienceKind / ExperienceLedger / SelfEvolution`。

### 3. `core/src/memory/long_term.rs`（最小改动）

`LocalEmbedding::embed` / `cosine` 由私有升为 `pub`，新增 `pub fn dim()`，供台账复用。

### 4. Tauri GUI（`gui/src-tauri/src/lib.rs`）

- `AppState` 新增 `evolution: Arc<Mutex<SelfEvolution>>`（Connection 非 Sync，以 Mutex 包裹满足 Send+Sync）。
- 新命令：`record_experience`（沉淀）、`search_experience`（召回），已注册进 `invoke_handler`。

### 5. React 前端（`gui/src/App.tsx` + `App.css`）

- 顶栏新增「经验台账」切换按钮；开启后工作区变三栏（对话 / 事件流 / 经验台账）。
- 经验面板：语义检索框 + 结果列表（类型徽章 + 已验证标记）+ 沉淀表单（类型选择 / 标题 / 内容）。
- P0 合规：蓝主色 `#3B82F6`、无 emoji 功能图标、无紫粉渐变、务实中文文案。

## 三、验证结果

| 项 | 结果 |
|----|------|
| `cargo test -p marguerite-core --lib` | **620 passed / 0 failed**（含 6 个新 experience 测试） |
| `cargo check -p marguerite-gui` | ✅（仅既有 core 警告） |
| `npm run build`（gui 前端） | ✅ 149KB JS |
| `npx tsc --noEmit`（严格类型） | ✅ 零错误 |

## 四、关键技术点 / 踩坑（已记入记忆复用）

- **rusqlite 列顺序陷阱**：`search` 的 `SELECT` 必须包含实际要解析的 `vector` 列；漏选会把 `signature` 当向量解析（长度≠`dim()` 全被过滤），导致检索永远空。已修复并补测试。
- `LocalEmbedding` 复用：特征哈希嵌入是无外部模型依赖的语义检索基线，台账直接复用，避免重复实现。
- `SelfEvolution` 含 `Connection`（Send 非 Sync），在 `Arc<AppState>` 中必须以 `std::sync::Mutex` 包裹。

## 五、自进化闭环补全（本轮，①②③ 全部完成）

> 用户指令：① run_turn 完成后由 LLM 自动抽取踩坑/决策写入台账；② AgentRuntime/ChainExecutor 构造提示时注入 recall 的相关经验；③ 台账 DB 从临时目录迁到应用数据目录。

### ① 自主提取（run_turn 完成后 LLM 自动抽取）

- `RealEngine::run_turn` 在 `AgentRuntime::process_input` 成功返回后，**best-effort** 调用 `extract_experiences(...)`：
  - 用 `MARGUERITE_API_KEY`（回退 `config.model.api_key`）构造 `LLMClient`，提示模型输出严格 JSON 数组（kind/title/body/tags）。
  - 解析（容错：`extract_json_array` 能从夹杂解释文字中截取 `[...]`）+ 逐条 `evolution.ledger.record(...)`。
  - 任一环节失败仅 `eprintln!` 记日志，**绝不阻断本轮对话**。
  - 无 API Key 直接跳过。

### ② 经验注入（越用越聪明）

- `AgentRuntime` 新增 `experience_hint: Option<String>` + `set_experience_hint(...)`。
- `AgentRuntime::process_input` 在调用 `chat_for` 前，将 hint 作为 **`Message::System`** 前置到模型消息序列（不污染可见会话历史，已被 `test_process_input_injects_experience_hint_as_system` 验证）。
- `RealEngine::run_turn` 每轮先 `recall_hint(&evolution, &text)` 召回 ≤5 条相关经验并格式化注入。
- `ChainExecutor` 同步新增 `set_experience_hint`，技能步骤执行时前置到输入文本（为后续文件上下文走调用链预留）。

### ③ 持久化路径迁移（应用数据目录）

- 新增 `Config::data_dir()` → `dirs::data_local_dir()/Marguerite`（Windows `AppData\Local\Marguerite`；macOS/Linux 对应）。
- `AppState::new()` 改用 `Config::data_dir()` 落 `marguerite_experience.db` / `marguerite_ltm_gui.db`，跨会话持久、不再落临时目录。

### 接入点

- `AppState::new()` 构造 `evolution` 后，经 `RealEngine::from_config(Config::default()).with_evolution(Some(evolution.clone()))` 注入同一 `Arc<Mutex<SelfEvolution>>`，使 GUI 同时具备「召回注入 + 自主抽取」能力；TUI 保持 `None`（不接，无额外 LLM 调用）。

## 六、验证结果（本轮）

| 项 | 结果 |
|----|------|
| `cargo check -p marguerite-core --tests` | ✅ |
| `cargo check -p marguerite-gui` | ✅ |
| `cargo test -p marguerite-core --lib` | **620 passed / 0 failed**（新增 `test_process_input_injects_experience_hint_as_system`、`test_extract_json_array_*`、`test_recall_hint_empty_without_evolution`） |
| `npm run build`（gui 前端，重建 dist） | ✅ 149KB JS + 5.5KB CSS（dist 曾因 safe-delete 拦截损坏，已用 PowerShell 清目录后重建） |
| `cargo build --release -p marguerite-gui` | 进行中（后台） |

## 七、下一步

转 **Phase F 发布**（打包 / 签名 / 更新通道）。详见 `PHASE_F_RELEASE.md`。
