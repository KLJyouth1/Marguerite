/** Marguerite Web UI — SVG icon paths (ported from legacy icons.js). */

export type IconName =
  | 'chat' | 'home' | 'channels' | 'sessions' | 'usage' | 'cron'
  | 'config' | 'logs' | 'skills' | 'agents'
  | 'sun' | 'moon' | 'monitor' | 'x' | 'copy' | 'check'
  | 'send' | 'play' | 'stop' | 'paperclip' | 'plus' | 'share' | 'trash'
  | 'refresh' | 'download' | 'save' | 'menu' | 'moreHorizontal' | 'user' | 'search' | 'eye' | 'eye-off'
  | 'edit' | 'info' | 'settings' | 'gear' | 'gauge' | 'router' | 'regenerate'
  | 'pencil' | 'fork' | 'listChecks' | 'chevronDown' | 'chevronLeft' | 'chevronRight' | 'arrowUp' | 'arrowDown'
  | 'expand' | 'collapse'
  | 'panel-left-open' | 'panel-left-close' | 'panel-right-open' | 'panel-right-close'
  | 'sidebar-visible' | 'sidebar-hidden'
  | 'clock' | 'microphone'
  | 'cloud' | 'folder' | 'fileText' | 'fileCode' | 'image' | 'table' | 'externalLink'
  | 'fileStack' | 'fileSpreadsheet' | 'filePdf' | 'fileQuestion' | 'presentation' | 'archive'
  | 'gitBranch' | 'gitMerge' | 'zap' | 'coins' | 'boxes' | 'listTree'
  | 'alertTriangle' | 'appWindow'
  | 'keyboard' | 'languages' | 'shield' | 'lock' | 'key' | 'terminal'
  | 'thumbs-up' | 'thumbs-down'
  | 'music' | 'pause' | 'volume' | 'video'
  | 'skipBack' | 'skipForward' | 'shuffle' | 'repeat' | 'repeatOne'
  | 'cursor' | 'sparkles' | 'blocks' | 'droplet'
  | 'camera' | 'crosshair' | 'timer' | 'command' | 'move' | 'drag' | 'window'
  | 'mouse-left' | 'mouse-right' | 'mouse-middle' | 'mouse-double' | 'scroll-vertical'
  | 'clipboard-write' | 'clipboard-read'
  // Semantic-exclusive icons (upgrade-masterplan §4): each concept gets one
  // named glyph so call sites stop overloading gauge/sparkles/agents/blocks.
  | 'mode' | 'model' | 'plugin' | 'capabilities' | 'knowledge' | 'pet' | 'collab' | 'swarm';

interface IconDef {
  path: string;
  strokeWidth?: number;
  size?: number;
}

const ICONS: Record<IconName, IconDef> = {
  chat:       { path: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>' },
  home:       { path: '<path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/>' },
  channels:   { path: '<rect x="2" y="7" width="20" height="14" rx="2" ry="2"/><path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16"/>' },
  sessions:   { path: '<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>' },
  usage:      { path: '<line x1="12" y1="20" x2="12" y2="10"/><line x1="18" y1="20" x2="18" y2="4"/><line x1="6" y1="20" x2="6" y2="16"/>' },
  cron:       { path: '<circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>' },
  config:     { path: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>' },
  logs:       { path: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/>' },
  skills:     { path: '<polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>' },
  agents:     { path: '<rect x="3" y="11" width="18" height="10" rx="2"/><circle cx="9" cy="16" r="1"/><circle cx="15" cy="16" r="1"/><path d="M12 2v4"/><circle cx="12" cy="2" r="1"/><path d="M8 11V9a4 4 0 0 1 8 0v2"/>' },
  sun:        { path: '<circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>' },
  moon:       { path: '<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/>' },
  monitor:    { path: '<rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>' },
  languages:  { path: '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>' },
  x:          { path: '<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>' },
  copy:       { path: '<rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>' },
  check:      { path: '<polyline points="20 6 9 17 4 12"/>' },
  'thumbs-up':   { path: '<path d="M7 10v12"/><path d="M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2a3.13 3.13 0 0 1 3 3.88Z"/>' },
  'thumbs-down': { path: '<path d="M17 14V2"/><path d="M9 18.12 10 14H4.17a2 2 0 0 1-1.92-2.56l2.33-8A2 2 0 0 1 6.5 2H20a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2.76a2 2 0 0 0-1.79 1.11L12 22a3.13 3.13 0 0 1-3-3.88Z"/>' },
  send:       { path: '<line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/>' },
  play:       { path: '<polygon points="7 4 20 12 7 20 7 4"/>', strokeWidth: 1.7 },
  stop:       { path: '<rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>' },
  paperclip:  { path: '<path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/>' },
  plus:       { path: '<line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>' },
  share:      { path: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="M8.59 13.51l6.83 3.98"/><path d="M15.41 6.51 8.59 10.49"/>', strokeWidth: 1.8 },
  trash:      { path: '<polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/>' },
  refresh:    { path: '<polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>' },
  download:   { path: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/>' },
  save:       { path: '<path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><polyline points="17 21 17 13 7 13 7 21"/><polyline points="7 3 7 8 15 8"/>' },
  menu:       { path: '<line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/>' },
  moreHorizontal: { path: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>', strokeWidth: 2 },
  user:       { path: '<path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>' },
  search:     { path: '<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>' },
  eye:        { path: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>' },
  'eye-off':  { path: '<path d="M9.88 9.88a3 3 0 1 0 4.24 4.24"/><path d="M10.73 5.08A10.43 10.43 0 0 1 12 5c6.5 0 10 7 10 7a18.18 18.18 0 0 1-4.29 5.31"/><path d="M6.61 6.61A18.35 18.35 0 0 0 2 12s3.5 7 10 7a10.54 10.54 0 0 0 5.39-1.61"/><line x1="2" y1="2" x2="22" y2="22"/>' },
  edit:       { path: '<path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>' },
  // 1.5 stroke variants
  info:       { path: '<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>', strokeWidth: 1.5 },
  settings:   { path: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>', strokeWidth: 1.5 },
  gear:       { path: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>', strokeWidth: 1.5 },
  gauge:      { path: '<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>', strokeWidth: 1.5 },
  router:     { path: '<circle cx="7" cy="12" r="3"/><circle cx="17" cy="6" r="2.5"/><circle cx="17" cy="18" r="2.5"/><path d="M9.6 10.5 14.8 7.4"/><path d="M9.6 13.5 14.8 16.6"/>', strokeWidth: 1.7 },
  regenerate: { path: '<path d="M3 12a9 9 0 0 1 15.5-6.36L21 8"/><polyline points="21 3 21 8 16 8"/><path d="M21 12a9 9 0 0 1-15.5 6.36L3 16"/><polyline points="3 21 3 16 8 16"/>', strokeWidth: 1.5 },
  pencil:     { path: '<path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4z"/>', strokeWidth: 1.5 },
  fork:       { path: '<circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/>', strokeWidth: 1.5 },
  listChecks: { path: '<path d="m3 7 2 2 4-4"/><path d="m3 17 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/>', strokeWidth: 1.7 },
  chevronDown:{ path: '<polyline points="6 9 12 15 18 9"/>', strokeWidth: 1.5 },
  chevronLeft:{ path: '<polyline points="15 18 9 12 15 6"/>', strokeWidth: 1.5 },
  chevronRight:{ path: '<polyline points="9 18 15 12 9 6"/>', strokeWidth: 1.5 },
  arrowUp:    { path: '<path d="M12 19V5"/><path d="M5 12l7-7 7 7"/>', strokeWidth: 2 },
  arrowDown:  { path: '<path d="M12 5v14"/><path d="M19 12l-7 7-7-7"/>', strokeWidth: 2 },
  expand:     { path: '<polyline points="15 3 21 3 21 9"/><polyline points="9 21 3 21 3 15"/><line x1="21" y1="3" x2="14" y2="10"/><line x1="3" y1="21" x2="10" y2="14"/>', strokeWidth: 1.5 },
  collapse:   { path: '<polyline points="4 14 10 14 10 20"/><polyline points="20 10 14 10 14 4"/><line x1="10" y1="14" x2="3" y2="21"/><line x1="14" y1="10" x2="21" y2="3"/>', strokeWidth: 1.5 },
  'panel-left-open':  { path: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/><path d="M14 9l6 6-6 6"/>' },
  'panel-left-close': { path: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18"/><path d="M20 9l-6 6 6 6"/>' },
  'panel-right-open': {
    path: '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M14.5 4.5v15"/>',
    strokeWidth: 1.5,
  },
  'panel-right-close': {
    path: '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M15.5 9v6"/>',
    strokeWidth: 1.5,
  },
  'sidebar-visible': {
    path: '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M9.5 4.5v15"/>',
    strokeWidth: 1.5,
  },
  'sidebar-hidden': {
    path: '<rect x="3.5" y="4.5" width="17" height="15" rx="3"/><path d="M8.5 9v6"/>',
    strokeWidth: 1.5,
  },
  clock: { path: '<circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/>' },
  microphone: { path: '<path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><path d="M12 19v3"/><path d="M8 22h8"/>', strokeWidth: 1.7 },
  cloud: { path: '<path d="M17.5 19H8a6 6 0 1 1 1.03-11.91A7 7 0 0 1 22 12.5 4.5 4.5 0 0 1 17.5 19z"/>', strokeWidth: 1.5 },
  folder: { path: '<path d="M3 6.5A2.5 2.5 0 0 1 5.5 4H9l2 2h7.5A2.5 2.5 0 0 1 21 8.5v8A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5z"/>', strokeWidth: 1.7 },
  fileText: { path: '<path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/><path d="M14 2v5h5"/><path d="M9 13h6"/><path d="M9 17h4"/>', strokeWidth: 1.5 },
  fileCode: { path: '<path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/><path d="M14 2v5h5"/><path d="m10 13-2 2 2 2"/><path d="m14 13 2 2-2 2"/>', strokeWidth: 1.5 },
  image: { path: '<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="8.5" cy="10.5" r="1.5"/><path d="m21 15-4.5-4.5L6 19"/>', strokeWidth: 1.5 },
  table: { path: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 10h18"/><path d="M9 4v16"/><path d="M15 4v16"/>', strokeWidth: 1.5 },
  externalLink: { path: '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>', strokeWidth: 1.5 },
  keyboard:   { path: '<rect width="20" height="16" x="2" y="4" rx="2"/><path d="M6 8h.01"/><path d="M10 8h.01"/><path d="M14 8h.01"/><path d="M18 8h.01"/><path d="M8 12h.01"/><path d="M12 12h.01"/><path d="M16 12h.01"/><path d="M7 16h10"/>' },
  terminal:   { path: '<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>' },
  shield:     { path: '<path d="M20 13c0 5-3.5 7.5-8 9-4.5-1.5-8-4-8-9V5l8-3 8 3v8z"/>', strokeWidth: 1.7 },
  lock:       { path: '<rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>', strokeWidth: 1.7 },
  key:        { path: '<circle cx="7.5" cy="15.5" r="4.5"/><path d="m10.7 12.3 8.3-8.3"/><path d="m16 6 2.5 2.5"/><path d="m12.5 9.5 2 2"/>', strokeWidth: 1.7 },
  music:      { path: '<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>', strokeWidth: 1.7 },
  skipBack:   { path: '<polygon points="19 20 9 12 19 4 19 20"/><line x1="5" y1="19" x2="5" y2="5"/>', strokeWidth: 1.7 },
  skipForward:{ path: '<polygon points="5 4 15 12 5 20 5 4"/><line x1="19" y1="5" x2="19" y2="19"/>', strokeWidth: 1.7 },
  shuffle:    { path: '<polyline points="16 3 21 3 21 8"/><line x1="4" y1="20" x2="21" y2="3"/><polyline points="21 16 21 21 16 21"/><line x1="15" y1="15" x2="21" y2="21"/><line x1="4" y1="4" x2="9" y2="9"/>', strokeWidth: 1.6 },
  repeat:     { path: '<polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/>', strokeWidth: 1.6 },
  repeatOne:  { path: '<polyline points="17 1 21 5 17 9"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><polyline points="7 23 3 19 7 15"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/><path d="M11 15v-4.5"/><path d="M13.5 11.5a1.3 1.3 0 1 0-2.6 0c0 .9 2.6 1.1 2.6 2.4" />', strokeWidth: 1.6 },
  video:      { path: '<rect x="3" y="5" width="14" height="14" rx="2"/><path d="m17 10 4-2v8l-4-2z"/>', strokeWidth: 1.7 },
  cursor:     { path: '<path d="M4 4l7 18 2-7 7-2z"/>', strokeWidth: 1.7 },
  sparkles:   { path: '<path d="M12 3l1.5 4.5L18 9l-4.5 1.5L12 15l-1.5-4.5L6 9l4.5-1.5z"/><path d="M19 14l.8 2.2L22 17l-2.2.8L19 20l-.8-2.2L16 17l2.2-.8z"/>', strokeWidth: 1.5 },
  blocks:     { path: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>', strokeWidth: 1.7 },
  droplet:    { path: '<path d="M12 2.7 17.7 8.4a8 8 0 1 1-11.4 0z"/><path d="M9.6 12.2a2.6 2.6 0 0 1 1.6-2.4"/>', strokeWidth: 1.6 },
  pause:      { path: '<rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/>', strokeWidth: 1.7 },
  volume:     { path: '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/>', strokeWidth: 1.7 },

  // Artifact-kind family. Every member shares fileText's page silhouette so a
  // sidebar column reads as one set — the inner mark is what names the format,
  // and the type colour (--artifact-doc/sheet/deck/...) carries the second
  // signal. fileStack breaks the silhouette on purpose: it means "the set of
  // artifacts", not one file, so it belongs to the empty state, not the rows.
  fileStack: { path: '<path d="M21 7h-3a2 2 0 0 1-2-2V2"/><path d="M21 6v6.5c0 .83-.67 1.5-1.5 1.5h-7c-.83 0-1.5-.67-1.5-1.5v-9c0-.83.67-1.5 1.5-1.5H17z"/><path d="M7 8v8.8c0 .66.54 1.2 1.2 1.2H15"/><path d="M3 12v8.8c0 .66.54 1.2 1.2 1.2H11"/>', strokeWidth: 1.5 },
  fileSpreadsheet: { path: '<path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/><path d="M14 2v5h5"/><path d="M8 13h8"/><path d="M8 17h8"/><path d="M11.6 11.5v8"/>', strokeWidth: 1.5 },
  // "PD" letterform strokes inside the page: dense-text page at 16px, legibly
  // PDF at 20/24px. Light stroke keeps the glyph from blotting at small sizes.
  filePdf: { path: '<path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/><path d="M14 2v5h5"/><path d="M8.6 18v-6"/><path d="M8.6 12h1.7a1.5 1.5 0 0 1 0 3H8.6"/><path d="M13.4 18v-6h1.1a2 2 0 0 1 2 2v2a2 2 0 0 1-2 2z"/>', strokeWidth: 1.5 },
  fileQuestion: { path: '<path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7z"/><path d="M14 2v5h5"/><path d="M9.7 13a2.4 2.4 0 0 1 4.4 1.3c0 1.5-2.2 1.9-2.2 1.9"/><path d="M12 19h.01"/>', strokeWidth: 1.5 },
  presentation: { path: '<path d="M2 3h20"/><path d="M21 3v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V3"/><path d="m7 21 5-5 5 5"/>', strokeWidth: 1.5 },
  archive: { path: '<rect x="2" y="3" width="20" height="5" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>', strokeWidth: 1.5 },

  // Routing family — the Bloom strip's vocabulary. gitBranch = a decision that
  // forked, gitMerge = candidates converging into a fused answer, boxes = a
  // parallel fan-out, zap = the direct/fast tier, coins = token spend.
  gitBranch: { path: '<line x1="6" y1="3" x2="6" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>', strokeWidth: 1.7 },
  gitMerge: { path: '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>', strokeWidth: 1.7 },
  zap: { path: '<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>', strokeWidth: 1.7 },
  coins: { path: '<circle cx="8" cy="8" r="6"/><path d="M18.09 10.37A6 6 0 1 1 10.34 18"/><path d="M7 6h1v4"/><path d="m16.71 13.88.7.71-2.82 2.82"/>', strokeWidth: 1.5 },
  boxes: { path: '<rect x="8.5" y="3" width="7" height="7" rx="1"/><rect x="3" y="13.5" width="7" height="7" rx="1"/><rect x="14" y="13.5" width="7" height="7" rx="1"/>', strokeWidth: 1.7 },

  listTree: { path: '<path d="M21 6h-8"/><path d="M21 12h-8"/><path d="M21 18h-8"/><path d="M4 4v13a2 2 0 0 0 2 2h2"/><path d="M4 10h5"/>', strokeWidth: 1.5 },
  alertTriangle: { path: '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>', strokeWidth: 1.7 },
  appWindow: { path: '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="M2 9h20"/><path d="M6 4v5"/><path d="M10 4v5"/>', strokeWidth: 1.5 },

  // Computer-use action icons. The mouse family shares one silhouette so the
  // button being described is read from the highlighted region, not from a
  // different drawing — keeps an 18-item grid scannable at 16px.
  camera:     { path: '<path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3z"/><circle cx="12" cy="13" r="3.5"/>', strokeWidth: 1.7 },
  crosshair:  { path: '<circle cx="12" cy="12" r="9"/><path d="M21 12h-4"/><path d="M7 12H3"/><path d="M12 7V3"/><path d="M12 21v-4"/>', strokeWidth: 1.7 },
  timer:      { path: '<path d="M10 2h4"/><circle cx="12" cy="14" r="8"/><path d="M12 14l3-3"/>', strokeWidth: 1.7 },
  command:    { path: '<path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/>', strokeWidth: 1.7 },
  move:       { path: '<path d="M12 3v18"/><path d="M3 12h18"/><path d="m9.5 5.5 2.5-2.5 2.5 2.5"/><path d="m9.5 18.5 2.5 2.5 2.5-2.5"/><path d="m5.5 9.5-2.5 2.5 2.5 2.5"/><path d="m18.5 9.5 2.5 2.5-2.5 2.5"/>', strokeWidth: 1.7 },
  drag:       { path: '<path d="M7 3.5l6 14 1.9-5.6 5.6-1.9z"/><path d="M3.5 8.5h2"/><path d="M3.5 12.5h3"/><path d="M3.5 16.5h4"/>', strokeWidth: 1.7 },
  window:     { path: '<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 8.5h18"/><circle cx="6.6" cy="5.8" r="0.9" fill="currentColor" stroke="none"/><circle cx="9.6" cy="5.8" r="0.9" fill="currentColor" stroke="none"/>', strokeWidth: 1.7 },
  'mouse-left':      { path: '<rect x="6" y="2" width="12" height="20" rx="6"/><path d="M12 2.5v6"/><path d="M6.2 8.5h11.6"/><circle cx="9" cy="5.2" r="1.1" fill="currentColor" stroke="none"/>', strokeWidth: 1.7 },
  'mouse-right':     { path: '<rect x="6" y="2" width="12" height="20" rx="6"/><path d="M12 2.5v6"/><path d="M6.2 8.5h11.6"/><circle cx="15" cy="5.2" r="1.1" fill="currentColor" stroke="none"/>', strokeWidth: 1.7 },
  'mouse-middle':    { path: '<rect x="6" y="2" width="12" height="20" rx="6"/><path d="M6.2 9.5h11.6"/><rect x="10.9" y="4" width="2.2" height="5" rx="1.1" fill="currentColor" stroke="none"/>', strokeWidth: 1.7 },
  'mouse-double':    { path: '<rect x="6" y="2" width="12" height="20" rx="6"/><path d="M12 2.5v6"/><path d="M6.2 8.5h11.6"/><circle cx="9" cy="4" r="0.95" fill="currentColor" stroke="none"/><circle cx="9" cy="6.8" r="0.95" fill="currentColor" stroke="none"/>', strokeWidth: 1.7 },
  'scroll-vertical': { path: '<rect x="7" y="2" width="10" height="20" rx="5"/><path d="m10.4 8.6 1.6-2 1.6 2"/><path d="m10.4 15.4 1.6 2 1.6-2"/>', strokeWidth: 1.7 },
  'clipboard-write': { path: '<path d="M16 4h2a2 2 0 0 1 2 2v3.5"/><path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h6"/><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M18.4 12.6a2 2 0 0 1 2.8 2.8L17 19.6l-3 .8.8-3z"/>', strokeWidth: 1.7 },
  'clipboard-read':  { path: '<path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/><path d="M9 13h6"/><path d="M9 17h4"/>', strokeWidth: 1.7 },

  // ── Semantic-exclusive family (upgrade-masterplan §4) ──
  // mode: a layered diamond — declared spec cascading into execution tier.
  mode: { path: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M12 8l4 2.25v4.5L12 17l-4-2.25v-4.5z"/><path d="M12 3v5"/>', strokeWidth: 1.6 },
  // model: a chip with a signal wave — the LLM service.
  model: { path: '<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 9h.01"/><path d="M15 9h.01"/><path d="M9 15h.01"/><path d="M15 15h.01"/><path d="M12 2v3"/><path d="M12 19v3"/><path d="M2 12h3"/><path d="M19 12h3"/><path d="M4 10v4"/><path d="M20 10v4"/>', strokeWidth: 1.6 },
  // plugin: a puzzle piece — extendable capability.
  plugin: { path: '<path d="M14 4a2 2 0 0 0-4 0v2H6a2 2 0 0 0-2 2v3h2a2 2 0 1 1 0 4H4v3a2 2 0 0 0 2 2h3v-2a2 2 0 1 1 4 0v2h3a2 2 0 0 0 2-2v-3h-2a2 2 0 1 1 0-4h2V8a2 2 0 0 0-2-2h-4z"/>', strokeWidth: 1.6 },
  // capabilities: a hexagon radar — capability surface.
  capabilities: { path: '<path d="M12 3l7.5 4.5v9L12 21l-7.5-4.5v-9z"/><path d="M12 8l4 2.4v4.8L12 16l-4-2.4V10.4z"/><path d="M12 3v5"/>', strokeWidth: 1.6 },
  // knowledge: a book with a node — KB / memory graph.
  knowledge: { path: '<path d="M4 5a2 2 0 0 1 2-2h5v16H6a2 2 0 0 0-2 2z"/><path d="M20 5a2 2 0 0 0-2-2h-5v16h5a2 2 0 0 1 2 2z"/><circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none"/>', strokeWidth: 1.6 },
  // pet: a paw — the companion creature.
  pet: { path: '<circle cx="6.5" cy="10" r="1.8"/><circle cx="11" cy="7.5" r="1.8"/><circle cx="15.5" cy="7.5" r="1.8"/><circle cx="17.5" cy="10.5" r="1.8"/><path d="M12 12c-2.2 0-4 1.6-4 3.4 0 1.7 1.5 2.6 4 2.6s4-.9 4-2.6c0-1.8-1.8-3.4-4-3.4z"/>', strokeWidth: 1.6 },
  // collab: two nodes linked by a chain — remote collaboration / SSH.
  collab: { path: '<circle cx="7" cy="8" r="3"/><circle cx="17" cy="16" r="3"/><path d="M9.5 10.2 14.5 13.8"/><path d="M14.5 10.2 9.5 13.8"/>', strokeWidth: 1.6 },
  // swarm: scattered dots converging into a core — swarm / fleet.
  swarm: { path: '<circle cx="12" cy="12" r="2.4" fill="currentColor" stroke="none"/><circle cx="5" cy="6" r="1.6"/><circle cx="19" cy="6" r="1.6"/><circle cx="4" cy="17" r="1.6"/><circle cx="20" cy="17" r="1.6"/><circle cx="12" cy="3" r="1.6"/><path d="M6 7l4 3.5M18 7l-4 3.5M5.5 15.5 10 13M18.5 15.5 14 13M12 5v4.5"/>', strokeWidth: 1.5 },
};

export function getIconSvg(name: IconName, size: number = 16): string {
  const icon = ICONS[name];
  if (!icon) {
    console.warn(`[icons] Unknown icon: ${name}`);
    return '';
  }
  const strokeWidth = icon.strokeWidth ?? 2;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${strokeWidth}" stroke-linecap="round" stroke-linejoin="round">${icon.path}</svg>`;
}
