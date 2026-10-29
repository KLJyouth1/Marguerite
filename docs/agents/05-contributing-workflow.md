<!-- 本文件原为 AGENTS.md §5，2026-10-29 由 scripts/split-agents-md.mjs 逐字节搬运；
     内容未删改，请勿在此加"摘要"，细节就该留在分册里。 -->
## 5. 贡献 workflow

1. 改动前先读 `docs/cli-reference.md`（CLI 契约）与 `docs/mcp.md` / `docs/development.md`，确保理解现状。
2. 改动 CLI 路由 / 子命令 → 同步更新 `argv.rs` 的 `SUBCOMMANDS`、`program.rs` 的路由注册与测试断言，并补 `docs/cli-reference.md` 对应小节。
3. 改动后运行 `bash scripts/verify.sh --clean --all`（发版加 `--release`），保证全绿、§2 的基线不回退；必要时更新本文件 §2 与 `README.md` 的基线数字。
4. **文档与代码冲突时以代码为权威，回改文档**——不要为了文档去虚构 CLI 动词（如 `mcp connect` 未实现就不应写进手册）；也不要为“凑齐文档示例”去新增未经验证的实现。
5. 改动 CLI 路由 / 文档示例后，运行 `python3 scripts/check-doc-cli-drift.py`，确保手册不再漂离代码（详见 §6）。

