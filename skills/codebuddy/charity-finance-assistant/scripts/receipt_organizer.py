#!/usr/bin/env python3
"""
票据自动扫描与分类整理脚本
扫描指定文件夹中的票据/发票文件（图片、PDF、Excel），
自动按类型分类、重命名，并生成汇总台账。

用法:
    python receipt_organizer.py <票据文件夹路径> [--output <输出文件夹路径>]

示例:
    python receipt_organizer.py "D:/公益机构/2025年票据"
    python receipt_organizer.py "D:/公益机构/2025年票据" --output "D:/公益机构/整理后"
"""

import os
import sys
import re
import shutil
import csv
import json
from datetime import datetime
from pathlib import Path
from collections import defaultdict

# ============================================================
# 配置
# ============================================================

# 支持的文件类型
IMAGE_EXTS = {".jpg", ".jpeg", ".png", ".bmp", ".tiff", ".tif", ".webp"}
PDF_EXTS = {".pdf"}
EXCEL_EXTS = {".xlsx", ".xls", ".csv"}
ALL_SUPPORTED = IMAGE_EXTS | PDF_EXTS | EXCEL_EXTS

# 分类关键词（文件名/路径中的关键词 → 类别）
CATEGORY_KEYWORDS = {
    "捐赠": "捐赠票据",
    "donation": "捐赠票据",
    "公益事业捐赠": "捐赠票据",
    "捐赠收据": "捐赠票据",
    "发票": "费用发票",
    "invoice": "费用发票",
    "增值税": "费用发票",
    "报销": "报销单据",
    "差旅": "报销单据",
    "交通": "报销单据",
    "住宿": "报销单据",
    "餐费": "报销单据",
    "办公": "报销单据",
    "银行": "银行回单",
    "回单": "银行回单",
    "转账": "银行回单",
    "合同": "合同协议",
    "协议": "合同协议",
    "agreement": "合同协议",
    "工资": "薪酬社保",
    "社保": "薪酬社保",
    "个税": "薪酬社保",
}

# 费用科目关键词（用于支出分类建议）
EXPENSE_KEYWORDS = {
    "差旅": "差旅费",
    "交通": "差旅费",
    "住宿": "差旅费",
    "机票": "差旅费",
    "火车": "差旅费",
    "打车": "差旅费",
    "出租": "差旅费",
    "办公": "办公费",
    "文具": "办公费",
    "打印": "办公费",
    "复印": "办公费",
    "快递": "办公费",
    "邮寄": "办公费",
    "餐费": "会议费/招待费",
    "茶歇": "会议费/招待费",
    "会议": "会议费/招待费",
    "场地": "会议费/招待费",
    "培训": "培训费",
    "讲师": "培训费",
    "教材": "培训费",
    "房租": "房租物业",
    "物业": "房租物业",
    "水电": "房租物业",
    "电话": "通讯费",
    "网络": "通讯费",
    "宽带": "通讯费",
}


# ============================================================
# 核心功能
# ============================================================

def scan_folder(folder_path):
    """扫描文件夹，返回所有支持的文件列表"""
    files = []
    folder = Path(folder_path)
    if not folder.exists():
        print(f"❌ 文件夹不存在: {folder_path}")
        sys.exit(1)

    for f in folder.rglob("*"):
        if f.is_file() and f.suffix.lower() in ALL_SUPPORTED:
            files.append(f)

    return files


def classify_file(file_path):
    """根据文件名和路径中的关键词分类"""
    name_lower = file_path.stem.lower() + " " + str(file_path.parent).lower()

    # 按关键词匹配分类
    for keyword, category in CATEGORY_KEYWORDS.items():
        if keyword in name_lower:
            return category

    # 按文件类型猜测
    ext = file_path.suffix.lower()
    if ext in EXCEL_EXTS:
        return "Excel数据表"
    if ext in PDF_EXTS:
        return "PDF文件"
    if ext in IMAGE_EXTS:
        return "未分类票据图片"

    return "其他"


def suggest_expense_category(file_path):
    """根据文件名猜测费用科目"""
    name_lower = file_path.stem.lower()
    for keyword, category in EXPENSE_KEYWORDS.items():
        if keyword in name_lower:
            return category
    return "待分类"


def extract_date_from_name(file_path):
    """尝试从文件名中提取日期"""
    name = file_path.stem
    # 匹配常见日期格式
    patterns = [
        r"(\d{4})[-_.](\d{1,2})[-_.](\d{1,2})",   # 2025-01-15, 2025_01_15
        r"(\d{4})(\d{2})(\d{2})",                    # 20250115
        r"(\d{1,2})[-_.](\d{1,2})",                  # 01-15 (无年份)
    ]
    for pattern in patterns:
        match = re.search(pattern, name)
        if match:
            groups = match.groups()
            if len(groups) == 3:
                try:
                    return f"{groups[0]}-{int(groups[1]):02d}-{int(groups[2]):02d}"
                except ValueError:
                    pass
            elif len(groups) == 2:
                year = datetime.now().year
                try:
                    return f"{year}-{int(groups[0]):02d}-{int(groups[1]):02d}"
                except ValueError:
                    pass
    return ""


def extract_amount_from_name(file_path):
    """尝试从文件名中提取金额"""
    name = file_path.stem
    # 匹配金额模式
    patterns = [
        r"(\d+(?:\.\d{1,2})?)\s*元",
        r"¥\s*(\d+(?:\.\d{1,2})?)",
        r"(\d+(?:\.\d{1,2})?)\s*yuan",
    ]
    for pattern in patterns:
        match = re.search(pattern, name, re.IGNORECASE)
        if match:
            return float(match.group(1))
    return None


def organize_files(files, output_folder):
    """将文件按分类复制到输出文件夹"""
    output = Path(output_folder)
    output.mkdir(parents=True, exist_ok=True)

    organized = []

    for f in files:
        category = classify_file(f)
        expense_cat = suggest_expense_category(f)
        date_str = extract_date_from_name(f)
        amount = extract_amount_from_name(f)

        # 创建分类文件夹
        cat_folder = output / category
        cat_folder.mkdir(exist_ok=True)

        # 生成新文件名（保留原名，前缀加日期）
        if date_str:
            new_name = f"{date_str}_{f.name}"
        else:
            new_name = f.name

        dest = cat_folder / new_name

        # 处理重名
        counter = 1
        while dest.exists():
            stem = dest.stem
            dest = cat_folder / f"{stem}_{counter}{dest.suffix}"
            counter += 1

        # 复制文件（不移动原文件，安全第一）
        shutil.copy2(str(f), str(dest))

        organized.append({
            "原文件路径": str(f),
            "新文件路径": str(dest),
            "分类": category,
            "费用科目建议": expense_cat,
            "识别日期": date_str,
            "识别金额": amount,
            "文件大小KB": round(f.stat().st_size / 1024, 1),
            "文件类型": f.suffix.lower(),
        })

    return organized


def generate_ledger(organized, output_folder):
    """生成票据台账 CSV"""
    output = Path(output_folder)
    ledger_path = output / "票据台账_自动生成.csv"

    with open(str(ledger_path), "w", newline="", encoding="utf-8-sig") as csvfile:
        fieldnames = [
            "序号", "日期", "分类", "费用科目建议", "金额(元)",
            "文件名", "文件类型", "文件大小KB", "状态", "备注"
        ]
        writer = csv.DictWriter(csvfile, fieldnames=fieldnames)
        writer.writeheader()

        for i, item in enumerate(organized, 1):
            writer.writerow({
                "序号": i,
                "日期": item["识别日期"] or "待补充",
                "分类": item["分类"],
                "费用科目建议": item["费用科目建议"],
                "金额(元)": item["识别金额"] if item["识别金额"] else "待补充",
                "文件名": Path(item["新文件路径"]).name,
                "文件类型": item["文件类型"],
                "文件大小KB": item["文件大小KB"],
                "状态": "待确认",
                "备注": "",
            })

    return ledger_path


def generate_summary(organized, output_folder):
    """生成整理汇总报告"""
    output = Path(output_folder)
    summary_path = output / "整理汇总报告.md"

    # 统计
    by_category = defaultdict(list)
    for item in organized:
        by_category[item["分类"]].append(item)

    total_files = len(organized)
    total_with_date = sum(1 for item in organized if item["识别日期"])
    total_with_amount = sum(1 for item in organized if item["识别金额"])
    total_amount = sum(item["识别金额"] for item in organized if item["识别金额"])

    lines = [
        "# 票据自动整理汇总报告",
        "",
        f"> 生成时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        "",
        "---",
        "",
        "## 一、总体概况",
        "",
        f"| 指标 | 数值 |",
        f"|------|------|",
        f"| 扫描文件总数 | **{total_files}** 个 |",
        f"| 已识别日期 | {total_with_date} 个（{total_with_date/total_files*100:.0f}%） |" if total_files else "",
        f"| 已识别金额 | {total_with_amount} 个（{total_with_amount/total_files*100:.0f}%） |" if total_files else "",
        f"| 已识别金额合计 | **{total_amount:,.2f} 元** |" if total_amount else "| 已识别金额合计 | 暂无 |",
        "",
        "## 二、分类明细",
        "",
        "| 分类 | 文件数 | 已识别金额(元) |",
        "|------|-------|-------------|",
    ]

    for cat, items in sorted(by_category.items()):
        cat_amount = sum(item["识别金额"] for item in items if item["识别金额"])
        amount_str = f"{cat_amount:,.2f}" if cat_amount else "-"
        lines.append(f"| {cat} | {len(items)} | {amount_str} |")

    lines.extend([
        "",
        "## 三、需要人工处理的事项",
        "",
    ])

    # 未分类的
    unclassified = [item for item in organized if "未分类" in item["分类"] or item["分类"] == "其他"]
    if unclassified:
        lines.append(f"### ⚠️ 未能自动分类的文件（{len(unclassified)} 个）")
        lines.append("")
        for item in unclassified:
            lines.append(f"- `{Path(item['原文件路径']).name}`")
        lines.append("")

    # 缺日期的
    no_date = [item for item in organized if not item["识别日期"]]
    if no_date:
        lines.append(f"### ⚠️ 未识别到日期的文件（{len(no_date)} 个）")
        lines.append("")
        lines.append("请在台账中手动补充日期。")
        lines.append("")

    # 缺金额的
    no_amount = [item for item in organized if not item["识别金额"]]
    if no_amount:
        lines.append(f"### ⚠️ 未识别到金额的文件（{len(no_amount)} 个）")
        lines.append("")
        lines.append("请在台账中手动补充金额。如果是图片票据，建议打开查看后填写。")
        lines.append("")

    lines.extend([
        "## 四、下一步建议",
        "",
        "1. 打开 `票据台账_自动生成.csv`，逐行确认分类和金额是否正确",
        "2. 补充标注为「待补充」的日期和金额",
        "3. 将确认无误的台账导入正式财务系统",
        "4. 如需给捐赠人开具票据，根据台账中的捐赠记录逐一处理",
        "",
        "---",
        "",
        "> 💡 建议使用腾讯文档·智能表格将台账在线化，方便多人协作确认。",
        "> 可通过腾讯技术公益数字工具箱 (techforgood.qq.com) 免费申请。",
    ])

    with open(str(summary_path), "w", encoding="utf-8") as f:
        f.write("\n".join(lines))

    return summary_path


# ============================================================
# 主流程
# ============================================================

def main():
    import argparse

    parser = argparse.ArgumentParser(
        description="票据自动扫描与分类整理工具",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
  python receipt_organizer.py "D:/公益机构/2025年票据"
  python receipt_organizer.py "D:/公益机构/2025年票据" --output "D:/公益机构/整理后"
  python receipt_organizer.py "D:/公益机构/2025年票据" --scan-only
        """
    )
    parser.add_argument("folder", help="要扫描的票据文件夹路径")
    parser.add_argument("--output", "-o", help="整理后的输出文件夹路径（默认: 原文件夹/整理结果_日期）")
    parser.add_argument("--scan-only", action="store_true", help="仅扫描统计，不复制文件")

    args = parser.parse_args()

    folder_path = args.folder
    if not args.output:
        output_folder = os.path.join(
            folder_path,
            f"整理结果_{datetime.now().strftime('%Y%m%d_%H%M%S')}"
        )
    else:
        output_folder = args.output

    print("=" * 60)
    print("📋 票据自动扫描与分类整理工具")
    print("=" * 60)
    print(f"📁 扫描目录: {folder_path}")
    print()

    # 第1步：扫描
    print("🔍 正在扫描文件...")
    files = scan_folder(folder_path)
    print(f"   找到 {len(files)} 个支持的文件")

    if not files:
        print("⚠️ 未找到任何支持的文件（支持: 图片/PDF/Excel）")
        sys.exit(0)

    # 第2步：分类预览
    print()
    print("📊 分类预览:")
    preview = defaultdict(int)
    for f in files:
        cat = classify_file(f)
        preview[cat] += 1
    for cat, count in sorted(preview.items()):
        print(f"   {cat}: {count} 个")

    if args.scan_only:
        print()
        print("✅ 扫描完成（仅扫描模式，未复制文件）")
        return

    # 第3步：整理
    print()
    print(f"📂 正在整理到: {output_folder}")
    organized = organize_files(files, output_folder)
    print(f"   已整理 {len(organized)} 个文件")

    # 第4步：生成台账
    print()
    print("📝 正在生成票据台账...")
    ledger_path = generate_ledger(organized, output_folder)
    print(f"   台账已保存: {ledger_path}")

    # 第5步：生成汇总报告
    print()
    print("📊 正在生成汇总报告...")
    summary_path = generate_summary(organized, output_folder)
    print(f"   报告已保存: {summary_path}")

    # 完成
    print()
    print("=" * 60)
    print("✅ 整理完成！")
    print(f"   📂 整理结果: {output_folder}")
    print(f"   📝 票据台账: {ledger_path}")
    print(f"   📊 汇总报告: {summary_path}")
    print()
    print("⚡ 下一步:")
    print("   1. 打开台账CSV，确认每行的分类和金额")
    print("   2. 补充标注为'待补充'的信息")
    print("   3. 查看汇总报告中的'需要人工处理'部分")
    print("=" * 60)


if __name__ == "__main__":
    main()
