# Building and releasing Hermod 0.4.0

[User guide](README.md) · [中文使用说明](README.zh-CN.md)

Release builds run in `.github/workflows/build-release.yml`. Local validation is
Linux-only; Windows and Android installation/runtime validation belongs to their
Actions jobs. A successful Linux build is not evidence that those jobs have passed.

## Repository layout

- `hermod-core`: cryptography, IPv6 signaling, WireGuard/QUIC, HTTP transfer, settings validation.
- `hermod-cli`: clap shell and terminal I/O.
- `hermod-app/src`: shared React pages and a small official Fluent UI / Material Web component adapter.
- `hermod-app/src-tauri/src/app`: persistent settings, jobs, cancellation, bounded streams, logs and verified inbox.
- `hermod-app/src-tauri/src`: desktop and Android Tauri commands; background job monitoring.
- `hermod-app/plugins/documents`: Android SAF, permissions, system color and foreground notification adapters. No native UI screens.
- `hermod-app/android`: pinned Android Gradle template; generated Tauri shell stays ignored under `src-tauri/gen/android`.
- `hermod-app/android-tests`: emulator tests and a debug-only document provider.
- `hermod-app/packaging`: flat MSIX manifest and public `.ps1` installer template; no `windows` subdirectory.
- `hermod-app/scripts`: GUI packaging, signing initialization and platform smoke checks.
- `scripts`: repository-wide version and Release allowlist checks. CI helpers are not Release attachments.

## Linux prerequisites and development

Use the Rust version required by the workspace (currently 1.99+), Node.js 22,
C/C++ build tools and GTK/WebKit dependencies:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config cmake perl clang musl-tools \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libfuse2 xvfb xdotool
rustup target add x86_64-unknown-linux-musl
npm ci --prefix hermod-app
npm run build --prefix hermod-app
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cd hermod-app
npm run tauri -- dev
```

End-to-end tests need a non-loopback local IPv6 address. On an isolated WSL host,
run the tests in a temporary network namespace (no host network changes):

```bash
sudo unshare --net bash -c '
  ip link set lo up
  ip link add hermodtest type dummy
  ip link set hermodtest up
  ip -6 addr add fd00:1234::1/64 dev hermodtest nodad
  cargo test --workspace --locked
'
```

The shell must have access to the installed Rust toolchain. Tests use a local signaling
fixture and do not need the public server. CLI/GUI interop tests run real CLI processes
against the shared GUI engine in both directions for file and text, on both protocols.
Browser tests mock IPC and validate UI behavior; they do not replace native dialog tests.

```bash
npm exec --prefix hermod-app -- playwright install chromium
npm test --prefix hermod-app
cd hermod-app
APPIMAGE_EXTRACT_AND_RUN=1 npm run tauri -- build --ci --bundles appimage -- --locked
cd ..
hermod-app/scripts/smoke-appimage.sh target/release/bundle/appimage/Hermod_0.4.0_amd64.AppImage
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --locked --release --target x86_64-unknown-linux-musl -p hermod-cli
readelf -l target/x86_64-unknown-linux-musl/release/hermod
```

The musl binary must have no `INTERP` program header. On ARM64 Actions runners the
same procedure uses `aarch64-unknown-linux-musl`, native `musl-gcc`, and an ARM64 AppImage.

## Fixed test signing: one-time setup

Create keys once on a trusted machine with OpenSSL and JDK `keytool`:

```bash
./hermod-app/scripts/init-signing-keys.sh
```

The script refuses to overwrite `signing-private/`. Back up that ignored directory
securely. It contains the PFX, Android JKS, public certificate and passwords. Do not
commit it, upload it as an artifact, or rerun it for every version. A new signing key
can prevent upgrades and require users to export their inbox and reinstall.

Set repository **Actions secrets** using the files, without pasting secrets into logs:

```bash
gh secret set WINDOWS_PFX_BASE64 < signing-private/windows-pfx-base64
gh secret set WINDOWS_PFX_PASSWORD < signing-private/windows-password
gh secret set ANDROID_KEYSTORE_BASE64 < signing-private/android-keystore-base64
gh secret set ANDROID_KEYSTORE_PASSWORD < signing-private/android-password
printf hermod | gh secret set ANDROID_KEY_ALIAS
gh secret set ANDROID_KEY_PASSWORD < signing-private/android-password
```

These commands are for the maintainer to run. Missing signing secrets make the package
jobs fail explicitly. PR checks use Android debug signing only for disposable emulator
builds; those APKs are not published.

Windows uses `Name=Hermod.Transfer.Test`, `Publisher=CN=HermodTest` and version
`major.minor.patch.0` for both x64 and ARM64. The same PFX signs both packages. Only its
public CER is exported. The installer embeds its expected thumbprint, checks the CMS
signature and package identity, checks WebView2, imports the public certificate into
`LocalMachine\TrustedPeople` through a separate elevated helper, then deploys the
MSIX in the original user context. Deployment verifies the complete signed package.
CI performs the same installation and launches the installed application. The generic
Authenticode root-chain check is not a reason to place a test leaf certificate in Root.
See Microsoft's [certificate guidance](https://learn.microsoft.com/windows/msix/package/create-certificate-package-signing)
and [WebView2 distribution guidance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution).

Android retains `org.hermod.transfer`; `versionCode=40000` for 0.4.0. Increase the code
on every release and keep the signing key and alias. Neither the public certificate
nor an MSIX signed as HermodTest grants a Microsoft Store publisher identity. Store
submission is a separate process using the identity assigned by Partner Center.

## Android toolchain and platform boundary

Both GUIs use one React application in `hermod-app/src`. `ui.tsx` selects Microsoft's
[Fluent UI React v9](https://fluent2.microsoft.design/get-started/develop) on desktop
and Google's [Material Web](https://material-web.dev/about/intro/) components on Android.
Business state and validation stay in Rust. There is no Compose UI, UniFFI bridge,
standalone Android application, or separate GUI-core crate to maintain.

The pinned Tauri CLI generates the native Android shell in CI. The script
`hermod-app/scripts/configure-android.mjs` applies the tracked Gradle template,
pins Gradle **9.6.0** with distribution SHA-256 verification, and pins AGP **9.4.0**.
JDK **17**, Build Tools **37.0.0**, NDK **29.0.14206865**, `compileSdk=37`,
`targetSdk=37`, `minSdk=34` are explicit. The application and library modules compile
against the same SDK. Install `platforms;android-37.0`.
See the [AGP compatibility table](https://developer.android.com/build/releases/agp-9-4-0-release-notes).
The Tauri/official plugin templates still use Kotlin **2.2.10** and the legacy Android
DSL, so the generator explicitly disables AGP built-in Kotlin/new DSL for compatibility.
This does not change API targets. Tauri's `bundle.android.minSdkVersion=34` also sets
the Rust native minimum API; target API 37 does not raise it.

CI runs `tauri android init`, applies the template, then runs `tauri android build`
for one ABI. Release uses ARM64 and the fixed keystore; x86_64 is only for debug emulator
tests. Gradle's release tasks reject missing signing secrets. All GUI-related Android
source and tooling live under `hermod-app`.

The native plugin uses `ACTION_OPEN_DOCUMENT`, `ACTION_OPEN_DOCUMENT_TREE` and
`ACTION_CREATE_DOCUMENT`. Persisted grants belong to the selected export directory;
there is no all-files permission and no URI-to-filesystem-path guess. The official
[Tauri file plugin](https://v2.tauri.app/plugin/file-system/) opens authorized descriptors
inside Rust. File bytes never cross JavaScript IPC. A blocking reader feeds two bounded
64 KiB chunks; unknown lengths are staged in private storage. Cancel stops the transfer
and closes its streams as pending provider reads return. Verified receive data is kept
when export or directory authorization fails; the UI explains how to retry.

Rust owns and monitors each transfer independently of WebView timers. The Android
foreground service keeps the process active, reports progress and offers cancellation;
activity recreation does not create a new transfer. Android 15+ service timeouts cancel
the current task. Process termination does not resume transfers: the next startup
cleans only unverified partial data after obtaining the storage lock.

Native libraries are uncompressed, APK-aligned and loaded from read-only installation
storage. Every ELF load segment is checked for 16 KB alignment. Android 17's
[local network permission](https://developer.android.com/about/versions/17/behavior-changes-17)
is requested explicitly; denial is reported as a permission problem. Notifications
have their own permission. There is no fixed orientation or aspect ratio. Material Web
uses the Android system accent seed, system dark/light mode, bottom navigation on phones
and a navigation rail on tablets. No native Material screens are maintained.

Android CI runs API 34 and API 37 instrumentation on unpublished x86_64 builds; API 37
uses the 16 KB system image. Checks exercise WebView/Rust loading, settings across
activity recreation, a real SAF picker with unknown-length and revoked documents,
API 37 permission denial, foreground-service survival and notification cancellation.
Linux tests cover actual transfers, bounded streaming and failed exports. Browser tests
exercise the same React flows on desktop, phone and tablet. These are complementary
checks; physical SAF providers and OEM background policies still need device acceptance.
The fixture document provider is only included in debug APKs.

## Pipeline and publishing

Pushes and PRs run Linux quality checks and Android API 34/37 tests. Tags and manual
workflow dispatch also build:

| Runner | Output |
| --- | --- |
| `windows-2025` | x64 GUI MSIX |
| `windows-11-arm` | ARM64 GUI MSIX |
| `ubuntu-22.04` | x64 AppImage and musl CLI |
| `ubuntu-22.04-arm` | ARM64 AppImage and musl CLI |
| `ubuntu-22.04` + Android SDK | ARM64 Tauri APK |

Only tags publish. Every required job must succeed first. `check-release-files.py`
requires an exact nine-file allowlist: seven programs, `Hermod-Test.cer`, and
`Install-Hermod.ps1`. No checksums, ZIPs, debs, debug symbols, build logs, private keys
or canceled-platform programs are uploaded to Release. Internal Actions artifact
containers are a GitHub transport mechanism, not ZIP release assets.

`check-release-version.mjs` checks workspace, npm, Tauri, Android and tag versions.
After reviewing the commit and configuring signing secrets, the maintainer pushes:

```bash
git push origin master
git push origin v0.4.0
```

Do not move an already published tag to hide a failed build. Fix it in a new commit
and publish a new version as appropriate. Workflow dispatch can validate packaging
without creating a GitHub Release.
