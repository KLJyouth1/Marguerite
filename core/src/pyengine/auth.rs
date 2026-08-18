//! Python 侧车认证：复用 Rust 网关同一个 token 做 SSOT
//!
//! Marguerite Python 网关从 `Marguerite_HOME/gateway-token` 读 token，
//! 与 Rust 网关共用同一份 `gateway-token` 文件，UI 不需要保存两个 token。
//! 实现：
//! - 在 spawn 前把 Rust 的 `GatewayState.token` 写入 `pyengine-home/gateway-token`
//! - 环境变量 `Marguerite_TOKEN_READONLY=1`（可选）禁止 Python 侧重新生成

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

/// 把共享 token 写入 Python 侧车的 home 目录，文件名对齐 Marguerite 约定
///
/// Windows 上 Defender/杀软的实时防护可能对"新建凭据文件"短暂加锁导致
/// `os error 5 拒绝访问`（瞬态）；这里做小指数退避重试（默认 5 次），
/// 把瞬态锁转换为确定性成功。
pub fn write_python_home_token(py_home: &Path, token: &str) -> Result<()> {
    fs::create_dir_all(py_home)
        .with_context(|| format!("创建 Python home 失败: {}", py_home.display()))?;
    let token_file = py_home.join("gateway-token");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match fs::write(&token_file, token.as_bytes()) {
            Ok(()) => return Ok(()),
            Err(e) if Instant::now() < deadline => {
                tracing::warn!(
                    attempt,
                    err = %e,
                    "写入 Python gateway-token 瞬态失败，{}ms 后重试",
                    200 * attempt
                );
                std::thread::sleep(Duration::from_millis(200 * u64::from(attempt)));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "写入 Python gateway-token 失败: {}（重试 {attempt} 次后放弃）: {e}",
                    token_file.display()
                ));
            }
        }
    }
}
