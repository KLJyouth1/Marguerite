# PRD：语义色令牌治理（S 档）

- 日期：2026-10-22
- 状态：执行中
- 触发：`gui/src` 归档后，`design/tokens.json` 的 `semantic.*` 失去唯一消费者；
  审计发现 webui 侧有 **10 个主题 × 5 个语义色 = 50 个值完全未登记**

## 1. 问题

`design/tokens.json` 自称「设计令牌单一来源」，`scripts/check-tokens-sync.mjs`
是它的守卫。但实测（2026-10-22）：

| 令牌族 | 登记于 tokens.json | 有守卫 | 实际情况 |
|---|---|---|---|
| `brand.*` | ✅ | ✅ | WebUI dark 有 4 项检查 |
| `webuiAdapted.dark.accent*` | ✅ | ✅ | 4 项检查 |
| `typography.fontFamily` | ✅ | ✅ | WebUI 3 项 |
| **`semantic.{success,warning,error,info}`** | ✅ | ❌ **无消费者** | 原 GUI `--status-*` 是唯一消费者，已归档 |
| **webui `--ok/--warn/--danger/--info/--queued`** | ❌ **未登记** | ❌ **无守卫** | **10 个主题 × 5 = 50 个值，谁都能改** |

**后果**：任何人随手改 `webui/src/themes/<theme>/tokens.css` 里的 `--danger`，
没有任何门禁会报错。而这类改动正是「深色主题下错误色对比度不足」那类难查缺陷的源头。

同时，`semantic.*` 现在是一条**没有任何运行时消费者的登记项** ——
要么给它建立到 webui 的映射，要么承认它是历史残留。悬在中间是最差的状态。

## 2. 目标

- **G1**：把 webui 全部主题的语义色**登记进 `design/tokens.json`**，成为可审计的单一真源。
- **G2**：加守卫 —— 主题文件与登记值不一致即 exit 1，且给出「哪个主题哪个令牌漂了」。

## 3. 非目标（明确不做）

- **NG1**：**不统一数值**。10 个主题的语义色是**为各自明度/色调调过的**
  （arctic 低饱和、crt-green 荧光、ink 水墨去饱和），强行改成同一组值会直接毁掉
  「同一语义在 10 种视觉语言下的正确表达」。要统一的是**登记与判据**，不是颜色值。
- **NG2**：不改任何 `.css` 的实际色值。本批只做「登记 + 守卫」，零视觉变化。
- **NG3**：不做 WCAG 对比度审计。那是独立项目（需要逐主题、逐背景两两配对计算），
  本批只建立「可被审计」的基础。
- **NG4**：不合并 `semantic.{success,warning,error,info}` 与 `--ok/--warn/--danger/--info`
  两套命名。它们语义不同：前者是**品牌规范值**（用于文档/外部交付），
  后者是**各主题运行时值**。用 `canonical` 字段建立映射，而不是改名。

## 4. 方案

### 4.1 数据结构（沿用既有 `webuiAdapted` 模式）

`design/tokens.json` 已有 `webuiAdapted.dark.accent` 这套「登记主题适应值」的先例，
注释明确写着「这些派生值注册于本单一来源，供 check-tokens-sync.mjs 校验」。
本批按同一模式扩展：

```jsonc
"webuiAdapted": {
  "dark": { "accent": {...} },        // 既有，不动
  "semanticByTheme": {
    "comment": "各主题语义色。canonical 指向 design/tokens.json 的品牌规范值；" +
               "value 是该主题按地面明度调过的运行时值。改值必须同时改这里，否则守卫报错。",
    "dark": {
      "ok":     { "value": "#39D7A2", "canonical": "semantic.success" },
      "warn":   { "value": "#E8B23A", "canonical": "semantic.warning" },
      "danger": { "value": "#FF6B6B", "canonical": "semantic.error" },
      "info":   { "value": "#56C2E6", "canonical": "semantic.info" },
      "queued": { "value": "#8C7DF2", "canonical": null }   // 无品牌对应（队列态，UI 专有）
    },
    "light": { ... }, "ink": { ... }, ...   // 共 10 个主题
  }
}
```

### 4.2 守卫（`check-tokens-sync.mjs` 扩展）

对每个已登记主题、每个令牌：
1. 从 `webui/src/themes/<theme>/tokens.css` 读 `--<token>`
2. 与 `semanticByTheme[theme][token].value` 逐字比对
3. 不一致 → `[FAIL] WebUI(<theme>) --<token> 漂移：登记 X，实际 Y`
4. 反向：CSS 里存在但未登记的令牌 → `[FAIL] 未登记 ...`（防"偷偷加一个令牌绕开登记"）
5. 主题目录存在但 `semanticByTheme` 无对应条目 → `[FAIL] 主题 <x> 未登记`

### 4.3 关闭 ARCHIVED_GAP

`semantic.*` 不再是无消费者：它的 `canonical` 映射现在被守卫引用，
且守卫会断言「每个 canonical 指向的字段真实存在」。

## 5. 验收口径

| 编号 | 判据 |
|---|---|
| AC1 | `design/tokens.json` 含 10 个主题的语义色登记，字段齐全（value + canonical） |
| AC2 | `node scripts/check-tokens-sync.mjs` exit 0，且输出里出现「10 主题 × 5 令牌」的通过计数 |
| AC3 | **变异测试**：改掉 `dark/tokens.css` 的 `--danger` → 守卫 exit 1 并点名该主题该令牌 |
| AC4 | **变异测试**：CSS 里加一个未登记的 `--danger-alt` → 守卫 exit 1 |
| AC5 | webui `npm run typecheck` exit 0（colors 守卫不受影响） |
| AC6 | webui `npx vitest run` 全绿（主题注册表测试不回归） |
| AC7 | `ARCHIVED_GAPS` 中 `semantic.*` 一项被移除（已由 canonical 映射接管） |

## 6. 影响面

- 消费方：`scripts/check-tokens-sync.mjs`（扩展）、`design/tokens.json`（新增字段）
- **不受影响**：所有 `.css`（零改动）、webui 组件、Rust 侧、安装包内容

## 7. 回滚

还原 `design/tokens.json` 与 `scripts/check-tokens-sync.mjs` 两个文件即可。
无数据迁移、无运行时依赖。
