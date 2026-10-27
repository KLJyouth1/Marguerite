import { test, expect } from '@playwright/test'

// 电脑操控（/control/computer-use）e2e：动作表 → 参数 → 真实 RPC 往返。
//
// 安全约定：只用**只读**动作（cursor_pos）与 `locate`（auto_click 默认关闭）验证链路，
// 绝不在测试里真的点鼠标/敲键盘——那会污染跑测试的这台机器。
//
// 前置条件：网关已启动并托管 webui（dist 或 Vite dev server）。
//   OPENSQUILLA_WEBUI_BASE_URL=http://127.0.0.1:<端口> npx playwright test e2e/computer-use.spec.ts
const CONTROL_URL = '/control/'

test.describe('Computer Use（/control/computer-use）', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(CONTROL_URL + 'computer-use')
    await page.waitForSelector('.conn-pill', { timeout: 15000 })
  })

  test('动作表覆盖三层能力，且视觉定位入口存在', async ({ page }) => {
    // 18 个既有动作 + 新增的 locate（此前前端没有 grounding 入口，网关能力走不到）
    await expect(page.locator('.cu-btn')).toHaveCount(19)
    await expect(page.getByTestId('cu-action-locate')).toBeVisible()
  })

  test('选 locate 才出现描述/视觉开关/采集参数；切到 click 则收起', async ({ page }) => {
    await page.getByTestId('cu-action-locate').click()
    await expect(page.getByTestId('cu-description')).toBeVisible()
    await expect(page.getByTestId('cu-vision')).toBeVisible()
    await expect(page.getByTestId('cu-mode')).toBeVisible()
    await expect(page.getByTestId('cu-max-edge')).toBeVisible()

    await page.getByTestId('cu-action-click').click()
    await expect(page.getByTestId('cu-description')).toHaveCount(0)
    await expect(page.getByTestId('cu-mode')).toHaveCount(0)
    // click 仍然要坐标
    await expect(page.getByTestId('cu-x')).toBeVisible()
  })

  test('只读动作 cursor_pos 真实往返（不产生输入副作用）', async ({ page }) => {
    await page.getByTestId('cu-action-cursor_pos').click()
    await page.getByTestId('cu-execute').click()
    const output = page.getByTestId('cu-output')
    // 轮询到出现真实读数为止（坐标必然含数字；「执行中」与失败文案都不满足）。
    // 75s 而非 30s：每个原生动作都是一次 PowerShell 5.1 + Add-Type，满负载
    // （4 worker 的 WebGL 用例同跑）实测会超过网关 30s 的内部超时；若真超时，
    // 输出会出现「失败」并被下面的断言判红 —— 延长等待只是不再把「慢」误判成
    // 「测试轮询超时」，不会放过真实失败。
    await expect.poll(async () => (await output.textContent()) ?? '', {
      timeout: 75000,
      message: 'cursor_pos 未返回坐标读数',
    }).toMatch(/\d/)
    const text = (await output.textContent()) ?? ''
    expect(text).not.toContain('失败')
    expect(text).not.toMatch(/Failed/)
  })

  test('视觉定位请求真的携带 vision（否则只回「截图已就绪」的降级文案）', async ({ page }) => {
    await page.getByTestId('cu-action-locate').click()
    await page.getByTestId('cu-description').fill('右上角的搜索框')
    // 默认：视觉开启、自动点击关闭（不产生任何不可逆操作）
    await expect(page.getByTestId('cu-vision')).toBeChecked()
    await expect(page.getByTestId('cu-auto-click')).not.toBeChecked()
    await page.getByTestId('cu-execute').click()

    const output = page.getByTestId('cu-output')
    const grounding = page.getByTestId('cu-grounding')
    const unavailable = page.getByTestId('cu-vision-unavailable')
    // 三种合法结局：真实 grounding 结果 / 未配视觉模型的诚实提示 / 引擎的定位或视觉降级文案
    await expect.poll(async () => {
      const text = (await output.textContent()) ?? ''
      return (await grounding.count()) > 0
        || (await unavailable.count()) > 0
        || /定位成功|视觉降级/.test(text)
    }, { timeout: 40000, message: 'locate 未产生任何 grounding 结局' }).toBe(true)

    // 关键不变量：**绝不能**是「vision 没送到」的降级文案（那说明前端没带 vision）
    await expect(output).not.toHaveText(/截图已就绪/)

    // 有 grounding 块时，坐标必须可读且不出现占位 NaN
    if ((await grounding.count()) > 0) {
      const coords = (await page.getByTestId('cu-grounding-coords').textContent()) ?? ''
      expect(coords).toMatch(/x\s-?\d/)
      expect(coords).not.toContain('NaN')
    }
  })

  test('清空按钮同时清掉定位结果与视觉提示', async ({ page }) => {
    await page.getByTestId('cu-action-locate').click()
    await page.getByTestId('cu-description').fill('任务栏')
    await page.getByTestId('cu-execute').click()
    const output = page.getByTestId('cu-output')
    await expect(output).not.toHaveText(/运行中/, { timeout: 20000 })

    await page.getByTestId('cu-clear').click()
    // 空态文案随界面语言（en / zh-Hans 两种最可能命中，其余语言不做硬断言）
    await expect(output).not.toHaveText(/运行中|Running/)
    await expect(output).not.toHaveText(/定位成功|Locate result/)
    await expect(page.getByTestId('cu-grounding')).toHaveCount(0)
    await expect(page.getByTestId('cu-vision-unavailable')).toHaveCount(0)
  })
})
