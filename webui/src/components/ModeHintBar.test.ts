// @vitest-environment happy-dom
// Guards for the mode→UI mapping surface (upgrade-masterplan §6 阶段 D).
// v2.1 收敛：ModeHintBar 已改为只读能力提示（不再提供切换按钮），
// 切换统一走顶栏 ModeBadge。本测试验证：反映共享 activeMode + 只读提示文案。
// v2.2: ModeHintBar 现整合 useGatewayReadiness；测试 mock 为 ready 状态，
// 专注验证模式展示逻辑，不依赖 Pinia/RPC 基建。
// v2.3: 「切换入口在顶栏」这句此前是**硬编码中文**，英文界面下会漏出中文，
// 而 check-i18n.mjs 只做 locale 间的键集合互比、看不见源码硬编码，所以守卫不会报。
// 现在它走 modeHintBar.gotoTopbarBadge，下面的英文用例就是这条回归的守卫。
// v2.4（真机实测修正）：①bloom 的 hint 自带别名前缀 → 渲染成「深度打磨 · 深度打磨 · …」，
// 文案已去掉前缀，并加「别名只出现一次」的守卫；②那句「顶栏徽标可切换模式」**不再常驻正文**
// （每个会话页面都挂一句 11px 灰字属噪声），改为顶栏 ModeBadge 的 tooltip，守卫在
// ModeBadge.test.ts；本文件反向断言它不再出现在提示条里。
import { createApp, nextTick, ref, type App } from 'vue'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'

import i18n, { loadLocaleMessages } from '@/i18n'
import ModeHintBar from '@/components/ModeHintBar.vue'

// Control the shared store so we can drive `activeMode`.
const activeModeRef = ref<{ kind?: string; mode?: string; label?: string } | null>(null)

vi.mock('@/composables/useActiveMode', () => ({
  useActiveMode: () => ({ activeMode: activeModeRef, lastChangedFrom: ref(null) }),
  switchActiveMode: vi.fn(),
}))

// Mock useGatewayReadiness so the test doesn't need Pinia/RPC. Default to
// 'ready' so the mode metadata path is exercised; individual tests can
// override the readiness value if needed.
const readinessRef = ref<'ready' | 'not_ready' | 'connecting' | 'disconnected'>('ready')
vi.mock('@/composables/useGatewayReadiness', () => ({
  useGatewayReadiness: () => ({
    readiness: readinessRef,
    isReady: ref(true),
    status: ref(null),
    loading: ref(false),
    refresh: vi.fn(),
  }),
}))

let app: App<Element> | null = null

afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
  activeModeRef.value = null
  readinessRef.value = 'ready'
})

// Load zh-Hans messages once so locale switch produces Chinese strings.
beforeAll(async () => {
  await loadLocaleMessages('zh-Hans')
  await loadLocaleMessages('en')
})

async function mountBar(locale: 'en' | 'zh-Hans' = 'zh-Hans') {
  // ModeHintBar i18n keys live under zh-Hans; set locale explicitly so the
  // test assertions match regardless of environment defaults.
  i18n.global.locale.value = locale
  const el = document.createElement('div')
  document.body.appendChild(el)
  app = createApp(ModeHintBar)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return el
}

describe('ModeHintBar', () => {
  it('shows a placeholder when no mode is known', async () => {
    const el = await mountBar()
    expect(el.textContent).toContain('初始化中')
    expect(el.textContent).toContain('网关正在加载运行时配置')
  })

  it('renders the current mode alias and hint from the shared store (zh-Hans alias, D-03)', async () => {
    activeModeRef.value = { kind: 'bloom', mode: 'bloom', label: 'Bloom' }
    const el = await mountBar()
    // zh-Hans 显示中文别名（不再是硬编码英文 Bloom）。
    expect(el.textContent).toContain('深度打磨')
    expect(el.textContent).toContain('声明式思考深度与验证遍数')
    expect(el.textContent).not.toContain('Bloom')
  })

  it('never repeats the mode alias inside the hint', async () => {
    // 真机实测（2026-09-27）：bloom 的 hint 文案自带别名前缀，渲染成
    // 「深度打磨 · 深度打磨 · 声明式思考深度与验证遍数」。
    activeModeRef.value = { kind: 'bloom', mode: 'bloom' }
    const el = await mountBar()
    const label = el.querySelector('.mode-hint-bar__label')?.textContent ?? ''
    const hint = el.querySelector('.mode-hint-bar__hint')?.textContent ?? ''
    expect(label).toBe('深度打磨')
    expect(hint, '提示语里不得再重复别名').not.toContain(label)
    expect(el.textContent?.match(/深度打磨/g) ?? [], '别名只应出现一次').toHaveLength(1)
  })

  it('keeps the technical mode name in English (en locale)', async () => {
    activeModeRef.value = { kind: 'bloom', mode: 'bloom' }
    const el = await mountBar('en')
    expect(el.textContent).toContain('Bloom')
    expect(el.textContent).toContain('Declarative thinking depth')
  })

  it('is read-only and carries no permanent "switch from the badge" chrome', async () => {
    activeModeRef.value = { kind: 'agent', mode: 'agent' }
    const el = await mountBar()
    expect(el.querySelectorAll('.mode-hint-bar__chip')).toHaveLength(0)
    // 「在哪切」的说明已移进顶栏徽标的 tooltip（ModeBadge.test.ts 守卫它），
    // 常驻正文会让每个会话页面都挂一句 11px 灰字，属噪声。
    expect(el.textContent).not.toContain('顶栏徽标可切换模式')
  })

  it('stays visually quiet when ready, and only flags attention when setup is needed', async () => {
    // 收敛（2026-10-13）：正常运行时不再是一整行带底色的横幅，只有需要用户动作
    // （未就绪 / 未连接）时才恢复醒目底色。
    activeModeRef.value = { kind: 'agent', mode: 'agent' }
    let el = await mountBar()
    const bar = () => el.querySelector('.mode-hint-bar')
    expect(bar()?.classList.contains('mode-hint-bar--attention')).toBe(false)

    readinessRef.value = 'not_ready'
    el = await mountBar()
    expect(bar()?.classList.contains('mode-hint-bar--attention')).toBe(true)
  })

  it('does not leak Chinese copy into the English UI', async () => {
    activeModeRef.value = { kind: 'agent', mode: 'agent' }
    const el = await mountBar('en')
    expect(el.textContent).toContain('Agent')
    expect(el.textContent).toContain('Default mode')
    expect(el.textContent, 'English UI must not leak Chinese copy').not.toContain('日常协作')
    expect(el.textContent).not.toContain('顶栏徽标可切换模式')
  })
})
