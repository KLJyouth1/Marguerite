param(
    [string]$Configuration = "Release",
    [string]$Payload,
    [string]$OutputRoot
)

$ErrorActionPreference = "Stop"

$projectRoot = $PSScriptRoot
$repoRoot = Split-Path -Parent $projectRoot

$setupProject = Join-Path $projectRoot "Marguerite.Setup\Marguerite.Setup.csproj"
$downloaderProject = Join-Path $projectRoot "Marguerite.Downloader\Marguerite.Downloader.csproj"
$setupExe = Join-Path $projectRoot "Marguerite.Setup\bin\$Configuration\net48\MargueriteSetup.exe"
$downloaderExe = Join-Path $projectRoot "Marguerite.Downloader\bin\$Configuration\net48\MargueriteDownloader.exe"

if (-not $OutputRoot) {
    $OutputRoot = Join-Path $repoRoot "deliverables"
}

if (-not $Payload) {
    $bundleRoot = Join-Path $repoRoot "marguerite_build_target\release\bundle\nsis"
    $candidates = @()
    if (Test-Path $bundleRoot) {
        $candidates = Get-ChildItem -Path $bundleRoot -Filter "Marguerite_*_x64-setup.exe" -File |
            ForEach-Object {
                $match = [regex]::Match($_.Name, '^Marguerite_(\d+\.\d+\.\d+)_x64-setup\.exe$')
                if ($match.Success) {
                    [pscustomobject]@{ Path = $_.FullName; Version = [version]$match.Groups[1].Value }
                }
            }
    }

    if (-not $candidates -or $candidates.Count -eq 0) {
        throw "No NSIS payload found under $bundleRoot. Pass -Payload explicitly."
    }

    $Payload = ($candidates | Sort-Object Version -Descending | Select-Object -First 1).Path
}

$payloadItem = Get-Item $Payload
$versionMatch = [regex]::Match($payloadItem.Name, '^Marguerite_(\d+\.\d+\.\d+)_x64-setup\.exe$')
if (-not $versionMatch.Success) {
    throw "Payload name must look like Marguerite_<x.y.z>_x64-setup.exe, got $($payloadItem.Name)"
}
$version = $versionMatch.Groups[1].Value

Write-Host "[BUILD] setup + downloader ($Configuration)"
& dotnet build $setupProject -c $Configuration
if ($LASTEXITCODE -ne 0) { throw "setup build failed ($LASTEXITCODE)" }
& dotnet build $downloaderProject -c $Configuration
if ($LASTEXITCODE -ne 0) { throw "downloader build failed ($LASTEXITCODE)" }

foreach ($exe in @($setupExe, $downloaderExe)) {
    if (-not (Test-Path $exe)) { throw "missing build output: $exe" }
}

$outputDir = Join-Path $OutputRoot "Marguerite-Setup-$version"
New-Item -ItemType Directory -Force $outputDir | Out-Null

$setupTarget = Join-Path $outputDir "MargueriteSetup.exe"
[System.IO.File]::Copy($setupExe, $setupTarget, $true)

$downloaderTarget = Join-Path $outputDir "MargueriteDownloader.exe"
[System.IO.File]::Copy($downloaderExe, $downloaderTarget, $true)

$payloadTarget = Join-Path $outputDir $payloadItem.Name
if (Test-Path $payloadTarget) {
    [System.IO.File]::Delete($payloadTarget)
}

$linked = $false
try {
    New-Item -ItemType HardLink -Path $payloadTarget -Target $payloadItem.FullName | Out-Null
    $linked = $true
}
catch {
    [System.IO.File]::Copy($payloadItem.FullName, $payloadTarget, $true)
}

$lines = @()
foreach ($file in @($setupTarget, $downloaderTarget, $payloadTarget)) {
    $lines += (Get-FileHash $file -Algorithm SHA256).Hash.ToLowerInvariant() + "  " + (Split-Path $file -Leaf)
}
$sums = Join-Path $outputDir "SHA256SUMS.txt"
$lines | Set-Content -Path $sums -Encoding ASCII

Write-Host ""
Write-Host "===== Marguerite cinematic setup + downloader ====="
Write-Host "OUTPUT        = $outputDir"
Write-Host "SETUP         = $setupTarget ($((Get-Item $setupTarget).Length) bytes)"
Write-Host "DOWNLOADER    = $downloaderTarget ($((Get-Item $downloaderTarget).Length) bytes)"
Write-Host "PAYLOAD       = $payloadTarget ($($payloadItem.Length) bytes, hardlink=$linked)"
Write-Host "SUMS          = $sums"
Write-Host ""
Write-Host "Standalone downloader run:"
Write-Host "  $downloaderTarget"
Write-Host "Installer only run:"
Write-Host "  $setupTarget"
