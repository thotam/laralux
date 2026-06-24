# nginx bind-capability preflight on Start Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Before starting nginx (Start All button, single-service start, tray Start All), re-apply `cap_net_bind_service` via setcap (pkexec) when the resolved nginx binary lacks it, so nginx can bind :80/:443 without the recurring "Permission denied" after apt upgrades.

**Architecture:** A core helper `ensure_nginx_bind_cap(paths, privileged)` resolves the nginx binary, checks the capability via unprivileged `getcap`, and only when it is positively missing calls `Privileged::setcap_nginx` (one pkexec prompt; best-effort). All three Start paths call it before starting nginx.

**Tech Stack:** Rust (laragon-core, zero Tauri deps), Tauri 2.

## Global Constraints

- `core` keeps **zero Tauri deps**.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD: failing test first for the pure parser.
- Best-effort: a setcap failure/cancel must NOT block startup; only escalate when the capability is positively missing (no prompt otherwise).
- Capability target is `resolve_bin("nginx", &[paths.bin()])` — the same binary the orchestrator spawns.
- Run core tests with `cargo test -p laragon-core`; build with `cargo build -p laragon-desktop`. If `cargo` isn't on PATH use `$HOME/.cargo/bin/cargo`.

---

### Task 1: Core — capability detection + preflight helper

**Files:**
- Modify: `core/src/bin.rs`
- Modify: `core/src/lib.rs` (re-export `ensure_nginx_bind_cap`)

**Interfaces:**
- Consumes: `resolve_bin`, `crate::paths::LaragonPaths`, `crate::privileged::Privileged`.
- Produces:
  - `pub fn getcap_indicates_cap(output: &str) -> bool`
  - `pub fn nginx_has_bind_cap(nginx_bin: &Path) -> bool`
  - `pub fn ensure_nginx_bind_cap(paths: &LaragonPaths, privileged: &dyn Privileged)`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/bin.rs`:

```rust
    #[test]
    fn getcap_output_detects_bind_cap() {
        assert!(getcap_indicates_cap("/usr/sbin/nginx cap_net_bind_service=ep"));
        assert!(getcap_indicates_cap("/usr/sbin/nginx cap_net_bind_service+ep"));
        assert!(!getcap_indicates_cap(""));
        assert!(!getcap_indicates_cap("/usr/sbin/nginx cap_chown=ep"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laragon-core bin`
Expected: FAIL to compile — `getcap_indicates_cap` not found.

- [ ] **Step 3: Implement the functions**

In `core/src/bin.rs`, add an import at the top (after the existing `use`):

```rust
use crate::paths::LaragonPaths;
use crate::privileged::Privileged;
```

Add the functions (after `detect_php_fpm_version` / `list_php_fpm_versions`):

```rust
/// True iff `getcap` output reports the net-bind capability.
pub fn getcap_indicates_cap(output: &str) -> bool {
    output.contains("cap_net_bind_service")
}

/// Whether the nginx binary already has `cap_net_bind_service`. Uses the
/// unprivileged `getcap`; if `getcap` can't be run, assume present (don't nag).
pub fn nginx_has_bind_cap(nginx_bin: &Path) -> bool {
    match std::process::Command::new("getcap").arg(nginx_bin).output() {
        Ok(out) => getcap_indicates_cap(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => true,
    }
}

/// Preflight before starting nginx: if the resolved nginx binary lacks the
/// net-bind capability, re-apply it via `setcap` (best-effort; a failure does
/// not block startup). No-op (no prompt) when the capability is already present.
pub fn ensure_nginx_bind_cap(paths: &LaragonPaths, privileged: &dyn Privileged) {
    if let Some(nginx) = resolve_bin("nginx", &[paths.bin()]) {
        if !nginx_has_bind_cap(&nginx) {
            let _ = privileged.setcap_nginx(&nginx);
        }
    }
}
```

- [ ] **Step 4: Re-export in lib.rs**

In `core/src/lib.rs`, change the `bin` re-export line to:

```rust
pub use bin::{ensure_nginx_bind_cap, list_php_fpm_versions};
```

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p laragon-core bin` then `cargo build -p laragon-core`
Expected: PASS — the parser test plus existing bin tests; crate builds (no unused-import warnings — `LaragonPaths`/`Privileged` are used by `ensure_nginx_bind_cap`).

- [ ] **Step 6: Commit**

```bash
git add core/src/bin.rs core/src/lib.rs
git commit -m "feat(core): ensure_nginx_bind_cap preflight (getcap + setcap if missing)"
```

---

### Task 2: Wire the preflight into all three Start paths

**Files:**
- Modify: `src-tauri/src/commands.rs` (`stack_start_all`, `service_start`)
- Modify: `src-tauri/src/main.rs` (tray `"start_all"` handler)

**Interfaces:**
- Consumes: `laragon_core::ensure_nginx_bind_cap`, `PkexecPrivileged`, `ServiceKind`.
- Produces: nginx capability is ensured before each Start path runs.

- [ ] **Step 1: Import the helper in commands.rs**

In `src-tauri/src/commands.rs`, add `ensure_nginx_bind_cap` to the `use laragon_core::{...}` imports (alongside the other core imports). `PkexecPrivileged` and `ServiceKind` are already imported.

- [ ] **Step 2: Preflight in `stack_start_all`**

In `stack_start_all`, inside the `spawn_blocking` closure, immediately before `let mut orch = state.orch.lock().map_err(lock_err)?;` add:

```rust
        // Ensure nginx can bind :80/:443 (re-setcap if a binary upgrade cleared it).
        ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged);
```

- [ ] **Step 3: Preflight in `service_start`**

In `service_start`, before `orch.start(kind)`, add a guard for nginx:

```rust
    if kind == ServiceKind::Nginx {
        ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged);
    }
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.start(kind).map_err(|e| e.to_string())?;
```

(Replace the existing first two lines of the body accordingly — the lock must be acquired AFTER the preflight so the brief pkexec prompt does not hold the orchestrator mutex. If the current body locks first, reorder so `ensure_nginx_bind_cap` runs before `state.orch.lock()`.)

- [ ] **Step 4: Preflight in the tray `start_all` handler (main.rs)**

In `src-tauri/src/main.rs`, in the tray `on_menu_event` `"start_all"` arm, call the preflight before starting. Change:

```rust
                    "start_all" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(mut orch) = state.orch.lock() {
                                let _ = orch.start_all();
                            }
                        }
                    }
```

to:

```rust
                    "start_all" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            laragon_core::ensure_nginx_bind_cap(
                                &state.paths,
                                &laragon_core::PkexecPrivileged,
                            );
                            if let Ok(mut orch) = state.orch.lock() {
                                let _ = orch.start_all();
                            }
                        }
                    }
```

(The preflight runs before `state.orch.lock()`, so the prompt — only shown when the capability is missing — does not hold the mutex.)

- [ ] **Step 5: Build**

Run: `cargo build -p laragon-desktop`
Expected: PASS — compiles cleanly (no unused imports/warnings).

- [ ] **Step 6: Manual verification (live)**

Drop the capability, then Start: `sudo setcap -r /usr/sbin/nginx` (remove caps) → in the app click **Start All** → a pkexec prompt appears once; after authorizing, `getcap /usr/sbin/nginx` shows `cap_net_bind_service=ep` and nginx binds :80/:443 (no "Permission denied"). Start again with the cap present → no prompt. (Capability/pkexec can't run in unit tests — human-verified.)

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs
git commit -m "feat(desktop): preflight nginx bind capability on Start (all start paths)"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 `getcap_indicates_cap` + `nginx_has_bind_cap` (getcap, conservative default) → Task 1. ✓
- §3.2 `ensure_nginx_bind_cap(paths, &dyn Privileged)` resolve→check→setcap best-effort + re-export → Task 1. ✓
- §3.3 wiring into stack_start_all + service_start(Nginx) + tray start_all → Task 2. ✓
- §4 no-prompt-when-present / best-effort / correct target / self-healing → Task 1 logic + Task 2 placement. ✓
- §6 testing: pure parser unit test (Task 1); live verification (Task 2 Step 6). ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows complete code. The one manual step (Task 2 Step 6) is an explicit human-verified gate (setcap/pkexec/getcap can't run in unit tests).

**3. Type consistency:**
- `ensure_nginx_bind_cap(&LaragonPaths, &dyn Privileged)` (Task 1) called identically from `stack_start_all`, `service_start`, and the tray handler (Task 2) with `&state.paths` + `&PkexecPrivileged`. ✓
- `setcap_nginx(&Path)` (existing) invoked by `ensure_nginx_bind_cap` with the `resolve_bin` result. ✓
- `resolve_bin("nginx", &[paths.bin()]) -> Option<PathBuf>` matches the orchestrator's spawn resolution. ✓
- lib re-export adds `ensure_nginx_bind_cap` next to `list_php_fpm_versions`. ✓

**Note:** `service_start` becomes a (rare) source of a pkexec prompt for nginx; it stays synchronous per spec, and the prompt only appears when the capability is actually missing. The preflight is ordered before `state.orch.lock()` in all paths so the prompt never holds the orchestrator mutex.
