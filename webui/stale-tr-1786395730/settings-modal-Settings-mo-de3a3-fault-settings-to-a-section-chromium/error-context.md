# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> keeps the SettingsDialog mounted when routing from default settings to a section
- Location: e2e\settings-modal.spec.ts:116:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' }).locator('.settings-banner')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' }).locator('.settings-banner')

```

```yaml
- navigation "Primary":
  - text: Marguerite
  - button "Collapse sidebar" [expanded]:
    - img
  - navigation "Control navigation":
    - button "New task":
      - img
      - text: New task
    - link "Skills & Channels":
      - /url: /control/skills
      - img
      - text: Skills & Channels
    - link "Cron":
      - /url: /control/cron
      - img
      - text: Cron
    - link "View usage":
      - /url: /control/usage
      - img
      - text: View usage
  - button "Settings":
    - img
    - text: Settings
- separator "Resize sidebar":
  - status
- banner:
  - button "Manage gateway connection": Disconnected
  - button "Language":
    - img
    - text: English
  - button "Theme":
    - img
- main
- status
- dialog "Settings":
  - tablist "Settings sections":
    - 'tab "Connection: Disconnected"': Connection
    - 'tab "Model Service: Review" [selected]': Model Service
    - 'tab "Model Routing: Provider first"': Model Routing
    - 'tab "Capabilities: Optional"': Capabilities
    - 'tab "Behavior: Applies on save"': Behavior
    - 'tab "Privacy: Applies on save"': Privacy
    - tab "Sandbox"
    - tab "Memory & Profile"
    - tab "Appearance"
    - tab "Keyboard"
    - tab "Advanced"
  - banner:
    - heading "Settings" [level=2]
    - button "Close":
      - img
  - 'tabpanel "Model Service: Review"':
    - group:
      - status:
        - status "Loading"
        - strong: Model Service
        - text: Loading
  - contentinfo:
    - text: More options live in
    - code: ~/.opensquilla/config.toml
    - button "Copy config path":
      - img
    - text: Most changes apply live; some need a gateway restart
```

# Test source

```ts
  19  |   dialog(page).getByRole('tab', { name: new RegExp(`^${name}:`) })
  20  | 
  21  | async function openFromSidebar(page: import('@playwright/test').Page) {
  22  |   await page.goto(CONTROL_URL)
  23  |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  24  |   await settingsRow(page).click()
  25  |   await expect(dialog(page)).toBeVisible()
  26  | }
  27  | 
  28  | test.describe('Settings modal', () => {
  29  |   test('opens from the sidebar with the curated section rail and readiness banner', async ({ page }) => {
  30  |     await openFromSidebar(page)
  31  | 
  32  |     // Dialog a11y: focus moves into the modal on open, before any interaction.
  33  |     await expect(page.getByRole('button', { name: 'Close' })).toBeFocused()
  34  | 
  35  |     // Rail has the backend-config sections (each with a readiness/status dot)
  36  |     // plus the client-only sections.
  37  |     const tabs = dialog(page).getByRole('tab')
  38  |     await expect(tabs).toHaveCount(SECTIONS.length + CLIENT_SECTIONS.length)
  39  |     for (const name of SECTIONS) {
  40  |       await expect(railTab(page, name)).toBeVisible()
  41  |     }
  42  |     for (const name of CLIENT_SECTIONS) {
  43  |       await expect(dialog(page).getByRole('tab', { name, exact: true })).toBeVisible()
  44  |     }
  45  | 
  46  |     // Readiness banner: quiet ready line or actionable count.
  47  |     const banner = dialog(page).locator('.settings-banner')
  48  |     await expect(banner).toBeVisible()
  49  |     await expect(banner.locator('.settings-banner__row')).toContainText(/Ready to run|Action needed \(\d+\)/)
  50  | 
  51  |     // CLI handoff disclosure expands with command groups and the config summary.
  52  |     await banner.getByRole('button', { name: 'CLI handoff' }).click()
  53  |     await expect(banner.locator('.setup-cli__group', { hasText: 'CLI handoff' })).toBeVisible()
  54  |     await expect(banner.locator('.setup-cli__group', { hasText: 'CLI recipes' })).toBeVisible()
  55  |     await expect(banner.locator('.setup-summary')).toContainText('Provider')
  56  |   })
  57  | 
  58  |   test('no YAML editor, raw key search, or guided-setup wording anywhere', async ({ page }) => {
  59  |     await openFromSidebar(page)
  60  | 
  61  |     await expect(dialog(page).getByRole('button', { name: 'YAML', exact: true })).toHaveCount(0)
  62  |     await expect(dialog(page).getByRole('button', { name: 'Form', exact: true })).toHaveCount(0)
  63  |     await expect(dialog(page).locator('#cfg-search')).toHaveCount(0)
  64  |     await expect(dialog(page).locator('textarea#cfg-yaml-area')).toHaveCount(0)
  65  |     await expect(dialog(page).getByText('Guided setup')).toHaveCount(0)
  66  | 
  67  |     // Footer keeps the config.toml escape hatch with a copy affordance.
  68  |     const foot = dialog(page).locator('.settings-foot')
  69  |     await expect(foot).toContainText('More options live in')
  70  |     // Honest restart copy: most edits apply live, only some need a restart.
  71  |     await expect(foot).toContainText('Most changes apply live; some need a gateway restart')
  72  |     // The old blanket "always restart" leak is gone.
  73  |     await expect(foot).not.toContainText('Restart the gateway after manual edits')
  74  |     await expect(foot.locator('.settings-foot__path')).toContainText(/config.*\.toml/)
  75  |     await foot.getByRole('button', { name: 'Copy config path' }).click()
  76  |     await expect(page.locator('.toast', { hasText: /Copied/ }).first()).toBeVisible()
  77  |   })
  78  | 
  79  |   test('rail switches sections, marks the active tab, and syncs the URL with replace', async ({ page }) => {
  80  |     await openFromSidebar(page)
  81  |     await expect(page).toHaveURL(/\/settings$/)
  82  | 
  83  |     await railTab(page, 'Capabilities').click()
  84  |     await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
  85  |     // Switching a section reflects in the URL so it stays deep-linkable.
  86  |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  87  |     // The Capabilities panel renders its tools as stacked sections (Web search,
  88  |     // Memory, Image, Audio) rather than a redundant "Capabilities" card heading.
  89  |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  90  | 
  91  |     await railTab(page, 'Model Routing').click()
  92  |     await expect(railTab(page, 'Model Routing')).toHaveAttribute('aria-selected', 'true')
  93  |     await expect(page).toHaveURL(/\/settings\/modelStrategy$/)
  94  |     await expect(dialog(page).getByRole('radiogroup', { name: 'Model routing', exact: true })).toBeVisible()
  95  | 
  96  |     // Section navigation uses replace, so a single Back exits Settings rather
  97  |     // than walking section history.
  98  |     await page.goBack()
  99  |     await expect(dialog(page)).toBeHidden()
  100 |     await expect(page).not.toHaveURL(/\/settings/)
  101 |   })
  102 | 
  103 |   test('opens Memory & Profile as a first-level action route without global save state', async ({ page }) => {
  104 |     await openFromSidebar(page)
  105 | 
  106 |     const memoryTab = dialog(page).getByRole('tab', { name: 'Memory & Profile', exact: true })
  107 |     await memoryTab.click()
  108 | 
  109 |     await expect(memoryTab).toHaveAttribute('aria-selected', 'true')
  110 |     await expect(page).toHaveURL(/\/settings\/memory$/)
  111 |     await expect(dialog(page).getByRole('heading', { name: 'Memory & Profile' })).toBeVisible()
  112 |     await expect(dialog(page).locator('[data-testid="settings-memory-panel"]')).toBeVisible()
  113 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  114 |   })
  115 | 
  116 |   test('keeps the SettingsDialog mounted when routing from default settings to a section', async ({ page }) => {
  117 |     await openFromSidebar(page)
  118 |     await expect(page).toHaveURL(/\/settings$/)
> 119 |     await expect(dialog(page).locator('.settings-banner')).toBeVisible()
      |                                                            ^ Error: expect(locator).toBeVisible() failed
  120 |     await expect(dialog(page).locator('.settings-loading')).toHaveCount(0)
  121 | 
  122 |     await page.evaluate(() => {
  123 |       const modal = document.querySelector('.settings-modal')
  124 |       if (!modal) throw new Error('settings modal missing')
  125 |       const state = {
  126 |         modal,
  127 |         removed: false,
  128 |         loadingSeen: false,
  129 |         observer: null as MutationObserver | null,
  130 |       }
  131 |       state.observer = new MutationObserver(() => {
  132 |         state.removed = state.removed || !document.body.contains(state.modal)
  133 |         state.loadingSeen = state.loadingSeen || !!document.querySelector('.settings-loading')
  134 |       })
  135 |       state.observer.observe(document.body, { childList: true, subtree: true })
  136 |       ;(window as typeof window & { __settingsModalProbe?: typeof state }).__settingsModalProbe = state
  137 |     })
  138 | 
  139 |     await railTab(page, 'Capabilities').click()
  140 |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  141 |     await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
  142 |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  143 |     // Wait past the route fade and Settings pop enter windows so delayed
  144 |     // remounts, re-entry animations, or loading fallbacks are caught.
  145 |     await page.waitForTimeout(350)
  146 | 
  147 |     const probe = await page.evaluate(() => {
  148 |       const state = (window as typeof window & {
  149 |         __settingsModalProbe?: {
  150 |           modal: Element
  151 |           removed: boolean
  152 |           loadingSeen: boolean
  153 |           observer: MutationObserver | null
  154 |         }
  155 |       }).__settingsModalProbe
  156 |       if (!state) throw new Error('settings modal probe missing')
  157 |       state.observer?.disconnect()
  158 |       return {
  159 |         sameNode: state.modal === document.querySelector('.settings-modal'),
  160 |         removed: state.removed,
  161 |         loadingSeen: state.loadingSeen,
  162 |       }
  163 |     })
  164 |     expect(probe.sameNode).toBe(true)
  165 |     expect(probe.removed).toBe(false)
  166 |     expect(probe.loadingSeen).toBe(false)
  167 |   })
  168 | 
  169 |   test('opens Settings without applying the app route fade to the overlay', async ({ page }) => {
  170 |     await page.goto(CONTROL_URL)
  171 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  172 |     await page.evaluate(() => {
  173 |       const state = {
  174 |         overlaySeen: false,
  175 |         routeFadeSeen: false,
  176 |         observer: null as MutationObserver | null,
  177 |       }
  178 |       const inspect = () => {
  179 |         const overlay = document.querySelector('.settings-overlay')
  180 |         if (!overlay) return
  181 |         state.overlaySeen = true
  182 |         state.routeFadeSeen = state.routeFadeSeen || String(overlay.className).includes('route-fade-')
  183 |       }
  184 |       state.observer = new MutationObserver(inspect)
  185 |       state.observer.observe(document.body, {
  186 |         attributeFilter: ['class'],
  187 |         attributes: true,
  188 |         childList: true,
  189 |         subtree: true,
  190 |       })
  191 |       inspect()
  192 |       ;(window as typeof window & { __settingsOpenProbe?: typeof state }).__settingsOpenProbe = state
  193 |     })
  194 | 
  195 |     await settingsRow(page).click()
  196 |     await expect(dialog(page)).toBeVisible()
  197 |     await expect(dialog(page).locator('.settings-banner')).toBeVisible()
  198 |     await page.waitForTimeout(200)
  199 | 
  200 |     const probe = await page.evaluate(() => {
  201 |       const state = (window as typeof window & {
  202 |         __settingsOpenProbe?: {
  203 |           overlaySeen: boolean
  204 |           routeFadeSeen: boolean
  205 |           observer: MutationObserver | null
  206 |         }
  207 |       }).__settingsOpenProbe
  208 |       if (!state) throw new Error('settings open probe missing')
  209 |       state.observer?.disconnect()
  210 |       return {
  211 |         overlaySeen: state.overlaySeen,
  212 |         routeFadeSeen: state.routeFadeSeen,
  213 |       }
  214 |     })
  215 |     expect(probe.overlaySeen).toBe(true)
  216 |     expect(probe.routeFadeSeen).toBe(false)
  217 |   })
  218 | 
  219 |   test('Model Routing section saves the routing panel style', async ({ page }) => {
```