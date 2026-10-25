param([string]$BinaryDirectory = "target/release", [string]$OutputDirectory = "dist")
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$version = (Get-Content hermod-app/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "MSIX requires a stable major.minor.patch version" }
$stageRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$stage = Join-Path $stageRoot "hermod-msix-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force "$stage/Assets", $OutputDirectory | Out-Null
Copy-Item "$BinaryDirectory/hermod-app.exe" $stage
Get-ChildItem "$BinaryDirectory/*.dll" -ErrorAction SilentlyContinue | Copy-Item -Destination $stage
Copy-Item "hermod-app/src-tauri/icons/StoreLogo.png", "hermod-app/src-tauri/icons/Square150x150Logo.png", "hermod-app/src-tauri/icons/Square44x44Logo.png" "$stage/Assets"
(Get-Content packaging/windows/AppxManifest.xml -Raw).Replace('__VERSION__', "$version.0") | Set-Content "$stage/AppxManifest.xml" -Encoding utf8
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory | Where-Object { Test-Path "$($_.FullName)/x64/makeappx.exe" } | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw "Windows SDK with MakeAppx and SignTool is required" }
$msix = Join-Path (Resolve-Path $OutputDirectory) "hermod-windows-x86_64-test-signed.msix"
& "$($kit.FullName)/x64/makeappx.exe" pack /d $stage /p $msix /o
if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed" }
# 每次生成临时测试证书，只发布公钥；私钥在 finally 中清理。
$cert = New-SelfSignedCertificate -Type Custom -Subject "CN=HermodTest" -KeyUsage DigitalSignature -FriendlyName "Hermod testing only" -CertStoreLocation "Cert:\CurrentUser\My" -KeyExportPolicy Exportable -NotAfter (Get-Date).AddYears(1) -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
try {
    Export-Certificate -Cert $cert -FilePath "$OutputDirectory/hermod-windows-test-certificate.cer" | Out-Null
    & "$($kit.FullName)/x64/signtool.exe" sign /fd SHA256 /sha1 $cert.Thumbprint /s My $msix
    if ($LASTEXITCODE -ne 0) { throw "MSIX signing failed" }
    # 签名完整性由 SignTool 检查；自签证书需要临时加入本机构建信任库。
    Import-Certificate -FilePath "$OutputDirectory/hermod-windows-test-certificate.cer" -CertStoreLocation Cert:\CurrentUser\TrustedPeople | Out-Null
    & "$($kit.FullName)/x64/signtool.exe" verify /pa /v $msix
    if ($LASTEXITCODE -ne 0) { throw "MSIX signature verification failed" }
} finally {
    Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
    Remove-Item "Cert:\CurrentUser\TrustedPeople\$($cert.Thumbprint)" -ErrorAction SilentlyContinue
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
Copy-Item packaging/windows/INSTALL.txt "$OutputDirectory/hermod-windows-INSTALL.txt"
