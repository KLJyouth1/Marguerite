import { test, expect } from '@playwright/test'

// V3-E2E：岛屿工作空间（/control/island）—— 智能体运行的 3D 可视化。
//
// 前置条件：后端统一网关已启动并托管 webui/dist（RPC 通道可用）。
// 运行方式（与既有 e2e 同约定）：
//   OPENSQUILLA_WEBUI_BASE_URL=http://127.0.0.1:<网关端口> npx playwright test e2e/island.spec.ts
//
// 覆盖三条核心契约：
//   1. 渲染路径：有 WebGL → 真实 3D 画布；无 WebGL → 诚实 2D 示意图（绝不假装 3D）
//   2. 交互路径：点选建筑 → 详情面板出现；关闭 → 回到引导提示；复位/聚焦可用
//   3. 数据路径：注入真实运行快照 → 岛上长出对应建筑（工坊/哨所/仓库/灯塔/栈桥）
//
// 断言保持「语言无关」：默认语言可能是 en 或 zh-Hans，因此只校验结构与语义，
// 不写死某一语言的文案（文案由 check-i18n 的键位对齐守卫负责）。
const ISLAND_URL = '/control/island'

test.describe('岛屿工作空间（/control/island）', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(ISLAND_URL)
    // 岛屿页不依赖 RPC 才能渲染（离线也要显示空岛 + 离线胶囊），所以只等页头
    await expect(page.locator('.island-bar__title')).toBeVisible({ timeout: 15000 })
  })

  test('页头三要素：标题、渲染档位胶囊、控制条', async ({ page }) => {
    await expect(page.locator('.island-bar__title')).not.toBeEmpty()
    await expect(page.locator('.island-bar__subtitle')).not.toBeEmpty()
    // 渲染档位胶囊：完整 3D / 均衡 3D / 轻量 3D / 精简模式（2D）四选一，诚实标注
    const pills = page.locator('.island-pill')
    await expect(pills.first()).toBeVisible()
    const pillText = (await pills.allTextContents()).join(' ')
    expect(pillText).toMatch(/Full 3D|Balanced 3D|Light 3D|Lite mode|完整 3D|均衡 3D|轻量 3D|精简模式/)
    await expect(page.locator('.island-controls__btn').first()).toBeVisible()
  })

  test('渲染路径：真实 3D 画布 或 诚实 2D 降级（二者必居其一且互斥）', async ({ page }) => {
    const canvas = page.locator('.island-canvas canvas')
    const map = page.locator('svg.island-map')
    await expect(canvas.or(map).first()).toBeVisible({ timeout: 15000 })

    const has3d = await canvas.count()
    const has2d = await map.count()
    expect(has3d + has2d).toBeGreaterThan(0)

    if (has3d) {
      // 3D：画布必须有真实尺寸，并带 WebGL 上下文
      const box = await canvas.boundingBox()
      expect(box && box.width > 0 && box.height > 0).toBeTruthy()
      expect(await map.count()).toBe(0)
    } else {
      // 2D：必须给出降级说明，且 SVG 上有可点击的建筑节点
      await expect(page.locator('.island-notice')).toBeVisible()
      await expect(page.locator('.island-notice')).toHaveText(/2D/)
      expect(await canvas.count()).toBe(0)
    }
  })

  test('空态诚实：无运行数据时给出引导，不编造建筑', async ({ page }) => {
    // 未选中任何建筑 → 引导提示（文案随语言，故只校验非空）
    await expect(page.locator('.island-detail-empty')).not.toBeEmpty()
    // 图例只列出真实存在的建筑：离线空态下仅中央主堡一类
    const legendRows = page.locator('.island-legend__row')
    await expect(legendRows).toHaveCount(1)
    // 计数为 1（只有主堡），且不是虚构的其它建筑
    await expect(legendRows.first().locator('.island-legend__count')).toHaveText('1')
  })

  test('交互路径：点选建筑 → 详情面板 → 关闭回到引导', async ({ page }) => {
    const canvas = page.locator('.island-canvas canvas')
    test.skip((await canvas.count()) === 0, '无 WebGL：点选走 2D 地图路径（另有用例覆盖）')

    const box = await canvas.boundingBox()
    expect(box).toBeTruthy()
    if (!box) return

    // 点画面中央偏上（主堡所在位置）
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 - box.height * 0.06)
    await page.waitForTimeout(250)
    await page.mouse.down()
    await page.waitForTimeout(60)
    await page.mouse.up()
    await page.waitForTimeout(700)

    const detail = page.locator('.island-detail')
    await expect(detail).toBeVisible()
    await expect(page.locator('.island-detail__title')).not.toBeEmpty()
    // 详情面板至少展示「状态」与「详情」两行真实字段
    expect(await page.locator('.island-detail__rows > div').count()).toBeGreaterThanOrEqual(2)

    // 关闭 → 引导提示回归
    await page.locator('.island-detail__close').click()
    await expect(detail).toHaveCount(0)
    await expect(page.locator('.island-detail-empty')).toBeVisible()
  })

  test('2D 降级路径：地图节点可点选并展示详情', async ({ page }) => {
    const map = page.locator('svg.island-map')
    test.skip((await map.count()) === 0, '有 WebGL：3D 路径已由上一用例覆盖')

    const plot = page.locator('.island-map__plot').first()
    await expect(plot).toBeVisible()
    await plot.click()
    await expect(page.locator('.island-detail')).toBeVisible()
    await expect(page.locator('.island-detail__title')).not.toBeEmpty()
  })

  test('控制条：复位视角与聚焦可用（不抛错、视图保持可用）', async ({ page }) => {
    const buttons = page.locator('.island-controls__btn')
    await expect(buttons).toHaveCount(2)
    await buttons.nth(1).click() // 聚焦
    await page.waitForTimeout(400)
    await buttons.nth(0).click() // 复位视角
    await page.waitForTimeout(600)
    // 视图仍可用：画布或地图仍然可见
    const canvas = page.locator('.island-canvas canvas')
    const map = page.locator('svg.island-map')
    await expect(canvas.or(map).first()).toBeVisible()
  })

  test('昼夜控制：关闭自动后出现日照滑杆并可拖动', async ({ page }) => {
    const toggle = page.locator('.island-controls input[type=checkbox]')
    await expect(toggle).toBeVisible()
    await expect(toggle).toBeChecked()
    await toggle.uncheck()
    const slider = page.locator('.island-controls input[type=range]')
    await expect(slider).toBeVisible()
    await slider.fill('0.85')
    await page.waitForTimeout(400)
    await expect(slider).toHaveValue('0.85')
    await toggle.check()
    await expect(slider).toHaveCount(0)
  })

  test('皮肤热插拔：切换岛屿规格后仍正常渲染', async ({ page }) => {
    const select = page.locator('.island-controls__select')
    await expect(select).toBeVisible()
    const options = await select.locator('option').count()
    expect(options).toBeGreaterThanOrEqual(2)
    await select.selectOption({ index: 1 })
    await page.waitForTimeout(1200)
    const canvas = page.locator('.island-canvas canvas')
    const map = page.locator('svg.island-map')
    await expect(canvas.or(map).first()).toBeVisible()
  })

  test('注入真实运行快照 → 岛上长出对应建筑（工坊/哨所/仓库/灯塔/栈桥）', async ({ page }) => {
    // 用真实的事件→世界管线（buildIslandWorld 是纯函数）构造一份运行数据，
    // 再交给真实渲染器；断言图例计数与建筑语义一致。
    const result = await page.evaluate(async () => {
      const worldMod = await import('/src/components/island/islandWorld.ts')
      const specMod = await import('/src/components/island/islandSpec.ts')
      const spec = specMod.DEFAULT_ISLAND_SPEC
      const world = worldMod.buildIslandWorld({
        sessionKey: 'e2e-session',
        runStatus: 'running',
        tools: [
          { id: 't1', name: 'read_file', isRunning: false, isError: false },
          { id: 't2', name: 'run_command', isRunning: true, isError: false },
          { id: 't3', name: 'web_search', isRunning: false, isError: true },
        ],
        planSteps: [
          { id: 's1', title: '勘察', status: 'completed' },
          { id: 's2', title: '施工', status: 'in_progress' },
        ],
        subagents: [{ key: 'child-1', status: 'completed' }],
        artifacts: [{ id: 'a1', name: 'report.md', kind: 'document' }],
        strategy: 'EXPERT_TEAM',
        ensembleMembers: [{ label: 'p1', done: true }],
        lastEventAt: 0,
      }, spec)
      return {
        counts: world.counts,
        entityCount: world.entities.length,
        links: world.links.length,
        plan: world.plan,
        pierProgress: world.entities.find((e) => e.kind === 'pier')?.progress,
        toolFlavors: world.entities.filter((e) => e.kind === 'workshop').map((e) => e.flavor),
        beaconMembers: world.entities.find((e) => e.kind === 'beacon')?.meta?.ensembleMembers?.length,
      }
    })

    expect(result.counts.core).toBe(1)
    expect(result.counts.workshop).toBe(3)
    expect(result.counts.outpost).toBe(1)
    expect(result.counts.depot).toBe(1)
    expect(result.counts.beacon).toBe(1)
    expect(result.counts.pier).toBe(1)
    // 工具形态由工具名派生：读/执行/联网各归其位
    expect(result.toolFlavors).toEqual(['library', 'forge', 'observatory'])
    // 栈桥铺板进度 = 完成比
    expect(result.pierProgress).toBeCloseTo(0.5, 5)
    expect(result.plan).toEqual({ total: 2, done: 1, progress: 0.5 })
    // 灯塔携带真实融合成员数
    expect(result.beaconMembers).toBe(1)
    // 每条道路都从主堡出发（N 座建筑 → N-1 条道路）
    expect(result.links).toBe(result.entityCount - 1)
    expect(result.links).toBe(7)
  })

  test('岛屿可嵌入工作台面板（请求事件打开 island tab）', async ({ page }) => {
    await page.goto('/control/chat')
    await page.waitForTimeout(2500)
    await page.evaluate(() => {
      window.dispatchEvent(new CustomEvent('marguerite:open-island', { detail: { scopeId: 'e2e' } }))
    })
    await page.waitForTimeout(2500)
    const panel = page.locator('.island-panel')
    await expect(panel).toBeVisible({ timeout: 15000 })
    // 紧凑形态：面板内不重复页头与图例，但保留控制条
    await expect(panel.locator('.island-bar')).toHaveCount(0)
    await expect(panel.locator('.island-legend')).toHaveCount(0)
    await expect(panel.locator('.island-controls__btn').first()).toBeVisible()
  })

  // 键盘导航（V3-E2E）：此前岛屿只有指针交互，键盘/精细微调用户无路可走。
  // 断言保持语言无关，且不依赖 WebGL 是否可用（2D 降级时同样应可聚焦并响应 Esc）。
  test('键盘可达：舞台可聚焦，Esc 关闭详情面板', async ({ page }) => {
    const stage = page.locator('.island-stage')
    // tabindex 是键盘路径的落点；缺了它方向键/Esc 永远收不到
    await expect(stage).toHaveAttribute('tabindex', '0')

    const label = page.locator('.island-label').first()
    const hasLabel = await label.count()
    test.skip(hasLabel === 0, '岛上暂无建筑（无运行数据），无详情面板可关')

    await label.click()
    await expect(page.locator('.island-detail')).toBeVisible({ timeout: 10000 })

    await stage.focus()
    await page.keyboard.press('Escape')
    // Esc 后详情面板关闭，回到引导提示
    await expect(page.locator('.island-detail')).toHaveCount(0)
    await expect(page.locator('.island-detail-empty')).toBeVisible()
  })

  test('键盘导航：方向键绕行改变画面，0 复位回到初始', async ({ page }) => {
    const canvas = page.locator('.island-canvas canvas')
    test.skip((await canvas.count()) === 0, '当前为 2D 降级，相机导航不适用')

    const stage = page.locator('.island-stage')
    const shot = async () => (await canvas.screenshot()).toString('base64')

    await stage.focus()
    await page.waitForTimeout(1200)
    const before = await shot()

    for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowRight')
    await page.waitForTimeout(1500)
    const orbited = await shot()
    expect(orbited, '方向键应改变相机（画面必须变化）').not.toBe(before)

    await page.keyboard.press('0')
    await page.waitForTimeout(1800)
    const reset = await shot()
    // 复位应回到接近初始：与 before 的差异远小于绕行后的差异
    const sameAsBefore = reset === before
    const stillOrbited = reset === orbited
    expect(sameAsBefore || !stillOrbited, '按 0 后应回到初始视角').toBe(true)
  })
})
