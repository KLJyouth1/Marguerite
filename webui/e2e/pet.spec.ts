import { test, expect, type Locator } from '@playwright/test'

// 桌宠双端握手（/control/pet 页面 + /pet-overlay 悬浮窗）。
//
// 渲染契约（当前产品）：舞台挂载**恰好一台**渲染器 —— WebGL 可用时是真三维
// `canvas.pet-3d-canvas`（容器带 `data-pet3d="ready"`），不可用/构造失败时如实回退
// `svg.pet-svg-canvas`。旧断言钉的是「只许 SVG、不许 canvas」，3D 成为默认后全红。
// 皮肤目录为 6 款（cat + 4 精灵 + princess），由前端 skins.ts 与 RPC 契约共同决定。
//
// 前置条件：统一网关（marguerite gateway）已启动并托管 webui/dist + pet.* RPC。
const CONTROL_URL = '/control/'

function rendererOf(stage: Locator): Locator {
  return stage.locator('svg.pet-svg-canvas, canvas.pet-3d-canvas')
}

test.describe('Pet 3D 页面握手（/control/pet）', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(CONTROL_URL + 'pet')
    // 等待 WebSocket 就绪（RPC 通道）
    await page.waitForSelector('.conn-pill', { timeout: 15000 })
    await expect(page.locator('.pet3d-stage')).toBeVisible()
    await expect(rendererOf(page.locator('.pet3d-stage'))).toHaveCount(1, { timeout: 15000 })
  })

  test('pet.list RPC → 6 款皮肤卡片（1 猫 + 4 精灵 + 公主）', async ({ page }) => {
    const cards = page.locator('.pet-skin-card')
    await expect(cards).toHaveCount(6)
    const names = (await page.locator('.pet-skin-card__name').allTextContents()).join(' ')
    expect(names).toMatch(/猫/)
    for (const k of ['雏菊', '樱花', '薰衣草', '四叶草', '玛格丽特公主']) {
      expect(names).toContain(k)
    }
  })

  test('舞台挂载真实渲染器：3D canvas 或 2.5D SVG（二选一，不得并存/空白）', async ({ page }) => {
    const stage = page.locator('.pet3d-stage')
    const canvases = stage.locator('canvas.pet-3d-canvas')
    const svgs = stage.locator('svg.pet-svg-canvas')
    expect((await canvases.count()) + (await svgs.count())).toBe(1)

    if ((await canvases.count()) === 1) {
      // 三维：就绪标记 + 真实画布尺寸 + 模型种类（非空壳）
      await expect(stage).toHaveAttribute('data-pet3d', 'ready', { timeout: 15000 })
      await expect(stage).toHaveAttribute('data-pet3d-kind', /./)
      await expect(stage).toHaveClass(/is-3d/)
      const canvas = canvases.first()
      await expect(canvas).toBeVisible()
      const box = await canvas.boundingBox()
      expect(box && box.width > 0 && box.height > 0).toBeTruthy()
      const backing = await canvas.evaluate((el) => {
        const c = el as HTMLCanvasElement
        return { w: c.width, h: c.height }
      })
      expect(backing.w, 'canvas 后备缓冲宽度').toBeGreaterThan(0)
      expect(backing.h, 'canvas 后备缓冲高度').toBeGreaterThan(0)
      // 只允许一台渲染器：三维舞台上不得叠着 SVG 降级层
      expect(await stage.locator('svg.pet-svg-canvas').count()).toBe(0)
    } else {
      // 2.5D 矢量：viewBox 恒定 200×200，几何节点足够多
      const svg = svgs.first()
      await expect(svg).toBeVisible()
      await expect(svg).toHaveAttribute('viewBox', '0 0 200 200')
      const shapes = await svg.evaluate(
        (el) => el.querySelectorAll('path, ellipse, circle, rect').length,
      )
      expect(shapes, 'SVG 主体几何不得为空壳').toBeGreaterThan(10)
      // 只允许一台渲染器：矢量舞台上不得残留 canvas
      expect(await stage.locator('canvas').count()).toBe(0)
    }
  })

  test('情绪与数值来自 pet.status（观察口真实接线，不是死代码）', async ({ page }) => {
    // 心情标签对两种渲染器都渲染（3D 不往 canvas 写 data-*，观察口在 .pet-mood）
    const mood = page.locator('.pet-mood')
    await expect(mood).toBeVisible()
    await expect(mood).toHaveText(/\S+/)
    const stats = page.locator('.pet-stats')
    await expect(stats).toBeVisible()
    await expect(stats).toHaveText(/能量 \d+\/100/)
    await expect(stats).toHaveText(/好感度 \d+/)

    // SVG 渲染器额外把心情/行为写在根节点 data-* 上（供 CSS 与测试观察）
    const svg = page.locator('.pet3d-stage svg.pet-svg-canvas')
    if ((await svg.count()) === 1) {
      await expect(svg).toHaveAttribute('data-mood', /./)
      await expect(svg).toHaveAttribute('data-behaviour', /./)
    }
  })

  test('视口缩放后渲染器仍完整成形', async ({ page }) => {
    const stage = page.locator('.pet3d-stage')
    const before = await rendererOf(stage).boundingBox()
    expect(before && before.width > 0 && before.height > 0).toBeTruthy()

    await page.setViewportSize({ width: 900, height: 700 })
    await expect(rendererOf(stage)).toBeVisible()
    const after = await rendererOf(stage).boundingBox()
    expect(after && after.width > 0 && after.height > 0).toBeTruthy()

    // 矢量契约：viewBox 恒定 200×200，不随视口变形；3D 契约：就绪标记保持
    const svg = page.locator('.pet3d-stage svg.pet-svg-canvas')
    if ((await svg.count()) === 1) {
      await expect(svg).toHaveAttribute('viewBox', '0 0 200 200')
    } else {
      await expect(stage).toHaveAttribute('data-pet3d', 'ready')
    }
  })

  // 悬停 → 局部聚焦注视：断言的是交互契约（聚焦态进出 + 渲染器仍在），
  // 「聚焦改变了什么」由 petPose / petSpec 单测负责。
  test('悬停 → 聚焦态进出（局部注视已接线，不是死代码）', async ({ page }) => {
    const stage = page.locator('.pet3d-stage')
    await expect(stage).not.toHaveClass(/is-focused/)

    await stage.hover()
    await expect(stage).toHaveClass(/is-focused/)

    await page.mouse.move(4, 4)
    await expect(stage).not.toHaveClass(/is-focused/)

    // 聚焦切换不得破坏渲染循环
    await expect(rendererOf(stage)).toBeVisible()
    const box = await rendererOf(stage).boundingBox()
    expect(box && box.width > 0 && box.height > 0).toBeTruthy()
  })

  test('顺毛抚摸 → pet.triggerAction 往返后状态更新且无错误横幅', async ({ page }) => {
    const stats = page.locator('.pet-stats')
    await expect(stats).toBeVisible()
    const before = await stats.textContent()

    await page.getByRole('button', { name: '顺毛抚摸' }).click()

    // 失败会落到 .pet-error（trigger 的真实错误面），先钉住「没有失败」
    await expect(page.locator('.pet-error')).toHaveCount(0)
    // RPC 往返必须更新能量/好感度；轮询等待避免与动画/加载竞速
    await expect
      .poll(async () => stats.textContent(), { timeout: 8000 })
      .not.toBe(before)
    await expect(stats).toHaveText(/能量 \d+\/100/)
  })
})

test.describe('Pet Overlay 悬浮窗握手（/pet-overlay）', () => {
  test.beforeEach(async ({ page }) => {
    // 桌面平台：注入 MargueriteDesktop 使 /pet-overlay 路由可达（与 Tauri 壳加载一致）
    await page.addInitScript(() => {
      ;(window as unknown as Record<string, unknown>).MargueriteDesktop = true
    })
    await page.goto(CONTROL_URL + 'pet-overlay')
    // 悬浮窗刻意隐藏整个应用外壳（topbar 不可见是设计），所以只等连接点挂载，
    // 不能等它可见 —— 旧写法等「可见 .conn-pill」在这条路由上永不成立。
    await page.waitForSelector('.conn-pill', { state: 'attached', timeout: 15000 })
    await expect(page.locator('.pet-overlay__stage')).toBeVisible()
    await expect(rendererOf(page.locator('.pet-overlay__stage'))).toHaveCount(1, { timeout: 15000 })
  })

  test('透明舞台 + 独立渲染器挂载（3D 或 2.5D，二选一）', async ({ page }) => {
    const overlay = page.locator('.pet-overlay')
    await expect(overlay).toBeVisible()
    // 悬浮窗视觉前提：overlay 根背景透明（真实 Tauri 窗口透明由装饰关闭 + html/body 透明决定）
    const overlayBg = await overlay.evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(overlayBg).toMatch(/^(rgba\(0, 0, 0, 0\)|transparent)$/)

    const stage = page.locator('.pet-overlay__stage')
    const canvases = stage.locator('canvas.pet-3d-canvas')
    const svgs = stage.locator('svg.pet-svg-canvas')
    expect((await canvases.count()) + (await svgs.count())).toBe(1)

    if ((await canvases.count()) === 1) {
      await expect(stage).toHaveAttribute('data-pet3d', 'ready', { timeout: 15000 })
      await expect(stage).toHaveClass(/is-3d/)
    } else {
      await expect(svgs.first()).toBeVisible()
    }
    const box = await rendererOf(stage).boundingBox()
    expect(box && box.width > 0 && box.height > 0).toBeTruthy()
  })

  test('右键菜单 → 互动/隐藏/设置三项（设置如实禁用）', async ({ page }) => {
    await page.locator('.pet-overlay').click({ button: 'right' })
    const menu = page.locator('.pet-overlay__menu')
    await expect(menu).toBeVisible()
    // 悬浮窗是独立 webview：没有路由与主窗口句柄，设置项如实禁用而不是假装可用
    const items = menu.locator('.pet-overlay__menu-item')
    await expect(items).toHaveCount(3)
    await expect(items.nth(2)).toBeDisabled()
    await expect(items.nth(2)).toHaveAttribute('aria-disabled', 'true')
  })

  test('双击舞台 → 抚摸 RPC 往返（好感度递增）', async ({ page }) => {
    const capsule = page.locator('.pet-overlay__capsule')
    await expect(capsule).toBeVisible({ timeout: 15000 })
    const affection = capsule.locator('.pet-overlay__affection')
    const before = await affection.textContent()
    await page.locator('.pet-overlay__stage').dblclick()
    await expect
      .poll(async () => affection.textContent(), { timeout: 8000 })
      .not.toBe(before)
    await expect(affection).toHaveText(/♥\s*\d+/)
  })

  test('心情胶囊随 pet.status 渲染（左下有真实数值）', async ({ page }) => {
    const capsule = page.locator('.pet-overlay__capsule')
    await expect(capsule).toBeVisible({ timeout: 15000 })
    await expect(capsule.locator('.pet-overlay__affection')).toHaveText(/♥\s*\d+/)
    await expect(capsule.locator('.pet-overlay__energy')).toHaveText(/⚡\s*\d+/)
  })
})
