<script setup lang="ts">
import { ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const METHOD = 'agent.profile.analyze'

interface Persona {
  role: string
  tone: string
  expertise: string[]
}

const { t } = useI18n()
const rpc = useRpcStore()

const raw = ref('')
const analyzing = ref(false)
const unsupported = ref(false)
const error = ref(false)
const persona = shallowRef<Persona | null>(null)

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}
function textValue(value: unknown): string {
  return typeof value === 'string' ? value : ''
}
function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
}

// 把粘贴的对话历史按换行拆成 user/assistant 交替消息，交后端确定性抽取画像。
function messagesFrom(rawText: string): Array<{ role: 'user' | 'assistant'; content: string }> {
  const lines = rawText.split('\n').map(line => line.trim()).filter(Boolean)
  if (lines.length === 0) return []
  return lines.map((content, index) => ({
    role: index % 2 === 0 ? 'user' : 'assistant',
    content,
  }))
}

async function analyze() {
  if (analyzing.value || !raw.value.trim()) return
  if (!rpc.supportsMethod(METHOD)) {
    unsupported.value = true
    return
  }
  analyzing.value = true
  error.value = false
  persona.value = null
  try {
    const data = objectValue(await rpc.call(METHOD, { messages: messagesFrom(raw.value) }))
    persona.value = {
      role: textValue(data.role),
      tone: textValue(data.tone),
      expertise: stringList(data.expertise),
    }
  } catch {
    error.value = true
  } finally {
    analyzing.value = false
  }
}
</script>

<template>
  <section class="profile-card" data-testid="profile-card">
    <header class="profile-card__head">
      <h3 class="profile-card__title">{{ t('settings.profile.title') }}</h3>
      <p class="profile-card__description">{{ t('settings.profile.description') }}</p>
    </header>

    <label class="profile-card__label" for="profile-raw">{{ t('settings.profile.textareaLabel') }}</label>
    <textarea
      id="profile-raw"
      v-model="raw"
      class="profile-card__textarea"
      :placeholder="t('settings.profile.textareaPlaceholder')"
    ></textarea>

    <div class="profile-card__actions">
      <button
        type="button"
        class="btn btn--primary"
        :disabled="analyzing || !raw.trim()"
        @click="analyze"
      >
        <span v-if="analyzing" class="profile-card__spinner" aria-hidden="true"></span>
        <Icon v-else name="search" :size="15" aria-hidden="true" />
        {{ analyzing ? t('settings.profile.analyzing') : t('settings.profile.analyze') }}
      </button>
    </div>

    <p v-if="unsupported" class="profile-card__notice" role="status">{{ t('settings.profile.unsupported') }}</p>
    <p v-else-if="error" class="profile-card__notice profile-card__notice--error" role="alert">
      {{ t('settings.profile.loadFailed') }}
    </p>

    <article v-else-if="persona" class="profile-card__result">
      <div class="profile-card__row">
        <span class="profile-card__key">{{ t('settings.profile.role') }}</span>
        <span class="profile-card__value">{{ persona.role || t('settings.profile.neutral') }}</span>
      </div>
      <div class="profile-card__row">
        <span class="profile-card__key">{{ t('settings.profile.tone') }}</span>
        <span class="profile-card__value">{{ persona.tone }}</span>
      </div>
      <div v-if="persona.expertise.length" class="profile-card__row profile-card__row--stack">
        <span class="profile-card__key">{{ t('settings.profile.expertise') }}</span>
        <span class="profile-card__tags">
          <span v-for="item in persona.expertise" :key="item" class="profile-card__tag">{{ item }}</span>
        </span>
      </div>
    </article>
  </section>
</template>

<style scoped>
.profile-card {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  margin: 0 auto;
  max-width: 880px;
  min-height: 100%;
}
.profile-card__head {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.profile-card__title {
  color: var(--text);
  font-size: var(--fs-lg);
  margin: 0;
}
.profile-card__description,
.profile-card__notice {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.55;
  margin: 0;
}
.profile-card__notice--error {
  color: var(--danger);
}
.profile-card__label {
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
  margin-bottom: calc(-1 * var(--sp-2));
}
.profile-card__textarea {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  font: inherit;
  font-size: var(--fs-sm);
  line-height: 1.55;
  min-height: 140px;
  padding: var(--sp-3) var(--sp-4);
  resize: vertical;
  width: 100%;
}
.profile-card__textarea:focus-visible {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--focus-ring);
  outline: none;
}
.profile-card__actions {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
  justify-content: flex-end;
}
.profile-card__spinner {
  animation: profile-spin 0.8s linear infinite;
  border: 2px solid currentColor;
  border-radius: 50%;
  border-top-color: transparent;
  display: inline-block;
  height: 14px;
  width: 14px;
}
@keyframes profile-spin {
  to { transform: rotate(360deg); }
}
.profile-card__result {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4);
}
.profile-card__row {
  align-items: baseline;
  display: flex;
  gap: var(--sp-3);
}
.profile-card__row--stack {
  align-items: flex-start;
  flex-direction: column;
  gap: var(--sp-2);
}
.profile-card__key {
  color: var(--text-dim);
  flex: 0 0 auto;
  font-size: var(--fs-xs);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.profile-card__value {
  color: var(--text);
  font-size: var(--fs-sm);
}
.profile-card__tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
}
.profile-card__tag {
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent) 22%, var(--border));
  border-radius: var(--radius-pill);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  padding: 2px var(--sp-2);
}
@media (prefers-reduced-motion: reduce) {
  .profile-card__spinner {
    animation-duration: 1.6s; /* motion-allow: spinner cadence under reduced motion */
  }
}
</style>
