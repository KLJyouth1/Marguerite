import { test, expect } from '@playwright/test'

const CONTROL_URL = '/control/'
// Rail contract (2026-09-25 re-baselined against the current dialog):
// - Backend-config sections advertise readiness **on the rail itself** as an
//   `aria-label` suffix ("Model Service: Optional") plus a tone dot on Connection.
//   The old standalone readiness banner + CLI-handoff disclosure were removed from
//   the dialog, so those assertions were replaced by this per-section surface.
// - Client-only sections carry their plain label (no status suffix).
// - Extension sections (plugin market, skin center, …) may append more entries,
//   so the rail length is asserted as a floor, never as a pinned count.
// (Runtime exists too, but it is desktop-only so the web rail hides it.)
const BACKEND_SECTIONS = ['Connection', 'Model Service', 'Model Routing', 'Capabilities', 'Behavior', 'Privacy']
const CLIENT_SECTIONS = ['Sandbox', 'Memory & Profile', 'Appearance', 'Keyboard', 'Advanced', 'Doctor', 'Plugins', 'Agent Presets']

const settingsRow = (page: import('@playwright/test').Page) =>
  page.locator('.sidebar-foot button')

const dialog = (page: import('@playwright/test').Page) =>
  page.getByRole('dialog', { name: 'Settings' })

const railTab = (page: import('@playwright/test').Page, name: string) =>
  dialog(page).getByRole('tab', { name: new RegExp(`^${name}:`) })

async function openFromSidebar(page: import('@playwright/test').Page) {
  await page.goto(CONTROL_URL)
  await page.waitForSelector('.conn-pill', { timeout: 10000 })
  await settingsRow(page).click()
  await expect(dialog(page)).toBeVisible()
}

test.describe('Settings modal', () => {
  // 本文件的所有用例共用**同一个网关配置**（脏栏保存会真的写 config），因此在
  // `--workers=4` 下并行执行同一个文件的两条用例时，两条 config.patch 会互相干扰
  // （实测：把本文件与另两个 spec 一起跑时，Save 后的脏栏不消失、rail 断言错位）。
  // 串行执行是这类「共享外部状态」测试的标准做法：单文件内顺序跑、仍可跨文件并行。
  test.describe.configure({ mode: 'serial' })

  test('opens from the sidebar with the curated section rail and per-section readiness', async ({ page }) => {
    await openFromSidebar(page)

    // Dialog a11y: focus moves into the modal on open, before any interaction.
    await expect(page.getByRole('button', { name: 'Close' })).toBeFocused()

    // Rail: backend sections (readiness advertised on the tab), client sections,
    // and any extension sections appended after them. ids must stay unique and the
    // curated set must all be present — but the total is a floor, not a magic number.
    const railItems = dialog(page).locator('.settings-rail__item')
    const ids = await railItems.evaluateAll((els) => els.map((el) => el.id))
    expect(new Set(ids).size, 'rail ids 必须唯一').toBe(ids.length)
    expect(ids.length).toBeGreaterThanOrEqual(BACKEND_SECTIONS.length + CLIENT_SECTIONS.length)
    for (const name of BACKEND_SECTIONS) {
      await expect(railTab(page, name), `${name} rail tab`).toBeVisible()
    }
    for (const name of CLIENT_SECTIONS) {
      await expect(dialog(page).getByRole('tab', { name, exact: true })).toBeVisible()
    }

    // The readiness surface is the rail status suffix (the standalone banner was
    // removed): every advertised status must be one of the real, known states.
    const labels = await dialog(page)
      .getByRole('tab')
      .evaluateAll((els) => els.map((el) => el.getAttribute('aria-label') || ''))
    const withStatus = labels.filter((l) => l.includes(': '))
    expect(withStatus.length).toBeGreaterThanOrEqual(BACKEND_SECTIONS.length)
    for (const label of withStatus) {
      expect(label, `未预期的 rail 状态后缀：${label}`).toMatch(
        /:\s(Connected|Connecting|Disconnected|Ready|Needs action|Provider first|Optional|Applies on save|Review|Missing)/,
      )
    }
    // Connection carries the live tone dot (ok / warn / muted) — honest socket state.
    await expect(dialog(page).locator('.settings-rail__dot')).toHaveClass(/is-(ok|warn|muted)/)
  })

  test('no YAML editor, raw key search, or guided-setup wording anywhere', async ({ page }) => {
    await openFromSidebar(page)

    await expect(dialog(page).getByRole('button', { name: 'YAML', exact: true })).toHaveCount(0)
    await expect(dialog(page).getByRole('button', { name: 'Form', exact: true })).toHaveCount(0)
    await expect(dialog(page).locator('#cfg-search')).toHaveCount(0)
    await expect(dialog(page).locator('textarea#cfg-yaml-area')).toHaveCount(0)
    await expect(dialog(page).getByText('Guided setup')).toHaveCount(0)

    // Footer keeps the config-file escape hatch with a copy affordance.
    // The path must point at the **config file** (Rust gateway: ~/.marguerite/config.yaml;
    // pyengine: *.toml) — a directory here would make both the disclosure and the copied
    // `--config <path>` command useless (that was a real Rust-side defect).
    const foot = dialog(page).locator('.settings-foot')
    await expect(foot).toContainText('More options live in')
    // Honest restart copy: most edits apply live, only some need a restart.
    await expect(foot).toContainText('Most changes apply live; some need a gateway restart')
    // The old blanket "always restart" leak is gone.
    await expect(foot).not.toContainText('Restart the gateway after manual edits')
    await expect(foot.locator('.settings-foot__path')).toContainText(/config\.(yaml|toml)$/)
    // 反向自检：正则必须真的能区分「文件」与「目录」
    expect(/config\.(yaml|toml)$/.test('C:\\Users\\me\\AppData\\Local\\Marguerite')).toBe(false)
    await foot.getByRole('button', { name: 'Copy config path' }).click()
    await expect(page.locator('.toast', { hasText: /Copied/ }).first()).toBeVisible()
  })

  test('rail switches sections, marks the active tab, and syncs the URL with replace', async ({ page }) => {
    await openFromSidebar(page)
    await expect(page).toHaveURL(/\/settings$/)

    await railTab(page, 'Capabilities').click()
    await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
    // Switching a section reflects in the URL so it stays deep-linkable.
    await expect(page).toHaveURL(/\/settings\/capabilities$/)
    // The Capabilities panel renders its tools as stacked sections (Web search,
    // Memory, Image, Audio) rather than a redundant "Capabilities" card heading.
    await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()

    await railTab(page, 'Model Routing').click()
    await expect(railTab(page, 'Model Routing')).toHaveAttribute('aria-selected', 'true')
    await expect(page).toHaveURL(/\/settings\/modelStrategy$/)
    // 该面板有两种**都合法**的渲染：有可用服务商时是策略卡 radiogroup，没有时是
    // 「先添加模型服务商」空状态（`data-testid=model-strategy-provider-first`）。
    // 隔离网关用的是空配置，所以必须接受后者 —— 旧断言无条件等 radiogroup，在
    // 「服务商数据尚未到达/本就没有」时 5s 后假红（实测两次全量门禁各红一次）。
    // 断言重点是「切到该节确实渲染了它自己的面板」：两种状态都不出现才算失败。
    const strategyCards = dialog(page).getByRole('radiogroup', { name: 'Choose how models are used' })
    const providerFirst = dialog(page).getByTestId('model-strategy-provider-first')
    await expect(strategyCards.or(providerFirst)).toBeVisible({ timeout: 10_000 })
    // 自证不是空匹配：两种状态**恰好**出现其一（面板没渲染时两者皆为 0，上一行会先失败）
    expect(
      (await strategyCards.count()) + (await providerFirst.count()),
      '面板必须渲染策略卡或「先添加服务商」空状态之一',
    ).toBe(1)

    // Section navigation uses replace, so a single Back exits Settings rather
    // than walking section history.
    await page.goBack()
    await expect(dialog(page)).toBeHidden()
    await expect(page).not.toHaveURL(/\/settings/)
  })

  test('opens Memory & Profile as a first-level action route without global save state', async ({ page }) => {
    await openFromSidebar(page)

    const memoryTab = dialog(page).getByRole('tab', { name: 'Memory & Profile', exact: true })
    await memoryTab.click()

    await expect(memoryTab).toHaveAttribute('aria-selected', 'true')
    await expect(page).toHaveURL(/\/settings\/memory$/)
    await expect(dialog(page).getByRole('heading', { name: 'Memory & Profile' })).toBeVisible()
    await expect(dialog(page).locator('[data-testid="settings-memory-panel"]')).toBeVisible()
    await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  })

  test('keeps the SettingsDialog mounted when routing from default settings to a section', async ({ page }) => {
    await openFromSidebar(page)
    await expect(page).toHaveURL(/\/settings$/)
    // 已加载的判据用 rail（旧的就绪横幅已移除；rail 与面板同属对话框骨架）
    await expect(dialog(page).locator('.settings-rail')).toBeVisible()
    await expect(dialog(page).locator('.settings-loading')).toHaveCount(0)

    await page.evaluate(() => {
      const modal = document.querySelector('.settings-modal')
      if (!modal) throw new Error('settings modal missing')
      const state = {
        modal,
        removed: false,
        loadingSeen: false,
        observer: null as MutationObserver | null,
      }
      state.observer = new MutationObserver(() => {
        state.removed = state.removed || !document.body.contains(state.modal)
        state.loadingSeen = state.loadingSeen || !!document.querySelector('.settings-loading')
      })
      state.observer.observe(document.body, { childList: true, subtree: true })
      ;(window as typeof window & { __settingsModalProbe?: typeof state }).__settingsModalProbe = state
    })

    await railTab(page, 'Capabilities').click()
    await expect(page).toHaveURL(/\/settings\/capabilities$/)
    await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
    await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
    // Wait past the route fade and Settings pop enter windows so delayed
    // remounts, re-entry animations, or loading fallbacks are caught.
    await page.waitForTimeout(350)

    const probe = await page.evaluate(() => {
      const state = (window as typeof window & {
        __settingsModalProbe?: {
          modal: Element
          removed: boolean
          loadingSeen: boolean
          observer: MutationObserver | null
        }
      }).__settingsModalProbe
      if (!state) throw new Error('settings modal probe missing')
      state.observer?.disconnect()
      return {
        sameNode: state.modal === document.querySelector('.settings-modal'),
        removed: state.removed,
        loadingSeen: state.loadingSeen,
      }
    })
    expect(probe.sameNode).toBe(true)
    expect(probe.removed).toBe(false)
    expect(probe.loadingSeen).toBe(false)
  })

  test('opens Settings without applying the app route fade to the overlay', async ({ page }) => {
    await page.goto(CONTROL_URL)
    await page.waitForSelector('.conn-pill', { timeout: 10000 })
    await page.evaluate(() => {
      const state = {
        overlaySeen: false,
        routeFadeSeen: false,
        observer: null as MutationObserver | null,
      }
      const inspect = () => {
        const overlay = document.querySelector('.settings-overlay')
        if (!overlay) return
        state.overlaySeen = true
        state.routeFadeSeen = state.routeFadeSeen || String(overlay.className).includes('route-fade-')
      }
      state.observer = new MutationObserver(inspect)
      state.observer.observe(document.body, {
        attributeFilter: ['class'],
        attributes: true,
        childList: true,
        subtree: true,
      })
      inspect()
      ;(window as typeof window & { __settingsOpenProbe?: typeof state }).__settingsOpenProbe = state
    })

    await settingsRow(page).click()
    await expect(dialog(page)).toBeVisible()
    await expect(dialog(page).locator('.settings-rail')).toBeVisible()
    await page.waitForTimeout(200)

    const probe = await page.evaluate(() => {
      const state = (window as typeof window & {
        __settingsOpenProbe?: {
          overlaySeen: boolean
          routeFadeSeen: boolean
          observer: MutationObserver | null
        }
      }).__settingsOpenProbe
      if (!state) throw new Error('settings open probe missing')
      state.observer?.disconnect()
      return {
        overlaySeen: state.overlaySeen,
        routeFadeSeen: state.routeFadeSeen,
      }
    })
    expect(probe.overlaySeen).toBe(true)
    expect(probe.routeFadeSeen).toBe(false)
  })

  test('Model Routing section edits routing config through the dirty bar', async ({ page }) => {
    test.setTimeout(90000)
    await openFromSidebar(page)

    // 旧版「路由面板样式」选择器（setup_model_strategy_router_visual_mode）已从面板移除
    // （面板单测明确断言其不存在）。当前可写的路由配置是策略单选（router/ensemble/single）。
    // ⚠️ 更正（R61）：这两项**并非**「无论有没有已存 provider 都可见」—— 没有服务商时
    // 面板渲染的是 `model-strategy-provider-first` 空状态（见下面两态分支）。
    // 档位/候选等写入项在**未配置 provider** 的网关上按设计不可用（面板会要求先选 provider）；
    // 有服务商时验证策略项的脏栏语义：改选 → 脏栏指名 Model Routing → Discard 还原
    // （跨网关写入往返由 Privacy 那条用例覆盖）。
    const dirtybar = dialog(page).locator('.settings-dirtybar')
    await railTab(page, 'Model Routing').click()

    // 与上面「rail switches sections」同款两态契约：有可用服务商时是策略卡 radiogroup，
    // 没有时是「先添加模型服务商」空状态。**两种状态都会真实出现在门禁里**：
    // 隔离网关是空配置，而侧车（Python 引擎）在线时 `config.get` 由侧车作答 → 面板走空状态；
    // 侧车离线（纯 Rust 模式）时由 Rust 配置作答 → 有默认服务商 → 走策略卡。
    // 旧写法无条件等 radiogroup，于是在「侧车在线」的那一跑必红（R61 实测：全量 e2e
    // 79 passed + 1 failed，本文件是 serial，1 例失败连带 15 例 skip）。
    const strategyGroup = dialog(page).getByRole('radiogroup', { name: 'Choose how models are used' })
    const providerFirst = dialog(page).getByTestId('model-strategy-provider-first')
    await expect(strategyGroup.or(providerFirst)).toBeVisible({ timeout: 10_000 })
    // 自证不是空匹配：两种状态**恰好**出现其一
    expect(
      (await strategyGroup.count()) + (await providerFirst.count()),
      '面板必须渲染策略卡或「先添加服务商」空状态之一',
    ).toBe(1)

    if (await providerFirst.isVisible()) {
      // 空状态分支：策略写入项按设计不存在，此处断言空状态**自己的**契约
      // （文案 + 可点击的去处 + 不出现半截策略输入），而不是跳过断言。
      await expect(providerFirst).toContainText('Add a model provider')
      await expect(providerFirst.getByRole('button')).toBeVisible()
      await expect(dialog(page).locator('input[name="setup_model_strategy"]')).toHaveCount(0)
      return
    }

    const strategies = dialog(page).locator('input[name="setup_model_strategy"]')
    const values = await strategies.evaluateAll((els) => els.map((e) => e.getAttribute('value') || ''))
    expect(values.length, '策略至少要有 router/ensemble/single 三选').toBeGreaterThanOrEqual(3)

    const initial = await strategies.evaluateAll((els) => els.find((e) => (e as HTMLInputElement).checked)?.getAttribute('value') || '')
    const next = values.find((v) => v && v !== initial)
    expect(next, '必须存在与当前不同的策略').toBeTruthy()
    if (!next) return

    // 卡片是包住 radio 的 <label>，radio 本身被视觉隐藏（opacity/clip），
    // 故按用户路径点卡片，再断言它真的选中了对应 radio（卡片↔输入接线是契约）。
    await dialog(page).locator(`[data-strategy-id="${next}"]`).click()
    await expect(
      dialog(page).locator(`input[name="setup_model_strategy"][value="${next}"]`),
    ).toBeChecked()
    await expect(dirtybar).toBeVisible()
    await expect(dirtybar).toContainText('Unsaved changes in Model Routing')

    await dirtybar.getByRole('button', { name: 'Discard routing changes' }).click()
    await expect(dirtybar).toBeHidden()
    // 丢弃必须真的回到原策略（否则「Discard」是假动作）
    await expect(
      dialog(page).locator(`input[name="setup_model_strategy"][value="${initial}"]`),
    ).toBeChecked()
  })

  test('Escape closes the overlay, leaves /settings, and never drops focus', async ({ page }) => {
    await openFromSidebar(page)
    await expect(page).toHaveURL(/\/settings$/)

    await page.keyboard.press('Escape')
    await expect(dialog(page)).toBeHidden()
    // The overlay is route-mounted: closing navigates back off /settings.
    await expect(page).not.toHaveURL(/\/settings/)

    // 真实不变量：关闭后焦点**不得丢失**（不落 body、不留在已卸载节点）。
    // 返回路由后应用会把焦点交给聊天输入区（桌面视口刻意如此：回到对话即可直接打字，
    // 见 ChatView 的 focusTextarea）；冷启动深链那条用例另行断言「交还侧栏设置按钮」。
    const focus = await page.evaluate(() => {
      const a = document.activeElement as HTMLElement | null
      return {
        connected: !!(a && a.isConnected),
        isBody: a === document.body,
        cls: a ? String(a.className) : '',
      }
    })
    expect(focus.connected, '焦点节点必须仍在文档中').toBe(true)
    expect(focus.isBody, '关闭后焦点不得掉在 body 上').toBe(false)

    // Escape inside the overlay must not collapse the docked sidebar.
    await expect(page.locator('.sidebar.docked')).toBeVisible()
  })

  test('closing Settings restores the previous route view without blanking the outlet', async ({ page }) => {
    await page.goto(CONTROL_URL + 'chat/new')
    await page.waitForSelector('.conn-pill', { timeout: 10000 })
    await expect(page.getByLabel('Message to send')).toBeVisible()

    await settingsRow(page).click()
    await expect(dialog(page)).toBeVisible()
    await page.getByRole('button', { name: 'Close' }).click()

    await expect(dialog(page)).toBeHidden()
    await expect(page).toHaveURL(/\/chat\/new$/)
    await expect(page.getByLabel('Message to send')).toBeVisible()
  })

  test('cold deep link to /settings/connection closes home and moves focus to the sidebar', async ({ page }) => {
    // Land directly on a section with no in-app invoker.
    await page.goto(CONTROL_URL + 'settings/connection')
    await page.waitForSelector('.conn-pill', { timeout: 10000 })
    await expect(dialog(page)).toBeVisible()
    await expect(railTab(page, 'Connection')).toHaveAttribute('aria-selected', 'true')
    // Connection renders even before/without a loaded config gate.
    await expect(dialog(page).getByRole('heading', { name: 'Connection' })).toBeVisible()

    await page.keyboard.press('Escape')
    await expect(dialog(page)).toBeHidden()
    await expect(page).not.toHaveURL(/\/settings/)
    // No detached focus: it lands on the sidebar Settings button.
    await expect(settingsRow(page)).toBeFocused()
  })

  test('/config deep link redirects into the settings overlay', async ({ page }) => {
    await page.goto(CONTROL_URL + 'config')
    await page.waitForSelector('.conn-pill', { timeout: 10000 })

    await expect(dialog(page)).toBeVisible()
    // /config now redirects to /settings (default section).
    await expect(page).toHaveURL(/\/settings$/)
    await expect(railTab(page, 'Model Service')).toHaveAttribute('aria-selected', 'true')
  })

  test('/setup deep link redirects into the overlay on the first not-ready section', async ({ page }) => {
    test.setTimeout(60000)
    await page.goto(CONTROL_URL + 'setup')
    await page.waitForSelector('.conn-pill', { timeout: 15000 })

    await expect(dialog(page)).toBeVisible()
    // /setup redirects to /settings/auto (first not-ready section).
    await expect(page).toHaveURL(/\/settings\/auto$/)

    // 恰好一个选中项；它必须**带就绪状态**（rail 就是当前的就绪面，横幅已移除）。
    const selected = dialog(page).getByRole('tab', { selected: true })
    await expect(selected).toHaveCount(1)
    const aria = (await selected.first().getAttribute('aria-label')) || ''
    expect(aria, `选中项必须带就绪状态后缀：${aria}`).toMatch(/:\s\S+/)

    // 状态语义：已就绪 → 落在 Model Service（配置入口）；需要处理 → 状态后缀必须是
    // 真实的「待处理」标签之一（Needs action / Provider first / Missing）。
    if (/:\s(Ready|Optional|Applies on save|Review)/.test(aria)) {
      await expect(railTab(page, 'Model Service')).toHaveAttribute('aria-selected', 'true')
    } else {
      expect(aria, `未就绪节的标签应为待处理态：${aria}`).toMatch(/:\s(Needs action|Provider first|Missing)/)
    }
  })

  test('dirty edits raise the bar, guard close, and Discard restores values', async ({ page }) => {
    await openFromSidebar(page)

    // 脏栏契约用「路由策略卡」（有服务商时）或 Privacy 开关（无服务商时）驱动，见下面两态分支。
    // 两条分支各用不同字段，serial 模式下也不会互相污染脏值。
    await railTab(page, 'Model Routing').click()
    const strategyGroup = dialog(page).getByRole('radiogroup', { name: 'Choose how models are used' })
    const providerFirst = dialog(page).getByTestId('model-strategy-provider-first')
    await expect(strategyGroup.or(providerFirst)).toBeVisible({ timeout: 10_000 })

    const dirtybar = dialog(page).locator('.settings-dirtybar')

    if (await providerFirst.isVisible()) {
      // 空配置网关（无已存服务商）下策略卡**按设计不存在**（与文件里其它两态用例同因），
      // 故改用 Privacy 开关驱动**同一条**脏栏契约：抬栏 → Escape 弹确认 → Cancel 保留 →
      // Discard 还原值。断言面不缩水，只是换了承载字段（本文件 serial，不会与其它用例互相污染）。
      await railTab(page, 'Privacy').click()
      const toggle = dialog(page).locator('input[name="setup_disable_network_observability"]')
      await expect(toggle).toBeVisible()
      const original = await toggle.isChecked()
      await toggle.setChecked(!original)
      await expect(dirtybar).toBeVisible()
      await expect(dirtybar).toContainText('Unsaved changes in Privacy')

      await page.keyboard.press('Escape')
      const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes?' })
      await expect(confirm).toBeVisible()
      await confirm.getByRole('button', { name: 'Cancel' }).click()
      await expect(confirm).toBeHidden()
      await expect(dialog(page)).toBeVisible()

      // Discard = 一次成功的 config 重载；重载失败时脏栏**故意**保留（绝不假装已还原），
      // 所以这里给足 20s：重载要跑好几个 RPC 往返，且 Python 引擎可能正在重启
      // （R61 修掉了「/health 被限流回 429 → 看门狗误杀引擎」这个根因，但重载本身仍是
      // 多步网络往返，5s 上限在负载下会假红）。
      await dirtybar.getByRole('button', { name: 'Discard', exact: true }).click()
      await expect(dirtybar).toBeHidden({ timeout: 20_000 })
      expect(await toggle.isChecked(), 'Discard 必须真的还原隐私开关').toBe(original)
      return
    }

    const radio = (v: string) => dialog(page).locator(`input[name="setup_model_strategy"][value="${v}"]`)
    const values = await dialog(page)
      .locator('input[name="setup_model_strategy"]')
      .evaluateAll((els) => els.map((e) => e.getAttribute('value') || ''))
    const original = await dialog(page)
      .locator('input[name="setup_model_strategy"]')
      .evaluateAll((els) => els.find((e) => (e as HTMLInputElement).checked)?.getAttribute('value') || '')
    const next = values.find((v) => v && v !== original)
    expect(next, '必须存在与当前不同的策略').toBeTruthy()
    if (!next) return
    await dialog(page).locator(`[data-strategy-id="${next}"]`).click()
    await expect(radio(next)).toBeChecked()

    await expect(dirtybar).toBeVisible()
    await expect(dirtybar).toContainText('Unsaved changes in Model Routing')

    // Closing with unsaved edits raises the themed confirm; declining keeps the modal.
    await page.keyboard.press('Escape')
    const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes?' })
    await expect(confirm).toBeVisible()
    await confirm.getByRole('button', { name: 'Cancel' }).click()
    await expect(confirm).toBeHidden()
    await expect(dialog(page)).toBeVisible()

    await dirtybar.getByRole('button', { name: 'Discard routing changes' }).click()
    await expect(dirtybar).toBeHidden()
    await expect(radio(original)).toBeChecked()
  })

  test('history Back with dirty edits raises the discard confirm instead of silently closing', async ({ page }) => {
    await openFromSidebar(page)

    await railTab(page, 'Privacy').click()
    await expect(page).toHaveURL(/\/settings\/privacy$/)
    // 与「live save round-trip」用例错开字段：并行 worker 下同字段会互相覆盖脏值。
    const capture = dialog(page).locator('input[name="setup_disable_network_observability"]')
    await expect(capture).toBeVisible()
    const original = await capture.isChecked()
    await capture.setChecked(!original)
    await expect(dialog(page).locator('.settings-dirtybar')).toBeVisible()

    // History traversal (browser Back, a trackpad back-swipe) unmounts the
    // route-mounted overlay without passing through requestClose, so the
    // router-level guard must raise the same confirm. Declining cancels the
    // navigation and restores the /settings URL.
    await page.goBack()
    const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes?' })
    await expect(confirm).toBeVisible()
    await confirm.getByRole('button', { name: 'Cancel' }).click()
    await expect(confirm).toBeHidden()
    await expect(dialog(page)).toBeVisible()
    await expect(page).toHaveURL(/\/settings\/privacy$/)
    expect(await capture.isChecked()).toBe(!original)

    // Accepting the discard lets the same Back proceed and close the overlay.
    // （确认框的主按钮当前文案是「Discard changes」，不再是旧的「Confirm」。）
    await page.goBack()
    await expect(confirm).toBeVisible()
    await confirm.getByRole('button', { name: 'Discard changes' }).click()
    await expect(dialog(page)).toBeHidden()
    await expect(page).not.toHaveURL(/\/settings/)
    // 丢弃后不得有残留编辑：重开对话框，值应回到原状
    await settingsRow(page).click()
    await expect(dialog(page)).toBeVisible()
    await railTab(page, 'Privacy').click()
    expect(await dialog(page).locator('input[name="setup_disable_network_observability"]').isChecked()).toBe(original)
  })

  test('viewport opts out of horizontal overscroll so a trackpad swipe cannot pop history', async ({ page }) => {
    await page.goto(CONTROL_URL)
    await page.waitForSelector('.conn-pill', { timeout: 10000 })
    // Chromium turns a two-finger horizontal overscroll into history navigation;
    // with Settings and chat sessions living on the history stack that gesture
    // would dismiss them wholesale. Both propagation roots must opt out.
    const behaviors = await page.evaluate(() => [
      getComputedStyle(document.documentElement).overscrollBehaviorX,
      getComputedStyle(document.body).overscrollBehaviorX,
    ])
    expect(behaviors).toEqual(['none', 'none'])
  })

  test('live save round-trip persists a harmless Privacy toggle', async ({ page }) => {
    test.setTimeout(90000)
    await openFromSidebar(page)

    // memory.auto_capture_enabled 现由 Privacy 节的开关承载，经脏栏保存写入配置（config.patch 路径）。
    const capture = () => dialog(page).locator('input[name="setup_memory_auto_capture"]')
    const dirtybar = () => dialog(page).locator('.settings-dirtybar')

    await railTab(page, 'Privacy').click()
    await expect(capture()).toBeVisible()
    const initial = await capture().isChecked()

    await capture().setChecked(!initial)
    await expect(dirtybar()).toBeVisible()
    await dirtybar().getByRole('button', { name: 'Save' }).click()
    await expect(dirtybar()).toBeHidden({ timeout: 15000 })

    // Reload: the persisted value must survive a fresh modal.
    await page.reload()
    await page.waitForSelector('.conn-pill', { timeout: 15000 })
    await expect(dialog(page)).toBeVisible()
    await railTab(page, 'Privacy').click()
    await expect(capture()).toBeVisible()
    expect(await capture().isChecked()).toBe(!initial)

    // Restore the original value.
    await capture().setChecked(initial)
    await dirtybar().getByRole('button', { name: 'Save' }).click()
    await expect(dirtybar()).toBeHidden({ timeout: 15000 })
    await page.reload()
    await page.waitForSelector('.conn-pill', { timeout: 15000 })
    await expect(dialog(page)).toBeVisible()
    await railTab(page, 'Privacy').click()
    await expect(capture()).toBeVisible()
    expect(await capture().isChecked()).toBe(initial)
  })

  test('mobile: full-screen dialog with horizontal section chips', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto(CONTROL_URL + 'config')
    await page.waitForSelector('.conn-pill', { timeout: 10000 })
    await expect(dialog(page)).toBeVisible()

    const modalBox = await dialog(page).boundingBox()
    expect(modalBox?.width).toBe(390)

    const rail = dialog(page).getByRole('tablist', { name: 'Settings sections' })
    await expect(rail).toHaveAttribute('aria-orientation', 'horizontal')
    await railTab(page, 'Capabilities').click()
    await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()

    // No horizontal scroll on the page at 390px.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
  })

  test('boolean settings render as the canonical control-switch and keep checkbox semantics', async ({ page }) => {
    await openFromSidebar(page)
    // 该开关（memory 自动捕获）如今在 Privacy 节；控件基元与语义断言保持不变。
    await railTab(page, 'Privacy').click()

    const capture = dialog(page).locator('input[name="setup_memory_auto_capture"]')
    await expect(capture).toBeVisible()

    // It carries the shared switch primitive, not a raw browser checkbox, and
    // is exposed to assistive tech as a switch.
    await expect(capture).toHaveClass(/control-switch/)
    await expect(capture).toHaveAttribute('role', 'switch')

    // The primitive CSS actually loaded and applied end-to-end (built bundle →
    // gateway → page): the checkbox is restyled (appearance:none) and sized as
    // a 36px switch track.
    const appearance = await capture.evaluate((el) => getComputedStyle(el).appearance)
    expect(appearance).toBe('none')
    const box = await capture.boundingBox()
    expect(box?.width).toBeGreaterThanOrEqual(34)
    expect(box?.width).toBeLessThanOrEqual(40)

    // Native checkbox semantics are preserved: it stays a real, keyboard-
    // operable input (Space toggles, :checked round-trips).
    const before = await capture.isChecked()
    await capture.focus()
    await page.keyboard.press('Space')
    expect(await capture.isChecked()).toBe(!before)
    await page.keyboard.press('Space')
    expect(await capture.isChecked()).toBe(before)
  })

  test('every boolean control in Capabilities uses the switch primitive (no raw checkboxes)', async ({ page }) => {
    await openFromSidebar(page)
    await railTab(page, 'Capabilities').click()
    await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()

    const checkboxes = dialog(page).locator('input[type="checkbox"]')
    const total = await checkboxes.count()
    expect(total).toBeGreaterThan(0)
    const switches = dialog(page).locator('input[type="checkbox"].control-switch')
    expect(await switches.count()).toBe(total)
  })

  test('Appearance section applies theme instantly without a dirty bar', async ({ page }) => {
    await openFromSidebar(page)

    const appearanceTab = dialog(page).getByRole('tab', { name: 'Appearance' })
    await expect(appearanceTab).toBeVisible()
    await appearanceTab.click()
    await expect(appearanceTab).toHaveAttribute('aria-selected', 'true')
    await expect(dialog(page).getByRole('heading', { name: 'Appearance' })).toBeVisible()

    // Theme radios flip the live document theme with no save step, and persist
    // to the same store the sidebar shortcut reads.
    const dark = dialog(page).getByRole('radio', { name: 'Dark' })
    const light = dialog(page).getByRole('radio', { name: 'Light' })
    await dark.click()
    await expect(dark).toBeChecked()
    await expect.poll(() => page.evaluate(() => document.documentElement.getAttribute('data-theme'))).toBe('dark')
    await light.click()
    await expect.poll(() => page.evaluate(() => document.documentElement.getAttribute('data-theme'))).toBe('light')
    expect(await page.evaluate(() => localStorage.getItem('marguerite-theme'))).toBe('light')

    // Visual effects are a low-frequency browser preference and live here
    // instead of adding another popover to the chat composer.
    const visualEffects = dialog(page).getByRole('switch', { name: 'Visual effects' })
    await expect(visualEffects).toBeVisible()
    await visualEffects.click()
    await expect(visualEffects).not.toBeChecked()
    expect(await page.evaluate(() => (
      JSON.parse(localStorage.getItem('marguerite.routerFx') || '{}').enabled
    ))).toBe(false)

    // Client-only: theme changes never raise the settings dirty bar.
    await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  })

  test('Appearance persists the tool-detail default without a save step', async ({ page }) => {
    await openFromSidebar(page)
    await dialog(page).getByRole('tab', { name: 'Appearance' }).click()

    const auto = dialog(page).getByRole('radio', { name: 'Auto', exact: true })
    const compact = dialog(page).getByRole('radio', { name: 'Compact', exact: true })
    await expect(auto).toBeChecked()

    await compact.click()
    await expect(compact).toBeChecked()
    await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
    expect(await page.evaluate(() => (
      localStorage.getItem('marguerite.appearance.toolDetails.v1')
    ))).toBe('compact')

    await page.reload()
    await page.waitForSelector('.conn-pill', { timeout: 10000 })
    await expect(dialog(page)).toBeVisible()
    await expect(dialog(page).getByRole('radio', { name: 'Compact', exact: true })).toBeChecked()

    await dialog(page).getByRole('radio', { name: 'Auto', exact: true }).click()
  })

  test('Advanced section surfaces homeless flags, applies instantly without a dirty bar', async ({ page }) => {
    await openFromSidebar(page)
    const advTab = dialog(page).getByRole('tab', { name: 'Advanced' })
    await expect(advTab).toBeVisible()
    await advTab.click()
    await expect(advTab).toHaveAttribute('aria-selected', 'true')
    await expect(dialog(page).getByRole('heading', { name: 'Advanced' })).toBeVisible()

    // A boolean Labs flag is the shared switch and writes its raw '1'/'0' key.
    const poll = dialog(page).locator('input[name="labs_approval_poll"]')
    await expect(poll).toHaveAttribute('role', 'switch')
    const before = await poll.isChecked()
    await poll.click()
    expect(await page.evaluate(() => localStorage.getItem('marguerite.chat.approvalPoll'))).toBe(before ? '0' : '1')

    // The answer-reveal window persists as a validated "min,max" string.
    const min = dialog(page).locator('input[name="labs_reveal_min"]')
    const max = dialog(page).locator('input[name="labs_reveal_max"]')
    await min.fill('1000')
    await max.fill('3000')
    await max.blur()
    expect(await page.evaluate(() => localStorage.getItem('marguerite.chat.answerReveal'))).toBe('1000,3000')

    // Client-only: none of this raises the settings dirty bar.
    await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()

    // Long-lived Agent management is available only from this advanced escape
    // hatch; routing through Vue preserves the /control base path.
    await dialog(page).getByRole('button', { name: 'Open: Agent configuration (advanced)' }).click()
    await expect(page).toHaveURL(/\/agents$/)
    await expect(dialog(page)).toHaveCount(0)
    await expect(page.locator('.agents-view, .agents-page, .ag-stage').first()).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Agents', level: 1 })).toBeFocused()
  })
})
