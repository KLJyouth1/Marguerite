# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: attachment-drag-upload.spec.ts >> attachment drag upload >> keeps image history replay attachments as thumbnails
- Location: e2e\attachment-drag-upload.spec.ts:380:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: locator('.chat-textarea')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for locator('.chat-textarea')

```

# Test source

```ts
  55  |             const messageIndex = capturedSends.length
  56  |             historyMessages.splice(0, historyMessages.length, {
  57  |               role: 'user',
  58  |               text: params.displayText || '',
  59  |               timestamp: new Date().toISOString(),
  60  |               id: `history-user-${messageIndex}`,
  61  |               message_id: `history-user-${messageIndex}`,
  62  |               attachments: historyAttachmentsFromSend(params, options.historyAttachmentFixture || 'send'),
  63  |             })
  64  |           }
  65  |           ws.send(wsResponse(String(frame.id), {
  66  |             sessionKey: params.sessionKey,
  67  |             status: 'accepted',
  68  |           }))
  69  |           if (options.replayHistoryAfterSend) {
  70  |             ws.send(wsEvent('session.event.done', {
  71  |               key: params.sessionKey || '',
  72  |               sessionKey: params.sessionKey || '',
  73  |               status: 'succeeded',
  74  |               stream_seq: capturedSends.length,
  75  |             }))
  76  |           }
  77  |           return
  78  |         }
  79  |         if (method === 'chat.history') {
  80  |           options.historyRequests?.push(frame.params || {})
  81  |           ws.send(wsResponse(String(frame.id), { messages: historyMessages, has_more: false }))
  82  |           return
  83  |         }
  84  | 
  85  |         const payloads: Record<string, unknown> = {
  86  |           'agents.list': { agents: [] },
  87  |           'commands.list_for_surface': { commands: [] },
  88  |           'config.get': {
  89  |             squilla_router: { enabled: false, rollout_phase: 'observe', tiers: {} },
  90  |             permissions: {},
  91  |             skills: {},
  92  |           },
  93  |           'sessions.list': { sessions: [], has_more: false },
  94  |           'sessions.messages.subscribe': {
  95  |             subscribed: true,
  96  |             replay_complete: true,
  97  |             current_stream_seq: 0,
  98  |             run_status: 'idle',
  99  |           },
  100 |           'usage.status': { sessions: [] },
  101 |         }
  102 | 
  103 |         ws.send(wsResponse(String(frame.id), payloads[method] ?? {}))
  104 |       } catch (err) {
  105 |         if (!(err instanceof SyntaxError)) throw err
  106 |       }
  107 |     })
  108 |   })
  109 | }
  110 | 
  111 | function historyAttachmentsFromSend(params: CapturedSend, fixture: HistoryAttachmentFixture): Array<Record<string, unknown>> {
  112 |   const first = params.attachments?.[0] || {}
  113 |   if (fixture === 'html') {
  114 |     return [{
  115 |       type: 'text/html',
  116 |       name: 'preview.html',
  117 |       data: 'PGh0bWw+',
  118 |     }]
  119 |   }
  120 |   if (fixture === 'image') {
  121 |     return [{
  122 |       type: 'image/png',
  123 |       name: 'photo.png',
  124 |       data: HISTORY_IMAGE_DATA,
  125 |     }]
  126 |   }
  127 |   if (fixture === 'staged') {
  128 |     return [{
  129 |       sha256_ref: 'b'.repeat(64),
  130 |       name: String(first.name || 'quarterly-report.pdf'),
  131 |       mime: String(first.mime || 'application/pdf'),
  132 |       size: 2_000_001,
  133 |       download_url: `/api/v1/attachments/${'b'.repeat(64)}?token=legacy&sessionKey=legacy&variant=download`,
  134 |     }]
  135 |   }
  136 |   return (params.attachments || []).map(att => ({
  137 |     type: att.type,
  138 |     name: att.name,
  139 |     data: att.data,
  140 |   }))
  141 | }
  142 | 
  143 | async function readDownloadBytes(download: Download): Promise<Buffer> {
  144 |   const stream = await download.createReadStream()
  145 |   if (!stream) throw new Error('download stream unavailable')
  146 |   const chunks: Buffer[] = []
  147 |   for await (const chunk of stream) chunks.push(Buffer.from(chunk))
  148 |   return Buffer.concat(chunks)
  149 | }
  150 | 
  151 | async function openMockedChat(page: Page, capturedSends: CapturedSend[], options: MockRpcOptions = {}) {
  152 |   await mockApprovals(page)
  153 |   await mockRpc(page, capturedSends, options)
  154 |   await page.goto(CONTROL_URL)
> 155 |   await expect(page.locator('.chat-textarea')).toBeVisible()
      |                                                ^ Error: expect(locator).toBeVisible() failed
  156 |   await expect(page.locator('.conn-pill.connected')).toBeVisible()
  157 | }
  158 | 
  159 | async function dropFiles(page: Page, files: Array<{ name: string; type: string; text?: string; size?: number }>) {
  160 |   await page.evaluate((fileSpecs) => {
  161 |     const dataTransfer = new DataTransfer()
  162 |     for (const spec of fileSpecs) {
  163 |       const parts = spec.size
  164 |         ? [new Uint8Array(spec.size)]
  165 |         : [spec.text || 'drag upload']
  166 |       dataTransfer.items.add(new File(parts, spec.name, { type: spec.type }))
  167 |     }
  168 |     const chat = document.querySelector('.chat')
  169 |     if (!chat) throw new Error('chat root not found')
  170 |     for (const type of ['dragenter', 'dragover', 'drop']) {
  171 |       chat.dispatchEvent(new DragEvent(type, {
  172 |         bubbles: true,
  173 |         cancelable: true,
  174 |         dataTransfer,
  175 |       }))
  176 |     }
  177 |   }, files)
  178 | }
  179 | 
  180 | async function expectDropOverlayCoversChat(page: Page) {
  181 |   const layout = await page.evaluate(() => {
  182 |     const rect = (box: DOMRect) => ({
  183 |       bottom: box.bottom,
  184 |       left: box.left,
  185 |       right: box.right,
  186 |       top: box.top,
  187 |     })
  188 |     const overlayEl = document.querySelector('.chat-drop-overlay')
  189 |     const chat = document.querySelector('.chat')?.getBoundingClientRect()
  190 |     const body = document.querySelector('.chat-body')?.getBoundingClientRect()
  191 |     const overlay = overlayEl?.getBoundingClientRect()
  192 |     const beacon = document.querySelector('.chat-drop-overlay__beacon')?.getBoundingClientRect()
  193 |     const overlayStyle = overlayEl ? getComputedStyle(overlayEl) : null
  194 |     if (!chat || !body || !overlay || !beacon || !overlayStyle) {
  195 |       throw new Error('drop overlay layout target not found')
  196 |     }
  197 |     return {
  198 |       chat: rect(chat),
  199 |       body: rect(body),
  200 |       overlay: rect(overlay),
  201 |       beacon: rect(beacon),
  202 |       pointerEvents: overlayStyle.pointerEvents,
  203 |     }
  204 |   })
  205 | 
  206 |   expect(Math.abs(layout.overlay.left - layout.chat.left)).toBeLessThanOrEqual(2)
  207 |   expect(Math.abs(layout.overlay.top - layout.chat.top)).toBeLessThanOrEqual(2)
  208 |   expect(Math.abs(layout.overlay.right - layout.chat.right)).toBeLessThanOrEqual(2)
  209 |   expect(Math.abs(layout.overlay.bottom - layout.chat.bottom)).toBeLessThanOrEqual(2)
  210 |   expect(layout.overlay.top).toBeLessThan(layout.body.top)
  211 |   expect(layout.overlay.bottom).toBeGreaterThan(layout.body.bottom)
  212 |   expect(layout.beacon.left).toBeGreaterThanOrEqual(layout.overlay.left)
  213 |   expect(layout.beacon.right).toBeLessThanOrEqual(layout.overlay.right)
  214 |   expect(layout.pointerEvents).toBe('none')
  215 | }
  216 | 
  217 | test.describe('attachment drag upload', () => {
  218 |   test('ignores non-file drags and keeps file drag affordance stable across children', async ({ page }) => {
  219 |     const capturedSends: CapturedSend[] = []
  220 |     await openMockedChat(page, capturedSends)
  221 | 
  222 |     await page.evaluate(() => {
  223 |       const dataTransfer = new DataTransfer()
  224 |       dataTransfer.setData('text/plain', 'not a file')
  225 |       const chat = document.querySelector('.chat')
  226 |       if (!chat) throw new Error('chat root not found')
  227 |       chat.dispatchEvent(new DragEvent('dragenter', { bubbles: true, cancelable: true, dataTransfer }))
  228 |       chat.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer }))
  229 |     })
  230 |     await expect(page.locator('.chat-drop-overlay')).toHaveCount(0)
  231 | 
  232 |     const nonFileDropPrevented = await page.evaluate(() => {
  233 |       const dataTransfer = new DataTransfer()
  234 |       dataTransfer.setData('text/uri-list', 'https://example.test/drop-target')
  235 |       dataTransfer.setData('text/plain', 'https://example.test/drop-target')
  236 |       const chat = document.querySelector('.chat')
  237 |       if (!chat) throw new Error('chat root not found')
  238 |       const event = new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer })
  239 |       chat.dispatchEvent(event)
  240 |       return event.defaultPrevented
  241 |     })
  242 |     expect(nonFileDropPrevented).toBe(true)
  243 |     await expect(page.locator('.attachment-chip')).toHaveCount(0)
  244 | 
  245 |     await page.evaluate(() => {
  246 |       const dataTransfer = new DataTransfer()
  247 |       dataTransfer.items.add(new File(['stable'], 'stable.txt', { type: 'text/plain' }))
  248 |       const chat = document.querySelector('.chat')
  249 |       const textarea = document.querySelector('.chat-textarea')
  250 |       if (!chat || !textarea) throw new Error('drag targets not found')
  251 |       chat.dispatchEvent(new DragEvent('dragenter', { bubbles: true, cancelable: true, dataTransfer }))
  252 |       textarea.dispatchEvent(new DragEvent('dragenter', { bubbles: true, cancelable: true, dataTransfer }))
  253 |       textarea.dispatchEvent(new DragEvent('dragleave', { bubbles: true, cancelable: true, dataTransfer }))
  254 |     })
  255 |     await expect(page.locator('.chat-drop-overlay')).toBeVisible()
```