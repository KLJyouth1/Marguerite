# 通过 v22 网关发送真实对话任务，监控智能体工具调用
param([string]$TaskMessage, [string]$SessionKey = "task-session", [int]$TimeoutSec = 180)

$ws = New-Object System.Net.WebSockets.ClientWebSocket
$ct = New-Object System.Threading.CancellationTokenSource
$ws.ConnectAsync([Uri]"ws://127.0.0.1:18790/ws", $ct.Token).Wait()

function WsSend($obj) {
    $msg = $obj | ConvertTo-Json -Depth 10 -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($msg)
    $ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct.Token).Wait()
}
function WsRecv() {
    $ms = New-Object System.IO.MemoryStream
    $buf = New-Object byte[] 131072
    do {
        $r = $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $ct.Token).Result
        $ms.Write($buf, 0, $r.Count)
    } while (-not $r.EndOfMessage)
    return [System.Text.Encoding]::UTF8.GetString($ms.ToArray()) | ConvertFrom-Json
}

# Handshake
WsRecv | Out-Null
WsSend @{ type="req"; id=[Guid]::NewGuid().ToString(); method="connect"; params=@{ minProtocol=3; maxProtocol=3; client=@{ name="task-runner" } } }
WsRecv | Out-Null

# Send chat.send
$msgId = [Guid]::NewGuid().ToString()
WsSend @{ type="req"; id=$msgId; method="chat.send"; params=@{ message=$TaskMessage; sessionKey=$SessionKey } }

# Collect events until done or timeout
$toolCalls = @()
$toolResults = @()
$thinkingParts = @()
$finalReply = ""
$errorMsg = ""
$planEvents = @()
$routerDecision = ""
$start = Get-Date
$done = $false

Write-Host "任务: $TaskMessage" -ForegroundColor Cyan
Write-Host "等待智能体执行(超时 ${TimeoutSec}s)..." -ForegroundColor Yellow

while (-not $done -and ((Get-Date) - $start).TotalSeconds -lt $TimeoutSec) {
    $obj = WsRecv
    if ($obj.type -eq "event") {
        $evt = $obj.event
        $payload = $obj.payload
        switch -Wildcard ($evt) {
            "session.event.tool_call" {
                $toolName = $payload.tool
                $args = $payload.args
                $argsStr = if ($args) { ($args -join ",") } else { "" }
                if ($argsStr.Length -gt 150) { $argsStr = $argsStr.Substring(0,150) + "..." }
                Write-Host "  [工具调用] $toolName($argsStr)" -ForegroundColor Green
                $toolCalls += "$toolName"
            }
            "session.event.tool_result" {
                $toolName = $payload.tool
                $ok = $payload.ok
                $summary = $payload.summary
                if ($summary -and $summary.Length -gt 120) { $summary = $summary.Substring(0,120) + "..." }
                $color = if ($ok) { "DarkGray" } else { "Red" }
                Write-Host "  [工具结果] $toolName ok=$ok $summary" -ForegroundColor $color
                $toolResults += "$toolName=$ok"
            }
            "session.event.thinking" {
                $t = $payload.text
                if ($t) { $thinkingParts += $t }
            }
            "session.event.plan_revision" {
                Write-Host "  [计划拆解] 任务已自动拆解为多步执行" -ForegroundColor Magenta
                $planEvents += "revision"
            }
            "session.event.plan_run" {
                $phase = $payload.phase
                $step = $payload.step
                Write-Host "  [计划执行] phase=$phase step=$step" -ForegroundColor Magenta
                $planEvents += "run:$phase"
            }
            "session.event.router_decision" {
                $routerDecision = "$($payload | ConvertTo-Json -Compress -Depth 3)"
                Write-Host "  [路由决策] $routerDecision" -ForegroundColor Cyan
            }
            "session.event.done" {
                $finalReply = $payload.text_snapshot
                if (-not $finalReply) { $finalReply = $payload.text }
                $reason = $payload.reason
                $done = $true
                Write-Host "  [完成] reason=$reason" -ForegroundColor Green
            }
            "session.event.error" {
                $errorMsg = "$($payload.message)"
                if (-not $errorMsg) { $errorMsg = "$($payload | ConvertTo-Json -Compress)" }
                $done = $true
                Write-Host "  [错误] $errorMsg" -ForegroundColor Red
            }
            default {
                # skip other events (status, ensemble_progress, compaction, etc.)
            }
        }
    }
}

$ws.Dispose()

$elapsed = [math]::Round(((Get-Date) - $start).TotalSeconds, 1)
Write-Host "`n===== 任务结果 =====" -ForegroundColor Cyan
Write-Host "耗时: ${elapsed}s"
Write-Host "工具调用次数: $($toolCalls.Count)"
Write-Host "调用的工具: $($toolCalls -join ', ')"
Write-Host "工具结果: $($toolResults -join '; ')"
if ($planEvents.Count -gt 0) { Write-Host "计划事件: $($planEvents -join ', ')" }
if ($routerDecision) { Write-Host "路由决策: $routerDecision" }
if ($errorMsg) { Write-Host "错误: $errorMsg" -ForegroundColor Red }
Write-Host "`n--- 思考过程(前500字) ---" -ForegroundColor DarkYellow
$think = $thinkingParts -join ""
if ($think.Length -gt 500) { $think.Substring(0,500) + "..." } else { $think }
Write-Host "`n--- 智能体回复(前3000字) ---" -ForegroundColor Yellow
$reply = if ($finalReply) { $finalReply } else { "(无回复)" }
if ($reply.Length -gt 3000) { $reply.Substring(0,3000) + "...`n[回复共 $($reply.Length) 字]" } else { $reply }
Write-Host ""
