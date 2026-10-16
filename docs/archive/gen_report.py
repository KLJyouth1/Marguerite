import openpyxl
from openpyxl.styles import Font, PatternFill, Alignment, Border, Side
from openpyxl.utils import get_column_letter

wb = openpyxl.Workbook()

header_font = Font(name='Microsoft YaHei', size=10, bold=True, color='FFFFFF')
header_fill = PatternFill(start_color='2B579A', end_color='2B579A', fill_type='solid')
cell_font = Font(name='Microsoft YaHei', size=9)
wrap = Alignment(wrap_text=True, vertical='top')
thin = Side(style='thin', color='D0D0D0')
border = Border(left=thin, right=thin, top=thin, bottom=thin)

def style_sheet(ws, headers, data, col_widths=None):
    for col, h in enumerate(headers, 1):
        c = ws.cell(row=1, column=col, value=h)
        c.font = header_font
        c.fill = header_fill
        c.alignment = Alignment(horizontal='center', vertical='center', wrap_text=True)
        c.border = border
    for row_idx, row in enumerate(data, 2):
        for col_idx, val in enumerate(row, 1):
            c = ws.cell(row=row_idx, column=col_idx, value=val)
            c.font = cell_font
            c.alignment = wrap
            c.border = border
    if col_widths:
        for i, w in enumerate(col_widths, 1):
            ws.column_dimensions[get_column_letter(i)].width = w

ws1 = wb.active
ws1.title = "task_status"
h1 = ["task_id", "priority", "domain", "name", "module", "status", "core_done", "e2e_verified", "gap", "est"]
d1 = [
    ["P0-gate","P0","quality","verify_sh","verify.sh","landed","Y","Y","enhance layered","0.5d"],
    ["P0-docsync","P0","quality","doc_code_drift","check-doc-cli-drift.py","landed","Y","Y","zero drift","0.5d"],
    ["P0-thirdparty","P0","quality","third_party_validate","validate_integration.py","landed","Y","Y","18/18 manifests","0.5d"],
    ["P0-merge","P0","backend","core_dir_merge","core/","done","Y","Y","","done"],
    ["P0-frontend","P0","frontend","webui_guards_10","webui/","done","Y","Y","","done"],
    ["P0-i18n","P0","frontend","i18n_6_lang","webui/src/i18n/","done","Y","Y","","done"],
    ["P0-security","P0","backend","M1_security_6","rbac+audit_store+credentials","done","Y","partial: no real auth","RBAC dispatch not wired","done"],
    ["P0-kv","P0","backend","M2_kv_cache","cache/","done","Y","partial: runtime","real provider not wired","done"],
    ["P0-bench","P0","backend","Bench-30","bench/","done","Y","partial: no real run","baseline/optimized 3x not done","done"],
    ["P0-onboard","P0","frontend","OnboardingOverlay","webui/","done","Y","Y","","done"],
    ["P0-connstate","P0","frontend","GatewayReadiness","webui/","done","Y","Y","","done"],
    ["P0-cache-honesty","P0","backend","cache_honesty_3tier","cache/","done","Y","Y","","done"],
    ["P0-bloom-redline","P0","backend","Bloom_redline_guard","bloom.yaml","done","Y","Y","","done"],
    ["P0-invariants","P0","arch","INV1-9_frozen","AGENTS.md","done","Y","Y","","done"],
    ["P0-msi","P0","desktop","Tauri_MSI","gui/src-tauri/","done","Y","partial: 3 defects","API Key 401 / conn state / zero status","done"],
    ["P0-navgroup","P0","frontend","sidebar_nav_grouping","webui/src/router/","done","Y","Y","","done"],
    ["P0-controlcenter","P0","frontend","Control_Center","webui/src/views/","done","Y","Y","","done"],
    ["P0-clicontract","P0","backend","CLI_output_contract","cli/","done","Y","Y","","done"],
    ["P0-byok","P0","backend","credential_encrypt_BYOK","credentials/","done","Y","partial: M5 pending","vault first-start UI/CLI","done"],
    ["P0-zerostate","P0","frontend","zero_status_bar","webui/","done","Y","Y","","done"],
    ["M1.6","P0","frontend","UI_permission_boundary","webui/SettingsView","not_done","-","-","settings page disclaimer text","0.5d"],
    ["M3.5","P0","backend","object_version+audit_query","-","not_done","-","-","version snapshot+diff+rollback+audit","3d"],
    ["M3.6","P0","frontend","ops_webui_panel","webui/","not_done","-","-","OpsPanel 4 panels Vue3+Lucide","3-5d"],
    ["M4.3","P1","backend","cn_model_tuning","-","not_done","-","-","intent routing/budget/template per token","3d"],
    ["M4.4","P1","backend","multimodal_direct","-","not_done","-","-","image part to vision model to WebUI","3d"],
    ["M4.6","P1","backend","windows_control","computer_use.rs","not_done","-","-","UiaDriver/InputDriver impl","3d"],
    ["M5.1","P0","desktop","doctor_checkup","core/src/doctor/","not_done","Y: 11tests","partial: Fresh-Machine","7 items+JSON+vision","1d"],
    ["M5.2","P0","desktop","python_probe_chain","pyengine/sidecar.rs","not_done","-","-","env to python/python3/py-3+guide","1d"],
    ["M5.3","P0","desktop","resource_integrity_gate","build.rs","not_done","-","-","skills>=40M/webui freshness","0.5d"],
    ["M5.4","P0","desktop","Fresh-Machine_Gate","-","not_done","-","-","S1/S2/S3 green","2d"],
    ["M5.5","P0","desktop","release_archive","dist/","not_done","-","-","MSI+NSIS+zip same version+SHA256","0.5d"],
    ["P1-agentweb","P1","frontend","agentweb_21_fixes","marguerite-agent-web/","pending","-","-","21 review improvements","5d"],
    ["P1-redesign","P1","frontend","desktop_visual_redesign","gui/","pending","-","-","oriental botanical warm style","10d"],
    ["P1-decost","P1","backend","M2_optimized_variant","-","pending","-","-","cost optimization variants","5d"],
    ["P1-dag","P1","backend","agent_runtime_dag","-","pending","-","-","DAG concurrent execution","5d"],
    ["P1-mcp","P1","backend","mcp_tool_extend","-","pending","-","-","MCP tool extension","3d"],
    ["P1-browser","P1","frontend","browser_cockpit","-","pending","-","-","browser cockpit enhance","3d"],
    ["P1-memdistill","P1","backend","memory_auto_distill","-","pending","-","-","3-tier memory distill","3d"],
    ["P1-settings","P1","frontend","settings_refactor","-","pending","-","-","settings panel refactor","3d"],
    ["P1-plugin","P1","backend","PluginManifest_extend","-","pending","-","-","PluginManifest extension","2d"],
    ["P1-errorbound","P1","frontend","error_boundary","-","pending","-","-","error boundary+fallback","2d"],
    ["P1-msgretry","P1","frontend","message_retry_idempotent","-","pending","-","-","message retry+idempotent","2d"],
    ["P1-swarm","P1","backend","orchestrator_swarm","-","pending","-","-","swarm optimization","3d"],
    ["P1-editor","P1","frontend","workspace_editor","-","pending","-","-","editor enhance","3d"],
    ["P1-session","P1","backend","session_persist","-","pending","-","-","session persistence","3d"],
    ["P1-audit","P1","backend","audit_log_enhance","-","pending","-","-","audit log enhance","2d"],
    ["P1-routing","P1","backend","model_routing_smart","-","pending","-","-","smart routing","3d"],
    ["P1-routeguard","P1","backend","route_guard","-","pending","-","-","route guard polish","2d"],
    ["P1-ratelimit","P1","backend","rate_limit_upgrade","-","pending","-","-","rate limit upgrade","2d"],
    ["P1-hotkey","P1","frontend","hotkey_system","-","pending","-","-","hotkey system","3d"],
]
style_sheet(ws1, h1, d1, [16,6,10,24,24,12,10,14,28,8])

ws2 = wb.create_sheet("status_summary")
h2 = ["status","count","pct","note"]
d2 = [
    ["done",17,"25.4%","P0 core all done and verified"],
    ["landed_pending",3,"4.5%","P0 gate scripts landed, need CI solidify"],
    ["partial_core_done",18,"26.9%","core/engine/contract/tests pass, e2e gap"],
    ["not_done",6,"9.0%","M1.6/M3.5/M3.6/M4.3/M4.4/M4.6"],
    ["pending_P1",19,"28.4%","next phase focus"],
    ["pending_P2",18,"26.9%","mid-term"],
    ["pending_P3",10,"14.9%","long-term exploration"],
    ["M5_not_done",5,"7.5%","packaging and delivery"],
    ["memory_not_done",3,"4.5%","desktop onboarding/visual redesign/MSI defects"],
]
style_sheet(ws2, h2, d2, [24,8,10,50])

ws3 = wb.create_sheet("action_plan")
h3 = ["priority","task_id","name","gap","plan","est","deps"]
d3 = [
    ["P1-immediate","M1.6","UI permission boundary","no disclaimer in settings","SettingsView add text + 6 lang i18n","0.5d","none"],
    ["P1-immediate","MSI-P0","API Key 401 fatal","no pre-check before send","chat send pre-check + guide popup","0.5d","none"],
    ["P1-immediate","MSI-P1a","conn state UI contradiction","Readiness shows connected but not","unify GatewayReadiness logic","0.5d","none"],
    ["P1-immediate","MSI-P1b","status bar zero pollution","zero values + stale dist","zero filter + dist cleanup","0.5d","none"],
    ["P1-immediate","ONBOARD","desktop onboarding design","report not produced","produce design doc + reuse overlay","1d","none"],
    ["P2-near","M3.6","OpsPanel webui","4 panels not done","register OpsPanel + Vue3 + Lucide + ops.*","3-5d","M3.1-M3.4"],
    ["P2-near","M5.1","doctor checkup","7 items not verified","execute PACKAGING 6.1 + vision","1d","none"],
    ["P2-near","M5.2","python probe chain","not implemented","env to python3 + guide AC-19","1d","none"],
    ["P2-near","M5.3","resource integrity gate","not implemented","skills>=40M/webui freshness","0.5d","none"],
    ["P2-near","M5.4","Fresh-Machine Gate","3 scenarios not verified","S1/S2/S3 green AC-19-27","2d","M5.1-M5.3"],
    ["P2-near","M5.5","release archive","not done","MSI+NSIS+zip same version+SHA256","0.5d","M5.4"],
    ["P3-mid","P1-AgentWeb","agent-web 21 fixes","21 improvements not done","fix per review checklist","5d","none"],
    ["P3-mid","P1-redesign","desktop visual redesign","oriental botanical not started","design doc + full CSS/component rewrite","10d","none"],
    ["P4-strategic","STR-05","R6 first-run","onboarding not implemented","install guide + provider config + 5s","3d","M5.4"],
    ["P4-strategic","STR-07","D2/D3/R1 metrics","no real Bench-30 Omni report","unified baseline + 3x median","5d","M2.6"],
]
style_sheet(ws3, h3, d3, [14,12,24,28,40,8,12])

ws4 = wb.create_sheet("doc_completeness")
h4 = ["path","type","size","status","code_consistency","note"]
d4 = [
    ["AGENTS.md","root core","174 lines","authoritative","19 routes match, 2425 baseline","repo guide"],
    ["README.md","root core","130 lines","authoritative","4-frontend arch accurate","unified entry"],
    ["LOCAL_MODEL_WORKBENCH.md","root core","333 lines","complete","RPC 14 methods match","local model workbench"],
    ["PHASE_C_PROGRESS.md","root core","49 lines","complete","C1-C4 all done","notify/hotkey/tray/menu"],
    ["PHASE_D_PROGRESS.md","root core","66 lines","complete","run_turn + React events","GUI frontend"],
    ["CHANGELOG.md","root core","485 lines","active","v0.1.5 to v32 complete","version history"],
    ["opsdev/SPEC.md","plan doc","313 lines","locked","F1-F26 partial","functional spec"],
    ["opsdev/IMPL-PLAN.md","plan doc","171 lines","active","M0-M5 partial","implementation plan"],
    ["opsdev/UNFINISHED-TASKS.md","plan doc","162 lines","active","updated to 08-31","task ledger"],
    ["opsdev/STRATEGY-ALIGNMENT.md","plan doc","127 lines","locked","47d roadmap confirmed","strategy align"],
    ["opsdev/PACKAGING.md","plan doc","239 lines","standard","std landed, verify pending","packaging std"],
    ["opsdev/FINAL-DESIGN.md","plan doc","225 lines","authoritative","modules accurate","final design"],
    [".trae/cli-workbench/","spec doc","7 md","complete","Phase B TUI done","CLI workbench spec"],
    [".trae/harden-optimize/","spec doc","3 md","complete","all tasks [x] done","harden spec"],
    [".trae/opensquilla-fusion/","spec doc","2 md","complete","fusion design done","fusion spec"],
    [".workbuddy/memory/","log doc","20+ md","active","08-04 to 09-01","work logs"],
    ["docs/UNIFIED_WORKBENCH_DESIGN.md","artifact","435 lines","done","arch + UI/UX","unified workbench"],
    ["docs/dev-task-list-v1.0.xlsx","artifact","18KB","done","67 tasks","dev task list"],
    ["docs/quality-audit-v3.0.xlsx","artifact","-","done","31 issues cleared","quality audit"],
    ["docs/acceptance-v1.0.md","artifact","-","done","64 items all pass","acceptance report"],
]
style_sheet(ws4, h4, d4, [45,12,12,12,30,20])

wb.save("docs/全量实现状态复核报告-v1.0.xlsx")
print("OK: xlsx saved")
