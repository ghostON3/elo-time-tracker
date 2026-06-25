//! Command-line surface.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "elo-time-tracker",
    version,
    about = "Per-app focus time tracker via Hyprland IPC"
)]
pub struct Cli {
    /// Poll interval in seconds (daemon mode).
    #[arg(long, default_value_t = 5)]
    pub interval_secs: u64,

    /// Maximum seconds a single aggregated event may grow before forced flush.
    #[arg(long, default_value_t = 300)]
    pub flush_every_secs: u64,

    /// SQLite file path. Defaults to $XDG_DATA_HOME/elo-time-tracker/log.db
    /// (or ~/.local/share/elo-time-tracker/log.db).
    #[arg(long)]
    pub db: Option<PathBuf>,

    /// Print a summary report of recorded events.
    #[arg(long)]
    pub report: bool,

    /// Free-form time window for --report (e.g. "1 hour ago", "24 hours ago",
    /// "7 days ago"). Defaults to "24 hours ago".
    #[arg(long)]
    pub since: Option<String>,

    /// Dump all events as JSON on stdout.
    #[arg(long)]
    pub export_json: bool,
}

#[derive(Debug)]
pub enum Mode {
    Daemon,
    Report { since: Option<String> },
    ExportJson,
}

impl Cli {
    pub fn mode(&self) -> Mode {
        if self.export_json {
            Mode::ExportJson
        } else if self.report {
            Mode::Report {
                since: self.since.clone(),
            }
        } else {
            Mode::Daemon
        }
    }

    pub fn init_tracing(&self) {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
    }

    pub fn resolved_db_path(&self) -> Result<PathBuf> {
        if let Some(p) = &self.db {
            if let Some(parent) = p.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create db parent dir {}", parent.display()))?;
                }
            }
            return Ok(p.clone());
        }
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .context("HOME unset")?;
        let dir = base.join("elo-time-tracker");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create data dir {}", dir.display()))?;
        Ok(dir.join("log.db"))
    }
}
