import { test, expect } from '@playwright/test'

// V2.2-E2E：桌宠 2D SVG（伪 3D 光照）页面握手（/control/pet）与悬浮窗握手（/pet-overlay）
//
// 前置条件：后端统一网关（marguerite gateway）已启动并托管 webui/dist + pet.* RPC。
// 运行方式（与既有 e2e 同约定，baseURL 指向网关）：
//   OPENSQUILLA_WEBUI_BASE_URL=http://127.0.0.1:<网关端口> npx playwright test e2e/pet.spec.ts
const CONTROL_URL = '/control/'

test.describe('Pet 3D 页面握手（/control/pet）', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(CONTROL_URL + 'pet')
    // 等待 WebSocket 就绪（RPC 通道）
    await page.waitForSelector('.conn-pill', { timeout: 15000 })
  })

  test('pet.list RPC → 5 款皮肤卡片（1 猫 + 4 精灵）', async ({ page }) => {
    const cards = page.locator('.pet-skin-card')
    await expect(cards).toHaveCount(5)
    const names = (await page.locator('.pet-skin-card__name').allTextContents()).join(' ')
    expect(names).toMatch(/猫/)
    for (const k of ['雏菊', '樱花', '薰衣草', '四叶草']) {
      expect(names).toContain(k)
    }
  })

  test('舞台挂载 2D SVG（伪 3D 光照），WebGL 渲染器已退场', async ({ page }) => {
    const stage = page.locator('.pet3d-stage')
    await expect(stage).toBeVisible()
    const svg = stage.locator('svg.pet-svg-canvas')
    await expect(svg).toBeVisible({ timeout: 15000 })
    // 几何契约：viewBox 由 PET_SVG_SIZE 决定（200×200），像素缩放交给 CSS
    await expect(svg).toHaveAttribute('viewBox', '0 0 200 200')
    const box = await svg.boundingBox()
    expect(box && box.width > 0 && box.height > 0).toBeTruthy()
    // 真成形而非空壳：矢量节点足够多（主体/五官/分层都由几何构成）
    const shapes = await svg.evaluate((el) => el.querySelectorAll('path, ellipse, circle, rect').length)
    expect(shapes, 'SVG 主体几何不得为空壳').toBeGreaterThan(10)
    // 渲染层已整体切到 SVG：不得再有 canvas（否则是旧 WebGL 渲染器残留）
    expect(await stage.locator('canvas').count()).toBe(0)
  })

  test('行为态/心情落在 data-* 上（观察口真实接线，不是死代码）', async ({ page }) => {
    const svg = page.locator('.pet3d-stage svg.pet-svg-canvas')
    await expect(svg).toBeVisible({ timeout: 15000 })
    // 行为态与心情由渲染器写到根 svg 的 data-*，CSS 与测试据此观察
    await expect(svg).toHaveAttribute('data-mood', /./)
    await expect(svg).toHaveAttribute('data-behaviour', /./)
  })

  test('视口缩放后仍完整成形（像素缩放交给 CSS，viewBox 不随视口变）', async ({ page }) => {
    const svg = page.locator('.pet3d-stage svg.pet-svg-canvas')
    await expect(svg).toBeVisible({ timeout: 15000 })
    const before = await svg.boundingBox()
    expect(before && before.width > 0 && before.height > 0).toBeTruthy()
    await page.setViewportSize({ width: 900, height: 700 })
    await expect(svg).toBeVisible()
    // 矢量契约：viewBox 恒定 200×200，不变形不重排
    await expect(svg).toHaveAttribute('viewBox', '0 0 200 200')
    const after = await svg.boundingBox()
    expect(after && after.width > 0 && after.height > 0).toBeTruthy()
  })

  test('pet.status RPC → 心情 + 能量/好感度展示', async ({ page }) => {
    await expect(page.locator('.pet-mood')).toBeVisible()
    const stats = page.locator('.pet-stats')
    await expect(stats).toBeVisible()
    await expect(stats).toHaveText(/能量 \d+\/100/)
  })

  // 悬停 → 局部聚焦注视（V3）：光标停到它身上应进入聚焦态，移开必须退出。
  // 断言的是**交互契约**（聚焦态进出 + 画布仍在渲染）；「聚焦到底改变了什么」由单测
  // （petPose 的 focus 增益、petSpec 的推近纯函数）与 dev-server 探针读 getCameraState
  // 负责 —— 截图比对在这里证明不了任何事（桌宠本来就在呼吸/眨眼，帧帧不同）。
  test('悬停 → 聚焦态进出（局部注视已接线，不是死代码）', async ({ page }) => {
    const stage = page.locator('.pet3d-stage')
    await expect(stage).toBeVisible()
    await expect(stage).not.toHaveClass(/is-focused/)

    await stage.hover()
    await expect(stage).toHaveClass(/is-focused/)

    // 移开 → 必须退出聚焦（否则「进入聚焦」会退化成永久状态）
    await page.mouse.move(4, 4)
    await expect(stage).not.toHaveClass(/is-focused/)

    // 舞台仍在正常渲染（聚焦切换不得破坏渲染循环）
    const svg = stage.locator('svg.pet-svg-canvas')
    await expect(svg).toBeVisible()
    const box = await svg.boundingBox()
    expect(box && box.width > 0 && box.height > 0).toBeTruthy()
  })

  test('顺毛抚摸 → pet.triggerAction 往返后状态保持健康', async ({ page }) => {
    const stats = page.locator('.pet-stats')
    await expect(stats).toBeVisible()
    const before = await stats.textContent()
    await page.getByRole('button', { name: '顺毛抚摸' }).click()
    await page.waitForTimeout(1000)
    const after = await stats.textContent()
    expect(after).toMatch(/能量 \d+\/100/)
    expect(after).not.toBe(before) // RPC 往返更新了状态（能量/好感度变化）
  })
})

test.describe('Pet Overlay 悬浮窗握手（/pet-overlay）', () => {
  test.beforeEach(async ({ page }) => {
    // 桌面平台：注入 MargueriteDesktop 使 /pet-overlay 路由可达（与 Tauri 壳加载一致）
    await page.addInitScript(() => {
      ;(window as unknown as Record<string, unknown>).MargueriteDesktop = true
    })
    await page.goto(CONTROL_URL + 'pet-overlay')
    await page.waitForSelector('.conn-pill', { timeout: 15000 })
  })

  test('透明舞台 + 独立 2D SVG 挂载', async ({ page }) => {
    const overlay = page.locator('.pet-overlay')
    await expect(overlay).toBeVisible()
    // 悬浮窗视觉前提：overlay 根背景透明（页面内嵌于 App 壳，body 由壳着色；
    // 真实 Tauri 窗口透明由装饰关闭 + html/body 透明决定，此处校验页面自身契约）
    const overlayBg = await overlay.evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(overlayBg).toMatch(/^(rgba\(0, 0, 0, 0\)|transparent)$/)
    const stage = page.locator('.pet-overlay__stage')
    await expect(stage).toBeVisible()
    // SVG 无 WebGL 依赖：任何环境都必须完整成形（不再有 2D 降级徽章这一说）
    const svg = stage.locator('svg.pet-svg-canvas')
    await expect(svg).toBeVisible({ timeout: 15000 })
    const box = await svg.boundingBox()
    expect(box && box.width > 0 && box.height > 0).toBeTruthy()
    expect(await stage.locator('canvas').count()).toBe(0)
  })

  test('右键菜单 → 5 款皮肤切换项', async ({ page }) => {
    await page.locator('.pet-overlay').click({ button: 'right' })
    const menu = page.locator('.pet-overlay__menu')
    await expect(menu).toBeVisible()
    await expect(menu.locator('.pet-overlay__skin')).toHaveCount(5)
  })

  test('心情胶囊随 pet.status 渲染（无连接时保持静默）', async ({ page }) => {
    // 状态胶囊存在（无论是否连接成功都渲染容器；能量/好感度仅在 statusReady 时填充）
    const capsule = page.locator('.pet-overlay__capsule')
    await expect(capsule).toBeVisible({ timeout: 15000 })
  })
})

// 说明：旧「WebGL 档位表（dpr=2）」用例组已随 3D 渲染器一并退场 ——
// 档位现在只调特效预算（模糊半径/粒子数），SVG 无 WebGL 依赖、任何环境都完整成形；
// 分层/材质/位姿的正确性由 petSvg / petSpec / petContract 单测覆盖（不在此重复）。
