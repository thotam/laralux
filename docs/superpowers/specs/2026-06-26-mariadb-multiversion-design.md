# Laragon Linux — MariaDB multi-version (catalog + version-parameterized install)

**Date:** 2026-06-26
**Status:** Done (follow-on to the Versioned Tool Manager foundation; after nginx).
**Goal:** Let the Setup → MariaDB modal install and switch between multiple MariaDB versions, filling
the foundation seam (`tools::available_versions` / `tools::install_version`) for mariadb — mirroring
nginx/PHP. No UI changes.

---

## 1. Context & key difference from nginx/php

MariaDB binaries come from `archive.mariadb.org` as full-basedir `bintar-linux-systemd-<arch>` tarballs
(~360 MB). The existing `install_mariadb` pins `11.4.12`, extracts the whole tarball into
`bin/mariadb/<ver>/` and creates top-level `mariadbd`/`mariadb-install-db`/`mariadb` symlinks. URL
pattern (verified June 2026 for the curated set via HEAD):
`https://archive.mariadb.org/mariadb-<ver>/bintar-linux-systemd-<arch>/mariadb-<ver>-linux-systemd-<arch>.tar.gz`.

**Unlike nginx/php, MariaDB is stateful:** the data directory (`data/mariadb`) is shared across
versions and is version-sensitive. Forward use generally works; a major **downgrade** (switching to a
version older than the one that created the datadir) can refuse to start. This is an accepted caveat —
a failed switch leaves MariaDB stopped with an error toast (no data loss); the user can switch back.
mariadbd binds **:3306** (a high port), so — unlike nginx — **no setcap is needed on switch**, and the
generic `Orchestrator::replace_version` path works as-is.

## 2. Components

### 2.1 `core/src/mariadb_static.rs`
- `pub const KNOWN_MARIADB_VERSIONS: [&str; 4] = ["11.8.2", "11.4.12", "10.11.10", "10.6.20"];`
  (latest stable + current/maintained LTS lines; all HEAD-verified on archive.mariadb.org).
- Refactor `install_mariadb` into a thin wrapper over a new
  `install_mariadb_version(paths, version, downloader, runner, sink)` — the existing
  download→extract→symlink→`set_current` body, parameterized by `version` (idempotent; an unknown
  version 404s → `MariadbError::Download`). `install_mariadb` = `install_mariadb_version(MARIADB_VERSION, …)`,
  so `run_setup` behavior is unchanged.

### 2.2 `core/src/tools.rs`
- Add a shared `known_catalog(known, installed, active)` helper (known ∪ installed, newest-first,
  flags set) and use it for BOTH `Nginx` and `Mariadb` arms of `available_versions` (DRY).
- `install_version`: add a `Mariadb` arm → `install_mariadb_version`. (nginx/php unchanged; redis,
  mailpit, mkcert, composer remain `Unsupported`.)

### 2.3 Desktop
- No change. mariadb switching uses the existing generic `set_tool_version` → `replace_version`
  (stop → reap bin/mariadb orphans → set_current → start). The hardened `stop()` + orphan reaper
  release the ibdata lock before the new mariadbd starts (the orphan/lock fix from earlier).

## 3. Testing
- `mariadb_static`: catalog includes the pinned default; `mariadb_url` for a non-default version builds
  the expected systemd-bintar URL.
- `tools`: `available_versions(Mariadb)` returns the known set newest-first with installed/active flags;
  `install_version(Mariadb)` no longer `Unsupported`. The "still single-version" tests repointed to
  **Redis**.
- URLs HEAD-verified for all four curated versions. Full 360 MB download/extract is the same code path
  already live-proven for 11.4.12 — not re-run per version.
- `cargo test -p laragon-core` (180 pass); `cargo build -p laragon-desktop && -p laragonctl`.

## 4. Out of scope / backlog
- A pre-switch warning in the UI for stateful tools (datadir downgrade risk) — currently surfaced only
  as a failed-start error toast.
- Per-version datadirs / automatic `mariadb-upgrade` on forward switch.
- aarch64 curation (pattern maps; x86_64 verified).
- Remaining tools: redis → mailpit → mkcert → composer.
