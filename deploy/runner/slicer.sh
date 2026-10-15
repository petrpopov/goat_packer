#!/bin/sh
set -eu
mkdir -p "$HOME" "${XDG_CONFIG_HOME:-$HOME/.config}" "${XDG_CACHE_HOME:-$HOME/.cache}"
# Private headless display: no host sockets/GPU, TCP, account or network needed.
# displayfd avoids xvfb-run's SIGUSR1 handshake (hangs under QEMU on ARM hosts).
ready=$(mktemp)
Xvfb -displayfd 3 -screen 0 1280x1024x24 -nolisten tcp -ac 3>"$ready" &
xpid=$!
trap 'kill "$xpid" 2>/dev/null || true; rm -f "$ready"' EXIT
trap 'exit 143' TERM INT
n=0
while [ ! -s "$ready" ]; do
  kill -0 "$xpid" 2>/dev/null || exit 1
  n=$((n+1)); [ "$n" -lt 300 ] || { echo 'Xvfb startup timed out' >&2; exit 1; }
  sleep 0.1
done
export DISPLAY=":$(cat "$ready")"
/opt/slicer/AppRun "$@"
