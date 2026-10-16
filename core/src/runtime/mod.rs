//! 重型运行时状态（诚实占位）
//!
//! 音频理解（ASR）与本地量化模型推理是两项纯叙事能力：本切片**不预下载任何模型**，
//! 只提供机器可校验的状态、下载源与「按用户机器配置选择」的指引，由用户自行下载
//! 并配置启用。任何未安装状态都如实上报 `bundled=false / status=not_installed`，
//! 绝不当做成「已可用」。

use serde::Serialize;

/// 一项重型运行时的可机器校验状态。
#[derive(Debug, Clone, Serialize)]
pub struct HeavyRuntime {
    /// 类别键（asr / local_llm）
    pub kind: &'static str,
    /// 中文展示名
    pub label: &'static str,
    /// 是否随包内置（当前全部 false——坚持 D-03「不预下载/不预置本地模型」）
    pub bundled: bool,
    /// 状态：not_installed / downloading / ready
    pub status: &'static str,
    /// 建议引擎
    pub engine: &'static str,
    /// 官方下载源（由用户自行下载，绝不代下）
    pub download: &'static str,
    /// 按用户机器配置（CPU/内存/显存）的选型指引
    pub guidance: &'static str,
}

/// 全量重型运行时状态清单（单一事实源，doctor / gateway runtime.status 共用）。
pub fn heavy_runtime_status() -> Vec<HeavyRuntime> {
    vec![
        HeavyRuntime {
            kind: "asr",
            label: "音频理解（ASR / Whisper）",
            bundled: false,
            status: "not_installed",
            engine: "whisper.cpp / faster-whisper",
            download: concat!(
                "https://github.com/ggml-org/whisper.cpp | ",
                "https://github.com/SYSTRAN/faster-whisper | ",
                "https://huggingface.co/openai/whisper-{tiny,base,small,medium,large-v3}"
            ),
            guidance: concat!(
                "CPU 内存 4GB→tiny；8GB→base/small；16GB→medium；GPU 显存 8GB→large-v3（CUDA/Metal）。",
                " 用户下载后放 ~/.marguerite/models/whisper/，配置 runtime.asr.engine 启用。"
            ),
        },
        HeavyRuntime {
            kind: "local_llm",
            label: "本地量化模型（离线推理）",
            bundled: false,
            status: "not_installed",
            engine: "llama.cpp（GGUF）/ Ollama",
            download: concat!(
                "https://github.com/ggml-org/llama.cpp | ",
                "https://ollama.com/library | ",
                "https://huggingface.co/TheBloke（Q4_K_M GGUF）"
            ),
            guidance: concat!(
                "Q4_K_M 起步：CPU 8GB→7B；16GB→13B；GPU 显存 8GB→13B/30B。",
                " 用户自行下载/安装后：marguerite model add --id local-llama",
                " --base-url http://localhost:11434/v1 --provider ollama --kind local。"
            ),
        },
    ]
}
