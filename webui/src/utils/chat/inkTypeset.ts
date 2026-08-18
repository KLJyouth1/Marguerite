// Ink typography post-processor (Track F) — CLReq Chinese typesetting.
//
// renderMarkdown() returns DOMPurify-sanitized HTML. CSS cannot express the
// W3C CLReq "中西文混排" rule (a hairline gap at every CJK↔Latin-digit
// boundary, ≤ 1/4 of a Han character width), so this module adds it as a DOM
// decoration that runs AFTER the sanitized body is already in the tree. It uses
// createElement / textContent / replaceChild only — never innerHTML — so it
// adds no HTML sink and can never bypass sanitization.
//
// Idempotent: clearGaps() removes prior gaps first, so the repeated
// re-decorations that streaming triggers (v-html swaps the subtree each flush)
// are safe. Scoped to the "ink" value theme and skipped inside <code>/<pre>/
// math so code and formulas keep exact spacing.

/** Classify a code point: 1 = CJK / kana / fullwidth punctuation, 2 = Latin or
 *  digit, 0 = everything else (spaces, ASCII punctuation, symbols). */
function charClass(ch: string): 0 | 1 | 2 {
  const cp = ch.codePointAt(0) ?? 0
  if (
    (cp >= 0x3000 && cp <= 0x303f) || // CJK symbols & punctuation
    (cp >= 0x3040 && cp <= 0x30ff) || // hiragana / katakana
    (cp >= 0x3400 && cp <= 0x9fff) || // CJK ideographs + ext A
    (cp >= 0xf900 && cp <= 0xfaff) || // CJK compatibility ideographs
    (cp >= 0xff00 && cp <= 0xffef) || // fullwidth forms
    (cp >= 0x20000 && cp <= 0x2ffff) // CJK ext B+ (rare)
  ) return 1
  if (/[A-Za-z0-9]/.test(ch)) return 2
  return 0
}

function isSkipped(el: Element | null): boolean {
  return !!el?.closest('pre, code, .katex, .citation-pill')
}

function clearGaps(root: HTMLElement): void {
  root.querySelectorAll('.ink-cjk-gap').forEach((n) => {
    n.replaceWith(document.createTextNode(''))
  })
}

/** Split a single text node, inserting an empty .ink-cjk-gap span at every
 *  CJK↔Latin-digit transition (unless already whitespace-separated). */
function splitIntoGaps(node: Text): void {
  const parent = node.parentElement
  if (!parent || isSkipped(parent)) return
  const text = node.data
  if (!text) return

  const frag = document.createDocumentFragment()
  let buf = ''
  let prev = 0
  const flush = () => {
    if (buf) {
      frag.appendChild(document.createTextNode(buf))
      buf = ''
    }
  }
  for (const ch of text) {
    const cls = charClass(ch)
    if (cls !== 0) {
      if ((prev === 1 && cls === 2) || (prev === 2 && cls === 1)) {
        // Already separated by whitespace? Skip to avoid double spacing.
        if (!/\s$/.test(buf)) {
          flush()
          const gap = document.createElement('span')
          gap.className = 'ink-cjk-gap'
          frag.appendChild(gap)
        }
      }
      prev = cls
    }
    buf += ch
  }
  flush()
  node.parentNode?.replaceChild(frag, node)
}

/** Apply ink CJK↔Latin spacing to a rendered markdown root. No-op unless the
 *  active value theme is "ink". Safe to call on every re-decoration. */
export function decorateInkTypeset(root: HTMLElement | null): void {
  if (!root) return
  if (document.documentElement.getAttribute('data-theme') !== 'ink') return
  clearGaps(root)
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  const nodes: Text[] = []
  let current: Node | null
  while ((current = walker.nextNode())) nodes.push(current as Text)
  for (const n of nodes) {
    if (isSkipped(n.parentElement)) continue
    splitIntoGaps(n)
  }
}
