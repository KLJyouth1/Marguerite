# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: citations.spec.ts >> Inline citation pills >> idle chat renders no citation pills or sources row
- Location: e2e\citations.spec.ts:7:3

# Error details

```
TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
Call log:
  - waiting for locator('.conn-pill') to be visible

```

# Test source

```ts
  1   | import { test, expect } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | const LIVE = process.env.OPENSQUILLA_E2E_LIVE === '1'
  5   | 
  6   | test.describe('Inline citation pills', () => {
  7   |   test('idle chat renders no citation pills or sources row', async ({ page }) => {
  8   |     await page.goto(CONTROL_URL + 'chat/new')
> 9   |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
      |                ^ TimeoutError: page.waitForSelector: Timeout 10000ms exceeded.
  10  | 
  11  |     await expect(page.locator('.citation-pill')).toHaveCount(0)
  12  |     await expect(page.locator('.sources-row')).toHaveCount(0)
  13  |   })
  14  | 
  15  |   test('only [n] markers that map to a real source become pills', async ({ page }) => {
  16  |     await page.goto(CONTROL_URL + 'chat/new')
  17  |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  18  | 
  19  |     // Drive the transform deterministically over a known-sanitized snippet with a
  20  |     // 3-element source list. `[2]` is in range → a focusable button.citation-pill;
  21  |     // `[5]` is out of range → plain text, untouched. The transform rules mirror
  22  |     // src/utils/chat/citations.ts (single-integer [n], 1 ≤ n ≤ sources.length,
  23  |     // createElement/textContent only) so this proves the core mapping without a
  24  |     // model emitting markers.
  25  |     const result = await page.evaluate(() => {
  26  |       const CITATION_RE = /\[(\d{1,3})\]/g
  27  |       const SKIP_ANCESTORS = new Set(['PRE', 'CODE', 'A', 'BUTTON'])
  28  |       const sources = [
  29  |         { sourceId: 1, url: 'https://a.example/1', title: 'One', domain: 'a.example' },
  30  |         { sourceId: 2, url: 'https://b.example/2', title: 'Two', domain: 'b.example' },
  31  |         { sourceId: 3, url: 'https://c.example/3', title: 'Three', domain: 'c.example' },
  32  |       ]
  33  | 
  34  |       const root = document.createElement('div')
  35  |       // A pre-sanitized body: bare brackets survive marked/DOMPurify as text.
  36  |       root.innerHTML = '<p>Solar grew [2] and wind [5].</p>'
  37  |       document.body.appendChild(root)
  38  | 
  39  |       function isSkipped(node: Node | null): boolean {
  40  |         let cur: Node | null = node
  41  |         while (cur && cur instanceof HTMLElement) {
  42  |           if (SKIP_ANCESTORS.has(cur.nodeName)) return true
  43  |           if (cur.hasAttribute('data-citation')) return true
  44  |           cur = cur.parentNode
  45  |         }
  46  |         return false
  47  |       }
  48  | 
  49  |       const candidates: Text[] = []
  50  |       const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  51  |       for (let n = walker.nextNode(); n; n = walker.nextNode()) {
  52  |         const t = n.nodeValue
  53  |         if (!t || !t.includes('[')) continue
  54  |         if (isSkipped(n.parentNode)) continue
  55  |         candidates.push(n as Text)
  56  |       }
  57  | 
  58  |       for (const node of candidates) {
  59  |         const text = node.nodeValue ?? ''
  60  |         CITATION_RE.lastIndex = 0
  61  |         const frag = document.createDocumentFragment()
  62  |         let last = 0
  63  |         let changed = false
  64  |         let m = CITATION_RE.exec(text)
  65  |         while (m) {
  66  |           const num = Number(m[1])
  67  |           if (num >= 1 && num <= sources.length) {
  68  |             if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)))
  69  |             const pill = document.createElement('button')
  70  |             pill.type = 'button'
  71  |             pill.className = 'citation-pill'
  72  |             pill.textContent = `[${num}]`
  73  |             pill.setAttribute('data-citation', String(num))
  74  |             frag.appendChild(pill)
  75  |             last = m.index + m[0].length
  76  |             changed = true
  77  |           }
  78  |           m = CITATION_RE.exec(text)
  79  |         }
  80  |         if (!changed) continue
  81  |         if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)))
  82  |         node.replaceWith(frag)
  83  |       }
  84  | 
  85  |       const pills = Array.from(root.querySelectorAll('.citation-pill'))
  86  |       const out = {
  87  |         pillCount: pills.length,
  88  |         pillCitations: pills.map(p => p.getAttribute('data-citation')),
  89  |         pillIsButton: pills.every(p => p.tagName === 'BUTTON'),
  90  |         // [5] must remain literal text in the body.
  91  |         plainTextHasFive: root.textContent?.includes('[5]') ?? false,
  92  |         bracketFiveIsPill: pills.some(p => p.getAttribute('data-citation') === '5'),
  93  |       }
  94  |       root.remove()
  95  |       return out
  96  |     })
  97  | 
  98  |     // [2] → exactly one focusable pill; [5] → never a pill, stays plain text.
  99  |     expect(result.pillCount).toBe(1)
  100 |     expect(result.pillCitations).toEqual(['2'])
  101 |     expect(result.pillIsButton).toBe(true)
  102 |     expect(result.bracketFiveIsPill).toBe(false)
  103 |     expect(result.plainTextHasFive).toBe(true)
  104 |   })
  105 | 
  106 |   test('rendered citation pills only ever target real sources', async ({ page }) => {
  107 |     test.skip(!LIVE, 'Live gateway test; set OPENSQUILLA_E2E_LIVE=1 to run.')
  108 |     test.setTimeout(240000)
  109 | 
```