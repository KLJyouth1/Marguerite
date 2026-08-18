# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> cold deep link to /settings/connection closes home and moves focus to the sidebar
- Location: e2e\settings-modal.spec.ts:284:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
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
> 287 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
      |                ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
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
```