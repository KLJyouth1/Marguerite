# sinohealth_skills_sdk、bizType 与 bizToken

本 Skill 使用 **sinohealth_skills_sdk**（`from sinohealth_skills_sdk import SkillsClient`）调用网关上的药品说明书能力。

## 依赖安装

```bash
pip3 install -U sinohealth_skills_sdk
```

## `SKILLS_BIZ_TYPE` 与 bizType

- 脚本在调用 `SkillsClient` **之前**将当前进程的 `SKILLS_BIZ_TYPE` 与客户端 `biz_type` **对齐**（与 `scripts/drug_instructions.py` 中的 `_BIZ_TYPE` 一致）。  
- 若环境中已有 `SKILLS_BIZ_TYPE`，会被覆盖为与脚本一致，以保证与网关侧约定相符。  
- **调用方无需**手动 `export SKILLS_BIZ_TYPE`。

## `SKILLS_BIZ_TOKEN`（仅环境变量）

- 必须由**宿主、Agent 或运行环境**注入；脚本从 `os.environ["SKILLS_BIZ_TOKEN"]` 读取，空则报错。  
- **禁止**将 token 写入本地文件或由本脚本持久化。

## 其它 SDK 环境变量

如 `SKILLS_TRACE_IDENTIFIER`、`SKILLS_TIMEOUT` 等仍以 SDK 官方文档为准。

## 网关路径与请求体

- `_SKILL_URL` 默认：`/dataset/drug`  
- body：`prompt`、`msgId`（如 `debug`）、`medBase`（`"药品"`）
