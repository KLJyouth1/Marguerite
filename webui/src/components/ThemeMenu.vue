<template>
  <div class="theme-menu-wrap">
    <button
      ref="themeButtonRef"
      class="btn btn--icon btn--ghost"
      :title="t('chrome.theme')"
      :aria-label="t('chrome.theme')"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click.stop="open = !open"
    >
      <Icon :name="themeIconName" :size="16" />
    </button>
    <div v-if="open" class="theme-menu" role="menu" :aria-label="t('chrome.theme')">
      <button
        v-for="opt in themeOptions"
        :key="opt.mode"
        type="button"
        class="theme-menu__item"
        role="menuitemradio"
        :aria-checked="theme === opt.mode"
        @click="pickTheme(opt.mode)"
      >
        <Icon :name="opt.icon" :size="15" />
        <span>{{ opt.labelKey ? t(opt.labelKey) : opt.label }}</span>
        <Icon v-if="theme === opt.mode" class="theme-menu__check" name="check" :size="14" />
      </button>
      <button
        type="button"
        class="theme-menu__item theme-menu__item--more"
        role="menuitem"
        :title="t('chrome.moreThemesHint')"
        @click="openMoreThemes"
      >
        <Icon name="chevronRight" :size="15" />
        <span>{{ t('chrome.moreThemes') }}</span>
        <Icon v-if="isCustomThemeActive" class="theme-menu__check" name="check" :size="14" />
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from './Icon.vue'
import { useDocumentEvent } from '@/composables/useDocumentEvent'
import { useDialogLayer } from '@/composables/useDialogA11y'
import { themePickerOptions, getManifest } from '@/themes/registry'
import type { ThemeMode } from '@/stores/app'

// App owns the open state so its Escape handler and workbench-blocks modal
// sync read it; this component owns the menu rendering + click-outside close.
const open = defineModel<boolean>('open', { default: false })

const props = defineProps<{
  theme: ThemeMode
  resolvedTheme: string
}>()

const emit = defineEmits<{
  pickTheme: [mode: ThemeMode]
  openMoreThemes: []
}>()

const { t } = useI18n()

const themeIconName = computed(() => {
  if (props.theme === 'system') return 'monitor'
  const active = getManifest(props.resolvedTheme)
  return active?.icon ?? (props.resolvedTheme === 'dark' ? 'moon' : 'sun')
})

useDialogLayer(open)
const themeButtonRef = ref<HTMLButtonElement | null>(null)

// The compact topbar menu deliberately lists only the basic modes (Light / Dark
// / System). Custom value themes live in Settings → Appearance, reached via the
// "More themes…" action below — see themePickerOptions({ scope }) in registry.ts.
const themeOptions = themePickerOptions({ scope: 'basic' })

// A custom value theme (chosen in Settings) is active but not shown in the basic
// topbar menu; mark "More themes…" instead of leaving no selection indicator.
const isCustomThemeActive = computed(
  () => !themeOptions.some((o) => o.mode === props.theme),
)

function pickTheme(mode: ThemeMode) {
  open.value = false
  focusButton()
  emit('pickTheme', mode)
}

// "More themes…": the full theme list lives in Settings → Appearance. Close the
// menu and deep-link straight to that section (App owns handleNavClick/router).
function openMoreThemes() {
  open.value = false
  emit('openMoreThemes')
}

// Exposed so App's global Escape handler can restore focus to the trigger after
// closing the menu by keyboard, exactly as before extraction.
function focusButton() {
  themeButtonRef.value?.focus()
}
defineExpose({ focusButton })

useDocumentEvent('click', (e) => {
  if (!open.value) return
  const wrap = themeButtonRef.value?.closest('.theme-menu-wrap')
  if (wrap && e.target instanceof Node && !wrap.contains(e.target)) {
    open.value = false
  }
})
</script>
