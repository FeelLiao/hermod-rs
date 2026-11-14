# Build, test and release

Hermod now has one GUI stack: Flutter Material 3. Tauri, React and WebView assets are not part of the repository.

## Repository layout

- `core`: cryptography, signaling, QUIC and transfer orchestration — the shared lower layer for CLI and GUI.
- `flutter-bridge/src/windows`: GUI Windows ACL/broker FFI modules, with narrow audited unsafe; no extra platform crate. Core exposes only safe storage-permission injection and shared ACL policy. CLI has its own thin Windows adapter.
- `flutter-bridge/src/app`: persistent settings, jobs, cancellation, logs, storage and private inbox.
- `flutter-bridge/src/gui_api`: stable typed GUI facade; private inbox paths remain internal.
- `flutter-bridge/src/api` and generated `frb_generated.rs`: flutter_rust_bridge (v2) glue compiled to `hermod_flutter_bridge.dll`. Generated glue and explicitly audited bridge/CLI Windows modules contain FFI unsafe; `flutter/scripts/check-unsafe-free.py` rejects handwritten unsafe elsewhere.
- `flutter`: FVM-pinned Flutter Material 3 GUI, Windows runner, Store metadata and packaging scripts.
- `cli`: CLI and integration tests.
- `flutter/scripts`: build, packaging, Store submission, repository version, Release allowlist and native safety checks (one consolidated scripts directory).

## Toolchains

- Rust: workspace `rust-version` in `Cargo.toml`.
- Flutter: FVM version in `.fvmrc` (`3.47.9`). Always use `fvm flutter` / `fvm dart` locally.
- Windows: Visual Studio C++ Build Tools, Windows SDK MakeAppx/MakePRI/SignTool, x64 and ARM64 MSVC components.
- Android: minSdk 34, compile/target SDK 36, NDK 28.2.13676358. Build only in GitHub Actions with `.github/workflows/android-release.yml`; do not build APKs locally. See [Android platform notes](docs/android-platform.md) for signing, SAF and foreground-service contracts; static checks are not device acceptance.
- flutter_rust_bridge: pinned `=2.13.0` on both the Dart and Rust sides. The `flutter_rust_bridge_codegen`
  CLI is a local tool, not stored in the repository: `cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked`.
  Run codegen from `flutter` after changing `flutter-bridge/src/api`: `flutter_rust_bridge_codegen generate`, then `dart format lib/src/rust` and `cargo fmt --all`. Use the pinned SDK; FRB can attempt `fvm install` when FVM is on PATH. Avoid recursively formatting prototype/build directories.

## Local quality gates

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python flutter/scripts/check-unsafe-free.py

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

## Windows AppContainer runtime

The packaged GUI uses `packagedClassicApp` + `appContainer`; no `runFullTrust`,
medium-IL helper, administrator token, firewall modification or loopback exemption.
System file authorization is implemented in Rust via Windows WinRT brokers. The
runner exposes only window/lifecycle interop. Direct EXE launch is not a supported
substitute for package identity/LocalFolder testing and can report storage errors.

Identity/contact files live under the package LocalFolder. The native security
module checks the effective package token and exact package SID; do not reuse the
CLI's normal-user directory or widen ACLs to ALL APPLICATION PACKAGES.

UI status uses bounded latest-value subscriptions with explicit acknowledgements,
not periodic snapshot polling. Cancelling observation does not cancel a transfer.

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

Store upload packages are intentionally unsigned and cannot be installed directly.
Use a separately identified development MSIX for local validation; do not uninstall
or replace the Store application merely to debug. Certificate trust, signing and
installation require explicit authorization. Attach Visual Studio to the installed
package process to retain AppContainer identity and LocalFolder access. A plain
`flutter run -d windows` / direct EXE is not equivalent to packaged validation.
The obsolete direct-run and Store-identity-replacing debug helpers were removed.

Manual diagnostics:

```powershell
Get-AppxPackage FeelLiao.HermodTransfer | Format-List PackageFullName,InstallLocation,Status
Get-AppPackageLog -ActivityID <ActivityId returned by Add-AppxPackage>
Get-WinEvent -LogName Microsoft-Windows-AppModel-Runtime/Admin -MaxEvents 30
explorer.exe shell:AppsFolder\FeelLiao.HermodTransfer_8qnpvd7fj1r9a!Hermod
```

Package data is under `%LOCALAPPDATA%\Packages\FeelLiao.HermodTransfer_8qnpvd7fj1r9a`. Export important inbox files before uninstalling because removal can delete private application data.

The GUI loads the Rust Core through flutter_rust_bridge FFI (`hermod_flutter_bridge.dll` placed next to `HermodTransfer.exe`).
The Windows support directory comes from `ApplicationData.Current.LocalFolder`
through the Rust platform adapter. It no longer falls back to a writable directory
beside the executable or accepts an environment override for the packaged GUI.
Test using an isolated package identity rather than replacing the installed Store
application; uninstall/reset may remove private inbox and identity data.

## Releases

CI checks Rust and Flutter. Release tags build x64/ARM64 static Linux CLI binaries and x64/ARM64 Windows Store packages. GitHub Releases contain only the two Linux CLI programs. MSIX/MSIXBundle artifacts remain internal to Actions and Store submission.

Store submission uses the pinned official Store CLI and these Actions secrets: `AZURE_AD_TENANT_ID`, `AZURE_AD_APPLICATION_CLIENT_ID`, `AZURE_AD_APPLICATION_SECRET`, `SELLER_ID`, and `MS_STORE_APP_ID`. The submission helper validates the bundle before authentication and must never print credentials, complete submission JSON or SAS URLs.
