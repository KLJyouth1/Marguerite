/**
 * Artifact type classification — the single source of truth behind the sidebar's
 * type column, the inline editor's write gate, and every "open in system app"
 * fallback (SPEC §5 artifacts.*, §8 artifact type colours, AC-A1/A3/A4).
 *
 * Two questions decide the whole UI, and they are NOT the same question:
 *
 *   1. What IS this file?          -> ArtifactKindKey (icon + type colour)
 *   2. Can Marguerite WRITE it?    -> inlineEditable (textual store or not)
 *
 * Conflating them is the bug this module exists to prevent: .xlsx and .csv are
 * both `sheet`, but only one of them survives a round-trip through a textarea.
 * A .docx that we let a user "edit" inline would be silently destroyed on save
 * — the file would still be named .docx and would no longer open. So editable
 * is gated on `binary === false`, never on the kind.
 */

import type { IconName } from '@/utils/icons'

export type ArtifactKindKey =
  | 'doc'
  | 'sheet'
  | 'deck'
  | 'image'
  | 'code'
  | 'pdf'
  | 'audio'
  | 'video'
  | 'archive'
  | 'unknown'

export interface ArtifactKindDescriptor {
  key: ArtifactKindKey
  /** Icon from the locked set (utils/icons.ts) — never an emoji. */
  icon: IconName
  /** CSS custom property reference for the type dot / icon ink. */
  colorVar: string
  /** Human category, shown in the row's tooltip and the editor header. */
  label: string
  /** Short uppercase format badge, e.g. MD / CSV / XLSX. */
  format: string
  /** True when the bytes are not a text document (blocks inline editing). */
  binary: boolean
  /** True when `artifacts.edit_inline` may be offered for this artifact. */
  inlineEditable: boolean
  /** True when the only faithful viewer is the OS-registered application. */
  systemOnly: boolean
}

/**
 * Extension -> kind. Binary-ness is tracked separately below because it cuts
 * across kinds (csv vs xlsx, svg vs png, html vs pdf).
 */
const EXT_KIND: Record<string, ArtifactKindKey> = {
  // doc — prose
  md: 'doc', markdown: 'doc', mdx: 'doc', txt: 'doc', text: 'doc', log: 'doc',
  rtf: 'doc', doc: 'doc', docx: 'doc', odt: 'doc',
  // sheet — tabular
  csv: 'sheet', tsv: 'sheet', xls: 'sheet', xlsx: 'sheet', xlsm: 'sheet', ods: 'sheet',
  // deck — slides
  ppt: 'deck', pptx: 'deck', odp: 'deck', key: 'deck',
  // pdf
  pdf: 'pdf',
  // image
  png: 'image', jpg: 'image', jpeg: 'image', gif: 'image', webp: 'image',
  bmp: 'image', ico: 'image', avif: 'image', tiff: 'image', tif: 'image', svg: 'image',
  // code / structured
  html: 'code', htm: 'code', css: 'code', scss: 'code', less: 'code',
  js: 'code', mjs: 'code', cjs: 'code', ts: 'code', tsx: 'code', jsx: 'code',
  vue: 'code', svelte: 'code', json: 'code', jsonl: 'code', ndjson: 'code',
  yaml: 'code', yml: 'code', toml: 'code', ini: 'code', cfg: 'code', conf: 'code',
  env: 'code', xml: 'code', py: 'code', rs: 'code', go: 'code', java: 'code',
  kt: 'code', swift: 'code', rb: 'code', php: 'code', c: 'code', h: 'code',
  cpp: 'code', hpp: 'code', cs: 'code', sh: 'code', bash: 'code', zsh: 'code',
  ps1: 'code', sql: 'code', graphql: 'code', proto: 'code', patch: 'code', diff: 'code',
  // audio / video
  mp3: 'audio', wav: 'audio', ogg: 'audio', flac: 'audio', m4a: 'audio', aac: 'audio',
  mp4: 'video', webm: 'video', mov: 'video', avi: 'video', mkv: 'video',
  // archive
  zip: 'archive', tar: 'archive', gz: 'archive', tgz: 'archive', bz2: 'archive',
  rar: 'archive', '7z': 'archive', xz: 'archive',
}

/**
 * Extensions whose bytes are text and therefore survive a read-edit-write
 * round trip. Anything absent is treated as binary — fail closed, because the
 * cost of guessing wrong is a corrupted user file, not a missing feature.
 */
const TEXTUAL_EXT = new Set([
  'md', 'markdown', 'mdx', 'txt', 'text', 'log',
  'csv', 'tsv',
  'html', 'htm', 'css', 'scss', 'less',
  'js', 'mjs', 'cjs', 'ts', 'tsx', 'jsx', 'vue', 'svelte',
  'json', 'jsonl', 'ndjson', 'yaml', 'yml', 'toml', 'ini', 'cfg', 'conf', 'env',
  'xml', 'svg',
  'py', 'rs', 'go', 'java', 'kt', 'swift', 'rb', 'php',
  'c', 'h', 'cpp', 'hpp', 'cs', 'sh', 'bash', 'zsh', 'ps1',
  'sql', 'graphql', 'proto', 'patch', 'diff',
])

/**
 * Kinds where an inline text editor is wrong even when the bytes are textual.
 * `svg` is the case that motivates this: it is text, but users reach for it as
 * an image, and a textarea is a hostile way to touch one.
 */
const NON_EDITABLE_KINDS: ReadonlySet<ArtifactKindKey> = new Set<ArtifactKindKey>([
  'image', 'pdf', 'deck', 'audio', 'video', 'archive',
])

const KIND_ICON: Record<ArtifactKindKey, IconName> = {
  doc: 'fileText',
  sheet: 'fileSpreadsheet',
  deck: 'presentation',
  image: 'image',
  code: 'fileCode',
  pdf: 'filePdf',
  audio: 'music',
  video: 'video',
  archive: 'archive',
  unknown: 'fileQuestion',
}

const KIND_COLOR: Record<ArtifactKindKey, string> = {
  doc: 'var(--artifact-doc)',
  sheet: 'var(--artifact-sheet)',
  deck: 'var(--artifact-deck)',
  image: 'var(--artifact-image)',
  code: 'var(--artifact-code)',
  pdf: 'var(--artifact-pdf)',
  audio: 'var(--artifact-media)',
  video: 'var(--artifact-media)',
  archive: 'var(--artifact-archive)',
  unknown: 'var(--artifact-unknown)',
}

const KIND_LABEL: Record<ArtifactKindKey, string> = {
  doc: '文档',
  sheet: '表格',
  deck: '演示文稿',
  image: '图片',
  code: '代码',
  pdf: 'PDF',
  audio: '音频',
  video: '视频',
  archive: '压缩包',
  unknown: '文件',
}

/** MIME prefix -> kind, used when the name carries no usable extension. */
function kindFromMime(mime: string): ArtifactKindKey | null {
  const m = mime.toLowerCase()
  if (!m) return null
  if (m.startsWith('image/')) return 'image'
  if (m.startsWith('audio/')) return 'audio'
  if (m.startsWith('video/')) return 'video'
  if (m === 'application/pdf') return 'pdf'
  if (m.startsWith('text/csv') || m.startsWith('text/tab-separated')) return 'sheet'
  if (m.includes('spreadsheet') || m.includes('ms-excel')) return 'sheet'
  if (m.includes('presentation') || m.includes('ms-powerpoint')) return 'deck'
  if (m.includes('wordprocessing') || m.includes('msword')) return 'doc'
  if (m.startsWith('text/markdown')) return 'doc'
  if (m.startsWith('text/plain')) return 'doc'
  if (m.startsWith('text/html') || m.includes('json') || m.includes('xml') || m.includes('yaml')) return 'code'
  if (m.startsWith('text/')) return 'doc'
  if (m.includes('zip') || m.includes('tar') || m.includes('gzip')) return 'archive'
  return null
}

/** True when a MIME type promises text bytes even if the extension is unknown. */
function mimeIsTextual(mime: string): boolean {
  const m = mime.toLowerCase()
  if (!m) return false
  if (m.startsWith('text/')) return true
  return m.includes('json') || m.includes('xml') || m.includes('yaml') || m.includes('javascript')
}

export function artifactExtension(nameOrPath: string): string {
  const tail = String(nameOrPath || '').split(/[\\/]/).pop() || ''
  // Strip a query/fragment tail so a download URL still classifies.
  const clean = tail.split(/[?#]/)[0]
  if (!clean.includes('.')) return ''
  const ext = clean.split('.').pop() || ''
  return ext.toLowerCase()
}

/**
 * Classify one artifact. `name` wins over `mime` because generators are far
 * more reliable about extensions than about content types (the gateway hands
 * us `application/octet-stream` for plenty of real .md files).
 */
export function classifyArtifact(nameOrPath: string, mime?: string | null): ArtifactKindDescriptor {
  const ext = artifactExtension(nameOrPath)
  const mimeStr = String(mime || '')
  const key: ArtifactKindKey = EXT_KIND[ext] || kindFromMime(mimeStr) || 'unknown'

  const textual = ext ? TEXTUAL_EXT.has(ext) : mimeIsTextual(mimeStr)
  const binary = !textual
  const inlineEditable = textual && !NON_EDITABLE_KINDS.has(key)

  return {
    key,
    icon: KIND_ICON[key],
    colorVar: KIND_COLOR[key],
    label: KIND_LABEL[key],
    format: ext ? ext.toUpperCase().slice(0, 5) : KIND_LABEL[key],
    binary,
    inlineEditable,
    // Anything we cannot render faithfully in-app has to defer to the OS: the
    // deck/office binaries, archives, and playable media.
    systemOnly: key === 'deck' || key === 'archive'
      || (binary && (key === 'doc' || key === 'sheet'))
      || key === 'audio' || key === 'video',
  }
}

/**
 * Why inline editing is unavailable, phrased as the next action rather than a
 * refusal — a dead-end "not supported" leaves the user with nowhere to go.
 */
export function artifactEditBlockedReason(d: ArtifactKindDescriptor): string {
  if (d.inlineEditable) return ''
  switch (d.key) {
    case 'image': return '图片请用系统程序打开编辑，Marguerite 内仅预览'
    case 'pdf': return 'PDF 为版式文件，请用系统程序打开编辑'
    case 'deck': return '演示文稿请用系统程序打开编辑'
    case 'audio':
    case 'video': return '媒体文件请用系统程序打开'
    case 'archive': return '压缩包请用系统程序打开'
    default:
      return d.binary
        ? `${d.format} 为二进制格式，请用系统程序打开编辑`
        : '该格式暂不支持应用内编辑'
  }
}
