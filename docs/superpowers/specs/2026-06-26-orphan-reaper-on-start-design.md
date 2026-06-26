# Laragon Linux — Orphan-process reaper on Start (incl. php-fpm)

**Date:** 2026-06-26
**Status:** Design (goal-directed).
**Goal:** Before spawning the managed stack, kill any *managed* process left over from a
prior session (an "orphan") so it cannot hold a port/socket/datadir lock and crash the
fresh service. Cover php-fpm specifically: a PHP version switch must reap an orphan
php-fpm (master + workers) that the orchestrator no longer tracks.

---

## 1. Context & current state

The orchestrator only knows about the child processes *it* spawned (`handles:
HashMap<ServiceKind, Box<dyn Process>>`). When `cargo run` is Ctrl-C'd (or the app dies
unexpectedly) the children are reparented to init and keep running. On the next Start,
`Orchestrator::start()` is idempotent only for processes it *tracks*; an untracked
orphan from the previous session still binds `:80/:443`, `tmp/php-fpm.sock`,
`127.0.0.1:5353`, or holds `data/mariadb/ibdata1` → the new service "crashes".

This is the documented root cause behind the recurring "X crashed" reports (nginx ports,
redis RDB, mariadbd `Unable to lock ./ibdata1`, coredns:5353). `kill_stale_coredns`
(desktop) already does this *ad hoc* for CoreDNS by `pkill -f <bin/coredns>`; we
generalize that to every managed tool, in `core`, keyed on the executable path rather
than a fragile cmdline match.

For PHP version switch, `replace_php_version` stops the *tracked* php-fpm (SIGTERM) and
starts the new one, but an orphan php-fpm master from a prior session is never killed —
it lingers (holds the old socket fd, wastes RAM) and can respawn workers.

## 2. Approach

Identify a "managed process" by resolving `/proc/<pid>/exe` (the kernel's canonical path
to the running executable) and testing whether it lives under `~/laragon/bin`. This is
robust where cmdline matching is not: nginx/php-fpm rewrite their argv (proctitle), but
`/proc/<pid>/exe` always points at the real binary; php-fpm workers share the master's
exe, so they match too. All managed processes run as the same user, so the symlink is
readable without privilege.

Add `core/src/orphans.rs`:
- `pub fn is_managed_exe(exe: &Path, match_dir: &Path) -> bool` — pure: true when `exe`
  (after stripping a trailing `" (deleted)"`) is `match_dir` or below it. `Path::starts_with`
  is component-wise, so `/x/bin` matches `/x/bin/php/8.4/...` but not `/x/binary`.
- `fn scan(match_dir, keep: &[u32]) -> Vec<u32>` — walk `/proc/<pid>/exe`, return PIDs
  whose exe `is_managed_exe(match_dir)`, excluding our own PID and any in `keep`.
- `pub fn reap(match_dir: &Path, keep: &[u32]) -> Vec<u32>` — scan, then SIGTERM every
  target, poll up to ~2 s for them to exit, SIGKILL survivors, wait until all are gone
  (so the listening socket / lock is released before the caller spawns a replacement).
  Returns the PIDs it acted on. No-op (empty Vec) when nothing matches.

Wire into the orchestrator:
- `Orchestrator::reap_orphans(&mut self) -> Vec<u32>` — `reap(bin(), keep = tracked PIDs)`.
- Call it at the **top** of `start_all`, before any spawn. `keep` = already-tracked PIDs,
  so a re-Start-All never kills the live stack (those survive; idempotent `start` then
  skips them) while orphans (untracked) are reaped. In a fresh session `handles` is empty
  → every managed orphan is reaped → clean stack.
- In `replace_php_version`, after stopping the tracked php-fpm and before starting the
  new version, call `reap(bin()/php, keep = tracked PIDs)`. This kills any orphan php-fpm
  under `bin/php` (and guarantees the just-SIGTERM'd old master is fully dead) before the
  new master binds the socket. nginx/mariadb/etc. are not under `bin/php`, so they are
  untouched regardless of `keep`.

`kill_stale_coredns` in desktop is kept: CoreDNS is (re)started via `set_coredns` earlier
in `run_full_start`, before `start_all`'s reap, so it needs its own pre-clear.

## 3. Files
- Create: `core/src/orphans.rs` (+ `pub mod orphans;` and `pub use orphans::reap;` in `lib.rs`).
- Modify: `core/src/orchestrator.rs` — `reap_orphans`, call in `start_all`, php-scoped reap in `replace_php_version`.
- No desktop change required (both UI and tray Start route through `start_all`; CLI `up` too).

## 4. Behavior & error handling
- Best-effort and self-healing: `reap` is a no-op when `/proc` is unreadable or nothing
  matches; failures to signal a PID are ignored (it may have exited between scan and kill).
- Only processes whose **executable** is under `~/laragon/bin` are ever signalled — an
  unrelated system nginx/mariadb/redis (e.g. `/usr/sbin/...`) is never touched. (Legacy
  apt binaries lived outside `bin/`; post-no-apt they no longer exist, and were out of
  scope anyway.)
- SIGTERM first (php-fpm master reaps its workers, mariadbd flushes), SIGKILL only for
  processes that ignore SIGTERM within the grace window.

## 5. Testing (TDD)
- `is_managed_exe`: match at dir, match below dir, sibling non-match (`/x/binary`),
  `php` sub-scope, `" (deleted)"` suffix handling. (pure unit tests)
- `reap` (live): copy `/bin/sleep` into `<tmp>/bin/sleep-cur/sleep`, spawn it, then
  `reap(<tmp>/bin, &[])` → process gone. Second case: `reap(<tmp>/bin, &[pid])` → process
  still alive (kept), then clean up. Covers scan + signal + wait end-to-end.
- Existing orchestrator tests stay green: `start_all` with `FakeSpawner` under a `/tmp`
  root scans `/proc` but matches nothing there → reap is a no-op.
- `cargo test -p laragon-core`; `cargo build -p laragon-desktop && cargo build -p laragonctl`.

## 6. Out of scope (backlog)
- Hardening tracked `RealProcess::stop` with a SIGKILL fallback (the Start-time reaper is
  the safety net; tracked SIGTERM stop is sufficient and the reaper catches any straggler
  on the next Start).
- Reaping non-managed leftovers (system services / apt binaries outside `bin/`).
