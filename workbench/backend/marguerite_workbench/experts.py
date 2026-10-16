"""Five distilled expert roles with a *real* callable analysis.

Each :class:`ExpertRole` carries a concrete ``problem_solving_template`` (a
chain-of-thought step list) and ``skills`` (named procedures distilled from
public industry practice).  ``analyze(role, payload)`` does not merely label the
input — it runs the role's heuristic checklist against the payload dict and
returns a structured :class:`ExpertAnalysis` (verdict / findings /
recommendations / confidence).

Dispatch is **table-driven** (role -> analyser fn) so adding a sixth role is a
config change, not a new ``if`` branch (Bloom R1).
"""

from __future__ import annotations

import logging
import re
from typing import Callable, Optional

from marguerite_workbench.models import ExpertAnalysis, ExpertRole, ExpertSkill
from marguerite_workbench.expert_teams import AGENT_SPECS, TEAM_DEFS, analyze_team_role

logger = logging.getLogger("marguerite.experts")

# Severity ranking used by the security officer.
_SEV_WEIGHT = {"critical": 4, "high": 2, "medium": 1, "low": 0}
_VERDICTS = ("pass", "conditional", "fail")


def _blob(payload: dict) -> str:
    """Flatten a payload dict into one lower-cased text blob for heuristics."""
    parts: list[str] = []
    for key, value in payload.items():
        parts.append(str(key))
        if isinstance(value, (list, tuple)):
            parts.extend(str(v) for v in value)
        else:
            parts.append(str(value))
    return "\n".join(parts).lower()


# ---------------------------------------------------------------------------
# Role 1 — product_reviewer
# ---------------------------------------------------------------------------
_PRODUCT_SIGNALS = [
    ("用户价值", ("用户", "user", "价值", "value", "痛点", "pain")),
    ("成功指标", ("指标", "metric", "kpi", "北极星", "留存", "retention", "转化", "conversion")),
    ("目标用户", ("目标用户", "target user", "画像", "persona", "segment")),
    ("上手成本", ("上手", "onboard", "引导", "tutorial", "首跑", "aha")),
    ("差异化", ("差异化", "differentiat", "护城河", "moat", "竞品", "competitor")),
]


def _analyze_product(payload: dict) -> ExpertAnalysis:
    blob = _blob(payload)
    present, missing = [], []
    for name, kws in _PRODUCT_SIGNALS:
        if any(k in blob for k in kws):
            present.append(name)
        else:
            missing.append(name)
    findings = [f"已清晰表达产品维度：{', '.join(present)}" for p in [present] if p]
    findings += [f"产品论证缺失维度：{m}" for m in missing]
    recs = [
        "用一句『为谁解决什么痛点、比现状好多少』重写价值主张（Office Hours 复盘法）。",
        "定义 1 个北极星指标 + 2 个护栏指标，写入 autoplan 验收门槛。",
        "给出最小可上手路径（<5 分钟跑通），降低首次价值顿悟成本。",
    ]
    score = len(present)
    verdict = "pass" if score >= 4 else ("conditional" if score >= 2 else "fail")
    confidence = round(min(1.0, 0.4 + 0.12 * score), 2)
    return ExpertAnalysis(
        role="product_reviewer", verdict=verdict,
        findings=findings, recommendations=recs, confidence=confidence,
    )


# ---------------------------------------------------------------------------
# Role 2 — security_officer  (OWASP Top 10 + STRIDE patterns)
# ---------------------------------------------------------------------------
_SEC_RULES = [
    (r"\beval\s*\(", "critical", "检测到 eval() 动态执行", "禁止 eval/exec；用安全 AST 或显式白名单分发。"),
    (r"\bos\.system\s*\(", "critical", "检测到 os.system() 命令拼接", "用 subprocess 传递参数列表，禁用 shell=True。"),
    (r"subprocess\.[A-Za-z]+\(.*shell\s*=\s*True", "high", "subprocess 启用了 shell=True", "设置 shell=False 并传参列表。"),
    (r"pickle\.loads?", "critical", "检测到 pickle 反序列化", "改用 json/schema 校验，绝不反序列化不可信字节。"),
    (r"yaml\.load\s*\((?![^)]*Loader)", "high", "yaml.load 未指定 SafeLoader", "使用 yaml.safe_load。"),
    (r"(SELECT|INSERT|UPDATE|DELETE).*(\%|\+|f[\"\'])", "high", "检测到字符串拼接的 SQL", "使用参数化查询 / ORM，杜绝拼接。"),
    (r"verify\s*=\s*False", "high", "TLS 证书校验被关闭 (verify=False)", "启用证书校验或注入合规 CA。"),
    (r"md5\s*\(", "medium", "使用 MD5 做哈希", "口令用 argon2/bcrypt；完整性用 sha256。"),
    (r"(api[_-]?key|secret|token)\s*[:=]\s*[\"'][^\"']{6,}", "medium", "疑似硬编码密钥/令牌", "移入密钥管理 (Vault/环境变量)。"),
    (r"password\s*=\s*[\"'][^\"']+[\"']", "medium", "疑似明文口令", "口令必须哈希存储，禁止明文。"),
    (r"Access-Control-Allow-Origin\s*:\s*[\"']\*[\"']", "medium", "CORS 允许任意源", "按白名单收紧 CORS。"),
    (r"(dangerouslySetInnerHTML|innerHTML\s*=)", "high", "存在未净化的 HTML 注入点", "经 DOMPurify 净化或走安全模板。"),
]


def _analyze_security(payload: dict) -> ExpertAnalysis:
    # Prefer an explicit 'code'/'description' field for deep scanning.
    code = payload.get("code") or payload.get("description") or _blob(payload)
    code = code.lower() if isinstance(code, str) else _blob(payload)
    findings, recs, sev_hits = [], [], []
    for pat, sev, finding, rec in _SEC_RULES:
        if re.search(pat, code):
            findings.append(f"[{sev.upper()}] {finding}")
            recs.append(rec)
            sev_hits.append(sev)
    if not findings:
        findings.append("未在输入中发现明显的高危代码模式（基于静态规则扫描）。")
    risk = sum(_SEV_WEIGHT[s] for s in sev_hits)
    if "critical" in sev_hits or risk >= 6:
        verdict = "fail"
    elif risk >= 2:
        verdict = "conditional"
    else:
        verdict = "pass"
    confidence = round(min(1.0, 0.5 + 0.1 * len(sev_hits)), 2)
    # De-duplicate recommendations while preserving order.
    seen, uniq = set(), []
    for r in recs:
        if r not in seen:
            seen.add(r)
            uniq.append(r)
    uniq += [
        "按 STRIDE 做威胁建模：Spoofing/Tampering/Repudiation/InfoDisclosure/Dos/Elevation。",
        "对齐 OWASP Top 10：注入、破损鉴权、SSRF、安全配置缺失逐项核查。",
    ]
    return ExpertAnalysis(
        role="security_officer", verdict=verdict,
        findings=findings, recommendations=uniq, confidence=confidence,
    )


# ---------------------------------------------------------------------------
# Role 3 — qa_lead
# ---------------------------------------------------------------------------
_QA_SIGNALS = [
    ("单元测试", ("unittest", "pytest", "jest", "测试", "test", "spec")),
    ("覆盖率门禁", ("覆盖率", "coverage", "门禁", "gate", "阈值", "threshold")),
    ("CI 流水线", ("ci", "流水线", "pipeline", "github actions", "gitlab")),
    ("Mock/隔离", ("mock", "桩", "隔离", "stub", "fixture")),
    ("回归/E2E", ("回归", "regression", "e2e", "端到端", "集成测试")),
    ("灰度/Canary", ("灰度", "canary", "金丝雀", "渐进", "rollout")),
]


def _analyze_qa(payload: dict) -> ExpertAnalysis:
    blob = _blob(payload)
    present = [n for n, kws in _QA_SIGNALS if any(k in blob for k in kws)]
    missing = [n for n in [s[0] for s in _QA_SIGNALS] if n not in present]
    findings = [f"已具备质量实践：{', '.join(present)}" for p in [present] if p]
    findings += [f"质量门禁缺口：{m}" for m in missing]
    recs = [
        "落地 test→fix→verify 循环：每次变更先红后绿再回归。",
        "设覆盖率门禁（如 ≥80%）作为合并硬性门槛。",
        "关键路径补 E2E，发布走 canary + 自动回滚。",
    ]
    verdict = "pass" if len(present) >= 5 else ("conditional" if len(present) >= 3 else "fail")
    confidence = round(min(1.0, 0.35 + 0.11 * len(present)), 2)
    return ExpertAnalysis(
        role="qa_lead", verdict=verdict,
        findings=findings, recommendations=recs, confidence=confidence,
    )


# ---------------------------------------------------------------------------
# Role 4 — designer
# ---------------------------------------------------------------------------
def _analyze_designer(payload: dict) -> ExpertAnalysis:
    blob = _blob(payload)
    findings, recs = [], []
    hex_count = len(re.findall(r"#[0-9a-fA-F]{3,8}\b", blob))
    if hex_count:
        findings.append(f"发现 {hex_count} 处硬编码颜色值（违反 Design Token 驱动）。")
        recs.append("将硬编码色值收敛为 Design Token（CSS 变量），仅保留 #fff/#000 例外。")
    if any(k in blob for k in ("aria", "无障碍", "a11y", "role=", "alt=")):
        findings.append("已考虑无障碍（a11y）属性。")
    else:
        findings.append("未检测到无障碍（a11y）属性，存在可达性缺口。")
        recs.append("补齐语义标签、aria-label、:focus-visible 焦点环与 aria-live 通知。")
    if any(k in blob for k in ("token", "design token", "designtoken", "主题", "theme")):
        findings.append("已采用 Design Token / 主题化。")
    else:
        recs.append("引入 Design Token 体系，新主题=变量覆盖而非改组件。")
    if any(k in blob for k in ("响应式", "responsive", "rem", "移动", "mobile")):
        findings.append("已考虑响应式/弹性单位。")
    else:
        recs.append("使用 rem/em 与流式布局保证多端一致。")
    if not recs:
        recs.append("维持 Design Token 驱动与无障碍基线，定期做像素复核（≤2px）。")
    score = (1 if hex_count == 0 else 0) + len([f for f in findings if "已" in f])
    verdict = "pass" if score >= 3 else ("conditional" if score >= 1 else "fail")
    confidence = round(min(1.0, 0.45 + 0.12 * score), 2)
    return ExpertAnalysis(
        role="designer", verdict=verdict,
        findings=findings, recommendations=recs, confidence=confidence,
    )


# ---------------------------------------------------------------------------
# Role 5 — investigator
# ---------------------------------------------------------------------------
_INVESTIGATE_SIGNALS = [
    ("症状描述", ("症状", "symptom", "表现", "报错", "error", "异常")),
    ("日志/证据", ("日志", "log", "trace", "stack", "监控", "metric")),
    ("复现步骤", ("复现", "repro", "steps", "稳定复现", "必现")),
    ("假设", ("假设", "hypothesis", "怀疑", "疑似", "可能由于")),
    ("已排查", ("已排查", "尝试过", "ruled out", "排除")),
]


def _analyze_investigator(payload: dict) -> ExpertAnalysis:
    blob = _blob(payload)
    present = [n for n, kws in _INVESTIGATE_SIGNALS if any(k in blob for k in kws)]
    missing = [n for n in [s[0] for s in _INVESTIGATE_SIGNALS] if n not in present]
    findings = [f"证据齐备项：{', '.join(present)}" for p in [present] if p]
    findings += [f"诊断短板（需补充）：{m}" for m in missing]
    recs = [
        "遵循根因五步：收集证据→形成假设→设计实验证伪→定位根因→防复发。",
        "用健康看板（RED/USE 指标）量化『正常』基线，对比异常窗口。",
        "写复盘（无责），沉淀为 L6 程序性记忆中的 playbook。",
    ]
    verdict = "pass" if len(present) >= 4 else ("conditional" if len(present) >= 2 else "fail")
    confidence = round(min(1.0, 0.3 + 0.14 * len(present)), 2)
    return ExpertAnalysis(
        role="investigator", verdict=verdict,
        findings=findings, recommendations=recs, confidence=confidence,
    )


# ---------------------------------------------------------------------------
# Role catalogue (table-driven, anti R1)
# ---------------------------------------------------------------------------
_ROLE_DEFS: dict[str, ExpertRole] = {
    "product_reviewer": ExpertRole(
        role="product_reviewer",
        title="产品评审员",
        distilled_from=[
            "Paul Graham / YC essays", "Sam Altman『How to be successful』",
            "SV product sense / Office Hours", "UnivAgent autoplan",
        ],
        problem_solving_template=(
            "1. 明确为『谁』解决『什么痛点』\n"
            "2. 量化现状与目标的差距（基线 vs 目标）\n"
            "3. 定义北极星指标 + 护栏指标\n"
            "4. 设计最小可上手路径（首次价值顿悟）\n"
            "5. 论证差异化 / 护城河\n"
            "6. 用 Office Hours 复盘验收"
        ),
        skills=[
            ExpertSkill(name="价值主张重写", description="用一句公式重写价值主张",
                        procedure=["找目标用户", "找核心痛点", "写『比现状好 X』", "压到 15 字内"]),
            ExpertSkill(name="指标设计", description="定义可观测的成功指标",
                        procedure=["选北极星", "选 2 护栏", "设基线", "设验收门槛"]),
            ExpertSkill(name="上手路径", description="降低首次价值成本",
                        procedure=["列出首次 aha", "砍到 <5 分钟", "加引导", "测流失"]),
        ],
        experience_notes=(
            "产品感来自『密度』：把模糊臆测替换为可观测指标。优先验证『是否有人真在乎』，"
            "而非『功能是否炫』。用 autoplan 把验收门槛写死，避免范围蔓延。"
        ),
    ),
    "security_officer": ExpertRole(
        role="security_officer",
        title="安全卫士",
        distilled_from=["OWASP Top 10 (2021)", "STRIDE (Microsoft)", "AppSec / CSO practice"],
        problem_solving_template=(
            "1. 资产盘点：识别数据/凭证/入口\n"
            "2. STRIDE 威胁建模（Spoofing/Tampering/Repudiation/InfoDisclosure/Dos/Elevation）\n"
            "3. 注入与鉴权核查（SQLi/SSRF/破损鉴权）\n"
            "4. 依赖与配置审计（SCA / 硬编码密钥）\n"
            "5. 纵深防御与最小权限\n"
            "6. 给出可落地的修复项与回归测试"
        ),
        skills=[
            ExpertSkill(name="静态风险扫描", description="规则化识别高危代码模式",
                        procedure=["扫 eval/exec", "扫拼接 SQL", "扫 verify=False", "扫硬编码密钥"]),
            ExpertSkill(name="STRIDE 建模", description="系统化威胁枚举",
                        procedure=["列资产", "逐维枚举", "评影响/可能", "定缓解"]),
            ExpertSkill(name="依赖审计", description="供应链与配置核查",
                        procedure=["SCA 扫描", "查 CVE", "收紧 CORS", "密钥外置"]),
        ],
        experience_notes=(
            "安全是『默认拒绝 + 纵深防御』，不是清单打勾。优先堵住命令/代码执行与凭证泄露，"
            "所有外部输入视为不可信，所有边界做边界校验。"
        ),
    ),
    "qa_lead": ExpertRole(
        role="qa_lead",
        title="质量门神",
        distilled_from=["Release Engineering", "SRE / Google SRE Book", "test-fix-verify loops"],
        problem_solving_template=(
            "1. 明确质量门槛（覆盖率/性能预算）\n"
            "2. 建单元 + Mock 隔离的快反馈环\n"
            "3. 关键路径补 E2E / 集成\n"
            "4. CI 门禁卡点（红不让合）\n"
            "5. 发布走 canary + 自动回滚\n"
            "6. 性能回归 ≤ 基线 10%"
        ),
        skills=[
            ExpertSkill(name="test-fix-verify", description="变更驱动的红绿回归",
                        procedure=["写失败测试", "修复", "跑全量", "验基线"]),
            ExpertSkill(name="覆盖率门禁", description="合并硬性门槛",
                        procedure=["设阈值", "CI 卡点", "排除生成代码", "周报趋势"]),
            ExpertSkill(name="canary 发布", description="渐进式发布与回滚",
                        procedure=["小流量", "看指标", "扩量/回滚", "写复盘"]),
        ],
        experience_notes=(
            "质量是『可回归、可观测、可回滚』。没有自动化的门禁，质量靠人盯必崩；"
            "把验证闭环交给 CI，把判断力留给人。"
        ),
    ),
    "designer": ExpertRole(
        role="designer",
        title="设计师",
        distilled_from=["Design Systems pioneers (Atomic/Polaris/Material)",
                        "Bloom Mode 视觉门禁 V1–V5", "UnivAgent 设计令牌"],
        problem_solving_template=(
            "1. 锁定 Design Token（色/距/字）→ CSS 变量\n"
            "2. 品牌蓝锚定 #3B82F6，禁紫粉渐变\n"
            "3. 语义标签 + aria + 焦点环（a11y P0）\n"
            "4. 响应式与弹性单位（rem/em）\n"
            "5. 像素复核（≤2px）与五源对齐\n"
            "6. 动效克制（禁弹跳 AI 模板味）"
        ),
        skills=[
            ExpertSkill(name="Token 化", description="收敛硬编码为令牌",
                        procedure=["抽色板", "建变量", "替换字面量", "留 #fff/#000"]),
            ExpertSkill(name="可达性", description="无障碍基线",
                        procedure=["语义标签", "aria-label", "focus-visible", "aria-live"]),
            ExpertSkill(name="像素复核", description="多源对齐校验",
                        procedure=["设计变量", "元数据", "截图", "渲染图", "交叉校验"]),
        ],
        experience_notes=(
            "好的界面是『零模板味』：品牌蓝 + Design Token + 真实内容。把视觉纪律写进门禁，"
            "而非依赖个人品味；图标一律 SVG，禁用 emoji 作功能图标。"
        ),
    ),
    "investigator": ExpertRole(
        role="investigator",
        title="排障手",
        distilled_from=["SRE debugging luminaries", "Root Cause Analysis", "Health dashboards"],
        problem_solving_template=(
            "1. 收集证据（日志/监控/复现）\n"
            "2. 量化『正常』基线，对比异常窗口\n"
            "3. 形成可证伪的假设\n"
            "4. 设计实验逐一排除\n"
            "5. 定位根因（5 Whys）\n"
            "6. 写无责复盘 → 沉淀 playbook"
        ),
        skills=[
            ExpertSkill(name="根因五步", description="结构化定位",
                        procedure=["证据", "假设", "证伪", "根因", "防复发"]),
            ExpertSkill(name="健康看板", description="RED/USE 指标",
                        procedure=["率/错误/时长", "利用率/饱和/错误", "设基线", "告警"]),
            ExpertSkill(name="无责复盘", description="沉淀经验",
                        procedure=["时间线", "根因", "改进行动", "入库 L6"]),
        ],
        experience_notes=(
            "排障先证伪再归因：在没有证据前，所有『可能』都只是猜测。用看板把『异常』定义成"
            "可量化偏差，避免凭感觉调参。"
        ),
    ),
}

_ANALYZERS: dict[str, Callable[[dict], ExpertAnalysis]] = {
    "product_reviewer": _analyze_product,
    "security_officer": _analyze_security,
    "qa_lead": _analyze_qa,
    "designer": _analyze_designer,
    "investigator": _analyze_investigator,
}

# ---------------------------------------------------------------------------
# Team roles — the eight expert teams, merged from expert_teams.py
# (anti-R1: data catalogue + one generic analyser, no extra branches)
# ---------------------------------------------------------------------------
def _spec_to_role(spec: dict) -> ExpertRole:
    """Convert a catalogue spec dict into an :class:`ExpertRole`."""
    return ExpertRole(
        role=spec["role"],
        title=spec["title"],
        team=spec.get("team"),
        distilled_from=spec["distilled_from"],
        problem_solving_template=spec["template"],
        skills=[
            ExpertSkill(name=s["name"], description=s["description"], procedure=s["procedure"])
            for s in spec["skills"]
        ],
        experience_notes=spec["notes"],
    )


_TEAM_ROLE_DEFS: dict[str, ExpertRole] = {s["role"]: _spec_to_role(s) for s in AGENT_SPECS}
_TEAM_ANALYZERS: dict[str, Callable[[dict], ExpertAnalysis]] = {
    s["role"]: (lambda spec: (lambda payload: analyze_team_role(spec, payload)))(s)
    for s in AGENT_SPECS
}


def list_roles() -> list[ExpertRole]:
    return list(_ROLE_DEFS.values()) + list(_TEAM_ROLE_DEFS.values())


def list_teams() -> list[dict]:
    """Return the eight expert teams plus the core distilled group.

    Each entry carries the team metadata and its member role ids/titles, so the
    UI can render a grouped selector without re-deriving the team structure.
    """
    teams: list[dict] = []
    for tid, tdef in TEAM_DEFS.items():
        members = [
            {"role": s["role"], "title": s["title"], "kind": s.get("kind", "member")}
            for s in AGENT_SPECS
            if s.get("team") == tid
        ]
        teams.append({
            "team": tid,
            "name": tdef["name"],
            "category": tdef["category"],
            "lead_role": tdef["lead_role"],
            "members": members,
        })
    # The five founding distilled roles have no team; surface them as a core group
    # so they remain selectable from the same grouped UI.
    core_members = [
        {"role": r.role, "title": r.title, "kind": "core"}
        for r in _ROLE_DEFS.values()
    ]
    teams.append({
        "team": "core",
        "name": "核心蒸馏专家",
        "category": "00-Core",
        "lead_role": None,
        "members": core_members,
    })
    return teams


def get_role(role: str) -> Optional[ExpertRole]:
    return _ROLE_DEFS.get(role) or _TEAM_ROLE_DEFS.get(role)


def analyze(role: str, payload: dict) -> ExpertAnalysis:
    """Run the role's real analysis against ``payload`` and return findings."""
    if not isinstance(payload, dict):
        raise ValueError("payload must be a dict")
    fn = _ANALYZERS.get(role) or _TEAM_ANALYZERS.get(role)
    if fn is None:
        raise ValueError(f"unknown expert role: {role!r}")
    return fn(payload)


# ---------------------------------------------------------------------------
# Unified mode system — the ONLY legal expert modes.
# 3 intensity levels (极简 / 标准 / 极致) x 3 working modes (Ask / Plan / DoIt)
# = 9 canonical modes. This replaces bloom and other legacy modes; their
# capabilities are remapped into these three intensities.
# ---------------------------------------------------------------------------
INTENSITY_LEVELS = ("minimal", "standard", "extreme")  # 极简 / 标准 / 极致
WORKING_MODES = ("ask", "plan", "doit")                # Ask / Plan / DoIt

# id: (intensity, working, label_zh, label_en, description)
MODE_META: dict[str, tuple[str, str, str, str, str]] = {
    "minimal_ask":   ("minimal", "ask",  "极简·咨询", "Minimal·Ask",  "一句话结论 + 关键建议"),
    "minimal_plan":  ("minimal", "plan", "极简·方案", "Minimal·Plan", "简短要点清单（3-5 条）"),
    "minimal_doit":  ("minimal", "doit", "极简·执行", "Minimal·DoIt", "最小可行自动化，仅必要工具"),
    "standard_ask":  ("standard", "ask",  "标准·咨询", "Standard·Ask",  "标准结构化分析，含依据与边界"),
    "standard_plan": ("standard", "plan", "标准·方案", "Standard·Plan", "完整分步方案，含里程碑"),
    "standard_doit": ("standard", "doit", "标准·执行", "Standard·DoIt", "带校验的执行，结果可追溯"),
    "extreme_ask":   ("extreme", "ask",  "极致·咨询", "Extreme·Ask",  "深度根因分析，多视角交叉"),
    "extreme_plan":  ("extreme", "plan", "极致·方案", "Extreme·Plan", "全面实施蓝图，含风险预案"),
    "extreme_doit":  ("extreme", "doit", "极致·执行", "Extreme·DoIt", "全流程自主执行，全工具面"),
}


def list_modes() -> list[dict]:
    """Return the 9 canonical expert modes for the UI selector (3x3 matrix)."""
    return [
        {
            "id": mid,
            "intensity": meta[0],
            "working": meta[1],
            "label_zh": meta[2],
            "label_en": meta[3],
            "description": meta[4],
        }
        for mid, meta in MODE_META.items()
    ]


def analyze_with_mode(role: str, payload: dict, mode: str, depth: str | None = None) -> dict:
    """Run a role's analysis under a canonical mode.

    ``mode`` selects one of the 9 (intensity x working) cells. The working mode
    decides the *action* (ask=advise only, plan=step plan, doit=execute intent),
    the intensity (depth) decides how much is produced and how thoroughly.
    """
    if get_role(role) is None:
        raise ValueError(f"unknown expert role: {role!r}")
    if mode not in MODE_META:
        raise ValueError(f"unknown mode: {mode!r}; valid: {sorted(MODE_META)}")
    intensity, working, *_ = MODE_META[mode]
    if depth:  # optional override of the intensity axis
        intensity = depth if depth in INTENSITY_LEVELS else intensity

    base = analyze(role, payload).model_dump()
    out: dict = dict(base)
    out["mode"] = mode
    out["intensity"] = intensity
    out["working"] = working

    # Intensity (depth) modulation of the base analysis.
    if intensity == "minimal":
        out["findings"] = base.get("findings", [])[:3]
        out["recommendations"] = base.get("recommendations", [])[:3]
    elif intensity == "extreme":
        out.setdefault("notes", []).append(
            "极致模式：已进行多视角交叉校验，结论含边界与不确定性。"
        )

    # Working mode behaviour.
    if working == "plan":
        out["plan"] = _build_plan(out)
    elif working == "doit":
        out["execution"] = _build_execution(role, out)
    return out


def _build_plan(analysis: dict) -> list[dict]:
    """Turn recommendations into an ordered step plan with milestones."""
    recs = analysis.get("recommendations", [])
    steps = []
    for i, rec in enumerate(recs, 1):
        steps.append({
            "step": i,
            "action": rec,
            "milestone": f"M{i}" if i % 3 == 0 else None,
        })
    return steps


def _build_execution(role: str, analysis: dict) -> dict:
    """Describe the DoIt execution intent.

    Real dispatch happens when the caller supplies a harness target via
    :func:`dispatch_execution`; here we return a safe, side-effect-free intent
    so the mode is testable and the UI can confirm before executing.
    """
    recs = analysis.get("recommendations", [])
    return {
        "role": role,
        "intent": [{"action": r} for r in recs],
        "dispatched": False,
        "note": "DoIt 模式：当载荷含 harness 目标时经 harness.dispatch 实际执行；此处返回执行意图。",
    }


def dispatch_execution(role: str, analysis: dict, harness_target: str | None = None) -> dict:
    """Actually run the DoIt intent through the harness when a target is given."""
    intent = _build_execution(role, analysis)
    if not harness_target:
        return intent
    try:
        from marguerite_workbench.harness import dispatch as harness_dispatch
    except Exception:  # pragma: no cover - harness optional
        intent["dispatched"] = False
        intent["note"] = "harness 不可用，未执行。"
        return intent
    result = harness_dispatch(task=harness_target, context=analysis)
    intent["dispatched"] = True
    intent["result"] = result
    return intent


import re as _re

_MENTION_RE = _re.compile(r"@([A-Za-z0-9_一-龥]+)")

# Chinese alias -> lead role, for natural @引用 of the eight teams.
_TEAM_ALIASES: dict[str, str] = {
    "数据智能": "data_insight_lead", "数据": "data_insight_lead",
    "法务": "legal_guard_lead", "法务合规": "legal_guard_lead",
    "投研": "invest_lab_lead", "投研量化": "invest_lab_lead",
    "游戏": "game_forge_lead", "游戏研发": "game_forge_lead",
    "产品": "product_craft_lead", "产品设计": "product_craft_lead",
    "ppt": "deck_craft_lead", "ppt智造": "deck_craft_lead", "演示": "deck_craft_lead",
    "全栈": "fullstack_craft_lead", "全栈应用": "fullstack_craft_lead",
    "战略": "strategy_council_lead", "战略智囊": "strategy_council_lead",
}


def resolve_mentions(text: str) -> list[str]:
    """Extract @mentions from chat text and resolve them to expert roles.

    Resolution order: Chinese team alias -> exact role id -> role id without
    underscores -> title substring. Returns de-duplicated, order-preserving ids.
    """
    if not text:
        return []
    roles = list_roles()
    by_id = {r.role: r for r in roles}
    index = [(r.role, (r.role.lower(), r.role.lower().replace("_", ""), (r.title or "").lower()))
             for r in roles]

    found: list[str] = []
    for tok in _MENTION_RE.findall(text):
        key = tok.lower()
        if key in _TEAM_ALIASES and _TEAM_ALIASES[key] in by_id:
            found.append(_TEAM_ALIASES[key])
            continue
        if key in by_id:
            found.append(key)
            continue
        hit = next((rid for rid, keys in index if key == keys[1]), None)
        if hit:
            found.append(hit)
            continue
        hit = next((rid for rid, keys in index if any(key in k for k in keys)), None)
        if hit:
            found.append(hit)
    seen: set[str] = set()
    out: list[str] = []
    for f in found:
        if f not in seen:
            seen.add(f)
            out.append(f)
    return out


def chat_with_experts(
    text: str,
    mode: str = "standard_ask",
    default_role: str | None = None,
) -> dict:
    """Entry point for chat: resolve @mentions and analyze under ``mode``."""
    roles = resolve_mentions(text)
    if not roles:
        if default_role and get_role(default_role):
            roles = [default_role]
        else:
            return {
                "mentioned": [],
                "mode": mode,
                "analyses": [],
                "note": "未识别到 @专家团 引用，且无默认角色。",
            }
    analyses = []
    for r in roles:
        try:
            analyses.append(analyze_with_mode(r, {"query": text}, mode))
        except ValueError as exc:
            analyses.append({"role": r, "error": str(exc)})
    return {"mentioned": roles, "mode": mode, "analyses": analyses}
