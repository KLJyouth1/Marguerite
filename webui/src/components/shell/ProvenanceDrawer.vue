<!-- 记忆溯源抽屉（Provenance Drawer）。
     5 Why 溯源：用户看不到智能体「记住了什么、为什么记得、为什么某条被丢掉」。
     `assistant.brief.get` 已经返回结构化简报与投影丢弃明细，但 UI 此前一处都看不到，
     记忆因此不可审计。本抽屉把记忆投影提升为可检查的一等公民：分区展示钉住 /
     偏好 / 事实 / 被投影丢弃的事实与原因，并提供诚实空态。
     非模态（aria-modal="false"）：用户仍可继续与主界面交互，所以不做遮罩。 -->
<template>
  <aside
    v-if="open && !narrow"
    class="prov-drawer"
    :class="{ 'prov-drawer--reduced': reducedMotion }"
    role="dialog"
    aria-modal="false"
    :aria-label="t('shell.provenance.label')"
    aria-keyshortcuts="Alt+P"
    data-testid="provenance-drawer"
    @keydown.esc="close"
  >
    <header class="prov-drawer__head">
      <h2
        ref="titleEl"
        class="prov-drawer__title"
        tabindex="-1"
        data-testid="provenance-title"
      >
        {{ t('shell.provenance.title') }}
      </h2>
      <span class="prov-drawer__shortcut">{{ t('shell.provenance.shortcut') }}</span>
      <button
        type="button"
        class="prov-drawer__button"
        :aria-label="t('shell.provenance.refresh')"
        :title="t('shell.provenance.refresh')"
        :disabled="state === 'loading'"
        data-testid="provenance-refresh"
        @click="refresh"
      >
        ↻
      </button>
      <button
        type="button"
        class="prov-drawer__button"
        :aria-label="t('shell.provenance.close')"
        :title="t('shell.provenance.close')"
        data-testid="provenance-close"
        @click="close"
      >
        ×
      </button>
    </header>

    <p v-if="state === 'loading'" class="prov-drawer__status" data-testid="provenance-loading">
      {{ t('shell.provenance.loading') }}
    </p>
    <p
      v-else-if="state === 'unsupported'"
      class="prov-drawer__empty"
      data-testid="provenance-unsupported"
    >
      {{ t('shell.provenance.unsupported') }}
    </p>
    <div v-else-if="state === 'error'" class="prov-drawer__status prov-drawer__status--error">
      <p data-testid="provenance-error">{{ error }}</p>
      <button type="button" class="prov-drawer__button" @click="refresh">
        {{ t('shell.provenance.refresh') }}
      </button>
    </div>
    <p v-else-if="state === 'idle'" class="prov-drawer__status">
      {{ t('shell.provenance.loading') }}
    </p>

    <div v-else class="prov-drawer__body">
      <p class="prov-drawer__meta" data-testid="provenance-meta">
        <span>{{ scopeText }}</span>
        <span class="prov-dot">·</span>
        <span>{{ workspaceText }}</span>
      </p>

      <section class="prov-drawer__section" data-testid="provenance-pinned">
        <h3 class="prov-drawer__section-title">{{ t('shell.provenance.sections.pinned') }}</h3>
        <p v-if="model.pinned.length === 0" class="prov-drawer__empty">
          {{ t('shell.provenance.empty.pinned') }}
        </p>
        <ul v-else class="prov-drawer__list">
          <li v-for="(item, index) in model.pinned" :key="index" class="prov-item">
            <p class="prov-item__text">{{ item.text }}</p>
            <p class="prov-item__meta">
              <span>{{ item.source || t('shell.provenance.noSource') }}</span>
              <span v-if="item.at" class="prov-dot">·</span>
              <span v-if="item.at">{{ item.at }}</span>
            </p>
          </li>
        </ul>
      </section>

      <section class="prov-drawer__section" data-testid="provenance-preferences">
        <h3 class="prov-drawer__section-title">{{ t('shell.provenance.sections.preferences') }}</h3>
        <p v-if="model.preferences.length === 0" class="prov-drawer__empty">
          {{ t('shell.provenance.empty.preferences') }}
        </p>
        <ul v-else class="prov-drawer__list">
          <li
            v-for="(item, index) in model.preferences"
            :key="`${item.key}-${index}`"
            class="prov-item"
          >
            <p class="prov-item__text">
              <span class="prov-item__key">{{ item.key }}</span>
              <span>{{ item.value }}</span>
            </p>
            <p class="prov-item__meta">
              <span>{{ t('shell.provenance.confidence') }} {{ item.confidence }}</span>
              <span class="prov-dot">·</span>
              <span>{{ t('shell.provenance.evidence') }} {{ item.evidence }}</span>
            </p>
          </li>
        </ul>
      </section>

      <section class="prov-drawer__section" data-testid="provenance-facts">
        <h3 class="prov-drawer__section-title">{{ t('shell.provenance.sections.facts') }}</h3>
        <p v-if="model.facts.length === 0" class="prov-drawer__empty">
          {{ t('shell.provenance.empty.facts') }}
        </p>
        <ul v-else class="prov-drawer__list">
          <li
            v-for="(item, index) in model.facts"
            :key="`${item.text}-${index}`"
            class="prov-item"
            data-testid="provenance-fact"
          >
            <p class="prov-item__text">{{ item.text }}</p>
            <p class="prov-item__meta">
              <span>{{ item.source || t('shell.provenance.noSource') }}</span>
              <span class="prov-dot">·</span>
              <span>{{ item.score }}</span>
              <!-- 可撤销性（R-02）：只有后端明确说 deletable 的事实才给入口。
                   外部 AI 工具写的 Markdown 记忆不归本应用写，那里没有按钮。 -->
              <button
                v-if="item.deletable && item.id"
                type="button"
                class="prov-item__forget"
                :disabled="deleting === item.id"
                :title="t('shell.provenance.forgetHint')"
                data-testid="provenance-fact-delete"
                @click="forget(item.id)"
              >
                {{ t('shell.provenance.forget') }}
              </button>
            </p>
          </li>
        </ul>
        <p
          v-if="state === 'ready' && error"
          class="prov-drawer__status prov-drawer__status--error"
          role="alert"
          data-testid="provenance-delete-error"
        >
          {{ error }}
        </p>
      </section>

      <section class="prov-drawer__section" data-testid="provenance-projection">
        <h3 class="prov-drawer__section-title">{{ t('shell.provenance.sections.dropped') }}</h3>
        <p
          v-if="!model.hasProjection"
          class="prov-drawer__empty"
          data-testid="provenance-projection-empty"
        >
          {{ t('shell.provenance.empty.projection') }}
        </p>
        <p
          v-else-if="model.dropped.length === 0"
          class="prov-drawer__empty"
          data-testid="provenance-nothing-dropped"
        >
          {{ t('shell.provenance.empty.dropped') }}
        </p>
        <ul v-else class="prov-drawer__list">
          <li
            v-for="(item, index) in model.dropped"
            :key="`${item.key}-${index}`"
            class="prov-item"
            data-testid="provenance-dropped"
          >
            <p class="prov-item__text">{{ item.key }}</p>
            <p class="prov-item__meta">
              <span
                class="prov-reason"
                :data-reason="item.reason"
              >{{ reasonLabel(item.reason) }}</span>
            </p>
          </li>
        </ul>
        <p v-if="model.hasProjection && model.policy" class="prov-drawer__policy">
          {{ t('shell.provenance.policy') }} · {{ model.policy }}
        </p>
      </section>

      <section v-if="model.budget.maxChars > 0" class="prov-drawer__section" data-testid="provenance-budget">
        <h3 class="prov-drawer__section-title">{{ t('shell.provenance.sections.budget') }}</h3>
        <p class="prov-drawer__meta">
          <span>{{ t('shell.provenance.budgetUsage', { used: model.budget.used, max: model.budget.maxChars }) }}</span>
        </p>
        <p v-if="model.budget.notes" class="prov-drawer__policy">{{ model.budget.notes }}</p>
      </section>
    </div>

    <footer class="prov-drawer__foot">
      <button
        type="button"
        class="prov-drawer__copy"
        :disabled="state !== 'ready'"
        data-testid="provenance-copy"
        @click="copySummary"
      >
        {{ t('shell.provenance.copy') }}
      </button>
      <p
        v-if="copyState !== 'idle'"
        class="prov-drawer__copy-status"
        role="status"
        data-testid="provenance-copy-status"
      >
        {{ copyState === 'copied'
          ? t('shell.provenance.copied')
          : t('shell.provenance.copyFailed') }}
      </p>
    </footer>
  </aside>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useMediaQuery } from '@/composables/chat/useMediaQuery'
import { useProvenance } from '@/composables/shell/useProvenance'

const { t, te } = useI18n()

// 与 <style> 里的 media query 同一判据；<900px 不渲染（移动端没有抽屉身位）。
const narrow = useMediaQuery('(max-width: 899.98px)')
const reducedMotion = useMediaQuery('(prefers-reduced-motion: reduce)')

const { state, model, error, deleting, loadProvenance, deleteFact } = useProvenance()

const open = ref(false)
const titleEl = ref<HTMLElement | null>(null)
const copyState = ref<'idle' | 'copied' | 'failed'>('idle')

// 打开前的焦点位置：关闭后要还回去，键盘用户不会掉到页面顶部。
let restoreTarget: HTMLElement | null = null

const scopeText = computed(() => {
  const scope = model.value.scopeLabel
  if (!scope) return t('shell.provenance.scope.unknown')
  const key = `shell.provenance.scope.${scope}`
  return te(key) ? t(key) : scope
})

const workspaceText = computed(() =>
  model.value.workspace || t('shell.provenance.workspaceNone'),
)

function reasonLabel(reason: string): string {
  if (!reason) return t('shell.provenance.reason.unknown')
  const key = `shell.provenance.reason.${reason}`
  return te(key) ? t(key) : reason
}

function refresh(): void {
  copyState.value = 'idle'
  void loadProvenance()
}

/**
 * 忘掉一条事实。
 *
 * 不做二次确认弹窗：抽屉是非模态的，弹一个 modal 会把「轻量检查」变成打断；
 * 而这条操作**只影响记忆、不影响对话内容**，且删错了可以重新说一遍让它再学到。
 * 失败时 `error` 会带原文显示在事实区下方 —— 静默失败才是不可接受的。
 */
function forget(id: string): void {
  copyState.value = 'idle'
  void deleteFact(id)
}

function openDrawer(): void {
  if (narrow.value) return
  if (open.value) return
  restoreTarget = document.activeElement instanceof HTMLElement ? document.activeElement : null
  open.value = true
  void nextTick(() => titleEl.value?.focus())
  refresh()
}

function close(): void {
  if (!open.value) return
  open.value = false
  copyState.value = 'idle'
  const target = restoreTarget
  restoreTarget = null
  if (target && typeof target.focus === 'function') target.focus()
}

function toggle(): void {
  if (open.value) close()
  else openDrawer()
}

function onKeydown(event: KeyboardEvent): void {
  if (event.altKey && !event.ctrlKey && !event.metaKey) {
    if (event.key === 'p' || event.key === 'P' || event.code === 'KeyP') {
      event.preventDefault()
      toggle()
      return
    }
  }
  if (event.key === 'Escape' && open.value) {
    event.preventDefault()
    close()
  }
}

function summaryText(): string {
  const m = model.value
  const lines: string[] = [
    `${t('shell.provenance.title')} · ${scopeText.value} · ${workspaceText.value}`,
  ]
  lines.push(`${t('shell.provenance.sections.pinned')} (${m.pinned.length})`)
  m.pinned.forEach((item) => lines.push(`- ${item.text}${item.source ? ` · ${item.source}` : ''}`))
  lines.push(`${t('shell.provenance.sections.preferences')} (${m.preferences.length})`)
  m.preferences.forEach((item) => lines.push(`- ${item.key}: ${item.value}`))
  lines.push(`${t('shell.provenance.sections.facts')} (${m.facts.length})`)
  m.facts.forEach((item) => lines.push(`- ${item.text} · ${item.source || t('shell.provenance.noSource')} · ${item.score}`))
  if (m.hasProjection) {
    lines.push(`${t('shell.provenance.sections.dropped')} (${m.dropped.length})`)
    m.dropped.forEach((item) => lines.push(`- ${item.key} · ${reasonLabel(item.reason)}`))
  } else {
    lines.push(`${t('shell.provenance.sections.dropped')} · ${t('shell.provenance.empty.projection')}`)
  }
  return lines.join('\n')
}

async function copySummary(): Promise<void> {
  copyState.value = 'idle'
  try {
    const clipboard = navigator.clipboard
    if (!clipboard || typeof clipboard.writeText !== 'function') {
      throw new Error('clipboard unavailable')
    }
    await clipboard.writeText(summaryText())
    copyState.value = 'copied'
  } catch {
    // 静默降级：权限被拒 / 非安全上下文都会走到这里，给一条可读提示即可，
    // 绝不能把拒绝变成未捕获异常。
    copyState.value = 'failed'
  }
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  restoreTarget = null
})
</script>

<style scoped>
/* 右侧非模态抽屉：宽 360px，贴 top/bottom/right。z-index 60 低于既有 modal
   （1100+）但高于内容（App 的 5 / 95 分层之外的可视区），且不与左下角
   Attention Shelf（z-index 55）抢位置。
   动效只用 transform + opacity，时长/缓动全部走 foundation.css 的 token。 */
.prov-drawer {
  position: fixed;
  top: 0;
  right: 0;
  bottom: 0;
  z-index: 60;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
  width: 360px;
  max-width: 100vw;
  border-left: 1px solid var(--hairline);
  background: var(--bg-surface);
  color: var(--text);
  font-family: var(--font-sans);
  font-size: var(--fs-sm);
  box-shadow: -12px 0 32px color-mix(in srgb, var(--bg-base, var(--bg)) 55%, transparent);
  animation: prov-slide-in var(--dur-enter) var(--ease-out) both;
}

.prov-drawer--reduced {
  animation: prov-fade-in var(--dur-base) var(--ease-standard) both;
}

.prov-drawer__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border-bottom: 1px solid var(--hairline);
}

.prov-drawer__title {
  flex: 1 1 auto;
  min-width: 0;
  margin: 0;
  color: var(--text);
  font-size: var(--fs-md);
  font-weight: 600;
  letter-spacing: 0.01em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.prov-drawer__title:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.prov-drawer__shortcut {
  flex: 0 0 auto;
  padding: 1px var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  white-space: nowrap;
}

.prov-drawer__button {
  flex: 0 0 auto;
  min-width: 26px;
  min-height: 26px;
  padding: 0 var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  color: var(--text-muted);
  font-family: var(--font-sans);
  font-size: var(--fs-sm);
  line-height: 1;
  cursor: pointer;
  transition: transform var(--dur-fast) var(--ease-standard);
}

.prov-drawer__button:hover {
  color: var(--text);
  border-color: var(--border-strong);
}

.prov-drawer__button:active {
  transform: translateY(1px);
}

.prov-drawer__button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.prov-drawer__button:disabled {
  opacity: 0.5;
  cursor: default;
}

.prov-drawer__status {
  margin: 0;
  padding: var(--sp-3) var(--sp-4);
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.prov-drawer__status--error {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--sp-2);
  color: var(--danger, var(--warn));
}

.prov-drawer__body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: var(--sp-2) var(--sp-4) var(--sp-4);
}

.prov-drawer__meta {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin: 0;
  padding: var(--sp-2) 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.prov-drawer__section {
  padding-top: var(--sp-2);
  border-top: 1px solid var(--hairline);
}

.prov-drawer__section-title {
  margin: var(--sp-2) 0 var(--sp-1);
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.prov-drawer__empty {
  margin: 0;
  padding: var(--sp-1) 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.prov-drawer__list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.prov-item {
  padding: var(--sp-1) 0;
}

.prov-item__text {
  margin: 0;
  color: var(--text);
  font-size: var(--fs-sm);
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.prov-item__key {
  margin-right: var(--sp-2);
  color: var(--accent);
  font-size: var(--fs-xs);
}

.prov-item__meta {
  display: flex;
  align-items: center;
  gap: var(--sp-1);
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  overflow-wrap: anywhere;
}

.prov-dot {
  color: var(--text-dim);
}

/* 「忘掉」：不抢视觉权重 —— 平时与元信息同色，hover 才升到正文色；
   危险语义靠文案表达，不靠一整块红按钮去吓人（它删的是一条记忆，不是文件）。 */
.prov-item__forget {
  margin-left: auto;
  padding: 0 var(--sp-2);
  border: 1px solid transparent;
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-dim);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  line-height: 1.6;
  cursor: pointer;
  transition: color var(--dur-fast) var(--ease-standard);
}

.prov-item__forget:hover {
  color: var(--text);
  border-color: var(--border);
}

.prov-item__forget:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.prov-item__forget:disabled {
  opacity: 0.5;
  cursor: default;
}

.prov-item__forget:active {
  transform: translateY(1px);
}

.prov-reason {
  padding: 1px var(--sp-2);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  color: var(--warn);
}

.prov-drawer__policy {
  margin: var(--sp-1) 0 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  overflow-wrap: anywhere;
}

.prov-drawer__foot {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border-top: 1px solid var(--hairline);
}

.prov-drawer__copy {
  min-height: 28px;
  padding: 2px var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: var(--bg-elevated);
  color: var(--text);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  cursor: pointer;
  transition: transform var(--dur-fast) var(--ease-standard);
}

.prov-drawer__copy:active {
  transform: translateY(1px);
}

.prov-drawer__copy:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.prov-drawer__copy:disabled {
  opacity: 0.5;
  cursor: default;
}

.prov-drawer__copy-status {
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

@keyframes prov-slide-in {
  from {
    opacity: 0;
    transform: translateX(16px);
  }
}

@keyframes prov-fade-in {
  from {
    opacity: 0;
  }
}

/* 系统要求减少动效：纯淡入（无位移）。 */
@media (prefers-reduced-motion: reduce) {
  .prov-drawer {
    animation: prov-fade-in var(--dur-base) var(--ease-standard) both;
  }

  .prov-drawer__button,
  .prov-drawer__copy,
  .prov-item__forget {
    transition: none;
  }

  .prov-drawer__button:active,
  .prov-drawer__copy:active,
  .prov-item__forget:active {
    transform: none;
  }
}

/* <900px 不渲染；这里再兜一层，防止 SSR/首帧时 media query 未就绪。 */
@media (max-width: 899.98px) {
  .prov-drawer {
    display: none;
  }
}
</style>
