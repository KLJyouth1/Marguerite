# 通过 WPS 演示 COM (KWPP.Application) 生成「Marguerite 智能工作台升级总览」PPT
$ErrorActionPreference = 'Stop'
$outDir = 'D:\Downloads\Marguerite\deliverables'
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Force -Path $outDir | Out-Null }
$outPath = Join-Path $outDir 'Marguerite_工作台升级总览.pptx'

$app = $null
$pres = $null
try {
    $app = New-Object -ComObject KWPP.Application
    $pres = $app.Presentations.Add()
    $pres.PageSetup.SlideWidth = 960
    $pres.PageSetup.SlideHeight = 540

    function Add-TitleSlide($title, $subtitle) {
        $s = $pres.Slides.Add($pres.Slides.Count + 1, 1)  # ppLayoutTitle
        $s.Shapes.Title.TextFrame.TextRange.Text = $title
        $s.Shapes.Title.TextFrame.TextRange.Font.Size = 40
        $s.Shapes.Title.TextFrame.TextRange.Font.Bold = $true
        $tb = $s.Shapes.AddTextbox(1, 120, 300, 720, 120)
        $tb.TextFrame.TextRange.Text = $subtitle
        $tb.TextFrame.TextRange.Font.Size = 16
        $tb.TextFrame.TextRange.Font.Color.RGB = 0x666666
    }

    function Add-SectionSlide($title) {
        $s = $pres.Slides.Add($pres.Slides.Count + 1, 1)
        $s.Shapes.Title.TextFrame.TextRange.Text = $title
        $s.Shapes.Title.TextFrame.TextRange.Font.Size = 36
        $s.Shapes.Title.TextFrame.TextRange.Font.Bold = $true
    }

    function Add-ContentSlide($title, [string[]]$bullets) {
        $s = $pres.Slides.Add($pres.Slides.Count + 1, 2)  # ppLayoutText
        $s.Shapes.Title.TextFrame.TextRange.Text = $title
        $s.Shapes.Title.TextFrame.TextRange.Font.Size = 26
        $s.Shapes.Title.TextFrame.TextRange.Font.Bold = $true
        # 正文占位符（标题+文本布局的第 2 个占位符）
        $body = $s.Shapes.Placeholders.Item(2).TextFrame.TextRange
        $body.Text = ($bullets -join "`n")
        $body.Font.Size = 17
        $body.ParagraphFormat.SpaceAfter = 8
    }

    # 1 封面
    Add-TitleSlide 'Marguerite 智能工作台' '升级总览 · 修复 · 研究 · 落地' 
    # 2 目录
    Add-ContentSlide '目录' @(
        '1. 背景与目标',
        '2. 会话持久化：历史不丢失、归属正确',
        '3. Bloom 模式：从标注到真实落地',
        '4. KV Cache 极简工程思维研究',
        '5. 工具链扩展：联网检索 / 隔离代码执行 / 桌面控制',
        '6. 记忆 · 上下文 · 性能 · 架构优化',
        '7. 质量与安全 · 路线图'
    )
    # 3 背景与目标
    Add-ContentSlide '背景与目标' @(
        '目标：让工作台 稳定、可信、能力超越主流工作台',
        '三大痛点：会话历史丢失、Bloom 能力虚假标注、工具能力不足',
        '方法：先联网研究（KV Cache 极简思维）→ 蒸馏 → 落地验证',
        '风格：极简工程，无冗余，可维护'
    )
    # 4 会话持久化
    Add-ContentSlide '会话持久化：历史不丢失、归属正确' @(
        '问题：离开页面历史丢失；关闭应用后会话掉进“最近”栏',
        '根因：上下文台账 JSON 从不记录 project_path，归属信息只有内存+SQLite',
        '修复：台账成为第三个持久归属源，回退链 内存→台账→DB',
        '结果：会话正确归组到项目，重启不丢'
    )
    # 5 Bloom 模式
    Add-ContentSlide 'Bloom 模式：从标注到真实落地' @(
        '核实：Bloom 引擎并非虚假实现——前端 switch → mode.switch → Orchestrator 多代理并行',
        '修复：去掉悬空 #[tokio::test] 造成的虚假/损坏标注',
        '能力：Captain + Copilot 并行编排，栅栏测试证明真并发',
        '目标：能力门控可见、标注真实，杜绝“虚假完成”'
    )
    # 6 KV Cache 研究
    Add-ContentSlide 'KV Cache 极简工程思维研究' @(
        'KV Cache：缓存 K/V 注意力信息，避免重复计算，是推理加速的核心',
        '极简思维：把“重算”变成“复用”，用缓存换延迟',
        '启示：缓存优化、命中率优化、架构优化、输出优化',
        '落地：把同一思维迁移到工作台的上下文与记忆层'
    )
    # 7 工具链：联网检索
    Add-ContentSlide '工具链扩展一：联网检索' @(
        'web_search：DDG 主引擎 + Bing 回退，结构化 标题/链接/摘要',
        'web_research：搜索 → 自动抓取前 N 条正文 → 汇总（search-then-read）',
        'SSRF 防护 + readability 正文提取 + 来源标注',
        '端到端实测通过（真实联网抓取）'
    )
    # 8 工具链：隔离代码执行
    Add-ContentSlide '工具链扩展二：隔离代码执行' @(
        'code_exec：Python / JS 隔离解释器，时空超限执行',
        '无 shell（Command::new 直接跑）→ 免疫 shell 注入',
        '临时目录隔离 + 超时强制终止 + 输出截断',
        '端到端实测通过（真实 python/node 执行与超时终止）'
    )
    # 9 工具链：桌面控制
    Add-ContentSlide '工具链扩展三：桌面控制增强' @(
        'system_control：鼠标拟人化移动 / 点击 / 键盘 / 多屏',
        '新增 monitors：多显示器虚拟桌面坐标枚举',
        '新增 screenshot + OCR：截屏 + Windows.Media.Ocr 文本识别（带包围盒坐标）',
        'OCR 坐标可直接回喂 mouse_move 做精确点击闭环'
    )
    # 10 记忆与上下文
    Add-ContentSlide '记忆与上下文优化' @(
        '上下文台账 + 4 层渐进压缩（折叠→语义摘要→滑动窗口→快照可回滚）',
        '记忆层：memory_save / memory_search / memory_list',
        '知识库与记忆检索优化、画像自动生成',
        'token 计算高精化，压缩率可度量'
    )
    # 11 性能与缓存
    Add-ContentSlide '性能与缓存优化' @(
        'KV Cache 思维迁移：缓存优化、命中率优化、输出优化',
        'MCP 工具缓存、会话运行时台账缓存',
        'DDoS 防护式 SSRF 预检，先检后推',
        '超大文件拆分（ChatView 4801 行）降低维护成本'
    )
    # 12 架构优化
    Add-ContentSlide '架构优化' @(
        '纯 Rust 引擎 + 网关 + 多代理编排（蜂群/子代理/专家团）',
        '事件总线：私总线 → 网关总线 桥回流，会话隔离',
        '幂等 send + 可中止 + 单一收据',
        '模块化：tool → chain → orchestration，职责清晰'
    )
    # 13 质量与安全
    Add-ContentSlide '质量与安全' @(
        '宝塔上传校验统一化：共享预检 + 先检后推 + 回滚备份',
        'SSRF 防护、超时终止、临时目录隔离、输出截断',
        '能力门控（canManageProjectWorkspaces）',
        '测试门禁：联网/解释器/桌面 实时集成测试全绿'
    )
    # 14 路线图
    Add-ContentSlide '路线图 / 下一步' @(
        '蜂群与子代理/专家团进一步优化',
        '知识库与记忆检索往语义化演进',
        '画像自动生成 + token 高精化打磨',
        '持续蒸馏第三方工作台（WorkBuddy/Trae）能力'
    )
    # 15 总结
    Add-ContentSlide '总结与致谢' @(
        '稳定：会话持久化、真实 Bloom 接线',
        '强大：联网检索、隔离代码执行、桌面控制三线补齐',
        '可信：能力标注真实、质量门禁全绿',
        '坚持极简工程——少即是多，精准落地'
    )

    $pres.SaveAs($outPath, 24)  # 24 = OpenXML Presentation (.pptx)
    Write-Output "SAVED: $outPath"
}
catch {
    Write-Output "ERROR: $($_.Exception.Message)"
    if ($_.InvocationInfo) { Write-Output "AT: line $($_.InvocationInfo.ScriptLineNumber)" }
    exit 1
}
finally {
    if ($pres) { try { $pres.Close() } catch {} }
    if ($app)  { try { $app.Quit() } catch {} }
}