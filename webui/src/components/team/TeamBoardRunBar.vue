<template>
  <div class="team-run-bar" data-testid="team-run-bar">
    <span
      v-if="run.form"
      class="team-run-bar__form"
      data-testid="team-run-bar-form"
      :data-form="run.form"
    >{{ formLabel }}</span>

    <span
      v-if="run.runId"
      class="team-run-bar__run"
      :title="run.runId"
      data-testid="team-run-bar-run-id"
    >#{{ runIdTail }}</span>

    <span class="team-run-bar__count" data-testid="team-run-bar-members">
      {{ t('chat.orchestration.memberCount', { n: memberCount }) }}
    </span>

    <!-- 缓冲溢出如实说出来：看板只投影了前 memberLimit 名，剩下的后端已经回传计数。 -->
    <span
      v-if="run.dropped > 0"
      class="team-run-bar__warn"
      data-testid="team-run-bar-dropped"
      :title="limitTitle"
    >{{ t('chat.orchestration.runFilesMore', { n: run.dropped }) }}</span>

    <!-- 内存投影空、回落磁盘快照 = 看的是上一次运行，不是此刻在跑 -->
    <span
      v-if="run.fromDisk"
      class="team-run-bar__disk"
      data-testid="team-run-bar-from-disk"
    >{{ t('cronSkills.list.metaLastRun') }}</span>

    <span
      v-if="updatedText"
      class="team-run-bar__updated"
      data-testid="team-run-bar-updated"
    >{{ t('team.board.lastActivity') }} <b>{{ updatedText }}</b></span>

    <span v-if="run.note" class="team-run-bar__note" data-testid="team-run-bar-note">{{ run.note }}</span>
  </div>
</template>

<script setup lang="ts">
/**
 * 团队看板的运行条 —— 「这几张卡属于哪一次编排」以及「这份数据现在有多可信」。
 *
 * 全部字段来自 `team.members` **既有**响应（`member_registry::render` / `snapshot`）：
 *   runId / form / dropped / memberLimit / fromDisk / note / updatedAt。
 * 此前 useTeamBoard 把它们整段丢掉，于是：
 *   · 蜂群超出投影容量时被丢弃的成员**完全不可见**（看板看着像「成员就这么几个」）；
 *   · daemon 重启后回落磁盘的**上一次**运行被当成实时状态展示。
 * 两者都属于「指标没有定义清楚就渲染」，这里把它们显式化。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRelativeNow } from '@/composables/useRelativeNow'
import { localizedRelativeTime } from '@/utils/messageTime'
import type { TeamRunInfo } from '@/composables/useTeamBoard'

const props = defineProps<{
  run: TeamRunInfo
  /** 真实成员卡数（不含后端合成的 lead 行），所以「N 成员」的分子定义是明确的。 */
  memberCount: number
  /** `team.members` 的 `updatedAt`（epoch ms）；null = 后端没给时间戳，如实留空。 */
  updatedAt: number | null
}>()

const { t, locale } = useI18n()
const now = useRelativeNow()

/** 后端 `form` 的已知取值复用编排面板既有文案；未知取值原样显示，不猜形态。 */
const FORM_KEYS: Record<string, string> = {
  team: 'chat.orchestration.formTeam',
  swarm: 'chat.orchestration.formSwarm',
  expert: 'chat.orchestration.formExpert',
  subagent: 'chat.orchestration.formSubagent',
  solo: 'chat.orchestration.formSingle',
}

const formLabel = computed(() => {
  const key = FORM_KEYS[props.run.form]
  return key ? t(key) : props.run.form
})

const runIdTail = computed(() => props.run.runId.slice(-8))

const limitTitle = computed(() =>
  props.run.memberLimit === null
    ? ''
    : `${props.memberCount}/${props.run.memberLimit}`,
)

const updatedText = computed(() => {
  if (props.updatedAt === null) return ''
  return localizedRelativeTime(props.updatedAt, locale.value, now.value) || ''
})
</script>

<style scoped>
.team-run-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px 10px;
  margin-top: 14px;
  padding: 8px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
  font-size: 11px;
  color: var(--text-dim);
}

.team-run-bar__form {
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--accent) 12%, var(--card));
  color: var(--accent);
  font-weight: 600;
}

.team-run-bar__run {
  font-family: var(--font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 140px;
}

.team-run-bar__count {
  font-variant-numeric: tabular-nums;
  color: var(--text);
}

.team-run-bar__warn {
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--warn) 14%, var(--card));
  color: var(--warn);
  font-weight: 600;
}

.team-run-bar__disk {
  padding: 1px 7px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--info) 14%, var(--card));
  color: var(--info);
  font-weight: 600;
}

.team-run-bar__updated b {
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  color: var(--text);
}

.team-run-bar__note {
  flex: 1 1 100%;
  color: var(--text-dim);
  overflow-wrap: anywhere;
}
</style>
