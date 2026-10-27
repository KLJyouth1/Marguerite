<template>
  <section class="multimodal-caps" data-testid="multimodal-caps" aria-labelledby="multimodal-caps-title">
    <header class="multimodal-caps__header">
      <div>
        <h3 id="multimodal-caps-title">多模态能力</h3>
        <p>
          告诉主智能体「这份档案能做什么」。勾选后 <code>image_generate</code> /
          <code>audio_transcribe</code> / <code>audio_speak</code> 才会走这份配置；
          不勾选时按模型名自动识别（例如模型名含 <code>dall-e</code> 才判为能出图）。
        </p>
      </div>
    </header>

    <div v-if="loading" class="multimodal-caps__state" role="status">加载中…</div>
    <div v-else-if="loadError" class="multimodal-caps__state" role="alert">
      <span>{{ loadError }}</span>
      <button type="button" class="btn" @click="void load()">重试</button>
    </div>

    <template v-else>
      <label v-if="profiles.length > 1" class="multimodal-caps__row">
        <span class="multimodal-caps__label">档案</span>
        <select
          class="control-input"
          :value="selectedId"
          data-testid="multimodal-profile-select"
          @change="onProfileChange(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="p in profiles" :key="p.id" :value="p.id">
            {{ p.label }} · {{ p.model }}{{ p.active ? '（主模型）' : '' }}
          </option>
        </select>
      </label>

      <p v-if="current" class="multimodal-caps__auto" data-testid="multimodal-auto">
        自动识别：{{ autoSummary }}
        <span v-if="current.capabilities?.declared" class="multimodal-caps__tag">已被手动声明覆盖</span>
      </p>

      <div class="multimodal-caps__grid" role="group" aria-label="能力勾选">
        <label v-for="item in FLAGS" :key="item.key" class="multimodal-caps__check">
          <ControlSwitch
            :checked="form[item.key]"
            :disabled="item.locked"
            :aria-label="item.label"
            :data-testid="`multimodal-flag-${item.key}`"
            @change="form[item.key] = $event"
          />
          <span>{{ item.label }}</span>
          <small>{{ item.hint }}</small>
        </label>
      </div>

      <details class="multimodal-caps__advanced">
        <summary>端点覆盖与自定义参数（留空即用供应商默认）</summary>
        <div class="multimodal-caps__fields">
          <label v-for="field in ENDPOINTS" :key="field.key" class="multimodal-caps__row">
            <span class="multimodal-caps__label">{{ field.label }}</span>
            <input
              class="control-input"
              :value="form[field.key]"
              :placeholder="field.placeholder"
              :data-testid="`multimodal-ep-${field.key}`"
              @input="form[field.key] = ($event.target as HTMLInputElement).value"
            >
          </label>
          <label v-for="field in PARAMS" :key="field.key" class="multimodal-caps__row">
            <span class="multimodal-caps__label">{{ field.label }}</span>
            <textarea
              class="control-input multimodal-caps__json"
              rows="2"
              :value="form[field.key]"
              :placeholder="field.placeholder"
              :data-testid="`multimodal-params-${field.key}`"
              @input="form[field.key] = ($event.target as HTMLTextAreaElement).value"
            />
          </label>
        </div>
      </details>

      <div class="multimodal-caps__actions">
        <button
          type="button"
          class="btn btn--primary"
          :disabled="saving || !current"
          data-testid="multimodal-save"
          @click="void save()"
        >{{ saving ? '保存中…' : '保存能力声明' }}</button>
        <button
          type="button"
          class="btn"
          :disabled="saving || !current?.capabilities?.declared"
          data-testid="multimodal-reset"
          @click="void reset()"
        >重置为自动识别</button>
        <span v-if="status" class="multimodal-caps__status" role="status" data-testid="multimodal-status">
          {{ status }}
        </span>
      </div>
    </template>
  </section>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'
import ControlSwitch from '@/components/ControlSwitch.vue'

interface CapabilityFlags {
  text?: boolean
  image_input?: boolean
  image_output?: boolean
  video_output?: boolean
  audio_input?: boolean
  audio_output?: boolean
  declared?: boolean
  image_endpoint?: string
  video_endpoint?: string
  audio_endpoint?: string
  image_params?: Record<string, unknown>
  video_params?: Record<string, unknown>
  audio_params?: Record<string, unknown>
}

interface ProfileRow {
  id: string
  label: string
  provider: string
  model: string
  active?: boolean
  capabilities?: CapabilityFlags
  effectiveCapabilities?: CapabilityFlags
}

interface ProfilesPayload {
  ok?: boolean
  error?: string
  active?: string
  profiles?: ProfileRow[]
}

type FlagKey = 'text' | 'image_input' | 'image_output' | 'video_output' | 'audio_input' | 'audio_output'
type EndpointKey = 'image_endpoint' | 'video_endpoint' | 'audio_endpoint'
type ParamsKey = 'image_params' | 'video_params' | 'audio_params'

/// 表单是**强类型**的：勾选框绑 boolean、端点绑 string、参数绑原始 JSON 文本。
/// 用 Record<string, unknown> 会让模板绑定退化成 any/unknown（vue-tsc 直接报错），
/// 也会让「把 boolean 塞进端点框」这类错误在运行时才暴露。
interface CapabilityForm {
  text: boolean
  image_input: boolean
  image_output: boolean
  video_output: boolean
  audio_input: boolean
  audio_output: boolean
  image_endpoint: string
  video_endpoint: string
  audio_endpoint: string
  image_params: string
  video_params: string
  audio_params: string
}

const FLAGS: Array<{ key: FlagKey, label: string, hint: string, locked?: boolean }> = [
  { key: 'text', label: '文本对话', hint: '所有档案都以此为准，取消无意义', locked: true },
  { key: 'image_input', label: '图像输入', hint: '能「看图」：读截图/设计稿' },
  { key: 'image_output', label: '图像输出', hint: '能生图：image_generate 可用' },
  { key: 'video_output', label: '视频输出', hint: '识别用；生成工具尚未内置' },
  { key: 'audio_input', label: '音频输入', hint: '能转写：audio_transcribe 走云端' },
  { key: 'audio_output', label: '语音输出', hint: '能朗读：audio_speak 走云端' },
]

const ENDPOINTS: Array<{ key: EndpointKey, label: string, placeholder: string }> = [
  { key: 'image_endpoint', label: '图像端点', placeholder: '留空 → /v1/images/generations' },
  { key: 'video_endpoint', label: '视频端点', placeholder: '留空 → /v1/videos/generations' },
  { key: 'audio_endpoint', label: '音频端点', placeholder: '留空 → /v1/audio/speech（或 transcriptions）' },
]

const PARAMS: Array<{ key: ParamsKey, label: string, placeholder: string }> = [
  { key: 'image_params', label: '图像参数', placeholder: '{"size":"1024x1024","n":1}' },
  { key: 'video_params', label: '视频参数', placeholder: '{"duration":5,"aspect_ratio":"16:9"}' },
  { key: 'audio_params', label: '音频参数', placeholder: '{"voice":"alloy","format":"mp3"}' },
]

const rpc = useRpcStore()
const loading = ref(true)
const loadError = ref('')
const saving = ref(false)
const status = ref('')
const profiles = ref<ProfileRow[]>([])
const selectedId = ref('')

const form = reactive<CapabilityForm>({
  text: true,
  image_input: false,
  image_output: false,
  video_output: false,
  audio_input: false,
  audio_output: false,
  image_endpoint: '',
  video_endpoint: '',
  audio_endpoint: '',
  image_params: '',
  video_params: '',
  audio_params: '',
})

const current = computed(() => profiles.value.find(p => p.id === selectedId.value) || null)

const FLAG_LABELS: Record<string, string> = {
  text: '文本',
  image_input: '图像输入',
  image_output: '图像输出',
  video_output: '视频输出',
  audio_input: '音频输入',
  audio_output: '语音输出',
}

const autoSummary = computed(() => {
  const eff = current.value?.effectiveCapabilities || {}
  const on = Object.keys(FLAG_LABELS)
    .filter(key => eff[key as keyof CapabilityFlags] === true)
    .map(key => FLAG_LABELS[key])
  return on.length ? on.join(' · ') : '无（判为纯文本，工具会如实拒绝）'
})

function applyProfile(p: ProfileRow | null): void {
  const caps = p?.capabilities || {}
  form.text = true
  form.image_input = caps.image_input === true
  form.image_output = caps.image_output === true
  form.video_output = caps.video_output === true
  form.audio_input = caps.audio_input === true
  form.audio_output = caps.audio_output === true
  form.image_endpoint = caps.image_endpoint || ''
  form.video_endpoint = caps.video_endpoint || ''
  form.audio_endpoint = caps.audio_endpoint || ''
  form.image_params = caps.image_params ? JSON.stringify(caps.image_params, null, 2) : ''
  form.video_params = caps.video_params ? JSON.stringify(caps.video_params, null, 2) : ''
  form.audio_params = caps.audio_params ? JSON.stringify(caps.audio_params, null, 2) : ''
}

function onProfileChange(id: string): void {
  selectedId.value = id
  status.value = ''
  applyProfile(current.value)
}

async function load(): Promise<void> {
  loading.value = true
  loadError.value = ''
  try {
    // 老网关没有这条 RPC 时给出明确原因，而不是把「方法不存在」当网络错误展示。
    // `methods.length === 0` 表示还没收到握手清单，此时不下结论（直接调用）。
    if (rpc.methods.length > 0 && !rpc.supportsMethod('models.profiles.list')) {
      loadError.value = '当前内核版本不提供模型档案接口（models.profiles.list），请更新内核后重试'
      return
    }
    const res = await rpc.call<ProfilesPayload>('models.profiles.list')
    if (res?.ok === false) {
      loadError.value = res.error || '无法读取模型档案'
      return
    }
    profiles.value = res?.profiles || []
    const preferred = selectedId.value
      || res?.active
      || profiles.value[0]?.id
      || ''
    selectedId.value = profiles.value.some(p => p.id === preferred)
      ? preferred
      : (profiles.value[0]?.id || '')
    applyProfile(current.value)
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

/// 把 textarea 里的 JSON 解析成对象；空串 → undefined（后端按"未填"处理）。
/// 语法错必须**当场拦住**：静默丢弃会让用户以为参数保存成功了。
function parseParams(key: ParamsKey): Record<string, unknown> | undefined | string {
  const raw = String(form[key] ?? '').trim()
  if (!raw) return undefined
  try {
    const parsed = JSON.parse(raw)
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
      return `${key} 必须是 JSON 对象`
    }
    return parsed as Record<string, unknown>
  } catch (e) {
    return `${key} 不是合法 JSON：${e instanceof Error ? e.message : String(e)}`
  }
}

async function save(): Promise<void> {
  if (!current.value) return
  saving.value = true
  status.value = ''
  try {
    const capabilities: Record<string, unknown> = {
      declared: true,
      text: true,
      image_input: Boolean(form.image_input),
      image_output: Boolean(form.image_output),
      video_output: Boolean(form.video_output),
      audio_input: Boolean(form.audio_input),
      audio_output: Boolean(form.audio_output),
      image_endpoint: String(form.image_endpoint ?? '').trim(),
      video_endpoint: String(form.video_endpoint ?? '').trim(),
      audio_endpoint: String(form.audio_endpoint ?? '').trim(),
    }
    for (const key of ['image_params', 'video_params', 'audio_params'] as ParamsKey[]) {
      const parsed = parseParams(key)
      if (typeof parsed === 'string') {
        status.value = parsed
        return
      }
      if (parsed) capabilities[key] = parsed
    }
    const res = await rpc.call<ProfilesPayload & { error?: string }>(
      'models.profiles.set_capabilities',
      { id: selectedId.value, capabilities },
    )
    if (res?.ok === false) {
      status.value = res.error || '保存失败'
      return
    }
    profiles.value = res?.profiles || profiles.value
    applyProfile(current.value)
    status.value = '已保存（下次调用工具即生效）'
  } catch (e) {
    status.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

async function reset(): Promise<void> {
  if (!current.value) return
  saving.value = true
  status.value = ''
  try {
    const res = await rpc.call<ProfilesPayload & { error?: string }>(
      'models.profiles.set_capabilities',
      { id: selectedId.value, reset: true },
    )
    if (res?.ok === false) {
      status.value = res.error || '重置失败'
      return
    }
    profiles.value = res?.profiles || profiles.value
    applyProfile(current.value)
    status.value = '已重置为按模型名自动识别'
  } catch (e) {
    status.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

onMounted(() => { void load() })
</script>

<style scoped>
.multimodal-caps {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  background: var(--bg-surface);
}

.multimodal-caps__header h3 {
  margin: 0 0 var(--sp-1);
  font-size: var(--fs-md);
}

.multimodal-caps__header p {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.55;
}

.multimodal-caps__header code {
  padding: 1px 4px;
  border-radius: var(--radius-sm);
  background: var(--bg-surface-2);
  font-size: var(--fs-xs);
}

.multimodal-caps__state {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.multimodal-caps__row {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

.multimodal-caps__label {
  flex: 0 0 96px;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.multimodal-caps__row .control-input {
  flex: 1 1 auto;
  min-width: 0;
}

.multimodal-caps__auto {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.multimodal-caps__tag {
  margin-inline-start: var(--sp-2);
  padding: 1px 6px;
  border-radius: var(--radius-pill);
  background: var(--bg-surface-2);
  font-size: var(--fs-xs);
}

.multimodal-caps__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
  gap: var(--sp-2);
}

.multimodal-caps__check {
  display: grid;
  grid-template-columns: auto 1fr;
  grid-template-rows: auto auto;
  column-gap: var(--sp-2);
  align-items: center;
  padding: var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
}

.multimodal-caps__check input {
  grid-row: span 2;
}

.multimodal-caps__check span {
  font-size: var(--fs-sm);
}

.multimodal-caps__check small {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.multimodal-caps__advanced summary {
  cursor: pointer;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}

.multimodal-caps__fields {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  margin-top: var(--sp-2);
}

.multimodal-caps__json {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  resize: vertical;
}

.multimodal-caps__actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-2);
}

.multimodal-caps__status {
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
</style>
