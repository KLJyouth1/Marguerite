// 产物与分享导出控制器（从 ChatView.vue 抽出的本地编排函数）。
// 聚合：下载附件/产物、交付物抽屉、行内产物聚焦、产物打开、分享模式与分享预览。
// shareTitle 因早于 currentChatTitle 构造且仅为透传，保留在 ChatView 装配层。
import { type Ref, type ComputedRef, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { useToasts } from '@/composables/useToasts'
import { useArtifactImageLightbox } from '@/composables/chat/useArtifactImageLightbox'
import { useWorkbenchStore } from '@/workbench/store'
import { usePlatform } from '@/platform'
import type { ChatViewState } from '../../views/chat/useChatViewState'
import type { ChatHeaderActionsHandle } from '../../views/chat/chat-view.types'
import type { ArtifactPayload } from '@/types/rpc'
import type { DisplayAttachment } from '@/types/chat'
import type { ShareExportTheme, ShareImageResult } from '@/composables/chat/useChatShareExport'
import { fetchDisplayAttachmentBlob } from '@/utils/chat/attachmentAccess'
import { downloadBlob, copyImageToClipboard } from '@/utils/browser'
import {
  artifactCategory,
  artifactDownloadUrl,
  isInlineMediaArtifact,
} from '@/utils/chat/artifacts'
import {
  artifactFromWorkbenchItem,
  createArtifactPreviewWorkbenchItem,
} from '@/workbench/artifactItems'
import {
  artifactUsesWorkbenchPreview,
  artifactWorkbenchPreviewKind,
} from '@/utils/workbench/artifactPreview'
import { focusArtifactInTranscript } from '@/utils/chat/artifactFocus'

export interface ChatArtifactShareControllerDeps {
  chatShareExport: {
    buildShareImage: (
      ids: Set<string>,
      opts?: { theme?: ShareExportTheme },
    ) => Promise<ShareImageResult | null>
  }
  artifactImageLightbox: ReturnType<typeof useArtifactImageLightbox>
  workbenchStore: ReturnType<typeof useWorkbenchStore>
  platform: ReturnType<typeof usePlatform>
  workbenchEnabled: Readonly<ComputedRef<boolean>>
  chatHeaderActionsRef: Ref<ChatHeaderActionsHandle | null>
  shareBannerRef: Ref<HTMLElement | null>
  sessionArtifacts: Readonly<Ref<ArtifactPayload[]>>
  sessionWorkbenchArtifacts: Readonly<ComputedRef<ArtifactPayload[]>>
  shareableMessageCount: Readonly<ComputedRef<number>>
  readAuthToken: () => string
}

export function useChatArtifactShareController(
  state: ChatViewState,
  deps: ChatArtifactShareControllerDeps,
) {
  const { t } = useI18n()
  const { pushToast } = useToasts()
  const {
    threadRef,
    sessionKey,
    shareMode,
    selectedShareMessageIds,
    sharePreview,
    shareTheme,
    shareSaving,
    deliverablesOpen,
  } = state
  const {
    chatShareExport,
    artifactImageLightbox,
    workbenchStore,
    platform,
    workbenchEnabled,
    chatHeaderActionsRef,
    shareBannerRef,
    sessionArtifacts,
    sessionWorkbenchArtifacts,
    shareableMessageCount,
    readAuthToken,
  } = deps

  async function downloadAttachment(attachment: DisplayAttachment): Promise<boolean> {
    const result = await fetchDisplayAttachmentBlob(attachment, {
      baseOrigin: window.location.origin,
      sessionKey: sessionKey.value,
      authToken: readAuthToken(),
    })
    if (!result.ok) {
      if (result.status > 0) {
        pushToast(t('chat.toast.downloadFailedHttp', { status: result.status }), { tone: 'danger' })
      } else {
        pushToast(t('chat.toast.downloadFailed'), { tone: 'danger' })
      }
      return false
    }
    downloadBlob(result.blob, result.filename)
    return true
  }

  async function downloadArtifact(artifact: ArtifactPayload) {
    const token = readAuthToken()
    const url = artifactDownloadUrl(artifact, window.location.origin, {
      sessionKey: sessionKey.value,
      includeSessionKey: false,
    })
    if (!url) return
    try {
      const headers: Record<string, string> = {}
      const sameOrigin = new URL(url, window.location.origin).origin === window.location.origin
      if (sameOrigin && sessionKey.value) headers['x-Marguerite-session-key'] = sessionKey.value
      if (sameOrigin && token) headers.Authorization = `Bearer ${token}`
      const response = await fetch(url, {
        method: 'GET',
        headers,
        credentials: sameOrigin ? 'same-origin' : 'omit',
      })
      if (!response.ok) {
        pushToast(t('chat.toast.downloadFailedHttp', { status: response.status }), { tone: 'danger' })
        return
      }
      const blob = await response.blob()
      downloadBlob(blob, artifact.name || 'artifact')
    } catch (err) {
      console.warn('Download failed:', err)
      pushToast(t('chat.toast.downloadFailed'), { tone: 'danger' })
    }
  }

  function focusHeaderAction(
    action: 'deliverables' | 'share' | 'copy-session-key',
  ) {
    void nextTick(() => chatHeaderActionsRef.value?.focusAction(action))
  }

  function openDeliverables() {
    if (sessionArtifacts.value.length === 0) return
    const allArtifactsUseWorkbench = sessionWorkbenchArtifacts.value.length
      === sessionArtifacts.value.length
    if (workbenchEnabled.value && allArtifactsUseWorkbench) {
      const recentPreview = workbenchStore.findMostRecentItem(item => {
        if (
          item.kind !== 'artifact-preview'
          || item.scope.type !== 'session'
          || item.scope.id !== sessionKey.value
        ) return false
        const artifact = artifactFromWorkbenchItem(item)
        if (!artifact) return false
        return artifactUsesWorkbenchPreview(artifact)
      })
      if (recentPreview) {
        workbenchStore.activateItem(recentPreview.id)
        workbenchStore.setExpanded(true)
        return
      }

      for (let index = sessionWorkbenchArtifacts.value.length - 1; index >= 0; index -= 1) {
        const artifact = sessionWorkbenchArtifacts.value[index]
        if (!artifact) continue
        openArtifact(artifact)
        return
      }
    }
    deliverablesOpen.value = true
  }

  function focusInlineDeliverable(artifact: ArtifactPayload): boolean {
    const reduceMotion = typeof window !== 'undefined'
      && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
    return focusArtifactInTranscript(
      threadRef.value,
      artifact,
      reduceMotion ? 'auto' : 'smooth',
    )
  }

  function openArtifact(artifact: ArtifactPayload): boolean {
    // Generated images are also inline media. Route every visual artifact to
    // the authenticated lightbox before the inline-focus fallback so clicking
    // either the thumbnail or its open affordance actually previews it.
    if (artifactCategory(artifact) === 'visual' && sessionKey.value) {
      artifactImageLightbox.open({
        artifact,
        navigationArtifacts: sessionArtifacts.value,
        sessionKey: sessionKey.value,
      })
      return true
    }
    if (
      isInlineMediaArtifact(artifact)
      || artifactWorkbenchPreviewKind(artifact) === 'unsupported'
    ) {
      return focusInlineDeliverable(artifact)
    }
    if (!workbenchEnabled.value || !sessionKey.value) return false
    const opened = workbenchStore.openItem(createArtifactPreviewWorkbenchItem({
      artifact,
      navigationArtifacts: sessionArtifacts.value,
      nativeHtml: Boolean(
        platform.capabilities.hasNativeWorkbenchSurfaces
        && platform.workbench.native,
      ),
      sessionKey: sessionKey.value,
    }))
    if (!opened) {
      pushToast(t('workbench.itemLimitReached'), { tone: 'warn', duration: 6000 })
    }
    return opened
  }

  function closeDeliverables() {
    deliverablesOpen.value = false
    focusHeaderAction('deliverables')
  }

  function startShareMode() {
    if (shareableMessageCount.value === 0) return
    shareMode.value = true
    selectedShareMessageIds.value = new Set()
    nextTick(() => shareBannerRef.value?.focus())
  }

  function endShareMode() {
    // Exiting tears down the banner and the bubble pickers; if focus was inside
    // ANY of that mode UI it would drop to <body>, so return it to the entry
    // button in every case.
    const active = document.activeElement
    const modeUiHadFocus = !!shareBannerRef.value?.contains(active)
      || !!(active instanceof HTMLElement
        && active.closest('[data-share-control], .msg-user--share-mode, .msg-ai--share-mode'))
    shareMode.value = false
    selectedShareMessageIds.value = new Set()
    // Leaving share mode invalidates any open preview (the selection it rendered
    // is gone), so drop the modal and its object URL alongside the mode.
    if (sharePreview.value) {
      URL.revokeObjectURL(sharePreview.value.url)
      sharePreview.value = null
    }
    if (modeUiHadFocus) focusHeaderAction('share')
  }

  function toggleShareMessage(messageId: string) {
    const next = new Set(selectedShareMessageIds.value)
    if (next.has(messageId)) next.delete(messageId)
    else next.add(messageId)
    selectedShareMessageIds.value = next
  }

  // Save renders the selected bubbles to a PNG blob and opens the preview modal;
  // it no longer downloads directly. Share mode stays active while previewing so
  // the user can still adjust the selection after closing the modal — it only
  // ends once they commit with Download.
  async function saveShareImage() {
    if (selectedShareMessageIds.value.size === 0 || shareSaving.value) return
    shareSaving.value = true
    try {
      await nextTick()
      const result = await chatShareExport.buildShareImage(selectedShareMessageIds.value, {
        theme: shareTheme.value,
      })
      if (!result) {
        pushToast(t('chat.toast.shareSaveFailed'), { tone: 'danger' })
        return
      }
      const url = URL.createObjectURL(result.blob)
      sharePreview.value = { url, blob: result.blob, filename: result.filename }
    } catch (err) {
      console.warn('Share image export failed:', err)
      pushToast(t('chat.toast.shareSaveFailed'), { tone: 'danger' })
    } finally {
      shareSaving.value = false
    }
  }

  function onShareDownload() {
    const preview = sharePreview.value
    if (!preview) return
    downloadBlob(preview.blob, preview.filename)
    pushToast(t('chat.toast.saved', { filename: preview.filename }), { duration: 4000 })
    // endShareMode revokes the preview URL and drops the modal. The modal's
    // Download button held focus outside the banner, so restore the best visible
    // Share entry (or the stable session-actions trigger) explicitly.
    endShareMode()
    focusHeaderAction('share')
  }

  async function onShareCopy() {
    const preview = sharePreview.value
    if (!preview) return
    const ok = await copyImageToClipboard(preview.blob)
    // Approved decision: the modal stays open after a copy so the user can copy
    // again or then download; only Download / Cancel / Escape closes it.
    pushToast(ok ? t('chat.toast.copiedToClipboard') : t('chat.toast.copyNotSupported'), {
      tone: ok ? undefined : 'danger',
    })
  }

  // Re-render the image in the chosen theme, swapping the object URL in place so
  // the modal stays open and shows a busy state during the rebuild.
  async function onShareSetTheme(next: ShareExportTheme) {
    if (next === shareTheme.value && sharePreview.value) return
    shareTheme.value = next
    if (!sharePreview.value || shareSaving.value) return
    shareSaving.value = true
    try {
      const result = await chatShareExport.buildShareImage(selectedShareMessageIds.value, { theme: next })
      if (!result) {
        pushToast(t('chat.toast.sharePreviewUpdateFailed'), { tone: 'danger' })
        return
      }
      const previous = sharePreview.value
      sharePreview.value = {
        url: URL.createObjectURL(result.blob),
        blob: result.blob,
        filename: result.filename,
      }
      if (previous) URL.revokeObjectURL(previous.url)
    } catch (err) {
      console.warn('Share image re-render failed:', err)
      // Not shareSaveFailed: nothing was being saved here — the theme switch
      // only re-renders the preview, so the copy must name that action.
      pushToast(t('chat.toast.sharePreviewUpdateFailed'), { tone: 'danger' })
    } finally {
      shareSaving.value = false
    }
  }

  // Close the preview without leaving share mode: revoke the URL and restore
  // focus. While share mode is still active the header Share button is unmounted
  // (v-if="!shareMode"), so focus returns to the share banner — the mode's anchor
  // and where startShareMode put it; only once the mode has ended does the entry
  // button exist to receive focus.
  function closeSharePreview() {
    const preview = sharePreview.value
    if (preview) URL.revokeObjectURL(preview.url)
    sharePreview.value = null
    nextTick(() => {
      if (shareMode.value) shareBannerRef.value?.focus()
      else chatHeaderActionsRef.value?.focusAction('share')
    })
  }

  return {
    downloadAttachment,
    downloadArtifact,
    focusHeaderAction,
    openDeliverables,
    focusInlineDeliverable,
    openArtifact,
    closeDeliverables,
    startShareMode,
    endShareMode,
    toggleShareMessage,
    saveShareImage,
    onShareDownload,
    onShareCopy,
    onShareSetTheme,
    closeSharePreview,
  }
}
