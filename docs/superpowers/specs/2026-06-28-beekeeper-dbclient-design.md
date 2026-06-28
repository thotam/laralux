# Laralux — Beekeeper Studio (native DB client)

**Date:** 2026-06-28
**Status:** Design (approved for spec).
**Goal:** Bundle **Beekeeper Studio** as a portable native database client (the HeidiSQL equivalent on
Linux — manages MariaDB **and** Redis), installed no-apt and launched globally from the Dashboard and
the tray.

Second of the two DB-management sub-projects (phpMyAdmin, the web admin, is deferred until its 6.0
release supports PHP 8.4).

---

## 1. Context & current state

- Laralux downloads tools no-apt into `~/laralux/` (the `*_static.rs` pattern: download + place), and
  installs them through `core/src/setup.rs` (`Component` enum + `run_setup`).
- Launching external processes is an established pattern: `core/src/terminal.rs::open_terminal` and
  `core/src/filemanager.rs::open_folder` both `std::process::Command::new(..).spawn()` detached. The
  Tauri commands `open_terminal`/`open_folder` mirror to those; the tray menu lives in
  `src-tauri/src/main.rs`.
- Beekeeper Studio ships a portable **AppImage** on GitHub releases (latest **v5.8.1**):
  `Beekeeper-Studio-5.8.1.AppImage` (x86_64) and `Beekeeper-Studio-5.8.1-arm64.AppImage` (aarch64),
  at `https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v<ver>/<asset>`.
  Community edition has full MariaDB/MySQL **and Redis** support (verified from its README).
- MariaDB runs on `127.0.0.1:3306` (root, no password); Redis (Valkey) on `127.0.0.1:6379`.

## 2. Approach

Install Beekeeper by downloading its AppImage and **extracting it once** into
`~/laralux/apps/beekeeper/squashfs-root/`, then launching the extracted `AppRun` directly. This needs
no `libfuse2` (avoids apt) and avoids re-extracting on every launch (chosen over
`--appimage-extract-and-run`). Beekeeper is exposed as one global "Open DB client" entry on the
Dashboard and in the tray; it's a desktop GUI (not a served URL), so it's launched as a detached
process. Beekeeper manages its own connections — the user adds the MariaDB/Redis endpoints once
(auto-seeding them is backlog).

## 3. Architecture & components

### 3.1 `core/src/beekeeper.rs` (new)
- `pub const BEEKEEPER_VERSION: &str = "5.8.1";`
- `pub fn beekeeper_arch() -> Option<&'static str>`: `x86_64` → `Some("")`, `aarch64` → `Some("-arm64")`, else `None`.
- `pub fn appimage_url(version, arch_suffix) -> String`:
  `https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v{version}/Beekeeper-Studio-{version}{arch_suffix}.AppImage`.
- `pub fn install_dir(paths) -> PathBuf` → `paths.root().join("apps/beekeeper")`.
- `pub fn apprun_path(paths) -> PathBuf` → `install_dir/squashfs-root/AppRun`.
- `pub fn is_installed(paths) -> bool` → `apprun_path` exists.
- `pub fn install_beekeeper(paths, downloader, runner, sink) -> Result<String, BeekeeperError>`:
  resolve arch (`Arch` error if unsupported); download the AppImage to `tmp/beekeeper.AppImage`;
  `chmod +x`; run `<appimage> --appimage-extract` with CWD = `install_dir` (the AppImage runtime's
  built-in extraction — no FUSE) producing `install_dir/squashfs-root`; remove the downloaded
  AppImage. Idempotent (skips when `is_installed`). Returns the version.
- `pub fn open_beekeeper(paths) -> Result<(), BeekeeperError>`: `NotInstalled` if `!is_installed`;
  else spawn `apprun_path` detached with `--no-sandbox` (the extracted Electron app runs unprivileged
  without a SUID chrome-sandbox, so the flag is required for it to start; safe for a local
  user-run tool).
- `#[derive(thiserror::Error)] pub enum BeekeeperError { Arch(String), Download(String), Extract(String), NotInstalled, Spawn(String), Io(#[from] std::io::Error) }`.
- Export from `lib.rs`.

### 3.2 Setup install — `core/src/setup.rs`
- Add `Component::Beekeeper` to the enum + `ALL` + `label()` ("beekeeper") + detect (present =
  `beekeeper::is_installed`) + `apt_packages_for` (empty). In `run_setup`, when missing, call
  `beekeeper::install_beekeeper(...)` and record `beekeeper_fetched: bool` in `SetupReport`. It then
  installs with "Install missing" and appears in the Setup list.

### 3.3 Desktop — command + tray
- `src-tauri/src/commands.rs`:
  - `#[tauri::command] pub fn beekeeper_status() -> BeekeeperStatus { installed: bool }` (via `is_installed`).
  - `#[tauri::command] pub fn open_beekeeper() -> Result<(), String>` → `laralux_core::open_beekeeper(&paths).map_err(|e| e.to_string())`.
- Register both in `main.rs`'s `generate_handler!`.
- `main.rs` tray: add a "DB client (Beekeeper)" menu item → calls `open_beekeeper` (when installed),
  else focuses the window for Setup.

### 3.4 Frontend
- `src/ipc/types.ts`: `BeekeeperStatus { installed: boolean }`.
- `src/ipc/commands.ts`: `beekeeperStatus()` and `openBeekeeper()` wrappers.
- Dashboard (`src/ui/views/dashboard.ts`): a global **"Open DB client"** button (header/quick-actions).
  On click: if installed → `openBeekeeper()`; else toast "Install the DB client from Setup" (and/or
  navigate to Setup). The new `Beekeeper` component appears in Setup to install.

## 4. Data flow
1. Setup → Install missing downloads the Beekeeper AppImage, extracts it once to
   `apps/beekeeper/squashfs-root/`, removes the AppImage.
2. Dashboard/tray "Open DB client" → `open_beekeeper` spawns `squashfs-root/AppRun --no-sandbox`
   detached → Beekeeper opens.
3. In Beekeeper, the user adds connections: MariaDB `127.0.0.1:3306` (root) and Redis
   `127.0.0.1:6379`.

## 5. Behavior & error handling
- Launch is best-effort/detached like `open_terminal`; failures (`NotInstalled`, spawn error) surface
  as a toast. The Dashboard button is enabled only when `installed`; otherwise it points to Setup.
- `--appimage-extract` and the extracted `AppRun` need NO `libfuse2` (no apt) — the runtime extracts
  via its own code; running the extracted tree is plain process exec.
- `--no-sandbox` is included because the extracted Electron app has no SUID chrome-sandbox when run
  from `~/laralux` as a normal user; this is the standard portable-Electron launch and is acceptable
  for a local, user-run GUI.
- Install is best-effort with the existing progress/toast machinery; a failed download/extract surfaces
  in the Setup report errors, like other components. The AppImage is ~100–150 MB (one-time).

## 6. Testing (TDD where it applies)
- `core/src/beekeeper.rs`: `beekeeper_arch()` mapping; `appimage_url()` exact for x86_64 (no suffix)
  and aarch64 (`-arm64`); `is_installed` false on a fresh root, true after seeding
  `apps/beekeeper/squashfs-root/AppRun`. (Real download/extract/launch not unit-tested, like other
  installers/launchers.)
- `setup.rs`: `Component::Beekeeper` present in `ALL` (count updated); `detect` reflects `is_installed`.
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display): Setup installs Beekeeper; Dashboard "Open DB client" launches it;
  add MariaDB (127.0.0.1:3306 root) and Redis (127.0.0.1:6379) connections and browse data; the tray
  "DB client (Beekeeper)" item launches the same app.

## 7. Out of scope / backlog
- Auto-seeding Beekeeper connections for the managed MariaDB/Redis (Beekeeper's connection store
  format is app-specific — manual add for now).
- Version self-update / upgrade of the bundled Beekeeper (re-run Setup to refresh).
- DbGate/DBeaver alternatives (Beekeeper chosen).
- phpMyAdmin web admin (deferred to its own spec).
