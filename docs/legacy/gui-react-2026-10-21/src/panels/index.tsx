import { memo } from "react"
import { AutomationPanel } from "./AutomationPanel"
import { CapabilitiesPanel } from "./CapabilitiesPanel"
import { ComputerUsePanel } from "./ComputerUsePanel"
import { DoctorPanel } from "./DoctorPanel"
import { EvolvePanel } from "./EvolvePanel"
import { FilePanel } from "./FilePanel"
import { FleetPanel } from "./FleetPanel"
import { GraphPanel } from "./GraphPanel"
import { KbPanel } from "./KbPanel"
import { McpPanel } from "./McpPanel"
import { ModlensPanel } from "./ModlensPanel"
import { PetPanel } from "./PetPanel"
import { PlaceholderPanel } from "./PlaceholderPanel"
import { PluginsPanel } from "./PluginsPanel"
import { PokerPanel } from "./PokerPanel"
import { RoutingPanel } from "./RoutingPanel"
import { SandboxPanel } from "./SandboxPanel"
import { SessionsPanel } from "./SessionsPanel"
import { SkillsPanel } from "./SkillsPanel"
import { UsagePanel } from "./UsagePanel"
import { WorkspacePanel } from "./WorkspacePanel"

function ToolPanelsImpl({ nav, running }: { nav: string; running: boolean }) {
  switch (nav) {
    case "doctor": return <DoctorPanel />;
    case "skills": return <SkillsPanel />;
    case "mcp": return <McpPanel />;
    case "sessions": return <SessionsPanel />;
    case "sandbox": return <SandboxPanel />;
    case "computer_use": return <ComputerUsePanel />;
    case "kb": return <KbPanel />;
    case "fleet": return <FleetPanel />;
    case "evolve": return <EvolvePanel />;
    case "plugins": return <PluginsPanel />;
    case "capabilities": return <CapabilitiesPanel />;
    case "workspace": return <WorkspacePanel />;
    case "routing": return <RoutingPanel />;
    case "modlens": return <ModlensPanel />;
    case "poker": return <PokerPanel />;
    case "graph": return <GraphPanel />;
    case "usage": return <UsagePanel />;
    case "pet": return <PetPanel running={running} />;
    case "files": return <FilePanel />;
    case "automation": return <AutomationPanel />;
    // 以下两项 core 里没有对应能力实现（不是"没接线"，是"功能本身不存在"）。
    // 保留入口但**如实说明**，而不是写"正在开发中，敬请期待"——
    // 后者让人以为排期在做，前者告诉用户现在该怎么达成目的。
    case "office_assistant": return (
      <PlaceholderPanel
        title="办公助理"
        subtitle="文档 / 表格 / 演示的批量处理"
        status="unsupported"
        hint="当前版本未内置办公套件处理。可用的替代路径：① 在对话里直接上传文件让模型处理；② 用「自动化」面板设定时任务；③ 通过 MCP 接入外部文档服务。"
      />
    );
    case "templates": return (
      <PlaceholderPanel
        title="模板库"
        subtitle="任务模板与预设指令集"
        status="unsupported"
        hint="当前版本没有独立的模板库。可用的替代路径：① 用「技能」面板管理可复用能力；② 在对话框输入 @ 引用专家；③ 用「自动化」保存重复任务。"
      />
    );
    default: return null;
  }
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const ToolPanels = memo(ToolPanelsImpl);
