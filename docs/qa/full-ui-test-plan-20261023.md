# Marguerite 全界面测试验证计划（2026-10-23）

> 范围：`http://localhost:18790/control/` 托管的全部核心界面（Chat / Task / Sessions / Usage / Skills / Cron / Pet / Wallpaper / Settings·Appearance / Engine / Local Models / Agents / Expert Center / Terminal / Editor / Browser Cockpit / App Control / Computer Use / Ops / Overview / Plugins / Knowledge / Graph / Fleet / Evolve / Brain / Island / Games / Poker / Device Twin / Changelog / Splash / Pet Overlay）。
>
> 目标：每个界面都有**可复现的前置条件、操作步骤、预期结果与判定标准**；执行中发现的缺陷当场修复或登记；最终形成可审计的用例清单与证据。

---

## 0. 测试元信息

| 项 | 值 |
| --- | --- |
| 被测版本 | Marguerite 0.2.16（release，`sha256: 71499BC86C104F21CDD74572928B63C494E793C795ED88C95457DFB8F1936E7F`） |
| 被测入口 | `http://localhost:18790/control/`（Rust 网关托管 WebUI） |
| 引擎 | 内置 Rust 网关 + Python 侧车（`pyengine`），模型 `deepseek-v4-flash`（DeepSeek 官方 API） |
| 数据目录 | `C:\Users\Administrator\.marguerite`（配置/台账）、`C:\Users\Administrator\AppData\Local\Marguerite`（资源/产物） |
| 浏览器 | Chromium（happy path），另需验证 Tauri 桌面壳 |
| 证据目录 | `docs/qa-shots/`（截图）、`.tmp/`（探针脚本与日志） |
| 用例编号 | `<模块>-<序号>`，如 `TOKEN-03`、`PET-02` |

### 0.1 环境前置（所有用例共用）

1. 应用已启动，`http://localhost:18790/control/` 返回 200；`/control/chat` 可打开。
2. WS 已连接（页面右上角状态为「已连接」，无「网关未连接」toast）。
3. `tools.catalog` 返回 60 项，且包含 `write_file / shell_exec / code_exec / browser / computer_use / office_automation / system_control / web_fetch`。
4. 模型配置可用：`config.get` 的 provider 与 `models.json` active 档案同源；发起一次对话能得到非 401 的回复。
5. 测试前记录基线：`usage.query` 的累计 token 与 `C:\Users\Administrator\.marguerite\usage.db` 的 mtime。

### 0.2 缺陷分级

| 级别 | 定义 | 处理 |
| --- | --- | --- |
| P0 | 页面白屏 / 崩溃 / 核心链路不可用 / 数据丢失 | 立即停测修复，修复后回归全部相关用例 |
| P1 | 主要功能失效但有替代路径 / 数据不一致 / 明确误导性文案 | 当轮修复并补回归测试 |
| P2 | 体验缺陷（遮挡、错位、无反馈、性能抖动） | 当轮修复或登记待排期 |
| P3 | 文案/样式微瑕 | 登记，不阻塞 |

### 0.3 缺陷记录模板（每个缺陷一条，附截图与日志）

```
[DEF-xxx] 标题（一句话）
级别: P0/P1/P2/P3    用例: <模块-序号>    发现时间: YYYY-MM-DD HH:mm
环境: 版本/入口/浏览器
前置: …
步骤: 1) … 2) … 3) …
实际: …
预期: …
证据: docs/qa-shots/<file>.png；控制台/日志摘录
根因: （定位后填）
修复: commit/文件:行；回归用例: <模块-序号>
```

### 0.4 统一判定口径（所有用例附加项）

- **响应**：点击后 ≤ 300ms 有可见反馈（loading/禁用态/内容变化）；RPC 调用 ≤ 5s 返回或进入明确的进行中态。
- **无崩溃**：页面不白屏、不出现 Vue 运行时错误、控制台无 `Uncaught`。
- **无卡顿**：连续 10 次交互无 >1s 的主线程长任务（3D 页面允许首帧 ≤ 3s）。
- **状态一致**：见 `SYS-03 重启一致性`（每模块必测）。

---

## 1. Chat（对话 / 单代理）

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| CHAT-01 空状态 | 新会话 `/chat/new` | 打开页面 | 显示空态引导与输入框，无历史残留 | 无报错；输入框可聚焦 |
| CHAT-02 单轮对话 | 模型可用 | 输入「只回复两个字：收到」→ 发送 | 流式渲染回答；用户消息与助手消息顺序正确 | 24h 内回复含「收到」；无乱码/截断；刷新后两条消息仍在 |
| CHAT-03 流式中断 | 回答进行中 | 点「停止」 | 立即停止，保留已生成内容，输入框恢复 | 停止后不再追加文本；按钮回到发送态 |
| CHAT-04 长文本渲染 | 模型可用 | 发送「用 Markdown 输出 30 行表格」 | 表格/代码块正常渲染，可横向滚动不撑破布局 | 无溢出、无截断、滚动条可用 |
| CHAT-05 附件 | 有本地图片 | 拖拽图片进输入框 → 发送 | 附件出现在消息中；模型能感知图片或明确说明不可见 | 上传成功（非 401/413）；消息含附件卡片 |
| CHAT-06 工具调用可见 | 模型可用 | 发送「用 write_file 写 a.txt 内容 ok」 | 对话内出现工具调用卡片（名称/参数/结果） | 工具卡片可见；磁盘真实出现 a.txt |
| CHAT-07 会话切换 | ≥2 个会话 | 在侧栏切换会话 | 消息、草稿、编排状态按会话隔离 | 不串会话；切换 ≤1s |
| CHAT-08 草稿持久化 | — | 输入半句话后刷新页面 | 草稿恢复 | 文本一致（含光标可继续输入） |
| CHAT-09 编排档位一致性 | 全局档位=标准·方案 | 打开编排面板查看档位 → 用顶栏切到标准·执行 → 再看面板 | 面板档位跟随顶栏变化 | 两处显示一致；此时发送蜂群不再被「档位不允许」拒绝 |
| CHAT-10 蜂群全链路可见 | 档位=执行；蜂群开启，规模 3 | 发送「用 write_file 写 ui.txt 内容 ok 并回报」→ 确认 | 出现「蜂群」卡片：3 名成员、每人工具行、耗时、最终答复 | 成员数=3；工具行含 write_file；卡片不遮挡；刷新后卡片仍在（本地快照） |
| CHAT-11 编排失败可见 | 断网或改错 key | 发送蜂群任务 | 卡片进入「执行失败」并显示原因 | 有明确错误文案，不无限转圈 |
| CHAT-12 压缩/上下文 | 长会话 | 触发上下文压缩 | 显示压缩事件，历史仍可读 | 无消息丢失；事件卡片出现 |

## 2. Task（任务工作区）与 Sessions

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| TASK-01 空态 | /task | 打开 | 无任务时显示引导，不报错 | 无 RPC 错误提示 |
| TASK-02 步骤流水线 | 有一次运行 | 打开任务页 | 步骤状态（待办/进行/完成）与真实运行一致 | 与 Chat 编排事件时间线不矛盾 |
| TASK-03 交付物 | 有产物 | 查看交付物区 | 列出产物并可下载/打开 | 文件存在且可打开 |
| SESS-01 会话列表 | ≥3 会话 | 打开 /sessions | 按最近活跃排序；显示消息数/时间 | 与 sessions.db 一致 |
| SESS-02 删除会话 | 有可删会话 | 删除 → 确认 | 列表移除，Chat 不再可进入 | 刷新后仍不存在 |

## 3. Usage / Token 计量（专项，见 §8 详细流程）

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| TOKEN-01 页面加载 | 网关就绪 | 打开 /usage | 展示总量/明细/币种切换/刷新/导出 | 首屏 ≤3s；无空白面板 |
| TOKEN-02 单次请求计量 | 记录基线 | 见 §8 | 增量 = 实际 input+output token | 误差 0（与 usage.query 逐字段一致） |
| TOKEN-03 实时性 | 连续两次请求 | 观察页面 | 每次完成后 5s 内数值更新 | 无需手动刷新 |
| TOKEN-04 重启持久化 | 记录退出前数值 | 彻底退出后重开 | 数值与退出前一致 | 误差 0 |
| TOKEN-05 导出 | 有数据 | 点导出 CSV | 下载成功，行数与明细一致 | 文件可被 Excel 打开，数值一致 |
| TOKEN-06 币种切换 | 有数据 | 切 USD/CNY | 数值按汇率换算且提示口径 | 两种币种相加关系正确 |

## 4. Pet（桌宠，专项，见 §9）

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| PET-01 目录 | /pet | 打开 | 显示 5 款皮肤，全部有预览 | 5/5 有预览，无空白格 |
| PET-02 逐款切换 | 同上 | 依次点击 5 款 | 舞台重建，模型完整 | 每款无缺件/错位；切换后状态栏同步 |
| PET-03 人形完整 | 人形皮肤 | 观察头/发/身/四肢/饰品 | 部件齐全，层次正确 | 无漂浮部件、无穿透错位 |
| PET-04 情绪/动作 | 任一皮肤 | 抚摸 / 投喂 | mood 与能量/好感度变化，表情跟随 | 数值单调变化；表情与 mood 文案一致 |
| PET-05 桌宠悬浮窗 | 桌面壳 | 设置→宠物 开启桌面桌宠 | 悬浮窗出现、可拖拽、双击抚摸 | 无白底/黑底；拖拽后位置保持 |
| PET-06 多视角（3D） | — | 尝试切换视角 | **当前无 3D 类桌宠**（全部为矢量 2.5D），能力缺失 | 记为能力缺口 GAP-01，非通过 |
| PET-07 状态持久化 | 改皮肤/互动 | 重启应用 | 皮肤与好感度/经验保持 | 与退出前一致 |

## 5. 皮肤中心 / 壁纸 / 主题（专项，见 §10）

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| SKIN-01 预设壁纸 | /wallpaper | 逐个点击 6 款预设 | 背景立即切换；「使用中」徽标跟随 | 6/6 生效，无 404/空白 |
| SKIN-02 预设持久化 | 选任一预设 | 刷新页面 / 重启 | 壁纸与徽标恢复 | 与退出前一致 |
| SKIN-03 本地图片 | 有 jpg ≤2MB | 选择图片 | 应用为背景并持久化 | 刷新后仍在；>2MB 时如实提示「仅本次会话」 |
| SKIN-04 沉浸预览 | 已选壁纸 | 点沉浸式预览 → 点击退出 | 全屏展示，点击即退 | 退出后界面完全恢复 |
| SKIN-05 主题遍历 | 设置→外观 | 逐个切换全部可选主题 | 全局 token 生效：顶栏/侧栏/卡片/输入框/图表 | 每个主题均无未换色区域；对比度可读 |
| SKIN-06 主题持久化 | 选非默认主题 | 刷新 / 重启 | 主题保持 | 无首帧闪烁（anti-flash 生效） |
| SKIN-07 预览素材 | 外观面板 | 检查缩略/色板 | 无破图、无模糊、无错位 | 全部素材 200；布局对齐 |

## 6. 能力与执行类界面

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| CAP-01 能力中心 | /capabilities-hub | 打开并展开分组 | 分组/条目来自 `capabilities.groups` | 条目不空；与 tools.catalog 语义一致 |
| CAP-02 专家中心 | /expert-center | 打开 | 预设智能体/专家库/蜂群规格可见 | 无报错；可创建/选择 |
| EXE-01 终端 | /terminal | 执行 `echo hi` | 输出 hi | 输出与退出码正确；PTY 不假死 |
| EXE-02 编辑器 | /editor | 打开文件 → 编辑 → 保存 | 内容落盘 | 重新打开内容一致 |
| EXE-03 浏览器驾驶舱 | /browser-cockpit | 打开 about:blank / 截图 | 有真实页面与截图 | 非占位图；CDP 状态真实 |
| EXE-04 应用自控 | /app-control | 刷新清单 → 点击某元素 | 元素清单来自真实 DOM；操作生效 | 操作日志有记录 |
| EXE-05 电脑操控 | /computer-use | `cursor_pos` / 截图 | 返回真实坐标与截图 | 坐标在屏幕范围内；截图非空 |
| OPS-01 运维面板 | /ops | 打开 | 主机/状态/文件/进程真实 | 数值与系统一致（内存/CPU） |
| ENG-01 引擎健康 | /engine | 打开 | doctor 9 项全部有结论 | 无「检查中」卡死；失败项有原因 |
| ENG-02 本地模型 | /local-models | 探测硬件 → 推荐 | 推荐与硬件匹配，标注本地/云 | 不虚构已安装模型 |
| CRON-01 定时任务 | /cron | 新建 1 分钟后任务 | 到点执行并留痕 | 状态从 pending→done；可删除 |
| SKL-01 技能/渠道 | /skills | 打开并刷新 | 技能数、需配置数、提案数真实 | 统计与列表一致 |
| GRAPH-01 知识图谱 | /graph | 打开项目图 | 渲染节点关系，无空画布报错 | 有节点或明确空态说明 |
| OVW-01 状态总览 | /overview | 打开 | 就绪项/告警项真实 | 与 doctor 结论一致 |

## 7. 娱乐与 3D 界面

| 用例 | 前置 | 步骤 | 预期 | 通过标准 |
| --- | --- | --- | --- | --- |
| ISL-01 岛屿工作区 | /island | 打开 | 3D 场景加载，事件驱动建筑 | 首帧 ≤3s；无 WebGL 时降级 2D 并说明 |
| ISL-02 岛屿游戏 | /island-game | WASD 漫游 | 移动/视角正常，无穿模 | 帧率 ≥30；相机不翻面 |
| GAME-01 游戏大厅 | /games | 打开并进入一个游戏 | 可玩、可退出 | 无白屏；分数/状态正确 |
| POKER-01 德州 | /poker | 开一局 | 发牌/下注/结算流程正确 | 数值守恒；可离开 |
| DT-01 设备孪生 | /device-twin | 打开 | 3D 冰箱可旋转/拾取部件 | 无穿模；拾取高亮正确 |
| BR-01 大脑/Viky | /brain | 打开 | 八维情感/Φ 为真实引擎值 | 非随机数；刷新后稳定 |
| FLEET-01 舰队 | /fleet | 打开 | 角色/路由策略可见 | 与模型配置同源 |

## 8. Token 计量专项流程（按用户指定顺序）

1. **前置**：应用正常启动；`/usage` 已加载完成；记录基线 `usage.query` 的 `total_tokens_in/out`、`requests`，并记 `usage.db` 的 mtime。
2. **步骤 1**：在 Chat 发起一个简单请求（「只回复两个字：收到」），记录请求前后的基线差值。
3. **步骤 2**：验证回复渲染——字号/换行/标点正确，无 `�` 乱码、无省略号截断、无溢出。
4. **步骤 3**：切到 `/usage` 或读数：总 token 增量应等于本次请求的 `prompt_tokens + completion_tokens`；同一请求在明细行中可定位；页面无需手刷即更新。
5. **步骤 4**：完全退出（结束 `marguerite-gui.exe` 与 `pyengine` 进程）→ 重新启动 → 打开 `/usage`。
6. **步骤 5**：数值与退出前一致（允许 0 误差；任何差额必须能由退出期间的新请求解释）。

**判定**：`ΔUI = ΔDB = Δ(provider usage)`；三者任一不等即 P1。

## 9. 桌宠专项流程

1. 打开 `/pet`，确认皮肤目录 5 款（`pet.list` 权威；离线兜底也不得少于 5）。
2. 逐款点击：每次切换后检查 SVG 舞台是否重建（节点数/尺寸变化）、角色是否完整（头/发/身/四肢/装饰）。
3. 人形款逐项核对：无漂浮部件、无错位、发饰/裙摆层次正确、无被裁切。
4. 交互：抚摸（+好感/心情）、投喂（+能量），数值与表情同步。
5. 悬浮窗：设置→宠物 开启桌面桌宠；检查透明背景（无黑底/白底）、拖拽、双击抚摸。
6. 重启后：皮肤、能量、好感度、经验与退出前一致。
7. **3D 多视角**（2026-10-23 已落地）：桌宠页顶部「3D 立体」开关 → 真正的 Three.js 场景（不是翻转贴图）。
   - 视角：正面 / 侧面 45° / 正侧面 / 背面四个预设 + 拖拽自由环绕 + 滚轮缩放 + 自动旋转；
   - 判定：每次切视角 `data-pet3d-azimuth` 必须落到 0 / -0.785 / -1.571 / 3.142；四张截图像素不相同；
     背面必须能看到后发 / 裙背 / 翼背面；无 `pageerror`、无贴图缺失（材质全部为纯色 + 卡通渐变，无外部纹理）；
   - 降级：WebGL 不可用时按钮禁用并说明「已回退 2.5D」，不得留白屏。

## 10. 皮肤中心专项流程

1. 打开 `/wallpaper`，逐个点击 6 款预设，检查背景即时生效与「使用中」徽标。
2. 刷新页面（不重启），确认预设恢复（本地快照）；再彻底重启确认。
3. 选择本地图片（≤2MB）→ 刷新 → 仍在；选 >2MB 图片 → 提示「仅当前会话生效」。
4. 沉浸式预览：进入/退出均正常，退出后布局无残留遮罩。
5. 进入 设置 → 外观：逐个切换全部可选主题（10 个 value theme + system），每个主题检查：顶栏、侧栏、卡片、输入框、按钮、图表、代码块、滚动条、悬浮层。
6. 每个主题截图；对比度不足即 P2；存在未换色的硬编码色即 P1。
7. 重启后主题保持，且首帧不闪回默认主题。
8. **面板遮罩（2026-10-23 新增）**：拖动「面板遮罩」滑块 45%↔95%，`--wp-panel-alpha` 与 `.content` 计算背景必须同步变化；
   图片壁纸时守卫可自动抬升（页面提示「已按对比度守卫抬到 N%」），正文对比度不低于 4.5:1；强度刷新/重启后保持。

## 11. 统一验收（每个模块都跑）

| 用例 | 步骤 | 通过标准 |
| --- | --- | --- |
| SYS-01 响应 | 每页主要按钮连续点击 10 次 | 均有 ≤300ms 反馈；无重复提交 |
| SYS-02 稳定性 | 全站巡检一遍（脚本遍历全部路由） | 无白屏、无 `Uncaught`、无 >5s 空白 |
| SYS-03 重启一致性 | 记录状态 → 彻底退出 → 重开 | 会话/设置/壁纸/主题/皮肤/用量/产物均与退出前一致 |
| SYS-04 离线性 | 断开侧车 | 页面给出明确降级提示，不假装成功 |

---

## 12. 执行记录（2026-10-23 第一轮）

执行方式：
- **服务端契约取证**：WS 探针（`.tmp/_token_probe.mjs`、`.tmp/_e2e_progress.mjs`、`.tmp/qa-pet-rpc.mjs`）直连 `ws://localhost:18790/ws`，绕过 UI 读真实 RPC。
- **UI 取证**：Playwright（`.tmp/qa-confirm-export.mjs`、`.tmp/qa-401.mjs`、`.tmp/qa-theme-wallpaper.mjs`、`.tmp/qa-verify-fixes.mjs`、`.tmp/qa-route-sweep.mjs`）+ 浏览器代理人工巡检（桌宠 / 壁纸 / 主题三份记录）。
- 证据目录：`docs/qa-shots/`。

### 12.1 用例结果

| 用例 | 结果 | 证据 / 说明 |
| --- | --- | --- |
| CHAT-01 空状态 | PASS | 新会话可打开，输入框可聚焦 |
| CHAT-02 单轮对话 | PASS | 回复「收到」；`sessions.db` 有 user+assistant 两条 |
| CHAT-06 工具调用可见 | PASS | 蜂群卡片出现 `write_file` 行，磁盘真实落盘 `e2e-swarm.txt` |
| CHAT-09 档位一致性 | PASS（已修） | 发送门禁改读全局档位 store，顶栏变化回填面板 |
| CHAT-10 蜂群全链路 | PASS（已修） | 3 名成员 / 6 次工具调用 / 最终答复；徽标「蜂群」 |
| CHAT-11 编排失败可见 | PASS | 失败路径发 `run_failed`，卡片显示原因 |
| TOKEN-01 页面加载 | PASS（已修） | KPI 非零：见 §12.2 |
| TOKEN-02 单次计量 | PASS | 1 次请求 → `+787 input / +1 output / +1 request`，与明细行逐字段一致 |
| TOKEN-03 实时性 | PASS | 返回用量页即已更新（无需手刷） |
| TOKEN-04 重启持久化 | PASS | 见 §12.3 |
| TOKEN-05 导出 | PASS | Playwright 实际下载 `Marguerite-usage-7d-cny7.25.csv`（浏览器代理环境禁止下载，非产品缺陷） |
| TOKEN-06 币种切换 | PASS | `$0.2167 ⇄ ¥1.5712` 双向换算一致 |
| PET-01 目录 | PASS | 5 款皮肤全部有预览 |
| PET-02 逐款切换 | PASS | SVG 节点 70–217，切换后舞台重建、高亮唯一 |
| PET-03 人形完整 | PASS | 尖耳/刘海/发饰/翅膀/胸衣/裙摆/花簇/底座齐全，无缺件 |
| PET-04 情绪/动作 | PASS | 抚摸 0→15 能量、好感 +1；投喂 15→40 |
| PET-05 悬浮窗 | N/A（浏览器不可达） | `/control/pet-overlay` 404（桌面壳专属路由，Web 构建未注册） |
| PET-06 3D 多视角 | **PASS（2026-10-23 落地）** | 新增 `pet3d/` 真三维渲染：四视角 azimuth 0 / -0.785 / -1.571 / 3.142 逐个命中；拖拽后 `view=free`；自动旋转角度持续变化；5 款皮肤均可切 3D；截图见 `docs/qa-shots/qa-pet3d-*.png` |
| PET-07 状态持久化 | PASS | 皮肤/能量/好感度/经验刷新后保持；能量衰减已修（见 DEF-10） |
| SKIN-01 预设壁纸 | PASS（已修） | 6/6 应用成功，徽标唯一 |
| SKIN-02 预设持久化 | PASS（已修） | 刷新后 `{type:'preset',id}` 恢复并「使用中」 |
| SKIN-03 本地图片 | 未执行 | 需准备真实图片文件，留待第二轮 |
| SKIN-04 沉浸预览 | PASS | 进入/退出无残留遮罩（首点偶发需二次点击，见 P3-2） |
| SKIN-05 主题遍历 | PASS | 10 主题 + 跟随系统全部生效；`data-theme`/body/侧栏 800ms 内同步 |
| SKIN-06 主题持久化 | PASS | 非默认主题重载后保持，首帧无闪白 |
| SKIN-07 预览素材 | PASS | 全为令牌色板，无破图/错位 |
| SYS-01 响应 | 部分 | 刷新按钮无 spinner / 无「已刷新」反馈（P3-3） |
| SYS-02 全站巡检 | 见 §12.4 | 30 路由遍历 |
| SYS-03 重启一致性 | 见 §12.3 | 用量 / 壁纸 / 主题 / 桌宠 |
| SYS-04 离线性 | 未执行 | 需断侧车，留待第二轮 |

## 13. 缺陷与修复清单

| 编号 | 级别 | 现象 | 根因 | 修复 | 状态 |
| --- | --- | --- | --- | --- | --- |
| DEF-01 | P0 | 用量页 KPI 全为 0；明细只覆盖最近 200 条 | Rust `usage.query` 只回 `{items}`，前端只读 `totals/sessions/models/days` | Rust 实现四视图契约 + 全量聚合（单趟 GROUP BY），保留 `items` 兼容 | 已修（测试 `query_contract_*`） |
| DEF-02 | P1 | 所有流式用量归到空会话，按会话统计全错 | `tokio::spawn` 不继承 task-local；`usage::record` 在生产者任务里读到空 SESSION | spawn 前 `current_session()` 取值，改为 `record_attributed` 显式传入 | 已修（实测 1 请求 → 1 行且 session 正确） |
| DEF-03 | P1 | 明细表「修改时间 / 来源 / 模型」三列全空 | `usage.query` 未返回 `lastUsageAtMs / firstUsageAtMs / modelBreakdown / costSource` | 聚合补首末时间、按会话模型拆分、`costSource=Marguerite_estimate` | 已修 |
| DEF-04 | P1 | 壁纸应用后整页看不到（提示已应用） | `.content` 不透明底把 body 背景 100% 覆盖 | `html[data-wallpaper]` 下内容面板半透明（62% / 聊天 70%） | 已修 |
| DEF-05 | P1 | 壁纸刷新/重启后丢失（预设完全不落盘） | 只有图片走 localStorage，预设无持久化；且仅壁纸页恢复 | v2 存储格式 + `utils/wallpaper.ts` 统一，App 挂载时冷启动恢复 | 已修 |
| DEF-06 | P1 | `/api/approvals`、`/api/system/update`、`/api/elevated-mode` 全 401 | 注入 token 只用于 WS，未写回 sessionStorage；REST 辅助函数只读 sessionStorage | `rpc.init()` 把注入 token 落盘；实测 `/api/*` 全部 200 | 已修 |
| DEF-07 | P2 | 编排泳道刷新即消失 | 时间线只在内存 | 会话维度 localStorage 快照（只存终态，running 不落盘） | 已修（含 2 个单测） |
| DEF-08 | P2 | 方案档下先弹确认→确认后被拒，连拒两次且不说怎么改 | 发送门禁读编排面板的本地档位副本，与顶栏全局档位漂移 | 门禁改读全局 store（`gearKey`）+ 档位不满足时给可操作提示 | 已修 |
| DEF-09 | P2 | 发送后蜂群浮层压住编排卡片 | 提交前未收浮层 | 发送按钮先 `closeAllPopovers()` | 已修 |
| DEF-10 | P2 | 「并行规模 3」实际只跑 1 个成员；能量 29 秒不衰减 | 规模参数从未传到后端；衰减按「每次调用取整 10 秒」且每次重置 `last_ms` | `members` 全链路传递 + 真扇出；衰减只消费整 10 秒、余数保留 | 已修（3 成员实测；单测 `status_accumulates_decay_across_frequent_polls`） |
| DEF-11 | **P0** | 任意对话都返回「本轮失败：请求上游模型服务失败（HTTP 400）」 | 生效模型 id `deepseek-v4.1-flash` 在 DeepSeek 侧不存在：`GET /models` 只列 `deepseek-flash` / `deepseek-v4-pro`，直接 POST 该 id 稳定 400；`config.yaml::model.model_name` 与 `models.json::profiles.deepseek-primary.model` 同时指向它 | 以供应商权威列表为准改回可用 id（`deepseek-v4-flash`，实测 200）；`config.yaml` / `models.json` / `.env` 三处对齐 | 已修（UI 实测回复「收到」，用量行 `deepseek-v4-flash`） |
| DEF-12 | P1 | 彻底退出重开后桌宠皮肤从 pink 退回 cat、心情退回 normal | `gateway/pet.rs` 的 skin/mood 只存在进程内 `OnceLock`；能量/好感在 `pet_state.json`，UI 侧偏好没有落盘 | 新增 `pet_prefs.json`（原子写），`shared()` 首次初始化从盘读取，`setSkin/setMood/triggerAction` 写盘；单测 `prefs_file_roundtrip_survives_restart` | 已修（pink 跨重启保持） |
| DEF-13 | P1 | 桌宠页 PSI 八维情感恒为默认 0.5（`brain.status` 从未被调用） | `getRpcStore_()` 用 `require('@/stores/rpc')`：Vite 产物没有 `require`，ReferenceError 被 catch 吞掉后恒返回 null | 改为静态 `import { useRpcStore }`；轮询与 `psi_broadcast` 订阅恢复 | 已修（`vue-tsc` 0 错） |
| DEF-14 | P2（合规/体积） | 随包分发带第三方水印的 AI 位图：`pets/seedream-pet.png`（3.7MB，右下角水印）、`brand/wallpaper.jpg`（左上角水印）；另 4 张未被任何代码引用的动漫人物立绘 | 早期素材直采，未做素材审校 | 删除 5 张未引用立绘与带水印壁纸/立绘；「玛格丽特·星夜」预设改写为主题令牌渐变（保留原 id 兼容旧快照）；桌宠改为纯本地矢量/3D 渲染，不随包分发人物位图 | 已修（`dist` 313 文件，无外部人物素材） |
| P3-1 | P3 | 桌宠心情刷新后回到初始（能量/好感正常） | 心情读进程内状态而非落盘 | 见 DEF-12：皮肤/心情已落 `pet_prefs.json`；能量低时 `DesktopPet` 的自动 `doze` 属产品行为 | 已修 |
| P3-2 | P3 | 沉浸式预览偶发首次点击无响应 | 未定位（按钮 disabled 态与点击同帧竞态） | 第二轮 3/3 立即点击均生效，**复现失败**，不再当作缺陷改代码 | 关闭 |
| P3-3 | P3 | 用量页手动刷新无 loading / 成功反馈 | 交互反馈缺失 | 刷新中禁用并换文案 + 成功后 toast「用量数据已刷新」 | 已修 |
| P3-4 | P3 | 重启后首屏瞬时显示上一进程的「宿主进程已退出」卡片 | 退出证据跨重启落盘；新进程侧车注册前的窗口内 `sidecar_present=false` | 落盘记录（`source:'record'`）加 12s 沉降窗口：本次运行真实观测到的退出立即显示，历史记录须等侧车始终未注册才显示（`HostExitCard.vue` + `useHostExitEvidence.ts`，6 项回归测试） | 已修（部署后实测 t+3s / t+15s 均不再出现） |
| GAP-01 | 能力缺口 | 无 3D 立体桌宠、无多视角切换 | 产品未实现（现有为 SVG 2.5D + 伪光照） | **已实现（2026-10-23）**：`pet3d/buildPet3D.ts` 参数化建模 + `Pet3DScene.ts` 舞台（四视角/拖拽/自动旋转/缩放）+ `views.ts` 角度数学；WebGL 不可用回退 2.5D；单测 10 项、e2e 四视角取证 | 已实现 |

### 13.1 复现失败（记录以免误修）

| 报告 | 复核方式 | 结论 |
| --- | --- | --- |
| 主题切换后外壳不跟随，需刷新 | Playwright 逐主题读 `data-theme` + body/侧栏计算色 | **不成立**：800ms 内全部同步；此前观察受设置遮罩压暗与切换动画影响 |
| 「导出」点击无反应 | Playwright 监听 download 事件 | **不成立**：实际下载成功（浏览器代理环境禁下载） |
| 确认框「取消」点不动、需空格 | Playwright 点击取消 | **不成立**：一次点击即关闭且保留草稿 |
| 桌宠缺件 / 空白 | 逐款读 SVG 节点 + 截图 | **不成立**：5 款全部完整渲染 |
| 用量台账「同一请求被记 2–3 次」 | 直查 `usage.db`：`(session, created_ms, model, tokens)` 分组 | **不成立**：同组三行时间戳相差 9 / 26 / 70 ms，是 Bloom 并行 3 个提案者各发一次真实请求（各自计费），不是重复写入；真重复写入会共享同一个 `created_ms` |
| 沉浸式预览首点无响应 | 应用预设后**不等待**立刻点按钮，重复 3 轮 | **不成立**：3/3 首点即开，`disabled` 态未产生可观测竞态（P3-2 关闭） |

## 14. 第二轮待办（2026-10-23 21:2x 复测后）

1. ~~SKIN-03 本地图片壁纸（含 >2MB 不持久化提示）~~ → **已执行**，见 §15。
2. ~~SYS-04 侧车离线降级~~ → **已执行**，见 §15。
3. PET-05 桌面悬浮窗：需在 Tauri 桌面壳内实测（`/control/pet-overlay` 在 Web 构建里 404）。
4. ~~P3 项排期：桌宠心情持久化、刷新反馈、沉浸预览首点~~ → 前两项已修，第三项复现失败关闭；新增 P3-4（重启首屏瞬时宿主退出卡片）。
5. ~~全量 `cargo test`（lib 2225+）与 WebUI 全量 vitest 回归~~ → **已执行**：`cargo test -p marguerite-core --lib` → 3549 passed / 0 failed / 19 ignored；`npm run test:unit` → 405 文件 / 4140 用例全绿。

## 15. 第二轮执行记录（构建部署后复测，2026-10-23 21:20–21:55）

**被测产物**
- `marguerite-gui.exe` sha256 `EB8001FB66108FFB68DA08C3B9FB50D9420260B127A6C0F14109B4628F302A74`
- `webui/dist` 314 个文件，入口 `assets/index-D0dU_pgP.js`（含 3D 桌宠、面板遮罩、宿主卡片沉降窗口）
- 网关 `127.0.0.1:18790`（engine=rust），Python 侧车 `opensquilla.cli.main gateway` 常驻，模型 `deepseek-v4-flash`

### 15.1 Token 计量专项（用户指定 5 步，逐步取证）

| 步骤 | 动作 | 结果 | 证据 |
| --- | --- | --- | --- |
| 前置 | 应用启动 + `/usage` 加载完成，记录全量基线 | PASS | `usage.query` 全量：input 8,257,910 / output 651,001 / requests 10,423 |
| 1 | 在 Chat 发「只回复两个字：收到」（DeepSeek） | PASS | 回复正文含「收到」，模型 `deepseek-v4-flash` |
| 2 | 渲染校验 | PASS | 无 `U+FFFD`、无结尾省略号、字号 16px/行高 24.8px、`scrollHeight == clientHeight`（未裁切）、整页无乱码 |
| 3 | 计量准确性 + 实时性 | PASS | 明细新增 1 行 `812 / 2 / 1 请求`；`usage.query` 增量 = 812 / +2 / +1，**与明细逐字段相等**；返回用量页即已更新 |
| 4 | 彻底退出（`marguerite-gui` 进程数归 0）后重开 | PASS | 重开后进程 1 个、网关重新监听 |
| 5 | 历史数据持久化 | PASS | 退出前 UI：`549,181 tok / $0.2330 / 24 会话 / $0.0097`；重开后完全一致；明细行 `812 / 2` 仍在 |

> 结论：`ΔUI = ΔDB = Δ(provider usage)` 三者一致；本轮未发现计量漂移。

### 15.2 桌宠专项（用户指定遍历 + 3 个验证点）

| 验证点 | 结果 | 证据 |
| --- | --- | --- |
| 逐款切换 5 款 | PASS | 5/5 卡片可点，选中态唯一（`is-active` 恒为 1） |
| 无缺失 / 加载失败 / 空白 | PASS | 舞台 `806×380`，SVG 节点 170 / 217 / 214 / 213 / 213；无 `pageerror`、无乱码 |
| 人形款部件完整 | PASS | 截图 `docs/qa-shots/qa-pet-awake-1..4.png`：尖耳、刘海、发饰、双翼、胸衣、裙摆、双手、花束、花簇、底座、投影齐全；能量喂满（100）后姿态一致 |
| 交互 | PASS | 抚摸 13→27（含期间 1 点衰减）、好感 +1；投喂 27→52 |
| 3D 立体类多视角 | **GAP-01（不适用）** | 产品当前全部为 SVG 2.5D，无任何 3D 模型/视角控件；不得记为通过 |
| 能量衰减 | PASS | 投喂到 25 后：+13s → 24，+29s → 23 |
| 皮肤跨重启 | PASS（DEF-12 修复后） | 设 pink → 彻底退出重开 → `pet.status.skin = pink` |

### 15.3 皮肤中心专项

| 验证点 | 结果 | 证据 |
| --- | --- | --- |
| 6 款预设逐一点击 | PASS | 6/6 应用成功，「使用中」徽标唯一；缩略图 185×96，均有实际背景（1 图 + 5 令牌渐变） |
| 预览图无模糊/错位 | PASS | `background-image` 均有效（非 `none`），无空占位 |
| 应用后全局生效 | PASS | `html[data-wallpaper=1]`；body 背景 = 所选预设；内容面板半透明（浅色 62% / 聊天 70%） |
| 刷新/重启保持 | PASS | 刷新后徽标与背景仍在；应用重启后 Chat 冷加载即恢复（`ink` 预设 + Terminal 主题） |
| 本地图片 ≤2MB | PASS | `logo.png`（1930 KB）→ 「已持久化，刷新后保留」，localStorage 2,634,988 B，刷新后仍生效 |
| 本地图片 >2MB | PASS | `seedream-pet.png`（3642 KB）→ 「超过 2MB，仅当前会话生效」，未写坏持久化（存储保持上一张） |
| 10 个 value theme 遍历 | PASS | `light/dark/arctic/crt-green/ember/ink/miami/synthwave/terminal/vapor` 全部 `data-theme` 切换，body/sidebar 计算色 10 个互不相同；「跟随系统」正确落到浅色 |
| 沉浸式预览 | PASS | 3/3 首点即开（P3-2 复现失败） |

### 15.4 统一验收

| 用例 | 结果 | 证据 |
| --- | --- | --- |
| SYS-01 响应反馈 | PASS | 用量页刷新：按钮 270ms 内变「刷新中」并禁用，成功 toast「用量数据已刷新」 |
| SYS-02 全站巡检 | PASS | 31 条路由 0 失败，单页首屏 2.29–2.62s，无空白/无 `Uncaught` |
| SYS-03 重启一致性 | PASS | 主题（Terminal）+ 壁纸（水墨）+ 桌宠皮肤（pink）+ 用量数值，四项跨应用重启保持一致 |
| SYS-04 侧车离线降级 | PASS | 强杀侧车后：`/api/elevated-mode`、`/api/usage` 返回 `502 sidecar_unreachable` 并附原因；`/api/system/update` 回落本地值（`latest:null`，不假装已检查）；侧车 18s 内自动重启，`sidecar_present=true` |

### 15.5 异常记录方式（每个失败项必填）

1. 截图：`docs/qa-shots/<用例编号>-<时间戳>.png`（全页 + 关键区域各一张）。
2. 日志：`~/.marguerite/`（`usage.db` / `sessions.db` / `pet_state.json` / `pet_prefs.json`）与 `AppData/Local/Marguerite/pyengine-home/default/logs/debug.log` 的相关片段。
3. 复现步骤：前置状态 → 逐步操作 → 期望/实际 → 稳定性（n/10）。
4. 判定：P0 阻断（无法使用）/ P1 数据错误或状态丢失 / P2 功能可用但行为不符 / P3 观感与反馈 / GAP 能力缺口。不允许用「大概」「可能」描述；未复现的一律进 §13.1 并注明判定依据。

### 15.6 构建门禁（本轮收尾）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| Rust 单测 | `cargo test -p marguerite-core --lib` | 3549 passed / 0 failed / 19 ignored |
| Rust 静态检查 | `cargo clippy -p marguerite-core --all-targets` | 0 warning / 0 error |
| WebUI 单测 | `npm run test:unit`（vitest） | 405 文件 / 4140 用例全绿 |
| WebUI 守卫 | `npm run typecheck` | 主题契约 / RPC 契约（388 声明全部有实现，幽灵 0）/ i18n 六语种等全部 PASS |
| WebUI 构建 | `npm run build` | 315 文件产物，主题与运行时包守卫通过 |
| 部署对账 | exe sha256 / `index.html` 入口引用 | 与构建产物逐字节一致（`assets/index-BptD3J8q.js`） |

## 16. 第三轮：3D 桌宠多视角 + 主题×壁纸落地（2026-10-23 22:40–23:35）

### 16.1 交付内容

| 模块 | 新增/变更 | 说明 |
| --- | --- | --- |
| 3D 建模 | `webui/src/components/pet/pet3d/buildPet3D.ts` | 由 `PetModelSpec` 参数化生成真三维网格：兽形（躯干/肚皮/四爪/项圈铃铛/头/锥耳+内耳/四层眼/眉/口鼻/胡须/腮红/球尾）；人形（腿/芭蕾鞋/花瓣裙/腰带/上身/双臂+手/颈/头/后发+长发+发梢/侧鬓/齐刘海/精灵尖耳/四层眼/眉/嘴/腮红/花冠/四片蝶翼/捧花/光环）。材质统一走 `PET_TOON_RAMP` 六档渐变，不加载任何外部纹理 |
| 3D 舞台 | `pet3d/Pet3DScene.ts` | 相机/灯光取自 `PET_CAMERA` / `PET_LIGHTING`；四预设视角 + 拖拽环绕 + 滚轮缩放 + 自动旋转；`computePose` 驱动呼吸/漂浮/眨眼/耳抽/尾摆/翼振/鞠躬/蹦跳；点击部位衰减冲量；粒子爆发带上限；`dispose()` 释放几何/材质/画布 |
| 视角数学 | `pet3d/views.ts` | 视角角度表、最短角差回绕、帧率无关指数逼近、极角夹紧（纯函数，Node 可测） |
| 桌宠页 | `PetView.vue` | 「2.5D 矢量 / 3D 立体」切换（持久化 `pet.renderMode`）、四视角按钮、自动旋转、WebGL 探测与诚实降级；修复 PSI 轮询的 `require()` 失效（DEF-13） |
| 主题×壁纸 | `utils/wallpaper.ts` / `WallpaperView.vue` / `base.css` | 「玛格丽特·星夜」改为主题令牌渐变（去水印位图）；新增「面板遮罩」滑块 45–95%（`--wp-panel-alpha` / `--wp-chat-alpha`）；图片壁纸按 8×8 采样亮度做对比度守卫（WCAG AA 4.5:1），可自动抬升并提示 |
| 素材治理 | `public/pets/*`、`public/brand/wallpaper.jpg` | 删除带第三方水印的 `seedream-pet.png`、`wallpaper.jpg` 与 4 张未引用动漫立绘；`skins.ts` 同步移除 image 皮肤 |

### 16.2 验收结果

| 用例 | 结果 | 证据 |
| --- | --- | --- |
| PET3D-01 四视角 | PASS | `data-pet3d-azimuth` 依次 0.000 / -0.785 / -1.571 / 3.142；四张截图字节 106526 / 105449 / 103776 / 98175（互不相同） |
| PET3D-02 背面是真实几何 | PASS | 背面截图可见后发、裙背、双翼背面（`qa-pet3d-3-back.png`），无镜像贴图、无穿模 |
| PET3D-03 自动旋转 | PASS | 开启后 1.3s 内 azimuth 3.142 → -2.429（变化 > 0.05） |
| PET3D-04 拖拽环绕 | PASS | 拖动 160px 后 `view=free`、azimuth 3.142 → -2.299 |
| PET3D-05 五款皮肤 3D | PASS | 兽形 1 款 + 人形 4 款均渲染（`data-pet3d-kind` = beast/humanoid），无 `pageerror` |
| PET3D-06 模式切换 | PASS | 切回 2.5D 后 canvas 移除、SVG 出现、视角工具条隐藏 |
| PET3D-07 降级 | PASS（代码路径） | `isWebGLAvailable()` 失败 → 按钮禁用 + 页面提示「已回退 2.5D 矢量渲染」；构造异常另有一层 catch 兜底 |
| SKIN-08 CSS 星夜预设 | PASS | 6 款预设全部为渐变/令牌，首款 `radial-gradient(...)`；无图片依赖 |
| SKIN-09 遮罩滑块 | PASS | 默认 62%；拉到 min → `--wp-panel-alpha: 45%`、`.content` alpha 0.45；拉到 88 → 0.88 |
| SKIN-10 持久化 + 深色主题 | PASS | 刷新后仍 88% + 「使用中」；深色主题下壁纸仍可见、面板按变量走（0.95 chat） |
| 合规 | PASS | `dist` 内无带水印位图、无外部人物素材；3D 材质零纹理 |

### 16.3 已知限制（如实记录）

1. 形象为**程序化 Q 版**（非 GLB 资产）：五官/服饰由几何拼装，细节量低于美术资产；后续可平滑替换为 GLB（`buildPet3D` 是唯一入口）。
2. 蝶翼是单层双面片：正侧方 90° 附近会看到「边缘线」，属几何形态而非纹理错误。
3. 桌面悬浮窗（PET-05）仍是 2.5D 矢量：3D 只在桌宠页启用，避免常驻透明窗口的 WebGL 功耗。
4. `cargo tauri build` 的 NSIS 打包本轮未重跑（部署走 exe + `webui/dist` 对账），安装包需下次发版时统一生成。

### 16.4 第三轮门禁

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 3D 单测 | `npx vitest run src/components/pet/pet3d` | 2 文件 / 10 用例全绿（部件齐全、包围盒有限、背面体积、RGBA 渐变、材质独立、视角数学） |
| 壁纸守卫单测 | `npx vitest run src/utils/wallpaper.test.ts` | 6 用例（量程/持久化/对比度反解/未知亮度/不达标不伪装） |
| WebUI 全量单测 | `npm run test:unit` | **411 文件 / 4165 用例全绿** |
| 类型检查 | `npm run typecheck`（含架构守卫） | 0 错误；RPC 契约 389 声明全部有实现、双实现守卫 47 在册 |
| WebUI 构建 | `npm run build` | 313 文件产物，主题/运行时包守卫通过 |
| Rust 全量单测 | `cargo test -p marguerite-core --lib` | **3576 passed / 0 failed / 19 ignored** |
| Rust 静态检查 | `cargo clippy -p marguerite-core --all-targets` | 0 warning / 0 error |
| Rust 构建 | `cargo build --release --features tauri/custom-protocol` | 成功（同时补上 `ModelProfile.capabilities` 缺失字段，解除构建阻断） |
| 部署对账 | exe sha256 / `index.html` | `EB8001FB…2A74`；`assets/index-D0dU_pgP.js`；`dist` 314 文件 |

> **环境类失败已排除**：首轮全量 Rust 单测出现 2 项失败，均为环境问题而非代码回归 ——
> ① `doctor::tests::disk_free_over_threshold_ok`：C 盘仅剩 1.62 GiB（阈值 2 GiB）；
> ② `usage::tests::budget_snapshot_uses_price_table_for_usd_cap`：临时库路径在磁盘告急时打开失败。
> 清理本轮部署产生的 14 份旧 exe 备份（996 MB）与 22 份旧 `dist.bak` 后 C 盘回到 4.06 GiB，
> 两项重跑均通过（全量 3576 全绿）。

## 17. 第四轮：「玛格丽特 · 星夜」美术设计体系落地（2026-10-23 23:40 – 10-24 01:00）

### 17.1 灵感来源与合规边界

| 真实原型 | 事实（已联网核对） | 提取的设计语汇 |
| --- | --- | --- |
| 玛格丽特补给站<br>Capanna Regina Margherita | 海拔 **4554 m**，位于 Monte Rosa 山群的 Punta Gnifetti（Signalkuppe）峰顶，意大利/瑞士边界；1889 年由意大利高山俱乐部动工，**1893-08-18 揭幕**，以意大利王后 Margherita of Savoy 命名；是**欧洲最高的建筑**，长期兼作高原医学与气象观测站 | 高山夜空（近黑的夜蓝底 + 冷色星）、雪脊剪影、极光带、石木小屋的暖点（金/琥珀仅作点缀）、经线与高度感的细线（`--hairline`） |
| 玛格丽特王后<br>Margherita of Savoy | 1893 年亲自登临并主持命名 | 银紫长发、象牙长裙、**海蓝宝石**（aquamarine）、冠冕弧线（→ 花冠环）、珍珠般的圆润（大圆角 + 柔光） |
| 玛格丽特花（雏菊） | 词源同名 | 雏菊母题：章节标题前的内联 SVG、Canvas 飘落花瓣、花冠、捧花、花心金 |

**合规红线（本轮硬约束）**：只取设计语汇，**不使用任何实景照片、王室肖像、第三方生成图**。UI 内所有视觉元素由 CSS / 内联 SVG / Canvas 2D / Three.js 现场绘制；主题与桌宠不随包分发任何人物位图（上一轮已下线的 6 张带水印素材不再回归）。

来源：[SAC Route Portal — Capanna Margherita](https://www.sac-cas.ch/en/huts-and-tours/sac-route-portal/capanna-margherita-regina-margherita-2147000164/) · [Visit Monterosa — 补给站历史](https://visitmonterosa.com/dove-dormire/capanna-margherita-19794) · [Exped Tribe — Margherita Hut](https://expedtribe.com/en/margherita-hut-monte-rosa/)

### 17.2 设计系统（value theme `marguerite-night`）

| 层 | 文件 | 内容 |
| --- | --- | --- |
| L1 令牌 | `webui/src/themes/marguerite-night/tokens.css` | 29 个必需角色全覆盖。地面 `--bg #070b1a` / `--bg-surface #0d1430`；文本 `--text #eef2ff`（16.4:1）；主色 `--accent #a8c7ff`（月光石蓝，9.6:1）；次色 `--accent-secondary #7fe3d4`（海蓝宝石）；星云紫 `--queued #c9b3ff`；雏菊金 `--warn #f2cc7a`。结构身份：`--font-display` Fraunces 衬线、圆角 12/20/24/999、月光双层焦点环、冷色深空投影 |
| 世界层 | `webui/src/themes/marguerite-night/world.css` | ① 排版：衬线展示字 + 月光 text-shadow，正文 `line-height 1.78 / letter-spacing .01em`，数字 `tabular-nums`；② 留白：`.control-stage` 28/32/40、区块间距 20；③ 圆润化 + 玻璃卡片（内高光 + 冷投影 + hover 抬升 1px）；④ 主按钮「月光扫过」（`--dur-slow` 1200ms 单向扫光）；⑤ 天象：三层径向极光（46s 缓慢漂移 + blur 12px）+ 雪脊 `clip-path` 剪影（原创多边形）；⑥ 雏菊母题（内联 SVG data-URI + `mask` 跟随主色）；⑦ 侧栏 / 气泡 / 桌宠舞台 / 选区 / 滚动条；⑧ `prefers-reduced-motion` 全静态降级 |
| 注册 | `themes/marguerite-night/manifest.ts` | `kind: 'value'`，懒加载 world 样式与 Fraunces 字体（构建守卫实测：6 个 world 全部 0 字节进入入口） |

### 17.3 动态视觉（Canvas 星空）

`webui/src/components/MargueriteSky.vue` + `webui/src/utils/skyMath.ts`

- 确定性种子 `0x5f3a91`（mulberry32）：同一片天空每次打开一致；星 120±（按视口面积 0.78/1/1.25 档）、雏菊花瓣 14±、流星 9–19 s 随机。
- 极光/星云用**半分辨率离屏画布预渲染**，每帧只 `drawImage`；DPR 上限 1.5；`visibilitychange` 暂停；`prefers-reduced-motion` 只画一帧静态星空。
- **颜色零字面量**：所有色调走主题令牌（`--text` / `--accent` / `--accent-secondary` / `--queued` / `--warn`），运行时用探针元素 + `getComputedStyle` 解析，透明度一律 `ctx.globalAlpha` —— 与 `ParticleField.vue` 同一套约定，`check-webui-colors` 通过。
- 诊断：`documentElement.dataset.sky = 'running' | 'static'`。

### 17.4 听觉（Web Audio 合成环境音）

`webui/src/composables/useAmbience.ts`

- 三层：山风（低通白噪声 + 0.06 Hz LFO 缓慢起伏）+ 双正弦 pad（98/147 Hz，纯五度）+ 五声音阶三角波铃音（11–25 s 随机，2.01 倍泛音）。
- 复用全局 `audioEngine` 单一 `AudioContext`；首次需用户手势解锁（`armGestureOnce`）。
- **诚实开关**：浏览器不支持或被拒时 `setEnabled` 返回 false 并**自动回滚**开关与 `localStorage`，不出现「开着但没声」。
- 设置页「外观」新增开关 + 音量滑块；实测落盘 `{"enabled":true,"volume":0.55}`。

### 17.5 桌宠「玛格丽特公主」

`petSpec.ts` 新增 `princess`（2.5D/3D 共用一份数据）：银紫发 + 淡紫发梢、象牙裙 + 海蓝宝石外纱、精灵尖耳、雏菊花冠、蝶翼、捧花；Rust 侧 `PetSkinId::Princess` + `SKIN_CATALOG` 六款 + `pet_prefs.json` 落盘。

**本轮由真机截图驱动修掉的 3D 缺陷**（全部在 `buildPet3D.ts`，逐条留了原因注释）：

| 症状（截图可见） | 根因 | 修法 |
| --- | --- | --- |
| 正面像「戴泳帽 / 一颗光头」 | 后发球只后偏 0.12·HR 且 x/y 半径 ≥ 头球 → 整颗头被发罩包住，正面只露小块脸 | 后发后移 0.40·HR（前表面 0.60·HR，让位给脸）+ **新增头顶发盖半球**（1.08·HR）定义发际线 |
| 侧视图头顶一块白色「帽檐」薄片 | 后发与头球在前上方只差 0.04·HR，形成露出的月牙 | 同上（后移后该月牙消失） |
| 颅顶一圈锯齿「蛋壳边」 | 发盖 1.03·HR 与头球 1.02·HR 近共面 → z-fighting | 发盖加厚到 1.08·HR，全周至少 0.06·HR 间隙 |
| 额头一排碎白片 | 5 片刘海球与发盖是两套同色曲面，交叠成锯齿交接 | **移除独立刘海**，发际线交给发盖前缘 + 花冠带 |
| 花冠像一顶帽子 / 侧视只剩一条绿线 | 环半径 0.68·HR 落在头球内部（该高度头围 0.83·HR） | 环降到 0.52·HR、椭圆 x=0.90·HR / z=1.00·HR，花头按径向朝外（`rotation.y = a + π/2`） |
| 花瓣与白脸糊在一起 | 白色花瓣落在象牙肤色上 | 花冠下沉到**刘海带**之上，以淡紫发为底 |
| 侧视眼球凸出轮廓（穿模）、腮红悬浮 | 眼部四层推到 1.002·HR，而脸面只有 0.92·HR | 头球 z 半径改成 0.99·HR（脸做圆） |
| 眉毛完全看不见 | 眉毛 z=0.78·HR 埋在头球内 | z 提到 0.93·HR（贴脸） |
| 鬓发像两根深色「铁管」 | 用发梢色 + 长 0.86·HR | 与主发同色、缩短到 0.70·HR、后压 |
| 精灵尖耳消失 | 发盖加厚后耳朵（x=0.92·HR）整根埋进发里 | 外移到 1.12·HR、降到耳高 |

### 17.6 验收结果（真实运行，非静态检查）

| 用例 | 结果 | 证据 |
| --- | --- | --- |
| NIGHT-01 主题生效 | PASS | `documentElement.dataset.theme = marguerite-night`；`body` 背景 `rgb(7,11,26)`、文本 `rgb(238,242,255)`；`--radius-card 20px`；`--font-display "Fraunces", "Noto Serif SC", …` |
| NIGHT-02 星空运行 | PASS | `data-sky = running`、`canvas.marguerite-sky` 存在（`qa-night-01-home.png`） |
| NIGHT-03 设置页外观 | PASS | `/settings/appearance` 打开正常，环境音开关存在并可切换（`qa-night-03-settings.png`） |
| NIGHT-04 环境音持久化 | PASS | 开启后 `localStorage.marguerite.ambience = {"enabled":true,"volume":0.35}`；音量拉到 55 → `0.55`（开关未被回滚，说明 AudioContext 真实创建成功） |
| PET-11 公主皮肤 | PASS | 皮肤卡「玛格丽特公主 · 星夜」可选；`data-pet3d-kind = humanoid`；`pet_prefs.json` 落盘 `"skin":"princess"`（Rust 侧接受该枚举，证明确实是新 exe） |
| PET-12 三视角真几何 | PASS | azimuth `0.000 / -1.571 / 3.142`；三张截图字节 122852 / 121717 / 112675 互不相同；正/侧/背均可见发、翼、裙、捧花 |
| PET-13 五官无穿模 | PASS（产品尺寸） | 2x 特写 `qa-night-head-front.png` / `qa-night-head-side.png` / `qa-night-crown.png`：发盖、花冠、眉毛、腮红、双耳均贴合，无 z-fighting、无锯齿接缝 |
| 页面错误 | PASS | 四条路由（首页/对话/设置/桌宠）`pageerror` 与 console error 均为 0 |

### 17.7 本轮门禁

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| 主题契约 | `node scripts/check-theme-contract.mjs` | 通过（**11** 个 value theme × 29 必需角色） |
| 颜色纪律 | `node scripts/check-webui-colors.mjs` | 通过（11 个主题对比度全部达 WCAG 门槛；`MargueriteSky` / `world.css` 零颜色字面量） |
| 动效/圆角/主题作用域/跨面 | `check-webui-motion` · `check-webui-radius` · `check-theme-scope` · `check-cross-surface-tokens` | 全部通过（world.css 选择器统一 `[data-theme="marguerite-night"]`，动效走 `--dur-*` / `--ease-*`） |
| 类型检查 | `npx vue-tsc --noEmit` | 0 错误 |
| i18n | `node scripts/check-i18n.mjs` | 3756 直接键 0 缺失；6 语种全量对齐（新增 3 条环境音文案） |
| RPC 契约 | `node scripts/check-rpc-contract.mjs` | 389 声明全有实现，幽灵声明 0 |
| WebUI 单测 | `npm run test:unit` | **411 文件 / 4166 用例**（新增 `wallpaper.test.ts` 等） |
| WebUI 构建 | `npm run build` | 314 文件产物；主题包守卫：1 skin + **6 world** 全部懒加载（入口 0 字节） |
| Rust 单测 | `cargo test -p marguerite-core --lib`（含 `gateway::pet::tests` 9 项） | 3586 passed / 3 failed / 19 ignored —— 3 项失败全部与本轮无关，根因见下方说明 |
| Rust 静态检查 | `cargo clippy -p marguerite-core -p marguerite-gui --all-targets -- -D warnings` | **37 处告警**（全部在没有本轮改动过的文件里：`goal.rs` / `selfevolve.rs` / `orchestrate.rs` / `llm_profile.rs` / `sessions.rs` / `usage/mod.rs` / `tui/workbench/state.rs` / `loopback_tests.rs` / `methods.rs`），本轮改动的 `gateway/pet.rs` 零告警 |
| 部署对账 | 双击启动 → `http://localhost:18790/control/` | HTTP 200；入口 chunk 两处（`<install>/webui/dist` 与 `resources/pyengine/.../static/dist`）同名同哈希 |

> **Rust 全量门禁的如实记录（2026-10-24 05:0x，含并发写入说明）**
>
> `cargo test -p marguerite-core --lib` = **3586 passed / 3 failed / 19 ignored**。三项失败均**不在本轮改动范围内**，根因已定位（未擅自修改他人正在编辑的文件）：
> 1. `gateway::market_remote::tests::entry_must_stay_relative_and_inside` —— 新模块 `core/src/gateway/market_remote.rs`（05:00 落盘，非本轮文件）。Windows 上 `Path::new("/usr/bin/sh").is_absolute()` 为 `false`（无盘符），故 `entry_is_contained` 需要同时判 `has_root()`。
> 2. `pyengine::sidecar::stability_hardening_tests::restarts_outside_the_window_do_not_trip` —— 测试构造 `Instant::now() - 20min`，而本机开机仅 12 min ⇒ `overflow when subtracting duration from instant`；开机满 20 分钟后自愈，属**环境相关**测试缺陷。
> 3. `tools::terminal::tests::spawn_write_read_kill_roundtrip` —— 单独重跑通过（控制台代码页相关的时序抖动）。
>
> 本轮真正改动的 `gateway::pet::tests` **9 项全绿**；`npx vitest run src/components/pet` **8 文件 / 168 用例全绿**（首轮 1 项失败是并发的 cargo 编译抢占 CPU 导致 worker 超时，空闲重跑即通过）。
>
> 另：作业期间本机于 **04:58 重启**（`uptime` 12 min），部署产物仍在盘上；重启后重新启动 exe 并**复跑同一套 Playwright 探针**，`data-theme=marguerite-night` / `data-sky=running` / 公主皮肤 / 环境音落盘四项与重启前完全一致。

### 17.8 已知限制（如实记录）

1. 3D 形象是**程序化 Q 版**：五官与服饰由基础几何拼装，细节量低于美术资产；`buildPet3D` 是唯一入口，后续可平滑替换为 GLB。
2. Canvas 星空与合成环境音**只在 `marguerite-night` 下启用**；切换主题时组件卸载、音频随开关状态保留。
3. 环境音是**合成音**（噪声 + 正弦 + 三角波），不是采样音乐 —— 刻意规避第三方音源版权。
4. 本轮仍未重跑 NSIS/MSI 打包（部署走 exe + `webui/dist` 对账），安装包留待下次发版统一生成。
5. 侧视 90° 附近虹膜边缘会出现极细的深色「须状」轮廓（扁平虹膜球被掠射），产品显示尺寸下不可见。

## 18. 第五轮：液态玻璃 · 流光（Liquid Glass）与参数面板（2026-10-24 05:00–06:10）

### 18.1 需求与设计取向

深色基底 + 磨砂玻璃 + 大块柔和背景（云朵/海面/沙丘）+ 1–2 种主色 + 白字白图标 + 8K 极简质感；
明确禁止：杂乱、细碎纹理、多繁杂色彩、纯色背景、低画质、文字不可辨认。克制原则四条：多留白、黑白灰为主、
靠字号/字重/间距分层、真实场景与可交互感。另需「把所有 UI 元素的尺寸写成一个面板手动调并记住参数」。

### 18.2 交付内容

| 交付 | 文件 | 要点 |
| --- | --- | --- |
| 新 value theme | `webui/src/themes/liquid-glass/{manifest.ts,tokens.css,world.css}` | 29 个必需 L1 角色 + 11 个 `--glass-*` 参数令牌；world 层只在激活时懒加载 |
| 背景质感层 | `webui/src/components/GlassBackdrop.vue` | 5 个超大径向色块（blur 72–96px、opacity 0.5–0.85）+ 双层托底渐变 + 暗角；漂移 96–142s `infinite`；`prefers-reduced-motion` 停止漂移 |
| 场景切换 | `webui/src/composables/useGlassScene.ts` | 云朵/海面/沙丘三套；`localStorage.marguerite.glass-scene` 持久化 |
| 参数面板 | `webui/src/components/settings/GlassLab.vue` + `composables/useGlassLab.ts` | 11 项滑块（圆角/模糊/饱和/白玻璃/暗纱/亮边/折射/留白/间距/抬升/文字底板）；改动即时写 `<html>` 内联变量并持久化 `marguerite.glass-lab`；可导出带 `cssVars` 的 JSON 回写 tokens.css |
| 入口接线 | `App.vue` / `SettingsAppearancePanel.vue` | `glassActive` 挂载背景层；外观页在该主题选中时显示参数面板 |
| 移动端修复（本轮发现） | `webui/src/assets/base.css` | 顶栏 4 项挤 3 列网格溢出压住标题；输入区工具条固有宽 785px 把发送键挤出屏幕（390px 视口）——两处均改为可换行并右对齐 |

### 18.3 关键实测（不是"看起来对"，是量出来的）

| 观测项 | 实测值 |
| --- | --- |
| `data-theme` | `liquid-glass` |
| 内容区 `backdrop-filter` | `blur(30px) saturate(1.65)` |
| 白玻璃叠层 `--glass-tint` | `10%`（设计基准） |
| 暗纱 `--glass-scrim` / 亮边 `--glass-edge` | `66%` / `36%` |
| 胶卷颗粒 `--grain-opacity` | `0`（关掉细碎噪点） |
| 单页 `backdrop-filter` 元素数 | **5**（内容区/侧栏/对话顶栏/设置弹窗/设置遮罩），卡片不叠模糊——无"玻璃叠玻璃" |
| 最坏情况对比度 | 正文 **9.51:1**、次要文字 **5.69:1**（按最亮背景块 + 暗纱 + 白玻璃解析式反解，AA 4.5 通过；`--text-dim` 不用于玻璃正文） |
| 背景层是否可见 | 修复 `.main/.content/.chat/.chat-body` 不透明底色后可见；修复前为"画了但看不见"的死黑（本文件 §18.4） |
| 参数面板 | 拖动 `tint 10→14`、`radius 22→30` 后 `<html>` 内联变量与 localStorage 同步，点「恢复默认」回到 `10%`；导出 JSON 含 `theme/scene/params/cssVars` |
| 场景切换 | `clouds → sea → dunes` 均可切换并落盘（`sceneStored: dunes`） |
| 页面错误 | 首页/对话/设置/桌宠/用量五条路由 `pageerror` 与 console error 均为 0 |
| 移动端 390×844 | 顶栏两行不再压标题；输入区三行自适应，发送键与全部 chip 可见（`send.x=29`，页面无横向溢出 `scrollWidth=390`） |

### 18.4 本轮踩到并修掉的坑（都留了注释与证据）

1. **`.main { background: var(--sidebar-bg) }` 把背景层整块盖住** → 玻璃"有模糊但没有东西可模糊"，只有死黑；
   修法：主题内把 `.main/.chat/.chat-body` 让成透明（`!important` 是实测必需，底色在多处被 `background` 简写重置回来）。
2. **上下文覆盖压过主题规则**：`.content .usage-stage .control-stat`（0,3,0）等规则让统计卡保持不透明黑块；
   卡片选择器必须带 `#app` 提升到 (1,2,0)。
3. **`.particle-field` 细碎粒子**：组件 scoped 规则与本主题规则同为 (0,2,0) 且后注入 → 需 `.chat .particle-field` 才生效；
   该类细碎纹理与极简质感冲突，本主题直接关闭。
4. **只有白玻璃没有暗纱时白字被亮背景吞掉** → 引入 `--glass-scrim`（白玻璃 10% + 暗纱 66% 双层）。
5. **移动端两处既有缺陷**（`dark` 主题同样复现，非本主题引入）：顶栏 4 项溢出、工具条 785px 挤掉发送键。

### 18.5 门禁

| 门禁 | 结果 |
| --- | --- |
| 主题契约 `check-theme-contract` | 通过（**12** 个 value theme × 29 角色） |
| 颜色纪律 `check-webui-colors` | 通过（12 主题对比度；新主题零颜色字面量） |
| 动效 / 圆角 / 作用域 / 跨面 | 全部通过（world.css 选择器统一 `[data-theme="liquid-glass"]`；动效走令牌） |
| i18n | 3774 直接键 0 缺失；6 语种对齐（新增 19 条 `glassLab.*`） |
| WebUI 单测 | **415 文件 / 4204 用例全绿**（新增 `useGlassLab.test.ts` 5 例） |
| WebUI 构建 | 315 文件；主题包守卫：1 skin + **7 world** 全懒加载（入口 0 字节） |
| 部署 | 新 `dist` 同步到 `<install>/webui/dist` 与 `resources/pyengine/.../gateway/static/dist`，入口 chunk 同名同哈希 |

### 18.6 已知限制（如实记录）

1. 参数面板的「文字底板 `--glass-text-veil`」只影响聊天气泡；其他长文本容器仍按各自组件样式。
2. `--glass-radius` 与 `--radius-card` 目前数值相同但未绑定（卡片走前者，其他面板走后者，两者独立可调）。
3. 背景层是纯 CSS 渐变（无位图、无 Canvas），因此在极低端 GPU 上仍有 5 个大面积 `filter: blur` 合成层；
   已在移动端与 `prefers-reduced-motion` 下静止，但未做"移动端降 blur"的自适应（保留为后续项）。
4. 本轮未重跑 NSIS/MSI 打包；Rust 侧无改动（`cargo test` / `clippy` 结论见 §17.7 的并发写入说明）。

## 19. 第六轮：「玛格丽特 · 星夜」升级为 Three.js 三维山脊（2026-10-24 06:40–08:20）

背景不再是 2D Canvas 星空，而是一块**实时三维场景**：前景雪脊 + 铜皮补给站 + 结组登山队 +
雏菊 + 云海 + 金字塔主峰 + **左半星空 / 右半晨曦**的一轮日出。全部程序化生成，零外部素材。

### 19.1 要素 → 实现对照（用户点名的每一项）

| 要素 | 实现 | 位置 |
| --- | --- | --- |
| 雪山 | `ridgeHeight(x,z)` 高度场：肩脊（z≈+8）+ 营地鞍部（z≈-10）+ 金字塔主峰（z≈-44）+ 右次峰，两处山坳压到云海以下；180×132 网格（≈4.75 万三角面），顶点色按坡度分「积雪 / 裸岩 / 冰川蓝」并叠一层确定性细噪声 | `ridgeMath.ts` / `ridgeScene.ts#buildRidge` |
| 补给站（玛格丽特山顶小屋） | 20.5×8×7 m 铜皮长条 + 浅坡双坡雪顶 + 屋脊压条 + 烟囱 + 长边木露台 + 栏杆 + **9 颗珍珠灯** + 4 扇暖光窗 + 气象杆与风杯；落位偏左三分线 | `#buildHut` |
| 一半星空 / 一半云海 / 翻山越出日出 | 天空是 `ShaderMaterial` 球壳：按**方位角** `smoothstep(-0.75, 0.95, az)` 把天穹切成「夜 / 晨」两半（不是按时间闪），夜侧叠银河带与哈希星点，晨侧叠地平线暖带；太阳高度角 -6°→+10°→-6° 240 s 一循环；柔边日轮贴图 + `toneMapped:false` | `SKY_FRAG` / `#buildSky` |
| 云海 | 两层反向漂移的 520×340 加色云面（程序化 Canvas 柔斑贴图）+ 8 团远景云 | `#buildCloudSea` |
| 登山客队伍 | 5 人结组：腿/躯干/手臂/头/头盔/背包/冰镐/头灯八件；沿 `ridgeHeight` 贴地横切，间距 0.08·路径，绳索每帧重算；头灯是画面唯一高光点 | `#buildTeam` + `ropePathPoint` |
| 雏菊 | 前景 7 株：15 枚白色舌状花 + 金黄管状花心的 64² 贴图，带茎；**随 dawn 张开**（夜里是合拢的花苞，符合 Bellis perennis 的日开夜合）；补给站旁另有 26 枚闭合花苞与 6 朵窗光下的花 | `#buildDaisies` / `#makeDaisyTexture` |
| 圆润化 / 留白 / 排版 | 沿用 §17.2 的 value theme 令牌：`--radius-*`、衬线展示字、`line-height 1.78`；本轮只改背景与其外壳透明度 | `world.css` §7 |
| 音效 | 沿用 §17.4 的 Web Audio 合成环境音（山风 + 五度 pad + 五声铃音），不引入任何采样 | `useAmbience.ts` |
| 桌宠 | 沿用 §17.5 的「玛格丽特公主」程序化 Q 版（银紫发 + 雏菊花冠 + 海蓝宝石外纱） | `petSpec.ts` / `buildPet3D.ts` |

### 19.2 真实依据（只取语汇，不取图像）

- 补给站：4554 m、Punta Gnifetti（Signalkuppe）峰顶、1980 年重建的现屋整体铜皮包覆、长边木露台、
  屋顶常年积雪、2000 年起为欧洲最高气象站 → 尺寸 20.5×8×7 m 直接进几何；屋顶取**浅坡**（陡坡存不住雪）。
- 「Margherita」词源为希腊语 *margarites*（珍珠），王后官方肖像常佩多串珍珠；1893 年大婚 15 周年冠冕为
  **11 道弧环 + 541 颗钻石 + 11 颗大珍珠垂** → 露台栏杆上 9 颗暖白珍珠灯、冠冕弧线 → 桌宠花冠环。
- 雏菊（Bellis perennis）：15–30 枚白色舌状花 + 75–125 枚黄色管状花，**日出张开、日落后闭合** → 贴图取 15 枚花瓣，开合随 dawn。
- 构图参考（只观察，不复制）：补给站悬在云海之上、金字塔峰顶的孤立建筑、日出侧的地平线暖带。
  参考来源：[Alamy — 补给站与云海](https://www.alamy.com/clothes-line-and-pegs-in-front-of-the-margherita-hut-on-the-summit-of-the-4554-meter-high-signalkuppe-or-punta-gnifetti-highest-hut-in-the-alps-belo-image551140904.html) ·
  [Adobe Stock — 航拍补给站](https://stock.adobe.com/nz/images/flying-view-of-the-capanna-regina-margherita-on-the-punta-gnifetti-mountain-peak-in-the-alps-of-switzerland-above-zermatt/1571153481) ·
  [Turin Epicurean — 王后的珍珠](https://www.turinepi.com/2026/03/the-queens-pearls.html?m=1) ·
  [Turin Guide — Margherita di Savoia](https://www.turinguide.eu/margherita-di-savoia-la-prima-regina-ditalia/?lang=fr)

**合规**：仓库与产物中**没有任何**实景照片、王室肖像或第三方生成图；山形、云、星、人、花全部由
`ridgeHeight` / GLSL / Canvas 2D 现场计算。

### 19.3 实现要点

| 文件 | 作用 |
| --- | --- |
| `webui/src/components/marguerite/ridgeMath.ts` | 纯函数：高度场、坡度、补给站落位、结组路径、太阳高度角/晨曦强度/太阳方向（10 条单测） |
| `webui/src/components/marguerite/ridgeScene.ts` | 场景装配（天空/灯光/地形/云海/补给站/登山队/雏菊）+ 帧循环 + 诊断属性 |
| `webui/src/components/marguerite/MargueriteRidge.vue` | 挂载容器（`position:fixed; z-index:-1; pointer-events:none; contain:strict`） |
| `webui/src/App.vue` | `skyActive && webglOk` 时挂 3D 场景，否则退回 Canvas 星空；WebGL 探测用原生 canvas，不额外加载 three |
| `webui/src/themes/marguerite-night/world.css` §7 | `html[data-ridge="1"]` 下的外壳：中间层透明、CSS 雪脊退场、极光降到 35%、面板改压暗玻璃 |

- **帧率无关**：位移类动效走夹紧后的 `dt`（≤50 ms），天光走**真实秒**（低帧率机器上太阳不会"慢放"）。
- **调试/取证钩子**：`?ridge-t=<秒>` 可把天光时钟直接拨到指定相位（0 夜 / 58 跃出 / 120 高日），
  避免在低帧率取证机上干等 4 分钟。
- **降级**：`prefers-reduced-motion` 只渲一帧；无 WebGL 时整块 3D 不挂载，退回 §17.3 的 Canvas 星空。

### 19.4 验收（真实运行，Playwright + SwiftShader 软件渲染）

| 用例 | 结果 | 证据（`docs/qa-shots/`） |
| --- | --- | --- |
| RIDGE-01 场景挂载 | PASS | `html[data-ridge="1"]`、`.marguerite-ridge` 的 `data-marguerite-ridge="running"`、canvas 1440×950、宿主 `z-index:-1` |
| RIDGE-02 三层相位 | PASS | 拨表后 `dawn / 太阳高度角` = `0.009 / -5.93°`（夜）、`1.000 / +2.78°`（跃出）、`0.556 / +9.91°`（高日）；三张截图 `qa-ridge-night/sunrise/morning.png` 画面明显不同 |
| RIDGE-03 场景本体 | PASS | `qa-scene-only-*.png`（把 UI 全隐藏后只拍 3D）：主峰、补给站与窗光、珍珠灯串、结组 5 人（含绳索与头灯）、云海、雏菊（含茎与 15 瓣花冠）均可辨认 |
| RIDGE-04 面板不压死背景 | PASS | `.content` 实测 `backdrop-filter: blur(6px) saturate(1.15)`、底色为 46%–70% 夜色渐变；山脊与补给站在应用界面里可见 |
| RIDGE-05 无回归 | PASS | 三条路由 `pageerror` = 0、console error = 0；`canvas.marguerite-sky` 不再出现（3D 已接管背景） |
| RIDGE-06 设置页 | PASS | `/settings/appearance` 主题卡「玛格丽特 · 星夜」选中态正常；模态面板遮挡处文字对比正常 |

### 19.5 本轮踩到并修掉的坑（每条都留了注释）

| 症状（截图可见） | 根因 | 修法 |
| --- | --- | --- |
| 登山队**整队消失** | 地形顶点用了 `z - 12` 做整体后移，而补给站/登山队/雏菊仍按 `ridgeHeight(x, z)` 落位 → 差出 12 m，人埋进雪里 | 去掉整体偏移，地形与所有落位共用同一坐标系 |
| 应用里看只是一片灰雾 | 面板 `backdrop-filter: blur(22px)` + 84% 暗纱把 3D 画面糊死 | `.content` 降到 `blur(6px)` + 46%–70% 渐变；侧栏/顶栏/输入区各自保留更实的底（74/70/76%） |
| 太阳是一枚硬边"纸圆片"，还发灰 | `MeshBasicMaterial` 受 ACES 色调映射压暗；`CircleGeometry` 边缘是硬切 | 换柔边径向贴图 + `toneMapped:false` + 提高日轮本体亮度（否则中心比光晕暗，像个孔） |
| 夜里补给站迎光面是一块红板 | 那盏"太阳"平行光在夜里仍是暖红且强度不低 | 太阳未升起时这盏灯按**月光**着色（冷），强度 `0.18 + dawn·1.7` |
| 前景一大片灰板，像贴了张塑料膜 | 云团 Sprite 放在 z≈-12…+12、尺寸 30–46，直接糊在镜头前 | 云团退到 z≈-72…-120、尺寸 24–34、不透明度 0.14–0.24 |
| 补给站被画面左缘切掉一半 | 相机回撤后 x=-30 超出取景 | 补给站移到 x=-21（左三分线），相机 (0,19,56) / lookAt (0,12,-34) / fov 40 |
| 开局 5 个人叠在视口外 | `teamPhase` 从 0 起，5 人全在 t≤0 被 clamp 到同一端点，且该点在 36° 偏角外 | 起始相位 0.45；间距 0.08（≈3.6 m/人） |
| 人像"青色软糖" | 只有躯干+头两个胶囊，纯黑又会和山体糊成一片 | 八件式人形（腿/躯干/双臂/头/头盔/背包/冰镐/头灯）+ 低饱和钢青外壳 + 1.15 缩放 |

### 19.6 门禁

| 门禁 | 结果 |
| --- | --- |
| 主题契约 / 颜色 / 动效 / 圆角 / 作用域 / 跨面 | 全部通过（12 个 value theme × 29 角色；world.css 选择器统一 `[data-theme="marguerite-night"]`） |
| `ridgeMath` 单测 | 10/10 通过（高度场有限、三层结构次序、山坳低于云海、坡度单调、补给站不悬空、结组路径贴地、日照周期、晨曦峰值、太阳单位向量） |
| WebUI 单测 | **418 文件 / 4239 用例全绿** |
| Rust 单测 | `cargo test -p marguerite-core --lib` = **3678 passed / 0 failed / 22 ignored** —— §17.7 记录的三条红项（`market_remote` / `sidecar` 溢出 / `terminal` 时序）本轮全绿 |
| Rust 静态检查 | `cargo clippy -p marguerite-core --all-targets -- -D warnings` = **0 warning / 0 error**。构成：①`RpcError::details` 装箱后 `RpcError = 80 B`（原 144 B → 41 处 `result_large_err` 消失，JSON 线格式不变）；②`EscalationAck::Resolved` 首字段装箱；③其余 **16 处**逐条改写：`starts_with_chars`（显式计数器）、`blink_on`（`is_multiple_of`）、`blend_pixel`（索引循环改 `enumerate`）、`title`（连续 `replace` 合并）、`is_scene_base` ×2（`char` 闭包改字符切片）、`last_nonempty_line`（`next_back`）、4 处 `&[x.clone()]` 改 `std::slice::from_ref`、`sniff_ext`（同体分支合并）、3 处 `map_or(false, …)` 改 `is_some_and`；④`query_contract` 的 8 个参数收成 `UsageQueryOptions` 结构 |
| 类型检查 + 构建 | `vue-tsc` 0 错误；产物 321 文件；主题包守卫 1 skin + 7 world 全懒加载 |
| 部署对账 | 新 `dist` 同步到 `<install>/webui/dist` 与 `resources/pyengine/.../gateway/static/dist`，Playwright 直接命中新构建（`contentBackdrop` 由 22px 变 6px 即为指纹） |

### 19.7 已知限制（如实记录）

1. 地形是**顶点色**低多边形：近看没有岩层纹理与冰裂缝，远处靠雾气与坡度色过渡；未上法线贴图/位移贴图。
2. 登山队只在固定路径上循环行进，没有各自的步态动画（只有整体上下起伏与轻微转向）。
3. 云海是加法混合的平面 + Sprite，**没有体积光**；日出时云顶没有单独的体积散射。
4. 240 s 的日照循环里有约 40% 时间处于暖调（dawn > 0.5），夜间主题会长时间偏暖 —— 这是"翻山越出日出"的取舍，若要更"夜"，需缩短暖调窗口。
5. 3D 场景只在 `marguerite-night` + 有 WebGL 时启用；软件渲染（SwiftShader）下可跑但帧率低，实机有独显/核显时正常。
6. 本轮仍未重跑 NSIS/MSI 打包；`webui/dist` 走对账部署。

### 19.8 智能体画像同步（UI 设计团队）

`core/src/agent/personas/ui-design-engineer.md` 从「专家」扩成「团队负责人」，内置智能体记录同步改名为
**「UI 设计团队 · 液态玻璃」**（id 仍是 `ui-designer`，不破坏既有会话与配置）。

| 新增/变更 | 内容 |
| --- | --- |
| §一 团队分工 | 设计系统 / 视觉 / 动效 / 空间 3D / 验收 五个角色，每个角色只回答一个问题、交一份东西（令牌表 / 构图与字号阶梯 / 时长与降级 / 坐标系与光源身份 / 对比度与截图） |
| §三 材质参数 | 玻璃参数区间之外，新增「三维背景」档位：内容区暗纱 46–70% + blur 4–8px、侧栏 70–78%、顶栏 66–72%、输入区 74–80%；单场景 ≤6 万三角面、≤2 张 64² 程序贴图 |
| §五 动态背景八条 | 背景是内容不是壁纸 / 坐标系唯一 / 光要有物理身份 / 自发光要 `toneMapped:false` / 位移走 `dt`·天象走真实秒 / 深色场景不能只靠剪影 / 留 `data-*` 与拨表钩子 / 三条降级 |
| §六 禁止清单 | 追加两条：把动态背景压到看不见、用受版权照片或第三方生成图充当主题美术 |
| 记录字段 | `skills` 增加「三维背景」，`description` 改成五角色表述 |

主智能体侧（`personas/senior-dev-engineer.md` §6）仍是"指针"：界面/主题改动一律转交该画像，避免两份方法论漂移；
本轮同时给它补了一条「三维/动态背景」要点（面板 blur 上限、坐标系唯一、夜里那盏灯是月光）。

> 生效链提醒：人设由 `include_str!` **编译期**内嵌，改 md 后必须重新构建 `marguerite-gui` 才会在「智能体」页看到新名字与新内容；
> 当前 <install> 下的 exe 仍内嵌旧文案（源码已改、产物未重建），属已知的"二进制滞后"，不是改动丢失。

## 20. 第七轮：输入框 / 选择器 / 工作区树 / 运行可见性 + 液态玻璃二次打磨（2026-10-24 07:00–08:40）

### 20.1 用户报告 → 根因 → 修法

| # | 用户看到的现象 | 根因（代码级） | 修法 |
| --- | --- | --- | --- |
| 1 | 输入框「运行模式与路由」内容缺失 | `ChatComposerRunMode` / `ChatComposerModelRouting` 自身是 `position: absolute; bottom: calc(100% + 8px)`，却被塞进 `RunModePanel` 的**滚动盒**里：它们相对最外层 `.run-mode-panel` 再弹一层，脱离父面板的 `max-height`，最后一段被输入区吃掉 | 两个子组件新增 `inline` 变体（`position: static`、无边框/底色/阴影、隐藏自带标题），`RunModePanel` 以 `inline` 引入；面板宽度 360→400、上限 560→680，三段一屏可读（实测 scrollHeight = clientHeight = 661） |
| 2 | 「模式」与「模式与能力」内容重复 | `ModeMatrix` 与 `ChatModeCapabilityPanel` 渲染同一份 `USER_MODE_META` 3×3，且都能写 `mode.switch`（两处可写副本） | 能力面板删掉矩阵，只保留「能力勾选 + 一行只读当前模式」；标题与按钮改为「能力组合」（`chat.orchestration.capabilitiesTitle`）；档位只在「模式」里改 |
| 3 | 「推理等级」滑轨不像 Codex / Claude Code | 轨道 6px、四段流动渐变（青→蓝→橙→金）、极限档光晕 + 上升火星 + 变色拇指 —— 装饰过多 | 改细轨（4px）+ 三枚点刻度 + 单一主色填充 + 干净圆钮；删掉 halo/sparks 两段 keyframes；刻度改成 4 个等宽胶囊，选中态用底色而不是颜色狂欢；拖动时出现「松手落到哪档」气泡；浮层从 `left:0` 改 `right:0`（原先 300px 面板顶到窗口右缘、被右侧栏压住一半） |
| 4 | 「选择模型」没有滚动条、模型溢出框体 | 面板 `display:grid` 只有 `max-height`，没有行模板也没有 `overflow:hidden`；滚动子项没有 `min-height:0` ⇒ 子项按内容撑高，`max-height` 只裁容器 | 面板改 `flex column` + `overflow:hidden`；滚动区 `flex:1 1 auto; min-height:0; overflow-y:auto` + 可见滚动条（thumb 用 `text-muted` 42%，`border-strong` 在深色面板上几乎看不见） |
| 5 | 侧边栏的树不是工作目录 | 前端只发 `{ path: '' }`，后端 `sandbox_path_root()` 自己实现「USERPROFILE → HOME → cwd」，于是列的是 `C:\Users\<user>`（AppData/Cookies/Desktop…） | 前端按 `activeWorkspace.path` → `workspace.status.root` → 空串 三级解析，作为 `sandbox.path.list` 的 `basePath`；后端 `sandbox_path_root()` 改为委托 `Workspace::resolve(None)`（唯一解析器：绑定 → 数据目录 → cwd），界面与工具用同一个根 |
| 6 | 看不见模型在跑什么（CLI / 读写文件 / 搜索 / 思考 / 总结） | 右侧「任务」只有一行标题（`调用工具：xxx`），思考只有「深度思考」+100 字；`thinkingContent` 甚至没有对外导出 | `useChatStream` 导出 `thinkingContent`；`useChatSidebarPanels` 重建任务轴：思考正文 / 工具名 + 入参 + 出参 / 时间线 / 本轮 assistant 正文总结；右侧栏改为可展开行（运行中与报错默认展开），`pre` 内滚动，单条上限 4000 字 |
| 7 | 液态玻璃"看不见背景、不够玻璃、但字要看得清" | ① 面板暗纱 66% + 22px 以上的模糊把背景压成灰雾；② 背景层色块中心常落在视口外，屏幕上只剩渐变尾巴；③ **`useGlassLab` 的 `base` 仍是旧值（66%/30px），启动时以 `<html>` 内联变量覆盖了 tokens.css** —— 改 CSS 完全没效果 | 暗纱 52%、模糊 34px、饱和 185%、白叠层 12%、亮边 42%、折射 22%；`backdrop-filter` 追加 `brightness(90%)`（用"压亮度"而不是"加厚黑纱"来保对比度）；背景补三处**固定落在屏内**的光池并提高色块不透明度；`GLASS_PARAM_DEFS.base` 与 tokens 逐项对齐（52/34/185/12/42/22） |

### 20.2 实测（Playwright，1440×950，liquid-glass）

```json
{
  "runModePanel": { "sections": 3, "runModePosition": "static", "routingPosition": "static",
                    "routingOptions": 5, "scrollHeight": 661, "clientHeight": 661,
                    "routingBottomInside": true },
  "modelPicker":  { "rows": 12, "panelOverflows": false, "scrollAreaScrollable": true,
                    "maxHeight": "520px", "scrollbar": "thin" },
  "capabilityPanel": { "matrixCells": 0, "caps": 22, "allMatrixCellsInApp": 0 },
  "effort":       { "thumb": true, "oldHalo": false, "segments": 4, "trackHeight": "32px" },
  "tree":         { "root": "C:\\Users\\Administrator\\AppData\\Local\\Marguerite", "count": 81 },
  "theme":        { "data-theme": "liquid-glass", "backdrop": "clouds",
                    "contentBackdropFilter": "blur(34px) saturate(1.85) brightness(0.9)",
                    "glassScrim": "52%", "glassBlur": "34px" },
  "pageErrors": []
}
```

截图：`qa-ui-runmode.png`（三段全展开）、`qa-ui-modelpicker.png`（12 行可滚）、`qa-ui-capabilities.png`（无矩阵）、
`qa-ui-effort.png`（新滑轨）、`qa-ui-glass-chat.png`（玻璃后面能看出蓝→青的色场）。

### 20.3 本轮踩到的坑（都留了注释）

1. **参数面板的 `base` 会覆盖 tokens.css**：面板初始化把 11 个 `--glass-*` 以内联变量写到 `<html>`，内联优先级高于主题令牌 —— 只改 tokens 会出现"改了样式毫无变化"。两处必须同步（已写进 §19.8 的方法论）。
2. **浮层不要塞进滚动盒**：子面板自带 `position:absolute` 时，父面板的 `max-height/overflow` 管不住它。
3. **`display:grid` + 只有 `max-height` 不会滚动**：必须给滚动子项 `min-height:0`，或干脆改 flex 列。
4. **右侧栏是更高的 stacking 层**：底栏浮层向右展开会被它压住 —— 右侧的触发器一律 `right:0` 向左展开。

### 20.4 门禁

| 门禁 | 结果 |
| --- | --- |
| 主题 / 颜色 / 动效 / 圆角 / 作用域 / 跨面 | 全部通过 |
| i18n | 6 语种全对齐（新增 5 个 `chat.rightSidebar.run*` 键） |
| WebUI 单测 | **424 文件 / 4285 用例全绿**（含本轮新增的 4 个测试文件，见 §20.6；并改写了 1 条已失效的「能力面板点选 modeKey」用例，保留"不重复渲染矩阵"的守卫） |
| 类型检查 + 构建 | `vue-tsc` 0 错误；产物 321 文件；主题包守卫 1 skin + 7 world 懒加载 |
| Rust | `clippy --all-targets -D warnings` 通过（`sandbox_path_root` 改为委托 `Workspace::resolve`） |
| 安装包 | MSI + NSIS 均产出，前端文件数源码=322 / 包内=322（一致性校验通过），见 §20.7 |

### 20.5 已知限制（如实记录）

1. 未绑定项目时，树根是**应用数据目录**（智能体真正写入的地方），不是用户主目录；想直接看某个项目请先绑定工作区。
2. 「运行」流只覆盖**当前会话**的实时/历史工具调用；跨会话的完整审计仍在「会话检查」抽屉。
3. 运行流里单条入参/出参上限 4000 字（超出截断并标 `…`），完整内容请在产物/工具结果弹窗里看。
4. 液态玻璃在**很亮**的自定义壁纸下，正文对比度仍会下降；`--glass-brightness` 已压到 90%，更亮壁纸建议换回主题自带背景。
5. Rust 侧 `sandbox_path_root` 的改动要重新构建 exe 才生效；本次前端已用 `workspace.status` 先行兜底，无需等 exe。

### 20.6 本轮新增的单元测试（4 个文件 / 27 条）

| 文件 | 锁住的契约 |
| --- | --- |
| `components/chat/RightSidebar.test.ts` | ① 有项目绑定时 `sandbox.path.list` 必须带 `basePath=<项目路径>`，标题显示工作区名；② 无绑定时先问 `workspace.status` 再用它的 `root`；③ 运行流：思考/总结按 kind 本地化标题、running/error 默认展开、点击可收放、无正文的行不可展开 |
| `components/chat/chatSurfaces.contract.test.ts` | 源码级布局不变量（happy-dom 不做布局）：模型面板 `flex + overflow:hidden`、滚动区 `min-height:0`；运行模式 popover 有 `max-height` 且内联子面板 `position:static`；推理浮层 `right:0` 且不再有 halo/sparks；能力面板模板里没有矩阵 |
| `composables/chat/useChatSidebarPanels.buildSidebarTasks.test.ts` | 运行流投影：顺序（思考→实时工具→历史→总结）、kind/入参出参、错误态、流式期间不出「总结」、4000 字截断、忽略非 assistant 消息 |
| `composables/useGlassLab.test.ts`（扩充 1 条） | `GLASS_PARAM_DEFS.base` 必须与 `tokens.css` 的 `--glass-*` **逐项相等** —— 就是这次"改 CSS 没效果"的根因守卫 |

另在 `RunModePanel.test.ts` 补 1 条：两个子面板必须以内联形态出现在 popover 的同一个滚动盒里，且不再渲染自己的标题。

### 20.7 安装包（2026-10-24 11:05 构建）

| 产物 | 大小 | SHA-256 |
| --- | --- | --- |
| `D:\marg_build_target\release\bundle\msi\Marguerite_0.2.16_x64_en-US.msi` | 416.59 MB | `72444A62A1115BC11FABF2624D3CBD6A2CB9DBB468F1145623A19028CC7F2852` |
| `D:\marg_build_target\release\bundle\nsis\Marguerite_0.2.16_x64-setup.exe` | 381.09 MB | `0B19E08D4981A163AE97474E6D1C0F1B16B7E472C1A0C73B1EB8E16BD6055CBD` |

- 构建命令：`scripts\package_app.cmd`（隔离 target=`D:\marg_build_target`，WebView2 offlineInstaller 静默内置）
- `cargo tauri build` release 编译 6m30s；打包层前端与源码产物 **322 = 322** 逐文件一致（脚本自带一致性校验）
- 构建期唯一告警：`STATIC_VCRUNTIME is deprecated`（Tauri 上游提示，历史遗留，不影响产物）
- `webui/dist` 已同步到 `<install>/webui/dist` 与 `resources/pyengine/.../gateway/static/dist`，可直接重启应用查看，无需先装包

## 21. 第八轮：模式 / 真实计量 / 磨砂 / 水印 / 项目绑定 / 调度简报 / 桌宠（2026-10-24 13:00–15:20）

### 21.1 用户报告 → 根因（代码级）→ 修法

| # | 用户看到的现象 | 根因 | 修法 |
| --- | --- | --- | --- |
| 1 | 模式点了没反应 | `mode.get` 回**对象** `{key,label,…}`，`mode.switch` 回**字符串** `"standard_doit"`；前端直赋给 `userMode` ⇒ 胶囊退化成默认、`dispatch`/发送门禁读到旧档（蜂群被判「档位不允许」） | `stores/activeMode.ts` 新增纯函数 `userModeFromWire` / `normalizeModePayload`，`useActiveMode` 的 `refresh` / `switchUserMode` / `switchMode` 三处写入全部归一；缺 `userMode` 时用 `userModeKeyForKind(kind)` 反推格位 |
| 2 | 咨询/方案列「选了又弹回」 | 后端 `from_user_mode` 对咨询/方案两列**不区分强度**，极简/极致会折叠到标准格位 —— 沉默弹回等于「没反应」 | `ModeMatrix.pick()` 比对「点了哪个 / 实际按哪个」，不一致时弹一次 info toast（`mode.intensityCollapsed`，6 语种） |
| 3 | 底部 token / 命中率是假的 | `usage.status` 按 `usage_events.session` 聚合，实测 10434 行里 **6982 行 `session=''`** ⇒ 任何会话恒为 0；`cache.status` 也是形状对但值恒 0。真实用量只在 `session.event.usage` 帧（snake_case `prompt_tokens` / `completion_tokens` / `cached_tokens`） | `useChatRpcEventHandlers` 累加该帧（payload 无 session 字段 → 用 `stream.isStreaming` 做归属判据）；`ChatView` 的 `sessionUsage` 改读累计值；**没有数据整条不渲染**，`cached=0` 时命中率显示 null 而不是 0% |
| 4 | 玛格丽特主题磨砂太高、背景看不见 | ① `.amb-scrim` 全屏雾层 58% + 9px；② 背景视频 `blur(2px)` + 34%；③ `world.css` 面板 62/52/56/70% + blur 6px；④ 输入框被 `#app .chat .chat-composer--floating .chat-input-panel`（id 权重 + 组件内注入更晚）压住，改了主题也不生效 | scrim → **30% / 2px**；视频去 blur、opacity 0.46；面板 → **52/42/46/60% + blur 3px**、侧栏/顶栏 18px→10px、输入框 76%/18px→60%/10px，并用同权重选择器覆盖；截图确认粒子雏菊与山脊雪线可见、白字仍可读 |
| 5 | 背景 MV 右下角「豆包 AI 生成」水印 | 素材自带水印，处于画面底部 120px 条带内 | 裁掉底部 120px（1280×600）替换 `webui/public/music/bg-seedance-25.mp4`（原件备份 `.tmp/bg-seedance-25.orig.mp4`）；逐帧抽样确认 30 s 无水印残留 |
| 6 | 主智能体回「本次会话未绑定到真实项目目录」 | 三层：① 前端只在 `intent==='new_chat'` 带 `workspaceId`，首轮一旦错过就永久为 null；② **Bloom 并行路径**用 `new_with_credentials_and_cache` 构造编排器 ⇒ lead 工作区恒为「未绑定默认根」，即使 `chat.send` 已带 `workspaceId`、台账已绑定；③ **计划路径**的子步骤编排器与完成门禁都硬编码 `default_workspace_root()` | ① `useChatSend.resolveSendWorkspaceId` 三级优先级，续聊每轮重发 `boundWorkspaceId`（自愈）；② Bloom 入口改为 `match resolve_session_project(&session)`，有绑定就用 `new_with_workspace_credentials_cache`；③ `plan::runner::run_autonomous` 的根改为 `context::session::session_project(&session)` → 回落默认根，子步骤与门禁共用同一根 |
| 7 | 主智能体不按输入框里选的东西分工 | 前端的选择（模式/智能体/形态/成员/团队/蜂群/能力）只影响本地 UI，从未上行；后端也没有「先出调度计划」的约束 | 新增 `ChatDispatchBrief` → `chat.send.dispatch` → Rust `dispatch_brief_hint()` 渲染系统提示段（要求 ≤8 行调度计划：目标 / 分工 / 顺序依赖 / 每步依据 / 验收标准），并注入**单轮 / 计划 / Bloom 三条路径**；无 dispatch 时零注入（旧客户端行为不变） |
| 8 | 桌宠不显示 / 显示不正常 | ① `localStorage` 标记缺省被当成「关闭」（只有写入 `'true'` 才显示）⇒ 新用户与老用户都看不到，设置页开关也长期显示关；② 尺寸只有 **100×110**，1440 宽屏上只有指甲盖大；③ 桌面悬浮窗在壳里有 `pet_overlay_show/hide/toggle` 三条命令，但 WebUI 桥**从未暴露**，任何入口都打不开 | ① 两处判据统一为「不等于 `'false'` 即开」，显式关过仍尊重；② 放大到 **168×186**（`--pet-w/--pet-h` 与默认落位、散步边界同源）；③ `usePetOverlay.ts` + `desktopBridge` 暴露三条命令 + 设置页/宠物页开关，命令缺失时如实置灰（不写假状态）；④ 启动时若上次开着就幂等恢复，且 `/pet-overlay` 自身排除（避免自我唤醒） |

### 21.2 实测（Playwright，安装版 `http://127.0.0.1:18790/control/*`）

```json
{
  "pet":  { "w": 168, "h": 186, "x": 1252, "y": 690, "svg": 1 },
  "mode": { "before": "标准·执行", "clicked": "极致·咨询", "after": "标准·咨询",
            "frame": "mode.switch {userMode:\"extreme_ask\"}",
            "toast": "「极致·咨询」按「标准·咨询」执行：咨询 / 方案两列不含强度档…" },
  "send": { "workspaceId": "ws-8516646b46a0673f",
            "dispatch": { "mode": "standard_plan", "agent": "main", "form": "single",
                          "swarmOn": false, "swarmSize": 3 } },
  "reply": "调度计划 … 结论：当前工作目录是 D:\\Downloads\\Marguerite … 会话已绑定该目录，"
           "list_dir 相对路径解析到的就是它，没有落到 AppData\\Local\\Marguerite 默认目录。",
  "overlayPage": { "mounted": true, "stage": "360×392", "svg": 1, "capsule": "打盹♥ 72⚡ 0",
                   "surface": "pet-overlay", "bodyBg": "rgba(0,0,0,0)", "sidebarVisible": false },
  "pageErrors": []
}
```

- 项目绑定与调度简报是同一条消息里同时验证的：`chat.send` 带 `workspaceId` + `dispatch`，模型回复以「调度计划」开头并读到了真实仓库（`Cargo.toml` 的 workspace 成员）。
- 悬浮窗页面在浏览器里需要注入最小 `window.MargueriteDesktop` 才会注册 Desktop 专属路由（真实窗口由壳提供桥）；该窗口里无侧栏、body 全透明、拖拽手柄与心情胶囊正常。

### 21.3 本轮踩到的坑（都留了注释）

1. **RPC 返回形状不一致是最隐蔽的一类缺陷**：同一个字段在 `get` 里是对象、在 `switch` 里是字符串，前端「赋值成功」不报错，但状态已经错了。写入点必须统一归一化。
2. **`usage_events.session` 为空串**：聚合查询按会话过滤后必然为空 —— 有表不等于有数据，指标要先验证分母。
3. **CSS 权重**：`[data-theme] .x` 打不过 `#app .a .b .x`，且组件内注入的样式更晚 —— 主题改不动时先查具体性与注入顺序。
4. **`chat.send` 的绑定必须每轮重发**：只在首轮带一次，一旦首轮错过（草稿从 URL/恢复态进入）就永久掉出项目。
5. **`run_turn` 参数超 7 个触发 clippy**：收进 `TurnOptions` 结构体，而不是 `#[allow]`。

### 21.4 门禁

| 门禁 | 结果 |
| --- | --- |
| `cargo clippy -p marguerite-core --all-targets -- -D warnings` | 0 warning / 0 error |
| `cargo test -p marguerite-core --lib plan::` | 71 passed / 0 failed |
| `cargo test -p marguerite-core --lib gateway::chat` | 28 passed / 0 failed（含 Bloom / 计划 / 单轮三条路径） |
| WebUI 单测 | **426 文件 / 4299 用例**：4298 通过；`SettingsDialog.dataMaintenance.test.ts` 1 条在满负载下 15 s 超时（单独复跑 2/2 通过，属既有慢用例） |
| `npm run typecheck` | 0 错误（架构守卫 + i18n 3861 键 0 undefined + 契约 408 条 0 幽灵） |
| 构建 | 产物 321 文件；主题包守卫 1 skin + 7 world 全懒加载 |
| 打包 | `scripts\package_app.cmd` 成功；前端文件数 源码=322 / 包内=322 |

### 21.5 安装包（2026-10-24 15:03 构建）

| 产物 | 大小 | SHA-256 |
| --- | --- | --- |
| `D:\marg_build_target\release\bundle\msi\Marguerite_0.2.16_x64_en-US.msi` | 419.9 MB | `0BBDB03B4DB5A16C95D4B67D2416E0C1E3208921B00117B013192D6C46A38C7F` |
| `D:\marg_build_target\release\bundle\nsis\Marguerite_0.2.16_x64-setup.exe` | 384.3 MB | `77352A4728827BB34B78728064CD8AA6A541714A559358FD2CDBEB79AD184ADC` |

- release 编译 10m35s；安装版已用新 exe 替换（备份 `marguerite-gui.exe.bak-20261024-151458`）并重启验证（PID 32540）。

### 21.6 已知限制（如实记录）

1. 桌面悬浮窗的**窗口创建**（Tauri 侧 360×420 透明置顶窗）本轮只验证到「路由渲染正确 + 桥命令已暴露」，未做桌面级自动化点击取证；Web 环境下开关如实置灰，不假装可用。
2. 命中率只在服务端回传 `cached_tokens` 时显示；没有缓存命中就**不显示**，不用 0% 冒充。
3. `usage.status` 的空会话行（6982 行）未回填，历史台账仍是脏数据；底部计量条走实时帧，因此不受影响。
4. 桌宠默认开启是产品取舍：显式关过（`pet.desktopEnabled = 'false'`）仍保持关闭，不会被本次改动翻回。

## 22. 第九轮：桌宠改真三维（2026-10-24 15:30–16:05）

### 22.1 现象 → 根因 → 修法

| # | 用户看到的现象 | 根因 | 修法 |
| --- | --- | --- | --- |
| 1 | 「桌宠要能显示 3D 的」 | 全仓**有**真三维场景（`pet3d/Pet3DScene`，可转相机看真实背面几何），但只有 `PetView` 用它；`DesktopPet.vue` 与桌面悬浮窗 `PetOverlayView.vue` 都**硬编码** `new PetSvg(...)` —— 同一个皮肤在「宠物页」是立体、在桌面/悬浮窗是扁的 | 新增 `components/pet/petRenderer.ts`：`createPetRenderer()` 优先三维（`three` 走**动态 import**，不进首屏包），WebGL 缺失或构造失败时如实回退 2.5D 并回传 `fallbackReason`；页内桌宠与悬浮窗统一走它 |
| 2 | （顺带）三维下拖桌宠会同时转动模型 | `Pet3DScene` 在自己 canvas 上绑了 pointerdown 环绕/滚轮缩放；桌宠的拖拽在**外层容器**上 | 两处宿主的画布统一 `pointer-events: none`（`is-3d :deep(canvas)`），手势归宿主；视线跟随仍由 window `mousemove → lookAt` 驱动 |
| 3 | （顺带）三维态没有「抚摸 / 散步」反应 | 桌宠用到的 `poke(region)` / `setWalking(bool)` 在 `Pet3DScene` 上不存在（只有 SVG 有） | 给 `Pet3DScene` 补两个真实实现：`poke` 复用 pointerup 的同一条通道（头=点头/背=侧倾/尾=甩尾），`setWalking` 驱动 `computePose` 的步态相位；`PetRenderer` 接口补齐到「两种渲染器都有的公共子集」 |
| 4 | （顺带）悬浮窗里白开第二个 WebGL 上下文 | 悬浮窗路由同样挂 App 外壳，`DesktopPet` 也在，只是被 `[data-surface="pet-overlay"]` 的 CSS 藏成 0×0 —— 但三维循环照跑 | `App.vue`：`<DesktopPet v-if="!isPetOverlayRoute" />`（那扇窗已有自己的 3D 舞台） |

### 22.2 实测（安装版，Playwright + 软件渲染 WebGL）

```json
[
  { "path": "/control/pet", "hostClass": "desktop-pet__3d is-3d", "box": "168×186",
    "canvas": { "w": 168, "h": 186, "pointerEvents": "none" },
    "pet3d": "ready", "pet3dKind": "beast", "view": "front", "svg": 0, "errors": [] },
  { "path": "/control/pet-overlay", "hostClass": "pet-overlay__stage is-3d",
    "canvas": { "pointerEvents": "none" },
    "pet3d": "ready", "pet3dKind": "humanoid", "view": "front", "svg": 0, "errors": [] }
]
```

截图：`docs/qa-shots/qa-pet3d-inline.png`（页内：橘猫三维，含落地阴影）、
`qa-pet3d-overlay.png`（悬浮窗 360×420：花冠精灵三维，正面视角，手脚与裙摆是真实几何）。

### 22.3 门禁（本轮新鲜）

| 门禁 | 结果 |
| --- | --- |
| `npm run typecheck` | 0 错误（架构 / i18n / 契约 / 主题守卫全过） |
| WebUI 单测 | **426 文件 / 4299 用例全绿** |
| `npm run build` | 产物 325 文件 / 82 个 JS（three 仍是懒加载 chunk，主题包守卫通过） |
| 回归守卫更新 | `detectPetRuntime.test.ts`：三个构造点仍须 `renderTierOf(` 透传；`DesktopPet` / `PetOverlayView` 必须经 `createPetRenderer(`（且档位在调用点透传）；工厂内 `quality: opts.quality` 必须出现 **2 次**（三维 + 矢量各一），漏一个就是某条路径上档位失效 |

### 22.4 安装包（2026-10-24 16:00 构建）

| 产物 | 大小 | SHA-256 |
| --- | --- | --- |
| `D:\marg_build_target\release\bundle\msi\Marguerite_0.2.16_x64_en-US.msi` | 419.9 MB | `4BCA1CCAAA0757B1ECEF42FC34386CFAF5FC31A52EE293F8DB1E90F456482069` |
| `D:\marg_build_target\release\bundle\nsis\Marguerite_0.2.16_x64-setup.exe` | 384.3 MB | `354EDE64887E290494EBBACF1512B38AA0EF33D454C2202CC8101D82B5DE1B05` |

- release 编译 5m03s；前端文件数 源码=326 / 包内=326；`webui/dist` 已同步到安装目录的两处副本，重启后实测为三维。

### 22.5 已知限制

1. 三维桌宠在**软件渲染**（无独显 / 远程桌面）下帧率会低于矢量版；这是真三维的固有代价，`renderTierOf` 只调粒子/阴影预算，不降几何。
2. 悬浮窗的三维模型是正面朝前（`view: 'front'`）——桌面桌宠要「看着用户」，不提供视角切换（想转视角请到「宠物」页，那里有正面/45°/正侧/背面与自动旋转）。
3. 三维画布不接收指针事件：桌宠上**不能**拖拽环绕模型（拖拽＝挪桌宠）。这是刻意取舍，环绕只在宠物页可用。

## 23. 第十轮：工作台「+」新增面板 / 内置编辑器 / 浏览器接管协议（2026-10-24 16:00–17:10）

### 23.1 需求 → 关键事实 → 落地方案

| 需求 | 查证到的事实 | 落地 |
| --- | --- | --- |
| 侧栏能加「终端 / 内置浏览器 / 编辑器」 | 终端面板与浏览器面板**早就存在**；但入口只有「无标签时标题栏那个键盘图标」——而工作台在**没有任何标签时压根不挂载**（`shouldMount = enabled && activeItem !== null`），所以那个图标永远点不到；浏览器只能靠点消息里的链接 | 标题栏新增「+」菜单（终端 / 内置浏览器 / 代码编辑器）；调色板补两条冷启动动作（终端、浏览器）先把 dock 建起来 |
| 「IDE 一样的编辑器」+ 智能体改动可见 | 全仓**没有**编辑器（无 CodeMirror/Monaco，只有 highlight.js）；后端却有 `workspace.file.read/write` 与 `workspace.file_changes.list/revert` 真台账 | 新增 `EditorPanel`（kind `file`）：行号 + highlight.js 高亮（按需注册 13 门语言）+ 查看/编辑/改动三态 + `Ctrl+S`；智能体改动按行标记（LCS 行对齐），保存前回读磁盘，内容变了必须「重新载入 / 仍然覆盖」 |
| 智能体操控浏览器时要有状态（荧光边框）与「用户接管」 | 壳里**完全没有** `createWorkbenchSurface` 一族命令 ⇒ 原生子 webview 面板打不开；但 `browser.open/navigate/screenshot/click/a11y/evaluate` 是真 CDP | 浏览器面板改为 **CDP 镜像**：真实 Chromium 截图 + 点击转发 + 操作日志；智能体的 `browser` 工具调用（`session.event.tool_use_start`）会让面板边框呼吸泛光并显示「第 N 步」 |
| 接管要「真」，不是换个颜色 | 原本没有任何控制权概念 | core 新增 **控制权租约** `browser/control.rs`：`browser.control.get/set`；`owner=user` 时 `tool.browser` 的所有页面动作被**硬拒绝**（`USER_IN_CONTROL`），工具错误文案写明「谁在控制、从何时起、下一步怎么办」 |
| 用户动手后怎么确认、怎么接着任务 | 网关有 `sessions.steer.v2`（运行中插话） | 点「交还」时把接管期间的操作摘要作为插话送给运行中的回合；回合已结束则**如实告知未送达**（不假装送达） |

### 23.2 实测（安装版，Playwright，1440×900）

```json
{
  "menu": ["沙箱终端", "内置浏览器", "代码编辑器"],
  "editor": { "loaded": "AGENTS.md 1083 行 / 134149 字节", "gutterLines": 1083,
              "hljsTokens": 2, "edit": "可输入", "save": "已保存（dirty 清除）",
              "root": "D:/Downloads/Marguerite" },
  "browser": { "shot": "589×348 data:image/png", "pills": ["智能体侧控制"],
               "buttons": ["导航", "截图", "接管"] },
  "afterTakeover": { "className": "pd-browser is-user", "pill": "你正在控制",
                     "log": "16:55:06 接管 / 16:55:02 打开 https://example.com",
                     "note": "已接管：智能体此后收到的浏览器动作会被拒绝…" },
  "afterHandback": { "pill": "智能体侧控制",
                     "note": "已交还控制权；但本轮已结束，插话未送达——请把操作记录发给智能体。" },
  "pageErrors": []
}
```

截图：`docs/qa-shots/qa-panels-menu.png`（+ 菜单）、`qa-panels-editor.png`（App.vue 2120 行，行号 + 高亮 + 根路径）、
`qa-panels-browser.png` / `qa-panels-browser-takeover.png`（截图 + 接管态 + 操作记录）。

### 23.3 本轮踩到并修掉的坑

1. **面板自己是入不了口的**：`shouldMount` 要求已有 activeItem，而「+」在面板标题栏里 —— 于是冷启动无路可走。补了调色板两条动作先把 dock 建起来。
2. **`workspace.file.*` 的根是「会话项目根」而不是用户点开的文件所在目录**：未绑定项目时会话读到的是应用数据目录。现在支持**绝对路径**（拆成「根 = 所在目录 + 文件名」），并在页脚显示真实根。
3. **`browser.status` 冷加载竞态**：面板刚挂载时 WS 还在 connecting，`loadStatus` 静默返回 ⇒ 面板只剩一个空框、连错误都没有。改为重试 3 次 + 显式「正在检测…/重新检测」。
4. **本机地址被 SSRF 防护拒绝**：`http://127.0.0.1` 直接报 `blocked address (private/loopback/metadata)`。这是既有的安全策略，本轮**不改**；预填地址改成 `https://example.com`，提示文案写明「回环与内网地址会被拒绝」。
5. **空文件不等于「一行空内容」**：`''.split('\n')` 得到 `['']`，会把「新建文件」算成 1 处修改 + n 处新增。`toLines('')` 现在返回零行。

### 23.4 门禁（本轮新鲜）

| 门禁 | 结果 |
| --- | --- |
| `cargo clippy -p marguerite-core --all-targets -- -D warnings` | 0 warning / 0 error |
| `cargo test -p marguerite-core --lib browser::` | 27 passed / 0 failed / 6 ignored（含新增控制权租约用例） |
| WebUI 单测 | **428 文件 / 4311 用例全绿**（新增 `lineDiff` 8 条、`editorItems` 7 条、浏览器 provider 5 条） |
| `npm run typecheck` | 0 错误（i18n 6 语种 3935 键 0 undefined；契约 410 条 0 幽灵） |
| `npm run build` | 产物 333 文件 / 85 个 JS（编辑器与浏览器面板均为异步组件，highlight.js 不进首屏） |

### 23.5 已知限制（如实记录）

1. **内置浏览器打不开本机回环地址**（`127.0.0.1` / `localhost`）：`browser.open` 走 SSRF 防护，回环/内网一律拒绝。要预览本机开发服务，需要产品层面决定是否给「用户显式发起」的打开动作开一条 scoped 白名单（仍未做）。
2. 面板是**截图镜像**，不是原生控件：滚动、键盘输入、hover 都不转发（点击会转发）。所以「接管」目前等于「可以点、可以导航、可以截图」，不等于完整浏览器。
3. **智能体用的是另一个无头 Chromium 实例**（`tool.browser` 的会话表与 `browser.*` RPC 的会话表是两份）。因此面板上的「智能体正在使用浏览器」是**状态**（来自工具调用事件），画面不是智能体那一窗。两者统一需要把两套会话表合并，本轮未做。
4. 接管的硬闸门只拦 `tool.browser`；若智能体通过 `browser.*` RPC（例如浏览驾驶舱页）操作，仍会受同一个租约影响（它们共用 `browser.control`），但没有针对「每会话」的细粒度锁。
5. 安装包：首次打包在 WiX 下载阶段失败（`io: Connection refused`），重试后成功（见 §23.6）。

### 23.6 安装包（2026-10-24 17:20 构建）

| 产物 | 大小 | SHA-256 |
| --- | --- | --- |
| `D:\marg_build_target\release\bundle\msi\Marguerite_0.2.16_x64_en-US.msi` | 420.1 MB | `4804F3E60CF7CDAE68C14AECF1EE1F12A98B68AD345C4670825FE3C56784D61D` |
| `D:\marg_build_target\release\bundle\nsis\Marguerite_0.2.16_x64-setup.exe` | 384.5 MB | `09069A720E4FECB0E0247F2339C385F0279204F075D4FEA4742E924BA89D19F1` |

- 前端文件数 源码=334 / 包内=334；安装版已换上新 exe（备份 `marguerite-gui.exe.bak-20261024-164349`）并完成真机验证。

## 24. 第十一轮：智能判断替代硬编码 / 团队名册 / 子代理思考 / 缓存命中（2026-10-24 18:00–22:00）

### 24.1 用户两次反馈（截图原文）→ 落地点

| 用户诉求 | 落地 |
| --- | --- |
| 超时 / 轮数 / 重试「限制太多反而影响发挥」 | 新增 `agent/budget_policy.rs`：四档（simple 12 轮/120s、standard 24/300s、complex 40/600s、deep 64/900s，整轮 15min…2h，读超时 120…480s，请求尝试 2…5 次）；模型可在计划里写 `[[budget:deep]]` 显式升档；env 仍可覆盖；全部经 `clamp_policy` 硬护栏 |
| 模型/工具自己的 `timeout_sec` 不生效 | `tool_cycle::declared_or_inferred_timeout`：优先读参数（+30s 收尾余量），`shell_exec` 的构建/装依赖/跑测试类命令自动 900s；普通命令仍 30s |
| MCP 工具 30s 就被判超时 | `mcp/client.rs`：控制面保持 30s，`tools/call` 改为 Standard 单步（300s），`MARGUERITE_MCP_TOOL_TIMEOUT_SECS` 可覆盖 |
| SSH 远程命令 60s 被掐死 | `collab.ssh.exec` 接受 `timeoutSec`，缺省 Standard 单步，钳制 5s…30min |
| 隔离子代理一律 120s | `harness` 的默认超时与 `IsolatedSpec::new` 都改为按任务解析（Standard 30min 上限） |
| 视觉模型（截图→坐标）频繁假超时 | 视觉 transport 读超时与主模型同源（240s） |
| 团队 / 专家团不能取消选择或多选 | `ChatTeamPicker` 多选 + 再点取消 + 「已选 N 个 / 全部取消」；`teams: string[]` 全链路透传到 `team_run_with`；8 个内建专家名册 + 自定义组合按角色去重合并 |
| 子代理思考不显示 | 后端每轮发 `member_thinking` 帧（截断 6000 字符、最多 40 轮），前端在编排卡片成员详情按「成员思考 · 第 N 轮」分块展示 |
| 输入框「+」只有上传文件 | `ChatComposerAddMenu` 七个真实入口：添加文件 / 命令（`/` 清单）/ 我的产物 / 工作区文件 / 技能 / 插件 / 计划模式 |
| 工作区、分支、工作位置（本地/云端/远程） | `ChatComposerContextBar` 三枚胶囊：工作区名/路径、`WorkspaceGitSummary` 分支与脏标记、工作位置（本地可用；云端/远程如实标注「未接入」并说明原因） |
| 缓存命中率低 | ① pyengine：`prompt_cache` 关闭时动态块不再拼回 system（每轮 system 必须逐字节稳定）；② 工具列表 `sort_unstable()`（`tool_guide_text`）与 `describe_*_tools` 结果排序，消除 HashMap 迭代顺序抖动导致的前缀失效 |
| 卡片排序 / 输出排版 | 存储层「最近优先」→ 渲染层反转成正序；编排卡片补「产出文件」区（只认写类工具，JSON/`path:` 双解析，最多 12 条） |

### 24.2 硬护栏（任何声明都不能突破）

| 项 | 上限 |
| --- | --- |
| 工具回路轮数 | 64 |
| 单步超时 | 30 分钟 |
| 整轮超时 | 2 小时 |
| 单次请求尝试 | 5 次 |
| 单批工具调用 | 64（Standard 默认 24，历史值 10 为下限） |

### 24.3 本轮踩到并修掉的坑

1. **模型声明了超时也不生效**：`ToolCall::new` 此前只按工具名给默认值，模型写 `timeout_sec=600` 仍被 30s 掐死 —— 改为参数优先，长命令特征兜底。
2. **兜底不能随手放宽**：一度把 `shell_exec` 兜底改成 180s，破坏「shell 风险在副作用不在耗时」的既有契约且有测试守着；最终保留 30s，只在**有证据**（参数/命令特征）时放宽。
3. **env 变量类型混用**：`MARGUERITE_BUDGET_SCALE` 是档位字符串，不是数字；误用 `env_u64` 会永远读不到。
4. **`export` 假成功**：无会话库时 `format=pdf` 直接返回 `ok:true` 绕过校验 —— 参数校验前移到早退之前。
5. **前端契约守卫漏登记**：`HarnessView` 增读 `sessions.insights` 后，`contractParity` 的冻结清单仍是 4 条 —— 补登记，避免「新增 RPC 只在前端可见」。

### 24.4 门禁（本轮新鲜）

| 门禁 | 结果 |
| --- | --- |
| `cargo test -p marguerite-core --lib` | **3751 passed / 0 failed / 22 ignored** |
| `cargo clippy -p marguerite-core --all-targets -- -D warnings` | exit 0（0 warning / 0 error） |
| WebUI `npm run typecheck` | exit 0（i18n 4004 键 0 undefined；契约 413 条 0 幽灵；主题/动效/配色守卫全过） |
| WebUI `npm run test:unit` | **429 文件 / 4321 用例全通过** |
| WebUI `npm run build` | 产物 334 文件（主题包 1 skin + 7 world 全懒加载） |

### 24.5 安装包（2026-10-24 21:48 / 22:0x 构建）

| 产物 | 大小 | SHA-256 |
| --- | --- | --- |
| `D:\marg_build_target\release\bundle\msi\Marguerite_0.2.16_x64_en-US.msi` | 420.3 MB | `5EB9D1B217337FEACF026B5C87B5DAC9FFFA9069697428C638120DAADD2C0378` |
| `D:\marg_build_target\release\bundle\nsis\Marguerite_0.2.16_x64-setup.exe` | 384.6 MB | `6A3B3EA065A2EA9ADAB41AAFB881318D01D4FD8687BADB4E410B8ABC6AE35C84` |

- release 编译 8m56s；`build.rs` 的技能树同步（并发写者、只读 git repo、瞬时消失路径）本轮未再触发 `os error 5`。
- NSIS 首次因 DNS（`os error 11001`）下载失败，重试后成功（与 §23.5 同一类环境问题）。
- **已把 MSI 静默安装到真机**（`msiexec /i … /qn`，exit 0），并逐项核对：
  - `<install>\marguerite-gui.exe` 21:46:56 构建，含 `sessions.insights` / `harness.ledger.status` / `plugins.mcp.call` / `skills.coverage`
  - `resources/pyengine/.../gateway/static/dist/index.html` 与源码 `webui/dist` **SHA-256 相同**；文件数 334 = 334
  - 入口 chunk `index-DfZiE3OZ.js` 哈希一致，且含 `chat.addMenu.*`、`chat.context.locationUnavailable`、`member_thinking`
  - 重启应用（PID 24324）后 `/control/` 实际返回的入口就是上述 chunk —— 「源码改了但桌面端没变」这次已闭环

### 24.6 已知限制（如实记录）

1. **云端 / 远程工作位置仍未接入**：没有对应 RPC，入口按「不可用 + 原因」呈现，不提供假开关。
2. `pyengine` 侧压缩常量（`_COMPACTED_STRING_MAX_CHARS`、`_DEFAULT_PROTECTED_RECENT_RESULTS` 等）保持为确定性启发式，且已有 `OPENSQUILLA_PROVIDER_COMPACTION_*` env 覆盖；不改成模型判断，避免压缩正确性变得不可复现。
3. 推理滑块的粒子流动画在 `prefers-reduced-motion` 下完全静止（含极限档呼吸），这是可访问性取舍。


