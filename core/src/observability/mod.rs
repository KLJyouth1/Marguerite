//! 可观测性：日志轮转、统一脱敏、诊断包导出（S6）、写入量自观测（W7）。
//!
//! 前四块能力共用一条纪律——**任何离开本机的文本都必须先过
//! [`redact::scrub_text`]**：
//!
//! - [`redact`]：全仓唯一的脱敏实现（Python 侧
//!   `opensquilla/observability/redact.py` 的 Rust 对照实现），覆盖
//!   Bearer / `key=value` 赋值 / 裸令牌 / URL 内嵌凭据 / 家目录五类。
//! - [`rotation`]：按大小轮转 + 目录总量封顶（10MB/文件、200MB/目录），
//!   侧车日志借此从「无上限裸File」变成有界增长。
//! - [`bundle`]：`--export-diagnostics` 的收集与归档（tar，只读、逐项降级）。
//! - [`io_report`]：写入量自观测（W7）——各库真实生效的 journal 模式、几个
//!   无界风险目录的字节数、本进程写入计数，随 `system.health` 的 `io` 段下发。
//!
//! 为什么不把轮转逻辑塞进 `pyengine::sidecar`：它与 pyengine 无关，
//! 任何第二处需要「有界日志」的地方（未来 CLI 自身的文件日志）都能直接复用。

pub mod bundle;
pub mod io_report;
pub mod redact;
pub mod rotation;