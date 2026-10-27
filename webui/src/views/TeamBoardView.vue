<template>
  <div class="control-stage team-board">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('team.board.title') }}</h1>
      </div>
      <div class="control-stage__actions">
        <button
          class="btn btn--ghost"
          :disabled="!supported || loading"
          @click="refresh"
        >
          <Icon name="refresh" :size="16" /><span>{{ t('team.board.refresh') }}</span>
        </button>
      </div>
    </header>

    <!-- 不可读原因就地展示：门控关闭 / 读取失败都不静默，但都不编造成员 -->
    <div v-if="note" class="team-board__note">{{ note }}</div>

    <section v-if="members.length" class="team-board__grid">
      <article
        v-for="member in members"
        :key="member.id"
        class="team-card"
        :data-member-id="member.id"
        :data-status="member.status"
      >
        <header class="team-card__head">
          <span
            class="team-card__avatar"
            :class="kindClass(member.kind)"
            aria-hidden="true"
          >{{ initialOf(member.label) }}</span>
          <div class="team-card__identity">
            <div class="team-card__name" :title="member.label">{{ member.label }}</div>
            <div v-if="member.role" class="team-card__role" :title="member.role">{{ member.role }}</div>
          </div>
          <span class="team-card__status" :class="statusClass(member.status)">
            {{ statusLabel(member.status) }}
          </span>
        </header>

        <span class="team-card__kind" :class="kindClass(member.kind)">{{ member.kind }}</span>

        <p
          class="team-card__step"
          :class="{ 'team-card__step--empty': !member.currentStep }"
          :title="member.currentStep || '—'"
        >{{ member.currentStep || '—' }}</p>

        <div class="team-card__stats">
          <span class="team-card__stat">
            <b>{{ formatCount(member.toolCalls) }}</b> {{ t('team.board.toolCalls') }}
          </span>
          <span class="team-card__stat">
            {{ t('usageLogs.metrics.tokens') }} <b>{{ formatCount(member.tokensUsed) }}</b>
          </span>
          <span class="team-card__stat">
            {{ t('team.board.lastActivity') }} <b>{{ relativeActivity(member.lastActivityMs) }}</b>
          </span>
        </div>
      </article>
    </section>

    <div v-else class="team-board__empty">{{ t('team.board.empty') }}</div>
  </div>
</template>

<script setup lang="ts">
/**
 * 团队看板 —— 每个成员一张独立卡片，回答「谁在做什么」。
 *
 * 数据只来自 `team.members`（见 useTeamBoard）：
 *   · 网关没声明该方法 → 诚实空态 + 说明，不发请求；
 *   · 返回空成员 → 空态，不编造占位成员；
 *   · 读取失败 / ok:false → 空态 + 可读原因，不保留上一轮陈旧成员。
 * 视图只做渲染与格式化，轮询、门控、事件订阅、清理都在 composable 里。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useTeamBoard } from '@/composables/useTeamBoard'
import { useRelativeNow } from '@/composables/useRelativeNow'
import { localizedRelativeTime } from '@/utils/messageTime'

const { t, locale } = useI18n()
const { members, supported, loading, error, refresh } = useTeamBoard()
const now = useRelativeNow()

const STATUS_KEYS = new Map<string, string>([
  ['running', 'team.board.status.running'],
  ['waiting', 'team.board.status.waiting'],
  ['finished', 'team.board.status.finished'],
  ['failed', 'team.board.status.failed'],
])

const KNOWN_KINDS = new Set(['lead', 'subagent', 'expert', 'swarm', 'team'])

const note = computed(() => {
  if (!supported.value) return '当前网关未提供 team.members，无法读取团队状态。'
  if (error.value) return `团队状态读取失败：${error.value}`
  return ''
})

function statusClass(status: string): string {
  return STATUS_KEYS.has(status) ? `team-status--${status}` : 'team-status--unknown'
}

function statusLabel(status: string): string {
  const key = STATUS_KEYS.get(status)
  return key ? t(key) : status
}

function kindClass(kind: string): string {
  return KNOWN_KINDS.has(kind) ? `team-kind--${kind}` : 'team-kind--unknown'
}

function initialOf(label: string): string {
  return label.trim().charAt(0).toUpperCase() || '?'
}

function formatCount(value: number | null): string {
  return value === null ? '—' : value.toLocaleString()
}

function relativeActivity(ms: number | null): string {
  if (ms === null) return '—'
  return localizedRelativeTime(ms, locale.value, now.value) || '—'
}
</script>

<style scoped>
.team-board {
  padding: 24px 28px;
  max-width: 1100px;
  margin: 0 auto;
}

.team-board__note {
  margin-top: 14px;
  padding: 8px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-dim);
}

.team-board__grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
  margin-top: 16px;
}

.team-board__empty {
  margin-top: 24px;
  padding: 28px;
  border: 1px dashed var(--hairline);
  border-radius: var(--radius-card);
  color: var(--text-dim);
  font-size: 13px;
  text-align: center;
}

.team-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
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

.team-card__head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.team-card__avatar {
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  width: 34px;
  height: 34px;
  border-radius: var(--radius-full);
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
.team-status--unknown { --status-ink: var(--text-dim); }

.team-card__kind {
  align-self: flex-start;
  padding: 1px 7px;
  border-radius: 999px;
  font-family: var(--font-mono);
  font-size: 10px;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  background: color-mix(in srgb, var(--kind-ink, var(--text-dim)) 10%, var(--card));
  color: var(--kind-ink, var(--text-dim));
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
