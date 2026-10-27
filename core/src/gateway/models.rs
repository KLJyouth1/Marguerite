//! 模型档案方法族（models.profiles.*）
//!
//! 管理用户与内置模型档案：列出、切换主模型、设置/关闭 Bloom 副模型、新增/删除自定义档案。
//! 持久化到 `models.json`（ModelSelector）。

use serde_json::{json, Value};

use crate::gateway::_helpers::{param_str, param_str_keys};

/// `models.list`：Rust 离线回退——模型档案目录 + 当前生效模型。
///
/// 此前该方法仅由 Python 侧车提供，侧车离线时返回 NOT_IMPLEMENTED，
/// 前端模型选择器为空。现在回退 Rust 实现：数据源与 `models.profiles.list`
/// 同一注册表（models.json），另附 config.yaml 生效的 provider/model_name。
pub fn models_list() -> Value {
    use crate::model::selector::ModelSelector;
    let cfg = crate::persistence::config::Config::load().ok();
    let current = cfg.as_ref().map(|c| {
        json!({
            "provider": c.model.provider,
            "model": c.model.model_name,
            "baseUrl": c.model.base_url,
            "apiKeyConfigured": !c.model.api_key.is_empty(),
        })
    });
    let profiles: Vec<Value> = match ModelSelector::load() {
        Ok(sel) => sel
            .registry()
            .list()
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "label": p.label,
                    "provider": p.provider,
                    "model": p.model,
                    "kind": format!("{:?}", p.kind),
                    "builtin": p.builtin,
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    json!({
        "ok": true,
        "source": "rust",
        "current": current,
        "models": profiles,
        "profiles": profiles,
    })
}

/// `models.config_path`：本地模型配置文件（`models.json`）的绝对路径。
///
/// 为什么由 Rust 给路径而不是前端自己拼：`models.json` 位于 `Config::config_dir()`，
/// 该目录取决于运行形态（用户级 `~/.marguerite` / 便携版 / 测试注入），只有 Rust 侧知道真值。
/// 前端拿到它之后，用**既有的** `artifacts.open_in_system { path }` 交给系统默认程序打开
/// —— 该 RPC 已支持裸 `path`（见 `gateway/artifacts.rs`），因此无需新增"打开文件"通道，
/// 也不用在前端硬编码任何绝对路径。
///
/// 只读、无副作用。文件不存在时如实回传 `exists=false`，不谎称"可打开"。
pub fn models_config_path() -> Value {
    let path = crate::persistence::config::Config::config_dir().join("models.json");
    json!({
        "ok": true,
        "path": path.display().to_string(),
        "exists": path.is_file(),
    })
}

/// `models.profiles.list`：列出 Rust 模型档案（ModelSelector/models.json），WebUI 可视。
///
/// 每条档案回传两组能力：
/// - `capabilities`：用户声明的原始值（`declared=false` 时全 false，前端据此回填勾选框为「未声明」）；
/// - `effectiveCapabilities`：真正生效的能力（声明优先，否则按模型名识别）。
///
/// 只给一组都会让前端"想显示自动识别结果，却把识别结果当成用户已声明"而写回磁盘。
pub fn models_profiles_list() -> Value {
    use crate::model::registry::ProfileKind;
    use crate::model::selector::ModelSelector;
    match ModelSelector::load() {
        Ok(sel) => {
            let reg = sel.registry();
            let profiles: Vec<Value> = reg
                .list()
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "label": p.label,
                        "provider": p.provider,
                        "model": p.model,
                        "kind": format!("{:?}", p.kind),
                        "source": if p.kind == ProfileKind::Local { "local" } else { "cloud" },
                        "builtin": p.builtin,
                        "active": reg.active == p.id,
                        "copilot": reg.copilot.as_deref() == Some(&p.id),
                        "capabilities": p.capabilities,
                        "effectiveCapabilities": p.effective_capabilities(),
                    })
                })
                .collect();
            json!({
                "ok": true,
                "active": reg.active,
                "copilot": reg.copilot,
                "profiles": profiles,
            })
        }
        Err(e) => json!({ "ok": false, "error": e.to_string(), "profiles": [] }),
    }
}

/// models.profiles.set_capabilities：保存某个档案的多模态能力声明（含端点覆盖与自定义参数）。
///
/// 单独一条 RPC 而不是复用 `onboarding.llmProfile.upsert`：后者要求同时提供
/// provider/model/baseUrl，会让「只想改一个勾选框」的前端被迫重发整份档案，
/// 任何一项取值漂移都会顺带改写凭据与端点。这条只动 `capabilities` 一个字段。
///
/// `reset=true` → 清除声明，回到自动识别（写 `declared=false` 的空结构）。
pub fn models_profiles_set_capabilities(params: &Value) -> Value {
    use crate::model::registry::ModelCapabilities;
    use crate::model::selector::ModelSelector;

    let id = param_str(params, "id", "id", "").trim().to_string();
    if id.is_empty() {
        return json!({ "ok": false, "error": "缺少 id" });
    }
    let reset = params.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let caps = if reset {
        ModelCapabilities::default()
    } else {
        match params.get("capabilities") {
            Some(v) => ModelCapabilities::from_json(v),
            None => return json!({ "ok": false, "error": "缺少 capabilities（或传 reset=true）" }),
        }
    };

    let mut selector = match ModelSelector::load() {
        Ok(s) => s,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let Some(mut profile) = selector.registry().get(&id).cloned() else {
        return json!({ "ok": false, "error": format!("档案不存在: {id}") });
    };
    profile.capabilities = caps;
    selector.registry_mut().upsert(profile);
    if let Err(e) = selector.save() {
        return json!({ "ok": false, "error": e.to_string() });
    }
    let mut out = models_profiles_list();
    out["ok"] = json!(true);
    out
}


/// `model.deepseek.render`：把一段对话按 DeepSeek 官方 recipe 渲染成 V4.1 prompt。
///
/// 用途是**诊断与自托管校验**（DeepInfer 服务接进来后，模型到底收到什么 prompt
/// 此前是黑盒）。不发请求、不碰磁盘，纯函数。
///
/// params: `{ messages: [{role, content}...], model?: string }`
///
/// 返回体里的 `tokensEstimate` 是**估算**（复用 `context::session` 的既有口径），
/// 刻意不命名为 `tokens`，避免被误读成精确计数。
pub fn deepseek_render_prompt(params: &Value) -> Value {
    let messages = params.get("messages").cloned().unwrap_or(Value::Null);
    let model = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match crate::model::deepseek_recipe::render_v41_prompt(&messages, model) {
        Ok(prompt) => {
            let (exact, exact_err) = match crate::model::tokenizer::count_exact(&prompt) {
                Ok(n) => (json!(n), Value::Null),
                Err(e) => (Value::Null, json!(e)),
            };
            json!({
                "ok": true,
                "model": model.unwrap_or("DeepSeek-V4.1-Flash"),
                "chars": prompt.chars().count(),
                "tokensEstimate": crate::model::deepseek_recipe::estimate_tokens(&prompt),
                // 精确值：词表可用时给数字，不可用时为 null 并在 tokensExactError 里说明原因。
                "tokensExact": exact,
                "tokensExactError": exact_err,
                "prompt": prompt,
            })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// `model.deepseek.parse`：把模型返回的 DeepSeek V4 / V4.1 原始输出解析成结构化 JSON。
///
/// 与 `model.deepseek.render` 成对：render 看「发出去的 prompt」，parse 看
/// 「收回来的原始输出」，用途是自托管 DeepInfer / 量化评测时核对模板对齐。
/// **不参与正常对话链路**（对话链路的解析由上游 OpenAI 兼容层负责）。
///
/// params: `{ raw: string }`
///
/// 返回：`{ ok: true, messages, reasoning, answer, tokensFound, tokensEstimate }`；
/// `raw` 缺失、非字符串或空白时返回 `{ ok: false, error }`，不 panic。
/// `tokensEstimate` 是对 `raw` 的**估算**，不是精确 token 数。
pub fn deepseek_parse_response(params: &Value) -> Value {
    let raw = match params.get("raw") {
        Some(Value::String(s)) => s,
        Some(_) => return json!({ "ok": false, "error": "raw 必须是字符串" }),
        None => return json!({ "ok": false, "error": "缺少 raw" }),
    };
    if raw.trim().is_empty() {
        return json!({ "ok": false, "error": "raw 不能为空" });
    }
    match crate::model::deepseek_recipe::parse_v41_response(raw) {
        Ok(mut parsed) => {
            if let Some(obj) = parsed.as_object_mut() {
                obj.insert("ok".to_string(), json!(true));
                obj.insert(
                    "tokensEstimate".to_string(),
                    json!(crate::model::deepseek_recipe::estimate_tokens(raw)),
                );
                match crate::model::tokenizer::count_exact(raw) {
                    Ok(n) => {
                        obj.insert("tokensExact".to_string(), json!(n));
                        obj.insert("tokensExactError".to_string(), Value::Null);
                    }
                    Err(e) => {
                        obj.insert("tokensExact".to_string(), Value::Null);
                        obj.insert("tokensExactError".to_string(), json!(e));
                    }
                }
            }
            parsed
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// models.profiles.set_active：切换主模型（persist models.json 后回读列表）。
pub fn models_profiles_set_active(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return json!({ "ok": false, "error": "缺少 id" });
    }
    match crate::model::selector::ModelSelector::load().and_then(|mut s| {
        s.use_active(id)?;
        Ok(())
    }) {
        Ok(()) => models_profiles_list(),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// models.profiles.set_copilot：设置 / 关闭 Bloom 副模型（id 缺省/空 = 关闭）。
pub fn models_profiles_set_copilot(params: &Value) -> Value {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .map(String::from)
        .filter(|s| !s.is_empty());
    match crate::model::selector::ModelSelector::load().and_then(|mut s| {
        s.use_copilot(id)?;
        Ok(())
    }) {
        Ok(()) => models_profiles_list(),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// 解析「新增档案」是否命中 shipped gate（纯函数，不含文件 IO，便于单测）。
///
/// 仅当档案指向一个「已声明权重的本地/第三方模型」时才返回放行报告：
/// - 显式 `catalogId` 命中聚合目录 `LocalModelEntry`；
/// - 或 `kind=Local` 且 `id`/`model` 命中目录项的 `id`/`ollama_tag`/`hf_repo`/`msc_repo`。
///
/// 返回 `None` 表示不涉及 gate（纯云 API 端点，无权重声明，无法诚实评估）。
/// 命中时返回 `{ passed, blocking, findings }` 报告，放行/阻断由调用方据
/// `requireGate`/`force` 决定（本函数只诚实计算结论）。
pub(crate) fn eval_profile_gate(
    id: &str,
    model: &str,
    kind: crate::model::registry::ProfileKind,
    params: &Value,
) -> Option<Value> {
    let cat = crate::gateway::local_models::local_catalog();
    let entry = if let Some(cid) = params.get("catalogId").and_then(Value::as_str) {
        cat.into_iter().find(|e| e.id == cid)
    } else if kind == crate::model::registry::ProfileKind::Local {
        cat.into_iter().find(|e| {
            e.id == id || e.ollama_tag == model || e.hf_repo == model || e.msc_repo == model
        })
    } else {
        return None;
    }?;
    let weights_present = params
        .get("weightsPresent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let runtime_supports = params
        .get("runtimeSupports")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let gate =
        crate::gateway::shipped_gate::gate_local_model(&entry, weights_present, runtime_supports);
    Some(json!({
        "passed": gate.passed,
        "blocking": gate.blocking.iter().map(|e| e.name()).collect::<Vec<_>>(),
        "findings": gate.findings.iter().map(|f| json!({
            "element": f.element.name(),
            "verdict": f.verdict.as_str(),
            "detail": f.detail,
        })).collect::<Vec<_>>(),
    }))
}

/// 解析 `base_url`：按别名 `baseUrl` / `baseUri` / `base_url` 依次取（首命中优先），
/// 内置 provider 有默认兜底；其余情况缺失返回 `None`（调用方据此报「缺少 base_url」）。
///
/// 纯函数（无 IO），专为解决历史命名不一致——前端发 camelCase `baseUrl`、
/// 早期调用方发 `baseUri`、持久化用 `base_url`，三者都必须被接受。
pub(crate) fn resolve_base_url(provider: &str, params: &Value) -> Option<String> {
    let explicit = param_str_keys(params, &["baseUrl", "baseUri", "base_url"], "")
        .trim()
        .to_string();
    if !explicit.is_empty() {
        return Some(explicit);
    }
    // 内置 provider 默认值兜底；custom / 其它无默认，缺失即返回 None。
    match provider {
        "openai" => Some("https://api.openai.com/v1".to_string()),
        "deepseek" => Some("https://api.deepseek.com".to_string()),
        "anthropic" => Some("https://api.anthropic.com".to_string()),
        "ollama" => Some("http://localhost:11434/v1".to_string()),
        _ => None,
    }
}

/// models.profiles.add：新增自定义模型档案（WebUI 表单；API Key 不随 RPC 回传）。
pub fn models_profiles_add(params: &Value) -> Value {
    use crate::model::registry::{ModelProfile, ProfileKind};
    let id = param_str(params, "id", "id", "").trim().to_string();
    if id.is_empty() {
        return json!({ "ok": false, "error": "缺少 id" });
    }
    let provider = param_str(params, "provider", "provider", "custom")
        .trim()
        .to_string();
    let model = param_str(params, "model", "model", "").trim().to_string();
    if model.is_empty() {
        return json!({ "ok": false, "error": "缺少 model" });
    }
    let base_url = match resolve_base_url(&provider, params) {
        Some(u) => u,
        None => return json!({ "ok": false, "error": "缺少 base_url（或使用内置 provider）" }),
    };
    let label = param_str(params, "label", "label", id.as_str()).to_string();
    let kind = match param_str(params, "kind", "kind", "cloud").trim() {
        "local" => ProfileKind::Local,
        "custom" => ProfileKind::Custom,
        _ => ProfileKind::Cloud,
    };

    // ── shipped gate 前移（蓝图纪律：第三方模型接入路由前须过 gate）──
    // 仅当档案指向一个「已声明权重的本地/第三方模型」时才校验：显式 `catalogId`，
    // 或 `kind=Local` 且 `model`/标签命中聚合目录条目。「纯云 API 端点」（无权重声明）
    // 不参与 gate——我们无法诚实评估其「权重是否实际落地」（宣布 ≠ 可部署）。
    // `eval_profile_gate` 为纯函数（无文件 IO），命中目录时返回放行报告，否则 `None`。
    let gate_payload: Option<Value> = eval_profile_gate(&id, &model, kind, params);
    if let Some(gp) = &gate_payload {
        let require_gate = params
            .get("requireGate")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let force = params
            .get("force")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let passed = gp.get("passed").and_then(Value::as_bool).unwrap_or(false);
        if !passed && require_gate && !force {
            // 阻断性要素（如权重宣布但未实际下载）→ 拒绝注册到路由。
            return json!({
                "ok": false,
                "error": "未通过 shipped gate，禁止注册到路由（宣布 ≠ 可部署）",
                "code": "gate.blocked",
                "gate": gp.clone(),
            });
        }
    }

    let profile = ModelProfile {
        id: id.clone(),
        label,
        provider,
        base_url,
        api_key: String::new(),
        api_key_env: String::new(),
        model,
        kind,
        builtin: false,
        temperature: 0.7,
        max_tokens: 4096,
        tier: crate::model::tier::ModelTier::Balanced,
        tier_caps: crate::model::tier::TierCaps::default(),
        tier_caps_declared: false,
        // 目录部署路径（本地权重）：只走 Chat Completions。
        wire_api: crate::model::wire::WireApi::Chat,
        // 自定义接入模型时用户可勾选/填写多模态能力；未填 → 走名字自动识别。
        capabilities: match params.get("capabilities") {
            Some(v) => crate::model::registry::ModelCapabilities::from_json(v),
            None => Default::default(),
        },
    };
    match crate::model::selector::ModelSelector::load().and_then(|mut s| {
        s.add(profile)?;
        Ok(())
    }) {
        Ok(()) => {
            let mut out = models_profiles_list();
            // 命中目录项时把 gate 校验结论一并回传，前端可透明展示放行状态。
            if let Some(gp) = &gate_payload {
                out["gate"] = gp.clone();
            }
            out
        }
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

/// models.profiles.remove：删除用户档案（内置档案受 registry 最少保留保护）。
pub fn models_profiles_remove(params: &Value) -> Value {
    let id = param_str(params, "id", "id", "").trim();
    if id.is_empty() {
        return json!({ "ok": false, "error": "缺少 id" });
    }
    match crate::model::selector::ModelSelector::load().and_then(|mut s| {
        s.remove(id)?;
        Ok(())
    }) {
        Ok(()) => models_profiles_list(),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::registry::ProfileKind;

    #[test]
    fn item10_chain_config_path_then_open_in_system() {
        // item 10 能力链窄指令验证（集成测试级证据）：
        // models.config_path →（拿到真实路径）→ artifacts.open_in_system {path}
        // 直接调 RPC handler（dispatch 同源实现），不起网络。
        use crate::persistence::config::Config;

        // 隔离配置目录：每个用例独立目录，避免并行串扰（同 methods.rs ctx() 模式）
        let dir = std::env::temp_dir().join(format!("item10-chain-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        Config::set_config_dir_override(dir.clone());

        // ① models.config_path：路径必须落在隔离配置目录内，存在性如实回传
        let cfg = models_config_path();
        assert_eq!(cfg["ok"], true, "config_path 应 ok: {cfg}");
        let path = cfg["path"].as_str().expect("path 应为字符串");
        let expected = dir.join("models.json");
        assert_eq!(
            std::path::Path::new(path),
            expected.as_path(),
            "config_path 必须指向 config_dir/models.json"
        );
        assert_eq!(
            cfg["exists"],
            expected.is_file(),
            "exists 字段必须如实反映磁盘状态"
        );

        // ② 直接落地 models.json（ModelSelector 写路径），让 open 有真实目标
        let _ = crate::model::registry::ModelRegistry::save(
            &crate::model::registry::ModelRegistry::with_seeds(),
            &expected,
        );
        assert!(expected.is_file(), "models.json 应已落盘");

        // ③ artifacts.open_in_system {path}：W3.2 之后裸路径只允许工作区内的文件。
        //    配置目录在 %TEMP%（工作区之外）→ 必须拒绝，且**不得**真的启动系统程序。
        let res = crate::gateway::artifacts::artifacts_open_in_system(
            &json!({ "path": path }),
        );
        assert_eq!(res["code"], 1403, "工作区外的裸路径应拒 1403: {res}");
        assert!(
            res["message"]
                .as_str()
                .unwrap_or("")
                .contains("PATH_NOT_ALLOWED_FOR_OPEN"),
            "错误信息必须带稳定可识别的码: {res}"
        );

        // ③b 已登记产物（有 id）允许在台账目录（即使在工作区外），但仍受可执行拦截。
        //     这里不真的打开：用一个不存在的 id 验证「未登记 → 1404」，避免测试拉起 GUI。
        let unknown = crate::gateway::artifacts::artifacts_open_in_system(
            &json!({ "id": "artifact:probe:missing" }),
        );
        assert_eq!(unknown["code"], 1404, "未登记产物应拒 1404: {unknown}");

        // ④ 路径校验反例：不存在的路径必须被拒（1404，不落 OS 打开）
        let bad = crate::gateway::artifacts::artifacts_open_in_system(
            &json!({ "path": dir.join("__item10_not_exist__.txt").display().to_string() }),
        );
        assert_eq!(bad["code"], 1404, "不存在路径应拒 1404: {bad}");

        // ⑤ 缺参反例：id/path 全空必须被拒（1401）
        let none = crate::gateway::artifacts::artifacts_open_in_system(&json!({}));
        assert_eq!(none["code"], 1401, "缺参应拒 1401: {none}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn eval_gate_skips_pure_cloud_profile() {
        // 纯云 API 端点（无权重声明）不涉及 gate。
        let p = json!({ "provider": "openai", "model": "gpt-4o" });
        assert!(
            eval_profile_gate("cloud-1", "gpt-4o", ProfileKind::Cloud, &p).is_none(),
            "纯云档案不应触发 gate"
        );
    }

    #[test]
    fn eval_gate_blocks_local_without_weights() {
        // kind=Local 且 model 命中目录 ollama_tag；权重未实际下载 → 阻断（宣布 ≠ 可部署）。
        let p = json!({ "model": "qwen3.8:27b" });
        let g = eval_profile_gate("local-qwen3", "qwen3.8:27b", ProfileKind::Local, &p)
            .expect("应命中聚合目录");
        assert_eq!(g["passed"], false, "权重宣布但未就位应判阻断: {g}");
        assert!(g["blocking"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == "权重"));
    }

    #[test]
    fn eval_gate_passes_local_with_weights_present() {
        // 权重已实际落地 + 运行时支持 → 过 gate。
        let p = json!({ "model": "qwen3.8:27b", "weightsPresent": true, "runtimeSupports": true });
        let g = eval_profile_gate("local-qwen3", "qwen3.8:27b", ProfileKind::Local, &p)
            .expect("应命中聚合目录");
        assert_eq!(g["passed"], true, "权重就位应过 gate: {g}");
    }

    #[test]
    fn eval_gate_reports_blocked_when_force_later() {
        // force 由调用方决定放行；这里确认报告本身仍为 blocked，便于前端透明展示。
        let p = json!({ "model": "qwen3.8:27b", "weightsPresent": false });
        let g = eval_profile_gate("local-qwen3", "qwen3.8:27b", ProfileKind::Local, &p)
            .expect("应命中聚合目录");
        assert_eq!(g["passed"], false);
    }

    #[test]
    fn profiles_add_rejects_local_without_weights() {
        // 集成验证：models.profiles.add 在写入前即阻断（返回早，无文件 IO）。
        let r = models_profiles_add(&json!({
            "id": "local-qwen3",
            "provider": "ollama",
            "model": "qwen3.8:27b",
            "baseUri": "http://localhost:11434/v1",
            "kind": "local",
        }));
        assert_eq!(r["ok"], false, "未过 gate 应拒绝注册: {r}");
        assert_eq!(r["code"], "gate.blocked");
        assert_eq!(r["gate"]["passed"], false);
        assert!(r["gate"]["blocking"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == "权重"));
    }

    // ── resolve_base_url：修复「baseUrl 别名不被接受」的潜伏 bug ──

    #[test]
    fn resolve_base_url_accepts_frontend_base_url_alias() {
        // 关键修复：前端发 camelCase `baseUrl`，后端须兼容，否则 provider=custom 时恒报「缺少 base_url」。
        assert_eq!(
            resolve_base_url("custom", &json!({ "baseUrl": "http://localhost:9999/v1" })),
            Some("http://localhost:9999/v1".to_string())
        );
        // 旧调用方发 baseUri / 持久化 base_url 仍须生效。
        assert_eq!(
            resolve_base_url("custom", &json!({ "baseUri": "http://x/v1" })),
            Some("http://x/v1".to_string())
        );
        assert_eq!(
            resolve_base_url("custom", &json!({ "base_url": "http://y/v1" })),
            Some("http://y/v1".to_string())
        );
    }

    #[test]
    fn resolve_base_url_falls_back_to_builtin_provider() {
        // 内置 provider 有默认兜底，缺省也可注册。
        assert_eq!(
            resolve_base_url("openai", &json!({})),
            Some("https://api.openai.com/v1".to_string())
        );
        assert_eq!(
            resolve_base_url("ollama", &json!({})),
            Some("http://localhost:11434/v1".to_string())
        );
    }

    #[test]
    fn resolve_base_url_missing_for_custom() {
        // 非内置 provider 且无显式 url → 返回 None（调用方据此报「缺少 base_url」）。
        assert_eq!(resolve_base_url("custom", &json!({})), None);
        assert_eq!(resolve_base_url("unknown", &json!({})), None);
    }

    // ── DeepSeek recipe RPC：渲染带估算字段 / 解析拒绝坏入参 ──

    #[test]
    fn deepseek_render_reports_tokens_estimate() {
        let r = deepseek_render_prompt(&json!({
            "messages": [{ "role": "user", "content": "你好" }]
        }));
        assert_eq!(r["ok"], true, "应渲染成功: {r}");
        // 字段名必须是 tokensEstimate（估算），不能被误读为精确 tokens 计数。
        assert!(
            r["tokensEstimate"].as_u64().is_some_and(|n| n >= 1),
            "tokensEstimate 应为正整数: {r}"
        );
        assert!(r.get("tokens").is_none(), "不得输出精确计数字段 tokens: {r}");
    }

    #[test]
    fn deepseek_parse_rpc_rejects_bad_raw() {
        for params in [json!({}), json!({ "raw": 42 }), json!({ "raw": "   " })] {
            let r = deepseek_parse_response(&params);
            assert_eq!(r["ok"], false, "坏入参应返回 ok:false（不得 panic）: {r}");
            assert!(
                r["error"].as_str().is_some_and(|s| !s.is_empty()),
                "应给出可读错误: {r}"
            );
        }
        let r = deepseek_parse_response(&json!({ "raw": "<｜User｜>问<｜Assistant｜>答" }));
        assert_eq!(r["ok"], true, "正常入参应 ok:true: {r}");
        assert_eq!(r["answer"], "答");
        assert!(r["tokensEstimate"].as_u64().is_some_and(|n| n >= 1));
    }
}

// ---------------------------------------------------------------------------
// 本地模型存在性校验（2026-10-22）
// ---------------------------------------------------------------------------

/// provider 是否属于「本地推理运行时」（只有这些才需要查 Ollama 的 tag 列表）。
fn is_ollama_like(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "ollama" | "llamacpp" | "llama.cpp" | "llama_cpp" | "lmstudio" | "koboldcpp" | "local" | "ggml"
    )
}

/// 把档案里的 `base_url` 归一成 Ollama 的根地址。
///
/// 档案里常见两种写法：`http://localhost:11434` 与 `http://localhost:11434/v1`
/// （后者是 OpenAI 兼容路径）。`fetch_ollama_tags` 会拼 `/api/tags`，
/// 若不剥掉 `/v1` 就会请求 `/v1/api/tags` —— 404，于是"探测失败"被误读成"模型不存在"。
fn normalize_ollama_root(base_url: &str) -> String {
    let s = base_url.trim().trim_end_matches('/');
    let s = s.strip_suffix("/v1").unwrap_or(s);
    let s = s.trim_end_matches('/');
    if s.is_empty() {
        "http://localhost:11434".to_string()
    } else {
        s.to_string()
    }
}

/// 目标模型名是否在 Ollama 的 tag 列表里。
///
/// Ollama 会把无 tag 的模型名补成 `:latest`（`omnipilot` ≡ `omnipilot:latest`），
/// 所以两侧都做一次 `:latest` 归一，避免"用户填 omnipilot、列表里是 omnipilot:latest"
/// 被误判为不存在。
fn local_model_exists(want: &str, available: &[String]) -> bool {
    let w = want.trim();
    if w.is_empty() {
        return true;
    }
    available.iter().any(|a| {
        let a = a.trim();
        a == w
            || a.strip_suffix(":latest") == Some(w)
            || w.strip_suffix(":latest") == Some(a)
    })
}

/// 保存 / 激活本地模型前的存在性校验。
///
/// 返回 `Some(Value)` = **应当拒绝本次调用**（值即 RPC 响应体，`ok:false` + 可操作提示）。
/// 返回 `None` = 放行。
///
/// # 为什么需要它（2026-10-22 实测）
///
/// 用户把 `qwen3.8:27b` 加进路由候选并保存成功，但该模型**在 Ollama 里根本不存在**
/// （`/api/tags` 只有 16 个模型，没有它）。应用照收不误，用户以为配好了，
/// 之后怎么都"接不上"—— 因为任何路由都不可能选中一个不存在的模型。
///
/// 保存时校验一次，就能把这类"静默假成功"变成一句可操作的报错。
///
/// # 三态判据（不许默认成功，也不许默认失败）
///
/// | 探测结果 | 处置 | 理由 |
/// |---|---|---|
/// | Ollama 可达且模型存在 | 放行 | — |
/// | Ollama 可达但模型不存在 | **拒绝** | 有确定证据，必须拦 |
/// | Ollama 不可达 | 放行 | 拿不到"确定不存在"的证据。用户可能先存档案、稍后再启动 Ollama |
pub async fn precheck_local_model(method: &str, params: &Value) -> Option<Value> {
    let (provider, model, base_url) = match method {
        "models.profiles.add" => (
            param_str(params, "provider", "provider", "custom")
                .trim()
                .to_string(),
            param_str(params, "model", "model", "").trim().to_string(),
            param_str(params, "baseUrl", "base_url", "").trim().to_string(),
        ),
        "models.profiles.set_active" => {
            let id = param_str(params, "id", "id", "").trim().to_string();
            if id.is_empty() {
                return None;
            }
            let Ok(sel) = crate::model::selector::ModelSelector::load() else {
                return None;
            };
            let p = sel.registry().get(&id).cloned()?;
            (p.provider.trim().to_string(), p.model.trim().to_string(), p.base_url.trim().to_string())
        }
        _ => return None,
    };

    if !is_ollama_like(&provider) || model.is_empty() {
        return None;
    }
    let root = normalize_ollama_root(if base_url.is_empty() {
        "http://localhost:11434"
    } else {
        &base_url
    });

    let tags = match crate::gateway::local_models::fetch_ollama_tags(&root).await {
        Ok(t) => t,
        // Ollama 不可达：放行（见函数文档的三态判据）。
        Err(_) => return None,
    };
    let names: Vec<String> = tags
        .iter()
        .filter_map(|t| {
            t.get("name")
                .or_else(|| t.get("model"))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
        })
        .collect();
    if local_model_exists(&model, &names) {
        return None;
    }
    Some(json!({
        "ok": false,
        "code": "local_model.not_found",
        "error": format!(
            "本地模型 `{model}` 不在 Ollama 里（{root}）。已安装的：{}",
            if names.is_empty() { "（无）".to_string() } else { names.join("、") }
        ),
        "hint": "先 `ollama pull <模型>`，或改用上面已存在的模型名。",
        "available": names,
    }))
}

#[cfg(test)]
mod precheck_tests {
    use super::*;

    #[test]
    fn ollama_root_normalization_strips_v1_and_trailing_slash() {
        assert_eq!(normalize_ollama_root("http://localhost:11434"), "http://localhost:11434");
        assert_eq!(normalize_ollama_root("http://localhost:11434/"), "http://localhost:11434");
        // 档案里最常见的写法：OpenAI 兼容路径
        assert_eq!(normalize_ollama_root("http://localhost:11434/v1"), "http://localhost:11434");
        assert_eq!(normalize_ollama_root("http://localhost:11434/v1/"), "http://localhost:11434");
        // 空值回落到默认端口
        assert_eq!(normalize_ollama_root(""), "http://localhost:11434");
        assert_eq!(normalize_ollama_root("   "), "http://localhost:11434");
    }

    #[test]
    fn model_existence_treats_latest_as_equivalent() {
        let avail = vec!["omnipilot:latest".to_string(), "qwen3:4b".to_string()];
        assert!(local_model_exists("omnipilot:latest", &avail));
        assert!(local_model_exists("omnipilot", &avail));
        assert!(local_model_exists("qwen3:4b", &avail));
        // 这正是用户踩到的那个：配了不存在的大模型
        assert!(!local_model_exists("qwen3.8:27b", &avail));
        // 前缀相同但 tag 不同，不能算命中
        assert!(!local_model_exists("qwen3:0.6b", &avail));
        // 空名字不拦（交给上层报"缺少 model"）
        assert!(local_model_exists("", &avail));
    }

    #[test]
    fn only_local_runtimes_are_checked() {
        assert!(is_ollama_like("ollama"));
        assert!(is_ollama_like("OLLAMA"));
        assert!(is_ollama_like("lmstudio"));
        // 云厂商不该走 Ollama 探测
        assert!(!is_ollama_like("deepseek"));
        assert!(!is_ollama_like("openai"));
        assert!(!is_ollama_like("custom"));
    }
}
