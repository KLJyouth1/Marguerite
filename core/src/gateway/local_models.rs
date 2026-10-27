//! 本地模型中心（Local Model Hub）— 2026-08「调研启示落地」
//!
//! 用户可在工作台内**聚合下载源 + 系统智能推荐 + 一键下载部署 / 启停**本地模型，
//! 并把本地模型**标记进模型路由**（kind=Local 档案，可 set_active 切换）。
//!
//! 核心设计（对齐调研结论）：
//! - **档案化 / 可热更新**：目录为数据表，部署台账持久化 `local_models.json`，运行时增删不重编译。
//! - **shipped gate 六要素**：每个目录项带 weights / license / eval / pricing / quantized / reproducible。
//! - **系统智能推荐**：按 OS / 内存 / 核心数 / GPU 分档（edge / laptop / workstation / server）
//!   结合意图（general / coding / reasoning / chat / multimodal）定向推荐。
//! - **本地/云标记**：deploy 后经 ModelSelector 注册 kind=Local 档案，路由可见、可切换主模型。
//! - **诚实边界**：真正下载/启动依赖 Ollama CLI 是否可用；不可用时如实上报并提供官方命令/URL，
//!   绝不假装「已就绪」。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::proc::SilentCommand;

use crate::persistence::config::Config;

// ─────────────────────────────────────────────────────────────────────────────
// 目录（data-driven，可热扩展；shipped gate 六要素）
// ─────────────────────────────────────────────────────────────────────────────

/// 目录项：一个可一键部署的本地模型（开放权重 / 量化 / 官方下载源）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModelEntry {
    pub id: String,
    pub label: String,
    pub family: String,
    /// 定向用途：general / coding / reasoning / chat / multimodal
    pub intent: String,
    /// Ollama 一键拉取标签（空 = 无 Ollama 版）
    pub ollama_tag: String,
    /// Hugging Face 仓库
    pub hf_repo: String,
    /// ModelScope 仓库
    pub msc_repo: String,
    /// 量化后约占用（GB）
    pub size_gb: f32,
    /// 最低内存（GB）—— Q4_K_M 起步经验值
    pub min_ram_gb: u32,
    /// 上下文窗口
    pub context: String,
    // ── shipped gate 六要素 ──
    pub weights: bool,
    pub license: String,
    pub eval: String,
    pub pricing: String,
    pub quantized: bool,
    pub reproducible: bool,
}

impl LocalModelEntry {
    /// 数据表构造器：12 个字段是 shipped-gate 目录的固有维度，允许超参。
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: &str,
        label: &str,
        family: &str,
        intent: &str,
        ollama_tag: &str,
        hf_repo: &str,
        msc_repo: &str,
        size_gb: f32,
        min_ram_gb: u32,
        context: &str,
        license: &str,
        eval: &str,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            family: family.into(),
            intent: intent.into(),
            ollama_tag: ollama_tag.into(),
            hf_repo: hf_repo.into(),
            msc_repo: msc_repo.into(),
            size_gb,
            min_ram_gb,
            context: context.into(),
            weights: true,
            license: license.into(),
            eval: eval.into(),
            pricing: "本地免费（自付电费）".into(),
            quantized: true,
            reproducible: true,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "family": self.family,
            "intent": self.intent,
            "ollama_tag": self.ollama_tag,
            "hf_repo": self.hf_repo,
            "msc_repo": self.msc_repo,
            "size_gb": self.size_gb,
            "min_ram_gb": self.min_ram_gb,
            "context": self.context,
            "shipped_gate": {
                "weights": self.weights,
                "license": self.license,
                "eval": self.eval,
                "pricing": self.pricing,
                "quantized": self.quantized,
                "reproducible": self.reproducible,
            },
        })
    }
}

/// 聚合下载源目录（可热扩展：追加条目即可，无需重编译）。
pub fn local_catalog() -> Vec<LocalModelEntry> {
    vec![
        // ── 通用 / 对话 ──
        LocalModelEntry::new(
            "qwen3.8-27b",
            "Qwen3.8-27B",
            "qwen",
            "general",
            "qwen3.8:27b",
            "Qwen/Qwen3.8-27B",
            "Qwen/Qwen3.8-27B",
            18.0,
            16,
            "128K",
            "Apache-2.0",
            "SWE-bench Pro 超越 Opus 4.6 Max；智能密度标杆",
        ),
        LocalModelEntry::new(
            "gemma4-31b",
            "Gemma 4 31B",
            "google",
            "general",
            "gemma4:31b",
            "google/gemma-4-31b-it",
            "google/gemma-4-31b-it",
            20.0,
            24,
            "128K",
            "Gemma License",
            "端侧最强开放模型；reasoning + agentic",
        ),
        LocalModelEntry::new(
            "deepseek-r1-distill-14b",
            "DeepSeek-R1-Distill 14B",
            "deepseek",
            "reasoning",
            "deepseek-r1:14b",
            "deepseek-ai/DeepSeek-R1-Distill-Qwen-14B",
            "deepseek-ai/DeepSeek-R1-Distill-Qwen-14B",
            9.0,
            16,
            "32K",
            "MIT",
            "AIME24 pass@1 69.7%；强推理小模型",
        ),
        // ── 编码 ──
        LocalModelEntry::new(
            "qwen2.5-coder-14b",
            "Qwen2.5-Coder 14B",
            "qwen",
            "coding",
            "qwen2.5-coder:14b",
            "Qwen/Qwen2.5-Coder-14B-Instruct",
            "Qwen/Qwen2.5-Coder-14B-Instruct",
            9.0,
            16,
            "32K",
            "Apache-2.0",
            "HumanEval 84.1；代码生成性价比之选",
        ),
        LocalModelEntry::new(
            "glm-4.5-air",
            "GLM-4.5-Air（106B MoE）",
            "glm",
            "coding",
            "",
            "zai-org/GLM-4.5-Air",
            "zai-org/GLM-4.5-Air",
            60.0,
            48,
            "128K",
            "MIT",
            "SWE-bench Verified 64.2；agentic 编码",
        ),
        // ── 轻量对话 / 边缘 ──
        LocalModelEntry::new(
            "qwen3-4b",
            "Qwen3-4B",
            "qwen",
            "chat",
            "qwen3:4b",
            "Qwen/Qwen3-4B",
            "Qwen/Qwen3-4B",
            2.6,
            8,
            "32K",
            "Apache-2.0",
            "4B 可匹敌 Qwen2.5-72B-Instruct（官方宣称）",
        ),
        LocalModelEntry::new(
            "llama3.2-3b",
            "Llama 3.2 3B",
            "meta",
            "chat",
            "llama3.2:3b",
            "meta-llama/Llama-3.2-3B-Instruct",
            "modelscope/Llama-3.2-3B-Instruct",
            2.0,
            8,
            "128K",
            "Llama 3.2 License",
            "端侧多模态；工具调用",
        ),
        // ── 多模态 ──
        LocalModelEntry::new(
            "qwen3-vl-8b",
            "Qwen3-VL-8B",
            "qwen",
            "multimodal",
            "qwen3-vl:8b",
            "Qwen/Qwen3-VL-8B",
            "Qwen/Qwen3-VL-8B",
            6.0,
            16,
            "32K",
            "Apache-2.0",
            "视觉/文档/图表理解",
        ),
        LocalModelEntry::new(
            "gemma4-12b",
            "Gemma 4 12B",
            "google",
            "multimodal",
            "gemma4:12b",
            "google/gemma-4-12b",
            "google/gemma-4-12b",
            8.0,
            16,
            "128K",
            "Gemma License",
            "统一 encoder-free 多模态",
        ),
        // ── 中国开源推理主力 ──
        LocalModelEntry::new(
            "qwen3-30b-a3b",
            "Qwen3-30B-A3B（MoE）",
            "qwen",
            "reasoning",
            "qwen3:30b-a3b-thinking",
            "Qwen/Qwen3-30B-A3B-Thinking-2507",
            "Qwen/Qwen3-30B-A3B-Thinking-2507",
            18.0,
            24,
            "256K",
            "Apache-2.0",
            "AIME25 85.0；MoE 仅激活 3B",
        ),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// 台账（local_models.json 持久化）
// ─────────────────────────────────────────────────────────────────────────────

/// 已纳入管理的本地模型条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedLocalModel {
    pub id: String,
    pub label: String,
    pub ollama_tag: String,
    /// not_installed / downloading / ready
    pub status: String,
    /// 是否已部署为模型路由档案（kind=Local）
    pub deployed: bool,
    /// 部署后的档案 id（models.json）
    pub profile_id: String,
    /// 运行开关（start/stop）
    pub started: bool,
    pub added_ms: u64,
}

/// 本地模型台账（持久化 `data_dir/local_models.json`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalModelStore {
    #[serde(default)]
    pub managed: BTreeMap<String, ManagedLocalModel>,
}

fn store_path() -> PathBuf {
    // 与 models.json 同目录（config_dir），保证测试覆盖与生产一致。
    Config::config_dir().join("local_models.json")
}

impl LocalModelStore {
    fn load() -> Self {
        match std::fs::read_to_string(store_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) {
        if let Some(dir) = store_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(store_path(), s);
        }
    }

    fn managed_json(&self) -> Vec<Value> {
        self.managed
            .values()
            .map(|m| {
                json!({
                    "id": m.id,
                    "label": m.label,
                    "ollama_tag": m.ollama_tag,
                    "status": m.status,
                    "deployed": m.deployed,
                    "profile_id": m.profile_id,
                    "started": m.started,
                    "added_ms": m.added_ms,
                })
            })
            .collect()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// 系统检测与智能推荐
// ─────────────────────────────────────────────────────────────────────────────

/// 内存分档：edge(<8G) / laptop(8-15G) / workstation(16-31G) / server(>=32G 或有 GPU)
fn tier_for(ram_gb: u32, has_gpu: bool) -> &'static str {
    if has_gpu || ram_gb >= 32 {
        "server"
    } else if ram_gb >= 16 {
        "workstation"
    } else if ram_gb >= 8 {
        "laptop"
    } else {
        "edge"
    }
}

/// 某档位推荐的最大参数规模（Q4_K_M 经验，约 0.6GB/10亿参数 + 上下文开销）。
fn max_billions_for(tier: &str) -> u32 {
    match tier {
        "edge" => 4,
        "laptop" => 14,
        "workstation" => 32,
        "server" => 106,
        _ => 14,
    }
}

/// 从目录里按「用途 + 档位」挑选模型；返回带匹配理由的候选。
fn recommend_models(intent: &str, ram_gb: u32, has_gpu: bool, os: &str) -> Value {
    let tier = tier_for(ram_gb, has_gpu);
    let max_b = max_billions_for(tier);
    let catalog = local_catalog();

    // 参数规模从 id/标签启发式解析：取**最大**整数段，末尾为 T 则 ×1000。
    // （Qwen 命名 qwen3.8-27b 首段是 "3"，必须取 max=27 才正确。）
    let billions = |e: &LocalModelEntry| -> u32 {
        let joined = format!("{} {}", e.id, e.label);
        let chars: Vec<char> = joined.chars().collect();
        let mut best = 0u32;
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if let Ok(n) = chars[start..i].iter().collect::<String>().parse::<u32>() {
                    // 若数字后紧跟 't'/'T'（如 2.4T），按千亿折算
                    let mult = if chars.get(i).is_some_and(|c| matches!(c, 't' | 'T')) {
                        1000
                    } else {
                        1
                    };
                    best = best.max(n * mult);
                }
            } else {
                i += 1;
            }
        }
        if best == 0 {
            u32::MAX
        } else {
            best
        }
    };

    // 按用途打分：intent 命中 +20；通用兜底 +6；规模适配 +10；超出档位**硬过滤**。
    // 内存惩罚用 u32 截断（`min_ram.saturating_sub(ram)` 为 0 当 RAM 充足），
    // 避免 i64::saturating_sub 对负数返回负值 → 误把「内存余量」加成导致小模型反超推理模型。
    let mut scored: Vec<(i64, Value)> = catalog
        .iter()
        .filter(|e| billions(e) <= max_b)
        .map(|e| {
            let mut score: i64 = 0;
            if e.intent == intent {
                score += 20;
            }
            if e.intent == "general" {
                score += 6; // 通用兜底
            }
            score += 10; // 规模适配
            score -= e.min_ram_gb.saturating_sub(ram_gb) as i64 * 2;
            (score, e.to_json())
        })
        .collect();
    scored.sort_by_key(|x| std::cmp::Reverse(x.0));

    let picks: Vec<Value> = scored
        .into_iter()
        .take(4)
        .map(|(s, mut v)| {
            v["match_score"] = json!(s.max(0));
            v
        })
        .collect();

    json!({
        "tier": tier,
        "max_billions": max_b,
        "os": os,
        "ram_gb": ram_gb,
        "gpu": has_gpu,
        "reason": match tier {
            "edge" => "内存 <8G：适合 1.5B~4B 量化模型，优先对话/轻量编码",
            "laptop" => "内存 8-15G：适合 7B~14B Q4 量化，兼顾质量与速度",
            "workstation" => "内存 16-31G：适合 14B~32B，可跑推理/编码/多模态",
            _ => "≥32G 或带 GPU：可跑 32B+ MoE 与旗舰开源模型",
        },
        "picks": picks,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 真实宿主系统探测（运行 Marguerite 的机器，非浏览器 navigator 猜值）
// ─────────────────────────────────────────────────────────────────────────────

/// 宿主真实系统规格。驱动「根据用户系统配置智能推荐」，取代前端 browser 近似。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HostSpec {
    pub os: String,
    pub ram_gb: u32,
    pub cores: u32,
    /// 是否检测到可用于本地推理/训练的**独立** GPU（仅 nvidia-smi / rocm-smi 命中）
    pub gpu: bool,
    /// 探测到的 GPU 名（优先加速器型号；无加速器时为系统枚举的显卡名，未知则空）
    pub gpu_name: String,
    /// GPU 类型：discrete / integrated / none。仅用于展示，不参与推荐分档。
    #[serde(default)]
    pub gpu_kind: String,
    /// 数据来源标记（恒为 "host"）
    pub source: String,
}

/// 跨平台真实物理内存（字节）。兜底 16GB 保守值。
fn detect_ram_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        return kb * 1024;
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl")
            .silent()
            .args(["-n", "hw.memsize"])
            .output()
        {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Ok(b) = s.trim().parse::<u64>() {
                        return b;
                    }
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // 依赖无关：通过 PowerShell CIM 取真实物理内存（字节）。
        if let Ok(out) = std::process::Command::new("powershell")
            .silent()
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ])
            .output()
        {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Ok(b) = s.trim().parse::<u64>() {
                        return b;
                    }
                }
            }
        }
    }
    16u64 * 1024 * 1024 * 1024
}

/// 逻辑核心数（stable since Rust 1.59，零依赖）。
fn detect_cores() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4)
}

/// GPU 探测结果。
///
/// 语义边界（关键）：`accelerator` 只由 nvidia-smi / rocm-smi 命中决定；
/// `name` 在没有加速器时会回退到系统枚举的显卡名（可能是核显），仅用于展示，
/// **不会**让 `gpu` 变 true，也不会影响 [`tier_for`] 的推荐档位。
struct GpuProbe {
    accelerator: bool,
    name: String,
    kind: &'static str,
}

/// 由「加速器探测结果 + 系统枚举显卡名」推导最终结果（纯函数，便于单测）。
fn resolve_gpu(smi_name: Option<String>, display_name: Option<String>) -> GpuProbe {
    match smi_name {
        Some(name) => GpuProbe {
            accelerator: true,
            name,
            kind: "discrete",
        },
        None => match display_name {
            Some(name) => GpuProbe {
                accelerator: false,
                name,
                kind: "integrated",
            },
            None => GpuProbe {
                accelerator: false,
                name: String::new(),
                kind: "none",
            },
        },
    }
}

/// 探测 GPU：加速器（nvidia-smi / rocm-smi）优先，未命中时再枚举系统显卡名兜底。
fn detect_gpu() -> GpuProbe {
    let smi = probe_gpu("nvidia-smi", &["--query-gpu=name", "--format=csv,noheader"])
        .or_else(probe_rocm);
    let display = if smi.is_none() {
        detect_display_adapter()
    } else {
        None
    };
    resolve_gpu(smi, display)
}

fn probe_gpu(exe: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(exe).silent().args(args).output().ok()?;
    if out.status.success() {
        let line = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()?
            .trim()
            .to_string();
        if !line.is_empty() {
            return Some(line);
        }
    }
    None
}

/// rocm-smi `--showproductname` 的输出首行常是横幅/表头，必须解析而非直接取首行。
fn probe_rocm() -> Option<String> {
    let out = std::process::Command::new("rocm-smi")
        .silent()
        .args(["--showproductname"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_rocm_product_name(&String::from_utf8_lossy(&out.stdout))
}

/// 从 `rocm-smi --showproductname` 输出中解析产品型号（纯函数）。
///
/// 跳过空行、纯分隔线、横幅与表头；`GPU[0] : Card series: AMD Radeon ...`
/// 只取最后一个冒号后的值；多卡取第一张有型号的。
fn parse_rocm_product_name(raw: &str) -> Option<String> {
    let mut bare: Option<String> = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // 纯分隔线：只由 = - + 组成
        if line.chars().all(|c| matches!(c, '=' | '-' | '+')) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        // 横幅 / 日志边界行
        if lower.contains("rocm system management")
            || lower.contains("rocm smi")
            || lower.contains("product info")
        {
            continue;
        }
        let Some((_, value)) = line.rsplit_once(':') else {
            // 少数版本直接输出裸型号行：保守收集，优先 Card series
            if bare.is_none() && !lower.contains("gpu") && !lower.contains("card") {
                bare = Some(line.to_string());
            }
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue; // 表头行，如 `GPU[0] : Card series:`
        }
        if lower.contains("card series") {
            return Some(value.to_string());
        }
        // Card model / vendor / SKU 是元数据，不是产品型号
        if lower.contains("card model")
            || lower.contains("card vendor")
            || lower.contains("card sku")
        {
            continue;
        }
        // 旧版无标签行：`GPU[0] : AMD Radeon ...`
        if lower.starts_with("gpu[") {
            return Some(value.to_string());
        }
    }
    bare
}

/// 解析 `Win32_VideoController` 的 `Name` 列表（一行一个，纯函数）。
///
/// 跳过 wmic 表头与「无实例」提示；微软基础/远程显示适配器不算真实型号，
/// 只有这类适配器时视为未探测到（返回 None，交由上层显示「未检测到」）。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_video_controller_names(raw: &str) -> Option<String> {
    let mut first: Option<String> = None;
    for line in raw.lines() {
        let name = line.trim();
        if name.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if lower == "name" || lower.contains("no instance(s) available") {
            continue;
        }
        if lower.contains("microsoft basic display adapter")
            || lower.contains("microsoft remote display adapter")
        {
            continue;
        }
        if first.is_none() {
            first = Some(name.to_string());
        }
    }
    first
}

/// 系统接口枚举显卡名（兜底，仅用于填充 gpu_name）。
///
/// Windows：优先 PowerShell CIM，失败/为空再退回 wmic（新系统可能已移除 wmic）；
/// 非 Windows 不臆测，直接返回 None。超时 5 秒，任何失败都当作「没探测到」。
fn detect_display_adapter() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(raw) = run_probe_with_timeout(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-CimInstance Win32_VideoController).Name",
            ],
        ) {
            if let Some(name) = parse_video_controller_names(&raw) {
                return Some(name);
            }
        }
        if let Some(raw) = run_probe_with_timeout(
            "wmic",
            &["path", "win32_VideoController", "get", "name"],
        ) {
            if let Some(name) = parse_video_controller_names(&raw) {
                return Some(name);
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// 带 5 秒超时执行探测命令；超时/启动失败/非零退出均返回 None（不向上抛错）。
#[cfg(target_os = "windows")]
fn run_probe_with_timeout(exe: &str, args: &[&str]) -> Option<String> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut cmd = std::process::Command::new(exe);
    cmd.silent()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;

    // 独立线程抽干 stdout，避免管道写满导致子进程阻塞
    let mut pipe = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = pipe.read_to_string(&mut buf);
        buf
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let raw = reader.join().ok()?;
    if raw.trim().is_empty() {
        None
    } else {
        Some(raw)
    }
}

/// 宿主真实规格（运行 Marguerite 的机器）。
pub fn host_system_spec() -> HostSpec {
    let ram_gb = (detect_ram_bytes() / (1024 * 1024 * 1024)) as u32;
    let probe = detect_gpu();
    HostSpec {
        os: std::env::consts::OS.to_string(),
        ram_gb: ram_gb.max(1),
        cores: detect_cores().max(1),
        gpu: probe.accelerator,
        gpu_name: probe.name,
        gpu_kind: probe.kind.to_string(),
        source: "host".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 训练与微调（Finetune）— 诚实编排
//
// 设计边界（对齐本模块「诚实」哲学）：
// - 任务台账持久化 `local_finetunes.json`，运行时增删不重编译。
// - 仅当检测到「可用训练后端」才真正派发子进程；否则如实标记 needs_backend
//   并给出配置指引，绝不假装已训练。
// - 可用后端：用户通过 `MGT_FINETUNE_COMMAND` 提供命令模板（含 {base}/{dataset}/
//   {out}/{epochs}/{lr}/{method} 占位），或 PATH 上探测到已知工具（llama-finetune /
//   axolotl / unsloth）。
// ─────────────────────────────────────────────────────────────────────────────

/// 微调任务状态：queued / running / done / failed / cancelled
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinetuneJob {
    pub id: String,
    /// 基底模型：目录 id 或本地模型台账 id
    pub base_id: String,
    pub base_label: String,
    /// lora / qlora / full
    pub method: String,
    pub dataset_path: String,
    pub output_dir: String,
    pub epochs: u32,
    pub learning_rate: f32,
    pub status: String,
    /// 实际派发的后端命令（空=未派发）
    pub backend: String,
    /// 派发的子进程 pid（0=未派发）
    pub pid: i64,
    /// 是否因缺少训练后端而未派发（诚实标记）
    pub needs_backend: bool,
    pub error: String,
    pub created_ms: u64,
    pub finished_ms: u64,
}

/// 微调任务台账（持久化 `data_dir/local_finetunes.json`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalFinetuneStore {
    #[serde(default)]
    pub jobs: BTreeMap<String, FinetuneJob>,
}

fn finetune_store_path() -> PathBuf {
    Config::config_dir().join("local_finetunes.json")
}

impl LocalFinetuneStore {
    fn load() -> Self {
        match std::fs::read_to_string(finetune_store_path()) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
    fn save(&self) {
        if let Some(dir) = finetune_store_path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(finetune_store_path(), s);
        }
    }
}

/// 检测可用训练后端：用户模板优先，其次 PATH 探测已知工具。
fn detect_finetune_backend() -> Option<String> {
    if let Ok(t) = std::env::var("MGT_FINETUNE_COMMAND") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    for tool in ["llama-finetune", "axolotl", "unsloth"] {
        let exe = if cfg!(windows) {
            format!("{tool}.exe")
        } else {
            tool.to_string()
        };
        let path = std::env::var_os("PATH").unwrap_or_default();
        let found = std::env::split_paths(&path).any(|d| d.join(&exe).is_file());
        if found {
            return Some(format!("auto:{tool}"));
        }
    }
    None
}

/// 派发训练子进程（测试态绝不真实派发，避免下载/训练副作用）。
#[cfg(not(test))]
fn spawn_finetune(
    template: &str,
    base: &str,
    dataset: &str,
    out: &str,
    epochs: u32,
    lr: f32,
    method: &str,
) -> std::io::Result<i64> {
    let substituted = template
        .replace("{base}", base)
        .replace("{dataset}", dataset)
        .replace("{out}", out)
        .replace("{epochs}", &epochs.to_string())
        .replace("{lr}", &format!("{lr}"))
        .replace("{method}", method);
    let mut parts = substituted.split_whitespace();
    let exe = parts
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "空训练命令"))?;
    let child = std::process::Command::new(exe).silent().args(parts).spawn()?;
    Ok(child.id() as i64)
}

#[cfg(test)]
fn spawn_finetune(
    _template: &str,
    _base: &str,
    _dataset: &str,
    _out: &str,
    _epochs: u32,
    _lr: f32,
    _method: &str,
) -> std::io::Result<i64> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "测试态不派发真实训练",
    ))
}

#[cfg(not(test))]
fn kill_pid(pid: i64) {
    let _ = std::process::Command::new(if cfg!(windows) { "taskkill" } else { "kill" })
        .silent()
        .args(if cfg!(windows) {
            vec!["/PID".to_string(), pid.to_string(), "/F".to_string()]
        } else {
            vec!["-TERM".to_string(), pid.to_string()]
        })
        .spawn();
}

fn job_to_json(j: &FinetuneJob) -> Value {
    json!({
        "id": j.id,
        "base_id": j.base_id,
        "base_label": j.base_label,
        "method": j.method,
        "dataset_path": j.dataset_path,
        "output_dir": j.output_dir,
        "epochs": j.epochs,
        "learning_rate": j.learning_rate,
        "status": j.status,
        "backend": j.backend,
        "needs_backend": j.needs_backend,
        "error": j.error,
        "created_ms": j.created_ms,
        "finished_ms": j.finished_ms,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// RPC 方法
// ─────────────────────────────────────────────────────────────────────────────

fn ollama_present() -> bool {
    // 纯文件系统探测（不 spawn 进程，避免 Windows 下 ollama --version 挂起/副作用）：
    // 遍历 PATH 查找 ollama(.exe)。Ollama 安装后还会在常见目录，但 PATH 探测已足够。
    let path = std::env::var_os("PATH").unwrap_or_default();
    let exe = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    for dir in std::env::split_paths(&path) {
        let probe = dir.join(exe);
        if probe.is_file() {
            return true;
        }
    }
    false
}

/// `models.local.catalog`：聚合下载源 + shipped gate 目录（可热扩展）。
pub fn local_catalog_rpc(_params: &Value) -> Value {
    json!({
        "ok": true,
        "sources": [
            { "name": "ollama", "url": "https://ollama.com/library", "one_click": true },
            { "name": "huggingface", "url": "https://huggingface.co", "one_click": false },
            { "name": "modelscope", "url": "https://modelscope.cn", "one_click": false },
        ],
        "items": local_catalog().iter().map(|e| e.to_json()).collect::<Vec<_>>(),
    })
}

/// `models.local.recommend`：按用户系统配置 + 用途智能推荐。
/// 未显式传 ram/gpu/os 时，回退到**宿主真实探测**（取代旧硬编码 16G/无 GPU 猜值）。
pub fn local_recommend(params: &Value) -> Value {
    let intent = params
        .get("intent")
        .and_then(Value::as_str)
        .unwrap_or("general");
    let spec = host_system_spec();
    let ram = params
        .get("ramGB")
        .or_else(|| params.get("ram_gb"))
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(spec.ram_gb);
    let gpu = params
        .get("gpu")
        .and_then(Value::as_bool)
        .unwrap_or(spec.gpu);
    let os = params
        .get("os")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or(spec.os);
    let mut r = recommend_models(intent, ram, gpu, &os);
    r["ok"] = json!(true); // 与本文件其它 RPC 一致，携带 ok 标记
    r
}

/// `models.local.list`：已纳入管理的本地模型台账。
pub fn local_list(_params: &Value) -> Value {
    let store = LocalModelStore::load();
    json!({ "ok": true, "managed": store.managed_json() })
}

/// `models.local.download`：一键下载意图（Ollama 可用则真拉取；否则如实给官方命令/URL）。
pub fn local_download(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let entry = local_catalog().into_iter().find(|e| e.id == id);
    let Some(entry) = entry else {
        return json!({ "ok": false, "error": "目录无此模型", "code": "local_model.unknown" });
    };
    let mut store = LocalModelStore::load();
    let ollama = ollama_present();

    let status = if ollama && !entry.ollama_tag.is_empty() {
        // 后台拉取（不阻塞）：fire-and-forget 子进程。测试态绝不真实拉取（避免下载大模型副作用）。
        #[cfg(not(test))]
        {
            let tag = entry.ollama_tag.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("ollama")
                    .silent()
                    .args(["pull", &tag])
                    .spawn();
            });
        }
        "downloading"
    } else {
        "not_installed"
    };

    let m = store
        .managed
        .entry(entry.id.clone())
        .or_insert(ManagedLocalModel {
            id: entry.id.clone(),
            label: entry.label.clone(),
            ollama_tag: entry.ollama_tag.clone(),
            status: "not_installed".into(),
            deployed: false,
            profile_id: String::new(),
            started: false,
            added_ms: now_ms(),
        });
    m.status = status.into();
    m.label = entry.label.clone();
    store.save();

    json!({
        "ok": true,
        "status": status,
        "ollama_present": ollama,
        "entry": entry.to_json(),
        "action": if ollama && !entry.ollama_tag.is_empty() {
            format!("已发起 ollama pull {}（后台进行）", entry.ollama_tag)
        } else {
            format!(
                "Ollama 不可用：请安装 Ollama 后 `ollama pull {}`，或从 {} 手动下载",
                entry.ollama_tag, entry.hf_repo
            )
        },
    })
}

/// `models.local.deploy`：把本地模型注册为模型路由档案（kind=Local），标记为本地。
pub fn local_deploy(ctx: &crate::gateway::methods::GatewayContext, params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let entry = local_catalog().into_iter().find(|e| e.id == id);
    let Some(entry) = entry else {
        return json!({ "ok": false, "error": "目录无此模型", "code": "local_model.unknown" });
    };
    // shipped gate 六要素校验（蓝图纪律：开源权重「宣布 ≠ 可部署」）。
    // 默认仅作提示（advisory）；`requireGate=true` 时未过 gate 禁止接入路由。
    let gate = crate::gateway::shipped_gate::gate_local_model(&entry, false, true);
    if params.get("requireGate").and_then(Value::as_bool) == Some(true) && !gate.passed {
        return json!({
            "ok": false,
            "error": "未通过 shipped gate，禁止接入路由",
            "code": "local_model.gate_blocked",
            "gate_passed": gate.passed,
            "gate_blocking": gate.blocking.iter().map(|e| e.name()).collect::<Vec<_>>(),
        });
    }
    // 兜底 base_url（默认 Ollama OpenAI 兼容端点的**根地址**，不含 /v1）。
    // ⚠️ 旧值写成 `http://localhost:11434/v1`，而 LLMClient::chat_url 会再拼一段
    // `/v1/chat/completions` → `/v1/v1/...` 404。根地址由 build_chat_url 归一化。
    let base_url = params
        .get("baseUrl")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| "http://localhost:11434".into());
    let provider = params
        .get("provider")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| "ollama".into());

    let mut store = LocalModelStore::load();
    let profile_id = format!("local-{}", entry.id);
    let m = store
        .managed
        .entry(entry.id.clone())
        .or_insert(ManagedLocalModel {
            id: entry.id.clone(),
            label: entry.label.clone(),
            ollama_tag: entry.ollama_tag.clone(),
            status: "ready".into(),
            deployed: false,
            profile_id: profile_id.clone(),
            started: false,
            added_ms: now_ms(),
        });
    m.deployed = true;
    m.profile_id = profile_id.clone();
    m.status = "ready".into();
    m.label = entry.label.clone();
    store.save();

    // 写入 ModelSelector（kind=Local → 路由中标记为本地模型）
    let deploy_res = crate::model::selector::ModelSelector::load().and_then(|mut sel| {
        let p = crate::model::registry::ModelProfile {
            id: profile_id.clone(),
            label: format!("本地 · {}", entry.label),
            provider,
            base_url,
            api_key: String::new(),
            api_key_env: String::new(),
            model: if entry.ollama_tag.is_empty() {
                entry.id.clone()
            } else {
                entry.ollama_tag.clone()
            },
            kind: crate::model::registry::ProfileKind::Local,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
            tier: crate::model::tier::ModelTier::Balanced,
            tier_caps: crate::model::tier::TierCaps::default(),
            tier_caps_declared: false,
            // 本地运行时（Ollama / 本地权重目录）只实现 Chat Completions，
            // 显式写死而非留默认值：将来若默认值变化，本地档案不应被牵连。
            wire_api: crate::model::wire::WireApi::Chat,
            // 不声明能力 → 交给 `recognize` 按模型名保守识别（llava 判能读图，
            // 普通权重判纯文本）。用户仍可在设置页手动勾选覆盖。
            capabilities: Default::default(),
        };
        sel.registry_mut().upsert(p);
        sel.save()
    });

    match deploy_res {
        Ok(()) => {
            // 默认激活（闭环核心）：部署即把该本地档案置为 active，并经
            // `activate_profile_by_id` 完成 use_active + 镜像 config.model +
            // 发布生效快照，路由引擎下一轮即走回环本地端点（见 ModelRouter::pin_active_model）。
            let activate = params
                .get("activate")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut activated = false;
            if activate {
                if let Err(e) =
                    crate::gateway::llm_profile::activate_profile_by_id(ctx, &profile_id)
                {
                    return json!({
                        "ok": false,
                        "error": format!("激活失败: {e}"),
                        "code": "local_model.activate_failed",
                    });
                }
                activated = true;
            }
            // 部署/激活即并入路由候选并发布生效快照（候选列表即时对 UI/引擎可见）。
            let _ = crate::gateway::llm_profile::reconcile_routing_candidates(ctx);
            // 广播模型变更事件，供前端被动刷新（P3 复用）。
            crate::events::product::emit(
                "models.changed",
                "",
                json!({ "reason": "local_deploy", "profile_id": profile_id, "activated": activated }),
            );
            let cand = Config::load()
                .map(|c| c.model_routing.candidate_models.len())
                .unwrap_or(0);
            let mode = Config::load()
                .map(|c| c.model_routing.mode.clone())
                .unwrap_or_default();
            json!({
                "ok": true,
                "deployed": true,
                "activated": activated,
                "profile_id": profile_id,
                "kind": "local",
                "candidate_count": cand,
                "routing_mode": mode,
                "gate": {
                    "passed": gate.passed,
                    "blocking": gate.blocking.iter().map(|e| e.name()).collect::<Vec<_>>(),
                },
                "note": if activated {
                    "已自动激活并接入路由"
                } else {
                    "已注册为本地模型路由档案，可在模型选择器切换主模型"
                },
            })
        }
        Err(e) => {
            json!({ "ok": false, "error": e.to_string(), "code": "local_model.deploy_failed" })
        }
    }
}

/// `models.local.start` / `models.local.stop`：启停开关。
pub fn local_set_started(params: &Value, started: bool) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let mut store = LocalModelStore::load();
    let Some(m) = store.managed.get_mut(id) else {
        return json!({ "ok": false, "error": "未管理该模型", "code": "local_model.not_managed" });
    };
    if !m.deployed {
        return json!({ "ok": false, "error": "请先部署该模型", "code": "local_model.not_deployed" });
    }
    m.started = started;
    store.save();
    json!({
        "ok": true,
        "started": started,
        "ollama_present": ollama_present(),
        "note": if started {
            "已置为运行态；确保本地推理服务（如 Ollama）已启动。"
        } else {
            "已停止（本地推理进程由用户/服务管理）。"
        },
    })
}

/// `models.local.remove`：从台账移除（可选同时移除路由档案）。
pub fn local_remove(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let remove_profile = params
        .get("removeProfile")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut store = LocalModelStore::load();
    let Some(m) = store.managed.remove(id) else {
        return json!({ "ok": false, "error": "未管理该模型", "code": "local_model.not_managed" });
    };
    store.save();

    let mut profile_removed = false;
    if remove_profile && !m.profile_id.is_empty() {
        profile_removed = crate::model::selector::ModelSelector::load()
            .map(|mut sel| {
                // 仅保护「最后一个非本地档案」：本地档案由用户主动部署/移除，允许删空；
                // 内置/云端档案始终保留至少一个，避免路由无可用模型。
                let last_is_builtin = sel.registry().list().len() <= 1
                    && sel
                        .registry()
                        .list()
                        .iter()
                        .all(|p| p.kind != crate::model::registry::ProfileKind::Local);
                if last_is_builtin {
                    return false;
                }
                let removed = sel.registry_mut().remove(&m.profile_id).is_ok();
                let _ = sel.save();
                removed
            })
            .unwrap_or(false);
    }
    json!({
        "ok": true,
        "removed": id,
        "profile_removed": profile_removed,
    })
}

/// `models.local.status`：本地模型中心整体状态（Ollama 可用性 + 台账 + 路由中本地档案）。
pub fn local_status(_params: &Value) -> Value {
    let store = LocalModelStore::load();
    let local_profiles: Vec<Value> = crate::model::selector::ModelSelector::load()
        .map(|sel| {
            sel.registry()
                .list()
                .iter()
                .filter(|p| p.kind == crate::model::registry::ProfileKind::Local)
                .map(|p| {
                    json!({
                        "id": p.id,
                        "label": p.label,
                        "model": p.model,
                        "provider": p.provider,
                        "base_url": p.base_url,
                        "active": sel.registry().active == p.id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "ok": true,
        "ollama_present": ollama_present(),
        "os": std::env::consts::OS,
        "managed": store.managed_json(),
        "local_profiles": local_profiles,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// 实时 Ollama 枚举 / 部署（models.local.ollama[.deploy]）
// ─────────────────────────────────────────────────────────────────────────────

/// 把任意 base URL 规范成 **Ollama 根地址**（不含 `/v1`、无末尾 `/`）。
///
/// 部署档案的 `base_url` 必须是根地址：chat URL 由 `build_chat_url` 归一化拼接；
/// 若这里残留 `/v1`，发请求会拼出 `/v1/v1/...`。纯函数便于单测。
pub fn normalize_ollama_root(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    // 去掉可能的 /v1 或 /api/v1 后缀（/api/v1 也以 /v1 结尾）。
    let stripped = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    stripped.to_string()
}

/// 解析 Ollama 根地址：入参 `baseUrl` > env `MARGUERITE_OLLAMA_URL` > `http://localhost:11434`。
fn resolve_ollama_base_url(params: &Value) -> String {
    let from_param = params
        .get("baseUrl")
        .or_else(|| params.get("base_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !from_param.is_empty() {
        return normalize_ollama_root(from_param);
    }
    if let Ok(env) = std::env::var("MARGUERITE_OLLAMA_URL") {
        let env = env.trim();
        if !env.is_empty() {
            return normalize_ollama_root(env);
        }
    }
    "http://localhost:11434".to_string()
}

/// 把 Ollama tag 规范成 ModelProfile id：`ollama-` 前缀 + 所有非 `[a-z0-9]` 字符
/// 归并为单个 `-`（大写先转小写），去首尾 `-`。纯函数便于单测。
/// `omnipilot17:latest` → `ollama-omnipilot17-latest`。
pub fn sanitize_ollama_id(model: &str) -> String {
    let mut out = String::from("ollama-");
    let mut prev_dash = false;
    for ch in model.chars() {
        if ch.is_ascii_alphanumeric() && ch.is_ascii_lowercase() {
            out.push(ch);
            prev_dash = false;
        } else if ch.is_ascii_uppercase() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if ch.is_ascii_digit() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// 解析 `/api/tags` 响应体为模型数组（纯函数：不碰网络，便于单测）。
fn parse_ollama_tags(body: &Value) -> Vec<Value> {
    let Some(arr) = body.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .map(|m| {
            let details = m.get("details");
            let family = details
                .and_then(|d| d.get("family"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let families = details
                .and_then(|d| d.get("families"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let parameter_size = details
                .and_then(|d| d.get("parameter_size"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let quantization = details
                .and_then(|d| d.get("quantization_level"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let size_bytes = m.get("size").and_then(Value::as_u64).unwrap_or(0);
            // GiB（1024^3），保留 2 位小数，与 `ollama list` 的 SIZE 列口径一致。
            let size_gb =
                ((size_bytes as f64 / 1_073_741_824.0) * 100.0).round() / 100.0;
            json!({
                "name": m.get("name").and_then(Value::as_str).unwrap_or(""),
                "model": m.get("model").and_then(Value::as_str).unwrap_or(""),
                "digest": m.get("digest").and_then(Value::as_str).unwrap_or(""),
                "size_bytes": size_bytes,
                "size_gb": size_gb,
                "family": family,
                "families": families,
                "parameter_size": parameter_size,
                "quantization": quantization,
                "modified_at": m.get("modified_at").and_then(Value::as_str).unwrap_or(""),
            })
        })
        .collect()
}

/// 拉取并解析 `{root}/api/tags`。Ok(模型数组)；Err(可读原因)。3s 短超时，避免拖死页面。
pub(crate) async fn fetch_ollama_tags(root: &str) -> Result<Vec<Value>, String> {
    let url = format!("{root}/api/tags");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| format!("HTTP client 构建失败: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Ollama 不可达（{url}）: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Ollama 返回 HTTP {status}"));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;
    Ok(parse_ollama_tags(&body))
}

/// `models.local.ollama`：枚举 **live** Ollama 服务已装模型（打 `GET /api/tags`）。
///
/// 与 `local_status` 的 `ollama_present`（纯 PATH 文件探测）互补：这里回答的是
/// 「服务在线吗、里面到底有哪些 tag」。服务不在线/超时**不报错**，而是
/// `{ online:false, models:[], error }` 诚实标注 —— 前端据此渲染离线态而非崩溃。
pub async fn local_ollama_list(params: &Value) -> Value {
    let root = resolve_ollama_base_url(params);
    match fetch_ollama_tags(&root).await {
        Ok(models) => {
            json!({ "ok": true, "online": true, "base_url": root, "models": models })
        }
        Err(error) => {
            json!({ "ok": true, "online": false, "base_url": root, "models": [], "error": error })
        }
    }
}

/// `models.local.ollama.deploy`：把一个 live Ollama tag 注册为 kind=Local 路由档案，
/// 可选**立即激活**（走 use_active + 镜像 config + 发布生效快照的完整路径）。
pub async fn local_ollama_deploy(
    ctx: &crate::gateway::methods::GatewayContext,
    params: &Value,
) -> Value {
    let model = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if model.is_empty() {
        return json!({
            "ok": false,
            "error": "缺少 model（tag，如 omnipilot17:latest）",
            "code": "ollama_deploy.no_model",
        });
    }
    let root = resolve_ollama_base_url(params);

    // 先在线校验该 tag 真实存在于 /api/tags（不在线 / 缺模型都明确报错，绝不静默建档）。
    let models = match fetch_ollama_tags(&root).await {
        Ok(m) => m,
        Err(e) => {
            return json!({
                "ok": false,
                "error": format!("Ollama 不可达，无法部署 {model}: {e}"),
                "code": "ollama_deploy.offline",
            });
        }
    };
    if !models.iter().any(|m| m["name"].as_str() == Some(model)) {
        return json!({
            "ok": false,
            "error": format!("Ollama 中没有 tag \"{model}\"（先 `ollama pull {model}` 或核对名称）"),
            "code": "ollama_deploy.model_not_found",
        });
    }

    let label = params
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("Ollama · {model}"));
    let activate = params.get("activate").and_then(Value::as_bool).unwrap_or(false);

    match upsert_ollama_profile(ctx, model, &root, activate, label) {
        Ok(v) => v,
        Err(e) => json!({
            "ok": false,
            "error": e,
            "code": "ollama_deploy.save_failed",
        }),
    }
}

/// 把一个已校验存在的 Ollama tag 写入 models.json（kind=Local），可选激活。
///
/// 与网络校验解耦：纯文件 I/O + 激活路径，便于在隔离临时目录单测，不必依赖本机
/// 是否真的跑着 Ollama。`base_url` 用根地址（不含 `/v1`）。
fn upsert_ollama_profile(
    ctx: &crate::gateway::methods::GatewayContext,
    model: &str,
    root: &str,
    activate: bool,
    label: String,
) -> Result<Value, String> {
    let profile_id = sanitize_ollama_id(model);
    let upsert = crate::model::selector::ModelSelector::load().and_then(|mut sel| {
        let p = crate::model::registry::ModelProfile {
            id: profile_id.clone(),
            label,
            provider: "ollama".into(),
            base_url: root.to_string(),
            api_key: String::new(),
            api_key_env: String::new(),
            model: model.to_string(),
            kind: crate::model::registry::ProfileKind::Local,
            builtin: false,
            temperature: 0.7,
            max_tokens: 4096,
            tier: crate::model::tier::ModelTier::Balanced,
            tier_caps: crate::model::tier::TierCaps::default(),
            tier_caps_declared: false,
            // 本地运行时（Ollama / 本地权重目录）只实现 Chat Completions，
            // 显式写死而非留默认值：将来若默认值变化，本地档案不应被牵连。
            wire_api: crate::model::wire::WireApi::Chat,
            // 同上：留给名字识别（llava 等能读图的本地权重不被误判为纯文本）。
            capabilities: Default::default(),
        };
        sel.registry_mut().upsert(p);
        sel.save()
    });
    upsert.map_err(|e| format!("档案写入失败: {e}"))?;

    let mut activated = false;
    if activate {
        crate::gateway::llm_profile::activate_profile_by_id(ctx, &profile_id)?;
        // P1·B：激活即并入路由候选并发布生效快照（与 local_deploy 一致），
        // 设置中心/本地中心共用同一候选真源，保存 provider 后即时对 UI/引擎可见。
        crate::gateway::llm_profile::reconcile_routing_candidates(ctx)?;
        crate::events::product::emit(
            "models.changed",
            "",
            json!({ "reason": "ollama_upsert", "profile_id": profile_id, "activated": true }),
        );
        activated = true;
    }

    Ok(json!({
        "ok": true,
        "deployed": true,
        "profile_id": profile_id,
        "activated": activated,
        "model": model,
        "provider": "ollama",
    }))
}

/// `models.local.system`：宿主真实系统规格（RAM/核心/GPU），驱动智能推荐。
pub fn local_system(_params: &Value) -> Value {
    let s = host_system_spec();
    json!({
        "ok": true,
        "source": s.source,
        "os": s.os,
        "ram_gb": s.ram_gb,
        "cores": s.cores,
        "gpu": s.gpu,
        "gpu_name": s.gpu_name,
        "gpu_kind": s.gpu_kind,
    })
}

/// `models.local.finetune`：发起一次本地微调（诚实编排）。
/// 有可用训练后端则派发后台子进程；否则标记 needs_backend 并给出配置指引。
pub fn local_finetune(params: &Value) -> Value {
    let base_id = params
        .get("baseId")
        .or_else(|| params.get("base_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if base_id.is_empty() {
        return json!({ "ok": false, "error": "缺少 baseId", "code": "finetune.no_base" });
    }
    let dataset = params
        .get("dataset")
        .or_else(|| params.get("datasetPath"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if dataset.is_empty() {
        return json!({ "ok": false, "error": "缺少 dataset 路径", "code": "finetune.no_dataset" });
    }
    let method = params
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("lora")
        .to_string();
    let epochs = params
        .get("epochs")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(3);
    let lr = params
        .get("lr")
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .unwrap_or(2e-4);
    let output_dir = params
        .get("outputDir")
        .or_else(|| params.get("output_dir"))
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| {
            Config::config_dir()
                .join("finetunes")
                .join(&base_id)
                .display()
                .to_string()
        });

    // 基底标签：优先目录，其次已管理台账，再兜底用 id。
    let base_label = local_catalog()
        .into_iter()
        .find(|e| e.id == base_id)
        .map(|e| e.label)
        .or_else(|| {
            LocalModelStore::load()
                .managed
                .get(&base_id)
                .map(|m| m.label.clone())
        })
        .unwrap_or_else(|| base_id.clone());

    let id = format!("ft-{}", now_ms());
    let backend = detect_finetune_backend();

    // 自定义命令模板（非 auto:）→ 真正派发；仅探测到工具名（auto:）→ 诚实标记待配置。
    let (status, needs_backend, pid, dispatched) = match backend {
        Some(ref cmd) if !cmd.starts_with("auto:") => {
            match spawn_finetune(cmd, &base_id, &dataset, &output_dir, epochs, lr, &method) {
                Ok(pid) => ("running".to_string(), false, pid, cmd.clone()),
                Err(e) => ("failed".to_string(), false, 0, format!("派发失败: {e}")),
            }
        }
        other => (
            "queued".to_string(),
            true,
            0,
            other.clone().unwrap_or_default(),
        ),
    };

    let job = FinetuneJob {
        id: id.clone(),
        base_id,
        base_label,
        method,
        dataset_path: dataset,
        output_dir,
        epochs,
        learning_rate: lr,
        status: status.clone(),
        backend: dispatched.clone(),
        pid,
        needs_backend,
        error: if status == "failed" {
            dispatched.clone()
        } else {
            String::new()
        },
        created_ms: now_ms(),
        finished_ms: 0,
    };
    let mut store = LocalFinetuneStore::load();
    store.jobs.insert(id.clone(), job);
    store.save();

    json!({
        "ok": true,
        "job_id": id,
        "status": status,
        "needs_backend": needs_backend,
        "backend": dispatched,
        "note": if needs_backend {
            "当前无可用训练后端；请设置 MGT_FINETUNE_COMMAND（含 {base}/{dataset}/{out}/{epochs}/{lr}/{method} 占位）或安装 llama.cpp/unsloth/axolotl 后重试。".to_string()
        } else if status == "failed" {
            format!("训练派发失败：{dispatched}")
        } else {
            "已派发训练任务（后台进行）；可用 models.local.finetune.list 查看状态。".to_string()
        },
    })
}

/// `models.local.finetune.list`：列出所有微调任务。
pub fn local_finetune_list(_params: &Value) -> Value {
    let store = LocalFinetuneStore::load();
    let jobs: Vec<Value> = store.jobs.values().map(job_to_json).collect();
    json!({ "ok": true, "jobs": jobs })
}

/// `models.local.finetune.status`：单个微调任务状态。
pub fn local_finetune_status(params: &Value) -> Value {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let store = LocalFinetuneStore::load();
    match store.jobs.get(&id) {
        Some(j) => json!({ "ok": true, "job": job_to_json(j) }),
        None => json!({ "ok": false, "error": "无此微调任务", "code": "finetune.unknown" }),
    }
}

/// `models.local.finetune.cancel`：取消微调任务（已派发的子进程尽力 kill）。
pub fn local_finetune_cancel(params: &Value) -> Value {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut store = LocalFinetuneStore::load();
    let Some(mut j) = store.jobs.get(&id).cloned() else {
        return json!({ "ok": false, "error": "无此微调任务", "code": "finetune.unknown" });
    };
    if j.pid != 0 {
        #[cfg(not(test))]
        {
            kill_pid(j.pid);
        }
        let _ = j.pid; // 测试态不真实 kill
    }
    j.status = "cancelled".into();
    j.finished_ms = now_ms();
    store.jobs.insert(id.clone(), j);
    store.save();
    json!({ "ok": true, "cancelled": id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// 共享测试配置根（OnceLock+Mutex，与 methods.rs 同款：避免并发测试互相覆写 ~/.marguerite）。
    static CONF_GUARD: OnceLock<Mutex<std::path::PathBuf>> = OnceLock::new();
    /// 文件类测试须持有返回的 guard 至测试结束，串行化对 models.json / local_models.json 的写。
    fn lock_home() -> MutexGuard<'static, std::path::PathBuf> {
        let dir = CONF_GUARD
            .get_or_init(|| {
                let d = std::env::temp_dir()
                    .join(format!("marguerite-local-hub-{}", std::process::id()));
                let _ = std::fs::create_dir_all(&d);
                Mutex::new(d)
            })
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 关键修复：线程局部配置目录覆盖须在「每个」调用 lock_home 的线程上生效，
        // 否则并行/同线程先后跑多个用例时只有首个初始化线程拿到覆盖目录，其余线程
        // 回落到真实 ~/.marguerite，造成 local_models.json 跨用例串扰（flaky）。
        Config::set_config_dir_override(dir.clone());
        let _ = std::fs::remove_file(dir.join("models.json"));
        let _ = std::fs::remove_file(dir.join("local_models.json"));
        dir
    }

    #[test]
    fn catalog_has_shipped_gate_six_elements() {
        let catalog = local_catalog();
        assert!(catalog.len() >= 8, "目录应足够丰富");
        for e in &catalog {
            assert!(
                e.weights && e.quantized && e.reproducible,
                "{} 缺 shipped gate",
                e.id
            );
            assert!(!e.license.is_empty() && !e.eval.is_empty() && !e.pricing.is_empty());
            assert!(!e.hf_repo.is_empty(), "{} 缺 HF 源", e.id);
            assert!(e.min_ram_gb > 0);
        }
    }

    #[test]
    fn recommendation_respects_tier_budget() {
        // 边缘设备：不应推荐超 4B 的大模型
        let edge = recommend_models("general", 4, false, "windows");
        assert_eq!(edge["tier"], "edge");
        assert_eq!(edge["max_billions"], 4);
        assert!(
            !edge["picks"].as_array().unwrap().is_empty(),
            "边缘档位至少应有轻量候选"
        );
        for pick in edge["picks"].as_array().unwrap() {
            assert!(
                pick["size_gb"].as_f64().unwrap() <= 5.0,
                "边缘档位推荐过大: {}",
                pick["id"]
            );
        }
        // 服务器/GPU：可推 30B+ MoE
        let server = recommend_models("reasoning", 32, true, "linux");
        assert_eq!(server["tier"], "server");
        let ids: Vec<&str> = server["picks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert!(
            ids.contains(&"qwen3-30b-a3b"),
            "推理用途应推荐 Qwen3-30B-A3B"
        );
    }

    #[test]
    fn intent_steers_recommendation() {
        let coding = recommend_models("coding", 24, false, "windows");
        let ids: Vec<&str> = coding["picks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert!(ids.contains(&"qwen2.5-coder-14b"), "编码用途应含 coder");
    }

    #[test]
    fn deploy_registers_local_profile_and_marks_source() {
        let _g = lock_home();
        let v = local_deploy(
            &test_ctx(),
            &json!({ "id": "qwen3-4b", "baseUrl": "http://localhost:11434/v1" }),
        );
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["kind"], "local");

        // 路由档案应存在且 kind=Local
        let st = local_status(&Value::Null);
        let profiles = st["local_profiles"].as_array().unwrap();
        assert!(
            profiles
                .iter()
                .any(|p| p["id"].as_str() == Some("local-qwen3-4b")),
            "本地档案应出现在路由中: {profiles:?}"
        );

        // remove 应清理台账 + 档案
        let r = local_remove(&json!({ "id": "qwen3-4b", "removeProfile": true }));
        assert_eq!(r["ok"], true);
        let st2 = local_status(&Value::Null);
        assert!(st2["managed"].as_array().unwrap().is_empty());
        assert!(!st2["local_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"].as_str() == Some("local-qwen3-4b")));
    }

    #[test]
    fn local_deploy_activates_and_reconciles_candidates() {
        let _g = lock_home();
        let ctx = test_ctx();
        let v = local_deploy(
            &ctx,
            &json!({ "id": "qwen3-4b", "baseUrl": "http://localhost:11434" }),
        );
        assert_eq!(v["ok"], true, "{v}");
        // 闭环核心：部署默认即激活，并返回候选计数
        assert_eq!(v["activated"], true, "部署默认应激活: {v}");
        assert!(v["candidate_count"].as_u64().unwrap_or(0) >= 1, "候选应 ≥1: {v}");

        // 执行侧真相源 1：models.json 的 active 必须是新本地档案
        let sel = crate::model::selector::ModelSelector::load().unwrap();
        assert_eq!(sel.registry().active, "local-qwen3-4b");

        // 执行侧真相源 2/3：candidate_models 并入 + base_url 回环（直接读 YAML，
        // 不过 Config::load，避免 CWD .env 的 MARGUERITE_BASE_URL 覆盖断言）
        let raw = std::fs::read_to_string(Config::config_path()).unwrap();
        let cfg: Config = serde_yaml::from_str(&raw).unwrap();
        assert!(
            cfg.model_routing
                .candidate_models
                .contains(&"local-qwen3-4b".to_string()),
            "候选列表应含部署的本地档案: {:?}",
            cfg.model_routing.candidate_models
        );

        // 激活后 config.model.base_url 指向回环本地端点
        // → ModelRouter::pin_active_model=true，路由引擎下一轮即走本地模型（无需重启）
        assert!(
            cfg.model.base_url.contains("localhost") || cfg.model.base_url.contains("127.0.0.1"),
            "激活后 base_url 应为回环: {}",
            cfg.model.base_url
        );
    }

    #[test]
    fn download_records_intent_without_side_effect() {
        let _g = lock_home();
        let v = local_download(&json!({ "id": "qwen3-4b" }));
        assert_eq!(v["ok"], true);
        assert!(v["status"].as_str().is_some());
        assert!(v["action"]
            .as_str()
            .is_some_and(|s| s.contains("ollama pull")));

        let lst = local_list(&Value::Null);
        assert!(
            lst["managed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["id"] == "qwen3-4b"),
            "下载意图应写入台账"
        );
    }

    #[test]
    fn start_requires_deploy_first() {
        let _g = lock_home();
        let v = local_set_started(&json!({ "id": "qwen3-4b" }), true);
        assert_eq!(v["ok"], false, "未部署不得启动: {v}");
        // 未纳入管理（not_managed）或已管理但未部署（not_deployed）均属正确拒绝
        let code = v["code"].as_str().unwrap();
        assert!(
            code == "local_model.not_managed" || code == "local_model.not_deployed",
            "意外错误码: {code}"
        );
    }

    // ── 真实宿主系统探测 ──
    #[test]
    fn host_system_spec_returns_real_values() {
        let s = host_system_spec();
        assert_eq!(s.source, "host");
        assert!(!s.os.is_empty(), "os 应非空");
        assert!(s.ram_gb >= 1, "RAM 应 ≥1G");
        assert!(s.cores >= 1, "核心数应 ≥1");
        assert!(
            matches!(s.gpu_kind.as_str(), "discrete" | "integrated" | "none"),
            "gpu_kind 取值非法: {}",
            s.gpu_kind
        );
        // gpu/gpu_name 由环境决定，仅断言类型一致
    }

    // ── GPU 探测：纯函数解析与语义 ──
    #[test]
    fn rocm_product_skips_banner_and_header() {
        // 真机巡检缺口：首行是分隔线/表头时不能当型号
        let raw = "======================= ROCm System Management Interface =======================\n\
                   GPU[0] : Card series:\n\
                   GPU[0] : Card series: AMD Radeon RX 7900 XTX\n\
                   ======================= End of ROCm SMI Log =======================";
        assert_eq!(
            parse_rocm_product_name(raw).as_deref(),
            Some("AMD Radeon RX 7900 XTX")
        );
    }

    #[test]
    fn rocm_product_reads_card_series_value() {
        let raw = "GPU[0]\t\t: Card series:\t\tAMD Radeon Graphics";
        assert_eq!(
            parse_rocm_product_name(raw).as_deref(),
            Some("AMD Radeon Graphics")
        );
    }

    #[test]
    fn rocm_product_multi_gpu_takes_first_named() {
        let raw = "GPU[0] : Card series:\n\
                   GPU[1] : Card series: AMD Radeon RX 6800\n\
                   GPU[2] : Card series: AMD Radeon RX 7900 XTX";
        assert_eq!(
            parse_rocm_product_name(raw).as_deref(),
            Some("AMD Radeon RX 6800")
        );
    }

    #[test]
    fn video_controller_names_skip_microsoft_basic() {
        let raw = "Microsoft Basic Display Adapter\nMicrosoft Remote Display Adapter\nIntel(R) UHD Graphics 630";
        assert_eq!(
            parse_video_controller_names(raw).as_deref(),
            Some("Intel(R) UHD Graphics 630")
        );
        // 只有微软虚拟/基础适配器时视为未探测到，避免把虚拟显卡当真实硬件
        assert_eq!(
            parse_video_controller_names("Microsoft Basic Display Adapter"),
            None
        );
    }

    #[test]
    fn video_controller_names_read_wmic_list() {
        // wmic 输出首行是 Name 表头，必须跳过
        let raw = "Name\r\nIntel(R) UHD Graphics 630\r\nNVIDIA GeForce RTX 4060 Laptop GPU\r\n";
        assert_eq!(
            parse_video_controller_names(raw).as_deref(),
            Some("Intel(R) UHD Graphics 630")
        );
    }

    #[test]
    fn gpu_flag_not_flipped_by_integrated_display() {
        // 核显只填 gpu_name，不得让 gpu=true，也不得改变推荐档位
        let integrated = resolve_gpu(None, Some("Intel(R) UHD Graphics 630".into()));
        assert!(!integrated.accelerator);
        assert_eq!(integrated.name, "Intel(R) UHD Graphics 630");
        assert_eq!(integrated.kind, "integrated");
        assert_eq!(tier_for(16, integrated.accelerator), "workstation");

        // 只有 nvidia-smi / rocm-smi 命中才算加速器（discrete）
        let discrete = resolve_gpu(
            Some("NVIDIA GeForce RTX 4090".into()),
            Some("Intel(R) UHD Graphics 630".into()),
        );
        assert!(discrete.accelerator);
        assert_eq!(discrete.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(discrete.kind, "discrete");
        assert_eq!(tier_for(16, discrete.accelerator), "server");

        // 什么都没探测到：空名、kind=none、仍不是加速器
        let none = resolve_gpu(None, None);
        assert!(!none.accelerator);
        assert!(none.name.is_empty());
        assert_eq!(none.kind, "none");
    }

    #[test]
    fn local_system_rpc_returns_host_spec() {
        let v = local_system(&Value::Null);
        assert_eq!(v["ok"], true);
        assert_eq!(v["source"], "host");
        assert!(v["ram_gb"].as_u64().unwrap_or(0) >= 1);
        // 旧字段保留，新增 gpu_kind 不破坏兼容
        assert!(v["gpu"].is_boolean());
        assert!(v["gpu_name"].is_string());
        assert!(matches!(
            v["gpu_kind"].as_str(),
            Some("discrete" | "integrated" | "none")
        ));
    }

    #[test]
    fn recommend_without_params_uses_host_spec() {
        // 不传 ram/gpu/os 时回退宿主探测，不应 panic 且返回结构化推荐
        let v = local_recommend(&json!({}));
        assert_eq!(v["ok"], true);
        assert!(v["tier"].is_string());
        assert!(v["picks"].is_array());
    }

    // ── 训练与微调（诚实编排）──
    #[test]
    fn finetune_without_backend_is_queued_and_flagged() {
        let _g = lock_home();
        // 测试态 spawn_finetune 恒返回 Err；无自定义模板时 detect 返回 None/auto:
        // → 诚实标记 needs_backend=true，status=queued（绝不假装已训练）。
        let v = local_finetune(&json!({
            "baseId": "qwen3-4b",
            "dataset": "/tmp/ds.jsonl",
            "method": "lora",
            "epochs": 3,
        }));
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["needs_backend"], true, "无训练后端应诚实标记");
        assert_eq!(v["status"], "queued");
        let job_id = v["job_id"].as_str().unwrap().to_string();

        let lst = local_finetune_list(&Value::Null);
        assert!(lst["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["id"] == job_id));

        let c = local_finetune_cancel(&json!({ "id": job_id }));
        assert_eq!(c["ok"], true);
        let st = local_finetune_status(&json!({ "id": job_id }));
        assert_eq!(st["job"]["status"], "cancelled");
    }

    #[test]
    fn finetune_rejects_missing_inputs() {
        let _g = lock_home();
        let no_base = local_finetune(&json!({ "dataset": "/tmp/x.jsonl" }));
        assert_eq!(no_base["ok"], false);
        assert_eq!(no_base["code"], "finetune.no_base");
        let no_ds = local_finetune(&json!({ "baseId": "qwen3-4b" }));
        assert_eq!(no_ds["ok"], false);
        assert_eq!(no_ds["code"], "finetune.no_dataset");
    }

    // ── Ollama 实时枚举 / 部署（纯函数 + 隔离目录）──

    #[test]
    fn sanitize_ollama_id_collapses_non_alnum() {
        assert_eq!(
            sanitize_ollama_id("omnipilot17:latest"),
            "ollama-omnipilot17-latest"
        );
        assert_eq!(sanitize_ollama_id("omnipilot16c"), "ollama-omnipilot16c");
        assert_eq!(
            sanitize_ollama_id("Qwen2.5-Coder:14b"),
            "ollama-qwen2-5-coder-14b"
        );
        // 连续非法字符归并为单个 '-'
        assert_eq!(
            sanitize_ollama_id("weird__name..v2"),
            "ollama-weird-name-v2"
        );
    }

    #[test]
    fn normalize_ollama_root_strips_version_suffix() {
        assert_eq!(
            normalize_ollama_root("http://localhost:11434"),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_ollama_root("http://localhost:11434/"),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_ollama_root("http://localhost:11434/v1"),
            "http://localhost:11434"
        );
        assert_eq!(
            normalize_ollama_root("http://localhost:11434/v1/"),
            "http://localhost:11434"
        );
    }

    #[test]
    fn parse_ollama_tags_extracts_fields() {
        // 真实形状的 /api/tags 响应（本机实测结构）
        let body = json!({
            "models": [
                {
                    "name": "omnipilot17:latest",
                    "model": "omnipilot17:latest",
                    "modified_at": "2026-10-19T10:00:00Z",
                    "digest": "sha256-421e1507f821",
                    "size": 1_034_000_000u64,
                    "details": {
                        "format": "gguf",
                        "family": "qwen2",
                        "families": ["qwen2"],
                        "parameter_size": "7.6B",
                        "quantization_level": "Q4_K_M"
                    }
                },
                { "name": "bare:tag" }
            ]
        });
        let out = parse_ollama_tags(&body);
        assert_eq!(out.len(), 2);
        let first = &out[0];
        assert_eq!(first["name"], "omnipilot17:latest");
        assert_eq!(first["family"], "qwen2");
        assert_eq!(first["families"], json!(["qwen2"]));
        assert_eq!(first["parameter_size"], "7.6B");
        assert_eq!(first["quantization"], "Q4_K_M");
        assert!(first["size_gb"].as_f64().unwrap() > 0.9);
        let second = &out[1];
        assert_eq!(second["family"], "");
        assert_eq!(second["families"], json!([]));
        // 无 models 字段 → 空数组不 panic
        assert!(parse_ollama_tags(&json!({})).is_empty());
    }

    #[tokio::test]
    async fn local_ollama_list_offline_returns_online_false_not_error() {
        // 不可达端口：必须诚实返回 online:false，而不是抛异常 / ok:false。
        let v = local_ollama_list(&json!({ "baseUrl": "http://127.0.0.1:1" })).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["online"], false);
        assert!(v["models"].as_array().unwrap().is_empty());
        assert!(v["error"].as_str().is_some_and(|s| !s.is_empty()));
        // base_url 应规范化为根地址（去末尾 '/'，不去端口）
        assert_eq!(v["base_url"], "http://127.0.0.1:1");
    }

    fn test_ctx() -> crate::gateway::methods::GatewayContext {
        crate::gateway::methods::GatewayContext {
            config: Config::default(),
            token: "t".into(),
            started_at: std::time::Instant::now(),
            bus: std::sync::Arc::new(crate::events::EventBus::default()),
        }
    }

    #[test]
    fn upsert_ollama_profile_writes_local_profile_and_activates() {
        let _g = lock_home();
        let ctx = test_ctx();
        let v = upsert_ollama_profile(
            &ctx,
            "omnipilot17:latest",
            "http://localhost:11434",
            true,
            "Ollama · omnipilot17:latest".to_string(),
        )
        .expect("upsert 应成功");
        assert_eq!(v["ok"], true);
        assert_eq!(v["profile_id"], "ollama-omnipilot17-latest");
        assert_eq!(v["activated"], true);

        // 执行侧真相源：models.json 的 active 必须是新档案
        let after = crate::model::selector::ModelSelector::load().unwrap();
        assert_eq!(after.registry().active, "ollama-omnipilot17-latest");
        let p = after.registry().get("ollama-omnipilot17-latest").unwrap();
        assert_eq!(p.provider, "ollama");
        assert_eq!(p.kind, crate::model::registry::ProfileKind::Local);
        assert_eq!(
            p.base_url, "http://localhost:11434",
            "base_url 必须是根地址，不含 /v1"
        );
        assert_eq!(p.model, "omnipilot17:latest");
        // config.yaml 三要素镜像（不过 Config::load，避免 .env 覆盖）
        let raw = std::fs::read_to_string(Config::config_path()).unwrap();
        let cfg: Config = serde_yaml::from_str(&raw).unwrap();
        assert_eq!(cfg.model.provider, "ollama");
        assert_eq!(cfg.model.model_name, "omnipilot17:latest");
        assert_eq!(cfg.model.base_url, "http://localhost:11434");
    }

    #[test]
    fn upsert_ollama_profile_without_activate_leaves_active_untouched() {
        let _g = lock_home();
        let ctx = test_ctx();
        // 先把 active 指到种子默认
        {
            let mut sel = crate::model::selector::ModelSelector::load().unwrap();
            sel.use_active("deepseek-chat").unwrap();
            sel.save().unwrap();
        }
        let v = upsert_ollama_profile(
            &ctx,
            "omnipilot16c:latest",
            "http://localhost:11434",
            false,
            String::new(),
        )
        .unwrap();
        assert_eq!(v["activated"], false);
        let after = crate::model::selector::ModelSelector::load().unwrap();
        assert_ne!(
            after.registry().active,
            "ollama-omnipilot16c-latest",
            "未要求激活不得改 active"
        );
        assert!(after.registry().get("ollama-omnipilot16c-latest").is_some());
    }
}
