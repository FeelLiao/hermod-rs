#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
output="${1:-$root/dist}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
build_tools="$ANDROID_HOME/build-tools/36.0.0"
mapfile -t apks < <(find "$root/hermod-app/src-tauri/gen/android/app/build/outputs/apk" -type f -name '*release-unsigned.apk')
if [[ ${#apks[@]} -ne 1 ]]; then printf 'Expected one unsigned release APK; got %s\n' "${#apks[@]}" >&2; exit 1; fi
private_dir="$(mktemp -d)"
trap 'rm -rf "$private_dir"' EXIT
export HERMOD_TEST_KEY_PASSWORD
HERMOD_TEST_KEY_PASSWORD="$(openssl rand -hex 24)"
keytool -genkeypair -noprompt -keystore "$private_dir/test.jks" -alias hermod-test \
  -storepass:env HERMOD_TEST_KEY_PASSWORD -keypass:env HERMOD_TEST_KEY_PASSWORD \
  -keyalg RSA -keysize 2048 -validity 3650 -dname 'CN=Hermod Test, O=Hermod, C=US'
"$build_tools/zipalign" -f -P 16 4 "${apks[0]}" "$private_dir/aligned.apk"
"$build_tools/apksigner" sign --ks "$private_dir/test.jks" --ks-key-alias hermod-test \
  --ks-pass env:HERMOD_TEST_KEY_PASSWORD --key-pass env:HERMOD_TEST_KEY_PASSWORD \
  --out "$output/hermod-android-arm64-test-signed.apk" "$private_dir/aligned.apk"
"$build_tools/apksigner" verify --verbose --print-certs "$output/hermod-android-arm64-test-signed.apk"
"$build_tools/zipalign" -c -P 16 4 "$output/hermod-android-arm64-test-signed.apk"
