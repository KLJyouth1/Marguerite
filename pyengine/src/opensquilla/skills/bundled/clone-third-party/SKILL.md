---
name: clone-third-party
description: "Use when the user asks to batch-clone GitHub repos for secondary development / third-party integration, or to set up a third_party/ vendor tree. Encapsulates a resilient, flaky-network-proof cloning protocol distilled from a 26-repo production clone run."
description_zh: "当用户要求批量克隆 GitHub 仓库用于二次开发 / 第三方集成，或搭建 third_party/ 供应商目录时使用。封装了从一次 26 仓生产级克隆中提炼出的抗弱网、防死循环克隆协议。"
always: false
triggers:
  - clone
  - cloning
  - github
  - batch clone
  - 批量克隆
  - 克隆仓库
  - third-party
  - third_party
  - 二开
  - 二次开发
  - 集成仓库
  - vendor
  - 供应商目录
  - fork all
provenance:
  origin: marguerite-derived
  license: MIT
  upstream_url: https://github.com/deepseek-ai/deepseek-harness
  maintained_by: Marguerite
metadata:
  opensquilla:
    # ``bash`` / ``git`` are *binary* requirements; see audit-third-party for why
    # listing them under ``requires_tools`` silently hid this skill from every prompt.
    requires:
      bins:
        - bash
        - git
    requires_tools:
      - exec_command
---

# Clone Third-Party Skill（批量克隆第三方仓库 · 生产级韧性协议）

当用户要求「把一批 GitHub 仓库克隆下来做二次开发 / 集成」时，调用本技能附带的
`clone-third-party.sh`。本技能不重新发明轮子，而是把一次**真实生产环境 26 仓克隆**
（deepseek-harness 等 harness 生态 + robotgo / OpenCUA 等 computer use 生态）中踩过的
所有坑固化成可复用协议。完整执行报告见 `C:\Users\Administrator\Desktop\克隆任务执行报告.md`。

## 何时使用

- 批量克隆 ≥3 个 GitHub 仓库到本地 `third_party/`（或任意 vendor 目录）做二开。
- 网络环境不稳定（github.com:443 间歇 `Connection was reset` / `Recv failure`）。
- 目标含 Windows 保留字符路径的仓库（如 `benchmark/results/{r[alternative]}:`）。

## 调用方式

```bash
# 克隆默认 26 仓清单（harness 17 + computer_use 9）到 third_party/
bash clone-third-party.sh --target /path/to/third_party

# 克隆自定义清单（manifest 每行: <分类> <owner/repo>）
bash clone-third-party.sh --target /path/to/third_party --manifest my_repos.txt
```

## 韧性协议（从生产事故提炼，必须遵守）

> 下列任何一条被违反，克隆都会陷入**静默 0 进度死循环**，且 `git` 实际根本没执行。

1. **绝不用 `rm -rf` 清理失败目录**——本机 `rm` 被 `genie-trash` 拦截（吞输出 + exit 1），
   导致目录没删、下次 `git clone` 进已存在非空目录立即失败、整轮空转。
   ✅ 改用 `mv` 把失败/临时目录挪到 `_garbage_<TS>/`（原子重命名，genie 不拦截）。

2. **绝不让 git 输出重定向到一个不可写/只读属性的共享日志**——重定向目标不可写时，
   bash **直接不执行 git**，每个仓库只打 `[RETRY]` 静默失败。
   ✅ git 输出写入**每仓独立日志** `third_party/_logs/<cat>_<name>.log`；
   主日志每次运行新建时间戳文件（自建自拥 → 可写）。

3. **`http.lowSpeedTime` 必须小（=30），绝不能 600**——`lowSpeedLimit=1 -c lowSpeedTime=600`
   要求「速度 <1 字节/秒持续 600 秒才中止」。弱网下连接常**假死**（已连但无数据）而非
   干净拒绝，于是 git 每次卡 ~10 分钟才超时，重试循环根本转不动。
   ✅ `http.lowSpeedLimit=1 -c http.lowSpeedTime=30`：假死 30s 即中止，重试循环真正转动。

4. **用 `--no-checkout` + `git archive HEAD | tar -x` 替代 `git clone` 直接 checkout**——
   弱网在 checkout 收尾阶段会锁/中断，导致目录回滚为空、`HEAD` 无效。
   ✅ 先只取对象（`--no-checkout`），再 `git -C <tmp> archive HEAD | tar -x -C <tmp>` 一次性提取工作树。

5. **同名仓库加所有者后缀**——`deepseek-harness`(deepseek-ai) 与 `deepseek-harness`(HenryZ838978)
   同名会互相覆盖。✅ 目标目录名用 `<repo>-<owner>` 或 `<owner>__<repo>` 区分。

6. **Windows 非法路径：逐路径 `git checkout HEAD -- <path>`**，不能用 sparse-checkout——
   git for Windows 在 checkout 时会遍历整棵树校验路径，sparse 过滤**之前**就报非法字符错。
   ✅ 对顶层文件/目录逐个 checkout；含非法字符的子目录（如 `benchmark/results/`）整体跳过，
   git 对象完整，Linux/macOS 下可正常 checkout。

7. **外层自愈循环**：网络窗口一开就自动续上，无需人工重拉。
   ✅ `while` 已集齐全部仓库（上限 40 阶段，阶段间等 45s）持续重试；每仓 3 次内重试 +
   网络预检（`curl -sI --max-time 10 github.com`，不可达则跳过该仓、不消耗重试配额）。

## 验证落盘

每个仓库落盘后必须满足：`[ -d <dest>/.git ] && git -C <dest> rev-parse HEAD` 有效 &&
`git -C <dest> ls-files | wc -l > 0`。成功仓库记入 `<target>/.cloned_ok`（每行 `cat/name`）。

## 与 Marguerite 融合

克隆完成后，调用 `explore.sh`（统一勘探产出能力清单）与 `security_audit.sh`
（基于 skill-vetter 的 RED FLAGS 协议做供应链审查，须排除测试/文档以消误报），再按
`core/src/harness/distill.rs` 的 `UPSTREAM_REPOS` 映射把能力蒸馏进 `core/src/harness`
与 `core/src/tools/computer_use`（三层：屏幕感知 / 键鼠控制 / GUI 定位）。
