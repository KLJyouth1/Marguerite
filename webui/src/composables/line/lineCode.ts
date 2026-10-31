// 工作流层的本地状态码 → 界面文案。
//
// `useLineWorkflow` 不认识 i18n（store 不该拿翻译），它只回大写 CODE；后端与校验器的原文
// 则本来就是给人看的话。这个文件是两者之间唯一的翻译点：认得的码翻成当前语言，
// 认不出的**原样透传**——把未知码咽掉，等于把一个真故障显示成空白。
const LINE_CODE_KEYS: Readonly<Record<string, string>> = {
  LINE_NOT_ARMED: 'notArmed',
  LINE_CATALOG_UNAVAILABLE: 'catalogUnavailable',
  LINE_PREFLIGHT_EMPTY: 'preflightEmpty',
  LINE_PREFLIGHT_MISSING: 'preflightMissing',
  LINE_RUN_UNEXPECTED_RESPONSE: 'runUnexpectedResponse',
  LINE_RUN_NOT_PERSISTED: 'runNotPersisted',
  LINE_RETRO_UNAVAILABLE: 'retroUnavailable',
}

export function lineCodeText(
  value: string | null | undefined,
  translate: (key: string) => string,
): string {
  if (!value) return ''
  const key = LINE_CODE_KEYS[value.trim()]
  return key ? translate(`chat.line.code.${key}`) : value
}
