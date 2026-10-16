//! 产物台账（Epic A）RPC：列出 / 查询 / 登记 / 系统打开 / 内联编辑。
//!
//! 与 `tools::file_change::file_change` 同数据源，面向前端「产物抽屉」：
//! `artifacts.list` 游标分页、`artifacts.get` 单条详情、`artifacts.register`
//! 显式登记、`artifacts.open_in_system` 调 OS 默认程序打开、
//! `artifacts.edit_inline` 写回文本文件内容。
//!
//! 错误码（历史固定，前端兼容）：
//! - 1401  缺少 id 或 path
//! - 1404  产物不存在 / 路径不存在
//! - 1502  系统打开失败
//! - 1503  内联编辑失败

use axum::body::Body;
use axum::body::Bytes as AxumBytes;
use axum::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::gateway::_helpers::{open_in_system, rpc_err, rpc_ok};
use crate::gateway::server::GatewayState;
use crate::tools::file_change::ArtifactEntry;

/// `artifacts.list`：返回指定会话中 Agent 生成的产物文件列表（游标分页）。
///
/// 参数：`sessionKey`/`session_key`/`session`（三选一）、`cursor`（上一页末条 ts）、`limit`（默认 50）。
/// 返回 `{artifacts, has_more, next_cursor}`，兼容既有前端调用（无 cursor/limit 时退化为全量首页）。
pub fn artifacts_list(params: &Value) -> Value {
    let session = params
        .get("sessionKey")
        .or_else(|| params.get("session_key"))
        .or_else(|| params.get("session"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let cursor = params.get("cursor").and_then(Value::as_u64);
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(50);
    let (items, has_more, next_cursor) =
        crate::tools::file_change::list_artifacts(session, cursor, limit);
    json!({
        "artifacts": items.iter().map(|a| json!({
            "id": a.id,
            "name": a.name,
            "path": a.path,
            "kind": a.kind,
            "size": a.size,
            "ts": a.ts,
            "mime": a.mime.as_deref().unwrap_or(""),
            "session_key": a.session_key,
            "sha256": a.sha256.as_deref().unwrap_or(""),
        })).collect::<Vec<_>>(),
        "has_more": has_more,
        "next_cursor": next_cursor,
    })
}

fn artifact_value(a: &ArtifactEntry) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "path": a.path,
        "kind": a.kind,
        "size": a.size,
        "ts": a.ts,
        "mime": a.mime.as_deref().unwrap_or(""),
        "session_key": a.session_key,
        "sha256": a.sha256.as_deref().unwrap_or(""),
    })
}

/// `artifacts.get`：按 id 返回完整产物条目（含 mime / session_key）。
pub fn artifacts_get(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return rpc_err(1401, "缺少 artifact id");
    }
    match crate::tools::file_change::get_artifact(id) {
        Some(a) => rpc_ok(artifact_value(&a)),
        None => rpc_err(1404, "产物不存在"),
    }
}

/// `artifacts.register`：显式重新登记产物（R1 回退通道：调用方传入 session_key + path）。
pub fn artifacts_register(params: &Value) -> Value {
    let session_key = params
        .get("session_key")
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("registered");
    if session_key.is_empty() {
        return rpc_err(1401, "缺少 session_key");
    }
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return rpc_err(1404, "路径不存在或不是文件");
    }
    crate::tools::file_change::record_artifact_with_session(session_key, path, kind);
    let id = format!("artifact:{}:{}", session_key, path);
    match crate::tools::file_change::get_artifact(&id) {
        Some(a) => rpc_ok(json!({
            "id": a.id,
            "path": a.path,
            "kind": a.kind,
            "mime": a.mime.as_deref().unwrap_or(""),
            "session_key": a.session_key,
            "sha256": a.sha256.as_deref().unwrap_or(""),
        })),
        None => rpc_err(1404, "登记失败"),
    }
}

/// `artifacts.open_in_system`：调用 OS 默认程序打开已登记产物。
/// 无 id 时退化为直接按 `path` 打开（前端降级路径）。
pub fn artifacts_open_in_system(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    let path = params.get("path").and_then(Value::as_str).unwrap_or("");
    let target = if !id.is_empty() {
        match crate::tools::file_change::get_artifact(id) {
            Some(a) => a.path,
            None => return rpc_err(1404, "产物不存在"),
        }
    } else if !path.is_empty() {
        if !std::path::Path::new(path).is_file() {
            return rpc_err(1404, "路径不存在或不是文件");
        }
        path.to_string()
    } else {
        return rpc_err(1401, "缺少 artifact id 或 path");
    };
    if open_in_system(&target) {
        rpc_ok(json!({ "id": id, "path": target }))
    } else {
        rpc_err(1502, "系统默认程序打开失败")
    }
}

/// `artifacts.edit_inline`：原地覆写已登记产物内容（带产物归属登记 + 竞态防护）。
///
/// 竞态防护（Epic A 前端契约）：前端在打开内联编辑器时记录文件 `sha256`，保存时通过
/// `expectedSha256` 回传。写入前比对磁盘当前内容哈希——若不一致，说明文件在用户编辑
/// 期间已被其它 turn 重新生成，直接写会静默覆盖模型新产物。此时返回可识别冲突错误
/// （code 1409，消息含 `sha256`/`conflict` 关键字），由前端提示「请重新载入」。
pub fn artifacts_edit_inline(params: &Value) -> Value {
    let id = params.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return rpc_err(1401, "缺少 artifact id");
    }
    let content = match params.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(_) => return rpc_err(1401, "content 必须是字符串"),
        None => return rpc_err(1401, "缺少 content"),
    };
    // 前端回传其最后所见内容的 sha256（camelCase 或 snake_case 均可）。
    let expected_sha = params
        .get("expectedSha256")
        .or_else(|| params.get("expected_sha256"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let art = match crate::tools::file_change::get_artifact(id) {
        Some(a) => a,
        None => return rpc_err(1404, "产物不存在"),
    };
    // 竞态校验：文件在编辑期间被其它 turn 重新生成 → 拒绝静默覆盖，返回可识别冲突错误。
    if let Some(exp) = expected_sha.as_deref() {
        let current = crate::tools::file_change::sha256_of(&art.path);
        if current.as_deref() != Some(exp) {
            return rpc_err(
                1409,
                &format!(
                    "sha256 冲突（conflict）：文件 '{}' 在您编辑期间已被更新，请重新载入后再保存",
                    art.name
                ),
            );
        }
    }
    // 安全：仅允许覆写已登记产物（路径明确归属），不接收任意外部路径。
    // 仅支持 UTF-8 文本；二进制一律拒绝，避免破坏媒体产物。
    let mime = art.mime.as_deref().unwrap_or("").to_ascii_lowercase();
    let is_text = mime.starts_with("text/")
        || mime.contains("json")
        || mime.contains("xml")
        || mime.contains("javascript")
        || mime.contains("typescript")
        || art.path.ends_with(".md")
        || art.path.ends_with(".txt")
        || art.path.ends_with(".yml")
        || art.path.ends_with(".yaml")
        || art.path.ends_with(".toml")
        || art.path.ends_with(".json");
    if !is_text {
        return rpc_err(1503, "仅支持文本类产物内联编辑");
    }
    if let Err(e) = std::fs::write(&art.path, content.as_bytes()) {
        return rpc_err(1502, &format!("写入失败: {e}"));
    }
    let sess = art.session_key.clone();
    crate::tools::file_change::record_artifact_with_session(&sess, &art.path, "modified");
    match crate::tools::file_change::get_artifact(id) {
        Some(a) => rpc_ok(artifact_value(&a)),
        None => rpc_err(1404, "登记失败"),
    }
}

// ---------------------------------------------------------------------------
// REST：GET /api/v1/artifacts/{id}（控制台唯一的产物取字节通道）
// ---------------------------------------------------------------------------

/// `GET /api/v1/artifacts/{id}`：把产物文件本体交给控制台。
///
/// # 为什么缺失它就是「产物下载必然坏」
///
/// 控制台的产物取字节**只有 REST 一条路**：`webui/src/utils/chat/artifacts.ts::artifactDownloadUrl`
/// 对「有 `id`、没有 `download_url`」的产物直接拼 `/api/v1/artifacts/{id}`（Rust 台账里的产物正是
/// 这个形状）。下载按钮（`useChatArtifactShareController::downloadArtifact`）与内联文本编辑
/// （`useArtifactActions::loadTextContent`）都走它。对照之下 `open_in_system` / `edit_inline`
/// 都是「RPC 优先、REST 兜底」，所以在纯 Rust 托管下只有下载这条是死的（501 → 前端弹「下载失败」）。
///
/// # 安全
///
/// API 只接受 **id**，且 id 必须命中 Rust 产物台账（`file_change::get_artifact`）——台账里的路径
/// 是 agent 真实产出并登记过的，调用方无法用它去读任意文件。换成「按 URL 里的路径读文件」就是
/// 任意文件读取漏洞，故此处绝不接受路径。
///
/// `?variant=thumb` 会被忽略（Rust 没有缩略图存储）：返回原图而不是伪造一个空图 —— 前端把它当
/// 图片显示即可，代价是缩略图网格会拉整图（已知差距，见 AGENTS.md）。
///
/// Rust 台账里查不到 id 时**兜底问侧车**（R61）：控制台展示的产物有两种来源 —— Rust
/// `artifacts.register` 登记的与侧车台账里的。此前 Rust 一律 404，于是「Python 引擎产出的产物
/// 点下载必失败」。侧车离线则保持 404（不假装成功、也不把 501 伪装成下载结果）。
pub async fn artifact_download(
    State(state): State<std::sync::Arc<GatewayState>>,
    AxumPath(id): AxumPath<String>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(entry) = crate::tools::file_change::get_artifact(&id) else {
        return match crate::gateway::sidecar_proxy::forward_get(&state, &uri, &headers).await {
            // 侧车也没有（或其自己回 404）→ 如实把侧车的回答交给控制台
            Some(resp) if resp.status() != StatusCode::NOT_FOUND => resp,
            // 侧车 404：回本进程同形的载荷（前端只认 code=NOT_FOUND）
            Some(_) => artifact_not_found(),
            // 侧车离线：保持既有 404 语义（此前行为逐字不变）
            None => artifact_not_found(),
        };
    };
    let path = std::path::PathBuf::from(&entry.path);
    // 目录类产物 / 已被删除的文件：都是「取不到」→ 404，而不是 500 或空响应
    match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_file() => {}
        _ => return artifact_not_found(),
    }
    let Ok(file) = tokio::fs::File::open(&path).await else {
        return artifact_not_found();
    };
    // 流式返回：产物可能是几十上百 MB，整块读进内存会顶高网关占用（`Content-Length` 仍如实给出，
    // 前端据此在解码前拦掉超大文本）。
    let stream = futures_util::stream::unfold(file, |mut file| async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 64 * 1024];
        match file.read(&mut buf).await {
            Ok(0) | Err(_) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<_, std::io::Error>(AxumBytes::from(buf)), file))
            }
        }
    });
    let mime = entry
        .mime
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, entry.size.to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", ascii_filename(&entry.name)),
        )
        // 产物可能在同一路径被重新生成，缓存副本会让内联编辑器读到过期内容
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| artifact_not_found())
}

/// 未登记 / 取不到的产物（载荷与 pyengine `api_artifacts` 同形）
fn artifact_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": "Artifact not found",
            "code": "NOT_FOUND",
        })),
    )
        .into_response()
}

/// `Content-Disposition` 里的文件名必须是纯 ASCII（非 ASCII 走 RFC 5987 会很啰嗦，
/// 而控制台自己用 `artifact.name` 落盘，这里只需给非浏览器客户端一个可读名字）。
fn ascii_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').to_string();
    if trimmed.is_empty() {
        "artifact".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod rest_tests {
    use super::*;

    #[test]
    fn ascii_filename_is_header_safe() {
        assert_eq!(ascii_filename("report final.txt"), "report final.txt");
        assert_eq!(ascii_filename("产物.png"), "__.png");
        // 引号/换行不能带进头（否则可被用来注入响应头）
        assert_eq!(ascii_filename("a\"b\r\nc.txt"), "a_b__c.txt");
        // 全被剥掉的名字不能变成空串或纯点号
        assert_eq!(ascii_filename("..."), "artifact");
        assert_eq!(ascii_filename(""), "artifact");
    }
}

