<template>
  <!-- 应用级右键菜单（2026-10-25）：桌面端此前弹的是 Chromium 默认菜单
       （重新加载 / 另存为 / 检查），既不像本应用，也把"页面"暴露成浏览器页。
       现在：可编辑控件保留原生菜单（粘贴/拼写），其余位置一律换成这份应用菜单。 -->
  <Teleport to="body">
    <div
      v-if="open"
      class="ctx-menu"
      role="menu"
      data-testid="app-context-menu"
      :style="{ left: `${pos.x}px`, top: `${pos.y}px` }"
      @pointerdown.stop
      @contextmenu.prevent
    >
      <button
        type="button"
        class="ctx-menu__item"
        role="menuitem"
        :disabled="!canCopy"
        data-testid="ctx-copy"
        @click="copySelection"
      >
        {{ t('contextMenu.copy') }}
      </button>
      <button
        type="button"
        class="ctx-menu__item"
        role="menuitem"
        data-testid="ctx-select-all"
        @click="selectAll"
      >
        {{ t('contextMenu.selectAll') }}
      </button>
      <div class="ctx-menu__sep" role="separator" />
      <button
        type="button"
        class="ctx-menu__item"
        role="menuitem"
        data-testid="ctx-reload"
        @click="reloadUi"
      >
        {{ t('contextMenu.reload') }}
      </button>
      <button
        type="button"
        class="ctx-menu__item"
        role="menuitem"
        data-testid="ctx-settings"
        @click="openSettings"
      >
        {{ t('nav.settings') }}
      </button>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

const { t } = useI18n()
const router = useRouter()

const open = ref(false)
const pos = reactive({ x: 0, y: 0 })
const canCopy = ref(false)

const MENU_W = 188
const MENU_H = 172

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null
  if (!el || typeof el.closest !== 'function') return false
  return Boolean(el.closest('input, textarea, [contenteditable="true"], [contenteditable=""]'))
}

function close() {
  open.value = false
}

function onContextMenu(ev: MouseEvent) {
  // 组件自己已经处理过（`@contextmenu.prevent`）→ 不再叠一层应用菜单。
  if (ev.defaultPrevented) return
  // 输入框/文本域保留原生菜单：粘贴、拼写检查都在那里。
  if (isEditable(ev.target)) return
  ev.preventDefault()
  const sel = window.getSelection()?.toString() ?? ''
  canCopy.value = sel.trim().length > 0
  pos.x = Math.max(8, Math.min(ev.clientX, window.innerWidth - MENU_W - 8))
  pos.y = Math.max(8, Math.min(ev.clientY, window.innerHeight - MENU_H - 8))
  open.value = true
}

function onPointerDown(ev: PointerEvent) {
  if (!open.value) return
  const el = ev.target as HTMLElement | null
  if (el?.closest?.('[data-testid="app-context-menu"]')) return
  close()
}

function onKeyDown(ev: KeyboardEvent) {
  if (ev.key === 'Escape') close()
}

async function copySelection() {
  const text = window.getSelection()?.toString() ?? ''
  close()
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    // 剪贴板权限被拒时如实失败，不假装成功（用户可改用 Ctrl+C）
  }
}

function selectAll() {
  close()
  const selection = window.getSelection()
  if (!selection) return
  const range = document.createRange()
  range.selectNodeContents(document.body)
  selection.removeAllRanges()
  selection.addRange(range)
}

function reloadUi() {
  close()
  window.location.reload()
}

function openSettings() {
  close()
  void router.push('/settings')
}

onMounted(() => {
  // 冒泡阶段（非捕获）：组件自己的 `@contextmenu.prevent` 先跑，我们再看
  // `defaultPrevented` 决定要不要接管 —— 用捕获会抢在组件前面，把它们的菜单盖掉。
  document.addEventListener('contextmenu', onContextMenu)
  document.addEventListener('pointerdown', onPointerDown, true)
  document.addEventListener('keydown', onKeyDown)
  window.addEventListener('blur', close)
})

onBeforeUnmount(() => {
  document.removeEventListener('contextmenu', onContextMenu)
  document.removeEventListener('pointerdown', onPointerDown, true)
  document.removeEventListener('keydown', onKeyDown)
  window.removeEventListener('blur', close)
})
</script>

<style scoped>
.ctx-menu {
  position: fixed;
  z-index: 4000;
  min-width: 180px;
  padding: 0.25rem;
  display: flex;
  flex-direction: column;
  gap: 1px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-md);
}

.ctx-menu__item {
  display: block;
  width: 100%;
  padding: 0.375rem 0.625rem;
  text-align: left;
  font-size: var(--fs-sm);
  color: var(--text);
  background: transparent;
  border: 0;
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.ctx-menu__item:hover:not(:disabled),
.ctx-menu__item:focus-visible {
  background: var(--bg-hover, color-mix(in srgb, var(--accent) 12%, transparent));
  outline: none;
}

.ctx-menu__item:disabled {
  color: var(--text-dim);
  cursor: default;
}

.ctx-menu__sep {
  height: 1px;
  margin: 0.25rem 0.375rem;
  background: var(--border);
}
</style>
