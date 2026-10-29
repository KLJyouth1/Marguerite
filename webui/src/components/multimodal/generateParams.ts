/**
 * `image_generate` / `video_generate` 的参数面 —— 字段定义**逐条对齐 Rust 工具的真实
 * input_schema 与 build_body 白名单**，只暴露后端真正消费的键；绝不为 UI 造后端不吃的参数。
 *
 * 权威取证（只读 core/，本模块每个字段都带 `文件:行` 出处）：
 * - `core/src/tools/image_gen.rs:208-218` input_schema；`:163` build_body 白名单
 *   （["size","n","quality","style","response_format","negative_prompt"]）；
 *   `:165-170` n 解析为 u32（失败回落 1）；`:288-291` out 在 execute 里读取。
 * - `core/src/tools/video_gen.rs:578-595` input_schema；`:420-430` build_body 白名单
 *   （duration/size/resolution/aspect_ratio/fps/seed/negative_prompt/image/response_format）；
 *   `:432-435` duration/fps/seed 尽量转数字；`:682-685` out 在 execute 里读取。
 *
 * **故意不暴露**（写在这里是为了审查时一眼能对上）：
 * - `response_format`：在白名单里但**不在 input_schema**（image_gen.rs:209-217 /
 *   video_gen.rs:579-594 都没有它）—— 属工具内部交付形态（默认 b64_json，见
 *   image_gen.rs:150），不是给调用方的参数，暴露它等于造假控件的反面（造真但不该露的控件）。
 * - 视频异步轮询四件套 `status_url` / `status_method` / `poll_interval_ms` /
 *   `poll_timeout_s`（video_gen.rs:590-593）：是异步任务的管道覆盖，默认值自洽
 *   （5s 起轮询 / 480s 预算，video_gen.rs:575），放面板只会诱导误配；留后端默认。
 *
 * 传输事实：工具 execute 收 `HashMap<String, String>`（image_gen.rs:220 /
 * video_gen.rs:597），所以**所有值都以字符串序列化**是正确形态，不是妥协。
 */

import type { MaterialSource } from './materialSource'

export type GenerateTool = 'image' | 'video'

/**
 * 文案命名空间（`src/locales/*.json` 的 `settings.<ns>.*`）。
 * 字段表里的 `labelKey` / `placeholderKey` 与 `materialSourceErrorKey()` 的返回值
 * 都是**后缀**，组件拼成 `settings.${ns}.${suffix}` 后交给 `t()` —— 所以命名空间
 * 只能有一个事实源（单测 `generateParams.test.ts` 会逐条比对 en 字典）。
 */
export const MATERIAL_GEN_NAMESPACE = 'multimodalGen'
export const MATERIAL_SOURCE_NAMESPACE = 'materialSource'

/** Rust 侧的真实工具名（下发给主智能体的指令必须用它，不用 UI 别名）。 */
export const GENERATE_TOOL_NAMES: Record<GenerateTool, string> = {
  image: 'image_generate',
  video: 'video_generate',
}

export type ParamInputType = 'text' | 'number' | 'integer'

export interface ParamFieldDef {
  /** 后端真实消费的参数名（原样进入工具参数表）。 */
  key: string
  /** `settings.multimodalGen.*` 下的文案键后缀。 */
  labelKey: string
  /** `settings.multimodalGen.*` 下的占位文案键后缀（可无）。 */
  placeholderKey?: string
  type: ParamInputType
  required?: boolean
  /**
   * 后端按**无符号**整数解析该字段（`n` → `parse::<u32>`，image_gen.rs:165）。
   * ⚠️ 只对显式声明它的字段拒绝负数：`seed` 走 `json_number_or_text` 的
   * `parse::<i64>`（video_gen.rs:443），**负种子在后端合法**，一律按 u32 口径
   * 拦就是闸门误伤 —— 用户填了合法值却被 `invalidNumber` 挡住提交。
   */
  unsigned?: boolean
  tools: readonly GenerateTool[]
  /** Rust 权威出处（文件:行），审查用。 */
  evidence: string
}

export const GENERATE_PARAM_FIELDS: readonly ParamFieldDef[] = [
  // —— 两工具共用 ——
  { key: 'prompt', labelKey: 'promptLabel', placeholderKey: 'promptPlaceholder', type: 'text', required: true, tools: ['image', 'video'], evidence: 'image_gen.rs:210 / video_gen.rs:580' },
  { key: 'size', labelKey: 'sizeLabel', placeholderKey: 'sizePlaceholder', type: 'text', tools: ['image', 'video'], evidence: 'image_gen.rs:211,163 / video_gen.rs:583,422' },
  { key: 'negative_prompt', labelKey: 'negativeLabel', placeholderKey: 'negativePlaceholder', type: 'text', tools: ['image', 'video'], evidence: 'image_gen.rs:215,163 / video_gen.rs:588,427' },
  { key: 'out', labelKey: 'outLabel', placeholderKey: 'outPlaceholder', type: 'text', tools: ['image', 'video'], evidence: 'image_gen.rs:216,288 / video_gen.rs:589,682' },
  // —— 仅图像 ——
  { key: 'n', labelKey: 'nLabel', type: 'integer', unsigned: true, tools: ['image'], evidence: 'image_gen.rs:212,165（u32 解析，失败回落 1）' },
  { key: 'quality', labelKey: 'qualityLabel', placeholderKey: 'qualityPlaceholder', type: 'text', tools: ['image'], evidence: 'image_gen.rs:213,163' },
  { key: 'style', labelKey: 'styleLabel', placeholderKey: 'stylePlaceholder', type: 'text', tools: ['image'], evidence: 'image_gen.rs:214,163' },
  // —— 仅视频 ——
  { key: 'image', labelKey: 'firstFrameLabel', placeholderKey: 'firstFramePlaceholder', type: 'text', tools: ['video'], evidence: 'video_gen.rs:581,428（URL 或 data URI）' },
  { key: 'duration', labelKey: 'durationLabel', type: 'number', tools: ['video'], evidence: 'video_gen.rs:582,433' },
  { key: 'resolution', labelKey: 'resolutionLabel', placeholderKey: 'resolutionPlaceholder', type: 'text', tools: ['video'], evidence: 'video_gen.rs:584,423' },
  { key: 'aspect_ratio', labelKey: 'aspectRatioLabel', type: 'text', tools: ['video'], evidence: 'video_gen.rs:585,424' },
  { key: 'fps', labelKey: 'fpsLabel', type: 'number', tools: ['video'], evidence: 'video_gen.rs:586,433' },
  { key: 'seed', labelKey: 'seedLabel', type: 'integer', tools: ['video'], evidence: 'video_gen.rs:587,433（json_number_or_text 按 i64）' },
]

export function fieldsForTool(tool: GenerateTool): ParamFieldDef[] {
  return GENERATE_PARAM_FIELDS.filter(f => f.tools.includes(tool)).map(f => ({ ...f, tools: [tool] }))
}

/** 数值字段：空串合法（不提交）；否则必须是有限数，integer 还必须是整数（`unsigned` 再要求非负）。 */
export function numberFieldIsValid(def: ParamFieldDef, raw: string): boolean {
  const v = String(raw ?? '').trim()
  if (!v) return true
  if (def.type === 'text') return true
  const n = Number(v)
  if (!Number.isFinite(n)) return false
  if (def.type === 'integer') return Number.isInteger(n) && (!def.unsigned || n >= 0)
  return true
}

/** 找到第一个非法的数值字段键；全部合法返回 null。 */
export function invalidNumberField(tool: GenerateTool, values: Record<string, string>): string | null {
  for (const def of fieldsForTool(tool)) {
    if (def.type === 'text') continue
    if (!numberFieldIsValid(def, values[def.key] ?? '')) return def.key
  }
  return null
}

/**
 * 「网络」源的 value 只有在**看起来是 URL / data URI** 时才能填进视频 `image`
 * 参数（video_gen.rs:581 只认这两种形态）；否则只作为指令文字随消息下发。
 */
export function looksLikeImageRef(value: string): boolean {
  const v = value.trim()
  return /^(https?:\/\/|data:)/i.test(v)
}

/**
 * 把面板状态序列化为工具真实参数表（全部字符串；空值不提交，与 Rust
 * `filter(|s| !s.is_empty())`（image_gen.rs:164）同口径）。
 *
 * 素材源联动规则（保守、可测）：
 * - A(web) + 视频 + value 像 URL/data URI + 用户没手填 firstFrame → 填 `image`；
 * - A 但手填了 firstFrame → **手填优先**（用户在参数面里的显式输入赢过来源推断）；
 * - B(generate)/C(directory)/D(custom) → **不写入任何参数**（后端没有对应键，
 *   造键就是造假控件），只随指令文字下发给主智能体。
 */
export function buildToolArgs(
  tool: GenerateTool,
  values: Record<string, string>,
  source: MaterialSource,
): Record<string, string> {
  const args: Record<string, string> = {}
  for (const def of fieldsForTool(tool)) {
    const raw = String(values[def.key] ?? '').trim()
    if (!raw) continue
    args[def.key] = raw
  }
  if (tool === 'video' && source.kind === 'web' && !args.image && looksLikeImageRef(source.value)) {
    args.image = source.value.trim()
  }
  return args
}

export type LaunchBlocking = 'needPrompt' | 'invalidNumber' | 'directoryRequired' | 'customRequired'

/**
 * 发起闸门（真值表见单测）：
 * prompt 缺失最优先，其次数值非法，最后素材源 C/D 未填。
 * 返回 null 表示可提交。
 */
export function launchBlocking(
  tool: GenerateTool,
  values: Record<string, string>,
  source: MaterialSource,
): LaunchBlocking | null {
  if (!String(values.prompt ?? '').trim()) return 'needPrompt'
  if (invalidNumberField(tool, values)) return 'invalidNumber'
  if (source.kind === 'directory' && !source.value.trim()) return 'directoryRequired'
  if (source.kind === 'custom' && !source.value.trim()) return 'customRequired'
  return null
}

/** 参数预览（语言中立的 `k="v"` 串；prompt 由消息模板单独引用，不重复列出）。 */
export function argsPreview(args: Record<string, string>): string {
  return Object.entries(args)
    .filter(([k]) => k !== 'prompt')
    .map(([k, v]) => `${k}="${v}"`)
    .join('; ')
}
