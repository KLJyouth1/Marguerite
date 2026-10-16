# verify_first_run.ps1 - Marguerite (M4 分发闭环) 安装后「启动 -> 网关 -> 首任务」全链路验收脚本
# 设计： PowerShell 5.1（Windows 原生）自包含，不依赖 Node/Python。
# 用法（干净机器，管理员）：
#   powershell -ExecutionPolicy Bypass -File verify_first_run.ps1 -Installer "D:\dist\Marguerite_0.2.0_x64_en-US.msi"
#   powershell -ExecutionPolicy Bypass -File verify_first_run.ps1 -Installer "...\setup.exe"        # NSIS 静默
#   powershell -ExecutionPolicy Bypass -File verify_first_run.ps1 -SkipInstall -Exe "...\marguerite-gui.exe"   # 已安装仅验证
# 退出码：0=全部硬项 PASS；2=有硬项 FAIL；1=首任务发起可达但执行未完成（如缺 LLM 凭据，advisory）。
param(
    [string]$Installer = "",
    [string]$Exe = "",
    [switch]$SkipInstall,
    # 端口单一事实源：core/src/gateway/mod.rs 的 DEFAULT_PORT（编译期常量不变量）。
    [int]$Port = 18790,
    [int]$ProbeTimeoutSec = 60,
    [switch]$SkipFirstTask
)
$ErrorActionPreference = "Stop"
$script:HARD_FAIL = $false
$script:ADVISORY = @()
$script:GO = 0

function Log($tag, $msg) { Write-Host ("[{0}] {1}" -f $tag, $msg) }
function Pass($msg)  { Log "PASS" $msg }
function Fail($msg)  { Log "FAIL" $msg; $script:HARD_FAIL = $true }
function Adv($msg)   { Log "ADV " $msg; $script:ADVISORY += $msg }

# ---------- 0. 参数归一：定位待验证 exe ----------
if (-not $SkipInstall) {
    if (-not (Test-Path $Installer)) { Write-Host "[FAIL] 安装器不存在: $Installer"; exit 2 }
}

$gatewayExe = $Exe
if (-not $gatewayExe -and -not $SkipInstall) {
    $cands = @(
        "$env:ProgramFiles\Marguerite\marguerite-gui.exe",
        "${env:ProgramFiles(x86)}\Marguerite\marguerite-gui.exe"
    )
    $gatewayExe = $cands | Where-Object { Test-Path $_ } | Select-Object -First 1
}

# ---------- 1. 安装（可选） ----------
$msiInstall = $false
if (-not $SkipInstall) {
    $ext = [System.IO.Path]::GetExtension($Installer).ToLower()
    if ($ext -eq ".msi") {
        Log "STEP" "静默安装 MSI: $Installer"
        $p = Start-Process msiexec.exe -ArgumentList @("/i", "`"$Installer`"", "/qn", "/norestart") -Wait -PassThru
        if ($p.ExitCode -ne 0) { Fail "msiexec 退出码 $($p.ExitCode)" } else { Pass "MSI 安装完成" }
        $msiInstall = $true
    } elseif ($ext -eq ".exe") {
        Log "STEP" "静默安装 NSIS: $Installer"
        $p = Start-Process $Installer -ArgumentList @("/S") -Wait -PassThru
        if ($p.ExitCode -ne 0) { Fail "NSIS 退出码 $($p.ExitCode)" } else { Pass "NSIS 安装完成" }
    } else {
        Fail "不支持的安装器类型: $ext"
    }
    if (-not $gatewayExe) {
        $cands = @("$env:ProgramFiles\Marguerite\marguerite-gui.exe", "${env:ProgramFiles(x86)}\Marguerite\marguerite-gui.exe")
        $gatewayExe = $cands | Where-Object { Test-Path $_ } | Select-Object -First 1
    }
}

if (-not $gatewayExe -or -not (Test-Path $gatewayExe)) {
    Fail "未找到 marguerite-gui.exe（安装目录或 -Exe 参数）"
    Write-Host "RESULT=FAIL"; exit 2
}
Pass "已定位可执行: $gatewayExe"
$appDataExe = Split-Path (Split-Path $gatewayExe)

# ---------- 2. 启动 ----------
Log "STEP" "启动 Marguerite"
$proc = Start-Process $gatewayExe -PassThru
Start-Sleep -Seconds 5
if ($proc.HasExited) { Fail "进程启动后立即退出 rc=$($proc.ExitCode)" } else { Pass "进程运行中 pid=$($proc.Id)" }

# ---------- 3. 网关 health 探活 ----------
Log "STEP" "网关 health 探活"
$healthOk = $false
for ($i = 1; $i -le 15; $i++) {
    try {
        $r = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/health" -TimeoutSec 3
        if ($r.status -eq "ok" -and $r.engine -eq "rust") { $healthOk = $true; Pass ("health=ok engine=" + $r.engine + " uptime_ms=" + $r.uptime_ms); break }
        else { Adv ("health 响应异常: " + ($r | ConvertTo-Json -Compress)) }
    } catch { Start-Sleep -Seconds 2 }
}
if (-not $healthOk) { Fail "网关 /api/health 未就绪" }

# ---------- 4. 控制台页面 ----------
Log "STEP" "控制台页面"
try {
    $c = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/control/" -TimeoutSec 5 -UseBasicParsing
    if ($c.StatusCode -eq 200 -and $c.Content.Length -gt 500) { Pass ("/control/ HTTP 200 len=" + $c.Content.Length) }
    else { Fail ("/control/ 异常 status=" + $c.StatusCode) }
} catch { Fail ("/control/ 不可达: " + $_.Exception.Message) }

# ---------- 5. 首任务：WebSocket connect + chat.send ----------
if (-not $SkipFirstTask) {
    Log "STEP" "首任务: WS 握手 + chat.send"
    $token = $null
    $tokenFile = Join-Path $env:LOCALAPPDATA "Marguerite\gateway-token"
    if (Test-Path $tokenFile) { $token = (Get-Content $tokenFile | Select-Object -First 1).Trim() }
    if (-not $token) { Adv "gateway-token 未生成（应用数据目录为空？）" }

    try {
        $ws = New-Object System.Net.WebSockets.ClientWebSocket
        $ct = New-Object System.Threading.CancellationToken
        $uri = [System.Uri]("ws://127.0.0.1:$Port/ws")
        $ws.ConnectAsync($uri, $ct).GetAwaiter().GetResult()
        Pass "WebSocket 已连接"

        $send = @{ type = "req"; id = "c"; method = "connect"; params = @{ auth = @{ token = $token }; protocol = @{ min = 3; max = 3 } } } | ConvertTo-Json -Depth 6
        $buf = [System.Text.Encoding]::UTF8.GetBytes($send)
        $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$buf)
        $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).GetAwaiter().GetResult()

        $deadline = (Get-Date).AddSeconds($ProbeTimeoutSec)
        $taskDone = $false
        while ((Get-Date) -lt $deadline) {
            $recvBuf = New-Object byte[] 65536
            $rseg = New-Object System.ArraySegment[byte] -ArgumentList @(,$recvBuf)
            $task = $ws.ReceiveAsync($rseg, $ct)
            if (-not $task.Wait(2000)) { continue }
            $res = $task.Result
            $text = [System.Text.Encoding]::UTF8.GetString($recvBuf, 0, $res.Count)
            try { $m = $text | ConvertFrom-Json } catch { continue }
            if ($m.type -eq "res" -and $m.id -eq "c") {
                if ($m.error) { Fail ("connect 失败: " + ($m.error | ConvertTo-Json -Compress -Depth 3)) }
                else {
                    Pass "connect 成功（协议握手 OK）"
                    # 发送首个任务
                    $send2 = @{ type = "req"; id = "t"; method = "chat.send"; params = @{ message = "你好，请用一句话回复。"; sessionKey = "m4.first.task" } } | ConvertTo-Json -Depth 6
                    $buf2 = [System.Text.Encoding]::UTF8.GetBytes($send2)
                    $seg2 = New-Object System.ArraySegment[byte] -ArgumentList @(,$buf2)
                    $ws.SendAsync($seg2, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).GetAwaiter().GetResult()
                }
            }
            if ($m.type -eq "event") {
                if ($m.event -eq "session.event.done") {
                    $len = [string]$m.payload.text_snapshot.Length
                    Pass ("首任务完成: text_snapshot len=" + $len)
                    $taskDone = $true; break
                }
                if ($m.event -eq "session.event.error") {
                    $err = ($m.payload | ConvertTo-Json -Compress -Depth 3)
                    Adv ("首任务 error 事件（发起已到达，执行受下游阻塞）: " + $err)
                    $taskDone = $true; break
                }
                if ($m.event -eq "session.event.started" -or $m.event -eq "session.queued") { Pass ("事件: " + $m.event) }
            }
        }
        if (-not $taskDone) { Adv "首任务超时未返回（${ProbeTimeoutSec}s）" }
        $ws.Dispose()
    } catch {
        Adv ("WebSocket 探测异常: " + $_.Exception.Message)
    }
}

# ---------- 6. 清理 ----------
if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 2 }
$still = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
if ($still) { Adv "端口 $Port 仍被监听 pid=$($still.OwningProcess)" } else { Pass "进程已清理，端口 $Port 已释放" }

# ---------- 结果 ----------
Write-Host "----------------------------------------"
$adv = $script:ADVISORY.Count
if ($script:HARD_FAIL) {
    Write-Host "RESULT=FAIL 硬项: 存在 FAIL；advisory: $adv"
    exit 2
} elseif ($adv -gt 0) {
    Write-Host "RESULT=ADVISORY 硬项全过，但 $adv 项需人工关注"
    exit 1
} else {
    Write-Host "RESULT=PASS 全链路硬项全部通过"
    exit 0
}
