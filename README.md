# elo-time-tracker

Rust daemon that polls the active window via Hyprland IPC every N seconds and
logs per-app focus time to SQLite. Built to feed the time-block view in the
elo-evolution cockpit.

- **Runtime target:** Arch Linux + Hyprland
- **Storage:** SQLite at `~/.local/share/elo-time-tracker/log.db`
- **Footprint:** single binary, < 20 MB RSS, < 1 % of one core at default 5 s polling

## Install

```bash
git clone https://github.com/ghostON3/elo-time-tracker
cd elo-time-tracker
cargo install --path .
```

The binary lands at `~/.cargo/bin/elo-time-tracker`.

## Run as a foreground daemon

```bash
elo-time-tracker --interval-secs 5
```

Useful flags:

| Flag | Default | Meaning |
|---|---|---|
| `--interval-secs N` | 5 | Poll cadence |
| `--flush-every-secs N` | 300 | Force-flush an open event when it grows past this |
| `--db PATH` | `~/.local/share/elo-time-tracker/log.db` | SQLite file |
| `--report` | – | Print summary instead of running the daemon |
| `--since "N hours ago"` | "24 hours ago" | Window for `--report` |
| `--export-json` | – | Dump all events as JSON on stdout |

## Run as a systemd user service

```bash
mkdir -p ~/.config/systemd/user
cp systemd/elo-time-tracker.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now elo-time-tracker.service
journalctl --user -u elo-time-tracker.service -f
```

The unit waits for `graphical-session.target` so it only starts after Hyprland.

## Sample report

```
$ elo-time-tracker --report --since "2 hours ago"
elo-time-tracker — events since 2026-05-16T13:00:00+00:00
14 event(s)

APP                                DURATION  BAR
----------------------------------------------------------------------
firefox                            1h12m30s  ##############################
kitty                                42m15s  #################
Code                                  18m05s  #######
slack                                  4m20s  ##

total: 2h17m10s
```

## JSON export

```bash
elo-time-tracker --export-json | jq '.[] | select(.app_class=="firefox")'
```

Each row:

```json
{
  "id": 17,
  "app_class": "firefox",
  "window_title": "Mozilla — GitHub",
  "monitor": "0",
  "started_at": "2026-05-16T13:42:08+00:00",
  "duration_secs": 312
}
```

## Schema

```sql
CREATE TABLE events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  app_class     TEXT NOT NULL,
  window_title  TEXT NOT NULL,
  monitor       TEXT NOT NULL,
  started_at    TEXT NOT NULL,   -- RFC 3339 UTC
  duration_secs INTEGER NOT NULL
);
```

Consecutive polls of the same `(app_class, window_title, monitor)` are
collapsed into a single row. A row is finalised when either:

1. the active window changes, **or**
2. the open event's duration reaches `--flush-every-secs` (default 5 min).

## Development

```bash
cargo fmt --all
cargo check --all-targets
cargo test --all
```

CI runs the same three steps on every push to `main`. The integration test
uses a hyprctl shell stub on `$PATH`, so it passes on runners without
Hyprland.

## License

AGPL-3.0
