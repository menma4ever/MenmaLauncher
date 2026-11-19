# Publish the Menma pack: refresh hashes, rebuild the friends' Prism zip, commit and push.
# Usage:  powershell -ExecutionPolicy Bypass -File scripts\publish.ps1 "what changed"
param([string]$Message = "Update pack")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$packwiz = Join-Path $root "tools\packwiz\packwiz.exe"

Push-Location (Join-Path $root "pack")
try {
    & $packwiz refresh
    if ($LASTEXITCODE -ne 0) { throw "packwiz refresh failed" }
} finally { Pop-Location }

# Friends import this zip into Prism once; after that every launch syncs automatically.
# Rebuilt only when the instance template changes, so the zip doesn't churn on every publish.
$zip = Join-Path $root "dist\Menma-Prism.zip"
$src = Join-Path $root "dist\friend-instance"
$newest = (Get-ChildItem $src -Recurse -File | Measure-Object LastWriteTime -Maximum).Maximum
if (-not (Test-Path $zip) -or (Get-Item $zip).LastWriteTime -lt $newest) {
    if (Test-Path $zip) { Remove-Item $zip }
    # Windows tar writes forward-slash zip paths, which Prism reads reliably.
    & "$env:SystemRoot\System32\tar.exe" -a -c -f $zip -C $src instance.cfg mmc-pack.json minecraft
    if ($LASTEXITCODE -ne 0) { throw "zip build failed" }
}

Push-Location $root
try {
    git add -A
    git diff --cached --quiet
    if ($LASTEXITCODE -eq 0) { Write-Host "Nothing changed."; exit 0 }
    git commit -m $Message
    git push
} finally { Pop-Location }
Write-Host "Published. Friends get it on their next launch."
