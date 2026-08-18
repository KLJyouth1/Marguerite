# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: channels-workspace-edit.spec.ts >> channel workspace compose + edit >> compose a telegram channel from the takeover
- Location: e2e\channels-workspace-edit.spec.ts:20:3

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: locator.click: Test timeout of 30000ms exceeded.
Call log:
  - waiting for locator('.chc').locator('button[data-channel-type="telegram"]')

```

# Page snapshot

```yaml
- generic [ref=e2]:
  - navigation "Primary" [ref=e3]:
    - generic [ref=e4]:
      - generic [ref=e5]:
        - img [ref=e6]
        - generic [ref=e7]: Marguerite
      - button "Collapse sidebar" [expanded] [ref=e8] [cursor=pointer]:
        - img [ref=e10]
    - navigation "Control navigation" [ref=e12]:
      - button "New task" [ref=e13] [cursor=pointer]:
        - img [ref=e15]
        - generic [ref=e16]: New task
      - link "Skills & Channels" [ref=e17] [cursor=pointer]:
        - /url: /control/skills
        - img [ref=e19]
        - generic [ref=e21]: Skills & Channels
      - link "Cron" [ref=e22] [cursor=pointer]:
        - /url: /control/cron
        - img [ref=e24]
        - generic [ref=e27]: Cron
      - link "View usage" [ref=e28] [cursor=pointer]:
        - /url: /control/usage
        - img [ref=e30]
        - generic [ref=e31]: View usage
    - generic "Recent tasks" [ref=e32]:
      - generic [ref=e33]:
        - generic [ref=e34]: Recents
        - button "Search tasks (Ctrl+K)" [ref=e35] [cursor=pointer]:
          - img [ref=e37]
      - generic [ref=e40]:
        - paragraph [ref=e41]: Unable to load tasks
        - button "Reload tasks" [ref=e42] [cursor=pointer]
    - button "Settings" [ref=e44] [cursor=pointer]:
      - img [ref=e46]
      - generic [ref=e49]: Settings
  - separator "Resize sidebar" [ref=e50]:
    - status [ref=e51]
  - generic [ref=e52]:
    - banner:
      - generic [ref=e53]:
        - button "Manage gateway connection" [ref=e54] [cursor=pointer]: Disconnected
        - button "Language" [ref=e56] [cursor=pointer]:
          - img [ref=e58]
          - generic [ref=e61]: English
        - button "Theme" [ref=e63] [cursor=pointer]:
          - img [ref=e65]
    - main [ref=e68]:
      - generic [ref=e69]:
        - navigation "Skills & Channels" [ref=e71]:
          - link "Skills" [ref=e72] [cursor=pointer]:
            - /url: /control/skills
            - img [ref=e74]
            - generic [ref=e76]: Skills
          - link "Channels" [ref=e77] [cursor=pointer]:
            - /url: /control/channels
            - img [ref=e79]
            - generic [ref=e82]: Channels
        - generic [ref=e84]:
          - button "Refresh" [disabled] [ref=e87]:
            - img [ref=e89]
            - generic [ref=e92]: Refresh
          - status "Loading" [ref=e94]
          - dialog "Add channel" [ref=e95]:
            - generic [active] [ref=e97]:
              - generic [ref=e98]:
                - button "Channels" [ref=e99] [cursor=pointer]:
                  - generic [ref=e100]: ‹
                  - generic [ref=e101]: Channels
                - heading "Choose a platform" [level=2] [ref=e102]
              - group "Choose a platform" [ref=e104]
  - status [ref=e114]
```

# Test source

```ts
  1   | import { expect, test, type Page } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | const NAME = 'e2e-edit-telegram'
  5   | 
  6   | // End-to-end: the /channels workspace owns channel setup. Covers the compose
  7   | // takeover (?compose=1), the legacy settings-hash redirect, in-place edit via
  8   | // ?channel=<name>&tab=configuration&edit=1, the no-'***' invariant, the
  9   | // stored-secret keep-current save merge, and cleanup.
  10  | //
  11  | // NOTE: updated for the workspace flow by inspection; like every spec here it
  12  | // needs a live gateway (`opensquilla gateway run`) to execute.
  13  | 
  14  | async function openChannels(page: Page, suffix = ''): Promise<void> {
  15  |   await page.goto(`${CONTROL_URL}channels${suffix}`)
  16  |   await expect(page.locator('.conn-pill')).toBeVisible({ timeout: 15000 })
  17  | }
  18  | 
  19  | test.describe.serial('channel workspace compose + edit', () => {
  20  |   test('compose a telegram channel from the takeover', async ({ page }) => {
  21  |     await openChannels(page, '?compose=1')
  22  |     const surface = page.locator('.chc')
  23  |     await expect(surface).toBeVisible({ timeout: 10000 })
  24  | 
  25  |     // Pick from the type gallery; the gallery collapses to a receipt chip
  26  |     // and the shared config editor grows beneath it in compose mode.
> 27  |     await surface.locator('button[data-channel-type="telegram"]').click()
      |                                                                   ^ Error: locator.click: Test timeout of 30000ms exceeded.
  28  |     await expect(surface.locator('.chc__chipname')).toHaveText('Telegram')
  29  |     await surface.locator('input[name="setup_channel_name"]').fill(NAME)
  30  |     await surface.locator('input[name="setup_channel_token"]').fill('0000:e2e-dummy-token')
  31  |     await surface.getByRole('button', { name: 'Save Channel' }).click()
  32  | 
  33  |     // Save dismisses the takeover and selects the new channel's page.
  34  |     await expect(surface).toHaveCount(0, { timeout: 15000 })
  35  |     await expect(page).toHaveURL(new RegExp(`channel=${NAME}`), { timeout: 15000 })
  36  |     await expect(page.locator('.chd h2')).toHaveText(NAME)
  37  |   })
  38  | 
  39  |   test('legacy settings hash redirects into the in-place editor', async ({ page }) => {
  40  |     await page.goto(`${CONTROL_URL}settings/channels#channel-${NAME}`)
  41  |     await expect(page).toHaveURL(new RegExp(`channels\\?.*channel=${NAME}`), { timeout: 15000 })
  42  |     await expect(page).toHaveURL(/tab=configuration/)
  43  |     await expect(page).toHaveURL(/edit=1/)
  44  |     await expect(page.locator('.cfge')).toBeVisible({ timeout: 15000 })
  45  |   })
  46  | 
  47  |   test('legacy bare and query settings paths redirect to the workspace', async ({ page }) => {
  48  |     await page.goto(`${CONTROL_URL}settings/channels`)
  49  |     await expect(page).toHaveURL(/\/channels$/, { timeout: 15000 })
  50  | 
  51  |     await page.goto(`${CONTROL_URL}settings/channels?compose=1&type=telegram`)
  52  |     await expect(page).toHaveURL(/\/channels\?compose=1&type=telegram$/, { timeout: 15000 })
  53  |     await expect(page.locator('.chc')).toBeVisible({ timeout: 15000 })
  54  |   })
  55  | 
  56  |   test('edit in place: masked secret, draft test, keep-current save', async ({ page }) => {
  57  |     await openChannels(page, `?channel=${NAME}&tab=configuration&edit=1`)
  58  |     const editor = page.locator('.cfge')
  59  |     await expect(editor).toBeVisible({ timeout: 15000 })
  60  | 
  61  |     // Name is locked text (not an input); the stored secret renders as a
  62  |     // masked row with Replace; no input anywhere holds the '***' sentinel.
  63  |     await expect(editor.locator('[data-field="name"] input')).toHaveCount(0)
  64  |     await expect(editor.locator('[data-field="name"] .cfge__value--locked')).toBeVisible()
  65  |     const tokenRow = editor.locator('[data-field="token"]')
  66  |     await expect(tokenRow).toContainText('Stored')
  67  |     await expect(tokenRow.getByRole('button', { name: 'Replace' })).toBeVisible()
  68  |     for (const value of await editor.locator('input').evaluateAll(
  69  |       els => els.map(el => (el as HTMLInputElement).value),
  70  |     )) {
  71  |       expect(value).not.toBe('***')
  72  |     }
  73  | 
  74  |     // Dirty the draft: the sticky bar names the changed field and offers
  75  |     // Test/Discard/Save. The draft probe merges blank secrets against the
  76  |     // stored entry server-side — no retyping. default_chat_id folded into the
  77  |     // Advanced disclosure, so expand it first.
  78  |     await editor.locator('.cfge__advanced > summary').click()
  79  |     await editor.locator('input[name="setup_channel_default_chat_id"]').fill('4242')
  80  |     const bar = page.locator('.ceb')
  81  |     await expect(bar).toBeVisible()
  82  |     await expect(bar).toContainText('Default chat id')
  83  |     await bar.getByRole('button', { name: 'Test connection' }).click()
  84  |     await expect(editor.locator('.cfge__transcript-row')).toBeVisible({ timeout: 20000 })
  85  | 
  86  |     // Save: probe → upsert, baseline reset, back to read mode; the secret
  87  |     // stays masked and the change persisted (expand Advanced to read it).
  88  |     await bar.getByRole('button', { name: 'Save changes' }).click()
  89  |     await expect(bar).toHaveCount(0, { timeout: 15000 })
  90  |     await editor.locator('.cfge__advanced > summary').click()
  91  |     await expect(editor.locator('[data-field="default_chat_id"]')).toContainText('4242', { timeout: 15000 })
  92  |     await expect(editor.locator('[data-field="token"]')).toContainText('Stored')
  93  |     await expect(page).not.toHaveURL(/edit=1/)
  94  |   })
  95  | 
  96  |   test('cleanup: remove the e2e channel', async ({ page }) => {
  97  |     await openChannels(page, `?channel=${NAME}`)
  98  |     const detail = page.locator('.chd')
  99  |     if (await detail.count()) {
  100 |       await detail.locator('.chd__remove').click()
  101 |       await page
  102 |         .getByRole('dialog', { name: 'Remove this channel?' })
  103 |         .getByRole('button', { name: 'Remove' })
  104 |         .click()
  105 |       await expect(page.locator('.chb-ledger').getByText(NAME)).toHaveCount(0, { timeout: 15000 })
  106 |     }
  107 |   })
  108 | })
  109 | 
```