<!-- 输入框旁的模型选择器：云端厂商 / 自定义 / 本地模型 三档。
     为什么需要它：切换模型的入口此前只存在于「设置 → 服务商」，而聊天页是用户最常驻的
     界面；本机实测用户有 5 份 Ollama 档案却只能看到一个，无法在对话中直接切档。
     数据源纪律：单一事实源 = `config.get`（`llm_profile_rows` + `llm`），不另算；
     变更通过 `onboarding.llmProfile.activate` / `models.local.ollama.deploy` 走后端既有路径，
     成功后由 `models.changed` 事件驱动刷新，前端不猜测生效状态。 -->
<template>
  <div
    ref="rootRef"
    class="model-picker"
    :class="{
      'model-picker--compact': variant === 'compact',
      'model-picker--below': panelPlacement === 'below',
    }"
  >
    <button
      type="button"
      class="model-picker__trigger"
      :class="{ 'is-open': open }"
      :title="triggerTitle"
      :aria-label="triggerTitle"
      aria-haspopup="dialog"
      :aria-expanded="open ? 'true' : 'false'"
      :disabled="rpc.state !== 'connected'"
      @click="toggle"
    >
      <span class="model-picker__dot" :class="{ 'model-picker__dot--local': currentIsLocal }" aria-hidden="true" />
      <span class="model-picker__name">{{ currentLabel }}</span>
      <Icon name="chevronDown" :size="12" />
    </button>

    <section
      v-if="open"
      ref="panelRef"
      class="model-picker__panel"
      role="dialog"
      :aria-label="t('chat.modelPicker.title')"
      tabindex="-1"
      @keydown.esc.stop="close"
    >
      <header class="model-picker__head">
        <span>{{ t('chat.modelPicker.title') }}</span>
        <button type="button" class="model-picker__close" :aria-label="t('chat.modelPicker.close')" @click="close">
          <Icon name="x" :size="14" />
        </button>
      </header>

      <p v-if="errorMessage" class="model-picker__error" role="alert">{{ errorMessage }}</p>

      <div class="model-picker__scroll">
        <template v-for="group in groupOrder" :key="group">
          <template v-if="groups[group].length || group === 'local' || group === 'custom'">
            <p class="model-picker__heading">
              <span>{{ t(`chat.modelPicker.${group}`) }}</span>
              <span class="model-picker__count">{{ t('chat.modelPicker.count', { count: groups[group].length }) }}</span>
              <button
                v-if="group === 'local'"
                type="button"
                class="model-picker__refresh"
                :disabled="Boolean(busy) || localLoading"
                @click="loadLocal"
              >
                {{ localLoading ? t('chat.modelPicker.refreshingLocal') : t('chat.modelPicker.refreshLocal') }}
              </button>
              <!-- 自定义档此前永远是空列表且无入口：用户看得见「自定义」却无法添加。
                   这里给出与云端/本地对称的添加入口。 -->
              <button
                v-else-if="group === 'custom'"
                type="button"
                class="model-picker__refresh"
                :disabled="Boolean(busy)"
                @click="toggleCustomForm"
              >
                {{ addingCustom ? t('chat.modelPicker.customCancel') : t('chat.modelPicker.addCustom') }}
              </button>
            </p>

            <form
              v-if="group === 'custom' && addingCustom"
              class="model-picker__form"
              @submit.prevent="submitCustom"
            >
              <input
                v-model="customDraft.label"
                class="model-picker__input"
                type="text"
                autocomplete="off"
                :placeholder="t('chat.modelPicker.customLabel')"
              >
              <input
                v-model="customDraft.baseUrl"
                class="model-picker__input"
                type="url"
                autocomplete="off"
                spellcheck="false"
                :placeholder="t('chat.modelPicker.customBaseUrl')"
              >
              <input
                v-model="customDraft.model"
                class="model-picker__input"
                type="text"
                autocomplete="off"
                spellcheck="false"
                :placeholder="t('chat.modelPicker.customModel')"
              >
              <input
                v-model="customDraft.apiKey"
                class="model-picker__input"
                type="password"
                autocomplete="off"
                :placeholder="t('chat.modelPicker.customApiKey')"
              >
              <button type="submit" class="model-picker__submit" :disabled="Boolean(busy)">
                {{ busy === 'custom' ? t('chat.modelPicker.activating') : t('chat.modelPicker.customSubmit') }}
              </button>
            </form>

            <button
              v-for="row in groups[group]"
              :key="row.id"
              type="button"
              class="model-picker__row"
              :class="{ 'is-active': row.active }"
              :disabled="Boolean(busy)"
              @click="activate(row)"
            >
              <span class="model-picker__row-main">
                <span class="model-picker__row-label">{{ row.label || row.model }}</span>
                <span class="model-picker__row-meta">{{ row.provider }} · {{ row.model }}</span>
              </span>
              <span v-if="busy === row.id" class="model-picker__row-busy">{{ t('chat.modelPicker.activating') }}</span>
              <Icon v-else-if="row.active" name="check" :size="14" />
            </button>

            <p v-if="group === 'local' && !groups.local.length" class="model-picker__empty">
              {{ t('chat.modelPicker.empty') }}
            </p>
            <p v-if="group === 'local' && localLive.length" class="model-picker__heading model-picker__heading--sub">
              {{ t('chat.modelPicker.refreshLocal') }}
            </p>
            <button
              v-for="m in localLive"
              :key="'live:' + m.name"
              type="button"
              class="model-picker__row model-picker__row--live"
              :disabled="Boolean(busy)"
              @click="deployLocal(m)"
            >
              <span class="model-picker__row-main">
                <span class="model-picker__row-label">{{ m.name }}</span>
                <span class="model-picker__row-meta">{{ formatSize(m.size) }}</span>
              </span>
              <span v-if="busy === 'live:' + m.name" class="model-picker__row-busy">{{ t('chat.modelPicker.activating') }}</span>
            </button>
            <p v-if="group === 'local'" class="model-picker__hint">{{ t('chat.modelPicker.localHint') }}</p>
          </template>
        </template>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useLlmProfiles, type LlmProfileRow } from '@/composables/useLlmProfiles'
import { useRpcStore } from '@/stores/rpc'

type Group = 'cloud' | 'custom' | 'local'

interface OllamaLiveModel {
  name: string
  size?: number
}

interface OllamaLiveResult {
  ok?: boolean
  online?: boolean
  base_url?: string
  models?: OllamaLiveModel[]
  error?: string
}

const { t } = useI18n()
const rpc = useRpcStore()
const { rows, active, load } = useLlmProfiles()

const props = withDefaults(defineProps<{
  /** `compact` 只收紧触发器外观，供落地页 hero 等非输入框位置内联使用。 */
  variant?: 'default' | 'compact'
  /** 面板默认向上展开（输入框贴着窗口底部）；落地页在中上部时必须向下。 */
  panelPlacement?: 'above' | 'below'
}>(), {
  variant: 'default',
  panelPlacement: 'above',
})

const open = ref(false)
const busy = ref('')
const errorMessage = ref('')
const rootRef = ref<HTMLElement | null>(null)
const panelRef = ref<HTMLElement | null>(null)
const localLoading = ref(false)
const localLive = ref<OllamaLiveModel[]>([])
const addingCustom = ref(false)
const customDraft = ref({ label: '', baseUrl: '', model: '', apiKey: '' })

const groupOrder: Group[] = ['cloud', 'custom', 'local']

/** 档案分档：`kind` 是唯一权威；旧网关缺 kind 时按 provider / 回环地址推断（与后端同语义）。 */
function kindOf(row: LlmProfileRow): Group {
  const kind = String(row.kind || '').toLowerCase()
  if (kind === 'cloud' || kind === 'custom' || kind === 'local') return kind
  const provider = String(row.provider || '').toLowerCase()
  const baseUrl = String(row.base_url || '').toLowerCase()
  if (provider === 'ollama' || /localhost|127\.0\.0\.1|\[::1\]/.test(baseUrl)) return 'local'
  return 'cloud'
}

const groups = computed<Record<Group, LlmProfileRow[]>>(() => {
  const buckets: Record<Group, LlmProfileRow[]> = { cloud: [], custom: [], local: [] }
  for (const row of rows.value) buckets[kindOf(row)].push(row)
  return buckets
})

const currentRow = computed(() => rows.value.find((row) => row.active))
const currentLabel = computed(() => currentRow.value?.label || active.value.model || '—')
const currentIsLocal = computed(() => (currentRow.value ? kindOf(currentRow.value) === 'local' : false))
const triggerTitle = computed(() =>
  t('chat.modelIndicator.tooltip', {
    model: active.value.model || '—',
    provider: active.value.provider || '—',
    endpoint: active.value.baseUrl || '—',
  }),
)

function toggle() {
  if (open.value) {
    close()
    return
  }
  errorMessage.value = ''
  open.value = true
  void load()
  void loadLocal()
  setTimeout(() => panelRef.value?.focus(), 0)
}

function close() {
  open.value = false
}

function onDocPointerDown(event: Event) {
  if (!open.value) return
  const target = event.target as Node | null
  if (target && rootRef.value && !rootRef.value.contains(target)) close()
}

async function activate(row: LlmProfileRow) {
  if (busy.value || row.active) return
  busy.value = row.id
  errorMessage.value = ''
  try {
    // 走后端既有激活路径：它会 use_active + 镜像 config.model + publish 生效快照，
    // 因此不需要重启即可让下一轮使用新模型。
    // 同时传 `profileId`：同一 provider 下多份档案（本机 5 个 Ollama 模型）时，
    // 只有档案 id 是无歧义主键；providerId+model 仅作为旧网关的兼容寻址。
    await rpc.call('onboarding.llmProfile.activate', {
      profileId: row.id,
      providerId: row.provider,
      model: row.model,
    })
    await load()
    open.value = false
  } catch (e) {
    errorMessage.value = t('chat.modelPicker.failed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    busy.value = ''
  }
}

function toggleCustomForm() {
  errorMessage.value = ''
  addingCustom.value = !addingCustom.value
}

/** providerId 只能是 ASCII 标识：中文名称走模型名/时间戳兜底，避免生成空 id。 */
function providerSlug(...candidates: string[]): string {
  for (const candidate of candidates) {
    const slug = candidate
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 40)
    if (slug) return slug
  }
  return `custom-${Date.now().toString(36)}`
}

async function submitCustom() {
  if (busy.value) return
  const label = customDraft.value.label.trim()
  const baseUrl = customDraft.value.baseUrl.trim()
  const model = customDraft.value.model.trim()
  if (!baseUrl || !model) {
    errorMessage.value = t('chat.modelPicker.customRequired')
    return
  }
  busy.value = 'custom'
  errorMessage.value = ''
  try {
    const providerId = providerSlug(label, model)
    const apiKey = customDraft.value.apiKey.trim()
    const saved = await rpc.call<{ ok?: boolean; error?: string }>('onboarding.llmProfile.upsert', {
      providerId,
      model,
      baseUrl,
      label: label || model,
      kind: 'custom',
      ...(apiKey ? { apiKey } : {}),
    })
    if (saved && saved.ok === false) {
      errorMessage.value = t('chat.modelPicker.failed', { error: saved.error || providerId })
      return
    }
    // 保存即启用：用户点「保存并启用」后不该还要再点一次档案行。
    await rpc.call('onboarding.llmProfile.activate', { providerId, model })
    customDraft.value = { label: '', baseUrl: '', model: '', apiKey: '' }
    addingCustom.value = false
    await load()
    open.value = false
  } catch (e) {
    errorMessage.value = t('chat.modelPicker.failed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    busy.value = ''
  }
}

async function loadLocal() {
  if (localLoading.value) return
  localLoading.value = true
  try {
    const result = await rpc.call<OllamaLiveResult>('models.local.ollama', {})
    localLive.value = result?.online === false ? [] : (result?.models || [])
  } catch {
    // 旧网关没有该 RPC 或 Ollama 未运行：静默降级为空列表，不抛裸错给用户。
    localLive.value = []
  } finally {
    localLoading.value = false
  }
}

async function deployLocal(model: OllamaLiveModel) {
  const key = 'live:' + model.name
  if (busy.value) return
  busy.value = key
  errorMessage.value = ''
  try {
    const result = await rpc.call<{ ok?: boolean; error?: string }>('models.local.ollama.deploy', {
      model: model.name,
      activate: true,
    })
    if (result && result.ok === false) {
      errorMessage.value = t('chat.modelPicker.failed', { error: result.error || model.name })
      return
    }
    await load()
    open.value = false
  } catch (e) {
    errorMessage.value = t('chat.modelPicker.failed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    busy.value = ''
  }
}

function formatSize(bytes?: number): string {
  if (!bytes) return ''
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`
}

onMounted(() => document.addEventListener('pointerdown', onDocPointerDown, true))
onBeforeUnmount(() => document.removeEventListener('pointerdown', onDocPointerDown, true))
</script>

<style scoped>
.model-picker {
  position: relative;
  display: inline-flex;
}

.model-picker__trigger {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  max-width: 15rem;
  min-height: 32px;
  padding: 0.125rem 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
}

.model-picker__trigger:hover:not(:disabled),
.model-picker__trigger.is-open {
  border-color: var(--border-strong);
  color: var(--text);
}

.model-picker__trigger:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* 紧凑变体：落地页 hero 用，与只读项目/分支 chip 同一行、同一高度。 */
.model-picker--compact .model-picker__trigger {
  min-height: 1.625rem;
  padding: 0.0625rem 0.5rem;
  gap: 5px;
  max-width: 12rem;
  background: var(--bg-elevated, var(--bg-surface));
  font-family: var(--font-mono);
}

.model-picker--compact .model-picker__dot {
  width: 5px;
  height: 5px;
}

.model-picker__dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  flex: 0 0 auto;
}

.model-picker__dot--local {
  background: var(--ok);
}

.model-picker__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.model-picker__panel {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 0;
  z-index: 40;
  display: flex;
  flex-direction: column;
  width: min(380px, calc(100vw - 48px));
  max-height: min(70vh, 520px);
  overflow: hidden;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-xl);
  outline: none;
}

/* 落地页 hero 在页面顶部，面板必须向下展开；默认仍向上。 */
.model-picker--below .model-picker__panel {
  top: calc(100% + 8px);
  bottom: auto;
}

.model-picker__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--sp-2) var(--sp-2) var(--sp-1);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 650;
}

.model-picker__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-dim);
  cursor: pointer;
}

.model-picker__error {
  margin: 0 var(--sp-2) var(--sp-1);
  padding: var(--sp-1) var(--sp-2);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  color: var(--danger);
  font-size: var(--fs-xs);
}

.model-picker__scroll {
  /* 面板是 flex 列：滚动区必须能收缩（min-height:0）才会出现滚动条，
     否则 flex 子项的 min-height:auto 会把它撑到内容高度、整块顶出面板。 */
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 0 var(--sp-2) var(--sp-2);
  scrollbar-width: thin;
  scrollbar-color: var(--border-strong) transparent;
}

.model-picker__scroll::-webkit-scrollbar {
  width: 8px;
}

.model-picker__scroll::-webkit-scrollbar-thumb {
  border: 2px solid transparent;
  border-radius: var(--radius-pill);
  /* 用 text-muted 而不是 border-strong：深色面板上 border-strong 几乎看不见，
     用户会以为"没有滚动条、列表被截断了"。 */
  background: color-mix(in srgb, var(--text-muted) 42%, transparent);
  background-clip: content-box;
}

.model-picker__scroll::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--text-muted) 55%, transparent);
  background-clip: content-box;
}

.model-picker__heading {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin: 0;
  padding: var(--sp-2) var(--sp-1) var(--sp-1);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 650;
}

.model-picker__heading--sub {
  padding-top: var(--sp-1);
  opacity: 0.8;
}

.model-picker__count {
  margin-inline-start: auto;
  opacity: 0.7;
  font-weight: 400;
}

.model-picker__refresh {
  border: 0;
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
}

.model-picker__refresh:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-picker__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
  width: 100%;
  min-height: 44px;
  padding: var(--sp-1) var(--sp-2);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.model-picker__row:hover:not(:disabled) {
  background: var(--bg-hover);
}

.model-picker__row.is-active {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.model-picker__row:disabled {
  cursor: not-allowed;
  opacity: 0.7;
}

.model-picker__row-main {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.model-picker__row-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
}

.model-picker__row-meta {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.model-picker__row-busy {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.model-picker__empty,
.model-picker__hint {
  margin: 0;
  padding: var(--sp-1) var(--sp-2);
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

/* 自定义模型新增表单：与档案行同宽同语汇，不引入新的视觉体系。 */
.model-picker__form {
  display: grid;
  gap: var(--sp-1);
  padding: var(--sp-1) var(--sp-1) var(--sp-2);
}

.model-picker__input {
  min-height: 30px;
  padding: 0 var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-surface-2);
  color: var(--text);
  font: inherit;
  font-size: var(--fs-sm);
}

.model-picker__input:focus {
  outline: none;
  border-color: var(--border-strong);
}

.model-picker__submit {
  min-height: 32px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-sm);
  font-weight: 600;
}

.model-picker__submit:hover:not(:disabled) {
  background: color-mix(in srgb, var(--accent) 22%, transparent);
}

.model-picker__submit:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>
