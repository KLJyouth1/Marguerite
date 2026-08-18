# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> Appearance persists the tool-detail default without a save step
- Location: e2e\settings-modal.spec.ts:540:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByRole('dialog', { name: 'Settings' })
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByRole('dialog', { name: 'Settings' })

```

```yaml
- navigation "主导航":
  - text: Marguerite
  - button "收起侧边栏" [expanded]:
    - img
  - navigation "控制台导航":
    - button "新建任务":
      - img
      - text: 新建任务
    - link "技能·渠道":
      - /url: /control/skills
      - img
      - text: 技能·渠道
    - link "定时任务":
      - /url: /control/cron
      - img
      - text: 定时任务
    - link "查看用量":
      - /url: /control/usage
      - img
      - text: 查看用量
  - button "设置":
    - img
    - text: 设置
- separator "调整侧边栏宽度":
  - status
- banner:
  - button "管理网关连接": 已连接
  - button "语言":
    - img
    - text: 中文
  - button "主题":
    - img
- main
- status
- dialog "设置":
  - tablist "设置分区":
    - 'tab "连接: 已连接"': 连接
    - 'tab "模型服务: 就绪" [selected]': 模型服务
    - 'tab "模型路由: 就绪"': 模型路由
    - 'tab "能力: 可选"': 能力
    - 'tab "行为: 保存后生效"': 行为
    - 'tab "隐私: 保存后生效"': 隐私
    - tab "沙箱"
    - tab "记忆与画像"
    - tab "外观"
    - tab "键盘"
    - tab "高级"
  - banner:
    - heading "设置" [level=2]
    - button "关闭":
      - img
  - 'tabpanel "模型服务: 就绪"':
    - group:
      - heading "模型服务" [level=3]
      - paragraph: 连接模型供应商,管理凭证与模型。
      - button "添加服务商"
      - group:
        - heading "已配置的服务商" [level=4]
        - paragraph: 1 / 1 就绪。测试会发送一次小请求。
        - list:
          - listitem:
            - button "编辑 DeepSeek — 当前使用 — 凭证就绪 — 未验证": DeepSeek
            - button "验证已保存配置 — DeepSeek": 验证已保存配置
            - button "编辑 DeepSeek": 编辑
  - contentinfo:
    - text: 更多选项位于
    - code: C:\Users\Administrator\AppData\Local\Marguerite\pyengine-home\default\config.toml
    - button "复制配置路径":
      - img
    - text: 大多数更改即时生效；部分需要重启网关
```

# Test source

```ts
  1   | import { test, expect } from '@playwright/test'
  2   | 
  3   | const CONTROL_URL = '/control/'
  4   | // Backend-config sections carry a readiness/status dot; Connection is the first
  5   | // entry (live socket state). Memory & Profile is an RPC action surface that
  6   | // deliberately stays outside config save state; the remaining entries below
  7   | // are ordinary client-only preferences.
  8   | // (Runtime exists too, but it is desktop-only so the web rail hides it.)
  9   | const SECTIONS = ['Connection', 'Model Service', 'Model Routing', 'Capabilities', 'Behavior', 'Privacy']
  10  | const CLIENT_SECTIONS = ['Sandbox', 'Memory & Profile', 'Appearance', 'Keyboard', 'Advanced']
  11  | 
  12  | const settingsRow = (page: import('@playwright/test').Page) =>
  13  |   page.locator('.sidebar-foot button')
  14  | 
  15  | const dialog = (page: import('@playwright/test').Page) =>
  16  |   page.getByRole('dialog', { name: 'Settings' })
  17  | 
  18  | const railTab = (page: import('@playwright/test').Page, name: string) =>
  19  |   dialog(page).getByRole('tab', { name: new RegExp(`^${name}:`) })
  20  | 
  21  | async function openFromSidebar(page: import('@playwright/test').Page) {
  22  |   await page.goto(CONTROL_URL)
  23  |   await page.waitForSelector('.conn-pill', { timeout: 10000 })
  24  |   await settingsRow(page).click()
> 25  |   await expect(dialog(page)).toBeVisible()
      |                              ^ Error: expect(locator).toBeVisible() failed
  26  | }
  27  | 
  28  | test.describe('Settings modal', () => {
  29  |   test('opens from the sidebar with the curated section rail and readiness banner', async ({ page }) => {
  30  |     await openFromSidebar(page)
  31  | 
  32  |     // Dialog a11y: focus moves into the modal on open, before any interaction.
  33  |     await expect(page.getByRole('button', { name: 'Close' })).toBeFocused()
  34  | 
  35  |     // Rail has the backend-config sections (each with a readiness/status dot)
  36  |     // plus the client-only sections.
  37  |     const tabs = dialog(page).getByRole('tab')
  38  |     await expect(tabs).toHaveCount(SECTIONS.length + CLIENT_SECTIONS.length)
  39  |     for (const name of SECTIONS) {
  40  |       await expect(railTab(page, name)).toBeVisible()
  41  |     }
  42  |     for (const name of CLIENT_SECTIONS) {
  43  |       await expect(dialog(page).getByRole('tab', { name, exact: true })).toBeVisible()
  44  |     }
  45  | 
  46  |     // Readiness banner: quiet ready line or actionable count.
  47  |     const banner = dialog(page).locator('.settings-banner')
  48  |     await expect(banner).toBeVisible()
  49  |     await expect(banner.locator('.settings-banner__row')).toContainText(/Ready to run|Action needed \(\d+\)/)
  50  | 
  51  |     // CLI handoff disclosure expands with command groups and the config summary.
  52  |     await banner.getByRole('button', { name: 'CLI handoff' }).click()
  53  |     await expect(banner.locator('.setup-cli__group', { hasText: 'CLI handoff' })).toBeVisible()
  54  |     await expect(banner.locator('.setup-cli__group', { hasText: 'CLI recipes' })).toBeVisible()
  55  |     await expect(banner.locator('.setup-summary')).toContainText('Provider')
  56  |   })
  57  | 
  58  |   test('no YAML editor, raw key search, or guided-setup wording anywhere', async ({ page }) => {
  59  |     await openFromSidebar(page)
  60  | 
  61  |     await expect(dialog(page).getByRole('button', { name: 'YAML', exact: true })).toHaveCount(0)
  62  |     await expect(dialog(page).getByRole('button', { name: 'Form', exact: true })).toHaveCount(0)
  63  |     await expect(dialog(page).locator('#cfg-search')).toHaveCount(0)
  64  |     await expect(dialog(page).locator('textarea#cfg-yaml-area')).toHaveCount(0)
  65  |     await expect(dialog(page).getByText('Guided setup')).toHaveCount(0)
  66  | 
  67  |     // Footer keeps the config.toml escape hatch with a copy affordance.
  68  |     const foot = dialog(page).locator('.settings-foot')
  69  |     await expect(foot).toContainText('More options live in')
  70  |     // Honest restart copy: most edits apply live, only some need a restart.
  71  |     await expect(foot).toContainText('Most changes apply live; some need a gateway restart')
  72  |     // The old blanket "always restart" leak is gone.
  73  |     await expect(foot).not.toContainText('Restart the gateway after manual edits')
  74  |     await expect(foot.locator('.settings-foot__path')).toContainText(/config.*\.toml/)
  75  |     await foot.getByRole('button', { name: 'Copy config path' }).click()
  76  |     await expect(page.locator('.toast', { hasText: /Copied/ }).first()).toBeVisible()
  77  |   })
  78  | 
  79  |   test('rail switches sections, marks the active tab, and syncs the URL with replace', async ({ page }) => {
  80  |     await openFromSidebar(page)
  81  |     await expect(page).toHaveURL(/\/settings$/)
  82  | 
  83  |     await railTab(page, 'Capabilities').click()
  84  |     await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
  85  |     // Switching a section reflects in the URL so it stays deep-linkable.
  86  |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  87  |     // The Capabilities panel renders its tools as stacked sections (Web search,
  88  |     // Memory, Image, Audio) rather than a redundant "Capabilities" card heading.
  89  |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  90  | 
  91  |     await railTab(page, 'Model Routing').click()
  92  |     await expect(railTab(page, 'Model Routing')).toHaveAttribute('aria-selected', 'true')
  93  |     await expect(page).toHaveURL(/\/settings\/modelStrategy$/)
  94  |     await expect(dialog(page).getByRole('radiogroup', { name: 'Model routing', exact: true })).toBeVisible()
  95  | 
  96  |     // Section navigation uses replace, so a single Back exits Settings rather
  97  |     // than walking section history.
  98  |     await page.goBack()
  99  |     await expect(dialog(page)).toBeHidden()
  100 |     await expect(page).not.toHaveURL(/\/settings/)
  101 |   })
  102 | 
  103 |   test('opens Memory & Profile as a first-level action route without global save state', async ({ page }) => {
  104 |     await openFromSidebar(page)
  105 | 
  106 |     const memoryTab = dialog(page).getByRole('tab', { name: 'Memory & Profile', exact: true })
  107 |     await memoryTab.click()
  108 | 
  109 |     await expect(memoryTab).toHaveAttribute('aria-selected', 'true')
  110 |     await expect(page).toHaveURL(/\/settings\/memory$/)
  111 |     await expect(dialog(page).getByRole('heading', { name: 'Memory & Profile' })).toBeVisible()
  112 |     await expect(dialog(page).locator('[data-testid="settings-memory-panel"]')).toBeVisible()
  113 |     await expect(dialog(page).locator('.settings-dirtybar')).toBeHidden()
  114 |   })
  115 | 
  116 |   test('keeps the SettingsDialog mounted when routing from default settings to a section', async ({ page }) => {
  117 |     await openFromSidebar(page)
  118 |     await expect(page).toHaveURL(/\/settings$/)
  119 |     await expect(dialog(page).locator('.settings-banner')).toBeVisible()
  120 |     await expect(dialog(page).locator('.settings-loading')).toHaveCount(0)
  121 | 
  122 |     await page.evaluate(() => {
  123 |       const modal = document.querySelector('.settings-modal')
  124 |       if (!modal) throw new Error('settings modal missing')
  125 |       const state = {
```