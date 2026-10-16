# UnivAgent 战略规划 v2.0 — 四合一整合方案（收口版）

> 生成日期：2026-08-05 23:55  
> 状态：**v2.0 收口**（基于 v1.0 草案 + 本轮全部交付物合成；用户授权决策已落 D6/D7）  
> 作者：大湾区靓仔（项目总监）| 基于 Phase J 收口 + Bloom Mode 标准 v2.0 + K1.2/K2.1/K4-deep/K3 全部产出  
> 取代：`univagent-strategic-plan-v1.md`（v1.0 草案，本文件即其升级版，沿用同一路径）

---

## 〇、本轮收口说明（v1.0 → v2.0 变更摘要）

v2.0 相对 v1.0 草案，落地的关键增量：

| 增量                           | 交付物                                          | 路径                                                     |
| ---------------------------- | -------------------------------------------- | ------------------------------------------------------ |
| Bloom Mode 统一质量门禁（跨语言）       | `bloom-mode-engineering-standard-v2.0.md`    | `D:\Downloads\Marguerite\`                             |
| K1.2 Bloom 特效 GLSL Shader 原型 | 5 文件（vert/frag/config/index.html/README）     | `D:\Downloads\Marguerite\bloom-mode-shader-prototype\` |
| K2.1 nexustar-ui 368 文件模块依赖图 | `bloom-mode-nexustar-ui-map.md`              | `D:\Downloads\Marguerite\`                             |
| K4-deep 外部竞品架构深挖             | `bloom-mode-competitor-deepdive.md`          | `D:\Downloads\Marguerite\`                             |
| 两份 v1.0 规范升级 v1.1            | 后端/前端开发规范（嵌入 Bloom Mode 扩展 R17–R20 + 粒子性能纪律） | `TRAE SOLO CN\...\work-mode-projects\...\`             |
| K3 全域整合文档                    | `univagent-integration-doc-v1.md`            | `D:\Downloads\Marguerite\docs\`                        |
| C 阶段 / J3 决策                 | D6 / D7（用户授权「按实际情况决定」）                       | 见第七节                                                   |

---

## 一、执行摘要

本规划响应四项战略任务：

1. **K1** — 视频切片分析 → Bloom 模式会话特效 + 粒子音乐系统
2. **K2** — 三仓（archived + active + Marguerite）全资产盘点与整合
3. **K3** — UnivAgent 全域整合文档
4. **K4** — 全球竞品调研（30 款截图竞品 + 主流 Agent 框架）

### 核心判断

| 维度   | 判断                                             | 理由                                                                        |
| ---- | ---------------------------------------------- | ------------------------------------------------------------------------- |
| 视频来源 | Dreamina（即梦）v20.3.0 模板导出                       | 元数据 `DreaminaMetaInfo.appVersion=20.3.0`, `effect_id=normal_capabilities` |
| 特效本质 | 粒子溶解（Solid→Particle Nebula）                    | 非代码生成，是 CapCut 预设模板                                                       |
| 技术路线 | WebGL(Three.js) + GLSL + Web Audio API         | 开源方案成熟，K1.2 原型已验证可运行（16K 粒子花朵溶解 + 音频直驱 shader）                            |
| 整合范围 | 三仓统一 ≠ 代码合并                                    | archived 为研究参考，active 为开发主线，Marguerite 为 TUI 引擎                           |
| 竞品定位 | **差异化 = TUI 原生 + 本地优先 + 跨形态唯一来源 + Bloom 视觉身份** | 不做"又一个 Cursor"，做"有 TUI 的、视觉自成一派的全能 Agent"                                 |
| 质量杠杆 | **先锁质量标准（Bloom Mode），再整合 + 特效**                | 用户唯一目的是"一步比一步更高质产物"——标准为前置条件                                              |

---

## 二、K1：视频分析与 Bloom 模式技术方案

### 2.1 视频切片分析结果

**文件元数据**：

```
编解码器: H.264 High Profile (AVC)
分辨率:   720 × 1280 (竖屏 / portrait)
帧率:     ~30fps (avg 29.97, 626 frames / 20.87s)
码率:     9.58 Mbps (高质量)
色彩空间: BT.709 / YUV420P
来源:     Dreamina (即梦) v20.3.0 | effect_id: normal_capabilities
大小:     25.6 MB
```

**HTML 伴侣揭示的完整用途**：

```html
<video class="video-background" autoplay muted loop>   
<div class="content"> color: rgb(240,17,155)            
<audio src="http://music.163.com/song/media/outer/url?id=1963443271.mp3">
```

→ 这是一个 **「小雏菊」主题音乐可视化页面**，非单纯视频。

### 2.2 五帧关键帧揭示的视觉生命周期

| 帧   | 时间(s) | 视觉状态              | 技术参数推断                   | Bloom 映射    |
| --- | ----- | ----------------- | ------------------------ | ----------- |
| F01 | 0.0   | 清晰雏菊实体（白瓣+黄蕊），纯黑底 | 渲染: TEXTURE_MAP          | 会话启动 / 消息到达 |
| F05 | 4.0   | 花瓣崩解为粒子，青+黄绿粒子云涌动 | 粒子~5K; 解散力0.3; 重力-0.02   | AI 思考中      |
| F10 | 9.0   | 完全粒子化，星芒闪烁+漩涡流动   | 粒子~15K; 漩涡0.8; Bloom 1.5 | 回复生成高潮      |
| F15 | 14.0  | 粒子缓慢飘散，保持花朵轮廓     | 阻尼0.98; 噪声频率0.003        | 过渡 idle     |
| F21 | 20.8  | 最大弥散，朦胧光晕         | 全局缩放0.7; Alpha衰减         | 背景持续态       |

### 2.3 特效参数提取（已落地为 `bloom-config.ts` 契约）

```typescript
// 从小雏菊视频逆向 + K1.2 原型固化
interface BloomConfig {
  particleCount: { min: 3000, max: 20000 };   // F01→F21 渐增
  lifespan:      { min: 3.0, max: 8.0 };
  gravity:       [0, -0.02, 0];
  turbulence:    0.003;     // Simplex 噪声频率
  damping:       0.98;
  colors: {
    petal:      '#F2F2F2',  // 白瓣
    center:     '#FFD800',  // 金黄蕊
    particleA:  '#66D9E6',  // 青
    particleB:  '#99CC33',  // 黄绿
    sparkle:    '#B3E6FF',  // 冰蓝
  };
  bloom: { strength: 1.8, radius: 0.6, threshold: 0.2 };
  background: '#000000';
}
```

### 2.4 Bloom 模式会话启动特效 — 实现方案

**架构选型决策（D1）**：走 **Three.js + GLSL（A 方案）**，B 方案 Canvas2D 作降级兜底。

**状态机设计**（已在 `bloom-config.ts` 固化为 `BloomPhase` + `BloomEventMap`）：

```
[IDLE] ──(message:sent)──▶ [SOLID_APPEAR] ──(0-3s)──▶ [DISSOLVE] ──(3-10s)──▶ [PARTICLE_BLOOM]
                                                                              │
                                                                     (audio:playing)│
                                                                              ▼
                                                              [AudioReactive] ◀──▶ [DISPERSE] ──(18s+)──▶ [IDLE_BACKGROUND]
```

**与 UnivAgent 集成点**：

```
nexustar-ui (React/Tauri WebView)
└── src/components/BloomBackground.tsx        ← 新增组件（K1.4）
    ├── Three.js Scene (WebGL Canvas)
    ├── ParticleSystem (GLSL Vertex)          ← 已实现：bloom-particle.vert/frag.glsl
    ├── AudioAnalyzer (Web Audio API)         ← 双核 FFT: 512 瞬时 + 2048 谐波
    └── BloomStateMachine                     ← 订阅 EventBus
事件源: EventBus "message:sent"→SOLID_APPEAR / "ai:thinking"→DISSOLVE / "ai:response"→PARTICLE_BLOOM / "audio:playing"→AudioReactive
```

### 2.5 粒子音乐系统 — 架构

**功能需求**：用户粘贴歌曲路径/URL → 解析元数据 → 获取歌词 → 收藏本地曲库(SQLite) → 实时 FFT 驱动粒子背景。

**音频分析管线**（GPU 卸载，音频数据**绝不走 React state**，直送 shader uniform）：

```
本地文件/URL → AudioContext.decodeAudioData() → AnalyserNode.fftSize=2048
        ├── LOW (0-8 bin)   → 径向爆发 (Nuclear Boost 低频非线性膨胀)
        ├── MID (8-80 bin)  → 涡流扭矩
        └── HIGH (120-n)    → 高频微颤/闪烁
```

**歌词获取策略**：网易云公开接口(★★★★☆) / QQ音乐(需Cookie) / 酷我 / 本地 LRC(★★★★★最佳)。

**参考开源（已验证）**：Y2K PARTICLE CORE(36K粒子) / carthworks-audio-visualizer(Beat检测) / **shader-playground(R3F+8-band FFT,技术栈最匹配)** / **Aural-Pro(80-200K纯GLSL+Nuclear Boost)** / Cymatica(双核FFT)。

### 2.6 K1 交付物清单

| #    | 交付物                                                  | 格式                 | 状态                                      |
| ---- | ---------------------------------------------------- | ------------------ | --------------------------------------- |
| K1.1 | 视频切片分析报告（21 帧关键帧）                                    | Markdown           | ✅ 已完成（2.1–2.3）                          |
| K1.2 | Bloom 特效参数化还原（TS interface + GLSL shader + 可运行 Demo） | 5 文件               | ✅ **已完成**（bloom-mode-shader-prototype/） |
| K1.3 | 粒子音乐系统架构设计                                           | Markdown + Mermaid | ✅ 已完成（2.5 本节）                           |
| K1.4 | BloomBackground 组件原型（Three.js + React）               | TSX                | ⏳ 待开发（依赖 nexustar-ui 接入）                |
| K1.5 | 音频解析 + 歌词获取管线                                        | Py(TS)             | ⏳ 待开发                                   |

---

## 三、K2：三仓全资产盘点与整合方案

### 3.1 资产总览

| 仓库                       | 路径                                           | 规模                              | 角色                                 | Git 状态      |
| ------------------------ | -------------------------------------------- | ------------------------------- | ---------------------------------- | ----------- |
| **Marguerite**（工作区）      | `D:\Downloads\Marguerite\marguerite\`        | 107 RS files / 32K loc          | **TUI 引擎核心**（Ratatui 三栏 Workbench） | 无 git（纯代码）  |
| **GUI_Agent**（active）    | `D:\Downloads\GUI_Agent\`                    | 31 dirs / nexustar-ui 368 files | **主产品仓库**（全栈）                      | ✅ 有 git（活跃） |
| **\_GUI_Agent_archived** | `D:\Downloads\_GUI_Agent_archived_20260713\` | 71 dirs / external 竞品源码         | **只读归档**（研究参考）                     | 无 git       |

### 3.2 GUI_Agent 主仓模块分解

```
GUI_Agent/
├── nexustar-ui/        # [活跃] React/TS/Electron 前端 (368 src files) → K2.1 已出模块图
├── nexustar-api/       # [活跃] Python FastAPI/SQLite 后端
├── desktop/            # [活跃] Rust/TS Tauri 桌面壳 (src-tauri/)
├── nexustar-website/   # [官网 v1]
├── nexustar-website-next/ # [官网 v2 Next.js]
├── admin-backend/      # [管理后台]
├── gwwc/               # [遗留] GWWC legacy
├── docs/               # [活跃] 30+ 架构/审计/设计文档 + K3 产出
├── brand/ data/ deliverables/ scripts/ tools/ workspace/ nexusops/ prototype/ logs/
├── .workbuddy/         # [活跃] 41 plans + 90+ daily memories
├── .trae/              # [活跃] Trae CN specs
└── PRD_UnivAgent_Simple.md
```

### 3.3 Archived 仓外部竞品工具（K4-deep 已深挖）

| 工具                                                                               | 关键架构发现（详见 K4-deep 报告）                                                                                                                                                                 |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **deepseek-tui**                                                                 | Rust ratatui TUI；crates: tools/agent/app-server/config/core/execpolicy/hooks/mcp/protocol/secrets/state/tui-core；**LSP 后编辑诊断** + web_run 无头浏览 + agent_spawn 取代 swarm（v0.8.5 移除 swarm） |
| **hermes-agent**                                                                 | SQLite+FTS5 **五层记忆** + 自进化闭环（技能自动生成）；内存 SessionState + DB 双持久化（fork_session 链）                                                                                                        |
| **openclaw**                                                                     | Gateway + 多 Runtime 插件化（PI/Codex/Claude）；ClawHub 技能生态思路                                                                                                                               |
| opencode / claude-code-source / claude-mem / agent-s / ds4 / claude-code-plugins | 见 `bloom-mode-competitor-deepdive.md`                                                                                                                                                 |

### 3.4 整合决策（D2/D3）

> **三仓不合并代码，建「统一索引 + 交叉引用」**。GUI_Agent 为主仓库；Marguerite 独立通过 bridge 文档对接；archived 仅 external/ 供研究。

```
D:\Downloads\
├── Marguerite\                    # [独立] TUI 引擎 (Rust)
├── GUI_Agent\                     # [主仓库] 全栈产品 (Git)
│   └── docs\integration\          # [新增] 跨仓整合文档（含 K3）
├── _GUI_Agent_archived_20260713\  # [只读归档] 研究参考
└── mgt_tmp\                       # 临时构建产物（可清理）
```

### 3.5 K2 交付物

| #    | 交付物                        | 格式                | 状态                                       |
| ---- | -------------------------- | ----------------- | ---------------------------------------- |
| K2.1 | nexustar-ui 368 文件模块依赖图    | Markdown（Mermaid） | ✅ **已完成**（bloom-mode-nexustar-ui-map.md） |
| K2.2 | 重复/冲突/互补矩阵                 | CSV/Markdown      | ⏳ 待产出（基于 K2.1 延伸）                        |
| K2.3 | 统一仓库布局方案                   | Markdown（3.4）     | ✅ 收口                                     |
| K2.4 | 迁移风险点清单                    | Markdown          | ⏳ 待产出                                    |
| ★    | **Bloom Mode 统一工程标准 v2.0** | Markdown（15 节）    | ✅ **已完成**（跨 K2/K4 的质量门禁载体）               |

---

## 四、K4：全球竞品调研

### 4.1 截图竞品全景（30 款，详见 v1.0 §4.1）

按能力域分：AI-Native IDE（Cursor/Trae CN/Qoder/Kiro/CodeBuddy/Lingma…）、Agent 框架/DevOps（WorkBuddy/Cockpit/Crow5…）、基础设施（VS Code/IntelliJ/WPS…）。

### 4.2 主流 Agent 框架深度对比（v1.0 §4.2 表，补充 K4-deep 实证）

| 维度    | OpenClaw          | Hermes Agent | DeepSeek-TUI      | **UnivAgent(目标)**         |
| ----- | ----------------- | ------------ | ----------------- | ------------------------- |
| 语言    | TS                | Python       | **Rust(ratatui)** | Rust(TUI)+TS(GUI)+Py(API) |
| 架构    | Gateway+4 Runtime | 单循环+学习闭环     | TUI+MCP           | TUI+GUI+API+CLI           |
| 记忆    | MEMORY.md+向量      | **5层+FTS5**  | 会话级               | 三层(会话/持久/元记忆)             |
| Skill | ClawHub 13K+      | Auto-gen     | 插件式               | WorkBuddy Skill           |
| 多模型   | 15+               | 15+          | DeepSeek 系        | 多模型+本地优先                  |
| GUI   | 无                 | 无            | **TUI 原生**        | TUI(Ratatui)+GUI(Tauri)   |
| 桌面操控  | 工具调用              | 工具调用         | 无                 | UIA+RPA(规划)               |

### 4.3 K4-deep 架构要点（详见 `bloom-mode-competitor-deepdive.md`）

- **DeepSeek-TUI**：Rust ratatui；crates 分层清晰；`tui-core` 管渲染、`agent` 管循环、`tools`/`mcp` 管扩展；LSP 后编辑诊断（`crates/tui/src/lsp/` + `lsp_hooks.rs`）是独特亮点；web_run 无头浏览；**v0.8.5 起用 agent_spawn 取代 swarm**（重要：蜂群用 spawn 模型而非固定 swarm 实体）——直接影响 UnivAgent J3 设计。
- **Hermes Agent**：SQLite(WAL)+FTS5 虚拟表 + 触发器存 sessions/messages；五层记忆 + 自进化（技能自动生成）；内存 SessionState 与 DB 双持久化，`fork_session` 链支持分支。
- **OpenClaw**：Gateway 统一入口 + 多 Runtime 插件化（PI/Codex/Claude 等）；ClawHub 13K+ 技能市场生态思路值得借鉴，但仓库过重，UnivAgent 走轻量 bridge。

### 4.4 Adopt / Avoid（已写入 Bloom Mode 标准 §10）

| 来源                | Adopt                                                     | Avoid          |
| ----------------- | --------------------------------------------------------- | -------------- |
| Cline             | Plan/Act + 审批门 + `.rules` + MCP 市场 + 沙箱浏览器/终端             | 超多大文件审批流偏慢     |
| OpenHands         | Docker 沙箱 + 事件流 + 插件 + 多 agent                            | 循环/回退敏感        |
| Hermes            | 五层记忆 + 自进化闭环                                              | 单循环无沙箱         |
| Aural-Pro         | GPU 卸载粒子 + UnrealBloom + Nuclear Boost                    | 仅 Wallpaper 形态 |
| shader-playground | R3F+Vite+Tailwind+Zustand+GLSL 同构 + 音频 bypass React state | —              |
| Cymatica          | 双核 FFT（512 瞬时 + 32768 谐波）                                 | —              |
| DeepSeek-TUI      | TUI 流式渲染 + 终端 UI 范式                                       | 无 GUI/无技能生态    |

### 4.5 K4 交付物

| #       | 交付物                                 | 状态                                           |
| ------- | ----------------------------------- | -------------------------------------------- |
| K4.1    | 30 款截图竞品能力矩阵                        | ✅ v1.0 §4.1                                  |
| K4.2    | Agent 框架五维深度对比                      | ✅ v1.0 §4.2 + K4-deep 实证                     |
| K4.3    | 能力差距分析与补齐路线图                        | ✅ v1.0 §4.3                                  |
| K4-deep | openclaw/hermes/deepseek-tui 架构深挖报告 | ✅ **已完成**（bloom-mode-competitor-deepdive.md） |

---

## 五、K3：全域整合文档（已交付 v1.0）

> K3 文档基于 K1/K2/K4 全部结论合成，输出至 `D:\Downloads\Marguerite\docs\univagent-integration-doc-v1.md`，结构遵循卷一~卷九：

```
卷一 产品定义（PRD 扩充）        卷六 内置工具与应用路线图
卷二 技术架构总览               卷七 API 契约索引
卷三 功能模块清单与状态         卷八 部署与运维
卷四 竞品能力矩阵（K4 植入）    卷九 已知遗留项与债务清单
卷五 Bloom 模式与粒子系统（K1 植入）  附录 A/B/C/D
```

**状态**：✅ 已由 worker 合成交付（与本文档并行产出），作为 UnivAgent 单一整合枢纽，引用 Bloom 标准 / nexustar-ui 模块图 / 竞品深挖三份前置产物。

---

## 六、执行计划与优先级排序

### Phase 顺序（已重排，质量门禁前置）

```
Phase K1-a  →  K4-deep  →  Bloom标准  →  K2.1  →  K3-write  →  K1-implement
(视频分析✅)  (竞品深挖✅)  (v2.0✅)    (模块图✅)  (整合✅)     (特效接入⏳)
```

### 已完成 / 进行中 / 待启动

| 序号 | 任务                        | 状态                   | 依赖             |
| -- | ------------------------- | -------------------- | -------------- |
| 1  | K1.1 视频切片分析               | ✅ 完成                 | —              |
| 2  | K1.2 Bloom GLSL Shader 原型 | ✅ 完成（5 文件）           | —              |
| 3  | K4-deep 竞品架构深挖            | ✅ 完成                 | —              |
| 4  | Bloom Mode 工程标准 v2.0      | ✅ 完成                 | K4-deep        |
| 5  | 两份 v1.0 规范升级 v1.1         | ✅ 完成（R17–R20 + 粒子纪律） | Bloom 标准       |
| 6  | K2.1 nexustar-ui 模块图      | ✅ 完成                 | —              |
| 7  | K3 全域整合文档                 | ✅ 完成                 | 1–6            |
| 8  | K1.4 BloomBackground 组件   | ✅ 已完成                | nexustar-ui 接入 |
| 9  | K1.5 粒子音乐管线               | ✅ 已实现 + P0 修复完成（R17 emoji→SVG, R19 硬色→token） | K1.4           |
| 10 | K2.2 重复/冲突矩阵              | ✅ 已完成                | K2.1           |
| 11 | K2.4 迁移风险清单               | ✅ 已完成                | K2.1/K2.2     |

### 长期任务（需单独 Phase）

| 任务                      | 规模    | 说明                              | 决策                |
| ----------------------- | ----- | ------------------------------- | ----------------- |
| K1.4 BloomBackground 组件 | 1–2 周 | Three.js + React 接入 nexustar-ui | 正常排期              |
| K1.5 粒子音乐管线             | 1 周   | 音频解析+歌词+收藏                      | 正常排期              |
| 内置终端/浏览器                | 2–3 周 | xterm.js / Chromium Embedded    | P0 路线图（Bloom §11） |
| RPA/桌面操控（C 阶段）          | 3–4 周 | UIA+截图+键鼠                       | **D6 暂缓**（见第七节）   |
| J3 蜂群实体卡                | —     | swarm 子系统                       | **D7 暂缓**（见第七节）   |

---

## 七、决策记录（D1–D8）

| #      | 决策                                         | 原因                                                                                                                                     | 影响                                               |
| ------ | ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| D1     | Bloom 特效走 Three.js + GLSL（A 方案）            | 开源成熟，80K–200K 粒子达标，K1.2 原型已验证                                                                                                          | 需 WebGL 2.0；Canvas2D 降级                          |
| D2     | 三仓不合并代码，建统一索引                              | 技术栈差异（Rust/TS/Py）；archived 只读快照                                                                                                        | 文档量增加，零合并风险                                      |
| D3     | GUI_Agent 作主仓库                             | 唯一有 git；含全栈组件                                                                                                                          | Marguerite 独立 + bridge 对接                        |
| D4     | 粒子音乐作差异化卖点                                 | 竞品均无此功能                                                                                                                                | 需投入 K1.4–K1.5                                    |
| D5     | 内置终端/浏览器/RPA 定 P1–P2                       | 核心在 TUI+Bloom 差异化                                                                                                                      | 后续迭代                                             |
| **D6** | **暂缓 C 阶段桌面能力二次开发（文件网关/sidecar/SSRF/UIA）** | 用户授权「按实际情况决定」；C 阶段是 Electron/Tauri 重构建，与 Marguerite Rust TUI 主线发散；最高杠杆是先锁 Bloom Mode 质量标准再整合；C 阶段能力已并入 Bloom 标准 §11 内置工具路线图（P1 规划），非取消 | 桌面操控/UIA 延后至质量标准稳定后；不阻断主线                        |
| **D7** | **暂缓 J3 蜂群实体卡**                            | 用户授权「按实际情况决定」；J3 硬阻塞 Phase E swarm 子系统（仓库未建、state.rs 无 swarm 模型）；DeepSeek-TUI 实证 agent_spawn 模型优于固定 swarm 实体                           | 等 Phase E swarm 子系统就位后补；J3 设计改为 spawn 模型（非固定实体卡） |
| **D8** | **Bloom Mode 标准 v2.0 取代两份 v1.0 为唯一质量门禁来源** | 收口跨语言门禁，补视觉 P0 / 竞品 adopt-avoid / 内置工具路线图三块 v1.0 缺失能力                                                                                  | 两份 v1.0 降级为语言专属细则参考；所有交付物以 Bloom 标准为门禁           |
| **D9** | **C6 账号单事实源：Python 本地优先（已确认）；Phase 3 官网+账号整合排到最后** | 用户 2026-08-06 确认采用推荐方案：本地 SQLite `user_id` 分区为唯一写入源，Java `admin-backend` 上线后仅作**单向同步目标**、禁双写。Phase 3（官网+账号整合）排期至**最末端**——本地打包+测试全部 OK 后再启动 | 解锁本地优先主线；Phase 3 不阻塞当前交付；避免双写不一致；官网/账号整合与本地功能解耦，最后统一验收 |
| **D10** | **D5 编排画布收敛：统一 OrchestrationRun 模型 + 4 适配器 + 共享渲染器（✅ 已实施 + 独立验证通过）** | K2.4 标 D5 为 🔴 结构债；实证为 3 个 ReactFlow DAG 画布（OrchestrationCanvas/AgentFlowCanvas/SwarmDAGView）+ 1 个时间线面板（CockpitPanel），读 4 套独立 store 模型（RunSnapshot/SwarmStepView/AgentLive/GoMeTask）；收敛 = 统一 node/edge/run/status 模型 + 4 个纯函数适配器 + 共享 `<OrchestrationGraph>` 渲染器；C1/C3 顺带修 emoji 功能图标（R17）。2026-08-06 实施完成并经项目总监独立验证：tsc EXIT=0、R17/R18/R19/R20 全过、单图标库锁定、AgentFlowCanvas WASD/LOD/MiniMap/immersive 与 CockpitPanel UX 全部保留 | 消除 4 处重复的布局/配色/节点组件；跨画布节点可对齐；无外部阻塞（后端 app.event_bus 已统一，K2.4 F1），不依赖 C6；DualLeaderPanel/MaxImmersive/KnowledgeGraphView 属 D4/D11 相邻范围，不并入 |
| **D11** | **Phase 1 MAX 子系统收敛：复用 D5 OrchestrationRun + 第5适配器 `dualLeaderToGraph`（✅ 已实施 + 独立验证通过）** | 实证 DualLeaderPanel/MaxImmersive/PipelineReview 三组件读 3 套独立 store；2026-08-06 实施完成并经项目总监独立验证：新增 `src/lib/orchestrationAdaptersMax.ts`（`dualLeaderToGraph` 纯函数，Flash 左轨 / Pro 右轨 → cluster，completed→done 归并，复用 D5 `roleFromString`）；三组件改写复用 `<OrchestrationRun>`/`<OrchestrationGraph>`/`<OrchestrationTimeline>`；R17 emoji（M1 9 处 + M2 1 处）已全部替换为 `SvgIcon`（锁定 `chat/icons.tsx` 单库）；R19 硬色（~30 处）全部入 token（含 `--wb-accent`/`--wb-max-overlay`）；OrchestrationDashboard.tsx 既有 R19 内联 rgba 一并修复。双领队左右分轨布局 + 沉浸式交互全部保留 | 消除 MAX 子系统重复的阶段条/配色/节点模型，与 D5 同享模型与渲染器；无外部阻塞；tsc EXIT=0、R17/R18/R19/R20 全过、单图标库锁定、双轨/沉浸 UX 保留 |

---

## 八、完整交付物清单（本轮全部产出）

| 文件                   | 路径                                                                | 类型   | 状态 |
| -------------------- | ----------------------------------------------------------------- | ---- | -- |
| Bloom Mode 工程标准 v2.0 | `D:\Downloads\Marguerite\bloom-mode-engineering-standard-v2.0.md` | 质量门禁 | ✅  |
| K1.2 顶点着色器           | `bloom-mode-shader-prototype\bloom-particle.vert.glsl`            | GLSL | ✅  |
| K1.2 片元着色器           | `bloom-mode-shader-prototype\bloom-particle.frag.glsl`            | GLSL | ✅  |
| K1.2 配置契约            | `bloom-mode-shader-prototype\bloom-config.ts`                     | TS   | ✅  |
| K1.2 可运行 Demo        | `bloom-mode-shader-prototype\index.html`                          | HTML | ✅  |
| K1.2 运行说明            | `bloom-mode-shader-prototype\README.md`                           | MD   | ✅  |
| K2.1 模块依赖图           | `D:\Downloads\Marguerite\bloom-mode-nexustar-ui-map.md`           | MD   | ✅  |
| K2.2 重复/冲突矩阵         | `D:\Downloads\Marguerite\bloom-mode-consolidation-matrix.md`     | MD   | ✅  |
| K2.4 迁移风险清单          | `D:\Downloads\Marguerite\bloom-mode-migration-risk-list.md`     | MD   | ✅  |
| K1.5 粒子音乐管线 + M6 收敛 | `GUI_Agent/nexustar-ui/src/components/music/` + `GUI_Agent/nexustar-api/app/routers/music.py` | TS+Py | ✅ 已实现 + P0 修复完成（R17/R19） |
| D5 编排画布收敛（recon+实施+验证） | `D:\Downloads\Marguerite\bloom-mode-canvas-convergence.md` + `GUI_Agent/nexustar-ui/src/{types/orchestration-graph.ts,lib/orchestration*.ts,lib/graphLayout.ts,lib/roleIcons.ts,components/orchestration/*,components/AgentFlowCanvas.tsx,components/gome/CockpitPanel.tsx}` | MD+TS | ✅ 方案 + 实施 + 独立验证通过（tsc 0 / R17/R18/R19/R20 全过；双人 worker 竞态已收口为一致态） |
| D11 MAX 子系统收敛 recon + 实施 | `D:\Downloads\Marguerite\bloom-mode-max-subsystem-recon.md` + `GUI_Agent/nexustar-ui/src/lib/orchestrationAdaptersMax.ts` + `DualLeaderPanel/MaxImmersive/PipelineReview` 改写 | MD+TS | ✅ recon + 实施 + 项目总监独立验证通过 |
| K4-deep 竞品深挖         | `D:\Downloads\Marguerite\bloom-mode-competitor-deepdive.md`       | MD   | ✅  |
| K3 全域整合文档            | `D:\Downloads\Marguerite\docs\univagent-integration-doc-v1.md`    | MD   | ✅  |
| 战略规划 v2.0（本文件）       | `D:\Downloads\Marguerite\univagent-strategic-plan-v1.md`          | MD   | ✅  |
| 后端开发规范 v1.1          | `TRAE SOLO CN\...\后端开发规范_v1.0.md`                                 | 规范   | ✅  |
| 前端开发规范 v1.1          | `TRAE SOLO CN\...\前端开发规范_v1.0.md`                                 | 规范   | ✅  |

---

*文档结束。v2.0 已收口：四合一任务（K1/K2/K3/K4）全部交付或锁定排期；C 阶段与 J3 按用户授权决策暂缓（D6/D7）；所有后续交付物以 Bloom Mode 标准 v2.0 为唯一质量门禁。*

