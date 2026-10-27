// @vitest-environment happy-dom

import { afterEach, describe, expect, it } from 'vitest'
import { createApp, nextTick } from 'vue'
import i18n from '@/i18n'
import PendingQueue from './PendingQueue.vue'
import type { Attachment } from '@/types/chat'

afterEach(() => {
  document.body.innerHTML = ''
  i18n.global.locale.value = 'en'
})

async function mountQueue(
  listeners: Record<string, (...args: number[]) => void> = {},
  items: Array<{
    text: string
    deliveryState?: 'steering' | 'retryable'
    attachments?: Attachment[]
    hiddenControl?: boolean
    displayTextOverride?: string
  }> = [
    { text: 'Follow the latest instruction' },
  ],
  props: { imageBlockedMessage?: string; steerAvailable?: boolean } = {},
) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const app = createApp(PendingQueue, {
    items,
    maxPending: 5,
    steerAvailable: true,
    ...listeners,
    ...props,
  })
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return { app, el }
}

describe('PendingQueue', () => {
  it('keeps the original steer affordance visible but disabled when capability is unavailable', async () => {
    const { app, el } = await mountQueue({}, undefined, { steerAvailable: false })

    const steer = el.querySelector<HTMLButtonElement>('.chat-pending-action--steer')
    expect(steer?.textContent).toContain('Steer')
    expect(steer?.disabled).toBe(true)
    expect(steer?.title).toContain('queues for after current response')
    expect(el.querySelector('[aria-label="Remove pending message 1"]')).not.toBeNull()
    app.unmount()
  })

  it('offers steer, remove, and quiet overflow actions on each queued message', async () => {
    let steered = 0
    let removed = 0
    const { app, el } = await mountQueue({
      onSteer: () => { steered += 1 },
      onRemove: () => { removed += 1 },
    })

    const steer = [...el.querySelectorAll<HTMLButtonElement>('button')]
      .find(button => button.textContent?.includes('Steer'))
    steer?.click()
    el.querySelector<HTMLButtonElement>('[aria-label="Remove pending message 1"]')?.click()

    expect(steered).toBe(1)
    expect(removed).toBe(1)
    expect(el.querySelector('.chat-pending-card')).not.toBeNull()
    app.unmount()
  })

  it('marks a steering item busy and disables every destructive or duplicate action', async () => {
    let steered = 0
    let removed = 0
    const { app, el } = await mountQueue({
      onSteer: () => { steered += 1 },
      onRemove: () => { removed += 1 },
    }, [{ text: 'Already steering', deliveryState: 'steering' }])

    expect(el.querySelector('.chat-pending-card')?.getAttribute('aria-busy')).toBe('true')
    const actions = [...el.querySelectorAll<HTMLButtonElement>('.chat-pending-actions button')]
    expect(actions).toHaveLength(6)
    expect(actions.every(button => button.disabled)).toBe(true)

    actions.forEach(button => button.click())
    await nextTick()
    expect(steered).toBe(0)
    expect(removed).toBe(0)
    expect(el.querySelector('[role="menu"]')).toBeNull()
    app.unmount()
  })

  it('keeps a retryable item available for an explicit retry', async () => {
    let steered = 0
    let edited = 0
    const { app, el } = await mountQueue({
      onSteer: () => { steered += 1 },
      onEdit: () => { edited += 1 },
    }, [{ text: 'Retry this steer', deliveryState: 'retryable' }])

    expect(el.querySelector('.chat-pending-card')?.hasAttribute('aria-busy')).toBe(false)
    const retry = [...el.querySelectorAll<HTMLButtonElement>('button')]
      .find(button => button.textContent?.includes('Retry'))
    expect(retry?.disabled).toBe(false)
    expect(retry?.title).toBe('Retry')
    retry?.click()
    expect(steered).toBe(1)

    el.querySelector<HTMLButtonElement>('[aria-label="More"]')?.click()
    await nextTick()
    const edit = [...el.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
      .find(button => button.textContent?.includes('Edit message'))
    expect(edit?.disabled).toBe(true)
    edit?.click()
    expect(edited).toBe(0)
    app.unmount()
  })

  it('keeps hidden control input removable without exposing same-turn retry', async () => {
    let retried = 0
    let removed = 0
    const { app, el } = await mountQueue({
      onSteer: () => { retried += 1 },
      onRemove: () => { removed += 1 },
    }, [{
      text: 'provider-only marker',
      displayTextOverride: 'Confirmed',
      hiddenControl: true,
      deliveryState: 'retryable',
    }])

    expect(el.querySelector('.chat-pending-text')?.textContent).toContain('Confirmed')
    const retry = [...el.querySelectorAll<HTMLButtonElement>('button')]
      .find(button => button.textContent?.includes('Retry'))
    expect(retry).toBeUndefined()
    el.querySelector<HTMLButtonElement>('[aria-label="Remove pending message 1"]')?.click()

    expect(retried).toBe(0)
    expect(removed).toBe(1)
    expect(el.querySelector('[aria-label="More"]')).toBeNull()
    app.unmount()
  })

  it.each(['/status', '!pwd'])(
    'keeps the original affordance disabled for queued control input %s',
    async (text) => {
      let steered = 0
      const { app, el } = await mountQueue({
        onSteer: () => { steered += 1 },
      }, [{ text }])

      const steer = el.querySelector<HTMLButtonElement>('.chat-pending-action--steer')
      expect(steer?.textContent).toContain('Steer')
      expect(steer?.disabled).toBe(true)
      expect(steered).toBe(0)
      app.unmount()
    },
  )

  it('allows only one queued delivery lease at a time', async () => {
    const { app, el } = await mountQueue({}, [
      { text: 'In flight', deliveryState: 'steering' },
      { text: 'Must wait' },
    ])

    const steerButtons = [...el.querySelectorAll<HTMLButtonElement>(
      '.chat-pending-action--steer',
    )]
    expect(steerButtons).toHaveLength(2)
    expect(steerButtons.every(button => button.disabled)).toBe(true)
    app.unmount()
  })

  it('keeps the original affordance disabled for a queued item with an attachment', async () => {
    const failed: Attachment = {
      kind: 'failed',
      local_id: 7,
      name: 'failed.pdf',
      mime: 'application/pdf',
      error: 'upload failed',
    }
    const { app, el } = await mountQueue({}, [{
      text: 'Keep this attachment',
      attachments: [failed],
    }])

    const steer = el.querySelector<HTMLButtonElement>('.chat-pending-action--steer')
    expect(steer?.disabled).toBe(true)
    expect(steer?.title).toContain('failed attachment')
    const describedBy = steer?.getAttribute('aria-describedby')
    expect(describedBy).toBeTruthy()
    expect(el.querySelector('.chat-pending-attachment-status')?.textContent)
      .toContain('retry or remove the failed attachment')
    expect(el.querySelector('.chat-pending-attachments')?.textContent)
      .toContain('Needs attention')
    app.unmount()
  })

  it('explains why current routing blocks a queued image', async () => {
    const image: Attachment = {
      kind: 'staged',
      local_id: 8,
      name: 'diagram.png',
      mime: 'image/png',
      file_uuid: 'image-8',
    }
    const blockedMessage = 'Ensemble mode does not support image attachments.'
    const { app, el } = await mountQueue({}, [{
      text: 'Review this image',
      attachments: [image],
    }], { imageBlockedMessage: blockedMessage })

    const steer = el.querySelector<HTMLButtonElement>('.chat-pending-action--steer')
    expect(steer?.disabled).toBe(true)
    expect(steer?.title).toBe(blockedMessage)
    const describedBy = steer?.getAttribute('aria-describedby')
    expect(el.querySelector(`#${describedBy}`)?.textContent).toContain(blockedMessage)
    expect(el.querySelector('.chat-pending-attachment-status')?.textContent)
      .toContain(blockedMessage)
    app.unmount()
  })

  it('offers send-now and reorder controls on each movable item', async () => {
    let sentNow = 0
    const moves: Array<[number, number]> = []
    const { app, el } = await mountQueue({
      onSendNow: () => { sentNow += 1 },
      onMove: (index: number, delta: number) => { moves.push([index, delta]) },
    }, [{ text: 'first' }, { text: 'second' }])

    const sendNow = [...el.querySelectorAll<HTMLButtonElement>('.chat-pending-action--send-now')]
    expect(sendNow).toHaveLength(2)
    sendNow[1]?.click()
    expect(sentNow).toBe(1)

    const cards = [...el.querySelectorAll<HTMLElement>('.chat-pending-card')]
    const upOf = (card: HTMLElement) =>
      card.querySelector<HTMLButtonElement>('[aria-label="Move up"]')
    const downOf = (card: HTMLElement) =>
      card.querySelector<HTMLButtonElement>('[aria-label="Move down"]')
    // 队首不能上移、队尾不能下移，中间两侧都可用
    expect(upOf(cards[0])?.disabled).toBe(true)
    expect(downOf(cards[0])?.disabled).toBe(false)
    expect(upOf(cards[1])?.disabled).toBe(false)
    expect(downOf(cards[1])?.disabled).toBe(true)

    downOf(cards[0])?.click()
    upOf(cards[1])?.click()
    expect(moves).toEqual([[0, 1], [1, -1]])
    app.unmount()
  })

  it('keeps edit and clear-all inside the overflow menu', async () => {
    let edited = 0
    let cleared = 0
    const { app, el } = await mountQueue({
      onEdit: () => { edited += 1 },
      onClear: () => { cleared += 1 },
    })

    el.querySelector<HTMLButtonElement>('[aria-label="More"]')?.click()
    await nextTick()
    expect(el.querySelector('[role="menu"]')).not.toBeNull()

    const buttons = [...el.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
    buttons.find(button => button.textContent?.includes('Edit message'))?.click()
    expect(edited).toBe(1)

    el.querySelector<HTMLButtonElement>('[aria-label="More"]')?.click()
    await nextTick()
    ;[...el.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
      .find(button => button.textContent?.includes('Clear queue'))
      ?.click()
    expect(cleared).toBe(1)
    app.unmount()
  })
})
