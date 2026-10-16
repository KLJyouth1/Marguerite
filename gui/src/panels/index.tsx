import { memo } from "react"
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
    case "automation": return <PlaceholderPanel title="自动化" subtitle="自动化工作流编排" />;
    case "office_assistant": return <PlaceholderPanel title="办公助理" subtitle="办公场景智能助理" />;
    case "templates": return <PlaceholderPanel title="模板库" subtitle="任务模板库" />;
    default: return null;
  }
}


// === B3: React.memo 包装 — 父级无关状态变更不再触发本组件重渲染 ===
export const ToolPanels = memo(ToolPanelsImpl);
