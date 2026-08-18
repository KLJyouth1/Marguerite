# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> /setup deep link redirects into the overlay on the first not-ready section
- Location: e2e\settings-modal.spec.ts:310:3

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: locator.textContent: Test timeout of 30000ms exceeded.
Call log:
  - waiting for getByRole('dialog', { name: 'Settings' }).locator('.settings-banner').locator('.settings-banner__row')

```

# Page snapshot

```yaml
- generic [ref=e1]:
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
      - button "Settings" [ref=e33] [cursor=pointer]:
        - img [ref=e35]
        - generic [ref=e38]: Settings
    - separator "Resize sidebar" [ref=e39]:
      - status [ref=e40]
    - generic [ref=e41]:
      - banner:
        - generic [ref=e42]:
          - button "Manage gateway connection" [ref=e43] [cursor=pointer]: Connected
          - button "Language" [ref=e45] [cursor=pointer]:
            - img [ref=e47]
            - generic [ref=e50]: English
          - button "Theme" [ref=e52] [cursor=pointer]:
            - img [ref=e54]
      - main [ref=e57]
    - status [ref=e58]
  - dialog "Settings" [ref=e60]:
    - generic [ref=e61]:
      - tablist "Settings sections" [ref=e62]:
        - generic [ref=e63]: Gateway
        - 'tab "Connection: Connected" [ref=e64] [cursor=pointer]':
          - img [ref=e66]
          - generic [ref=e69]: Connection
        - generic [ref=e71]: AI configuration
        - 'tab "Model Service: Ready" [selected] [ref=e72] [cursor=pointer]':
          - img [ref=e74]
          - generic [ref=e80]: Model Service
        - 'tab "Model Routing: Ready" [ref=e81] [cursor=pointer]':
          - img [ref=e83]
          - generic [ref=e89]: Model Routing
        - generic [ref=e90]: Capabilities
        - 'tab "Capabilities: Optional" [ref=e91] [cursor=pointer]':
          - img [ref=e93]
          - generic [ref=e95]: Capabilities
        - generic [ref=e96]: Preferences
        - 'tab "Behavior: Applies on save" [ref=e97] [cursor=pointer]':
          - img [ref=e99]
          - generic [ref=e101]: Behavior
        - 'tab "Privacy: Applies on save" [ref=e102] [cursor=pointer]':
          - img [ref=e104]
          - generic [ref=e106]: Privacy
        - tab "Sandbox" [ref=e107] [cursor=pointer]:
          - img [ref=e109]
          - generic [ref=e111]: Sandbox
        - tab "Memory & Profile" [ref=e112] [cursor=pointer]:
          - img [ref=e114]
          - generic [ref=e117]: Memory & Profile
        - tab "Appearance" [ref=e118] [cursor=pointer]:
          - img [ref=e120]
          - generic [ref=e122]: Appearance
        - tab "Keyboard" [ref=e123] [cursor=pointer]:
          - img [ref=e125]
          - generic [ref=e127]: Keyboard
        - tab "Advanced" [ref=e128] [cursor=pointer]:
          - img [ref=e130]
          - generic [ref=e133]: Advanced
      - generic [ref=e134]:
        - banner [ref=e135]:
          - heading "Settings" [level=2] [ref=e136]
          - button "Close" [active] [ref=e137] [cursor=pointer]:
            - img [ref=e139]
        - 'tabpanel "Model Service: Ready" [ref=e142]':
          - group [ref=e143]:
            - generic [ref=e144]:
              - generic [ref=e145]:
                - generic [ref=e146]:
                  - heading "Model Service" [level=3] [ref=e147]
                  - paragraph [ref=e148]: Connect providers and manage their credentials and models.
                - button "Add provider" [ref=e149] [cursor=pointer]:
                  - img [ref=e151]
                  - text: Add provider
              - group [ref=e152]:
                - generic [ref=e153]:
                  - generic [ref=e155]:
                    - heading "Configured providers" [level=4] [ref=e157]
                    - paragraph [ref=e158]: 1 of 1 ready. Testing sends one small model request.
                  - list [ref=e159]:
                    - listitem [ref=e160]:
                      - button "Edit DeepSeek — Active — Credentials ready — Not verified" [ref=e161] [cursor=pointer]:
                        - generic [ref=e163]: DeepSeek
                      - generic [ref=e164]:
                        - button "Verify saved configuration — DeepSeek" [ref=e165] [cursor=pointer]: Verify saved configuration
                        - button "Edit DeepSeek" [ref=e166] [cursor=pointer]:
                          - img [ref=e168]
                          - text: Edit
                - text: • ▸ ▸
    - contentinfo [ref=e171]:
      - generic [ref=e172]: More options live in
      - code [ref=e173]: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
      - button "Copy config path" [ref=e174] [cursor=pointer]:
        - img [ref=e176]
      - generic [ref=e179]: ·
      - generic [ref=e180]: Most changes apply live; some need a gateway restart
```

# Test source

```ts
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
  234 |     await expect(visualMode()).toBeVisible()
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
> 322 |     const ready = await banner.locator('.settings-banner__row').textContent()
      |                                                                 ^ Error: locator.textContent: Test timeout of 30000ms exceeded.
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
  335 |     const maxResults = dialog(page).locator('input[name="setup_search_max_results"]')
  336 |     await expect(maxResults).toBeVisible()
  337 |     const original = await maxResults.inputValue()
  338 |     await maxResults.fill(String(Number(original || '5') + 3))
  339 | 
  340 |     const dirtybar = dialog(page).locator('.settings-dirtybar')
  341 |     await expect(dirtybar).toBeVisible()
  342 |     await expect(dirtybar).toContainText('Unsaved changes in Capabilities')
  343 | 
  344 |     // Closing with unsaved edits raises the themed confirm; declining keeps the modal.
  345 |     await page.keyboard.press('Escape')
  346 |     const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes?' })
  347 |     await expect(confirm).toBeVisible()
  348 |     await confirm.getByRole('button', { name: 'Cancel' }).click()
  349 |     await expect(confirm).toBeHidden()
  350 |     await expect(dialog(page)).toBeVisible()
  351 | 
  352 |     await dirtybar.getByRole('button', { name: 'Discard' }).click()
  353 |     await expect(dirtybar).toBeHidden()
  354 |     await expect(maxResults).toHaveValue(original)
  355 |   })
  356 | 
  357 |   test('history Back with dirty edits raises the discard confirm instead of silently closing', async ({ page }) => {
  358 |     await openFromSidebar(page)
  359 | 
  360 |     await railTab(page, 'Capabilities').click()
  361 |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  362 |     const maxResults = dialog(page).locator('input[name="setup_search_max_results"]')
  363 |     await expect(maxResults).toBeVisible()
  364 |     const original = await maxResults.inputValue()
  365 |     await maxResults.fill(String(Number(original || '5') + 3))
  366 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeVisible()
  367 | 
  368 |     // History traversal (browser Back, a trackpad back-swipe) unmounts the
  369 |     // route-mounted overlay without passing through requestClose, so the
  370 |     // router-level guard must raise the same confirm. Declining cancels the
  371 |     // navigation and restores the /settings URL.
  372 |     await page.goBack()
  373 |     const confirm = page.getByRole('dialog', { name: 'Discard unsaved changes?' })
  374 |     await expect(confirm).toBeVisible()
  375 |     await confirm.getByRole('button', { name: 'Cancel' }).click()
  376 |     await expect(confirm).toBeHidden()
  377 |     await expect(dialog(page)).toBeVisible()
  378 |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  379 |     await expect(maxResults).not.toHaveValue(original)
  380 | 
  381 |     // Accepting the discard lets the same Back proceed and close the overlay.
  382 |     await page.goBack()
  383 |     await expect(confirm).toBeVisible()
  384 |     await confirm.getByRole('button', { name: 'Confirm' }).click()
  385 |     await expect(dialog(page)).toBeHidden()
  386 |     await expect(page).not.toHaveURL(/\/settings/)
  387 |     await expect(settingsRow(page)).toBeFocused()
  388 |   })
  389 | 
  390 |   test('viewport opts out of horizontal overscroll so a trackpad swipe cannot pop history', async ({ page }) => {
  391 |     await page.goto(CONTROL_URL)
  392 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  393 |     // Chromium turns a two-finger horizontal overscroll into history navigation;
  394 |     // with Settings and chat sessions living on the history stack that gesture
  395 |     // would dismiss them wholesale. Both propagation roots must opt out.
  396 |     const behaviors = await page.evaluate(() => [
  397 |       getComputedStyle(document.documentElement).overscrollBehaviorX,
  398 |       getComputedStyle(document.body).overscrollBehaviorX,
  399 |     ])
  400 |     expect(behaviors).toEqual(['none', 'none'])
  401 |   })
  402 | 
  403 |   test('live save round-trip persists a harmless Capabilities toggle', async ({ page }) => {
  404 |     test.setTimeout(90000)
  405 |     await openFromSidebar(page)
  406 | 
  407 |     // memory.auto_capture_enabled is hot-applied via the config.patch path.
  408 |     const capture = () => dialog(page).locator('input[name="setup_memory_auto_capture"]')
  409 |     const saveMemory = () => dialog(page).getByRole('button', { name: 'Save memory embedding' })
  410 | 
  411 |     await railTab(page, 'Capabilities').click()
  412 |     await expect(capture()).toBeVisible()
  413 |     const initial = await capture().isChecked()
  414 | 
  415 |     await capture().setChecked(!initial)
  416 |     await saveMemory().click()
  417 |     await expect(page.locator('.toast', { hasText: /Memory/ }).first()).toBeVisible()
  418 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden({ timeout: 10000 })
  419 | 
  420 |     // Reload: the persisted value must survive a fresh modal.
  421 |     await page.reload()
  422 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
```