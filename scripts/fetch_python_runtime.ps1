# ============================================================================
# 把可移植 CPython 3.12 + pyengine 运行期依赖落到 gui/src-tauri/resources/python
#
# 为什么需要：安装包要让一台没有 Python 的新机器也能跑 pyengine 侧车。
# `gui/src-tauri/resources/` 在 .gitignore 里（体量大），所以这里提供一个幂等的
# 物化脚本；打包前跑一次即可，产物会被 Tauri 的 resources 一起打进安装包。
#
# 依赖：uv（https://docs.astral.sh/uv/）—— 用它拿到 python-build-standalone 版
#       CPython（可整目录搬走，不写注册表）。
# ============================================================================
param(
    [string]$PythonVersion = "3.12"
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $repo "gui\src-tauri\resources\python"
$reqs = Join-Path $PSScriptRoot "pyengine-runtime-requirements.txt"
$py = Join-Path $dest "python.exe"

function Test-Ready {
    if (-not (Test-Path $py)) { return $false }
    & $py -c "import starlette, pydantic, sqlmodel, uvicorn" 2>$null | Out-Null
    return $LASTEXITCODE -eq 0
}

if (Test-Ready) {
    Write-Host "[python] 已就绪：$py"
    & $py -c "import sys; print(sys.version)"
    exit 0
}

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    throw "需要 uv 来获取可移植 CPython：https://docs.astral.sh/uv/"
}

Write-Host "[python] 准备 CPython $PythonVersion ..."
& uv python install $PythonVersion
if ($LASTEXITCODE -ne 0) { throw "uv python install 失败（$LASTEXITCODE）" }

$found = (& uv python find $PythonVersion).Trim()
if (-not (Test-Path $found)) { throw "uv python find 返回了不存在的路径：$found" }
$baseDir = Split-Path -Parent $found

Write-Host "[python] 复制 $baseDir -> $dest"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
& robocopy $baseDir $dest /E /NFL /NDL /NJH /NJS /NP | Out-Null
# robocopy 0-7 都是成功；>=8 才是失败
if ($LASTEXITCODE -ge 8) { throw "robocopy 失败（$LASTEXITCODE）" }

Write-Host "[python] 安装 pyengine 运行期依赖 ..."
& uv pip install --python $py --system --break-system-packages -r $reqs
if ($LASTEXITCODE -ne 0) { throw "uv pip install 失败（$LASTEXITCODE）" }

& $py -c "import starlette, pydantic, sqlmodel, uvicorn, sqlalchemy, httpx, reportlab, lark_oapi; print('[python] imports OK')"
if ($LASTEXITCODE -ne 0) { throw "内置运行时自检失败" }

$sizeMb = [math]::Round((Get-ChildItem $dest -Recurse -File | Measure-Object Length -Sum).Sum / 1MB, 1)
Write-Host "[python] 完成：$dest（$sizeMb MB）"
