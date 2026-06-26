# Laralux — Plan 3a: Tauri GUI Shell, Tray & Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Tauri 2 desktop app with a system-tray icon and a dashboard window that starts/stops the stack and lists sites, by owning a long-lived `Orchestrator` for the app's lifetime.

**Architecture:** A new `src-tauri` crate (`laralux-desktop`) joins the Cargo workspace. It holds the `laralux_core::Orchestrator` in Tauri managed state behind a `Mutex`, exposes `#[tauri::command]` IPC functions the frontend calls, draws a tray menu, and stops the stack on exit. A static HTML/JS frontend (`dist/`) uses the `window.__TAURI__` global (no JS bundler). `core` gains serde derives and a `snapshot()` so service state crosses the IPC boundary.

**Tech Stack:** Rust + Tauri 2 (`tray-icon`, `image-png` features), `serde`/`serde_json`, vanilla HTML/JS frontend, reuses `laralux_core` (Plan 1 + 2).

## Global Constraints

- App identifier: `com.laralux.linux`. Product/window title: `Laralux`. Window label: `main`.
- Frontend is static files in `dist/`; `tauri.conf.json` sets `build.frontendDist = "../dist"` and `app.withGlobalTauri = true` (frontend calls `window.__TAURI__.core.invoke`). No npm/bundler.
- The `Orchestrator` lives for the app's lifetime in managed state; the app MUST `stop_all()` before exiting so no child processes are orphaned.
- Tray menu items (exact labels): `Start All`, `Stop All`, `Dashboard`, `Quit`.
- IPC command names (snake_case): `stack_status`, `stack_start_all`, `stack_stop_all`, `service_start`, `service_stop`, `list_sites`. Commands return `Result<_, String>` (errors stringified).
- `core` keeps zero Tauri deps; all Tauri code lives in `src-tauri`.
- Linux system prerequisites for Tauri 2 (webkit2gtk 4.1, ayatana appindicator) must be installed before the app compiles — this is a one-time human step (needs interactive sudo).
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD applies to `core` changes (Task 1). GUI/tray/frontend tasks (2–5) are integration glue verified by `cargo build` + a manual smoke checklist; follow the steps exactly and report build output.

---

### Task 1: Core — serde derives + status snapshot

**Files:**
- Modify: `core/src/service/mod.rs` (derive serde on `ServiceKind`, `ServiceState`)
- Modify: `core/src/sites.rs` (derive `Serialize` on `Site`)
- Modify: `core/src/orchestrator.rs` (add `ServiceStatus` + `Orchestrator::snapshot()`)
- Modify: `core/Cargo.toml` (add `serde_json` dev-dependency)
- Modify: `core/src/lib.rs` (re-export `ServiceStatus`)

**Interfaces:**
- Consumes: `ServiceKind`, `ServiceState`, `Orchestrator::{start_order, state}` (Plan 1).
- Produces:
  - `ServiceKind` and `ServiceState` derive `serde::Serialize + serde::Deserialize` (serialize as their variant names, e.g. `"Nginx"`, `"Running"`).
  - `Site` derives `serde::Serialize`.
  - `struct ServiceStatus { pub kind: ServiceKind, pub state: ServiceState }` (derives `Serialize, Deserialize, Clone, Debug, PartialEq, Eq`).
  - `Orchestrator::snapshot(&self) -> Vec<ServiceStatus>` — one entry per registered service in `start_order()`, each carrying its current `state(kind)`.

- [ ] **Step 1: Add the serde_json dev-dependency**

In `core/Cargo.toml`, add a dev-dependencies section (after `[dependencies]`):

```toml
[dev-dependencies]
serde_json = "1"
```

- [ ] **Step 2: Write the failing test**

Add to the `tests` module in `core/src/orchestrator.rs` (reuses the existing `Dummy` test service + `FakeSpawner`):

```rust
    #[test]
    fn snapshot_lists_services_with_states() {
        let spawner = crate::process::FakeSpawner::new();
        let services: Vec<Box<dyn Service>> =
            vec![Box::new(Dummy { kind: ServiceKind::Redis, name: "redis-server" })];
        let mut o = Orchestrator::new(
            LaraluxPaths::new("/tmp/lara".into()),
            services,
            Box::new(spawner),
        );

        let before = o.snapshot();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].kind, ServiceKind::Redis);
        assert_eq!(before[0].state, ServiceState::Stopped);

        o.start(ServiceKind::Redis).unwrap();
        let after = o.snapshot();
        assert_eq!(after[0].state, ServiceState::Running);
    }

    #[test]
    fn service_status_serializes_to_variant_names() {
        let s = ServiceStatus { kind: ServiceKind::Nginx, state: ServiceState::Running };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"Nginx","state":"Running"}"#);
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p laralux-core orchestrator`
Expected: FAIL — `cannot find type ServiceStatus` / `no method named snapshot`.

- [ ] **Step 4: Add serde derives to the enums**

In `core/src/service/mod.rs`, add the import near the top (after the existing `use` lines):

```rust
use serde::{Deserialize, Serialize};
```

Change the `ServiceKind` derive line to:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
```

Change the `ServiceState` derive line to:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
```

- [ ] **Step 5: Derive Serialize on Site**

In `core/src/sites.rs`, change the `Site` derive line to:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
```

- [ ] **Step 6: Add ServiceStatus + snapshot**

In `core/src/orchestrator.rs`, add near the top after the existing `use` lines:

```rust
use serde::{Deserialize, Serialize};
```

Add this struct above `pub struct Orchestrator`:

```rust
/// A serializable point-in-time view of one service.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ServiceStatus {
    pub kind: ServiceKind,
    pub state: ServiceState,
}
```

Add this method inside `impl Orchestrator`:

```rust
    /// Snapshot of every registered service in dependency-start order.
    pub fn snapshot(&self) -> Vec<ServiceStatus> {
        self.start_order()
            .into_iter()
            .map(|kind| ServiceStatus { kind, state: self.state(kind) })
            .collect()
    }
```

- [ ] **Step 7: Re-export ServiceStatus**

In `core/src/lib.rs`, add to the re-export block:

```rust
pub use orchestrator::ServiceStatus;
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p laralux-core`
Expected: PASS — all prior tests plus the 2 new ones.

- [ ] **Step 9: Commit**

```bash
git add core/
git commit -m "feat(core): add serde derives and Orchestrator::snapshot"
```

---

### Task 2: Tauri app scaffold + window

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/icons/icon.png` (generated)
- Create: `src-tauri/src/main.rs`
- Create: `dist/index.html`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Produces: a buildable `laralux-desktop` binary that opens one empty window. Later tasks add state, commands, tray.

**Human prerequisite (run once, interactive sudo — NOT the implementer subagent):**
Install Tauri 2 Linux system dependencies:
```bash
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```
If these are absent, `cargo build -p laralux-desktop` fails with a `webkit2gtk-4.1` / `glib` pkg-config error; that means the prerequisite has not been run yet — report BLOCKED with the error so the human can install them.

- [ ] **Step 1: Add the crate to the workspace**

In the root `Cargo.toml`, change the members line to:

```toml
members = ["core", "laraluxctl", "src-tauri"]
```

- [ ] **Step 2: Create `src-tauri/Cargo.toml`**

```toml
[package]
name = "laralux-desktop"
edition.workspace = true
version.workspace = true
license.workspace = true

[[bin]]
name = "laralux-desktop"
path = "src/main.rs"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon", "image-png"] }
serde = { workspace = true }
serde_json = "1"
laralux-core = { path = "../core" }
```

- [ ] **Step 3: Create `src-tauri/build.rs`**

```rust
fn main() {
    tauri_build::build();
}
```

- [ ] **Step 4: Create `src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Laralux",
  "version": "0.1.0",
  "identifier": "com.laralux.linux",
  "build": {
    "frontendDist": "../dist"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "label": "main",
        "title": "Laralux",
        "width": 900,
        "height": 600,
        "visible": true
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": ["deb"],
    "icon": ["icons/icon.png"]
  }
}
```

- [ ] **Step 5: Create the capability `src-tauri/capabilities/default.json`**

```json
{
  "identifier": "default",
  "description": "Default capability for the main window",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

- [ ] **Step 6: Generate the icon `src-tauri/icons/icon.png`**

Run this (python3 stdlib only — emits a valid 32×32 RGBA PNG):

```bash
mkdir -p src-tauri/icons
python3 - <<'PY'
import zlib, struct
def chunk(t, d):
    c = t + d
    return struct.pack(">I", len(d)) + c + struct.pack(">I", zlib.crc32(c) & 0xffffffff)
w = h = 32
raw = b""
for _ in range(h):
    raw += b"\x00" + bytes([60, 90, 200, 255]) * w
png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(raw))
       + chunk(b"IEND", b""))
open("src-tauri/icons/icon.png", "wb").write(png)
print("wrote", len(png), "bytes")
PY
```
Expected: prints `wrote <N> bytes` and creates `src-tauri/icons/icon.png`.

- [ ] **Step 7: Create `src-tauri/src/main.rs` (minimal app)**

```rust
// Prevent a console window on Windows release builds (no-op on Linux).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Laralux");
}
```

- [ ] **Step 8: Create `dist/index.html` (placeholder)**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Laralux</title>
  </head>
  <body>
    <h1>Laralux</h1>
    <p>Dashboard loading…</p>
  </body>
</html>
```

- [ ] **Step 9: Build the app**

Run: `cargo build -p laralux-desktop`
Expected: PASS — compiles. (If it fails on `webkit2gtk-4.1`/`glib` pkg-config, the human prerequisite above has not been installed — report BLOCKED with the error.)

- [ ] **Step 10: Commit**

```bash
git add src-tauri dist Cargo.toml
git commit -m "feat(desktop): scaffold Tauri 2 app with empty window"
```

- [ ] **Step 11: Manual smoke (human, optional now)**

`cargo run -p laralux-desktop` should open a 900×600 window titled "Laralux". Note in the report that the live window check is a human step.

---

### Task 3: App state + IPC commands

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `laralux_core::{Config, LaraluxPaths, Orchestrator, RealSpawner, build_services, ServiceKind, ServiceStatus, scan_sites, Site}`.
- Produces:
  - `struct AppState { orch: std::sync::Mutex<Orchestrator>, paths: LaraluxPaths, tld: String }`
  - Commands (all `#[tauri::command]`, in `commands.rs`):
    - `stack_status(state) -> Result<Vec<ServiceStatus>, String>`
    - `stack_start_all(state) -> Result<Vec<ServiceStatus>, String>`
    - `stack_stop_all(state) -> Result<Vec<ServiceStatus>, String>`
    - `service_start(state, kind: ServiceKind) -> Result<Vec<ServiceStatus>, String>`
    - `service_stop(state, kind: ServiceKind) -> Result<Vec<ServiceStatus>, String>`
    - `list_sites(state) -> Result<Vec<Site>, String>`
  - `commands::build_state() -> AppState` (loads config, ensures dirs, builds orchestrator with `RealSpawner`).

- [ ] **Step 1: Create `src-tauri/src/commands.rs`**

```rust
use laralux_core::{
    build_services, scan_sites, Config, LaraluxPaths, Orchestrator, RealSpawner, ServiceKind,
    ServiceStatus, Site,
};
use std::sync::Mutex;

/// Shared, app-lifetime state. The orchestrator owns the running child
/// processes, so it must live as long as the app and be stopped on exit.
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub paths: LaraluxPaths,
    pub tld: String,
}

/// Build the managed state from the on-disk config.
pub fn build_state() -> AppState {
    let paths = LaraluxPaths::new(LaraluxPaths::default_root());
    let config = Config::load(&paths.config_file()).unwrap_or_default();
    let _ = paths.ensure_dirs();
    let orch = Orchestrator::new(paths.clone(), build_services(&config, &paths), Box::new(RealSpawner));
    AppState { orch: Mutex::new(orch), paths, tld: config.tld }
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> String {
    "internal lock poisoned".to_string()
}

#[tauri::command]
pub fn stack_status(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.refresh();
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn stack_start_all(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.start_all().map_err(|e| e.to_string())?;
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn stack_stop_all(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.stop_all();
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn service_start(
    state: tauri::State<AppState>,
    kind: ServiceKind,
) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.start(kind).map_err(|e| e.to_string())?;
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn service_stop(
    state: tauri::State<AppState>,
    kind: ServiceKind,
) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.stop(kind).map_err(|e| e.to_string())?;
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn list_sites(state: tauri::State<AppState>) -> Result<Vec<Site>, String> {
    scan_sites(&state.paths, &state.tld).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: Wire state, handler, and graceful shutdown in `src-tauri/src/main.rs`**

Replace `src-tauri/src/main.rs` with:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::{build_state, AppState};
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(build_state())
        .invoke_handler(tauri::generate_handler![
            commands::stack_status,
            commands::stack_start_all,
            commands::stack_stop_all,
            commands::service_start,
            commands::service_stop,
            commands::list_sites,
        ])
        .on_window_event(|window, event| {
            // Stop the stack cleanly when the last window is destroyed.
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    if let Ok(mut orch) = state.orch.lock() {
                        orch.stop_all();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Laralux");
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p laralux-desktop`
Expected: PASS — compiles (commands + state wired).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src
git commit -m "feat(desktop): add app state and stack/sites IPC commands"
```

---

### Task 4: System-tray icon + menu

**Files:**
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `AppState`, the commands' underlying orchestrator (via `AppState`).
- Produces: a tray icon with a menu (`Start All`, `Stop All`, `Dashboard`, `Quit`) wired in the Tauri `setup` hook; `Start All`/`Stop All` drive the orchestrator, `Dashboard` shows/focuses the main window, `Quit` stops the stack then exits.

- [ ] **Step 1: Replace `src-tauri/src/main.rs` with the tray-enabled version**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::{build_state, AppState};
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};

fn main() {
    tauri::Builder::default()
        .manage(build_state())
        .invoke_handler(tauri::generate_handler![
            commands::stack_status,
            commands::stack_start_all,
            commands::stack_stop_all,
            commands::service_start,
            commands::service_stop,
            commands::list_sites,
        ])
        .setup(|app| {
            let start = MenuItemBuilder::with_id("start_all", "Start All").build(app)?;
            let stop = MenuItemBuilder::with_id("stop_all", "Stop All").build(app)?;
            let dashboard = MenuItemBuilder::with_id("dashboard", "Dashboard").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&start, &stop, &dashboard, &quit])
                .build()?;

            let icon = Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "start_all" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(mut orch) = state.orch.lock() {
                                let _ = orch.start_all();
                            }
                        }
                    }
                    "stop_all" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(mut orch) = state.orch.lock() {
                                orch.stop_all();
                            }
                        }
                    }
                    "dashboard" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "quit" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(mut orch) = state.orch.lock() {
                                orch.stop_all();
                            }
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    if let Ok(mut orch) = state.orch.lock() {
                        orch.stop_all();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Laralux");
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p laralux-desktop`
Expected: PASS — compiles with the tray.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(desktop): add system-tray icon and menu"
```

- [ ] **Step 4: Manual smoke (human)**

`cargo run -p laralux-desktop`: a tray icon appears; its menu shows Start All / Stop All / Dashboard / Quit; `Quit` closes the app. Note this as a human verification step in the report.

---

### Task 5: Frontend dashboard

**Files:**
- Modify: `dist/index.html`
- Create: `dist/main.js`
- Create: `dist/styles.css`

**Interfaces:**
- Consumes (via `window.__TAURI__.core.invoke`): `stack_status`, `stack_start_all`, `stack_stop_all`, `service_start`, `service_stop`, `list_sites`. Each status command returns `[{ kind, state }]`; `list_sites` returns `[{ name, root, hostname }]`.

- [ ] **Step 1: Replace `dist/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Laralux</title>
    <link rel="stylesheet" href="styles.css" />
  </head>
  <body>
    <header>
      <h1>Laralux</h1>
      <div class="actions">
        <button id="start-all">Start All</button>
        <button id="stop-all">Stop All</button>
      </div>
    </header>
    <main>
      <section>
        <h2>Services</h2>
        <table>
          <tbody id="services"></tbody>
        </table>
      </section>
      <section>
        <h2>Sites</h2>
        <ul id="sites"></ul>
      </section>
    </main>
    <script src="main.js"></script>
  </body>
</html>
```

- [ ] **Step 2: Create `dist/main.js`**

```javascript
const { invoke } = window.__TAURI__.core;

const servicesEl = document.querySelector("#services");
const sitesEl = document.querySelector("#sites");

function stateClass(state) {
  return state === "Running" ? "running" : state === "Crashed" ? "crashed" : "stopped";
}

function renderServices(list) {
  servicesEl.innerHTML = "";
  for (const { kind, state } of list) {
    const tr = document.createElement("tr");

    const nameTd = document.createElement("td");
    nameTd.textContent = kind;

    const stateTd = document.createElement("td");
    stateTd.textContent = state;
    stateTd.className = stateClass(state);

    const actionTd = document.createElement("td");
    const btn = document.createElement("button");
    const running = state === "Running";
    btn.textContent = running ? "Stop" : "Start";
    btn.addEventListener("click", async () => {
      const cmd = running ? "service_stop" : "service_start";
      try {
        renderServices(await invoke(cmd, { kind }));
      } catch (e) {
        alert(`${cmd} failed: ${e}`);
      }
    });
    actionTd.appendChild(btn);

    tr.append(nameTd, stateTd, actionTd);
    servicesEl.appendChild(tr);
  }
}

function renderSites(list) {
  sitesEl.innerHTML = "";
  if (list.length === 0) {
    const li = document.createElement("li");
    li.textContent = "No sites in www/";
    sitesEl.appendChild(li);
    return;
  }
  for (const site of list) {
    const li = document.createElement("li");
    const a = document.createElement("a");
    a.href = `https://${site.hostname}`;
    a.target = "_blank";
    a.textContent = `${site.name} — https://${site.hostname}`;
    li.appendChild(a);
    sitesEl.appendChild(li);
  }
}

async function refresh() {
  try {
    renderServices(await invoke("stack_status"));
    renderSites(await invoke("list_sites"));
  } catch (e) {
    console.error(e);
  }
}

document.querySelector("#start-all").addEventListener("click", async () => {
  try {
    renderServices(await invoke("stack_start_all"));
  } catch (e) {
    alert(`start failed: ${e}`);
  }
});

document.querySelector("#stop-all").addEventListener("click", async () => {
  try {
    renderServices(await invoke("stack_stop_all"));
  } catch (e) {
    alert(`stop failed: ${e}`);
  }
});

refresh();
setInterval(refresh, 2000);
```

- [ ] **Step 3: Create `dist/styles.css`**

```css
* { box-sizing: border-box; }
body {
  font-family: system-ui, sans-serif;
  margin: 0;
  color: #1c1e26;
  background: #f6f7f9;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.5rem;
  background: #fff;
  border-bottom: 1px solid #e3e6ea;
}
h1 { font-size: 1.25rem; margin: 0; }
.actions button, td button {
  cursor: pointer;
  border: 1px solid #c4c9d0;
  background: #fff;
  border-radius: 6px;
  padding: 0.35rem 0.8rem;
  margin-left: 0.5rem;
}
.actions button:hover, td button:hover { background: #eef1f5; }
main { padding: 1.5rem; display: grid; gap: 2rem; }
h2 { font-size: 1rem; color: #5b6472; }
table { width: 100%; border-collapse: collapse; }
td { padding: 0.5rem; border-bottom: 1px solid #e3e6ea; }
td:first-child { font-weight: 600; }
.running { color: #1a8a4b; font-weight: 600; }
.stopped { color: #8a8f98; }
.crashed { color: #c0392b; font-weight: 600; }
#sites { list-style: none; padding: 0; }
#sites li { padding: 0.4rem 0; }
#sites a { color: #2a6df5; text-decoration: none; }
#sites a:hover { text-decoration: underline; }
```

- [ ] **Step 4: Build (frontend is static; verify the app still compiles)**

Run: `cargo build -p laralux-desktop`
Expected: PASS (frontend files are bundled at runtime; no Rust change).

- [ ] **Step 5: Commit**

```bash
git add dist
git commit -m "feat(desktop): add dashboard frontend (services + sites)"
```

- [ ] **Step 6: Manual smoke (human)**

`cargo run -p laralux-desktop`: the dashboard lists the five services (all `Stopped` initially) with Start/Stop buttons and a Start All / Stop All header; the Sites section lists `www/` projects with `https://<name>.dev` links; status refreshes every 2s. With the stack installed, `Start All` flips services to `Running`. Record this as a human verification step.

---

## Self-Review

**1. Spec coverage (Plan 3a scope = GUI shell + tray + dashboard):**
- GUI desktop + tray (spec §2 framework Tauri, §7 Phase-1 tray + main window) → Tasks 2, 4 ✓
- Start/Stop All + per-service + status display (spec §7 Phase-1 "Start/Stop All, trạng thái từng service") → Tasks 3, 5 ✓
- Long-lived orchestrator owning processes, stop-on-exit / no orphans (spec §4) → Tasks 3, 4 ✓ (resolves the Plan-1/2 stateless-`status` limitation since the GUI process persists state)
- Sites list with `https://*.dev` links (spec §6) → Tasks 3, 5 ✓
- serde across IPC boundary → Task 1 ✓ (TDD)
- **Correctly deferred to Plan 3b:** setup wizard, apt install of the stack, mkcert CA install, `setcap`, datadir init, first-run detection, and GUI-driven site sync with privilege escalation. Tray "Start All" here assumes the stack is installed and sites already synced (via `laraluxctl`/Plan 3b).

**2. Placeholder scan:** No "TBD/handle edge cases". Tasks 2–5 are explicitly build-and-manual-smoke (GUI glue is not unit-testable); the manual steps are concrete checklists, and the human-only steps (system-dep install, visible-window/tray checks) are called out as such, not left vague.

**3. Type consistency:** `ServiceStatus { kind, state }` (Task 1) is the exact shape the frontend consumes (Task 5) and the commands return (Task 3). `ServiceKind` serializes as variant names (`"Nginx"`…) — the frontend sends `{ kind }` back to `service_start`/`service_stop` and Tauri deserializes it via the same serde derive. `Site { name, root, hostname }` serialize fields match `main.js`'s `site.name`/`site.hostname`. Command names (`stack_status`, `stack_start_all`, `stack_stop_all`, `service_start`, `service_stop`, `list_sites`) are identical across `commands.rs`, `generate_handler!`, and `main.js`. `AppState` field `orch`/`paths`/`tld` consistent across Tasks 3–4. Tray menu IDs (`start_all`/`stop_all`/`dashboard`/`quit`) match their `on_menu_event` arms.

**Note on deferred findings:** Plan-2's logged item — nginx binary resolution divergence between `setup-perms` (setcap target) and the bare `nginx` spawn — is unchanged here and belongs to Plan 3b's setup wizard, which owns `setcap` and stack installation.
