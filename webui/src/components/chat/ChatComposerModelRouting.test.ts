// @vitest-environment happy-dom
import { defineComponent, h, nextTick, ref } from 'vue'
import { describe, expect, it } from 'vitest'
import { createApp } from 'vue'
import i18n from '@/i18n'
import type { BloomSubmode, ModelRoutingMode } from '@/types/modelRouting'
import ChatComposerModelRouting from './ChatComposerModelRouting.vue'

async function mountRouting(options: { modelRoutingMode: ModelRoutingMode; bloomSubmode?: BloomSubmode; busy?: boolean }) {
  const el = document.createElement('div')
  document.body.appendChild(el)
  const emitted = ref<string[]>([])
  const Harness = defineComponent({
    setup() {
      return () => h(ChatComposerModelRouting, {
        modelRoutingMode: options.modelRoutingMode,
        bloomSubmode: options.bloomSubmode ?? 'deep',
        busy: options.busy ?? false,
        onSetModelRoutingMode: (mode: string) => emitted.value.push(mode),
      })
    },
  })
  const app = createApp(Harness)
  app.use(i18n)
  app.mount(el)
  await nextTick()
  return { app, el, emitted }
}

describe('ChatComposerModelRouting', () => {
  it('renders the four routing modes including Bloom fusion routing', async () => {
    const { app, el } = await mountRouting({ modelRoutingMode: 'off' })

    const options = Array.from(el.querySelectorAll<HTMLButtonElement>('.composer-model-routing__option'))
    expect(options).toHaveLength(4)
    expect(options[0].getAttribute('role')).toBe('radio')
    expect(options.some(option => option.textContent?.includes('Bloom fusion routing'))).toBe(true)
    expect(el.textContent).toContain('Fusion routing + AI auto-orchestration')

    app.unmount()
    el.remove()
  })

  it('selects bloom_ensemble and emits the set event', async () => {
    const { app, el, emitted } = await mountRouting({ modelRoutingMode: 'llm_ensemble' })

    const bloomOption = Array.from(el.querySelectorAll<HTMLButtonElement>('.composer-model-routing__option'))
      .find(option => option.classList.contains('composer-model-routing__option--bloom_ensemble'))
    expect(bloomOption).toBeTruthy()
    expect(bloomOption?.getAttribute('aria-checked')).toBe('false')

    bloomOption?.click()
    await nextTick()

    expect(emitted.value).toEqual(['bloom_ensemble'])

    app.unmount()
    el.remove()
  })

  it('marks the active bloom_ensemble option as checked', async () => {
    const { app, el } = await mountRouting({ modelRoutingMode: 'bloom_ensemble' })

    const active = el.querySelector<HTMLButtonElement>('.composer-model-routing__option--bloom_ensemble.is-active')
    expect(active?.getAttribute('aria-checked')).toBe('true')

    app.unmount()
    el.remove()
  })
})
