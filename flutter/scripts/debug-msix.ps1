param(
    [string]$Package = 'dist/windows/hermod-windows-x64.msix',
    [switch]$NoLaunch,
    [switch]$Uninstall,
    [switch]$InstallCertificate
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$identity = 'FeelLiao.HermodTransfer'
$publisher = 'CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C'
$family = 'FeelLiao.HermodTransfer_8qnpvd7fj1r9a'
$appId = 'Hermod'
$friendlyName = 'Hermod local MSIX development'

function Find-WindowsSdkTool([string]$name) {
    $kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory |
        Where-Object { Test-Path "$($_.FullName)/x64/$name" } |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if (-not $kit) { throw "Windows SDK tool is required: $name" }
    "$($kit.FullName)/x64/$name"
}

function Remove-DevelopmentIdentity {
    Get-AppxPackage -Name $identity -ErrorAction SilentlyContinue | Remove-AppxPackage
    Get-ChildItem Cert:\CurrentUser\TrustedPeople,Cert:\CurrentUser\Root,Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
        Where-Object FriendlyName -eq $friendlyName | Remove-Item -Force
}

if ($Uninstall) {
    Remove-DevelopmentIdentity
    Write-Output 'Removed the local Hermod development package and certificate.'
    exit 0
}

$root = (Resolve-Path "$PSScriptRoot/../..").Path
$source = (Resolve-Path $Package).Path
$work = "$root/target/msix-debug"
New-Item -ItemType Directory -Force $work | Out-Null
Remove-DevelopmentIdentity
$signed = "$work/hermod-local-debug.msix"
$cer = "$work/hermod-local-debug.cer"
Copy-Item $source $signed -Force

# 创建一天有效、不可导出私钥的开发证书；只签名副本。
$certificate = New-SelfSignedCertificate `
    -Type Custom -Subject $publisher -FriendlyName $friendlyName `
    -KeyUsage DigitalSignature -KeyAlgorithm RSA -KeyLength 2048 `
    -HashAlgorithm SHA256 -CertStoreLocation Cert:\CurrentUser\My `
    -NotAfter (Get-Date).AddDays(1) `
    -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3')
Export-Certificate -Cert $certificate -FilePath $cer -Force | Out-Null

$signtool = Find-WindowsSdkTool 'signtool.exe'
& $signtool sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $signed
if ($LASTEXITCODE -ne 0) { throw 'SignTool failed' }

if (-not $InstallCertificate) {
    Write-Output "Signed debug copy: $signed"
    Write-Output "Development certificate: $cer"
    Write-Output 'Trust installation requires Windows confirmation. Rerun with -InstallCertificate in an interactive terminal.'
    exit 0
}

# Root trust intentionally invokes the Windows confirmation UI. Never bypass that confirmation.
$trust = Start-Process certutil.exe -ArgumentList @('-user','-addstore','Root',$cer) -Wait -PassThru
if ($trust.ExitCode -ne 0) { throw 'Certificate trust installation was declined or failed' }
Add-AppxPackage -Path $signed -ForceApplicationShutdown
$installed = Get-AppxPackage -Name $identity
Write-Output "Installed: $($installed.PackageFullName)"
Write-Output "Install location: $($installed.InstallLocation)"
Write-Output "Application data: $env:LOCALAPPDATA\Packages\$family"
if (-not $NoLaunch) {
    Start-Process explorer.exe "shell:AppsFolder\$family!$appId"
}
