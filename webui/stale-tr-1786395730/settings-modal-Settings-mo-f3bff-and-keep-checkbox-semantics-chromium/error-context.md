# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> boolean settings render as the canonical control-switch and keep checkbox semantics
- Location: e2e\settings-modal.spec.ts:459:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' }).locator('input[name="setup_memory_auto_capture"]')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' }).locator('input[name="setup_memory_auto_capture"]')

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
  437 |     expect(await capture().isChecked()).toBe(initial)
  438 |   })
  439 | 
  440 |   test('mobile: full-screen dialog with horizontal section chips', async ({ page }) => {
  441 |     await page.setViewportSize({ width: 390, height: 844 })
  442 |     await page.goto(CONTROL_URL + 'config')
  443 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  444 |     await expect(dialog(page)).toBeVisible()
  445 | 
  446 |     const modalBox = await dialog(page).boundingBox()
  447 |     expect(modalBox?.width).toBe(390)
  448 | 
  449 |     const rail = dialog(page).getByRole('tablist', { name: 'Settings sections' })
  450 |     await expect(rail).toHaveAttribute('aria-orientation', 'horizontal')
  451 |     await railTab(page, 'Capabilities').click()
  452 |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  453 | 
  454 |     // No horizontal scroll on the page at 390px.
  455 |     const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
  456 |     expect(overflow).toBeLessThanOrEqual(0)
  457 |   })
  458 | 
  459 |   test('boolean settings render as the canonical control-switch and keep checkbox semantics', async ({ page }) => {
  460 |     await openFromSidebar(page)
  461 |     await railTab(page, 'Capabilities').click()
  462 | 
  463 |     const capture = dialog(page).locator('input[name="setup_memory_auto_capture"]')
> 464 |     await expect(capture).toBeVisible()
      |                           ^ Error: expect(locator).toBeVisible() failed
  465 | 
  466 |     // It carries the shared switch primitive, not a raw browser checkbox, and
  467 |     // is exposed to assistive tech as a switch.
  468 |     await expect(capture).toHaveClass(/control-switch/)
  469 |     await expect(capture).toHaveAttribute('role', 'switch')
  470 | 
  471 |     // The primitive CSS actually loaded and applied end-to-end (built bundle →
  472 |     // gateway → page): the checkbox is restyled (appearance:none) and sized as
  473 |     // a 36px switch track.
  474 |     const appearance = await capture.evaluate((el) => getComputedStyle(el).appearance)
  475 |     expect(appearance).toBe('none')
  476 |     const box = await capture.boundingBox()
  477 |     expect(box?.width).toBeGreaterThanOrEqual(34)
  478 |     expect(box?.width).toBeLessThanOrEqual(40)
  479 | 
  480 |     // Native checkbox semantics are preserved: it stays a real, keyboard-
  481 |     // operable input (Space toggles, :checked round-trips).
  482 |     const before = await capture.isChecked()
  483 |     await capture.focus()
  484 |     await page.keyboard.press('Space')
  485 |     expect(await capture.isChecked()).toBe(!before)
  486 |     await page.keyboard.press('Space')
  487 |     expect(await capture.isChecked()).toBe(before)
  488 |   })
  489 | 
  490 |   test('every boolean control in Capabilities uses the switch primitive (no raw checkboxes)', async ({ page }) => {
  491 |     await openFromSidebar(page)
  492 |     await railTab(page, 'Capabilities').click()
  493 |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  494 | 
  495 |     // Open the advanced search disclosure so its toggles render.
  496 |     const advanced = dialog(page).getByText('Advanced search options')
  497 |     if (await advanced.count()) await advanced.click()
  498 | 
  499 |     const checkboxes = dialog(page).locator('input[type="checkbox"]')
  500 |     const total = await checkboxes.count()
  501 |     expect(total).toBeGreaterThan(0)
  502 |     const switches = dialog(page).locator('input[type="checkbox"].control-switch')
  503 |     expect(await switches.count()).toBe(total)
  504 |   })
  505 | 
  506 |   test('Appearance section applies theme instantly without a dirty bar', async ({ page }) => {
  507 |     await openFromSidebar(page)
  508 | 
  509 |     const appearanceTab = dialog(page).getByRole('tab', { name: 'Appearance' })
  510 |     await expect(appearanceTab).toBeVisible()
  511 |     await appearanceTab.click()
  512 |     await expect(appearanceTab).toHaveAttribute('aria-selected', 'true')
  513 |     await expect(dialog(page).getByRole('heading', { name: 'Appearance' })).toBeVisible()
  514 | 
  515 |     // Theme radios flip the live document theme with no save step, and persist
  516 |     // to the same store the sidebar shortcut reads.
  517 |     const dark = dialog(page).getByRole('radio', { name: 'Dark' })
  518 |     const light = dialog(page).getByRole('radio', { name: 'Light' })
  519 |     await dark.click()
  520 |     await expect(dark).toBeChecked()
  521 |     await expect.poll(() => page.evaluate(() => document.documentElement.getAttribute('data-theme'))).toBe('dark')
  522 |     await light.click()
  523 |     await expect.poll(() => page.evaluate(() => document.documentElement.getAttribute('data-theme'))).toBe('light')
  524 |     expect(await page.evaluate(() => localStorage.getItem('marguerite-theme'))).toBe('light')
  525 | 
  526 |     // Visual effects are a low-frequency browser preference and live here
  527 |     // instead of adding another popover to the chat composer.
  528 |     const visualEffects = dialog(page).getByRole('switch', { name: 'Visual effects' })
  529 |     await expect(visualEffects).toBeVisible()
  530 |     await visualEffects.click()
  531 |     await expect(visualEffects).not.toBeChecked()
  532 |     expect(await page.evaluate(() => (
  533 |       JSON.parse(localStorage.getItem('marguerite.routerFx') || '{}').enabled
  534 |     ))).toBe(false)
  535 | 
  536 |     // Client-only: theme changes never raise the settings dirty bar.
  537 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  538 |   })
  539 | 
  540 |   test('Appearance persists the tool-detail default without a save step', async ({ page }) => {
  541 |     await openFromSidebar(page)
  542 |     await dialog(page).getByRole('tab', { name: 'Appearance' }).click()
  543 | 
  544 |     const auto = dialog(page).getByRole('radio', { name: 'Auto', exact: true })
  545 |     const compact = dialog(page).getByRole('radio', { name: 'Compact', exact: true })
  546 |     await expect(auto).toBeChecked()
  547 | 
  548 |     await compact.click()
  549 |     await expect(compact).toBeChecked()
  550 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  551 |     expect(await page.evaluate(() => (
  552 |       localStorage.getItem('marguerite.appearance.toolDetails.v1')
  553 |     ))).toBe('compact')
  554 | 
  555 |     await page.reload()
  556 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  557 |     await expect(dialog(page)).toBeVisible()
  558 |     await expect(dialog(page).getByRole('radio', { name: 'Compact', exact: true })).toBeChecked()
  559 | 
  560 |     await dialog(page).getByRole('radio', { name: 'Auto', exact: true }).click()
  561 |   })
  562 | 
  563 |   test('Advanced section surfaces homeless flags, applies instantly without a dirty bar', async ({ page }) => {
  564 |     await openFromSidebar(page)
```