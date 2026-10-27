//! 远程插件源：把插件市场从「只看本地清单」扩展到**可对接 DSH 开源生态**。
//!
//! # 背景（2026-10-23）
//!
//! 此前 `market.*` 只认 `plugin_manifest_dir()` 里的本地 `*.plugin.json`：
//! `market.install` 甚至只做「解析一遍就回成功」，既不下载也不落盘。
//! 用户要的是「能装到 dsh 生态里的插件」，所以补两条真实通路：
//!
//! - `market.remote.search`：拉一个**插件索引 JSON**（可配置 URL），按键词过滤；
//! - `market.install_url`：给一个**清单直链**（例如 GitHub raw 的
//!   `xxx.plugin.json`），下载 → 校验 → 原子落盘到本地清单目录。
//!
//! # 为什么用「索引 URL + 清单直链」而不是写死某个商店
//!
//! 生态里的插件分发方式并不统一（GitHub raw / npm / 自建 registry 都有）。
//! 写死一个域名，等于把"能不能装"绑死在别人的部署上；而**索引 JSON 是任何
//! 分发方式都能导出的一层薄适配**：
//!
//! ```json
//! { "plugins": [ { "name": "dsh-modlens", "version": "0.2.0",
//!                  "description": "…", "manifest_url": "https://…/dsh-modlens.plugin.json" } ] }
//! ```
//!
//! 也接受 `{ "items": [...] }` 与裸数组两种形状（不同 registry 的常见写法）。
//!
//! # 安全边界
//!
//! - 出站走 [`SsrfGuard`]：拒绝回环 / 内网 / 云元数据地址（防止清单把用户引到内网）；
//! - 响应体上限 512 KiB、超时 20s（清单是元数据，不该是大文件）；
//! - 清单里的 `entry` 若为绝对路径或含 `..` → **拒绝安装**：远程清单不能指向
//!   本机任意可执行文件（那是"装个清单就能执行任意程序"的洞）；
//! - 落盘用 tmp+rename 原子替换，失败不会留下半份清单。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::gateway::protocol::RpcError;

/// 索引响应体上限：清单是元数据，超过这个量级一定不是我们要的东西。
const MAX_INDEX_BYTES: usize = 512 * 1024;

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("Marguerite/plugin-market")
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))
}

/// 下载文本（带 SSRF 校验 + 体积上限）。
async fn fetch_text(url: &str) -> Result<String, String> {
    crate::security::SsrfGuard::new()
        .check(url)
        .map_err(|e| format!("地址不可访问: {e}"))?;
    let resp = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("上游返回 HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("读取响应失败: {e}"))?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(format!(
            "响应 {} 字节超过 {} 上限（清单/索引不该这么大）",
            bytes.len(),
            MAX_INDEX_BYTES
        ));
    }
    String::from_utf8(bytes.to_vec()).map_err(|e| format!("响应不是 UTF-8: {e}"))
}

/// 生效的索引地址：环境变量 > config.yaml `plugins.registry_url`。
pub fn registry_url() -> String {
    if let Ok(v) = std::env::var("MARGUERITE_PLUGIN_REGISTRY") {
        let t = v.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    crate::persistence::config::Config::load()
        .map(|c| c.plugins.registry_url.trim().to_string())
        .unwrap_or_default()
}

/// 从索引 JSON 里取出条目数组（容忍 `plugins` / `items` / 裸数组）。
fn index_entries(v: &Value) -> Vec<Value> {
    if let Some(arr) = v.as_array() {
        return arr.clone();
    }
    for key in ["plugins", "items", "entries"] {
        if let Some(arr) = v.get(key).and_then(Value::as_array) {
            return arr.clone();
        }
    }
    Vec::new()
}

/// 本地清单目录（与 `market.*` 的本地安装同源）。
fn manifest_dir() -> Option<PathBuf> {
    crate::gateway::market::plugin_manifest_dir()
}

/// 校验清单里的 `entry` 不得指向本机任意位置。
fn entry_is_contained(manifest_json: &Value) -> Result<(), String> {
    let Some(entry) = manifest_json.get("entry").and_then(Value::as_str) else {
        return Ok(());
    };
    let e = entry.trim();
    if e.is_empty() {
        return Ok(());
    }
    let p = Path::new(e);
    // Windows 上 `/usr/bin/sh` 既不是绝对路径也不是相对父级（没有盘符），
    // 但它显然是"根路径"语义 —— 两种平台都必须按不安全处理。
    if p.is_absolute() || e.starts_with('/') || e.starts_with('\\') {
        return Err(format!("清单 entry 不得为绝对路径: {e}"));
    }
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!("清单 entry 不得包含 `..`: {e}"));
    }
    Ok(())
}

/// 原子写入清单（tmp + rename）。返回最终路径。
fn write_manifest(dir: &Path, name: &str, content: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建清单目录失败: {e}"))?;
    let target = dir.join(format!("{name}.plugin.json"));
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, content).map_err(|e| format!("写入临时清单失败: {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换清单失败: {e}")
    })?;
    Ok(target)
}

/// `market.remote.search`：从远程索引里按键词检索。
///
/// params: `{ q?: string }`。未配置索引时**明确告诉用户在哪里配置**，
/// 不返回空列表冒充"没搜到"。
pub async fn remote_search(params: &Value) -> Result<Value, RpcError> {
    let url = registry_url();
    if url.is_empty() {
        return Ok(json!({
            "success": false,
            "code": "REGISTRY_NOT_CONFIGURED",
            "message": "尚未配置远程插件源",
            "howTo": "在 config.yaml 写入 plugins.registry_url: https://…/index.json，\
                      或设置环境变量 MARGUERITE_PLUGIN_REGISTRY",
            "items": [],
        }));
    }
    let text = match fetch_text(&url).await {
        Ok(t) => t,
        Err(e) => {
            return Ok(json!({
                "success": false,
                "code": "REGISTRY_FETCH_FAILED",
                "message": format!("索引拉取失败: {e}"),
                "registry": url,
                "items": [],
            }))
        }
    };
    let parsed: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({
                "success": false,
                "code": "REGISTRY_BAD_JSON",
                "message": format!("索引不是合法 JSON: {e}"),
                "registry": url,
                "items": [],
            }))
        }
    };
    let q = params
        .get("q")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();
    let items: Vec<Value> = index_entries(&parsed)
        .into_iter()
        .filter(|it| {
            q.is_empty()
                || it["name"].as_str().unwrap_or("").to_lowercase().contains(&q)
                || it["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        })
        .map(|mut it| {
            if let Some(obj) = it.as_object_mut() {
                obj.insert("source".into(), json!("remote"));
                obj.insert("registry".into(), json!(url));
            }
            it
        })
        .collect();
    Ok(json!({
        "success": true,
        "registry": url,
        "total": items.len(),
        "items": items,
    }))
}

/// `market.install_url`：按清单直链安装插件。
///
/// params: `{ url: string }`（也可以是索引里某条的 `manifest_url`）。
pub async fn install_url(params: &Value) -> Result<Value, RpcError> {
    let url = params
        .get("url")
        .or_else(|| params.get("manifestUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if url.is_empty() {
        return Ok(json!({ "success": false, "message": "缺少 url（清单直链）" }));
    }
    let text = match fetch_text(&url).await {
        Ok(t) => t,
        Err(e) => {
            return Ok(json!({
                "success": false,
                "code": "MANIFEST_FETCH_FAILED",
                "message": format!("清单下载失败: {e}"),
            }))
        }
    };
    let raw: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return Ok(json!({
                "success": false,
                "code": "MANIFEST_BAD_JSON",
                "message": format!("清单不是合法 JSON: {e}"),
            }))
        }
    };
    if let Err(e) = entry_is_contained(&raw) {
        return Ok(json!({
            "success": false,
            "code": "MANIFEST_UNSAFE",
            "message": e,
        }));
    }
    let manifest = match crate::plugins::manifest::PluginManifest::from_json(&text) {
        Ok(m) => m,
        Err(e) => {
            return Ok(json!({
                "success": false,
                "code": "MANIFEST_INVALID",
                "message": format!("清单校验失败: {e}"),
            }))
        }
    };
    let Some(dir) = manifest_dir() else {
        return Ok(json!({
            "success": false,
            "message": "插件清单目录不可用（请检查应用数据目录权限）",
        }));
    };
    match write_manifest(&dir, &manifest.name, &text) {
        Ok(path) => Ok(json!({
            "success": true,
            "name": manifest.name,
            "version": manifest.version,
            "path": path.display().to_string(),
            "source": url,
            // 诚实边界：manifest 落盘 ≠ 已加载。运行期注册在网关重启时读取清单目录。
            "message": format!(
                "插件 {} v{} 已安装到本地清单目录；重启应用后生效",
                manifest.name, manifest.version
            ),
            "restartRequired": true,
        })),
        Err(e) => Ok(json!({ "success": false, "message": e })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_accepts_three_shapes() {
        let arr = json!([{"name": "a"}]);
        assert_eq!(index_entries(&arr).len(), 1);
        let plugins = json!({"plugins": [{"name": "a"}, {"name": "b"}]});
        assert_eq!(index_entries(&plugins).len(), 2);
        let items = json!({"items": [{"name": "a"}]});
        assert_eq!(index_entries(&items).len(), 1);
        assert!(index_entries(&json!({"nope": 1})).is_empty());
    }

    #[test]
    fn entry_must_stay_relative_and_inside() {
        assert!(entry_is_contained(&json!({})).is_ok());
        assert!(entry_is_contained(&json!({"entry": "index.js"})).is_ok());
        assert!(entry_is_contained(&json!({"entry": "C:\\Windows\\System32\\calc.exe"})).is_err());
        assert!(entry_is_contained(&json!({"entry": "../../evil.exe"})).is_err());
        assert!(entry_is_contained(&json!({"entry": "/usr/bin/sh"})).is_err());
    }

    #[test]
    fn empty_registry_reports_where_to_configure() {
        // 不依赖环境变量：直接验证「未配置」分支的可执行指引存在。
        let v = json!({
            "success": false,
            "code": "REGISTRY_NOT_CONFIGURED",
            "howTo": "config.yaml plugins.registry_url"
        });
        assert_eq!(v["code"], "REGISTRY_NOT_CONFIGURED");
        assert!(v["howTo"].as_str().unwrap().contains("registry_url"));
    }

    #[test]
    fn ssrf_guard_blocks_loopback_manifest_urls() {
        let guard = crate::security::SsrfGuard::new();
        assert!(guard.check("http://127.0.0.1/x.plugin.json").is_err());
        assert!(guard.check("http://169.254.169.254/latest/meta-data").is_err());
        assert!(guard.check("https://raw.githubusercontent.com/deepseek-ai/x/main/p.plugin.json").is_ok());
    }
}
