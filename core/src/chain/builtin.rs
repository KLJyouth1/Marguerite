//! 内置基础文件工具（v2 工作台）
//!
//! 供调用链 CLI 集成使用的最小工具集：读文件 / 编辑文件 / 代码搜索 /
//! 测试运行 / 调试（后两者为占位，真实执行待 A3 沙箱落地后接入）。

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::{ToolExecutor, ToolRegistry};

/// 注册全部内置基础工具
pub fn register_builtin_tools(registry: &mut ToolRegistry) {
    for tool in [
        Box::new(ReadFileTool) as Box<dyn ToolExecutor>,
        Box::new(EditFileTool),
        Box::new(SearchCodeTool),
        Box::new(RunTestsTool),
        Box::new(DebugTool),
    ] {
        registry.register_local(tool);
    }
}

fn args_str<'a>(args: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    args.get(key)
        .map(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("缺少参数: {key}"))
}

/// 读取文件内容
pub struct ReadFileTool;

#[async_trait]
impl ToolExecutor for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "读取指定路径的文件内容"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let path = args_str(args, "path")?;
        match std::fs::read_to_string(path) {
            Ok(content) => Ok(ToolResult::Success {
                data: content,
                metadata: HashMap::new(),
            }),
            Err(e) => Ok(ToolResult::Error {
                message: format!("读取失败: {e}"),
                code: "IO_ERROR".to_string(),
            }),
        }
    }
}

/// 写入/覆盖文件内容
pub struct EditFileTool;

#[async_trait]
impl ToolExecutor for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "将内容写入指定路径（覆盖写入）"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {
            "path": {"type": "string"}, "content": {"type": "string"}
        }})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let path = args_str(args, "path")?;
        let content = args.get("content").map(|s| s.as_str()).unwrap_or("");
        match std::fs::write(path, content) {
            Ok(()) => Ok(ToolResult::Success {
                data: format!("已写入 {}（{} 字节）", path, content.len()),
                metadata: HashMap::new(),
            }),
            Err(e) => Ok(ToolResult::Error {
                message: format!("写入失败: {e}"),
                code: "IO_ERROR".to_string(),
            }),
        }
    }
}

/// 在目录内递归搜索包含关键词的代码行
pub struct SearchCodeTool;

#[async_trait]
impl ToolExecutor for SearchCodeTool {
    fn name(&self) -> &str {
        "search_code"
    }
    fn description(&self) -> &str {
        "在目录内递归搜索包含关键词的代码行"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {
            "query": {"type": "string"}, "path": {"type": "string"}
        }})
    }
    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let query = args_str(args, "query")?;
        let root = args.get("path").map(|s| s.as_str()).unwrap_or(".");
        let hits = search_dir(root, query, 0);
        if hits.is_empty() {
            return Ok(ToolResult::Success {
                data: format!("未找到包含 {:?} 的代码行", query),
                metadata: HashMap::new(),
            });
        }
        Ok(ToolResult::Success {
            data: hits.join("\n"),
            metadata: HashMap::new(),
        })
    }
}

/// 递归搜索（深度 ≤ 6，忽略隐藏目录与二进制目录）
fn search_dir(dir: &str, query: &str, depth: usize) -> Vec<String> {
    if depth > 6 {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return hits;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            hits.extend(search_dir(&path.to_string_lossy(), query, depth + 1));
        } else if let Ok(content) = std::fs::read_to_string(&path) {
            for (idx, line) in content.lines().enumerate() {
                if line.contains(query) {
                    hits.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
                }
            }
        }
    }
    hits
}

/// 运行测试（占位：真实执行待 A3 沙箱）
pub struct RunTestsTool;

#[async_trait]
impl ToolExecutor for RunTestsTool {
    fn name(&self) -> &str {
        "run_tests"
    }
    fn description(&self) -> &str {
        "运行测试（占位，待 A3 沙箱接入）"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"file": {"type": "string"}}})
    }
    async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
        Ok(ToolResult::Error {
            message: "测试运行器未接入（待 A3 终端沙箱落地）".to_string(),
            code: "NOT_IMPLEMENTED".to_string(),
        })
    }
}

/// 调试（占位）
pub struct DebugTool;

#[async_trait]
impl ToolExecutor for DebugTool {
    fn name(&self) -> &str {
        "debug"
    }
    fn description(&self) -> &str {
        "调试器（占位，待 A3 沙箱接入）"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {"file": {"type": "string"}}})
    }
    async fn execute(&self, _args: &HashMap<String, String>) -> Result<ToolResult> {
        Ok(ToolResult::Error {
            message: "调试器未接入（待 A3 终端沙箱落地）".to_string(),
            code: "NOT_IMPLEMENTED".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_read_file_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("marguerite_builtin_{}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, "hello world").unwrap();
        let tool = ReadFileTool;
        let mut args = HashMap::new();
        args.insert("path".into(), tmp.to_string_lossy().to_string());
        let r = tool.execute(&args).await.unwrap();
        match r {
            ToolResult::Success { data, .. } => assert_eq!(data, "hello world"),
            _ => panic!("应成功"),
        }
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_edit_file_write() {
        let tmp =
            std::env::temp_dir().join(format!("marguerite_builtin_w_{}", uuid::Uuid::new_v4()));
        let tool = EditFileTool;
        let mut args = HashMap::new();
        args.insert("path".into(), tmp.to_string_lossy().to_string());
        args.insert("content".into(), "new content".into());
        let r = tool.execute(&args).await.unwrap();
        assert!(matches!(r, ToolResult::Success { .. }));
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), "new content");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_read_missing_file_returns_error() {
        let tool = ReadFileTool;
        let mut args = HashMap::new();
        args.insert("path".into(), "/nonexistent/xxx.rs".into());
        let r = tool.execute(&args).await.unwrap();
        assert!(matches!(r, ToolResult::Error { .. }));
    }

    #[tokio::test]
    async fn test_run_tests_is_placeholder() {
        let tool = RunTestsTool;
        let r = tool.execute(&HashMap::new()).await.unwrap();
        match r {
            ToolResult::Error { code, .. } => assert_eq!(code, "NOT_IMPLEMENTED"),
            _ => panic!("应为占位错误"),
        }
    }

    #[tokio::test]
    async fn test_register_builtin_tools() {
        let mut registry = ToolRegistry::new();
        register_builtin_tools(&mut registry);
        assert_eq!(registry.list_tools().len(), 5);
        assert!(registry.has_tool("read_file"));
        assert!(registry.has_tool("edit_file"));
        let _ = Arc::new(registry);
    }
}
