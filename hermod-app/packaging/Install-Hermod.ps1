#Requires -Version 5.1
param([switch]$TrustCertificateOnly, [switch]$VerifyOnly)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# 打包脚本将固定证书指纹写入此处，两种架构共用同一份安装脚本。
$ExpectedThumbprint = '__CERT_THUMBPRINT__'
$Identity = 'Hermod.Transfer.Test'
$Publisher = 'CN=HermodTest'
$certificatePath = Join-Path $PSScriptRoot 'Hermod-Test.cer'
$certificate = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($certificatePath)
if ($certificate.Thumbprint -ne $ExpectedThumbprint -or $certificate.Subject -ne $Publisher -or $certificate.HasPrivateKey) { throw 'Unexpected public signing certificate' }
if ((Get-Date) -lt $certificate.NotBefore -or (Get-Date) -gt $certificate.NotAfter) { throw 'The signing certificate is not currently valid' }
$trustPath = "Cert:\LocalMachine\TrustedPeople\$ExpectedThumbprint"
if ($TrustCertificateOnly) {
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) { throw 'Certificate import requires administrator permission' }
    Import-Certificate -FilePath $certificatePath -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    exit 0
}
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($arch -notin @('x64', 'arm64')) { throw "Unsupported Windows architecture: $arch" }
$package = Join-Path $PSScriptRoot "hermod-windows-$arch.msix"
if (-not (Test-Path -LiteralPath $package -PathType Leaf)) { throw "Download hermod-windows-$arch.msix into the same folder as this script and Hermod-Test.cer" }
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.Security
$zip = [IO.Compression.ZipFile]::OpenRead($package)
try {
    $entry = $zip.GetEntry('AppxSignature.p7x')
    if (-not $entry) { throw 'MSIX has no signature' }
    $stream = $entry.Open(); $buffer = New-Object IO.MemoryStream
    try { $stream.CopyTo($buffer); $bytes = $buffer.ToArray() } finally { $stream.Dispose(); $buffer.Dispose() }
    if ([Text.Encoding]::ASCII.GetString($bytes, 0, 4) -ne 'PKCX') { throw 'Invalid MSIX signature header' }
    $cms = New-Object Security.Cryptography.Pkcs.SignedCms
    $cms.Decode([byte[]]$bytes[4..($bytes.Length - 1)])
    # 验证 CMS 的密码学签名；完整包完整性和 TrustedPeople 信任由 Add-AppxPackage 再验证。
    $cms.CheckSignature($true)
    if ($cms.SignerInfos.Count -ne 1 -or $cms.SignerInfos[0].Certificate.Thumbprint -ne $ExpectedThumbprint) { throw 'MSIX signer does not match the supplied certificate' }
    $reader = New-Object IO.StreamReader($zip.GetEntry('AppxManifest.xml').Open())
    try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
    if ($manifest.Package.Identity.Name -ne $Identity -or $manifest.Package.Identity.Publisher -ne $Publisher -or $manifest.Package.Identity.ProcessorArchitecture -ne $arch) { throw 'MSIX identity or architecture does not match Hermod' }
} finally { $zip.Dispose() }
if ($VerifyOnly) { Write-Output 'MSIX signature and identity verified'; exit 0 }
$webview = @(
    'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
) | Where-Object { $version = Get-ItemPropertyValue -LiteralPath $_ -Name pv -ErrorAction SilentlyContinue; $version -and $version -ne '0.0.0.0' }
if (-not $webview) {
    Write-Output 'Install the Microsoft Edge WebView2 Evergreen Runtime, then run this script again.'
    Start-Process 'https://developer.microsoft.com/microsoft-edge/webview2/#download-section'
    exit 1
}
if (-not (Test-Path -LiteralPath $trustPath)) {
    Write-Output 'Administrator permission is needed only to trust the supplied public certificate.'
    $process = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -Verb RunAs -Wait -PassThru -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-TrustCertificateOnly')
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $trustPath)) { throw 'Certificate trust was not installed' }
}
# 返回原用户上下文：Windows 部署服务验证整个包，绝不使用 AllowUnsigned。
Add-AppxPackage -Path $package
Write-Output 'Hermod installed. Open Hermod from the Start menu.'
