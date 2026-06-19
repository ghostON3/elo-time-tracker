//! Daemon loop — wires hyprctl polling → aggregator → SQLite store.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, info, warn};

use crate::aggregator::Aggregator;
use crate::hypr::query_active_window;
use crate::store::Store;

pub async fn run(
    interval_secs: u64,
    flush_every_secs: u64,
    idle_gap_secs: u64,
    stale_after_secs: i64,
    db_path: PathBuf,
) -> Result<()> {
    info!(
        interval_secs,
        flush_every_secs,
        idle_gap_secs,
        stale_after_secs,
        db = %db_path.display(),
        "elo-time-tracker daemon starting"
    );
    let store = Store::open(&db_path)?;
    let mut agg = Aggregator::new(
        interval_secs as i64,
        flush_every_secs as i64,
        idle_gap_secs as i64,
    );

    // Liveness self-watch: a live process is NOT proof of live data. Warn loudly
    // when no event has been persisted for longer than the staleness threshold
    // (clamped to at least two flush windows so a normal single-window session
    // does not false-alarm). This is what would have screamed during the 13h
    // alive-but-dark stall instead of grinning green.
    let stale_warn_secs = stale_after_secs.max((flush_every_secs as i64) * 2 + 1);
    let mut last_insert_at = Utc::now();
    let mut last_warn_at: Option<chrono::DateTime<Utc>> = None;

    let mut ticker = interval(Duration::from_secs(interval_secs.max(1)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = Utc::now();
                match query_active_window().await {
                    Ok(w) => {
                        // `{}` from hyprctl is a real "no focused window" → flush.
                        let win_opt = if w.is_empty() { None } else { Some(w) };
                        let outcome = agg.tick(win_opt.as_ref(), now);
                        for ev in outcome.flushed {
                            match store.insert(&ev) {
                                Ok(id) => {
                                    last_insert_at = now;
                                    last_warn_at = None;
                                    debug!(id, app=%ev.app_class, secs=ev.duration_secs, "flushed event");
                                }
                                Err(e) => warn!(error=%e, "sqlite insert failed"),
                            }
                        }
                    }
                    // A transient hyprctl failure is NOT "no window" — do not
                    // flush the open event; just skip this poll. The dark-watch
                    // below still runs, so a PERSISTENT failure surfaces.
                    Err(e) => warn!(error=%e, "hyprctl query failed; skipping tick"),
                }
                // Scream if we have gone dark.
                let dark_for = (now - last_insert_at).num_seconds();
                if dark_for > stale_warn_secs {
                    let warn_due = last_warn_at
                        .map(|w| (now - w).num_seconds() > stale_warn_secs)
                        .unwrap_or(true);
                    if warn_due {
                        warn!(
                            dark_for_secs = dark_for,
                            threshold_secs = stale_warn_secs,
                            "elo-time-tracker ALIVE BUT DARK — no event persisted recently; capture may be broken"
                        );
                        last_warn_at = Some(now);
                    }
                }
            }
            _ = sigint.recv() => {
                info!("SIGINT received, flushing and exiting");
                if let Some(ev) = agg.flush() {
                    let _ = store.insert(&ev);
                }
                return Ok(());
            }
            _ = sigterm.recv() => {
                info!("SIGTERM received, flushing and exiting");
                if let Some(ev) = agg.flush() {
                    let _ = store.insert(&ev);
                }
                return Ok(());
            }
        }
    }
}
