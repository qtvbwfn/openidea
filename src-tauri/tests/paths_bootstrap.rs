//! A legacy platform install moves into the built-in base on the first start;
//! HOME is redirected to a temp dir (Unix only), so the process owns its
//! directories. One scenario per binary: `paths::init` resolves only once.

#![cfg(unix)]

#[test]
fn a_legacy_install_is_moved_into_the_built_in_base() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: sole test in this binary; no other thread reads the environment.
    unsafe { std::env::set_var("HOME", &home) };

    let defaults = openidea_lib::paths::default_dirs("ai.l0ng.openidea");
    std::fs::create_dir_all(&defaults.config).unwrap();
    std::fs::write(defaults.config.join("settings.toml"), "theme = \"dark\"").unwrap();
    std::fs::create_dir_all(defaults.data.join("lsp").join("jdtls")).unwrap();
    std::fs::write(defaults.data.join("lsp").join("jdtls").join("marker"), "x").unwrap();

    openidea_lib::paths::init("ai.l0ng.openidea");
    openidea_lib::paths::bootstrap_legacy_migrate();

    let base = home.join(".openidea");
    assert_eq!(std::fs::read_to_string(base.join("config/settings.toml")).unwrap(), "theme = \"dark\"");
    assert!(base.join("system/lsp/jdtls/marker").is_file());
    // The moved entries leave the old tree; the directories themselves stay
    // for the window state and logs.
    assert!(!defaults.config.join("settings.toml").exists());
    assert!(!defaults.data.join("lsp").exists());
    assert!(defaults.config.is_dir());
}
