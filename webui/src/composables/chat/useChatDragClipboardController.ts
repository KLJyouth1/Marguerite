// 拖拽 / 剪贴板 控制器（从 ChatView.vue 抽出的本地编排函数）。
//
// 这一批函数负责「拖拽文件进对话线程即作为附件、系统剪贴板粘贴文件即作为附件」。
// 它们横跨 ChatView 的两个区域：模板 @dragenter/@dragover/@dragleave/@drop 绑定
// 与 onMounted 内 useDocumentEvent('paste', …) 注册。函数体只依赖共享原语
// （threadDragDepth / threadDragOver 在共享状态袋内、composerRef 在共享状态袋内）、
// 一份来自 useChatAttachments 的 addAttachments、以及若干纯工具函数，耦合面窄、风险低，
// 故整体迁入本控制器。注意：autoResizeTextarea 与 onDocumentKeydown 刻意留在本视图——
// 前者是 composer 关注点（被 5+ 组合式与 onSend 共用），后者是横跨 share 模式 /
// 待发送队列 / 停止的全局 ESC 处理，属高危跨切面逻辑，按"不盲拆高危"原则保留原位。
import { type ComputedRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { useToasts } from '@/composables/useToasts'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import { dragEventHasFiles } from '@/composables/chat/chatViewUtils'
import { collectClipboardFiles, shouldCaptureFilePaste } from '@/utils/chat/attachments'
import { hasOpenDialogLayer } from '@/composables/useDialogA11y'

export interface ChatDragClipboardControllerDeps {
  /** 来自 useChatPlans；replan（改方案）进行中时禁止拖入/粘贴附件。 */
  replanActive: ComputedRef<boolean>
  /** 来自 useChatAttachments；把文件推入待发送附件列表。 */
  addAttachments: (files: File[]) => Promise<void>
}

export function useChatDragClipboardController(
  state: ChatViewState,
  deps: ChatDragClipboardControllerDeps,
) {
  const { t } = useI18n()
  const { pushToast } = useToasts()
  const { composerRef, threadDragDepth, threadDragOver } = state
  const { replanActive, addAttachments } = deps

  /* ── Tool calls ────────────────────────────────────────────────────── */

  function onChatDragEnter(e: DragEvent) {
    if (!dragEventHasFiles(e)) return
    e.preventDefault()
    if (replanActive.value) return
    threadDragDepth.value += 1
    threadDragOver.value = true
  }

  function onChatDragOver(e: DragEvent) {
    if (!dragEventHasFiles(e)) return
    e.preventDefault()
    if (replanActive.value) {
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'none'
      return
    }
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'
    threadDragOver.value = true
  }

  function onChatDragLeave(e: DragEvent) {
    if (!dragEventHasFiles(e)) return
    threadDragDepth.value = Math.max(0, threadDragDepth.value - 1)
    if (threadDragDepth.value === 0) {
      threadDragOver.value = false
    }
  }

  function onChatDrop(e: DragEvent) {
    e.preventDefault()
    threadDragDepth.value = 0
    threadDragOver.value = false
    if (!dragEventHasFiles(e)) return
    if (replanActive.value) {
      pushToast(t('chat.plan.attachmentsUnavailable'), { tone: 'warn' })
      return
    }
    const files = Array.from(e.dataTransfer?.files || [])
    if (files.length === 0) return
    void addAttachments(files)
    composerRef.value?.focusTextarea()
  }

  /* ── Clipboard paste ───────────────────────────────────────────────── */

  function onDocumentPaste(e: ClipboardEvent) {
    // Pastes aimed at another editable surface (clarify/approval inputs, the
    // command palette) or at an open dialog keep their default behavior — only
    // composer-bound pastes claim clipboard files, mirroring onDocumentKeydown.
    if (!shouldCaptureFilePaste(e.target, {
      composerTextareaFocused: composerRef.value?.isTextareaFocused() ?? false,
      dialogLayerOpen: hasOpenDialogLayer(),
    })) return
    const files = collectClipboardFiles(e.clipboardData)
    if (files.length === 0) return
    if (replanActive.value) {
      e.preventDefault()
      pushToast(t('chat.plan.attachmentsUnavailable'), { tone: 'warn' })
      return
    }
    void addAttachments(files)
    // File managers and screenshot tools put both the file and its name/path as
    // text on the clipboard; once we have attached the files, suppress the
    // default paste so that text is not also dumped into the composer (and then
    // sent to the agent). Plain-text pastes with no file fall through unchanged.
    e.preventDefault()
  }

  return {
    onChatDragEnter,
    onChatDragOver,
    onChatDragLeave,
    onChatDrop,
    onDocumentPaste,
  }
}
