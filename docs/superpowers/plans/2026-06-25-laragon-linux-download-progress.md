# Download Progress (combined ring) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a two-ring circular progress indicator (outer = component step, inner = byte percent) with animation while the app downloads binaries.

**Architecture:** A core `progress` seam (`ProgressEvent` enum + `ProgressSink` trait + `NullProgress`) is threaded through `run_setup` and every installer, exactly like the `Downloader`/`Privileged` seams. `CurlDownloader::fetch_with_progress` gets byte progress by HEAD-ing the total then polling the destination file size. The desktop bridges the sink to a Tauri `download-progress` event; the frontend renders concentric SVG rings.

**Tech Stack:** Rust (laragon-core, zero Tauri deps), Tauri 2 (Emitter + events), vanilla JS, SVG/CSS.

## Global Constraints

- `core` keeps **zero Tauri deps**. Commit messages MUST NOT contain a `Co-Authored-By` trailer. TDD: failing test first.
- `ProgressEvent` (serde, `#[serde(tag="kind", rename_all="lowercase")]`): `Phase{label}`, `Step{done,total,label}`, `Bytes{current,total}` (`total==0` = unknown).
- `Downloader::fetch_with_progress(url,dest,sink)` has a DEFAULT that calls `fetch` (existing impls/tests unaffected); only `CurlDownloader` overrides it with real byte progress.
- Byte progress = `curl -sIL` Content-Length total + poll `dest` file size every ~150 ms while `curl -fL -o dest` runs (no parsing of curl's live meter).
- Progress is best-effort: a missing Content-Length → indeterminate inner ring; an `emit`/poll failure never breaks the download.
- The CLI (`laragonctl`) and all core tests pass `&NullProgress`. The desktop passes a `TauriProgress` only where a UI ring is wanted.
- Tauri event name: `download-progress`. Frontend reaches Tauri via `window.__TAURI__` (`TAURI.event.listen`, `TAURI.core.invoke`). The frontend clears the ring when the command promise settles (no backend "done" event).
- Run core tests `cargo test -p laragon-core`; build `cargo build -p laragon-desktop && cargo build -p laragonctl`; frontend `node --check dist/app.js` (node fallback `$HOME/.nvm/versions/node/v24.16.0/bin/node`). cargo fallback `$HOME/.cargo/bin/cargo`.

---

### Task 1: `progress` module + `fetch_with_progress`

**Files:** Create `core/src/progress.rs`; modify `core/src/setup.rs` (Downloader trait + CurlDownloader), `core/src/lib.rs`.

**Interfaces produced:** `progress::{ProgressEvent, ProgressSink, NullProgress}`; `Downloader::fetch_with_progress(&self, url:&str, dest:&Path, sink:&dyn ProgressSink) -> Result<(),SetupError>` (default delegates to `fetch`); `setup::parse_content_length(headers:&str) -> u64`.

- [ ] **Step 1: Write the failing tests** — create `core/src/progress.rs`:

```rust
//! A small progress-reporting seam so `core` can report download/step progress
//! to a UI (the desktop bridges it to a Tauri event) without any UI dependency.

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProgressEvent {
    /// A coarse phase change.
    Phase { label: String },
    /// Component/step progress: `done` of `total`, current item `label`.
    Step { done: usize, total: usize, label: String },
    /// Byte progress for the current file. `total == 0` means unknown.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_serializes_with_kind_tag() {
        let j = serde_json::to_string(&ProgressEvent::Bytes { current: 5, total: 10 }).unwrap();
        assert_eq!(j, r#"{"kind":"bytes","current":5,"total":10}"#);
        let s = serde_json::to_string(&ProgressEvent::Step { done: 1, total: 3, label: "php".into() }).unwrap();
        assert_eq!(s, r#"{"kind":"step","done":1,"total":3,"label":"php"}"#);
    }

    #[test]
    fn null_sink_is_noop() {
        NullProgress.emit(ProgressEvent::Phase { label: "x".into() }); // must not panic
    }
}
```
And add to `core/src/setup.rs` tests a `parse_content_length` test:

```rust
    #[test]
    fn parse_content_length_picks_last_case_insensitive() {
        let h = "HTTP/2 200\r\nContent-Length: 100\r\n\r\nHTTP/2 200\r\ncontent-length: 4096\r\n";
        assert_eq!(parse_content_length(h), 4096);
        assert_eq!(parse_content_length("no header here"), 0);
    }
```

- [ ] **Step 2: Run** — `cargo test -p laragon-core progress` and `cargo test -p laragon-core parse_content_length` → FAIL.

- [ ] **Step 3: Implement** — in `core/src/lib.rs` add `pub mod progress;` and `pub use progress::{ProgressEvent, ProgressSink, NullProgress};`.

In `core/src/setup.rs`, add `use crate::progress::ProgressSink;` near the top, add the default trait method, and override it for `CurlDownloader`:

```rust
pub trait Downloader: Send + Sync {
    fn fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError>;
    /// Fetch while reporting byte progress to `sink`. Default: no byte progress.
    fn fetch_with_progress(&self, url: &str, dest: &Path, sink: &dyn ProgressSink) -> Result<(), SetupError> {
        let _ = sink;
        self.fetch(url, dest)
    }
}

/// Last `content-length` header value (case-insensitive) in a raw HTTP header
/// blob, or 0 if none/unparsable.
pub fn parse_content_length(headers: &str) -> u64 {
    let mut total = 0u64;
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                if let Ok(n) = v.trim().parse::<u64>() {
                    total = n;
                }
            }
        }
    }
    total
}
```

In `impl Downloader for CurlDownloader`, add the override below the existing `fetch`:

```rust
    fn fetch_with_progress(&self, url: &str, dest: &Path, sink: &dyn ProgressSink) -> Result<(), SetupError> {
        use crate::progress::ProgressEvent;
        // Total size via a HEAD; 0 (unknown) is fine — the UI shows an indeterminate ring.
        let total = std::process::Command::new("curl")
            .args(["-sIL", url])
            .output()
            .ok()
            .map(|o| parse_content_length(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or(0);
        // Start the download in the background; poll the growing dest file for progress.
        let mut child = std::process::Command::new("curl")
            .arg("-fL").arg(url).arg("-o").arg(dest)
            .spawn()
            .map_err(|e| SetupError::Download(format!("spawn curl: {e}")))?;
        loop {
            let current = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
            sink.emit(ProgressEvent::Bytes { current, total });
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        let done = if total > 0 { total } else { current };
                        sink.emit(ProgressEvent::Bytes { current: done, total });
                        return Ok(());
                    }
                    return Err(SetupError::Download(format!("curl failed for {url}")));
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(150)),
                Err(e) => return Err(SetupError::Download(format!("curl wait: {e}"))),
            }
        }
    }
```

- [ ] **Step 4: Run** — `cargo test -p laragon-core` → PASS; `cargo build -p laragon-desktop && cargo build -p laragonctl` clean.

- [ ] **Step 5: Commit**

```bash
git add core/src/progress.rs core/src/setup.rs core/src/lib.rs
git commit -m "feat(core): progress seam + Downloader::fetch_with_progress (curl HEAD+poll)"
```

---

### Task 2: Thread `sink` through `run_setup` + installers

**Files:** Modify `core/src/setup.rs`, `core/src/php_static.rs`, `core/src/php_cli.rs`, `core/src/coredns.rs`, `core/src/scaffold.rs`, `laragonctl/src/main.rs`. (The `src-tauri` callers are updated in Task 3.)

**Interfaces produced (every download-bearing fn gains a trailing `sink: &dyn ProgressSink`):**
- `run_setup(paths, privileged, downloader, runner, sink)`
- `install_php_static(paths, requested, downloader, runner, sink) -> Result<String, PhpStaticError>`
- `install_php_cli(paths, requested, downloader, runner, sink) -> Result<String, PhpStaticError>`
- `ensure_active_php_cli(paths, version, downloader, runner, sink)`
- `ensure_coredns(paths, downloader, runner, sink)`
- `install_composer(paths, downloader, sink)`
- `create_site(paths, name, tld, template, ssl, runner, downloader, sink) -> Result<CreateReport, ScaffoldError>`

This is an atomic signature change: all core call sites must be updated in the same commit so the crate compiles. Use `&crate::progress::NullProgress` at every test call site and in `laragonctl`. **Do NOT change the `src-tauri` callers here** — they're in Task 3; this task must still leave `cargo build -p laragon-desktop` GREEN, which it does because the desktop calls these via the same names and Task 3 hasn't run — WAIT: changing a core signature breaks the desktop callers immediately. Therefore this task MUST also update the `src-tauri/src/commands.rs` call sites to pass `&crate::progress::NullProgress` (a temporary placeholder) so the workspace compiles; Task 3 swaps those placeholders for `&TauriProgress`. Update them to `&laragon_core::NullProgress` in this task.

- [ ] **Step 1: Add the param + byte-progress downloads in each installer.**

`core/src/php_static.rs`: add `sink: &dyn crate::progress::ProgressSink` as the last param of `install_php_static`, `install_php_cli`, and the private `download_static_php`; in `download_static_php` change `downloader.fetch(url, &tarball)` to `downloader.fetch_with_progress(url, &tarball, sink)`; pass `sink` from the two public fns into `download_static_php`. (The `fetch_index` JSON download stays plain `fetch`.) Add `use crate::progress::ProgressSink;` if it makes the signature cleaner, else use the full path.

`core/src/php_cli.rs`: add `sink` to `install_composer` (use `fetch_with_progress` for the phar) and to `ensure_active_php_cli` (forward `sink` to `install_php_cli`).

`core/src/coredns.rs`: add `sink` to `ensure_coredns`; change the tgz `downloader.fetch(...)` to `downloader.fetch_with_progress(..., sink)`.

`core/src/scaffold.rs`: add `sink` to `create_site`; change the WordPress `downloader.fetch(WORDPRESS_URL, &tarball)` to `downloader.fetch_with_progress(WORDPRESS_URL, &tarball, sink)`.

- [ ] **Step 2: `run_setup` gains `sink` + emits `Step` per missing component.** In `core/src/setup.rs`, add `sink: &dyn crate::progress::ProgressSink` as the last param of `run_setup`. After computing `missing`, drive the installs through a counter so each emits a `Step`:

```rust
    use crate::progress::ProgressEvent;
    let total = missing.len();
    let mut done = 0usize;
    // helper closure label per component for the Step event
    let label_of = |c: Component| c.display_name().to_string();
```
Before each component's install block (PHP, mailpit, composer), emit `sink.emit(ProgressEvent::Step { done, total, label: label_of(Component::X) });` and `done += 1;` after it. (Reuse the component's existing display-name accessor; if none exists, use a `match` to a `&str`.) Pass `sink` into `install_php_static`, `install_composer`, and use `downloader.fetch_with_progress(MAILPIT_URL, &tarball, sink)` for mailpit. The apt block and `apply_versions` are unchanged.

- [ ] **Step 3: Update ALL core call sites + laragonctl + (temporarily) the desktop callers** to pass a sink:
  - `core/src/setup.rs` tests (3× `run_setup(...)`) → append `, &crate::progress::NullProgress`.
  - `core/src/php_static.rs` tests (`install_php_static(...)`) → append `, &crate::progress::NullProgress`.
  - `core/src/php_cli.rs` tests (`install_composer(...)`, `ensure_active_php_cli(...)`) → append the sink.
  - `core/src/scaffold.rs` tests (5× `create_site(...)`) → append `, &crate::progress::NullProgress`.
  - `laragonctl/src/main.rs:93` `run_setup(&paths, &SudoPrivileged, &CurlDownloader, &RealCommandRunner)` → append `, &laragon_core::NullProgress` (add `NullProgress` to the `laragon_core::{...}` import).
  - `src-tauri/src/commands.rs` — TEMPORARILY append `&laragon_core::NullProgress` to: `run_setup(...)` (~line 155), `install_php_static(...)` (install_php_version ~line 403), `ensure_active_php_cli(...)` (set_php_version + the toggle path, ~lines 433/459), `ensure_coredns(...)` (apply_wildcard_dns ~line 527), `install_composer(...)` (~line 461), and `core_create_site(...)` (create_site command ~line 162 body). Add `NullProgress` to the `laragon_core::{...}` import. (Task 3 replaces these with `&TauriProgress` where a UI ring is wanted.)

- [ ] **Step 4: Add a `run_setup` Step-emit test** — in `core/src/setup.rs` tests, add a `FakeProgress` sink and assert a `Step` per missing component:

```rust
    struct FakeProgress(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl crate::progress::ProgressSink for FakeProgress {
        fn emit(&self, ev: crate::progress::ProgressEvent) {
            if let crate::progress::ProgressEvent::Step { done, total, label } = ev {
                self.0.lock().unwrap().push(format!("{done}/{total}:{label}"));
            }
        }
    }

    #[test]
    fn run_setup_emits_a_step_per_missing_component() {
        let root = std::env::temp_dir().join(format!("lara-setup-prog-{}", std::process::id()));
        let paths = LaragonPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let priv_ = FakePrivileged::new();
        let dl = FakeDownloader::new();
        let runner = crate::scaffold::FakeCommandRunner::new();
        let steps = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = FakeProgress(steps.clone());
        let _ = run_setup(&paths, &priv_, &dl, &runner, &sink);
        // At least the PHP/mailpit/composer steps fire (all components are missing on a fresh root).
        assert!(!steps.lock().unwrap().is_empty(), "expected Step events");
        assert!(steps.lock().unwrap().iter().all(|s| s.contains('/')));
        std::fs::remove_dir_all(&root).ok();
    }
```
(If `FakeDownloader`/`FakeCommandRunner` don't fully satisfy a real install on a fresh root, the test still asserts that `Step` events were emitted — keep the assertion to "non-empty + well-formed", not an exact count, since component availability depends on the fakes.)

- [ ] **Step 5: Run** — `cargo test -p laragon-core` → PASS; `cargo build -p laragon-desktop && cargo build -p laragonctl` clean.

- [ ] **Step 6: Commit**

```bash
git add core/src/setup.rs core/src/php_static.rs core/src/php_cli.rs core/src/coredns.rs core/src/scaffold.rs laragonctl/src/main.rs src-tauri/src/commands.rs
git commit -m "feat(core): thread ProgressSink through run_setup + installers"
```

---

### Task 3: Desktop bridge — `TauriProgress` + wire commands

**Files:** Modify `src-tauri/src/commands.rs`.

**Interfaces:** `struct TauriProgress(tauri::AppHandle)` impl `laragon_core::ProgressSink`. Replaces the `&laragon_core::NullProgress` placeholders from Task 2 with `&TauriProgress` in the commands that should show a ring.

- [ ] **Step 1: Add the bridge** — at the top of `src-tauri/src/commands.rs`, ensure `use tauri::Emitter;` is present (add if missing), and add:

```rust
struct TauriProgress(tauri::AppHandle);
impl laragon_core::ProgressSink for TauriProgress {
    fn emit(&self, ev: laragon_core::ProgressEvent) {
        let _ = self.0.emit("download-progress", ev);
    }
}
```

- [ ] **Step 2: Wire the downloading commands** — in each, build `let progress = TauriProgress(app.clone());` (these commands already have `app: tauri::AppHandle` via `spawn_blocking(move || { let state = app.state...})` — clone `app` BEFORE the closure moves it, or use `app.clone()` where `app` is still available; if the command captured `app` into the closure, capture a clone for `TauriProgress`). Replace the Task-2 placeholder `&laragon_core::NullProgress` with `&progress` in:
  - `run_setup_cmd` → `run_setup(&state.paths, &privileged, &downloader, &RealCommandRunner, &progress)`.
  - `install_php_version` → `install_php_static(&state.paths, &version, &CurlDownloader, &RealCommandRunner, &progress)`.
  - `set_php_version` → its `ensure_active_php_cli(&state.paths, &version, &CurlDownloader, &RealCommandRunner, &progress)`.
  - `create_site` → `core_create_site(..., &progress)`.
  - Leave `ensure_coredns` (in `apply_wildcard_dns`) and the shell-toggle `ensure_active_php_cli`/`install_composer` calls on `&laragon_core::NullProgress` (no ring there for now — `apply_wildcard_dns` has only `&AppState`, no AppHandle; documented as deferred).
  - For the AppHandle: these commands are `pub async fn name(app: tauri::AppHandle, ...)`. Inside, do `let app_for_progress = app.clone();` before `spawn_blocking(move || ...)` and move `app_for_progress` into the closure to build `TauriProgress(app_for_progress)`. (The closure already moves `app` for `app.state()`; clone first.)

- [ ] **Step 3: Build** — `cargo build -p laragon-desktop` clean; `cargo build -p laragonctl` clean (laragonctl unchanged from Task 2).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(desktop): bridge ProgressSink to the download-progress Tauri event"
```

---

### Task 4: Frontend — combined progress ring

**Files:** Modify `dist/app.js`, `dist/styles.css`.

**Interfaces:** consumes the `download-progress` event (`{kind:"phase"|"step"|"bytes", ...}`).

- [ ] **Step 1: State + listener** — in `dist/app.js`, add to `state` (near `setup`): `download: { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 } },`. Add a reset helper and an apply function, and register the listener once at startup (where the app initializes — near the initial `refresh()`/load):

```js
  function resetDownload() {
    state.download = { active: false, label: "", step: { done: 0, total: 0 }, bytes: { current: 0, total: 0 } };
  }
  function applyProgress(p) {
    if (!p || !p.kind) return;
    state.download.active = true;
    if (p.kind === "phase") state.download.label = String(p.label || "");
    else if (p.kind === "step") { state.download.step = { done: p.done | 0, total: p.total | 0 }; if (p.label) state.download.label = String(p.label); }
    else if (p.kind === "bytes") state.download.bytes = { current: Number(p.current) || 0, total: Number(p.total) || 0 };
  }
  if (TAURI && TAURI.event && TAURI.event.listen) {
    TAURI.event.listen("download-progress", (e) => { applyProgress(e.payload); render(); });
  }
```

- [ ] **Step 2: `progressRing()` renderer** — add a function that returns the two-ring SVG. Outer ring = step fraction, inner ring = byte fraction (indeterminate spin when `bytes.total===0`), center shows the byte percent (or step count), label below:

```js
  function progressRing() {
    const d = state.download;
    const R1 = 26, R2 = 18, C1 = 2 * Math.PI * R1, C2 = 2 * Math.PI * R2;
    const stepFrac = d.step.total > 0 ? d.step.done / d.step.total : 0;
    const byteKnown = d.bytes.total > 0;
    const byteFrac = byteKnown ? Math.min(1, d.bytes.current / d.bytes.total) : 0;
    const pct = byteKnown ? Math.round(byteFrac * 100) + "%"
      : (d.step.total > 0 ? d.step.done + "/" + d.step.total : "");
    const off = (frac, C) => C * (1 - frac);
    const inner = byteKnown
      ? '<circle class="ring-fg" cx="32" cy="32" r="' + R2 + '" stroke-dasharray="' + C2 + '" stroke-dashoffset="' + off(byteFrac, C2) + '"/>'
      : '<circle class="ring-spin spin" cx="32" cy="32" r="' + R2 + '" stroke-dasharray="' + (C2 * 0.25) + ' ' + C2 + '"/>';
    return (
      '<div class="ring" role="status" aria-label="Downloading">' +
      '<svg width="64" height="64" viewBox="0 0 64 64">' +
      '<circle class="ring-bg" cx="32" cy="32" r="' + R1 + '"/>' +
      '<circle class="ring-fg outer" cx="32" cy="32" r="' + R1 + '" stroke-dasharray="' + C1 + '" stroke-dashoffset="' + off(stepFrac, C1) + '"/>' +
      '<circle class="ring-bg" cx="32" cy="32" r="' + R2 + '"/>' + inner +
      '<text class="ring-pct" x="32" y="36" text-anchor="middle">' + esc(pct) + '</text>' +
      '</svg>' +
      (d.label ? '<span class="ring-label">' + esc(d.label) + '</span>' : '') +
      '</div>'
    );
  }
```

- [ ] **Step 3: Activate + clear around downloading commands** — in the install/setup handlers, set `state.download.active = true` before the `invoke(...)` and `resetDownload()` in the `finally`:
  - In the run_setup handler (`state.setup.phase === "installing"` flow): before `invoke("run_setup_cmd")` add `state.download.active = true; state.download.label = "Installing components…"; render();`; in the existing `finally` add `resetDownload();`.
  - In the PHP install handler (`installPhp`, where `state.phpBusy = true`): set `state.download.active = true` before `invoke("install_php_version"...)`; `resetDownload()` in `finally`.
  - In the create-site handler: same pattern around `invoke("create_site", ...)`.
  Where the view currently renders the plain `spinner()` for these busy states (Setup installing button/section and the PHP install busy state), render `progressRing()` instead when `state.download.active` is true. Keep other spinners (service start, etc.) as-is.

- [ ] **Step 4: CSS** — in `dist/styles.css`, add a `.ring` block (reuse `@keyframes spin` already present; rotate the SVG so 0% starts at top). Use existing accent/muted custom properties (check `:root` for the names, e.g. `--accent`, `--muted`/`--line`; use the actual names in the file):

```css
.ring { display:flex; flex-direction:column; align-items:center; gap:8px; }
.ring svg { transform:rotate(-90deg); }
.ring .ring-bg { fill:none; stroke:var(--line); stroke-width:5; }
.ring .ring-fg { fill:none; stroke:var(--accent); stroke-width:5; stroke-linecap:round; transition:stroke-dashoffset .2s ease; }
.ring .ring-fg.outer { stroke-width:6; }
.ring .ring-spin { fill:none; stroke:var(--accent); stroke-width:5; stroke-linecap:round; transform-origin:32px 32px; }
.ring .ring-pct { transform:rotate(90deg); transform-origin:32px 32px; fill:var(--text); font-size:13px; font-weight:600; }
.ring .ring-label { font-size:13px; color:var(--muted); }
```
(Use the file's real variable names — open `:root` in `styles.css` first. `prefers-reduced-motion` already disables `.spin` via the existing rule, so the indeterminate arc just stops; determinate rings still fill.)

- [ ] **Step 5: Syntax check** — `node --check dist/app.js` → exit 0.

- [ ] **Step 6: Manual verification (live)** — `cargo run -p laragon-desktop`; trigger Setup (fresh `~/laragon/bin`) and a PHP install; confirm the outer ring advances per component, the inner ring fills per file (or spins when size is unknown), the percent/label update, and `prefers-reduced-motion` stops the spin while rings still fill.

- [ ] **Step 7: Commit**

```bash
git add dist/app.js dist/styles.css
git commit -m "feat(desktop): combined component+byte download progress ring"
```

---

## Self-Review

**1. Spec coverage:** progress seam + enum/trait/NullProgress (T1, §3.1); `fetch_with_progress` + curl HEAD/poll + `parse_content_length` (T1, §3.2); thread `sink` through run_setup + installers + `Step` emit (T2, §3.3); `TauriProgress` + event + command wiring (T3, §3.4); frontend listener + `progressRing` + state + activation/clear + CSS + reduced-motion (T4, §3.5). Error handling (§4) is covered by the best-effort `let _ = emit`, `unwrap_or(0)` total, and `resetDownload` in `finally`. Out-of-scope items (§6: speed/ETA, JSON-index byte UI, cancel) are not implemented — correct.

**2. Placeholder scan:** No "TBD"/"handle errors"/"similar to". The two deferred coredns/shell-toggle sinks are explicitly `NullProgress` with a stated reason (no AppHandle in `apply_wildcard_dns`), not a placeholder.

**3. Type consistency:** `ProgressEvent`/`ProgressSink`/`NullProgress` (T1) are used by every `sink: &dyn ProgressSink` param (T2) and by `TauriProgress` (T3); `fetch_with_progress(url,dest,sink)` (T1) is called in T2's installers; the event payload `{kind, ...}` (T1 serde tag) matches the frontend `applyProgress` switch on `p.kind` (T4); `install_php_static`/`install_php_cli` keep their `Result<String,...>` return (only a trailing param added). Atomic signature change in T2 keeps the workspace compiling (all core + laragonctl + desktop call sites updated together, desktop temporarily on `NullProgress`).
