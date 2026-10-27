<template>
  <ContentSheet :open="open" :label="t('chat.fullMessage.title')" @close="emit('close')">
    <template #header>
      <div class="full-message__titles">
        <p class="full-message__eyebrow">{{ title || t('chat.fullMessage.title') }}</p>
        <p class="full-message__meta">{{ metaText }}</p>
      </div>
    </template>
    <template #actions>
      <button
        type="button"
        class="btn btn--icon btn--ghost"
        :title="copyLabel"
        :aria-label="copyLabel"
        @click="copy"
      >
        <Icon :name="copied ? 'check' : 'copy'" :size="16" />
      </button>
    </template>
    <!-- eslint-disable-next-line vue/no-v-html -- renderMarkdown output is sanitized (marked + DOMPurify). -->
    <div class="full-message__scroll" v-html="html" />
  </ContentSheet>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ContentSheet from '@/components/chat/ContentSheet.vue'
import { useChatTextRendering } from '@/composables/chat/useChatTextRendering'
import { copyTextWithFallback } from '@/utils/browser'

const props = withDefaults(defineProps<{
  open: boolean
  content: string
  title?: string
  meta?: string
}>(), {
  title: '',
  meta: '',
})

const emit = defineEmits<{
  close: []
}>()

const { t } = useI18n()
const { renderMarkdown } = useChatTextRendering()

const html = computed(() => renderMarkdown(props.content || ''))
const metaText = computed(() => (
  props.meta || t('chat.fullMessage.meta', { n: (props.content || '').length.toLocaleString() })
))
const copied = ref(false)
let copiedResetId: ReturnType<typeof setTimeout> | null = null
const copyLabel = computed(() => (
  copied.value ? t('chat.fullMessage.copied') : t('chat.fullMessage.copy')
))

watch(() => props.open, open => {
  if (open) copied.value = false
})

onBeforeUnmount(() => {
  if (copiedResetId) clearTimeout(copiedResetId)
})

async function copy() {
  try {
    await copyTextWithFallback(props.content || '')
    copied.value = true
    if (copiedResetId) clearTimeout(copiedResetId)
    copiedResetId = setTimeout(() => {
      copied.value = false
      copiedResetId = null
    }, 1600)
  } catch {
    copied.value = false
  }
}
</script>

<style scoped>
.full-message__titles {
  flex: 1 1 12rem;
  min-width: 0;
}

.full-message__eyebrow {
  color: var(--text);
  font-size: var(--fs-md);
  font-weight: 600;
  margin: 0 0 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.full-message__meta {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  margin: 0;
}

.full-message__scroll {
  background: var(--bg-surface);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-md);
  color: var(--text);
  flex: 1;
  font-size: var(--fs-md);
  line-height: 1.75;
  min-height: 0;
  overflow: auto;
  overflow-wrap: anywhere;
  padding: var(--sp-4) var(--sp-5);
}

.full-message__scroll :deep(pre) {
  overflow-x: auto;
}

@media (max-width: 768px) {
  .full-message__scroll {
    padding: var(--sp-3);
  }
}
</style>
