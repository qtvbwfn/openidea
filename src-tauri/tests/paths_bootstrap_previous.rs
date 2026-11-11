//! A pointer that already records a previous location means the user decided
//! about that move: the startup bootstrap must leave the dialog flow alone.

#![cfg(unix)]

#[test]
fn a_recorded_previous_location_blocks_the_bootstrap() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: sole test in this binary; no other thread reads the environment.
    unsafe { std::env::set_var("HOME", &home) };

    let defaults = openidea_lib::paths::default_dirs("ai.l0ng.openidea");
    std::fs::create_dir_all(&defaults.config).unwrap();
    std::fs::write(defaults.config.join("settings.toml"), "theme = \"dark\"").unwrap();
    openidea_lib::paths::write_pointer(
        &defaults.config,
        &openidea_lib::paths::Layout::Default,
        Some(&openidea_lib::paths::Layout::Custom(tmp.path().join("old-base"))),
    )
    .unwrap();

    openidea_lib::paths::init("ai.l0ng.openidea");
    openidea_lib::paths::bootstrap_legacy_migrate();

    assert!(defaults.config.join("settings.toml").is_file(), "the legacy data stays put");
    assert!(!home.join(".openidea/config/settings.toml").exists());
}
