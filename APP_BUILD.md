# Building and releasing Hermod 0.4.2

[User guide](README.md) · [中文使用说明](README.zh-CN.md)

Branch and PR checks run in `.github/workflows/ci.yml`. Release builds run in
`.github/workflows/build-release.yml` and reuse those checks. Local validation is
Linux-only. Windows installation/runtime checks run in Actions. Android builds and
signs the ARM64 APK in Actions and checks the resulting package; no Android emulator
or runtime tests run. A successful Linux build does not verify other platforms.

## Repository layout

- `hermod-core`: cryptography, IPv6 signaling, WireGuard/QUIC, HTTP transfer, settings validation.
- `hermod-cli`: clap shell and terminal I/O.
- `hermod-app/src`: shared React pages and a small official Fluent UI / Material Web component adapter.
- `hermod-app/src-tauri/src/app`: persistent settings, jobs, cancellation, bounded streams, logs and verified inbox.
- `hermod-app/src-tauri/src`: desktop and Android Tauri commands; background job monitoring.
- `hermod-app/plugins/documents`: Android SAF, permissions, system color and foreground notification adapters. No native UI screens.
- `hermod-app/android`: pinned Android Gradle template; generated Tauri shell stays ignored under `src-tauri/gen/android`.
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
hermod-app/scripts/smoke-appimage.sh target/release/bundle/appimage/Hermod_0.4.2_amd64.AppImage
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

This creates a Windows code-signing certificate and its private key in a PFX, plus
a separate Android signing key in a JKS. These are not SSH keys. Windows x64 and
ARM64 reuse the same PFX. Do not regenerate the keys for each workflow run.

In the GitHub repository, open **Settings → Secrets and variables → Actions →
Secrets → New repository secret**. Use repository secrets, not Actions variables.
Enter the file **contents**, not the filename or path:

| Secret name | Value from the one-time initializer |
| --- | --- |
| `WINDOWS_PFX_BASE64` | Contents of `signing-private/windows-pfx-base64` |
| `WINDOWS_PFX_PASSWORD` | Contents of `signing-private/windows-password` |
| `ANDROID_KEYSTORE_BASE64` | Contents of `signing-private/android-keystore-base64` |
| `ANDROID_KEYSTORE_PASSWORD` | Contents of `signing-private/android-password` |
| `ANDROID_KEY_ALIAS` | Literal `hermod` |
| `ANDROID_KEY_PASSWORD` | Contents of `signing-private/android-password` |

The `.cer` contains only the Windows public certificate; it cannot replace the
PFX signing secret. The initializer also works in WSL with OpenSSL and JDK `keytool`.
If either command is missing, install `openssl` and `openjdk-17-jdk-headless` first.
The directory stays local and ignored by Git; only the public certificate is released.

Alternatively, with GitHub CLI installed and authenticated, set the same repository
secrets from the repository root without printing their values:

```bash
gh secret set WINDOWS_PFX_BASE64 < signing-private/windows-pfx-base64
gh secret set WINDOWS_PFX_PASSWORD < signing-private/windows-password
gh secret set ANDROID_KEYSTORE_BASE64 < signing-private/android-keystore-base64
gh secret set ANDROID_KEYSTORE_PASSWORD < signing-private/android-password
printf hermod | gh secret set ANDROID_KEY_ALIAS
gh secret set ANDROID_KEY_PASSWORD < signing-private/android-password
```

These commands are for the maintainer to run. Missing signing secrets make the package
jobs fail before toolchain setup and compilation. Pushes and PRs run Linux quality
checks; Android release builds run for tags and manual workflow dispatch. See
[GitHub's repository secret instructions](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets).

If only secrets were missing, configure them and re-run the failed jobs. If source or
workflow changes are also needed, push the fix and use **Actions → Build and release →
Run workflow → master** to test the new commit. Re-running the old tag's jobs still uses
that tag's old source and workflow. Publish a new version after the fixed pipeline
passes; do not move an already pushed release tag.

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

Android retains `org.hermod.transfer`; `versionCode=40200` for 0.4.2. Increase the code
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

Each native plugin declares its own AndroidX compile dependencies. In particular,
`plugins/documents/android` directly depends on `androidx.activity:activity:1.10.1`
for `ActivityResult`, aligned with the application's Activity version. An
`implementation` dependency in the app or Tauri module is not exported onto the
document plugin's compile classpath. AGP legacy-DSL deprecation warnings are separate
from Kotlin `e:` compilation errors; inspect the first error and `What went wrong`
section, not only Tauri's final Gradle exit-code wrapper.

CI runs `tauri android init`, applies the template, then runs `tauri android build`
for ARM64 with the fixed keystore. Gradle's release tasks reject missing signing
secrets. All GUI-related Android source and tooling live under `hermod-app`.

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

Android CI only compiles and signs the ARM64 APK, then verifies its signature,
minimum/target SDK, native ABI, ZIP alignment and every ELF load segment's 16 KB
alignment. It does not install an emulator or run instrumentation, Android UI tests
or a separate Android lint task. Linux tests cover transfers, bounded streaming and
failed exports; browser tests cover the desktop GUI. Android runtime behavior is not
an automated release gate.

## Pipeline and publishing

The workflows have separate responsibilities:

| Event | Workflow | Result |
| --- | --- | --- |
| Push to `master` or another branch; pull request | `CI` (`ci.yml`) | Linux quality checks only |
| Push a `v*` tag | `Build and release` (`build-release.yml`) | Quality checks, all platform packages, then Release |
| Manually run `Build and release` on a branch or tag | `Build and release` | Quality checks and downloadable Actions artifacts; no Release |

The release workflow calls `ci.yml` through `workflow_call` at the same commit,
using GitHub's [reusable workflow mechanism](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows).
All package jobs depend on that call succeeding. Test commands have one source of
truth, and a tag does not start a second standalone CI run. CI has read-only repository
permissions and does not receive signing secrets; publishing alone gets `contents: write`.

Tags and manual runs build the following packages after the checks pass:

| Runner | Output |
| --- | --- |
| `windows-2025` | x64 GUI MSIX |
| `windows-11-arm` | ARM64 GUI MSIX |
| `ubuntu-22.04` | x64 AppImage and musl CLI |
| `ubuntu-22.04-arm` | ARM64 AppImage and musl CLI |
| `ubuntu-22.04` + Android SDK | ARM64 Tauri APK |

Only version-tag pushes publish. Every required job must succeed first. `check-release-files.py`
requires an exact nine-file allowlist: seven programs, `Hermod-Test.cer`, and
`Install-Hermod.ps1`. No checksums, ZIPs, debs, debug symbols, build logs, private keys
or canceled-platform programs are uploaded to Release. Internal Actions artifact
containers are a GitHub transport mechanism, not ZIP release assets.

`check-release-version.mjs` checks workspace, npm, Tauri, Android and tag versions.
After reviewing the commit and configuring signing secrets, the maintainer pushes:

```bash
git push origin master
git push origin v0.4.2
```

Do not move an already published tag to hide a failed build. Fix it in a new commit
and publish a new version as appropriate. Workflow dispatch can validate packaging
without creating a GitHub Release.
