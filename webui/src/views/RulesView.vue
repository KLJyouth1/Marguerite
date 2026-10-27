<template>
  <div class="control-stage rules-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('rules.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('rules.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <button class="btn btn--ghost" :disabled="busy" @click="load">
          <Icon name="refresh" :size="16" /><span>{{ t('rules.refresh') }}</span>
        </button>
        <button class="btn btn--ghost" :disabled="busy" @click="startNew">
          <Icon name="plus" :size="16" /><span>{{ t('rules.newRule') }}</span>
        </button>
      </div>
    </header>

    <p v-if="note" class="rules-note">{{ note }}</p>

    <div class="rules-grid">
      <section class="rules-card rules-list">
        <div class="rules-card__head">
          <span>{{ t('rules.list') }}</span>
          <span class="rules-dim">{{ t('rules.activeChars', { count: activeChars }) }}</span>
        </div>
        <p v-if="!rules.length" class="rules-note rules-note--inline">{{ t('rules.empty') }}</p>
        <ul v-else class="rules-list__items">
          <li v-for="r in rules" :key="r.id">
            <button
              type="button"
              class="rules-list__item"
              :class="{ 'is-active': r.id === selectedId }"
              @click="select(r.id)"
            >
              <span class="rules-list__name">{{ r.name }}</span>
              <span class="rules-list__meta">
                <span class="rules-badge">{{ r.scope === 'project' ? t('rules.scopeProject') : t('rules.scopePersonal') }}</span>
                <span v-if="!r.enabled" class="rules-badge rules-badge--muted">{{ t('rules.disabled') }}</span>
                <span class="rules-dim">{{ t('rules.chars', { count: r.chars }) }}</span>
              </span>
              <span v-if="r.error" class="rules-error">{{ r.error }}</span>
            </button>
          </li>
        </ul>
      </section>

      <section class="rules-card rules-editor">
        <template v-if="draft">
          <div class="rules-field">
            <label class="rules-label" for="rule-name">{{ t('rules.name') }}</label>
            <input id="rule-name" v-model="draft.name" class="rules-input" :placeholder="t('rules.namePlaceholder')" />
          </div>

          <div class="rules-row">
            <div class="rules-field">
              <label class="rules-label" for="rule-scope">{{ t('rules.scope') }}</label>
              <select id="rule-scope" v-model="draft.scope" class="rules-input">
                <option value="personal">{{ t('rules.scopePersonal') }}</option>
                <option value="project">{{ t('rules.scopeProject') }}</option>
              </select>
            </div>
            <label class="rules-check">
              <input v-model="draft.enabled" type="checkbox" />
              <span>{{ t('rules.enabled') }}</span>
            </label>
          </div>

          <div class="rules-field rules-field--grow">
            <label class="rules-label" for="rule-content">{{ t('rules.content') }}</label>
            <textarea
              id="rule-content"
              v-model="draft.content"
              class="rules-textarea"
              spellcheck="false"
              :placeholder="t('rules.contentPlaceholder')"
            />
            <div class="rules-dim">{{ t('rules.chars', { count: draft.content.length }) }}</div>
          </div>

          <div class="rules-actions">
            <button class="btn btn--primary" :disabled="saving || !draft.name.trim()" @click="save">
              <Icon name="save" :size="15" /><span>{{ saving ? t('rules.saving') : t('rules.save') }}</span>
            </button>
            <button v-if="selectedId" class="btn btn--ghost rules-danger" :disabled="saving" @click="remove">
              <Icon name="trash" :size="15" /><span>{{ t('rules.delete') }}</span>
            </button>
            <span v-if="savedAt" class="rules-dim">{{ t('rules.saved') }}</span>
          </div>
        </template>
        <p v-else class="rules-note rules-note--inline">{{ t('rules.selectHint') }}</p>
      </section>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 用户规则编辑器（规则 = 每轮都注入的硬约束）。
 *
 * 数据全部走真实 RPC（rules.list/get/save/delete，均 Rust-first）：
 * 文件落在 `<config_dir>/rules/*.md`，正文即规则；保存后下一轮对话即生效
 * （`agent/runtime.rs` 在静态段注入，缓存 2 秒）。
 *
 * 诚实边界：删除不可恢复，前端二次确认；保存失败就地显示错误，不假装成功。
 */
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

interface RuleEntry {
  id: string
  name: string
  scope: string
  enabled: boolean
  chars: number
  updatedAt?: string
  error?: string | null
}
interface Draft {
  id?: string
  name: string
  scope: 'personal' | 'project'
  enabled: boolean
  content: string
}

const { t } = useI18n()
const rpc = useRpcStore()

const rules = ref<RuleEntry[]>([])
const activeChars = ref(0)
const selectedId = ref('')
const draft = ref<Draft | null>(null)
const busy = ref(false)
const saving = ref(false)
const note = ref('')
const savedAt = ref(false)

const selected = computed(() => rules.value.find((r) => r.id === selectedId.value) ?? null)

async function load() {
  busy.value = true
  note.value = ''
  try {
    if (!rpc.isConnected) await rpc.waitForConnection(8_000)
    const res = await rpc.call<{ ok?: boolean; rules?: RuleEntry[]; activeChars?: number; error?: string }>(
      'rules.list',
      {},
    )
    if (res.ok === false) {
      note.value = res.error || t('rules.unavailable')
      return
    }
    rules.value = res.rules ?? []
    activeChars.value = res.activeChars ?? 0
    // 选中项仍在列表里就刷新正文，否则清空选择。
    if (selectedId.value && rules.value.some((r) => r.id === selectedId.value)) {
      await select(selectedId.value)
    } else if (selectedId.value) {
      selectedId.value = ''
      draft.value = null
    }
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function select(id: string) {
  selectedId.value = id
  savedAt.value = false
  try {
    const res = await rpc.call<{ ok?: boolean; rule?: RuleEntry; content?: string; error?: string }>(
      'rules.get',
      { id },
    )
    if (res.ok === false || !res.rule) {
      note.value = res.error || t('rules.unavailable')
      return
    }
    draft.value = {
      id: res.rule.id,
      name: res.rule.name,
      scope: res.rule.scope === 'project' ? 'project' : 'personal',
      enabled: res.rule.enabled,
      content: res.content ?? '',
    }
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  }
}

function startNew() {
  selectedId.value = ''
  savedAt.value = false
  draft.value = { name: '', scope: 'personal', enabled: true, content: '' }
}

async function save() {
  const d = draft.value
  if (!d || !d.name.trim()) return
  saving.value = true
  note.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; rule?: RuleEntry; error?: string }>('rules.save', {
      id: d.id,
      name: d.name.trim(),
      scope: d.scope,
      enabled: d.enabled,
      content: d.content,
    })
    if (res.ok === false || !res.rule) {
      note.value = res.error || t('rules.saveFailed')
      return
    }
    savedAt.value = true
    selectedId.value = res.rule.id
    await load()
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

async function remove() {
  if (!selectedId.value) return
  if (!window.confirm(t('rules.deleteConfirm', { name: selected.value?.name ?? selectedId.value }))) return
  saving.value = true
  note.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; error?: string }>('rules.delete', { id: selectedId.value })
    if (res.ok === false) {
      note.value = res.error || t('rules.saveFailed')
      return
    }
    selectedId.value = ''
    draft.value = null
    await load()
  } catch (e) {
    note.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.rules-grid {
  display: grid;
  grid-template-columns: minmax(220px, 320px) minmax(0, 1fr);
  gap: 16px;
  align-items: start;
}
@media (max-width: 900px) {
  .rules-grid {
    grid-template-columns: 1fr;
  }
}
.rules-card {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  padding: 12px;
}
.rules-card__head {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border);
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 700;
}
.rules-list__items {
  margin: 0;
  padding: 0;
  list-style: none;
}
.rules-list__item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 100%;
  padding: 8px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.rules-list__item.is-active,
.rules-list__item:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.rules-list__name {
  font-size: var(--fs-sm);
  font-weight: 600;
}
.rules-list__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
}
.rules-badge {
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.rules-badge--muted {
  opacity: 0.7;
}
.rules-dim {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.rules-error {
  color: var(--danger);
  font-size: var(--fs-xs);
}
.rules-note {
  margin: 0 0 12px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.rules-note--inline {
  border: 0;
  padding: 8px 2px;
  margin: 0;
}
.rules-editor {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 420px;
}
.rules-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.rules-field--grow {
  flex: 1;
  min-height: 0;
}
.rules-row {
  display: flex;
  gap: 12px;
  align-items: flex-end;
  flex-wrap: wrap;
}
.rules-label {
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-weight: 700;
}
.rules-input {
  padding: 7px 9px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text);
  font: inherit;
  font-size: var(--fs-sm);
}
.rules-check {
  display: flex;
  gap: 6px;
  align-items: center;
  color: var(--text-muted);
  font-size: var(--fs-sm);
}
.rules-textarea {
  flex: 1;
  min-height: 300px;
  padding: 10px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  line-height: 1.6;
  resize: vertical;
}
.rules-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}
.rules-danger {
  color: var(--danger);
}
</style>
