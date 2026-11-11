//! The shell reads every app directory through `paths`: this test fails when a
//! new call site asks Tauri or `dirs` directly, which is what an upstream merge
//! would otherwise slip in unnoticed. Run it after every sync.

use std::fs;
use std::path::{Path, PathBuf};

/// Files allowed to name the platform dirs directly, with the reason.
const ALLOWED: &[(&str, &str)] = &[
    ("paths.rs", "the resolver itself"),
    ("logging.rs", "the Tauri app_log_dir equivalent for the legacy layout"),
    ("linux_window_state.rs", "the window-state plugin mirror; geometry stays platform-keyed"),
];

/// The direct lookups the shell must not use.
const FORBIDDEN: &[&str] = &[
    "app.path().app_config_dir",
    "app.path().app_data_dir",
    "app.path().app_local_data_dir",
    "app.path().app_cache_dir",
    "app.path().app_log_dir",
    "dirs::config_dir(",
    "dirs::data_dir(",
    "dirs::data_local_dir(",
    "dirs::cache_dir(",
];

#[test]
fn shell_directories_go_through_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let mut hits = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
        if ALLOWED.iter().any(|(allowed, _)| relative == *allowed) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else { continue };
        for (number, line) in text.lines().enumerate() {
            for forbidden in FORBIDDEN {
                if line.contains(forbidden) {
                    hits.push(format!("{relative}:{}: {forbidden}", number + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "route these through `paths` (or add a justified allowlist entry in this test):\n{}",
        hits.join("\n")
    );
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}
