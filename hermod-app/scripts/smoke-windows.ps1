param([string]$Directory = 'dist')
$ErrorActionPreference = 'Stop'
$directoryPath = (Resolve-Path -LiteralPath $Directory).Path
$certificate = Import-Certificate -FilePath "$directoryPath/Hermod-Test.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPeople
try {
    # 单独的原生 PowerShell 进程以与用户相同的流程验证包完整性和安装。
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$directoryPath/Install-Hermod.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'MSIX installation smoke test failed' }
    $package = Get-AppxPackage -Name Hermod.Transfer.Test
    if (-not $package -or $package.Status -ne 'Ok') { throw 'MSIX deployment validation failed' }
    Start-Process -FilePath (Join-Path $package.InstallLocation "hermod-app.exe") | Out-Null
    $process = $null
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Seconds 1
        $process = Get-Process -Name hermod-app -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if ($process) { break }
    }
    if (-not $process) { throw 'Installed Hermod did not create a window' }
    Start-Sleep -Seconds 3
    if ($process.HasExited) { throw 'Installed Hermod exited unexpectedly' }
    Stop-Process -Id $process.Id
    Remove-AppxPackage -Package $package.PackageFullName
} finally { Remove-Item -LiteralPath "Cert:\LocalMachine\TrustedPeople\$($certificate.Thumbprint)" -ErrorAction SilentlyContinue }
