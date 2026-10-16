// 3×3 user-mode copy ("极简·咨询" / "Minimal · Ask") — i18n first, metadata fallback.
//
// WHY THIS EXISTS
// The 3×3 user mode is the single source of truth for the runtime mode
// (`webui/src/stores/activeMode.ts`). Its labels used to be hardcoded Chinese in
// each component (ModeMatrix) or absent entirely (nothing existed for the top-bar
// badge), so an English UI leaked Chinese — or the badge fell back to the legacy
// five runtime kinds and showed a different vocabulary than the composer grid.
//
// The catalog lives under `mode.userMode.<key>` / `mode.userModeHint.<key>` in every
// bundled locale. `USER_MODE_META` stays the fallback so a payload from an older
// gateway (or a locale chunk that failed to load) still renders a real label
// instead of leaking the raw i18n key.
import { t } from '@/i18n'
import { USER_MODE_META } from '@/stores/activeMode'

function translated(key: string, fallback: string): string {
  const value = t(key)
  // vue-i18n returns the key itself when the message is missing; never let that
  // reach the UI (a visible `mode.userMode.minimal_ask` is worse than a fallback).
  return typeof value === 'string' && value !== key ? value : fallback
}

/** Localized label for a 3×3 slot key, e.g. `standard_doit` -> "标准·执行". */
export function userModeLabel(key: string): string {
  const meta = USER_MODE_META[key]
  return translated(`mode.userMode.${key}`, meta?.label ?? key)
}

/** Localized one-line description for a 3×3 slot key. */
export function userModeHint(key: string): string {
  return translated(`mode.userModeHint.${key}`, '')
}

/** Localized intensity-axis label, e.g. `extreme` -> "极致" / "Extreme". */
export function userModeIntensityLabel(intensity: string): string {
  const meta = Object.values(USER_MODE_META).find((m) => m.intensity === intensity)
  return translated(
    `mode.intensity.${intensity}`,
    meta?.intensityLabel ?? intensity,
  )
}

/** Localized working-axis label, e.g. `doit` -> "执行" / "Act". */
export function userModeWorkingLabel(working: string): string {
  const meta = Object.values(USER_MODE_META).find((m) => m.working === working)
  return translated(`mode.working.${working}`, meta?.workingLabel ?? working)
}
