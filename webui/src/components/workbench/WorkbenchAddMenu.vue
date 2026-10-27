<!--
  工作台「新增面板」菜单：终端 / 内置浏览器 / 代码编辑器。

  为什么抽成组件：同一个菜单需要一个在**面板标题栏**（已有标签时），另一个在
  **右边缘常驻悬浮条**（还没有任何标签、或 dock 被收起时——此时标题栏不可见）。
  两处各写一份必然漂移，入口又会「时有时无」。

  渲染方式（2026-10-24 修）：菜单 **Teleport 到 body + position: fixed**。
  此前菜单是 `.workbench-host { overflow: hidden }` 的后代，绝对定位一旦超出
  面板边界就被裁掉（用户看到「半个框体」），被裁掉的区域还点不到。Teleport
  后菜单不再受任何祖先的 overflow/transform 影响，并按视口做碰撞收边。
-->
<template>
  <div ref="rootEl" class="wam" :class="`wam--${variant}`">
    <button
      ref="triggerEl"
      type="button"
      class="wam__trigger"
      :aria-label="t('workbench.panels.title')"
      :aria-expanded="open"
      aria-haspopup="menu"
      :data-testid="variant === 'rail' ? 'workbench-add-panel-rail' : 'workbench-add-panel'"
      :title="t('workbench.panels.title')"
      @click="toggle"
    >
      <Icon name="plus" :size="variant === 'rail' ? 18 : 16" />
      <span v-if="variant === 'rail'" class="wam__rail-label">{{ t('workbench.panels.title') }}</span>
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        ref="menuEl"
        class="wam__menu wam__menu--floating"
        :style="menuStyle"
        role="menu"
      >
        <template v-if="stage === 'root'">
          <div class="wam__section">{{ t('workbench.panels.title') }}</div>
          <button type="button" class="wam__item" role="menuitem" @click="pickTerminal">
            <Icon name="terminal" :size="15" aria-hidden="true" />
            <span>{{ t('workbench.panels.terminal') }}</span>
          </button>
          <button type="button" class="wam__item" role="menuitem" @click="startBrowser">
            <Icon name="languages" :size="15" aria-hidden="true" />
            <span>{{ t('workbench.panels.browser') }}</span>
          </button>
          <button type="button" class="wam__item" role="menuitem" @click="startEditor">
            <Icon name="fileCode" :size="15" aria-hidden="true" />
            <span>{{ t('workbench.panels.editor') }}</span>
          </button>
          <button type="button" class="wam__item" role="menuitem" @click="pickRuntime">
            <Icon name="gauge" :size="15" aria-hidden="true" />
            <span>{{ t('workbench.panels.runtime') }}</span>
          </button>
        </template>
        <form v-else class="wam__form" @submit.prevent="submit">
          <label class="wam__label" for="workbench-panel-input">
            {{ stage === 'browser' ? t('workbench.panels.browserUrl') : t('workbench.panels.editorPath') }}
          </label>
          <input
            id="workbench-panel-input"
            v-model="input"
            class="wam__input"
            :placeholder="stage === 'browser'
              ? t('workbench.panels.browserUrlPlaceholder')
              : t('workbench.panels.editorPathPlaceholder')"
            spellcheck="false"
            autocomplete="off"
          >
          <p class="wam__hint">
            {{ stage === 'browser' ? t('workbench.panels.browserUrlHint') : t('workbench.panels.editorPathHint') }}
          </p>
          <div class="wam__actions">
            <button type="button" class="btn btn--sm btn--ghost" @click="stage = 'root'">
              {{ t('workbench.panels.back') }}
            </button>
            <button type="submit" class="btn btn--sm btn--primary" :disabled="!input.trim()">
              {{ stage === 'browser' ? t('workbench.panels.openBrowser') : t('workbench.panels.openEditor') }}
            </button>
          </div>
        </form>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'

const props = withDefaults(defineProps<{
  /** icon：面板标题栏里的图标按钮；rail：右边缘常驻悬浮条 */
  variant?: 'icon' | 'rail'
  align?: 'end' | 'start'
}>(), { variant: 'icon', align: 'end' })

const emit = defineEmits<{
  (e: 'terminal'): void
  (e: 'browser', url: string): void
  (e: 'editor', path: string): void
  (e: 'runtime'): void
}>()

const { t } = useI18n()
const rootEl = ref<HTMLElement | null>(null)
const triggerEl = ref<HTMLButtonElement | null>(null)
const menuEl = ref<HTMLElement | null>(null)
const open = ref(false)
const stage = ref<'root' | 'editor' | 'browser'>('root')
const input = ref('')
const position = ref({ top: 0, left: 0 })

const MENU_WIDTH = 248
const VIEWPORT_GAP = 8

/**
 * 菜单定位：以 trigger 的视口矩形为锚点，优先贴右侧展开，空间不够就向左；
 * 下方空间不够则向上翻。任何情况下都收进视口，避免「只能看到一半」。
 */
const menuStyle = computed(() => ({
  top: `${position.value.top}px`,
  left: `${position.value.left}px`,
}))

function placeMenu(): void {
  const trigger = triggerEl.value
  if (!trigger) return
  const rect = trigger.getBoundingClientRect()
  const viewportW = window.innerWidth
  const viewportH = window.innerHeight
  const menuHeight = menuEl.value?.offsetHeight || 220
  const preferRight = props.variant !== 'rail'
  let left = preferRight ? rect.left : rect.left - MENU_WIDTH - VIEWPORT_GAP
  if (left + MENU_WIDTH > viewportW - VIEWPORT_GAP) {
    left = Math.max(VIEWPORT_GAP, rect.right - MENU_WIDTH)
  }
  if (left < VIEWPORT_GAP) left = VIEWPORT_GAP
  let top = rect.bottom + 6
  if (top + menuHeight > viewportH - VIEWPORT_GAP) {
    top = Math.max(VIEWPORT_GAP, rect.top - menuHeight - 6)
  }
  position.value = { top, left }
}

function reset(): void {
  stage.value = 'root'
  input.value = ''
}

function toggle(): void {
  open.value = !open.value
  if (open.value) {
    reset()
    void nextTick(() => {
      placeMenu()
      // 高度在渲染后才可知，再校正一次
      void nextTick(placeMenu)
    })
  }
}

function close(restoreFocus = false): void {
  open.value = false
  reset()
  if (restoreFocus) void nextTick(() => triggerEl.value?.focus())
}

function pickTerminal(): void {
  close()
  emit('terminal')
}

function startBrowser(): void {
  stage.value = 'browser'
  input.value = 'https://example.com'
  void nextTick(() => {
    placeMenu()
    menuEl.value?.querySelector<HTMLInputElement>('input')?.focus()
  })
}

function startEditor(): void {
  stage.value = 'editor'
  input.value = ''
  void nextTick(() => {
    placeMenu()
    menuEl.value?.querySelector<HTMLInputElement>('input')?.focus()
  })
}

function pickRuntime(): void {
  close()
  emit('runtime')
}

function submit(): void {
  const value = input.value.trim()
  if (!value) return
  const target = stage.value
  close()
  if (target === 'browser') emit('browser', value)
  else emit('editor', value)
}

function onPointerDown(event: Event): void {
  if (!open.value) return
  const node = event.target as Node | null
  if (!node) return
  if (rootEl.value?.contains(node) || menuEl.value?.contains(node)) return
  close()
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && open.value) {
    event.stopPropagation()
    close(true)
  }
}

function onViewportChange(): void {
  if (open.value) placeMenu()
}

onMounted(() => {
  document.addEventListener('pointerdown', onPointerDown, true)
  document.addEventListener('keydown', onKeydown)
  window.addEventListener('resize', onViewportChange)
  window.addEventListener('scroll', onViewportChange, true)
})

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onPointerDown, true)
  document.removeEventListener('keydown', onKeydown)
  window.removeEventListener('resize', onViewportChange)
  window.removeEventListener('scroll', onViewportChange, true)
})
</script>

<style scoped>
.wam { position: relative; display: inline-flex; }

/* 右边缘常驻悬浮条：dock 收起或没有任何标签时它是唯一入口，必须够大、带文字 */
.wam--rail {
  position: fixed;
  right: 0;
  top: calc(50% + 58px);
  z-index: 91;
  display: block;
}

.wam__trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border);
  background: var(--bg-elevated, var(--bg-surface));
  color: var(--text-muted);
  cursor: pointer;
  font: inherit;
}

/* 标题栏态：与其它图标按钮同尺寸 */
.wam:not(.wam--rail) .wam__trigger {
  width: 30px;
  height: 30px;
  justify-content: center;
  border-color: transparent;
  border-radius: var(--radius-sm);
  background: transparent;
}

.wam:not(.wam--rail) .wam__trigger:hover { background: var(--bg-hover); color: var(--text); }

/* 悬浮条态：40×86 的可点区域，右侧贴边（与 rs-toggle 同一列） */
.wam--rail .wam__trigger {
  flex-direction: column;
  justify-content: center;
  width: 40px;
  height: 86px;
  padding: 6px 2px;
  border-right: none;
  border-radius: var(--radius-md) 0 0 var(--radius-md);
  box-shadow: 0 8px 20px -12px color-mix(in srgb, var(--scrim) 60%, transparent);
}

.wam--rail .wam__trigger:hover { background: var(--bg-hover); color: var(--accent); }

.wam__rail-label {
  font-size: 9.5px;
  line-height: 1.15;
  writing-mode: vertical-rl;
  letter-spacing: 0.08em;
}

/* 浮动菜单：fixed + 视口坐标，不受任何祖先 overflow 裁剪 */
.wam__menu--floating {
  position: fixed;
  z-index: 1200;
  width: 248px;
  max-height: min(70vh, 520px);
  overflow-y: auto;
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, var(--bg-surface));
  box-shadow: 0 18px 40px -16px color-mix(in srgb, var(--scrim) 55%, transparent);
}

.wam__section {
  padding: 4px 8px 2px;
  color: var(--text-dim);
  font-size: 10px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.wam__item {
  display: flex;
  width: 100%;
  align-items: center;
  gap: var(--sp-2);
  padding: 7px 8px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
  text-align: left;
}

.wam__item:hover { background: var(--bg-hover); }

.wam__form { display: grid; gap: 6px; padding: 6px 8px 8px; }
.wam__label { color: var(--text-dim); font-size: 10.5px; }

.wam__input {
  width: 100%;
  padding: 4px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input, var(--bg-surface));
  color: var(--text);
  font: inherit;
  font-size: var(--fs-xs);
}

.wam__hint { margin: 0; color: var(--text-dim); font-size: 10.5px; line-height: 1.55; }
.wam__actions { display: flex; justify-content: flex-end; gap: 6px; }
</style>
