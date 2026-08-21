# Marguerite 四大插件集成与系统重构 — 整合路线图（Track B–G）

> 依据：本地五目录只读审计（audit-local）+ 上游仓库/全网审计（audit-web），2026-08-18。
> 原则：四插件均为 TS+Cordis，无法直接接入 Rust 引擎；最高杠杆 = 翻译契约/策略成 Rust 原生能力（Track A 已证此路可行）。
> 纪律：每 Track 独立过 cargo/vue 门禁 + commit，不接受「完成」为空口证据。第二 QA 层独立复跑 `cargo test`（须 `export PATH="/c/Users/Administrator/.cargo/bin:$PATH"`，沙箱 Bash 默认无 cargo）。
> P0 视觉纪律全程生效：禁 emoji 图标、禁紫粉渐变、禁硬编码色、禁 AI 模板味（尤其 C/D/F）。

---

## 0. 前提纠正（skeptical，已核实）

- `dataelement/dsh-desktop` 仅是 Electron **宿主壳**，无四插件代码。真实上游为独立社区 Cordis 仓库：
  | 插件 | 真实上游仓库 |
  |------|------|
  | modlens（视觉取证） | `liustack/modlens` |
  | dsh-genui（交互 UI） | `omdsh-dev/dsh-genui` |
  | dsh-web-ui（工作台） | `zhu1090093659/dsh-web-ui` + `@linxin666/dsh-web-ui-all` |
  | 鲸鱼娘桌宠 | `Small-tailqwq/dsh-deep-whale` |
  | dsh-routing-suite（路由） | `yjh051108/dsh-routing-suite` + `dsh-router-standard` |
- 本地五目录**无任何插件源码**，仅 `third_party/harness/.../plugins.json`（159 条 awesome 清单）。
  - `dsh-web-ui` 在 159 条中**零命中**；本地近似是 `dsh-web-ui-notify`（桌面通知，非工作台）。
  - `dsh-routing-suite` 在 159 条中**零命中** → Track A 仅以 opensquilla 为参照（合理）。
- **Track D 锚点修正**：本地真实工作台对应物是 `DSH-better-sidebar`（全功能侧栏工作台，支持第三方注册 tab），非 dsh-web-ui。
- **推背图**：零来源 → 视为推测项，排最后。
- 缓存命中率：DSH 核心已原生（状态栏 ~99%），我们只需**暴露**而非发明。

---

## 1. Track B — 视觉取证（modlens / dsh-vision-toolkit）

**上游契约**：modlens「贴图 → 结构化 JSON 证据（OCR/版面/语义）」；dsh-vision-toolkit 是其超集（意图感知图像问答、长截图 OCR、UI 复现、grounding、pixel diff）。
**本地可复用**：opensquilla `vision_followup_gate.py`(376 行，完整可照抄 + 有测试)；GUI_Agent `vision_adapter.py` 的 `SemanticLocator/LocateResult`（但 UIA rect 回填恒 None，须修）。
**Rust 现状**：`core/src/vision/` 仅 913 行，`binary_html.rs` 是 base64 兜底，非结构化证据。

**改进项（切片推进）**：
- **B1** `core/src/vision/evidence.rs`：`VisionEvidence{ ocr:[{text,bbox,conf}], layout:[{role,bbox,children}], semantics, provenance{model,ts,hash} }`（hash 复用 `crate::cache::hash_text`，绝不存原始图片字节/API key）。
- **B2** 对译 `vision_followup_gate.py` → `core/src/router/vision_gate.rs`（中英 opt-out 正则、explicit_previous_image_reference、unknown=image_if_recent+fallback_recent_turns、3.0s 超时常量、六字段进 decision_log）。**ROI 最高，先跑**。
- **B3** grounding：对译 `SemanticLocator`，但**必须补 UIA rect 回填**修 (b)1 恒 None；双通道 confidence uia 1.0 / vlm 0.7。
- **B4** 长截图 OCR 拼接 + pixel diff（纯新建）。

**验收**：纯逻辑单测覆盖 gate 六态 + evidence 序列化；`cargo test` 绿。

---

## 2. Track C — 生成式 UI 交互（dsh-genui）

**上游契约**：`omdsh-dev/dsh-genui`（layout/charts/forms/quizzes/mermaid/3D + action event loop 回模型）。
**缺口**：仅 web profile 渲染；30+ 组件无版本化 schema；模型手写 HTML/JS = XSS 面。
**Rust 现状**：零实现。

**改进项**：
- **C1** `core/src/genui/`：`Widget` enum{Layout,Chart,Form,Quiz,Mermaid,Scene3D} + JSON schema + **回传 action event loop**（契约要求 event 回模型）。
- **C2** 版本化契约 + 沙箱运行时（防 XSS）。
- **C3** 注册表套用 GUI_Agent `screens.ts` lazy 注册 + ComingSoon 兜底（枚举外 widget 落 Unsupported 非 panic）。

**验收**：Widget 解析/序列化单测 + action 回传回路单测；`cargo test` 绿。

---

## 3. Track D — 工作台（锚 DSH-better-sidebar）

**上游契约**：`DSH-better-sidebar`（全功能侧栏工作台：文件渲染/编辑、终端、Git、子代理；第三方注册 tab）。皮肤实际 8 款（非 10）；鲸鱼娘 = `dsh-deep-whale` 独立插件；主题重启丢失(#60)；原生右侧栏缺失。
**本地可复用**：GUI_Agent `screens.ts`(SCREEN_REGISTRY 37 条 lazy+ComingSoon)、`agents.ts`(AGENTS 88 → `agent_catalog.json` 619KB 单一真相源)、`theme/tokens.css`+`statusColors.ts`(token 化)、24 孤儿屏 + 33 死代码组件（仅建议转 4：KnowledgeGraphCanvas/MemoryLayersView/KnowledgeVersionPanel/MaxWorkbenchScreen）。
**改进项**：
- **D1** `core/src/tui/workbench` 引入 `PanelRegistry`（对译 37 条注册 + ComingSoon 兜底 + 第三方注册新 tab）。
- **D2** `agent_catalog.json`(88) 收敛为 Rust `AgentMeta` 单一真相源。
- **D3** `tokens.css`+`statusColors.ts` → Rust theme token 表（P0 纪律：禁硬编码色/禁紫粉渐变/禁 emoji 图标）。
- **D4** usage dashboard 接 `core/src/usage`（余额/24h/小时时间线）。
- **D5** 打捞 4 项孤儿/死代码屏；其余明确弃用记档。
- **D6** 鲸鱼娘桌宠：原生集成 `dsh-deep-whale` 契约（独立 pet 模块，非皮肤）。

**验收**：PanelRegistry 注册/兜底单测；theme token 解析单测；`cargo test` 绿。

---

## 4. Track E — 第三阶段研究（RAG/知识图谱/NLP/缓存命中率）

**本地可复用**：opensquilla `context_budget.py`(四态决策)、`prompt_cache_keepalive.py`(443 行，会话级 prefix 保温租约 TTL 300–86400/probe 0.8/idle 3600)、`self_learning/`(3017 行 + 真实 `lgbm_*.bin`/`bge_onnx`)。`kb/graph.rs`(387) 规则抽取；`kb/hybrid.rs`(284) 混检。
**缺口**：`cache/semantic.rs:349 MultiTierStats` 不导出命中率；`kb/graph.rs` 纯规则；NLP 无专用管线；推背图零基础。
**上游**：graph-memory(关系图谱/PageRank/社区发现/向量检索) + dsh-memory-evolve(跨会话记忆)；缓存命中率 DSH 已原生，我们只需暴露。
**改进项**：
- **E1** 对译 prompt cache 租约 + 给 `MultiTierStats` 补 `cache_hit_rate` 导出。
- **E2** `context_budget` 四态 → `core/src/context`。
- **E3** self-learning 闸门/晋升对译（`lgbm`/`bge` 须先决策是否接 ONNX runtime，否则落规则版）。
- **E4** KG embedding 召回（扩 `kb/graph.rs`+`kb/hybrid.rs`）。
- **E5** Rust 原生 NLP 原语（分词/实体/语义）。
- **E6** 推背图：纯推测，最后排；若指「预测/推演」NLP，化为 speculative item。

**验收**：cache 命中率导出单测；context budget 四态单测；`cargo test` 绿。

---

## 5. Track F — 艺术化水墨视觉

未在审计深覆盖；由 D3 token 表 + 内核「只要做到极致 什么都能成功」驱动。重做 LLM 输出格式/排版/布局，中国水墨人文气质，创新视觉语言。P0 纪律全程生效。
**验收**：vue-tsc + 构建绿 + 设计 token 合规。

---

## 6. Track G — 企业级实时多人协同

未在审计覆盖；架构地基（事件总线/CRDT/权限）先立，再谈「超飞书」。
**验收**：协同协议单测 + 本地多端模拟。

---

## 7. 执行顺序

`A(✅)` → `B(切片1 vision_gate 先跑)` → `C` → `D` → `E` → `F` → `G`。每 Track 内部再切片，单测门禁，commit 可追溯。
