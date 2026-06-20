#!/usr/bin/env bash
# prove.sh — single re-prove entrypoint for the core capability.
# Runs the full test suite, then the hermetic e2e proof (mocked hyprctl)
# that drives the REAL binary: start → accrue → flush → --report (real
# focused-window wall time) → liveness gate. Exit 0 = capability proven.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
echo "== unit + integration tests =="
cargo test --release
echo
echo "== e2e capability proof =="
bash "$ROOT/scripts/e2e-proof.sh"
