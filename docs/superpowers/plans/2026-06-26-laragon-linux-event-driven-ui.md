# Event-Driven Realtime UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the frontend 2 s poll with backend-pushed Tauri events: a service monitor thread (`services-changed`, crash-aware) and a `notify` filesystem watcher (`sites-changed`), with the frontend listening instead of polling.

**Architecture:** Two detached backend threads spawned in `main.rs` `.setup`: one that calls `orch.refresh()`+`snapshot()` every ~1 s and emits `services-changed` only when the snapshot changes; one `notify` watcher on `~/laragon/www` + `sites.toml` that emits `sites-changed` (debounced). The frontend drops `setInterval(refresh,2000)`, keeps one startup `refresh()`, and updates from the events.

**Tech Stack:** Tauri 2 (Emitter/Manager + events), `notify` crate, vanilla JS. `laragon-core` is NOT touched (zero new core code).

## Global Constraints

- Use **Tauri events** (`AppHandle::emit` / `TAURI.event.listen`) — NOT websockets.
- `services-changed` emits ONLY when the snapshot differs from the last (no idle churn); `ServiceStatus` already derives `Clone+PartialEq+Serialize`.
- `notify` watcher: `www` non-recursive + `sites_file` non-recursive; debounce emits to ~once/300 ms; keep the watcher alive in a parked thread (it's `Send`, not `Sync` → not `app.manage`-able).
- Every `emit` is best-effort (`let _ =`); a poisoned lock / missing state skips a tick; never crash a command.
- Frontend keeps the `if (!state.modal) render()` guard and the existing scroll/focus preservation in `render()`.
- Commits MUST NOT contain a `Co-Authored-By` trailer. Build: `cargo build -p laragon-desktop`; `cargo test -p laragon-core` stays green; `node --check dist/app.js`. cargo fallback `$HOME/.cargo/bin/cargo`; node fallback `$HOME/.nvm/versions/node/v24.16.0/bin/node`.

---

### Task 1: backend service monitor → `services-changed`

**Files:** Modify `src-tauri/src/main.rs` (spawn in `.setup`); add a helper (inline in main.rs or `src-tauri/src/watch.rs`).

**Interfaces:** emits `services-changed` with payload `Vec<ServiceStatus>`.

- [ ] **Step 1: Add the monitor spawn.** In `src-tauri/src/main.rs`, ensure `use tauri::{Emitter, Manager};` is present (add what's missing — `Emitter` may already be there from earlier work). Inside the `.setup(|app| { ... Ok(()) })` closure, before `Ok(())`, add:

```rust
            // Realtime service status: poll liveness server-side every ~1s and
            // push `services-changed` ONLY when the snapshot actually changes, so
            // crashes surface within ~1s and the UI never re-renders while idle.
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let mut last: Option<Vec<laragon_core::ServiceStatus>> = None;
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
            }
```
(`laragon_core::ServiceStatus` is re-exported; `AppState` is imported via `use commands::{build_state, AppState};`. `app.handle()` returns the `AppHandle`; `.clone()` it for the thread.)

- [ ] **Step 2: Build** — `cargo build -p laragon-desktop` clean; `cargo test -p laragon-core` still green (unchanged).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(desktop): service monitor thread emits services-changed on transition"
```

---

### Task 2: filesystem watcher → `sites-changed`

**Files:** Modify `src-tauri/Cargo.toml` (add `notify`), `src-tauri/src/main.rs`.

**Interfaces:** emits `sites-changed` (payload `()`).

- [ ] **Step 1: Add the dependency.** In `src-tauri/Cargo.toml` `[dependencies]`, add `notify = "6"` (use the latest 6.x that resolves; if 6 fails to resolve, use the current major that `cargo` accepts and note it). Run `cargo build -p laragon-desktop` to fetch/lock it.

- [ ] **Step 2: Spawn the watcher.** In `.setup`, after the monitor block, add:

```rust
            // Realtime site list: watch ~/laragon/www (non-recursive: immediate
            // subdirs are sites) and sites.toml; push `sites-changed` (debounced)
            // so external folder/registry edits appear without polling.
            {
                let handle = app.handle().clone();
                let paths = handle.state::<AppState>().paths.clone();
                let www = paths.www();
                let _ = std::fs::create_dir_all(&www);
                let last = std::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));
                if let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if res.is_ok() {
                        let mut l = last.lock().unwrap();
                        if l.elapsed() >= std::time::Duration::from_millis(300) {
                            *l = std::time::Instant::now();
                            let _ = handle.emit("sites-changed", ());
                        }
                    }
                }) {
                    use notify::Watcher;
                    let _ = watcher.watch(&www, notify::RecursiveMode::NonRecursive);
                    let _ = watcher.watch(&paths.sites_file(), notify::RecursiveMode::NonRecursive);
                    // Keep the watcher alive for the app lifetime (Send, not Sync).
                    std::thread::spawn(move || { let _keep = watcher; loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } });
                }
            }
```
(`LaragonPaths` derives `Clone`; `paths.www()` and `paths.sites_file()` exist. The closure captures `handle` + `last` by move.)

- [ ] **Step 3: Build** — `cargo build -p laragon-desktop` clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/main.rs
git commit -m "feat(desktop): notify watcher emits sites-changed on www/registry changes"
```

---

### Task 3: frontend listens; remove the 2 s poll

**Files:** Modify `dist/app.js`.

**Interfaces:** consumes `services-changed` (Vec<ServiceStatus>) and `sites-changed` (()).

- [ ] **Step 1: Register listeners + drop the interval.** In `dist/app.js`, find the startup block `refresh(); setInterval(refresh, 2000);` (near the end, ~line 1315). Replace the `setInterval(...)` line so the poll is removed and event listeners are registered (keep the one-time `refresh()`):

```js
  refresh();
  if (TAURI && TAURI.event && TAURI.event.listen) {
    TAURI.event.listen("services-changed", (e) => {
      applyServices(e.payload);
      if (!state.modal) render();
    });
    TAURI.event.listen("sites-changed", () => {
      invoke("list_sites").then((s) => {
        state.sites = Array.isArray(s) ? s : [];
        if (!state.modal) render();
      }).catch(() => {});
    });
  }
```
(`applyServices` and the `download-progress` listener already exist; this sits alongside them. Do NOT remove the `download-progress` listener.)

- [ ] **Step 2: Syntax check** — `node --check dist/app.js` → exit 0.

- [ ] **Step 3: Manual verification (live)** — `cargo run -p laragon-desktop`: (a) start a service, then `kill` its OS process → the row flips to Crashed within ~1 s with no interaction; (b) `mkdir ~/laragon/www/<name>` → the site appears within ~300 ms; (c) idle for a while → no perpetual re-render (the UI is static when nothing changes); (d) scroll the page during a change → position is preserved.

- [ ] **Step 4: Commit**

```bash
git add dist/app.js
git commit -m "feat(desktop): event-driven UI — listen for services/sites changes, drop 2s poll"
```

---

## Self-Review

**1. Spec coverage:** services-changed monitor (T1, §3.1); sites-changed notify watcher (T2, §3.2); frontend listeners + remove poll (T3, §3.3). Components stay command-driven (§3.3, in scope as "no change"). Out-of-scope (bin watch, websockets) excluded.

**2. Placeholder scan:** No TBD; the `notify = "6"` version has a concrete fallback instruction. All code blocks are complete.

**3. Type consistency:** `services-changed` payload is `Vec<ServiceStatus>` (T1) consumed by `applyServices(e.payload)` (T3); `sites-changed` is `()` (T2) → T3 re-fetches `list_sites`; `AppState`/`paths`/`orch`/`snapshot`/`refresh` names match the existing backend. The watcher-keep-alive (parked thread) matches the spec's Send-not-Sync note.
