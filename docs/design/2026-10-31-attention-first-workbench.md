# 注意力优先工作台（Attention-First Workbench）

> 状态：**已实现（前端）** · 2026-10-31
> 代码：`webui/src/composables/ui/attentionQueue.ts` · `attentionSources.ts` · `motionGrammar.ts`
> 视图：`webui/src/components/ui/AttentionShelf.vue` · `AttentionLayer.vue`
> 样式：`webui/src/styles/attention-system.css`
> 挂载：`shell.overlay` 插槽（`registerAttentionLayer.ts`，由 `main.ts` 安装）

---

## 1. 为什么做（5 Why）

1. **为什么用户盯着转圈？** 因为不知道智能体是在算，还是已经卡死。
2. **为什么不知道？** 因为进度只以「文本 token」的形式出现，首个 token 到达前是一片空白。
3. **为什么是文本？** 因为界面的第一公民是「消息」。
4. **为什么「消息」是错的抽象？** 消息是**同步对话**的载体；agent 工作是**异步委派**，需要的是「任务 / 阶段 / 产物 / 证据」。
5. **根因**：工作台缺少一等公民的「工作对象」。屏幕空间按**功能分类**固定切分（侧栏、顶栏、输入框），而不是按**紧迫度**分配。于是「正在阻塞你的一条审批」会随着聊天记录滚出视野——恰恰在它开始消耗你时间的那一刻消失。

## 2. 第一性原理（三条，可反驳）

| # | 原理 | 反例即证伪 |
|---|---|---|
| P1 | **空间按紧迫度分配，不按功能分类。** | 若一个搁置 2 小时的审批能被刚出现的次要选项挤到后面，P1 就是假的。 |
| P2 | **动效只能编码因果，不能编码装饰。** | 若某个动画在没有状态跃迁时也会播，它就是在训练用户忽略动效。 |
| P3 | **用户对运行中工作的意图是动词，不是导航。** | 若界面只允许「跳转」而不允许「停止/接管」，它就把异步委派当成了静态文档。 |

> P3 本轮只落地了「路由」这一半：货架把你送到决策现场，**不**在角落替你裁决（见 §5 决策 D3）。

## 3. 系统三层

### L1 布局 —— 注意力货架（Attention Shelf）

- 位置：**左下角**（`position: fixed; left; bottom`）。选择依据不是审美，而是**唯一性**：输入框在中央、桌宠与悬浮球在右下、顶栏在顶部，左下是唯一未被占用且处于余光的位置。
- 折叠态：一枚胶囊，显示**债务总数** + 各类别圆点。空闲时组件**完全不渲染**（`v-if="total > 0"`），零像素、零定时器。
- 展开态：最多 5 行（超出需滚动即说明用户在「处理队列」而非「余光扫视」）。
- 移动端让开 `--mobile-tabbar-h`；桌宠/悬浮球窗口（`data-surface`）整层隐藏。

### L2 动效 —— 因果语态（Motion Grammar）

四个动词，全部对应**状态跃迁**，没有第五个：

| cue | 跃迁 | 观感结论 |
|---|---|---|
| `arrive` | 无 → 有新工作落地 | 「有东西来了」 |
| `settle` | 待定 → 已提交 | 「这已成定局」 |
| `yield` | 空闲 → 在等别人 | 「它在干活，不是卡住」 |
| `refuse` | 在途 → 被拒/阻塞 | 「这个得我来」 |

- **故意没有 `success`**：成功就是 `settle`。给成功单独做庆祝动画，是教用户忽略动效的第一步。
- 时长/缓动**只用 token**（`--dur-fast/base/enter/pulse`、`--ease-*`），由 `check-webui-motion.mjs` 强制。
- `prefers-reduced-motion` 不是「关闭反馈」，而是「取消位移」：每个 cue 都有 `-static` 变体，只改透明度。
- **无跃迁不播**：`cueFromCounts(prev, next)` 在数量不变时返回 `null`，调用方保持静止。

### L3 交互 —— 路由而非裁决

- `Alt + .` 打开/收起（对齐「让我看看有什么要处理」的系统惯例）。
- 点击条目：写 `requestApprovalFocus` → 跳 `/chat?session=<key>`（与 `App.vue:switchToSession` 同一 URL 形状）→ 收起。
- 无可路由目标 → 退到 `/sessions`（其注意力条已含待办计数，`/approvals` 已重定向到此处）。

## 4. 排序不变量（本设计的承重墙）

```
score = KIND_BASE[kind] + recencyBonus(at, now)
KIND_BASE = { approval: 1000, blocked: 700, failed: 400, choice: 150 }
RECENCY_MAX = 99   <  最小基座间隔 250
```

因为 `RECENCY_MAX` 严格小于任意两个基座之差，**时间只能在类别内部影响顺序，永远不能翻转类别**。于是：

> 刚出现的 `choice` 永远不会越过搁置两小时的 `approval`。

这条不变量由 `attentionQueue.test.ts` 直接断言（它断言的是**基座间隔 > RECENCY_MAX**，而不是某个具体分数）。一旦有人调小基座，测试先红，而不是等用户在真机上发现货架变成了通知流。

## 5. 决策台账

| ID | 决策 | 备选 | 理由 |
|---|---|---|---|
| D1 | 货架放**左下** | 顶部横幅 / 右下 / 中央浮层 | 顶部与顶栏抢层叠、右下被桌宠/悬浮球/Toast 占、中央抢焦点。左下是唯一空位，且符合 calm technology 的「余光」定位。 |
| D2 | 只显示**审批**一种来源 | 同时接失败任务/待选项 | 失败任务与待选项在现有 store 里没有稳定的「未处理」语义（有的是瞬时事件，不是持久债务）。先做能证明正确的那一种，接口（`attentionSources.ts`）留给下一种。 |
| D3 | **不做**就地批准/拒绝 | 货架上一键通过 | 340px 角落里没有命令全文、没有权限矩阵、没有 diff。在这里批准工具执行是**安全倒退**；已从组件与 emit 签名中移除，避免有人顺手加回来。 |
| D4 | 复用 `shell.overlay` 插槽 | 直接写进 `App.vue` | 插槽是仓库既有的 append-only 扩展缝；`App.vue` 已 96 KB，只加一行 `<SlotHost>`。 |
| D5 | 年龄用「首次观测」时间 | 用网关时间戳 | 网关不报「你何时开始被阻塞」。首次观测会低估年龄，方向安全：重载后不会凭空多出两小时等待。 |
| D6 | 不新增任何 RPC 方法 | 新增 `attention.list` | 新增方法要在 Rust 与 Python 两侧各实现一次，正是本仓最高频的「双实现漂移」缺陷形态。本轮零新增方法。 |

## 6. 经济性三视角

- **成本（C）**：零新增网络请求、零新增模型调用、零新增 RPC 方法。新增 4 个模块 + 1 个样式文件；单测 33 例（10 + 8 + 12 + 8 组件中的前 3 组 + 组件 8 例）。
- **缓存（E）**：不触碰系统提示与工具目录，**不影响 provider 前缀缓存**。这一点是刻意的：任何进入稳定半区的字节都会让缓存前缀失效，而货架是纯渲染层。
- **体感（T）**：把「11–14s 首 token 的空白」从**无信息**变为**有债务可见**。注意它不缩短首 token（那是 provider 首包问题，已由 `merged_turn_hint` 归因证实），它只消除「不知道在等什么」。

## 7. 可编码契约

```ts
// 排序（纯函数，无 Vue 依赖）
rankAttention(items: readonly AttentionItem[], now: number, limit = 0): RankedAttention[]
attentionCounts(items): Record<AttentionKind, number>
attentionDebt(items): number            // 去重后的债务数
dedupeAttention(items): AttentionItem[] // 同 id 保留最早 at

// 来源适配（唯一知道 store 形状的地方）
approvalsToAttention(approvals, firstSeen, now): AttentionItem[]

// 动效（纯函数）
cueFromCounts(prev, next): MotionCue | null
motionAttrs(cue, reducedMotion): Record<string, string>
animationShorthand(cue, reducedMotion): string
```

**扩展点**：新增一种注意力来源 = 在 `attentionSources.ts` 加一个 `xxxToAttention()`，**不碰**排序模块。

## 8. 诚实边界（未做）

1. **未真机验证**：以上验证止于单测、`vue-tsc`、13 道架构/设计守卫。**没有**在打包后的 Tauri 窗口里看过实际渲染（位置是否与桌宠重叠、`z-index: 55` 是否与某层遮挡）。
2. **未做焦点管理**：展开面板不做 focus trap（刻意非模态），但也没有「打开后自动聚焦第一行」——键盘用户需要 Tab 进去。
3. **未接第二种来源**：失败任务、待选择项仍是缺口（D2）。
4. **`Alt + .` 未进设置页快捷键表**：目前只在组件内监听，未纳入可发现性（settings → shortcuts）。
