<!-- 输入框「+」菜单：附件 + 命令 + 产物 / 工作区文件 + 技能 / 插件。
     用户反馈旧菜单只有「上传附件」，而参考形态里应有「选择命令」等入口。
     这里每一项都接到**真实能力**（斜杠命令清单、产物面板、工作区文件树、技能页、插件页），
     不做点了没反应的占位项。 -->
<template>
  <section
    ref="rootRef"
    tabindex="-1"
    class="composer-add-menu"
    role="menu"
    :aria-label="t('chat.add')"
    @keydown.esc.stop="$emit('close')"
  >
    <div class="composer-add-menu__heading">{{ t('chat.add') }}</div>
    <button
      type="button"
      class="composer-add-menu__item"
      role="menuitem"
      :disabled="attachmentsDisabled"
      @click="run('attachFiles')"
    >
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="paperclip" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.attachFiles') }}</strong>
        <span>{{ t('chat.addMenu.attachHint') }}</span>
      </span>
    </button>

    <div class="composer-add-menu__divider" role="separator" />

    <button type="button" class="composer-add-menu__item" role="menuitem" @click="run('openCommands')">
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="command" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.addMenu.commands') }}</strong>
        <span>{{ t('chat.addMenu.commandsHint') }}</span>
      </span>
      <kbd class="composer-add-menu__kbd">/</kbd>
    </button>

    <button type="button" class="composer-add-menu__item" role="menuitem" @click="run('openArtifacts')">
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="fileText" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.addMenu.artifacts') }}</strong>
        <span>{{ t('chat.addMenu.artifactsHint') }}</span>
      </span>
    </button>

    <button type="button" class="composer-add-menu__item" role="menuitem" @click="run('openFileTree')">
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="listTree" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.addMenu.fileTree') }}</strong>
        <span>{{ t('chat.addMenu.fileTreeHint') }}</span>
      </span>
    </button>

    <button type="button" class="composer-add-menu__item" role="menuitem" @click="run('openSkills')">
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="skills" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.addMenu.skills') }}</strong>
        <span>{{ t('chat.addMenu.skillsHint') }}</span>
      </span>
    </button>

    <button type="button" class="composer-add-menu__item" role="menuitem" @click="run('openPlugins')">
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="plugin" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.addMenu.plugins') }}</strong>
        <span>{{ t('chat.addMenu.pluginsHint') }}</span>
      </span>
    </button>

    <button
      v-if="planModeAvailable"
      type="button"
      class="composer-add-menu__item"
      role="menuitem"
      :disabled="planModeBusy || planModeActive"
      :aria-pressed="planModeActive"
      @click="activatePlanMode"
    >
      <span class="composer-add-menu__icon" aria-hidden="true">
        <Icon name="listChecks" :size="17" />
      </span>
      <span class="composer-add-menu__copy">
        <strong>{{ t('chat.planMode.label') }}</strong>
        <span>
          {{ planModeActive
            ? t('chat.planMode.readOnly')
            : t('chat.planMode.turnOn') }}
        </span>
      </span>
    </button>
  </section>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'

defineProps<{
  attachmentsDisabled?: boolean
  planModeActive: boolean
  planModeAvailable: boolean
  planModeBusy: boolean
}>()

type MenuAction =
  | 'attachFiles'
  | 'openCommands'
  | 'openArtifacts'
  | 'openFileTree'
  | 'openSkills'
  | 'openPlugins'
  | 'activatePlanMode'

const emit = defineEmits<{
  attachFiles: []
  openCommands: []
  openArtifacts: []
  openFileTree: []
  openSkills: []
  openPlugins: []
  activatePlanMode: []
  close: []
}>()

const { t } = useI18n()
const rootRef = ref<HTMLElement | null>(null)

function run(action: Exclude<MenuAction, 'activatePlanMode'>) {
  switch (action) {
    case 'attachFiles': emit('attachFiles'); break
    case 'openCommands': emit('openCommands'); break
    case 'openArtifacts': emit('openArtifacts'); break
    case 'openFileTree': emit('openFileTree'); break
    case 'openSkills': emit('openSkills'); break
    case 'openPlugins': emit('openPlugins'); break
  }
  emit('close')
}

function activatePlanMode() {
  emit('activatePlanMode')
  emit('close')
}

onMounted(() => rootRef.value?.focus())
</script>

<style scoped>
.composer-add-menu {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 0;
  z-index: 30;
  display: grid;
  width: min(340px, calc(100vw - 48px));
  padding: var(--sp-2);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-xl);
}

.composer-add-menu__heading {
  padding: var(--sp-1) var(--sp-2) var(--sp-2);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 650;
}

.composer-add-menu__divider {
  height: 1px;
  margin: 4px var(--sp-2);
  background: var(--border);
}

.composer-add-menu__item {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  min-height: 44px;
  padding: var(--sp-2);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.composer-add-menu__item:hover:not(:disabled),
.composer-add-menu__item:focus-visible {
  outline: 0;
  background: var(--bg-hover);
}

.composer-add-menu__item:disabled {
  cursor: default;
  opacity: var(--state-disabled-opacity);
}

.composer-add-menu__icon {
  display: inline-flex;
  color: var(--text-muted);
}

.composer-add-menu__copy {
  display: grid;
  min-width: 0;
}

.composer-add-menu__copy strong {
  font-size: var(--fs-sm);
  font-weight: 600;
}

.composer-add-menu__copy span {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.composer-add-menu__kbd {
  padding: 1px 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}

@media (max-width: 520px) {
  .composer-add-menu {
    left: -0.5rem;
    width: calc(100vw - 32px);
  }
}
</style>
