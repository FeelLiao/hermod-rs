param(
    [string]$PackageDirectory = 'dist/windows',
    [string]$OutputDirectory = 'dist/windows'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$packages = (Resolve-Path $PackageDirectory).Path
$expected = @('hermod-windows-x64.msix','hermod-windows-arm64.msix')
$entries = @(Get-ChildItem $packages -File -Filter '*.msix')
if ($entries.Count -ne 2 -or @($entries | Where-Object Name -cnotin $expected).Count -ne 0) { throw 'Bundle requires exactly the tested x64 and ARM64 packages' }
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$version = (Get-Content "$root/Cargo.toml" | Select-String '^version = "([0-9]+)\.([0-9]+)\.([0-9]+)"' | Select-Object -First 1).Matches[0].Groups
$storeVersion = "$([int]$version[1].Value + 1).$($version[2].Value).$($version[3].Value).0"
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory | Where-Object { Test-Path "$($_.FullName)/x64/makeappx.exe" } | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw 'Windows SDK MakeAppx is required' }
$output = New-Item -ItemType Directory -Force $OutputDirectory
$bundle = Join-Path $output.FullName 'hermod-windows.msixbundle'
& "$($kit.FullName)/x64/makeappx.exe" bundle /d $packages /p $bundle /bv $storeVersion /o
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx bundle failed' }
Write-Output $bundle
