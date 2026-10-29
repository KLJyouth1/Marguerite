/**
 * 「图片来源 / 素材源」四选一的**纯值形状**与校验 —— 不含任何 UI / DOM / i18n 依赖。
 *
 * 语义（与其它团队问卷共用的同一套口径，供跨团队复用）：
 * - `web`（A 网络）：素材来自网络（URL 等，可选填具体地址）；
 * - `generate`（B 自动生成）：不预先指定素材，由模型自行生成；
 * - `directory`（C 指定目录）：从用户填写的本机目录取材 —— **必须填目录路径**；
 * - `custom`（D 其他）：用户自由填写来源说明 —— **必须填写**。
 *
 * 值形状刻意保持最小：`{ kind, value }`，`value` 对 A 是**可选**的 URL、对 B 恒为空；
 * `normalizeMaterialSource` 丢弃 B 的残留值，A 的选填地址则保留 —— 因为
 * `generateParams.buildToolArgs` 只在 `kind === 'web'` 且值「像 URL」时才把它写进
 * 视频的 `image` 参数（A 的地址是全仓唯一能进入真实后端参数的素材源值）。
 * ⚠️ 「切换选项后不残留上一选项的输入值」由 UI 层负责（选项目变更即清空输入框）：
 * 纯函数若连 A 的值一起抹掉，A→URL 这条链路就永远取不到值，等于把上面的联动写成死代码。
 */

export type MaterialSourceKind = 'web' | 'generate' | 'directory' | 'custom'

export interface MaterialSource {
  kind: MaterialSourceKind
  /** A 选填（URL）；C（目录路径）与 D（自由填写）必须非空才可提交；B 恒为空串。 */
  value: string
}

/** 展示顺序固定为 A→B→C→D（与问卷口径一致）。 */
export const MATERIAL_SOURCE_KINDS: readonly MaterialSourceKind[] = [
  'web',
  'generate',
  'directory',
  'custom',
]

/** 默认选 B（自动生成）：文生图/文生视频的主路径，无需任何附加值。 */
export const DEFAULT_MATERIAL_SOURCE_KIND: MaterialSourceKind = 'generate'

export function emptyMaterialSource(): MaterialSource {
  return { kind: DEFAULT_MATERIAL_SOURCE_KIND, value: '' }
}

/** C/D 是「必须填值」的选项；A/B 无附加值也可提交。 */
export function materialSourceNeedsValue(kind: MaterialSourceKind): boolean {
  return kind === 'directory' || kind === 'custom'
}

/**
 * 「允许携带值」的选项：A（选填 URL）与 C/D（必填）都携带，只有 B 恒不携带。
 * ⚠️ 与 `materialSourceNeedsValue` 是**两个不同的谓词**：后者答「是否必须填」，
 * 本函数答「归一化时是否保留输入」。用后者去做保留判断会抹掉 A 的 URL。
 */
export function materialSourceAcceptsValue(kind: MaterialSourceKind): boolean {
  return kind !== 'generate'
}

/**
 * 归一化：trim 输入；只有 B（自动生成）一律丢弃 value。
 * 非法 kind 回落默认选项（不抛错：调用方可能是持久化恢复的脏数据）。
 */
export function normalizeMaterialSource(kind: MaterialSourceKind, rawValue: string): MaterialSource {
  const safeKind: MaterialSourceKind = MATERIAL_SOURCE_KINDS.includes(kind) ? kind : DEFAULT_MATERIAL_SOURCE_KIND
  const value = materialSourceAcceptsValue(safeKind) ? String(rawValue ?? '').trim() : ''
  return { kind: safeKind, value }
}

/** 是否可提交（C 未填目录 / D 未填写 → false）。 */
export function materialSourceIsValid(source: MaterialSource): boolean {
  return materialSourceErrorKey(source) === null
}

/**
 * 校验失败的 i18n 键后缀（组件用它取文案）；合法时返回 null。
 * 键定义在 `settings.materialSource.*` 命名空间下。
 */
export function materialSourceErrorKey(source: MaterialSource): 'directoryRequired' | 'customRequired' | null {
  if (source.kind === 'directory' && !source.value.trim()) return 'directoryRequired'
  if (source.kind === 'custom' && !source.value.trim()) return 'customRequired'
  return null
}
