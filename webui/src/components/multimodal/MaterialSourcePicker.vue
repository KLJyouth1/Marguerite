<template>
  <section class="msp" data-testid="material-source-picker" :aria-labelledby="headingId">
    <h4 :id="headingId" class="msp__title">{{ t('settings.multimodalGen.title') }}</h4>

    <div class="msp__row">
      <span class="msp__label">{{ t('settings.multimodalGen.toolLabel') }}</span>
      <div class="msp__segment" role="radiogroup" :aria-label="t('settings.multimodalGen.toolLabel')">
        <button
          v-for="option in toolOptions"
          :key="option.tool"
          type="button"
          role="radio"
          class="msp__segment-btn"
          :class="{ 'is-active': tool === option.tool }"
          :aria-checked="tool === option.tool"
          :data-testid="`msp-tool-${option.tool}`"
          @click="tool = option.tool"
        >{{ option.label }}</button>
      </div>
    </div>

    <div class="msp__source" role="radiogroup" :aria-label="t('settings.materialSource.title')">
      <label
        v-for="option in sourceOptions"
        :key="option.kind"
        class="msp__option"
        :class="{ 'is-active': option.kind === source.kind }"
      >
        <input
          v-model="kind"
          type="radio"
          class="msp__radio"
          :name="groupName"
          :value="option.kind"
          :data-testid="`msp-source-${option.kind}`"
        >
        <span class="msp__option-text">{{ option.label }}</span>
      </label>
    </div>

    <label v-if="activeSource" class="msp__field">
      <span class="msp__label">{{ activeSource.label }}</span>
      <input
        v-model="sourceValue"
        type="text"
        class="control-input msp__input"
        :class="{ 'msp__input--invalid': sourceErrorSuffix !== null }"
        :placeholder="activeSource.placeholder"
        :aria-invalid="sourceErrorSuffix !== null"
        :data-testid="`msp-source-value-${source.kind}`"
        @keydown.enter.prevent="launch"
      >
    </label>
    <p v-if="sourceErrorText" class="msp__error" role="alert" data-testid="msp-source-error">{{ sourceErrorText }}</p>

    <div class="msp__params">
      <label v-for="def in paramFields" :key="def.key" class="msp__field">
        <span class="msp__label" :class="{ 'msp__label--required': def.required }">{{ fieldText(def.labelKey) }}</span>
        <input
          :value="values[def.key] ?? ''"
          type="text"
          class="control-input msp__input"
          :class="{ 'msp__input--invalid': invalidKeys.includes(def.key) }"
          :inputmode="inputModeFor(def.type)"
          :placeholder="def.placeholderKey ? fieldText(def.placeholderKey) : ''"
          :aria-invalid="invalidKeys.includes(def.key)"
          :data-testid="`msp-field-${def.key}`"
          @input="setValue(def.key, $event)"
        >
      </label>
    </div>

    <p v-if="blockingText" class="msp__error" role="alert" data-testid="msp-blocking">{{ blockingText }}</p>

    <footer class="msp__footer">
      <div v-if="preview" class="msp__preview" data-testid="msp-preview">
        <span class="msp__label">{{ t('settings.multimodalGen.previewLabel') }}</span>
        <code class="msp__preview-value">{{ preview }}</code>
      </div>
      <button
        type="button"
        class="btn btn--primary msp__submit"
        :disabled="blocking !== null"
        data-testid="msp-launch"
        @click="launch"
      >{{ t('settings.multimodalGen.submitLabel') }}</button>
    </footer>
  </section>
</template>

<script setup lang="ts">
/**
 * 素材源四选一 + 生成参数面。本组件**只做前端状态与校验**，不发任何 RPC：
 * 网关没有 `tools.invoke` 之类的通用「直接跑一个工具」通道（`tools.catalog` 只读、
 * `tools.web_*` 是三个固定端点），所以真实生成只能由宿主把 `launch` 的载荷
 * 组织成一句话走既有的 `chat.send` 交给主智能体 —— 载荷里同时给了
 * `toolName`（Rust 真实工具名）与 `args`（后端真吃的键），宿主不需要再猜。
 *
 * 校验全走两个纯模块（`materialSource` / `generateParams`），组件里不写第二套规则。
 */
import { computed, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  MATERIAL_SOURCE_KINDS,
  emptyMaterialSource,
  materialSourceAcceptsValue,
  materialSourceErrorKey,
  normalizeMaterialSource,
  type MaterialSource,
  type MaterialSourceKind,
} from './materialSource'
import {
  GENERATE_TOOL_NAMES,
  MATERIAL_GEN_NAMESPACE,
  MATERIAL_SOURCE_NAMESPACE,
  argsPreview,
  buildToolArgs,
  fieldsForTool,
  launchBlocking,
  numberFieldIsValid,
  type GenerateTool,
  type ParamInputType,
} from './generateParams'

const props = withDefaults(defineProps<{
  /** 初始产出类型；切换由组件内的分段控件负责。 */
  tool?: GenerateTool
  /** 初始素材源（通常是持久化恢复的 `{ kind, value }`）。 */
  initialSource?: MaterialSource
  /** 初始参数面取值（键 = 后端真实参数名）。 */
  initialValues?: Record<string, string>
}>(), {
  tool: 'image',
  initialSource: () => emptyMaterialSource(),
  initialValues: () => ({}),
})

const emit = defineEmits<{
  launch: [{
    tool: GenerateTool
    /** Rust 注册表里的真实工具名（image_generate / video_generate）。 */
    toolName: string
    source: MaterialSource
    args: Record<string, string>
    preview: string
  }]
}>()

const { t } = useI18n()
const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '')
const headingId = `msp-title-${uid}`
const groupName = `msp-source-${uid}`

const tool = ref<GenerateTool>(props.tool)
const kind = ref<MaterialSourceKind>(props.initialSource.kind)
const sourceValue = ref(props.initialSource.value)
const values = ref<Record<string, string>>({ ...props.initialValues })

const toolOptions = computed(() => [
  { tool: 'image' as GenerateTool, toolName: GENERATE_TOOL_NAMES.image, label: t('settings.multimodalGen.toolImage') },
  { tool: 'video' as GenerateTool, toolName: GENERATE_TOOL_NAMES.video, label: t('settings.multimodalGen.toolVideo') },
])

/**
 * 四个选项的文案写成**字面量** `t()`（`check-i18n` 能静态核对），
 * 但**渲染顺序仍然来自** `MATERIAL_SOURCE_KINDS` —— 不在组件里另立一份 A→D 口径。
 */
const sourceText = computed(() => ({
  web: { label: t('settings.materialSource.optionWeb'), placeholder: t('settings.materialSource.webPlaceholder') },
  generate: { label: t('settings.materialSource.optionGenerate'), placeholder: '' },
  directory: { label: t('settings.materialSource.optionDirectory'), placeholder: t('settings.materialSource.directoryPlaceholder') },
  custom: { label: t('settings.materialSource.optionCustom'), placeholder: t('settings.materialSource.customPlaceholder') },
}))

const sourceOptions = computed(() => MATERIAL_SOURCE_KINDS.map(option => ({
  kind: option,
  ...sourceText.value[option],
  acceptsValue: materialSourceAcceptsValue(option),
})))

/**
 * 切换选项即清空输入 —— 「不残留上一选项的取值」是 **UI 层**的责任
 * （`normalizeMaterialSource` 只无条件清 B，A 的选填 URL 必须留着，
 * 否则视频首帧联动就恒不生效）。初始值来自 props，watcher 只在变更后触发，
 * 所以持久化恢复不会被抹掉。
 */
watch(kind, () => {
  sourceValue.value = ''
})

const activeSource = computed(() => {
  const matched = sourceOptions.value.find(option => option.kind === kind.value)
  return matched && matched.acceptsValue ? matched : null
})

const source = computed(() => normalizeMaterialSource(kind.value, sourceValue.value))
const sourceErrorSuffix = computed(() => materialSourceErrorKey(source.value))
const sourceErrorText = computed(() => {
  const suffix = sourceErrorSuffix.value
  return suffix === null ? '' : t(`settings.${MATERIAL_SOURCE_NAMESPACE}.${suffix}`)
})

const paramFields = computed(() => fieldsForTool(tool.value))
const invalidKeys = computed(() => paramFields.value
  .filter(def => !numberFieldIsValid(def, values.value[def.key] ?? ''))
  .map(def => def.key))

const blocking = computed(() => launchBlocking(tool.value, values.value, source.value))
/** 素材源的缺口已有紧邻的专用提示，这里只报「提示词/数值」两类，避免同屏重复。 */
const blockingText = computed(() => {
  const reason = blocking.value
  return reason === 'needPrompt' || reason === 'invalidNumber'
    ? t(`settings.${MATERIAL_GEN_NAMESPACE}.${reason}`)
    : ''
})

const args = computed(() => buildToolArgs(tool.value, values.value, source.value))
const preview = computed(() => argsPreview(args.value))

function fieldText(suffix: string): string {
  return t(`settings.${MATERIAL_GEN_NAMESPACE}.${suffix}`)
}

function inputModeFor(type: ParamInputType): 'text' | 'numeric' | 'decimal' {
  if (type === 'integer') return 'numeric'
  return type === 'number' ? 'decimal' : 'text'
}

/**
 * 数值字段刻意用 `type="text"` + `inputmode`：后端的参数表本来就是
 * `HashMap<String, String>`，`numberFieldIsValid` 校验的是**字符串**；
 * 换成原生 number 输入会让非法文本变成空值，闸门与单测的口径就分叉了。
 */
function setValue(key: string, event: Event): void {
  const target = event.target as HTMLInputElement
  values.value[key] = target.value
}

function launch(): void {
  if (blocking.value !== null) return
  emit('launch', {
    tool: tool.value,
    toolName: GENERATE_TOOL_NAMES[tool.value],
    source: source.value,
    args: args.value,
    preview: preview.value,
  })
}

defineExpose({ launch, blocking, source, args })
</script>

<style scoped>
.msp {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  background: var(--bg-surface);
  color: var(--text);
}

.msp__title {
  margin: 0;
  font-size: 0.95rem;
  font-weight: 600;
  letter-spacing: .01em;
}

.msp__row,
.msp__footer {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.msp__label {
  font-size: .82rem;
  color: var(--text-muted);
}

.msp__label--required::after {
  content: "*";
  margin-inline-start: 3px;
  color: var(--danger);
}

.msp__segment {
  display: inline-flex;
  padding: 2px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--bg) 55%, transparent);
}

.msp__segment-btn {
  padding: 4px 12px;
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  font-size: .82rem;
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease-standard), color var(--dur-fast) var(--ease-standard);
}

.msp__segment-btn.is-active {
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--text);
}

.msp__source {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 8px;
}

.msp__option {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  cursor: pointer;
  transition: border-color var(--dur-fast) var(--ease-standard), background var(--dur-fast) var(--ease-standard);
}

.msp__option.is-active {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}

.msp__option-text {
  font-size: .84rem;
}

.msp__radio {
  accent-color: var(--accent);
}

.msp__field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.msp__params {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 10px;
}

.msp__input--invalid {
  border-color: var(--danger);
}

.msp__error {
  margin: 0;
  font-size: .8rem;
  color: var(--danger);
}

.msp__preview {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.msp__preview-value {
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg) 60%, transparent);
  color: var(--text-muted);
  font-size: .78rem;
  overflow-wrap: anywhere;
}

@media (prefers-reduced-motion: reduce) {
  .msp__segment-btn,
  .msp__option {
    transition: none;
  }
}
</style>
