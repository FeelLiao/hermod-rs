param(
    [Parameter(Mandatory)][ValidateSet('x64','arm64')][string]$Architecture,
    [string]$OutputDirectory = 'dist/windows'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$flutter = "$root/flutter"
& "$PSScriptRoot/build-windows.ps1" -Architecture $Architecture -Mode release
$build = "$flutter/build/windows/$Architecture/runner/Release"
if (-not (Test-Path $build)) { throw "Flutter output not found: $build" }
$version = (Get-Content "$root/Cargo.toml" | Select-String '^version = "([0-9]+)\.([0-9]+)\.([0-9]+)"' | Select-Object -First 1).Matches[0].Groups
$storeVersion = "$([int]$version[1].Value + 1).$($version[2].Value).$($version[3].Value).0"
$kit = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Directory | Where-Object { (Test-Path "$($_.FullName)/x64/makeappx.exe") -and (Test-Path "$($_.FullName)/x64/makepri.exe") } | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $kit) { throw 'Windows SDK MakeAppx and MakePRI are required' }
$output = New-Item -ItemType Directory -Force $OutputDirectory
$work = "$root/target/msix-$Architecture"
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
$stage = New-Item -ItemType Directory -Force "$work/payload"
Copy-Item "$build/*" $stage -Recurse
Copy-Item "$flutter/windows/store/Assets" $stage -Recurse
Copy-Item "$flutter/windows/store/Strings" $stage -Recurse
[xml]$manifest = Get-Content "$flutter/windows/store/AppxManifest.xml" -Raw
$manifest.Package.Identity.SetAttribute('Version', $storeVersion)
$manifest.Package.Identity.SetAttribute('ProcessorArchitecture', $Architecture)
$manifest.Save("$stage/AppxManifest.xml")
& "$($kit.FullName)/x64/makepri.exe" createconfig /cf "$work/priconfig.xml" /dq en-US /o
if ($LASTEXITCODE -ne 0) { throw 'MakePRI configuration failed' }
[xml]$pri = Get-Content "$work/priconfig.xml" -Raw
foreach ($node in @($pri.SelectNodes('//packaging'))) { $node.ParentNode.RemoveChild($node) | Out-Null }
$pri.Save("$work/priconfig.xml")
& "$($kit.FullName)/x64/makepri.exe" new /pr $stage /cf "$work/priconfig.xml" /mn "$stage/AppxManifest.xml" /of "$stage/resources.pri" /o
if ($LASTEXITCODE -ne 0) { throw 'MakePRI failed' }
$msix = Join-Path $output.FullName "hermod-windows-$Architecture.msix"
& "$($kit.FullName)/x64/makeappx.exe" pack /d $stage /p $msix /l /o
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx failed' }
$python = Get-Command python -ErrorAction SilentlyContinue
if ($python) {
    & $python.Source --version *> $null
}
if ($python -and $LASTEXITCODE -eq 0) {
    & $python.Source "$PSScriptRoot/verify-msix.py" $msix $Architecture
    if ($LASTEXITCODE -ne 0) { throw 'Microsoft Store MSIX validation failed' }
} else {
    Write-Warning 'Python was not found; CI will run the mandatory Store package verifier.'
}
Write-Output $msix
