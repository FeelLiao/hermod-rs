param(
    [Parameter(Mandatory)][string]$BinaryDirectory,
    [Parameter(Mandatory)][ValidateSet('x64','arm64')][string]$Architecture,
    [string]$OutputDirectory = 'dist'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binary = (Resolve-Path -LiteralPath $BinaryDirectory).Path
if (-not $env:WINDOWS_PFX_PATH -or -not $env:WINDOWS_PFX_PASSWORD) { throw 'Configure fixed WINDOWS_PFX_BASE64 and WINDOWS_PFX_PASSWORD secrets' }
$certificate = New-Object Security.Cryptography.X509Certificates.X509Certificate2($env:WINDOWS_PFX_PATH, $env:WINDOWS_PFX_PASSWORD)
if (-not $certificate.HasPrivateKey -or $certificate.Subject -ne 'CN=HermodTest') { throw 'Expected the fixed HermodTest code signing PFX' }
if ($certificate.NotAfter -lt (Get-Date)) { throw 'The signing certificate has expired' }
$config = Get-Content -Raw "$root/hermod-app/src-tauri/tauri.conf.json" | ConvertFrom-Json
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path "$($_.FullName)/x64/makeappx.exe") } |
    Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw 'Windows SDK MakeAppx and SignTool are required' }
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stage = Join-Path ([IO.Path]::GetTempPath()) "hermod-msix-$([guid]::NewGuid())"
try {
    New-Item -ItemType Directory -Force "$stage/Assets" | Out-Null
    Copy-Item -LiteralPath "$binary/hermod-app.exe" -Destination $stage
    Get-ChildItem -LiteralPath $binary -Filter '*.dll' -File | Copy-Item -Destination $stage
    foreach ($icon in @('StoreLogo.png','Square150x150Logo.png','Square44x44Logo.png')) { Copy-Item -LiteralPath "$root/hermod-app/src-tauri/icons/$icon" -Destination "$stage/Assets" }
    [xml]$manifest = Get-Content -LiteralPath "$root/hermod-app/packaging/AppxManifest.xml" -Raw
    $manifest.Package.Identity.SetAttribute('Name', 'Hermod.Transfer.Test')
    $manifest.Package.Identity.SetAttribute('Publisher', $certificate.Subject)
    $manifest.Package.Identity.SetAttribute('Version', "$($config.version).0")
    $manifest.Package.Identity.SetAttribute('ProcessorArchitecture', $Architecture)
    $manifest.Package.Properties.DisplayName = 'Hermod'
    $manifest.Package.Properties.PublisherDisplayName = 'Hermod'
    $manifest.SelectSingleNode("//*[local-name()='VisualElements']").SetAttribute('DisplayName', 'Hermod')
    $manifest.Save("$stage/AppxManifest.xml")
    $msix = Join-Path $output "hermod-windows-$Architecture.msix"
    & "$($kit.FullName)/x64/makeappx.exe" pack /d $stage /p $msix /o
    if ($LASTEXITCODE -ne 0) { throw 'MakeAppx failed' }
    & "$($kit.FullName)/x64/signtool.exe" sign /fd SHA256 /f $env:WINDOWS_PFX_PATH /p $env:WINDOWS_PFX_PASSWORD $msix
    if ($LASTEXITCODE -ne 0) { throw 'MSIX signing failed' }
    [IO.File]::WriteAllBytes("$output/Hermod-Test.cer", $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
    $installer = (Get-Content "$root/hermod-app/packaging/Install-Hermod.ps1" -Raw).Replace('__CERT_THUMBPRINT__', $certificate.Thumbprint)
    [IO.File]::WriteAllText("$output/Install-Hermod.ps1", $installer, [Text.UTF8Encoding]::new($true))
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$output/Install-Hermod.ps1" -VerifyOnly
    if ($LASTEXITCODE -ne 0) { throw 'MSIX signature verification failed' }
} finally { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue; $certificate.Dispose() }
