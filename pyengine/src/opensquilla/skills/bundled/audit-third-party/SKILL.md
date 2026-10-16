---
name: audit-third-party
description: "Use when the user asks to security-audit / supply-chain-scan cloned third-party repos before integrating them. Implements a refined (v2) version of the skill-vetter RED FLAGS protocol with test/doc false-positive suppression."
description_zh: "当用户要求对克隆落地的第三方仓库做供应链安全审查/集成前闸门时使用。实现 skill-vetter RED FLAGS 协议的改进版（v2，已抑制测试/文档误报）。"
always: false
triggers:
  - audit
  - auditing
  - security audit
  - 安全审查
  - 供应链
  - supply-chain
  - 风险审查
  - 扫描仓库
  - skill-vetter
  - 红队扫描
  - 集成前闸门
  - scan third party
provenance:
  origin: marguerite-derived
  license: MIT
  upstream_url: https://github.com/deepseek-ai/deepseek-harness
  maintained_by: Marguerite
metadata:
  opensquilla:
    # ``bash`` is a *binary* requirement, not a tool requirement: listing it under
    # ``requires_tools`` asked the runtime gate for a tool named "bash", which does
    # not exist, so this skill was silently dropped from every prompt on every host.
    requires:
      bins:
        - bash
    requires_tools:
      - exec_command
---

# Audit Third-Party Skill（供应链安全审查 · skill-vetter 协议 v2）

克隆落地后、集成前的安全闸门：对 `third_party/{harness,computer_use}` 做只读静态安全审查，
适配自 skill-vetter 的 RED FLAGS 协议。仅做静态分析，**绝不执行任何仓库代码**。

## 何时使用

- 一批社区仓库已克隆，集成进 Marguerite 前需过一道供应链安全闸门。
- 用户明确 @skill-vetter 或要求「安全审查 / 供应链扫描 / 风险复核」。

## 调用方式

```bash
# 审查默认 third_party/（或任意 vendor 目录）
bash audit-third-party.sh /path/to/third_party
```

## v2 关键改进（消除误报）

纯正则 RED FLAGS 扫描**必然误报**——会把「安全扫描器测试数据」「文档提及」也判为危险信号。
v2 已：

1. **排除测试/文档/示例**：`*.test.*` `*.spec.*` `/tests` `/docs` `node_modules` `.git` 等；
2. **收紧正则**：`eval(` 排除 `evaluate(`/`.exec()`；`exec(` 排除 `.exec()`；`http://N.N.N.N`
   才算真实外联；`sudo` 仅匹配真实提权调用而非 CI 定义。

## 风险分级与处置

| 级别 | 含义 | 处置 |
|------|------|------|
| LOW | 基础审查即可 | 可集成 |
| MEDIUM | 需完整代码审查 | 审查后集成 |
| HIGH | 需人工批准 | 批准 + 完整审查 |
| EXTREME | 疑似高危 | **禁止集成**，人工复核源码 |

## ⚠️ 方法论边界（重要）

即便 v2 仍可能有 EXTREME 命中，但**纯正则无法区分「恶意代码」与「危险模式引用」**
（例如某仓库把 `curl | sh`、`/etc/passwd` 作为测试用例，CI 里写 `sudo apt-get`）。
**任何 EXTREME/HIGH 命中都必须人工复核上下文**，不能仅凭字符串匹配下结论。本审查是
必要非充分条件——真正的闸门是语义理解 + 人工核验。

## 产出

- `third_party/security_audit_report.md`：每仓许可证 + 危险信号命中（按严重度）+ 风险定级 + 处置建议。
