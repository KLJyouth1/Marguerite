<template>
  <div class="landing-hero">
    <h1 class="landing-hero__title">{{ t('chat.landingHero.title') }}</h1>
    <p class="landing-hero__subtitle">{{ t('chat.landingHero.subtitle') }}</p>
    <div class="landing-hero__context">
      <span
        class="landing-hero__chip"
        :title="projectPath || undefined"
        data-testid="landing-project"
      >
        <Icon name="folder" :size="12" aria-hidden="true" />
        <span class="landing-hero__chip-text">{{ projectLabel }}</span>
      </span>
      <span v-if="branchLabel" class="landing-hero__chip" data-testid="landing-branch">
        <Icon name="gitBranch" :size="12" aria-hidden="true" />
        <span class="landing-hero__chip-text">{{ branchLabel }}</span>
      </span>
      <!-- 模型选择器由 ChatView 注入：本组件保持纯展示，测试与文案改动不牵动 RPC。 -->
      <slot name="model" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'

const props = withDefaults(defineProps<{
  /** 已绑定项目的展示名；为空时显示「未绑定项目」。 */
  projectName?: string
  /** 完整路径，仅作为 chip 的 title 提示，不直接展示。 */
  projectPath?: string
  branch?: string | null
  detached?: boolean
}>(), {
  projectName: '',
  projectPath: '',
  branch: null,
  detached: false,
})

const { t } = useI18n()

const projectLabel = computed(() => props.projectName.trim() || t('chat.landingHero.unbound'))
const branchLabel = computed(() => {
  if (props.detached) return t('chat.landingHero.detached')
  return (props.branch || '').trim()
})
</script>

<style scoped>
.landing-hero {
  width: var(--landing-col, min(calc(100% - 48px), 45rem));
  margin: 0 auto var(--sp-2);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-2);
  text-align: center;
  pointer-events: auto;
}

.landing-hero__title {
  margin: var(--sp-2) 0 0;
  font-family: var(--font-display);
  font-size: clamp(1.75rem, 1rem + 1.8vw, 2.25rem);
  font-weight: 600;
  letter-spacing: var(--track-display);
  color: var(--text);
  animation: landing-hero-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
}

.landing-hero__subtitle {
  margin: 0;
  max-width: 38ch;
  font-size: var(--fs-sm);
  line-height: 1.5;
  color: var(--text-dim);
  animation: landing-hero-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  animation-delay: 30ms;
}

.landing-hero__context {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 6px;
  margin-top: var(--sp-2);
  animation: landing-hero-reveal calc(var(--dur-base) * 2.5) var(--ease-out) both;
  animation-delay: 60ms;
}

.landing-hero__chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: min(18rem, 60vw);
  min-height: 1.625rem;
  padding: 0.0625rem 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated, var(--bg-surface));
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
}

.landing-hero__chip-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@keyframes landing-hero-reveal {
  from {
    filter: blur(5px);
    opacity: 0;
    transform: translateY(7px);
  }
  to {
    filter: blur(0);
    opacity: 1;
    transform: translateY(0);
  }
}

@media (prefers-reduced-motion: reduce) {
  .landing-hero__title,
  .landing-hero__subtitle,
  .landing-hero__context {
    animation: none;
  }
}
</style>
