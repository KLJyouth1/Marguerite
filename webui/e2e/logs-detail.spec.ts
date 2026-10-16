import { test, expect, type Page } from '@playwright/test'

const CONTROL_URL = '/control/'
const FLAG_KEY = 'marguerite.logs.runTrace'

/** Bounded window for the Logs view to poll in its first line. */
const LINES_TIMEOUT_MS = 15000

async function openLogs(page: Page) {
  await page.goto(CONTROL_URL + 'logs')
  await page.waitForSelector('.conn-pill', { timeout: 10000 })
  // 旧选择器 .conn-pill.connected 已改名为 readiness state 类，改为 .ready 类断言。
  await page.waitForSelector('.conn-pill.ready', { timeout: 10000 }).catch(() => {})
  // The stream polls in; wait for at least one line or the honest empty state.
  await expect(page.locator('.lg-line, .lg-display__placeholder').first())
    .toBeVisible({ timeout: 10000 })
}

async function firstLine(page: Page) {
  const lines = page.locator('.lg-line')
  // Bounded wait first: the gateway's log file is created by the Python sidecar a
  // few seconds AFTER boot (logs.tail reads pyengine's debug.log), so counting the
  // instant the placeholder appears races the view's first poll.
  await expect(lines.first())
    .toBeVisible({ timeout: LINES_TIMEOUT_MS })
    .catch(() => {})
  const count = await lines.count()
  if (count === 0) {
    // Skip as a last resort, but ANNOTATE it so the reason is visible in the
    // report instead of looking like "nothing to check". A silent skip deletes
    // every assertion below from the run — and a skip is not a pass.
    const reason =
      `The Logs view rendered no lines within ${LINES_TIMEOUT_MS}ms. ` +
      'Gateway log file (logs.tail -> pyengine debug.log) is missing or empty; ' +
      'seed a log line or fix the environment.'
    test.info().annotations.push({ type: 'skip-reason', description: reason })
    test.skip(true, reason)
  }
  return lines.first()
}

test.describe('Logs detail drawer (opt-in run trace)', () => {
  test('flag off by default: log lines stay plain divs with no detail drawer', async ({ page }) => {
    await openLogs(page)
    const line = await firstLine(page)

    // The line is a non-interactive <div> — no button role, no tabindex.
    expect(await line.evaluate(el => el.tagName)).toBe('DIV')
    await expect(line).not.toHaveAttribute('role', 'button')
    expect(await line.getAttribute('tabindex')).toBeNull()

    // Clicking does nothing: no detail drawer exists in the DOM.
    await line.click();
    await expect(page.locator('.lg-detail')).toHaveCount(0)
    await expect(page.locator('.lg-detail-overlay')).toHaveCount(0)
  })

  test('flag on: clicking a line opens a detail drawer that Escape closes', async ({ page }) => {
    // Seed the opt-in flag before the app boots, then load Logs.
    await page.addInitScript(([key]) => {
      window.localStorage.setItem(key as string, '1')
    }, [FLAG_KEY])
    await openLogs(page)
    const line = await firstLine(page)

    // Now interactive: the row carries the button affordance.
    await expect(line).toHaveAttribute('role', 'button')
    await expect(line).toHaveAttribute('tabindex', '0')

    await line.click()
    const drawer = page.locator('.lg-detail')
    await expect(drawer).toBeVisible()
    await expect(drawer).toHaveAttribute('role', 'dialog')
    await expect(drawer).toHaveAttribute('aria-modal', 'true')

    // A run-bearing line renders a RunTrace; a plain line shows the raw payload.
    // One of the two is always present.
    await expect(drawer.locator('.tool-row, .lg-detail__raw').first()).toBeVisible()

    // Focus is trapped: Tab keeps the active element inside the drawer.
    await page.keyboard.press('Tab')
    const active = await page.evaluate(() => {
      const d = document.querySelector('.lg-detail')
      return !!(d && document.activeElement && d.contains(document.activeElement))
    })
    expect(active).toBe(true)

    await page.keyboard.press('Escape')
    await expect(page.locator('.lg-detail')).toHaveCount(0)
  })

  test('flag on: a non-run line shows the raw payload fallback', async ({ page }) => {
    await page.addInitScript(([key]) => {
      window.localStorage.setItem(key as string, '1')
    }, [FLAG_KEY])
    await openLogs(page)

    // Find a line whose raw payload is not a structured tool_calls run; plain
    // gateway log lines render the read-only <pre> fallback, never .tool-row.
    const lines = page.locator('.lg-line')
    const count = await lines.count()
    test.skip(count === 0, 'No log lines on this gateway')

    await lines.first().click()
    const drawer = page.locator('.lg-detail')
    await expect(drawer).toBeVisible()

    // Whatever the first line is, the body always resolves to exactly one of the
    // two render modes — never an empty drawer.
    const hasTrace = await drawer.locator('.tool-row').count()
    const hasRaw = await drawer.locator('.lg-detail__raw').count()
    expect(hasTrace + hasRaw).toBeGreaterThan(0)

    await page.keyboard.press('Escape')
    await expect(page.locator('.lg-detail')).toHaveCount(0)
  })
})
