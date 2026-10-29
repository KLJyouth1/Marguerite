// @vitest-environment happy-dom
// 团队看板：成员卡片、状态徽章、空态与门控降级。数据全部来自 team.members fixture。
import { createApp, defineComponent, h, nextTick, type App } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const rpcMock = vi.hoisted(() => ({
  call: null as unknown as ReturnType<typeof vi.fn>,
  methods: null as unknown as { value: string[] },
  handlers: {} as Record<string, (payload?: unknown, meta?: unknown) => void>,
}))

vi.mock('@/stores/rpc', async () => {
  const { ref: vueRef } = await import('vue')
  const methods = vueRef<string[]>(['team.members'])
  const call = vi.fn()
  rpcMock.call = call as unknown as ReturnType<typeof vi.fn>
  rpcMock.methods = methods as unknown as { value: string[] }
  rpcMock.handlers = {}
  return {
    useRpcStore: () => ({
      call,
      on: (event: string, handler: (payload?: unknown, meta?: unknown) => void) => {
        rpcMock.handlers[event] = handler
        return () => {
          delete rpcMock.handlers[event]
        }
      },
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
  'team.board.viewGrid': '网格视图',
  'team.board.viewKanban': '看板视图',
  'team.board.colEmpty': '暂无成员',
  'team.board.status.running': '进行中',
  'team.board.status.waiting': '等待中',
  'team.board.status.finished': '已完成',
  'team.board.status.failed': '失败',
  'usageLogs.metrics.tokens': 'Token',
  // 以下全部是**既有**字典键（本次零新增），这里只给出中文取值用于断言。
  'console.channels.notReported': '未报告',
  'chat.thinking': '思考中',
  'harness.runs.started': '开始时间',
  'harness.runs.duration': '耗时',
  'chat.orchestration.runMemberThinking': '成员思考',
  'chat.orchestration.runMemberOutcome': '结果',
  'chat.orchestration.runMemberNoTools': '尚未调用工具',
  'chat.orchestration.runSynthesizing': '总编排正在综合成员结论',
  'chat.orchestration.memberCount': '{n} 成员',
  'chat.orchestration.formTeam': '团队',
  'chat.orchestration.formSwarm': '蜂群',
  'chat.orchestration.runFilesMore': '另有 {n} 个未列出',
  'cronSkills.list.metaLastRun': '上次运行',
  'chat.activityDurationSeconds': '{seconds} 秒',
  'chat.activityDurationMinutes': '{minutes} 分 {seconds} 秒',
}

vi.mock('vue-i18n', async (importOriginal) => {
  const actual = await importOriginal<typeof import('vue-i18n')>()
  const { ref: vueRef } = await import('vue')
  return {
    ...actual,
    useI18n: () => ({
      t: (key: string, named?: Record<string, unknown>) => {
        const raw = messages[key] ?? key
        if (!named) return raw
        // tsconfig 的 lib 是 ES2020，`String.replaceAll` 属 ES2021 —— 用 split/join
        // 等价实现占位符替换，避免为了一个测试把整个项目的 lib 抬到 ES2021。
        return Object.entries(named).reduce(
          (text, [name, value]) => text.split(`{${name}}`).join(String(value)),
          raw,
        )
      },
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

/** 走真实链路：WS 帧 → rpc.on('session.event.progress') → 看板动向表。 */
function fireFrame(payload: Record<string, unknown>): void {
  rpcMock.handlers['session.event.progress']?.(payload)
}

function orchestration(overrides: Record<string, unknown> = {}) {
  return { kind: 'orchestration', runId: 'run-1', ...overrides }
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
  localStorage.clear()
  rpcMock.methods.value = ['team.members']
  rpcMock.call.mockReset()
  rpcMock.call.mockResolvedValue({
    ok: true,
    session: 'agent:main:webchat:test',
    updatedAt: Date.now(),
    runId: 'run-1',
    form: 'team',
    memberLimit: 512,
    dropped: 0,
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

  it('passes the backend reason for the empty board through verbatim', async () => {
    // 这条正是「团队看板看着什么也没有」的真实原因之一：单选直答不产生编排运行记录
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: null,
      form: null,
      members: [],
      note: '当前没有团队编排运行记录（单选直答或尚未派活）',
    })

    const el = await mountBoard()

    expect(cards(el)).toHaveLength(0)
    expect(el.querySelector('[data-testid="board-empty-note"]')?.textContent?.trim()).toBe(
      '当前没有团队编排运行记录（单选直答或尚未派活）',
    )
  })

  it('shows an explicit note and no cards when team.members is not supported', async () => {
    rpcMock.methods.value = []

    const el = await mountBoard()
    const refreshButton = el.querySelector<HTMLButtonElement>('[data-testid="board-refresh"]')

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

describe('TeamBoardView per-member activity (event stream)', () => {
  it('shows 未报告 on every activity row when no orchestration frame was observed', async () => {
    const el = await mountBoard()
    const first = cards(el)[0]

    expect(first.dataset.live).toBe('false')
    expect(first.querySelector('.member-activity')).not.toBeNull()

    const rowText = (testid: string) =>
      first.querySelector<HTMLElement>(`[data-testid="${testid}"] dd`)?.textContent?.trim() ?? ''

    // 后端在事件流里发了、但看板没观察到 → 只能是空态，不能是 0 / 猜测。
    expect(rowText('member-thinking')).toBe('未报告')
    expect(rowText('member-outcome')).toBe('未报告')
    expect(rowText('member-started')).toBe('未报告')
    expect(rowText('member-duration')).toBe('未报告')
    // 没观察到起点时不能替成员断言「尚未调用工具」
    expect(rowText('member-tool')).toBe('未报告')
    expect(first.textContent).not.toContain('尚未调用工具')
    expect(first.textContent).not.toContain('0 秒')
    // 页脚的真实计数仍在（快照里的 toolCalls）
    expect(first.textContent).toContain('12')
  })

  it('renders thinking / last tool / outcome / duration from the frame payload', async () => {
    const el = await mountBoard()
    fireFrame(orchestration({ phase: 'member_started', memberId: 'sub-1', roleLabel: '编码' }))
    fireFrame(orchestration({ phase: 'member_thinking', memberId: 'sub-1', round: 2, thinking: '先补断言再改样式' }))
    fireFrame(orchestration({ phase: 'tool_call', memberId: 'sub-1', tool: 'shell' }))
    fireFrame(orchestration({ phase: 'tool_result', memberId: 'sub-1', tool: 'shell', ok: true, elapsedMs: 4_812 }))
    fireFrame(
      orchestration({
        phase: 'member_completed',
        memberId: 'sub-1',
        ok: true,
        elapsedMs: 90_500,
        summary: '改了 3 个断言',
      }),
    )
    await flush()

    const card = cards(el).find((item) => item.dataset.memberId === 'sub-1')!
    expect(card.dataset.live).toBe('true')
    expect(card.textContent).toContain('先补断言再改样式')
    expect(card.textContent).toContain('shell')
    expect(card.textContent).toContain('改了 3 个断言')
    expect(card.textContent).toContain('1 分 31 秒')
    // 终态帧压过快照的 running
    expect(card.dataset.status).toBe('finished')
    expect(card.querySelector('[data-testid="member-tool"] .member-activity__tool-mark')?.textContent).toBe('✓')
  })

  it('reports a pending tool call without dressing it up as success', async () => {
    const el = await mountBoard()
    fireFrame(orchestration({ phase: 'member_started', memberId: 'sub-1' }))
    fireFrame(orchestration({ phase: 'tool_call', memberId: 'sub-1', tool: 'read_file' }))
    await flush()

    const card = cards(el).find((item) => item.dataset.memberId === 'sub-1')!
    const toolRow = card.querySelector<HTMLElement>('[data-testid="member-tool"]')
    expect(toolRow).not.toBeNull()
    expect(toolRow?.textContent).toContain('read_file')
    // 没有 tool_result 之前不显示成功/失败标记
    expect(toolRow?.querySelector('.member-activity__tool-mark')).toBeNull()
    expect(toolRow?.querySelector<HTMLElement>('.member-activity__tool')?.getAttribute('data-tool-state')).toBe('pending')
    expect(card.querySelector('[data-testid="member-outcome"]')?.textContent).toContain('未报告')
  })

  it('turns the badge into 思考中 only for a real member_thinking frame', async () => {
    const el = await mountBoard()
    fireFrame(orchestration({ phase: 'member_started', memberId: 'sub-1' }))
    fireFrame(orchestration({ phase: 'member_thinking', memberId: 'sub-1', round: 1, thinking: '在想' }))
    await flush()

    const card = cards(el).find((item) => item.dataset.memberId === 'sub-1')!
    // 快照里这位是 finished —— 比快照新的观察才生效，故 finished 不被降级
    expect(card.dataset.status).toBe('finished')
    expect(card.textContent).toContain('在想')

    fireFrame(orchestration({ phase: 'member_started', memberId: 'lead-1' }))
    fireFrame(orchestration({ phase: 'member_thinking', memberId: 'lead-1', round: 1, thinking: '在拆任务' }))
    await flush()
    const lead = cards(el).find((item) => item.dataset.memberId === 'lead-1')!
    expect(lead.dataset.status).toBe('thinking')
    expect(lead.querySelector('.team-card__status')?.className).toBe('team-card__status team-status--thinking')
    expect(lead.querySelector('.team-card__status')?.textContent?.trim()).toBe('思考中')
  })

  it('collects the gateway-only run_failed frame that never reaches team.members', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      runId: 'run-9',
      form: 'team',
      members: [
        {
          id: 'lead:run-9',
          label: '总编排',
          role: 'lead',
          kind: 'lead',
          status: 'running',
          currentStep: null,
          toolCalls: 0,
          lastActivityMs: Date.now(),
          tokensUsed: null,
        },
      ],
    })
    const el = await mountBoard()
    const lead = cards(el).find((item) => item.dataset.memberId === 'lead:run-9')!
    expect(lead.dataset.status).toBe('running')

    // 后端这条是直接 bus.publish 的（core/src/gateway/team.rs），投影里永远没有
    fireFrame({ kind: 'orchestration', phase: 'run_failed', runId: 'run-9', ok: false, error: '已取消' })
    await flush()

    expect(lead.dataset.status).toBe('failed')
    expect(lead.querySelector('.team-card__status')?.textContent?.trim()).toBe('失败')
    expect(lead.querySelector('[data-testid="member-error"]')?.textContent?.trim()).toBe('已取消')
  })

  it('shows the synthesizing hint only when lead_synthesizing was observed', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      runId: 'run-9',
      form: 'team',
      members: [
        { id: 'lead:run-9', label: '总编排', role: 'lead', kind: 'lead', status: 'running', currentStep: null, toolCalls: 0, lastActivityMs: Date.now(), tokensUsed: null },
      ],
    })
    const el = await mountBoard()
    expect(el.querySelector('[data-testid="member-synthesizing"]')).toBeNull()

    fireFrame(orchestration({ runId: 'run-9', phase: 'member_started', memberId: 'sub-9' }))
    fireFrame(orchestration({ runId: 'run-9', phase: 'lead_synthesizing', memberId: '' }))
    await flush()
    expect(el.querySelector('[data-testid="member-synthesizing"]')?.textContent?.trim()).toBe(
      '总编排正在综合成员结论',
    )
  })
})

describe('TeamBoardView run bar', () => {
  it('names the run and keeps the synthesized lead out of the member count', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      runId: 'run-42',
      form: 'swarm',
      memberLimit: 512,
      dropped: 0,
      members: [
        { id: 'lead:run-42', label: '总编排', role: 'lead', kind: 'lead', status: 'running', currentStep: null, toolCalls: 0, lastActivityMs: null, tokensUsed: null },
        { id: 's-1', label: 'One', role: '编码', kind: 'subagent', status: 'running', currentStep: 'a', toolCalls: 1, lastActivityMs: null, tokensUsed: null },
        { id: 's-2', label: 'Two', role: '评审', kind: 'subagent', status: 'finished', currentStep: 'b', toolCalls: 2, lastActivityMs: null, tokensUsed: null },
      ],
    })

    const el = await mountBoard()

    expect(cards(el)).toHaveLength(3)
    const bar = el.querySelector<HTMLElement>('[data-testid="team-run-bar"]')
    expect(bar).not.toBeNull()
    expect(bar?.textContent).toContain('蜂群')
    expect(bar?.textContent).toContain('#run-42')
    // 3 张卡但 2 名成员：lead 行是后端合成的，不计入成员数
    expect(el.querySelector('[data-testid="team-run-bar-members"]')?.textContent).toContain('2 成员')
    expect(cards(el)[0].querySelector('[data-testid="member-run-label"]')?.textContent?.trim()).toBe('蜂群 · #run-42')
  })

  it('says so when members were dropped or the snapshot came off disk', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now() - 40_000,
      runId: 'run-77',
      form: 'team',
      memberLimit: 512,
      dropped: 37,
      fromDisk: true,
      note: '当前没有团队编排运行记录（单选直答或尚未派活）',
      members: [{ id: 'lead:run-77', label: '总编排', role: 'lead', kind: 'lead', status: 'running', currentStep: null, toolCalls: 0, lastActivityMs: null, tokensUsed: null }],
    })

    const el = await mountBoard()

    expect(el.querySelector('[data-testid="team-run-bar-dropped"]')?.textContent).toContain('另有 37 个未列出')
    expect(el.querySelector('[data-testid="team-run-bar-dropped"]')?.getAttribute('title')).toBe('0/512')
    expect(el.querySelector('[data-testid="team-run-bar-from-disk"]')?.textContent?.trim()).toBe('上次运行')
    expect(el.querySelector('[data-testid="team-run-bar-note"]')?.textContent).toContain(
      '当前没有团队编排运行记录（单选直答或尚未派活）',
    )
  })

  it('renders no run bar at all for the empty board', async () => {
    rpcMock.call.mockResolvedValue({ ok: true, session: 's', updatedAt: Date.now(), members: [] })
    const el = await mountBoard()

    expect(el.querySelector('[data-testid="team-run-bar"]')).toBeNull()
    expect(el.querySelector('[data-testid="team-run-bar-dropped"]')).toBeNull()
  })
})

describe('TeamBoardView kanban lanes (A3)', () => {
  const toggle = (el: HTMLElement, testid: string) =>
    el.querySelector<HTMLButtonElement>(`[data-testid="${testid}"]`)!

  it('defaults to the grid view so existing behaviour has zero regression', async () => {
    const el = await mountBoard()

    expect(el.querySelector('.team-board__grid')).not.toBeNull()
    expect(el.querySelector('[data-testid="board-kanban"]')).toBeNull()
  })

  it('partitions members into honest status lanes and shows per-lane counts', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      runId: 'run-1',
      form: 'team',
      members: [
        member({ id: 'lead-1', status: 'running' }),
        member({ id: 's-wait', label: 'Wait', status: 'waiting' }),
        member({ id: 's-done', label: 'Done', status: 'finished' }),
        member({ id: 's-bad', label: 'Bad', status: 'failed' }),
        member({ id: 's-odd', label: 'Odd', status: 'queued' }),
      ],
    })
    const el = await mountBoard()
    toggle(el, 'board-view-kanban').click()
    await flush()

    const kanban = el.querySelector('[data-testid="board-kanban"]')
    expect(kanban).not.toBeNull()
    expect(el.querySelector('.team-board__grid')).toBeNull()

    // 五条泳道固定列序：待命 → 进行中 → 已完成 → 异常 → 未报告
    const columns = Array.from(el.querySelectorAll<HTMLElement>('.team-board__col'))
    expect(columns.map((col) => col.dataset.column)).toEqual([
      'waiting',
      'running',
      'finished',
      'failed',
      'other',
    ])

    const laneMembers = (id: string) =>
      columns
        .find((col) => col.dataset.column === id)!
        .querySelectorAll<HTMLElement>('.team-card')

    expect(laneMembers('waiting')).toHaveLength(1)
    expect(laneMembers('running')).toHaveLength(1)
    expect(laneMembers('finished')).toHaveLength(1)
    expect(laneMembers('failed')).toHaveLength(1)
    // 后端没声明的状态落「未报告」列，卡片上仍显示原值 —— 不冒充已知状态
    const odd = laneMembers('other')
    expect(odd).toHaveLength(1)
    expect(odd[0].dataset.memberId).toBe('s-odd')
    expect(odd[0].querySelector('.team-card__status')?.textContent?.trim()).toBe('queued')

    // 计数徽章与桶内卡数一致
    const counts = columns.map((col) => col.querySelector('.team-board__col-count')?.textContent?.trim())
    expect(counts).toEqual(['1', '1', '1', '1', '1'])
  })

  it('shows the empty-lane copy instead of hiding empty columns', async () => {
    rpcMock.call.mockResolvedValue({
      ok: true,
      session: 'agent:main:webchat:test',
      updatedAt: Date.now(),
      members: [member({ id: 'lead-1', status: 'running' })],
    })
    const el = await mountBoard()
    toggle(el, 'board-view-kanban').click()
    await flush()

    const columns = Array.from(el.querySelectorAll<HTMLElement>('.team-board__col'))
    expect(columns).toHaveLength(5)
    const emptyLane = columns.find((col) => col.dataset.column === 'waiting')!
    expect(emptyLane.querySelector('.team-board__col-empty')?.textContent?.trim()).toBe('暂无成员')
    expect(emptyLane.querySelectorAll('.team-card')).toHaveLength(0)
  })

  it('persists the kanban preference and restores it on the next mount', async () => {
    let el = await mountBoard()
    toggle(el, 'board-view-kanban').click()
    await flush()
    expect(localStorage.getItem('marguerite.teamBoard.view.v1')).toBe('kanban')

    app?.unmount()
    document.body.innerHTML = ''
    el = await mountBoard()
    expect(el.querySelector('[data-testid="board-kanban"]')).not.toBeNull()

    // 切回网格同样持久化
    toggle(el, 'board-view-grid').click()
    await flush()
    expect(localStorage.getItem('marguerite.teamBoard.view.v1')).toBe('grid')
    expect(el.querySelector('.team-board__grid')).not.toBeNull()
  })
})
