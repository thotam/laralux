# Laralux — Event-Driven Realtime UI Design

**Date:** 2026-06-26
**Status:** Design (goal-directed); proceeding to plan + implementation.
**Goal:** Replace the frontend's 2-second polling with backend-pushed Tauri events so the UI reflects backend state in realtime — service status (incl. crashes) and the site list (incl. external `www/` changes) — with no perpetual re-render.

---

## 1. Context & current state

The frontend runs `setInterval(refresh, 2000)` (`dist/app.js`), and `refresh()` invokes `stack_status` + `list_sites` + `setup_status` every 2 s, then calls `render()` (full `app.innerHTML` replacement, guarded by a `lastSig` string-equality short-circuit). This means: a fixed 2 s latency for any change, constant IPC + HTML-string rebuild even when idle, and full-DOM churn whenever anything differs (the cause of recent scroll/focus issues). There are NO backend→frontend pushes except the `download-progress` event added earlier.

Backend facts: `AppState { orch: Mutex<Orchestrator>, paths, tld, starting }`. `Orchestrator::refresh()` marks any service whose process died as `Crashed`; `snapshot() -> Vec<ServiceStatus>` (`ServiceStatus` derives `Clone, PartialEq, Eq, Serialize`). `main.rs` has a `.setup(|app| {...})` hook with `app.handle()`. `LaraluxPaths::www()` and `sites_file()` exist. Service-mutating commands already RETURN the fresh snapshot, and the frontend applies it (instant feedback for user actions). The gap is **asynchronous/external** changes: a service crashing, or a `www/` folder added outside the app — only the 2 s poll currently catches those.

## 2. Approach (chosen: Tauri events, NOT websockets)

Push state changes from the backend to the webview over **Tauri's native event bridge** (`AppHandle::emit` → frontend `TAURI.event.listen`), the same mechanism already used for `download-progress`. A websocket is explicitly rejected: the webview and Rust run in one process connected by a native IPC channel; a WS server/client would add a TCP listener, a port, a second serialization path, and higher latency for zero benefit. Tauri events are the lowest-latency, zero-config push channel here.

Two realtime sources replace the poll:
1. **A backend service monitor thread** that periodically (~1 s) calls `orch.refresh()` + `snapshot()` and emits `services-changed` **only when the snapshot changed** — so crashes surface within ~1 s and the UI never re-renders while idle. (A server-side liveness check is still needed because a child process dying produces no OS push we can cheaply await across the orchestrator mutex; emitting only on change keeps the UI churn-free. Per-child waiter threads were rejected — they complicate handle ownership and risk double-reaping.)
2. **A filesystem watcher** (`notify` crate) on `~/laralux/www` + `~/laralux/sites.toml` that emits `sites-changed` (debounced) when a site folder/registry changes on disk — covering external edits in realtime.

The frontend drops the 2 s interval, keeps ONE startup `refresh()` to populate, and updates from these events. Service-mutating commands keep returning the snapshot (instant local feedback); the monitor is the safety net + crash/async path.

## 3. Architecture & components

### 3.1 `services-changed` — backend monitor thread (`src-tauri/src/main.rs` + a small module)

In `.setup(|app| …)`, spawn a detached thread holding `app.handle().clone()`:
```rust
let handle = app.handle().clone();
std::thread::spawn(move || {
    let mut last: Option<Vec<ServiceStatus>> = None;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let Some(state) = handle.try_state::<AppState>() else { continue };
        let snap = match state.orch.lock() {
            Ok(mut orch) => { orch.refresh(); orch.snapshot() }
            Err(_) => continue,
        };
        if last.as_ref() != Some(&snap) {
            let _ = handle.emit("services-changed", &snap);
            last = Some(snap);
        }
    }
});
```
- Emits only on change → zero idle UI churn; ~1 s worst-case crash latency.
- `use tauri::{Emitter, Manager};` for `emit`/`try_state`. `ServiceStatus` is already `Clone+PartialEq+Serialize`.
- Short critical section under the existing `orch` mutex (same `refresh()` the old poll did, moved server-side).
- Place the thread body in a helper `fn spawn_service_monitor(app: &tauri::AppHandle)` (in main.rs or a new `src-tauri/src/watch.rs`) for clarity.

### 3.2 `sites-changed` — filesystem watcher (`notify`)

Add `notify = "6"` (or the current 6.x) to `src-tauri/Cargo.toml`. In `.setup`:
```rust
let handle = app.handle().clone();
let paths = handle.state::<AppState>().paths.clone(); // LaraluxPaths is Clone
let www = paths.www();
let _ = std::fs::create_dir_all(&www); // ensure it exists to watch
let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
    if res.is_ok() {
        let _ = handle.emit("sites-changed", ());
    }
})?;
watcher.watch(&www, notify::RecursiveMode::NonRecursive)?;
let _ = watcher.watch(&paths.sites_file(), notify::RecursiveMode::NonRecursive); // file may not exist yet
// Keep the watcher alive for the app lifetime by moving it into a parked thread
// (notify::RecommendedWatcher is Send but not necessarily Sync, so app.manage()
// is unsuitable; a parked thread owns it without leaking via mem::forget).
std::thread::spawn(move || { let _keep = watcher; loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } });
```
- `www` is watched **non-recursively** (immediate subdirs are the sites — create/remove fires; deep writes inside a project don't spam).
- `sites_file()` (`sites.toml`) is watched so registry edits (link/proxy/domains) also fire.
- **Debounce:** wrap the emit so it fires at most once per ~300 ms (a `Mutex<Option<Instant>>` captured in the closure; `Instant::now()` is fine in `src-tauri`). notify bursts several events per change; the frontend re-fetch is cheap but debouncing avoids redundant IPC.
- The watcher object MUST be kept alive (dropping it stops watching): move it into a parked thread (`let _keep = watcher; loop { sleep(1h) }`) — `RecommendedWatcher` is `Send` but not necessarily `Sync`, so `app.manage` is unsuitable. `sites_file()` may not exist yet on first run — `let _ = watcher.watch(sites_file, …)` (ignore the error; `www` is the primary watch).
- Payload is `()` — the frontend re-fetches `list_sites` (the authoritative list) on receipt. (Keeps the event tiny; the list is small.)

### 3.3 Frontend — listeners replace the poll (`dist/app.js`)

- **Remove** `setInterval(refresh, 2000)`. Keep the single startup `refresh()` (populates services + sites + components once).
- Register listeners once at startup (next to the existing `download-progress` listener, guarded by `TAURI && TAURI.event && TAURI.event.listen`):
  - `services-changed` → `applyServices(e.payload)`; then `if (!state.modal) render();` (mirror the poll's modal guard so an open modal/input isn't disrupted).
  - `sites-changed` → `invoke("list_sites").then((s) => { state.sites = Array.isArray(s) ? s : []; if (!state.modal) render(); }).catch(()=>{})`.
- `applyServices` already exists and maps a `Vec<ServiceStatus>` into `state.services`.
- Components: keep the existing command-driven refresh (run_setup / install handlers already re-fetch `setup_status`); no poll needed (component presence rarely changes outside those commands). The startup `refresh()` covers the initial state.
- The recent scroll/focus-preservation in `render()` stays (defends any remaining full renders, e.g. on a `services-changed` during scroll).

## 4. Behavior, latency & error handling

- **Realtime:** user actions are instant (command return); crashes/external changes surface within ~1 s (monitor) / ~300 ms debounce (fs watch).
- **No idle churn:** the monitor emits only on change; the fs watch only fires on disk events; the frontend has no interval. When nothing happens, the UI does nothing.
- **Best-effort:** every `emit` is `let _ =`; a poisoned `orch` lock or a `try_state` miss skips that tick; a failed `list_sites` re-fetch is ignored. Backend pushes never crash a command.
- **Modal/input safety:** the `if (!state.modal) render()` guard (kept from the poll) plus the existing focus/scroll preservation in `render()` prevent event-driven renders from disrupting a user mid-input.
- **Watcher lifetime:** owned by Tauri state (`app.manage`), dropped on app exit.

## 5. Testing

- `notify`/monitor are runtime/threaded and **verified live** (start a service then `kill` its process → the UI flips to Crashed within ~1 s without interaction; create a folder in `~/laralux/www` → it appears within ~300 ms). No unit test for the thread/watcher wiring (I/O + timing).
- Frontend: `node --check dist/app.js`; live verification that removing the interval still yields a populated UI at startup and live updates on service/site changes; confirm scrolling during a change no longer jumps (combined with the prior fix).
- Core is unchanged (zero new core code); existing `cargo test -p laralux-core` stays green; `cargo build -p laralux-desktop` must compile with the new `notify` dep.

## 6. Out of scope (backlog)

- A filesystem watch on `~/laralux/bin` for realtime component-presence changes (rare; component refresh stays command-driven).
- Granular per-field events (e.g. per-service deltas) — the snapshot/`sites-changed` re-fetch is small and simpler.
- Replacing the command-return feedback path with pure event-sourcing (commands still return snapshots; YAGNI to remove).
- Configurable monitor interval / disabling the monitor.
- A websocket or external transport (explicitly rejected — Tauri events are optimal here).
