# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: desktop-cli-invocation.spec.ts >> desktop shell CLI invocation >> web keeps flat, unprefixed commands
- Location: e2e\desktop-cli-invocation.spec.ts:75:3

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: page.waitForSelector: Test timeout of 30000ms exceeded.
Call log:
  - waiting for locator('.health-finding') to be visible

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
        - generic [ref=e70]:
          - navigation "View usage" [ref=e71]:
            - link "Usage" [ref=e72] [cursor=pointer]:
              - /url: /control/usage
              - img [ref=e74]
              - generic [ref=e75]: Usage
            - link "Overview" [ref=e76] [cursor=pointer]:
              - /url: /control/overview
              - img [ref=e78]
              - generic [ref=e81]: Overview
          - button "Support & diagnostics" [ref=e84] [cursor=pointer]:
            - img [ref=e86]
            - generic [ref=e89]: Support & diagnostics
            - img [ref=e91]
        - generic [ref=e94]:
          - region "Health summary" [ref=e95]:
            - img [ref=e98]
            - strong [ref=e101]: Checking
            - button "Refresh" [ref=e103] [cursor=pointer]:
              - img [ref=e105]
              - generic [ref=e108]: Refresh
          - region "Status" [ref=e109]:
            - button "Total tokens — —" [ref=e110] [cursor=pointer]:
              - generic [ref=e111]: Total tokens
              - generic [ref=e112]: —
              - generic [ref=e113]: —
            - button "Total sessions — view all →" [ref=e114] [cursor=pointer]:
              - generic [ref=e115]: Total sessions
              - generic [ref=e116]: —
              - generic [ref=e117]: view all →
            - generic [ref=e118]:
              - generic [ref=e119]: Uptime
              - generic [ref=e120]: —
              - generic [ref=e121]: —
          - region "Environment" [ref=e122]:
            - generic [ref=e123]: v— · —
          - region "Health findings" [ref=e124]:
            - article [ref=e125]: Loading health report
  - status [ref=e126]
```

# Test source

```ts
  1  | import { test, expect, type Page } from '@playwright/test'
  2  | 
  3  | // Desktop-shell emulation: the SPA detects the platform from the injected
  4  | // bridge object, and the CLI invocation prefix comes from the same bridge.
  5  | // The prefix mirrors the shape the Electron main process reports (env pair +
  6  | // quoted bundled binary path).
  7  | const PREFIX =
  8  |   "OPENSQUILLA_STATE_DIR='/tmp/e2e desk/state' "
  9  |   + "OPENSQUILLA_GATEWAY_CONFIG_PATH='/tmp/e2e desk/config.toml' "
  10 |   + "'/Applications/OpenSquilla.app/Contents/Resources/runtime/gateway/opensquilla-gateway/opensquilla-gateway'"
  11 | 
  12 | async function installDesktopBridge(page: Page) {
  13 |   await page.addInitScript((prefix: string) => {
  14 |     const w = window as unknown as Record<string, unknown>
  15 |     w.opensquillaDesktop = {
  16 |       getOsLocale: async () => 'en-US',
  17 |       isAutoUpdateEnabled: async () => false,
  18 |       getGatewayStatus: async () => ({ url: '', port: 0, owned: true, status: 'ready', logPath: '' }),
  19 |       getCliInvocation: async () => ({ mode: 'bundled', prefix }),
  20 |       revealGatewayLog: async () => true,
  21 |       getDesktopSettings: async () => ({}),
  22 |       saveDesktopSettings: async () => ({}),
  23 |       resetDesktopSettings: async () => ({ ok: true }),
  24 |       setNativeTheme: async () => undefined,
  25 |       openArtifact: async () => ({ ok: true }),
  26 |       getOnboardingDefaults: async () => ({}),
  27 |       saveOnboarding: async () => ({}),
  28 |       cancelOnboarding: async () => ({}),
  29 |       getBootState: async () => ({}),
  30 |       retryStartup: async () => ({}),
  31 |       quitApp: async () => ({}),
  32 |       onBootStatus: () => () => {},
  33 |       onBootError: () => () => {},
  34 |     }
  35 |   }, PREFIX)
  36 | }
  37 | 
  38 | test.describe('desktop shell CLI invocation', () => {
  39 |   test('folds health-card commands and rewrites them to the bundled CLI', async ({ page }) => {
  40 |     await installDesktopBridge(page)
  41 |     await page.goto('/control/overview')
  42 |     // The first doctor pass on a cold gateway can take a while, hence the
  43 |     // generous wait for the first rendered finding card.
  44 |     await page.waitForSelector('.health-finding', { timeout: 45000 })
  45 | 
  46 |     // Findings with CLI fix steps fold behind the advanced disclosure on
  47 |     // desktop. A fully configured gateway may surface none — skip then
  48 |     // rather than asserting against an uncontrolled doctor state.
  49 |     const folds = page.locator('details.cli-fold')
  50 |     test.skip(await folds.count() === 0, 'gateway reports no findings with CLI fix steps')
  51 | 
  52 |     const firstFold = folds.first()
  53 |     await firstFold.locator('summary').click()
  54 |     await expect(firstFold.locator('.cli-fold__hint')).toBeVisible()
  55 | 
  56 |     // Every opensquilla command inside the fold carries the paste-ready prefix.
  57 |     const firstCommand = firstFold.locator('code').first()
  58 |     await expect(firstCommand).toContainText("OPENSQUILLA_STATE_DIR='/tmp/e2e desk/state'")
  59 |     await expect(firstCommand).toContainText('opensquilla-gateway')
  60 | 
  61 |     // Gateway lifecycle commands are guidance on desktop, never a rewritten,
  62 |     // runnable command that would fight the shell-supervised gateway.
  63 |     await expect(page.locator('.health-step__command code', { hasText: 'gateway restart' })).toHaveCount(0)
  64 |   })
  65 | 
  66 |   test('hides the CLI handoff disclosure in settings', async ({ page }) => {
  67 |     await installDesktopBridge(page)
  68 |     await page.goto('/control/settings/connection')
  69 |     const dialog = page.getByRole('dialog', { name: 'Settings' })
  70 |     await expect(dialog).toBeVisible({ timeout: 15000 })
  71 |     await expect(dialog.locator('.settings-banner')).toBeVisible({ timeout: 15000 })
  72 |     await expect(dialog.getByRole('button', { name: 'CLI handoff' })).toHaveCount(0)
  73 |   })
  74 | 
  75 |   test('web keeps flat, unprefixed commands', async ({ page }) => {
  76 |     await page.goto('/control/overview')
> 77 |     await page.waitForSelector('.health-finding', { timeout: 45000 })
     |                ^ Error: page.waitForSelector: Test timeout of 30000ms exceeded.
  78 |     await expect(page.locator('details.cli-fold')).toHaveCount(0)
  79 |     const commands = page.locator('.health-step__command code').filter({ hasText: 'opensquilla' })
  80 |     test.skip(await commands.count() === 0, 'gateway reports no findings with CLI fix steps')
  81 |     const command = commands.first()
  82 |     await expect(command).toContainText(/^opensquilla /)
  83 |     await expect(command).not.toContainText('OPENSQUILLA_STATE_DIR=')
  84 |   })
  85 | })
  86 | 
```