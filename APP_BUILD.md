# Building and releasing Hermod 0.4.3

[User guide](README.md) · [中文使用说明](README.zh-CN.md)

Branch and PR checks run in `.github/workflows/ci.yml`. Release builds run in
`.github/workflows/build-release.yml` and reuse those checks. Local validation is
Linux-only. Windows checks use a temporary signed copy in Actions; released MSIX files
remain unsigned for Microsoft Store submission. Android builds and
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
- `hermod-app/packaging`: MSIX manifest and English/Chinese Windows string resources; no `windows` subdirectory or public installer script.
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
hermod-app/scripts/smoke-appimage.sh target/release/bundle/appimage/Hermod_0.4.3_amd64.AppImage
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --locked --release --target x86_64-unknown-linux-musl -p hermod-cli
readelf -l target/x86_64-unknown-linux-musl/release/hermod
```

The musl binary must have no `INTERP` program header. On ARM64 Actions runners the
same procedure uses `aarch64-unknown-linux-musl`, native `musl-gcc`, and an ARM64 AppImage.

## Microsoft Store packages

Windows x64 and ARM64 release files are **unsigned MSIX packages for Partner Center**.
Download `hermod-windows-x64.msix` and `hermod-windows-arm64.msix` from the same Release,
then upload both to the app submission's **Packages** page. No ZIP conversion, certificate,
PowerShell installer or additional signing is needed. Microsoft signs the packages after
certification; these GitHub files are not direct-install downloads. See Microsoft's
[package upload guide](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/upload-app-packages)
and [Store signing guidance](https://learn.microsoft.com/en-us/windows/msix/package/sign-msix-package-guide).

The checked-in manifest uses the exact Partner Center values:

| Property | Value |
| --- | --- |
| Identity name | `FeelLiao.HermodTransfer` |
| Identity publisher | `CN=37A01BFF-1DAB-45E5-9B67-AA76C0655A7C` |
| Publisher display name | `FeelLiao` |
| Package family name (derived, checked in CI) | `FeelLiao.HermodTransfer_8qnpvd7fj1r9a` |
| Store ID (listing metadata, not a manifest identity field) | `9PMPWN3HMR3M` |
| Languages | `en-US`, `zh-CN` |

The supplied Package SID is account metadata, not an AppxManifest identity attribute.
Display name remains **Hermod**. `Strings/en-US/Resources.resw` and
`Strings/zh-CN/Resources.resw` provide localized descriptions referenced by `ms-resource:`;
MakePRI compiles both into one `resources.pri`. The GUI has matching English/Chinese support.
In Partner Center, add and complete both language listings; declaring languages in the
package does not create translated screenshots, descriptions or other Store listing text.
The app's `runFullTrust` capability is needed to run the Rust/Tauri desktop process;
explain its userspace networking and user-selected file access during certification.
Submission remains subject to Microsoft's review and account requirements.

The Store [version rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements#package-version-numbering)
require a nonzero first component and a final `0`. `msix-version.mjs` maps application
`major.minor.patch` to **`(major + 1).minor.patch.0`**: release **0.4.3 → 1.4.3.0**,
and future app 1.0.0 → MSIX 2.0.0.0. All components are range-checked. Keep this mapping
stable so upgrades remain ordered; the app's About/version field still shows 0.4.3.

Windows CI requires no `WINDOWS_PFX_BASE64` or `WINDOWS_PFX_PASSWORD` secrets. Any old
Windows secrets may be removed from GitHub, but local signing backups are not touched.
MakeAppx validates the package, then `verify-msix.py` checks identity, version, executable
architecture, both languages, resource files and the absence of a package signature.
For installation/startup checks only, CI copies the MSIX into runner temporary storage,
creates a one-day non-exportable certificate with the same publisher, signs the copy,
and installs it. It verifies the PFN and window creation, then removes the installed
copy, certificate and key. A hash comparison and another package check ensure the
Release MSIX is unchanged. No temporary signing material enters artifacts or Release.
The runner installs Microsoft's WebView2 Runtime if needed.

After publication, users install the Store-signed app (Store ID above); links become
usable only after Microsoft makes the listing available. Old `Hermod.Transfer.Test`
installations have a different identity: export important inbox files before migration
or uninstalling. No automatic migration between these Windows package identities is claimed.

## Fixed Android signing: one-time setup

Keep the existing Android key when upgrading. New maintainers can create a key once,
using OpenSSL and JDK `keytool`:

```bash
./hermod-app/scripts/init-signing-keys.sh
```

The script creates **only Android signing material** and refuses to overwrite an existing
`signing-private/` directory. Back it up securely; never commit or upload its contents.
Existing directories containing older Windows signing files are left untouched and must
not be regenerated simply because Windows now uses Store signing.

In **Settings → Secrets and variables → Actions → Secrets**, create repository secrets
from the file contents, not filenames or Actions variables:

| Secret name | Value |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | Contents of `signing-private/android-keystore-base64` |
| `ANDROID_KEYSTORE_PASSWORD` | Contents of `signing-private/android-password` |
| `ANDROID_KEY_ALIAS` | Literal `hermod` |
| `ANDROID_KEY_PASSWORD` | Contents of `signing-private/android-password` |

With GitHub CLI authenticated, the maintainer can set them without printing values:

```bash
gh secret set ANDROID_KEYSTORE_BASE64 < signing-private/android-keystore-base64
gh secret set ANDROID_KEYSTORE_PASSWORD < signing-private/android-password
printf hermod | gh secret set ANDROID_KEY_ALIAS
gh secret set ANDROID_KEY_PASSWORD < signing-private/android-password
```

Android packaging checks missing secrets before toolchain setup. If only secrets were
missing, configure them and re-run the failed jobs. Source/workflow changes require a
new commit; retrying an old tag still uses its old source. Use **Actions → Build and
release → Run workflow → master** to check new packaging, then publish a new version.
Do not move an already pushed tag. Android retains `org.hermod.transfer` and uses
`versionCode=40300` for 0.4.3; keep its signing key and alias for future upgrades.

## GUI localization

`hermod-app/src/i18n.ts` selects `en-US` or `zh-CN` from the language setting or browser
system preferences. English strings are catalog keys; `src/locales/zh-CN.json` contains
translations and is also embedded by Rust for native file dialogs and Android notification
text. The shared `Settings.language` accepts only `system`, `en-US`, or `zh-CN`; older
settings files default to `system`. The UI updates `html.lang`, previews changes immediately,
and saves the preference with the other settings. Raw log content and unknown diagnostics
remain intact. Desktop browser checks exercise both languages, persistence, errors and
transfer states; no Android test project is introduced.

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
| `windows-2025` | x64 GUI MSIX (Store upload) |
| `windows-11-arm` | ARM64 GUI MSIX (Store upload) |
| `ubuntu-22.04` | x64 AppImage and musl CLI |
| `ubuntu-22.04-arm` | ARM64 AppImage and musl CLI |
| `ubuntu-22.04` + Android SDK | ARM64 Tauri APK |

Only version-tag pushes publish. Every required job must succeed first. `check-release-files.py`
requires an exact seven-file allowlist: two Store MSIX packages, four Linux programs
and one Android APK. No certificates, installation scripts, checksums, ZIPs, debs, debug symbols, build logs, private keys
or canceled-platform programs are uploaded to Release. Internal Actions artifact
containers are a GitHub transport mechanism, not ZIP release assets.

`check-release-version.mjs` checks workspace, npm, Tauri, Android and tag versions.
After reviewing the commit and configuring signing secrets, the maintainer pushes:

```bash
git push origin master
git push origin v0.4.3
```

Do not move an already published tag to hide a failed build. Fix it in a new commit
and publish a new version as appropriate. Workflow dispatch can validate packaging
without creating a GitHub Release.
