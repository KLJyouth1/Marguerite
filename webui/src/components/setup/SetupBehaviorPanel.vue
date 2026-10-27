<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import ControlSwitch from '@/components/ControlSwitch.vue'
import { useTurnNotifyPreference } from '@/composables/useTurnNotification'

const { t } = useI18n()

interface BehaviorPanelContract {
  autoSessionTitles: boolean
  autoSessionTitlesDirty: boolean
  statusText: string
}

defineProps<{
  panel: BehaviorPanelContract
}>()

const emit = defineEmits<{
  updateAutoSessionTitles: [enabled: boolean]
}>()

// 回合结束通知是**纯客户端偏好**（localStorage），不进 config 面板的 dirty/save 流程；
// 这里直接绑定 composable，开关即时生效（此前只有 composable、没有任何 UI 能改它）。
const { enabled: turnNotifyEnabled } = useTurnNotifyPreference()

function setTurnNotify(enabled: boolean) {
  turnNotifyEnabled.value = enabled
}
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('setup.behavior.title') }}</h3>
      <p class="control-section__desc">{{ panel.statusText }}</p>
    </div>
    <label class="control-row">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.behavior.autoTitlesLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.behavior.autoTitlesDesc') }}</span>
      </div>
      <div class="control-row__control">
        <ControlSwitch
          :checked="panel.autoSessionTitles"
          name="setup_auto_session_titles"
          :aria-label="t('setup.behavior.autoTitlesLabel')"
          @change="(value) => emit('updateAutoSessionTitles', value)"
        />
      </div>
    </label>
    <label class="control-row">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('setup.behavior.turnNotifyLabel') }}</span>
        <span class="control-row__desc">{{ t('setup.behavior.turnNotifyDesc') }}</span>
      </div>
      <div class="control-row__control">
        <ControlSwitch
          :checked="turnNotifyEnabled"
          name="turn_end_notify"
          :aria-label="t('setup.behavior.turnNotifyLabel')"
          @change="setTurnNotify"
        />
      </div>
    </label>
  </section>
</template>
