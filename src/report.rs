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

pub fn print_report(store: &Store, since: Option<&str>, implausible_day_hours: i64) -> Result<()> {
    let from = parse_since(since)?;
    let events = store.events_since(from)?;
    let totals = aggregate_by_app(&events);

    println!("elo-time-tracker — events since {}", from.to_rfc3339());
    // Honesty header: say exactly what this measures. This host has no
    // input-idle sensor, so the number is focused-window WALL time, not
    // verified-active time.
    println!(
        "metric: focused-window wall time (NOT idle-adjusted — no input-idle sensor on this host)"
    );
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

    // Flag any day whose focused-window total exceeds the plausibility ceiling —
    // it almost certainly includes away/idle time the host cannot detect.
    let flagged = days_exceeding(&events, implausible_day_hours * 3600);
    if !flagged.is_empty() {
        println!();
        println!(
            "⚠ {} day(s) exceed {}h focused-window time — likely include away/idle time (no input-idle sensor):",
            flagged.len(),
            implausible_day_hours
        );
        for (day, secs) in &flagged {
            println!("    {day}: {}", format_duration(*secs));
        }
    }
    Ok(())
}

/// Report capture liveness: is the daemon recording, or alive but dark?
/// Returns `true` when fresh, `false` when STALE or no data yet.
pub fn print_health(store: &Store, stale_after_secs: i64) -> Result<bool> {
    match store.latest_started_at()? {
        None => {
            println!("elo-time-tracker health: UNKNOWN — no events recorded yet");
            Ok(false)
        }
        Some(ts) => {
            let age = (Utc::now() - ts).num_seconds().max(0);
            let stale = age > stale_after_secs;
            println!(
                "elo-time-tracker health: {}",
                if stale { "STALE" } else { "OK" }
            );
            println!(
                "  latest event: {} ({} ago)",
                ts.to_rfc3339(),
                format_duration(age)
            );
            println!("  threshold:    {}", format_duration(stale_after_secs));
            if stale {
                println!("  ⚠ daemon may be alive but NOT capturing — a live process is not proof of live data. Investigate hyprctl / restart.");
            }
            Ok(!stale)
        }
    }
}

/// Total focused-window seconds per calendar day (UTC date of `started_at`),
/// keeping only days at/over `threshold_secs`.
pub fn days_exceeding(events: &[Event], threshold_secs: i64) -> Vec<(String, i64)> {
    let mut per_day: BTreeMap<String, i64> = BTreeMap::new();
    for ev in events {
        let day = ev.started_at.get(0..10).unwrap_or("").to_string();
        *per_day.entry(day).or_insert(0) += ev.duration_secs;
    }
    per_day
        .into_iter()
        .filter(|(_, secs)| *secs >= threshold_secs)
        .collect()
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
    fn days_exceeding_flags_only_over_threshold() {
        let mut a = mk("kitty", 17 * 3600); // 17h focused on day A
        a.started_at = "2026-06-18T02:00:00Z".into();
        let mut b = mk("kitty", 3 * 3600); // 3h on day B
        b.started_at = "2026-06-17T10:00:00Z".into();
        let flagged = days_exceeding(&[a, b], 16 * 3600);
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].0, "2026-06-18");
        assert_eq!(flagged[0].1, 17 * 3600);
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
