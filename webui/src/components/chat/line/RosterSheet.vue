<template>
  <ContentSheet :open="open" :label="sheetLabel" @close="emit('close')">
    <template #header>
      <div class="cast__head">
        <strong>{{ stageTitle }}</strong>
        <span class="cast__kind">{{ kindHint }}</span>
      </div>
    </template>

    <div class="cast" data-testid="line-roster-sheet">
      <p v-if="!catalog.loaded" class="cast__notice">{{ t('chat.line.catalogUnavailable', { error: '—' }) }}</p>

      <template v-else-if="kind === 'teams'">
        <p class="cast__lead">{{ t('chat.line.castLeadTeams') }}</p>
        <ul class="cast__list">
          <li v-for="team in catalog.teams" :key="team.id">
            <label class="cast__item" :class="{ 'cast__item--on': selected(team.id) }">
              <input type="checkbox" :checked="selected(team.id)" @change="toggle(team.id)" />
              <span class="cast__name">{{ team.label }}</span>
              <code class="cast__id">{{ team.id }}</code>
              <span class="cast__roles">{{ team.roles.join(' · ') }}</span>
            </label>
          </li>
        </ul>
      </template>

      <template v-else-if="kind === 'roles'">
        <p class="cast__lead">{{ t('chat.line.castLeadRoles') }}</p>
        <ul class="cast__list">
          <li v-for="role in catalog.roles" :key="role.id">
            <label class="cast__item" :class="{ 'cast__item--on': selected(role.id) }">
              <input type="checkbox" :checked="selected(role.id)" @change="toggle(role.id)" />
              <span class="cast__name">{{ role.label }}</span>
              <code class="cast__id">{{ role.agentKey }}</code>
              <!-- 没匹配到内建人设时如实标出来：这是"这一棒按席位名跑"的真相，不是装饰。 -->
              <span v-if="role.personaFallback" class="cast__flag">
                {{ t('chat.line.personaFallback') }}
              </span>
            </label>
          </li>
        </ul>
      </template>

      <template v-else-if="stage && stage.pattern === 'swarm'">
        <p class="cast__lead">{{ t('chat.line.facetLead') }}</p>
        <ul class="cast__list cast__list--facets">
          <li v-for="facet in catalog.facets" :key="facet.id">
            <label class="cast__item" :class="{ 'cast__item--on': facetSelected(facet.id) }">
              <input type="checkbox" :checked="facetSelected(facet.id)" @change="toggleFacet(facet.id)" />
              <span class="cast__name">{{ facet.id }}</span>
              <span class="cast__roles">{{ facet.instruction }}</span>
            </label>
          </li>
        </ul>
        <p class="cast__notice">
          {{ t('chat.line.facetRotationNote', { width: stage.width }) }}
        </p>
      </template>

      <p v-else class="cast__notice">{{ t('chat.line.castNoneNotice') }}</p>

      <div class="cast__foot">
        <button type="button" class="cast__btn" :disabled="stage?.roster.length === 0" @click="clearCast">
          {{ t('chat.line.castClear') }}
        </button>
        <button type="button" class="cast__btn cast__btn--primary" @click="emit('close')">
          {{ t('chat.line.castDone') }}
        </button>
      </div>
    </div>
  </ContentSheet>
</template>

<script setup lang="ts">
// 名册面板（RosterSheet）—— 一格阶段卡背后的"由谁来做"。
//
// 面板内容**由后端目录的 `rosterKind` 决定**，不是由组件猜测：
// panel 只能选专家团 id，fanout/relay 只能选角色席位名，solo 干脆不给控件、只说明原因。
// 这条约束的存在理由：`selected_rosters` 只认团队 id，把角色名塞进去会解析不到，
// 而引擎的旧行为是静默降级成自动检测 —— 界面看起来"选了 4 个团"，实际跑的是猜出来的角色。
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import ContentSheet from '@/components/chat/ContentSheet.vue'
import { toggleRoster } from '@/composables/line/lineSpec'
import type { LineCatalog, LineStage, RosterKind } from '@/composables/line/types'

const props = defineProps<{
  open: boolean
  stage: LineStage | null
  catalog: LineCatalog
}>()

const emit = defineEmits<{
  close: []
  'update:stage': [LineStage]
}>()

const { t } = useI18n()

const kind = computed<RosterKind>(() => {
  if (!props.stage) return 'none'
  return props.catalog.patterns.find((p) => p.id === props.stage?.pattern)?.rosterKind ?? 'none'
})

const sheetLabel = computed(() => t('chat.line.castTitle'))

const stageTitle = computed(() => {
  if (!props.stage) return ''
  const label = props.catalog.patterns.find((p) => p.id === props.stage?.pattern)?.label ?? props.stage.pattern
  return `${props.stage.id} · ${label}`
})

const kindHint = computed(() => {
  if (kind.value === 'teams') return t('chat.line.rosterKindTeams')
  if (kind.value === 'roles') return t('chat.line.rosterKindRoles')
  return t('chat.line.rosterKindNone')
})

function selected(id: string): boolean {
  return props.stage?.roster.includes(id) === true
}

function facetSelected(id: string): boolean {
  return props.stage?.facets.includes(id) === true
}

function toggle(id: string): void {
  if (!props.stage) return
  emit('update:stage', toggleRoster(props.stage, id))
}

function toggleFacet(id: string): void {
  if (!props.stage) return
  const facets = props.stage.facets.includes(id)
    ? props.stage.facets.filter((f) => f !== id)
    : [...props.stage.facets, id]
  emit('update:stage', { ...props.stage, facets })
}

function clearCast(): void {
  if (!props.stage) return
  // 名册清空 ⇒ 宽度回落到 1：留着旧宽度就是"界面写 4 路、实际 0 个成员"的那类说谎。
  emit('update:stage', { ...props.stage, roster: [], width: 1 })
}
</script>

<style scoped>
.cast {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  overflow-y: auto;
  width: 100%;
  font-size: 12.5px;
  color: var(--text);
}

.cast__head {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.cast__kind {
  font-size: 11px;
  color: var(--text-dim);
}

.cast__lead {
  margin: 0;
  color: var(--text-dim);
}

.cast__list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.cast__item {
  display: grid;
  grid-template-columns: auto 1fr auto;
  grid-template-areas: 'check name id' '. roles roles';
  gap: 2px 8px;
  padding: 7px 9px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.cast__item--on {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 9%, var(--card));
}

.cast__item input {
  grid-area: check;
}

.cast__name {
  grid-area: name;
  font-weight: 600;
}

.cast__id {
  grid-area: id;
  font-size: 11px;
  color: var(--text-dim);
}

.cast__roles {
  grid-area: roles;
  font-size: 11px;
  color: var(--text-dim);
}

.cast__flag {
  grid-area: id;
  font-size: 10.5px;
  color: var(--warn);
}

.cast__notice {
  margin: 0;
  padding: 8px 10px;
  border: 1px dashed var(--hairline);
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  color: var(--text-dim);
}

.cast__foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: auto;
}

.cast__btn {
  border: 1px solid var(--hairline);
  border-radius: 999px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 12px;
  padding: 5px 12px;
}

.cast__btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.cast__btn--primary {
  border-color: color-mix(in srgb, var(--accent) 60%, var(--hairline));
  background: color-mix(in srgb, var(--accent) 16%, var(--card));
}
</style>
