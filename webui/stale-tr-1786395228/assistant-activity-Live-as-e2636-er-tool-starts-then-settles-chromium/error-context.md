# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: assistant-activity.spec.ts >> Live assistant activity lifecycle >> moves draft text back into activity when a later tool starts, then settles
- Location: e2e\assistant-activity.spec.ts:402:3

# Error details

```
Error: expect(locator).toHaveAttribute(expected) failed

Locator:  locator('.assistant-activity--live').locator('.assistant-activity__live-head')
Expected: "false"
Received: "true"
Timeout:  5000ms

Call log:
  - Expect "toHaveAttribute" with timeout 5000ms
  - waiting for locator('.assistant-activity--live').locator('.assistant-activity__live-head')
    14 × locator resolved to <button type="button" data-v-343e1a50="" aria-expanded="true" data-share-control="" data-share-activity-label="" class="assistant-activity__live-head" aria-controls="assistant-activity-body-v-0">…</button>
       - unexpected value "true"

```

```yaml
- button [expanded]:
  - status: Working
```

# Test source

```ts
  317 |     await expect(detailTrigger).toHaveAttribute('aria-label', /view details/i)
  318 |     await expect(details).not.toContainText('INPUT')
  319 |     await expect(details).not.toContainText('RESULT')
  320 |     await expect(details).not.toContainText('/private/')
  321 |     await expect(activity.locator('.tool-row-section')).toHaveCount(0)
  322 |     const flatStyles = await activity.evaluate((element) => {
  323 |       const readAll = (selector: string) =>
  324 |         Array.from(element.querySelectorAll<HTMLElement>(selector)).map((target) => {
  325 |           const style = getComputedStyle(target)
  326 |           return {
  327 |             backgroundColor: style.backgroundColor,
  328 |             borderWidths: [
  329 |               style.borderTopWidth,
  330 |               style.borderRightWidth,
  331 |               style.borderBottomWidth,
  332 |               style.borderLeftWidth,
  333 |             ],
  334 |             borderRadius: style.borderRadius,
  335 |             boxShadow: style.boxShadow,
  336 |           }
  337 |         })
  338 |       return {
  339 |         cards: readAll('.step-card'),
  340 |         rows: readAll('.tool-row'),
  341 |         details: readAll('.activity-tool-details'),
  342 |       }
  343 |     })
  344 |     for (const style of [
  345 |       ...flatStyles.cards,
  346 |       ...flatStyles.rows,
  347 |       ...flatStyles.details,
  348 |     ]) {
  349 |       expect(style).toMatchObject({
  350 |         backgroundColor: 'rgba(0, 0, 0, 0)',
  351 |         borderWidths: ['0px', '0px', '0px', '0px'],
  352 |         borderRadius: '0px',
  353 |         boxShadow: 'none',
  354 |       })
  355 |     }
  356 | 
  357 |     await summary.press('Space')
  358 |     await expect(summary).toHaveAttribute('aria-expanded', 'false')
  359 |     await expect(activity).toHaveAttribute('data-share-expanded', 'false')
  360 |     expect(await summary.evaluate(element => document.activeElement === element)).toBe(true)
  361 |     await expect(answer).toBeVisible()
  362 |   })
  363 | 
  364 |   test('omits failed work from the activity disclosure', async ({ page }) => {
  365 |     await mockActivityHistory(page, { failed: true })
  366 |     await page.setViewportSize({ width: 320, height: 844 })
  367 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(`${SESSION_KEY}-failed`))
  368 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  369 | 
  370 |     const activity = page.getByTestId('assistant-activity')
  371 |     await expect(activity).toBeVisible()
  372 |     await expect(activity).toHaveAttribute('data-share-expanded', 'false')
  373 |     await expect(activity).not.toContainText('failure recovered')
  374 | 
  375 |     const errorRow = activity.locator('.tool-row--error')
  376 |     await expect(errorRow).toHaveCount(0)
  377 |     const summary = activity.locator('.assistant-activity__summary')
  378 |     await summary.press('Enter')
  379 |     await expect(activity).toHaveAttribute('data-share-expanded', 'true')
  380 |     await expect(errorRow).toHaveCount(0)
  381 |     await expect(activity).not.toContainText('Search service unavailable')
  382 |     await expect(activity.locator('.tool-row-section--error')).toHaveCount(0)
  383 |     await expect(
  384 |       page.getByText('The canonical answer is complete.', { exact: true }),
  385 |     ).toBeVisible()
  386 | 
  387 |     await activity.locator('.assistant-activity__label').evaluate((element) => {
  388 |       element.textContent = 'Sehr lange lokalisierte Aktivitätszusammenfassung'
  389 |     })
  390 |     const summaryOverflow = await summary.evaluate(element =>
  391 |       element.scrollWidth - element.clientWidth,
  392 |     )
  393 |     expect(summaryOverflow).toBeLessThanOrEqual(1)
  394 |     const pageOverflow = await page.evaluate(() =>
  395 |       document.documentElement.scrollWidth - document.documentElement.clientWidth,
  396 |     )
  397 |     expect(pageOverflow).toBeLessThanOrEqual(1)
  398 |   })
  399 | })
  400 | 
  401 | test.describe('Live assistant activity lifecycle', () => {
  402 |   test('moves draft text back into activity when a later tool starts, then settles', async ({ page }) => {
  403 |     await page.emulateMedia({ reducedMotion: 'no-preference' })
  404 |     const lifecycle = await mockControlledActivityLifecycle(page)
  405 |     await page.goto(
  406 |       CONTROL_URL + 'chat?session=' + encodeURIComponent(LIFECYCLE_SESSION_KEY),
  407 |     )
  408 |     await expect(page.locator('.conn-pill.connected')).toBeVisible({ timeout: 10000 })
  409 | 
  410 |     await page.locator('.chat-textarea').fill('Inspect, draft, verify, and answer.')
  411 |     await page.locator('.chat-send-btn[aria-label="Send"]').click()
  412 | 
  413 |     const liveActivity = page.locator('.assistant-activity--live')
  414 |     await expect(liveActivity).toBeVisible()
  415 |     await expect(page.locator('.work-card')).toHaveCount(0)
  416 |     const liveSummary = liveActivity.locator('.assistant-activity__live-head')
> 417 |     await expect(liveSummary).toHaveAttribute('aria-expanded', 'false')
      |                               ^ Error: expect(locator).toHaveAttribute(expected) failed
  418 |     await liveSummary.click()
  419 |     await expect(liveSummary).toHaveAttribute('aria-expanded', 'true')
  420 |     const liveStatus = liveActivity.locator('.assistant-activity__live-label')
  421 |     await expect(liveStatus).toHaveText('Working')
  422 |     await expect(liveStatus).toHaveAttribute('role', 'status')
  423 |     await expect(liveStatus).toHaveAttribute('aria-live', 'polite')
  424 |     await expect(liveStatus).toHaveAttribute('aria-atomic', 'true')
  425 |     await expect(liveActivity.getByText('Working', { exact: true })).toHaveCount(1)
  426 |     // The phase label is the only polite live region. Failed work is omitted
  427 |     // from the disclosure, so it must not mount a second announcement region.
  428 |     await expect(liveActivity.locator('[role="status"]')).toHaveCount(1)
  429 |     await expect(liveActivity.locator('.assistant-activity__live-failure')).toHaveCount(0)
  430 |     await expect(liveActivity.locator('.assistant-activity-status__row')).toHaveCount(0)
  431 |     const liveMotion = await liveActivity.evaluate((element) => ({
  432 |       dot: getComputedStyle(
  433 |         element.querySelector<HTMLElement>('.assistant-activity__live-dot')!,
  434 |       ).animationName,
  435 |       label: getComputedStyle(
  436 |         element.querySelector<HTMLElement>('.assistant-activity__live-label')!,
  437 |       ).animationName,
  438 |     }))
  439 |     // The pulsing dot is the single working signal; the label is deliberately
  440 |     // static (the shimmer gradient was removed as visual noise).
  441 |     expect(liveMotion.dot).not.toBe('none')
  442 |     expect(liveMotion.label).toBe('none')
  443 | 
  444 |     lifecycle.emit('session.event.tool_use_start', {
  445 |       tool_use_id: 'activity-inspect',
  446 |       name: 'read_file',
  447 |       input: { path: '/private/project/chat.ts' },
  448 |     })
  449 |     const inspectRow = liveActivity.locator('.tool-row[data-op="file.inspect"]')
  450 |     await expect(inspectRow).toBeVisible()
  451 |     await expect(inspectRow).toHaveAttribute('aria-expanded', 'false')
  452 |     await expect(inspectRow.locator('.tool-row__activity-arrow')).toHaveCount(1)
  453 |     await expect(inspectRow.locator('.tool-row__bullet')).toHaveCount(0)
  454 |     await expect(liveActivity).not.toContainText('/private/project/chat.ts')
  455 |     await expect(liveStatus).toHaveText('Working')
  456 |     // A cluster still in flight reads in the present tense; it settles into
  457 |     // the past tense once its result lands.
  458 |     await expect(liveActivity.getByText('Inspecting files', { exact: true })).toHaveCount(1)
  459 | 
  460 |     lifecycle.emit('session.event.tool_result', {
  461 |       tool_use_id: 'activity-inspect',
  462 |       name: 'read_file',
  463 |       input: { path: '/private/project/chat.ts' },
  464 |       result: 'read',
  465 |       execution_status: { status: 'success' },
  466 |     })
  467 |     await expect(liveActivity.getByText('Inspected files', { exact: true })).toHaveCount(1)
  468 |     lifecycle.emit('session.event.text_delta', { text: 'Draft candidate.' })
  469 | 
  470 |     const answerCandidate = page.locator('.live-answer-candidate')
  471 |     await expect(answerCandidate).toHaveText('Draft candidate.')
  472 |     await expect(liveActivity.getByText('Draft candidate.', { exact: true })).toHaveCount(0)
  473 |     await expect(liveStatus).toHaveText('Writing the answer')
  474 |     await expect(
  475 |       liveActivity.getByText('Writing the answer', { exact: true }),
  476 |     ).toHaveCount(1)
  477 |     await expect(liveActivity.locator('.assistant-activity-status__row')).toHaveCount(0)
  478 | 
  479 |     lifecycle.emit('session.event.tool_use_start', {
  480 |       tool_use_id: 'activity-verify',
  481 |       name: 'bash_exec',
  482 |       input: { command: 'npm test' },
  483 |     })
  484 |     await expect(answerCandidate).toHaveCount(0)
  485 |     await expect(liveActivity.getByText('Draft candidate.', { exact: true })).toBeVisible()
  486 |     await expect(liveActivity.locator('.tool-row[data-op="command.run"]')).toBeVisible()
  487 |     await expect(liveStatus).toHaveText('Working')
  488 |     await expect(liveActivity.getByText('Running commands', { exact: true })).toHaveCount(1)
  489 | 
  490 |     lifecycle.emit('session.event.tool_result', {
  491 |       tool_use_id: 'activity-verify',
  492 |       name: 'bash_exec',
  493 |       input: { command: 'npm test' },
  494 |       result: 'verified',
  495 |       execution_status: { status: 'success' },
  496 |     })
  497 |     lifecycle.emit('session.event.text_delta', { text: 'Final verified answer.' })
  498 |     await expect(answerCandidate).toHaveText('Final verified answer.')
  499 |     await expect(liveActivity).toBeVisible()
  500 | 
  501 |     lifecycle.finish()
  502 |     await expect(liveActivity).toHaveCount(0)
  503 |     const settled = page.locator('.msg-ai .assistant-activity')
  504 |     await expect(settled).toBeVisible()
  505 |     await expect(settled).toHaveAttribute('data-share-expanded', 'false')
  506 |     const finalAnswer = page.locator('.msg-ai-text').filter({
  507 |       hasText: 'Final verified answer.',
  508 |     })
  509 |     await expect(finalAnswer).toBeVisible()
  510 |     expect(await finalAnswer.evaluate(element =>
  511 |       element.closest('.assistant-activity') === null,
  512 |     )).toBe(true)
  513 |     await settled.locator('.assistant-activity__summary').click()
  514 |     await expect(settled.getByText('Draft candidate.', { exact: true })).toBeVisible()
  515 |     await expect(
  516 |       settled.getByText('Final verified answer.', { exact: true }),
  517 |     ).toHaveCount(0)
```