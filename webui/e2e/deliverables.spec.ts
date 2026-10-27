import { test, expect, type Locator, type Page } from '@playwright/test'

const CONTROL_URL = '/control/'
const SESSION_KEY = 'agent:main:webchat:e2edeliverables'
const EMPTY_SESSION_KEY = 'agent:main:webchat:e2edeliverablesempty'
const INDEXED_SESSION_KEY = 'agent:main:webchat:e2edeliverablesindexed'

interface SeedHistoryOptions {
  indexedArtifacts?: Array<Record<string, unknown>>
}

// Seed a finished turn through the real WS pipeline: the page talks to the
// real gateway, but chat.history responses are rewritten in flight so a
// deliverable-bearing assistant turn renders without a live agent run.
async function seedHistory(
  page: Page,
  withArtifacts: boolean,
  options: SeedHistoryOptions = {},
) {
  await page.routeWebSocket(/\/ws$/, ws => {
    const server = ws.connectToServer()
    const historyIds = new Set<string>()
    ws.onMessage(message => {
      try {
        const frame = JSON.parse(String(message))
        if (frame?.type === 'req' && frame.method === 'chat.history') {
          historyIds.add(String(frame.id))
        }
        if (
          frame?.type === 'req'
          && frame.method === 'artifacts.list'
          && options.indexedArtifacts
        ) {
          ws.send(JSON.stringify({
            type: 'res',
            id: frame.id,
            ok: true,
            payload: {
              artifacts: options.indexedArtifacts,
              has_more: false,
              oldest_cursor: options.indexedArtifacts[0]?.id || null,
              newest_cursor: options.indexedArtifacts.at(-1)?.id || null,
            },
          }))
          return
        }
      } catch {}
      server.send(message)
    })
    server.onMessage(message => {
      try {
        const frame = JSON.parse(String(message))
        if (frame?.protocol !== undefined && options.indexedArtifacts) {
          const methods = Array.isArray(frame.features?.methods) ? frame.features.methods : []
          frame.features = {
            ...frame.features,
            methods: [...new Set([...methods, 'artifacts.list'])],
          }
        }
        if (frame?.type === 'res' && frame.id !== undefined && historyIds.has(String(frame.id))) {
          historyIds.delete(String(frame.id))
          frame.ok = true
          delete frame.error
          frame.payload = {
            messages: [
              {
                role: 'user',
                text: 'Save a couple of files for me.',
                id: 'msg-deliv-user',
                timestamp: Math.floor(Date.now() / 1000) - 120,
              },
              {
                role: 'assistant',
                text: withArtifacts ? 'Saved the files.' : 'Nothing to save on this turn.',
                id: 'msg-deliv-assistant',
                timestamp: Math.floor(Date.now() / 1000) - 60,
                artifacts: withArtifacts
                  ? [
                    { id: 'art-deliv-1', name: 'report.csv', mime: 'text/csv', size: 2048 },
                    { id: 'art-deliv-2', name: 'notes.txt', mime: 'text/plain', size: 512 },
                  ]
                  : [],
              },
            ],
            has_more: false,
          }
          ws.send(JSON.stringify(frame))
          return
        }
      } catch {}
      ws.send(message)
    })
  })
}

async function openSeededSession(
  page: Page,
  key: string,
  withArtifacts: boolean,
  options: SeedHistoryOptions = {},
) {
  await page.addInitScript(() => {
    window.localStorage.setItem('marguerite-locale', 'en')
  })
  await seedHistory(page, withArtifacts, options)
  await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(key))
  await page.waitForSelector('.conn-pill', { timeout: 10000 })
  await page.waitForSelector('.chat-header', { timeout: 10000 })
}

interface DeliverablesInvoker {
  trigger: Locator
  fromMenu: boolean
}

// 顶部栏右簇的 `--topbar-right-reserve` 首帧是 224px 回退值，挂载后由
// ResizeObserver 换成实测值（更大），于是 `.chat-header` 宽度会在首帧后收缩，
// 可能跨过 wide/compact 断点：宽版交付物按钮被紧凑版主按钮替换。若在宽度稳定
// 前快照触发器，Playwright 会拿到一个即将卸载的元素并重试到超时。
async function waitForHeaderSettled(page: Page): Promise<void> {
  const header = page.getByTestId('chat-header-actions')
  let previous = -1
  await expect.poll(async () => {
    const width = await header.evaluate(el => Math.round(el.getBoundingClientRect().width))
    const stable = width > 0 && width === previous
    previous = width
    return stable
  }, { timeout: 10000, intervals: [250, 250, 250, 250, 250, 250, 250, 250] }).toBe(true)
}

// ChatHeaderActions 会在交付物数据变化时关闭已打开的动作菜单，因此「过早打开
// 菜单」会与历史/索引水合赛跑：菜单里的交付物项会短暂出现又消失，若把它当成
// 触发器的快照，后续 locator 就会永久等待（曾让 --all 门禁在 Escape 用例超时）。
// 先等种子轮次渲染完（= 历史已注入），再轮询稳定触发面；菜单被水合关掉就重开，
// 每次都当场返回，不持有临时快照。
async function deliverablesTrigger(page: Page): Promise<DeliverablesInvoker> {
  const directAction = page.getByTestId('chat-session-action-deliverables')
  const primaryAction = page.getByTestId('chat-header-primary-action')
  const menuTrigger = page.getByTestId('chat-session-actions-trigger')

  await expect(page.locator('.msg-ai-main').last()).toBeVisible({ timeout: 10000 })
  await waitForHeaderSettled(page)

  const deadline = Date.now() + 10000
  while (Date.now() < deadline) {
    if (await directAction.isVisible()) return { trigger: directAction, fromMenu: false }
    if (await primaryAction.isVisible()
      && (await primaryAction.getAttribute('data-action')) === 'deliverables') {
      return { trigger: primaryAction, fromMenu: false }
    }
    if (await menuTrigger.isVisible()) {
      if ((await menuTrigger.getAttribute('aria-expanded')) !== 'true') {
        await menuTrigger.click()
      }
      if (await directAction.isVisible()) return { trigger: directAction, fromMenu: true }
    }
    await page.waitForTimeout(100)
  }
  throw new Error('deliverables trigger did not stabilise within 10s')
}

test.describe('Per-session deliverables drawer', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      const featureWindow = window as typeof window & {
        OPENSQUILLA_FEATURES?: Record<string, boolean>
      }
      featureWindow.OPENSQUILLA_FEATURES = {
        ...(featureWindow.OPENSQUILLA_FEATURES || {}),
        artifactWorkbench: false,
      }
    })
  })

  test('trigger is hidden when the session has no artifacts', async ({ page }) => {
    await openSeededSession(page, EMPTY_SESSION_KEY, false)
    await expect(page.locator('.msg-ai-main').last()).toBeVisible({ timeout: 10000 })
    await expect(page.getByTestId('chat-session-action-deliverables')).toHaveCount(0)
    await expect(page.locator('[data-testid="chat-header-primary-action"][data-action="deliverables"]'))
      .toHaveCount(0)
  })

  test('trigger opens the drawer with dialog a11y and lists every deliverable', async ({ page }) => {
    await openSeededSession(page, SESSION_KEY, true)

    const { trigger } = await deliverablesTrigger(page)
    await expect(trigger).toBeVisible({ timeout: 10000 })
    await expect(trigger).toHaveAccessibleName('Deliverables (2)')

    await trigger.click()

    const drawer = page.locator('.deliv-drawer')
    await expect(drawer).toBeVisible()
    await expect(drawer).toHaveAttribute('role', 'dialog')
    await expect(drawer).toHaveAttribute('aria-modal', 'true')
    await expect(drawer).toHaveAttribute('aria-label', /Deliverables \(2\)/)

    // Both deliverables render as tiles.
    await expect(page.locator('.deliv-tile')).toHaveCount(2)
    await expect(page.locator('.deliv-tile__name').first()).toHaveText('report.csv')
    // Tile meta uses the clean TYPE · size copy, not a doubled category.
    await expect(page.locator('.deliv-tile__meta').first()).toHaveText('CSV · 2 KB')

    // Focus moved into the drawer (close button is focused on open).
    const focusInside = await page.evaluate(() => {
      const drawerEl = document.querySelector('.deliv-drawer')
      return !!drawerEl && !!document.activeElement && drawerEl.contains(document.activeElement)
    })
    expect(focusInside).toBe(true)
  })

  test('Escape closes the drawer and returns focus to the trigger', async ({ page }) => {
    await openSeededSession(page, SESSION_KEY, true)

    const { trigger, fromMenu } = await deliverablesTrigger(page)
    const focusTarget = fromMenu
      ? page.getByTestId('chat-session-actions-trigger')
      : trigger
    await trigger.click()
    await expect(page.locator('.deliv-drawer')).toBeVisible()

    await page.keyboard.press('Escape')
    await expect(page.locator('.deliv-drawer')).toHaveCount(0)

    await expect(focusTarget).toBeFocused()
  })

  test('non-image deliverable opens a metadata preview with a download action', async ({ page }) => {
    await openSeededSession(page, SESSION_KEY, true)

    const { trigger } = await deliverablesTrigger(page)
    await trigger.click()
    await page.locator('.deliv-tile').first().click()

    const preview = page.locator('.deliv-preview')
    await expect(preview).toBeVisible()
    await expect(preview).toHaveAttribute('aria-modal', 'true')
    await expect(preview.locator('.deliv-preview__file')).toBeVisible()
    await expect(preview.getByRole('button', { name: 'Download' })).toBeVisible()

    // Escape backs out of the preview to the drawer, not all the way out.
    await page.keyboard.press('Escape')
    await expect(preview).toHaveCount(0)
    await expect(page.locator('.deliv-drawer')).toBeVisible()
  })

  test('mobile renders the drawer full-screen', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 })
    await openSeededSession(page, SESSION_KEY, true)

    const { trigger } = await deliverablesTrigger(page)
    await trigger.click()
    const drawer = page.locator('.deliv-drawer')
    await expect(drawer).toBeVisible()

    const width = await drawer.evaluate(el => el.getBoundingClientRect().width)
    expect(width).toBeGreaterThanOrEqual(375 - 1)
  })
})

test.describe('Indexed deliverables with the default Workbench', () => {
  test('unsupported indexed deliverables remain discoverable outside history', async ({ page }) => {
    await openSeededSession(page, INDEXED_SESSION_KEY, false, {
      indexedArtifacts: [{
        id: 'art-deliv-indexed',
        name: 'archived-report.csv',
        mime: 'text/csv',
        size: 4096,
        created_at: '2026-08-01T00:00:00Z',
        download_url: '/api/v1/artifacts/art-deliv-indexed',
      }],
    })

    // The default Workbench cannot preview CSV, and the current history page
    // has no artifact card. The index must still expose the complete Drawer.
    await expect(page.locator('.msg-artifact-chip')).toHaveCount(0)
    const { trigger } = await deliverablesTrigger(page)
    await expect(trigger).toHaveAccessibleName('Deliverables (1)')
    await trigger.click()

    const drawer = page.locator('.deliv-drawer')
    await expect(drawer).toBeVisible()
    await expect(drawer).toHaveAttribute('aria-label', /Deliverables \(1\)/)
    await expect(page.locator('.deliv-tile')).toHaveCount(1)
    await expect(page.locator('.deliv-tile__name')).toHaveText('archived-report.csv')
    await expect(page.locator('.deliv-tile__meta')).toHaveText('CSV · 4 KB')
    await page.locator('.deliv-tile').click()
    await expect(page.locator('.deliv-preview')).toBeVisible()
    // Scoped + exact: `name: 'Download'` is a substring match, so a real
    // sidebar project path like `D:\Downloads\…` also matches it.
    await expect(page.locator('.deliv-preview')
      .getByRole('button', { name: 'Download', exact: true })).toBeVisible()
  })
})
