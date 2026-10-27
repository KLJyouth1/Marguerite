// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, nextTick, type App } from 'vue'
import i18n from '@/i18n'
import LandingHero from './LandingHero.vue'

const apps: App<Element>[] = []

afterEach(() => {
  apps.splice(0).forEach(app => app.unmount())
  document.body.innerHTML = ''
})

async function mountHero(props: Record<string, unknown> = {}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(LandingHero, props, { model: () => h('button', { class: 'slot-model' }, 'model') }),
  })
  app.use(i18n)
  app.mount(host)
  apps.push(app)
  await nextTick()
  return host
}

describe('LandingHero', () => {
  it('把项目与分支作为只读 chip 展示，模型走注入的 slot', async () => {
    const host = await mountHero({
      projectName: 'marguerite',
      projectPath: 'D:/work/marguerite',
      branch: 'master',
    })

    expect(host.querySelector('.landing-hero__title')?.textContent)
      .toBe(i18n.global.t('chat.landingHero.title'))
    expect(host.querySelector('[data-testid="landing-project"]')?.textContent).toContain('marguerite')
    // 完整路径只在 title 里，不占首屏文字
    expect(host.querySelector('[data-testid="landing-project"]')?.getAttribute('title'))
      .toBe('D:/work/marguerite')
    expect(host.querySelector('[data-testid="landing-branch"]')?.textContent).toContain('master')
    expect(host.querySelector('.slot-model')).not.toBeNull()
  })

  it('未绑定项目时如实说明，而不是留空或猜一个目录', async () => {
    const host = await mountHero()

    expect(host.querySelector('[data-testid="landing-project"]')?.textContent)
      .toContain(i18n.global.t('chat.landingHero.unbound'))
    expect(host.querySelector('[data-testid="landing-branch"]')).toBeNull()
  })

  it('分离 HEAD 时显示状态，不显示空的分支 chip', async () => {
    const host = await mountHero({ projectName: 'marguerite', detached: true })

    expect(host.querySelector('[data-testid="landing-branch"]')?.textContent)
      .toContain(i18n.global.t('chat.landingHero.detached'))
  })
})
