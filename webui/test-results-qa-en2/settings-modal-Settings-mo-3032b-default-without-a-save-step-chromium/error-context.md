# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> Appearance persists the tool-detail default without a save step
- Location: e2e\settings-modal.spec.ts:540:3

# Error details

```
Error: expect(received).toBe(expected) // Object.is equality

Expected: "compact"
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
        - 'tab "Model Service: Ready" [ref=e72] [cursor=pointer]':
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
        - tab "Appearance" [selected] [ref=e118] [cursor=pointer]:
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
          - button "Close" [ref=e137] [cursor=pointer]:
            - img [ref=e139]
        - tabpanel "Appearance" [ref=e142]:
          - group [ref=e143]:
            - generic [ref=e144]:
              - generic [ref=e145]:
                - heading "Appearance" [level=3] [ref=e146]
                - paragraph [ref=e147]: Theme, language, and browser-local interface preferences. Presets apply instantly; custom widths use Apply.
              - generic [ref=e148]:
                - generic [ref=e149]:
                  - generic [ref=e150]: Theme
                  - generic [ref=e151]: Follow your system, or force light or dark. The sidebar toggle is a shortcut to this same setting.
                - radiogroup "Theme" [ref=e153]:
                  - generic [ref=e154] [cursor=pointer]:
                    - radio "Light" [ref=e155]
                    - img [ref=e157]
                    - generic [ref=e163]: Light
                  - generic [ref=e164] [cursor=pointer]:
                    - radio "Dark" [ref=e165]
                    - img [ref=e167]
                    - generic [ref=e169]: Dark
                  - generic [ref=e170] [cursor=pointer]:
                    - radio "Arctic" [ref=e171]
                    - img [ref=e173]
                    - generic [ref=e175]: Arctic
                  - generic [ref=e176] [cursor=pointer]:
                    - radio "CRT Green" [ref=e177]
                    - img [ref=e179]
                    - generic [ref=e181]: CRT Green
                  - generic [ref=e182] [cursor=pointer]:
                    - radio "Ember" [ref=e183]
                    - img [ref=e185]
                    - generic [ref=e188]: Ember
                  - generic [ref=e189] [cursor=pointer]:
                    - radio "Miami" [ref=e190]
                    - img [ref=e192]
                    - generic [ref=e198]: Miami
                  - generic [ref=e199] [cursor=pointer]:
                    - radio "Synthwave" [ref=e200]
                    - img [ref=e202]
                    - generic [ref=e204]: Synthwave
                  - generic [ref=e205] [cursor=pointer]:
                    - radio "Terminal" [ref=e206]
                    - img [ref=e208]
                    - generic [ref=e211]: Terminal
                  - generic [ref=e212] [cursor=pointer]:
                    - radio "Vapor" [ref=e213]
                    - img [ref=e215]
                    - generic [ref=e217]: Vapor
                  - generic [ref=e218] [cursor=pointer]:
                    - radio "System" [checked] [ref=e219]
                    - img [ref=e221]
                    - generic [ref=e223]: System
              - generic [ref=e224]:
                - generic [ref=e225]:
                  - generic [ref=e226]: Language
                  - generic [ref=e227]: Display language for the interface and channel system notices. Applies instantly here and saves the Gateway-wide notice language.
                - radiogroup "Language" [ref=e229]:
                  - generic [ref=e230] [cursor=pointer]:
                    - radio "English" [checked] [ref=e231]
                    - generic [ref=e232]: English
                  - generic [ref=e233] [cursor=pointer]:
                    - radio "中文" [ref=e234]
                    - generic [ref=e235]: 中文
                  - generic [ref=e236] [cursor=pointer]:
                    - radio "日本語" [ref=e237]
                    - generic [ref=e238]: 日本語
                  - generic [ref=e239] [cursor=pointer]:
                    - radio "Français" [ref=e240]
                    - generic [ref=e241]: Français
                  - generic [ref=e242] [cursor=pointer]:
                    - radio "Deutsch" [ref=e243]
                    - generic [ref=e244]: Deutsch
                  - generic [ref=e245] [cursor=pointer]:
                    - radio "Español" [ref=e246]
                    - generic [ref=e247]: Español
              - generic [ref=e248]:
                - generic [ref=e249]:
                  - generic [ref=e250]: Tool details by default
                  - generic [ref=e251]: Auto follows the tool type; Compact collapses ordinary tools; Expanded opens them. Errors open automatically by default and can still be collapsed.
                - radiogroup "Tool details by default" [ref=e253]:
                  - generic [ref=e254] [cursor=pointer]:
                    - radio "Auto" [ref=e255]
                    - generic [ref=e256]: Auto
                  - generic [ref=e257] [cursor=pointer]:
                    - radio "Compact" [checked] [active] [ref=e258]
                    - generic [ref=e259]: Compact
                  - generic [ref=e260] [cursor=pointer]:
                    - radio "Expanded" [ref=e261]
                    - generic [ref=e262]: Expanded
              - generic [ref=e263]:
                - generic [ref=e264]:
                  - generic [ref=e265]: Visual effects
                  - generic [ref=e266]: Animate model-routing decisions and answer transitions in conversations.
                - switch "Visual effects" [checked] [ref=e268] [cursor=pointer]
              - generic [ref=e269]:
                - generic [ref=e270]:
                  - generic [ref=e271]: Floating composer
                  - generic [ref=e272]: Float the input over the transcript and collapse it to a single line while scrolling up.
                - switch "Floating composer" [checked] [ref=e274] [cursor=pointer]
              - generic [ref=e275]:
                - generic [ref=e276]:
                  - generic [ref=e277]: Sidebar width
                  - generic [ref=e278]: Choose the desktop sidebar width. Compact and drawer layouts use a fixed width.
                - generic [ref=e279]:
                  - radiogroup "Sidebar width" [ref=e280]:
                    - generic [ref=e281] [cursor=pointer]:
                      - radio "Compact 240px" [ref=e282]
                      - generic [ref=e283]: Compact
                      - generic [ref=e284]: 240px
                    - generic [ref=e285] [cursor=pointer]:
                      - radio "Default 260px" [checked] [ref=e286]
                      - generic [ref=e287]: Default
                      - generic [ref=e288]: 260px
                    - generic [ref=e289] [cursor=pointer]:
                      - radio "Wide 360px" [ref=e290]
                      - generic [ref=e291]: Wide
                      - generic [ref=e292]: 360px
                    - generic [ref=e293] [cursor=pointer]:
                      - radio "Custom" [ref=e294]
                      - generic [ref=e295]: Custom
                  - generic [ref=e296]:
                    - generic [ref=e297]:
                      - generic [ref=e298]: Custom width
                      - generic [ref=e299]: Use the minus and plus buttons or enter a value, then apply it.
                    - generic [ref=e300]:
                      - button "Decrease sidebar width" [disabled] [ref=e301]: −
                      - generic [ref=e302]:
                        - spinbutton "Custom width" [disabled] [ref=e303]: "260"
                        - generic [ref=e304]: px
                      - button "Increase sidebar width" [disabled] [ref=e305]: +
                      - button "Apply" [disabled] [ref=e306]
                    - generic [ref=e307]: Enter a whole number from 240 to 480.
                  - generic [ref=e308]:
                    - generic [ref=e309]: Resizable
                    - generic [ref=e310]: "Current desktop width: 260px."
              - generic [ref=e311]:
                - generic [ref=e312]:
                  - generic [ref=e313]: Background music
                  - generic [ref=e314]: Show the background-music control in the top bar. Off by default; also toggleable from the command palette.
                - switch "Background music" [ref=e316] [cursor=pointer]
    - contentinfo [ref=e317]:
      - generic [ref=e318]: More options live in
      - code [ref=e319]: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
      - button "Copy config path" [ref=e320] [cursor=pointer]:
        - img [ref=e322]
      - generic [ref=e325]: ·
      - generic [ref=e326]: Most changes apply live; some need a gateway restart
```

# Test source

```ts
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
  524 |     expect(await page.evaluate(() => localStorage.getItem('opensquilla-theme'))).toBe('light')
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
> 553 |     ))).toBe('compact')
      |         ^ Error: expect(received).toBe(expected) // Object.is equality
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