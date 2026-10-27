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

    <!-- 运行条：这几张卡属于哪一次编排 + 这份读数有多可信（dropped / fromDisk / note） -->
    <TeamBoardRunBar
      v-if="members.length"
      :run="run"
      :member-count="teamMemberCount"
      :updated-at="updatedAt"
    />

    <section v-if="members.length" class="team-board__grid">
      <TeamMemberCard
        v-for="member in members"
        :key="member.id"
        :member="member"
        :status="statusOf(member)"
        :activity="activity[member.id] ?? null"
        :run="run"
      />
    </section>

    <div v-else class="team-board__empty">
      {{ t('team.board.empty') }}
      <!-- 空态的原因由后端给出（`snapshot` 的 note：单选直答 / 尚未派活 / 重启后无落盘）。
           它比我们的泛化文案更准确，所以原文透出，不改写也不猜。 -->
      <div v-if="run.note" class="team-board__empty-note" data-testid="board-empty-note">{{ run.note }}</div>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 团队看板 —— 每个成员一张独立卡片，回答「谁在做什么」。
 *
 * 两条数据来源，都不是新契约：
 *   · `team.members`（快照）→ useTeamBoard 的成员行 + 运行元信息；
 *   · `session.event.progress` 的 `kind:"orchestration"` 帧（事件流）→ 成员动向
 *     （思考摘要 / 最近工具 / 耗时 / 结论 / 兜底 run_failed 收口）。
 * 为什么两条都要：`team.members` 只是进度帧的**内存投影**，投影里没有
 * `member_thinking`、没有 `elapsedMs`/`summary`，也没有网关直接 publish 的兜底
 * `run_failed` —— 只接 RPC 的话，这三类「动向」在看板上永远空白。
 *
 * 视图只做渲染与格式化：轮询、门控、事件订阅、清理、状态机合并都在 composable 里。
 * 诚实纪律（R-10）：网关没声明方法 / 返回空成员 / 读取失败 → 空态 + 说明，
 * 不编造成员、不保留陈旧成员；后端没给的字段一律显示「未报告」。
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import TeamMemberCard from '@/components/team/TeamMemberCard.vue'
import TeamBoardRunBar from '@/components/team/TeamBoardRunBar.vue'
import {
  isLeadRow,
  resolveMemberStatus,
  useTeamBoard,
  type TeamMember,
} from '@/composables/useTeamBoard'

const { t } = useI18n()
const { members, supported, loading, error, updatedAt, run, activity, refresh } = useTeamBoard()

const note = computed(() => {
  if (!supported.value) return '当前网关未提供 team.members，无法读取团队状态。'
  if (error.value) return `团队状态读取失败：${error.value}`
  return ''
})

/**
 * 卡片状态 = 快照 status 与实时帧的合并结果（唯一状态机在 useTeamBoard）。
 * 没有观测到帧的成员，显示的就是 `team.members` 的原值。
 */
function statusOf(member: TeamMember): string {
  return resolveMemberStatus(member.status, activity.value[member.id], updatedAt.value)
}

/**
 * 「N 成员」的分子定义：卡数减去后端合成的那一条 lead 行
 * （`member_registry::render` 恒定前置一条 `lead:<runId>`）。
 * 直接把卡数当成员数会把主智能体算成成员，指标定义就不诚实了。
 */
const teamMemberCount = computed(() => members.value.filter((member) => !isLeadRow(member)).length)
</script>

<style scoped>
.team-board {
  padding: 24px 28px;
  max-width: 1180px;
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
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
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

.team-board__empty-note {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-dim);
  overflow-wrap: anywhere;
}
</style>
