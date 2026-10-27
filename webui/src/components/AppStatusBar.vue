<template>
  <footer
    class="app-status-bar"
    :class="`app-status-bar--${connectionTone}`"
    :aria-label="t('appStatus.label')"
    data-testid="app-status-bar"
  >
    <span class="app-status-bar__item app-status-bar__project" :title="projectPath || project">
      <Icon name="folder" :size="12" aria-hidden="true" />
      <span class="app-status-bar__label">{{ project }}</span>
    </span>
    <span class="app-status-bar__sep" aria-hidden="true" />
    <span class="app-status-bar__item">
      <Icon name="monitor" :size="12" aria-hidden="true" />{{ mode }}
    </span>
    <span v-if="branch" class="app-status-bar__item app-status-bar__branch" :title="branch">
      <Icon name="gitBranch" :size="12" aria-hidden="true" />{{ branch }}
    </span>
    <span v-else-if="detached" class="app-status-bar__item">
      <Icon name="gitBranch" :size="12" aria-hidden="true" />{{ t('appStatus.detached') }}
    </span>
    <span class="app-status-bar__spacer" />
    <span class="app-status-bar__item app-status-bar__connection">
      <i class="app-status-bar__dot" aria-hidden="true" />{{ connectionLabel }}
    </span>
    <span class="app-status-bar__version">v{{ version }}</span>
  </footer>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'

withDefaults(defineProps<{
  project: string
  projectPath?: string
  mode: string
  branch?: string | null
  detached?: boolean
  connectionLabel: string
  connectionTone: 'ready' | 'connecting' | 'not_ready' | 'disconnected'
}>(), {
  projectPath: '',
  branch: null,
  detached: false,
})

const { t } = useI18n()
const version = __APP_VERSION__
</script>

<style scoped>
.app-status-bar {
  align-items: center;
  background: color-mix(in srgb, var(--bg-surface) 82%, transparent);
  border-top: 1px solid var(--border);
  color: var(--text-muted);
  display: flex;
  flex-shrink: 0;
  font-size: var(--fs-xs);
  gap: var(--sp-2);
  height: 24px;
  /* 桌面宠默认停在右下角且 z-index 高于外壳；给右侧留出它的身位，
     避免「连接状态 / 版本」被遮住。 */
  padding: 0 calc(var(--sp-3) + 72px) 0 var(--sp-3);
  user-select: none;
}

.app-status-bar__item {
  align-items: center;
  display: inline-flex;
  gap: 4px;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.app-status-bar__project {
  max-width: 240px;
}

.app-status-bar__label {
  overflow: hidden;
  text-overflow: ellipsis;
}

.app-status-bar__branch {
  color: var(--text-muted);
  max-width: 200px;
}

.app-status-bar__sep {
  background: var(--border);
  height: 12px;
  width: 1px;
}

.app-status-bar__spacer {
  flex: 1;
}

.app-status-bar__dot {
  background: var(--text-dim);
  border-radius: var(--radius-pill);
  height: 6px;
  width: 6px;
}

.app-status-bar--ready .app-status-bar__dot {
  background: var(--ok);
}

.app-status-bar--connecting .app-status-bar__dot,
.app-status-bar--not_ready .app-status-bar__dot {
  background: var(--warn);
}

.app-status-bar--disconnected .app-status-bar__dot {
  background: var(--danger);
}

.app-status-bar__version {
  color: var(--text-dim);
  font-family: var(--font-mono, monospace);
}

/* Small windows: the shell already gives up to the mobile tab bar; a 24px
   footer would push the composer below the fold. */
@media (max-width: 900px), (max-height: 600px) {
  .app-status-bar {
    display: none;
  }
}
</style>
