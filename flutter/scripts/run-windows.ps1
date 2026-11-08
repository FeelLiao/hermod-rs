param(
    [ValidateSet('x64','arm64')][string]$Architecture = 'x64'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$flutter = "$root/flutter"
$temp = "$root/target/flutter-run-temp-$Architecture"
New-Item -ItemType Directory -Force $temp | Out-Null

$previousTemp = $env:TEMP
$previousTmp = $env:TMP
$previousSuppressAnalytics = $env:FLUTTER_SUPPRESS_ANALYTICS
$env:TEMP = $temp
$env:TMP = $temp
$env:FLUTTER_SUPPRESS_ANALYTICS = 'true'
Push-Location $flutter
try {
    $arguments = @('run','-d','windows')
    if ($Architecture -eq 'arm64') {
        $arguments += @('--target-platform','windows-arm64')
    }
    & fvm flutter @arguments
    exit $LASTEXITCODE
} finally {
    Pop-Location
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
    $env:FLUTTER_SUPPRESS_ANALYTICS = $previousSuppressAnalytics
}
