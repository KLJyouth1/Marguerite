# G 期 · Web 端退役冻结清单

> 生成日期:2026-08-06
> 依据:`univagent-consolidation-to-marguerite-v1.md` 第 302 行(G 期定义) + 决策 D12(放弃 web 端) + D18(边整边冻 web)
> 操作性质:**可逆重命名冻结(非删除)**,随时可改名还原。

## 决策背景

Marguerite(Rust) 为唯一整合目标,web 端(nexustar-ui / desktop Tauri / website / admin-backend / gwwc web)整体放弃。
对应能力已在前面各期以 Rust 原生二次落地:

| 退役 web 端 | 对应已落地的 Marguerite 原生模块 |
|---|---|
| nexustar-ui(React/Tauri GUI) | `tui/bloom/`(Bloom TUI 原生美学,F 期)+ `tools/`(文件网关) |
| desktop(Tauri WebView 壳) | `desktop/`(egui 原生 GUI crate,D16 已定) |
| nexustar-website / -next(官网) | 整体放弃(社区/会员不在 TUI 整合范围) |
| admin-backend(Java) | `auth/`(账号下沉为本地优先,仅数据能力) |
| gwwc(遗留 web) | `skills/`(技能审查并入) |

按 D3:GUI_Agent 代码**仅作能力/架构审查参考,禁止直接搬运**。本次冻结即落实"参考完毕、代码退役"。

## 冻结目标(已执行)

根目录:`D:\Downloads\GUI_Agent\`

| 原目录 | 冻结后 | 类型 |
|---|---|---|
| `nexustar-ui/` | `nexustar-ui.frozen-20260806/` | React/Tauri 前端 |
| `desktop/` | `desktop.frozen-20260806/` | Tauri WebView 壳 |
| `nexustar-website/` | `nexustar-website.frozen-20260806/` | Vite 官网 |
| `nexustar-website-next/` | `nexustar-website-next.frozen-20260806/` | Next.js 官网 |
| `admin-backend/` | `admin-backend.frozen-20260806/` | Java SpringBoot |
| `gwwc/` | `gwwc.frozen-20260806/` | 遗留(含 web/) |

## 保留(未冻结)

| 目录 | 原因 |
|---|---|
| `nexustar-api/` | 第 302 行明文"仅留作参考";Python 后端能力主供给源,待后续按需审查 |
| `nexusops/` | web 运维层,待二次开发为 Marguerite CLI 子命令(doctor 模式),不在 G 期冻结清单 |
| `nexustar-qa/` | QA 验证脚本,非 web 产品 |

## 还原方法(如需)

```bash
cd D:/Downloads/GUI_Agent
for d in nexustar-ui desktop nexustar-website nexustar-website-next admin-backend gwwc; do
  if [ -e "$d.frozen-20260806" ]; then mv "$d.frozen-20260806" "$d"; fi
done
```

## 门禁结果

- Marguerite 本体(`D:\Downloads\Marguerite\marguerite\`)零接触,测试套件 `cargo test -p marguerite-core` 仍 **全绿无回归**（冻结前 **784 passed【截至 2026-08-06 冻结时历史快照】**，冻结后复测同；后续 Marguerite 演进至 909 passed／911 total，详见 `docs/reconciliation-audit-report.md`）。
- web 端从整合目标中彻底移除,统一收敛到 Rust 单引擎 + 双原生前端(TUI ratatui / 桌面 egui)。
