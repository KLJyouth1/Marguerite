# PRD：桌面四端模型能力与像素级体验完善

- 日期：2026-10-21
- 档位：M
- 范围：`webui`、`core/src/tui`、`core/src/cli`、`pyengine`/桌面网关
- 关联：`deliverables/PHASE_B_P3_MATRIX.md`
- 当前阶段：批次 1（PRD、TUI 模型面板、`llmProfiles` 消费闭环、四端契约排查）

## 1. 问题

当前模型能力已有多个入口，但仍存在三类断点：

1. TUI 能执行模型操作，却缺少用户可浏览、理解、切换当前档案的独立面板；
2. 后端 `config.get` 已返回 `llm_profiles`，webui 仅在设置目录内部间接读取，模型指示器仍只显示当前主模型，档案集合没有清晰的展示与刷新闭环；
3. PHASE_B 的 11 项矩阵只有静态源码对账，尚未形成逐页、逐状态、逐交互的验证回执。

此外，四个端对同源字段可能各自解释，导致“某一端已更新、另一端不显示”的隐性契约漂移。

## 2. 用户故事

- US1：作为 TUI 用户，我可以打开模型面板，看到当前主模型、所有有效档案和状态，并在不离开界面的情况下完成激活。
- US2：作为桌面用户，我可以在聊天顶栏看到当前模型与已保存档案数量，并确认模型变更后信息自动刷新。
- US3：作为设置用户，我能从 `llm_profiles` 得到一致的已保存服务商信息，而不是依赖各页面重复推导。
- US4：作为维护者，我能用自动化测试判断四端消费的字段名、事件名、错误语义是否仍保持一致。
- US5：作为验收者，我能逐行看到 11 项 P3 能力的真实状态、证据和不能在当前环境完成的原因。

## 3. 目标

- G1：为 TUI 增加可访问的模型档案面板，覆盖列表、当前项、空态、错误态、键盘导航与激活反馈。
- G2：建立 webui 对 `llm_profiles` 的单一消费点，并让模型指示器消费档案集合信息。
- G3：补齐四端对 `llm_profiles`、`models.changed`、当前主模型和错误返回的契约测试。
- G4：按 PHASE_B 矩阵逐项形成真机验证回执，区分“完成”“部分完成”“未验证”。
- G5：依据现有设计 token 和组件规范完善布局、响应式、焦点、对比度及边界状态。

## 4. 非目标（NG）

- NG1：不重建整个模型选择器、模型注册表或 provider 凭据体系。
- NG2：不在本轮引入新的 RPC 方法；优先复用 `config.get`、`models.changed` 和现有 `ModelOp`。
- NG3：不删除 CLI、TUI 或设置页现有入口。
- NG4：不把静态源码检查冒充真机点击；无法在沙箱执行的项目必须标为 🟡、🔴 或 ⚪。
- NG5：不引入新的前端框架、图标库或全局样式主题。
- NG6：不修改模型准入 tier 策略、计费预算或路由算法。
- NG7：不展示、持久化或通过 RPC 回传 API key。
- NG8：不以“视觉更好看”替代可测验收；每项改动必须能由测试、构建、截图或逐页记录验证。

## 5. 需求池

### P0

- P0-1：TUI 模型面板。
- P0-2：webui 的 `llm_profiles` 单一消费与刷新。
- P0-3：四端契约和回归测试。

### P1

- P1-1：11 项 P3 逐页验证回执。
- P1-2：模型相关界面的状态穷举、响应式和无障碍修复。
- P1-3：构建产物与部署副本指纹对账。

### P2

- P2-1：基于截图或浏览器自动化的真实视觉 QA。
- P2-2：跨端统一诊断文案与遥测。

## 6. 接口与状态契约

### 6.1 `config.get`

响应继续包含：

```json
{
  "llm": {
    "provider": "string",
    "model": "string",
    "base_url": "string",
    "api_key_configured": false
  },
  "llm_profiles": {
    "providerKey": {
      "model": "string",
      "base_url": "string",
      "profileId": "string",
      "label": "string"
    }
  }
}
```

约束：

- `api_key` 不回传；
- `profileId` 是档案稳定标识；
- `providerKey` 可用于展示与归组，但不替代 `profileId`；
- 字段缺失时前端按空集合处理，不显示伪造档案；
- 响应字段名以 `llm_profiles` 为准，代码和测试不得混写为 camelCase。

### 6.2 刷新事件

- 事件：`models.changed`
- 消费方：模型指示器、设置目录、TUI 状态刷新。
- 更新需防抖，避免一次激活的多条事件造成重复请求。

### 6.3 TUI 面板状态

必须覆盖：

- loading：正在读取档案；
- ready：至少一个当前模型或档案；
- empty：无可展示模型；
- error：读取或激活失败；
- activating：激活请求进行中；
- activated：激活成功并显示 notice；
- active：当前档案有唯一视觉和 ARIA 标识。

### 6.4 四端一致性

| 语义 | webui | TUI | CLI | pyengine/网关 |
|---|---|---|---|---|
| 当前模型 | `llm.model` | `WorkbenchState.model` | `model` 输出 | `effective_model_fields` |
| 档案集合 | `llm_profiles` | `ModelSelector::registry()` | `model list` | 同源 selector |
| 刷新 | `models.changed` | `ModelOp` 后状态回写 | 命令后重读 | `publish("models.changed")` |
| 凭据 | 仅 `api_key_configured` | 不显示密钥 | 不显示密钥 | 不输出明文 |

## 7. EARS 验收标准

- AC1：WHEN 用户在 TUI 请求模型面板，THEN 面板显示当前模型、有效档案、状态和键盘提示。
- AC2：WHEN 档案集合为空，THEN 显示明确空态，不渲染空白表格。
- AC3：WHEN 读取档案失败，THEN 显示错误态并允许重试，不把错误伪装为“无模型”。
- AC4：WHEN 用户激活非当前档案，THEN 成功后当前项、状态栏和 notice 后同步一致。
- AC5：WHEN `models.changed` 连续触发，THEN webui 防抖后重新读取 `config.get`。
- AC6：WHEN `llm_profiles` 缺失，THEN webui 将其归一为空集合且不崩溃。
- AC7：WHEN P3 某项无法在沙箱执行，THEN 回执写明环境限制并保留下一步命令。
- AC8：WHEN 四端任一消费字段改名，THEN 契约测试必须失败。
- AC9：WHEN 改动完成，THEN `cargo test`、`vue-tsc`、vitest、i18n/RPC 守卫均通过。
- AC10：WHEN 新增可见文案，THEN 6 个 locale 的 key 齐全且引用为字面量。

## 8. 视觉与交互验收

- 使用现有 CSS token，不出现裸 hex/rgb 字面量。
- 正文对比度目标 ≥ 4.5:1，图标和焦点边界 ≥ 3:1；无法测量的项记录实测限制。
- 触摸目标高度不低于 44px，桌面紧凑工具项不低于 32px。
- 焦点顺序与视觉顺序一致；`Esc` 关闭浮层；当前项使用 `aria-current`。
- loading 使用稳定尺寸的 skeleton/占位，避免布局跳动。
- 空态、错误态、离线态、超长模型名、窄屏和深色模式均有明确表现。
- 模型名超出时单行省略并保留完整 `title`；provider 和状态不得互相遮挡。
- 模型指示器只展示用户需要的主信息，完整档案通过明确入口查看。

## 9. 风险与缓解

- 风险：重复读取 `config.get`。缓解：共享 composable/store 并合并事件。
- 风险：provider key 不是稳定 ID。缓解：优先使用 `profileId`，展示时才使用 provider/label。
- 风险：TUI 状态与持久化分叉。缓解：所有激活动作复用现有 `ModelOp`，不另写第二条持久化路径。
- 风险：真机验证受沙箱限制。缓解：将静态验证、RPC 验证、人工真机验证分层记录。
- 风险：并发写入导致覆盖。缓解：编辑前检查 mtime/内容，使用精确编辑而非整文件覆盖。
- 风险：视觉重构扩大回归面。缓解：只改模型相关组件和明确发现的缺陷，分批提交并保留回滚。

## 10. 改动契约

- 触发：R2、R3、R5、R6、R7。
- 改动面：`core/src/tui`、`core/src/gateway/config.rs`、`webui/src`、locale、测试。
- 影响面：桌面聊天顶栏、设置页、TUI、CLI 数据解释、网关事件。
- 回归清单：`cargo test`、`npm run typecheck`、`npm run test -- --run`、i18n/RPC contract guards。
- 回滚：按文件恢复 Git diff；新状态字段使用 `#[serde(default)]`；前端消费点保持向后兼容。

## 11. 批次与终止条件

### 批次 1

- PRD 落盘。
- TUI 模型面板完成。
- `llmProfiles` 消费闭环完成。
- 四端契约排查完成。
- 全量门禁通过。

### 批次 2

- 11 项 P3 真机逐页验证。
- 截图/浏览器实测和对比度记录。
- 独立 QA 放行结论。

## 12. 回滚

1. 保留改动前 Git 状态或文件副本。
2. 单独回滚 TUI 面板不影响 webui 消费。
3. 前端消费失败时回退到只读取 `llm.model`，不删除字段。
4. 构建失败时恢复上一份 `webui/dist` 和 TUI 二进制。
5. 回滚后重新执行全量门禁和部署指纹对账。
