#!/usr/bin/env python3
"""重构 methods.rs - 将剩余实现替换为转发器，调用新模块。

⚠️ 一次性重构脚本（历史诊断），**不维护、不进门禁**。它会把 `methods.rs`
改写成转发器 —— 运行前请先确保工作区干净，否则很难回退。
本文件原先放在 `core/src/gateway/` 里（Rust 源码树中唯一的 .py），R25 移到
`scripts/dev/`，并把写死的本机路径改为从仓库根推导。
"""

import re
from pathlib import Path

# scripts/dev/ -> scripts/ -> 仓库根
BASE = Path(__file__).resolve().parents[2] / "core" / "src" / "gateway"
METHODS_RS = BASE / "methods.rs"

def main():
    content = METHODS_RS.read_text(encoding="utf-8")
    
    # 1. 添加新模块导入
    new_imports = """use crate::gateway::fleet;
use crate::gateway::mcp;
use crate::gateway::sessions;
use crate::gateway::system;
use crate::gateway::workspace;
"""
    
    # 找到 use crate::gateway::team; 后面，添加新导入
    content = content.replace(
        "use crate::gateway::team;\n",
        "use crate::gateway::team;\n" + new_imports
    )
    
    # 2. 替换系统方法实现为转发器
    content = _replace_function(content, "health", "system::health(ctx)")
    content = _replace_function(content, "system_status", "system::system_status(ctx)")
    content = _replace_function(content, "doctor_status", "system::doctor_status(ctx)")
    content = _replace_function(content, "runtime_status", "system::runtime_status()")
    
    # 3. 替换会话方法实现为转发器
    content = _replace_function(content, "sessions_list", "sessions::sessions_list(ctx, params)")
    content = _replace_function(content, "chat_history", "sessions::chat_history(ctx, params)")
    
    # 4. 替换 MCP 方法实现为转发器
    content = _replace_function(content, "mcp_servers_list", "mcp::mcp_servers_list()")
    content = _replace_function(content, "mcp_servers_add", "mcp::mcp_servers_add(params)")
    content = _replace_function(content, "mcp_servers_remove", "mcp::mcp_servers_remove(params)")
    
    # 5. 替换 Fleet/Evolve/Plugins/Pages 方法实现为转发器
    content = _replace_function(content, "fleet_list", "fleet::fleet_list()")
    content = _replace_function(content, "fleet_route", "fleet::fleet_route(params)")
    content = _replace_function(content, "evolve_history", "fleet::evolve_history()")
    content = _replace_function(content, "plugins_list", "fleet::plugins_list()")
    content = _replace_function(content, "pages_list", "fleet::pages_list(params)")
    content = _replace_function(content, "pages_add", "fleet::pages_add(params)")
    content = _replace_function(content, "pages_search", "fleet::pages_search(params)")
    
    # 6. 替换工作区文件方法实现为转发器
    content = _replace_function(content, "workspace_file_read", "workspace::workspace_file_read(params)")
    content = _replace_function(content, "workspace_file_write", "workspace::workspace_file_write(params)")
    content = _replace_function(content, "file_changes_list", "workspace::file_changes_list(params)")
    content = _replace_function(content, "file_changes_revert", "workspace::file_changes_revert(params)")
    
    # 7. 替换 tools_catalog
    content = _replace_function(content, "tools_catalog", "workspace::tools_catalog()")
    
    # 8. 更新 dispatch_rust 中的调用（改为直接调用新模块）
    dispatch_replacements = {
        '"health" => health(ctx)': '"health" => system::health(ctx)',
        '"status" => system_status(ctx)': '"status" => system::system_status(ctx)',
        '"doctor.status" => doctor_status(ctx)': '"doctor.status" => system::doctor_status(ctx)',
        '"runtime.status" => runtime_status()': '"runtime.status" => system::runtime_status()',
        '"sessions.list" => sessions_list(ctx, params)': '"sessions.list" => sessions::sessions_list(ctx, params)',
        '"chat.history" => chat_history(ctx, params)': '"chat.history" => sessions::chat_history(ctx, params)',
        '"mcp.servers.list" => mcp_servers_list()': '"mcp.servers.list" => mcp::mcp_servers_list()',
        '"mcp.servers.add" => mcp_servers_add(params)': '"mcp.servers.add" => mcp::mcp_servers_add(params)',
        '"mcp.servers.remove" => mcp_servers_remove(params)': '"mcp.servers.remove" => mcp::mcp_servers_remove(params)',
        '"fleet.list" => fleet_list()': '"fleet.list" => fleet::fleet_list()',
        '"fleet.route" => fleet_route(params)': '"fleet.route" => fleet::fleet_route(params)',
        '"evolve.history" => evolve_history()': '"evolve.history" => fleet::evolve_history()',
        '"plugins.list" => plugins_list()': '"plugins.list" => fleet::plugins_list()',
        '"pages.list" => pages_list(params)': '"pages.list" => fleet::pages_list(params)',
        '"pages.add" => pages_add(params)': '"pages.add" => fleet::pages_add(params)',
        '"pages.search" => pages_search(params)': '"pages.search" => fleet::pages_search(params)',
        '"workspace.file_changes.list" => file_changes_list(params)': '"workspace.file_changes.list" => workspace::file_changes_list(params)',
        '"workspace.file_changes.revert" => file_changes_revert(params)': '"workspace.file_changes.revert" => workspace::file_changes_revert(params)',
        '"workspace.file.read" => workspace_file_read(params)': '"workspace.file.read" => workspace::workspace_file_read(params)',
        '"workspace.file.write" => workspace_file_write(params)': '"workspace.file.write" => workspace::workspace_file_write(params)',
        '"tools.catalog" => tools_catalog()': '"tools.catalog" => workspace::tools_catalog()',
    }
    for old, new in dispatch_replacements.items():
        content = content.replace(old, new)
    
    # 9. 更新 dispatch_sync 中的调用
    content = content.replace(
        '"artifacts.list" => return Ok(artifacts_list(params))',
        '"artifacts.list" => return Ok(artifacts::artifacts_list(params))'
    )
    
    # 10. 删除不再需要的本地 forwarder 函数（保持以避免编译错误，但替换为简单形式）
    # 保留 forwarder 函数以避免对其他调用点的破坏
    # 但简化它们
    
    # 11. 修复测试模块的未使用导入警告
    content = content.replace(
        "use crate::gateway::config::config_patch_local;\n    use crate::gateway::mode::{mode_switch, mode_spec, mode_get};\n    use crate::gateway::onboarding::{onboarding_catalog, onboarding_status};",
        "use crate::gateway::mode::mode_switch;"
    )
    
    # 12. 删除 mcp_servers_probe 的 forwarder（它走 async 分支，不需要在 methods.rs 中）
    # 保留以兼容
    
    # 13. 移除 model_config_to_llm 函数（已迁移到 fleet.rs）
    # 找到函数定义并替换为对 fleet 的调用
    # 注意：此函数可能仍被 fleet_list/fleet_route 使用，但现在它们已在 fleet.rs 内部
    # 检查是否还有其他引用
    if "model_config_to_llm" in content:
        # 如果只有定义没有外部引用，可以保留但标记为已弃用
        pass
    
    METHODS_RS.write_text(content, encoding="utf-8")
    print("✅ methods.rs 重构完成")

def _replace_function(content: str, fn_name: str, replacement: str) -> str:
    """替换一个函数的实现为转发器。"""
    # 匹配函数定义（fn fn_name(...) -> Value { 或 fn fn_name(...) { 或 async fn fn_name(...) -> Value {）
    # 找到函数起始位置
    pattern = re.compile(
        r'((?:///[^\n]*\n(?:///[^\n]*\n)*)?(?:\s*//[^\n]*\n)*)'  # 文档注释
        r'(pub\s+)?(async\s+)?fn\s+' + re.escape(fn_name) + r'\s*\([^)]*\)\s*(?:->\s*\w+)?\s*\{',
        re.MULTILINE
    )
    
    match = pattern.search(content)
    if not match:
        print(f"⚠️  未找到函数: {fn_name}")
        return content
    
    # 找到函数体的结束位置（大括号匹配）
    start = match.end() - 1  # 指向 {
    depth = 0
    end = start
    for i in range(start, len(content)):
        if content[i] == '{':
            depth += 1
        elif content[i] == '}':
            depth -= 1
            if depth == 0:
                end = i + 1
                break
    
    # 获取函数签名
    full_match = match.group(0)
    # 提取文档注释
    doc = match.group(1) if match.group(1) else ""
    # 提取可见性和 async
    vis = match.group(2) or ""
    async_kw = match.group(3) or ""
    
    # 构建新的转发器函数
    new_fn = f"{doc}{vis}{async_kw}fn {fn_name}() -> Value {{\n    {replacement}\n}}"
    
    # 处理签名中可能有参数的情况，检查原函数签名
    # 从 full_match 中提取参数签名
    sig_match = re.search(r'fn\s+' + re.escape(fn_name) + r'\s*\(([^)]*)\)', full_match)
    params_str = sig_match.group(1) if sig_match else ""
    
    # 如果原函数有参数，需要调整转发器
    if params_str.strip():
        # 检查参数名
        params = [p.strip().split(':')[0].strip().lstrip('&') for p in params_str.split(',') if p.strip()]
        params = [p for p in params if p and p != 'self']
        # 转发器直接使用 replacement（已包含参数）
        new_fn = f"{doc}{vis}{async_kw}fn {fn_name}({params_str}) -> Value {{\n    {replacement}\n}}"
    
    # 替换原函数
    new_content = content[:match.start()] + new_fn + content[end:]
    print(f"✅ 替换函数: {fn_name}")
    return new_content

if __name__ == "__main__":
    main()
