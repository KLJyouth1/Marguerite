<template>
  <div v-if="report" class="gate-report" :class="report.passed ? 'is-pass' : 'is-fail'">
    <div class="gate-report__head">
      <span>{{ report.passed ? '✅ 通过 shipped gate' : '⛔ 未通过 shipped gate' }}</span>
      <span v-if="report.blocking.length" class="gate-report__muted">
        阻断项：{{ report.blocking.join('、') }}
      </span>
    </div>
    <ul v-if="report.findings.length" class="gate-report__list">
      <li v-for="(f, i) in report.findings" :key="i" :class="'v-' + f.verdict">
        <span class="gate-report__v">{{ verdictLabel(f.verdict) }}</span>
        <b>{{ f.element }}</b>：{{ f.detail }}
      </li>
    </ul>
  </div>
</template>

<script setup lang="ts">
import type { GateReport, GateFinding } from './gateTypes'

defineProps<{ report: GateReport | null }>()

function verdictLabel(v: GateFinding['verdict']): string {
  return v === 'pass' ? '通过' : v === 'warn' ? '提示' : '阻断'
}
</script>

<style scoped>
.gate-report {
  margin-top: 10px;
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  background: color-mix(in srgb, var(--card) 60%, transparent);
  border: 1px solid var(--hairline);
}
.gate-report.is-pass { border-color: color-mix(in srgb, var(--ok) 35%, var(--hairline)); }
.gate-report.is-fail { border-color: color-mix(in srgb, var(--danger) 35%, var(--hairline)); }
.gate-report__head {
  font-weight: 700;
  margin-bottom: 6px;
  display: flex;
  gap: 8px;
  align-items: baseline;
  flex-wrap: wrap;
}
.gate-report__muted { color: var(--text-dim); font-weight: 400; }
.gate-report__list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.gate-report__list li { display: flex; gap: 6px; line-height: 1.5; }
.gate-report__v {
  flex-shrink: 0;
  padding: 0 6px;
  border-radius: var(--radius-xs);
  font-size: 10.5px;
  font-weight: 700;
}
.gate-report__list li.v-pass .gate-report__v { background: color-mix(in srgb, var(--ok) 16%, transparent); color: var(--ok); }
.gate-report__list li.v-warn .gate-report__v { background: color-mix(in srgb, var(--warn) 16%, transparent); color: var(--warn); }
.gate-report__list li.v-fail .gate-report__v { background: color-mix(in srgb, var(--danger) 16%, transparent); color: var(--danger); }
.gate-report__list li.v-pass b { color: var(--ok); }
.gate-report__list li.v-fail b { color: var(--danger); }
</style>
