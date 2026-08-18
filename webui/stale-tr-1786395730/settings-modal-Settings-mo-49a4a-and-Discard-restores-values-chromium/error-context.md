# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> dirty edits raise the bar, guard close, and Discard restores values
- Location: e2e\settings-modal.spec.ts:331:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' }).locator('input[name="setup_search_max_results"]')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' }).locator('input[name="setup_search_max_results"]')

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
    - 'tab "Model Service: Review"': Model Service
    - 'tab "Model Routing: Provider first"': Model Routing
    - 'tab "Capabilities: Optional" [selected]': Capabilities
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
  - 'tabpanel "Capabilities: Optional"':
    - group:
      - status:
        - status "Loading"
        - strong: Capabilities
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
  335 |     const maxResults = dialog(page).locator('input[name="setup_search_max_results"]')
> 336 |     await expect(maxResults).toBeVisible()
      |                              ^ Error: expect(locator).toBeVisible() failed
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
  423 |     await expect(dialog(page)).toBeVisible()
  424 |     await railTab(page, 'Capabilities').click()
  425 |     await expect(capture()).toBeVisible()
  426 |     expect(await capture().isChecked()).toBe(!initial)
  427 | 
  428 |     // Restore the original value.
  429 |     await capture().setChecked(initial)
  430 |     await saveMemory().click()
  431 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden({ timeout: 10000 })
  432 |     await page.reload()
  433 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  434 |     await expect(dialog(page)).toBeVisible()
  435 |     await railTab(page, 'Capabilities').click()
  436 |     await expect(capture()).toBeVisible()
```