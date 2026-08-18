# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> Appearance section applies theme instantly without a dirty bar
- Location: e2e\settings-modal.spec.ts:506:3

# Error details

```
Error: expect(received).toBe(expected) // Object.is equality

Expected: "light"
Received: null
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
      - main [ref=e61]
    - status [ref=e62]
  - dialog "Settings" [ref=e64]:
    - generic [ref=e65]:
      - tablist "Settings sections" [ref=e66]:
        - generic [ref=e67]: Gateway
        - 'tab "Connection: Connected" [ref=e68] [cursor=pointer]':
          - img [ref=e70]
          - generic [ref=e73]: Connection
        - generic [ref=e75]: AI configuration
        - 'tab "Model Service: Ready" [ref=e76] [cursor=pointer]':
          - img [ref=e78]
          - generic [ref=e84]: Model Service
        - 'tab "Model Routing: Ready" [ref=e85] [cursor=pointer]':
          - img [ref=e87]
          - generic [ref=e93]: Model Routing
        - generic [ref=e94]: Capabilities
        - 'tab "Capabilities: Optional" [ref=e95] [cursor=pointer]':
          - img [ref=e97]
          - generic [ref=e99]: Capabilities
        - generic [ref=e100]: Preferences
        - 'tab "Behavior: Applies on save" [ref=e101] [cursor=pointer]':
          - img [ref=e103]
          - generic [ref=e105]: Behavior
        - 'tab "Privacy: Applies on save" [ref=e106] [cursor=pointer]':
          - img [ref=e108]
          - generic [ref=e110]: Privacy
        - tab "Sandbox" [ref=e111] [cursor=pointer]:
          - img [ref=e113]
          - generic [ref=e115]: Sandbox
        - tab "Memory & Profile" [ref=e116] [cursor=pointer]:
          - img [ref=e118]
          - generic [ref=e121]: Memory & Profile
        - tab "Appearance" [selected] [ref=e122] [cursor=pointer]:
          - img [ref=e124]
          - generic [ref=e126]: Appearance
        - tab "Keyboard" [ref=e127] [cursor=pointer]:
          - img [ref=e129]
          - generic [ref=e131]: Keyboard
        - tab "Advanced" [ref=e132] [cursor=pointer]:
          - img [ref=e134]
          - generic [ref=e137]: Advanced
      - generic [ref=e138]:
        - banner [ref=e139]:
          - heading "Settings" [level=2] [ref=e140]
          - button "Close" [ref=e141] [cursor=pointer]:
            - img [ref=e143]
        - tabpanel "Appearance" [ref=e146]:
          - group [ref=e147]:
            - generic [ref=e148]:
              - generic [ref=e149]:
                - heading "Appearance" [level=3] [ref=e150]
                - paragraph [ref=e151]: Theme, language, and browser-local interface preferences. Presets apply instantly; custom widths use Apply.
              - generic [ref=e152]:
                - generic [ref=e153]:
                  - generic [ref=e154]: Theme
                  - generic [ref=e155]: Follow your system, or force light or dark. The sidebar toggle is a shortcut to this same setting.
                - radiogroup "Theme" [ref=e157]:
                  - generic [ref=e158] [cursor=pointer]:
                    - radio "Light" [checked] [active] [ref=e159]
                    - img [ref=e161]
                    - generic [ref=e167]: Light
                  - generic [ref=e168] [cursor=pointer]:
                    - radio "Dark" [ref=e169]
                    - img [ref=e171]
                    - generic [ref=e173]: Dark
                  - generic [ref=e174] [cursor=pointer]:
                    - radio "Arctic" [ref=e175]
                    - img [ref=e177]
                    - generic [ref=e179]: Arctic
                  - generic [ref=e180] [cursor=pointer]:
                    - radio "CRT Green" [ref=e181]
                    - img [ref=e183]
                    - generic [ref=e185]: CRT Green
                  - generic [ref=e186] [cursor=pointer]:
                    - radio "Ember" [ref=e187]
                    - img [ref=e189]
                    - generic [ref=e192]: Ember
                  - generic [ref=e193] [cursor=pointer]:
                    - radio "Miami" [ref=e194]
                    - img [ref=e196]
                    - generic [ref=e202]: Miami
                  - generic [ref=e203] [cursor=pointer]:
                    - radio "Synthwave" [ref=e204]
                    - img [ref=e206]
                    - generic [ref=e208]: Synthwave
                  - generic [ref=e209] [cursor=pointer]:
                    - radio "Terminal" [ref=e210]
                    - img [ref=e212]
                    - generic [ref=e215]: Terminal
                  - generic [ref=e216] [cursor=pointer]:
                    - radio "Vapor" [ref=e217]
                    - img [ref=e219]
                    - generic [ref=e221]: Vapor
                  - generic [ref=e222] [cursor=pointer]:
                    - radio "System" [ref=e223]
                    - img [ref=e225]
                    - generic [ref=e227]: System
              - generic [ref=e228]:
                - generic [ref=e229]:
                  - generic [ref=e230]: Language
                  - generic [ref=e231]: Display language for the interface and channel system notices. Applies instantly here and saves the Gateway-wide notice language.
                - radiogroup "Language" [ref=e233]:
                  - generic [ref=e234] [cursor=pointer]:
                    - radio "English" [checked] [ref=e235]
                    - generic [ref=e236]: English
                  - generic [ref=e237] [cursor=pointer]:
                    - radio "中文" [ref=e238]
                    - generic [ref=e239]: 中文
                  - generic [ref=e240] [cursor=pointer]:
                    - radio "日本語" [ref=e241]
                    - generic [ref=e242]: 日本語
                  - generic [ref=e243] [cursor=pointer]:
                    - radio "Français" [ref=e244]
                    - generic [ref=e245]: Français
                  - generic [ref=e246] [cursor=pointer]:
                    - radio "Deutsch" [ref=e247]
                    - generic [ref=e248]: Deutsch
                  - generic [ref=e249] [cursor=pointer]:
                    - radio "Español" [ref=e250]
                    - generic [ref=e251]: Español
              - generic [ref=e252]:
                - generic [ref=e253]:
                  - generic [ref=e254]: Tool details by default
                  - generic [ref=e255]: Auto follows the tool type; Compact collapses ordinary tools; Expanded opens them. Errors open automatically by default and can still be collapsed.
                - radiogroup "Tool details by default" [ref=e257]:
                  - generic [ref=e258] [cursor=pointer]:
                    - radio "Auto" [checked] [ref=e259]
                    - generic [ref=e260]: Auto
                  - generic [ref=e261] [cursor=pointer]:
                    - radio "Compact" [ref=e262]
                    - generic [ref=e263]: Compact
                  - generic [ref=e264] [cursor=pointer]:
                    - radio "Expanded" [ref=e265]
                    - generic [ref=e266]: Expanded
              - generic [ref=e267]:
                - generic [ref=e268]:
                  - generic [ref=e269]: Visual effects
                  - generic [ref=e270]: Animate model-routing decisions and answer transitions in conversations.
                - switch "Visual effects" [checked] [ref=e272] [cursor=pointer]
              - generic [ref=e273]:
                - generic [ref=e274]:
                  - generic [ref=e275]: Floating composer
                  - generic [ref=e276]: Float the input over the transcript and collapse it to a single line while scrolling up.
                - switch "Floating composer" [checked] [ref=e278] [cursor=pointer]
              - generic [ref=e279]:
                - generic [ref=e280]:
                  - generic [ref=e281]: Sidebar width
                  - generic [ref=e282]: Choose the desktop sidebar width. Compact and drawer layouts use a fixed width.
                - generic [ref=e283]:
                  - radiogroup "Sidebar width" [ref=e284]:
                    - generic [ref=e285] [cursor=pointer]:
                      - radio "Compact 240px" [ref=e286]
                      - generic [ref=e287]: Compact
                      - generic [ref=e288]: 240px
                    - generic [ref=e289] [cursor=pointer]:
                      - radio "Default 260px" [checked] [ref=e290]
                      - generic [ref=e291]: Default
                      - generic [ref=e292]: 260px
                    - generic [ref=e293] [cursor=pointer]:
                      - radio "Wide 360px" [ref=e294]
                      - generic [ref=e295]: Wide
                      - generic [ref=e296]: 360px
                    - generic [ref=e297] [cursor=pointer]:
                      - radio "Custom" [ref=e298]
                      - generic [ref=e299]: Custom
                  - generic [ref=e300]:
                    - generic [ref=e301]:
                      - generic [ref=e302]: Custom width
                      - generic [ref=e303]: Use the minus and plus buttons or enter a value, then apply it.
                    - generic [ref=e304]:
                      - button "Decrease sidebar width" [disabled] [ref=e305]: −
                      - generic [ref=e306]:
                        - spinbutton "Custom width" [disabled] [ref=e307]: "260"
                        - generic [ref=e308]: px
                      - button "Increase sidebar width" [disabled] [ref=e309]: +
                      - button "Apply" [disabled] [ref=e310]
                    - generic [ref=e311]: Enter a whole number from 240 to 480.
                  - generic [ref=e312]:
                    - generic [ref=e313]: Resizable
                    - generic [ref=e314]: "Current desktop width: 260px."
              - generic [ref=e315]:
                - generic [ref=e316]:
                  - generic [ref=e317]: Background music
                  - generic [ref=e318]: Show the background-music control in the top bar. Off by default; also toggleable from the command palette.
                - switch "Background music" [ref=e320] [cursor=pointer]
    - contentinfo [ref=e321]:
      - generic [ref=e322]: More options live in
      - code [ref=e323]: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
      - button "Copy config path" [ref=e324] [cursor=pointer]:
        - img [ref=e326]
      - generic [ref=e329]: ·
      - generic [ref=e330]: Most changes apply live; some need a gateway restart
```

# Test source

```ts
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
  464 |     await expect(capture).toBeVisible()
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
> 524 |     expect(await page.evaluate(() => localStorage.getItem('opensquilla-theme'))).toBe('light')
      |                                                                                  ^ Error: expect(received).toBe(expected) // Object.is equality
  525 | 
  526 |     // Visual effects are a low-frequency browser preference and live here
  527 |     // instead of adding another popover to the chat composer.
  528 |     const visualEffects = dialog(page).getByRole('switch', { name: 'Visual effects' })
  529 |     await expect(visualEffects).toBeVisible()
  530 |     await visualEffects.click()
  531 |     await expect(visualEffects).not.toBeChecked()
  532 |     expect(await page.evaluate(() => (
  533 |       JSON.parse(localStorage.getItem('opensquilla.routerFx') || '{}').enabled
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
  552 |       localStorage.getItem('opensquilla.appearance.toolDetails.v1')
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
  569 |     await expect(dialog(page).getByRole('heading', { name: 'Advanced' })).toBeVisible()
  570 | 
  571 |     // A boolean Labs flag is the shared switch and writes its raw '1'/'0' key.
  572 |     const poll = dialog(page).locator('input[name="labs_approval_poll"]')
  573 |     await expect(poll).toHaveAttribute('role', 'switch')
  574 |     const before = await poll.isChecked()
  575 |     await poll.click()
  576 |     expect(await page.evaluate(() => localStorage.getItem('opensquilla.chat.approvalPoll'))).toBe(before ? '0' : '1')
  577 | 
  578 |     // The answer-reveal window persists as a validated "min,max" string.
  579 |     const min = dialog(page).locator('input[name="labs_reveal_min"]')
  580 |     const max = dialog(page).locator('input[name="labs_reveal_max"]')
  581 |     await min.fill('1000')
  582 |     await max.fill('3000')
  583 |     await max.blur()
  584 |     expect(await page.evaluate(() => localStorage.getItem('opensquilla.chat.answerReveal'))).toBe('1000,3000')
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