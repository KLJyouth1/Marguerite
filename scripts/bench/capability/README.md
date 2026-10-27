# Capability Bench 60（能力评测集）

这是 W4.7 的**离线路由评测集**：60 条确定性用例，用来回答一个具体问题——
给定「用户意图 + 当前工具面」，路由函数会不会把它送到正确的**能力族**、
会不会用错工具（尤其是「本应结构化、却写脚本替代」）。

用例定义在 `core/src/agent/capability_bench.rs`（`pub fn tasks()`，恰好 60 条，
常量断言钉住数量）。本目录只放运行说明，不放数据副本。

## 覆盖分布（60 条）

| 能力族 | 条数 | 说明 |
|---|---|---|
| `file` | 10 | 读/写/列目录/grep/补丁/重构，禁止用 `python`/`node` 脚本替代 |
| `shell` | 5 | 正例：构建/测试/git/依赖安装，本来就用 shell |
| `browser` | 10 | 网页导航/表单/登录/抓取，禁止 `curl`/`selenium`/`playwright` 脚本 |
| `desktop.uia` | 7 | 记事本/计算器/窗口/按钮/输入框/菜单/弹窗，禁止 `pyautogui` 类脚本 |
| `desktop.pixel` | 3 | 游戏/画布/截图/拖拽，走 `computer_use` |
| `memory` | 5 | 记住/偏好/上次/习惯，禁止写进脚本或临时文件 |
| `kb` | 5 | 知识库检索/灌入/手册/规范 |
| `skill` | 5 | 技能/流程/最佳实践 |
| `mcp` | 5 | 外部系统/数据库/工单（工具名是 sanitize 后的动态外部名，样例仅作代表性夹具） |
| `cron` | 3 | 定时/每天/每周/周期，禁止 `sleep`/`while` 轮询脚本 |
| `team` | 2 | 并行/分工（模型侧无工具，入口在编排层） |

其中 **55 条是负例**：`forbidden_tools` 含 `shell_exec` / `code_exec`。
只有 `shell` 族 5 条正例不设禁止工具。

## 离线运行（不需要 key，可进 CI）

```bash
cargo test -p marguerite-core --lib capability_bench
```

测试覆盖：60 条唯一 id、11 族全覆盖、负例不少于 12 条；
oracle 假路由 60/60 通过；「全部路由到 shell」的假路由必须触发
`forbidden_hits > 0`；JSON 与 Markdown 两次输出逐字节相同。

如果要评测**真实的路由函数**，用 `evaluate` 注入即可（不需要网络）：

```rust
use marguerite_core::agent::capability_bench::{evaluate, tasks};
use marguerite_core::agent::capability_cards::route;

// available = 本轮真实 function schema（ToolDef）
let report = evaluate(route, &available);
println!("{}", report.render_markdown());
println!("{}", serde_json::to_string_pretty(&report).unwrap());
```

⚠️ 两个必须如实接受的限制：

1. **`mcp` 与 `team` 两族共 7 条，用当前 `capability_cards::route` 必然不通过**——
   这两张能力卡没有模型侧工具（`tools: &[]`），`route()` 按设计跳过它们。
   报告会逐条列出失败原因；这是「已知缺口」，不是评分器 bug。
2. `tool_visible_at_first_turn` 只统计「首轮工具面里能否看到期望工具」。
   看不到属于工具面问题（W4.2/W4.3），会被单独计数，不会被静默算成路由失败。

## 在线端到端评测（**手动，需要真实模型 key，本模块未实现**）

离线评测只证明**路由纯函数**的确定性，不证明真实模型会选结构化能力。
端到端评测需要真实/本地 mock 模型，明确**不在 CI**、也不在本模块内：

1. 用隔离网关启动（`MARGUERITE_CONFIG_DIR` + `LOCALAPPDATA` 指向临时目录；
   Windows 上 `USERPROFILE`/`HOME` 不生效，见 `AGENTS.md` §4）。
2. 配置真实模型 provider 与 API key（或本地 mock LLM）。
3. 逐条把 `tasks()` 的 `intent` 作为用户消息发送，记录：
   首个工具调用名、是否出现「写脚本」、是否触发纠偏、任务是否完成。
4. 把结果写成 `artifacts/capability-bench-<date>.json`（该目录被 gitignore，不随仓库发布），
   并在报告里注明模型名、日期、失败项原文。

**诚实边界**：截至本文件落地，**没有跑过真实模型的 60 条在线基准**。
任何「结构化能力首选率 ≥ 80% / 脚本替代率 ≤ 10%」的数字都必须来自一次真实运行后才能写进报告；
不得把离线 oracle 的 60/60 当成模型能力证明。
