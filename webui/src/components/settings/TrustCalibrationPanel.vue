<script setup lang="ts">
import { onMounted, ref } from 'vue'
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

const entries = ref<TrustEntry[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const message = ref<string | null>(null)

// 授予表单
const grantLevel = ref<'low' | 'medium' | 'high'>('low')
const grantTool = ref('*')
const grantMode = ref('')

const LEVEL_HINTS: Record<string, string> = {
  low: '低 — 本会话内记住授权，退出即失效（不落盘）',
  medium: '中 — 记住当前会话的授权',
  high: '高 — 永久信任，落盘持久化，可随时撤销',
}

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
    message.value = '信任授权已记录'
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function revoke(entry: TrustEntry) {
  const ok = await confirm({
    title: '撤销信任',
    body: `确定撤销对「${entry.tool}」（${entry.level_label}）的信任？该授权凭据将被删除，可重新授予。`,
    primaryLabel: '撤销',
    primaryClass: 'btn--danger',
  })
  if (!ok) return
  try {
    await rpc.call('trust.revoke', { id: entry.id })
    message.value = `已撤销 ${entry.tool} 的信任`
    await refresh()
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  }
}

async function revokeAll() {
  const ok = await confirm({
    title: '撤销全部信任',
    body: '确定撤销全部信任授权？之后高影响工具将恢复逐次确认。',
    primaryLabel: '全部撤销',
    primaryClass: 'btn--danger',
  })
  if (!ok) return
  try {
    const data = await rpc.call<{ removed: number }>('trust.revoke_all')
    message.value = `已撤销 ${data.removed} 条信任`
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
      <h3 class="control-section__title">信任校准（Trust Calibration）</h3>
      <p class="control-section__desc">
        0-确认默认：高影响任务默认逐次确认；可在下方授予工具信任以跳过确认。信任分 low / medium / high 三档，覆盖全部 5 种运行模式，支持跨会话持久化与随时撤销。
      </p>
    </div>

    <div class="control-row control-row--stack">
      <div class="control-row__label-block">
        <span class="control-row__label">授予信任</span>
        <span class="control-row__desc">选择档位、工具名（含 * 通配）与可选模式；高影响工具命中信任时不再弹确认卡</span>
      </div>
      <div class="control-row__control trust-grant">
        <select v-model="grantLevel" class="control-input control-input--narrow" aria-label="信任档位">
          <option value="low">low（不落盘）</option>
          <option value="medium">medium（会话级）</option>
          <option value="high">high（永久落盘）</option>
        </select>
        <input v-model="grantTool" class="control-input" placeholder="tool 或 *" aria-label="工具名" />
        <input v-model="grantMode" class="control-input" placeholder="mode（留空=全部）" aria-label="模式" />
        <button type="button" class="btn" :disabled="loading || !grantTool.trim()" @click="grant">授予</button>
      </div>
      <p v-if="grantLevel" class="trust-hint">{{ LEVEL_HINTS[grantLevel] }}</p>
    </div>

    <div v-if="error" class="trust-banner trust-banner--err" role="alert">{{ error }}</div>
    <div v-else-if="message" class="trust-banner trust-banner--ok" role="status">{{ message }}</div>

    <div class="trust-toolbar">
      <span class="trust-toolbar__count">{{ entries.length }} 条信任</span>
      <button type="button" class="btn btn--ghost" :disabled="loading || entries.length === 0" @click="revokeAll">撤销全部</button>
      <button type="button" class="btn btn--ghost" :disabled="loading" @click="refresh">刷新</button>
    </div>

    <div v-if="loading" class="trust-empty">加载中…</div>
    <div v-else-if="entries.length === 0" class="trust-empty">暂无信任授权。所有高影响工具均走逐次确认。</div>
    <table v-else class="trust-table">
      <thead>
        <tr>
          <th>档位</th><th>工具</th><th>模式</th><th>会话</th><th>授予时间</th><th></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="e in entries" :key="e.id">
          <td><span class="trust-level" :class="'trust-level--' + e.level">{{ e.level_label }}</span></td>
          <td><code>{{ e.tool }}</code></td>
          <td>{{ e.mode ?? '全部' }}</td>
          <td>{{ e.session }}</td>
          <td>{{ fmtTime(e.granted_at_ms) }}</td>
          <td><button type="button" class="btn btn--ghost btn--sm" @click="revoke(e)">撤销</button></td>
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
