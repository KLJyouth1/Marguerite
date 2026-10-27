<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'
import { useConfirm } from '@/composables/useConfirm'

/** M2-C5 信任校准设置面板：查看 / 授予 / 撤销（可撤销的信任授权）。 */

interface TrustEntry {
  id: string
  level: string
  level_label: string
  tool: string
  mode: string | null
  session: string
  granted_at_ms: number
}

const rpc = useRpcStore()
const { confirm } = useConfirm()
const { t } = useI18n()

const entries = ref<TrustEntry[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)

// 授予表单
const grantLevel = ref<'low' | 'medium' | 'high'>('low')
const grantTool = ref('*')
const grantMode = ref('')

const levelHints = computed<Record<string, string>>(() => ({
  low: t('settings.trust.hintLow'),
  medium: t('settings.trust.hintMedium'),
  high: t('settings.trust.hintHigh'),
}))

async function refresh() {
  loading.value = true
  error.value = null
  try {
    await rpc.waitForConnection()
    const data = await rpc.call<{ entries: TrustEntry[] }>('trust.list')
    entries.value = data.entries ?? []
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function grant() {
  if (!grantTool.value.trim()) return
  error.value = null
  message.value = null
  try {
    await rpc.call('trust.grant', {
      level: grantLevel.value,
      tool: grantTool.value.trim(),
      mode: grantMode.value.trim() || undefined,
    })
    grantTool.value = '*'
    grantMode.value = ''
    message.value = t('settings.trust.granted')
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function revoke(entry: TrustEntry) {
  const ok = await confirm({
    title: t('settings.trust.confirmRevokeTitle'),
    body: t('settings.trust.confirmRevokeBody', { tool: entry.tool, level: entry.level_label }),
    primaryLabel: t('settings.trust.confirmRevokeAction'),
    primaryClass: 'btn--danger',
  })
  if (!ok) return
  try {
    await rpc.call('trust.revoke', { id: entry.id })
    message.value = t('settings.trust.revoked', { tool: entry.tool })
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function revokeAll() {
  const ok = await confirm({
    title: t('settings.trust.confirmRevokeAllTitle'),
    body: t('settings.trust.confirmRevokeAllBody'),
    primaryLabel: t('settings.trust.confirmRevokeAllAction'),
    primaryClass: 'btn--danger',
  })
  if (!ok) return
  try {
    const data = await rpc.call<{ removed: number }>('trust.revoke_all')
    message.value = t('settings.trust.revokedAll', { count: data.removed })
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

function fmtTime(ms: number): string {
  if (!ms) return '—'
  return new Date(ms).toISOString().slice(0, 19).replace('T', ' ')
}

onMounted(refresh)
</script>

<template>
  <section class="control-section">
    <div class="control-section__head">
      <h3 class="control-section__title">{{ t('settings.trust.title') }}</h3>
      <p class="control-section__desc">
        {{ t('settings.trust.description') }}
      </p>
    </div>

    <div class="control-row control-row--stack">
      <div class="control-row__label-block">
        <span class="control-row__label">{{ t('settings.trust.grantTitle') }}</span>
        <span class="control-row__desc">{{ t('settings.trust.grantDesc') }}</span>
      </div>
      <div class="control-row__control trust-grant">
        <select v-model="grantLevel" class="control-input control-input--narrow" :aria-label="t('settings.trust.levelAria')">
          <option value="low">{{ t('settings.trust.optLow') }}</option>
          <option value="medium">{{ t('settings.trust.optMedium') }}</option>
          <option value="high">{{ t('settings.trust.optHigh') }}</option>
        </select>
        <input v-model="grantTool" class="control-input" :placeholder="t('settings.trust.toolPlaceholder')" :aria-label="t('settings.trust.toolAria')" />
        <input v-model="grantMode" class="control-input" :placeholder="t('settings.trust.modePlaceholder')" :aria-label="t('settings.trust.modeAria')" />
        <button type="button" class="btn" :disabled="loading || !grantTool.trim()" @click="grant">{{ t('settings.trust.grantAction') }}</button>
      </div>
      <p v-if="grantLevel" class="trust-hint">{{ levelHints[grantLevel] }}</p>
    </div>

    <div v-if="error" class="trust-banner trust-banner--err" role="alert">{{ error }}</div>
    <div v-else-if="message" class="trust-banner trust-banner--ok" role="status">{{ message }}</div>

    <div class="trust-toolbar">
      <span class="trust-toolbar__count">{{ t('settings.trust.countLabel', { count: entries.length }) }}</span>
      <button type="button" class="btn btn--ghost" :disabled="loading || entries.length === 0" @click="revokeAll">{{ t('settings.trust.revokeAll') }}</button>
      <button type="button" class="btn btn--ghost" :disabled="loading" @click="refresh">{{ t('settings.trust.refresh') }}</button>
    </div>

    <div v-if="loading" class="trust-empty">{{ t('settings.trust.loading') }}</div>
    <div v-else-if="entries.length === 0" class="trust-empty">{{ t('settings.trust.empty') }}</div>
    <table v-else class="trust-table">
      <thead>
        <tr>
          <th>{{ t('settings.trust.colLevel') }}</th>
          <th>{{ t('settings.trust.colTool') }}</th>
          <th>{{ t('settings.trust.colMode') }}</th>
          <th>{{ t('settings.trust.colSession') }}</th>
          <th>{{ t('settings.trust.colGranted') }}</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="e in entries" :key="e.id">
          <td><span class="trust-level" :class="'trust-level--' + e.level">{{ e.level_label }}</span></td>
          <td><code>{{ e.tool }}</code></td>
          <td>{{ e.mode ?? t('settings.trust.all') }}</td>
          <td>{{ e.session }}</td>
          <td>{{ fmtTime(e.granted_at_ms) }}</td>
          <td><button type="button" class="btn btn--ghost btn--sm" @click="revoke(e)">{{ t('settings.trust.revoke') }}</button></td>
        </tr>
      </tbody>
    </table>
  </section>
</template>

<style scoped>
.trust-grant {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}
.trust-hint {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  margin: 0;
}
.trust-banner {
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  margin: var(--sp-2) 0;
  padding: var(--sp-2) var(--sp-3);
}
.trust-banner--err { background: color-mix(in srgb, var(--danger) 15%, var(--bg-surface)); color: var(--danger); }
.trust-banner--ok { background: color-mix(in srgb, var(--ok) 15%, var(--bg-surface)); color: var(--ok); }
.trust-toolbar {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
  justify-content: flex-end;
  margin: var(--sp-2) 0;
}
.trust-toolbar__count {
  color: var(--text-dim);
  flex: 1;
  font-size: var(--fs-sm);
}
.trust-empty {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  padding: var(--sp-4) 0;
}
.trust-table {
  border-collapse: collapse;
  font-size: var(--fs-sm);
  width: 100%;
}
.trust-table th,
.trust-table td {
  border-bottom: 1px solid var(--hairline, var(--border));
  padding: var(--sp-2);
  text-align: left;
}
.trust-level {
  border-radius: var(--radius-sm);
  font-size: var(--fs-xs);
  font-weight: 600;
  padding: 2px var(--sp-2);
}
.trust-level--high { background: color-mix(in srgb, var(--accent) 20%, var(--bg-surface)); color: var(--accent); }
.trust-level--medium { background: color-mix(in srgb, var(--warn) 18%, var(--bg-surface)); color: var(--warn); }
.trust-level--low { background: color-mix(in srgb, var(--info) 18%, var(--bg-surface)); color: var(--info); }
.btn--sm { padding: 2px var(--sp-2); }
</style>
