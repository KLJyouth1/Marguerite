import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

// jsdom cannot lay out a flex column, so the sidebar defect this file guards
// against is not reachable from a rendered-DOM test. What *is* pinnable is the
// set of CSS invariants whose absence caused it. Measured on a 900px viewport
// with the default groups expanded, before the fix:
//
//   .sidebar-core   clientHeight 14px   against scrollHeight 191px
//                   -> every rail row's text was clipped away, leaving an
//                      unlabelled bar and three unreachable destinations
//   .sidebar-foot   y = 1062 in a 900px fixed column, overflow:hidden
//                   -> Settings was not clickable at all
//
// Root cause: `.sidebar-core` is a flex item with `overflow-y: auto`, and the
// flexbox automatic-minimum-size rule makes that combination resolve
// min-height to ZERO. It was therefore the only item able to absorb a viewport
// deficit and it absorbed all of it, while `.sidebar-history-list` (`flex: 1`,
// i.e. flex-basis 0%) contributed nothing to shrink distribution.

// Read the stylesheet from disk rather than via `?raw`: a `base.css?raw` import
// resolved to something the assertions could not see (every rule body came back
// empty), so this stays on the plain filesystem to keep the contract honest.
const css = readFileSync(fileURLToPath(new URL('./base.css', import.meta.url)), 'utf8')
  .replace(/\r\n/g, '\n')
  // Strip comments first. The rules here carry long explanatory comments that
  // quote the very declarations being guarded (e.g. prose mentioning
  // `overflow: hidden`), and matching those would fail the contract for a
  // comment rather than for code.
  .replace(/\/\*[\s\S]*?\*\//g, '')

/** Body of the first rule whose selector starts a line. */
function ruleBody(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = css.match(new RegExp(`^${escaped}\\s*\\{([\\s\\S]*?)\\}`, 'm'))
  return match ? match[1] : ''
}

function minHeightPx(selector: string): number {
  const raw = ruleBody(selector).match(/min-height\s*:\s*([^;]+);/)?.[1]?.trim() ?? ''
  const px = raw.match(/^(\d+(?:\.\d+)?)px$/)
  return px ? Number(px[1]) : Number.NaN
}

describe('sidebar vertical layout contract', () => {
  it('lets the sidebar column scroll instead of hiding its overflow', () => {
    const body = ruleBody('.sidebar')
    expect(body, '.sidebar rule must exist in base.css').not.toBe('')

    // `overflow: hidden` made the settings footer unreachable (y=1062 in a
    // 900px column). The column must be able to scroll.
    expect(body).toMatch(/overflow-y:\s*auto/)
    expect(body).not.toMatch(/overflow:\s*hidden/)
  })

  it('floors the primary rail so it can never collapse into a sliver', () => {
    // Two full 42px rows + 2px top padding + 4px gap + 8px bottom padding.
    const floor = minHeightPx('.sidebar-core')
    expect(Number.isNaN(floor), '.sidebar-core needs an explicit px min-height').toBe(false)
    expect(floor).toBeGreaterThanOrEqual(84)
  })

  it('floors the conversation list so it keeps a usable window', () => {
    const floor = minHeightPx('.sidebar-history-list')
    expect(Number.isNaN(floor), '.sidebar-history-list needs an explicit px min-height').toBe(false)
    expect(floor).toBeGreaterThanOrEqual(96)
  })

  it('keeps the rail as the only item allowed to shrink internally', () => {
    // Both scrollable regions must carry a floor; if either loses it the other
    // silently absorbs the deficit (that is exactly how the rail got crushed).
    expect(ruleBody('.sidebar-core')).toMatch(/overflow-y:\s*auto/)
    expect(ruleBody('.sidebar-history-list')).toMatch(/overflow-y:\s*auto/)
  })
})
