# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> Advanced section surfaces homeless flags, applies instantly without a dirty bar
- Location: e2e\settings-modal.spec.ts:563:3

# Error details

```
Error: expect(received).toBe(expected) // Object.is equality

Expected: "1"
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
        - tab "Appearance" [ref=e118] [cursor=pointer]:
          - img [ref=e120]
          - generic [ref=e122]: Appearance
        - tab "Keyboard" [ref=e123] [cursor=pointer]:
          - img [ref=e125]
          - generic [ref=e127]: Keyboard
        - tab "Advanced" [selected] [ref=e128] [cursor=pointer]:
          - img [ref=e130]
          - generic [ref=e133]: Advanced
      - generic [ref=e134]:
        - banner [ref=e135]:
          - heading "Settings" [level=2] [ref=e136]
          - button "Close" [ref=e137] [cursor=pointer]:
            - img [ref=e139]
        - tabpanel "Advanced" [ref=e142]:
          - group [ref=e143]:
            - generic [ref=e144]:
              - generic [ref=e145]:
                - heading "Advanced" [level=3] [ref=e146]
                - paragraph [ref=e147]:
                  - text: Experimental client preferences for this browser. They apply instantly — no save needed. Items marked
                  - emphasis [ref=e148]: reload
                  - text: take effect after a page refresh.
              - generic [ref=e149]:
                - generic [ref=e150]:
                  - generic [ref=e151]: New live turn renderer
                  - generic [ref=e152]: Build the streaming work card from an ordered event log. Turn off only to temporarily restore the compatibility renderer.
                - generic [ref=e153]:
                  - generic [ref=e154]: reload
                  - switch "New live turn renderer (takes effect after reload)" [checked] [ref=e155] [cursor=pointer]
              - generic [ref=e156]:
                - generic [ref=e157]:
                  - generic [ref=e158]: Answer reveal window (ms)
                  - generic [ref=e159]: Hold the live answer's reveal in a [min, max] window so the router can decide first. min ≤ max.
                - group "Answer reveal window (ms)" [ref=e160]:
                  - spinbutton "Answer reveal minimum milliseconds" [ref=e161]: "1800"
                  - generic [ref=e162]: –
                  - spinbutton "Answer reveal maximum milliseconds" [ref=e163]: "4000"
              - generic [ref=e164]:
                - generic [ref=e165]:
                  - generic [ref=e166]: Approval recovery polling
                  - generic [ref=e167]: Restore the ~2s approvals poll as a self-healing fallback (default off; approvals already stream).
                - switch "Approval recovery polling" [checked] [active] [ref=e169] [cursor=pointer]
              - generic [ref=e170]:
                - generic [ref=e171]: Memory & self-learning
                - generic [ref=e172]:
                  - generic [ref=e173]:
                    - generic [ref=e174]: Memory consolidation (dream) token usage
                    - generic [ref=e175]: Periodically runs a model pass to consolidate session memory (every 24h). Model-routing self-learning relies on it for idle training windows.
                  - switch "Memory consolidation (dream)" [checked] [ref=e177] [cursor=pointer]
                - generic [ref=e178]:
                  - generic [ref=e179]:
                    - generic [ref=e180]: Model-routing self-learning experimental
                    - generic [ref=e181]: Learns routing preferences from your corrections. Fully on-device — stores feature vectors, never raw conversation text.
                  - switch "Model-routing self-learning" [checked] [ref=e183] [cursor=pointer]
                - generic [ref=e186]: "Depends on memory consolidation above: turning this on enables it too; turning consolidation off alone keeps sampling but pauses automatic training."
                - generic [ref=e187]:
                  - generic [ref=e188]:
                    - generic [ref=e189]: Routing model
                    - generic [ref=e190]: Factory baseline
                  - generic [ref=e192]:
                    - generic [ref=e193]: Corrections
                    - generic [ref=e194]:
                      - text: 0 / 200
                      - text: 0 collected in total
                  - generic [ref=e196]:
                    - generic [ref=e197]: Last action
                    - generic [ref=e198]: No samples collected yet
              - generic [ref=e200]:
                - generic [ref=e201]:
                  - generic [ref=e202]: Run-trace drawer in Logs
                  - generic [ref=e203]: Make log lines interactive and open a per-line node-step detail drawer in the Logs view.
                - generic [ref=e204]:
                  - generic [ref=e205]: reload
                  - switch "Run-trace drawer in Logs (takes effect after reload)" [ref=e206] [cursor=pointer]
              - generic [ref=e207]:
                - generic [ref=e208]:
                  - generic [ref=e209]: Agent configuration (advanced)
                  - generic [ref=e210]: Manage existing agent configurations. This area will change in a future multi-agent redesign.
                - 'button "Open: Agent configuration (advanced)" [ref=e212] [cursor=pointer]': Open
              - generic [ref=e213]:
                - generic [ref=e214]:
                  - generic [ref=e215]: Data maintenance
                  - generic [ref=e216]: Review imports and backups. Desktop can also reset or delete local profile data.
                - button "Open data maintenance" [ref=e218] [cursor=pointer]: Manage
    - contentinfo [ref=e219]:
      - generic [ref=e220]: More options live in
      - code [ref=e221]: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
      - button "Copy config path" [ref=e222] [cursor=pointer]:
        - img [ref=e224]
      - generic [ref=e227]: ·
      - generic [ref=e228]: Most changes apply live; some need a gateway restart
```

# Test source

```ts
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
> 576 |     expect(await page.evaluate(() => localStorage.getItem('opensquilla.chat.approvalPoll'))).toBe(before ? '0' : '1')
      |                                                                                              ^ Error: expect(received).toBe(expected) // Object.is equality
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