# Readiness gating + auto-restart cho Procfile process — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development hoặc superpowers:executing-plans để triển khai theo từng task. Các step dùng checkbox (`- [ ]`).

**Goal:** Service chỉ báo `Running` khi thật sự nhận được kết nối, và process khai báo trong `Procfile` tự khởi động lại có backoff khi chết — để `queue:work` không còn chết vì MariaDB chưa kịp mở cổng, và không nằm chết luôn sau đó.

**Architecture:** Đấu `Service::health_check` (đang là dead code) vào `Orchestrator::start()` bằng vòng poll có timeout. `SiteProcs` lưu thêm `root`+`command` (hiện không lưu nên không thể respawn) cùng bảng giám sát theo từng process, và `refresh()` trở thành nhịp supervision với backoff/bỏ cuộc/reset.

**Tech Stack:** Rust (core lib + Tauri, `cargo test`), TypeScript/Vite frontend (không có test harness — verify bằng `npm run build`).

## Global Constraints

- KHÔNG thêm `Co-Authored-By` hay chữ ký "Generated with Claude"/🤖 vào commit.
- **Không test nào được `sleep` theo thời gian thật.** Orchestrator: timeout/interval là field, test set giá trị tí xíu. SiteProcs: logic nằm trong `tick_at(now: Instant)`, test bơm mốc thời gian giả.
- Restart **bất kể exit code** (`queue:work` thoát code 0 khi `queue:restart` và cần được chạy lại).
- Hai bất biến của supervision: **stop tay không bao giờ bị respawn**; **start tay reset bộ đếm**.
- Hằng số: poll readiness **100ms**; timeout **15s** (bình thường) / **60s** (`needs_init`); backoff lần thử lại thứ n = **1s, 5s, 15s, 30s**; bỏ cuộc sau **5** lần chết liên tiếp; reset bộ đếm sau **30s** sống ổn định.

---

### Task 1: Orchestrator — chờ health_check trước khi báo Running

**Files:**
- Modify: `core/src/orchestrator.rs`
- Test: `core/src/orchestrator.rs` (module `#[cfg(test)]` sẵn có)

**Interfaces:**
- Produces: `Orchestrator::with_readiness(timeout, init_timeout, interval) -> Self` (builder cho test); `start()` chỉ đặt `Running` sau khi `health_check` pass, hết giờ thì đặt `Crashed` và trả `Err(ServiceError::HealthCheck)`.

- [ ] **Step 1: Viết test thất bại**

Trong module test của `core/src/orchestrator.rs`: **copy struct `Dummy` sẵn có** rồi tạo một biến thể có health check điều khiển được (chỉ đổi `health_check`, các method khác giữ y hệt `Dummy`):

```rust
    /// Như `Dummy` nhưng health check fail `fails_left` lần đầu rồi mới pass.
    struct FlakyHealth {
        kind: ServiceKind,
        fails_left: std::sync::atomic::AtomicUsize,
    }
```

`health_check` của nó:

```rust
        fn health_check(&self, _p: &LaraluxPaths) -> Result<(), ServiceError> {
            use std::sync::atomic::Ordering;
            if self.fails_left.load(Ordering::SeqCst) == 0 {
                return Ok(());
            }
            self.fails_left.fetch_sub(1, Ordering::SeqCst);
            Err(ServiceError::HealthCheck("not up yet".into()))
        }
```

Rồi thêm hai test:

```rust
    #[test]
    fn start_waits_for_health_then_reports_running() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-orch-ready-{}", std::process::id())));
        let svc = FlakyHealth { kind: ServiceKind::Nginx, fails_left: std::sync::atomic::AtomicUsize::new(3) };
        let mut orch = Orchestrator::new(paths, vec![Box::new(svc)], Box::new(FakeSpawner::default()))
            .with_readiness(
                std::time::Duration::from_millis(500),
                std::time::Duration::from_millis(500),
                std::time::Duration::from_millis(1),
            );
        orch.start(ServiceKind::Nginx).unwrap();
        assert_eq!(orch.state(ServiceKind::Nginx), ServiceState::Running);
    }

    #[test]
    fn start_times_out_into_crashed_when_never_healthy() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-orch-to-{}", std::process::id())));
        // Không bao giờ pass.
        let svc = FlakyHealth { kind: ServiceKind::Nginx, fails_left: std::sync::atomic::AtomicUsize::new(usize::MAX) };
        let mut orch = Orchestrator::new(paths, vec![Box::new(svc)], Box::new(FakeSpawner::default()))
            .with_readiness(
                std::time::Duration::from_millis(20),
                std::time::Duration::from_millis(20),
                std::time::Duration::from_millis(1),
            );
        let err = orch.start(ServiceKind::Nginx).unwrap_err();
        assert!(matches!(err, ServiceError::HealthCheck(_)), "expected HealthCheck, got {err:?}");
        assert_eq!(orch.state(ServiceKind::Nginx), ServiceState::Crashed);
    }
```

Lưu ý người thực thi: dùng đúng cách khởi tạo `FakeSpawner` và `LaraluxPaths` mà các test sẵn có trong file này đang dùng; nếu `Dummy` cần thêm method nào của trait `Service` thì copy nguyên từ `Dummy`.

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core orchestrator::tests::start_times_out_into_crashed_when_never_healthy`
Expected: FAIL — chưa có `with_readiness` (không compile), và `start()` hiện đặt `Running` ngay.

- [ ] **Step 3: Cài đặt**

Trong `core/src/orchestrator.rs`:

1. Thêm import ở đầu file (nếu chưa có): `use std::time::{Duration, Instant};`

2. Thêm field vào `struct Orchestrator`:

```rust
    readiness_timeout: Duration,
    readiness_init_timeout: Duration,
    readiness_interval: Duration,
```

3. Trong `new()`, thêm giá trị mặc định vào literal `Self { … }`:

```rust
            readiness_timeout: Duration::from_secs(15),
            readiness_init_timeout: Duration::from_secs(60),
            readiness_interval: Duration::from_millis(100),
```

4. Thêm builder ngay sau `new()`:

```rust
    /// Override the readiness poll parameters. Tests use tiny values so they
    /// never sleep for real.
    pub fn with_readiness(
        mut self,
        timeout: Duration,
        init_timeout: Duration,
        interval: Duration,
    ) -> Self {
        self.readiness_timeout = timeout;
        self.readiness_init_timeout = init_timeout;
        self.readiness_interval = interval;
        self
    }
```

5. Ở cuối `start()`, thay:

```rust
        self.states.insert(kind, ServiceState::Running);
        Ok(())
    }
```

bằng:

```rust
        // Spawning only means the process exists. Wait until the service actually
        // answers before calling it Running — otherwise whatever starts next (a
        // site's Procfile worker dialling MariaDB) races the port opening.
        let timeout = if needs_init { self.readiness_init_timeout } else { self.readiness_timeout };
        match self.await_ready(kind, timeout) {
            Ok(()) => {
                self.states.insert(kind, ServiceState::Running);
                Ok(())
            }
            Err(e) => {
                self.states.insert(kind, ServiceState::Crashed);
                Err(e)
            }
        }
    }

    /// Poll the service's own health check until it passes or `timeout` elapses.
    fn await_ready(&self, kind: ServiceKind, timeout: Duration) -> Result<(), ServiceError> {
        let deadline = Instant::now() + timeout;
        let mut last = String::new();
        loop {
            let svc = self
                .find(kind)
                .ok_or_else(|| ServiceError::Config(format!("no such service: {kind:?}")))?;
            match svc.health_check(&self.paths) {
                Ok(()) => return Ok(()),
                Err(e) => last = e.to_string(),
            }
            if Instant::now() >= deadline {
                return Err(ServiceError::HealthCheck(format!(
                    "{kind:?} not ready after {:?}: {last}",
                    timeout
                )));
            }
            std::thread::sleep(self.readiness_interval);
        }
    }
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core orchestrator`
Expected: PASS toàn bộ (các test cũ dùng `Dummy` có `health_check` trả `Ok` nên vẫn xanh).

- [ ] **Step 5: Commit**

```bash
git add core/src/orchestrator.rs
git commit -m "feat(orchestrator): wait for a service's health check before reporting Running"
```

---

### Task 2: Tray không block main thread

**Files:**
- Modify: `src-tauri/src/main.rs`

**Interfaces:** none (chỉ đổi threading).

- [ ] **Step 1: Bọc nhánh `stack_toggle` trong thread**

Trong `on_menu_event`, nhánh `"stack_toggle"` đang gọi thẳng `commands::run_full_start(&state)` trên main thread GTK. Đổi thành spawn thread, theo đúng mẫu đường `launch.autostart_services` trong cùng file:

```rust
                    "stack_toggle" => {
                        // Tauri delivers tray events on the GTK main thread, and the
                        // start path now blocks until every service passes its health
                        // check (up to 60s on a first-run MariaDB init). Doing that
                        // inline would freeze the tray and the window.
                        let handle = app.clone();
                        std::thread::spawn(move || {
                            let Some(state) = handle.try_state::<AppState>() else { return };
                            // … phần thân cũ của nhánh này, giữ nguyên logic …
                        });
                    }
```

Người thực thi: di chuyển **nguyên vẹn** phần thân hiện tại (đọc `all_running`, nhánh `stop_all()`, `ResetGuard`, `run_full_start`) vào trong closure; chỉ đổi cách lấy `state` (từ `app.try_state()` sang `handle.try_state()`). Không đổi logic.

- [ ] **Step 2: Build**

Run: `cargo check -p laralux-desktop`
Expected: sạch. Nếu lỗi lifetime/move, nhớ `app.clone()` (AppHandle clone được) và chuyển mọi thứ cần dùng vào closure.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "fix(tray): run Start All off the GTK main thread"
```

---

### Task 3: SiteProcs — lưu command/root và giám sát có backoff

**Files:**
- Modify: `core/src/site_procs.rs`
- Test: `core/src/site_procs.rs`

**Interfaces:**
- Produces: `SiteProcs::tick_at(&mut self, now: Instant)` (public, để test bơm thời gian); `refresh()` = `tick_at(Instant::now())`; `SiteProcs::failures_of(site, name) -> u32`.

- [ ] **Step 1: Viết test thất bại**

Thêm vào module test của `core/src/site_procs.rs` (dùng `FakeSpawner` sẵn có trong crate; nếu module test của file này chưa có helper tạo `SiteProcs`, tạo trực tiếp bằng `SiteProcs::new(paths, Box::new(FakeSpawner::default()))`):

```rust
    fn t0() -> Instant { Instant::now() }

    #[test]
    fn dead_proc_is_retried_with_backoff_then_given_up() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-sp-bo-{}", std::process::id())));
        let spawner = FakeSpawner::default();
        let mut sp = SiteProcs::new(paths, Box::new(spawner));
        sp.start("site", std::path::Path::new("/tmp"), "web", "true").unwrap();

        let start = t0();
        // Tiến trình giả chết ngay; mỗi lần tick sau mốc backoff sẽ respawn.
        // 4 lần thử lại: +1s, +5s, +15s, +30s; lần chết thứ 5 thì bỏ cuộc.
        for (i, secs) in [1u64, 5, 15, 30].iter().enumerate() {
            sp.tick_at(start + Duration::from_secs(*secs) - Duration::from_millis(1));
            assert_eq!(sp.failures_of("site", "web"), (i as u32) + 1, "chưa tới hạn thì chưa retry");
            sp.tick_at(start + Duration::from_secs(*secs));
        }
        // Sau 4 lần thử lại, lần chết thứ 5 -> bỏ cuộc, không hẹn lần nữa.
        sp.tick_at(start + Duration::from_secs(120));
        assert_eq!(sp.state_of("site", "web"), ServiceState::Crashed);
        assert_eq!(sp.failures_of("site", "web"), 5);
    }

    #[test]
    fn manual_stop_is_never_respawned() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-sp-stop-{}", std::process::id())));
        let mut sp = SiteProcs::new(paths, Box::new(FakeSpawner::default()));
        sp.start("site", std::path::Path::new("/tmp"), "web", "true").unwrap();
        sp.stop("site", "web");
        let start = t0();
        sp.tick_at(start + Duration::from_secs(600));
        assert_eq!(sp.state_of("site", "web"), ServiceState::Stopped, "stop tay phải giữ nguyên Stopped");
        assert_eq!(sp.failures_of("site", "web"), 0);
    }

    #[test]
    fn manual_start_resets_the_failure_counter() {
        let paths = LaraluxPaths::new(std::env::temp_dir().join(format!("lara-sp-reset-{}", std::process::id())));
        let mut sp = SiteProcs::new(paths, Box::new(FakeSpawner::default()));
        sp.start("site", std::path::Path::new("/tmp"), "web", "true").unwrap();
        let start = t0();
        sp.tick_at(start + Duration::from_secs(1));
        assert!(sp.failures_of("site", "web") > 0);
        sp.start("site", std::path::Path::new("/tmp"), "web", "true").unwrap();
        assert_eq!(sp.failures_of("site", "web"), 0);
    }
```

Lưu ý người thực thi: `FakeSpawner` trả tiến trình giả — kiểm tra xem `is_alive()` của nó trả gì. Nếu nó báo **còn sống mãi**, các test trên cần một spawner giả báo chết; hãy tạo một `DeadSpawner` nhỏ trong module test (spawn ra process có `is_alive() == false`) và dùng nó cho hai test đầu. Đừng sửa `FakeSpawner` dùng chung.

- [ ] **Step 2: Chạy test để xác nhận fail**

Run: `cargo test -p laralux-core site_procs`
Expected: FAIL — chưa có `tick_at` / `failures_of` (không compile).

- [ ] **Step 3: Cài đặt**

Trong `core/src/site_procs.rs`:

1. Thêm import: `use std::time::{Duration, Instant};` và `use std::path::PathBuf;` (nếu chưa có).

2. Thêm hằng số + helper backoff ở cấp module:

```rust
/// Số lần chết liên tiếp trước khi thôi hồi sinh một process.
pub const MAX_RESTARTS: u32 = 5;
/// Sống liên tục đủ lâu thì coi như đã ổn định: bộ đếm lỗi được reset.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Khoảng chờ trước lần thử lại thứ `failures` (1-indexed). Lần thứ 5 không bao
/// giờ dùng tới vì `MAX_RESTARTS` chặn trước.
fn backoff_for(failures: u32) -> Duration {
    match failures {
        1 => Duration::from_secs(1),
        2 => Duration::from_secs(5),
        3 => Duration::from_secs(15),
        _ => Duration::from_secs(30),
    }
}

/// Mọi thứ cần để hồi sinh một process, cộng trạng thái backoff của nó.
struct Supervised {
    root: PathBuf,
    command: String,
    failures: u32,
    started_at: Instant,
    next_attempt_at: Option<Instant>,
    /// false sau khi user stop tay, hoặc sau khi đã bỏ cuộc.
    supervised: bool,
}
```

3. Thêm field vào `struct SiteProcs`: `supervision: HashMap<Key, Supervised>,` và khởi tạo `supervision: HashMap::new(),` trong `new()`.

4. Trong `start()`, sau khi spawn thành công, ghi/reset bản ghi giám sát (đây là điều kiện bắt buộc để respawn được — trước đây `command`/`root` bị vứt đi):

```rust
            Ok(handle) => {
                self.handles.insert(key.clone(), handle);
                self.states.insert(key.clone(), ServiceState::Running);
                // Starting by hand clears the slate: a fresh set of retries.
                self.supervision.insert(
                    key,
                    Supervised {
                        root: root.to_path_buf(),
                        command: command.to_string(),
                        failures: 0,
                        started_at: Instant::now(),
                        next_attempt_at: None,
                        supervised: true,
                    },
                );
                Ok(())
            }
```

5. Trong `stop()`, đánh dấu không giám sát nữa (thêm ngay sau phần gỡ handle hiện có):

```rust
        if let Some(s) = self.supervision.get_mut(&key) {
            s.supervised = false;
            s.next_attempt_at = None;
        }
```

`stop_site()` và `stop_all()` đã gọi `stop()` cho từng key nên tự động thừa hưởng.

6. Thay `refresh()` bằng cặp `refresh()` + `tick_at()`:

```rust
    pub fn refresh(&mut self) {
        self.tick_at(Instant::now());
    }

    /// Supervision tick. Split from `refresh()` so tests can feed synthetic
    /// instants instead of sleeping.
    pub fn tick_at(&mut self, now: Instant) {
        let mut running: Vec<Key> = Vec::new();
        let mut dead: Vec<Key> = Vec::new();
        for (key, h) in self.handles.iter_mut() {
            if h.is_alive() {
                running.push(key.clone());
            } else {
                dead.push(key.clone());
            }
        }

        for key in running {
            // Alive long enough → the earlier deaths were a blip, not a crash loop.
            if let Some(s) = self.supervision.get_mut(&key) {
                if s.failures > 0 && now.duration_since(s.started_at) >= STABLE_AFTER {
                    s.failures = 0;
                }
            }
            self.states.insert(key, ServiceState::Running);
        }

        for key in dead {
            self.handles.remove(&key);
            if let Some(s) = self.supervision.get_mut(&key) {
                if s.supervised {
                    s.failures += 1;
                    if s.failures >= MAX_RESTARTS {
                        s.supervised = false;
                        s.next_attempt_at = None;
                    } else {
                        s.next_attempt_at = Some(now + backoff_for(s.failures));
                    }
                }
            }
            self.states.insert(key, ServiceState::Crashed);
        }

        // Respawn whatever is due. Restarted regardless of exit code: `queue:work`
        // exits 0 on `queue:restart` and must come back.
        let due: Vec<Key> = self
            .supervision
            .iter()
            .filter(|(k, s)| {
                s.supervised
                    && !self.handles.contains_key(*k)
                    && s.next_attempt_at.map(|t| now >= t).unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in due {
            let Some((root, command)) = self
                .supervision
                .get(&key)
                .map(|s| (s.root.clone(), s.command.clone()))
            else {
                continue;
            };
            let spec = self.spawn_spec(&root, &key.0, &key.1, &command);
            match self.spawner.spawn(&spec) {
                Ok(handle) => {
                    self.handles.insert(key.clone(), handle);
                    self.states.insert(key.clone(), ServiceState::Running);
                    if let Some(s) = self.supervision.get_mut(&key) {
                        s.started_at = now;
                        s.next_attempt_at = None;
                    }
                }
                Err(_) => {
                    // A failed respawn counts like any other death: back off again.
                    if let Some(s) = self.supervision.get_mut(&key) {
                        s.failures += 1;
                        if s.failures >= MAX_RESTARTS {
                            s.supervised = false;
                            s.next_attempt_at = None;
                        } else {
                            s.next_attempt_at = Some(now + backoff_for(s.failures));
                        }
                    }
                    self.states.insert(key, ServiceState::Crashed);
                }
            }
        }
    }

    /// Consecutive failures for a process (0 when healthy or never started).
    pub fn failures_of(&self, site: &str, name: &str) -> u32 {
        self.supervision
            .get(&(site.to_string(), name.to_string()))
            .map(|s| s.failures)
            .unwrap_or(0)
    }
```

Lưu ý: `stop()` phải đặt state `Stopped` **sau** khi gỡ handle như code hiện có — giữ nguyên thứ tự đó để `tick_at` không thấy handle chết rồi ghi đè thành `Crashed`.

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `cargo test -p laralux-core site_procs`
Expected: PASS toàn bộ.

- [ ] **Step 5: Commit**

```bash
git add core/src/site_procs.rs
git commit -m "feat(site-procs): restart crashed processes with backoff and a give-up cap"
```

---

### Task 4: `ProcStatus.failures` + nối vào command layer

**Files:**
- Modify: `core/src/site_procs.rs` (struct `ProcStatus`)
- Modify: `src-tauri/src/commands.rs` (`site_procs_view`)

**Interfaces:**
- Consumes: `SiteProcs::failures_of` (Task 3).
- Produces: `ProcStatus` có thêm `pub failures: u32` (serialize xuống frontend).

- [ ] **Step 1: Thêm field**

Trong `core/src/site_procs.rs`, thêm vào `struct ProcStatus`:

```rust
    /// Consecutive restart failures; 0 when healthy.
    pub failures: u32,
```

- [ ] **Step 2: Điền giá trị**

Trong `src-tauri/src/commands.rs::site_procs_view`, thêm vào literal `ProcStatus { … }`:

```rust
            failures: sp.failures_of(name, &e.name),
```

- [ ] **Step 3: Build**

Run: `cargo check -p laralux-desktop`
Expected: sạch (nếu còn chỗ nào tạo `ProcStatus`, thêm field ở đó).

- [ ] **Step 4: Commit**

```bash
git add core/src/site_procs.rs src-tauri/src/commands.rs
git commit -m "feat(site-procs): expose the restart failure count to the UI"
```

---

### Task 5: Frontend — hiện "Retrying (n/5)…" và "gave up"

**Files:**
- Modify: `src/ipc/types.ts`
- Modify: `src/ui/modals/procs.ts`

**Interfaces:**
- Consumes: `ProcStatus.failures` (Task 4).

- [ ] **Step 1: Type**

Trong `src/ipc/types.ts`, thêm vào `interface ProcStatus` (sau `pid`):

```ts
  /** Consecutive restart failures; 0 when healthy. */
  failures: number;
```

- [ ] **Step 2: Hiển thị**

Trong `src/ui/modals/procs.ts`, trong hàm map dựng `rows`, ngay sau dòng `const running = …`, thêm:

```ts
          // Without this the user just sees "Crashed" and cannot tell whether
          // laralux is still retrying or has stopped trying.
          const note =
            p.failures === 0 || running
              ? ""
              : p.failures >= 5
                ? '<span class="proc-note">gave up after 5 restarts</span>'
                : '<span class="proc-note">retrying (' + p.failures + '/5)…</span>';
```

rồi chèn `note` vào trong khối `.proc-info`, ngay sau `proc-name`:

```ts
            '<div class="proc-name"><span class="dot bgc-' + meta.cls + '"></span>' + esc(p.name) + note + "</div>" +
```

- [ ] **Step 3: CSS**

Thêm vào `src/styles.css` (đặt cạnh các rule `.proc-*` sẵn có):

```css
.proc-note { margin-left: 8px; font-size: 11.5px; color: var(--text-muted); }
```

- [ ] **Step 4: Build**

Run: `npm run build`
Expected: `tsc --noEmit` + vite build sạch.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/types.ts src/ui/modals/procs.ts src/styles.css
git commit -m "feat(ui): show whether a crashed process is still being retried"
```

---

## Self-Review

**Spec coverage:**
- §1 Readiness gating trong `Orchestrator::start()` (poll 100ms, 15s/60s, timeout → Crashed) → Task 1. ✅
- §2 Tray không block main thread → Task 2. ✅
- §3 Auto-restart (lưu root+command, backoff 1/5/15/30s, bỏ cuộc sau 5, reset sau 30s, 2 bất biến) → Task 3. ✅
- §4 UI `ProcStatus.failures` + phân biệt retrying/gave-up → Task 4 (backend) + Task 5 (hiển thị). ✅
- Testing (không sleep thật) → Task 1 dùng `with_readiness` giá trị nhỏ; Task 3 dùng `tick_at` với mốc giả. ✅

**Type consistency:**
- `failures: u32` (Rust) ↔ `failures: number` (TS) khớp qua serde, dùng nhất quán Task 3 → 4 → 5.
- `tick_at(now: Instant)` / `refresh()` / `failures_of` khai ở Task 3, tiêu thụ ở Task 4.
- `with_readiness(timeout, init_timeout, interval)` khai và dùng cùng thứ tự tham số trong Task 1.
- Ngưỡng `5` xuất hiện ở `MAX_RESTARTS` (Task 3) và chuỗi UI "(n/5)" / "gave up after 5 restarts" (Task 5) — nếu đổi hằng số nhớ đổi cả chuỗi.

**Placeholder scan:** Task 1 Step 1 và Task 3 Step 1 chủ ý yêu cầu người thực thi kiểm tra hành vi `FakeSpawner`/`Dummy` sẵn có và tự tạo `DeadSpawner` nếu cần — đây là hướng dẫn cụ thể kèm tiêu chí, không phải placeholder logic.
