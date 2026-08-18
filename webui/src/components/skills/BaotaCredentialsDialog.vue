<template>
  <Transition name="modal">
    <div
      v-if="open"
      class="bcd-modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="bcd-title"
      @click.self="emit('close')"
    >
      <section class="bcd-panel">
        <header class="bcd-head">
          <div>
            <span class="bcd-eyebrow">BAOTA PANEL</span>
            <h2 id="bcd-title">{{ t('baotaCredentials.title') }}</h2>
            <p>{{ t('baotaCredentials.desc') }}</p>
          </div>
          <button
            class="btn btn--ghost bcd-close"
            type="button"
            :aria-label="t('common.close')"
            @click="emit('close')"
          >
            <Icon name="x" :size="18" />
          </button>
        </header>

        <div class="bcd-status" :class="statusClass">
          <Icon :name="statusIcon" :size="14" />
          <span>{{ statusText }}</span>
        </div>

        <form class="bcd-form" @submit.prevent="save">
          <label class="bcd-field">
            <span class="bcd-field__label">{{ t('baotaCredentials.baseUrl') }}</span>
            <input
              v-model="form.baseUrl"
              class="bcd-input"
              type="url"
              required
              :placeholder="'http://127.0.0.1:8888/安全路径'"
              autocomplete="url"
            />
          </label>
          <label class="bcd-field">
            <span class="bcd-field__label">{{ t('baotaCredentials.username') }}</span>
            <input
              v-model="form.username"
              class="bcd-input"
              type="text"
              required
              autocomplete="username"
            />
          </label>
          <label class="bcd-field">
            <span class="bcd-field__label">{{ t('baotaCredentials.password') }}</span>
            <input
              v-model="form.password"
              class="bcd-input"
              type="password"
              required
              autocomplete="current-password"
            />
          </label>
          <div class="bcd-actions">
            <button
              v-if="status.configured"
              class="btn btn--ghost bcd-clear"
              type="button"
              :disabled="busy"
              @click="clear"
            >
              {{ t('baotaCredentials.clear') }}
            </button>
            <button class="btn btn--primary" type="submit" :disabled="busy || !formValid">
              <Icon name="check" :size="14" />
              <span>{{ busy ? t('baotaCredentials.saving') : t('baotaCredentials.save') }}</span>
            </button>
          </div>
        </form>
      </section>
    </div>
  </Transition>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useToasts } from '@/composables/useToasts'
import { useRpcStore } from '@/stores/rpc'

interface CredentialsStatus {
  configured?: boolean
  base_url?: string
  has_username?: boolean
}

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: []; saved: [] }>()

const { t } = useI18n()
const rpc = useRpcStore()
const { pushToast } = useToasts()

const busy = ref(false)
const status = ref<CredentialsStatus>({ configured: false })
const form = reactive({ baseUrl: '', username: '', password: '' })

const formValid = computed(() =>
  form.baseUrl.trim().startsWith('http')
  && form.username.trim().length > 0
  && form.password.length > 0,
)

const statusIcon = computed(() => (status.value.configured ? 'check' : 'info'))
const statusClass = computed(() => (status.value.configured ? 'is-ok' : 'is-empty'))
const statusText = computed(() => {
  if (busy.value) return t('baotaCredentials.checking')
  if (status.value.configured) {
    const url = status.value.base_url || ''
    return t('baotaCredentials.configured', { url })
  }
  return t('baotaCredentials.notConfigured')
})

async function loadStatus() {
  try {
    const res = await rpc.call<CredentialsStatus>('baota.credentials.get')
    status.value = res || { configured: false }
    if (status.value.base_url) form.baseUrl = status.value.base_url
  } catch {
    status.value = { configured: false }
  }
}

async function save() {
  if (!formValid.value || busy.value) return
  busy.value = true
  try {
    const res = await rpc.call<{ ok?: boolean; msg?: string }>('baota.credentials.set', {
      base_url: form.baseUrl.trim(),
      username: form.username.trim(),
      password: form.password,
    })
    if (res?.ok) {
      pushToast(t('baotaCredentials.saved'), { tone: 'ok' })
      status.value = { configured: true, base_url: form.baseUrl.trim(), has_username: true }
      emit('saved')
    } else {
      pushToast(res?.msg || t('baotaCredentials.failed'), { tone: 'danger' })
    }
  } catch {
    pushToast(t('baotaCredentials.failed'), { tone: 'danger' })
  } finally {
    busy.value = false
  }
}

async function clear() {
  if (busy.value) return
  busy.value = true
  try {
    const res = await rpc.call<{ ok?: boolean; msg?: string }>('baota.credentials.clear')
    pushToast(res?.msg || t('baotaCredentials.cleared'), { tone: res?.ok ? 'ok' : 'warn' })
    status.value = { configured: false }
    form.password = ''
  } finally {
    busy.value = false
  }
}

watch(() => props.open, open => {
  if (open) {
    form.password = ''
    void loadStatus()
  }
})
</script>

<style scoped>
.bcd-modal {
  align-items: center;
  background: var(--scrim);
  display: flex;
  inset: 0;
  justify-content: center;
  padding: 24px;
  position: fixed;
  z-index: 1100;
}

.bcd-panel {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--elev-3);
  max-width: 440px;
  padding: 22px;
  width: 100%;
}

.bcd-head {
  align-items: flex-start;
  display: flex;
  gap: 16px;
  justify-content: space-between;
  margin-bottom: 14px;
}

.bcd-eyebrow {
  color: var(--accent);
  font-size: 10px;
  font-weight: 750;
  letter-spacing: .13em;
}

.bcd-head h2 {
  font-size: 1.125rem;
  margin: 4px 0 0;
}

.bcd-head p {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  margin: 5px 0 0;
}

.bcd-close {
  padding: 6px;
}

.bcd-status {
  align-items: center;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text-dim);
  display: flex;
  font-size: 12px;
  gap: 7px;
  margin-bottom: 14px;
  padding: 8px 10px;
}

.bcd-status.is-ok {
  border-color: color-mix(in srgb, var(--ok) 45%, var(--border));
  color: var(--ok);
}

.bcd-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.bcd-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.bcd-field__label {
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 650;
  letter-spacing: .04em;
}

.bcd-input {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  outline: none;
  padding: 8px 10px;
  transition: border-color var(--transition), box-shadow var(--transition);
}

.bcd-input:focus {
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
}

.bcd-actions {
  align-items: center;
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  margin-top: 4px;
}

@media (max-width: 700px) {
  .bcd-modal {
    align-items: flex-end;
    padding: 0;
  }

  .bcd-panel {
    border-bottom-left-radius: 0;
    border-bottom-right-radius: 0;
    max-height: 88vh;
    overflow: auto;
    padding: 18px;
  }
}
</style>
