param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$BinaryDirectory,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$IdentityName,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Publisher,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$PublisherDisplayName,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$DisplayName,
    [Parameter(Mandatory)][ValidatePattern('^[1-9][0-9]*\.[0-9]+\.[0-9]+\.0$')][string]$PackageVersion,
    [string]$OutputDirectory = "store-output"
)
$ErrorActionPreference = "Stop"
foreach ($part in $PackageVersion.Split('.')) {
    if ([long]$part -gt 65535) { throw "Each MSIX version component must be between 0 and 65535" }
}
$binary = (Resolve-Path -LiteralPath $BinaryDirectory).Path
if (-not (Test-Path -LiteralPath "$binary/hermod-app.exe" -PathType Leaf)) {
    throw "BinaryDirectory must contain the extracted Windows hermod-app.exe"
}
# 同时支持仓库脚本和发布 ZIP 内的 Store 目录；不依赖调用者当前目录。
$template = Join-Path $PSScriptRoot 'AppxManifest.xml'
$icons = Join-Path $PSScriptRoot 'Assets'
if (-not (Test-Path -LiteralPath $template)) {
    $root = Split-Path $PSScriptRoot -Parent
    $template = Join-Path $root 'packaging/windows/AppxManifest.xml'
    $icons = Join-Path $root 'hermod-app/src-tauri/icons'
}
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path "$($_.FullName)/x64/makeappx.exe") } |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw "Install the Windows SDK with the x64 MakeAppx tool" }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stage = Join-Path ([IO.Path]::GetTempPath()) "hermod-msix-$([guid]::NewGuid())"
try {
    New-Item -ItemType Directory -Force "$stage/Assets" | Out-Null
    Copy-Item -LiteralPath "$binary/hermod-app.exe" -Destination $stage
    Get-ChildItem -LiteralPath $binary -Filter '*.dll' -File | Copy-Item -Destination $stage
    foreach ($icon in @('StoreLogo.png', 'Square150x150Logo.png', 'Square44x44Logo.png')) {
        Copy-Item -LiteralPath "$icons/$icon" -Destination "$stage/Assets"
    }
    # 使用 XML DOM 写入身份，正确转义发布者名称中的 & 等字符。
    [xml]$manifest = Get-Content -LiteralPath $template -Raw
    $manifest.Package.Identity.SetAttribute('Name', $IdentityName)
    $manifest.Package.Identity.SetAttribute('Publisher', $Publisher)
    $manifest.Package.Identity.SetAttribute('Version', $PackageVersion)
    $manifest.Package.Properties.DisplayName = $DisplayName
    $manifest.Package.Properties.PublisherDisplayName = $PublisherDisplayName
    $visuals = $manifest.SelectSingleNode("//*[local-name()='VisualElements']")
    $visuals.SetAttribute('DisplayName', $DisplayName)
    $manifest.Save("$stage/AppxManifest.xml")
    $msix = Join-Path $output 'hermod-windows-x86_64-store-unsigned.msix'
    & "$($kit.FullName)/x64/makeappx.exe" pack /d $stage /p $msix /o
    if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed" }
    Write-Output "Unsigned Microsoft Store submission package: $msix"
    Write-Output "Upload the MSIX to Partner Center. It is not a signed sideload installer."
} finally {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
