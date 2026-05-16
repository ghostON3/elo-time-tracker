//! Read-side: summary report + JSON export.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};

use crate::store::{Event, Store};

/// Parse free-form `--since` strings like "1 hour ago", "24 hours ago",
/// "7 days ago", "30 minutes ago". Falls back to 24 hours ago.
pub fn parse_since(input: Option<&str>) -> Result<DateTime<Utc>> {
    let raw = input.unwrap_or("24 hours ago").trim().to_lowercase();
    let stripped = raw.strip_suffix(" ago").unwrap_or(&raw).trim();
    let mut parts = stripped.split_whitespace();
    let n: i64 = parts
        .next()
        .context("missing duration number")?
        .parse()
        .context("duration number")?;
    let unit = parts.next().unwrap_or("hours");
    let delta = match unit.trim_end_matches('s') {
        "second" | "sec" => Duration::seconds(n),
        "minute" | "min" => Duration::minutes(n),
        "hour" | "hr" | "h" => Duration::hours(n),
        "day" | "d" => Duration::days(n),
        "week" | "wk" | "w" => Duration::weeks(n),
        other => anyhow::bail!("unknown time unit: {other}"),
    };
    Ok(Utc::now() - delta)
}

pub fn print_report(store: &Store, since: Option<&str>) -> Result<()> {
    let from = parse_since(since)?;
    let events = store.events_since(from)?;
    let totals = aggregate_by_app(&events);

    println!("elo-time-tracker — events since {}", from.to_rfc3339());
    println!("{} event(s)\n", events.len());
    if totals.is_empty() {
        println!("(no data in window)");
        return Ok(());
    }
    println!("{:<32} {:>10}  {}", "APP", "DURATION", "BAR");
    println!("{}", "-".repeat(70));
    let max_secs = totals.values().copied().max().unwrap_or(1);
    let mut rows: Vec<(&String, &i64)> = totals.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (app, secs) in rows {
        let bar_len = ((*secs as f64 / max_secs as f64) * 30.0).round() as usize;
        let bar = "#".repeat(bar_len);
        println!(
            "{:<32} {:>10}  {}",
            truncate(app, 32),
            format_duration(*secs),
            bar
        );
    }
    println!();
    println!("total: {}", format_duration(totals.values().sum::<i64>()));
    Ok(())
}

pub fn export_json(store: &Store) -> Result<()> {
    let events = store.all_events()?;
    let json = serde_json::to_string_pretty(&events)?;
    println!("{json}");
    Ok(())
}

pub fn aggregate_by_app(events: &[Event]) -> BTreeMap<String, i64> {
    let mut out: BTreeMap<String, i64> = BTreeMap::new();
    for ev in events {
        *out.entry(ev.app_class.clone()).or_insert(0) += ev.duration_secs;
    }
    out
}

pub fn format_duration(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Event;

    fn mk(app: &str, secs: i64) -> Event {
        Event {
            id: None,
            app_class: app.into(),
            window_title: "t".into(),
            monitor: "0".into(),
            started_at: "2026-05-16T10:00:00Z".into(),
            duration_secs: secs,
        }
    }

    #[test]
    fn aggregate_sums_by_app() {
        let evs = vec![mk("kitty", 30), mk("firefox", 60), mk("kitty", 15)];
        let totals = aggregate_by_app(&evs);
        assert_eq!(*totals.get("kitty").unwrap(), 45);
        assert_eq!(*totals.get("firefox").unwrap(), 60);
    }

    #[test]
    fn format_duration_buckets() {
        assert_eq!(format_duration(5), "5s");
        assert_eq!(format_duration(65), "1m05s");
        assert_eq!(format_duration(3661), "1h01m01s");
    }

    #[test]
    fn parse_since_variants() {
        let now = Utc::now();
        let h1 = parse_since(Some("1 hour ago")).unwrap();
        assert!((now - h1).num_seconds() >= 3500);
        assert!((now - h1).num_seconds() <= 3700);
        let d7 = parse_since(Some("7 days ago")).unwrap();
        assert!((now - d7).num_days() == 7 || (now - d7).num_days() == 6);
        let m30 = parse_since(Some("30 minutes ago")).unwrap();
        assert!((now - m30).num_seconds() >= 1700);
        let default = parse_since(None).unwrap();
        assert!((now - default).num_hours() >= 23);
    }

    #[test]
    fn parse_since_rejects_garbage() {
        assert!(parse_since(Some("yesterday")).is_err());
    }
}
