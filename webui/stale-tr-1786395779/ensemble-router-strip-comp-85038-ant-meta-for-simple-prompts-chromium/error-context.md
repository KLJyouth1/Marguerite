# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: ensemble-router-strip.spec.ts >> completed ensemble trace stays in assistant meta for simple prompts
- Location: e2e\ensemble-router-strip.spec.ts:790:3

# Error details

```
Error: expect(locator).toHaveCount(expected) failed

Locator:  locator('.router-fx[data-panel="llm-ensemble"]')
Expected: 0
Received: 1
Timeout:  5000ms

Call log:
  - Expect "toHaveCount" with timeout 5000ms
  - waiting for locator('.router-fx[data-panel="llm-ensemble"]')
    14 × locator resolved to 1 element
       - unexpected value "1"

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
          - heading "Summarize why model routing …" [level=1] [ref=e70]
          - button "Copy session ID" [ref=e71] [cursor=pointer]:
            - img [ref=e73]
        - generic [ref=e77]:
          - button "Share" [ref=e78] [cursor=pointer]:
            - img [ref=e80]
            - generic [ref=e86]: Share
          - button "Compact context (LLM summary)" [ref=e87] [cursor=pointer]:
            - img [ref=e89]
      - generic [ref=e94]:
        - button "Manage gateway connection" [ref=e95] [cursor=pointer]: Connected
        - button "Language" [ref=e97] [cursor=pointer]:
          - img [ref=e99]
          - generic [ref=e102]: English
        - button "Theme" [ref=e104] [cursor=pointer]:
          - img [ref=e106]
    - main [ref=e109]:
      - generic [ref=e110]:
        - region "Chat conversation" [ref=e113]:
          - generic [ref=e114]:
            - generic [ref=e115]:
              - generic [ref=e117]: Summarize why model routing matters in one sentence.
              - generic [ref=e118]:
                - button "Copy" [ref=e119] [cursor=pointer]:
                  - img [ref=e121]
                - button "Edit" [ref=e125] [cursor=pointer]:
                  - img [ref=e127]
                - time [ref=e130]:
                  - generic [ref=e131]: Aug 11, 2026, 05:00 AM
                  - generic [ref=e132]: ·
                  - generic [ref=e133]: 1m ago
            - button "Show ensemble route trace" [ref=e135] [cursor=pointer]:
              - status [ref=e137]: 8 candidates synthesized
              - generic [ref=e138]: view trace
            - generic [ref=e140]:
              - paragraph [ref=e144]: Model routing chooses the right model path for a turn.
              - generic [ref=e145]:
                - button "Usage details" [ref=e148] [cursor=pointer]:
                  - img [ref=e150]
                - generic [ref=e152]:
                  - button "Copy" [ref=e153] [cursor=pointer]:
                    - img [ref=e155]
                  - button "Regenerate" [ref=e159] [cursor=pointer]:
                    - img [ref=e161]
                  - button "Fork from here" [ref=e164] [cursor=pointer]:
                    - img [ref=e166]
                  - time [ref=e171]:
                    - generic [ref=e172]: Aug 11, 2026, 05:01 AM
                    - generic [ref=e173]: ·
                    - generic [ref=e174]: just now
        - generic [ref=e177]:
          - generic [ref=e178]:
            - textbox "Message to send" [active] [ref=e180]:
              - /placeholder: Send a message...
            - generic [ref=e182]:
              - generic [ref=e183]:
                - button "Add" [ref=e185] [cursor=pointer]:
                  - img [ref=e187]
                - button "Model routing" [ref=e189] [cursor=pointer]:
                  - img [ref=e191]
                  - generic: NEW
                - button "Execution mode" [ref=e198] [cursor=pointer]:
                  - img [ref=e200]
                - button "More" [ref=e203] [cursor=pointer]:
                  - img [ref=e205]
              - button "Send" [ref=e210] [cursor=pointer]:
                - img [ref=e212]
          - note [ref=e215]: Content generated by AI may be inaccurate. For reference only; verify important information.
        - status [ref=e216]
  - status [ref=e217]
```

# Test source

```ts
  695 |   })
  696 |   await page.route('**/api/approvals', route => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ pending: [] }) }))
  697 |   await page.routeWebSocket(/\/ws$/, ws => {
  698 |     ws.send(JSON.stringify({ type: 'event', event: 'connect.challenge', payload: {} }))
  699 |     ws.onMessage(message => {
  700 |       try {
  701 |         const frame = JSON.parse(String(message))
  702 |         if (frame?.type !== 'req') return
  703 |         const method = String(frame.method || '')
  704 |         if (method === 'connect') { ws.send(JSON.stringify({ protocol: 3, policy: { tick_interval_ms: 30000 } })); return }
  705 |         if (method === 'chat.send') {
  706 |           ws.send(wsResponse(String(frame.id), { accepted: true, session: KEY, task_id: 'persist-task', stream_seq: 1 }))
  707 |           ws.send(wsEvent('task.running', { key: KEY, task_id: 'persist-task', stream_seq: 1 }))
  708 |           ws.send(wsEvent('session.event.state_change', { key: KEY, task_id: 'persist-task', stream_seq: 2, to_state: 'thinking' }))
  709 |           const prog = (seq: number, p: Record<string, unknown>) => ws.send(wsEvent('session.event.ensemble_progress', { key: KEY, task_id: 'persist-task', stream_seq: seq, ...p }))
  710 |           prog(3, { event_type: 'proposer_finish', proposer_label: 'anchor', proposer_provider: 'openrouter', proposer_model: 'qwen/qwen3.7-plus', input_tokens: 120, output_tokens: 30 })
  711 |           prog(4, { event_type: 'proposer_start', proposer_label: 'critic', proposer_provider: 'openrouter', proposer_model: 'z-ai/glm-5.2' })
  712 |           // Keep the turn alive: stream answer text deltas AFTER the inspector opens.
  713 |           let seq = 5
  714 |           const timer = setInterval(() => {
  715 |             ws.send(wsEvent('session.event.text_delta', { key: KEY, task_id: 'persist-task', stream_seq: seq++, text: 'streaming ' }))
  716 |             if (seq > 12) clearInterval(timer)
  717 |           }, 150)
  718 |           return
  719 |         }
  720 |         const payloads: Record<string, unknown> = {
  721 |           'agents.list': { agents: [] },
  722 |           'chat.history': { messages: [], has_more: false },
  723 |           'commands.list_for_surface': { commands: [] },
  724 |           'config.get': { squilla_router: { enabled: true, rollout_phase: 'full', tiers: {} }, llm_ensemble: { enabled: true }, permissions: {}, skills: {} },
  725 |           'sessions.list': { sessions: [], has_more: false },
  726 |           'sessions.messages.subscribe': { subscribed: true, replay_complete: true, current_stream_seq: 0, run_status: 'idle' },
  727 |           'usage.status': { sessions: [] },
  728 |         }
  729 |         ws.send(wsResponse(String(frame.id), payloads[method] ?? {}))
  730 |       } catch {}
  731 |     })
  732 |   })
  733 |   await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(KEY))
  734 |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  735 |   await page.locator('.chat-textarea').fill('Compare and synthesize.')
  736 |   await page.locator('.chat-send-btn[aria-label="Send"]').click()
  737 | 
  738 |   const strip = page.locator('.router-fx[data-panel="llm-ensemble"]')
  739 |   await expect(strip).toBeVisible({ timeout: 10000 })
  740 |   await strip.locator('[data-testid="router-ensemble-toggle"]').click()
  741 |   const inspector = strip.locator('[data-testid="router-ensemble-inspector"]')
  742 |   await expect(inspector).toBeVisible()
  743 | 
  744 |   // While answer text keeps streaming (recomputing the message list), the opened
  745 |   // inspector must NOT collapse — this is the "detail keeps disappearing" bug.
  746 |   await page.waitForTimeout(1000)
  747 |   await expect(inspector).toBeVisible()
  748 | })
  749 | 
  750 | async function openLiveEnsembleInspector(page: Page) {
  751 |   await page.locator('.chat-textarea').fill('Compare three model families and synthesize.')
  752 |   await page.locator('.chat-send-btn[aria-label="Send"]').click()
  753 |   const strip = page.locator('.router-fx[data-panel="llm-ensemble"]')
  754 |   await expect(strip).toBeVisible({ timeout: 10000 })
  755 |   await strip.locator('[data-testid="router-ensemble-toggle"]').click()
  756 |   await expect(strip.locator('[data-testid="router-ensemble-inspector"]')).toBeVisible()
  757 |   return strip
  758 | }
  759 | 
  760 | test('live ensemble strip animates the scan line, pulse dot, and running spinner', async ({ page }) => {
  761 |   await mockStreamingEnsembleWithProgress(page)
  762 |   await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(PROGRESS_SESSION_KEY))
  763 |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  764 |   const strip = await openLiveEnsembleInspector(page)
  765 | 
  766 |   const scanAnim = await strip.locator('.router-fx-ensemble__scan').evaluate(el => getComputedStyle(el).animationName)
  767 |   expect(scanAnim).not.toBe('none')
  768 |   const dotAnim = await strip.locator('.router-fx-ensemble__dot').evaluate(el => getComputedStyle(el).animationName)
  769 |   expect(dotAnim).not.toBe('none')
  770 |   const spinAnim = await strip.locator('.router-fx-inspector__spin').first().evaluate(el => getComputedStyle(el).animationName)
  771 |   expect(spinAnim).not.toBe('none')
  772 | })
  773 | 
  774 | test('live ensemble strip disables all animation under prefers-reduced-motion', async ({ page }) => {
  775 |   await page.emulateMedia({ reducedMotion: 'reduce' })
  776 |   await mockStreamingEnsembleWithProgress(page)
  777 |   await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(PROGRESS_SESSION_KEY))
  778 |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  779 |   const strip = await openLiveEnsembleInspector(page)
  780 | 
  781 |   const scanAnim = await strip.locator('.router-fx-ensemble__scan').evaluate(el => getComputedStyle(el).animationName)
  782 |   expect(scanAnim).toBe('none')
  783 |   const dotAnim = await strip.locator('.router-fx-ensemble__dot').evaluate(el => getComputedStyle(el).animationName)
  784 |   expect(dotAnim).toBe('none')
  785 |   const spinAnim = await strip.locator('.router-fx-inspector__spin').first().evaluate(el => getComputedStyle(el).animationName)
  786 |   expect(spinAnim).toBe('none')
  787 | })
  788 | 
  789 | for (const complexity of ['simple', 'medium', 'complex'] as const) {
  790 |   test(`completed ensemble trace stays in assistant meta for ${complexity} prompts`, async ({ page }) => {
  791 |     await mockEnsembleHistory(page, complexity)
  792 |     await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(`${SESSION_KEY}-${complexity}`))
  793 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  794 | 
> 795 |     await expect(page.locator('.router-fx[data-panel="llm-ensemble"]')).toHaveCount(0)
      |                                                                         ^ Error: expect(locator).toHaveCount(expected) failed
  796 | 
  797 |     const assistant = page.locator('.msg-ai').first()
  798 |     await expect(assistant.locator('.msg-meta__ensemble')).toContainText('Ensemble · 3 models')
  799 |     await expect(assistant).not.toContainText('qwen3.7-plus')
  800 |     await expect(assistant).not.toContainText('kimi-k2.6')
  801 | 
  802 |     await assistant.locator('.msg-meta__more-btn').click()
  803 | 
  804 |     const details = assistant.locator('.msg-meta-popover')
  805 |     await expect(details).toBeVisible()
  806 |     await expect(details).toContainText('qwen3.7-plus')
  807 |     await expect(details).toContainText('kimi-k2.6')
  808 |     await expect(details).toContainText('glm-5.2')
  809 |   })
  810 | }
  811 | 
  812 | test('live ensemble routing shows the strip alongside flat activity', async ({ page }) => {
  813 |   await mockStreamingEnsembleRun(page)
  814 |   await page.goto(CONTROL_URL + 'chat?session=' + encodeURIComponent(STREAM_SESSION_KEY))
  815 |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  816 | 
  817 |   await page.locator('.chat-textarea').fill('Build a tiny team collaboration app.')
  818 |   await page.locator('.chat-send-btn[aria-label="Send"]').click()
  819 | 
  820 |   // The flat activity surface runs its normal execution phase...
  821 |   await expect(page.locator('.assistant-activity--live')).toBeVisible({ timeout: 10000 })
  822 |   // ...and the ensemble strip is surfaced independently — here as the pre-decision
  823 |   // reserve, before any member arrives — instead of being hidden behind it.
  824 |   await expect(page.locator('.router-fx[data-panel="llm-ensemble"]')).toHaveCount(1)
  825 |   // It is live, not settled.
  826 |   await expect(page.locator('.router-fx[data-panel="llm-ensemble"][data-settled="true"]')).toHaveCount(0)
  827 | })
  828 | 
```