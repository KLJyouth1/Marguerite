<!-- 本文件原为 AGENTS.md §7，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
## 7. 第三方集成层（third_party/）

> 🔴 **`/third_party/` 被 `.gitignore` 整个忽略（实测 `git ls-files third_party` = 0），所以它不在任何 clone 里。**
> 本机约 **121 GB**：`external/` 101.6 GB / 230,560 文件、`local/` 18.9 GB、`harness/` 705 MB、`computer_use/` 461 MB。
> 忽略规则本身是对的 —— 那是上游仓库快照，不是本项目源码。
> **但 §7.1 / §7.2 描述的那些「本项目产出」的小文件（17 个清单 + 5 个技能 + 校验脚本）也埋在同一个目录里**，
> 于是**本节列出的产物默认不随仓库发布**：新人 clone 下来一个都看不到。
> 是否把它们移出 `third_party/`（例如把 5 个技能放进**已发布**的根 `skills/`）是**待决策项**，本轮未做 ——
> `.gitignore` 取反规则很容易连带把 GB 级快照放进来，风险高于收益。

`third_party/` 是**参考 + 适配层**，不编译进 cargo workspace。`third_party/manifest.json` 记录 **28 个**二开仓库
（**18 harness + 10 computer_use**，全部 `status: cloned`；实测 `counts` 与 `repos` 列表自洽，均为 28/18/10。
⚠️ 本节此前写「27 个（18 + 9）」—— 错的，已按实测更正）。价值通过两条路径进入 Marguerite：

1. **模式 → Rust 实现**：`core/src/tools/computer_use.rs`（`ScreenPerception` 截图 / `InputControl` 键鼠 / `GuiGrounding` 视觉闭环）。**这一项确实随仓库发布**（在 `core/` 内且已被跟踪）。
2. **能力 → 清单 / 适配器**：harness 类项目以声明式 `PluginManifest` 接入既有插件系统（`core/src/plugins/manifest.rs`），computer use 类以外部 mcp/视觉适配器接入。**这一项的清单文件不随仓库发布**（见上面的红框）。

### 7.1 插件清单（harness → PluginManifest）

- 位置：`third_party/plugin-examples/*.plugin.json`，**实测 17 个**（⚠️ 本节此前写 18 —— 错的）。
  17 个**全部可解析且满足必填字段**（`name`/`version`/`description`）。
- ⚠️ 覆盖缺口：`manifest.json` 声明 **18** 个 harness 仓，而清单只有 **17** 个 → **有 1 个 harness 仓没有对应清单**。
  这属于「完整性缺口」而不是自相矛盾，校验脚本把它列在 REPORT 段（见 §7.3），不会误报成 FAIL。
- Schema：`core/src/plugins/manifest.rs::PluginManifest`（必填 `name`/`version`/`description`；可选 `capabilities`/`hooks`/`slash_commands`/`subagents`/`skills`/`dependencies`）。
- 接线：`PluginManifest::from_json(&text)` → `register_manifests(&mut PluginRegistry, &[manifest])`；`build_hook_registry(...)` 把 `hooks[].handler` 注入 L1/L2/L3 护栏。
- 索引与逐仓映射：`third_party/harness-manifests-index.md` + `third_party/harness/HARNESS_BRIEF.md`（同样不随仓库发布）。

### 7.2 JIT 工程纪律技能包（Skills）

- 位置：`third_party/skills/*.yaml`，**5 个**：`engineering-discipline` / `tdd` / `systematic-debugging` / `verification-before-completion` / `write-plan-first`。
- ⚠️ **R20 之前，其中 4 个是 0 字节空文件**（只有 `write-plan-first.yaml` 有内容），
  而 `write-plan-first` 又声明依赖那个空的 `engineering-discipline` → **依赖不可解析**。
  本轮按同一 schema 补齐了 4 个，现在 **5/5 结构合法**且依赖链闭合（校验脚本已验证）。
- ⚠️ **这 5 个技能在产品的技能加载路径上取不到**：`SkillLoader::new(skills_dir).load_all()` 只扫描**顶层** `skills_dir`
  （`doctor` 实测解析到 `…\AppData\Local\Marguerite\skills`，24 个技能），而**已发布**的根 `skills/` 里
  **没有**这 5 个名字 —— 它们只存在于被忽略的 `third_party/skills/`。
- Schema：`core/src/skills/registry.rs::Skill`（必填 `name`/`description`/`version`/`prompt_template`；可选 `author`/`intent`/`tools`/`dependencies`）。
- 来源：HARNESS_BRIEF 综合模式 #7（LeAgent Agent Skills v1.0 + superpowers-reasonix）。

### 7.3 校验（**本地检查 —— 不是 CI 门禁**）

```bash
python third_party/validate_integration.py   # 0 = 无不变式违例；1 = 有（REPORT 段仍必须阅读）
```

> ⚠️ **两点必须说清，因为本节此前两点都写错了**：
> 1. **它曾经是一个 0 字节的空文件**，而本节把它描述成「CI 门禁」并罗列了它会检查什么。
>    空脚本 `exit 0`，于是「永远绿、什么都没查」—— 正是 §8 明令禁止的**假门禁**。R20 已实现为真。
> 2. **它不可能成为 CI 门禁**：`/third_party/` 被忽略，脚本与它读的数据都不随仓库发布。
>    因此它**没有**接入 `verify.sh`，只是本机自检工具。

实现后的两级严重度（刻意分开，避免把「绿」误读成「完整」）：
- **FAIL（exit 1）**：不变式被破坏 —— `manifest.json` 的 `counts` 与自身 `repos` 不一致；
  清单不可解析或缺必填字段；**非空**技能文件不可解析或缺必填字段；技能声明的依赖无人提供。
- **REPORT（不影响 exit code）**：完整性缺口 —— 0 字节技能占位文件；声明了 harness 仓却没有清单。

**R20 实测**：`PASS`（0 条 FAIL）+ **1 条 REPORT**（18 个 harness 仓 vs 17 个清单）。
**已做反向验证**：删掉清单必填字段 → FAIL/exit 1；清空一个技能 → REPORT 增加而 exit 仍 0；
把依赖指向不存在的技能 → FAIL/exit 1；三次都正确复原。

⚠️ 本节此前写「当前环境未装 PyYAML，技能 YAML 用启发式校验」—— **已过时**：本机实测 PyYAML **6.0.2** 可用，脚本走严格解析。

### 7.4 升级路线（优先级，详见 INTEGRATION.md §5.4）

- **P0（已落地）**：JIT 工程纪律技能包 + harness 清单 + computer_use 三层重构 + 校验脚本。
  ⚠️ 「已落地」需要限定：**只有 computer_use 三层重构真的随仓库发布**（在 `core/` 内且已被跟踪）；
  技能包与清单都在被忽略的 `third_party/` 里 —— clone 拿不到，技能也加载不进产品（见 §7.2）。
- **P1**：扩展 `PluginManifest`（dependencies / 能力级 model）；自演化闭环（`harness/distill.rs` + `automation/cron.rs`）；fleet 模型路由（`multi_strategy.rs`）；P2P 团队（`agent/team.rs`）。
- **P2**：memory + distill 自动蒸馏；`tool_cycle` + `stream_loop` 的 DAG 并发 + 断点续跑；`events` + `bus` + tui/workbench 结构化 model↔UI 协议。

