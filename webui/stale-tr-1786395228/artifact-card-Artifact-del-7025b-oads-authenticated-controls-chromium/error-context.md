# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: artifact-card.spec.ts >> Artifact deliverable cards >> video stays in the message, performs zero initial requests, and loads authenticated controls
- Location: e2e\artifact-card.spec.ts:372:3

# Error details

```
Test timeout of 30000ms exceeded.
```

```
Error: locator.click: Test timeout of 30000ms exceeded.
Call log:
  - waiting for getByRole('button', { name: 'Play video sample.webm' })

```

# Page snapshot

```yaml
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
    - generic "Recent tasks" [ref=e32]:
      - generic [ref=e33]:
        - generic [ref=e34]: Recents
        - generic [ref=e35]: "1"
        - button "Search tasks (Ctrl+K)" [ref=e36] [cursor=pointer]:
          - img [ref=e38]
        - button "Manage tasks" [ref=e41] [cursor=pointer]:
          - img [ref=e43]
      - generic [ref=e49]:
        - text: Recents
        - button "Current task" [ref=e50]:
          - generic [ref=e51]: Current task
        - button "Actions for Current task" [ref=e54] [cursor=pointer]:
          - generic [ref=e55]: ⋯
    - button "Settings" [ref=e57] [cursor=pointer]:
      - img [ref=e59]
      - generic [ref=e62]: Settings
  - separator "Resize sidebar" [ref=e63]:
    - status [ref=e64]
  - generic [ref=e65]:
    - banner [ref=e66]:
      - generic [ref=e68]:
        - generic [ref=e69]:
          - heading "Produce a few deliverables." [level=1] [ref=e70]
          - button "Copy session ID" [ref=e71] [cursor=pointer]:
            - img [ref=e73]
        - generic [ref=e77]:
          - button "Deliverables (1)" [ref=e78] [cursor=pointer]:
            - img [ref=e80]
            - generic [ref=e83]: Deliverables (1)
          - button "Share" [ref=e84] [cursor=pointer]:
            - img [ref=e86]
            - generic [ref=e92]: Share
          - button "Compact context (LLM summary)" [ref=e93] [cursor=pointer]:
            - img [ref=e95]
      - generic [ref=e100]:
        - button "Manage gateway connection" [ref=e101] [cursor=pointer]: Connected
        - button "Language" [ref=e103] [cursor=pointer]:
          - img [ref=e105]
          - generic [ref=e108]: English
        - button "Theme" [ref=e110] [cursor=pointer]:
          - img [ref=e112]
    - main [ref=e115]:
      - generic [ref=e116]:
        - region "Chat conversation" [ref=e119]:
          - generic [ref=e120]:
            - generic [ref=e121]:
              - generic [ref=e123]: Produce a few deliverables.
              - generic [ref=e124]:
                - button "Copy" [ref=e125] [cursor=pointer]:
                  - img [ref=e127]
                - button "Edit" [ref=e131] [cursor=pointer]:
                  - img [ref=e133]
                - time [ref=e136]:
                  - generic [ref=e137]: Aug 11, 2026, 04:36 AM
                  - generic [ref=e138]: ·
                  - generic [ref=e139]: 2m ago
            - generic [ref=e141]:
              - paragraph [ref=e145]: Here you go.
              - figure "sample.webm, WEBM · 1 KB" [ref=e149]:
                - button "Load video preview for sample.webm" [ref=e152] [cursor=pointer]:
                  - generic [ref=e154]: Load video preview
                - generic [ref=e155]:
                  - generic [ref=e156]: sample.webm
                  - generic [ref=e157]: WEBM · 1 KB
                  - button "Download sample.webm" [ref=e158] [cursor=pointer]:
                    - img [ref=e160]
              - generic [ref=e164]:
                - button "Copy" [ref=e165] [cursor=pointer]:
                  - img [ref=e167]
                - button "Regenerate" [ref=e171] [cursor=pointer]:
                  - img [ref=e173]
                - button "Fork from here" [ref=e176] [cursor=pointer]:
                  - img [ref=e178]
                - time [ref=e183]:
                  - generic [ref=e184]: Aug 11, 2026, 04:37 AM
                  - generic [ref=e185]: ·
                  - generic [ref=e186]: 1m ago
        - generic [ref=e189]:
          - generic [ref=e190]:
            - textbox "Message to send" [active] [ref=e192]:
              - /placeholder: Send a message...
            - generic [ref=e194]:
              - generic [ref=e195]:
                - button "Add" [ref=e197] [cursor=pointer]:
                  - img [ref=e199]
                - button "Model routing" [ref=e201] [cursor=pointer]:
                  - img [ref=e203]
                  - generic: NEW
                - button "Execution mode" [ref=e210] [cursor=pointer]:
                  - img [ref=e212]
                - button "More" [ref=e215] [cursor=pointer]:
                  - img [ref=e217]
              - button "Send" [ref=e222] [cursor=pointer]:
                - img [ref=e224]
          - note [ref=e227]: Content generated by AI may be inaccurate. For reference only; verify important information.
        - status [ref=e228]
  - status [ref=e229]
```

# Test source

```ts
  298 |     await expect(preview).toBeVisible()
  299 |     await expect(preview).toHaveAttribute('sandbox', 'allow-scripts')
  300 | 
  301 |     const downloadPromise = page.waitForEvent('download')
  302 |     await htmlCard.getByRole('button', { name: 'Download interactive.html' }).click()
  303 |     const download = await downloadPromise
  304 | 
  305 |     expect(download.suggestedFilename()).toBe('interactive.html')
  306 |     expect(nativeOpenCount).toBe(0)
  307 |     expect(htmlDownloadCount).toBe(2)
  308 |   })
  309 | 
  310 |   test('audio performs zero initial requests and fetches authenticated bytes only after Play', async ({ page }) => {
  311 |     const requests: Array<{
  312 |       authorization?: string
  313 |       sessionKey?: string
  314 |     }> = []
  315 |     await page.addInitScript(() => {
  316 |       sessionStorage.setItem('marguerite.wsToken', 'audio-token-e2e')
  317 |     })
  318 |     await page.route('**/api/v1/artifacts/art-card-audio*', route => {
  319 |       const request = route.request()
  320 |       requests.push({
  321 |         authorization: request.headers().authorization,
  322 |         sessionKey: request.headers()['x-opensquilla-session-key'],
  323 |       })
  324 |       return route.fulfill({
  325 |         status: 200,
  326 |         contentType: 'audio/wav',
  327 |         body: silentWav(),
  328 |       })
  329 |     })
  330 |     await openAudioSeeded(page)
  331 | 
  332 |     await page.waitForTimeout(200)
  333 |     expect(requests).toHaveLength(0)
  334 | 
  335 |     await page.getByRole('button', { name: 'Play audio sample.wav' }).click()
  336 |     const player = page.locator('.msg-audio-card__player')
  337 |     await expect(player).toBeVisible({ timeout: 10000 })
  338 |     await expect(player).toHaveAttribute('controls', '')
  339 |     expect(requests).toEqual([{
  340 |       authorization: 'Bearer audio-token-e2e',
  341 |       sessionKey: SESSION_KEY,
  342 |     }])
  343 |   })
  344 | 
  345 |   test('audio failure exposes Retry and Download, then recovers to native controls', async ({ page }) => {
  346 |     let shouldFail = true
  347 |     let requests = 0
  348 |     await page.route('**/api/v1/artifacts/art-card-audio*', route => {
  349 |       requests += 1
  350 |       if (shouldFail) return route.fulfill({ status: 500, body: 'audio failed' })
  351 |       return route.fulfill({
  352 |         status: 200,
  353 |         contentType: 'audio/wav',
  354 |         body: silentWav(),
  355 |       })
  356 |     })
  357 |     await openAudioSeeded(page)
  358 | 
  359 |     const card = page.locator('.msg-audio-card')
  360 |     await page.getByRole('button', { name: 'Play audio sample.wav' }).click()
  361 |     await expect(card).toHaveAttribute('data-state', 'error')
  362 |     await expect(card.getByText('Audio could not be loaded.')).toBeVisible()
  363 |     await expect(card.getByRole('button', { name: 'Retry sample.wav' })).toBeVisible()
  364 |     await expect(card.getByRole('button', { name: 'Download sample.wav' })).toBeVisible()
  365 | 
  366 |     shouldFail = false
  367 |     await card.getByRole('button', { name: 'Retry sample.wav' }).click()
  368 |     await expect(card.locator('.msg-audio-card__player')).toBeVisible({ timeout: 10000 })
  369 |     expect(requests).toBe(2)
  370 |   })
  371 | 
  372 |   test('video stays in the message, performs zero initial requests, and loads authenticated controls', async ({ page }) => {
  373 |     const requests: Array<{
  374 |       authorization?: string
  375 |       sessionKey?: string
  376 |     }> = []
  377 |     await page.addInitScript(() => {
  378 |       sessionStorage.setItem('marguerite.wsToken', 'video-token-e2e')
  379 |     })
  380 |     await page.route('**/api/v1/artifacts/art-card-video*', route => {
  381 |       const request = route.request()
  382 |       requests.push({
  383 |         authorization: request.headers().authorization,
  384 |         sessionKey: request.headers()['x-opensquilla-session-key'],
  385 |       })
  386 |       return route.fulfill({
  387 |         status: 200,
  388 |         contentType: 'video/webm',
  389 |         body: WEBM_TINY,
  390 |       })
  391 |     })
  392 |     await openVideoSeeded(page)
  393 | 
  394 |     await expect(page.locator('.msg-artifact-chip')).toHaveCount(0)
  395 |     await page.waitForTimeout(200)
  396 |     expect(requests).toHaveLength(0)
  397 | 
> 398 |     await page.getByRole('button', { name: 'Play video sample.webm' }).click()
      |                                                                        ^ Error: locator.click: Test timeout of 30000ms exceeded.
  399 |     const player = page.locator('.msg-video-card__player')
  400 |     await expect(player).toBeVisible({ timeout: 10000 })
  401 |     await expect(player).toHaveAttribute('controls', '')
  402 |     await expect(player).toHaveAttribute('playsinline', '')
  403 |     await expect(player).toHaveAttribute('preload', 'metadata')
  404 |     expect(requests).toEqual([{
  405 |       authorization: 'Bearer video-token-e2e',
  406 |       sessionKey: SESSION_KEY,
  407 |     }])
  408 |   })
  409 | })
  410 | 
```