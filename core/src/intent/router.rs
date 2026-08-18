//! 意图路由
//!
//! 维护意图 → 技能/处理器的映射表，支持热加载。
//! 根据分类结果将请求路由到对应的处理器执行。

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::classifier::{ClassificationResult, IntentType};
use crate::cli::argv::Args;
use crate::persistence::config::Config;

/// 意图处理上下文
#[derive(Debug, Clone)]
pub struct IntentContext {
    /// 用户输入文本
    pub text: String,
    /// 关联的文件路径列表
    pub files: Vec<std::path::PathBuf>,
    /// 命令行参数
    pub args: Args,
    /// 应用配置
    pub config: Config,
}

impl IntentContext {
    /// 创建新的意图上下文
    pub fn new(text: String, args: Args, config: Config) -> Self {
        Self {
            text,
            files: Vec::new(),
            args,
            config,
        }
    }
}

/// 意图处理输出
#[derive(Debug, Clone)]
pub struct IntentOutput {
    /// 处理的意图类型
    pub intent: IntentType,
    /// 处理结果文本
    pub result: String,
    /// 附加元数据
    pub metadata: HashMap<String, String>,
}

impl IntentOutput {
    /// 创建新的意图输出
    pub fn new(intent: IntentType, result: String) -> Self {
        Self {
            intent,
            result,
            metadata: HashMap::new(),
        }
    }
}

/// 意图处理器
pub struct IntentHandler {
    /// 处理器名称
    pub name: String,
    /// 处理器描述
    pub description: String,
    /// 处理的意图类型
    pub intent: IntentType,
    /// 处理函数
    pub handler: fn(IntentContext) -> Result<IntentOutput>,
    /// 优先级 (0-255，越高越优先)
    pub priority: u8,
}

/// 可序列化的处理器条目（用于保存/加载路由表）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandlerEntry {
    name: String,
    description: String,
    intent: IntentType,
    priority: u8,
}

/// 可序列化的路由表结构
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RouterTableData {
    mappings: HashMap<String, Vec<HandlerEntry>>,
    fallback: Option<HandlerEntry>,
}

/// 意图路由
pub struct IntentRouter {
    /// 意图 → 处理器列表的映射
    mappings: HashMap<IntentType, Vec<IntentHandler>>,
    /// Unknown 意图的兜底处理器
    fallback: Option<IntentHandler>,
    /// 最后加载/保存的路径（用于热加载）
    last_path: Option<std::path::PathBuf>,
}

impl IntentRouter {
    /// 创建空路由
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
            fallback: None,
            last_path: None,
        }
    }

    /// 注册处理器
    pub fn register(&mut self, handler: IntentHandler) {
        let entry = self.mappings.entry(handler.intent.clone()).or_default();
        entry.push(handler);
    }

    /// 设置兜底处理器
    pub fn set_fallback(&mut self, handler: IntentHandler) {
        self.fallback = Some(handler);
    }

    /// 路由到对应处理器
    ///
    /// 路由逻辑：
    /// 1. 如果 `needs_llm = true`，直接路由到 `fallback` 处理器
    /// 2. 根据 `ClassificationResult.intent` 查找 `mappings`
    /// 3. 按 `priority` 排序，选最高优先级的处理器
    /// 4. 执行处理器，返回 `IntentOutput`
    pub fn route(&self, result: &ClassificationResult, ctx: IntentContext) -> Result<IntentOutput> {
        // 需要 LLM 消歧时走兜底处理器
        if result.needs_llm {
            return self.execute_fallback(ctx);
        }

        // 查找匹配的处理器列表
        let handlers = self.mappings.get(&result.intent);

        match handlers {
            Some(handler_list) if !handler_list.is_empty() => {
                // 按优先级降序排序，选最高优先级
                let best = handler_list
                    .iter()
                    .max_by_key(|h| h.priority)
                    .ok_or_else(|| anyhow::anyhow!("没有可用的处理器"))?;
                (best.handler)(ctx)
            }
            _ => {
                // 没有找到处理器，走兜底
                self.execute_fallback(ctx)
            }
        }
    }

    /// 执行兜底处理器
    fn execute_fallback(&self, ctx: IntentContext) -> Result<IntentOutput> {
        match &self.fallback {
            Some(handler) => (handler.handler)(ctx),
            None => Ok(IntentOutput::new(
                IntentType::Unknown,
                "没有注册的处理器".to_string(),
            )),
        }
    }

    /// 热加载路由表（从最后加载的文件重新读取）
    pub fn reload(&mut self) -> Result<()> {
        let path = self
            .last_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("没有已加载的路由表文件，请先调用 load"))?;
        self.load(&path)
    }

    /// 保存路由表到文件
    ///
    /// 只保存处理器元数据（名称、描述、意图、优先级），
    /// 不保存函数指针。加载后需要重新注册处理器。
    pub fn save(&self, path: &Path) -> Result<()> {
        let data = self.to_table_data();
        let content = serde_yaml::to_string(&data)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    /// 从文件加载路由表
    ///
    /// 加载处理器元数据，但不包含函数指针。
    /// 加载后 mappings 中的处理器列表为空，需要通过 register 重新注册。
    pub fn load(&mut self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        let data: RouterTableData = serde_yaml::from_str(&content)?;

        // 清空当前映射
        self.mappings.clear();

        // 重建映射（不含函数指针，加载后需重新注册）
        for (intent_str, entries) in &data.mappings {
            let intent: IntentType =
                serde_yaml::from_str(intent_str).unwrap_or(IntentType::Unknown);
            let handlers: Vec<IntentHandler> = entries
                .iter()
                .map(|e| IntentHandler {
                    name: e.name.clone(),
                    description: e.description.clone(),
                    intent: e.intent.clone(),
                    handler: |_| {
                        Ok(IntentOutput::new(
                            IntentType::Unknown,
                            "处理器未注册，请先调用 register".to_string(),
                        ))
                    },
                    priority: e.priority,
                })
                .collect();
            self.mappings.insert(intent, handlers);
        }

        // 记录最后加载的路径
        self.last_path = Some(path.to_path_buf());
        Ok(())
    }

    /// 获取所有已注册的映射
    pub fn mappings(&self) -> &HashMap<IntentType, Vec<IntentHandler>> {
        &self.mappings
    }

    /// 获取兜底处理器引用
    pub fn fallback(&self) -> Option<&IntentHandler> {
        self.fallback.as_ref()
    }

    /// 转换为可序列化的路由表数据
    fn to_table_data(&self) -> RouterTableData {
        let mut mappings_data = HashMap::new();
        for (intent, handlers) in &self.mappings {
            let entries: Vec<HandlerEntry> = handlers
                .iter()
                .map(|h| HandlerEntry {
                    name: h.name.clone(),
                    description: h.description.clone(),
                    intent: h.intent.clone(),
                    priority: h.priority,
                })
                .collect();
            let key = serde_yaml::to_string(intent).unwrap_or_default();
            mappings_data.insert(key.trim().to_string(), entries);
        }
        RouterTableData {
            mappings: mappings_data,
            fallback: self.fallback.as_ref().map(|h| HandlerEntry {
                name: h.name.clone(),
                description: h.description.clone(),
                intent: h.intent.clone(),
                priority: h.priority,
            }),
        }
    }
}

impl Default for IntentRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::argv::CliMode;

    fn dummy_handler(ctx: IntentContext) -> Result<IntentOutput> {
        Ok(IntentOutput::new(
            IntentType::Explain,
            format!("处理: {}", ctx.text),
        ))
    }

    fn fallback_handler(ctx: IntentContext) -> Result<IntentOutput> {
        Ok(IntentOutput::new(
            IntentType::Unknown,
            format!("兜底处理: {}", ctx.text),
        ))
    }

    fn create_test_context(text: &str) -> IntentContext {
        IntentContext::new(
            text.to_string(),
            Args {
                mode: CliMode::Direct,
                prompt: Some(text.to_string()),
                file: None,
                pipe: false,
                sub_args: vec![],
                ..Args::default()
            },
            Config::default(),
        )
    }

    #[test]
    fn test_register_and_route() {
        let mut router = IntentRouter::new();
        router.register(IntentHandler {
            name: "explainer".to_string(),
            description: "代码解释器".to_string(),
            intent: IntentType::Explain,
            handler: dummy_handler,
            priority: 100,
        });

        let result = ClassificationResult {
            intent: IntentType::Explain,
            confidence: 0.85,
            needs_llm: false,
        };

        let output = router
            .route(&result, create_test_context("explain this code"))
            .unwrap();
        assert_eq!(output.intent, IntentType::Explain);
        assert!(output.result.contains("explain this code"));
    }

    #[test]
    fn test_route_priority() {
        let mut router = IntentRouter::new();

        // 注册两个同意图处理器，不同优先级
        router.register(IntentHandler {
            name: "low_priority".to_string(),
            description: "低优先级".to_string(),
            intent: IntentType::Fix,
            handler: |ctx: IntentContext| {
                Ok(IntentOutput::new(
                    IntentType::Fix,
                    format!("低: {}", ctx.text),
                ))
            },
            priority: 50,
        });
        router.register(IntentHandler {
            name: "high_priority".to_string(),
            description: "高优先级".to_string(),
            intent: IntentType::Fix,
            handler: |ctx: IntentContext| {
                Ok(IntentOutput::new(
                    IntentType::Fix,
                    format!("高: {}", ctx.text),
                ))
            },
            priority: 200,
        });

        let result = ClassificationResult {
            intent: IntentType::Fix,
            confidence: 0.9,
            needs_llm: false,
        };

        let output = router
            .route(&result, create_test_context("fix bug"))
            .unwrap();
        // 应选择高优先级处理器
        assert!(output.result.starts_with("高:"));
    }

    #[test]
    fn test_route_fallback_when_needs_llm() {
        let mut router = IntentRouter::new();
        router.set_fallback(IntentHandler {
            name: "llm_fallback".to_string(),
            description: "LLM 兜底".to_string(),
            intent: IntentType::Unknown,
            handler: fallback_handler,
            priority: 0,
        });

        let result = ClassificationResult {
            intent: IntentType::Unknown,
            confidence: 0.3,
            needs_llm: true,
        };

        let output = router
            .route(&result, create_test_context("random text"))
            .unwrap();
        assert!(output.result.contains("兜底处理"));
    }

    #[test]
    fn test_route_fallback_when_no_handler() {
        let mut router = IntentRouter::new();
        router.set_fallback(IntentHandler {
            name: "fallback".to_string(),
            description: "兜底".to_string(),
            intent: IntentType::Unknown,
            handler: fallback_handler,
            priority: 0,
        });

        let result = ClassificationResult {
            intent: IntentType::Doc,
            confidence: 0.8,
            needs_llm: false,
        };

        // Doc 没有注册处理器，应走兜底
        let output = router
            .route(&result, create_test_context("write docs"))
            .unwrap();
        assert!(output.result.contains("兜底处理"));
    }

    #[test]
    fn test_route_no_fallback() {
        let router = IntentRouter::new();
        let result = ClassificationResult {
            intent: IntentType::Unknown,
            confidence: 0.0,
            needs_llm: true,
        };

        let output = router
            .route(&result, create_test_context("anything"))
            .unwrap();
        assert_eq!(output.result, "没有注册的处理器");
    }

    #[test]
    fn test_save_and_load() {
        let mut router = IntentRouter::new();
        router.register(IntentHandler {
            name: "explainer".to_string(),
            description: "代码解释器".to_string(),
            intent: IntentType::Explain,
            handler: dummy_handler,
            priority: 100,
        });

        let tmp_path = std::env::temp_dir().join("marguerite_test_router.yaml");
        router.save(&tmp_path).unwrap();
        assert!(tmp_path.exists());

        // 加载到新路由
        let mut loaded = IntentRouter::new();
        loaded.load(&tmp_path).unwrap();

        // 验证元数据已加载
        let mappings = loaded.mappings();
        let explain_handlers = mappings.get(&IntentType::Explain);
        assert!(explain_handlers.is_some());
        assert_eq!(explain_handlers.unwrap().len(), 1);
        assert_eq!(explain_handlers.unwrap()[0].name, "explainer");

        // 清理
        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn test_reload() {
        let mut router = IntentRouter::new();
        router.register(IntentHandler {
            name: "fixer".to_string(),
            description: "修复器".to_string(),
            intent: IntentType::Fix,
            handler: dummy_handler,
            priority: 100,
        });

        let tmp_path = std::env::temp_dir().join("marguerite_test_reload.yaml");
        router.save(&tmp_path).unwrap();

        // 通过 load 加载后，再 reload
        let mut router2 = IntentRouter::new();
        router2.load(&tmp_path).unwrap();

        // 验证 reload 工作
        router2.reload().unwrap();
        let mappings = router2.mappings();
        let fix_handlers = mappings.get(&IntentType::Fix);
        assert!(fix_handlers.is_some());

        // 清理
        let _ = std::fs::remove_file(&tmp_path);
    }
}
