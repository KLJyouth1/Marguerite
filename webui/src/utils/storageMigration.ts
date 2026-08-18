/**
 * 旧版 storage 键迁移（融合 Phase 1.3）：Marguerite 时代的键使用
 * `Marguerite.*` 前缀（主题键为无点的 `Marguerite-theme`），融合后统一为
 * `marguerite.*`。应用启动时一次性迁移，老用户既有偏好（主题、草稿、
 * 会话键、工作台宽度、WS 连接等）不丢失；目标键已存在时绝不覆盖。
 */

const LEGACY_PREFIX = 'Marguerite.'
const BRAND_PREFIX = 'marguerite.'
const LEGACY_THEME_KEY = 'Marguerite-theme'
const BRAND_THEME_KEY = 'marguerite-theme'

function migrateStore(store: Storage): void {
  const legacyKeys: string[] = []
  for (let i = 0; i < store.length; i++) {
    const key = store.key(i)
    if (key && (key.startsWith(LEGACY_PREFIX) || key === LEGACY_THEME_KEY)) {
      legacyKeys.push(key)
    }
  }
  for (const key of legacyKeys) {
    const target =
      key === LEGACY_THEME_KEY
        ? BRAND_THEME_KEY
        : key.replace(LEGACY_PREFIX, BRAND_PREFIX)
    const value = store.getItem(key)
    if (value !== null && store.getItem(target) === null) {
      store.setItem(target, value)
    }
    store.removeItem(key)
  }
}

/**
 * 启动迁移入口。必须在任何读取 storage 的模块初始化之前调用
 * （main.ts 顶部，createApp 前）。隐私模式或非浏览器环境下静默跳过。
 */
export function migrateLegacyStorageKeys(): void {
  if (typeof localStorage === 'undefined') return
  try {
    migrateStore(localStorage)
  } catch {
    // localStorage 不可用（隐私模式）时静默跳过
  }
  if (typeof sessionStorage === 'undefined') return
  try {
    migrateStore(sessionStorage)
  } catch {
    // sessionStorage 不可用（隐私模式）时静默跳过
  }
}
