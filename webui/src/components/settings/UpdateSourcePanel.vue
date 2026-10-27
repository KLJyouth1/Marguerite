<template>
  <section class="update-source" data-testid="update-source-panel">
    <header class="update-source__head">
      <h3 class="update-source__title">{{ t('settings.update.title') }}</h3>
      <p class="update-source__desc">{{ t('settings.update.desc') }}</p>
    </header>

    <p v-if="!supported" class="update-source__hint" data-testid="update-source-unsupported">
      {{ t('settings.update.unsupported') }}
    </p>

    <template v-else>
      <!-- 概览：当前版本 / 仓库 / 分支 / 上次检查与同步 / 本地内容计数 -->
      <section class="update-source__card">
        <div v-if="loadingConfig" class="update-source__hint" role="status" aria-live="polite">
          {{ t('settings.update.loading') }}
        </div>
        <template v-else>
          <dl class="update-source__meta">
            <div>
              <dt>{{ t('settings.update.currentVersion') }}</dt>
              <dd>{{ config?.currentVersion || '—' }}</dd>
            </div>
            <div>
              <dt>{{ t('settings.update.repo') }}</dt>
              <dd>{{ config?.repo || '—' }}</dd>
            </div>
            <div>
              <dt>{{ t('settings.update.branch') }}</dt>
              <dd>{{ config?.branch || '—' }}</dd>
            </div>
            <div>
              <dt>{{ t('settings.update.checkedAt') }}</dt>
              <dd>{{ formatTime(config?.checkedAt) }}</dd>
            </div>
            <div>
              <dt>{{ t('settings.update.syncedAt') }}</dt>
              <dd>{{ formatTime(config?.syncedAt) }}</dd>
            </div>
            <div>
              <dt>{{ t('settings.update.content') }}</dt>
              <dd>
                {{ t('settings.update.skills') }} {{ config?.content?.skills ?? 0 }}
                · {{ t('settings.update.plugins') }} {{ config?.content?.plugins ?? 0 }}
              </dd>
            </div>
          </dl>
          <p class="update-source__hint" data-testid="update-source-configured">
            {{ config?.configured ? t('settings.update.configured') : t('settings.update.notConfigured') }}
          </p>
        </template>

        <p v-if="loadError" class="update-source__error" role="alert">{{ loadError }}</p>
      </section>

      <!-- 内容源：仓库 + 分支 -->
      <section class="update-source__card">
        <label class="update-source__field" for="update-source-repo">
          <span>{{ t('settings.update.repo') }}</span>
          <input
            id="update-source-repo"
            v-model="repoInput"
            data-testid="update-source-repo"
            type="text"
            spellcheck="false"
            autocomplete="off"
            :disabled="busy"
            placeholder="owner/name"
          />
        </label>
        <label class="update-source__field" for="update-source-branch">
          <span>{{ t('settings.update.branch') }}</span>
          <input
            id="update-source-branch"
            v-model="branchInput"
            data-testid="update-source-branch"
            type="text"
            spellcheck="false"
            autocomplete="off"
            :disabled="busy"
            placeholder="main"
          />
        </label>
        <div class="update-source__row">
          <button
            type="button"
            class="btn btn--ghost"
            data-testid="update-source-save"
            :disabled="busy"
            :aria-busy="saving ? 'true' : undefined"
            @click="saveSource"
          >
            {{ saving ? t('settings.update.saving') : t('settings.update.save') }}
          </button>
          <span v-if="saveMessage" class="update-source__ok" role="status" aria-live="polite">
            {{ saveMessage }}
          </span>
        </div>
        <p v-if="saveError" class="update-source__error" role="alert">{{ saveError }}</p>
      </section>

      <!-- 版本检查 -->
      <section class="update-source__card">
        <div class="update-source__row">
          <button
            type="button"
            class="btn btn--ghost"
            data-testid="update-source-check"
            :disabled="busy"
            :aria-busy="checking ? 'true' : undefined"
            @click="checkUpdate"
          >
            <Icon name="refresh" :size="15" aria-hidden="true" />
            <span>{{ checking ? t('settings.update.checking') : t('settings.update.check') }}</span>
          </button>
          <span v-if="checkResult?.ok" class="update-source__ok" role="status" aria-live="polite">
            {{
              checkResult.updateAvailable === true
                ? t('settings.update.updateAvailable')
                : checkResult.updateAvailable === false
                  ? t('settings.update.upToDate')
                  : t('settings.update.unknownCompare')
            }}
          </span>
        </div>

        <p v-if="checkError" class="update-source__error" role="alert" data-testid="update-source-check-error">
          {{ checkError }}
        </p>

        <template v-if="checkResult?.ok">
          <dl class="update-source__meta">
            <div>
              <dt>{{ t('settings.update.latestVersion') }}</dt>
              <dd>{{ checkResult.latestTag || '—' }}</dd>
            </div>
            <div v-if="checkResult.latestName">
              <dt>{{ t('settings.update.currentVersion') }}</dt>
              <dd>{{ checkResult.currentVersion || '—' }}</dd>
            </div>
          </dl>
          <p v-if="checkResult.note" class="update-source__hint">{{ checkResult.note }}</p>

          <div class="update-source__notes">
            <h4 class="update-source__subtitle">{{ t('settings.update.releaseNotes') }}</h4>
            <pre v-if="notesSummary" class="update-source__notes-body">{{ notesSummary }}</pre>
            <p v-else class="update-source__hint">{{ t('settings.update.noNotes') }}</p>
          </div>

          <div v-if="checkResult.assets?.length" class="update-source__assets">
            <h4 class="update-source__subtitle">{{ t('settings.update.assets') }}</h4>
            <ul class="update-source__list">
              <li v-for="asset in checkResult.assets ?? []" :key="asset.name" class="update-source__asset">
                <span>{{ asset.name }}</span>
                <span class="update-source__dim">{{ formatByteSize(asset.size ?? null) }}</span>
              </li>
            </ul>
          </div>

          <!-- 下载页：用既有 anchor + noopener 打开（桌面壳会接管），不直连 window.open -->
          <a
            v-if="checkResult.htmlUrl"
            class="btn btn--ghost update-source__link"
            :href="checkResult.htmlUrl"
            target="_blank"
            rel="noopener noreferrer"
            data-testid="update-source-release-link"
          >
            <Icon name="externalLink" :size="15" aria-hidden="true" />
            <span>{{ t('settings.update.releasePage') }}</span>
          </a>
        </template>
      </section>

      <!-- 内容同步：先 dry-run 展示真实差异，再二次确认 apply -->
      <section class="update-source__card">
        <div class="update-source__row">
          <button
            type="button"
            class="btn btn--ghost"
            data-testid="update-source-sync"
            :disabled="busy"
            :aria-busy="syncing ? 'true' : undefined"
            @click="syncDryRun"
          >
            <Icon name="download" :size="15" aria-hidden="true" />
            <span>{{ syncing ? t('settings.update.syncing') : t('settings.update.sync') }}</span>
          </button>
          <span v-if="dryRun?.ok" class="update-source__dim" data-testid="update-source-total">
            {{ t('settings.update.diffTotal', { total: dryRun.total ?? 0 }) }}
          </span>
        </div>

        <p v-if="syncError" class="update-source__error" role="alert" data-testid="update-source-sync-error">
          {{ syncError }}
        </p>
        <p v-if="applyMessage" class="update-source__ok" role="status" aria-live="polite">
          {{ applyMessage }}
        </p>

        <template v-if="dryRun?.ok">
          <div class="update-source__diff">
            <div>
              <h4 class="update-source__subtitle">
                {{ t('settings.update.diffAdded') }} ({{ dryRun.added?.length ?? 0 }})
              </h4>
              <ul v-if="dryRun.added?.length" class="update-source__list">
                <li v-for="item in (dryRun.added ?? []).slice(0, 12)" :key="`added-${item.kind}-${item.path}`">
                  <code>{{ item.kind }}/{{ item.name }}</code>
                </li>
              </ul>
              <p v-else class="update-source__hint">{{ t('settings.update.none') }}</p>
            </div>
            <div>
              <h4 class="update-source__subtitle">
                {{ t('settings.update.diffChanged') }} ({{ dryRun.changed?.length ?? 0 }})
              </h4>
              <ul v-if="dryRun.changed?.length" class="update-source__list">
                <li v-for="item in (dryRun.changed ?? []).slice(0, 12)" :key="`changed-${item.kind}-${item.path}`">
                  <code>{{ item.kind }}/{{ item.name }}</code>
                </li>
              </ul>
              <p v-else class="update-source__hint">{{ t('settings.update.none') }}</p>
            </div>
            <div>
              <h4 class="update-source__subtitle">
                {{ t('settings.update.diffUnchanged') }} ({{ dryRun.unchanged?.length ?? 0 }})
              </h4>
              <p v-if="!dryRun.unchanged?.length" class="update-source__hint">
                {{ t('settings.update.none') }}
              </p>
              <p v-else class="update-source__hint">
                {{ t('settings.update.unchangedHint', { n: dryRun.unchanged.length }) }}
              </p>
            </div>
          </div>

          <p v-if="pendingCount === 0" class="update-source__hint">
            {{ t('settings.update.nothingToApply') }}
          </p>
          <div v-else class="update-source__row">
            <button
              type="button"
              class="btn btn--primary"
              data-testid="update-source-apply"
              :disabled="busy"
              :aria-busy="applying ? 'true' : undefined"
              @click="applySync"
            >
              {{ applying ? t('settings.update.applying') : t('settings.update.apply') }}
            </button>
          </div>
        </template>
      </section>
    </template>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useConfirm } from '@/composables/useConfirm'
import { useRpcStore } from '@/stores/rpc'
import { formatByteSize } from '@/utils/migrationReport'

interface UpdateConfigView {
  ok: boolean
  configured?: boolean
  repo?: string
  branch?: string
  currentVersion?: string
  checkedAt?: string | null
  syncedAt?: string | null
  content?: { skills?: number; plugins?: number }
  code?: string
  message?: string
}

interface ReleaseAsset {
  name: string
  browser_download_url: string
  size?: number | null
}

interface UpdateCheckView {
  ok: boolean
  code?: string
  message?: string
  latestTag?: string
  latestName?: string
  notes?: string
  htmlUrl?: string
  assets?: ReleaseAsset[]
  currentVersion?: string
  updateAvailable?: boolean | null
  note?: string
}

interface SyncItem {
  kind: string
  name: string
  path: string
}

interface SyncView {
  ok: boolean
  applied?: boolean
  code?: string
  message?: string
  added?: SyncItem[]
  changed?: SyncItem[]
  unchanged?: SyncItem[]
  total?: number
}

const { t } = useI18n()
const rpc = useRpcStore()
const { confirm } = useConfirm()

const REPO_RE = /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/

const supported = computed(() => rpc.supportsMethod('update.config.get'))

const config = ref<UpdateConfigView | null>(null)
const repoInput = ref('')
const branchInput = ref('')
const loadingConfig = ref(false)
const loadError = ref('')

const saving = ref(false)
const saveError = ref('')
const saveMessage = ref('')

const checking = ref(false)
const checkError = ref('')
const checkResult = ref<UpdateCheckView | null>(null)

const syncing = ref(false)
const applying = ref(false)
const syncError = ref('')
const applyMessage = ref('')
const dryRun = ref<SyncView | null>(null)

const busy = computed(
  () => loadingConfig.value || saving.value || checking.value || syncing.value || applying.value,
)
const pendingCount = computed(
  () => (dryRun.value?.added?.length ?? 0) + (dryRun.value?.changed?.length ?? 0),
)
const notesSummary = computed(() => {
  const notes = checkResult.value?.notes?.trim() ?? ''
  if (!notes) return ''
  const clipped = notes.split('\n').slice(0, 8).join('\n')
  return clipped.length > 600 ? `${clipped.slice(0, 600)}…` : clipped
})

function errorText(err: unknown): string {
  if (err instanceof Error) return err.message
  return String(err)
}

function isTimeout(err: unknown): boolean {
  return typeof err === 'object' && err !== null && (err as { code?: string }).code === 'RPC_TIMEOUT'
}

function failureText(err: unknown): string {
  return isTimeout(err) ? t('settings.update.timeout') : errorText(err)
}

function checkFailureText(res: UpdateCheckView): string {
  const code = res.code ?? ''
  if (code === 'NO_RELEASES') return t('settings.update.noReleases')
  if (code === 'NET_ERROR') return t('settings.update.netError', { error: res.message ?? '' })
  if (code.startsWith('HTTP_')) return t('settings.update.httpError', { status: code.slice(5) })
  if (code === 'BAD_RESPONSE') return t('settings.update.badResponse')
  return t('settings.update.checkFailed', { code: code || 'UNKNOWN', message: res.message ?? '' })
}

function syncFailureText(res: SyncView): string {
  const code = res.code ?? ''
  if (code === 'MANIFEST_MISSING') return t('settings.update.manifestMissing')
  if (code === 'UNSUPPORTED_MANIFEST') return t('settings.update.unsupportedManifest')
  if (code === 'INVALID_MANIFEST') return t('settings.update.invalidManifest')
  if (code === 'CHECKSUM_MISMATCH') return t('settings.update.checksumMismatch')
  if (code === 'APPLY_FAILED') return t('settings.update.applyFailed', { error: res.message ?? '' })
  if (code === 'NET_ERROR') return t('settings.update.netError', { error: res.message ?? '' })
  if (code.startsWith('HTTP_')) return t('settings.update.httpError', { status: code.slice(5) })
  return t('settings.update.syncFailed', { code: code || 'UNKNOWN', message: res.message ?? '' })
}

function formatTime(iso?: string | null): string {
  if (!iso) return t('settings.update.never')
  const date = new Date(iso)
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString()
}

function applyConfig(view: UpdateConfigView) {
  config.value = view
  repoInput.value = view.repo ?? ''
  branchInput.value = view.branch ?? ''
}

async function loadConfig() {
  if (!supported.value) return
  loadingConfig.value = true
  loadError.value = ''
  try {
    const view = await rpc.call<UpdateConfigView>('update.config.get', {}, { timeoutMs: 15_000 })
    if (!view.ok) {
      loadError.value = t('settings.update.loadFailed', { error: view.message ?? view.code ?? '' })
      return
    }
    applyConfig(view)
  } catch (err) {
    loadError.value = t('settings.update.loadFailed', { error: failureText(err) })
  } finally {
    loadingConfig.value = false
  }
}

async function saveSource() {
  saveError.value = ''
  saveMessage.value = ''
  if (!REPO_RE.test(repoInput.value.trim())) {
    saveError.value = t('settings.update.invalidRepo')
    return
  }
  if (!branchInput.value.trim() || /\s/.test(branchInput.value.trim())) {
    saveError.value = t('settings.update.invalidBranch')
    return
  }
  saving.value = true
  try {
    const view = await rpc.call<UpdateConfigView>(
      'update.config.set',
      { repo: repoInput.value.trim(), branch: branchInput.value.trim() },
      { timeoutMs: 15_000, queue: false, timeoutAction: 'reject' },
    )
    if (!view.ok) {
      saveError.value = view.message ?? t('settings.update.saveFailed', { code: view.code ?? 'UNKNOWN' })
      return
    }
    applyConfig(view)
    saveMessage.value = t('settings.update.saved')
  } catch (err) {
    saveError.value = t('settings.update.saveFailed', { code: failureText(err) })
  } finally {
    saving.value = false
  }
}

async function checkUpdate() {
  checking.value = true
  checkError.value = ''
  try {
    const view = await rpc.call<UpdateCheckView>(
      'update.check',
      {},
      { timeoutMs: 25_000, timeoutAction: 'reject' },
    )
    checkResult.value = view
    if (!view.ok) {
      checkError.value = checkFailureText(view)
      return
    }
    // checkedAt 写回了服务端，回读一次保证顶部时间戳是真实值。
    await loadConfig()
  } catch (err) {
    checkResult.value = null
    checkError.value = t('settings.update.checkFailed', {
      code: isTimeout(err) ? 'TIMEOUT' : 'RPC',
      message: failureText(err),
    })
  } finally {
    checking.value = false
  }
}

async function syncDryRun() {
  syncing.value = true
  syncError.value = ''
  applyMessage.value = ''
  dryRun.value = null
  try {
    const view = await rpc.call<SyncView>(
      'update.syncContent',
      { apply: false },
      { timeoutMs: 90_000, timeoutAction: 'reject' },
    )
    if (!view.ok) {
      syncError.value = syncFailureText(view)
      return
    }
    dryRun.value = view
  } catch (err) {
    syncError.value = t('settings.update.syncFailed', { code: 'RPC', message: failureText(err) })
  } finally {
    syncing.value = false
  }
}

async function applySync() {
  const pending = dryRun.value
  if (!pending?.ok || pendingCount.value === 0) return
  const ok = await confirm({
    title: t('settings.update.confirmTitle'),
    body: t('settings.update.confirmBody', {
      added: pending.added?.length ?? 0,
      changed: pending.changed?.length ?? 0,
    }),
    primaryLabel: t('settings.update.confirmApply'),
    primaryClass: 'btn--primary',
  })
  if (!ok) return
  applying.value = true
  syncError.value = ''
  applyMessage.value = ''
  try {
    const view = await rpc.call<SyncView>(
      'update.syncContent',
      { apply: true },
      { timeoutMs: 120_000, queue: false, timeoutAction: 'reject' },
    )
    if (!view.ok || view.applied !== true) {
      syncError.value = syncFailureText(view)
      return
    }
    applyMessage.value = t('settings.update.applyDone', {
      added: view.added?.length ?? 0,
      changed: view.changed?.length ?? 0,
    })
    dryRun.value = null
    await loadConfig()
  } catch (err) {
    syncError.value = t('settings.update.applyFailed', { error: failureText(err) })
  } finally {
    applying.value = false
  }
}

onMounted(loadConfig)
watch(supported, (available) => {
  if (available && !config.value && !loadingConfig.value) void loadConfig()
})
</script>

<style scoped>
.update-source {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
}

.update-source__head {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.update-source__title {
  font-size: var(--fs-lg);
  font-weight: 700;
  margin: 0;
}

.update-source__desc,
.update-source__hint {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  margin: 0;
}

.update-source__card {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4);
}

.update-source__meta {
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  margin: 0;
}

.update-source__meta dt {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.update-source__meta dd {
  font-size: var(--fs-sm);
  margin: 2px 0 0;
  overflow-wrap: anywhere;
}

.update-source__field {
  display: flex;
  flex-direction: column;
  font-size: var(--fs-sm);
  font-weight: 600;
  gap: 4px;
}

.update-source__field input {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-family: var(--font-mono, monospace);
  font-size: var(--fs-sm);
  padding: 6px 8px;
}

.update-source__row {
  align-items: center;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-3);
}

.update-source__ok {
  color: var(--ok, var(--accent));
  font-size: var(--fs-sm);
}

.update-source__error {
  color: var(--danger);
  font-size: var(--fs-sm);
  margin: 0;
  overflow-wrap: anywhere;
}

.update-source__dim {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.update-source__subtitle {
  font-size: var(--fs-sm);
  font-weight: 700;
  margin: 0 0 var(--sp-2);
}

.update-source__notes-body {
  background: var(--bg);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-family: var(--font-mono, monospace);
  font-size: var(--fs-xs);
  margin: 0;
  max-height: 180px;
  overflow: auto;
  padding: var(--sp-2);
  white-space: pre-wrap;
}

.update-source__list {
  display: flex;
  flex-direction: column;
  font-size: var(--fs-xs);
  gap: 4px;
  list-style: none;
  margin: 0;
  padding: 0;
}

.update-source__asset {
  display: flex;
  gap: var(--sp-3);
  justify-content: space-between;
}

.update-source__link {
  align-self: flex-start;
  text-decoration: none;
}

.update-source__diff {
  display: grid;
  gap: var(--sp-3);
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
}
</style>
