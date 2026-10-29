<!-- 编排执行卡（蜂群 / 团队 / 专家团 / 子智能体共用）。
     设计目标（用户要求）：一次编排 = 一张可点进去的卡片。
       1. 卡片头：形态、目标、整体状态、成员/调用计数、耗时；
       2. 分工视图：每个代理一张小卡（角色、任务、耗时、调用数、状态）；
       3. 点某个代理 → 进入该代理的执行详情：完整任务、逐次工具调用（参数 + 输出）、
          该代理的结论/产出；「返回分工」回到网格。
     工具调用与输出由 ToolInvocationCard 统一美化（按 JSON / 代码 / diff / 链接 / 文本换渲染）。
     没有这张卡时，用户只能看到一个 taskId，执行过程对用户是黑盒。 -->
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import ToolInvocationCard from './ToolInvocationCard.vue'
import { useChatTextRendering } from '@/composables/chat/useChatTextRendering'
import type { OrchestrationRun } from '@/composables/chat/useOrchestrationRuns'

const props = defineProps<{
  run: OrchestrationRun
  /** 用于「运行中 12s」的实时秒数（父级每秒更新）。 */
  now: number
}>()

const { t } = useI18n()
// 运行中默认展开（用户此刻最需要看到进度）；已结束默认收起，避免占满输入区。
const open = ref(props.run.status === 'running')
const openReply = ref(false)
/** 非空 = 已点进某个代理的详情（分工网格暂时让位）。 */
const selectedMemberId = ref<string | null>(null)

// 本次会话内「刚跑完」时自动展开结论：后台任务的结果不会作为流式消息回到
// 对话流（它写的是会话历史），如果卡片也收起，用户会误以为任务没有产出。
watch(
  () => props.run.status,
  (next, prev) => {
    if (prev === 'running' && next === 'completed') {
      open.value = true
      if (props.run.reply) openReply.value = true
    }
    if (prev === 'running' && next === 'failed') open.value = true
  },
)

// 换一轮编排时回到分工视图（否则会停留在上一轮某个成员上，看着像串台）。
watch(
  () => props.run.runId,
  () => {
    selectedMemberId.value = null
  },
)

const FORM_LABEL: Record<string, string> = {
  swarm: 'chat.orchestration.formSwarm',
  team: 'chat.orchestration.formTeam',
  expert: 'chat.orchestration.formExpert',
  subagent: 'chat.orchestration.formSubagent',
  single: 'chat.orchestration.formSingle',
}

const formLabel = computed(() => t(FORM_LABEL[props.run.form] ?? FORM_LABEL.swarm))

const elapsedSeconds = computed(() => {
  const end = props.run.endedAtMs ?? props.now
  return Math.max(0, Math.round((end - props.run.startedAtMs) / 1000))
})

/**
 * 失败成员数。
 *
 * 后端只在「整轮崩掉」时发 `run_failed`；个别成员超时/报错时仍发 `run_completed`，
 * 于是卡片头部会打出绿色「已完成」，而展开后成员行却明晃晃写着 `timeout` —— 自相矛盾。
 * 这不是文案问题，是**完成度谎报**：用户据此以为任务做成了（实测：前端工程师 120s 超时、
 * 一个产物都没生成，头部依然是「已完成」）。所以这里把成员的失败如实上抬到轮次徽标。
 */
const failedMembers = computed(() => props.run.members.filter((m) => m.status === 'error'))
/** 轮次结束但有成员失败 —— 既不是成功也不是整轮失败，必须单独说清楚。 */
const isPartial = computed(
  () => props.run.status === 'completed' && failedMembers.value.length > 0,
)

const statusText = computed(() => {
  if (props.run.status === 'failed') return t('chat.orchestration.runFailed')
  if (isPartial.value) {
    return t('chat.orchestration.runCompletedPartial', { n: failedMembers.value.length })
  }
  if (props.run.status === 'completed') return t('chat.orchestration.runCompleted')
  if (props.run.synthesizing) return t('chat.orchestration.runSynthesizing')
  return t('chat.orchestration.runRunning', { s: elapsedSeconds.value })
})

const statusIconName = computed<'check' | 'alertTriangle' | 'play'>(() => {
  if (props.run.status === 'failed' || isPartial.value) return 'alertTriangle'
  if (props.run.status === 'completed') return 'check'
  return 'play'
})

const memberCount = computed(() =>
  props.run.members.length || props.run.memberEstimate || 0,
)

const toolTotal = computed(() =>
  props.run.members.reduce((sum, m) => sum + m.tools.length, 0),
)

/* ── 产出文件（真实参数解析，零推测）────────────────────────────────
 * 只在这些工具上取路径：写入/编辑/补丁/产物创建。工具名用**精确集合**判定，
 * 不做模糊包含匹配（否则 `file.read` 会被误当成写文件）。
 * 路径从 `argsPreview` 里解析：优先 JSON，失败再用 `path: ...` 行兜底；
 * 解析不出就跳过——宁可不显示，也不给用户一个假路径。
 */
const FILE_WRITE_TOOLS = new Set([
  'file.write',
  'file.edit',
  'str_replace_editor',
  'apply_patch',
  'artifact.create',
  'write_file',
  'edit_file',
])

interface ProducedFile {
  path: string
  member: string
}

function pathFromToolArgs(preview: string): string {
  const raw = (preview || '').trim()
  if (!raw) return ''
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>
    for (const key of ['path', 'file_path', 'filePath', 'filename', 'target_file', 'file']) {
      const value = parsed[key]
      if (typeof value === 'string' && value.trim()) return value.trim()
    }
  } catch {
    // 非 JSON：走下面的行内 key: value 兜底
  }
  const match = raw.match(/"?file_?path"?\s*[:=]\s*"?([^"\n,}]+)/i)
  return match ? match[1].trim() : ''
}

const producedFiles = computed<ProducedFile[]>(() => {
  const byPath = new Map<string, ProducedFile>()
  for (const member of props.run.members) {
    for (const tool of member.tools) {
      if (!FILE_WRITE_TOOLS.has(tool.tool.toLowerCase())) continue
      const path = pathFromToolArgs(tool.argsPreview)
      if (!path) continue
      if (!byPath.has(path)) {
        byPath.set(path, { path, member: member.roleLabel || member.role || member.id })
      }
    }
  }
  return [...byPath.values()]
})
/** 最多列 12 条：再长就不是总结而是清单，用户要点开详情看。 */
const MAX_LISTED_FILES = 12
const visibleProducedFiles = computed(() => producedFiles.value.slice(0, MAX_LISTED_FILES))

// 分工进度：卡片头部用一条细进度条表达「几个代理已经跑完」。
// 没有它时，用户只能靠「运行中 42s」猜这轮到底推进到哪一步。
const doneMembers = computed(
  () => props.run.members.filter((m) => m.status !== 'running').length,
)
const memberTotal = computed(() => memberCount.value)
/** 骨架数量跟随预估成员数（2–6 个占位块），避免「写着 3 名成员却只画 2 个占位块」。 */
const skeletonCount = computed(() =>
  Math.min(6, Math.max(2, props.run.memberEstimate || props.run.members.length || 2)),
)
const progressPercent = computed(() => {
  if (props.run.status === 'completed') return 100
  if (!memberTotal.value) return props.run.status === 'failed' ? 100 : 0
  return Math.min(100, Math.round((doneMembers.value / memberTotal.value) * 100))
})
const progressLabel = computed(() =>
  t('chat.orchestration.runProgressLabel', {
    done: doneMembers.value,
    total: memberTotal.value,
  }),
)

/** 终态才显示总耗时：运行中已有「运行中 Ns」，重复展示会互相打架。 */
const elapsedText = computed(() =>
  props.run.endedAtMs ? `${elapsedSeconds.value}s` : '',
)

/** 供样式选择器使用（partial 要覆盖 completed 的绿色）。 */
const stateKey = computed(() => {
  if (props.run.status === 'failed') return 'failed'
  if (isPartial.value) return 'partial'
  if (props.run.status === 'completed') return 'ok'
  return 'running'
})

const selectedMember = computed(
  () => props.run.members.find((m) => m.id === selectedMemberId.value) ?? null,
)

// 代理结论与最终答复都是模型产出的 markdown 文本。
// 真机验收（2026-10-23，18899 网关）里它们被当成纯文本吐出：`## 结论`、`**加粗**`、
// 还有整张 GFM 表格的竖线全部原样显示 —— 这是「输出没美化」最扎眼的一处。
// 复用对话气泡同一套渲染管线（marked + DOMPurify + 高亮 + KaTeX）。
const { renderMarkdown } = useChatTextRendering()
const outcomeHtml = computed(() =>
  selectedMember.value?.summary ? renderMarkdown(selectedMember.value.summary) : '',
)
const replyHtml = computed(() => (props.run.reply ? renderMarkdown(props.run.reply) : ''))

/** 当前成员的真实思考轮次（旧快照没有该字段 → 空数组，不显示空块）。 */
const selectedMemberThinking = computed(() => selectedMember.value?.thinkingRounds ?? [])

function memberElapsed(ms: number | null, startedAt: number): string {
  if (ms !== null) return `${(ms / 1000).toFixed(1)}s`
  const live = Math.max(0, Math.round((props.now - startedAt) / 1000))
  return `${live}s`
}

/** 代理状态文案复用工具状态词（运行中 / 完成 / 失败），避免再造一套同义词。 */
function memberStateText(status: 'running' | 'ok' | 'error'): string {
  if (status === 'running') return t('chat.tool.running')
  if (status === 'error') return t('chat.tool.failed')
  return t('chat.tool.done')
}

function shortTask(text: string, max = 90): string {
  const clean = text.replace(/\s+/g, ' ').trim()
  return clean.length > max ? `${clean.slice(0, max)}…` : clean
}
</script>

<template>
  <section
    class="orch-run"
    :class="[`orch-run--${run.status}`, { 'orch-run--partial': isPartial }]"
    :data-state="stateKey"
    data-testid="orchestration-run"
    :aria-label="t('chat.orchestration.runTitle')"
  >
    <header class="orch-run__head">
      <div class="orch-run__headline">
        <span class="orch-run__badge">{{ formLabel }}</span>
        <span class="orch-run__status">
          <Icon :name="statusIconName" :size="11" aria-hidden="true" />
          <span>{{ statusText }}</span>
        </span>
        <span v-if="elapsedText" class="orch-run__elapsed">{{ elapsedText }}</span>
        <button
          type="button"
          class="orch-run__toggle"
          :aria-expanded="open"
          :title="open ? t('chat.orchestration.runCollapse') : t('chat.orchestration.runExpand')"
          @click="open = !open"
        >
          <Icon name="chevronRight" :size="13" :class="['orch-run__chev', { 'is-open': open }]" />
        </button>
      </div>

      <p class="orch-run__goal" :title="run.goal">{{ run.goal || '—' }}</p>

      <div v-if="memberTotal > 0" class="orch-run__meter">
        <div
          class="orch-run__track"
          role="progressbar"
          :aria-label="progressLabel"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-valuenow="progressPercent"
        >
          <span class="orch-run__fill" :style="{ width: progressPercent + '%' }" />
        </div>
        <span class="orch-run__ratio">{{ doneMembers }}/{{ memberTotal }}</span>
      </div>

      <p class="orch-run__meta">
        <span>{{ t('chat.orchestration.runMemberCount', { n: memberCount }) }}</span>
        <span v-if="toolTotal > 0">· {{ t('chat.orchestration.runToolCount', { n: toolTotal }) }}</span>
        <span v-if="failedMembers.length > 0" class="orch-run__meta-fail">
          · {{ t('chat.orchestration.runFailedCount', { n: failedMembers.length }) }}
        </span>
      </p>
    </header>

    <div v-if="open" class="orch-run__body">
      <!-- 代理详情（点进某个代理后） -->
      <section v-if="selectedMember" class="orch-detail" :data-testid="'orchestration-member-detail'">
        <header class="orch-detail__head">
          <button type="button" class="orch-detail__back" @click="selectedMemberId = null">
            <Icon name="chevronRight" :size="12" class="orch-detail__back-ic" aria-hidden="true" />
            {{ t('chat.orchestration.runBackToRoster') }}
          </button>
          <strong class="orch-detail__role">{{ selectedMember.roleLabel }}</strong>
          <span class="orch-detail__state" :data-state="selectedMember.status">
            {{ memberStateText(selectedMember.status) }}
          </span>
          <span class="orch-detail__time">
            {{ memberElapsed(selectedMember.elapsedMs, selectedMember.startedAtMs) }}
          </span>
        </header>

        <section class="orch-detail__block">
          <p class="orch-detail__label">{{ t('chat.orchestration.runMemberTask') }}</p>
          <p class="orch-detail__task">{{ selectedMember.task || '—' }}</p>
        </section>

        <!-- 成员思考：后端逐轮 `member_thinking` 帧，真实内容直出（不做摘要/改写）。
             用户反馈「子代理思考不显示」，此前成员详情只有任务与工具行。 -->
        <section v-if="selectedMemberThinking.length" class="orch-detail__block">
          <p class="orch-detail__label">
            {{ t('chat.orchestration.runMemberThinking') }}
            <span class="orch-detail__count">
              {{ t('chat.orchestration.runThinkingRounds', { n: selectedMemberThinking.length }) }}
            </span>
          </p>
          <ol class="orch-thinking">
            <li
              v-for="item in selectedMemberThinking"
              :key="item.round"
              class="orch-thinking__round"
            >
              <span class="orch-thinking__round-no">
                {{ t('chat.orchestration.runThinkingRound', { n: item.round }) }}
              </span>
              <pre class="orch-thinking__text">{{ item.text }}</pre>
            </li>
          </ol>
        </section>

        <section class="orch-detail__block">
          <p class="orch-detail__label">
            {{ t('chat.orchestration.runMemberActions') }}
            <span class="orch-detail__count">
              {{ t('chat.orchestration.runToolCount', { n: selectedMember.tools.length }) }}
            </span>
          </p>
          <div v-if="selectedMember.tools.length" class="orch-detail__tools">
            <ToolInvocationCard
              v-for="tool in selectedMember.tools"
              :key="tool.id"
              :tool="tool.tool"
              :args="tool.argsPreview"
              :output="tool.outputPreview"
              :status="tool.status"
              :elapsed-ms="tool.elapsedMs"
              :default-open="selectedMember.tools.length === 1"
            />
          </div>
          <p v-else class="orch-detail__empty">
            {{ t('chat.orchestration.runMemberNoTools') }}
          </p>
        </section>

        <section v-if="selectedMember.summary" class="orch-detail__block">
          <p class="orch-detail__label">{{ t('chat.orchestration.runMemberOutcome') }}</p>
          <!-- eslint-disable-next-line vue/no-v-html — renderMarkdown 输出经 DOMPurify 消毒 -->
          <div class="orch-detail__out msg-ai-text" v-html="outcomeHtml" />
        </section>
      </section>

      <!-- 分工网格（默认视图） -->
      <section v-else class="orch-roster" :data-testid="'orchestration-roster'">
        <template v-if="run.members.length">
          <div class="orch-roster__head">
            <p class="orch-roster__title">{{ t('chat.orchestration.runRoster') }}</p>
          </div>
          <div class="orch-roster__grid">
            <button
              v-for="m in run.members"
              :key="m.id"
              type="button"
              class="orch-agent"
              :class="[`orch-agent--${m.status}`]"
              :data-state="m.status"
              :data-testid="'orchestration-agent'"
              :aria-label="`${m.roleLabel} · ${memberStateText(m.status)}`"
              @click="selectedMemberId = m.id"
            >
              <span class="orch-agent__top">
                <span class="orch-agent__dot" aria-hidden="true" />
                <strong class="orch-agent__role">{{ m.roleLabel }}</strong>
                <span class="orch-agent__time">{{ memberElapsed(m.elapsedMs, m.startedAtMs) }}</span>
              </span>
              <span class="orch-agent__task" :title="m.task">{{ shortTask(m.task) || '—' }}</span>
              <span class="orch-agent__foot">
                <span class="orch-agent__calls">
                  {{ t('chat.orchestration.runToolCount', { n: m.tools.length }) }}
                </span>
                <span
                  v-if="(m.thinkingRounds?.length ?? 0) > 0"
                  class="orch-agent__calls"
                >
                  · {{ t('chat.orchestration.runThinkingRounds', { n: m.thinkingRounds!.length }) }}
                </span>
                <span class="orch-agent__state">{{ memberStateText(m.status) }}</span>
                <Icon name="chevronRight" :size="11" class="orch-agent__go" aria-hidden="true" />
              </span>
            </button>
          </div>
        </template>

        <!-- 成员尚未上报：给出「正在组建」的骨架，而不是一句像报错的空文案 -->
        <div v-else class="orch-roster__empty">
          <p class="orch-run__empty">
            <span v-if="run.status === 'running'" class="orch-run__pulse" aria-hidden="true" />
            {{ t('chat.orchestration.runNoMembers') }}
          </p>
          <div v-if="run.status === 'running'" class="orch-roster__grid" aria-hidden="true">
            <span v-for="n in skeletonCount" :key="n" class="orch-skeleton" />
          </div>
        </div>
      </section>

      <p v-if="run.synthesizing" class="orch-run__stage">
        <span class="orch-run__pulse" aria-hidden="true" />
        {{ t('chat.orchestration.runSynthesizing') }}
      </p>
    </div>

    <p v-if="run.error" class="orch-run__error">{{ run.error }}</p>

    <div v-if="run.reply" class="orch-run__reply">
      <button type="button" class="orch-run__reply-toggle" :aria-expanded="openReply" @click="openReply = !openReply">
        <Icon name="chevronRight" :size="12" :class="['orch-run__chev', { 'is-open': openReply }]" />
        {{ t('chat.orchestration.runReply') }}
      </button>
      <div v-if="openReply" class="orch-run__reply-body msg-ai-text" v-html="replyHtml" />
    </div>

    <!-- 产出文件：只认工具调用参数里**真实出现**的路径，不做任何推测。
         用户要求「会话任务结束怎么显示整理总结、文件增删改查与产出怎么显示」——
         蜂群/团队跑完后，成员各自写了哪些文件此前完全不可见。 -->
    <section v-if="producedFiles.length" class="orch-run__files">
      <div class="orch-run__files-title">
        {{ t('chat.orchestration.runFiles', { n: producedFiles.length }) }}
      </div>
      <ul class="orch-run__files-list">
        <li
          v-for="file in visibleProducedFiles"
          :key="file.path"
          class="orch-run__file"
          :title="`${file.path} · ${file.member}`"
        >
          <Icon name="fileText" :size="11" aria-hidden="true" />
          <span class="orch-run__file-path">{{ file.path }}</span>
          <span class="orch-run__file-member">{{ file.member }}</span>
        </li>
      </ul>
      <p v-if="producedFiles.length > visibleProducedFiles.length" class="orch-run__files-more">
        {{ t('chat.orchestration.runFilesMore', { n: producedFiles.length - visibleProducedFiles.length }) }}
      </p>
    </section>
  </section>
</template>

<style scoped>
.orch-run {
  /* 会话阅读列口径（见 chat-view.css 的 --chat-gutter / --chat-col-start）。
     它是 `.chat-orch-runs`（`.chat-thread` 弹性列的直接子块，chat-view.css 里的
     注释就写着「限制宽度跟随消息列」）里的子卡，过去 `margin: 0 auto` 会把每张
     运行卡推到屏幕几何中心，而它上下的正文是左起排的。
     宽度仍按 --composer-col 收一档（执行卡比正文窄是刻意的），但左边缘必须和
     正文同一起排点，多出来的空间全部留在右侧。
     窄屏由 --chat-gutter 的媒体查询退回 24px，这里不再硬编码。 */
  width: min(100%, var(--composer-col, 820px));
  max-width: var(--chat-col-start, var(--chat-col, min(calc(100% - 48px), 980px)));
  margin-block: 0 10px;
  margin-inline: var(--chat-gutter, 24px) auto;
  padding: 12px 14px;
  border: 1px solid color-mix(in srgb, var(--accent) 24%, var(--border));
  border-left: 2px solid color-mix(in srgb, var(--accent) 55%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent) 4%, var(--bg-surface));
  box-shadow: var(--shadow-sm);
  font-size: var(--fs-xs);
}
.orch-run--completed {
  border-color: color-mix(in srgb, var(--ok, var(--accent)) 28%, var(--border));
  border-left-color: var(--ok, var(--accent));
}
.orch-run--failed {
  border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  border-left-color: var(--danger);
}
/* 完成但有成员失败：边框/状态色走 warn，别再给成功绿 —— 绿会把「有产物」和
   「有成员超时、什么都没产出」混成同一个视觉信号。 */
.orch-run--partial {
  border-color: color-mix(in srgb, var(--warn) 44%, var(--border));
  border-left-color: var(--warn);
}

/* 卡头三行结构：身份/状态 → 目标 → 进度+计数。
   目标独占一行（长 goal 不再把状态挤成省略号），进度条让「跑到哪」一眼可见。 */
.orch-run__head { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.orch-run__headline { display: flex; align-items: center; gap: 8px; min-width: 0; }
.orch-run__badge {
  flex-shrink: 0;
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  color: var(--accent);
  font-size: 10.5px;
  letter-spacing: .04em;
}
.orch-run__goal {
  margin: 0;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  color: var(--text);
  font-size: var(--fs-sm);
  font-weight: 600;
  line-height: 1.45;
  word-break: break-word;
}
.orch-run__status {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
  font-size: 10.5px;
  white-space: nowrap;
}
.orch-run--completed .orch-run__status {
  background: color-mix(in srgb, var(--ok, var(--accent)) 14%, transparent);
  color: var(--ok, var(--accent));
}
.orch-run--failed .orch-run__status {
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  color: var(--danger);
}
.orch-run--partial .orch-run__status {
  background: color-mix(in srgb, var(--warn) 16%, transparent);
  color: var(--warn);
}
.orch-run__elapsed {
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.orch-run__toggle {
  margin-left: auto;
  flex-shrink: 0;
  padding: 2px;
  border: 0;
  background: none;
  color: var(--text-dim);
  cursor: pointer;
}
.orch-run__toggle:hover { color: var(--text-muted); }
.orch-run__chev { transition: transform var(--dur-fast) var(--ease-standard); }
.orch-run__chev.is-open { transform: rotate(90deg); }

.orch-run__meter { display: flex; align-items: center; gap: 8px; }
.orch-run__track {
  flex: 1;
  height: 4px;
  overflow: hidden;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text-dim) 22%, transparent);
}
.orch-run__fill {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
  transition: width var(--dur-fast) var(--ease-standard);
}
.orch-run--completed .orch-run__fill { background: var(--ok, var(--accent)); }
.orch-run--failed .orch-run__fill { background: var(--danger); }
.orch-run--partial .orch-run__fill { background: var(--warn); }
.orch-run__ratio {
  flex-shrink: 0;
  min-width: 30px;
  text-align: right;
  color: var(--text-muted);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}

.orch-run__meta {
  margin: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  color: var(--text-muted);
  font-size: 11px;
}
.orch-run__meta-fail { color: var(--warn); }

.orch-run__body { margin-top: 10px; display: flex; flex-direction: column; gap: 10px; }

/* ── 分工网格 ── */
.orch-roster__head { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
.orch-roster__title {
  margin: 0;
  /* 小字号必须用 muted：dim 在深色主题下只有约 4.2:1，低于 WCAG AA 的 4.5:1。 */
  color: var(--text-muted);
  font-size: 10px;
  letter-spacing: .06em;
  text-transform: uppercase;
}
.orch-roster__grid {
  display: grid;
  /* auto-fit：2 个成员时卡片撑满整行，不会在右侧留一整列空白。 */
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 8px;
}
.orch-agent {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
  padding: 9px 11px 8px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-surface-2) 55%, transparent);
  color: var(--text-muted);
  font-size: 11px;
  text-align: left;
  cursor: pointer;
  transition:
    background var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard),
    transform var(--dur-fast) var(--ease-standard);
}
/* 左侧状态条：整块卡片的「忙/成/败」由它一眼区分（不再依赖小图标颜色）。 */
.orch-agent::before {
  content: '';
  position: absolute;
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 2px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--accent) 55%, transparent);
}
.orch-agent--ok::before { background: var(--ok, var(--accent)); }
.orch-agent--error::before { background: var(--danger); }
.orch-agent--running::before { animation: orch-bar 1.4s ease-in-out infinite; }
.orch-agent:hover {
  background: var(--bg-hover);
  border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
  transform: translateY(-1px);
}
.orch-agent:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 60%, transparent);
  outline-offset: 1px;
}
.orch-agent__top { display: flex; align-items: center; gap: 6px; min-width: 0; padding-left: 2px; }
.orch-agent__dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--accent) 70%, transparent);
}
.orch-agent--ok .orch-agent__dot { background: var(--ok, var(--accent)); }
.orch-agent--error .orch-agent__dot { background: var(--danger); }
.orch-agent--running .orch-agent__dot { animation: orch-pulse 1.2s ease-in-out infinite; }
.orch-agent__role {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
  font-size: 12.5px;
  font-weight: 600;
}
.orch-agent__time {
  margin-left: auto;
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.orch-agent__task {
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  padding-left: 2px;
  /* 任务摘要用 muted 而不是 dim：深色主题下 dim 与卡片底几乎同亮度，
     截图里任务那行读不出来（对比度不足），而它正是「分工」的核心信息。 */
  color: var(--text-muted);
  font-size: 11.5px;
  line-height: 1.5;
}
.orch-agent__foot {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 1px;
  padding-left: 2px;
  color: var(--text-muted);
  font-size: 10.5px;
}
.orch-agent__state { margin-left: auto; }
.orch-agent--ok .orch-agent__state { color: var(--ok, var(--accent)); }
.orch-agent--error .orch-agent__state { color: var(--danger); }
.orch-agent--running .orch-agent__state { color: var(--accent); }
.orch-agent__go { flex-shrink: 0; color: var(--text-dim); }

/* 成员尚未上报时的骨架：告诉用户「正在组建」，而不是一句像报错的空文案。 */
.orch-roster__empty { display: flex; flex-direction: column; gap: 8px; }
.orch-skeleton {
  display: block;
  height: 62px;
  border: 1px dashed color-mix(in srgb, var(--text-dim) 28%, transparent);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-surface-2) 35%, transparent);
  animation: orch-pulse 1.4s ease-in-out infinite;
}
@keyframes orch-bar { 0%, 100% { opacity: .35; } 50% { opacity: 1; } }

/* ── 代理详情 ── */
.orch-detail {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-surface-2) 45%, transparent);
}
.orch-detail__head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--hairline);
}
.orch-detail__back {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
  padding: 1px 6px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font-size: 10px;
  cursor: pointer;
}
.orch-detail__back:hover { color: var(--accent); border-color: var(--accent); }
.orch-detail__back-ic { transform: rotate(180deg); }
.orch-detail__role {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
  font-size: 12px;
}
.orch-detail__state { flex-shrink: 0; color: var(--text-muted); }
.orch-detail__state[data-state='ok'] { color: var(--ok, var(--accent)); }
.orch-detail__state[data-state='error'] { color: var(--danger); }
.orch-detail__state[data-state='running'] { color: var(--accent); }
.orch-detail__time {
  margin-left: auto;
  flex-shrink: 0;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
.orch-detail__block { display: flex; flex-direction: column; gap: 5px; }
.orch-detail__label {
  margin: 0;
  display: flex;
  align-items: baseline;
  gap: 6px;
  color: var(--text-muted);
  font-size: 10px;
  letter-spacing: .06em;
  text-transform: uppercase;
}
.orch-detail__count {
  color: var(--text-muted);
  font-size: 10px;
  letter-spacing: 0;
  text-transform: none;
}
.orch-detail__task {
  margin: 0;
  padding: 8px 10px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg) 50%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}
.orch-detail__tools {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg) 40%, transparent);
}
.orch-detail__empty { margin: 0; color: var(--text-muted); font-size: 11px; }
.orch-detail__out {
  margin: 0;
  max-height: 260px;
  overflow: auto;
  padding: 8px 10px 8px 12px;
  border-left: 2px solid color-mix(in srgb, var(--accent) 55%, transparent);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg) 45%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.6;
  word-break: break-word;
}
/* markdown 注入内容的间距由这里收口（scoped 样式到不了 v-html 子节点） */
.orch-detail__out :deep(p:first-child),
.orch-detail__out :deep(h1:first-child),
.orch-detail__out :deep(h2:first-child),
.orch-detail__out :deep(h3:first-child) { margin-top: 0; }
.orch-detail__out :deep(p:last-child),
.orch-detail__out :deep(ul:last-child),
.orch-detail__out :deep(ol:last-child),
.orch-detail__out :deep(table:last-child) { margin-bottom: 0; }

.orch-run__empty {
  margin: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
}
.orch-run__stage {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  padding: 6px 10px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  color: var(--accent);
  font-size: 11.5px;
}
.orch-run__pulse {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  animation: orch-pulse 1.2s ease-in-out infinite;
}
@keyframes orch-pulse { 0%, 100% { opacity: .25; } 50% { opacity: 1; } }
.orch-run__error {
  margin: 10px 0 0;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 10%, transparent);
  color: var(--danger);
  white-space: pre-wrap;
  font-size: 11.5px;
  line-height: 1.55;
}
.orch-run__reply { margin-top: 10px; padding-top: 8px; border-top: 1px solid var(--hairline); }

/* 成员思考：按轮次分块，正文可滚动（单轮可能上千字），等宽字体便于看结构化推理。 */
.orch-thinking {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 4px 0 0;
  padding: 0;
  list-style: none;
}
.orch-thinking__round {
  padding: 6px 8px;
  border: 1px solid var(--border);
  border-left: 2px solid color-mix(in srgb, var(--accent) 45%, var(--border));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent) 3%, transparent);
}
.orch-thinking__round-no {
  color: var(--text-dim);
  font-size: var(--fs-2xs, 11px);
  letter-spacing: 0.02em;
}
.orch-thinking__text {
  margin: 3px 0 0;
  max-height: 14rem;
  overflow-y: auto;
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--fs-2xs, 11px);
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  overscroll-behavior: contain;
}

/* 产出文件：与最终答复同属「总结区」，用同一条分隔线归拢。 */
.orch-run__files { margin-top: 10px; padding-top: 8px; border-top: 1px solid var(--hairline); }
.orch-run__files-title {
  color: var(--text-dim);
  font-size: var(--fs-2xs, 11px);
  letter-spacing: 0.02em;
}
.orch-run__files-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 4px 0 0;
  padding: 0;
  list-style: none;
}
.orch-run__file {
  display: flex;
  align-items: center;
  gap: 5px;
  color: var(--text-muted);
  font-size: var(--fs-2xs, 11px);
  line-height: 1.5;
}
.orch-run__file-path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
}
.orch-run__file-member {
  flex: 0 0 auto;
  margin-left: auto;
  color: var(--text-dim);
}
.orch-run__files-more {
  margin: 4px 0 0;
  color: var(--text-dim);
  font-size: var(--fs-2xs, 11px);
}
.orch-run__reply-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 9px;
  border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--hairline));
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--accent) 6%, transparent);
  color: var(--accent);
  font-size: 10.5px;
  cursor: pointer;
}
.orch-run__reply-toggle:hover { background: color-mix(in srgb, var(--accent) 12%, transparent); }
.orch-run__reply-body {
  margin: 8px 0 0;
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg) 50%, transparent);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.7;
  max-height: 300px;
  overflow: auto;
}
.orch-run__reply-body :deep(p:first-child),
.orch-run__reply-body :deep(h1:first-child),
.orch-run__reply-body :deep(h2:first-child),
.orch-run__reply-body :deep(h3:first-child) { margin-top: 0; }
.orch-run__reply-body :deep(p:last-child),
.orch-run__reply-body :deep(ul:last-child),
.orch-run__reply-body :deep(ol:last-child),
.orch-run__reply-body :deep(table:last-child) { margin-bottom: 0; }

/* 窄屏（手机 / 侧栏收起）：分工网格改单列，避免 2 列时每个代理卡只剩两三个字。 */
@media (max-width: 560px) {
  .orch-run { padding: 10px 11px; }
  .orch-roster__grid { grid-template-columns: 1fr; }
  .orch-run__goal { font-size: var(--fs-xs); }
}

@media (prefers-reduced-motion: reduce) {
  .orch-run__pulse { animation: none; }
  .orch-skeleton { animation: none; }
  .orch-agent--running::before { animation: none; }
  .orch-agent--running .orch-agent__dot { animation: none; }
  .orch-run__chev { transition: none; }
  .orch-run__fill { transition: none; }
  .orch-agent { transition: none; }
  .orch-agent:hover { transform: none; }
}
</style>
