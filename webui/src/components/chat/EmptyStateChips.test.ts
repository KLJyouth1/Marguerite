// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, nextTick, ref } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import i18n from '@/i18n'
import zhHans from '@/locales/zh-Hans.json'
import EmptyStateChips from './EmptyStateChips.vue'
import { useFreshTaskDraft } from '@/composables/useFreshTaskDraft'

const mockRpc = vi.hoisted(() => {
  const calls: Array<{ method: string; params?: unknown }> = []
  const responses: Record<string, unknown> = {}
  return {
    calls,
    responses,
    state: 'connected',
    supportsMethod: (_method: string) => true,
    canChooseProject: true,
    canManageProjectWorkspaces: true,
    isLocalOwner: true,
    call: async (method: string, params?: unknown) => {
      calls.push({ method, params })
      return responses[method]
    },
  }
})

vi.mock('@/composables/useRpc', () => ({
  useRpcCall: () => ({
    data: ref(null),
    loading: ref(false),
    error: ref(null),
    execute: vi.fn(),
  }),
}))

vi.mock('@/stores/rpc', () => ({
  useRpcStore: () => mockRpc,
}))

async function mountChips(props: {
  suppressed?: boolean
  disabled?: boolean
  showIntro?: boolean
  onPick?: (text: string) => void
} = {}) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(EmptyStateChips, {
    agentId: 'main',
    ...props,
  })
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return { app, el }
}

beforeEach(() => {
  i18n.global.locale.value = 'en'
  document.body.innerHTML = ''
})

describe('EmptyStateChips', () => {
  it('hides task actions while suppressed', async () => {
    const { app, el } = await mountChips({
      suppressed: true,
    })

    expect(el.querySelector('.empty-state__chips')).toBeNull()
    expect(el.querySelector('.empty-state__greeting')).not.toBeNull()
    app.unmount()
  })

  it('showIntro=false 时让出标题位，只保留身份行与建议', async () => {
    const { app, el } = await mountChips({
      showIntro: false,
    })

    expect(el.querySelector('.empty-state__greeting')).toBeNull()
    expect(el.querySelector('.empty-state__lead')).toBeNull()
    expect(el.querySelector('.empty-state__chips')).not.toBeNull()
    app.unmount()
  })

  it('emits an ordinary suggestion when selected', async () => {
    const onPick = vi.fn()
    const { app, el } = await mountChips({
      onPick,
    })

    el.querySelector<HTMLButtonElement>('.empty-state__chip')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await nextTick()

    expect(onPick).toHaveBeenCalledTimes(1)
    app.unmount()
  })

  it('uses the game label while emitting the full voxel-racing prompt', async () => {
    i18n.global.setLocaleMessage('zh-Hans', zhHans)
    i18n.global.locale.value = 'zh-Hans'
    const onPick = vi.fn()
    // 主智能体已改为工程化入口，游戏示例只在其它的专家会话里出现，
    // 故此处显式挂到非 main 智能体上（用例意图不变：短标签 → 完整提示词）。
    const el = document.createElement('div')
    document.body.appendChild(el)
    const app = createApp(EmptyStateChips, { agentId: 'code-engineer', onPick })
    app.use(i18n)
    app.mount(el)
    await nextTick()
    const game = [...el.querySelectorAll<HTMLButtonElement>('.empty-state__chip')]
      .find(button => button.textContent?.trim() === '帮我做个游戏')

    expect(game).toBeTruthy()
    game?.click()
    await nextTick()

    expect(onPick).toHaveBeenCalledWith(
      '构建一个可玩的浏览器端体素竞速游戏原型，包含响应式操控、加速/刹车、可读赛道、碰撞检测、检查点、圈速计时和重启流程。用代码生成体素风格的赛车和场景，不使用付费素材。运行游戏、测试控制、修复阻塞性问题后交付。我希望风景更好,开放世界。且地图在有Marguerite和 tokenrhythm这两个标识 是赞助商彩蛋',
    )
    app.unmount()
  })

  it('keeps the suggestion row visible but inert while disabled', async () => {
    const onPick = vi.fn()
    const { app, el } = await mountChips({
      disabled: true,
      onPick,
    })

    const task = el.querySelector<HTMLButtonElement>('.empty-state__chip')
    expect(task?.disabled).toBe(true)
    expect(el.querySelector('.empty-state__chips')).not.toBeNull()
    expect(el.querySelector('.empty-state__meta')).toBeNull()

    task?.click()
    await nextTick()
    expect(onPick).not.toHaveBeenCalled()
    app.unmount()
  })

  // 主智能体 = 高级开发工程师「严工」：落地页必须给出工程化入口，
  // 而不是通用写作/游戏示例（人设与引导语不一致 = 专家包只接了一半）。
  it('offers engineering prompts for the main agent', async () => {
    i18n.global.setLocaleMessage('zh-Hans', zhHans)
    i18n.global.locale.value = 'zh-Hans'
    const onPick = vi.fn()
    const { app, el } = await mountChips({ onPick })

    const labels = [...el.querySelectorAll<HTMLButtonElement>('.empty-state__chip')]
      .map(button => button.textContent?.trim())
    expect(labels).toEqual([
      '系统性排查这个项目',
      '这个功能能跑但我不满意',
      '别闭门造车，先研究',
    ])

    // 点击必须发出**完整提示词**（不是按钮短标签），否则专家拿到的指令是残缺的。
    el.querySelector<HTMLButtonElement>('.empty-state__chip')?.click()
    await nextTick()
    expect(onPick).toHaveBeenCalledWith(
      '帮我系统性排查这个项目：先诊断根因，再修一处、扫全类，最后端到端验证到真正跑通。',
    )
    app.unmount()
  })

  // 非主智能体保持能力驱动（回归保护：别把通用入口一起换掉）。
  it('keeps capability chips for non-main agents', async () => {
    i18n.global.locale.value = 'en'
    const el = document.createElement('div')
    document.body.appendChild(el)
    const app = createApp(EmptyStateChips, { agentId: 'code-engineer' })
    app.use(i18n)
    app.mount(el)
    await nextTick()

    const labels = [...el.querySelectorAll<HTMLButtonElement>('.empty-state__chip')]
      .map(button => button.textContent?.trim())
    expect(labels).not.toContain('系统性排查这个项目')
    expect(labels.length).toBeGreaterThan(0)
    app.unmount()
  })
})

// W8.4: the landing page must be able to reopen a recent project without the
// folder dialog. Data/action reuse workspaces.list + workspaces.open (no new
// RPC); the switch itself rides the shared fresh-task signal ChatView already
// watches. Kept in its own describe because it is the only case that needs a
// Pinia context.
describe('EmptyStateChips recent workspaces', () => {
  async function settle() {
    for (let i = 0; i < 4; i++) {
      await nextTick()
      await new Promise(resolve => setTimeout(resolve, 0))
    }
  }

  it('lists recent workspaces and switches on click', async () => {
    i18n.global.locale.value = 'en'
    mockRpc.calls.length = 0
    const alpha = { id: 'ws-alpha', name: 'Alpha Studio', path: '/work/alpha', pinned: false, lastOpenedAt: '2026-10-26T10:00:00Z' }
    const beta = { id: 'ws-beta', name: 'Beta Lab', path: '/work/beta', pinned: false, lastOpenedAt: '2026-10-25T10:00:00Z' }
    mockRpc.responses['workspaces.list'] = { workspaces: [alpha, beta] }
    mockRpc.responses['workspaces.open'] = { workspace: beta }

    const pinia = createPinia()
    setActivePinia(pinia)
    const el = document.createElement('div')
    document.body.appendChild(el)
    const app = createApp(EmptyStateChips, { agentId: 'code-engineer' })
    app.use(pinia)
    app.use(i18n)
    app.mount(el)
    await settle()

    expect(el.querySelector('.empty-state__recent-label')?.textContent).toBe('Recently opened')
    const chips = [...el.querySelectorAll<HTMLButtonElement>('.empty-state__recent-chip')]
    expect(chips.map(chip => chip.textContent?.trim())).toEqual(['Alpha Studio', 'Beta Lab'])

    chips[1].click()
    await settle()

    expect(mockRpc.calls.some(call =>
      call.method === 'workspaces.open'
      && JSON.stringify(call.params) === JSON.stringify({ path: '/work/beta', trusted: true }),
    )).toBe(true)
    // Same fresh-task signal the palette uses; ChatView turns it into the draft
    // for this project, so the landing switches without any folder picker.
    expect(useFreshTaskDraft().request.value).toMatchObject({
      agentId: 'code-engineer',
      workspaceId: 'ws-beta',
    })

    app.unmount()
    setActivePinia(undefined)
  })
})
