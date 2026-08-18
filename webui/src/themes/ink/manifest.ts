import type { ThemeManifest } from '../types'

// Value theme "ink" — 水墨 / Chinese ink-wash on xuan paper. A light paper
// ground with a five-step ink ladder (焦/浓/重/淡/清) and a 印泥 seal-red accent,
// in the spirit of the W3C CLReq (www.w3.org/TR/clreq/) Chinese typography
// spec. Applied token values live in ./tokens.css (validated by
// check-theme-contract.mjs + the WCAG contrast guard); they are not duplicated
// here. `name` is a proper-noun literal (like 'Arctic'), so it needs no i18n key.
const ink: ThemeManifest = {
  id: 'ink',
  name: 'Ink',
  kind: 'value',
  icon: 'pencil',
  capabilities: { colorScheme: 'light', userSelectable: true },
}

export default ink
