# 技能系统

技能（Skill）是封装特定任务能力的可复用单元，以 YAML 描述文件定义，由技能引擎加载与执行。

## 技能 YAML 格式

```yaml
name: fix              # 技能名（唯一）
description: 修复代码中的错误   # 技能描述
version: 1.0.0         # 版本号
author: marguerite     # 作者
intent:                # 触发该技能的意图关键词（与意图系统联动）
  - fix
  - 修复
  - bug
tools: []              # 技能可用的工具列表
prompt_template: |     # 提示词模板（handlebars 风格，支持 {{text}} / {{#files}}）
  请为以下内容生成修复：
  {{text}}
  {{#files}}相关文件：{{files}}{{/files}}
```

字段说明：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 是 | 技能唯一标识 |
| `description` | 是 | 技能用途描述 |
| `version` | 是 | 语义化版本号 |
| `author` | 否 | 作者信息 |
| `intent` | 否 | 触发的意图关键词列表 |
| `tools` | 否 | 允许使用的工具白名单 |
| `prompt_template` | 是 | 渲染给模型的提示词模板 |

## 内置技能

项目自带 6 个内置技能（位于 [`skills/`](../skills/) 目录）：

| 技能 | 文件 | 用途 |
| --- | --- | --- |
| explain | `explain.yaml` | 代码解释 |
| refactor | `refactor.yaml` | 代码重构 |
| fix | `fix.yaml` | 缺陷修复 |
| test | `test.yaml` | 测试生成 |
| commit | `commit.yaml` | 提交信息生成 |
| doc | `doc.yaml` | 文档生成 |

## 架构组件

### 技能引擎（`core/src/skills/engine.rs`）

- 技能加载与卸载
- 技能执行沙箱：渲染 prompt 模板，隔离主 Agent 状态
- 技能效果评分：记录并评估执行效果

### 技能加载器（`core/src/skills/loader.rs`）

- 解析本地 YAML 技能描述文件
- 远程 GitHub 技能安装（`marguerite skills install <url>`）
- 技能签名验证

### 技能注册表（`core/src/skills/registry.rs`）

- 技能注册、查询与索引
- 支持热加载（运行时更新）

## 与意图系统联动

意图分类器识别用户意图（如 `fix`）→ 意图路由表将其映射到对应技能 → 技能引擎加载模板并执行。规则见 [`core/src/intent/rules.rs`](../core/src/intent/rules.rs)。

## CLI 管理命令

```bash
marguerite skills list            # 列出已安装技能
marguerite skills install <url>   # 安装远程技能（GitHub）
```

## 新增技能步骤

1. 在 `skills/` 目录创建 `<name>.yaml`，填写上述字段。
2. 在 `intent` 中加入触发关键词（如需意图联动）。
3. 在技能注册表注册（或依赖自动扫描）。
4. 运行测试验证加载与执行链路。
