param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][ValidateSet('x64','arm64')][string]$Architecture,
    [string]$OutputDirectory = 'dist'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binary = (Resolve-Path -LiteralPath $BinaryDirectory).Path
$config = Get-Content -Raw "$root/hermod-app/src-tauri/tauri.conf.json" | ConvertFrom-Json
$version = & node "$PSScriptRoot/msix-version.mjs" $config.version
if ($LASTEXITCODE -ne 0) { throw 'Invalid Microsoft Store package version' }
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path "$($_.FullName)/x64/makeappx.exe") -and (Test-Path "$($_.FullName)/x64/makepri.exe") } |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw 'Windows SDK MakeAppx and MakePRI are required' }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$work = Join-Path ([IO.Path]::GetTempPath()) "hermod-msix-$([guid]::NewGuid())"
$stage = Join-Path $work 'payload'
try {
    New-Item -ItemType Directory -Force "$stage/Assets" | Out-Null
    Copy-Item -LiteralPath "$binary/hermod-app.exe" -Destination $stage
    Get-ChildItem -LiteralPath $binary -Filter '*.dll' -File | Copy-Item -Destination $stage
    foreach ($icon in @('StoreLogo.png','Square150x150Logo.png','Square44x44Logo.png')) { Copy-Item -LiteralPath "$root/hermod-app/src-tauri/icons/$icon" -Destination "$stage/Assets" }
    Copy-Item -LiteralPath "$root/hermod-app/packaging/Strings" -Destination $stage -Recurse
    [xml]$manifest = Get-Content -LiteralPath "$root/hermod-app/packaging/AppxManifest.xml" -Raw
    $manifest.Package.Identity.SetAttribute('Version', $version)
    $manifest.Package.Identity.SetAttribute('ProcessorArchitecture', $Architecture)
    $manifest.Save("$stage/AppxManifest.xml")
    & "$($kit.FullName)/x64/makepri.exe" createconfig /cf "$work/priconfig.xml" /dq en-US /o
    if ($LASTEXITCODE -ne 0) { throw 'MakePRI configuration failed' }
    # 两种语言都放在同一包中，禁止 MakePRI 自动拆分成资源包。
    [xml]$pri = Get-Content -LiteralPath "$work/priconfig.xml" -Raw
    foreach ($node in @($pri.SelectNodes('//packaging'))) { $node.ParentNode.RemoveChild($node) | Out-Null }
    $pri.Save("$work/priconfig.xml")
    & "$($kit.FullName)/x64/makepri.exe" new /pr $stage /cf "$work/priconfig.xml" /mn "$stage/AppxManifest.xml" /of "$stage/resources.pri" /o
    if ($LASTEXITCODE -ne 0) { throw 'MakePRI resource compilation failed' }
    $msix = Join-Path $output "hermod-windows-$Architecture.msix"
    & "$($kit.FullName)/x64/makeappx.exe" pack /d $stage /p $msix /l /o
    if ($LASTEXITCODE -ne 0) { throw 'MakeAppx failed' }
    & python "$PSScriptRoot/verify-msix.py" $msix $Architecture
    if ($LASTEXITCODE -ne 0) { throw 'Microsoft Store MSIX validation failed' }
} finally { Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue }
