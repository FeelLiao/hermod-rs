#!/usr/bin/env bash
# CI-only Android build entrypoint. Never invoke Android builds on a developer host.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == "true" ]] || { echo 'Android builds are allowed only in GitHub Actions' >&2; exit 1; }
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
ndk_version=28.2.13676358
: "${ANDROID_SDK_ROOT:?Android SDK required}"
ndk="$ANDROID_SDK_ROOT/ndk/$ndk_version"
[[ -f "$ndk/source.properties" ]] || { echo "Missing pinned NDK $ndk_version" >&2; exit 1; }
export ANDROID_NDK_HOME="$ndk"
toolbin="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CARGO_TARGET_DIR="$root/target/android"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$toolbin/aarch64-linux-android34-clang"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$toolbin/x86_64-linux-android34-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export CC_x86_64_linux_android="$CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER"
export CXX_aarch64_linux_android="$toolbin/aarch64-linux-android34-clang++"
export CXX_x86_64_linux_android="$toolbin/x86_64-linux-android34-clang++"
export AR_aarch64_linux_android="$toolbin/llvm-ar"
export AR_x86_64_linux_android="$toolbin/llvm-ar"
[[ -z "${CARGO_ENCODED_RUSTFLAGS:-}" ]] || { echo 'CARGO_ENCODED_RUSTFLAGS would override required 16KB flags' >&2; exit 1; }
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
# Do not silently regenerate mismatched FRB bindings.
python3 - <<'PY'
import pathlib, re
root = pathlib.Path.cwd()
assert re.search(r'^\s*flutter_rust_bridge:\s*[\'"]?2\.13\.0[\'"]?\s*$', (root/'flutter/pubspec.yaml').read_text(), re.M), 'Dart FRB must be exactly 2.13.0'
assert 'flutter_rust_bridge = "=2.13.0"' in (root/'flutter-bridge/Cargo.toml').read_text(), 'Rust FRB must be exactly 2.13.0'
PY
for entry in 'aarch64-linux-android:arm64-v8a' 'x86_64-linux-android:x86_64'; do
  target="${entry%%:*}"
  abi="${entry##*:}"
  cargo build --locked --release -p hermod-flutter-bridge --target "$target"
  mkdir -p "flutter/android/app/src/main/jniLibs/$abi"
  cp "$CARGO_TARGET_DIR/$target/release/libhermod_flutter_bridge.so" "flutter/android/app/src/main/jniLibs/$abi/"
done
cd "$root/flutter"
flutter pub get
flutter build apk --release --target-platform android-arm64,android-x64
cd "$root"
mkdir -p dist
if [[ "${HERMOD_ANDROID_UNSIGNED:-false}" == 'true' ]]; then
  destination=dist/hermod-android-UNSIGNED.apk
else
  destination=dist/hermod-android-release.apk
fi
cp flutter/build/app/outputs/flutter-apk/app-release.apk "$destination"
# Inspect actual APK contents, all native ELF load segments, and ZIP alignment.
python3 - "$destination" "$toolbin/llvm-readelf" <<'PY'
import pathlib, re, subprocess, sys, tempfile, zipfile
apk, readelf = sys.argv[1:]
expected = {'arm64-v8a': 'AArch64', 'x86_64': 'Advanced Micro Devices X86-64'}
with zipfile.ZipFile(apk) as archive, tempfile.TemporaryDirectory() as directory:
    libs = [name for name in archive.namelist() if name.startswith('lib/') and name.endswith('.so')]
    assert {name.split('/')[1] for name in libs} == set(expected), 'Unexpected/missing APK ABI'
    for abi, machine in expected.items():
        assert f'lib/{abi}/libhermod_flutter_bridge.so' in libs, f'Missing bridge: {abi}'
    for index, name in enumerate(libs):
        target = pathlib.Path(directory) / f'{index}.so'
        target.write_bytes(archive.read(name))
        header = subprocess.check_output([readelf, '-h', str(target)], text=True)
        assert expected[name.split('/')[1]] in header, f'Wrong ELF machine: {name}'
        segments = subprocess.check_output([readelf, '-lW', str(target)], text=True)
        loads = [line.split() for line in segments.splitlines() if line.strip().startswith('LOAD ')]
        assert loads and all(int(line[-1], 16) >= 16384 and
                             (int(line[1], 16) - int(line[2], 16)) % 16384 == 0
                             for line in loads), f'Non-16KB ELF: {name}'
PY
build_tools="$ANDROID_SDK_ROOT/build-tools/36.0.0"
"$build_tools/zipalign" -c -P 16 -v 4 "$destination"
# Workflow exports the discovered command-line-tools directory, not necessarily latest/.
apkanalyzer="$(command -v apkanalyzer || true)"
[[ -n "$apkanalyzer" && -x "$apkanalyzer" ]] || { echo 'apkanalyzer not found in configured Android tools PATH' >&2; exit 1; }
"$apkanalyzer" manifest print "$destination" > dist/android-manifest.xml
python3 - <<'PY'
import pathlib, xml.etree.ElementTree as ET
root = ET.fromstring(pathlib.Path('dist/android-manifest.xml').read_text())
ns = '{http://schemas.android.com/apk/res/android}'
permissions = {node.get(ns+'name') for node in root.findall('uses-permission')}
required = {'android.permission.INTERNET', 'android.permission.WAKE_LOCK', 'android.permission.POST_NOTIFICATIONS', 'android.permission.FOREGROUND_SERVICE', 'android.permission.FOREGROUND_SERVICE_DATA_SYNC'}
assert required <= permissions, f'Missing required permissions: {sorted(required - permissions)}'
assert 'android.permission.MANAGE_EXTERNAL_STORAGE' not in permissions, 'All-files access is forbidden'
app = root.find('application')
assert app is not None, 'Missing application element'
assert app.get(ns+'debuggable', 'false') == 'false', 'Release APK is debuggable'
def is_data_sync(value):
    # Compiled Android flags may be decoded as names, decimal or hexadecimal.
    if value == 'dataSync':
        return True
    try:
        return int(value, 0) == 1
    except (TypeError, ValueError):
        return False
services = [node for node in app.findall('service') if node.get(ns+'name', '').endswith('.TransferService')]
assert len(services) == 1, f'Expected one TransferService, found {len(services)}'
service = services[0]
assert service.get(ns+'exported') == 'false', 'TransferService must not be exported'
service_type = service.get(ns+'foregroundServiceType')
assert is_data_sync(service_type), f'TransferService must be dataSync (flag 1), found {service_type!r}'
PY
if [[ "${HERMOD_ANDROID_UNSIGNED:-false}" == 'true' ]]; then
  if "$build_tools/apksigner" verify "$destination" >/dev/null 2>&1; then
    echo 'Unsigned artifact unexpectedly has a signature' >&2; exit 1
  fi
else
  "$build_tools/apksigner" verify --verbose --print-certs "$destination" | tee dist/android-signature.txt
  # Fail if any certificate is the Android development key.
  if grep -qi 'Android Debug' dist/android-signature.txt; then echo 'Debug certificate is forbidden' >&2; exit 1; fi
fi
sha256sum "$destination" > "$destination.sha256"
