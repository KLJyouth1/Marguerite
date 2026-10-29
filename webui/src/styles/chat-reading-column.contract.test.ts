import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

// 用户点名的缺陷：长任务结束后，历史转写「整段居中」——宽窗口下正文被 clamp 成
// 一条 1280px 的列，再被两侧 auto 外边距推到屏幕几何中心，左右各剩几百 px 空白，
// 读起来像排版错乱。
//
// 精确机制（复核后的结论，与「行号 455」那一版假设不同处已注明）：
//   1. `styles/chat-view.css` 是以 `<style scoped src="../styles/chat-view.css">`
//      挂进 ChatView 的，所以它的 `.msg-ai` 编译成 `.msg-ai[data-v-chatview]`，
//      **只命中 ChatView 自己模板里的实时气泡 / thinking 行**；历史行的 `.msg-ai`
//      带的是 AssistantMessage 自己的 scopeId，跨不过组件边界
//      （CompactionEvent.vue 的文件头注释早就写明了这一点）。
//      → 「一处根规则」并不存在，两个渲染路径各有权威，任何一边居中都会让版式
//        在「实时」与「结束后」之间来回变。
//   2. ChatMessageList.vue 曾叠加 `.chat .chat-thread > .msg-ai` /
//      `.chat .chat-turn-row > .msg-ai` 覆盖：子组合器只在某一种 DOM 形状下命中，
//      水合包裹层一变就整条失配，回落到居中 —— 即「有时像修过、有时又居中」。
//   3. 左侧 minimap 轨道绝对定位在 shell 的 0.875rem 处、刻度线聚焦时全长
//      1.875rem + 4px 焦点环 ≈ 48px，`e2e/conversation-minimap.spec.ts` 要求正文
//      左边界越过它再留 8px → 阅读起点下限 56px（所以下面钉的是「≥56px」，
//      而不是任人喜欢的 24px；24px 只在 minimap 隐藏的 ≤768px 分支成立）。
//
// jsdom/happy-dom 都不会跑 flex 布局与 scoped CSS 层叠，所以这里守的是**导致缺陷的
// 那组 CSS 不变量**（与 sidebar-layout.contract.test.ts 同一手法）。
//
// 第二批（卡片 / 横幅）复核出的机制，补在这里以免下轮重蹈：
//   这些卡片各有**两种 DOM 形状** —— ① `.chat-thread`（或 `.chat`）弹性列的直接子块
//   （回滚分支 foldLiveTurn=0 / 配置卡），② 折进历史行 `.msg-ai` 里。
//   只有 ① 才是「整块被推到屏幕中心」；② 的外层已经排在阅读起点了，再套一次
//   --chat-gutter 就是假缩进。② 的复位钩子必须继续存在：AssistantMessage 的
//   `.msg-ai-main > :deep(.approval-card|.clarify-card)`、`.plan-message-card`、
//   各组件自己的 `--timeline` 变体，以及下面守的 ClarifyCard `.msg-ai` 分支。

const read = (rel: string) =>
  readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8')
    .replace(/\r\n/g, '\n')

/** 剥掉注释：这些规则的说明文字会逐字引用被守卫的声明（`margin: 0 auto` 等），
 *  不剥掉就会因为一句注释而误红（本仓库踩过）。 */
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

const chatViewCss = stripComments(read('./chat-view.css'))

/** 组件 `<style>` 块（本仓库的会话行组件都只有一个样式块）。
 *  参数是相对 `src/components/` 的路径 —— 哨兵不在 `chat/` 下，别为它开第二条读取分支。 */
function componentCss(rel: string): string {
  const source = read(`../components/${rel}`)
  const block = source.match(/<style[^>]*>([\s\S]*?)<\/style>/)
  return stripComments(block ? block[1] : '')
}

/** 选择器**整行开头**的那条规则体（非贪婪到行首 `}`，规则体内无嵌套块）。 */
function ruleBody(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = css.match(new RegExp(`^${escaped}\\s*\\{([\\s\\S]*?)\\n\\}`, 'm'))
  return match ? match[1] : ''
}

const decl = (body: string, property: string): string =>
  body.match(new RegExp(`(?:^|\\n)\\s*${property}\\s*:\\s*([^;]+);`))?.[1]?.trim() ?? ''

/** 一条「阅读列」定义必须满足的不变量。 */
function expectReadingColumn(body: string, label: string) {
  expect(body, `${label}: 规则必须存在`).not.toBe('')

  // 缺陷本体：列宽被当成**固定宽度** + 两侧 auto 居中。
  expect(body, `${label}: 禁止 width: var(--chat-col)（上限不是固定宽）`)
    .not.toMatch(/(?:^|\n)\s*width\s*:\s*var\(--chat-col/)
  expect(body, `${label}: 禁止 margin 简写里的 auto（居中）`)
    .not.toMatch(/(?:^|\n)\s*margin\s*:\s*[^;]*\bauto\b/)

  // 修法：宽度只作上限，起排固定在阅读起点，多出来的宽度全部留在右侧。
  expect(body, `${label}: max-width 必须走 --chat-col-start 口径`)
    .toMatch(/max-width\s*:\s*var\(--chat-col-start/)
  expect(body, `${label}: 宽度必须铺满到上限，而不是写死列宽`)
    .toMatch(/(?:^|\n)\s*width\s*:\s*100%/)
  expect(body, `${label}: margin-inline 必须是「起点 + auto」`)
    .toMatch(/margin-inline\s*:\s*var\(--chat-gutter[^)]*\)\s+auto/)
  // 6 种语言 / RTL：起排方向只能是逻辑起点。
  expect(body, `${label}: text-align 必须是 start（不得用 left/center）`)
    .toMatch(/text-align\s*:\s*start/)
  expect(body, `${label}: 禁止 text-align: center`).not.toMatch(/text-align\s*:\s*center/)
}

/** 卡片 / 横幅类块级组件的同一组不变量。
 *  与 expectReadingColumn 的差别只有两处，且都是刻意的：
 *    - max-width 允许在 --chat-col-start 外面再套一层 min()（裁决卡就是比正文窄一档）；
 *    - 不强制声明 `text-align: start`（卡片不靠 text-align 建立层级，加了是空声明），
 *      但仍然禁止 center —— 那才是「整段居中」的另一半。 */
function expectReadingColumnBlock(body: string, label: string) {
  expect(body, `${label}: 规则必须存在`).not.toBe('')

  expect(body, `${label}: 禁止 width: var(--chat-col)（上限不是固定宽）`)
    .not.toMatch(/(?:^|\n)\s*width\s*:\s*(?:min\(\s*100%,\s*)?var\(--chat-col/)
  expect(body, `${label}: 禁止 margin 简写里的 auto（居中）`)
    .not.toMatch(/(?:^|\n)\s*margin\s*:\s*[^;]*\bauto\b/)

  expect(body, `${label}: 宽度必须铺满到上限（或按容器百分比收，不得写死列宽）`)
    .toMatch(/(?:^|\n)\s*width\s*:\s*(?:100%|min\(100%,\s*var\(--)/)
  expect(body, `${label}: max-width 必须引用 --chat-col-start`).toContain('var(--chat-col-start')
  expect(body, `${label}: margin-inline 必须是「起点 + auto」`)
    .toMatch(/margin-inline\s*:\s*var\(--chat-gutter[^)]*\)\s+auto/)
  expect(body, `${label}: 起点不得再硬编码 24px（口径只在 chat-view.css 定义一次）`)
    .not.toMatch(/margin-inline\s*:\s*\d+px\s+auto/)
  expect(body, `${label}: 禁止 text-align: center`).not.toMatch(/text-align\s*:\s*center/)
}

describe('会话阅读列（左对齐）契约 — 历史转写不得再被推到屏幕中心', () => {
  const definitions: Array<[string, string, string]> = [
    ['chat-view.css', '.msg-ai（实时流式 / thinking 气泡）', '.msg-ai'],
    ['chat-view.css', '.chat-compaction-event（回合边界维护行）', '.chat-compaction-event'],
    ['chat/AssistantMessage.vue', '历史行 .msg-ai', '.msg-ai'],
    ['chat/UserMessage.vue', '历史行 .msg-user', '.msg-user'],
    ['chat/CompactionEvent.vue', '历史行 .chat-compaction-event', '.chat-compaction-event'],
    ['chat/TurnSuggestions.vue', '回合结束建议行', '.turn-suggestions'],
  ]

  it.each(definitions)('%s 的 %s 用「上限宽度 + 左起点」而不是居中列', (file, _label, selector) => {
    expectReadingColumn(file === 'chat-view.css' ? ruleBody(chatViewCss, selector) : ruleBody(componentCss(file), selector), `${file} ${selector}`)
  })

  // ── 消息流里的卡片 / 横幅（第二轮收口）────────────────────────────────
  // 这些都是 `.chat-thread`（或 `.chat`）弹性列的**直接子块**，过去逐条写着
  // `width: var(--chat-col) + margin: … auto`，于是每一块各自被推到屏幕几何
  // 中心；它们左边的转写行已经改成左起排，读起来就是「版式散架」。
  const streamBlocks: Array<[string, string, string]> = [
    ['chat/PlanCard.vue', '计划卡（.chat-thread 直接子块 / 历史行内由 .plan-message-card 复位）', '.plan-card'],
    ['chat/ApprovalCard.vue', '待裁决审批卡（回滚分支的直接子块；时间线内由 --timeline 复位）', '.approval-card'],
    ['chat/ApprovalCard.vue', '裁决后的折叠行（同为直接子块）', '.approval-outcome'],
    ['chat/ClarifyCard.vue', '待回答澄清卡（回滚分支的直接子块；历史行内由 .msg-ai 分支复位）', '.clarify-card'],
    ['chat/ClarifyCard.vue', '「已收到回答」横幅（同为直接子块）', '.clarify-outcome'],
    ['chat/ChatStallNotice.vue', '长任务软提示横幅（正文左起排，横幅不是居中语义）', '.stall-notice'],
    ['HistoryLoadSentinel.vue', '历史加载哨兵（块走阅读列，内部药丸仍按 justify-content 居中）', '.history-load-sentinel'],
    ['chat/MetaSkillSetupCard.vue', 'MetaSkill 配置卡（已有会话里 .chat 的直接子块）', '.meta-setup-card'],
    ['chat/OrchestrationRunCard.vue', '编排运行卡（.chat-orch-runs 的子卡；宽度仍按 --composer-col 收一档，但左边缘对齐正文）', '.orch-run'],
  ]

  it.each(streamBlocks)('%s 的 %s 已改成左起排', (file, _label, selector) => {
    expectReadingColumnBlock(ruleBody(componentCss(file), selector), `${file} ${selector}`)
  })

  it('嵌套进历史行的复位分支不吃第二次留边（同一条口径不能在 .msg-ai 里再套一遍）', () => {
    // 卡片有两种 DOM 形状：`.chat-thread` 的直接子块（回滚分支）与折进历史行。
    // 后者外层已经排在阅读起点了，必须复位成整行宽，否则会叠出 64px 假缩进。
    const assistant = componentCss('chat/AssistantMessage.vue')
    expect(assistant, '历史行仍负责把子卡复位成整行宽').toMatch(
      /\.msg-ai-main\s*>\s*:deep\(\.approval-card\)[\s\S]*?width:\s*100%;[\s\S]*?margin-inline:\s*0/,
    )
    expect(ruleBody(assistant, '.plan-message-card')).toMatch(/max-width:\s*none/)
    expect(ruleBody(assistant, '.plan-message-card')).toMatch(/margin:\s*0/)

    // 裁决行折进活动时间线时同样要放开上限（--timeline 变体）。
    const approval = componentCss('chat/ApprovalCard.vue')
    expect(ruleBody(approval, '.approval-outcome--timeline')).toMatch(/max-width:\s*none/)
    expect(ruleBody(approval, '.approval-card--timeline')).toMatch(/max-width:\s*none/)

    // 澄清卡没有 timeline 变体，父层的 `>` 钩子又盖不到时间线插槽那几层，
    // 所以由组件自己按 `.msg-ai` 祖先复位。
    const clarify = componentCss('chat/ClarifyCard.vue')
    const nested = clarify.match(/\.msg-ai\s+\.clarify-card,\s*\.msg-ai\s+\.clarify-outcome\s*\{([\s\S]*?)\n\}/)
    expect(nested, '嵌套在历史行里的澄清卡/横幅必须有复位分支').not.toBeNull()
    expect(nested![1]).toMatch(/max-width:\s*none/)
    expect(nested![1]).toMatch(/margin-inline:\s*0/)
  })

  it('刻意保留居中的两处：浮动问卷坞与落地页叙事列', () => {
    // ① `.plan-questionnaire-dock` 是 `position: absolute; left: 50%` 的浮层，
    //    锚的是输入列而不是转写列 —— 居中就是它的语义，不许被「收干净」误伤。
    const clarify = componentCss('chat/ClarifyCard.vue')
    expect(clarify).toMatch(/\.clarify-card--docked\s*\{[^}]*width:\s*min\(100%,\s*var\(--composer-col/)
    expect(clarify).toMatch(/\.clarify-card--docked\s*\{[^}]*margin:\s*0 auto/)

    // ② 落地页（hero + 品牌 chip + 输入框）是一组刻意居中的叙事，配置卡在那里
    //    也必须留在居中列上；只有「已有会话」那一支才该左起排。
    const setup = componentCss('chat/MetaSkillSetupCard.vue')
    const landing = setup.match(/\.chat--new-landing\s+\.meta-setup-card\s*\{([\s\S]*?)\n\}/)
    expect(landing, '落地页分支必须继续居中，否则会把新会话首屏改成左对齐').not.toBeNull()
    expect(landing![1]).toMatch(/margin-inline:\s*auto/)
    expect(landing![1]).toMatch(/var\(--landing-col/)
  })

  it('窄屏不再各自硬编码 24px 留边（口径只在 chat-view.css 定义一次）', () => {
    // `--chat-gutter` 已经带 ≤768px 回落，组件里再减一次 24px 会把右边缘推到容器边界。
    for (const file of ['chat/PlanCard.vue', 'chat/ApprovalCard.vue']) {
      expect(componentCss(file), `${file} 不应再写 width: calc(100% - 24px)`)
        .not.toMatch(/width:\s*calc\(100%\s*-\s*24px\)/)
    }
  })

  it('两个渲染路径的 .msg-ai 口径逐条一致（谁生效都不该改变版式）', () => {
    const root = ruleBody(chatViewCss, '.msg-ai')
    const settled = ruleBody(componentCss('chat/AssistantMessage.vue'), '.msg-ai')
    expect(root).not.toBe('')
    expect(settled).not.toBe('')

    // 同名 scoped 规则特异度相同，命中先后取决于打包顺序 —— 所以两条声明必须等价，
    // 否则「实时气泡左对齐、结束后又居中」会随构建顺序复活。
    expect(decl(settled, 'margin-inline')).toBe(decl(root, 'margin-inline'))
    expect(decl(settled, 'text-align')).toBe(decl(root, 'text-align'))
    expect(decl(settled, 'width')).toBe(decl(root, 'width'))
    expect(decl(root, 'max-width')).toMatch(/var\(--chat-col-start/)
    expect(decl(settled, 'max-width')).toMatch(/var\(--chat-col-start/)
  })

  it('口径只在 chat-view.css 的 .chat 上定义一次，并且左起点能越过 minimap 轨道', () => {
    const chat = ruleBody(chatViewCss, '.chat')
    const gutter = decl(chat, '--chat-gutter').match(/^(\d+(?:\.\d+)?)px$/)
    expect(gutter, '--chat-gutter 必须是显式 px 值').not.toBeNull()
    // 48px = 轨道起点 14px + 聚焦刻度线 30px + 4px 焦点环；再加 8px 安全间距。
    expect(Number(gutter![1])).toBeGreaterThanOrEqual(56)
    expect(chat).toMatch(/--chat-col-start\s*:\s*min\(\s*var\(--chat-col/)
    expect(decl(chat, '--chat-col-start')).toContain('var(--chat-gutter')
    expect(decl(chat, '--chat-col-start')).toContain('var(--chat-gutter-end')
  })

  it('窄屏分支把起点退回 24px 口径（minimap 轨道在该断点隐藏）', () => {
    const mobile = chatViewCss.match(/@media \(max-width: 768px\) \{([\s\S]*)$/)
    expect(mobile, '移动端媒体查询必须存在').not.toBeNull()
    const gutter = (mobile![1].match(/\.chat\s*\{([^]*?)\}/)?.[1] ?? '')
      .match(/--chat-gutter\s*:\s*(\d+(?:\.\d+)?)px/)
    expect(gutter, '移动端需要显式覆盖 --chat-gutter').not.toBeNull()
    expect(Number(gutter![1])).toBeLessThanOrEqual(24)
  })

  it('输入面板跟随同一条阅读列，不再单独居中', () => {
    const composer = ruleBody(chatViewCss, '.chat-composer')
    expect(composer).not.toBe('')
    expect(composer, '正文左起排、输入框居中会读成版式散架').not.toMatch(/margin\s*:\s*0\s+auto/)
    expect(composer).toMatch(/margin-inline\s*:\s*calc\(\s*var\(--chat-gutter[^)]*\)\s*-\s*1\.5rem\s*\)\s+auto/)
    // 宽度上限同样按起点收窄后的列宽，否则左起点会把面板右边缘推出容器。
    expect(composer).toMatch(/width\s*:\s*min\(100%,\s*calc\(var\(--chat-col-start/)
  })

  it('ChatMessageList 不再叠加 `>` 覆盖：覆盖只在某一种 DOM 形状下命中', () => {
    const list = componentCss('chat/ChatMessageList.vue')
    expect(list).not.toMatch(/>\s*\.msg-ai/)
    expect(list).not.toMatch(/>\s*\.msg-user/)
    expect(list).not.toMatch(/\.msg-ai|\.msg-user/)
    expect(list).not.toMatch(/margin-inline\s*:\s*24px\s+auto/)
    expect(list, '行容器只做容器，不参与阅读列定义').not.toMatch(/--chat-col|--chat-gutter/)

    const row = ruleBody(list, '.chat-turn-row')
    expect(row).not.toBe('')
    expect(row).not.toMatch(/margin\s*:\s*[^;]*\bauto\b/)
    expect(row).toMatch(/width\s*:\s*100%/)
  })

  it('主题世界层不得用居中列覆盖阅读列（只允许块轴向间距）', () => {
    for (const theme of ['liquid-glass', 'marguerite-night']) {
      const world = stripComments(read(`../themes/${theme}/world.css`))
      for (const selector of ['.msg-ai', '.msg-user']) {
        const body = ruleBody(world, `[data-theme="${theme}"] ${selector}`)
        if (!body) continue
        expect(body, `${theme} 不得给转写行加水平 auto 外边距`).not.toMatch(/margin(?:-inline)?\s*:\s*[^;]*\bauto\b/)
      }
    }
  })

  it('没有把转写行的正文重新压回 center', () => {
    // 兜底：历史正文容器不得出现 text-align: center（落地页 hero 不在此列）。
    const textSelectors = ['.msg-ai-text', '.msg-ai-main', '.assistant-answer']
    for (const file of ['chat/AssistantMessage.vue', 'chat/parts/TextPart.vue']) {
      const css = componentCss(file)
      for (const selector of textSelectors) {
        const body = ruleBody(css, selector)
        if (body) expect(body, `${file} ${selector}`).not.toMatch(/text-align\s*:\s*center/)
      }
    }
    expect(componentCss('chat/parts/TextPart.vue'), 'TextPart 必须存在，否则这条兜底是空守卫')
      .not.toBe('')
    for (const selector of ['.msg-ai', '.msg-ai-main', '.chat-compaction-event']) {
      const body = ruleBody(chatViewCss, selector)
      if (body) expect(body, `chat-view.css ${selector}`).not.toMatch(/text-align\s*:\s*center/)
    }
  })
})
