#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
case "${HERMOD_TEST_API:?}" in
  34) image='system-images;android-34;google_apis;x86_64' ;;
  37) image='system-images;android-37.0;google_apis_ps16k;x86_64' ;;
  *) exit 1 ;;
esac
sdk="$ANDROID_HOME/cmdline-tools/latest/bin"
"$sdk/sdkmanager" "$image"
printf 'no\n' | "$sdk/avdmanager" create avd --force --name hermod-test --package "$image" --device pixel_7
"$ANDROID_HOME/emulator/emulator" -avd hermod-test -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect > "$RUNNER_TEMP/hermod-emulator.log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT
adb="$ANDROID_HOME/platform-tools/adb"
for attempt in $(seq 1 180); do
  kill -0 "$pid" || { cat "$RUNNER_TEMP/hermod-emulator.log"; exit 1; }
  if [[ $("$adb" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r') == 1 ]]; then break; fi
  sleep 2
done
[[ $("$adb" shell getprop sys.boot_completed | tr -d '\r') == 1 ]]
[[ $("$adb" shell getprop ro.build.version.sdk | tr -d '\r') == "$HERMOD_TEST_API" ]]
"$adb" shell input keyevent 82
(cd hermod-app/src-tauri/gen/android && ./gradlew --no-daemon -PabiList=x86_64 -ParchList=x86_64 -PtargetList=x86_64 \
  :app:connectedX86_64DebugAndroidTest :app:lintX86_64Debug -x :app:rustBuildX86_64Debug)
