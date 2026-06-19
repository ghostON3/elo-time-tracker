# elo-time-tracker

A lean Rust daemon that tracks per-app focus time on Hyprland. Every N seconds
it queries `hyprctl activewindow`, collapses consecutive polls of the same window
into a single aggregated event, and writes to SQLite when the focused window
changes or a flush interval fires. The result is an accurate, low-overhead record
of exactly where your attention went.

- **Target:** Arch Linux + Hyprland (Wayland)
- **Storage:** SQLite at `$XDG_DATA_HOME/elo-time-tracker/log.db`
  (fallback: `~/.local/share/elo-time-tracker/log.db`)
- **Footprint:** single static binary, < 20 MB RSS, < 1 % of one core at
  default 5 s polling
- **Pairs with:** [ghost-dotfiles](https://github.com/ghostON3/ghost-dotfiles)
  — Hyprland config, waybar, and the wider rice that this daemon feeds

## Why Rust for this?

A focus tracker runs all day, every day. It must not leak memory, must not spike
CPU, and must survive 10-day uptimes without a restart. Rust gives us:

- **Zero GC pauses** — no stop-the-world events between polls
- **Deterministic memory** — the event loop allocates once; steady state is flat
- **Async I/O without a runtime tax** — `tokio::select!` handles the ticker and
  SIGTERM/SIGINT in ~50 lines with no threads
- **`rusqlite` bundled build** — no system sqlite dependency, the binary is
  genuinely self-contained

The alternative (Python cron + sqlite3) worked but used ~30 MB RSS and jittered
badly under load. This uses ~8 MB and is timing-stable.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                  tokio event loop                    │
│                                                      │
│   ┌─────────┐   tick every N s   ┌────────────┐     │
│   │  Timer  │ ─────────────────► │  hyprctl   │     │
│   └─────────┘                    │ activewindow│     │
│                                  └─────┬──────┘     │
│                                        │ JSON        │
│                                  ┌─────▼──────┐     │
│                                  │ Aggregator │     │
│                                  │ (in-memory)│     │
│                                  └─────┬──────┘     │
│              window changed / max_dur  │             │
│                                  ┌─────▼──────┐     │
│   SIGINT/SIGTERM ──────────────► │   Store    │     │
│                  flush + exit    │  (SQLite)  │     │
│                                  └────────────┘     │
└──────────────────────────────────────────────────────┘
```

**Key modules:**

| Module | Role |
|---|---|
| `hypr.rs` | Spawns `hyprctl activewindow -j`, parses JSON into `ActiveWindow` |
| `aggregator.rs` | Collapses consecutive same-window polls; owns the single in-memory open event |
| `store.rs` | SQLite schema + insert/query via `rusqlite` |
| `daemon.rs` | `tokio::select!` loop wiring ticker → aggregator → store + signal handling |
| `report.rs` | `--report` summary table and `--export-json` read paths |
| `cli.rs` | `clap` surface; resolves XDG db path |

**Aggregation rule:** each poll adds `interval_secs` to the open event's
duration. A row is written to SQLite when either:

1. the active window changes (`app_class`, `window_title`, or `monitor` differs), or
2. the open event's duration reaches `--flush-every-secs` (default 5 min).

On SIGINT / SIGTERM the open event is flushed before exit — no data lost on
controlled shutdown.

## Install

```bash
git clone https://github.com/ghostON3/elo-time-tracker
cd elo-time-tracker
cargo build --release
# copy binary to somewhere on $PATH
install -Dm755 target/release/elo-time-tracker ~/.local/bin/elo-time-tracker
```

Or install via cargo directly:

```bash
cargo install --path .
```

Requires: Rust stable 1.78+ (see `rust-toolchain.toml`), Hyprland running.

## Run as a foreground daemon

```bash
elo-time-tracker --interval-secs 5
```

Useful flags:

| Flag | Default | Meaning |
|---|---|---|
| `--interval-secs N` | 5 | Poll cadence (seconds) |
| `--flush-every-secs N` | 300 | Force-flush an open event at this duration |
| `--db PATH` | `$XDG_DATA_HOME/elo-time-tracker/log.db` | SQLite file |
| `--report` | – | Print summary instead of running the daemon |
| `--since "N hours ago"` | "24 hours ago" | Time window for `--report` |
| `--export-json` | – | Dump all events as newline-delimited JSON on stdout |

Set `RUST_LOG=debug` for verbose per-event output.

## Run as a systemd user service

```bash
mkdir -p ~/.config/systemd/user
cp systemd/elo-time-tracker.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now elo-time-tracker.service
journalctl --user -u elo-time-tracker.service -f
```

The unit waits for `graphical-session.target` so it starts only after Hyprland
and stops cleanly when the session ends (flushing the open event on SIGTERM).

## Sample report

```
$ elo-time-tracker --report --since "2 hours ago"
elo-time-tracker — events since 2026-05-16T13:00:00+00:00
14 event(s)

APP                                DURATION  BAR
----------------------------------------------------------------------
firefox                            1h12m30s  ##############################
kitty                                42m15s  #################
code                                 18m05s  #######
slack                                 4m20s  ##

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

## Querying the database directly

The SQLite file at `$XDG_DATA_HOME/elo-time-tracker/log.db` is plain sqlite3 —
query it with any tool.

**Total focus time per app today:**
```sql
SELECT app_class,
       printf('%dh %02dm', SUM(duration_secs)/3600, (SUM(duration_secs)%3600)/60) AS total
FROM events
WHERE started_at >= date('now')
GROUP BY app_class
ORDER BY SUM(duration_secs) DESC;
```

**Top 5 most-visited window titles this week:**
```sql
SELECT window_title, COUNT(*) AS sessions, SUM(duration_secs)/60 AS total_min
FROM events
WHERE started_at >= date('now', '-7 days')
GROUP BY window_title
ORDER BY total_min DESC
LIMIT 5;
```

**Hourly breakdown (useful for time-block analysis):**
```sql
SELECT strftime('%H:00', started_at) AS hour,
       SUM(duration_secs)/60         AS total_min
FROM events
WHERE started_at >= date('now')
GROUP BY hour
ORDER BY hour;
```

## Schema

```sql
CREATE TABLE events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  app_class     TEXT NOT NULL,     -- e.g. "firefox", "kitty"
  window_title  TEXT NOT NULL,     -- window title at focus time
  monitor       TEXT NOT NULL,     -- monitor id (0, 1, "DP-1", …)
  started_at    TEXT NOT NULL,     -- RFC 3339 UTC, e.g. "2026-05-16T13:42:08+00:00"
  duration_secs INTEGER NOT NULL   -- seconds this window was focused
);

CREATE INDEX idx_events_started_at ON events(started_at);
CREATE INDEX idx_events_app_class  ON events(app_class);
```

## Development

```bash
cargo fmt --all
cargo check --all-targets
cargo test --all
```

CI runs the same three steps on every push to `main`. The integration test
(`tests/integration_hyprctl_mock.rs`) installs a `hyprctl` shell stub on `$PATH`
so the suite passes on runners without a live Hyprland session.

## Pairing with ghost-dotfiles

This daemon feeds the time-block view in the
[ghost-dotfiles](https://github.com/ghostON3/ghost-dotfiles) cockpit. The
dotfiles repo ships the Hyprland config, waybar widgets, and the scripts that
query `log.db` and render focus heat-maps in the status bar. Run both repos
together for the full picture.

## License

AGPL-3.0 — see [LICENSE](LICENSE).
