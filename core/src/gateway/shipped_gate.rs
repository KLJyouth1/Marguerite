//! shipped gate 六要素校验器
//!
//! 开源权重「宣布 ≠ 可部署」。把第三方 / 本地模型接入路由前，须逐要素校验：
//! **权重 / 许可证 / 评测 / 定价 / 可用性 / 兼容性**。出现任一 `Fail` 即未过 gate，
//! 不得接入路由（蓝图 shipped gate 纪律）。
//!
//! 设计边界（对齐本模块「诚实」哲学）：校验器只验证「能从模型元数据 + 调用方提供的
//! 部署事实推导出的结论」，绝不臆测网络/格式。外部事实由两个参数传入：
//! - `weights_present`：权重文件**实际已下载/存在**（而非仅「宣布」）；
//! - `runtime_supports`：本地运行时是否支持该模型格式/架构（gguf/safetensors 等）。
//! 声明有权重但未实际落地 → `Fail`，这正是「宣布 ≠ 可部署」的核心。

#![allow(clippy::doc_lazy_continuation)] // 模块级文档续行
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gateway::local_models::LocalModelEntry;

/// 六要素之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateElement {
    /// 权重：实际可用的权重文件
    Weights,
    /// 许可证：本地推理许可边界
    License,
    /// 评测：独立基准/第三方验证
    Eval,
    /// 定价：成本/用量可观测
    Pricing,
    /// 可用性：至少一条可达的下载源
    Availability,
    /// 兼容性：本地运行时支持其格式/架构
    Compatibility,
}

impl GateElement {
    pub fn name(&self) -> &'static str {
        match self {
            GateElement::Weights => "权重",
            GateElement::License => "许可证",
            GateElement::Eval => "评测",
            GateElement::Pricing => "定价",
            GateElement::Availability => "可用性",
            GateElement::Compatibility => "兼容性",
        }
    }
}

/// 单要素校验结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateVerdict {
    /// 通过
    Pass,
    /// 提示（不阻断；如许可证未知、缺评测）
    Warn,
    /// 阻断（未过 gate，禁止接入路由）
    Fail,
}

impl GateVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateVerdict::Pass => "pass",
            GateVerdict::Warn => "warn",
            GateVerdict::Fail => "fail",
        }
    }
}

/// 单要素发现。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateFinding {
    pub element: GateElement,
    pub verdict: GateVerdict,
    pub detail: String,
}

/// 六要素校验总报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateReport {
    pub model: String,
    pub findings: Vec<GateFinding>,
    /// 无 `Fail` 即通过（仅 `Warn` 不阻断，仅提示）
    pub passed: bool,
    /// 阻断性要素（出现 `Fail` 的要素）；未过 gate 不得接入路由
    pub blocking: Vec<GateElement>,
}

fn finding(element: GateElement, verdict: GateVerdict, detail: impl Into<String>) -> GateFinding {
    GateFinding {
        element,
        verdict,
        detail: detail.into(),
    }
}

/// 校验一个本地 / 第三方模型是否满足 shipped gate 六要素。
///
/// - `weights_present`：权重文件**实际已下载/存在**（而非仅「宣布」）。
/// - `runtime_supports`：本地运行时是否支持该模型格式/架构（由调用方据实际部署环境提供）。
pub fn gate_local_model(
    entry: &LocalModelEntry,
    weights_present: bool,
    runtime_supports: bool,
) -> GateReport {
    let mut findings = Vec::new();

    // 权重：声明 + 实际落地 → Pass；声明但缺失 → Fail（宣布≠可部署）；未声明 → Fail
    findings.push(if entry.weights && weights_present {
        finding(
            GateElement::Weights,
            GateVerdict::Pass,
            "权重已声明且实际落地",
        )
    } else if entry.weights && !weights_present {
        finding(
            GateElement::Weights,
            GateVerdict::Fail,
            "权重已宣布但未实际下载/存在（宣布 ≠ 可部署）",
        )
    } else {
        finding(GateElement::Weights, GateVerdict::Fail, "未声明可用权重")
    });

    // 许可证：非空且非「未知」类 → Pass；空 → Fail；未知类 → Warn（部署前请确认边界）
    let lic = entry.license.trim();
    findings.push(if lic.is_empty() {
        finding(GateElement::License, GateVerdict::Fail, "缺少许可证声明")
    } else if lic.to_lowercase().contains("unknown") || lic.contains("未知") {
        finding(
            GateElement::License,
            GateVerdict::Warn,
            format!("许可证为「{lic}」，部署前请确认本地推理许可边界"),
        )
    } else {
        finding(
            GateElement::License,
            GateVerdict::Pass,
            format!("许可证：{lic}"),
        )
    });

    // 评测：非空 → Pass；空 → Warn（仅提示：宣布但未经第三方验证）
    findings.push(if entry.eval.trim().is_empty() {
        finding(
            GateElement::Eval,
            GateVerdict::Warn,
            "未提供独立评测/基准（宣布但未经第三方验证）",
        )
    } else {
        finding(GateElement::Eval, GateVerdict::Pass, "已提供评测/基准")
    });

    // 定价：非空（含「本地免费」）→ Pass；空 → Warn
    findings.push(if entry.pricing.trim().is_empty() {
        finding(GateElement::Pricing, GateVerdict::Warn, "未声明定价/成本")
    } else {
        finding(
            GateElement::Pricing,
            GateVerdict::Pass,
            format!("定价：{}", entry.pricing),
        )
    });

    // 可用性：至少一条下载源（ollama 标签 / HF / ModelScope）→ Pass；无源 → Fail
    let has_source =
        !entry.ollama_tag.is_empty() || !entry.hf_repo.is_empty() || !entry.msc_repo.is_empty();
    findings.push(if has_source {
        finding(
            GateElement::Availability,
            GateVerdict::Pass,
            "至少一条聚合下载源可用",
        )
    } else {
        finding(
            GateElement::Availability,
            GateVerdict::Fail,
            "未声明任何下载源（无法获取权重）",
        )
    });

    // 兼容性：运行时支持该格式/架构 → Pass；否则 → Fail
    findings.push(if runtime_supports {
        finding(
            GateElement::Compatibility,
            GateVerdict::Pass,
            "本地运行时支持该模型格式/架构",
        )
    } else {
        finding(
            GateElement::Compatibility,
            GateVerdict::Fail,
            "本地运行时不支持该模型格式/架构",
        )
    });

    let blocking: Vec<GateElement> = findings
        .iter()
        .filter(|f| f.verdict == GateVerdict::Fail)
        .map(|f| f.element)
        .collect();
    let passed = blocking.is_empty();
    GateReport {
        model: entry.id.clone(),
        findings,
        passed,
        blocking,
    }
}

/// `models.gate.check`：按目录 id 校验 shipped gate 六要素。
///
/// 可选覆盖（默认：无实际权重落地 = `false`、运行时支持 = `true`）：
/// - `weightsPresent`：权重文件是否已实际下载/存在；
/// - `runtimeSupports`：本地运行时是否支持该模型格式/架构。
pub fn gate_check(params: &Value) -> Value {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        return json!({ "ok": false, "error": "缺少 id", "code": "gate.no_id" });
    }
    let entry = crate::gateway::local_models::local_catalog()
        .into_iter()
        .find(|e| e.id == id);
    let Some(entry) = entry else {
        return json!({ "ok": false, "error": "目录无此模型", "code": "gate.unknown" });
    };
    let weights_present = params
        .get("weightsPresent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let runtime_supports = params
        .get("runtimeSupports")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let report = gate_local_model(&entry, weights_present, runtime_supports);
    json!({
        "ok": true,
        "model": report.model,
        "passed": report.passed,
        "blocking": report.blocking.iter().map(|e| e.name()).collect::<Vec<_>>(),
        "findings": report.findings.iter().map(|f| json!({
            "element": f.element.name(),
            "verdict": f.verdict.as_str(),
            "detail": f.detail,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::local_models::local_catalog;

    /// 取目录中一个真实条目做基线（避免暴露私有构造器）。
    fn base_entry() -> LocalModelEntry {
        local_catalog()
            .into_iter()
            .find(|e| e.id == "qwen3-4b")
            .expect("qwen3-4b 应在目录中")
    }

    #[test]
    fn gate_passes_when_all_declared_and_present() {
        let e = base_entry();
        let r = gate_local_model(&e, true, true);
        assert!(r.passed, "六要素齐全应过 gate：{:?}", r.blocking);
        assert!(r.blocking.is_empty());
        assert!(r.findings.iter().all(|f| f.verdict != GateVerdict::Fail));
    }

    #[test]
    fn gate_fails_when_weights_announced_but_missing() {
        // 「宣布 ≠ 可部署」核心：声明有权重但未实际落地 → 阻断。
        let e = base_entry();
        let r = gate_local_model(&e, false, true);
        assert!(!r.passed);
        assert!(r.blocking.contains(&GateElement::Weights));
    }

    #[test]
    fn gate_warns_on_unknown_license_and_missing_eval() {
        // Warn 不阻断：未知许可证 + 缺评测仍可通过 gate，仅提示。
        let mut e = base_entry();
        e.license = "unknown".into();
        e.eval = String::new();
        let r = gate_local_model(&e, true, true);
        assert!(r.passed, "Warn 不应阻断 gate");
        assert!(r
            .findings
            .iter()
            .any(|f| f.element == GateElement::License && f.verdict == GateVerdict::Warn));
        assert!(r
            .findings
            .iter()
            .any(|f| f.element == GateElement::Eval && f.verdict == GateVerdict::Warn));
    }

    #[test]
    fn gate_fails_on_unsupported_runtime() {
        let e = base_entry();
        let r = gate_local_model(&e, true, false);
        assert!(!r.passed);
        assert!(r.blocking.contains(&GateElement::Compatibility));
    }

    #[test]
    fn gate_fails_on_no_source() {
        let mut e = base_entry();
        e.ollama_tag = String::new();
        e.hf_repo = String::new();
        e.msc_repo = String::new();
        let r = gate_local_model(&e, true, true);
        assert!(!r.passed);
        assert!(r.blocking.contains(&GateElement::Availability));
    }

    #[test]
    fn gate_check_rpc_resolves_catalog_id() {
        let v = gate_check(&json!({ "id": "qwen3-4b", "weightsPresent": true }));
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["model"], "qwen3-4b");
        assert_eq!(v["passed"], true);
        assert!(v["findings"].as_array().unwrap().len() == 6);
        // announced-but-missing 权重 → 阻断
        let v2 = gate_check(&json!({ "id": "qwen3-4b", "weightsPresent": false }));
        assert_eq!(v2["passed"], false);
        assert!(v2["blocking"].as_array().unwrap().contains(&json!("权重")));
        // 未知 id → 错误
        let v3 = gate_check(&json!({ "id": "nope" }));
        assert_eq!(v3["ok"], false);
        assert_eq!(v3["code"], "gate.unknown");
    }
}
