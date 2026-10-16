import { test, expect } from '@playwright/test'

// 岛屿工作空间 smoke（/control/island）—— 路由可达 + 关键元素渲染 + 记忆塔数据管线。
//
// 前置条件与既有 e2e（island.spec.ts / chat.spec.ts）一致：网关或 `npm run dev` 已托管 webui。
// 运行方式：
//   OPENSQUILLA_WEBUI_BASE_URL=http://127.0.0.1:<port> npx playwright test e2e/island-athenaeum.spec.ts
//
// 覆盖两件事：
//   1. 路由与渲染：页头/控制条可见，渲染路径为真实 3D 画布 或 诚实 2D 降级（headless 下
//      WebGL 不确定，两条路径都接受，但二者互斥且 2D 必须明确标注）。
//   2. 记忆塔数据管线：走真实事件归约（session.event.memory_recall → 运行时快照 → 世界），
//      0 命中不出塔、有命中才出塔 —— 呼应「诚实可视化」铁律。
//
// 断言保持语言无关（默认语言可能是 en / zh-Hans），只校验结构与语义；不发明后端方法名。
const ISLAND_URL = '/control/island'

test.describe('岛屿工作空间 smoke（/control/island）', () => {
  test('路由可达 + 关键元素渲染（3D 画布 或 诚实 2D 降级）', async ({ page }) => {
    await page.goto(ISLAND_URL)
    await expect(page.locator('.island-bar__title')).toBeVisible({ timeout: 15000 })
    await expect(page.locator('.island-bar__title')).not.toBeEmpty()
    await expect(page.locator('.island-controls__btn').first()).toBeVisible()

    const canvas = page.locator('.island-canvas canvas')
    const map = page.locator('svg.island-map')
    await expect(canvas.or(map).first()).toBeVisible({ timeout: 15000 })
    const has3d = await canvas.count()
    const has2d = await map.count()
    expect(has3d + has2d).toBeGreaterThan(0)

    if (has2d > 0) {
      // 2D 降级：必须诚实标注（绝不假装 3D）
      expect(has3d).toBe(0)
      await expect(page.locator('.island-notice')).toHaveText(/2D/)
    }
  })

  test('空态诚实：无运行数据时只有中央主堡，不编造建筑', async ({ page }) => {
    await page.goto(ISLAND_URL)
    await expect(page.locator('.island-bar__title')).toBeVisible({ timeout: 15000 })
    const legendRows = page.locator('.island-legend__row')
    await expect(legendRows).toHaveCount(1)
    await expect(legendRows.first().locator('.island-legend__count')).toHaveText('1')
  })

  test('记忆塔数据管线：无召回不出塔，有召回才出塔（诚实可视化）', async ({ page }) => {
    await page.goto(ISLAND_URL)
    await expect(page.locator('.island-bar__title')).toBeVisible({ timeout: 15000 })

    const result = await page.evaluate(async () => {
      const worldMod = await import('/src/components/island/islandWorld.ts')
      const runtimeMod = await import('/src/components/island/islandRuntime.ts')
      const specMod = await import('/src/components/island/islandSpec.ts')
      const contractMod = await import('/src/components/island/islandContract.ts')
      const spec = specMod.DEFAULT_ISLAND_SPEC
      const key = 'e2e-memory'
      const opts = { sessionKey: key, now: 1_700_000_000_000 }

      // 只收到 0 命中：视为无召回 → 不出记忆塔
      const zero = runtimeMod.applyIslandEvent(
        runtimeMod.createIslandRuntimeState(key),
        'session.event.memory_recall',
        { session_key: key, query: 'noop', hits: 0, topScore: 0, ids: [], reinforced: 0 },
        opts,
      )
      const zeroWorld = worldMod.buildIslandWorld(zero, spec)

      // 真实命中（命中并强化突触）→ 出记忆塔，状态 success
      const hit = runtimeMod.applyIslandEvent(
        runtimeMod.createIslandRuntimeState(key),
        'session.event.memory_recall',
        {
          session_key: key,
          query: 'deployment checklist',
          hits: 4,
          topScore: 0.83,
          ids: ['m1', 'm2', 'm3', 'm4'],
          reinforced: 2,
          ts: 1_700_000_000_123,
        },
        opts,
      )
      const hitWorld = worldMod.buildIslandWorld(hit, spec)
      const tower = hitWorld.entities.find((e) => e.kind === 'athenaeum')

      return {
        zeroCount: zeroWorld.counts.athenaeum,
        hitCount: hitWorld.counts.athenaeum,
        kindForMemory: contractMod.kindForSource('memory'),
        status: tower?.status,
        hits: tower?.meta?.recallHits,
        topScore: tower?.meta?.recallTopScore,
        reinforced: tower?.meta?.reinforced,
        memoryIds: tower?.meta?.memoryIds,
      }
    })

    expect(result.kindForMemory).toBe('athenaeum')
    expect(result.zeroCount).toBe(0)
    expect(result.hitCount).toBe(1)
    expect(result.status).toBe('success')
    expect(result.hits).toBe(4)
    expect(result.topScore).toBeCloseTo(0.83, 5)
    expect(result.reinforced).toBe(2)
    expect(result.memoryIds).toEqual(['m1', 'm2', 'm3', 'm4'])
  })
})