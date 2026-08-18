<template>
  <div class="control-stage plugins-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">插件与技能</h1>
        <p class="control-stage__subtitle">PluginSlot 统一注册槽 — 工具 / 技能 / MCP / 命令的统一发现、调用与创建</p>
      </div>
      <div class="control-stage__actions">
        <div class="plugins-search">
          <Icon name="search" :size="16" />
          <input v-model="search" type="search" placeholder="搜索插件…" />
        </div>
        <button class="btn btn--ghost" @click="load">
          <Icon name="refresh" :size="16" /><span>刷新</span>
        </button>
        <button class="btn btn--primary" @click="showCreate = !showCreate">
          <Icon name="plus" :size="16" /><span>创建技能</span>
        </button>
      </div>
    </header>

    <section v-if="showCreate" class="skill-create">
      <h2 class="skill-create__title">创建新技能</h2>
      <div class="skill-create__form">
        <label class="skill-field">
          <span>技能标识（英文）</span>
          <input v-model="form.name" class="skill-input" placeholder="如 my-skill" />
        </label>
        <label class="skill-field">
          <span>描述</span>
          <input v-model="form.description" class="skill-input" placeholder="技能的简短描述" />
        </label>
        <label class="skill-field">
          <span>触发词（逗号分隔，可选）</span>
          <input v-model="form.triggers" class="skill-input" placeholder="如 翻译,translation" />
        </label>
        <label class="skill-field skill-field--full">
          <span>Prompt 模板（可用 <code v-pre>{{text}}</code> 等变量）</span>
          <textarea v-model="form.prompt_template" class="skill-textarea" rows="5" placeholder="你是一个专业的翻译专家。请将以下文本翻译为中文：&#10;[[text]]"></textarea>
        </label>
        <div class="skill-create__actions">
          <button class="btn btn--ghost" @click="showCreate = false">取消</button>
          <button class="btn btn--primary" :disabled="creating" @click="createSkill">
            {{ creating ? '创建中…' : '保存技能' }}
          </button>
        </div>
        <div v-if="createResult" class="skill-create__result" :class="{ 'is-error': !createOk }">{{ createResult }}</div>
      </div>
    </section>

    <section v-if="error" class="plugins-error">{{ error }}</section>

    <section class="plugins-grid">
      <div v-for="p in filtered" :key="p.name" class="plugin-card">
        <div class="plugin-card__header">
          <span class="plugin-card__name">{{ p.name }}</span>
          <span class="plugin-card__maturity" :class="maturityClass(p.maturity)">{{ p.maturity }}</span>
        </div>
        <p class="plugin-card__desc">{{ p.description }}</p>
        <div class="plugin-card__meta">
          <span class="plugin-card__group">{{ p.group }}</span>
          <span class="plugin-card__tier">{{ p.tier }}</span>
        </div>
      </div>
    </section>
    <div v-if="!filtered.length && !error" class="plugins-empty">无匹配插件</div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, reactive } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

const rpc = useRpcStore()

interface Plugin { name: string; description: string; group: string; tier: string; maturity: string; }

const plugins = ref<Plugin[]>([])
const search = ref('')
const error = ref('')
const showCreate = ref(false)
const creating = ref(false)
const createResult = ref('')
const createOk = ref(false)
const form = reactive({ name: '', description: '', prompt_template: '', triggers: '' })

const filtered = computed(() => {
  const q = search.value.toLowerCase()
  if (!q) return plugins.value
  return plugins.value.filter(p => p.name.toLowerCase().includes(q) || p.description.toLowerCase().includes(q))
})

function maturityClass(m: string) {
  return { 'maturity--real': m === 'Real', 'maturity--partial': m === 'Partial', 'maturity--narrative': m === 'Narrative' }
}

async function load() {
  error.value = ''
  try {
    const res = await rpc.call<{ plugins: Plugin[] }>('plugins.list')
    plugins.value = res.plugins || []
  } catch (e) {
    error.value = `加载失败: ${e instanceof Error ? e.message : String(e)}`
  }
}

async function createSkill() {
  if (!form.name.trim() || !form.description.trim() || !form.prompt_template.trim()) {
    createResult.value = '请填写所有必填字段'
    createOk.value = false
    return
  }
  creating.value = true
  createResult.value = ''
  try {
    const res = await rpc.call<{ success: boolean; message: string }>('skills.create', {
      name: form.name.trim(),
      description: form.description.trim(),
      prompt_template: form.prompt_template,
      triggers: form.triggers.split(',').map(s => s.trim()).filter(Boolean),
    })
    createOk.value = res.success
    createResult.value = res.message
    if (res.success) {
      form.name = ''; form.description = ''; form.prompt_template = ''; form.triggers = ''
      showCreate.value = false
    }
  } catch (e) {
    createOk.value = false
    createResult.value = `创建失败: ${e instanceof Error ? e.message : String(e)}`
  } finally {
    creating.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.plugins-stage { padding: 24px 28px; max-width: 1200px; margin: 0 auto; }
.plugins-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.plugins-search { display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); }
.plugins-search input { border: none; background: transparent; color: var(--text); font-size: 12px; outline: none; width: 180px; }
.plugins-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 10px; margin-top: 20px; }
.plugin-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.plugin-card__header { display: flex; justify-content: space-between; align-items: center; }
.plugin-card__name { font-family: var(--font-mono, monospace); font-size: 12px; font-weight: 600; }
.plugin-card__maturity { font-size: 9px; font-weight: 700; padding: 2px 7px; border-radius: var(--radius-pill); }
.maturity--real { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.maturity--partial { background: color-mix(in srgb, var(--warn) 15%, transparent); color: var(--warn); }
.maturity--narrative { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }
.plugin-card__desc { font-size: 11px; color: var(--text-muted); margin: 6px 0; line-height: 1.5; }
.plugin-card__meta { display: flex; gap: 8px; font-size: 10px; color: var(--text-dim); }
.plugin-card__group { padding: 1px 6px; background: var(--bg-hover); border-radius: var(--radius-sm); }
.plugins-empty { text-align: center; padding: 40px; color: var(--text-dim); font-size: 13px; }
.skill-create { margin-top: 20px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px; }
.skill-create__title { font-size: 14px; font-weight: 600; margin: 0 0 12px; }
.skill-create__form { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
.skill-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--text-dim); }
.skill-field--full { grid-column: 1 / -1; }
.skill-input { padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; outline: none; }
.skill-input:focus { border-color: var(--accent); }
.skill-textarea { padding: 8px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; outline: none; resize: vertical; font-family: var(--font-mono, monospace); }
.skill-textarea:focus { border-color: var(--accent); }
.skill-create__actions { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
.skill-create__result { grid-column: 1 / -1; font-size: 12px; padding: 8px 12px; border-radius: var(--radius-sm); }
.skill-create__result:not(.is-error) { background: color-mix(in srgb, var(--ok) 10%, var(--card)); color: var(--ok); }
.skill-create__result.is-error { background: color-mix(in srgb, var(--danger) 10%, var(--card)); color: var(--danger); }
</style>
