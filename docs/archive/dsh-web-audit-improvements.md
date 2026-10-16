# DSH Harness Web GUI — 视觉/能力探查与改进点总结

日期: 2026-08-22   目标: http://127.0.0.1:4567/   模型: deepseek-v4-flash-vision-exp (text-only)

## 0. 结论先行
整体是插件化可组合的 harness Web 端，**任务执行逻辑(任务/认领/生命周期/attempt_id 绑定)质量良好**，自研编排链路可靠。但**视觉、记忆、SSH 三块外部能力当前处于未配置/静默失效状态**，GUI 对能力是否可用没有可见指示；前端仅分发已构建的 dist，源码在 git 仓库不在当前 checkout，无法直接改 React 组件。以下为实测证据 + 分级改进点。

## 1. 能力实测 (已执行的真实测试)
| 能力 | 结果 | 质量/说明 |
|---|---|---|
| 代码/Shell 执行 (run_code/pwsh) | ✅ | danger-full-access, 正常 |
| Web 搜索 (web_search) | ✅ | 网络正常, 返回结果 |
| 技能机制 (skill openviking-memory) | ✅ | 正常加载 |
| 子代理委托 (subagent) | ✅ | 高质量: 自主发现 github 未固定/孤儿依赖/缺密钥 |
| 智能体团队/蜂群 (agent_teams) | ✅ | 生命周期 pending→claimed→in_progress→completed, attempt_id 绑定无旧尝试冲突 |
| 视觉 (read_image/modlens) | ❌ | 模型 text-only, 无 vision provider; describe-image 桥 baseURL 未配置 |
| SSH/企业协同 (ssh_list) | ⚠️ | 无任何主机 ([]), 功能空缺 |
| 记忆 (hindsight) | ❌ | 401 缺 API key; OpenViking 插件已装但后端外置 |
| 像素级视觉代理 | ✅ | sharp: 平均RGB(24,24,27), 95%暗色, 0%亮色, 25 粗色级 → 深色主题有内容, 非空白 |

## 2. Web Profile 组成 (运行中的 bundle 顺序)
1. @deepseek-ai/dsh-base
2. @deepseek-ai/dsh-web-app
3. dshmarket
4. @liustack/modlens (3.21.1, 视觉引擎)
5. dsh-usage-stats
6. @linxin666/dsh-web-ui-all (^0.2.6)
7. @omdsh-dev/dsh-genui (github, 未固定)
8. @openviking/dsh-memory-plugin (github, 未固定)
9. @nanmicoder/dsh-agent-teams (^0.1.7)
10. @vectorize-io/hindsight-coding-agents (^0.3.4)
> cordis.patch.yml 为空 ([]), 无覆盖层。

## 3. 改进点 (P0 优先)
- **P0 供应链/可复现性**: github: 依赖未固定到 commit SHA (dsh-genui、dsh-memory-plugin、dsh-router-standard), mutable HEAD 有供应链风险; 建议锁定 commit。
- **P0 孤儿依赖**: dsh-router-standard 声明在 dependencies 但不在 bundles 数组, **从未被加载**(死配置/可能遗漏)。
- **P1 UI 包冲突**: dsh-web-ui-all + 独立 github dsh-genui 疑似重叠的全家桶/单功能 UI 包, 可能冲突, 需去重或明确职责。
- **P1 外部密钥缺失导致静默失效**: modlens(视觉)、hindsight(记忆)、openviking(后端) 均需外置配置/密钥; GUI 无能力不可用提示, 用户无感知 → 建议加能力健康度横幅。
- **P1 SSH 空缺**: 无主机, 企业协同与 SSH 不可用; 建议加入口/快速导入 + 空状态引导。
- **P2 前端不可改**: 仅 dist, 源码在 git 仓库; 需 clone + pnpm build 才可改 React 组件(当前 checkout 不满足)。

## 4. GUI 功能面 (来自 client-ui-* 模块与 settings)
桌面宠物(whale-girl, visible, 160px)、皮肤/壁纸/主题(dark)、onboarding(welcomeNoticeVersion=2026-08-13.1)、模型选择(deepseek-v4-flash/pro/flash-vision-exp)、设置面板(模型/插件清单/插件/权限预设/通用)、goal·todo·plan·skill·subagent·jobs·trajectory·workspace·workflow-run·message-feedback·user-questions·attachment·directory-picker·session-log-export 等; cordis.patch.yml 为空。

## 5. 建议的开发改进动作
1. 把 github 依赖固定到 commit; 激活或移除 dsh-router-standard。
2. 接入真实 vision provider(配 gemini/openai/anthropic 密钥, 或登录 claude-cli/kimi-cli), 使视觉探查/图像工具真正可用; 当前用户视觉要求因此受限于模型 text-only。
3. 配置 Hindsight/OpenViking 密钥并加记忆不可用可见标识。
4. 增加 SSH 主机管理与 GUI 空状态。
5. 在前端源码(tsx)中加外部能力健康度横幅与更清晰的空状态; 需在 git 仓库构建。
6. 保留现有任务执行逻辑(已验证良好), 不作大改。