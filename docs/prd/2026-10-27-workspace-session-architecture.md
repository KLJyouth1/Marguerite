# PRD：工作区会话页架构 —— 独立性 · 关联性 · 产物渲染 · 清空 · 问答回合体系

> 日期：2026-10-27
> 性质：**设计提案**（本文档全部为设计与规划，不代表任何条目已实现；实现前须按 §10 门禁逐项验证）
> 现状依据：2026-10-27 对 `core/src` 与 `webui/src` 的逐文件调研（文中路径:行号均为当日快照，
> 存在并发写入者，落地前须重新核对 mtime）。
> 命名约定：本文「工作区」= 项目工作区（`workspaces.json` 注册表条目）；「会话页」= `/control/chat`
> 的单会话视图；「会话」= `sessions.db` 中一行 + `context/<key>.json` 台账。

---

## 0. 现状快照（设计的事实地基）

### 0.1 会话模型

| 层 | 事实 |
|---|---|
| 持久化 | SQLite `~/.marguerite/sessions.db`（`core/src/persistence/session.rs`），表 `sessions` + `session_messages`（外键级联）；第二轨：`data_dir/context/<session>.json` 上下文台账（4 层渐进压缩，含流式 `partial_text`） |
| 元数据 | `SessionInfo`：summary / message_count / state / tokens_used / project_path + project_paths（多根）/ reasoning_effort / context_window |
| RPC | `sessions.list/create/delete/get/search/patch/reset/preview/subscribe/abort`、`chat.history/send/abort`、`sessions.export/import`（`core/src/gateway/sessions.rs`） |
| 前端 | **无 sessions Pinia store**：`useSessions.ts`（组合式单例）持有列表，含 missing-tolerance（2 轮）与自愈重试 [1s,3s,8s]；当前会话 id 三处同步（URL `?session=` + `localStorage` + ChatView 局部 ref，`useChatSessionRoute.ts`）；草稿路由 `/chat/new` 首条消息才固化 |

### 0.2 会话 × 工作区

- 会话多对一绑定工作区：DB `project_paths` JSON 列（`project_path` = 主根），`bind_projects` 双写进程 map + 台账（`gateway/chat.rs:1924-1968`）。
- 前端侧栏按项目分桶渲染 workspace 头行（`useSessions.ts:689-764`）。
- **跨会话共享域（现状）**：全局记忆 `remember/recall`（**不分工作区、不分会话**，`core/src/memory/inject.rs`）；工作区目录本身；产物台账 `artifacts.list { allSessions: true }`。

### 0.3 隔离现状（前端切换会话时）

`switchSession`（`useChatSessionRuntime.ts:108-146`）：重置 epoch/streamSeq/activeTaskGroups → 清 messages/usage/pendingQueue → 重新 bootstrap 拉历史。审批/澄清卡随 `watch(sessionKey)` 全清重水合（`useChatApprovals.ts:1069-1080`）。后端按会话键管理运行句柄（`gateway/chat.rs:855-890`），删除会话先停任务再删库。

**已有跨会话通道**：fork（`useChatForkController.ts` + `session.epoch_changed`）、`@` 引用历史会话（摘要+截断 ≤3000 字符，`gateway/context_refs.rs`）、跨会话全文搜索（`sessions.search`）、侧聊派生。

### 0.4 产物渲染现状

分类事实源 `webui/src/utils/chat/artifacts.ts`（MIME 表 + 扩展名表），四分渲染 `ChatArtifactList.vue`：

| 类别 | 现状 | 缺口 |
|---|---|---|
| 图片 visual | 懒加载缩略图 + 重试卡 + App 级 Lightbox | — |
| 音频/视频 | 显式 Play 契约（点播才拉鉴权字节）+ 状态机 + 降级下载 | — |
| 文档 document | pdf/html/md/txt 可预览；Chip 卡 Open/Download | xlsx/docx 无 MIME 条目，落 `file` 只能下载 |
| 数据 data | csv/json/ipynb 归 data 但 **download-only** | 无表格/JSON 树预览 |
| 3D / 工程文件 | **完全没有分类**（glb/gltf/obj/stl/fbx/step/dwg 全落 `file`） | 无查看器；`components/three/` 只有辉光贴图工具 |
| 代码 code | sql 归 code，download-only | — |

### 0.5 清空 / 删除现状

- `sessions.reset` = 只清消息保留会话（**无 UI 入口**，仅斜杠命令 `/reset`）；`sessions.delete` = 物理删除（停任务 + 删台账 + 删 DB 行，级联消息与压缩快照）。
- 确认走 `useConfirm.ts` + `ConfirmModal.vue`，结果走 toast；侧栏支持行级与批量删除，当前会话被删落回新草稿。
- **无归档、无回收站**。唯一近似物是 `sessions.export`。

### 0.6 问答回合现状（四套并存）

| 机制 | 后端 | 前端 | 形态 |
|---|---|---|---|
| 工具审批 approval | `guard/approval_registry.rs` + REST `/api/approvals` + WS `session.event.approval`；裁决 `allow-once/allow-always/deny` | `useChatApprovals.ts` → `ApprovalCard.vue`（经 `InterruptPart.vue`） | 聊天流内联卡 |
| 澄清问卷 clarify | `gateway/clarify.rs` + `mode/ask.rs::QuestionQueue`（fail-closed）；载荷 `clarify_schema.fields[]`（choice/string + allow_other），阶段推进换 request_id | `submitClarify` → `ClarifyCard.vue` | 聊天流内联表单卡 |
| 升级确认 escalation | `gateway/escalation_rpc.rs`：`escalation.ack/pending`；触发类型 4 种（高危副作用/意图歧义/超时/预算超限） | `useChatEscalations.ts`（同一时刻仅 1 条 pending，其余排队）→ `EscalationCard.vue` | 聊天流内联卡 |
| 本地确认 ConfirmModal | —（纯 UI 动作，与智能体无关） | `useConfirm.ts` 单例 Promise | 模态 |

**已知结构性问题**：① 审批传输层分裂（WS 推送 + REST 裁决 + Rust/侧车双队列合并）；② 智能体没有「通用提问」工具——clarify 的 QuestionQueue 是 fail-closed 队列，模型无法在任意回合主动发起带 schema 的表单式提问并阻塞等待；③ 问答状态水合依赖前端会话切换全清重拉，后台会话的 pending 问答无汇聚视图；④ Rust 侧实时流快照声明式降级（`degraded: true`，实时态由 Python 侧车供给）。

---

## 1. 设计原则（判断一切争议的依据）

- **P1 会话是默认隔离单元**：消息、流式输出、工具中间态、待发队列、问答卡片，一律按会话键隔离；任何跨会话能力（搜索、引用、共享产物）必须显式声明、显式进入 UI。
- **P2 关联是显式的、可追溯的**：会话之间不共享「隐式上下文」。工作区内共享的只有三类资源——文件、产物台账、记忆注入——且每类在 UI 上可见其来源。
- **P3 产物按类型感知渲染**：能安全预览的内嵌预览，不能预览的诚实降级为下载卡；预览分级可配置，「显式加载」是大媒体文件的默认契约（沿用音视频现状）。
- **P4 破坏性操作可解释、可恢复**：清空 ≠ 删除 ≠ 归档，三个动作的 UI 文案必须说明影响范围；删除默认软删除。
- **P5 智能体问用户 = 统一「问答回合」协议**：审批、澄清、升级、通用提问共用一套 request_id / 生命周期 / 超时 / 恢复语义，前端按类型选形态（内联卡为主，模态为例外）。
- **P6 双引擎不外泄**：问答与状态聚合必须让 UI 感知为「一个队列」，Rust/侧车双源差异在网关层抹平。

---

## 2. 会话页独立性设计（§用户问题 1、2）

### 2.1 三域状态模型

把 UI 全部状态划入三个域，作为后续一切组件归属的判据：

| 域 | 内容 | 生命周期 | 归属 |
|---|---|---|---|
| **会话私有域** | messages、流式 epoch/seq、activeTaskGroups、pendingQueue、审批/澄清/升级卡、usageAccum、滚动态、草稿文本 | 随会话键建立/销毁 | ChatView + `useChatSessionRuntime`（现状已基本如此，保留） |
| **工作区共享域** | 工作区文件树、工作区产物台账、工作区级规则 | 随工作区打开 | 各专用视图（`/artifacts`、`/editor`） |
| **全局域** | 全局记忆、模型路由、3×3 用户模式、主题、通知设置 | 随应用 | `stores/app.ts` 等 Pinia store |

**规则**：组件新增状态时必须声明归属域；禁止把会话私有状态放进全局 store（现状合规——保持「无 sessions store、由组合式单例持有」也可接受，若未来重构成 Pinia，`useSessions.ts` 的 missing-tolerance 与重试预算语义必须原样保留）。

### 2.2 独立性验收清单（每项须有测试）

1. 切换会话后旧会话的流式事件不得串扰（epoch/seq 门控，现状已有——补契约测试）。
2. 旧会话仍在后台运行时：侧栏行显示运行态（`sessions.changed` 补丁，现状已有）；切回时经 `partial_text` 恢复「正在生成」占位（`usePartialTextRestore.ts`，现状已有）。
3. 审批/澄清卡按会话键水合：A 会话的 pending 审批不得出现在 B 会话的聊天流（现状已按会话重水合——补测试）。
4. 删除当前会话 → 落回新草稿，不残留旧 DOM 状态（现状已有，保留）。
5. **补强**：滚动位置、未发送草稿文本目前切走即丢 → 移入会话私有域缓存（`Map<sessionKey, {scrollTop, draft}>`，P1）。

### 2.3 「相对独立性」

同工作区会话之间**不是**完全隔离——共享的只有：工作区文件系统、产物台账、记忆召回。UI 要求：
- 会话内引用工作区产物时展示其来源（哪个会话产出 / 哪次工具写入）——`artifacts.get` 元数据补 `produced_by_session` 字段（P1，RPC 三处齐全：methods + 派发 + protocol，见仓库硬约束）。
- 记忆召回条目注入时标注「来自全局记忆」，与「来自本会话上下文」视觉区分（P2）。

---

## 3. 会话关联性设计（§用户问题 3）

### 3.1 现有四条通道（保留并显性化）

| 通道 | 现状 | 补强 |
|---|---|---|
| fork 分叉 | `useChatForkController` + `forkedFromParent` 标记 | 会话卡/侧栏 tooltip 显示「fork 自某会话」（数据已有，UI 未展示） |
| `@` 历史会话引用 | 摘要+截断拼进消息 | 引用块在消息气泡内渲染为可点击卡（点击跳转源会话），而非纯文本尾巴 |
| 跨会话搜索 | `sessions.search`，命令面板入口 | 结果卡直接内联预览命中片段（`>>> <<<` 高亮已有），P1 |
| 侧聊 | 派生独立会话键 | 侧聊与主会话在侧栏成组显示（现状被过滤——改为折叠组） |

### 3.2 工作区内关联（新增，P2）

- **会话脉络图**：工作区详情页（或 `/sessions` 复活后）按 fork 树 + 引用关系渲染 DAG，节点 = 会话，边 = fork/引用。
- **产物血缘**：产物卡片显示「由哪个会话、哪个工具产出」；同一文件被多会话修改时显示版本链（`file_change` 台账已具备数据基础）。

---

## 4. 产物渲染矩阵（§用户问题 4）

### 4.1 分类体系扩展

`ARTIFACT_EXTENSION_CATEGORIES` 新增（`webui/src/utils/chat/artifacts.ts`）：

| 新类别 | 扩展名 | 预览等级 |
|---|---|---|
| `model3d` | glb / gltf / obj / stl / fbx（只读显示）；step / dwg / blend / psd / ai 等专业工程格式 | 见 4.3 |
| `sheet` | xlsx / xls / csv / tsv | 表格预览（前 N 行） |
| `archive` | zip / tar / gz / 7z | 永远 download-only，Chip 显示内含条目数（后端补） |

MIME 表同步补 xlsx/docx/doc/pptx；未识别类型仍落 `file`。

> **P0 实现口径（2026-10-28）**：xlsx / xls / xlsm / ods / csv / tsv 与 zip / tar / gz / tgz / bz2 / xz / 7z / rar、
> glb / gltf / obj / stl / fbx / step / dwg 已落表，**一律 download-only（L0）**；`canPreview` 未放宽。
> **docx / doc / pptx 刻意不落表**：还没有任何预览器（mammoth 属 P1），登记成 `document` 只会给出点开是乱码的
> Open 按钮，所以它们继续落 `file`（文件图标 + 下载），等 P1 预览器一起补。psd / blend / ai 同理留在 `file`。

### 4.2 预览分级（四级，全类型统一）

| 级别 | 形态 | 适用 |
|---|---|---|
| **L0 Chip** | 图标 + 名称 + 徽标 + 大小，Open/Download | archive、未知类型、超大文件（>50MB） |
| **L1 内嵌卡** | 聊天流内直接渲染主体 | 图片、音视频（显式加载）、md/txt/log、代码片段 |
| **L2 弹层** | Lightbox / 全屏 Modal | 图片放大（现状）、PDF、表格（虚拟滚动 + sheet 切换）、JSON 树、3D 查看器 |
| **L3 面板** | 跳转 `/editor` 带会话上下文 | 可编辑文本类；后续 3D 工程的「编辑」意图 |

### 4.3 各类型规格

- **3D（P1 重点，当前最大空白）**：
  - L2 查看器：`<model-viewer>`（glb/gltf 零依赖起步）或轻量 three 加载器；支持旋转/缩放/线框切换；鉴权拉取同音视频契约（显式加载，点开才拉字节）。
  - obj/stl：先转 glb（侧车 Python trimesh）或前端 loader；不支持的格式（step/dwg 等 CAD 源文件）诚实显示「暂不支持预览，提供转换/下载」。
  - 工程源文件（.blend/.psd/.max 等）永不做假装预览——L0 Chip + 「用系统程序打开」。
- **表格（P1）**：csv/tsv 前端直接解析虚拟滚动预览；xlsx 经侧车转 JSON（或 SheetJS）后同管；单 sheet >10k 行只载前 500 行 + 「完整查看 → L3」。
- **文档（P1）**：docx 走 mammoth 转 HTML 只读预览；pptx 列缩略图占位（P2）。
- **数据（P1）**：json 树形折叠查看；ipynb 渲染 cell + 输出（P2）。
- **音频/视频**：维持现状显式加载契约（这是正确设计，写进守卫测试防回退）。
- **生成型产物（ImageGen / VideoGen）**：生成完成的事件卡片附「重新生成 / 变体 / 下载」快捷动作，产物本体仍是标准 artifact（现状架构不变，只补动作区）。
- **失败态统一**：沿用图片卡的超时/失败/重试三态规格，扩展到全部 L1/L2 类型；鉴权 401 显示「重新登录」而非裸报错。

### 4.4 「更多清空类内容」的显示（衔接 §5）

产物被「清空会话」后：**产物台账独立于消息**，清空消息不删产物；产物卡在消息被清后仍可从 `/artifacts`（我的文件，`allSessions` 聚合）按会话过滤找回。设计上把「消息流产物引用」与「产物台账记录」明确为两回事。

---

## 5. 清空 / 删除 / 归档（§用户问题 5）

### 5.1 语义三分（核心决策）

| 动作 | RPC | 影响 | UI 入口（现状 → 目标） |
|---|---|---|---|
| **清空消息** | `sessions.reset`（已存在） | 只删消息，保留会话壳与产物台账 | 仅斜杠命令 → **补 UI**：侧栏 ⋯ 菜单「清空对话」 + 会话内命令面板 |
| **归档** | 新增 `sessions.archive`（`state='archived'`，复用 `sessions.patch` 亦可） | 侧栏默认隐藏，可筛选查看；随时恢复 | 新增：侧栏 ⋯ 菜单「归档」 |
| **删除** | `sessions.delete` | **P1 起软删除**：`deleted_at` 时间戳 + 30 天保留期；到期物理清理（定时任务） | 现状有；确认弹窗文案升级 |

### 5.2 确认弹窗文案规范（P4 原则落地）

每条确认必须回答三个问题：影响什么、保留什么、能否撤销。

- 清空：「将删除本会话的 **N 条消息**，会话设置与**产出的文件保留**。此操作不可撤销。」
- 归档：「会话将从列表隐藏，随时可在『已归档』中恢复。」（低风险，首次免确认，toast 内嵌「撤销」）
- 删除运行中会话：「该会话正在运行任务，删除将**中止任务**。文件保留在『我的文件』，30 天内可在回收站恢复。」
- 批量删除：逐行列出 + 汇总数；部分失败沿用现状 toast（区分成功/失败数）。

### 5.3 回收站（P1）

- `SessionsView`（已退出主导航，保留深链）增加「回收站」分区：列表显示 deleted_at + 恢复/彻底删除按钮。
- 运行中会话不可软删：先 `sessions.abort` 再删（后端已有停任务前置，补「必须显式中止」的用户提示）。

### 5.4 反馈

全部走 toast（现状机制不变）；归档/恢复的 toast 必须带「撤销」动作按钮（`useToasts` 需支持 action slot，P1 小改）。

---

## 6. 问答回合体系（§用户问题 6：Q&A 弹窗怎么出现、怎么设计、智能体怎么识别与调用）

### 6.1 统一协议：所有智能体→用户的提问收敛为一个模型

```
QuestionTurn {
  request_id: 全局唯一，阶段推进换新（沿用 clarify 现状）
  kind:        approval | clarify | escalation | ask_user   ← 新增第 4 类
  session_key: 归属会话（决定渲染位置与水合）
  schema:      字段列表（choice / string / confirm / multi_choice / file）
  payload:     approval=工具意图预览；ask_user=模型自然语言问题
  deadline:    可选超时；超时策略 kind 各自定义
  state:       pending → answered | dismissed | expired
}
```

- **传输收敛（P1）**：REST `/api/approvals` 的双队列合并保留在网关层，但**推送统一走 WS** `session.event.question`（approval 事件名保留兼容别名一个版本）；前端只认一个队列。
- **生命周期（P4）**：pending 卡片可被用户「稍后处理」（收进会话徽标，非丢答）；会话切换后水合只恢复 pending 项（现状已按会话重水合，保留并测试化）。

### 6.2 出现方式策略（内联为主、模态为例外）

| 场景 | 形态 | 理由 |
|---|---|---|
| 当前会话、用户在场 | **聊天流内联卡**（现状 approval/clarify/escalation 三类已如此） | 保留上下文、可回滚查看、不打断阅读 |
| 不可逆高危操作（删除/覆盖/发布） | 内联卡 danger 态 + 输入框二次确认（现状 M2-C1 已有意图预览，保留） | 模态易造成「确认疲劳」盲目点确定 |
| 用户切走、后台会话产生问答 | 侧栏会话行**问答徽标**（计数） + 汇聚入口 + 系统通知（复用 `useTurnNotification` 通道） | 跨会话问答不能等用户切回才发现 |
| 同屏 >1 条 pending | 按序内联 + 侧栏徽标计数；**不弹队列式模态** | 升级类「同一时刻仅一条 pending」现状保留 |

**结论**：本 PRD 明确「Q&A 不以弹窗为主形态」——聊天流内联卡是主形态，ConfirmModal 只用于**纯 UI 动作**（与智能体无关，现状边界正确，写进守卫）；「弹窗感」通过徽标 + 通知 + 汇聚面板实现。

### 6.3 智能体如何识别与调用（第 4 类 `ask_user`，P1 核心新增）

- **模型侧**：注册 `ask_user` 工具（进 `tools/builtin.rs` 注册表 + 工具目录/能力卡），schema 即上表 fields；工具调用阻塞回合直到 answered/expired（复用 `mode/ask.rs::QuestionQueue`，去掉 fail-closed 对模型的不可达——改为：模型声明了工具即可发起，队列只管路由与超时）。3×3 模式的「咨询(Ask)」档在系统提示中引导模型优先用 `ask_user` 而非自答。
- **守卫侧**：`ask_user` 属零副作用工具，永不进审批队列；频率上限（同回合 ≤3 次）防骚扰，超限走 escalation（`intent_ambiguity`）。
- **前端**：`ClarifyCard.vue` 泛化为 `QuestionCard.vue` 按 kind 分支渲染（approval 意图预览 / clarify 表单 / escalation 双按钮 / ask_user 自由问答 + 表单），复用 `submitClarify` 通道扩展 `chat.ask_submit`。
- **RPC 三处齐全**（仓库硬约束）：`methods.rs` 注册 + 派发 + `protocol.rs` 声明，`check-rpc-contract` 拦截遗漏。

### 6.4 汇聚中心（P2）

`/control` 顶栏新增「待回应」铃铛：跨会话聚合全部 pending QuestionTurn，点击跳转对应会话并高亮卡片；解决「问答散落在后台会话里」的寻回问题。

---

## 7. 分期落地

| 期 | 内容 | 依赖/风险 |
|---|---|---|
| **P0**（纯前端 + 零后端风险） | §2.2 隔离契约测试补齐 ✅；清空确认文案与 `sessions.reset` UI 入口 ✅（**归档未做**：`sessions.archive` 后端尚不存在）；产物分类表扩展（model3d/sheet/archive → L0/降级）✅；「fork 自」tooltip ✅（**仅在服务端下发 `forked_from_parent` 时出现**） | ⚠️ ChatView 一带是并发写者活跃区，动手前先核对 mtime（Z-B2） |
| **P1**（前后端） | 软删除 + 回收站；表格/docx/json 预览（L1/L2）；3D 查看器（model-viewer 起步）；`ask_user` 工具全链路；WS 推送统一；滚动/草稿会话缓存；产物 `produced_by_session` | `ask_user` 触及 `mode/ask.rs` 与 pyengine 侧车，须双引擎同步；RPC 三处齐全 |
| **P2** | 会话脉络图与产物血缘；汇聚中心；pptx/ipynb 预览；记忆来源标注；侧聊折叠组 | 纯增量，无迁移 |

> **P0 的两条诚实边界（2026-10-28 实测）**：
> 1. **「fork 自」徽标的显示条件不是前端能决定的**。`forked_from_parent` / `parent_session_key` 只有 pyengine
>    `gateway/rpc_sessions.py` 的会话行会下发（实测 `rpc_sessions.py:1936-1941`）；**Rust core 既没有 `sessions.fork`，
>    也不产出这两个字段**（`grep -rn "sessions\.fork\|forked_from_parent" core/src` = 0 命中）。所以纯 Rust 托管下
>    徽标与 tooltip 不会出现 —— 前端已按「字段缺失即不显示」处理，不是壳上写死的假 UI。
> 2. **归档整条未做**：`sessions.archive` 在 core / pyengine / webui 三侧均 **0 命中**，core 也没有会话 `state` 字段
>    （只有 `sessions.delete`）。§5 里「归档 = 新增 `sessions.archive`」仍是设计，不是现状。

## 8. 验收门禁（每期完成前）

`bash scripts/verify.sh --clean --all` 全绿（`[SKIP]` 不算通过）；webui 侧 `npm run typecheck` + `test:unit` 不回退基线（当前 4898 passed / 0 failed / 495 files，2026-10-28 口径）；新增 RPC 过 `check-rpc-contract`；动效/色彩/圆角/主题守卫随 `check:architecture` 全过。

## 9. 开放问题（待用户决策）

1. 软删除保留期 30 天是否合适？回收站是否需要容量上限？
2. 3D 预览优先 `<model-viewer>`（零依赖、仅 glb/gltf）还是 three 全家桶（多格式、包体 +）？
3. `ask_user` 是否允许在「咨询(Ask)」档之外的模式自动触发，还是一律先询问？
4. 归档会话是否继续参与记忆召回与跨会话搜索（默认：参与搜索、不参与召回）？

## 10. 与既有文档/守卫的关系

- 无重复 PRD（调研确认 `docs/prd/` 无会话架构专篇）。
- 遵守：前端只在 `webui/`；`gui/src` 不得重现；`artifacts/` 目录不进入本文档任何引用；webui E2E 端口 18877 与审批 REST 链路互斥提示继续有效。
- 并发写者警示：§7 P0 涉及的 ChatView / useSessions 均在其活跃清单内，实施前以 `.tmp/task-backlog.md` Z-B 节为准先确认对方停手。
