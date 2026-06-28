# Laralux — DbGate (native DB client)

**Date:** 2026-06-28
**Status:** Implemented.
**Goal:** Bundle **DbGate** as a portable native database client (the HeidiSQL equivalent on Linux —
manages MariaDB **and** Redis), installed no-apt **on first use** and launched globally from the
Dashboard and the tray, with a circular download-progress indicator on the first install.

Second of the two DB-management sub-projects (phpMyAdmin, the web admin, is deferred until its 6.0
release supports PHP 8.4).

> **Change note (why DbGate, not Beekeeper):** this sub-project first shipped with Beekeeper Studio,
> but Beekeeper's only prebuilt binaries (AppImage/deb/rpm/snap) are the **paid Ultimate edition on a
> 2-week trial** — it nags "Buy a License / Enter License". The free Community edition has no prebuilt
> AppImage (source build only). **DbGate** is genuinely free (**GPL-3.0**), its non-`premium` AppImage
> is the full open-source build, and it supports **MariaDB + Redis** (+ Postgres/SQLite/Mongo) with no
> license nag. DBeaver was rejected because its Community edition lacks Redis (NoSQL is paid PRO only).
> The install/extract/launch mechanism is identical; only the app, URL, version, and labels changed.

---

## 1. Context & current state

- Laralux downloads tools no-apt into `~/laralux/`. Downloads report byte progress through a
  `ProgressSink`; the desktop wraps it as `TauriProgress` which emits a `download-progress` event. The
  frontend's `applyProgress()` + `progressRing()` + `updateRing()` render a **circular ring** with
  percentage (used today for tool installs / setup).
- Launching external processes is an established pattern: `core/src/terminal.rs::open_terminal` and
  `core/src/filemanager.rs::open_folder` `std::process::Command::new(..).spawn()` detached. Tauri
  commands mirror them; the tray menu is in `src-tauri/src/main.rs`.
- DbGate ships a portable **AppImage** on GitHub releases (latest **v7.2.1**):
  `dbgate-7.2.1-linux_x86_64.AppImage` (x86_64) and `dbgate-7.2.1-linux_arm64.AppImage` (aarch64),
  at `https://github.com/dbgate/dbgate/releases/download/v<ver>/dbgate-<ver>-linux_<arch>.AppImage`.
  The non-`premium` asset is the free GPL-3.0 build with full MariaDB/MySQL **and Redis** support.
- MariaDB runs on `127.0.0.1:3306` (root, no password); Redis (Valkey) on `127.0.0.1:6379`.
- The Setup component list is for **ManagedTools** (version + symlink modal). DbGate has neither, so it
  is intentionally NOT a Setup component — it installs on demand from its own button.

## 2. Approach

Install DbGate **on demand from the "Open DB client" entry**, not via Setup. The first time the user
opens it, Laralux downloads the AppImage (showing a **circular download-progress** ring), extracts it
once into `~/laralux/apps/dbgate/squashfs-root/` (no `libfuse2` — the AppImage runtime's built-in
`--appimage-extract`), then launches the extracted `AppRun`. Subsequent opens just launch.
Self-contained: outside the Setup ManagedTool list and not bundled into "Install missing".

## 3. Architecture & components

### 3.1 `core/src/dbgate.rs`
- `pub const DBGATE_VERSION: &str = "7.2.1";`
- `pub fn dbgate_arch() -> Option<&'static str>`: `x86_64` → `Some("x86_64")`, `aarch64` → `Some("arm64")`, else `None`.
- `pub fn appimage_url(version, arch) -> String`:
  `https://github.com/dbgate/dbgate/releases/download/v{version}/dbgate-{version}-linux_{arch}.AppImage`.
- `pub fn install_dir(paths) -> PathBuf` → `paths.root().join("apps/dbgate")`.
- `pub fn apprun_path(paths) -> PathBuf` → `install_dir/squashfs-root/AppRun`.
- `pub fn is_installed(paths) -> bool` → `apprun_path` exists.
- `pub fn ensure_dbgate(paths, downloader, runner, sink) -> Result<String, DbgateError>`:
  no-op (return version) when `is_installed`; else resolve arch (`Arch` error if unsupported);
  `fetch_with_progress` the AppImage to `tmp/dbgate.AppImage` (this drives the download ring);
  `chmod +x`; run `<appimage> --appimage-extract` with CWD = `install_dir` (the runtime's built-in
  extractor — no FUSE) producing `install_dir/squashfs-root`; remove the downloaded AppImage. Returns
  the version.
- `pub fn open_dbgate(paths) -> Result<(), DbgateError>`: `NotInstalled` if `!is_installed`; else spawn
  `apprun_path` detached with the `APPDIR` env var set to `squashfs-root` and **no args**. DbGate's
  AppRun auto-detects the AppDir using `$1` as a sentinel filename when `$APPDIR` is empty, so passing
  any flag (e.g. `--no-sandbox`) breaks detection. DbGate runs unprivileged without needing
  `--no-sandbox` (its `dbgate` binary rejects that flag — verified empirically on x86_64).
- `#[derive(thiserror::Error)] pub enum DbgateError { Arch(String), Download(String), Extract(String), NotInstalled, Spawn(String), Io(#[from] std::io::Error) }`.
- Exported from `lib.rs`: `ensure_dbgate, open_dbgate, DbgateError`.

### 3.2 Desktop — `open_db_client` command + tray
- `src-tauri/src/commands.rs`:
  - `#[tauri::command] pub async fn open_db_client(app: tauri::AppHandle) -> Result<(), String>`:
    run in `spawn_blocking`; if `!dbgate::is_installed(&paths)`, call
    `dbgate::ensure_dbgate(&paths, &CurlDownloader, &RealCommandRunner, &TauriProgress(app))` —
    `TauriProgress` emits `download-progress` so the UI shows the ring; then `dbgate::open_dbgate`.
    Errors map to `String`.
- `src-tauri/src/main.rs`: register `open_db_client`; tray item "DB client (DbGate)" whose handler —
  when installed — calls `laralux_core::open_dbgate(&state.paths)` directly (no download from the
  tray); when not installed, focuses the window so the user uses the Dashboard button (which shows the
  download ring).

### 3.3 Frontend
- `src/state.ts`: `dbClientBusy: boolean` (default `false`).
- `src/ipc/commands.ts`: `openDbClient()` → `invoke("open_db_client")`.
- Dashboard (`src/ui/views/dashboard.ts`): a **"Tools"** section card ("DB client", desc
  "DbGate — manage MariaDB & Redis") with an **Open** button.
  - While `state.dbClientBusy`: render `progressRing()` in place of the button so the first-time
    download shows a circular percentage (fed by `download-progress` → `updateRing()`).
- `src/ui/events.ts`: `open-db-client` → `launchDbClient()`:
  `state.dbClientBusy = true; render();` → `await openDbClient()` (success: app launches; error: toast)
  → `finally { state.dbClientBusy = false; resetDownload(); render(); }`.

## 4. Data flow
1. Click "Open" (Dashboard Tools) → sets `dbClientBusy`, shows the ring, calls `open_db_client`.
2. Backend: if not installed → `ensure_dbgate` downloads the AppImage (emitting `download-progress` →
   the ring fills) and extracts to `apps/dbgate/squashfs-root/`; then `open_dbgate` launches `AppRun`
   detached with `APPDIR` set.
3. Subsequent clicks: already installed → launches immediately (no download).
4. Tray "DB client (DbGate)" → launches if installed, else focuses the window.
5. In DbGate, the user adds connections: MariaDB `127.0.0.1:3306` (root) and Redis `127.0.0.1:6379`.

## 5. Behavior & error handling
- First-time install shows a circular download ring; on failure (`Arch`, download, extract) a toast
  surfaces the error and `dbClientBusy` is cleared.
- Launch is best-effort/detached like `open_terminal`; `NotInstalled`/spawn errors → toast.
- `--appimage-extract` and the extracted `AppRun` need NO `libfuse2`.
- The AppImage is ~175 MB (one-time download). `ensure_dbgate` is idempotent (skips when installed).
  Re-running it never re-downloads unless `squashfs-root/AppRun` is missing.

## 6. Testing
- `core/src/dbgate.rs`: `dbgate_arch()` mapping; `appimage_url()` exact for x86_64 and aarch64;
  `is_installed` false on a fresh root, true after seeding `apps/dbgate/squashfs-root/AppRun`. (Real
  download/extract/launch not unit-tested, like other installers/launchers.)
- `cargo test -p laralux-core` green; `cargo build -p laralux-desktop` green; `npm run build` green.
- Manual smoke (needs a display): Dashboard "Open" first click shows a download ring, then DbGate
  launches; add MariaDB (127.0.0.1:3306 root) and Redis (127.0.0.1:6379) connections and browse;
  second click launches instantly; the tray item launches the installed client.

## 7. Out of scope / backlog
- Auto-seeding DbGate connections for the managed MariaDB/Redis (manual add for now).
- Version self-update of the bundled DbGate (delete `apps/dbgate` to force re-download).
- Download ring on the **tray** launch (tray only launches an already-installed client).
- phpMyAdmin web admin (deferred to its own spec until pma 6.0 supports PHP 8.4).
