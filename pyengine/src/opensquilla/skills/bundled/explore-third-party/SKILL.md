---
name: explore-third-party
description: "Use when the user asks to survey / inventory cloned third-party repos (language, build system, entry points, absorbable capability patterns) after a batch clone. Produces a capability map consumed by fusion/secondary-development."
description_zh: "当用户要求对已克隆的第三方仓库做统一勘探/能力清单（语言、构建系统、入口、可吸收模式）时使用，产出供融合二开消费的能力地图。"
always: false
triggers:
  - explore
  - exploring
  - 勘探
  - 统一勘探
  - 能力清单
  - 能力地图
  - capability map
  - inventory
  - 盘点
  - 二开调研
  - survey
  - 调研第三方
provenance:
  origin: marguerite-derived
  license: MIT
  upstream_url: https://github.com/deepseek-ai/deepseek-harness
  maintained_by: Marguerite
metadata:
  opensquilla:
    # ``bash`` is a *binary* requirement; see audit-third-party for why listing it
    # under ``requires_tools`` silently hid this skill from every prompt.
    requires:
      bins:
        - bash
    requires_tools:
      - exec_command
---

# Explore Third-Party Skill（统一勘探 · 产出能力清单）

克隆完成后（通常由 `clone-third-party` 技能触发），对 `third_party/{harness,computer_use}`
逐仓只读侦察，产出可吸收能力清单。本技能不修改任何上游代码。

## 何时使用

- 一批 GitHub 仓库已落地 `third_party/` 做二开，需要先摸清每仓的**语言 / 构建系统 / 入口 /
  可吸收能力模式 / 许可证**，再决定融合映射。
- 需要一份供 `core/src/harness/distill.rs` 蒸馏映射参考的逐仓报告。

## 调用方式

```bash
# 勘探默认 third_party/（或任意 vendor 目录）
bash explore-third-party.sh /path/to/third_party
```

## 产出

- `third_party/exploration_report.md`：分类表（仓库 / 主语言 / 构建系统 / 文件数 / 顶层结构）
  + 按 harness / computer_use 分类的能力模式深掘（子代理框架、生命周期钩子、动态路由、
  git 检查点、工具系统、屏幕捕获、键鼠控制、GUI 定位、视觉 grounding、视觉模型集成）
  + 许可证字段。

## 与 Marguerite 融合

勘探结果应回流到 `core/src/harness/distill.rs` 的 `UPSTREAM_REPOS` 蒸馏映射，并按模式改写
`core/src/harness` 与 `core/src/tools/computer_use`（三层：屏幕感知 / 键鼠控制 / GUI 定位）。
