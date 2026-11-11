//! Without a switch, the built-in base is `~/.openidea`; this binary owns its
//! process (HOME is redirected to a temp dir, which `dirs` resolves through).
//! Windows resolves HOME through the known-folder API instead, so the test is
//! Unix-only; the Windows-shaped migration is covered by `paths.rs` unit tests.
#![cfg(unix)]

#[test]
fn default_resolution_is_the_built_in_base() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: sole test in this binary; no other thread reads the environment.
    unsafe { std::env::set_var("HOME", &home) };

    openidea_lib::paths::init("ai.l0ng.openidea");
    let resolved = openidea_lib::paths::current();
    let base = home.join(".openidea");

    assert!(resolved.is_default());
    assert_eq!(resolved.base(), base.as_path());
    assert_eq!(resolved.locations.config, base.join("config"));
    assert_eq!(resolved.locations.data, base.join("system"));
    assert_eq!(openidea_lib::paths::plugins_dir(&resolved.layout, &resolved.locations), base.join("config"));
    assert_eq!(openidea_lib::paths::tools_home(&resolved.layout, &resolved.defaults), Some(base.clone()));
    assert_eq!(
        openidea_lib::paths::login_cache_root(&resolved.layout, &resolved.locations, "ai.l0ng.openidea"),
        base.join("system")
    );
    assert_eq!(openidea_lib::paths::log_dir(&resolved.layout, "ai.l0ng.openidea"), Some(base.join("log")));
}
