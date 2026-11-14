param(
    [ValidateSet('x64','arm64')][string]$Architecture = 'x64',
    [ValidateSet('debug','release')][string]$Mode = 'release'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$flutter = "$root/flutter"
$rustTarget = if ($Architecture -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'aarch64-pc-windows-msvc' }
if ($Architecture -eq 'arm64' -and $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    $rustTarget = 'aarch64-pc-windows-msvc'
}
$profile = if ($Mode -eq 'release') { '--release' } else { '' }
$temp = "$root/target/msvc-temp-$Architecture"
New-Item -ItemType Directory -Force $temp | Out-Null
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = if (Test-Path $vswhere) { & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath } else { 'D:\Programs\MSVC\BuildTools' }
if (-not $vs) { throw 'Visual Studio C++ Build Tools are required' }
$vcvars = if ($Architecture -eq 'x64') { 'vcvars64.bat' } else { 'vcvarsamd64_arm64.bat' }
$command = "call `"$vs\VC\Auxiliary\Build\$vcvars`" >nul && set TMP=$temp&&set TEMP=$temp&&cargo build -p hermod-flutter-bridge --target $rustTarget --locked $profile"
cmd /d /s /c $command
if ($LASTEXITCODE -ne 0) { throw 'Rust Core bridge build failed' }
$rustProfile = if ($Mode -eq 'release') { 'release' } else { 'debug' }
Copy-Item "$root/target/$rustTarget/$rustProfile/hermod_flutter_bridge.dll" "$flutter/windows/hermod_flutter_bridge.dll" -Force
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
$flutterTemp = "$root/target/flutter-temp-$Architecture"
New-Item -ItemType Directory -Force $flutterTemp | Out-Null
$env:TEMP = $flutterTemp
$env:TMP = $flutterTemp
Push-Location $flutter
try {
    $modeFlag = "--$Mode"
    # Capture the conditional output as an array even when only flutter is used.
    $flutterCommand = @(if (Get-Command fvm -ErrorAction SilentlyContinue) { 'fvm'; 'flutter' } else { 'flutter' })
    $arguments = @('build','windows',$modeFlag)
    # Flutter 3.47 selects the Windows target from the native Dart process ABI.
    # There is no --target-platform option for this command.
    $useFvm = $flutterCommand.Count -eq 2
    $toolAbi = if ($useFvm) { & fvm dart "$PSScriptRoot/tool_arch.dart" } else { & dart "$PSScriptRoot/tool_arch.dart" }
    if ($LASTEXITCODE -ne 0) { throw 'Could not determine Flutter Dart tool ABI' }
    $expectedAbi = if ($Architecture -eq 'arm64') { 'windows_arm64' } else { 'windows_x64' }
    if (($toolAbi -join "`n").Trim() -ne $expectedAbi) {
        throw "Flutter tool ABI must be $expectedAbi, found $($toolAbi -join ' '). Use a native SDK matching the package architecture."
    }
    if ($flutterCommand.Count -eq 2) { & $flutterCommand[0] $flutterCommand[1] @arguments } else { & $flutterCommand[0] @arguments }
    if ($LASTEXITCODE -ne 0) { throw 'Flutter Windows build failed' }
} finally {
    Pop-Location
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
}
