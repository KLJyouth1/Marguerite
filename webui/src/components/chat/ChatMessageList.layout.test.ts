// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import type { ChatRenderedMessage } from '@/types/chat'
import ChatMessageList from './ChatMessageList.vue'
import messageListSource from './ChatMessageList.vue?raw'
import assistantSource from './AssistantMessage.vue?raw'

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

function assistantMessage(overrides: Partial<ChatRenderedMessage>): ChatRenderedMessage {
  return {
    id: 'assistant-1',
    messageId: 'assistant-1',
    role: 'assistant',
    displayRole: 'assistant',
    roleLabel: 'Assistant',
    text: '收到',
    timeStr: '',
    showHeader: false,
    ts: '2026-10-25T10:00:02Z',
    turnKey: 'turn:1',
    parts: [
      { type: 'text', html: '<p>收到</p>', rawText: '收到', key: 'assistant-1:text' },
    ],
    toolCalls: [],
    timelineItems: [],
    sources: [],
    ...overrides,
  }
}

async function mountList(messages: ChatRenderedMessage[]): Promise<HTMLElement> {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp(ChatMessageList, {
    messages,
    shareMode: false,
    selectedMessageIds: new Set<string>(),
    stripTimePrefix: (value: string) => value,
    renderMarkdown: (value: string) => `<p>${value}</p>`,
    fmtTok: (value: number) => String(value),
    toolCallGroups: () => [],
    isToolGroupOpen: () => false,
    isToolItemOpen: () => false,
    toolGroupStatusText: () => '',
    toolStatusText: () => '',
    toolSecondaryText: () => '',
    copyMessage: async () => true,
    downloadAttachment: async () => true,
    sessionKey: 'probe-session',
  })
  app.use(i18n)
  app.mount(host)
  apps.push(app)
  await nextTick()
  return host
}

function classSignature(el: Element): string {
  return `${el.tagName}:${[...el.classList].sort().join('.')}`
}

describe('ChatMessageList 阅读列版式', () => {
  it('行容器只做容器：阅读列的权威在根规则，不在这层叠加 `>` 覆盖', () => {
    // 曾经的形态：本组件用 `.chat .chat-thread > .msg-ai` /
    // `.chat .chat-turn-row > .msg-ai` 去覆盖居中。子组合器只在一种 DOM 形状下
    // 命中（水合包裹层 / 分享态一变就整条失配），失配时回落到居中 —— 就是
    // 「有时像修过、有时又居中」的机制。现在根规则本身即左对齐：
    //   · 实时气泡 → styles/chat-view.css 的 `.msg-ai`
    //   · 历史行 → AssistantMessage.vue / UserMessage.vue / CompactionEvent.vue
    // 完整不变量见 src/styles/chat-reading-column.contract.test.ts。
    const listCss = (messageListSource.match(/<style[^>]*>([\s\S]*?)<\/style>/)?.[1] ?? '')
      .replace(/\/\*[\s\S]*?\*\//g, '')
    expect(listCss).not.toMatch(/>\s*\.msg-(ai|user)/)
    expect(listCss).not.toMatch(/--chat-col|--chat-gutter/)
    expect(listCss).not.toMatch(/margin-inline:\s*24px\s+auto/)

    // AssistantMessage 自己那一条必须已经是「上限 + 左起点」，不靠外层覆盖救。
    // ⚠️ 必须**先剥掉注释**再匹配：规则体里的说明文字逐字引用了被守卫的声明原文
    // （`margin: 0 auto` / `width: var(--chat-col)`），否则会因为一句注释而误红
    // —— 这个坑 sidebar-layout.contract.test.ts 已经踩过。
    const assistantCss = (assistantSource.match(/<style[^>]*>([\s\S]*?)<\/style>/)?.[1] ?? '')
      .replace(/\/\*[\s\S]*?\*\//g, '')
    const assistantRule = assistantCss.match(/\.msg-ai\s*\{([\s\S]*?)\n\}/)?.[1] ?? ''
    expect(assistantRule).not.toBe('')
    expect(assistantRule).not.toMatch(/margin:\s*0\s+auto/)
    expect(assistantRule).not.toMatch(/(?:^|\n)\s*width\s*:\s*var\(--chat-col/)
    expect(assistantRule).toMatch(/max-width:\s*var\(--chat-col-start/)
    expect(assistantRule).toMatch(/margin-inline:\s*var\(--chat-gutter,\s*24px\)\s+auto/)
    expect(assistantRule).toMatch(/text-align:\s*start/)

    // 行容器本身不得居中。
    expect(messageListSource).not.toMatch(/\.chat-turn-row\s*\{[^}]*margin:\s*0 auto/s)
  })

  it('历史水合与实时消息落在同一行容器、同一版式结构', async () => {
    const live = assistantMessage({
      id: 'assistant-live',
      messageId: 'assistant-live',
      turnKey: 'turn:live',
      isStreaming: true,
    })
    const history = assistantMessage({
      id: 'assistant-history',
      messageId: 'assistant-history',
      turnKey: 'turn:history',
      restoredFromHistory: true,
    })
    const user: ChatRenderedMessage = {
      id: 'user-1',
      role: 'user',
      displayRole: 'user',
      roleLabel: 'You',
      text: '只回复两个字：收到',
      timeStr: '',
      showHeader: false,
      ts: '2026-10-25T10:00:00Z',
      turnKey: 'turn:live',
    }
    const host = await mountList([user, live, history])

    const rows = host.querySelectorAll('.chat-turn-row')
    expect(rows).toHaveLength(3)
    for (const row of rows) {
      expect(row.hasAttribute('data-chat-turn-row')).toBe(true)
      expect(row.getAttribute('style')).toBeNull()
    }

    const assistants = [...host.querySelectorAll<HTMLElement>('.msg-ai')]
    expect(assistants).toHaveLength(2)
    for (const assistant of assistants) {
      expect(assistant.closest('.chat-turn-row')).toBeTruthy()
      expect(assistant.getAttribute('style')).toBeNull()
      expect(assistant.querySelector('.msg-ai-main')).toBeTruthy()
    }
    expect(classSignature(assistants[0])).toBe(classSignature(assistants[1]))
    expect(assistants[0].querySelector('.msg-ai-main')?.className)
      .toBe(assistants[1].querySelector('.msg-ai-main')?.className)
    expect(host.querySelector('[style*="text-align: center"]')).toBeNull()
  })
})
