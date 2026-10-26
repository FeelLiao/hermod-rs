param(
    [Parameter(Mandatory)][string]$PackageDirectory,
    [string]$OutputDirectory = 'dist'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$packages = (Resolve-Path -LiteralPath $PackageDirectory).Path
$expected = @('hermod-windows-x64.msix', 'hermod-windows-arm64.msix')
$entries = @(Get-ChildItem -LiteralPath $packages -Force)
if ($entries.Count -ne 2 -or @($entries | Where-Object { $_.PSIsContainer -or $_.Name -cnotin $expected }).Count -ne 0) {
    throw 'Bundle input must contain exactly the tested x64 and ARM64 MSIX packages'
}
foreach ($architecture in @('x64', 'arm64')) {
    & python "$PSScriptRoot/verify-msix.py" "$packages/hermod-windows-$architecture.msix" $architecture
    if ($LASTEXITCODE -ne 0) { throw "Invalid $architecture Store package" }
}
$config = Get-Content -Raw "$root/hermod-app/src-tauri/tauri.conf.json" | ConvertFrom-Json
$version = & node "$PSScriptRoot/msix-version.mjs" $config.version
if ($LASTEXITCODE -ne 0) { throw 'Invalid Microsoft Store bundle version' }
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path "$($_.FullName)/x64/makeappx.exe") } |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw 'Windows SDK MakeAppx is required' }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$bundle = Join-Path (Resolve-Path -LiteralPath $OutputDirectory).Path 'hermod-windows.msixbundle'
# 显式版本避免 MakeAppx 以当前时间生成包版本；合并时仍启用语义验证。
& "$($kit.FullName)/x64/makeappx.exe" bundle /d $packages /p $bundle /bv $version /o
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx bundle failed' }
& python "$PSScriptRoot/verify-msix.py" $bundle --bundle
if ($LASTEXITCODE -ne 0) { throw 'Microsoft Store MSIXBundle validation failed' }
