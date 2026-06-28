# Beekeeper DB Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bundle Beekeeper Studio as a portable native DB client (MariaDB + Redis), installed no-apt **on first use** (with a circular download ring) and launched globally from the Dashboard and tray.

**Architecture:** A `core` module downloads + extracts the Beekeeper AppImage once into `~/laralux/apps/beekeeper/squashfs-root/` and launches the extracted `AppRun --no-sandbox`. An async `open_db_client` command ensures-installed (emitting `download-progress`) then launches; the Dashboard button shows the existing progress ring while installing; the tray launches an already-installed client.

**Tech Stack:** Rust (`laralux-core`, no Tauri deps), Tauri 2 command + tray, TypeScript (strict), Vite, morphdom.

## Global Constraints

- `laralux-core` keeps ZERO Tauri dependencies.
- No `libfuse2` / apt: use the AppImage runtime's `--appimage-extract`, run the extracted `AppRun` with `--no-sandbox`.
- Beekeeper is installed **on demand from its button**, NOT a Setup `Component` and NOT in "Install missing".
- Reuse the existing download-progress ring: `TauriProgress` emits `download-progress`; the frontend's `progressRing()` / `updateRing()` / `resetDownload()` render it.
- Strict TypeScript: no `@ts-nocheck`, no `as any` in `src/`. `render.ts` stays the sole `#app` mount.
- Pinned `BEEKEEPER_VERSION = "5.8.1"`; URL `https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v<ver>/Beekeeper-Studio-<ver><arch>.AppImage` (`<arch>` = "" for x86_64, "-arm64" for aarch64).
- Git commits have NO `Co-Authored-By` trailer. Work on master. DO NOT create a git worktree.
- Run tools with `PATH="$HOME/.cargo/bin:$PATH"`. No automated UI tests; the UI task's gate is a green build + manual smoke.

## File Structure

- **Create** `core/src/beekeeper.rs` — download/extract/launch + helpers + tests.
- **Modify** `core/src/lib.rs` — module + re-export.
- **Modify** `src-tauri/src/commands.rs` + `src-tauri/src/main.rs` — `open_db_client` command + registration + tray item.
- **Modify** `src/state.ts`, `src/ipc/commands.ts`, `src/ui/views/dashboard.ts`, `src/ui/events.ts` — busy state, IPC wrapper, Tools button, dispatch.

---

### Task 1: core `beekeeper.rs` (download + extract + launch)

**Files:**
- Create: `core/src/beekeeper.rs`
- Modify: `core/src/lib.rs`

**Interfaces:**
- Produces: `BEEKEEPER_VERSION`, `beekeeper_arch()`, `appimage_url(version, arch_suffix)`, `install_dir(paths)`, `apprun_path(paths)`, `is_installed(paths)`, `ensure_beekeeper(paths, downloader, runner, sink) -> Result<String, BeekeeperError>`, `open_beekeeper(paths) -> Result<(), BeekeeperError>`, `BeekeeperError`.

- [ ] **Step 1: Write `core/src/beekeeper.rs` (impl + tests)**

```rust
use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::PathBuf;

pub const BEEKEEPER_VERSION: &str = "5.8.1";

#[derive(Debug, thiserror::Error)]
pub enum BeekeeperError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("not installed")]
    NotInstalled,
    #[error("failed to launch: {0}")]
    Spawn(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// AppImage asset arch suffix: x86_64 → "" , aarch64 → "-arm64".
pub fn beekeeper_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some(""),
        "aarch64" => Some("-arm64"),
        _ => None,
    }
}

pub fn appimage_url(version: &str, arch_suffix: &str) -> String {
    format!(
        "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v{version}/Beekeeper-Studio-{version}{arch_suffix}.AppImage"
    )
}

pub fn install_dir(paths: &LaraluxPaths) -> PathBuf {
    paths.root().join("apps/beekeeper")
}

pub fn apprun_path(paths: &LaraluxPaths) -> PathBuf {
    install_dir(paths).join("squashfs-root").join("AppRun")
}

pub fn is_installed(paths: &LaraluxPaths) -> bool {
    apprun_path(paths).is_file()
}

/// Download the AppImage and extract it once into apps/beekeeper/squashfs-root/.
/// Idempotent: a no-op (returns the version) when already installed.
pub fn ensure_beekeeper(
    paths: &LaraluxPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<String, BeekeeperError> {
    if is_installed(paths) {
        return Ok(BEEKEEPER_VERSION.to_string());
    }
    let arch = beekeeper_arch().ok_or_else(|| BeekeeperError::Arch(std::env::consts::ARCH.to_string()))?;
    let dir = install_dir(paths);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(paths.tmp())?;
    let appimage = paths.tmp().join("beekeeper.AppImage");
    downloader
        .fetch_with_progress(&appimage_url(BEEKEEPER_VERSION, arch), &appimage, sink)
        .map_err(|e| BeekeeperError::Download(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&appimage, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_dir_all(dir.join("squashfs-root")); // clear any stale extract
    // `--appimage-extract` uses the AppImage runtime's built-in extractor (no FUSE);
    // it writes `squashfs-root` into the CWD, so run it with CWD = install dir.
    runner
        .run(&appimage.display().to_string(), &["--appimage-extract".into()], Some(&dir))
        .map_err(|e| BeekeeperError::Extract(e.to_string()))?;
    if !is_installed(paths) {
        return Err(BeekeeperError::Extract("AppRun not found after --appimage-extract".into()));
    }
    let _ = std::fs::remove_file(&appimage);
    Ok(BEEKEEPER_VERSION.to_string())
}

/// Launch the extracted Beekeeper detached. `--no-sandbox` is required because the
/// extracted Electron app has no SUID chrome-sandbox when run unprivileged from ~/laralux.
pub fn open_beekeeper(paths: &LaraluxPaths) -> Result<(), BeekeeperError> {
    if !is_installed(paths) {
        return Err(BeekeeperError::NotInstalled);
    }
    std::process::Command::new(apprun_path(paths))
        .arg("--no-sandbox")
        .spawn()
        .map_err(|e| BeekeeperError::Spawn(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_and_url() {
        assert_eq!(
            appimage_url("5.8.1", ""),
            "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v5.8.1/Beekeeper-Studio-5.8.1.AppImage"
        );
        assert_eq!(
            appimage_url("5.8.1", "-arm64"),
            "https://github.com/beekeeper-studio/beekeeper-studio/releases/download/v5.8.1/Beekeeper-Studio-5.8.1-arm64.AppImage"
        );
        assert_eq!(
            beekeeper_arch(),
            match std::env::consts::ARCH { "x86_64" => Some(""), "aarch64" => Some("-arm64"), _ => None }
        );
    }

    #[test]
    fn is_installed_reflects_apprun() {
        let root = std::env::temp_dir().join(format!("lara-bk-{}", std::process::id()));
        let paths = LaraluxPaths::new(root.clone());
        assert!(!is_installed(&paths));
        std::fs::create_dir_all(install_dir(&paths).join("squashfs-root")).unwrap();
        std::fs::write(apprun_path(&paths), b"x").unwrap();
        assert!(is_installed(&paths));
        std::fs::remove_dir_all(&root).ok();
    }
}
```

- [ ] **Step 2: Wire `core/src/lib.rs`**

Add next to `pub mod filemanager;`:
```rust
pub mod beekeeper;
```
And next to `pub use filemanager::{open_folder, FileManagerError};`:
```rust
pub use beekeeper::{ensure_beekeeper, open_beekeeper, BeekeeperError};
```

- [ ] **Step 3: Run the tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core beekeeper 2>&1 | tail -8`
Expected: PASS (`arch_and_url`, `is_installed_reflects_apprun`), output pristine. (Declare the lib.rs module first so it compiles.)

- [ ] **Step 4: Build the desktop crate to confirm the re-export wires**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2`
Expected: `Finished`.

- [ ] **Step 5: Commit**

```bash
git add core/src/beekeeper.rs core/src/lib.rs
git commit -m "feat(beekeeper): core download/extract/launch for the DB client"
```

---

### Task 2: `open_db_client` command + tray item

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `laralux_core::beekeeper::{is_installed, ensure_beekeeper}`, `laralux_core::open_beekeeper`, `TauriProgress`, `CurlDownloader`, `RealCommandRunner`, `AppState`.
- Produces: Tauri command `open_db_client`; tray menu item `db_client`.

- [ ] **Step 1: Add the command in `src-tauri/src/commands.rs`**

Mirror `run_setup_cmd`'s async + `TauriProgress` shape. Add (near `open_folder`):
```rust
#[tauri::command]
pub async fn open_db_client(app: tauri::AppHandle) -> Result<(), String> {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        if !laralux_core::beekeeper::is_installed(&state.paths) {
            let progress = TauriProgress(app_for_progress);
            laralux_core::beekeeper::ensure_beekeeper(
                &state.paths,
                &CurlDownloader,
                &RealCommandRunner,
                &progress,
            )
            .map_err(|e| e.to_string())?;
        }
        laralux_core::open_beekeeper(&state.paths).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
```
(`TauriProgress`, `CurlDownloader`, `RealCommandRunner`, `AppState` are already imported/defined in this file — they are used by `run_setup_cmd`. Match the existing imports.)

- [ ] **Step 2: Register + add the tray item in `src-tauri/src/main.rs`**

Register the command in `generate_handler![]` (next to `commands::open_folder,`):
```rust
            commands::open_db_client,
```
Add the tray menu item — alongside the existing `MenuItemBuilder::with_id(...)` lines:
```rust
            let db_client = MenuItemBuilder::with_id("db_client", "DB client (Beekeeper)").build(app)?;
```
Add it to the `MenuBuilder` items list (e.g. after `dashboard`):
```rust
                .items(&[&start, &stop, &dashboard, &db_client, &quit])
```
(Match the existing `.items(&[...])` call — insert `&db_client` into it.)
Add the handler arm in the `on_menu_event` match (next to `"dashboard" =>`):
```rust
                    "db_client" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if laralux_core::beekeeper::is_installed(&state.paths) {
                                let _ = laralux_core::open_beekeeper(&state.paths);
                            } else if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                    }
```

- [ ] **Step 3: Build**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(beekeeper): open_db_client command (ensure-install + launch) + tray item"
```

---

### Task 3: Frontend — Tools button with download ring

**Files:**
- Modify: `src/state.ts`, `src/ipc/commands.ts`, `src/ui/views/dashboard.ts`, `src/ui/events.ts`

**Interfaces:**
- Consumes: `open_db_client` command; existing `progressRing`, `resetDownload`, `render`, `toast`, `state`.
- Produces: `state.dbClientBusy`; `openDbClient()` IPC; `launchDbClient()` action; dashboard Tools button.

- [ ] **Step 1: Add `dbClientBusy` to state — `src/state.ts`**

In the `AppState` interface, after `download: DownloadState;` add:
```ts
  dbClientBusy: boolean;
```
In the `state` literal, after the `download: { ... }` line add:
```ts
  dbClientBusy: false,
```

- [ ] **Step 2: IPC wrapper — `src/ipc/commands.ts`**

Add (near `openTerminalAt`/`openFolderAt`):
```ts
export const openDbClient = (): Promise<void> => invoke<void>("open_db_client");
```

- [ ] **Step 3: Dashboard Tools section + action — `src/ui/views/dashboard.ts`**

Add imports as needed at the top (merge into existing import lines): `progressRing`, `resetDownload`, `render` from `../render`; `toast` from `../toast`; `openDbClient` from `../../ipc/commands`. Add the action function:
```ts
export async function launchDbClient(): Promise<void> {
  state.dbClientBusy = true;
  render();
  try {
    await openDbClient();
  } catch (e) {
    toast({ type: "error", title: "Couldn't open DB client", msg: String(e) });
  } finally {
    state.dbClientBusy = false;
    resetDownload();
    render();
  }
}
```
In `dashboard()`'s returned markup, add a Tools section before the closing `'</div>'` of the view (after the Sites `stack-col`):
```ts
    '<div class="row-between mt4"><h2 class="section-label">Tools</h2></div>' +
    '<div class="card"><div class="set-row"><div class="grow"><div class="t">DB client</div>' +
    '<div class="h">Beekeeper — manage MariaDB &amp; Redis</div></div>' +
    (state.dbClientBusy
      ? progressRing()
      : '<button class="btn-sm" data-action="open-db-client">' + I.external + "Open</button>") +
    "</div></div>" +
```
(Place this immediately before the final `"</div>"` that closes the `<div class="view">`. Keep the existing Sites block above it.)

- [ ] **Step 4: Wire the click — `src/ui/events.ts`**

Add `launchDbClient` to the existing `import { ... } from "./views/dashboard";` line. Add a branch to the click chain (near the other dashboard actions):
```ts
    else if (a === "open-db-client") launchDbClient();
```

- [ ] **Step 5: Build**

Run:
```bash
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -3
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop 2>&1 | tail -2
```
Expected: green (strict tsc + vite) + `Finished`. `grep -rn "as any\|@ts-nocheck" src/` → none.

- [ ] **Step 6: Smoke test (requires a display + network)**

`PATH="$HOME/.cargo/bin:$PATH" cargo run -p laralux-desktop` → Dashboard → Tools → "Open":
- First click shows a circular download ring (filling as the ~140 MB AppImage downloads), then Beekeeper launches.
- In Beekeeper, add MariaDB (127.0.0.1:3306, root) and Redis (127.0.0.1:6379) connections and browse.
- Second click launches instantly (no download).
- Tray "DB client (Beekeeper)" launches the installed app.
If no display/network, state build-only.

- [ ] **Step 7: Commit**

```bash
git add src/state.ts src/ipc/commands.ts src/ui/views/dashboard.ts src/ui/events.ts
git commit -m "feat(ui): Dashboard + tray DB client launcher with download ring"
```

---

## Final verification
```bash
PATH="$HOME/.cargo/bin:$PATH" cargo test -p laralux-core 2>&1 | grep "test result: ok" | head -1
PATH="$HOME/.cargo/bin:$PATH" npm run build 2>&1 | tail -2
PATH="$HOME/.cargo/bin:$PATH" cargo build -p laralux-desktop -p laraluxctl 2>&1 | tail -2
grep -rn "as any\|@ts-nocheck" src/   # expect none
```
Expected: core tests pass (incl. the new beekeeper tests); frontend + cargo builds green; no `as any`/`@ts-nocheck`.
