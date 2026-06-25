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

pub async fn run(interval_secs: u64, flush_every_secs: u64, db_path: PathBuf) -> Result<()> {
    info!(
        interval_secs,
        flush_every_secs,
        db = %db_path.display(),
        "elo-time-tracker daemon starting"
    );
    let store = Store::open(&db_path)?;
    let mut agg = Aggregator::new(interval_secs as i64, flush_every_secs as i64);

    let mut ticker = interval(Duration::from_secs(interval_secs.max(1)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = Utc::now();
                let win_res = query_active_window().await;
                let win_opt = match win_res {
                    Ok(w) if w.is_empty() => None,
                    Ok(w) => Some(w),
                    Err(e) => {
                        warn!(error=%e, "hyprctl query failed; skipping tick");
                        continue;
                    }
                };
                let outcome = agg.tick(win_opt.as_ref(), now);
                for ev in outcome.flushed {
                    match store.insert(&ev) {
                        Ok(id) => debug!(id, app=%ev.app_class, secs=ev.duration_secs, "flushed event"),
                        Err(e) => warn!(error=%e, "sqlite insert failed"),
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
