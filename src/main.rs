//! elo-time-tracker — Hyprland focus time daemon.
//!
//! Polls the active window via `hyprctl activewindow -j`, aggregates consecutive
//! polls of the same window into single events, and flushes to SQLite when the
//! active window changes OR every `flush_every_secs` (whichever first).

use anyhow::Result;
use clap::Parser;

mod aggregator;
mod cli;
mod daemon;
mod hypr;
mod report;
mod store;

use cli::{Cli, Mode};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    cli.init_tracing();

    let db_path = cli.resolved_db_path()?;

    match cli.mode() {
        Mode::Report { since } => {
            let store = store::Store::open(&db_path)?;
            report::print_report(&store, since.as_deref(), cli.implausible_day_hours)?;
        }
        Mode::ExportJson => {
            let store = store::Store::open(&db_path)?;
            report::export_json(&store)?;
        }
        Mode::Health => {
            let store = store::Store::open(&db_path)?;
            let healthy = report::print_health(&store, cli.stale_after_secs)?;
            if !healthy {
                std::process::exit(2);
            }
        }
        Mode::Daemon => {
            daemon::run(
                cli.interval_secs,
                cli.flush_every_secs,
                cli.idle_gap_secs,
                cli.stale_after_secs,
                db_path,
            )
            .await?;
        }
    }
    Ok(())
}
