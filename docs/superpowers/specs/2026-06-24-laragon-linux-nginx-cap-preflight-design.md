# Laragon Linux — nginx bind-capability preflight on Start Design Spec

**Date:** 2026-06-24
**Status:** Design approved, pending spec review
**Goal:** Before starting nginx (Start All button, single-service start, and the tray "Start All"), check whether the resolved nginx binary has `cap_net_bind_service`; if it does not, re-apply it via `setcap` (pkexec) so nginx can bind :80/:443. This prevents the `bind() to 0.0.0.0:80 failed (13: Permission denied)` failure that recurs whenever an apt upgrade replaces the nginx binary and clears its file capabilities.

---

## 1. Context & current state

The Setup wizard applies `setcap cap_net_bind_service=+ep` to the resolved nginx binary once (`run_setup` step 4, via `Privileged::setcap_nginx`). nginx runs as the non-root user, so without that capability it cannot bind privileged ports. File capabilities are cleared when the binary is replaced (e.g. `apt upgrade nginx`), after which Start fails with a permission error and the user must re-run Setup or `setcap` manually. The orchestrator resolves and spawns nginx via `bin::resolve_or_name("nginx", &[paths.bin()])` (so the capability must be on that exact binary). `Privileged::setcap_nginx(&Path)` already exists (sudo/pkexec/fake impls). Reading capabilities (`getcap`) is unprivileged.

Three Start entry points exist: `commands::stack_start_all` (GUI Start All, runs in `spawn_blocking`), `commands::service_start` (per-service), and the tray `"start_all"` menu handler in `main.rs` (calls `orch.start_all()` directly on the main thread).

## 2. Approach (chosen: detect-then-setcap preflight, shared core helper)

A core helper `ensure_nginx_bind_cap(paths, privileged)` resolves the nginx binary, checks for `cap_net_bind_service` via `getcap`, and only when it is **positively missing** calls `privileged.setcap_nginx(nginx)` (one pkexec prompt). It is best-effort: a cancelled/failed setcap does not block startup (nginx then emits its existing clear error). Normal runs (capability present) prompt for nothing. All three Start paths call this helper before starting nginx.

Rejected:
- **Always setcap on every Start** — a pkexec prompt on every start is unacceptable.
- **Reading the `security.capability` xattr directly** — needs a new crate / binary xattr parsing; `getcap` is already present (ships with `setcap` in `libcap2-bin`) and simpler.

## 3. Architecture & components

### 3.1 `core/src/bin.rs` — capability detection

- `pub fn getcap_indicates_cap(output: &str) -> bool` (pure, unit-tested): `true` iff `output` contains the substring `cap_net_bind_service`.
- `pub fn nginx_has_bind_cap(nginx_bin: &Path) -> bool`: run `getcap <nginx_bin>` (unprivileged); if the command runs successfully, return `getcap_indicates_cap(stdout)`; if `getcap` cannot be executed at all (not installed / spawn error), return `true` (do not nag — nginx will surface its own error if the capability is truly absent). This conservative default means the preflight only acts when it positively confirms the capability is missing.

### 3.2 `core/src/bin.rs` (or a small helper) — the preflight

- `pub fn ensure_nginx_bind_cap(paths: &LaragonPaths, privileged: &dyn Privileged)`:
  1. `nginx = resolve_bin("nginx", &[paths.bin()])`; if `None`, return (nothing to do — start will fail/handle as today).
  2. if `nginx_has_bind_cap(&nginx)` → return (no prompt).
  3. else `let _ = privileged.setcap_nginx(&nginx);` (best-effort; errors/cancellation ignored).
- Lives in `core` so it is reusable by both `commands.rs` and `main.rs`, takes `&dyn Privileged` (testable with `FakePrivileged`), and keeps zero Tauri deps. Re-export from `lib.rs`.

### 3.3 Wiring (desktop)

- `commands::stack_start_all`: inside the existing `spawn_blocking`, call `ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged)` just before `orch.start_all()` (off the main thread — no freeze).
- `commands::service_start`: when `kind == ServiceKind::Nginx`, call `ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged)` before `orch.start(kind)`. (service_start is currently synchronous; the prompt only appears when the cap is missing, which is rare — acceptable. Other kinds are unaffected.)
- `main.rs` tray `"start_all"` handler: call `laragon_core::ensure_nginx_bind_cap(&state.paths, &laragon_core::PkexecPrivileged)` before `orch.start_all()`. The pkexec prompt (only when missing) briefly blocks the tray click — acceptable for a one-time, user-initiated action.

## 4. Behavior details & decisions

- **No prompt in the common case**: when the capability is present (or `getcap` is unavailable), nothing is escalated.
- **Best-effort**: a setcap failure/cancel never blocks Start; the user still sees nginx's own bind error if it truly can't bind, exactly as today.
- **Correct target**: the check and setcap both use `resolve_bin("nginx", &[paths.bin()])`, the same resolution the orchestrator uses to spawn nginx, so the capability lands on the binary that actually runs.
- **Self-healing across apt upgrades**: the next Start after an nginx upgrade re-applies the capability automatically.

## 5. Error handling

- `getcap` spawn failure → treated as "capability present" (no nag), per §3.1.
- `setcap` (pkexec) failure/cancel → ignored (best-effort); startup proceeds.
- No new error surfaces are added; the existing nginx start error path is unchanged.

## 6. Testing (TDD; no privilege/tools in unit tests)

- `getcap_indicates_cap`: `"… cap_net_bind_service=ep"` → `true`; empty string / unrelated text → `false`.
- `ensure_nginx_bind_cap` is verified live (it shells out to `getcap` and may prompt pkexec); the pure parser carries the unit coverage. Optionally, an integration-style test may create a fake nginx file under a temp `paths.bin()` with no capabilities and assert that `ensure_nginx_bind_cap` with a `FakePrivileged` records a `setcap` call (depends on `getcap` being installed on the test host) — keep it `#[ignore]`-free only if the host reliably has `getcap`; otherwise rely on the pure parser test + live verification.

## 7. Out of scope (backlog)

- Re-applying capabilities for other privileged binaries (none currently need it; only nginx binds privileged ports).
- A background watcher that re-setcaps proactively; this is on-demand at Start only.
- Surfacing a UI toast about the re-cap (silent best-effort is sufficient).
