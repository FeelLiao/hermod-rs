#!/usr/bin/env bash
set -euo pipefail
app=$(realpath "${1:?AppImage path required}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
chmod +x "$app"
cd "$work"
"$app" --appimage-extract >/dev/null
export XDG_DATA_HOME="$work/data" XDG_CONFIG_HOME="$work/config"
export WEBKIT_DISABLE_DMABUF_RENDERER=1
# 在独立图形会话中检查实际窗口；只检查退出码无法发现白屏前崩溃。
dbus-run-session -- xvfb-run -a bash -euc '
  ./squashfs-root/AppRun >app.log 2>&1 &
  pid=$!
  trap "kill $pid 2>/dev/null || true" EXIT
  for attempt in $(seq 1 40); do
    kill -0 "$pid" || { cat app.log; exit 1; }
    if xdotool search --onlyvisible --name "Hermod" >/dev/null; then sleep 3; kill -0 "$pid"; exit 0; fi
    sleep 1
  done
  cat app.log
  exit 1
'
