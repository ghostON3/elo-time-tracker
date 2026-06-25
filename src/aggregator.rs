//! Collapses consecutive polls of the same window into aggregated events.
//!
//! The aggregator owns one "open" event in memory; each tick either extends
//! it (same window) or flushes + opens a new one (window changed). A forced
//! flush is also triggered when the open event has grown past `max_dur_secs`.

use chrono::{DateTime, Utc};

use crate::hypr::ActiveWindow;
use crate::store::Event;

#[derive(Debug, Clone)]
pub struct OpenEvent {
    pub app_class: String,
    pub window_title: String,
    pub monitor: String,
    pub started_at: DateTime<Utc>,
    pub duration_secs: i64,
}

impl OpenEvent {
    fn from(w: &ActiveWindow, now: DateTime<Utc>, first_tick_secs: i64) -> Self {
        Self {
            app_class: w.class.clone(),
            window_title: w.title.clone(),
            monitor: w.monitor_string(),
            started_at: now,
            duration_secs: first_tick_secs,
        }
    }

    pub fn matches(&self, w: &ActiveWindow) -> bool {
        self.app_class == w.class
            && self.window_title == w.title
            && self.monitor == w.monitor_string()
    }

    pub fn into_event(self) -> Event {
        Event {
            id: None,
            app_class: self.app_class,
            window_title: self.window_title,
            monitor: self.monitor,
            started_at: self.started_at.to_rfc3339(),
            duration_secs: self.duration_secs,
        }
    }
}

pub struct Aggregator {
    open: Option<OpenEvent>,
    /// Each poll adds this many seconds to the open event's duration.
    tick_secs: i64,
    /// When a single open event reaches this many seconds, flush it.
    max_dur_secs: i64,
}

#[derive(Debug, Default)]
pub struct TickOutcome {
    /// Event(s) finalised this tick.
    pub flushed: Vec<Event>,
}

impl Aggregator {
    pub fn new(tick_secs: i64, max_dur_secs: i64) -> Self {
        Self {
            open: None,
            tick_secs: tick_secs.max(1),
            max_dur_secs: max_dur_secs.max(1),
        }
    }

    /// Feed one poll. Returns any events that were finalised this tick.
    ///
    /// `window=None` represents "no focused window" (hyprctl returned `{}`):
    /// we flush the open event but do not start a new one.
    pub fn tick(&mut self, window: Option<&ActiveWindow>, now: DateTime<Utc>) -> TickOutcome {
        let mut outcome = TickOutcome::default();

        let Some(w) = window else {
            if let Some(open) = self.open.take() {
                outcome.flushed.push(open.into_event());
            }
            return outcome;
        };

        match self.open.as_mut() {
            Some(open) if open.matches(w) => {
                open.duration_secs += self.tick_secs;
                if open.duration_secs >= self.max_dur_secs {
                    // forced flush; start a fresh open window
                    let finished = self.open.take().unwrap().into_event();
                    outcome.flushed.push(finished);
                    self.open = Some(OpenEvent::from(w, now, self.tick_secs));
                }
            }
            Some(_) => {
                let finished = self.open.take().unwrap().into_event();
                outcome.flushed.push(finished);
                self.open = Some(OpenEvent::from(w, now, self.tick_secs));
            }
            None => {
                self.open = Some(OpenEvent::from(w, now, self.tick_secs));
            }
        }

        outcome
    }

    /// Flush whatever is currently open, e.g. on shutdown.
    pub fn flush(&mut self) -> Option<Event> {
        self.open.take().map(|o| o.into_event())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(class: &str, title: &str) -> ActiveWindow {
        ActiveWindow {
            class: class.into(),
            title: title.into(),
            monitor: serde_json::Value::Number(0.into()),
        }
    }

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn collapses_consecutive_same_window() {
        let mut agg = Aggregator::new(5, 300);
        let kitty = w("kitty", "tmux");
        let now = t("2026-05-16T10:00:00Z");

        let o1 = agg.tick(Some(&kitty), now);
        let o2 = agg.tick(Some(&kitty), now);
        let o3 = agg.tick(Some(&kitty), now);

        assert!(o1.flushed.is_empty());
        assert!(o2.flushed.is_empty());
        assert!(o3.flushed.is_empty());

        let final_ev = agg.flush().unwrap();
        assert_eq!(final_ev.app_class, "kitty");
        assert_eq!(final_ev.duration_secs, 15);
    }

    #[test]
    fn flushes_on_window_change() {
        let mut agg = Aggregator::new(5, 300);
        let kitty = w("kitty", "tmux");
        let firefox = w("firefox", "GitHub");

        let _ = agg.tick(Some(&kitty), t("2026-05-16T10:00:00Z"));
        let _ = agg.tick(Some(&kitty), t("2026-05-16T10:00:05Z"));
        let out = agg.tick(Some(&firefox), t("2026-05-16T10:00:10Z"));

        assert_eq!(out.flushed.len(), 1);
        assert_eq!(out.flushed[0].app_class, "kitty");
        assert_eq!(out.flushed[0].duration_secs, 10);

        // firefox is now open with 5s
        let last = agg.flush().unwrap();
        assert_eq!(last.app_class, "firefox");
        assert_eq!(last.duration_secs, 5);
    }

    #[test]
    fn forced_flush_at_max_duration() {
        // tick 5s, max 10s → after the 2nd tick (10s) we force-flush and open
        // a fresh event covering the same tick.
        let mut agg = Aggregator::new(5, 10);
        let kitty = w("kitty", "tmux");

        let o1 = agg.tick(Some(&kitty), t("2026-05-16T10:00:00Z"));
        let o2 = agg.tick(Some(&kitty), t("2026-05-16T10:00:05Z"));

        assert!(o1.flushed.is_empty());
        assert_eq!(o2.flushed.len(), 1);
        assert_eq!(o2.flushed[0].duration_secs, 10);

        // a fresh open event was started for the same window
        let leftover = agg.flush().unwrap();
        assert_eq!(leftover.app_class, "kitty");
        assert_eq!(leftover.duration_secs, 5);
    }

    #[test]
    fn empty_window_flushes_open() {
        let mut agg = Aggregator::new(5, 300);
        let kitty = w("kitty", "tmux");
        let _ = agg.tick(Some(&kitty), t("2026-05-16T10:00:00Z"));
        let out = agg.tick(None, t("2026-05-16T10:00:05Z"));
        assert_eq!(out.flushed.len(), 1);
        assert_eq!(out.flushed[0].app_class, "kitty");
        assert!(agg.flush().is_none());
    }

    #[test]
    fn title_change_breaks_aggregation() {
        let mut agg = Aggregator::new(5, 300);
        let a = w("firefox", "GitHub");
        let b = w("firefox", "Hacker News");
        let _ = agg.tick(Some(&a), t("2026-05-16T10:00:00Z"));
        let out = agg.tick(Some(&b), t("2026-05-16T10:00:05Z"));
        assert_eq!(out.flushed.len(), 1);
        assert_eq!(out.flushed[0].window_title, "GitHub");
    }
}
