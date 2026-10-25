param(
    [string]$BinaryDirectory = "target/release",
    [string]$OutputDirectory = "dist"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$binary = (Resolve-Path -LiteralPath $BinaryDirectory).Path
if (-not (Test-Path -LiteralPath "$binary/hermod-app.exe" -PathType Leaf)) {
    throw "Build hermod-app.exe before packaging the Windows ZIP"
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stage = Join-Path ([IO.Path]::GetTempPath()) "hermod-zip-$([guid]::NewGuid())"
try {
    New-Item -ItemType Directory -Force "$stage/Store/Assets" | Out-Null
    Copy-Item -LiteralPath "$binary/hermod-app.exe" -Destination $stage
    Get-ChildItem -LiteralPath $binary -Filter '*.dll' -File | Copy-Item -Destination $stage
    Copy-Item -LiteralPath "$root/packaging/windows/INSTALL.txt" -Destination "$stage/README.txt"
    Copy-Item -LiteralPath "$root/LICENSE" -Destination $stage
    # 商店工具随 ZIP 分发，用户无需重新编译，也无需签名证书。
    Copy-Item -LiteralPath "$root/scripts/package-msix.ps1", "$root/packaging/windows/AppxManifest.xml" -Destination "$stage/Store"
    foreach ($icon in @('StoreLogo.png', 'Square150x150Logo.png', 'Square44x44Logo.png')) {
        Copy-Item -LiteralPath "$root/hermod-app/src-tauri/icons/$icon" -Destination "$stage/Store/Assets"
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = Join-Path $output 'hermod-windows-x86_64.zip'
    # 使用临时归档，压缩失败时保留已有的发布文件。
    [IO.Compression.ZipFile]::CreateFromDirectory($stage, "$stage.zip")
    Move-Item -LiteralPath "$stage.zip" -Destination $archive -Force
    Write-Output "Portable Windows ZIP: $archive"
} finally {
    Remove-Item -LiteralPath $stage, "$stage.zip" -Recurse -Force -ErrorAction SilentlyContinue
}
