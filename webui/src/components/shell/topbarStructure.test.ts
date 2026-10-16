// @vitest-environment node
/**
 * Regression guard for the topbar child count that caused the title-bar outage.
 *
 * `.topbar--chat` is a GRID whose four columns were declared for exactly four
 * children. Adding an extra in-flow child made `.topbar-right` auto-place onto
 * an implicit second row, which dropped the window controls below the bar where
 * the chat banner covered them — the `/chat`-only disappearance QA reported.
 *
 * This asserts the source-level invariant directly: the `<header class="topbar">`
 * block must expose exactly four in-flow element children, and any topbar-level
 * addition must be absolutely positioned or gated so it cannot take a grid cell.
 *
 * Geometry still needs a real window; this only locks the structural cause.
 */
import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { parse as parseSfc, compileTemplate } from '@vue/compiler-sfc'

// Parse the real component with Vue's own compiler, so the assertion runs against
// App.vue's actual AST (comment removal, attribute quoting and nesting included)
// rather than a hand-rolled tag scanner or a fixture that could drift.
const source = readFileSync(resolve(process.cwd(), 'src/App.vue'), 'utf8')
const { descriptor } = parseSfc(source)
const template = descriptor.template!.content

// `compileTemplate` walks the template into a program AST (the bare template
// parser exposes none). Pass only the template block, otherwise the AST is
// wrapped in a synthetic <template> root.
const { ast } = compileTemplate({
  source: template,
  filename: 'App.vue',
  id: 'app-topbar-guard',
})

interface AstNode {
  type: number
  tag: string
  children?: AstNode[]
}

/** Depth-first search for the first `<header>` in the template AST. */
function findTag(node: AstNode, tag: string): AstNode | null {
  if (node.type === 1 && node.tag === tag) return node
  for (const child of node.children ?? []) {
    const hit: AstNode | null = findTag(child, tag)
    if (hit) return hit
  }
  return null
}

/**
 * Direct element children of the topbar `<header>`, in document order.
 *
 * Element-type nodes only (NODE_ELEMENT === 1). A `v-if`-gated element such as
 * the drag region compiles to a COMPOUND_EXPRESSION wrapper rather than a bare
 * element node, so it is deliberately not counted here — it is taken out of the
 * grid flow by CSS instead, and the two dedicated assertions below cover it.
 */
function topbarChildren(): string[] {
  const header = findTag(ast as unknown as AstNode, 'header')
  if (!header) throw new Error('no <header> in App.vue template')
  return (header.children ?? []).filter(node => node.type === 1).map(node => node.tag)
}

const scopedCss = descriptor.styles[0]?.content ?? ''

describe('topbar structure', () => {
  it('exposes the four grid participants in declared column order', () => {
    // `.topbar--chat` declares `auto minmax(0,1fr) auto auto` — four columns for
    // four participants. A fifth grid-participating child makes `.topbar-right`
    // auto-place onto an implicit second row, which is what pushed the window
    // controls out of the bar on `/chat`.
    //
    // Five element children exist; the fifth is the absolutely positioned
    // TitleBarControls, which takes no grid cell. Grid participants:
    const children = topbarChildren()
    expect(children).toContain('TitleBarControls')
    expect(children.filter(tag => tag !== 'TitleBarControls')).toEqual([
      'div',      // .topbar-left
      'div',      // #app-route-header (Chat teleport target)
      'ModeBadge',
      'div',      // .topbar-right (app cluster)
    ])
  })

  it('marks the desktop-shell class so the app cluster can reserve room', () => {
    expect(template).toContain("'topbar--desktop-shell': isDesktopShell")
  })

  it('keeps the v-if drag region out of the grid flow', () => {
    expect(template).toContain('class="topbar-drag-region"')
    // Out of flow: takes no grid cell regardless of how wide the route actions get.
    expect(scopedCss).toMatch(/\.topbar-drag-region\s*\{[^}]*position:\s*absolute/)
    // …but always keeps real draggable pixels even when the band is squeezed.
    expect(scopedCss).toMatch(/\.topbar-drag-region\s*\{[^}]*min-width:\s*48px/)
  })

  it('pins the window controls to the corner, outside the app cluster', () => {
    // Direct <header> child (not nested in .topbar-right, which is a grid item)
    // and absolutely positioned so it can never wrap below the bar.
    expect(template).toMatch(/<TitleBarControls \/>[\s\S]{0,80}<\/header>/)
    const controlsCss = readFileSync(
      resolve(process.cwd(), 'src/components/shell/TitleBarControls.vue'),
      'utf8',
    )
    expect(controlsCss).toMatch(/\.title-bar-controls\s*\{[^}]*position:\s*absolute/)
    expect(controlsCss).toMatch(/\.title-bar-controls\s*\{[^}]*z-index:\s*21/)
  })
})