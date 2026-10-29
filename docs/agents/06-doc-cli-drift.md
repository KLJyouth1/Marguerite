<!-- 本文件原为 AGENTS.md §6，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
## 6. 文档/代码 CLI 漂移校验（CI 门禁）

`scripts/check-doc-cli-drift.py` 自动校验 `docs/*.md` 里出现的 `marguerite <sub>` / `marguerite <sub> <verb>` 是否都有真实 CLI 实现，防止手册漂离代码（本次优化即修复了 `mcp connect` 虚构、「`gui/` 是成员」两处漂移）。

- **Tier 1（FAIL）**：文档里的顶层子命令必须在 `argv.rs::SUBCOMMANDS` 或 `program.rs::route_table.register(...)` 中注册。
- **Tier 2（FAIL）**：对会派发嵌套动词的 handler（如 `mcp` 读 `sub_args`），文档里的嵌套动词必须在该 handler 体内作为字符串分支出现。

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

实现要点：脚本只扫描 fenced code block 与 inline `code`（忽略散文）；flag（`--health` 等）与非派发参数（session id / 路径）不检查，避免误报。根因是 CLAUDE/AGENTS 类文档与代码易漂移，本脚本把“对账”固化成可重复的门禁。

