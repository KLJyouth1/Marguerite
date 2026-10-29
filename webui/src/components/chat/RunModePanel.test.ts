// @vitest-environment happy-dom
import { createApp, nextTick } from 'vue'
import { describe, expect, it } from 'vitest'
import i18n from '@/i18n'
import RunModePanel from './RunModePanel.vue'

interface PanelProps {
  runMode?: 'safe' | 'full'
  allowedRunModes?: Array<'safe' | 'full'>
  safeSetupAvailable?: boolean
  runModeLocked?: boolean
  runModeLockMessage?: string
  collaborationMode?: 'plan' | 'default'
  planModeAvailable?: boolean
  planModeBusy?: boolean
  planModeDisabled?: boolean
  modelRoutingMode?: string
  modelRoutingSettingsBusy?: boolean
  showNewBadge?: boolean
}

function mountPanel(props: PanelProps = {}) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const emitted = {
    runMode: [] as string[],
    collaborationMode: [] as string[],
    modelRoutingMode: [] as string[],
  }
  const app = createApp(RunModePanel, {
    runMode: props.runMode ?? 'safe',
    allowedRunModes: props.allowedRunModes ?? ['safe', 'full'],
    safeSetupAvailable: props.safeSetupAvailable ?? false,
    runModeLocked: props.runModeLocked ?? false,
    runModeLockMessage: props.runModeLockMessage ?? '',
    collaborationMode: props.collaborationMode ?? 'default',
    planModeAvailable: props.planModeAvailable ?? true,
    planModeBusy: props.planModeBusy ?? false,
    planModeDisabled: props.planModeDisabled ?? false,
    modelRoutingMode: props.modelRoutingMode ?? 'off',
    modelRoutingSettingsBusy: props.modelRoutingSettingsBusy ?? false,
    showNewBadge: props.showNewBadge ?? false,
    onSetRunMode: (m: string) => emitted.runMode.push(m),
    onSetCollaborationMode: (m: string) => emitted.collaborationMode.push(m),
    onSetModelRoutingMode: (m: string) => emitted.modelRoutingMode.push(m),
  })
  app.use(i18n)
  app.mount(el)
  return { app, el, emitted }
}

function chip(el: HTMLElement): HTMLButtonElement {
  const btn = el.querySelector<HTMLButtonElement>('button[aria-label="Run mode & routing"]')
  expect(btn).toBeTruthy()
  return btn!
}

describe('RunModePanel', () => {
  it('shows a summary chip reflecting the three dimensions', async () => {
    i18n.global.locale.value = 'en'
    const { app, el } = mountPanel({
      runMode: 'safe',
      collaborationMode: 'default',
      modelRoutingMode: 'omni_agent',
    })
    const label = chip(el).querySelector('.run-mode-panel__chip-label')?.textContent
    expect(label).toBe('Safe·Default·Omni')
    app.unmount()
    el.remove()
  })

  it('opens the unified popover with all three dimensions', async () => {
    i18n.global.locale.value = 'en'
    const { app, el } = mountPanel()
    expect(el.querySelector('.run-mode-panel__popover')).toBeNull()

    chip(el).click()
    await nextTick()

    expect(el.querySelector('.run-mode-panel__popover')).toBeTruthy()
    expect(el.querySelectorAll('.composer-run-mode__option')).toHaveLength(2)
    expect(el.querySelectorAll('.composer-model-routing__option')).toHaveLength(5)
    expect(el.querySelectorAll('.run-mode-panel__plan-option')).toHaveLength(2)
    app.unmount()
    el.remove()
  })

  it('emits setRunMode and closes after selecting a sandbox option', async () => {
    i18n.global.locale.value = 'en'
    const { app, el, emitted } = mountPanel({ runMode: 'safe' })
    chip(el).click()
    await nextTick()

    const full = Array.from(el.querySelectorAll<HTMLButtonElement>('.composer-run-mode__option'))
      .find(o => o.textContent?.includes('Full'))
    full?.click()
    await nextTick()

    expect(emitted.runMode).toEqual(['full'])
    expect(el.querySelector('.run-mode-panel__popover')).toBeNull()
    app.unmount()
    el.remove()
  })

  it('emits setModelRoutingMode and closes', async () => {
    i18n.global.locale.value = 'en'
    const { app, el, emitted } = mountPanel({ modelRoutingMode: 'off' })
    chip(el).click()
    await nextTick()

    const omni = Array.from(el.querySelectorAll<HTMLButtonElement>('.composer-model-routing__option'))
      .find(o => o.classList.contains('composer-model-routing__option--omni_agent'))
    omni?.click()
    await nextTick()

    expect(emitted.modelRoutingMode).toEqual(['omni_agent'])
    expect(el.querySelector('.run-mode-panel__popover')).toBeNull()
    app.unmount()
    el.remove()
  })

  it('emits setCollaborationMode and closes when choosing a plan', async () => {
    i18n.global.locale.value = 'en'
    const { app, el, emitted } = mountPanel({ collaborationMode: 'default' })
    chip(el).click()
    await nextTick()

    const plan = Array.from(el.querySelectorAll<HTMLButtonElement>('.run-mode-panel__plan-option'))
      .find(o => o.textContent?.includes('Plan'))
    plan?.click()
    await nextTick()

    expect(emitted.collaborationMode).toEqual(['plan'])
    expect(el.querySelector('.run-mode-panel__popover')).toBeNull()
    app.unmount()
    el.remove()
  })

  it('disables sandbox options (but not the chip) while the run mode is locked', async () => {
    i18n.global.locale.value = 'en'
    const { app, el } = mountPanel({
      runModeLocked: true,
      runModeLockMessage: 'locked',
    })
    const c = chip(el)
    expect(c.disabled).toBe(false)
    expect(c.getAttribute('aria-describedby')).toBe('run-mode-panel-lock-tip')

    c.click()
    await nextTick()
    expect(el.querySelector('.run-mode-panel__lock-note')?.textContent?.trim()).toBe('locked')
    expect(el.querySelector<HTMLButtonElement>('.composer-run-mode__option')?.disabled).toBe(true)
    app.unmount()
    el.remove()
  })

  it('closes the popover on outside pointerdown', async () => {
    i18n.global.locale.value = 'en'
    const { app, el } = mountPanel()
    chip(el).click()
    await nextTick()
    expect(el.querySelector('.run-mode-panel__popover')).toBeTruthy()

    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true, composed: true }))
    await nextTick()
    expect(el.querySelector('.run-mode-panel__popover')).toBeNull()
    app.unmount()
    el.remove()
  })

  // 回归（2026-10-24）：「运行模式与路由」最后一段内容缺失。
  // 两个子面板原本各自 `position: absolute`，塞进父面板的滚动盒后会再弹一层、
  // 逃出 max-height。契约：它们必须以内联形态渲染在 popover 内，且不再自带标题。
  it('renders both sub-panels inline inside the scrollable popover', async () => {
    i18n.global.locale.value = 'en'
    const { app, el } = mountPanel()
    chip(el).click()
    await nextTick()

    const popover = el.querySelector('.run-mode-panel__popover')!
    const runMode = popover.querySelector('.composer-run-mode')!
    const routing = popover.querySelector('.composer-model-routing')!

    expect(runMode.classList.contains('is-inline')).toBe(true)
    expect(routing.classList.contains('is-inline')).toBe(true)
    // 内联形态不再重复渲染自己的标题/关闭按钮（父面板已有段标题）
    expect(runMode.querySelector('.composer-run-mode__head')).toBeNull()
    expect(routing.querySelector('.composer-model-routing__head')).toBeNull()
    // 四段都在同一个滚动容器里，顺序为 执行模式 → PLAN → 审批旁路 → 模型路由
    const sections = Array.from(popover.querySelectorAll('.run-mode-panel__section'))
    expect(sections).toHaveLength(4)
    expect(sections[3].querySelectorAll('.composer-model-routing__option')).toHaveLength(5)
    app.unmount()
    el.remove()
  })
})
