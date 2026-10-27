<template>
  <article
    class="team-card"
    :data-member-id="member.id"
    :data-status="status"
    :data-live="hasLiveActivity ? 'true' : 'false'"
  >
    <header class="team-card__head">
      <span class="team-card__avatar" :class="kindClass" aria-hidden="true">{{ initialOf(member.label) }}</span>
      <div class="team-card__identity">
        <div class="team-card__name" :title="member.label">{{ member.label }}</div>
        <div v-if="member.role" class="team-card__role" :title="member.role">{{ member.role }}</div>
      </div>
      <span class="team-card__status" :class="statusClass">{{ statusLabel }}</span>
    </header>

    <!-- 所属团队：一行「形态 · #runId 尾巴」，比一个孤立的 id 有用得多。 -->
    <div v-if="runLabel" class="team-card__run" :title="run?.runId || runLabel" data-testid="member-run-label">
      {{ runLabel }}
    </div>

    <p
      class="team-card__step"
      :class="{ 'team-card__step--empty': !member.currentStep }"
      :title="member.currentStep || notReportedLabel"
    >{{ member.currentStep || notReportedLabel }}</p>

    <TeamMemberActivity :member="member" :activity="activity" />

    <div class="team-card__stats">
      <span class="team-card__stat">
        <b>{{ formatCount(member.toolCalls) }}</b> {{ t('team.board.toolCalls') }}
      </span>
      <span class="team-card__stat">
        {{ t('usageLogs.metrics.tokens') }} <b>{{ formatCount(member.tokensUsed) }}</b>
      </span>
      <span class="team-card__stat">
        {{ t('team.board.lastActivity') }} <b>{{ relativeActivityText }}</b>
      </span>
    </div>
  </article>
</template>

<script setup lang="ts">
/**
 * 团队看板的成员卡片：一个成员一张卡。
 *
 * 三个区回答三个问题，且每个字段都指得出来源：
 *   1. **是谁** —— label / role / kind（`team.members` 行）+ 所属编排（`run.form` + `run.runId`）；
 *   2. **什么状态** —— `resolveMemberStatus` 合并「快照 status」与「实时终态帧」；
 *   3. **在做什么** —— 当前步骤（快照）+ 动向/时间线（TeamMemberActivity，来自事件流）
 *      + 工具调用与 token（快照；token 后端目前恒为 null，如实显示「—」）。
 *
 * 本组件**只渲染**，不读 RPC、不订阅事件：数据形状全部由 useTeamBoard 归一化过。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import TeamMemberActivity from '@/components/team/TeamMemberActivity.vue'
import { useRelativeNow } from '@/composables/useRelativeNow'
import { localizedRelativeTime } from '@/utils/messageTime'
import type { MemberLiveActivity, TeamMember, TeamRunInfo } from '@/composables/useTeamBoard'

const props = defineProps<{
  member: TeamMember
  /** 合并后的状态（快照 + 实时帧），不是线缆原值 —— 见 resolveMemberStatus。 */
  status: string
  activity?: MemberLiveActivity | null
  run?: TeamRunInfo | null
}>()

const { t, locale } = useI18n()
const now = useRelativeNow()

const STATUS_KEYS = new Map<string, string>([
  ['running', 'team.board.status.running'],
  ['waiting', 'team.board.status.waiting'],
  ['finished', 'team.board.status.finished'],
  ['failed', 'team.board.status.failed'],
  // 'thinking' 是本看板从 member_thinking 帧还原出来的**前端状态**：
  // 后端没有这个 status 取值，所以它只会在真的收到思考帧时出现。
  ['thinking', 'chat.thinking'],
])

const KNOWN_KINDS = new Set(['lead', 'subagent', 'expert', 'swarm', 'team'])

const FORM_KEYS: Record<string, string> = {
  team: 'chat.orchestration.formTeam',
  swarm: 'chat.orchestration.formSwarm',
  expert: 'chat.orchestration.formExpert',
  subagent: 'chat.orchestration.formSubagent',
  solo: 'chat.orchestration.formSingle',
}

const notReportedLabel = computed(() => t('console.channels.notReported'))

const statusClass = computed(() =>
  STATUS_KEYS.has(props.status) ? `team-status--${props.status}` : 'team-status--unknown',
)

const statusLabel = computed(() => {
  const key = STATUS_KEYS.get(props.status)
  return key ? t(key) : props.status
})

const kindClass = computed(() =>
  KNOWN_KINDS.has(props.member.kind) ? `team-kind--${props.member.kind}` : 'team-kind--unknown',
)

const hasLiveActivity = computed(() => Boolean(props.activity))

const runLabel = computed(() => {
  const run = props.run
  if (!run) return ''
  const formKey = FORM_KEYS[run.form]
  const form = formKey ? t(formKey) : run.form
  const tail = run.runId ? `#${run.runId.slice(-8)}` : ''
  return [form, tail].filter(Boolean).join(' · ')
})

const relativeActivityText = computed(() => {
  const ms = props.member.lastActivityMs
  if (ms === null) return '—'
  return localizedRelativeTime(ms, locale.value, now.value) || '—'
})

function initialOf(label: string): string {
  return label.trim().charAt(0).toUpperCase() || '?'
}

function formatCount(value: number | null): string {
  return value === null ? '—' : value.toLocaleString()
}
</script>

<style scoped>
.team-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  min-width: 0;
  background: var(--card);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-card);
  transition:
    transform var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard),
    box-shadow var(--dur-fast) var(--ease-standard);
}

.team-card:hover {
  transform: translateY(-2px);
  border-color: color-mix(in srgb, var(--accent) 35%, var(--hairline));
  box-shadow: var(--elev-1);
}

.team-card[data-live='true'] {
  border-color: color-mix(in srgb, var(--accent) 22%, var(--hairline));
}

.team-card__head {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.team-card__avatar {
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  font-size: 13px;
  font-weight: 700;
  background: color-mix(in srgb, var(--kind-ink, var(--text-dim)) 20%, var(--card));
  color: var(--kind-ink, var(--text-dim));
}

.team-card__identity {
  min-width: 0;
  flex: 1;
}

.team-card__name {
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.team-card__role {
  font-size: 11px;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.team-card__status {
  flex: 0 0 auto;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 600;
  background: color-mix(in srgb, var(--status-ink, var(--text-dim)) 14%, var(--card));
  color: var(--status-ink, var(--text-dim));
}

.team-status--running { --status-ink: var(--accent); }
.team-status--waiting { --status-ink: var(--warn); }
.team-status--finished { --status-ink: var(--ok); }
.team-status--failed { --status-ink: var(--danger); }
.team-status--thinking { --status-ink: var(--info); }
.team-status--unknown { --status-ink: var(--text-dim); }

.team-card__run {
  align-self: flex-start;
  max-width: 100%;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid var(--hairline);
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.02em;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.team-kind--lead { --kind-ink: var(--accent); }
.team-kind--subagent { --kind-ink: var(--info); }
.team-kind--expert { --kind-ink: var(--ok); }
.team-kind--swarm { --kind-ink: var(--warn); }
.team-kind--team { --kind-ink: var(--text-dim); }
.team-kind--unknown { --kind-ink: var(--text-dim); }

.team-card__step {
  margin: 0;
  font-size: 12px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.team-card__step--empty {
  color: var(--text-dim);
}

.team-card__stats {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 12px;
  padding-top: 8px;
  border-top: 1px solid var(--hairline);
  font-size: 11px;
  color: var(--text-dim);
}

.team-card__stat b {
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  color: var(--text);
}

@media (prefers-reduced-motion: reduce) {
  .team-card {
    transition: none;
  }
  .team-card:hover {
    transform: none;
  }
}
</style>
