import { test, expect } from '@playwright/test'

// V3-E2E：岛屿画质档位表（数据驱动分档，不得被压回两档）。
//
// 为什么单独一个文件、不放 island.spec.ts：那份 spec 里有一条用例动态
// `import('/src/...')`（只能打 Vite dev server），因此整份文件进不了「网关托管 dist」
// 的闸门；本文件只依赖 dist + 网关 RPC，可稳定入闸门（verify.sh 的棘轮清单）。
//
// 契约（两条，都语言无关）：
//   1. 渲染器**实际生效**的开关（像素比上限、抗锯齿）必须落在档位表内 —— 表外组合
//      说明渲染器没查表（散落硬编码）。
//   2. 页头胶囊上报的档位必须与渲染器实际生效的档位一致 —— 视图一旦把检测结果
//      压档（如把 medium 压成 low），胶囊说「均衡 3D」而画布按最低档渲染，即判红。
//
// 前提：网关已启动并托管 webui/dist。运行：
//   OPENSQUILLA_WEBUI_BASE_URL=http://127.0.0.1:<网关端口> npx playwright test e2e/island-quality.spec.ts
const ISLAND_URL = '/control/island'

/** 档位胶囊文案 → 档位（六语言，避免写死单一语言） */
const TIER_LABELS: ReadonlyArray<readonly [string, RegExp]> = [
  ['high', /Full 3D|完整 3D|フル3D|3D complète|Volles 3D|3D completa/],
  ['medium', /Balanced 3D|均衡 3D|バランス3D|3D équilibrée|Ausgeglichenes 3D|3D equilibrada/],
  ['low', /Light 3D|轻量 3D|軽量3D|3D légère|Leichtes 3D|3D ligera/],
  ['fallback', /Lite mode|精简模式|簡易モード|Mode léger|Sparmodus|Modo ligero/],
]

/**
 * 渲染层实际生效的开关 → 档位。
 *
 * dpr=2 下三档两两可区分：high=(像素比 2, 开抗锯齿) / medium=(1.5, 无) / low=(1, 无)。
 * 返回 null = 表外组合（渲染器没查档位表）。
 */
function tierOfApplied(ratio: number, antialias: boolean): string | null {
  const rows = [
    { tier: 'high', ratio: 2, antialias: true },
    { tier: 'medium', ratio: 1.5, antialias: false },
    { tier: 'low', ratio: 1, antialias: false },
  ]
  const hit = rows.find(r => Math.abs(r.ratio - ratio) < 0.02 && r.antialias === antialias)
  return hit ? hit.tier : null
}

type Applied = { ratio: number; antialias: boolean; dpr: number }

async function readApplied(page: import('@playwright/test').Page): Promise<Applied | null> {
  const canvas = page.locator('.island-canvas canvas')
  if (!(await canvas.count())) return null
  return canvas.first().evaluate((el: HTMLCanvasElement) => {
    const gl = (el.getContext('webgl2') || el.getContext('webgl')) as WebGLRenderingContext | null
    const attrs = gl ? gl.getContextAttributes() : null
    const box = el.getBoundingClientRect()
    return {
      ratio: box.width ? +(el.width / box.width).toFixed(3) : 0,
      antialias: attrs ? !!attrs.antialias : false,
      dpr: window.devicePixelRatio,
    }
  })
}

/** 页头胶囊文案 → 档位（未命中任何档位文案时返回 null） */
async function readPillTier(page: import('@playwright/test').Page): Promise<string | null> {
  const text = (await page.locator('.island-pill').allTextContents()).join(' ')
  return TIER_LABELS.find(([, re]) => re.test(text))?.[0] ?? null
}

// 档位表的可判别性依赖设备像素比：dpr=1 时三档的像素比都退化成 1。
test.describe('岛屿画质档位表（dpr=2 可判别）', () => {
  test.use({ deviceScaleFactor: 2 })

  test.beforeEach(async ({ page }) => {
    await page.goto(ISLAND_URL)
    await page.waitForSelector('.island-pill', { timeout: 20000 })
  })

  test('渲染器实际生效的开关必须落在档位表内（不是散落硬编码）', async ({ page }) => {
    const applied = await readApplied(page)
    test.skip(applied === null, '本机无 WebGL → 走 2D 示意图，无档位可验')
    if (!applied) return
    expect(applied.dpr).toBe(2)
    expect(
      tierOfApplied(applied.ratio, applied.antialias),
      `实际生效开关 ${JSON.stringify(applied)} 不在档位表内（high=2/AA, medium=1.5/无AA, low=1/无AA）`,
    ).not.toBeNull()
  })

  test('页头胶囊档位与渲染器实际生效档位一致（视图不得把检测结果压档）', async ({ page }) => {
    const pillTier = await readPillTier(page)
    expect(pillTier, '页头必须诚实标注渲染档位').not.toBeNull()

    const applied = await readApplied(page)
    if (applied === null) {
      // 2D 路径：胶囊必须是 fallback，且给出降级说明
      expect(pillTier).toBe('fallback')
      await expect(page.locator('.island-notice')).toBeVisible()
      return
    }
    const appliedTier = tierOfApplied(applied.ratio, applied.antialias)
    expect(appliedTier, `实际生效开关 ${JSON.stringify(applied)} 不在档位表内`).not.toBeNull()
    // 压档回归：视图把 medium 判成 low 时，胶囊仍是 medium、画布却是 low → 此处判红
    expect(appliedTier, `胶囊档位 ${pillTier} 与渲染器实际档位 ${appliedTier} 不一致：${JSON.stringify(applied)}`).toBe(pillTier)
  })
})
