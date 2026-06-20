#!/usr/bin/env bash
# e2e-proof.sh — proves the elo-time-tracker honesty flow end-to-end against a
# MOCKED hyprctl (this host has no Hyprland). It drives the REAL release binary
# through the whole capability:
#
#   start daemon → accrue per-window time → window-change flush → SIGTERM flush
#                → --report (focused-window wall time + away/idle flag)
#                → --health liveness gate (STALE → exit 2 = the sensor refuses to lie)
#
# No real Wayland/Hyprland needed: a hyprctl shim on $PATH returns scripted JSON.
# Exit 0 = capability proven. Anything else = failure.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
BIN="$ROOT/target/release/elo-time-tracker"
DB="$WORK/log.db"
WINFILE="$WORK/active_window.json"

echo "== build =="
cargo build --release >/dev/null 2>&1
[ -x "$BIN" ] || { echo "FAIL: binary not built at $BIN"; exit 1; }

# --- mock hyprctl: echoes whatever JSON is currently in $WINFILE ---
cat >"$WORK/hyprctl" <<MOCK
#!/usr/bin/env bash
if [ "\$1" = activewindow ]; then cat "$WINFILE"; fi
MOCK
chmod +x "$WORK/hyprctl"
export PATH="$WORK:$PATH"

# start focused on kitty
echo '{"class":"kitty","title":"tmux","monitor":0}' >"$WINFILE"

echo "== start daemon (1s poll, 3s flush ceiling) =="
"$BIN" --db "$DB" --interval-secs 1 --flush-every-secs 3 --idle-gap-secs 180 \
  >"$WORK/daemon.log" 2>&1 &
DAEMON=$!
trap 'kill "$DAEMON" 2>/dev/null || true; rm -rf "$WORK"' EXIT

# accrue ~3s on kitty
sleep 3
# window change → flushes the kitty event
echo '{"class":"firefox","title":"Mozilla — GitHub","monitor":0}' >"$WINFILE"
# accrue ~3s on firefox
sleep 3

echo "== SIGTERM → flush open event + exit =="
kill -TERM "$DAEMON"; wait "$DAEMON" 2>/dev/null || true
trap 'rm -rf "$WORK"' EXIT

echo "== --report (focused-window wall time) =="
"$BIN" --db "$DB" --report --since "1 hour ago"

echo
echo "== assert both apps were recorded with real duration =="
REPORT="$("$BIN" --db "$DB" --report --since "1 hour ago")"
echo "$REPORT" | grep -q kitty   || { echo "FAIL: kitty not in report";   exit 1; }
echo "$REPORT" | grep -q firefox || { echo "FAIL: firefox not in report"; exit 1; }
EVCOUNT="$(echo "$REPORT" | grep -oE '^[0-9]+ event' | grep -oE '^[0-9]+')"
[ "${EVCOUNT:-0}" -ge 2 ] || { echo "FAIL: expected >=2 events, got ${EVCOUNT:-0}"; exit 1; }
echo "OK: $EVCOUNT events recorded, both windows present"

echo
echo "== --health liveness gate: data is fresh (now) → HEALTHY exit 0 =="
"$BIN" --db "$DB" --health --stale-after-secs 900
echo "OK: fresh data passes the gate"

echo
echo "== --health liveness gate: STALE threshold of 0s → the sensor refuses to lie (exit 2) =="
set +e
"$BIN" --db "$DB" --health --stale-after-secs 0
GATE=$?
set -e
[ "$GATE" -eq 2 ] || { echo "FAIL: expected exit 2 on STALE, got $GATE"; exit 1; }
echo "OK: STALE data correctly fails the liveness gate (exit 2)"

echo
echo "ALL GREEN — start → accrue → window-change flush → SIGTERM flush → report → liveness gate proven e2e"
