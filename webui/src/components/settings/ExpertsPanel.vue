<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import LoadingSpinner from '@/components/LoadingSpinner.vue'
import { useRpcStore } from '@/stores/rpc'

const METHOD = 'experts.list'

interface Expert {
  id: string
  label: string
  lineage: string
  mindset: string
  expertise: string[]
  skills: string[]
  playbook: string[]
}

const { t } = useI18n()
const rpc = useRpcStore()

const state = ref<'loading' | 'ready' | 'unsupported' | 'error'>('loading')
const experts = shallowRef<Expert[]>([])

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' ? value as Record<string, unknown> : {}
}
function textValue(value: unknown): string {
  return typeof value === 'string' ? value : ''
}
function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : []
}

function normalizeExpert(value: unknown): Expert | null {
  const data = objectValue(value)
  const id = textValue(data.id)
  if (!id) return null
  return {
    id,
    label: textValue(data.label) || id,
    lineage: textValue(data.lineage),
    mindset: textValue(data.mindset),
    expertise: stringList(data.expertise),
    skills: stringList(data.skills),
    playbook: stringList(data.playbook),
  }
}

async function load() {
  state.value = 'loading'
  try {
    await rpc.waitForConnection(8000)
    if (!rpc.supportsMethod(METHOD)) {
      state.value = 'unsupported'
      return
    }
    const result = objectValue(await rpc.call(METHOD))
    const list = Array.isArray(result.experts)
      ? result.experts.map(normalizeExpert).filter((e): e is Expert => e !== null)
      : []
    experts.value = list
    state.value = 'ready'
  } catch {
    state.value = 'error'
  }
}

onMounted(() => void load())
</script>

<template>
  <section class="experts-panel" data-testid="experts-panel">
    <header class="experts-panel__head">
      <h3 class="experts-panel__title">{{ t('settings.experts.title') }}</h3>
      <p class="experts-panel__description">{{ t('settings.experts.description') }}</p>
    </header>

    <div v-if="state === 'loading'" class="experts-panel__center">
      <LoadingSpinner />
    </div>
    <p v-else-if="state === 'unsupported'" class="experts-panel__notice" role="status">
      {{ t('settings.experts.unsupported') }}
    </p>
    <p v-else-if="state === 'error'" class="experts-panel__notice experts-panel__notice--error" role="alert">
      {{ t('settings.experts.loadFailed') }}
    </p>
    <ul v-else class="experts-panel__grid">
      <li v-for="expert in experts" :key="expert.id" class="experts-panel__card">
        <div class="experts-panel__card-head">
          <h4 class="experts-panel__card-title">{{ expert.label }}</h4>
          <span class="experts-panel__chip">{{ expert.skills.length }}</span>
        </div>
        <dl class="experts-panel__facts">
          <div>
            <dt>{{ t('settings.experts.lineage') }}</dt>
            <dd>{{ expert.lineage }}</dd>
          </div>
          <div>
            <dt>{{ t('settings.experts.mindset') }}</dt>
            <dd>{{ expert.mindset }}</dd>
          </div>
          <div v-if="expert.expertise.length">
            <dt>{{ t('settings.experts.expertise') }}</dt>
            <dd>{{ expert.expertise.join(' · ') }}</dd>
          </div>
          <div v-if="expert.skills.length">
            <dt>{{ t('settings.experts.skills') }}</dt>
            <dd class="experts-panel__tags">
              <span v-for="skill in expert.skills" :key="skill" class="experts-panel__tag">{{ skill }}</span>
            </dd>
          </div>
        </dl>
        <ol v-if="expert.playbook.length" class="experts-panel__playbook">
          <li v-for="(step, index) in expert.playbook" :key="index">
            <span class="experts-panel__step">{{ index + 1 }}</span>
            <span>{{ step }}</span>
          </li>
        </ol>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.experts-panel {
  display: flex;
  flex-direction: column;
  gap: var(--sp-5);
  margin: 0 auto;
  max-width: 880px;
  min-height: 100%;
}
.experts-panel__head {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.experts-panel__title {
  color: var(--text);
  font-size: var(--fs-lg);
  margin: 0;
}
.experts-panel__description,
.experts-panel__notice {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.55;
  margin: 0;
}
.experts-panel__notice--error {
  color: var(--danger);
}
.experts-panel__center {
  align-items: center;
  display: flex;
  justify-content: center;
  min-height: 160px;
}
.experts-panel__grid {
  display: grid;
  gap: var(--sp-4);
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  list-style: none;
  margin: 0;
  padding: 0;
}
.experts-panel__card {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4);
}
.experts-panel__card-head {
  align-items: center;
  display: flex;
  gap: var(--sp-2);
  justify-content: space-between;
}
.experts-panel__card-title {
  color: var(--text);
  font-size: var(--fs-md);
  margin: 0;
}
.experts-panel__chip {
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  border-radius: var(--radius-pill);
  color: var(--accent);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  padding: 2px var(--sp-2);
}
.experts-panel__facts {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  margin: 0;
}
.experts-panel__facts > div {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.experts-panel__facts dt {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.experts-panel__facts dd {
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.5;
  margin: 0;
}
.experts-panel__tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
}
.experts-panel__tag {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  padding: 2px var(--sp-2);
}
.experts-panel__playbook {
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  list-style: none;
  margin: 0;
  padding: var(--sp-3) 0 0;
}
.experts-panel__playbook li {
  align-items: flex-start;
  color: var(--text-muted);
  display: flex;
  font-size: var(--fs-sm);
  gap: var(--sp-2);
  line-height: 1.5;
}
.experts-panel__step {
  align-items: center;
  background: var(--bg-surface);
  border-radius: var(--radius-pill);
  color: var(--text-dim);
  display: inline-flex;
  flex: 0 0 auto;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  height: 20px;
  justify-content: center;
  margin-top: 1px;
  min-width: 20px;
}
</style>
