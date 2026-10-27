export interface SettingsSearchEntry {
  id: string
  /** Localized rail label for the current UI language. */
  label: string
}

// Technical synonyms stay language-neutral on purpose: users search for
// "api key"/"ssh"/"pet" regardless of the UI language, while localized labels
// are always included by the caller.
const SECTION_KEYWORDS: Record<string, string[]> = {
  connection: ['gateway', 'endpoint', 'host', 'port', 'token', '连接', '网关'],
  runtime: ['desktop', 'logs', 'restart', 'update', '运行时', '日志', '重启'],
  doctor: ['health', 'diagnostics', 'status', '体检', '诊断'],
  provider: ['api key', 'model service', 'base url', 'provider', '密钥', '模型', '服务商'],
  modelStrategy: ['routing', 'router', 'ensemble', 'fallback', '路由', '模型策略'],
  capabilities: ['tools', 'skills', 'vision', '能力', '工具', '技能'],
  behavior: ['persona', 'titles', 'temperature', '行为', '人格'],
  privacy: ['telemetry', 'network', 'observability', '隐私', '遥测'],
  sandbox: ['permissions', 'approval', 'policy', '沙箱', '权限', '审批'],
  memory: ['profile', 'knowledge', 'layers', '记忆', '画像', '知识库'],
  appearance: ['theme', 'glass', 'font', '外观', '主题', '字体'],
  keyboard: ['shortcuts', 'hotkeys', '快捷键'],
  advanced: ['debug', 'logs', 'data migration', '高级', '调试', '迁移'],
  plugins: ['extensions', 'plugin', '插件'],
  agentPresets: ['presets', 'persona', '预设', '智能体'],
  pluginMarket: ['market', 'install', '市场', '安装'],
  webUiPlugins: ['web ui', 'plugins', '界面插件'],
  skinCenter: ['skin', 'theme', '皮肤', '主题'],
  pet: ['desktop pet', 'companion', '宠物', '桌宠'],
  communityPlugins: ['community', 'plugins', '社区', '插件'],
  graph: ['graph', 'knowledge graph', '图谱'],
  collabSsh: ['ssh', 'collaboration', 'remote', '协同', '远程'],
}

export function normalizeSettingsQuery(value: string): string {
  return value.trim().toLowerCase()
}

export function settingsSectionKeywords(id: string): string[] {
  return SECTION_KEYWORDS[id] ?? []
}

export function matchesSettingsSection(entry: SettingsSearchEntry, query: string): boolean {
  const normalized = normalizeSettingsQuery(query)
  if (!normalized) return true
  const haystack = [entry.id, entry.label, ...settingsSectionKeywords(entry.id)]
    .join(' ')
    .toLowerCase()
  return haystack.includes(normalized)
}

export function filterSettingsSections<T extends SettingsSearchEntry>(
  sections: T[],
  query: string,
): T[] {
  const normalized = normalizeSettingsQuery(query)
  if (!normalized) return sections
  return sections.filter(section => matchesSettingsSection(section, normalized))
}
