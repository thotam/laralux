# Laragon Linux — certutil bundle for browser (NSS) trust, no-apt

**Date:** 2026-06-26
**Status:** Design (goal-directed); end-to-end verified before implementation.
**Goal:** Make Firefox/Chrome trust the mkcert CA for `*.dev` without an `apt install`
step and without the raw `mkcert` certutil warning. mkcert needs `certutil` (from
`libnss3-tools`) to write the browsers' NSS stores. We download it as a binary bundle
(Ubuntu `.deb` extraction), like the rest of the stack.

---

## 1. Context & current state

`run_setup` resolves the bundled `mkcert` binary and calls
`privileged.install_mkcert_ca(&mk)`, which runs `mkcert -install` **as the current user**
(it does NOT escalate — the escalator is mkcert itself). That installs the CA into the
system trust store but emits:

> Warning: "certutil" is not available, so the CA can't be automatically installed in
> Firefox and/or Chrome/Chromium! Install "certutil" with "apt install libnss3-tools" …

The system store covers OS tools/curl; **browsers use their own NSS stores** (Chrome:
`~/.pki/nssdb`; Firefox: per-profile `cert9.db`), which mkcert can only populate via
`certutil`. There is **no clean single-binary certutil**: Mozilla ships only source for
Linux, and certutil dynamically links the whole NSS/NSPR library set. The user accepts an
**Ubuntu/Debian-only** solution (the target platform; kernel is `-generic`).

## 2. Approach (verified end-to-end before coding)

Download four pinned Ubuntu `.deb`s, extract each with `dpkg-deb -x` (always present on
Debian/Ubuntu), and bundle `certutil` + every extracted `*.so*` into
`bin/certutil/<ver>/{bin,lib}`. Then run `mkcert -install` **as the user** with
`PATH` prepending the certutil bin dir, `LD_LIBRARY_PATH` = the bundled lib dir, and
`TRUST_STORES=nss` (NSS stores only — the system store is handled separately). Splitting
by `TRUST_STORES` lets the existing privileged call do `system` only (silencing its
certutil warning) and the new user call do `nss`.

Verified manually: the four debs (~2.8 MB) extract to 14 `.so` (~5.7 MB bundle);
`ldd certutil` resolves fully with `LD_LIBRARY_PATH`; `TRUST_STORES=nss mkcert -install`
prints "installed in the Firefox and/or Chrome/Chromium trust store 🦊" (no warning) and
the CA appears in `~/.pki/nssdb`.

Pinned set — Ubuntu 24.04 LTS "noble" (glibc 2.39; coherent NSS 3.98 across nss + tools):
- `pool/main/n/nss/libnss3_3.98-1build1_amd64.deb`
- `pool/main/n/nss/libnss3-tools_3.98-1build1_amd64.deb` (provides `certutil`)
- `pool/main/n/nspr/libnspr4_4.35-1.1build1_amd64.deb`
- `pool/main/s/sqlite3/libsqlite3-0_3.45.1-1ubuntu2_amd64.deb`
(`zlib1g`/`libc6`/`libstdc++` are Priority:required and resolved from the system.)

## 3. Files
- Create: `core/src/certutil_static.rs` (+ `pub mod` / `pub use` in `lib.rs`).
- Modify: `core/src/setup.rs` — `SetupReport { certutil_fetched, mkcert_nss }`; a step
  after the mkcert system-CA step that installs the bundle then runs the NSS install.
- Modify: `core/src/privileged.rs` — `install_mkcert_ca` runs mkcert with
  `TRUST_STORES=system` (system store only; no certutil warning). Trait signature
  unchanged; `FakePrivileged` unchanged.

### `core/src/certutil_static.rs`
- `CERTUTIL_VERSION = "3.98"`; pool base + the four `.deb` relative paths (consts).
- `certutil_arch() -> Option<&'static str>` — `x86_64 → "amd64"`, else `None`.
- `deb_url(rel) -> String` — `http://archive.ubuntu.com/ubuntu/pool/main/<rel>`.
- `certutil_dir/bin/lib(paths)` helpers → `bin/certutil/<ver>/…`.
- `install_certutil(paths, downloader, runner, sink) -> Result<PathBuf, CertutilError>`:
  idempotent (returns the bin path if `bin/certutil/<ver>/bin/certutil` already exists);
  arch-gate (amd64 only); download the four debs to `tmp/` (the big nss one via
  `fetch_with_progress`); `dpkg-deb -x` each into a fresh `tmp/certutil-stage`; copy
  `usr/bin/certutil` → bin (chmod 0755) and `usr/lib/x86_64-linux-gnu/*.so*` → lib;
  return the certutil path.
- `mkcert_install_nss(mkcert_bin, certutil_bin_dir, certutil_lib_dir) -> Result<(), CertutilError>`:
  run `mkcert -install` with `PATH` prepended, `LD_LIBRARY_PATH` set, `TRUST_STORES=nss`.
- `enum CertutilError`: `Arch(String)`, `Download(String)`, `Extract(String)`,
  `Layout(String)`, `Io(#[from] io::Error)`.

## 4. Behavior & error handling
- Best-effort and idempotent — failures land in `report.errors`; never abort other setup.
- Only the bundled certutil + its own libs are used (own `LD_LIBRARY_PATH`); the system's
  NSS/glibc are not relied on, so it works across machines (one Ubuntu/Debian family).
- The system-store install is unchanged in privilege model (still user-run); we only
  scope it to `system` so the certutil warning is gone.

## 5. Testing
- Unit: `certutil_arch` map; `deb_url` exact string; `certutil_bin/lib` paths.
- `install_certutil` / `mkcert_install_nss` are **live-verified** (download + extract +
  mkcert), like the other heavy installers.
- Existing `run_setup` tests stay green (they assert on disabled-services count and Step
  events, not on `report.errors`; the new step uses byte-progress, emits no Step events).
- `cargo test -p laragon-core`; `cargo build -p laragon-desktop && -p laragonctl`.

## 6. Out of scope (backlog)
- aarch64 (`arm64` debs map the same way; only amd64 is the verified target).
- Non-Debian families (Fedora/Arch) — different package format; YAGNI now.
- Escalating the *system*-store install for a truly fresh machine (pre-existing: mkcert
  is run as user; unchanged here).
