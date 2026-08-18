# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: code-highlight.spec.ts >> Code block syntax highlighting >> idle chat renders no highlighted code blocks
- Location: e2e\code-highlight.spec.ts:7:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
  1  | import { test, expect } from '@playwright/test'
  2  | 
  3  | const CONTROL_URL = '/control/'
  4  | const LIVE = process.env.OPENSQUILLA_E2E_LIVE === '1'
  5  | 
  6  | test.describe('Code block syntax highlighting', () => {
  7  |   test('idle chat renders no highlighted code blocks', async ({ page }) => {
  8  |     await page.goto(CONTROL_URL + 'chat/new')
> 9  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
     |                ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
  10 | 
  11 |     await expect(page.locator('code.hljs')).toHaveCount(0)
  12 |     await expect(page.locator('.code-lang')).toHaveCount(0)
  13 |   })
  14 | 
  15 |   test('live python snippet renders distinct token colors and a language label', async ({ page }) => {
  16 |     test.skip(!LIVE, 'Live gateway test; set OPENSQUILLA_E2E_LIVE=1 to run.')
  17 |     test.setTimeout(240000)
  18 | 
  19 |     await page.goto(CONTROL_URL + 'chat/new')
  20 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  21 | 
  22 |     await page.locator('.chat-textarea').fill(
  23 |       'Reply with exactly one fenced python code block (```python) defining def greet(name): return f"hello {name}" — keep the def keyword and the string literal. No prose outside the block.',
  24 |     )
  25 |     await page.locator('.chat-send-btn[aria-label="Send"]').click()
  26 | 
  27 |     const block = page.locator('.msg-ai .msg-ai-text pre code.hljs').first()
  28 |     await expect(block).toBeVisible({ timeout: 180000 })
  29 |     await expect(page.locator('.stream-activity')).toHaveCount(0, { timeout: 180000 })
  30 | 
  31 |     // Highlighter token spans survive sanitization.
  32 |     const keyword = block.locator('.hljs-keyword').first()
  33 |     const literal = block.locator('.hljs-string').first()
  34 |     await expect(keyword).toBeVisible()
  35 |     await expect(literal).toBeVisible()
  36 | 
  37 |     // Two token kinds resolve to different computed colors, and both differ
  38 |     // from the block's base text color — the --syntax-* tokens applied.
  39 |     const keywordColor = await keyword.evaluate(node => getComputedStyle(node).color)
  40 |     const stringColor = await literal.evaluate(node => getComputedStyle(node).color)
  41 |     const baseColor = await block.evaluate(node => getComputedStyle(node).color)
  42 |     expect(keywordColor).not.toBe(stringColor)
  43 |     expect(keywordColor).not.toBe(baseColor)
  44 |     expect(stringColor).not.toBe(baseColor)
  45 | 
  46 |     // The language label chip renders on the block.
  47 |     const label = page.locator('.msg-ai .msg-ai-text pre .code-lang').first()
  48 |     await expect(label).toBeVisible()
  49 |     await expect(label).toHaveText(/python/i)
  50 | 
  51 |     // Highlighting survives a reload: history replays through the same renderer.
  52 |     await page.reload()
  53 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  54 |     const replayed = page.locator('.msg-ai .msg-ai-text pre code.hljs .hljs-keyword').first()
  55 |     await expect(replayed).toBeVisible({ timeout: 30000 })
  56 |   })
  57 | })
  58 | 
```