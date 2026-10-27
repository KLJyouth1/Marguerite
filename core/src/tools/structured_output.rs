//! P4·T1 结构化输出校验工具（补足「模型 JSON 不可靠」这个结构性短板）。
//!
//! # 为什么需要它（而不是让模型"注意点"）
//!
//! 工具调用 / 批处理 / 配置生成这三类任务，失败几乎都发生在同一个地方：
//! 模型吐出**看起来像 JSON 的东西**——多一个逗号、类型写错、少一个必填字段。
//! 这类错误有两个恶劣性质：
//! 1. **下游报错点在很远的地方**（解析器、反序列化、甚至业务逻辑里），真因被埋掉；
//! 2. **模型自己不知道自己错了**，于是反复重试同一个错误。
//!
//! 本工具的定位不是"再实现一个 JSON 解析器"，而是**把校验失败翻译成模型能照着修的结构**：
//! `RepairHint { path, expected, got, hint }`。经验上，给出精确路径 + 期望类型 + 实际类型
//! 之后，模型一次修正的成功率远高于"invalid JSON"这类笼统提示。
//!
//! # 边界（刻意不做的事）
//!
//! - **不做完整 JSON Schema**：只覆盖实际会被踩到的子集（type / required / properties /
//!   items / enum / additionalProperties）。引入完整实现会带进大量依赖与边界争议，
//!   而 95% 的失败集中在这个子集里。
//! - **不自动修复**：自动修复会掩盖模型的错误，让它学不到"哪里错了"。只报告，不改写。
//! - **不校验业务语义**：schema 表达不了的约束不属于本工具。

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::agent::tool_cycle::ToolResult;
use crate::mcp::registry::ToolExecutor;
use crate::tools::builtin::make_local_tool;

/// 一条可指导修复的差异。
///
/// 字段刻意从简：`path`（`$` 开头的 JSON pointer 风格路径）、`expected`、`got`、`hint`。
/// 多加字段会让模型注意力分散在格式上，而不是错误本身。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairHint {
    /// 出错位置，如 `$.items[2].price`；根节点为 `$`。
    pub path: String,
    /// 期望（来自 schema），如 `number` / `required` / `enum[a,b]`
    pub expected: String,
    /// 实际（来自 value），如 `string` / `missing` / `"c"`（枚举越界时给值）
    pub got: String,
    /// 一句可以直接照做的话
    pub hint: String,
}

impl RepairHint {
    fn new(path: &str, expected: &str, got: &str, hint: impl Into<String>) -> Self {
        Self {
            path: path.to_string(),
            expected: expected.to_string(),
            got: got.to_string(),
            hint: hint.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "expected": self.expected,
            "got": self.got,
            "hint": self.hint,
        })
    }
}

/// JSON 值的类型名（用于 `got` 字段，保持与 schema 的 `type` 词表一致）。
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            // schema 里 `integer` 与 `number` 是两种声明，报告时要能区分
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// `type` 是否匹配（`integer` 可被 `number` 接受，反之不然）。
fn type_matches(expected: &str, v: &Value) -> bool {
    let got = json_type_name(v);
    if expected == got {
        return true;
    }
    // JSON Schema 语义：integer 是 number 的子集
    expected == "number" && got == "integer"
}

/// 把 `$` 与子键拼成可读路径（数组用 `[i]`，对象用 `.key`）。
fn child_path(base: &str, key: Option<&str>, index: Option<usize>) -> String {
    match (key, index) {
        (Some(k), None) => {
            // 含特殊字符的键用括号形式，避免 `$.a.b` 与 `$["a.b"]` 混淆
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                format!("{base}.{k}")
            } else {
                format!("{base}[\"{k}\"]")
            }
        }
        (None, Some(i)) => format!("{base}[{i}]"),
        _ => base.to_string(),
    }
}

/// 校验 `value` 是否符合 `schema` 的**受支持子集**，返回全部差异（不是遇到第一个就停）。
///
/// 为什么收集全部而不早退：模型拿到"第 1 个错"→ 改完 → 又撞第 2 个错，会消耗多轮。
/// 一次给全，能让它一次改完。
pub fn validate(value: &Value, schema: &Value) -> Vec<RepairHint> {
    let mut out = Vec::new();
    walk(value, schema, "$", &mut out);
    out
}

fn walk(value: &Value, schema: &Value, path: &str, out: &mut Vec<RepairHint>) {
    let Some(obj) = schema.as_object() else {
        // schema 本身不是对象（如 `true`/`false` 布尔 schema）：不做校验
        return;
    };

    // 1) type
    if let Some(t) = obj.get("type") {
        let ok = match t {
            Value::String(s) => type_matches(s, value),
            // type 可以是数组（联合类型）：任一匹配即可
            Value::Array(types) => types
                .iter()
                .filter_map(Value::as_str)
                .any(|s| type_matches(s, value)),
            _ => true,
        };
        if !ok {
            let expected = match t {
                Value::String(s) => s.clone(),
                Value::Array(a) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("|"),
                _ => "unknown".to_string(),
            };
            out.push(RepairHint::new(
                path,
                &expected,
                json_type_name(value),
                format!("把 {path} 改成 {expected} 类型；当前是 {}", json_type_name(value)),
            ));
            // 类型都不对，继续深入 properties/items 只会产生噪声
            return;
        }
    }

    // 2) enum
    if let Some(Value::Array(allowed)) = obj.get("enum") {
        if !allowed.contains(value) {
            let list = allowed
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            out.push(RepairHint::new(
                path,
                &format!("enum[{list}]"),
                &value.to_string(),
                format!("{path} 只能取 {list} 之一"),
            ));
        }
    }

    // 3) object: required / properties / additionalProperties
    if let Value::Object(map) = value {
        if let Some(Value::Array(req)) = obj.get("required") {
            for r in req.iter().filter_map(Value::as_str) {
                if !map.contains_key(r) {
                    out.push(RepairHint::new(
                        &child_path(path, Some(r), None),
                        "required",
                        "missing",
                        format!("补上必填字段 {r}"),
                    ));
                }
            }
        }
        if let Some(Value::Object(props)) = obj.get("properties") {
            for (k, v) in map {
                if let Some(sub) = props.get(k) {
                    walk(v, sub, &child_path(path, Some(k), None), out);
                } else if obj.get("additionalProperties") == Some(&Value::Bool(false)) {
                    out.push(RepairHint::new(
                        &child_path(path, Some(k), None),
                        "no-additional-properties",
                        "present",
                        format!("删掉未声明字段 {k}（schema 不允许额外字段）"),
                    ));
                }
            }
        }
    }

    // 4) array: items
    if let (Value::Array(items), Some(item_schema)) = (value, obj.get("items")) {
        for (i, v) in items.iter().enumerate() {
            walk(v, item_schema, &child_path(path, None, Some(i)), out);
        }
    }
}

/// 把差异渲染成给模型看的一段文字（工具返回值）。
///
/// JSON 里的 `path/expected/got/hint` 已足够机器解析，但模型读**一行一条**更省 token。
pub fn render_hints(hints: &[RepairHint]) -> String {
    let mut s = String::new();
    for (i, h) in hints.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push_str(&format!(
            "{}. {} 期望={} 实际={} → {}",
            i + 1,
            h.path,
            h.expected,
            h.got,
            h.hint
        ));
    }
    s
}

/// `structured_output` 工具：校验一个 JSON 值是否符合给定 schema 子集。
pub fn structured_output_tool() -> Box<dyn ToolExecutor> {
    make_local_tool(
        "structured_output",
        "结构化输出校验：检查一段 JSON 是否符合给定 JSON Schema（子集：type/required/properties/items/enum/additionalProperties）。\
         失败时返回可照做的修复清单（path + 期望 + 实际 + hint），不会自动改你的数据。\
         参数: value(必填, 待校验的 JSON 文本), schema(必填, JSON Schema 文本)",
        json!({
            "type": "object",
            "properties": {
                "value": { "type": "string", "description": "待校验的 JSON 文本" },
                "schema": { "type": "string", "description": "JSON Schema 文本（受支持子集）" }
            },
            "required": ["value", "schema"]
        }),
        std::sync::Arc::new(move |args: &HashMap<String, String>| {
            let raw_value = args.get("value").cloned().unwrap_or_default();
            let raw_schema = args.get("schema").cloned().unwrap_or_default();
            Box::pin(async move {
                if raw_value.trim().is_empty() || raw_schema.trim().is_empty() {
                    return Ok(ToolResult::Error {
                        message: "structured_output 需要非空 value 与 schema".into(),
                        code: "INVALID_PARAMS".into(),
                    });
                }
                let value: Value = match serde_json::from_str(&raw_value) {
                    Ok(v) => v,
                    Err(e) => {
                        // 连 JSON 都解析不了：这本身是最该被"翻译"的错误
                        return Ok(ToolResult::Success {
                            data: format!(
                                "结果: 非法 JSON\n位置: line {} column {}\n原始错误: {}\n提示: 先修语法（尾逗号 / 单引号 / 未闭合括号是最常见的三类）",
                                e.line(),
                                e.column(),
                                e
                            ),
                            metadata: HashMap::new(),
                        });
                    }
                };
                let schema: Value = match serde_json::from_str(&raw_schema) {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(ToolResult::Error {
                            message: format!("schema 不是合法 JSON: {e}"),
                            code: "INVALID_PARAMS".into(),
                        })
                    }
                };

                let hints = validate(&value, &schema);
                if hints.is_empty() {
                    return Ok(ToolResult::Success {
                        data: "结果: 通过（0 处差异）".to_string(),
                        metadata: HashMap::new(),
                    });
                }
                let mut data = format!("结果: 不通过（{} 处差异）\n", hints.len());
                data.push_str(&render_hints(&hints));
                Ok(ToolResult::Success {
                    data,
                    metadata: HashMap::new(),
                })
            })
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(v: Value) -> Value {
        v
    }

    #[test]
    fn valid_payload_yields_no_hints() {
        let s = schema(json!({
            "type": "object",
            "required": ["a"],
            "properties": { "a": { "type": "number" } }
        }));
        assert!(validate(&json!({ "a": 1 }), &s).is_empty());
    }

    #[test]
    fn wrong_type_reports_path_expected_and_got() {
        // 这是最高频的失败：模型把数字写成字符串
        let s = schema(json!({
            "type": "object",
            "properties": { "price": { "type": "number" } }
        }));
        let hints = validate(&json!({ "price": "12.5" }), &s);
        assert_eq!(hints.len(), 1, "{hints:?}");
        assert_eq!(hints[0].path, "$.price");
        assert_eq!(hints[0].expected, "number");
        assert_eq!(hints[0].got, "string");
    }

    #[test]
    fn missing_required_field_is_named() {
        let s = schema(json!({ "type": "object", "required": ["id", "name"] }));
        let hints = validate(&json!({ "id": 1 }), &s);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].path, "$.name");
        assert_eq!(hints[0].expected, "required");
        assert_eq!(hints[0].got, "missing");
    }

    #[test]
    fn nested_object_and_array_paths_are_precise() {
        // 嵌套深处的错误必须报出完整路径，否则模型找不到位置
        let s = schema(json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "price": { "type": "number" } }
                    }
                }
            }
        }));
        let hints = validate(&json!({ "items": [{ "price": 1 }, { "price": "x" }] }), &s);
        assert_eq!(hints.len(), 1, "{hints:?}");
        assert_eq!(hints[0].path, "$.items[1].price");
    }

    #[test]
    fn all_errors_are_collected_not_just_first() {
        // 一次给全，避免模型"改一个又撞一个"消耗多轮
        let s = schema(json!({
            "type": "object",
            "required": ["a", "b"],
            "properties": {
                "a": { "type": "number" },
                "b": { "type": "boolean" }
            }
        }));
        let hints = validate(&json!({ "a": "x", "b": "y" }), &s);
        assert_eq!(hints.len(), 2, "{hints:?}");
    }

    #[test]
    fn enum_out_of_range_reports_allowed_values() {
        let s = schema(json!({
            "type": "object",
            "properties": { "level": { "type": "string", "enum": ["low", "high"] } }
        }));
        let hints = validate(&json!({ "level": "medium" }), &s);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].expected, "enum[\"low\", \"high\"]");
        assert!(hints[0].hint.contains("low"));
    }

    #[test]
    fn additional_properties_false_rejects_unknown_keys() {
        let s = schema(json!({
            "type": "object",
            "properties": { "a": { "type": "number" } },
            "additionalProperties": false
        }));
        let hints = validate(&json!({ "a": 1, "extra": 2 }), &s);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].path, "$.extra");
        assert_eq!(hints[0].got, "present");
    }

    #[test]
    fn integer_is_accepted_where_number_is_expected_but_not_reverse() {
        let num = schema(json!({ "type": "object", "properties": { "n": { "type": "number" } } }));
        assert!(validate(&json!({ "n": 3 }), &num).is_empty(), "integer 应满足 number");

        let int = schema(json!({ "type": "object", "properties": { "n": { "type": "integer" } } }));
        assert_eq!(
            validate(&json!({ "n": 3.5 }), &int).len(),
            1,
            "number 不应满足 integer"
        );
    }

    #[test]
    fn union_type_accepts_any_member() {
        let s = schema(json!({
            "type": "object",
            "properties": { "v": { "type": ["string", "null"] } }
        }));
        assert!(validate(&json!({ "v": null }), &s).is_empty());
        assert!(validate(&json!({ "v": "x" }), &s).is_empty());
        assert_eq!(validate(&json!({ "v": 1 }), &s).len(), 1);
    }

    #[test]
    fn type_mismatch_does_not_cascade_into_child_noise() {
        // 类型错时若继续下钻 properties，会产出大量无意义差异，淹没真问题
        let s = schema(json!({
            "type": "object",
            "properties": {
                "cfg": {
                    "type": "object",
                    "required": ["deep"],
                    "properties": { "deep": { "type": "string" } }
                }
            }
        }));
        let hints = validate(&json!({ "cfg": "not-an-object" }), &s);
        assert_eq!(hints.len(), 1, "只应报类型错，不应再报 $.cfg.deep 缺失: {hints:?}");
        assert_eq!(hints[0].path, "$.cfg");
    }

    #[test]
    fn special_char_keys_use_bracket_path() {
        let s = schema(json!({
            "type": "object",
            "properties": { "a.b": { "type": "number" } }
        }));
        let hints = validate(&json!({ "a.b": "x" }), &s);
        assert_eq!(hints[0].path, "$[\"a.b\"]");
    }

    #[test]
    fn render_hints_is_one_line_per_error() {
        let s = schema(json!({ "type": "object", "required": ["a", "b"] }));
        let hints = validate(&json!({}), &s);
        let text = render_hints(&hints);
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains("$.a"));
        assert!(text.contains("$.b"));
    }

    #[test]
    fn boolean_schema_is_ignored_not_panicking() {
        // `true` / `false` 布尔 schema 不在支持子集内，必须静默跳过而不是 panic
        assert!(validate(&json!({ "a": 1 }), &json!(true)).is_empty());
    }
}
