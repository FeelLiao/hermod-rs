#!/usr/bin/env bash
set -euo pipefail
umask 077
cd "$(dirname "$0")/../.."
destination=signing-private
[[ ! -e "$destination" ]] || { echo 'signing-private already exists; keep the existing keys for upgrades' >&2; exit 1; }
command -v openssl >/dev/null
command -v keytool >/dev/null
mkdir "$destination"
openssl rand -hex 32 > "$destination/android-password"
keytool -genkeypair -keystore "$destination/android.jks" -storetype JKS -alias hermod \
  -keyalg RSA -keysize 3072 -validity 10000 -dname 'CN=HermodTest' \
  -storepass:file "$destination/android-password" -keypass:file "$destination/android-password"
base64 -w0 "$destination/android.jks" > "$destination/android-keystore-base64"
printf '%s\n' 'Fixed Android signing key created in signing-private. Back up this directory securely.' 'See APP_BUILD.md for GitHub Secrets. Never upload this directory to a Release.'
