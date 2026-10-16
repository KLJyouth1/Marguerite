import { describe, expect, it } from 'vitest'

import { getIconSvg } from './icons'

describe('sidebar toggle icons', () => {
  it('uses a full divider for the visible sidebar state', () => {
    const svg = getIconSvg('sidebar-visible', 18)

    expect(svg).toContain('M9.5 4.5v15')
    expect(svg).not.toContain('l6 6')
    expect(svg).not.toContain('l-6 6')
  })

  it('uses a short rail for the hidden sidebar state', () => {
    const svg = getIconSvg('sidebar-hidden', 18)

    expect(svg).toContain('M8.5 9v6')
    expect(svg).not.toContain('l6 6')
    expect(svg).not.toContain('l-6 6')
  })
})

describe('plan disclosure icons', () => {
  it('uses outward corners for expand and inward corners for collapse', () => {
    expect(getIconSvg('expand', 15)).toContain('15 3 21 3 21 9')
    expect(getIconSvg('collapse', 15)).toContain('4 14 10 14 10 20')
  })
})

describe('icon registry integrity (upgrade-masterplan §4)', () => {
  // Source-of-truth name list; fails if a name is added without a matching ICONS entry.
  const ALL_NAMES = [
    'chat', 'home', 'channels', 'sessions', 'usage', 'cron',
    'config', 'logs', 'skills', 'agents',
    'sun', 'moon', 'monitor', 'x', 'copy', 'check',
    'send', 'play', 'stop', 'paperclip', 'plus', 'share', 'trash',
    'refresh', 'download', 'save', 'menu', 'moreHorizontal', 'user', 'search', 'eye', 'eye-off',
    'edit', 'info', 'settings', 'gear', 'gauge', 'router', 'regenerate',
    'pencil', 'fork', 'listChecks', 'chevronDown', 'chevronLeft', 'chevronRight', 'arrowUp',
    'expand', 'collapse',
    'panel-left-open', 'panel-left-close', 'panel-right-open', 'panel-right-close',
    'sidebar-visible', 'sidebar-hidden',
    'clock', 'microphone',
    'cloud', 'folder', 'fileText', 'fileCode', 'image', 'table', 'externalLink',
    'fileStack', 'fileSpreadsheet', 'filePdf', 'fileQuestion', 'presentation', 'archive',
    'gitBranch', 'gitMerge', 'zap', 'coins', 'boxes', 'listTree',
    'alertTriangle', 'appWindow',
    'keyboard', 'languages', 'shield', 'lock', 'key', 'terminal',
    'thumbs-up', 'thumbs-down',
    'music', 'pause', 'volume', 'video',
    'cursor', 'sparkles', 'blocks',
    'camera', 'crosshair', 'timer', 'command', 'move', 'drag', 'window',
    'mouse-left', 'mouse-right', 'mouse-middle', 'mouse-double', 'scroll-vertical',
    'clipboard-write', 'clipboard-read',
    'mode', 'model', 'plugin', 'capabilities', 'knowledge', 'pet', 'collab', 'swarm',
  ] as const

  it('every IconName has a non-empty SVG path and renders', () => {
    for (const name of ALL_NAMES) {
      const svg = getIconSvg(name, 16)
      expect(svg, `icon "${name}" should render`).toContain('<svg')
      expect(svg, `icon "${name}" should contain a path`).toMatch(/<(path|circle|rect|polyline|polygon|line)/)
    }
  })

  it('rejects the legacy "star" name (removed — was never defined)', () => {
    // getIconSvg returns '' for unknown names and warns; the union type already
    // excludes it, this guards against accidental reintroduction.
    expect(getIconSvg('star' as never, 16)).toBe('')
  })

  it('semantic-exclusive icons render with distinct silhouettes', () => {
    const paths = ['mode', 'model', 'plugin', 'capabilities', 'knowledge', 'pet', 'collab', 'swarm']
      .map((n) => getIconSvg(n as never, 16))
    const unique = new Set(paths)
    expect(unique.size).toBe(paths.length)
  })
})
