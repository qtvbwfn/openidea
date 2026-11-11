# Fork-only: the configuration directory

Upstream closed #1297 (its #1488 decision keeps settings under the OS and only
migrates rebuildable data through `OPENIDEA_LOCAL_DATA_DIR`). This fork keeps
the feature: everything the app owns lives under one base, `~/.openidea`, with a
settings row to choose another base and a one-time move of a legacy install.

## Where things live

```
~/.openidea/
  config/   settings.toml, languages/, recent.json, recent-groups.json,
            window-groups.json, plugins/ (installed plugins and plugins/.data)
  system/   credentials, history, caches, lsp/, agents/, workspaces/, remote/,
            server/, http-history/, consoles/, scratches/, project-trust.json
  log/
  tools/
  language-server-runtimes/
  worktrees/   agent task worktrees (not this feature's; never migrated or trashed)
  data/        a remote server's base, when one runs on this machine (same)
```

Development builds use `~/.openidea/dev/w<hash>` (the dev identifier carries the
worktree hash, so worktrees stay isolated). `OPENIDEA_HOME` overrides the base.

## Resolution

```
OPENIDEA_HOME > pointer <platform config>/<identifier>/home.txt > the built-in base
```

The pointer anchor stays in the platform config dir, so existing pointers and
`.skip-migration` markers keep working. Line 1 is `default` (the built-in base)
or an absolute path. Line 2 (previous) is `default`/`platform` (the legacy
platform dirs) or an absolute path.

## Upgrading a legacy install

On the first start after this change, a pure legacy install (no `OPENIDEA_HOME`,
no chosen base, no recorded previous location) is moved into the built-in base
before settings are read. It is best effort: a partial move leaves
`.migration-incomplete` in the target's config side, and the next start retries
with `replace_existing`, which only touches entries whose source still exists.
The old platform directories are left in place (window state, logs, unknown
files; the moved entries are gone) and the pointer is untouched. A switch made
through Settings still uses the migration dialog.

Windows: the legacy layout split rebuildable data between `%APPDATA%` and
`%LOCALAPPDATA%`; the migration scans both (`ide_host::data_dirs::is_local` picks
the preferred root, the other is the fallback). The default base is
`C:\Users\<user>\.openidea`.

## What moves

`MOVES` and `DROPS` in `src-tauri/src/paths.rs` are the table. Rebuildable
caches (`index`, `cache`) drop. Logs and `.window-state.json` stay
platform-keyed. The embedded `java-usages`/`kotlin-bridge` jars self-heal.
`terminal-images`, `container-debug` and `ruby-reporter` are disposable. The
remote server's `plugin-pins.json` and `remote-plugins` belong to its own
`~/.openidea/data`, not to this layout.

## After an upstream sync

`src-tauri/tests/paths_guard.rs` fails when the shell reads a platform app
directory directly. Every sync:

1. `cargo nextest run -p openidea --test paths_guard`;
2. route each hit through `paths::*`, or add a justified allowlist entry;
3. review `state.rs` (`HostConfig`) and `lib.rs` (`paths::init`) for new
   directory fields;
4. grep new stores (`config_dir.join("…")`, `data_dir.join("…")`) and add them
   to `MOVES` when they hold user data.

## Known limits

- Windows has not been verified through a GUI here; the path logic runs in CI on
  `windows-latest`.
- Uninstallers do not know the base: uninstalling leaves `~/.openidea` behind.
- A chosen base should be a new or empty directory. A switch trashes only
  `config/`, `system/`, `log/`, `tools/` and `language-server-runtimes/`.
- Rolling back to a version before this change is not supported.
- `.openidea` is not hidden on Windows.
