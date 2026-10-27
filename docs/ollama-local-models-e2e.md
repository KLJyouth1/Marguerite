# Ollama 本地模型端到端闭环验证（隔离网关 debug）

日期：2026-10-20　执行方：独立 QA（非实现者）　范围：仅隔离网关（debug，端口 18900），未碰常驻桌面端。

## 环境与隔离方式

- 二进制：`D:\Downloads\Marguerite\target\debug\marguerite-core.exe`（本次 `cargo build -p marguerite-core --bin marguerite-core` 后）。
- 隔离目录（三个 env 成对设置，避免复用真实用户 sidecar/config）：
  - `MARGUERITE_CONFIG_DIR = MARGUERITE_DATA_DIR = D:\Downloads\Marguerite\.tmp\ollama-e2e`
  - `MARGUERITE_WEBUI_DIST = D:\Downloads\Marguerite\webui\dist`
- 网关端口 18900；token `hi2_…JExw`（隔离目录 `gateway-token`）。
- Ollama：127.0.0.1:11434，在线 16 个模型；三个自训模型 omnipilot:latest / omnipilot17:latest / omnipilot16c:latest。
- WS 调用：浏览器 bu 平面内自握手（connect.challenge → connect HelloOk → req/res），`window.__call(method,params)` 封装；事件流落盘 `events*.json`。

## 逐项结果

### 1. 检测 `models.local.ollama` —— PASS
- 返回 `online:true`、`base_url:"http://localhost:11434"`、**models 数 = 16**。
- 含 omnipilot:latest / omnipilot17:latest / omnipilot16c:latest。
- omnipilot17 字段核对：`family=qwen2`（报告称 qwen）、`quantization=Q4_K_M`、`size_gb≈0.92`、`digest` 前缀 `421e1507f821…`。

### 2. 可用-部署 `models.local.ollama.deploy` —— PASS
- `{model:"omnipilot17:latest", activate:true}` → `{ok:true, deployed:true, activated:true, profile_id:"ollama-omnipilot17-latest", provider:"ollama"}`。
- 隔离 `models.json`：该 profile `kind=local`、`base_url="http://localhost:11434"`（根地址，无 `/v1`）。
- 隔离 `config.yaml` model 段三要素镜像：`provider=ollama` / `model_name=omnipilot17:latest` / `base_url=http://localhost:11434`。
- 后再 deploy omnipilot16c activate=true → `profile_id="ollama-omnipilot16c-latest"`，active 切到它。

### 3. 可用-真实推理（chat.send）—— PASS
- chat.send 异步回 `accepted+taskId`，真回文走 WS 事件流（`session.event.usage` 给用量、`session.event.done` 给正文）。
- omnipilot17：`usage.model="omnipilot17:latest"`，`prompt=2001 / completion=57 / total=2058`，约 3.5 s。回文：
  > 我是基于Llama-2语言模型开发的AI工具。我的目标是通过自动化帮助用户提高效率和生产力。我能够执行多种任务，包括但不限于代码编辑、文件操作、知识搜索和系统管理等。
- omnipilot16c：`usage.model="omnipilot16c:latest"`，`completion=30`，约 8 s。回文：
  > 您好，我是一个用于自动任务处理和协作的工具。我可以帮助您执行各种操作，并根据您的需求提供所需信息和资源。
- 判定：非离线回显、非云报错（usage 事件 model 字段确证打到本地 Ollama）。

### 4. 可用-工具调用桥接（P2-B）—— PASS（证据为间接但强）
- 任务：“请用 list_dir 工具列出当前工作目录下的文件”。
- 证据链（同一 turn 两段 usage）：
  1. 第一段 `completion_tokens=21`、`prompt=2482` —— 模型输出工具调用；
  2. 第二段 `completion_tokens=486`、`prompt=2947` —— prompt 净增 **+465 token**（工具结果被注入上下文）；
  3. done 正文列出隔离目录里**真实存在**的文件名：`gw.log / gw-err.log / gw-pid.txt / gateway-token / sidecar.json / pet_state.json / state.json / coll.txt / kicked*.txt / config.yaml / models.json / agents.json / files.txt …`。这些文件名是本次测试期间才创建的临时产物，1.5B 模型不可能编造。
- 诚实边界：WS 流中未观察到独立命名的 `bridge_content_action` / `tool.executed` 事件；但「prompt +465 注入 + 回文出现真实文件名」即证明 list_dir 真执行并回喂。未改 prompt 工程硬凑。

### 5. 持久化（重启后 active 仍在）—— PASS
- 关停：侧车 `POST /api/system/shutdown?token=…` → `202 {"status":"accepted"}`；网关 pid 终止。
- 用**同一隔离目录、同一端口 18900、同一组 env** 重启，**不重新 deploy**。
- 重启后读盘：`models.json active="ollama-omnipilot16c-latest"`；`config.yaml` 三要素仍为 ollama / omnipilot16c:latest / 根地址。
- 重启后直接发一条 chat：`usage.model="omnipilot16c:latest"`（completion=18），`router_decision.routed_model="omnipilot16c:latest"`，回文“我是一个语言模型助手，用于提供信息、解答问题和协助完成任务。”——确证仍走本地 omnipilot。

### 6. 重复枚举稳定性 —— PASS
- 连调 3 次 `models.local.ollama`：三次均 `online:true / 16 models / 含三个 omnipilot`，结果一致。

## 发现的问题（非本次功能阻塞，供跟踪）

- **P1（数据卫生，既存）**：隔离 `models.json` 中某**内置** profile 的 label 为 GBK 乱码且引号未闭合：
  `"label": "OpenAI 鍏煎锛堣嚜瀹氫箟锛?` —— 导致严格 JSON 解析器（PowerShell `ConvertFrom-Json`）整文件解析失败（报错位置 3087）。
  该问题在本次 ollama deploy 之前的种子档案里就存在（与新 RPC 无关），但会让任何按 JSON 严格解析 models.json 的工具/客户端读不动整份文件。建议修内置种子 label 的编码与引号闭合。

## 收尾与环境状态

- 隔离网关 18900 已停（侧关 202，网关 pid 终止，端口已释放，复测 `/control/` 不可达）；临时浏览器标签已关。
- ⚠️ **需要你知悉（非我所致）**：本任务全程我只 `Stop-Process` 了两个我自己的隔离 debug 网关（首次 pid 34372、重启后 pid 32992，均监听 18900），从未 targeting 常驻进程。收尾复测时发现：常驻桌面网关 pid 21772 **已不在进程表**，其 Python 侧车 `pid 40284`（`python -m opensquilla.cli.main gateway run --port 64717`，parent=21772）成为孤儿进程仍在跑。也就是说桌面 Rust 网关在我收尾前已自行退出（或被外部关闭）。我没有重启它，也没有清理那个孤儿侧车——这属于你侧桌面端的既有状态，请你确认。

## 总体结论

**PASS（go）**。两个新后端 WS RPC（检测 + 部署）在真实隔离网关上完成端到端闭环：检测到 16 个本地模型、部署落盘并镜像 config、两个自训模型均真实推理（usage 事件 model 字段确证）、工具调用经桥接真执行了 list_dir、重启后 active 与配置持久化且仍走本地推理、重复枚举稳定。唯一附带发现是既存内置 label 的 JSON 编码/引号问题（不阻塞本功能）。
