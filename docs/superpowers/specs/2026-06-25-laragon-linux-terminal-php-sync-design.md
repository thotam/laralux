# Laragon Linux — Terminal PHP/composer Sync (Phase 2, Version slice 1c) Design Spec

**Date:** 2026-06-25
**Status:** Design approved, pending spec review
**Goal:** Make the shell `php` and `composer` use Laragon's **active** PHP version (not the distro PHP). Install the CLI php binary alongside php-fpm, expose the active version as `~/laragon/bin/php`, provide a `composer` that runs under it, and (opt-in) prepend `~/laragon/bin` to the user's shell PATH so a normal terminal picks them up. Switching the active version in the app transparently changes `php`/`composer`.

This is **Slice A** of the "terminal integration" work. **Slice B — an "Open terminal" button** (launch a terminal emulator with the PATH already set, optionally in a site directory) is tracked separately and out of scope here, because terminal-emulator detection/launch is environment-specific and not unit-testable.

---

## 1. Context & current state

Today only **php-fpm** is installed (static `bulk` build into `~/laragon/bin/php-fpm<minor>`), used to serve sites via nginx. The shell `php` and `composer` are the **distro** ones: `/usr/bin/php` (8.5.4) and `/usr/bin/composer` whose shebang is the absolute `#!/usr/bin/php` — so composer is pinned to the distro interpreter regardless of PATH. `~/laragon/bin` is not on PATH. Result: switching Laragon's PHP changes site serving but not the CLI/composer. (Verified: laragon active 8.4, `php -v` 8.5.4, composer runs under `/usr/bin/php8.5`.)

`core::php_static` downloads the fpm SAPI; `core::install_php_static` is called by the Setup wizard and the version-manager Install command; `Orchestrator::replace_php_version` swaps the active fpm at runtime; `commands::set_php_version` persists `Config.php_version` and swaps. dl.static-php.dev `bulk` provides **cli**, **fpm**, and **micro** SAPIs in the same directory listing (e.g. `php-8.4.22-cli-linux-x86_64.tar.gz`); the cli tarball contains a single `php` binary.

## 2. Approach (chosen: install cli too + active `php` symlink + composer wrapper + opt-in PATH block)

Extend the static install to also fetch the **cli** binary as `~/laragon/bin/php<minor>`. Maintain a symlink `~/laragon/bin/php` → the active `php<minor>`, updated whenever the active version changes. Download `composer.phar` into `~/laragon/bin` and write a `composer` wrapper that execs the sibling `php` against it. An opt-in Settings toggle writes a managed block prepending `~/laragon/bin` to `~/.bashrc`/`~/.zshrc`; disabling removes the block. Everything lives under the user-owned `~/laragon` — no privilege.

Rejected:
- **Wrapper over the distro `/usr/bin/composer`** — the user chose a self-contained `composer.phar` (distro-independent).
- **App-launched terminal only** — does not fix the user's existing terminals; the user chose the PATH block too (the Open-terminal button is Slice B).
- **Editing `/usr/bin/php` or system alternatives** — invasive, needs root, fights the distro.

## 3. Architecture & components

### 3.1 `core/src/php_static.rs` — fetch the cli SAPI too

- Generalize `latest_patch_url(version, arch, listing_json)` → `latest_patch_url(version, arch, sapi, listing_json)` where `sapi ∈ {"fpm","cli"}`; the matched suffix becomes `-<sapi>-linux-<arch>.tar.gz`. (One `bulk/?format=json` fetch serves both SAPIs.)
- `install_php_static(paths, version, downloader, runner)` now installs **both**:
  - fpm → `~/laragon/bin/php-fpm<version>` (as today),
  - cli → `~/laragon/bin/php<version>` (extract the `php` member; mode 0755).
- A new helper `download_static_php(paths, version, sapi, member, dest_name, downloader, runner)` performs the resolve→download→extract→place for one SAPI, reused for both; keeps the function DRY. (`member` = `"php-fpm"` or `"php"`; `dest_name` = `php-fpm<v>` or `php<v>`.)

### 3.2 `core/src/php_cli.rs` (new) — active symlink + composer

- `set_active_php(paths, version) -> std::io::Result<()>`: atomically replace the `~/laragon/bin/php` symlink to point at `php<version>` (remove existing, `symlink`). Unix-only (`std::os::unix::fs::symlink`).
- `ensure_active_php_cli(paths, version, downloader, runner) -> Result<(), PhpStaticError>`: if `~/laragon/bin/php<version>` is missing (e.g. a version installed before this slice had only fpm), download the cli via `download_static_php`; then `set_active_php`.
- `const COMPOSER_URL: &str = "https://getcomposer.org/composer.phar";`
- `install_composer(paths, downloader) -> std::io::Result<()>`: download `COMPOSER_URL` → `~/laragon/bin/composer.phar`; write `~/laragon/bin/composer` =
  ```sh
  #!/bin/sh
  exec "$(dirname "$0")/php" "$(dirname "$0")/composer.phar" "$@"
  ```
  chmod the wrapper 0755. (Runs composer under the sibling active `php`.)

### 3.3 `core/src/shell_env.rs` (new) — managed PATH block

- `pub fn shell_block(home: &Path) -> String` (pure): the exact managed block text:
  ```
  # >>> laragon >>>
  export PATH="<home>/laragon/bin:$PATH"
  # <<< laragon <<<
  ```
- `apply_shell_block(contents: &str, home: &Path) -> String` (pure): like `hosts::apply_block` — insert/replace the marked block in a file's contents, return the updated contents (idempotent; preserves the rest).
- `remove_shell_block(contents: &str) -> String` (pure): strip the marked block.
- `enable_shell_path(home: &Path) -> std::io::Result<()>`: for each of `~/.bashrc` and `~/.zshrc` **that already exists** (don't create new rc files), read → `apply_shell_block` → write if changed.
- `disable_shell_path(home: &Path) -> std::io::Result<()>`: for each existing rc file, read → `remove_shell_block` → write if changed.
- No privilege (user-owned files). Marker `# >>> laragon >>>` / `# <<< laragon <<<`.

### 3.4 `core/src/config.rs` — remember the toggle

- Add `#[serde(default)] pub shell_integration: bool` to `Config` (default `false`). Round-trips in `laragon.toml`.

### 3.5 IPC (Tauri) — `src-tauri/src/commands.rs`

- `set_php_version` (existing): after persisting `Config.php_version` and `replace_php_version`, also call `ensure_active_php_cli(&paths, &version, &CurlDownloader, &RealCommandRunner)` so the `php` symlink (and cli binary) follow the active version. (Runs in the existing `spawn_blocking`.)
- `terminal_integration_status() -> Result<bool, String>`: returns `Config.shell_integration`.
- `set_terminal_integration(app, enabled: bool) -> Result<bool, String>` (async + spawn_blocking):
  - if `enabled`: `ensure_active_php_cli(...)` for the active version; `install_composer(...)`; `enable_shell_path(home)`; set `Config.shell_integration = true` and save.
  - if `!enabled`: `disable_shell_path(home)`; set `Config.shell_integration = false` and save. (Binaries/composer left in place; only the PATH block is removed.)
  - `home` from `std::env::var("HOME")`.
  - Returns the new flag. Errors → `Err(String)`.
- Setup wizard (`run_setup`): after the static PHP install + persist, call `set_active_php(paths, version)` so `~/laragon/bin/php` exists for the freshly-installed default version (composer/PATH only wired when the user enables the toggle). Non-fatal.
- Register the two new commands in `main.rs`.

### 3.6 Frontend — Settings (`dist/`)

- Add a **"Terminal integration"** row to Settings: a toggle **"Use Laragon PHP in terminal (php + composer)"** bound to `terminal_integration_status()` / `set_terminal_integration({enabled})`. While busy, disable it; on success show a toast and a hint: *"Open a new terminal to apply."* On error, revert the toggle and toast.
- Load the status when entering Settings (alongside `loadPhpVersions`).

## 4. Behavior details & decisions

- **CLI follows active**: `~/laragon/bin/php` is a symlink to the active `php<minor>`; `set_php_version` re-points it; `composer` wrapper execs that `php`. Switching version in the app changes both with no extra step.
- **Opt-in PATH**: nothing touches the user's shell until they enable the toggle; disabling cleanly removes the managed block. New terminals (or `source ~/.bashrc`) are needed to pick up the change — surfaced in the UI hint.
- **Self-contained composer**: `composer.phar` from getcomposer.org, run under the active Laragon php — independent of the distro composer.
- **No privilege**: everything is under `~/laragon` and the user's own rc files.
- **Pre-existing fpm-only versions**: `ensure_active_php_cli` downloads the missing cli on demand, so switching to a version installed before this slice still yields a working `php`.
- **rc files**: only modify `~/.bashrc`/`~/.zshrc` if they already exist; never create them (avoid surprising users without those shells).

## 5. Error handling

- Download/extract failures (cli, composer.phar) → `PhpStaticError`/`io::Error` → `Err(String)` → toast; the toggle reverts.
- Missing `HOME` → `Err("HOME not set")`.
- `set_active_php`/shell-block writes are best-effort within their command; a failure is surfaced as `Err(String)` (toggle) or pushed to `report.errors` (setup), consistent with existing patterns.
- Disabling the toggle never deletes binaries (reversible, non-destructive).

## 6. Testing (TDD; fakes only, no network/tools)

- `latest_patch_url(version, arch, sapi, json)`: returns the cli URL for `sapi="cli"` and the fpm URL for `sapi="fpm"` from the same sample listing; ignores the other SAPI/arch.
- `install_php_static` (module-local fakes): fetches the index once then **two** tarball URLs (fpm + cli), runs `tar` for both, and places **both** `php-fpm<v>` and `php<v>` (mode 0755) in `~/laragon/bin`.
- `shell_env`: `apply_shell_block` inserts the block once and is idempotent on re-apply; `remove_shell_block` strips it and leaves surrounding lines intact; `shell_block(home)` contains `export PATH="<home>/laragon/bin:$PATH"` and both markers.
- `set_active_php` (temp root): creates `~/laragon/bin/php` symlink → `php<v>`; re-pointing to another version replaces it.
- `install_composer` (FakeDownloader): writes `composer.phar` and a `composer` wrapper that contains `exec` + `composer.phar`; wrapper is mode 0755.
- `config`: `shell_integration` defaults false and round-trips.

## 7. Out of scope (backlog)

- **Slice B — "Open terminal" button** (per-site + general), launching the user's terminal emulator with the PATH set.
- Node (nvm) / MariaDB version slices.
- Per-site PHP version; choosing a non-`bulk` preset; `micro` SAPI.
- Auto-`source`ing the shell for already-open terminals (not possible without user action).
- Fish/other shells beyond bash/zsh.
