//! 命令路由表
//!
//! 借鉴 opencode route.ts 的设计，实现命令路由注册和匹配。
//! 支持 Interactive / Direct / Subcommand 三种模式的路由分发。

use anyhow::Result;

use super::argv::{Args, CliMode};
use crate::persistence::config::Config;

/// 路由条目
pub struct Route {
    /// 路由名称（子命令名称）
    pub name: &'static str,
    /// 处理函数
    pub handler: fn(&Args, &Config) -> Result<()>,
    /// 描述信息
    pub description: &'static str,
}

/// 路由表
pub struct RouteTable {
    routes: Vec<Route>,
}

impl RouteTable {
    /// 创建空路由表
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// 注册路由
    pub fn register(
        &mut self,
        name: &'static str,
        handler: fn(&Args, &Config) -> Result<()>,
        description: &'static str,
    ) {
        self.routes.push(Route {
            name,
            handler,
            description,
        });
    }

    /// 根据参数匹配路由
    ///
    /// - `Interactive` 模式：返回 `None`，表示进入交互式 TUI
    /// - `Direct` 模式：返回 `None`，由主程序直接处理单指令
    /// - `Subcommand(name)` 模式：按名称匹配已注册的路由
    pub fn route(&self, args: &Args) -> Option<&Route> {
        match &args.mode {
            CliMode::Subcommand(name) => self.routes.iter().find(|r| r.name == name.as_str()),
            // Interactive 和 Direct 模式不由路由表处理
            CliMode::Interactive | CliMode::Direct => None,
        }
    }

    /// 获取所有已注册路由
    pub fn routes(&self) -> &[Route] {
        &self.routes
    }
}

impl Default for RouteTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::argv::CliMode;

    fn dummy_handler(_args: &Args, _config: &Config) -> Result<()> {
        Ok(())
    }

    #[test]
    fn test_register_and_route() {
        let mut table = RouteTable::new();
        table.register("doctor", dummy_handler, "系统诊断");
        table.register("model", dummy_handler, "模型管理");

        let doctor_args = Args {
            mode: CliMode::Subcommand("doctor".to_string()),
            prompt: None,
            file: None,
            pipe: false,
            sub_args: vec![],
            ..Args::default()
        };
        let matched = table.route(&doctor_args);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "doctor");
    }

    #[test]
    fn test_route_not_found() {
        let table = RouteTable::new();
        let unknown_args = Args {
            mode: CliMode::Subcommand("unknown".to_string()),
            prompt: None,
            file: None,
            pipe: false,
            sub_args: vec![],
            ..Args::default()
        };
        assert!(table.route(&unknown_args).is_none());
    }

    #[test]
    fn test_interactive_returns_none() {
        let mut table = RouteTable::new();
        table.register("doctor", dummy_handler, "系统诊断");

        let interactive_args = Args {
            mode: CliMode::Interactive,
            prompt: None,
            file: None,
            pipe: false,
            sub_args: vec![],
            ..Args::default()
        };
        assert!(table.route(&interactive_args).is_none());
    }

    #[test]
    fn test_direct_returns_none() {
        let mut table = RouteTable::new();
        table.register("doctor", dummy_handler, "系统诊断");

        let direct_args = Args {
            mode: CliMode::Direct,
            prompt: Some("hello".to_string()),
            file: None,
            pipe: false,
            sub_args: vec![],
            ..Args::default()
        };
        assert!(table.route(&direct_args).is_none());
    }
}
