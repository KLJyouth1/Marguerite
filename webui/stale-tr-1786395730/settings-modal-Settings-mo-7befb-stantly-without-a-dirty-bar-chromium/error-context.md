# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> Advanced section surfaces homeless flags, applies instantly without a dirty bar
- Location: e2e\settings-modal.spec.ts:563:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' }).getByRole('heading', { name: 'Advanced' })
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' }).getByRole('heading', { name: 'Advanced' })

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
    - 'tab "Capabilities: Optional"': Capabilities
    - 'tab "Behavior: Applies on save"': Behavior
    - 'tab "Privacy: Applies on save"': Privacy
    - tab "Sandbox"
    - tab "Memory & Profile"
    - tab "Appearance"
    - tab "Keyboard"
    - tab "Advanced" [selected]
  - banner:
    - heading "Settings" [level=2]
    - button "Close":
      - img
  - tabpanel "Advanced":
    - group:
      - status:
        - status "Loading"
        - strong: Advanced
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
  565 |     const advTab = dialog(page).getByRole('tab', { name: 'Advanced' })
  566 |     await expect(advTab).toBeVisible()
  567 |     await advTab.click()
  568 |     await expect(advTab).toHaveAttribute('aria-selected', 'true')
> 569 |     await expect(dialog(page).getByRole('heading', { name: 'Advanced' })).toBeVisible()
      |                                                                           ^ Error: expect(locator).toBeVisible() failed
  570 | 
  571 |     // A boolean Labs flag is the shared switch and writes its raw '1'/'0' key.
  572 |     const poll = dialog(page).locator('input[name="labs_approval_poll"]')
  573 |     await expect(poll).toHaveAttribute('role', 'switch')
  574 |     const before = await poll.isChecked()
  575 |     await poll.click()
  576 |     expect(await page.evaluate(() => localStorage.getItem('marguerite.chat.approvalPoll'))).toBe(before ? '0' : '1')
  577 | 
  578 |     // The answer-reveal window persists as a validated "min,max" string.
  579 |     const min = dialog(page).locator('input[name="labs_reveal_min"]')
  580 |     const max = dialog(page).locator('input[name="labs_reveal_max"]')
  581 |     await min.fill('1000')
  582 |     await max.fill('3000')
  583 |     await max.blur()
  584 |     expect(await page.evaluate(() => localStorage.getItem('marguerite.chat.answerReveal'))).toBe('1000,3000')
  585 | 
  586 |     // Client-only: none of this raises the settings dirty bar.
  587 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  588 | 
  589 |     // Long-lived Agent management is available only from this advanced escape
  590 |     // hatch; routing through Vue preserves the /control base path.
  591 |     await dialog(page).getByRole('button', { name: 'Open: Agent configuration (advanced)' }).click()
  592 |     await expect(page).toHaveURL(/\/agents$/)
  593 |     await expect(dialog(page)).toHaveCount(0)
  594 |     await expect(page.locator('.agents-view, .agents-page, .ag-stage').first()).toBeVisible()
  595 |     await expect(page.getByRole('heading', { name: 'Agents', level: 1 })).toBeFocused()
  596 |   })
  597 | })
  598 | 
```