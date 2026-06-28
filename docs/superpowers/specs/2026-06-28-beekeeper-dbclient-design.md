# Laralux — Beekeeper Studio (native DB client)

**Date:** 2026-06-28
**Status:** Design (approved for spec).
**Goal:** Bundle **Beekeeper Studio** as a portable native database client (the HeidiSQL equivalent on
Linux — manages MariaDB **and** Redis), installed no-apt **on first use** and launched globally from
the Dashboard and the tray, with a circular download-progress indicator on the first install.

Second of the two DB-management sub-projects (phpMyAdmin, the web admin, is deferred until its 6.0
release supports PHP 8.4).

---

## 1. Context & current state

- Laralux downloads tools no-apt into `~/laralux/`. Downloads report byte progress through a
  `ProgressSink`; the desktop wraps it as `TauriProgress` (used by `run_setup_cmd`) which emits a
  `download-progress` event. The frontend's `applyProgress()` + `progressRing()` + `updateRing()`
  render a **circular ring** with percentage (used today for tool installs / setup).
- Launching external processes is an established pattern: `core/src/terminal.rs::open_terminal` and
  `core/src/filemanager.rs::open_folder` `std::process::Command::new(..).spawn()` detached. Tauri
  commands `open_terminal`/`open_folder` mirror them; the tray menu is in `src-tauri/src/main.rs`.
- Beekeeper Studio ships a portable **AppImage** on GitHub releases (latest **v5.8.1**):
  `Beekeeper-Studio-5.8.1.AppImage` (x86_64) and `Beekeeper-Studio-5.8.1-arm64.AppImage` (aarch64),
  at `https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v<ver>/<asset>`.
  Community edition has full MariaDB/MySQL **and Redis** support (verified from its README).
- MariaDB runs on `127.0.0.1:3306` (root, no password); Redis (Valkey) on `127.0.0.1:6379`.
- The Setup component list is for **ManagedTools** (version + symlink modal). Beekeeper has neither,
  so it is intentionally NOT a Setup component — it installs on demand from its own button.

## 2. Approach

Install Beekeeper **on demand from the "Open DB client" entry**, not via Setup. The first time the
user opens it, Laralux downloads the AppImage (showing a **circular download-progress** ring),
extracts it once into `~/laralux/apps/beekeeper/squashfs-root/` (no `libfuse2` — the AppImage
runtime's built-in `--appimage-extract`), then launches the extracted `AppRun`. Subsequent opens just
launch. Self-contained: outside the Setup ManagedTool list and not bundled into "Install missing".

## 3. Architecture & components

### 3.1 `core/src/beekeeper.rs` (new)
- `pub const BEEKEEPER_VERSION: &str = "5.8.1";`
- `pub fn beekeeper_arch() -> Option<&'static str>`: `x86_64` → `Some("")`, `aarch64` → `Some("-arm64")`, else `None`.
- `pub fn appimage_url(version, arch_suffix) -> String`:
  `https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v{version}/Beekeeper-Studio-{version}{arch_suffix}.AppImage`.
- `pub fn install_dir(paths) -> PathBuf` → `paths.root().join("apps/beekeeper")`.
- `pub fn apprun_path(paths) -> PathBuf` → `install_dir/squashfs-root/AppRun`.
- `pub fn is_installed(paths) -> bool` → `apprun_path` exists.
- `pub fn ensure_beekeeper(paths, downloader, runner, sink) -> Result<String, BeekeeperError>`:
  no-op (return version) when `is_installed`; else resolve arch (`Arch` error if unsupported);
  `fetch_with_progress` the AppImage to `tmp/beekeeper.AppImage` (this drives the download ring);
  `chmod +x`; run `<appimage> --appimage-extract` with CWD = `install_dir` (the runtime's built-in
  extractor — no FUSE) producing `install_dir/squashfs-root`; remove the downloaded AppImage. Returns
  the version.
- `pub fn open_beekeeper(paths) -> Result<(), BeekeeperError>`: `NotInstalled` if `!is_installed`;
  else spawn `apprun_path` detached with `--no-sandbox` (the extracted Electron app has no SUID
  chrome-sandbox when run unprivileged from `~/laralux`, so the flag is required to start; safe for a
  local user-run GUI).
- `#[derive(thiserror::Error)] pub enum BeekeeperError { Arch(String), Download(String), Extract(String), NotInstalled, Spawn(String), Io(#[from] std::io::Error) }`.
- Export from `lib.rs`.

### 3.2 Desktop — `open_db_client` command + tray
- `src-tauri/src/commands.rs`:
  - `#[tauri::command] pub async fn open_db_client(app: tauri::AppHandle) -> Result<(), String>`:
    run in `spawn_blocking`; if `!beekeeper::is_installed(&paths)`, call
    `beekeeper::ensure_beekeeper(&paths, &CurlDownloader, &RealCommandRunner, &TauriProgress(app))` —
    `TauriProgress` emits `download-progress` so the UI shows the ring; then
    `beekeeper::open_beekeeper(&paths)`. Errors map to `String`.
- `src-tauri/src/main.rs`: register `open_db_client`; add a tray item "DB client (Beekeeper)" whose
  handler launches it — when already installed, call `laralux_core::open_beekeeper(&state.paths)`
  directly (no download from the tray); when not installed, focus the window so the user uses the
  Dashboard button (which shows the download ring). (The tray doesn't show a ring, so it only
  launches an already-installed client.)

### 3.3 Frontend
- `src/state.ts`: add `dbClientBusy: boolean` (default `false`).
- `src/ipc/commands.ts`: `openDbClient()` → `invoke("open_db_client")`.
- Dashboard (`src/ui/views/dashboard.ts`): a **"Tools"** section with an **"Open DB client"** button.
  - Normal: `<button data-action="open-db-client">DB client</button>`.
  - While `state.dbClientBusy`: render `progressRing()` in place of the button label so the first-time
    download shows a circular percentage (fed by the existing `download-progress` → `updateRing()`).
- `src/ui/events.ts`: `open-db-client` → `openDbClient()` action:
  `state.dbClientBusy = true; render();` → `await openDbClient()` (success: app launches; error: toast)
  → `finally { state.dbClientBusy = false; resetDownload(); render(); }`. Mirrors the tool-modal
  install busy/ring pattern.

## 4. Data flow
1. Click "Open DB client" (Dashboard) → `open-db-client` sets `dbClientBusy`, shows the ring, calls
   `open_db_client`.
2. Backend: if not installed → `ensure_beekeeper` downloads the AppImage (emitting `download-progress`
   → the ring fills) and extracts to `apps/beekeeper/squashfs-root/`; then `open_beekeeper` launches
   `AppRun --no-sandbox` detached.
3. Subsequent clicks: already installed → launches immediately (no download).
4. Tray "DB client (Beekeeper)" → launches if installed, else focuses the window.
5. In Beekeeper, the user adds connections: MariaDB `127.0.0.1:3306` (root) and Redis `127.0.0.1:6379`.

## 5. Behavior & error handling
- First-time install shows a circular download ring; on failure (`Arch`, download, extract) a toast
  surfaces the error and `dbClientBusy` is cleared.
- Launch is best-effort/detached like `open_terminal`; `NotInstalled`/spawn errors → toast.
- `--appimage-extract` and the extracted `AppRun` need NO `libfuse2`; `--no-sandbox` is the standard
  portable-Electron launch for an unprivileged extracted app.
- The AppImage is ~100–150 MB (one-time download). `ensure_beekeeper` is idempotent (skips when
  installed). Re-running it never re-downloads unless `squashfs-root/AppRun` is missing.

## 6. Testing (TDD where it applies)
- `core/src/beekeeper.rs`: `beekeeper_arch()` mapping; `appimage_url()` exact for x86_64 (no suffix)
  and aarch64 (`-arm64`); `is_installed` false on a fresh root, true after seeding
  `apps/beekeeper/squashfs-root/AppRun`. (Real download/extract/launch not unit-tested, like other
  installers/launchers.)
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display): Dashboard "Open DB client" first click shows a download ring, then
  Beekeeper launches; add MariaDB (127.0.0.1:3306 root) and Redis (127.0.0.1:6379) connections and
  browse; second click launches instantly; the tray item launches the installed client.

## 7. Out of scope / backlog
- Auto-seeding Beekeeper connections for the managed MariaDB/Redis (Beekeeper's store format is
  app-specific — manual add for now).
- Version self-update of the bundled Beekeeper (delete `apps/beekeeper` to force re-download).
- Download ring on the **tray** launch (tray only launches an already-installed client).
- DbGate/DBeaver alternatives (Beekeeper chosen); phpMyAdmin web admin (deferred to its own spec).
