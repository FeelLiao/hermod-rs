# Build, test and release

Hermod now has one GUI stack: Flutter Material 3. Tauri, React and WebView assets are not part of the repository.

## Repository layout

- `core`: cryptography, signaling, QUIC/WireGuard and transfer orchestration — the shared lower layer for CLI and GUI.
- `flutter-bridge/src/app`: persistent settings, jobs, cancellation, logs, storage and private inbox.
- `flutter-bridge/src/gui_api`: stable typed GUI facade; private inbox paths remain internal.
- `flutter-bridge/src/api` and generated `frb_generated.rs`: flutter_rust_bridge (v2) glue compiled to `hermod_flutter_bridge.dll`. Only generated code contains unsafe; `scripts/check-unsafe-free.py` fails the build for any handwritten unsafe.
- `flutter`: FVM-pinned Flutter Material 3 GUI, Windows runner, Store metadata and packaging scripts.
- `cli`: CLI and integration tests.
- `scripts`: repository version and Release allowlist checks.

## Toolchains

- Rust: workspace `rust-version` in `Cargo.toml`.
- Flutter: FVM version in `.fvmrc` (`3.44.8`). Always use `fvm flutter` / `fvm dart` locally.
- Windows: Visual Studio C++ Build Tools, Windows SDK MakeAppx/MakePRI/SignTool, x64 and ARM64 MSVC components.
- Android runner: minSdk 34. It is retained for later integration and is not currently a release artifact.
- flutter_rust_bridge: pinned `=2.13.0` on both the Dart and Rust sides. The `flutter_rust_bridge_codegen`
  CLI is a local tool, not stored in the repository: `cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked`.
  The code generator detects FVM automatically; run it from `flutter` after changing `flutter-bridge/src/api`: `flutter_rust_bridge_codegen generate`.

## Local quality gates

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python scripts/check-unsafe-free.py

Push-Location flutter
fvm dart format --output=none --set-exit-if-changed lib test
fvm flutter analyze
fvm flutter test
Pop-Location
```

Build the Windows GUI and Rust bridge:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\flutter\scripts\build-windows.ps1 `
  -Architecture x64 -Mode debug
```

Release build:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\flutter\scripts\build-windows.ps1 `
  -Architecture x64 -Mode release
```

## Microsoft Store identity

The checked-in manifest uses the reserved Partner Center identity:

| Property | Value |
| --- | --- |
| Display name | `HermodTransfer` |
| Identity name | `FeelLiao.HermodTransfer` |
| Publisher | `CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C` |
| Publisher display name | `FeelLiao` |
| Package family name | `FeelLiao.HermodTransfer_8qnpvd7fj1r9a` |
| Store ID | `9PMPWN3HMR3M` |
| Languages | `en-US`, `zh-CN` |

Do not change this identity for an update. Store versions map app `major.minor.patch` to `(major + 1).minor.patch.0`; app `0.4.5` becomes MSIX `1.4.5.0`.

Build one unsigned Store MSIX:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\flutter\scripts\package-msix.ps1 `
  -Architecture x64 -OutputDirectory dist\windows
```

ARM64 uses `-Architecture arm64`. Combine the two packages:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\flutter\scripts\bundle-msix.ps1 `
  -PackageDirectory dist\windows -OutputDirectory dist\windows
```

Partner Center receives the unsigned `hermod-windows.msixbundle` and signs it after certification. Never place signing certificates or private keys in an artifact.

## Local MSIX debugging

Store upload packages are intentionally unsigned and Windows will not install them directly. The helper creates a one-day, non-exportable development certificate with the exact manifest publisher, trusts it for the current user, signs a copy (never the Store upload original), installs it, launches the AppUserModelId and optionally tails AppModel event logs:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\flutter\scripts\debug-msix.ps1 `
  -Package .\dist\windows\hermod-windows-x64.msix `
  -InstallCertificate
```

Useful options:

```powershell
# Reinstall without launching
.\flutter\scripts\debug-msix.ps1 -Package .\dist\windows\hermod-windows-x64.msix -InstallCertificate -NoLaunch

# Remove the installed development package and certificate
.\flutter\scripts\debug-msix.ps1 -Uninstall
```

The script copies the package to `target/msix-debug/`, signs only that copy, installs for the current user and reports paths. Direct executable debugging remains faster: build debug, open `flutter/windows/runner.sln`, or launch `HermodTransfer.exe` from `build/windows/x64/runner/Debug` and attach Visual Studio. Use MSIX debugging specifically for package identity, capabilities, application data location, PRI/localization and Store-style launch behavior.

Manual diagnostics:

```powershell
Get-AppxPackage FeelLiao.HermodTransfer | Format-List PackageFullName,InstallLocation,Status
Get-AppPackageLog -ActivityID <ActivityId returned by Add-AppxPackage>
Get-WinEvent -LogName Microsoft-Windows-AppModel-Runtime/Admin -MaxEvents 30
explorer.exe shell:AppsFolder\FeelLiao.HermodTransfer_8qnpvd7fj1r9a!Hermod
```

Package data is under `%LOCALAPPDATA%\Packages\FeelLiao.HermodTransfer_8qnpvd7fj1r9a`. Export important inbox files before uninstalling because removal can delete private application data.

The GUI loads the Rust Core through flutter_rust_bridge FFI (`hermod_flutter_bridge.dll` placed next to `HermodTransfer.exe`).
Inside a Windows Store package the application support directory resolves through
package identity; for terminal debugging the loader falls back to
`app-support` next to the executable, and `HERMOD_APP_SUPPORT` can point the engine
to a custom directory. Uninstalling clears `app-support`, so export the private inbox
before uninstalling a terminal-run build.

## Releases

CI checks Rust and Flutter. Release tags build x64/ARM64 static Linux CLI binaries and x64/ARM64 Windows Store packages. GitHub Releases contain only the two Linux CLI programs. MSIX/MSIXBundle artifacts remain internal to Actions and Store submission.

Store submission uses the pinned official Store CLI and these Actions secrets: `AZURE_AD_TENANT_ID`, `AZURE_AD_APPLICATION_CLIENT_ID`, `AZURE_AD_APPLICATION_SECRET`, `SELLER_ID`, and `MS_STORE_APP_ID`. The submission helper validates the bundle before authentication and must never print credentials, complete submission JSON or SAS URLs.
