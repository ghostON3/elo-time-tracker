//! Hyprland active-window query + JSON parsing.

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::process::Command;

/// Subset of `hyprctl activewindow -j` we care about.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct ActiveWindow {
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub monitor: serde_json::Value,
}

impl ActiveWindow {
    pub fn monitor_string(&self) -> String {
        match &self.monitor {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        }
    }

    /// True when hyprctl returns the empty `{}` (no focused window).
    pub fn is_empty(&self) -> bool {
        self.class.is_empty() && self.title.is_empty()
    }
}

/// Parse the JSON `hyprctl activewindow -j` emits.
pub fn parse_active_window(json: &str) -> Result<ActiveWindow> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Ok(ActiveWindow {
            class: String::new(),
            title: String::new(),
            monitor: serde_json::Value::Null,
        });
    }
    serde_json::from_str(trimmed).with_context(|| format!("parse activewindow json: {trimmed}"))
}

/// Spawn `hyprctl activewindow -j` once.
pub async fn query_active_window() -> Result<ActiveWindow> {
    let out = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .await
        .context("spawn hyprctl")?;
    if !out.status.success() {
        anyhow::bail!(
            "hyprctl failed: status={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    parse_active_window(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_active_window() {
        let j = r#"{"class":"kitty","title":"tmux","monitor":0}"#;
        let w = parse_active_window(j).unwrap();
        assert_eq!(w.class, "kitty");
        assert_eq!(w.title, "tmux");
        assert_eq!(w.monitor_string(), "0");
        assert!(!w.is_empty());
    }

    #[test]
    fn parses_empty_object() {
        let w = parse_active_window("{}").unwrap();
        assert!(w.is_empty());
        assert_eq!(w.monitor_string(), "");
    }

    #[test]
    fn parses_real_hyprctl_sample() {
        // shape captured from a live Hyprland session
        let j = r#"{
            "address":"0x562c1c7057c0","mapped":true,"hidden":false,
            "at":[22,22],"size":[1876,1036],
            "workspace":{"id":2,"name":"2"},
            "floating":false,"monitor":0,
            "class":"firefox","title":"Mozilla — GitHub",
            "pid":1234,"xwayland":false
        }"#;
        let w = parse_active_window(j).unwrap();
        assert_eq!(w.class, "firefox");
        assert_eq!(w.title, "Mozilla — GitHub");
        assert_eq!(w.monitor_string(), "0");
    }

    #[test]
    fn monitor_may_be_string() {
        let j = r#"{"class":"a","title":"b","monitor":"DP-1"}"#;
        let w = parse_active_window(j).unwrap();
        assert_eq!(w.monitor_string(), "DP-1");
    }
}
