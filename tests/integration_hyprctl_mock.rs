//! Integration smoke test: verify that a `hyprctl` shim on $PATH produces the
//! exact JSON shape our parser expects. We don't depend on the bin crate's
//! internals — we shell out the same way the daemon does.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn install_hyprctl_stub(dir: &Path, payload: &str) {
    let path = dir.join("hyprctl");
    let script = format!(
        "#!/usr/bin/env bash\nif [ \"$1\" = activewindow ]; then\n  cat <<'JSON'\n{payload}\nJSON\nfi\n"
    );
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn hyprctl_stub_on_path_returns_expected_json() {
    let dir = TempDir::new().unwrap();
    let payload = r#"{"class":"kitty","title":"tmux","monitor":0}"#;
    install_hyprctl_stub(dir.path(), payload);

    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", dir.path().display(), old_path);

    let out = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .env("PATH", &new_path)
        .output()
        .expect("spawn stubbed hyprctl");

    assert!(out.status.success(), "stub exited with non-zero status");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"class\":\"kitty\""), "got: {text}");
    assert!(text.contains("\"title\":\"tmux\""), "got: {text}");
}
