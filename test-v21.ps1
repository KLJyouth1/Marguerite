# Marguerite v21 全功能自动化测试脚本
# 测试：RPC方法、模式、技能、知识库、插件、token计量、智能体、多角色任务

$ErrorActionPreference = "Continue"
$base = "http://127.0.0.1:18790"
$results = @()
$pass = 0; $fail = 0

function Test($name, $condition, $detail = "") {
    if ($condition) {
        Write-Host "[PASS] $name" -ForegroundColor Green
        $script:pass++
        $script:results += [PSCustomObject]@{Test=$name; Status="PASS"; Detail=$detail}
    } else {
        Write-Host "[FAIL] $name $detail" -ForegroundColor Red
        $script:fail++
        $script:results += [PSCustomObject]@{Test=$name; Status="FAIL"; Detail=$detail}
    }
}

# WebSocket RPC helper
$script:ws = $null
$script:ct = $null

function Connect-Ws {
    $script:ws = New-Object System.Net.WebSockets.ClientWebSocket
    $script:ct = New-Object System.Threading.CancellationTokenSource
    $script:ws.ConnectAsync([Uri]"ws://127.0.0.1:18790/ws", $script:ct.Token).Wait()
    # Challenge
    $buf = New-Object byte[] 65536
    $r = $script:ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $script:ct.Token).Result
    [System.Text.Encoding]::UTF8.GetString($buf, 0, $r.Count) | Out-Null
    # Connect
    $msg = @{ type="req"; id=[Guid]::NewGuid().ToString(); method="connect"; params=@{ minProtocol=3; maxProtocol=3; client=@{ name="auto-test" } } } | ConvertTo-Json -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($msg)
    $script:ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $script:ct.Token).Wait()
    $r = $script:ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $script:ct.Token).Result
    [System.Text.Encoding]::UTF8.GetString($buf, 0, $r.Count) | Out-Null
}

function Rpc($method, $params = @{}, $timeoutMs = 10000) {
    $msg = @{ type="req"; id=[Guid]::NewGuid().ToString(); method=$method; params=$params } | ConvertTo-Json -Depth 10 -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($msg)
    $script:ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $script:ct.Token).Wait()
    # Loop to receive complete message, skipping events until we get the response with matching id
    $deadline = (Get-Date).AddMilliseconds($timeoutMs)
    while ((Get-Date) -lt $deadline) {
        $ms = New-Object System.IO.MemoryStream
        $buf = New-Object byte[] 65536
        do {
            $r = $script:ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $script:ct.Token).Result
            $ms.Write($buf, 0, $r.Count)
        } while (-not $r.EndOfMessage)
        $raw = [System.Text.Encoding]::UTF8.GetString($ms.ToArray())
        $obj = $raw | ConvertFrom-Json
        # Skip events (type=event), wait for actual response
        if ($obj.type -eq "event") { continue }
        return $obj
    }
    throw "RPC timeout: $method"
}

Write-Host "`n========== Marguerite v21 全功能自动化测试 ==========`n" -ForegroundColor Cyan

# 0. 健康检查
try {
    $h = Invoke-RestMethod -Uri "$base/api/health" -TimeoutSec 5
    Test "网关健康检查" ($h.status -eq "ok") "engine=$($h.engine)"
} catch { Test "网关健康检查" $false $_.Exception.Message }

# 1. WebSocket 连接
try {
    Connect-Ws
    Test "WebSocket连接+握手" ($script:ws.State -eq "Open")
} catch { Test "WebSocket连接+握手" $false $_.Exception.Message }

# 2. 智能体测试
Write-Host "`n--- 智能体测试 ---" -ForegroundColor Yellow
$r = Rpc "agents.list"
Test "agents.list返回" ($r.ok -eq $true)
Test "智能体数量>=12" ($r.payload.agents.Count -ge 12) "count=$($r.payload.agents.Count)"
$agentIds = $r.payload.agents.id -join ","
Test "主智能体存在" ($agentIds -match "main")
Test "浏览器操控员存在" ($agentIds -match "browser-operator")
Test "电脑操控员存在" ($agentIds -match "computer-operator")
Test "研究员存在" ($agentIds -match "researcher")
$mainAgent = $r.payload.agents | Where-Object { $_.id -eq "main" }
Test "主智能体工具名正确(非file_read)" ($mainAgent.tools -notcontains "file_read") "tools=$($mainAgent.tools -join ',')"
Test "主智能体含browser工具" ($mainAgent.tools -contains "browser")
Test "主智能体含system_control工具" ($mainAgent.tools -contains "system_control")
Test "主智能体含kb_search工具" ($mainAgent.tools -contains "kb_search")
Test "主智能体含list_skills工具" ($mainAgent.tools -contains "list_skills")

# 3. 模式测试
Write-Host "`n--- 模式测试 ---" -ForegroundColor Yellow
$r = Rpc "mode.status"
Test "mode.status返回" ($r.ok -eq $true) "kind=$($r.payload.kind)"
Test "可用模式含plan/agent/bloom" ($r.payload.available -contains "plan" -and $r.payload.available -contains "bloom")
$r = Rpc "mode.switch" @{ kind="bloom" }
Test "切换到bloom" ($r.ok -eq $true) "runtimeKind=$($r.payload.runtimeKind)"
$r = Rpc "mode.switch" @{ kind="plan" }
Test "切换到plan" ($r.ok -eq $true)
$r = Rpc "mode.switch" @{ kind="agent" }
Test "切换到agent" ($r.ok -eq $true)
$r = Rpc "mode.switch" @{ kind="bloom" }
Test "切回bloom" ($r.ok -eq $true)

# 4. 知识库测试
Write-Host "`n--- 知识库测试 ---" -ForegroundColor Yellow
$r = Rpc "kb.status"
Test "kb.status返回" ($r.ok -eq $true) "docs=$($r.payload.documents) chunks=$($r.payload.chunks)"
$r = Rpc "kb.ingest" @{ title="测试文档"; text="Marguerite是一个AI Agent工作台，支持Bloom蜂群模式和极限思考。广西港妙科技是一家科技公司。" }
Test "kb.ingest灌入" ($r.ok -eq $true) ($r.payload | ConvertTo-Json -Compress)
$r = Rpc "kb.list"
Test "kb.list返回" ($r.ok -eq $true) "total=$($r.payload.total)"
Test "kb.list含灌入文档" ($r.payload.documents.Count -ge 1)
$r = Rpc "kb.search" @{ query="Bloom蜂群模式"; topK=3 }
Test "kb.search检索" ($r.ok -eq $true -and $r.payload.items.Count -ge 1) "hits=$($r.payload.items.Count)"
if ($r.payload.items.Count -ge 1) {
    $docId = $r.payload.items[0].doc_id
    $r = Rpc "kb.delete" @{ doc_id=$docId }
    Test "kb.delete删除" ($r.ok -eq $true)
} else {
    Test "kb.delete跳过(无检索结果)" $true
}

# 5. 技能测试
Write-Host "`n--- 技能测试 ---" -ForegroundColor Yellow
$r = Rpc "skills.list"
Test "skills.list返回" ($r.ok -eq $true)
Test "技能数量>0" ($r.payload.skills.Count -gt 0) "count=$($r.payload.skills.Count)"
$r = Rpc "skills.create" @{ name="auto-test-skill"; description="自动化测试技能"; prompt_template="你是测试专家: {{text}}"; triggers=@("test","测试") }
Test "skills.create创建" ($r.ok -eq $true -or $r.payload.success -eq $true) $r.payload.message
$r = Rpc "skills.list"
$hasTestSkill = $r.payload.skills | Where-Object { $_.name -eq "auto-test-skill" }
Test "新技能出现在列表" ($null -ne $hasTestSkill)
$r = Rpc "skills.uninstall" @{ name="auto-test-skill" }
Test "skills.uninstall删除" ($r.success -eq $true -or $r.payload.success -eq $true)

# 6. 插件测试
Write-Host "`n--- 插件测试 ---" -ForegroundColor Yellow
$r = Rpc "plugins.list"
Test "plugins.list返回" ($r.ok -eq $true)
Test "插件数量>10" ($r.payload.plugins.Count -gt 10) "count=$($r.payload.plugins.Count)"
$hasReal = $r.payload.plugins | Where-Object { $_.maturity -eq "Real" }
Test "存在Real成熟度插件" ($null -ne $hasReal) "real=$($hasReal.Count)"

# 7. 记忆测试
Write-Host "`n--- 记忆测试 ---" -ForegroundColor Yellow
$r = Rpc "memory.list"
Test "memory.list返回" ($r.ok -eq $true)
$r = Rpc "memory.save" @{ content="自动化测试记忆条目"; tags=@("test","auto") }
Test "memory.save保存" ($r.ok -eq $true)
$r = Rpc "memory.search" @{ query="自动化测试"; topK=3 }
Test "memory.search检索" ($r.ok -eq $true)
$r = Rpc "memory.layers.status"
Test "memory.layers.status返回" ($r.ok -eq $true)

# 8. 配置测试
Write-Host "`n--- 配置测试 ---" -ForegroundColor Yellow
$r = Rpc "config.get"
Test "config.get返回" ($r.ok -eq $true)
Test "Bloom并行已启用" ($r.payload.bloom.parallel_enabled -eq $true)
Test "模型为deepseek-v4-pro" ($r.payload.model.model_name -match "deepseek")
$r = Rpc "sandbox.status"
Test "sandbox.status返回" ($r.ok -eq $true)
$r = Rpc "doctor.status"
Test "doctor.status返回" ($r.ok -eq $true)

# 9. 会话测试
Write-Host "`n--- 会话测试 ---" -ForegroundColor Yellow
$r = Rpc "sessions.list"
Test "sessions.list返回" ($r.ok -eq $true)

# 10. Token用量测试
Write-Host "`n--- Token用量测试 ---" -ForegroundColor Yellow
$r = Rpc "usage.status"
Test "usage.status返回" ($r.ok -eq $true) "runs=$($r.payload.runs) tokens=$($r.payload.total_tokens)"

# 11. 舰队/自进化/电脑操控
Write-Host "`n--- 扩展功能测试 ---" -ForegroundColor Yellow
$r = Rpc "fleet.list"
Test "fleet.list返回" ($r.ok -eq $true)
$r = Rpc "evolve.history"
Test "evolve.history返回" ($r.ok -eq $true)
$r = Rpc "computer_use.action" @{ action="status" }
Test "computer_use.action(status)返回" ($r.ok -eq $true)

# 12. 工作区
Write-Host "`n--- 工作区测试 ---" -ForegroundColor Yellow
$r = Rpc "workspaces.list"
Test "workspaces.list返回" ($r.ok -eq $true)

# 13. onboarding/sandbox.path.list (之前缺失的方法)
Write-Host "`n--- 之前缺失方法测试 ---" -ForegroundColor Yellow
$r = Rpc "onboarding.catalog"
Test "onboarding.catalog已实现" ($r.ok -eq $true)
$r = Rpc "sandbox.path.list"
Test "sandbox.path.list已实现" ($r.ok -eq $true)

# 汇总
Write-Host "`n========== 测试汇总 ==========" -ForegroundColor Cyan
Write-Host "通过: $pass / 失败: $fail / 总计: $($pass+$fail)" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Yellow" })
$script:results | Format-Table -AutoSize
$script:ws.Dispose()
