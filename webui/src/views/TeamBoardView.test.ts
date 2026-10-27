// @vitest-environment happy-dom
// 团队看板：成员卡片、状态徽章、空态与门控降级。数据全部来自 team.members fixture。
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const rpcMock = vi.hoisted(() => ({
  call: null as unknown as ReturnType<typeof vi.fn>,
  methods: null as unknown as { value: string[] },
}))

vi.mock('@/stores/rpc', async () => {
  const { ref: vueRef } = await import('vue')
  const methods = vueRef<string[]>(['team.members'])
  const call = vi.fn()
  rpcMock.call = call as unknown as ReturnType<typeof vi.fn>
  rpcMock.methods = methods as unknown as { value: string[] }
  return {
    useRpcStore: () => ({
      call,
      on: vi.fn(() => () => {}),
      supportsMethod: (method: string) => methods.value.includes(method),
    }),
  }
})

const messages: Record<string, string> = {
  'team.board.title': '团队看板',
  'team.board.empty': '当前会话没有团队任务',
  'team.board.refresh': '刷新',
  'team.board.toolCalls': '次工具调用',
  'team.board.lastActivity': '最近活动',
  'team.board.status.running': '进行中',
  'team.board.status.waiting': '等待中',
  'team.board.status.finished': '已完成',
  'team.board.status.failed': '失败',
  'usageLogs.metrics.tokens': 'Token',
}

vi.mock('vue-i18n', async (importOriginal) => {
  const actual = await importOriginal<typeof import('vue-i18n')>()
  const { ref: vueRef } = await import('vue')
  return {
    ...actual,
    useI18n: () => ({
      t: (key: string) => messages[key] ?? key,
      locale: vueRef('zh-Hans'),
    }),
  }
})

vi.mock('@/components/Icon.vue', () => ({
  default: defineComponent({
    name: 'IconStub',
    props: { name: { type: String, default: '' } },
    setup(props) {
      return () => h('span', { 'data-icon': props.name })
    },
  }),
}))

import TeamBoardView from '@/views/TeamBoardView.vue'

let app: App<Element> | null = null

async function flush(): Promise<void> {
  for (let index = 0; index < 6; index += 1) await Promise.resolve()
  await nextTick()
}

async function mountBoard(): Promise<HTMLElement> {
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(TeamBoardView)
  app.mount(el)
  await flush()
  return el
}

function member(overrides: Record<string, unknown> = {}) {
  return {
    id: 'lead-1',
    label: 'Ada',
    role: '协调者',
    kind: 'lead',
    status: 'running',
    currentStep: '拆解任务并分配子任务',
    toolCalls: 12,
    lastActivityMs: Date.now() - 60_000,
    tokensUsed: 3456,
    ...overrides,
  }
}

const cards = (el: HTMLElement) => Array.from(el.querySelectorAll<HTMLElement>('.team-card'))

beforeEach(() => {
  rpcMock.methods.value = ['team.members']
  rpcMock.call.mockReset()
  rpcMock.call.mockResolvedValue({
    ok: true,
    session: 'agent:main:webchat:test',
    updatedAt: Date.now(),
    members: [
      member(),
      member({
        id: 'sub-1',
        label: 'Bolt',
        role: '编码',
        kind: 'subagent',
        status: 'finished',
        currentStep: null,
        toolCalls: 4,
        tokensUsed: null,
      }),
    ],
  })
})

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  vi.clearAllMocks()
})

describe('TeamBoardView member cards', () => {
  it('renders one independent card per contract member', async () => {
    const el = await mountBoard()
    const rendered = cards(el)

    expect(rendered).toHaveLength(2)
    expect(el.textContent).toContain('Ada')
    expect(el.textContent).toContain('协调者')
    expect(el.textContent).toContain('Bolt')

    // 首字母头像 + 全量步骤放进 title（正文单行省略）
    const first = rendered[0]
    expect(first.querySelector('.team-card__avatar')?.textContent?.trim()).toBe('A')
    expect(first.querySelector('.team-card__step')?.textContent?.trim()).toBe('拆解任务并分配子任务')
    expect(first.querySelector('.team-card__step')?.getAttribute('title')).toBe('拆解任务并分配子任务')

    // 统计行：工具调用 / token / 最近活动
    expect(first.textContent).toContain('12')
    expect(first.textContent).toContain('次工具调用')
    expect(first.textContent).toContain('3,456')

    // 没有 token 数据的成员如实显示占位，而不是 0
    expect(rendered[1].textContent).toContain('Token —')
  })

  it('maps the four contract statuses onto four semantic badge classes', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      members: [
        member({ id: 'm-running', status: 'running' }),
        member({ id: 'm-waiting', status: 'waiting' }),
        member({ id: 'm-finished', status: 'finished' }),
        member({ id: 'm-failed', status: 'failed' }),
      ],
    })

    const el = await mountBoard()
    const badges = Array.from(el.querySelectorAll<HTMLElement>('.team-card__status'))

    expect(badges.map((badge) => badge.textContent?.trim())).toEqual([
      '进行中',
      '等待中',
      '已完成',
      '失败',
    ])
    expect(badges.map((badge) => badge.className)).toEqual([
      'team-card__status team-status--running',
      'team-card__status team-status--waiting',
      'team-card__status team-status--finished',
      'team-card__status team-status--failed',
    ])
    expect(cards(el).map((card) => card.dataset.status)).toEqual([
      'running',
      'waiting',
      'finished',
      'failed',
    ])
  })
})

describe('TeamBoardView honest empty states', () => {
  it('shows the empty copy when the gateway returns no members', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      members: [],
    })

    const el = await mountBoard()

    expect(cards(el)).toHaveLength(0)
    expect(el.textContent).toContain('当前会话没有团队任务')
    expect(el.querySelector('.team-board__note')).toBeNull()
  })

  it('shows an explicit note and no cards when team.members is not supported', async () => {
    rpcMock.methods.value = []

    const el = await mountBoard()
    const refreshButton = el.querySelector<HTMLButtonElement>('header button')

    expect(cards(el)).toHaveLength(0)
    expect(rpcMock.call).not.toHaveBeenCalled()
    expect(el.textContent).toContain('当前会话没有团队任务')
    expect(el.textContent).toContain('当前网关未提供 team.members')
    expect(refreshButton?.disabled).toBe(true)
  })

  it('surfaces a read failure instead of stale cards', async () => {
    rpcMock.call.mockRejectedValue(new Error('gateway offline'))

    const el = await mountBoard()

    expect(cards(el)).toHaveLength(0)
    expect(el.textContent).toContain('当前会话没有团队任务')
    expect(el.textContent).toContain('团队状态读取失败：gateway offline')
  })
})
