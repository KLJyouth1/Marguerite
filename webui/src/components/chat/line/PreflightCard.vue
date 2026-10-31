<template>
  <section v-if="preflight" class="pf" :data-testid="'line-preflight'" :class="{ 'pf--bad': !preflight.ok }">
    <header class="pf__head">
      <strong class="pf__title">{{ t('chat.line.preflightTitle') }}</strong>
      <span class="pf__shape">{{ preflight.shape }}</span>
      <span v-if="loading" class="pf__spin">{{ t('chat.line.preflightRunning') }}</span>
    </header>

    <ul v-if="!preflight.ok" class="pf__errors" data-testid="line-preflight-errors">
      <li v-for="(message, index) in errorList" :key="`${index}-${message}`">{{ message }}</li>
      <li v-if="errorList.length === 0">{{ t('chat.line.preflightRejectedUnknown') }}</li>
    </ul>

    <template v-else>
      <table class="pf__table">
        <thead>
          <tr>
            <th>{{ t('chat.line.colStage') }}</th>
            <th>{{ t('chat.line.colTactic') }}</th>
            <th class="pf__num">{{ t('chat.line.colSuccess') }}</th>
            <th class="pf__num">{{ t('chat.line.colP90Ms') }}</th>
            <th class="pf__num">{{ t('chat.line.colP90Tokens') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, index) in preflight.stages" :key="index" data-testid="line-preflight-stage">
            <td>
              <span class="pf__ord">{{ index + 1 }}</span>
              {{ stageLabel(index) }}
            </td>
            <td>
              {{ row.tactic }}
              <span v-if="row.gated" class="pf__gated">{{ t('chat.line.gated') }}</span>
            </td>
            <td class="pf__num">{{ formatPct(row.successRate, 1) }}</td>
            <td class="pf__num">{{ formatMs(row.p90Ms) }}</td>
            <td class="pf__num">{{ formatTokens(row.p90Tokens) }}</td>
          </tr>
          <tr class="pf__total" data-testid="line-preflight-total">
            <td :colspan="2">{{ t('chat.line.totalRow') }}</td>
            <td class="pf__num">{{ formatPct(preflight.total.successRate, 1) }}</td>
            <td class="pf__num">{{ formatMs(preflight.total.p90Ms) }}</td>
            <td class="pf__num">{{ formatTokens(preflight.total.p90Tokens) }}</td>
          </tr>
        </tbody>
      </table>

      <!-- 这条解释是这块面板存在的一半理由：三段各 90% 不是"整体 90%"，是 72.9%。 -->
      <p class="pf__formula">
        <Icon name="info" :size="12" />
        {{ t('chat.line.productNote', { stages: preflight.stages.length, rate: formatPct(preflight.total.successRate, 1) }) }}
      </p>

      <p v-if="preflight.chosenVsRecommended" class="pf__vs" data-testid="line-preflight-vs">
        {{ preflight.chosenVsRecommended }}
      </p>

      <!-- 执行映射：`backend` 与出演名单是引擎自己的 `route_meta` 生成的，
           卡片声明的形状和真正会被调用的东西**同源**，这一段就是那句同源摆在界面上的证据。
           两条诚实说明（swarm 实为同步会诊、solo 实为单个编排子智能体）也在 `note` 里，
           不写出来就等于让"蜂群"这个词继续骗人。 -->
      <div v-if="planRows.length" class="pf__plan" data-testid="line-plan-stages">
        <strong class="pf__planHead">{{ t('chat.line.plan.title') }}</strong>
        <p class="pf__planHint">{{ t('chat.line.plan.hint') }}</p>
        <ol class="pf__planList">
          <li
            v-for="row in planRows"
            :key="row.key"
            class="pf__planRow"
            data-testid="line-plan-stage"
          >
            <span class="pf__planMain">
              <span class="pf__ord">{{ row.ord }}</span>
              <span class="pf__planStage">{{ row.stageText }}</span>
              <code class="pf__planBackend" :title="t('chat.line.plan.backendLabel')">{{ row.backend }}</code>
              <span class="pf__planCast" :title="t('chat.line.plan.castLabel')">{{ row.castText }}</span>
              <span v-if="row.autonomy" class="pf__planAutonomy" :title="t('chat.line.plan.autonomyLabel')">
                {{ row.autonomy }}
              </span>
            </span>
            <span v-if="row.note" class="pf__planNote">{{ row.note }}</span>
          </li>
        </ol>
      </div>

      <div v-if="unknowns.length" class="pf__unknowns" data-testid="line-preflight-unknowns">
        <strong>{{ t('chat.line.unknownsTitle') }}</strong>
        <ul>
          <li v-for="(u, index) in unknowns" :key="`${u.field ?? 'field'}-${index}`">
            <code>{{ u.field ?? '—' }}</code>
            <span>{{ u.reason || t('chat.line.unknownNoReason') }}</span>
          </li>
        </ul>
      </div>

      <div v-if="premortem.length" class="pf__kill">
        <strong>{{ t('chat.line.premortemTitle') }}</strong>
        <ul>
          <li v-for="(item, index) in premortem" :key="`${item.id ?? 'risk'}-${index}`">
            <span class="pf__killFreq">{{ formatPct(item.frequency, 0) }}</span>
            <span class="pf__killText">{{ item.evidence ?? '—' }}</span>
            <span v-if="item.mitigation" class="pf__killFix">{{ item.mitigation }}</span>
          </li>
        </ul>
      </div>
    </template>

    <footer class="pf__gate" :class="`pf__gate--${gateTone}`" data-testid="line-preflight-gate">
      <Icon :name="gateTone === 'allow' ? 'check' : 'shield'" :size="13" />
      <span class="pf__gateText">
        {{ t(`chat.line.gate.${gateDecision}`, { autonomy: preflight.gate?.autonomy ?? '—' }) }}
      </span>
      <span v-if="preflight.gate?.reason" class="pf__gateReason">{{ preflight.gate.reason }}</span>
    </footer>
  </section>

  <section v-else-if="!loading" class="pf pf--idle">{{ t('chat.line.preflightIdle') }}</section>
</template>

<script setup lang="ts">
// 预演卡（PreflightCard）—— 派单**之前**的代价与裁决。
//
// 它是"对话式生产工程"和"聊天框"的分界：这一段数字不是装饰，而是 `line.run` 之前
// 唯一一次"要不要按这个形状花这些 token"的决策依据。所有数字来自 `tactics::simulate`
// 的蒙特卡洛（逐阶段）+ 串行汇总（耗时相加、成功率相乘），前端只做格式化。
//
// 诚实约束：`unknowns` 非空 ⇒ 显式说明"这些数字没有实测先验"，绝不因为"有数"就当"准"。
// `line.plan` 是**配角**：它有就显示、没有就整段消失（预演表照常站着），
// 把"没拿到映射"演成"映射为空"就是又一处静默缺陷。
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import Icon from '@/components/Icon.vue'
import { formatMs, formatPct, formatTokens } from '@/utils/tactics/model'
import type { LineCatalog, LinePlan, LinePreflight, LineSpec } from '@/composables/line/types'

const props = defineProps<{
  preflight: LinePreflight | null
  loading: boolean
  spec: LineSpec
  catalog: LineCatalog
  plan?: LinePlan | null
}>()

const { t } = useI18n()

/**
 * 被拒时的错误来自 `validation.errors`（数组），顶层 `error` 只是它的拼接。
 * 拼接串在网关参数解析失败时才有内容，两条都要读，否则"拒绝但没有理由"。
 */
const errorList = computed<string[]>(() => {
  const fromValidation = props.preflight?.validation?.errors ?? []
  if (fromValidation.length > 0) return fromValidation
  const single = props.preflight?.error
  return single ? [single] : []
})

/** 事前验尸按发生概率排序已经由后端做完，这里只取前三条最要命的。 */
const premortem = computed(() => (props.preflight?.premortem ?? []).slice(0, 3))
const unknowns = computed(() => props.preflight?.unknowns ?? [])
const gateDecision = computed(() => props.preflight?.gate?.decision ?? 'allow')
const gateTone = computed(() =>
  gateDecision.value === 'allow' ? 'allow' : gateDecision.value === 'confirm' ? 'confirm' : 'deny',
)

function stageLabel(index: number): string {
  const stage = props.spec.stages[index]
  if (!stage) return String(index + 1)
  return props.catalog.patterns.find((p) => p.id === stage.pattern)?.label ?? stage.pattern
}

/* ── 执行映射（line.plan）─────────────────────────────────────────────── */

interface PlanRow {
  key: string
  ord: number
  stageText: string
  backend: string
  castText: string
  autonomy: string
  note: string
}

/** 字段没带就是没带：留空由模板 `v-if` 收掉，绝不替它编一个看起来像值的占位。 */
function textOf(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function patternLabelOf(pattern: unknown): string {
  const id = textOf(pattern)
  if (!id) return ''
  return props.catalog.patterns.find((p) => p.id === id)?.label ?? id
}

/**
 * `ok:false` 的计划（后端没通过校验时给的那份）不排：它连阶段都没解析出来，
 * 摆上去就是一排"看起来像执行计划"的空行。
 */
const planRows = computed<PlanRow[]>(() => {
  const plan = props.plan
  if (!plan || plan.ok === false) return []
  return (Array.isArray(plan.stages) ? plan.stages : []).map((row, index) => {
    const width = typeof row.width === 'number' && row.width > 1 ? `×${Math.trunc(row.width)}` : ''
    return {
      key: `${typeof row.index === 'number' ? row.index : index}-${textOf(row.pattern) || index}`,
      ord: (typeof row.index === 'number' ? row.index : index) + 1,
      stageText: [patternLabelOf(row.pattern), width].filter(Boolean).join(' '),
      backend: textOf(row.backend) || '—',
      castText: (Array.isArray(row.roster) ? row.roster : []).filter(Boolean).join(' · ') || '—',
      autonomy: textOf(row.autonomy),
      note: textOf(row.note),
    }
  })
})
</script>

<style scoped>
.pf {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: 10px 12px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  background: var(--card);
  font-size: 12px;
  color: var(--text);
}

.pf--idle,
.pf--bad {
  color: var(--text-dim);
}

.pf__head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
}

.pf__title {
  font-size: 12.5px;
}

.pf__shape {
  font-size: 11px;
  color: var(--text-dim);
  font-variant-numeric: tabular-nums;
}

.pf__spin {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-dim);
}

.pf__table {
  width: 100%;
  border-collapse: collapse;
  font-size: 11.5px;
}

.pf__table th,
.pf__table td {
  text-align: left;
  padding: 4px 6px;
  border-bottom: 1px solid var(--hairline);
}

.pf__num {
  text-align: right !important;
  font-variant-numeric: tabular-nums;
}

.pf__ord {
  color: var(--text-dim);
  margin-right: 4px;
}

.pf__total td {
  font-weight: 600;
  border-bottom: none;
}

.pf__gated {
  color: var(--text-dim);
  font-size: 10.5px;
  border: 1px solid var(--hairline);
  border-radius: 999px;
  padding: 0 5px;
  margin-left: 5px;
}

.pf__formula,
.pf__vs {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  margin: 0;
  color: var(--text-dim);
  font-size: 11.5px;
}

.pf__vs {
  border-left: 2px solid color-mix(in srgb, var(--accent) 45%, var(--hairline));
  padding-left: 8px;
  color: var(--text);
}

.pf__errors {
  margin: 0;
  padding-left: 16px;
  color: var(--danger);
}

.pf__unknowns,
.pf__kill {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

/* 执行映射刻意比主表低一层：虚线框 + 更小的字号，读它是"确认这行会调什么"，
   不是抢预演表的数字。 */
.pf__plan {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px 8px;
  border: 1px dashed var(--hairline);
  border-radius: var(--radius-sm);
}

.pf__planHead {
  font-size: 11.5px;
  color: var(--text);
}

.pf__planHint {
  margin: 0;
  font-size: 11px;
  color: var(--text-dim);
}

.pf__planList {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.pf__planRow {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.pf__planMain {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 6px;
  font-size: 11.5px;
}

.pf__planStage {
  color: var(--text);
}

.pf__planBackend {
  font-size: 11px;
  color: var(--text);
  word-break: break-all;
}

.pf__planCast,
.pf__planAutonomy {
  font-size: 11px;
  color: var(--text-dim);
}

.pf__planAutonomy {
  margin-left: auto;
}

.pf__planNote {
  padding-left: 8px;
  border-left: 2px solid var(--hairline);
  font-size: 11px;
  color: var(--text-dim);
}

.pf__unknowns ul,
.pf__kill ul {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.pf__unknowns li {
  display: flex;
  gap: 6px;
  color: var(--text-dim);
}

.pf__unknowns code {
  font-size: 11px;
}

.pf__kill li {
  display: flex;
  gap: 6px;
  align-items: baseline;
}

.pf__killFreq {
  font-variant-numeric: tabular-nums;
  color: var(--warn);
}

.pf__killText {
  color: var(--text);
}

.pf__killFix {
  color: var(--text-dim);
}

.pf__gate {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--hairline);
}

.pf__gate--allow {
  border-color: color-mix(in srgb, var(--ok) 45%, var(--hairline));
}

.pf__gate--confirm {
  border-color: color-mix(in srgb, var(--warn) 55%, var(--hairline));
}

.pf__gate--deny {
  border-color: color-mix(in srgb, var(--danger) 55%, var(--hairline));
}

.pf__gateReason {
  color: var(--text-dim);
  font-size: 11px;
}
</style>
