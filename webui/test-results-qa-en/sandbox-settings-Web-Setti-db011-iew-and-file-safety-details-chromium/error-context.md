# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: sandbox-settings.spec.ts >> Web Settings opens the Sandbox overview and file-safety details
- Location: e2e\sandbox-settings.spec.ts:3:1

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
  1  | import { expect, test } from '@playwright/test'
  2  | 
  3  | test('Web Settings opens the Sandbox overview and file-safety details', async ({ page }) => {
  4  |   await page.goto('/control/')
  5  |   await page.waitForSelector('.conn-pill', { timeout: 10_000 })
  6  | 
  7  |   await page.locator('.sidebar-fn-item[data-icon="settings"]').click()
  8  |   const settings = page.getByRole('dialog', { name: 'Settings' })
> 9  |   await expect(settings).toBeVisible()
     |                          ^ Error: expect(locator).toBeVisible() failed
  10 | 
  11 |   const sandboxTab = settings.getByRole('tab', { name: 'Sandbox', exact: true })
  12 |   await expect(sandboxTab).toBeVisible()
  13 |   await sandboxTab.click()
  14 |   await expect(page).toHaveURL(/\/settings\/sandbox$/)
  15 | 
  16 |   await expect(page.getByTestId('sandbox-overview')).toBeVisible()
  17 |   await expect(page.getByTestId('sandbox-safe-mode')).toBeVisible()
  18 |   await expect(page.getByTestId('sandbox-full-mode')).toBeVisible()
  19 |   await expect(page.locator('.sandbox-settings__status')).toBeVisible()
  20 | 
  21 |   await page.getByTestId('sandbox-open-files').click()
  22 |   await expect(page.getByTestId('sandbox-detail')).toBeVisible()
  23 |   await expect(page.getByTestId('builtin-file-rules')).toBeVisible()
  24 |   await expect(page.getByTestId('sandbox-backup-quota')).toBeVisible()
  25 | 
  26 |   await page.getByTestId('sandbox-detail-back').click()
  27 |   await expect(page.getByTestId('sandbox-overview')).toBeVisible()
  28 | })
  29 | 
```