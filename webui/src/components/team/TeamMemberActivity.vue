<template>
  <dl class="member-activity" :data-member-id="member.id">
    <!-- 动向：最近一条真实观测。后端没发、或本轮还没跑到那一步，就写「未报告」，
         绝不拿 0 / 空串 / 猜测冒充「已经做过」。 -->
    <div class="member-activity__row" data-testid="member-thinking">
      <dt>{{ t('chat.orchestration.runMemberThinking') }}</dt>
      <dd
        class="member-activity__value"
        :class="{ 'member-activity__value--empty': !thinkingText }"
        :title="thinkingText || notReportedLabel"
      >{{ thinkingText || notReportedLabel }}</dd>
    </div>

    <div class="member-activity__row" data-testid="member-outcome">
      <dt>{{ t('chat.orchestration.runMemberOutcome') }}</dt>
      <dd
        v-if="outcomeText"
        class="member-activity__value member-activity__value--outcome"
        :class="{ 'member-activity__value--failed': outcomeFailed }"
        :title="outcomeText"
      >{{ outcomeText }}</dd>
      <dd
        v-else
        class="member-activity__value member-activity__value--empty"
      >{{ notReportedLabel }}</dd>
    </div>

    <div class="member-activity__row" data-testid="member-tool">
      <dt>{{ t('team.board.toolCalls') }}</dt>
      <dd
        v-if="lastToolName"
        class="member-activity__value member-activity__tool"
        :data-tool-state="lastToolState"
      >
        <span class="member-activity__tool-name" :title="lastToolName">{{ lastToolName }}</span>
        <span v-if="lastToolState !== 'pending'" class="member-activity__tool-mark" aria-hidden="true">{{ lastToolState === 'ok' ? '✓' : '✗' }}</span>
      </dd>
      <dd v-else class="member-activity__value member-activity__value--empty">
        {{ toolFallbackLabel }}
      </dd>
    </div>

    <div v-if="activity?.synthesizing" class="member-activity__note" data-testid="member-synthesizing">
      {{ t('chat.orchestration.runSynthesizing') }}
    </div>
    <div v-else-if="activity?.error" class="member-activity__note member-activity__note--error" data-testid="member-error">
      {{ activity.error }}
    </div>

    <!-- 时间线：只有后端/事件真有数据才出现数值 -->
    <div class="member-activity__row member-activity__row--time" data-testid="member-started">
      <dt>{{ t('harness.runs.started') }}</dt>
      <dd
        class="member-activity__value"
        :class="{ 'member-activity__value--empty': startedText === notReportedLabel }"
      >{{ startedText }}</dd>
    </div>
    <div class="member-activity__row member-activity__row--time" data-testid="member-duration">
      <dt>{{ t('harness.runs.duration') }}</dt>
      <dd
        class="member-activity__value"
        :class="{ 'member-activity__value--empty': durationText === notReportedLabel }"
      >{{ durationText }}</dd>
    </div>
  </dl>
</template>

<script setup lang="ts">
/**
 * 单个成员的动向与时间线 —— 团队看板卡片的内容区。
 *
 * 输入只有两样，都来自既有契约：
 *   · `member`：`team.members` 里那一行（快照投影）；
 *   · `activity`：`session.event.progress` 编排帧的前端投影（见 useTeamBoard）。
 *
 * 诚实纪律（铁律 R-10）：
 *   · `thinking` / `summary` / `elapsedMs` 这些字段后端**在事件流里发**、但
 *     `team.members` 的投影里没有 —— 只有真的收到帧才显示，否则「未报告」；
 *   · `toolCalls === 0` 是**真实计数**（后端从 member_started 起累加），
 *     不当成「没上报」；本组件只补充「最近一次工具名」这一帧里真有的信息；
 *   · 开始时间取前端**收到 member_started 的墙钟**，没收到过就不显示，
 *     不拿 `lastActivityMs` 反推一个假的起点。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { MemberLiveActivity, TeamMember } from '@/composables/useTeamBoard'

const props = defineProps<{
  member: TeamMember
  activity?: MemberLiveActivity | null
}>()

const { t, locale } = useI18n()

const notReportedLabel = computed(() => t('console.channels.notReported'))

const thinkingText = computed(() => props.activity?.thinking ?? '')

const outcomeText = computed(() => props.activity?.summary || '')
const outcomeFailed = computed(
  () => props.activity?.terminalStatus === 'failed'
    || props.activity?.timedOut === true
    || props.activity?.cancelled === true,
)

const lastToolName = computed(() => props.activity?.lastTool ?? '')
const lastToolState = computed<'ok' | 'error' | 'pending'>(() => {
  if (!lastToolName.value) return 'pending'
  const ok = props.activity?.lastToolOk
  if (ok === true) return 'ok'
  if (ok === false) return 'error'
  return 'pending'
})

/**
 * 「最近一次工具」没有帧时的补语。
 * 只有**从头观察到这个成员**（真的收到过它的 `member_started`）才谈得上
 * 「尚未调用工具」；中途打开看板、没观察到起点时不能替它下这个结论 ——
 * 那属于「没有观测」，写「未报告」。快照里的 `toolCalls` 计数仍如实显示在页脚。
 */
const toolFallbackLabel = computed(() => {
  const activity = props.activity
  if (activity && activity.startedAtMs !== null) return t('chat.orchestration.runMemberNoTools')
  return notReportedLabel.value
})

/** 只在真的观察到过 member_started 时才有开始时间。 */
const startedText = computed(() => {
  const ms = props.activity?.startedAtMs
  if (ms === null || ms === undefined) return notReportedLabel.value
  try {
    return new Date(ms).toLocaleTimeString(locale.value)
  } catch {
    return new Date(ms).toLocaleTimeString()
  }
})

/** 耗时取后端 `member_completed.elapsedMs`（真实测量值），没有就留空态。 */
const durationText = computed(() => {
  const ms = props.activity?.elapsedMs
  if (ms === null || ms === undefined || ms < 0) return notReportedLabel.value
  const totalSeconds = Math.round(ms / 1000)
  if (totalSeconds < 60) {
    return t('chat.activityDurationSeconds', { seconds: totalSeconds })
  }
  return t('chat.activityDurationMinutes', {
    minutes: Math.floor(totalSeconds / 60),
    seconds: totalSeconds % 60,
  })
})
</script>

<style scoped>
.member-activity {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin: 0;
  padding-top: 8px;
  border-top: 1px solid var(--hairline);
  font-size: 11px;
}

.member-activity__row {
  display: grid;
  grid-template-columns: 84px minmax(0, 1fr);
  align-items: baseline;
  gap: 8px;
}

.member-activity dt {
  color: var(--text-dim);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.member-activity dd {
  margin: 0;
  min-width: 0;
  color: var(--text);
  overflow-wrap: anywhere;
}

.member-activity__value--empty {
  color: var(--text-dim);
  font-style: italic;
}

.member-activity__value--outcome {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.member-activity__value--failed {
  color: var(--danger);
}

.member-activity__tool {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.member-activity__tool-name {
  font-family: var(--font-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.member-activity__tool-mark {
  flex: 0 0 auto;
  font-weight: 700;
}

.member-activity__tool[data-tool-state='ok'] .member-activity__tool-mark {
  color: var(--ok);
}

.member-activity__tool[data-tool-state='error'] .member-activity__tool-mark {
  color: var(--danger);
}

.member-activity__tool[data-tool-state='pending'] .member-activity__tool-name {
  color: var(--text-dim);
}

.member-activity__note {
  margin-top: 2px;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--info) 12%, var(--card));
  color: var(--text);
  font-size: 11px;
}

.member-activity__note--error {
  background: color-mix(in srgb, var(--danger) 12%, var(--card));
  color: var(--danger);
}

.member-activity__row--time dd {
  font-variant-numeric: tabular-nums;
}
</style>
