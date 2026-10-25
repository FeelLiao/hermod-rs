#!/usr/bin/env bash
set -euo pipefail
umask 077
cd "$(dirname "$0")/../.."
destination=signing-private
[[ ! -e "$destination" ]] || { echo 'signing-private already exists; keep the existing keys for upgrades' >&2; exit 1; }
command -v openssl >/dev/null
command -v keytool >/dev/null
mkdir "$destination"
openssl rand -hex 32 > "$destination/windows-password"
openssl rand -hex 32 > "$destination/android-password"
openssl req -x509 -newkey rsa:3072 -sha256 -nodes -days 3650 -subj '/CN=HermodTest' \
  -addext 'keyUsage=critical,digitalSignature' -addext 'extendedKeyUsage=codeSigning' \
  -addext 'basicConstraints=critical,CA:FALSE' \
  -keyout "$destination/windows.key" -out "$destination/windows.pem"
openssl pkcs12 -export -out "$destination/windows.pfx" -inkey "$destination/windows.key" \
  -in "$destination/windows.pem" -passout "file:$destination/windows-password"
openssl x509 -in "$destination/windows.pem" -outform der -out "$destination/Hermod-Test.cer"
keytool -genkeypair -keystore "$destination/android.jks" -storetype JKS -alias hermod \
  -keyalg RSA -keysize 3072 -validity 10000 -dname 'CN=HermodTest' \
  -storepass:file "$destination/android-password" -keypass:file "$destination/android-password"
base64 -w0 "$destination/windows.pfx" > "$destination/windows-pfx-base64"
base64 -w0 "$destination/android.jks" > "$destination/android-keystore-base64"
printf '%s\n' 'Fixed test keys created in signing-private. Back up this directory securely.' 'See APP_BUILD.md for GitHub Secrets. Never upload this directory to a Release.'
