# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: empty-state.spec.ts >> Empty draft state >> chip click fills the composer without sending
- Location: e2e\empty-state.spec.ts:26:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
  1  | import { test, expect, type Page } from '@playwright/test'
  2  | 
  3  | const CONTROL_URL = '/control/'
  4  | 
  5  | async function openDraft(page: Page) {
  6  |   await page.goto(CONTROL_URL + 'chat/new')
> 7  |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
     |              ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
  8  |   await page.waitForSelector('.conn-pill.connected', { timeout: 10000 }).catch(() => {})
  9  | }
  10 | 
  11 | test.describe('Empty draft state', () => {
  12 |   test('draft greets with the identity line and at least 3 chips, no brand mark', async ({ page }) => {
  13 |     await openDraft(page)
  14 | 
  15 |     await expect(page.locator('.empty-state__greeting')).toHaveText(/Good (morning|afternoon|evening)\./)
  16 |     await expect(page.locator('.empty-state__identity')).toContainText('· ready')
  17 | 
  18 |     // Brand lives in the sidebar chrome only: no wordmark, no mark in the content area.
  19 |     await expect(page.locator('.chat-landing-lockup')).toHaveCount(0)
  20 |     await expect(page.locator('.empty-state__mark')).toHaveCount(0)
  21 |     await expect(page.locator('.msg-ai-avatar')).toHaveCount(0)
  22 | 
  23 |     expect(await page.locator('.empty-state__chip').count()).toBeGreaterThanOrEqual(3)
  24 |   })
  25 | 
  26 |   test('chip click fills the composer without sending', async ({ page }) => {
  27 |     await openDraft(page)
  28 | 
  29 |     const chip = page.locator('.empty-state__chip').first()
  30 |     const text = (await chip.innerText()).trim()
  31 |     await chip.click()
  32 | 
  33 |     await expect(page.locator('.chat-textarea')).toHaveValue(text)
  34 |     await expect(page.locator('.chat-textarea')).toBeFocused()
  35 | 
  36 |     // Draft only: nothing was sent and no session materialized.
  37 |     await expect(page.locator('.msg-user, .msg-ai')).toHaveCount(0)
  38 |     await expect(page).toHaveURL(/\/chat\/new/)
  39 |     expect(page.url()).not.toContain('session=')
  40 |   })
  41 | 
  42 |   test('renders and fills at 390px', async ({ page }) => {
  43 |     await page.setViewportSize({ width: 390, height: 844 })
  44 |     await openDraft(page)
  45 | 
  46 |     await expect(page.locator('.empty-state__greeting')).toBeVisible()
  47 |     expect(await page.locator('.empty-state__chip').count()).toBeGreaterThanOrEqual(3)
  48 | 
  49 |     const chip = page.locator('.empty-state__chip').first()
  50 |     const text = (await chip.innerText()).trim()
  51 |     await chip.click()
  52 | 
  53 |     await expect(page.locator('.chat-textarea')).toHaveValue(text)
  54 |     await expect(page.locator('.msg-user, .msg-ai')).toHaveCount(0)
  55 |   })
  56 | 
  57 |   test('Sessions hand-off sends in one step and leaves the landing', async ({ page }) => {
  58 |     // The Sessions Hub "Start task" button hands the draft off with autosend,
  59 |     // so the composer fires immediately instead of parking on the empty state.
  60 |     await page.goto(CONTROL_URL + 'sessions')
  61 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  62 | 
  63 |     const taskText = 'Summarize the latest gateway logs'
  64 |     await page.locator('.hub-task__input').fill(taskText)
  65 |     await page.getByRole('button', { name: 'Start task' }).click()
  66 | 
  67 |     await expect(page).toHaveURL(/\/chat/)
  68 |     // The send fired: the user bubble is present and the landing is gone.
  69 |     await expect(
  70 |       page.locator('.msg-user').filter({ hasText: taskText }),
  71 |     ).toHaveCount(1, { timeout: 15000 })
  72 |     await expect(page.locator('.empty-state__greeting')).toHaveCount(0)
  73 |   })
  74 | })
  75 | 
```