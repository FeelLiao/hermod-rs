param([string]$Directory = 'dist')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$original = Join-Path (Resolve-Path -LiteralPath $Directory).Path "hermod-windows-$architecture.msix"
$originalHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $original).Hash
$work = Join-Path ([IO.Path]::GetTempPath()) "hermod-store-smoke-$([guid]::NewGuid())"
$certificate = $null
$package = $null
$process = $null
try {
    New-Item -ItemType Directory $work | Out-Null
    $copy = Join-Path $work 'smoke.msix'
    Copy-Item -LiteralPath $original -Destination $copy
    # 仅签名 CI 临时副本以验证部署，绝不修改或签名上传商店的原包。
    $certificate = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C' -CertStoreLocation Cert:\CurrentUser\My -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 -KeyExportPolicy NonExportable -NotAfter (Get-Date).AddDays(1)
    Export-Certificate -Cert $certificate -FilePath "$work/smoke.cer" | Out-Null
    Import-Certificate -FilePath "$work/smoke.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    $kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' -and (Test-Path "$($_.FullName)/x64/signtool.exe") } |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if (-not $kit) { throw 'Windows SDK SignTool is required for the temporary smoke-test copy' }
    & "$($kit.FullName)/x64/signtool.exe" sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $copy
    if ($LASTEXITCODE -ne 0) { throw 'Temporary smoke-test signing failed' }
    Add-AppxPackage -Path $copy
    $package = Get-AppxPackage -Name FeelLiao.HermodTransfer
    if (-not $package -or $package.Status -ne 'Ok' -or $package.PackageFamilyName -ne 'FeelLiao.HermodTransfer_8qnpvd7fj1r9a') { throw 'Store package identity or deployment validation failed' }
    Start-Process -FilePath (Join-Path $package.InstallLocation 'hermod-app.exe') | Out-Null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Seconds 1
        $process = Get-Process -Name hermod-app -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if ($process) { break }
    }
    if (-not $process) { throw 'Installed Hermod did not create a window' }
    Start-Sleep -Seconds 3
    $process.Refresh()
    if ($process.HasExited) { throw 'Installed Hermod exited unexpectedly' }
} finally {
    if ($process -and -not $process.HasExited) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue }
    if ($package) { Remove-AppxPackage -Package $package.PackageFullName -ErrorAction SilentlyContinue }
    if ($certificate) {
        Remove-Item -LiteralPath "Cert:\LocalMachine\TrustedPeople\$($certificate.Thumbprint)" -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -DeleteKey -ErrorAction SilentlyContinue
    }
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $original).Hash -ne $originalHash) { throw 'Smoke test modified the Store upload package' }
& python "$PSScriptRoot/verify-msix.py" $original $architecture
if ($LASTEXITCODE -ne 0) { throw 'Store upload package validation failed after smoke test' }
