// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { createApp, h, nextTick } from 'vue'
import i18n from '@/i18n'
import OrchestrationRunCard from './OrchestrationRunCard.vue'
import type {
  OrchestrationMember,
  OrchestrationRun,
  OrchestrationToolRow,
} from '@/composables/chat/useOrchestrationRuns'

function member(id: string, status: OrchestrationMember['status']): OrchestrationMember {
  return {
    id,
    role: id,
    roleLabel: id,
    task: '生成 HTML 网页',
    status,
    startedAtMs: 1_000,
    elapsedMs: status === 'running' ? null : 120_000,
    summary: status === 'error' ? 'timeout' : 'ok',
    tools: [],
  }
}

/** 真机工具行样本（2026-10-23 截图）：检索调用 + shell 调用。 */
function tool(overrides: Partial<OrchestrationToolRow> = {}): OrchestrationToolRow {
  return {
    id: `tool-${Math.random()}`,
    tool: 'web_search',
    argsPreview: '{"query":"three.js island","max_results":3}',
    outputPreview: '',
    status: 'ok',
    elapsedMs: 693,
    at: '12:00:00',
    ...overrides,
  }
}

function run(overrides: Partial<OrchestrationRun> = {}): OrchestrationRun {
  return {
    runId: 'orch-1',
    form: 'expert',
    goal: '做一个唯美的空岛+树的WEB 3D演示页面',
    status: 'completed',
    startedAtMs: 0,
    endedAtMs: 130_000,
    memberEstimate: 2,
    members: [member('researcher', 'ok'), member('frontend', 'ok')],
    synthesizing: false,
    reply: '',
    error: '',
    ...overrides,
  }
}

function mount(r: OrchestrationRun) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp({ render: () => h(OrchestrationRunCard, { run: r, now: 130_000 }) })
  app.use(i18n)
  app.mount(el)
  return { app, el }
}

describe('OrchestrationRunCard —— 完成度必须如实（成员失败不得打成功绿标）', () => {
  it('全部成员成功 → 已完成，且不带 partial 修饰', () => {
    const { app, el } = mount(run())
    const section = el.querySelector('[data-testid="orchestration-run"]')!
    expect(section.textContent).toContain(i18n.global.t('chat.orchestration.runCompleted'))
    expect(section.textContent).not.toContain('失败')
    expect(section.classList.contains('orch-run--partial')).toBe(false)
    app.unmount()
  })

  it('回执 completed 但有成员超时 → 必须显式说明「N 名成员失败」并走 warn 态', () => {
    // 复现实测：前端工程师 120s timeout、零产物，后端仍发 run_completed
    const { app, el } = mount(run({
      members: [member('researcher', 'ok'), member('frontend', 'error')],
    }))
    const section = el.querySelector('[data-testid="orchestration-run"]')!
    expect(section.textContent).toContain(
      i18n.global.t('chat.orchestration.runCompletedPartial', { n: 1 }),
    )
    expect(section.classList.contains('orch-run--partial')).toBe(true)
    // 只有失败成员时报错数要跟着变，不能写死 1
    app.unmount()
    const two = mount(run({
      members: [member('a', 'error'), member('b', 'error')],
    }))
    expect(two.el.textContent).toContain(
      i18n.global.t('chat.orchestration.runCompletedPartial', { n: 2 }),
    )
    two.app.unmount()
  })

  it('整轮失败仍优先显示「执行失败」，不被 partial 覆盖', () => {
    const { app, el } = mount(run({
      status: 'failed',
      members: [member('a', 'error')],
      error: 'boom',
    }))
    const section = el.querySelector('[data-testid="orchestration-run"]')!
    expect(section.textContent).toContain(i18n.global.t('chat.orchestration.runFailed'))
    expect(section.classList.contains('orch-run--failed')).toBe(true)
    expect(section.classList.contains('orch-run--partial')).toBe(false)
    app.unmount()
  })
})

/**
 * 用户要求：每个蜂群一张卡，点进去能看见代理的分工、动作、调用、输出。
 * 此前成员只是扁平列表，工具行只有「名字 + 参数」，输出完全不渲染。
 */
describe('OrchestrationRunCard —— 分工网格与代理详情', () => {
  function runningRun(): OrchestrationRun {
    const researcher = member('researcher', 'ok')
    researcher.roleLabel = '检索专家'
    researcher.task = '搜集 Three.js 低多边形岛屿的实现资料'
    researcher.tools = [
      tool({
        tool: 'web_search',
        argsPreview: '{"query":"three.js floating island","max_results":3}',
        outputPreview: '1. Three.js floating island\n2. Low poly island tutorial',
      }),
      tool({
        id: 'tool-shell',
        tool: 'shell_exec',
        argsPreview: '{"command":"node --version; ls"}',
        outputPreview: 'v20.11.0\nindex.html',
        elapsedMs: 567,
      }),
    ]
    const frontend = member('frontend', 'running')
    frontend.roleLabel = '前端工程师'
    frontend.task = '实现岛屿页面'
    frontend.elapsedMs = null
    return run({
      runId: 'orch-live',
      status: 'running',
      endedAtMs: null,
      members: [researcher, frontend],
    })
  }

  it('头部进度条按成员完成数推进，并带无障碍语义', () => {
    const { app, el } = mount(runningRun())
    const bar = el.querySelector('.orch-run__track')!
    expect(bar.getAttribute('role')).toBe('progressbar')
    expect(bar.getAttribute('aria-valuenow')).toBe('50')
    expect(bar.getAttribute('aria-label')).toBe(
      i18n.global.t('chat.orchestration.runProgressLabel', { done: 1, total: 2 }),
    )
    // 一条已完成 + 一条运行中 → 1/2（头部进度条右侧的唯一比例文案）
    expect(el.querySelector('.orch-run__ratio')?.textContent?.trim()).toBe('1/2')
    app.unmount()
  })

  it('成员尚未上报时给出「组建中」骨架，而不是像报错的空白', () => {
    const { app, el } = mount(run({
      runId: 'orch-boot',
      status: 'running',
      endedAtMs: null,
      members: [],
      memberEstimate: 3,
    }))
    expect(el.textContent).toContain(i18n.global.t('chat.orchestration.runNoMembers'))
    // 骨架数量跟随预估成员数（3 名 → 3 个占位块），不能与头部计数自相矛盾
    expect(el.querySelectorAll('.orch-skeleton')).toHaveLength(3)
    // 骨架不是可点击的代理卡（不能被误点进详情）
    expect(el.querySelectorAll('[data-testid="orchestration-agent"]')).toHaveLength(0)
    app.unmount()
  })

  it('每个代理一张卡：角色、任务、调用数、状态都在网格里', () => {
    const { app, el } = mount(runningRun())
    const cards = el.querySelectorAll('[data-testid="orchestration-agent"]')
    expect(cards).toHaveLength(2)
    const text = el.textContent || ''
    expect(text).toContain('检索专家')
    expect(text).toContain('前端工程师')
    expect(text).toContain(i18n.global.t('chat.orchestration.runRoster'))
    // 调用数：检索专家 2 次
    expect(cards[0].textContent).toContain(
      i18n.global.t('chat.orchestration.runToolCount', { n: 2 }),
    )
    app.unmount()
  })

  it('点代理进入详情：完整任务 + 逐次调用（参数/输出分别美化）+ 结果', async () => {
    const { app, el } = mount(runningRun())
    ;(el.querySelectorAll('[data-testid="orchestration-agent"]')[0] as HTMLButtonElement).click()
    await nextTick()

    const detail = el.querySelector('[data-testid="orchestration-member-detail"]')!
    expect(detail).toBeTruthy()
    // 分工网格让位给详情
    expect(el.querySelector('[data-testid="orchestration-roster"]')).toBeNull()
    const text = detail.textContent || ''
    expect(text).toContain('搜集 Three.js 低多边形岛屿的实现资料')
    // 两次调用各一张工具卡（默认折叠：过程可见但不喧宾夺主）
    const toolCards = detail.querySelectorAll('[data-testid="tool-invocation"]')
    expect(toolCards).toHaveLength(2)
    expect(text).not.toContain(i18n.global.t('chat.orchestration.runToolOutput'))

    // 展开第一次调用：参数与输出分块渲染
    ;(toolCards[0].querySelector('.tool-inv__head') as HTMLButtonElement).click()
    await nextTick()
    const expanded = toolCards[0].textContent || ''
    expect(expanded).toContain(i18n.global.t('chat.orchestration.runToolArgs'))
    expect(expanded).toContain(i18n.global.t('chat.orchestration.runToolOutput'))

    // JSON 参数被 prettify（带缩进换行），而不是挤成一行原始串
    const pres = Array.from(detail.querySelectorAll('pre'))
    expect(pres.some(pre => pre.textContent?.includes('\n  "query"'))).toBe(true)
    // 输出可见（此前 outputPreview 从未渲染）
    expect(pres.some(pre => pre.textContent?.includes('Low poly island tutorial'))).toBe(true)
    // 代理结论
    expect(text).toContain(i18n.global.t('chat.orchestration.runMemberOutcome'))

    // 返回分工
    ;(detail.querySelector('.orch-detail__back') as HTMLButtonElement).click()
    await nextTick()
    expect(el.querySelector('[data-testid="orchestration-roster"]')).toBeTruthy()
    expect(el.querySelector('[data-testid="orchestration-member-detail"]')).toBeNull()
    app.unmount()
  })

  it('没有工具调用的代理：详情里明确写「尚未调用工具」', async () => {
    const { app, el } = mount(runningRun())
    ;(el.querySelectorAll('[data-testid="orchestration-agent"]')[1] as HTMLButtonElement).click()
    await nextTick()
    const detail = el.querySelector('[data-testid="orchestration-member-detail"]')!
    expect(detail.textContent).toContain(
      i18n.global.t('chat.orchestration.runMemberNoTools'),
    )
    app.unmount()
  })

  /**
   * 真机验收缺陷（2026-10-23，18899 网关真实 swarm）：代理结论与最终答复是 markdown，
   * 但此前按纯文本渲染，用户看到的是 `## 结论`、`**加粗**` 和整张表格的竖线。
   *
   * 注：happy-dom 下 DOMPurify 对块级标签的行为与 Chromium 不一致（会吞标签），
   * 所以这里只断言「原始 markdown 标记不再以字面量出现」；标题/表格的真实渲染
   * 由浏览器端验收脚本确认。
   */
  it('代理结论经 markdown 渲染（不再字面吐出 ## / **）', async () => {
    const live = runningRun()
    live.members[0].summary = '## 结论\n\n**官网**：https://threejs.org/'
    const { app, el } = mount(live)
    ;(el.querySelectorAll('[data-testid="orchestration-agent"]')[0] as HTMLButtonElement).click()
    await nextTick()

    const out = el.querySelector('.orch-detail__out')!
    expect(out.textContent).toContain('结论')
    expect(out.textContent).not.toContain('##')
    expect(out.textContent).not.toContain('**')
    app.unmount()
  })

  it('最终答复也按 markdown 渲染（列表不再是 - 前缀纯文本）', async () => {
    const live = runningRun()
    live.status = 'completed'
    live.endedAtMs = 200_000
    live.reply = '- 要点一\n- 要点二'
    const { app, el } = mount(live)
    // 已结束的卡默认收起 → 先展开
    ;(el.querySelector('.orch-run__toggle') as HTMLButtonElement).click()
    await nextTick()
    ;(el.querySelector('.orch-run__reply-toggle') as HTMLButtonElement).click()
    await nextTick()

    const body = el.querySelector('.orch-run__reply-body')!
    expect(body.querySelectorAll('li')).toHaveLength(2)
    expect(body.textContent).not.toContain('- 要点一')
    app.unmount()
  })
})
