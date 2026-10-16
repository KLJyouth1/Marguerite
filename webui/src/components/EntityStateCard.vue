<script setup lang="ts">
/**
 * EntityStateCard —— 通用动态对象状态卡（任务书 B §1.2）。
 *
 * 极简、无业务：只负责渲染 名称 / 现在做什么 / 下一步做什么。
 * “现在/下一步”的文案键与色调由调用方用纯函数（如 cardModel.describeTrainCard）算好、
 * 经 t() 解析后传入 —— 本组件不做相位判断，因此任务书 A 的设备节点与岛屿 agent
 * 节点可直接复用同一张卡。
 *
 * 样式只走 design token（色/圆角/时长），不写裸 hex/px 时长/radius。
 */
export type EntityCardTone = 'accent' | 'ok' | 'warn' | 'info' | 'muted'

defineProps<{
  /** 实体名（已解析，如 G102） */
  title: string
  /** 种类标签（已解析，如“列车”） */
  kindLabel: string
  /** “现在做什么”（已解析） */
  nowLabel: string
  /** “现在”补充细节（车站名等，已解析；可空） */
  nowDetail?: string
  /** “下一步做什么”（已解析） */
  nextLabel: string
  /** “下一步”补充细节（已解析；可空） */
  nextDetail?: string
  /** 当前状态色调 */
  tone?: EntityCardTone
  /** 关闭按钮无障碍标签（已解析） */
  closeLabel?: string
}>()

const emit = defineEmits<{ close: [] }>()
</script>

<template>
  <aside class="esc" :data-tone="tone ?? 'muted'" data-entity-state-card>
    <button
      type="button"
      class="esc__close"
      :aria-label="closeLabel ?? 'Close'"
      @click="emit('close')"
    >×</button>
    <p class="esc__kind">{{ kindLabel }}</p>
    <h3 class="esc__title">{{ title }}</h3>

    <dl class="esc__rows">
      <div class="esc__row">
        <dt class="esc__label">
          <span class="esc__dot" aria-hidden="true" />
          <span>{{ nowLabel }}</span>
        </dt>
        <dd v-if="nowDetail" class="esc__detail">{{ nowDetail }}</dd>
      </div>
      <div class="esc__row esc__row--next">
        <dt class="esc__label esc__label--next">
          <span class="esc__dot esc__dot--next" aria-hidden="true" />
          <span>{{ nextLabel }}</span>
        </dt>
        <dd v-if="nextDetail" class="esc__detail esc__detail--next">{{ nextDetail }}</dd>
      </div>
    </dl>
  </aside>
</template>

<style scoped>
.esc {
  position: relative;
  min-width: 208px;
  max-width: 260px;
  padding: var(--sp-3) var(--sp-4);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  box-shadow: var(--elev-2);
  color: var(--text);
}

.esc__kind {
  margin: 0;
  font-size: var(--fs-xs);
  letter-spacing: var(--label-track);
  text-transform: uppercase;
  color: var(--text-dim);
}

.esc[data-tone='accent'] .esc__kind { color: var(--accent); }
.esc[data-tone='ok'] .esc__kind { color: var(--ok); }
.esc[data-tone='warn'] .esc__kind { color: var(--warn); }
.esc[data-tone='info'] .esc__kind { color: var(--info); }

.esc__title {
  margin: var(--sp-1) 0 var(--sp-3);
  font-size: var(--fs-sm);
  font-weight: 600;
}

.esc__rows {
  display: grid;
  gap: var(--sp-2);
  margin: 0;
}

.esc__row {
  display: grid;
  gap: 2px;
}

.esc__label {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-sm);
  color: var(--text);
}

.esc__dot {
  flex: 0 0 auto;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-pill);
  background: var(--text-dim);
}

.esc[data-tone='accent'] .esc__dot { background: var(--accent); }
.esc[data-tone='ok'] .esc__dot { background: var(--ok); }
.esc[data-tone='warn'] .esc__dot { background: var(--warn); }
.esc[data-tone='info'] .esc__dot { background: var(--info); }

.esc__row--next {
  padding-top: var(--sp-2);
  border-top: 1px solid var(--hairline);
}

.esc__label--next {
  color: var(--text-muted);
  font-weight: 500;
}

.esc__dot--next {
  background: var(--text-dim);
}

.esc__detail {
  margin: 0 0 0 var(--sp-4);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.esc__detail--next {
  color: var(--text-dim);
}

.esc__close {
  position: absolute;
  top: var(--sp-1);
  right: var(--sp-2);
  padding: 2px var(--sp-1);
  border: none;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-dim);
  font-size: var(--fs-lg);
  line-height: 1;
  cursor: pointer;
}

.esc__close:hover {
  color: var(--text);
  background: var(--bg-hover);
}
</style>
