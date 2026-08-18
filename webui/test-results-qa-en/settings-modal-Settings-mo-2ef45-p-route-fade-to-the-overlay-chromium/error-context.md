# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: settings-modal.spec.ts >> Settings modal >> opens Settings without applying the app route fade to the overlay
- Location: e2e\settings-modal.spec.ts:169:3

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
  126 |         modal,
  127 |         removed: false,
  128 |         loadingSeen: false,
  129 |         observer: null as MutationObserver | null,
  130 |       }
  131 |       state.observer = new MutationObserver(() => {
  132 |         state.removed = state.removed || !document.body.contains(state.modal)
  133 |         state.loadingSeen = state.loadingSeen || !!document.querySelector('.settings-loading')
  134 |       })
  135 |       state.observer.observe(document.body, { childList: true, subtree: true })
  136 |       ;(window as typeof window & { __settingsModalProbe?: typeof state }).__settingsModalProbe = state
  137 |     })
  138 | 
  139 |     await railTab(page, 'Capabilities').click()
  140 |     await expect(page).toHaveURL(/\/settings\/capabilities$/)
  141 |     await expect(railTab(page, 'Capabilities')).toHaveAttribute('aria-selected', 'true')
  142 |     await expect(dialog(page).getByRole('heading', { name: 'Web search' })).toBeVisible()
  143 |     // Wait past the route fade and Settings pop enter windows so delayed
  144 |     // remounts, re-entry animations, or loading fallbacks are caught.
  145 |     await page.waitForTimeout(350)
  146 | 
  147 |     const probe = await page.evaluate(() => {
  148 |       const state = (window as typeof window & {
  149 |         __settingsModalProbe?: {
  150 |           modal: Element
  151 |           removed: boolean
  152 |           loadingSeen: boolean
  153 |           observer: MutationObserver | null
  154 |         }
  155 |       }).__settingsModalProbe
  156 |       if (!state) throw new Error('settings modal probe missing')
  157 |       state.observer?.disconnect()
  158 |       return {
  159 |         sameNode: state.modal === document.querySelector('.settings-modal'),
  160 |         removed: state.removed,
  161 |         loadingSeen: state.loadingSeen,
  162 |       }
  163 |     })
  164 |     expect(probe.sameNode).toBe(true)
  165 |     expect(probe.removed).toBe(false)
  166 |     expect(probe.loadingSeen).toBe(false)
  167 |   })
  168 | 
  169 |   test('opens Settings without applying the app route fade to the overlay', async ({ page }) => {
  170 |     await page.goto(CONTROL_URL)
  171 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  172 |     await page.evaluate(() => {
  173 |       const state = {
  174 |         overlaySeen: false,
  175 |         routeFadeSeen: false,
  176 |         observer: null as MutationObserver | null,
  177 |       }
  178 |       const inspect = () => {
  179 |         const overlay = document.querySelector('.settings-overlay')
  180 |         if (!overlay) return
  181 |         state.overlaySeen = true
  182 |         state.routeFadeSeen = state.routeFadeSeen || String(overlay.className).includes('route-fade-')
  183 |       }
  184 |       state.observer = new MutationObserver(inspect)
  185 |       state.observer.observe(document.body, {
  186 |         attributeFilter: ['class'],
  187 |         attributes: true,
  188 |         childList: true,
  189 |         subtree: true,
  190 |       })
  191 |       inspect()
  192 |       ;(window as typeof window & { __settingsOpenProbe?: typeof state }).__settingsOpenProbe = state
  193 |     })
  194 | 
  195 |     await settingsRow(page).click()
> 196 |     await expect(dialog(page)).toBeVisible()
      |                                ^ Error: expect(locator).toBeVisible() failed
  197 |     await expect(dialog(page).locator('.settings-banner')).toBeVisible()
  198 |     await page.waitForTimeout(200)
  199 | 
  200 |     const probe = await page.evaluate(() => {
  201 |       const state = (window as typeof window & {
  202 |         __settingsOpenProbe?: {
  203 |           overlaySeen: boolean
  204 |           routeFadeSeen: boolean
  205 |           observer: MutationObserver | null
  206 |         }
  207 |       }).__settingsOpenProbe
  208 |       if (!state) throw new Error('settings open probe missing')
  209 |       state.observer?.disconnect()
  210 |       return {
  211 |         overlaySeen: state.overlaySeen,
  212 |         routeFadeSeen: state.routeFadeSeen,
  213 |       }
  214 |     })
  215 |     expect(probe.overlaySeen).toBe(true)
  216 |     expect(probe.routeFadeSeen).toBe(false)
  217 |   })
  218 | 
  219 |   test('Model Routing section saves the routing panel style', async ({ page }) => {
  220 |     test.setTimeout(90000)
  221 |     await openFromSidebar(page)
  222 | 
  223 |     const visualMode = () => dialog(page).locator('select[name="setup_model_strategy_router_visual_mode"]')
  224 |     const dirtybar = dialog(page).locator('.settings-dirtybar')
  225 | 
  226 |     await railTab(page, 'Model Routing').click()
  227 |     // The picker rides with the router details, so make the router strategy
  228 |     // active first if the gateway under test starts on another strategy (the
  229 |     // strategy choice is then saved along with the style — acceptable for the
  230 |     // disposable gateways this suite runs against).
  231 |     const routerCard = dialog(page).locator('[data-strategy-id="router"]')
  232 |     await expect(routerCard).toBeVisible()
  233 |     if ((await routerCard.getAttribute('aria-checked')) !== 'true') await routerCard.click()
  234 |     await expect(visualMode()).toBeVisible()
  235 |     const initial = await visualMode().inputValue()
  236 |     const next = initial === 'legacy_grid' ? 'real_candidates' : 'legacy_grid'
  237 | 
  238 |     await visualMode().selectOption(next)
  239 |     await expect(dirtybar).toBeVisible()
  240 |     await expect(dirtybar).toContainText('Unsaved changes in Model Routing')
  241 |     await dirtybar.getByRole('button', { name: 'Save' }).click()
  242 |     await expect(page.locator('.toast', { hasText: /Router saved/ }).first()).toBeVisible()
  243 |     await expect(dirtybar).toBeHidden({ timeout: 10000 })
  244 | 
  245 |     await page.reload()
  246 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  247 |     await expect(dialog(page)).toBeVisible()
  248 |     await railTab(page, 'Model Routing').click()
  249 |     await expect(visualMode()).toHaveValue(next)
  250 | 
  251 |     await visualMode().selectOption(initial)
  252 |     await dirtybar.getByRole('button', { name: 'Save' }).click()
  253 |     await expect(dirtybar).toBeHidden({ timeout: 10000 })
  254 |   })
  255 | 
  256 |   test('Escape closes the overlay, leaves /settings, and returns focus to the invoker', async ({ page }) => {
  257 |     await openFromSidebar(page)
  258 |     await expect(page).toHaveURL(/\/settings$/)
  259 | 
  260 |     await page.keyboard.press('Escape')
  261 |     await expect(dialog(page)).toBeHidden()
  262 |     // The overlay is route-mounted: closing navigates back off /settings.
  263 |     await expect(page).not.toHaveURL(/\/settings/)
  264 |     await expect(settingsRow(page)).toBeFocused()
  265 | 
  266 |     // Escape inside the overlay must not collapse the docked sidebar.
  267 |     await expect(page.locator('.sidebar.docked')).toBeVisible()
  268 |   })
  269 | 
  270 |   test('closing Settings restores the previous route view without blanking the outlet', async ({ page }) => {
  271 |     await page.goto(CONTROL_URL + 'chat/new')
  272 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  273 |     await expect(page.getByLabel('Message to send')).toBeVisible()
  274 | 
  275 |     await settingsRow(page).click()
  276 |     await expect(dialog(page)).toBeVisible()
  277 |     await page.getByRole('button', { name: 'Close' }).click()
  278 | 
  279 |     await expect(dialog(page)).toBeHidden()
  280 |     await expect(page).toHaveURL(/\/chat\/new$/)
  281 |     await expect(page.getByLabel('Message to send')).toBeVisible()
  282 |   })
  283 | 
  284 |   test('cold deep link to /settings/connection closes home and moves focus to the sidebar', async ({ page }) => {
  285 |     // Land directly on a section with no in-app invoker.
  286 |     await page.goto(CONTROL_URL + 'settings/connection')
  287 |     await page.waitForSelector('.conn-pill', { timeout: 10000 })
  288 |     await expect(dialog(page)).toBeVisible()
  289 |     await expect(railTab(page, 'Connection')).toHaveAttribute('aria-selected', 'true')
  290 |     // Connection renders even before/without a loaded config gate.
  291 |     await expect(dialog(page).getByRole('heading', { name: 'Connection' })).toBeVisible()
  292 | 
  293 |     await page.keyboard.press('Escape')
  294 |     await expect(dialog(page)).toBeHidden()
  295 |     await expect(page).not.toHaveURL(/\/settings/)
  296 |     // No detached focus: it lands on the sidebar Settings button.
```