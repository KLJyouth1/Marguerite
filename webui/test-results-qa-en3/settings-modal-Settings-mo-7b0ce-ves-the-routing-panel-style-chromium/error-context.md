# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> Model Routing section saves the routing panel style
- Location: e2e\settings-modal.spec.ts:219:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' }).locator('select[name="setup_model_strategy_router_visual_mode"]')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' }).locator('select[name="setup_model_strategy_router_visual_mode"]')

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
  - button "Manage gateway connection": Connected
  - button "Language":
    - img
    - text: English
  - button "Theme":
    - img
- main
- status
- dialog "Settings":
  - tablist "Settings sections":
    - 'tab "Connection: Connected"': Connection
    - 'tab "Model Service: Ready"': Model Service
    - 'tab "Model Routing: Ready, unsaved changes" [selected]': Model Routing
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
  - 'tabpanel "Model Routing: Ready, unsaved changes"':
    - group:
      - heading "Model routing" [level=3]
      - paragraph: Choose how models are picked for each request.
      - button "Manage Model Service"
      - region "Choose how models are used":
        - radiogroup "Choose how models are used":
          - radio "Model ensemble Capability-first Candidate models answer in parallel, then one aggregator model produces the final reply. Uses more tokens."
          - text: Model ensemble Capability-first Candidate models answer in parallel, then one aggregator model produces the final reply. Uses more tokens.
          - radio "Intelligent model routing Token-efficient Each request goes to a fitting model, so simple tasks stay cheap." [checked]
          - text: Intelligent model routing Token-efficient Each request goes to a fitting model, so simple tasks stay cheap.
          - radio "Fixed model Predictable One model for everything. Predictable and simple."
          - text: Fixed model Predictable One model for everything. Predictable and simple.
      - heading "Intelligent model routing" [level=4]
      - paragraph: Pick a model for each request level. One provider can supply every level.
      - heading "Model roles" [level=5]
      - table:
        - row "Tier Model Thinking Image"
        - row "C0 Fast/simple deepseek-v4-flash off c0 supports image":
          - text: C0 Fast/simple
          - textbox "c0 model": deepseek-v4-flash
          - combobox "c0 thinking level":
            - option "-"
            - option "off" [selected]
            - option "none"
            - option "minimal"
            - option "low"
            - option "medium"
            - option "high"
            - option "xhigh"
          - switch "c0 supports image"
        - row "C1 Balanced deepseek-v4-flash low c1 supports image":
          - text: C1 Balanced
          - textbox "c1 model": deepseek-v4-flash
          - combobox "c1 thinking level":
            - option "-"
            - option "off"
            - option "none"
            - option "minimal"
            - option "low" [selected]
            - option "medium"
            - option "high"
            - option "xhigh"
          - switch "c1 supports image"
        - row "C2 Strong reasoning deepseek-v4-pro medium c2 supports image":
          - text: C2 Strong reasoning
          - textbox "c2 model": deepseek-v4-pro
          - combobox "c2 thinking level":
            - option "-"
            - option "off"
            - option "none"
            - option "minimal"
            - option "low"
            - option "medium" [selected]
            - option "high"
            - option "xhigh"
          - switch "c2 supports image"
        - row "C3 Max quality deepseek-v4-pro high c3 supports image":
          - text: C3 Max quality
          - textbox "c3 model": deepseek-v4-pro
          - combobox "c3 thinking level":
            - option "-"
            - option "off"
            - option "none"
            - option "minimal"
            - option "low"
            - option "medium"
            - option "high" [selected]
            - option "xhigh"
          - switch "c3 supports image"
      - group: Advanced options
      - text: Current model provider
      - strong: DeepSeek
      - text: Fixed and fallback model * Used for every request in Fixed model mode, and as the fallback if routing or ensemble execution cannot complete.
      - textbox "Fixed and fallback model * Used for every request in Fixed model mode, and as the fallback if routing or ensemble execution cannot complete.":
        - /placeholder: Enter a model ID
        - text: deepseek-v4-flash
  - status: Unsaved changes in Model Routing
  - button "Discard routing changes"
  - button "Save routing changes"
  - contentinfo:
    - text: More options live in
    - code: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
    - button "Copy config path":
      - img
    - text: Most changes apply live; some need a gateway restart
```

# Test source

```ts
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
  220 |     test.setTimeout(90000)
  221 |     await openFromSidebar(page)
  222 | 
  223 |     const visualMode = () => dialog(page).locator('select[name="setup_model_strategy_router_visual_mode"]')
  224 |     const dirtybar = dialog(page).locator('.settings-dirtybar')
  225 | 
  226 |     await railTab(page, 'Model Routing').click()
  227 |     // The picker rides with the router details, so make the router strategy
  228 |     // active first if the gateway under test starts on another strategy (the
  229 |     // strategy choice is then saved along with the style — acceptable for the
  230 |     // disposable gateways this suite runs against).
  231 |     const routerCard = dialog(page).locator('[data-strategy-id="router"]')
  232 |     await expect(routerCard).toBeVisible()
  233 |     if ((await routerCard.getAttribute('aria-checked')) !== 'true') await routerCard.click()
> 234 |     await expect(visualMode()).toBeVisible()
      |                                ^ Error: expect(locator).toBeVisible() failed
  235 |     const initial = await visualMode().inputValue()
  236 |     const next = initial === 'legacy_grid' ? 'real_candidates' : 'legacy_grid'
  237 | 
  238 |     await visualMode().selectOption(next)
  239 |     await expect(dirtybar).toBeVisible()
  240 |     await expect(dirtybar).toContainText('Unsaved changes in Model Routing')
  241 |     await dirtybar.getByRole('button', { name: 'Save' }).click()
  242 |     await expect(page.locator('.toast', { hasText: /Router saved/ }).first()).toBeVisible()
  243 |     await expect(dirtybar).toBeHidden({ timeout: 10000 })
  244 | 
  245 |     await page.reload()
  246 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  247 |     await expect(dialog(page)).toBeVisible()
  248 |     await railTab(page, 'Model Routing').click()
  249 |     await expect(visualMode()).toHaveValue(next)
  250 | 
  251 |     await visualMode().selectOption(initial)
  252 |     await dirtybar.getByRole('button', { name: 'Save' }).click()
  253 |     await expect(dirtybar).toBeHidden({ timeout: 10000 })
  254 |   })
  255 | 
  256 |   test('Escape closes the overlay, leaves /settings, and returns focus to the invoker', async ({ page }) => {
  257 |     await openFromSidebar(page)
  258 |     await expect(page).toHaveURL(/\/settings$/)
  259 | 
  260 |     await page.keyboard.press('Escape')
  261 |     await expect(dialog(page)).toBeHidden()
  262 |     // The overlay is route-mounted: closing navigates back off /settings.
  263 |     await expect(page).not.toHaveURL(/\/settings/)
  264 |     await expect(settingsRow(page)).toBeFocused()
  265 | 
  266 |     // Escape inside the overlay must not collapse the docked sidebar.
  267 |     await expect(page.locator('.sidebar.docked')).toBeVisible()
  268 |   })
  269 | 
  270 |   test('closing Settings restores the previous route view without blanking the outlet', async ({ page }) => {
  271 |     await page.goto(CONTROL_URL + 'chat/new')
  272 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  273 |     await expect(page.getByLabel('Message to send')).toBeVisible()
  274 | 
  275 |     await settingsRow(page).click()
  276 |     await expect(dialog(page)).toBeVisible()
  277 |     await page.getByRole('button', { name: 'Close' }).click()
  278 | 
  279 |     await expect(dialog(page)).toBeHidden()
  280 |     await expect(page).toHaveURL(/\/chat\/new$/)
  281 |     await expect(page.getByLabel('Message to send')).toBeVisible()
  282 |   })
  283 | 
  284 |   test('cold deep link to /settings/connection closes home and moves focus to the sidebar', async ({ page }) => {
  285 |     // Land directly on a section with no in-app invoker.
  286 |     await page.goto(CONTROL_URL + 'settings/connection')
  287 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  288 |     await expect(dialog(page)).toBeVisible()
  289 |     await expect(railTab(page, 'Connection')).toHaveAttribute('aria-selected', 'true')
  290 |     // Connection renders even before/without a loaded config gate.
  291 |     await expect(dialog(page).getByRole('heading', { name: 'Connection' })).toBeVisible()
  292 | 
  293 |     await page.keyboard.press('Escape')
  294 |     await expect(dialog(page)).toBeHidden()
  295 |     await expect(page).not.toHaveURL(/\/settings/)
  296 |     // No detached focus: it lands on the sidebar Settings button.
  297 |     await expect(settingsRow(page)).toBeFocused()
  298 |   })
  299 | 
  300 |   test('/config deep link redirects into the settings overlay', async ({ page }) => {
  301 |     await page.goto(CONTROL_URL + 'config')
  302 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  303 | 
  304 |     await expect(dialog(page)).toBeVisible()
  305 |     // /config now redirects to /settings (default section).
  306 |     await expect(page).toHaveURL(/\/settings$/)
  307 |     await expect(railTab(page, 'Model Service')).toHaveAttribute('aria-selected', 'true')
  308 |   })
  309 | 
  310 |   test('/setup deep link redirects into the overlay on the first not-ready section', async ({ page }) => {
  311 |     await page.goto(CONTROL_URL + 'setup')
  312 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  313 | 
  314 |     await expect(dialog(page)).toBeVisible()
  315 |     // /setup redirects to /settings/auto (first not-ready section).
  316 |     await expect(page).toHaveURL(/\/settings\/auto$/)
  317 | 
  318 |     // The selected tab matches the readiness state: with everything ready it
  319 |     // is Model Service, otherwise the first section whose rail dot needs action.
  320 |     await expect(dialog(page).getByRole('tab', { selected: true })).toHaveCount(1)
  321 |     const banner = dialog(page).locator('.settings-banner')
  322 |     const ready = await banner.locator('.settings-banner__row').textContent()
  323 |     if (ready && ready.includes('Ready to run')) {
  324 |       await expect(railTab(page, 'Model Service')).toHaveAttribute('aria-selected', 'true')
  325 |     } else {
  326 |       const selected = dialog(page).getByRole('tab', { selected: true })
  327 |       await expect(selected).toHaveAttribute('aria-label', /Needs action|Provider first|Missing/)
  328 |     }
  329 |   })
  330 | 
  331 |   test('dirty edits raise the bar, guard close, and Discard restores values', async ({ page }) => {
  332 |     await openFromSidebar(page)
  333 | 
  334 |     await railTab(page, 'Capabilities').click()
```