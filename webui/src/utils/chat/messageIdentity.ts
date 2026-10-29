import type { ChatRenderedMessage } from '@/types/chat'

export function chatMessageKey(message: ChatRenderedMessage, index: number): string {
  if (message.isRouterStrip && message.routerTurnKey) return message.routerTurnKey
  // `clientId` first: an optimistic row keeps that id for its whole life, while
  // `messageId` only appears when `chat.send` is accepted. Keying on messageId
  // first made the row's :key change mid-turn → Vue tore down and re-created the
  // message DOM (scroll jump, collapsed artifacts reopening, inline editor lost).
  return message.clientId || message.messageId || message.id || `${message.displayRole || message.role}-${message.sourceIndex ?? index}`
}

let clientMessageSequence = 0
let clientRequestSequence = 0

export function createClientMessageId(): string {
  clientMessageSequence += 1
  return `local-${Date.now().toString(36)}-${clientMessageSequence.toString(36)}`
}

export function createClientRequestId(): string {
  if (typeof globalThis.crypto?.randomUUID === 'function') {
    return globalThis.crypto.randomUUID()
  }
  clientRequestSequence += 1
  return `web-${Date.now().toString(36)}-${clientRequestSequence.toString(36)}`
}

export function isShareableChatMessage(message: ChatRenderedMessage): boolean {
  if (message.stopNotice) return false
  return message.displayRole === 'user' || message.displayRole === 'assistant'
}
