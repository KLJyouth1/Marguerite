//! M5 doctor 体检模块（纯本机、可亲验、不依赖真实 API key / 真机 SSH / 用户新决策）。
//!
//! 输出**可复制**的 JSON 诊断报告：版本号 + 逐项 `OK/MISSING/DEGRADED` + 修复动作。
//! 离线环境下这是唯一能传给支持方的证据。
//!
//! # 设计铁律
//! - 绝不输出 API key 明文、不把密钥写进报告。
//! - 绝对路径中可能含用户名/盘符名的部分保留（用户可自证），但对密钥/令牌脱敏。
//! - 第 2 项（WSL/Docker）本机可能无：返回 `DEGRADED`（不装也不失败）。
//! - 第 7 项（API 连通）无 key：返回 `DEGRADED` +「未配置密钥」指引（不因缺 key 判 FAIL）。
//! - 第 8 项（路由权重）缺失：返回 `DEGRADED` + 修复动作（ML 路由是可选增强，不判 FAIL）。
//! - 诊断器逻辑下沉到本模块的 service 层；聚合/输出在 `mod.rs`。
//!
//! # 分层
//! ```text
//! mod.rs        —— 聚合 + JSON 输出（run_doctor）
//! appliance.rs  —— 第 1/3/4/5/6 项（python / 端口 / 数据目录 / 资源 / 磁盘）
//! backend.rs    —— 第 2 项（WSL / Docker）
//! connectivity.rs —— 第 7 项（模型 API 探活）
//! router_weights.rs —— 第 8 项（SquillaRouter ML 权重完整性）
//! ```

pub mod appliance;
pub mod auth_probe;
pub mod backend;
pub mod connectivity;
pub mod disk;
pub mod router_weights;

use serde::Serialize;

use crate::persistence::config::Config;

// ───────────────────────────────────────────────────────────────────────────
// 昂贵探测项的短 TTL 缓存
//
// 「昂贵项」= 会**起子进程**的探测：Python 版本（`detect_python` 逐候选 spawn）、
// WSL/Docker（`wsl.exe -l -q` / `docker info`）、磁盘余量（`fsutil`）。
// WebUI 概览页每 30s 轮询一次 `doctor.status`（浅层，`probe=Off`），若每次都重跑，
// 就是每 30s 白白 spawn ~10 个子进程。此处给浅层路径加一层 120s 进程内缓存：
//
// - 仅**浅层**（`probe=Off`）命中缓存；**深层 / 手动刷新**（`probe=On`）恒重算，
//   并把新结果回写缓存（TTL=0 → 强制重算，但结果仍是新鲜的）。
// - 缓存是**进程内全局**：CLI（`marguerite doctor`）是独立进程，缓存天然为空 →
//   CLI 永远拿到当次新鲜结果，不受影响。
// ───────────────────────────────────────────────────────────────────────────

/// 昂贵探测项在浅层路径下的缓存 TTL。
const EXPENSIVE_ITEM_TTL: std::time::Duration = std::time::Duration::from_secs(120);

type ExpensiveItemCache =
    std::sync::Mutex<std::collections::HashMap<&'static str, (std::time::Instant, DoctorItem)>>;

fn expensive_item_cache() -> &'static ExpensiveItemCache {
    static CACHE: std::sync::OnceLock<ExpensiveItemCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 取缓存项（未过期）或重算并回写。`ttl == Duration::ZERO` 表示强制重算。
fn cached_expensive_item(
    key: &'static str,
    ttl: std::time::Duration,
    compute: impl FnOnce() -> DoctorItem,
) -> DoctorItem {
    let cache = expensive_item_cache();
    if let Ok(map) = cache.lock() {
        if let Some((at, item)) = map.get(key) {
            if at.elapsed() < ttl {
                return item.clone();
            }
        }
    }
    let fresh = compute();
    if let Ok(mut map) = cache.lock() {
        map.insert(key, (std::time::Instant::now(), fresh.clone()));
    }
    fresh
}

/// 单项体检状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    /// 通过
    Ok,
    /// 缺失 / 失败（需要修复）
    Missing,
    /// 降级（可用但不理想 / 未配置密钥 / 本机无 WSL/Docker）
    Degraded,
}

impl Status {
    /// 机器可读字符串（JSON 中也是 `"OK"` 等，保持一致）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Missing => "MISSING",
            Status::Degraded => "DEGRADED",
        }
    }
}

/// 单项诊断结果。
#[derive(Debug, Clone, Serialize)]
pub struct DoctorItem {
    /// 检查项 ID（如 `python` / `gateway_port` / `data_dir` / `resources` / `disk` / `backend` / `model_api`）
    pub id: String,
    /// 人类可读名称
    pub label: String,
    /// 状态
    pub status: Status,
    /// 详情（脱敏后的实际探测结果）
    pub detail: String,
    /// 修复动作（为空表示无需修复）
    pub fix_hint: String,
}

/// 聚合诊断报告。
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    /// 系统版本（来自 `CARGO_PKG_VERSION`）
    pub version: String,
    /// 机器架构 + OS（`rust_arch` / `os` 组合信息）
    pub platform: String,
    /// UTC 时间戳（ISO8601）
    pub generated_at: String,
    /// 各检查项
    pub items: Vec<DoctorItem>,
    /// 汇总统计
    pub summary: DoctorSummary,
    /// 报告级状态（所有项 OK -> ok；存在 MISSING -> degraded；否则 degraded）
    pub status: Status,
}

/// 汇总统计。
#[derive(Debug, Clone, Serialize)]
pub struct DoctorSummary {
    /// 检查项总数
    pub total: usize,
    /// OK 数
    pub ok: usize,
    /// MISSING 数
    pub missing: usize,
    /// DEGRADED 数
    pub degraded: usize,
}

/// 运行完整 doctor 体检，返回可复制 JSON 报告（`--json` 用）。
///
/// 使用 [`Config::default()`] 的探测路径；单测可改用 [`run_doctor_with_config`]。
pub fn run_doctor() -> String {
    run_doctor_with_config(&Config::default())
}

/// 运行完整 doctor 体检，返回 **JSON 字符串**（可被 CLI / RPC 直接输出）。
///
/// 聚合全部 8 项，逐项收集 `{id,label,status,detail,fix_hint}`，最后排序为
/// `python -> gateway_port -> data_dir -> resources -> disk -> backend -> model_api -> router_weights`。
pub fn run_doctor_with_config(config: &Config) -> String {
    run_doctor_with_config_probe(config, connectivity::AuthProbeTier::Off)
}

/// 带鉴权探测档位的完整体检（同步外壳，供 CLI / 单测使用）。
///
/// ⚠️ 本函数会自建 runtime 并 `block_on`，**不可**在 async 上下文里调用。
/// 网关 RPC 请改用 [`run_doctor_with_config_probe_async`]。
pub fn run_doctor_with_config_probe(
    config: &Config,
    probe: connectivity::AuthProbeTier,
) -> String {
    let config = config.clone();
    match tokio::runtime::Runtime::new() {
        Ok(rt) => rt.block_on(run_doctor_with_config_probe_async(&config, probe)),
        Err(e) => serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "status": "DEGRADED",
            "detail": format!("诊断运行时创建失败: {e}"),
        })
        .to_string(),
    }
}

/// 带鉴权探测档位的完整体检（**纯异步**，方案 B —— 网关 RPC 的唯一正确入口）。
///
/// 与同步版同源同契约，唯一区别是不自建 runtime、不 `block_on`：7 项探测
/// （尤其第 7 项的网络探针）由调用方所在的 runtime 驱动，因此可安全地在
/// 慢方法独立 runtime 的 worker 线程上执行，不会触发 tokio 的
/// `Cannot start a runtime from within a runtime`。
///
/// `probe = On` 时第 7 项（model_api）在 TCP 可达后发起一次真实 `GET /models`，
/// 用 2xx / 401 / 403 判定密钥真伪；默认 `Off` 保持离线契约。
pub async fn run_doctor_with_config_probe_async(
    config: &Config,
    probe: connectivity::AuthProbeTier,
) -> String {
    // 浅层路径（WebUI 30s 轮询）复用昂贵项的短 TTL 缓存；深层恒重算。
    let use_cache = probe == connectivity::AuthProbeTier::Off;
    let items = collect_items_async(config, probe, use_cache).await;
    let summary = summarize(&items);
    let status = report_status(&summary);
    let report = DoctorReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: format!(
            "{} ({} bit)",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        generated_at: chrono::Utc::now().to_rfc3339(),
        items,
        summary,
        status,
    };
    // 结构化 JSON（缩进，便于复制 / 人工审阅）
    serde_json::to_string_pretty(&report).unwrap_or_else(|_| {
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "status": "MISSING",
            "detail": "报告序列化失败",
        })
        .to_string()
    })
}

/// 9. 档位能力声明审计（5c 三档分层 → M2 准入的可见性出口）。
///
/// 为什么必须进 doctor：档位准入（`tier.rs::filter_chain`）只在档案**显式声明**
/// `tier_caps` 时才真正过滤，未声明者按"信任档位声明"放行。若不体检，
/// 用户会以为旗舰档经过了能力校验，实际可能全是未声明的空壳；
/// 而"声明与档位矛盾"（如标旗舰却不支持视觉）会让预算预扣与降级行为异常。
pub fn check_tier_caps() -> DoctorItem {
    use crate::model::selector::ModelSelector;

    let audit = match ModelSelector::load() {
        Ok(s) => s.registry().audit_tier_caps(),
        // 读不到档案表时如实报 degraded —— 不 panic、也不假装通过
        Err(e) => {
            return degraded_item(
                "tier_caps",
                "档位能力声明",
                format!("无法读取模型档案表：{e}"),
                "检查模型档案文件是否存在且可读".to_string(),
            )
        }
    };

    let detail = audit.summary();
    if !audit.has_issues() {
        return ok_item("tier_caps", "档位能力声明", detail);
    }
    let mut hint: Vec<String> = Vec::new();
    if !audit.unverified.is_empty() {
        hint.push(format!("未声明能力：{}", audit.unverified.join(", ")));
    }
    for (id, tier, why) in &audit.mismatched {
        hint.push(format!("{id} 声明为 {tier:?} 但不满足准入：{}", why.join("；")));
    }
    degraded_item("tier_caps", "档位能力声明", detail, hint.join(" | "))
}

/// 逐项收集（不含聚合排序状态判断）。
///
/// `use_cache`：浅层路径为 `true`，昂贵项（起子进程的 Python/WSL-Docker/磁盘）
/// 走短 TTL 缓存；深层路径为 `false`，恒重算并回写缓存。
#[allow(clippy::vec_init_then_push)]
async fn collect_items_async(
    config: &Config,
    probe: connectivity::AuthProbeTier,
    use_cache: bool,
) -> Vec<DoctorItem> {
    // 按 PACKAGING.md §6.1 的 8 项顺序逐项收集；每一项都是独立、可亲验的诊断器。
    let mut items = Vec::with_capacity(8);
    let ttl = if use_cache {
        EXPENSIVE_ITEM_TTL
    } else {
        std::time::Duration::ZERO
    };

    // 1. Python 3.12+（昂贵：逐候选 spawn 解释器）
    items.push(cached_expensive_item("python", ttl, appliance::check_python));

    // 2. WSL / Docker 可用性（昂贵：`wsl.exe -l -q` / `docker info`）
    items.push(cached_expensive_item(
        "backend",
        ttl,
        backend::check_linux_backend,
    ));

    // 3. 网关端口 18790（廉价：仅回环 bind 探测，不起子进程）
    items.push(appliance::check_gateway_port());

    // 4. 数据目录读写（廉价：写临时探针文件）
    items.push(appliance::check_data_dir_write(config));

    // 5. 嵌入资源完整性（廉价：目录存在性）
    items.push(appliance::check_resources());

    // 6. 磁盘余量（昂贵：`fsutil` / `df`）
    items.push(cached_expensive_item("disk", ttl, || {
        appliance::check_disk_free(config)
    }));

    // 7. 模型 API 连通（异步探测：不 block_on，可安全跑在 runtime worker 线程上）
    items.push(connectivity::check_model_api_async(config, probe).await);

    // 8. SquillaRouter ML 权重完整性（廉价：只读文件，不起子进程，故不进昂贵项缓存）
    items.push(router_weights::check_router_weights());

    // 9. 档位能力声明审计（廉价：只读档案文件；把"未声明/声明矛盾"暴露给用户） ——
    //    接上 M2 的准入校验：否则用户看不到"旗舰档其实未经过能力校验"。
    items.push(check_tier_caps());

    items
}

/// 统计各状态数量。
fn summarize(items: &[DoctorItem]) -> DoctorSummary {
    let ok = items.iter().filter(|i| i.status == Status::Ok).count();
    let missing = items.iter().filter(|i| i.status == Status::Missing).count();
    let degraded = items.iter().filter(|i| i.status == Status::Degraded).count();
    DoctorSummary {
        total: items.len(),
        ok,
        missing,
        degraded,
    }
}

/// 报告级状态：全 OK -> ok；任一 MISSING 或 DEGRADED -> degraded（不阻断，但如实标注）。
fn report_status(s: &DoctorSummary) -> Status {
    if s.missing == 0 && s.degraded == 0 {
        Status::Ok
    } else {
        Status::Degraded
    }
}

/// 构造一个 `OK` 项（便捷辅助，供子模块复用）。
pub fn ok_item(id: &str, label: &str, detail: String) -> DoctorItem {
    DoctorItem {
        id: id.to_string(),
        label: label.to_string(),
        status: Status::Ok,
        detail,
        fix_hint: String::new(),
    }
}

/// 构造一个 `MISSING` 项（便捷辅助，供子模块复用）。
pub fn missing_item(id: &str, label: &str, detail: String, fix_hint: String) -> DoctorItem {
    DoctorItem {
        id: id.to_string(),
        label: label.to_string(),
        status: Status::Missing,
        detail,
        fix_hint,
    }
}

/// 构造一个 `DEGRADED` 项（便捷辅助，供子模块复用）。
pub fn degraded_item(id: &str, label: &str, detail: String, fix_hint: String) -> DoctorItem {
    DoctorItem {
        id: id.to_string(),
        label: label.to_string(),
        status: Status::Degraded,
        detail,
        fix_hint,
    }
}

#[cfg(test)]
mod tests;
