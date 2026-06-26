# Laralux — Plan 3d: Free Stack Ports from Distro systemd Services Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After installing the stack, stop+disable the distro's auto-started systemd services (nginx/mariadb/redis) so the app-managed processes can bind their ports (80/3306/6379).

**Architecture:** Acceptance testing showed `apt install` auto-starts and enables the distro `nginx`, `mariadb`, and `redis-server` systemd units, which hold ports 80/3306/6379 — so the app-managed nginx fails with `bind() to 0.0.0.0:80 failed (Address already in use)`. Per the "app orchestrates its own services, doesn't use OS services" design, the setup wizard now disables those system units (`systemctl disable --now`) through the existing `Privileged` seam. Also drops the php-fpm pool `user` directive that emits a harmless-but-noisy NOTICE when FPM runs as a non-root user.

**Tech Stack:** Rust, reuses `laralux_core` (Plans 1–3c); live tool: `systemctl` via pkexec/sudo.

## Global Constraints

- Stack systemd units to disable: `nginx`, `mariadb`, `redis-server`. Disable command: `systemctl disable --now <units>` (stops + disables in one shot), run through the `Privileged` escalator (pkexec in GUI, sudo in CLI).
- Disabling is non-fatal: a failure (e.g. a unit that doesn't exist) is collected into `SetupReport.errors`, never aborts setup.
- `run_setup` disables the stack units after the apt install steps and before/around the mailpit step (order among the post-apt steps is not significant).
- php-fpm pool config MUST NOT set a `user` directive (it is ignored when FPM runs as the launching non-root user and only produces a NOTICE).
- `core` keeps zero Tauri deps. Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD for all changes; failing test first. Live `systemctl` behavior is human-verified.

---

### Task 1: Disable distro systemd stack services during setup

**Files:**
- Modify: `core/src/privileged.rs` (add `disable_system_services` to trait + impls + helper)
- Modify: `core/src/setup.rs` (`run_setup` calls it after apt installs)

**Interfaces:**
- Consumes: existing `Privileged` trait, `run_escalated`, `FakePrivileged`.
- Produces:
  - `Privileged::disable_system_services(&self, units: &[String]) -> Result<(), PrivError>` (trait method) — implemented by `SudoPrivileged` (sudo), `PkexecPrivileged` (pkexec), `FakePrivileged` (records).
  - private helper `systemctl_disable_argv(units: &[String]) -> Vec<String>` = `["systemctl","disable","--now", <units...>]`.
  - `FakePrivileged::disabled_services(&self) -> Arc<Mutex<Vec<Vec<String>>>>` (records each call's unit list).
  - `run_setup` calls `privileged.disable_system_services(&["nginx".into(),"mariadb".into(),"redis-server".into()])` after the apt-install steps; a failure is pushed to `report.errors`.

- [ ] **Step 1: Write the failing privileged tests**

Add to the `tests` module in `core/src/privileged.rs`:

```rust
    #[test]
    fn systemctl_disable_argv_builds_disable_now() {
        let argv = systemctl_disable_argv(&["nginx".to_string(), "mariadb".to_string()]);
        assert_eq!(
            argv,
            vec![
                "systemctl".to_string(),
                "disable".to_string(),
                "--now".to_string(),
                "nginx".to_string(),
                "mariadb".to_string(),
            ]
        );
    }

    #[test]
    fn fake_records_disabled_services() {
        let f = FakePrivileged::new();
        let log = f.disabled_services();
        f.disable_system_services(&["nginx".to_string()]).unwrap();
        assert_eq!(log.lock().unwrap().len(), 1);
        assert_eq!(log.lock().unwrap()[0], vec!["nginx".to_string()]);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core privileged`
Expected: FAIL — `cannot find function systemctl_disable_argv` / `no method named disabled_services`.

- [ ] **Step 3: Implement in privileged.rs**

(a) Add the trait method to `pub trait Privileged`:

```rust
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError>;
```

(b) Add the helper near `apt_argv`:

```rust
fn systemctl_disable_argv(units: &[String]) -> Vec<String> {
    let mut argv = vec!["systemctl".to_string(), "disable".to_string(), "--now".to_string()];
    argv.extend(units.iter().cloned());
    argv
}
```

(c) Implement for `SudoPrivileged`:

```rust
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        run_escalated("sudo", &systemctl_disable_argv(units))
    }
```

(d) Implement for `PkexecPrivileged`:

```rust
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        run_escalated("pkexec", &systemctl_disable_argv(units))
    }
```

(e) Extend `FakePrivileged`: add the field, accessor, and impl:

```rust
// struct field:
    disabled_services: Arc<Mutex<Vec<Vec<String>>>>,
```
```rust
// impl FakePrivileged:
    pub fn disabled_services(&self) -> Arc<Mutex<Vec<Vec<String>>>> {
        self.disabled_services.clone()
    }
```
```rust
// impl Privileged for FakePrivileged:
    fn disable_system_services(&self, units: &[String]) -> Result<(), PrivError> {
        self.disabled_services.lock().unwrap().push(units.to_vec());
        Ok(())
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core privileged`
Expected: PASS — the 2 new tests plus the existing privileged tests.

- [ ] **Step 5: Write the failing run_setup test**

Add to the `tests` module in `core/src/setup.rs` (reuses the hermetic style from Plan 3c's `run_setup_adds_no_ppa`):

```rust
    #[test]
    fn run_setup_disables_distro_stack_services() {
        let root = std::env::temp_dir().join(format!("lara-disable-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        let priv_ = FakePrivileged::new();
        let disabled = priv_.disabled_services();
        let dl = FakeDownloader::new();

        let _ = run_setup(&paths, &priv_, &dl);

        let calls = disabled.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()]
        );
        std::fs::remove_dir_all(&root).ok();
    }
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test -p laralux-core setup::tests::run_setup_disables_distro_stack_services`
Expected: FAIL — `disabled` is empty (run_setup doesn't call `disable_system_services` yet).

- [ ] **Step 7: Call disable_system_services in run_setup**

In `core/src/setup.rs` `run_setup`, after the two apt-install blocks (the `if !php_packages.is_empty() { ... }` block) and before the mailpit step, add:

```rust
    // apt auto-starts + enables the distro nginx/mariadb/redis systemd units, which
    // hold ports 80/3306/6379. Disable them so the app-managed processes can bind.
    let stack_units = vec!["nginx".to_string(), "mariadb".to_string(), "redis-server".to_string()];
    if let Err(e) = privileged.disable_system_services(&stack_units) {
        report.errors.push(format!("disable system services: {e}"));
    }
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p laralux-core`
Expected: PASS — all tests green (the new run_setup test sees exactly one disable call with the three units).

- [ ] **Step 9: Build the binaries**

Run: `cargo build -p laralux-desktop && cargo build -p laraluxctl`
Expected: PASS — additive trait method; all impls present; no caller changes needed (GUI/CLI call `run_setup`, which now disables internally).

- [ ] **Step 10: Commit**

```bash
git add core/src/privileged.rs core/src/setup.rs
git commit -m "fix(core): disable distro systemd stack services so app can bind ports"
```

---

### Task 2: Drop the php-fpm pool `user` directive

**Files:**
- Modify: `core/src/service/php_fpm.rs` (remove the `user = ...` line from the generated pool config)

**Interfaces:**
- Consumes: nothing new.
- Produces: php-fpm pool config without a `user` directive (silences the `'user' directive is ignored when FPM is not running as root` NOTICE).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/service/php_fpm.rs`:

```rust
    #[test]
    fn write_config_omits_user_directive() {
        let tmp = std::env::temp_dir().join(format!("lara-php-nouser-{}", std::process::id()));
        let p = LaraluxPaths::new(tmp.clone());
        let svc = PhpFpmService::new("8.4");
        svc.write_config(&p).unwrap();
        let conf =
            std::fs::read_to_string(p.etc_for("php").join("8.4").join("php-fpm.conf")).unwrap();
        assert!(!conf.contains("user ="), "pool must not set a user directive");
        // sanity: the pool is still defined and listens on the socket
        assert!(conf.contains("[www]"));
        assert!(conf.contains("listen = "));
        std::fs::remove_dir_all(&tmp).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laralux-core php_fpm::tests::write_config_omits_user_directive`
Expected: FAIL — the generated config currently contains `user = <USER>`.

- [ ] **Step 3: Remove the user directive from the pool config**

In `core/src/service/php_fpm.rs`, in `write_config`'s pool config format string, delete the `user = {user}` line and remove the now-unused `user = std::env::var("USER")...` argument from the `format!` call. The `[www]` section should go straight from the header to `listen = {sock}`. Concretely the pool section becomes:

```rust
             [www]\n\
             listen = {sock}\n\
             listen.mode = 0660\n\
             pm = dynamic\n\
             pm.max_children = 10\n\
             pm.start_servers = 2\n\
             pm.min_spare_servers = 1\n\
             pm.max_spare_servers = 4\n",
```

and the `format!` arguments drop the `user = ...` binding entirely (keep `pid`, `log`, `sock`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core php_fpm`
Expected: PASS — the new test plus the existing php_fpm tests (which assert `[www]`, `listen = `, and the socket — none assert `user`).

- [ ] **Step 5: Commit**

```bash
git add core/src/service/php_fpm.rs
git commit -m "fix(core): drop php-fpm pool user directive (silences non-root NOTICE)"
```

---

## Self-Review

**1. Coverage of the reported failure:**
- nginx `bind() to 0.0.0.0:80 failed (Address already in use)` caused by the distro nginx systemd service holding :80: fixed by `run_setup` disabling `nginx`/`mariadb`/`redis-server` units (Task 1). The same fix frees 3306 (mariadb) and 6379 (redis) for the app-managed mariadb/redis. ✓
- php-fpm `'user' directive is ignored when FPM is not running as root` NOTICE: fixed by dropping the directive (Task 2). ✓

**2. Placeholder scan:** No "TBD/handle edge cases". The disable step is concrete (exact units, non-fatal). Live `systemctl` is human-verified.

**3. Type consistency:** `disable_system_services(&[String])` (Task 1) is implemented by all three `Privileged` impls and called by `run_setup` with `["nginx","mariadb","redis-server"]`; `systemctl_disable_argv` and `FakePrivileged::disabled_services()` are used consistently in the tests. Task 2 only removes a line + a `format!` arg — no signature changes. No caller (GUI/CLI) changes needed since both go through `run_setup`.

**Note:** This is a one-time setup action; once disabled, the distro units stay disabled across reboots, so the app owns the stack thereafter. Remaining tracked-hardening items (orphan-on-SIGKILL process groups, dnsmasq wildcard, idempotency gating) are unchanged and out of scope here.
