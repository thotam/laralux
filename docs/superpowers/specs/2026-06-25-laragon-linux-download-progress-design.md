# Laragon Linux — Download Progress (combined ring) Design

**Date:** 2026-06-25
**Status:** Design approved, pending spec review
**Goal:** Show a circular progress indicator with animation while the app downloads binaries — an OUTER ring for component/step progress (e.g. 3/6 components) and an INNER ring for the current file's byte percentage. Built on a general core progress seam so every download (setup, PHP install, and future Spec 1/2 installers) reports through it.

---

## 1. Context & current state

Downloads go through `core::setup::Downloader::fetch(&self, url: &Path, dest) -> Result<(), SetupError>` (`CurlDownloader` runs `curl -fL url -o dest` synchronously, no progress). Installers call `downloader.fetch(...)`: `php_static` (index json + fpm/cli tarballs), `coredns::ensure_coredns` (tgz), `setup::run_setup` (mailpit tarball), `php_cli::install_composer` (composer.phar), `scaffold::create_site` (WordPress tarball). `core` keeps **zero Tauri deps**.

The desktop uses NO Tauri events yet; IPC commands (`run_setup_cmd`, `install_php_version`, `set_php_version`) run on `spawn_blocking` and the frontend awaits the blocking result. During a download the frontend shows only an indeterminate CSS spinner (`.spin`) — `state.setup.phase==="installing"` / `state.phpBusy`. The frontend reaches Tauri via `const TAURI = window.__TAURI__` and `TAURI.core.invoke`; events are available at `TAURI.event`. CSS already has `@keyframes spin`, `.spin`, and a `@media (prefers-reduced-motion: reduce)` block.

## 2. Approach (chosen: a core `ProgressSink` seam + curl HEAD-then-poll byte progress)

A new `core::progress` seam (trait + event enum) is threaded through `run_setup` and the installers, exactly like the existing `Downloader`/`CommandRunner`/`Privileged` seams — so `core` stays Tauri-free and the desktop bridges progress to a Tauri event. Byte progress is obtained without parsing curl's live meter: do a `HEAD` for the total size, then poll the destination file's growing size while curl writes it.

**Rejected:**
- Parsing curl's stderr progress meter: fragile, locale/format-dependent.
- A streaming Rust downloader (reqwest): a heavy new dep replacing the simple curl shell-out; unnecessary when polling the dest file size gives byte progress.
- Tauri `Channel<T>`: events are simpler here and the frontend already has `TAURI.event`; one app-wide `download-progress` event suffices.

## 3. Architecture & components

### 3.1 `core/src/progress.rs` (new)

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProgressEvent {
    /// A coarse phase change, e.g. "Installing components".
    Phase { label: String },
    /// Component/step progress: `done` of `total` finished, current item `label`.
    Step { done: usize, total: usize, label: String },
    /// Byte progress for the current file. `total == 0` means unknown (indeterminate).
    Bytes { current: u64, total: u64 },
}

pub trait ProgressSink: Send + Sync {
    fn emit(&self, ev: ProgressEvent);
}

/// No-op sink for the CLI and tests.
pub struct NullProgress;
impl ProgressSink for NullProgress {
    fn emit(&self, _ev: ProgressEvent) {}
}
```
Re-export `ProgressEvent`, `ProgressSink`, `NullProgress` from `lib.rs`.

### 3.2 `Downloader::fetch_with_progress` + `CurlDownloader` override (`core/src/setup.rs`)

Add a default method so existing impls/tests keep working:

```rust
pub trait Downloader: Send + Sync {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError>;
    /// Fetch while reporting byte progress to `sink`. Default: no byte progress.
    fn fetch_with_progress(&self, url: &str, dest: &Path, sink: &dyn ProgressSink) -> Result<(), SetupError> {
        let _ = sink;
        self.fetch(url, dest)
    }
}
```

`CurlDownloader::fetch_with_progress` (the real byte progress):
1. **Total size:** run `curl -sIL <url>` (capture stdout), parse the LAST `content-length:` header (case-insensitive) → `total: u64` (0 if absent → indeterminate). Helper `parse_content_length(headers: &str) -> u64` is pure and unit-tested.
2. **Download + poll:** spawn `curl -fL <url> -o <dest>` as a child (do NOT block on `status()`). Loop: `sink.emit(Bytes{current: dest_len, total})` where `dest_len` = `fs::metadata(dest).map(len).unwrap_or(0)`; `std::thread::sleep(150ms)`; `try_wait()` the child; break when it exits. Emit a final `Bytes{current: total, total}` (or `current: dest_len` if total unknown) on success.
3. Map a non-success exit to `SetupError::Download` (same message as `fetch`).
4. Throttle is inherent (one emit per ~150 ms poll).

`FakeDownloader` (test) keeps the default `fetch_with_progress` (delegates to `fetch`); a dedicated test downloader can emit synthetic `Bytes` events to exercise the sink.

### 3.3 Thread `sink` through `run_setup` + installers

Add a trailing `sink: &dyn ProgressSink` parameter to the download-bearing functions, and have multi-step flows emit `Step`:

- `setup::run_setup(paths, privileged, downloader, runner, sink)`: compute `total = missing.len()`; before installing each missing component emit `Step{done, total, label}` (label = component display name); pass `sink` into each installer; downloads use `fetch_with_progress`. The mailpit block uses `downloader.fetch_with_progress(MAILPIT_URL, &tarball, sink)`.
- `php_static::{install_php_static, install_php_cli}(…, sink)`: the tarball downloads use `fetch_with_progress`; the tiny index json keeps plain `fetch` (no byte UI for a small list).
- `coredns::ensure_coredns(paths, downloader, runner, sink)`: the tgz uses `fetch_with_progress`.
- `php_cli::install_composer(paths, downloader, sink)`: the phar uses `fetch_with_progress`.
- `php_cli::ensure_active_php_cli(…, sink)`: forwards `sink` to `install_php_cli`.
- `scaffold::create_site(…, sink)`: the WordPress tarball uses `fetch_with_progress` (so "Add site" also shows progress).

**Callers that have no UI sink pass `&NullProgress`:** `laragonctl` (CLI) and all existing core tests. This keeps the signatures uniform and the CLI silent.

### 3.4 Desktop bridge — `TauriProgress` (`src-tauri/src/commands.rs`)

```rust
struct TauriProgress(tauri::AppHandle);
impl laragon_core::ProgressSink for TauriProgress {
    fn emit(&self, ev: laragon_core::ProgressEvent) {
        let _ = self.0.emit("download-progress", ev); // best-effort
    }
}
```
`tauri::Emitter` is in scope (`use tauri::Emitter;`). Each downloading command builds `let progress = TauriProgress(app.clone());` and passes `&progress` to the core call. Commands updated: `run_setup_cmd`, `install_php_version`, `set_php_version` (its `ensure_active_php_cli`), and `create_site` (WordPress). The backend does NOT emit a terminal "done" event — the frontend clears the ring when the command's promise settles (§3.5). `ProgressEvent` serializes to the JS payload `{kind:"step",done,total,label}` / `{kind:"bytes",current,total}` / `{kind:"phase",label}`.

### 3.5 Frontend — combined progress ring (`dist/app.js`, `dist/styles.css`)

- **State:** `state.download = { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 } }`.
- **Listener (once, at startup):** `TAURI.event.listen("download-progress", (e) => { applyProgress(e.payload); render(); })`. `applyProgress` updates `state.download` by `payload.kind`: `phase`→ set label (and `active=true`); `step`→ set `step`+label+`active=true`; `bytes`→ set `bytes`. Guard for `TAURI.event` absence (no-op).
- **Activation/clear:** a download command sets `state.download.active = true` before `invoke(...)` and resets it to the idle object in the `finally` (so the ring shows for the whole operation and disappears when the promise settles). Applies to `installSetup` (run_setup), `installPhp`/`setPhp` (PHP), and create-site.
- **`progressRing()` renderer:** an inline SVG with TWO concentric circles using `stroke-dasharray`/`stroke-dashoffset`:
  - OUTER ring = component step fraction `step.done/step.total` (full circle when `total===0`).
  - INNER ring = byte fraction `bytes.current/bytes.total` (when `bytes.total>0`); when `total===0` the inner ring is hidden and a thin accent arc spins (`.spin`) to signal indeterminate.
  - Center text: the byte percentage when known, else the step count `done/total`.
  - Below: the current `label` ("Đang tải coredns…" — label text comes from the backend; keep backend labels human-readable).
  - A thin accent arc on the outer ring carries the `.spin` animation for liveliness.
  - `prefers-reduced-motion`: the existing CSS rule already disables `.spin`; the rings still fill (they are not animations, just stroke offsets).
- **Placement:** replace the plain `spinner()` in the Setup "installing" state and the PHP install/switch busy state with `progressRing()`. Other spinners (service start, etc.) are unchanged.
- **CSS:** add a `.ring` block (sizes, two stroke colors using existing accent/muted vars, rotate -90deg so 0% starts at top) and reuse `@keyframes spin`. No new keyframes required.

## 4. Behavior & error handling

- **Best-effort:** progress never affects download success. A failing `emit`, a missing `content-length`, or a poll error degrades gracefully (indeterminate inner ring; outer ring still advances per component).
- **Unknown total:** `Bytes{total:0}` → inner ring indeterminate (spin only), center shows the step count.
- **Throttle:** ~150 ms poll bounds event rate; the frontend coalesces by re-rendering current state.
- **Multiple files per component:** PHP installs fpm then cli — each emits its own `Bytes` stream; the inner ring resets per file. The outer ring advances once the component completes.
- **Listener lifecycle:** one listener registered at startup; never torn down (single-page app lifetime).

## 5. Testing (TDD)

- `progress`: `NullProgress::emit` is a no-op (compile/serialize smoke); `ProgressEvent` serializes with the `kind` tag (serde_json round-trip asserts `{"kind":"bytes",...}`).
- `parse_content_length`: case-insensitive, picks the last header value, returns 0 when absent.
- `Downloader::fetch_with_progress` default delegates to `fetch` (a fake recording downloader sees the same call).
- `run_setup` emits a `Step` per missing component (a `FakeProgress` sink records events; assert the count/labels) — using the existing `FakePrivileged`/`FakeDownloader`/`FakeCommandRunner` test rig with a `&FakeProgress`.
- Installers compile + pass with `&NullProgress` (existing tests updated to pass it).
- `CurlDownloader::fetch_with_progress` (real curl + poll) is **verified live** (network), like the existing download paths; the pure `parse_content_length` carries the unit coverage.
- Frontend: `node --check dist/app.js`; live visual verification of the two-ring fill + spin + reduced-motion.

## 6. Out of scope (backlog)

- Transfer speed / ETA / bytes-remaining text (only a percentage + label for now).
- Per-file progress for the small index/listing JSON fetches (kept on plain `fetch`).
- A cancel button for in-flight downloads.
- Robustness beyond curl (resumable downloads, checksum-during-stream).
- Reusing the ring for non-download long operations (it is download-specific here).
