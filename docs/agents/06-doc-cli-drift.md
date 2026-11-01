<!-- 本文件原为 AGENTS.md §6，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
## 6. 文档/代码 CLI 漂移校验（CI 门禁）

`scripts/check-doc-cli-drift.py` 自动校验文档里出现的 `marguerite <sub>` / `marguerite <sub> <verb>` 是否都有真实 CLI 实现，并核对「路由面」三处是否同一份，防止手册漂离代码（首轮修复了 `mcp connect` 虚构、「`gui/` 是成员」两处漂移；第二轮修复了门禁自身的覆盖盲区，见 Tier 0）。

- **Tier 0（FAIL）**：路由面必须自洽且每处声明都要说真话 ——①`argv.rs::SUBCOMMANDS` 与 `program.rs` 的 `route_table.register(...)` 必须是同一集合（只加一处 = 半落地改动，历史上有 `export-diagnostics` 只进了一边）；②文档里任何 `权威子命令表（N 个` / `当前 **N 个路由**` 的 N 必须等于真实注册数；③`### 权威子命令表` 的表格必须**逐行等于**注册名——不许少一行、也不许多一行。
- **Tier 1（FAIL）**：文档里的顶层子命令必须在 `argv.rs::SUBCOMMANDS` 或 `program.rs::route_table.register(...)` 中注册。
- **Tier 2（FAIL）**：对会派发嵌套动词的 handler（如 `mcp` 读 `sub_args`），文档里的嵌套动词必须在该 handler 体内作为字符串分支出现。

**扫描范围（第二轮扩口；`docs/README.md` 同步说明）**：`docs/*.md` 跑 Tier 0+1+2；`docs/agents/*.md` 跑 Tier 0+1（分册会复述路由契约——19 与 20 的矛盾就住在这一册，但分册的代码块混着 `marguerite <sub>` 这类占位语法，强行 Tier 2 会系统性误报，而「会喊狼来了」的门禁最后都是被放宽而不是被修好）；`internal/` `planning/` `reports/` `research/` `uiux/` `archive/` `decisions/` `legacy/` `prd/` 整体排除（`legacy/`、`prd/` 是 AGENTS.md §1 引用的退役材料库，与 `archive/` 同类：列出不存在的目录是无操作，漏列一个则首跑就误红）；`plans/` `product/` `qa/` `design/` 同样排除 —— 它们是**带日期的时点工作文档**（整改路线图、PRD、测试计划、液态玻璃与溯源规格），2026-11-01 实测四册共 13 个 md、`marguerite <verb>` 出现 **0 次**，Tier 1 无处可校；排除它们是因为它们**不是 CLI 契约的所在地**，不是因为它们红。**任何未分类的 `docs/<子目录>/` 直接判红**，必须显式写进 `SCANNED_SUBDIRS` 或 `EXCLUDED_SUBDIRS` —— 旧的 `glob("*.md")` 正是靠「根本走不到那个目录」让矛盾活了几个月。（`assets/`、`qa-shots/` 不含 md，天然不进目录枚举；一旦有人往里放 `.md`，门禁会红并要求分类，这是有意的。）

```bash
python3 scripts/check-doc-cli-drift.py   # exit 0 = 无漂移；exit 1 = 漂移（打印明细）
```

CI 接入（`docs` job，checkout 之后的**六个**纯标准库检查 —— 都只需要 `python3`，不需要 setup-python）：

```yaml
- name: Doc/Code CLI drift check
  run: python3 scripts/check-doc-cli-drift.py
- name: Version consistency across the four front ends
  run: python3 scripts/check-version-sync.py
- name: Secret scan of the shipping file set
  run: python3 scripts/check-secrets.py
- name: Doc link check (first-party docs)
  run: python3 scripts/check-doc-links.py
- name: README claims vs verify.sh
  run: python3 scripts/check-readme-claims.py
- name: Website claims vs the shipping file set
  run: python3 scripts/check-website-claims.py
```

实现要点：脚本只扫描 fenced code block 与 inline `code`（忽略散文）；flag（`--health` 等）与非派发参数（session id / 路径）不检查，避免误报。子命令名统一用 `[a-z][a-z0-9_-]*` 匹配（**必须含连字符**——旧版写成 `[a-z][a-z0-9_]*`，`export-diagnostics` 会被截成 `export`，这正是 19/20 能长期共存而无人发现的技术原因）。路由集合只认 `route_table.register(\s*"<name>"` ——裸 `\bregister\(` 会把 `ToolRegistry`、插件容器等无关注册也扫进来，静默放大期望集合。Tier 0c 只在同一文件也声明了计数时才校验表格：是那句计数把表格从「分类列表」变成「完备性声明」，拿它去判普通分类列表会系统性地误报。根因是 CLAUDE/AGENTS 类文档与代码易漂移，本脚本把“对账”固化成可重复的门禁。

