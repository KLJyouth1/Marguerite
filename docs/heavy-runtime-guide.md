# 重型运行时接入指南（ASR / 本地量化模型）

> 原则：Marguerite **不预下载、不预置**任何重型模型（D-03）。本指南只提供
> 官方下载源与「按用户机器配置」的选型指引；模型由用户自行下载/安装后配置启用。

## 1. 音频理解（ASR / Whisper）

能力：`vis.audio`（纯叙事 → 用户安装后认证为已实装）。

### 下载源
- whisper.cpp: https://github.com/ggml-org/whisper.cpp （提供 ggml 官方模型权重下载脚本）
- faster-whisper: https://github.com/SYSTRAN/faster-whisper
- HuggingFace openai/whisper-{tiny,base,small,medium,large-v3}: https://huggingface.co/openai

### 按机器配置选型
| 机器 | 建议模型 | 说明 |
|---|---|---|
| CPU 内存 4GB | whisper-tiny | 实时性优先 |
| CPU 内存 8GB | whisper-base / small | 中文质量与速度平衡 |
| CPU 内存 16GB | whisper-medium | 准确性优先 |
| GPU 显存 8GB | whisper-large-v3 | CUDA / Metal 加速 |

### 启用步骤（用户自行下载）
1. 下载模型到 `$1marguerite/models/whisper/`（或任意目录）
2. 配置 `config.yaml` 的 `runtime.asr.engine`（后续版本提供）
3. `marguerite doctor` 应显示 `audio_runtime` 状态为 `ready`

## 2. 本地量化模型（离线推理）

能力：`model.local`（纯叙事 → 用户安装后可经 `model add` 接入）。

### 下载源
- llama.cpp GGUF: https://github.com/ggml-org/llama.cpp ；模型仓库见 https://huggingface.co/TheBloke
- Ollama（一键安装）：https://ollama.com/library

### 按机器配置选型（Q4_K_M 起步）
| 机器 | 建议模型 | 说明 |
|---|---|---|
| CPU 内存 8GB | 7B Q4_K_M | 通用编码 |
| CPU 内存 16GB | 13B Q4_K_M | 质量提升 |
| GPU 显存 8GB | 13B~30B Q4_K_M | CUDA 加速 |

### 启用步骤
Ollama 方式：
```bash
ollama pull qwen2.5:7b   # 或 llama3.1:8b 等
marguerite model add --id local-llama --base-url http://localhost:11434/v1 --provider ollama --kind local
marguerite model use local-llama
```

## 3. 状态查询
- CLI：`marguerite doctor`（含 `audio_runtime` / `local_model_runtime` 两行 + 下载源/选型提示）
- 网关：`runtime.status` RPC（WebUI 能力健康度横幅可直接消费）
- 能力清单：`marguerite capabilities`（`vis.audio` / `model.local` 保持 `纯叙事`，安装后才翻转）
