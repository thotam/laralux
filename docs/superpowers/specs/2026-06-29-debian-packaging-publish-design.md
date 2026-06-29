# Laralux — Debian packaging & publish (toward apt install)

**Date:** 2026-06-29
**Status:** Design (approved for spec).
**Goal:** Make Laralux publicly installable as a Debian package, with an immediately-usable
download-and-install channel (`sudo apt install ./laralux_*.deb` from a public GitHub Release) and
the foundation + paperwork to pursue inclusion in the **official Debian/Ubuntu archives** (so a bare
`apt install laralux` eventually needs nothing added).

**Honest deliverable boundary (read first).** Acceptance into the official archives is an
**external, multi-month, gated process** (a Debian Developer must sponsor/upload; it then migrates
to testing and syncs into a *future* Ubuntu release). This project therefore delivers what is
buildable now — clean package metadata, a CI-built `.deb` on public Releases, a Debian source-package
skeleton, and the ITP/RFS submission drafts — and **documents** the remainder (sponsor review, full
offline/vendored buildd compliance) as work that happens outside this repo. Existing Ubuntu users
install via the Release `.deb` until/unless official acceptance lands.

---

## 1. Context & current state

- **App:** Tauri 2 desktop app. `src-tauri/tauri.conf.json` has `productName: "Laralux"`,
  `identifier: "com.laralux.linux"`, `version: "0.1.0"`, and already bundles `"targets": ["deb"]`
  but with **no Linux/deb metadata** (no maintainer, depends, section, description, homepage).
- **License:** workspace `Cargo.toml` declares `license = "MIT"`, but there is **no `LICENSE` file**
  in the repo (so the published code is effectively all-rights-reserved until one is added). MIT is
  DFSG-free and Debian-acceptable. The user approved **MIT as the public license**.
- **No `README`**, **no CI** (`.github/workflows` absent), **no `debian/` directory**.
- **Runtime nature to disclose:** Laralux downloads static third-party binaries (nginx, mariadb,
  php, …) into `~/laralux/` at the user's request at runtime. This is legal and user-initiated but is
  unusual for a Debian package and must be disclosed in the ITP so a sponsor isn't surprised.
- **Repo:** currently `github.com/thotam/laragon-linux`; the user will rename it to
  `github.com/thotam/laralux`. All URLs in this work use `thotam/laralux`; because GitHub auto-
  redirects and CI/Release use repo-relative paths, the rename is non-breaking (the user runs
  `git remote set-url origin https://github.com/thotam/laralux.git` afterward).
- **Build deps (from Tauri/webkit):** `libwebkit2gtk-4.1-dev libgtk-3-dev
  libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config` + Rust + Node/npm.
- **Runtime deps:** the webkit2gtk/gtk shared libs (auto via shlibs) plus
  `libayatana-appindicator3-1` (Tauri **dlopens** the tray lib, so it is NOT picked up by shlibs and
  must be declared explicitly).

**Decisions fixed by the design (approved):** license MIT (public); architecture **amd64** first
(arm64 later); **`.deb` only** (no AppImage/rpm); maintainer = `thotam <thanhtamtotaa@gmail.com>`;
package name `laralux`; homepage `https://github.com/thotam/laralux`.

## 2. Component A — Package metadata, LICENSE, README

- **`LICENSE`** (repo root): the standard MIT license text, copyright `2026 thotam`. Formalizes the
  declared license for both the public release and Debian copyright.
- **`README.md`** (repo root): concise project description + install instructions (the Release `.deb`
  one-liner) + a short "what it does" and a link to the docs. Serves the public-repo goal and the
  package `Homepage`.
- **`src-tauri/tauri.conf.json` `bundle` additions:** `publisher`, `homepage`, `license: "MIT"`,
  `category` (e.g. `DeveloperTool`), `shortDescription`, `longDescription`, and
  `bundle.linux.deb.depends` (add `libayatana-appindicator3-1`), `bundle.linux.deb.section`
  (`web` or `devel`), `bundle.linux.deb.priority` (`optional`). Exact key names/casing per the Tauri
  v2 schema are pinned in the plan.
- The maintainer field in the produced `.deb` derives from `publisher`/maintainer config; the plan
  sets it to `thotam <thanhtamtotaa@gmail.com>`.

## 3. Component B — CI build → public GitHub Release (install-today channel)

- **`.github/workflows/release.yml`**, triggered on pushing a tag matching `v*` (e.g. `v0.1.0`).
- Runner: `ubuntu-24.04`. Steps: checkout; install the build deps listed in §1; set up Node (with
  `npm ci`) and the Rust toolchain (with cargo cache); build the app and bundle the `.deb`.
- Use **`tauri-apps/tauri-action`** (it runs the Tauri build, produces the `.deb`, and creates/updates
  a GitHub Release with the artifact attached) OR an explicit `cargo tauri build` + `gh release`
  upload — the plan picks one and pins it. The Release is **public** and its body includes the
  install instructions:
  `sudo apt install ./laralux_<version>_amd64.deb` (apt resolves runtime deps from the user's
  existing archives; no repo added).
- Output artifact name: `laralux_<version>_amd64.deb`. amd64 only for v1 (arm64 is a documented
  follow-up: add an `aarch64` matrix leg later).
- The workflow has `permissions: contents: write` to create the Release. No secrets needed (no
  signing for the Release `.deb`; integrity via the Release page + checksums the action emits).

## 4. Component C — Debian source package (`debian/`) — submission foundation

A debhelper-based source package so Laralux can be built with `dpkg-buildpackage` and submitted.

- **`debian/control`:** `Source: laralux`, `Section: web`, `Priority: optional`,
  `Maintainer: thotam <thanhtamtotaa@gmail.com>`, `Build-Depends: debhelper-compat (= 13), cargo,
  rustc, nodejs, npm, libwebkit2gtk-4.1-dev, libgtk-3-dev, libayatana-appindicator3-dev,
  librsvg2-dev, libssl-dev, pkg-config`, `Standards-Version` (current), `Homepage`,
  `Rules-Requires-Root: no`. Binary stanza: `Package: laralux`, `Architecture: any`,
  `Depends: ${shlibs:Depends}, ${misc:Depends}, libayatana-appindicator3-1`, plus the long
  Description.
- **`debian/rules`:** debhelper `dh $@` with overrides — `override_dh_auto_build` runs `npm ci &&
  npm run build` then `cargo build --release -p laralux-desktop`; `override_dh_auto_install` installs
  the binary to `/usr/bin/laralux`, the `.desktop` file to `/usr/share/applications/`, and the icon
  to `/usr/share/icons/hicolor/…`. `override_dh_auto_test` is a no-op (or runs `cargo test -p
  laralux-core`). A `.desktop` file (`debian/laralux.desktop` or generated) and an `install` file map
  artifacts.
- **`debian/copyright`:** machine-readable **DEP-5** — upstream MIT for the project's own code, with
  a clear note that the binary downloads (not packaged) third-party software at runtime; bundled
  vendored assets (if any) listed.
- **`debian/changelog`:** initial `laralux (0.1.0-1) UNRELEASED; urgency=medium`, maintainer entry.
- **`debian/source/format`:** `3.0 (quilt)` (non-native; the orig tarball is the upstream source).
- **Honest scope note (also in §7):** this `debian/` **builds with network access** (cargo fetches
  crates, npm fetches packages). Debian's buildds build **offline** from vendored/packaged
  dependencies. Full compliance — vendoring the Rust crate tree (`dh-cargo`/`debcargo`) and providing
  the npm/frontend assets in a policy-allowed way — is the **largest remaining task** and is left to
  the sponsor-guided submission phase. The skeleton here is the correct starting point a DD expects,
  not a finished archive-ready package.

## 5. Component D — ITP / RFS / roadmap drafts (`docs/debian/`)

- **`docs/debian/ITP.md`:** the wnpp **Intent To Package** report text (Package: laralux; Version;
  Upstream Author: thotam <thanhtamtotaa@gmail.com>; URL: https://github.com/thotam/laralux; License:
  MIT; Section; the short + long Description) and how to file it (`reportbug wnpp`, or email to
  `submit@bugs.debian.org` against pseudo-package `wnpp` with `severity wishlist`, retitled
  `ITP: laralux -- <short desc>`). Includes the **runtime-download disclosure** paragraph.
- **`docs/debian/RFS.md`:** the **Request For Sponsorship** text for mentors.debian.net (package
  summary, dsc URL placeholder, lintian status, what review help is needed — especially the
  vendoring approach).
- **`docs/debian/roadmap.md`:** the path to the archives — (1) LICENSE/copyright ✓ (MIT), (2) clean
  `debian/` skeleton ✓, (3) **vendor Rust crates + handle npm offline** (remaining, large),
  (4) `lintian` clean, (5) file ITP, upload to mentors, find a sponsor, (6) Debian unstable →
  testing → next Ubuntu. Plus the note that current Ubuntu users use the Release `.deb` meanwhile.

## 6. Data / release flow
1. Developer tags a release: `git tag v0.1.0 && git push origin v0.1.0`.
2. CI (`release.yml`) builds the `.deb` and publishes a **public GitHub Release** with the artifact +
   `apt install ./…` instructions.
3. A user downloads `laralux_0.1.0_amd64.deb` and runs `sudo apt install ./laralux_0.1.0_amd64.deb`
   (apt pulls runtime deps from their existing archives; the tray dep is declared).
4. Toward official archives: build the source package locally (`dpkg-buildpackage -us -uc`), run
   `lintian`, file the ITP, push the source to mentors.debian.net, and request sponsorship (RFS).
   Acceptance and the eventual bare `apt install laralux` are external and time-gated.

## 7. Error handling / risks
- **CI build failure** (missing system dep, Tauri/webkit version drift): the workflow pins the runner
  image and the dep list; a failed build fails the tag job visibly and uploads nothing (no broken
  Release). The plan includes a manual `workflow_dispatch` trigger to dry-run before tagging.
- **deb dependency gaps** (tray lib dlopened): mitigated by the explicit `libayatana-appindicator3-1`
  Depends; verified by installing the `.deb` in a clean container and launching.
- **Debian non-compliance** (network build, un-vendored crates, runtime binary downloads): explicitly
  documented as remaining/at-risk in §4 and `roadmap.md`; not hidden. A sponsor may require changes
  or decline — disclosed upfront.
- **Repo rename timing:** URLs assume `thotam/laralux`; if the rename hasn't happened yet, the
  Release still works (CI is repo-relative) and only the documented Homepage/clone URLs need the
  rename to resolve.

## 8. Testing / verification
- `.deb` builds in CI for a test tag (use `workflow_dispatch` or a `v0.0.0-test` tag) and attaches to
  a (pre-)release.
- Local verification: `sudo apt install ./laralux_*.deb` on a clean Ubuntu → app launches from the
  app menu and the tray works → `sudo apt remove laralux` cleans up.
- `lintian laralux_*.deb` reviewed; the plan records the baseline warnings to drive toward clean.
- `dpkg-buildpackage -us -uc` from the `debian/` skeleton produces a `.deb` locally.
- Markdown drafts (ITP/RFS/roadmap) reviewed for accuracy (correct package name, license, URLs,
  disclosure paragraph present).

## 9. Out of scope / backlog
- arm64 (`aarch64`) build leg — documented follow-up in the CI matrix.
- AppImage / rpm / Flatpak / Snap targets.
- A self-hosted signed APT repo (GitHub Pages + GPG) — explicitly **not** chosen; the user targets
  the official archives + Release `.deb`.
- Full Debian-buildd offline compliance (crate vendoring via `dh-cargo`/`debcargo`, packaged npm) —
  the externally-gated remainder, tracked in `roadmap.md`.
- The actual filing/acceptance of the ITP/RFS and any sponsor-requested changes (external).
- Code signing / reproducible builds.
