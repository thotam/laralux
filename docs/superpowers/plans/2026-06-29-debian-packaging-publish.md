# Debian Packaging & Publish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Laralux publicly installable as a Debian `.deb` (built in CI, attached to a public GitHub Release, installable via `sudo apt install ./Laralux_*.deb`) and lay the Debian source-package + ITP/RFS foundation toward the official archives.

**Architecture:** Four mostly-independent deliverables: (A) package metadata + `LICENSE` + `README`; (B) a tag-triggered GitHub Actions release workflow using `tauri-apps/tauri-action`; (C) a debhelper-based `debian/` source package; (D) ITP/RFS/roadmap drafts under `docs/debian/`. No application code changes.

**Tech Stack:** Tauri 2 deb bundler, GitHub Actions (`tauri-apps/tauri-action@v0`), debhelper/dpkg, Markdown.

## Global Constraints

- **This is packaging/devops/docs — no application logic changes.** Do not touch `core/`, `src/`, or `src-tauri/src/`.
- License is **MIT** (public). Architecture **amd64** only for v1 (arm64 is a documented follow-up). **`.deb` only** (no AppImage/rpm). Maintainer `thotam <thanhtamtotaa@gmail.com>`. Package name `laralux`. Homepage `https://github.com/thotam/laralux`.
- **Honest scope:** the `debian/` source package builds **with network access** (npm/cargo fetch); full Debian-buildd offline/vendored compliance is explicitly out of scope and documented in `docs/debian/roadmap.md`. Do not claim archive-readiness.
- Runtime-download disclosure: Laralux downloads third-party static binaries into `~/laralux/` at runtime; this must be stated in `debian/control` Description and the ITP.
- The dlopened tray library `libayatana-appindicator3-1` must be an explicit `Depends` (shlibs misses it).
- Commits: **no `Co-Authored-By` trailer.** Work on `master` (direct commits, this session's convention).
- Verified facts (Tauri v2): bundle keys `publisher`, `homepage`, `licenseFile`, `category`, `shortDescription`, `longDescription`, and `bundle.linux.deb.{depends,section,priority}`; deb output path `src-tauri/target/release/bundle/deb/<productName>_<version>_amd64.deb`; `tauri-apps/tauri-action@v0` inputs `tagName`/`releaseName`/`releaseBody`/`args` with `env.GITHUB_TOKEN` and `permissions: contents: write`.

---

### Task 1: Package metadata + LICENSE + README

**Files:**
- Create: `LICENSE`
- Create: `README.md`
- Modify: `src-tauri/tauri.conf.json` (the `bundle` object)

**Interfaces:**
- Consumes: nothing.
- Produces: a `LICENSE` file at repo root (referenced by `bundle.licenseFile = "../LICENSE"` and by `debian/copyright` in Task 3); deb metadata that Task 2's build embeds.

- [ ] **Step 1: Create `LICENSE` (MIT)**

```
MIT License

Copyright (c) 2026 thotam

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 2: Create `README.md`**

```markdown
# Laralux

A native Linux clone of [Laragon](https://laragon.org/): a system-tray + GUI
manager for a local web-development stack — **nginx, PHP-FPM, MariaDB,
PostgreSQL, MongoDB, Redis, Mailpit** — with pretty `*.dev` HTTPS, automatic
[mkcert](https://github.com/FiloSottile/mkcert) SSL, multi-version tool
switching, and per-site Procfile process management.

Tool binaries are downloaded as **static builds into `~/laralux/`** at your
request at runtime — no `apt` needed for the managed stack.

## Install (Debian/Ubuntu, x86_64)

Download the latest `Laralux_<version>_amd64.deb` from the
[Releases page](https://github.com/thotam/laralux/releases) and install it with
apt (it resolves the runtime dependencies automatically):

```sh
sudo apt install ./Laralux_<version>_amd64.deb
```

To remove:

```sh
sudo apt remove laralux
```

## License

[MIT](LICENSE) © thotam
```

- [ ] **Step 3: Expand the `bundle` object in `src-tauri/tauri.conf.json`**

Replace the existing `bundle` object:

```json
  "bundle": {
    "active": true,
    "targets": ["deb"],
    "icon": ["icons/icon.png"]
  }
```

with:

```json
  "bundle": {
    "active": true,
    "targets": ["deb"],
    "icon": ["icons/icon.png"],
    "publisher": "thotam",
    "homepage": "https://github.com/thotam/laralux",
    "licenseFile": "../LICENSE",
    "category": "DeveloperTool",
    "shortDescription": "Native Linux local web-development environment manager",
    "longDescription": "Laralux is a native Linux clone of Laragon: a tray and GUI manager for a local web-development stack (nginx, PHP-FPM, MariaDB, PostgreSQL, MongoDB, Redis, Mailpit) with pretty *.dev HTTPS, automatic mkcert SSL, and multi-version tools downloaded as static binaries into ~/laralux.",
    "linux": {
      "deb": {
        "depends": ["libwebkit2gtk-4.1-0", "libgtk-3-0", "libayatana-appindicator3-1"],
        "section": "web",
        "priority": "optional"
      }
    }
  }
```

- [ ] **Step 4: Validate the JSON + files**

Run: `node -e "JSON.parse(require('fs').readFileSync('src-tauri/tauri.conf.json','utf8')); console.log('json ok')"`
Expected: prints `json ok` (no parse error).

Run: `test -f LICENSE && test -f README.md && head -1 LICENSE`
Expected: prints `MIT License`.

Note: a full `.deb` build is NOT part of this task's gate (it needs the webkit/tauri toolchain and is exercised by CI in Task 2 + the user's manual smoke). This task verifies config validity + file presence only.

- [ ] **Step 5: Commit**

```bash
git add LICENSE README.md src-tauri/tauri.conf.json
git commit -m "chore(release): MIT LICENSE, README, and deb bundle metadata"
```

---

### Task 2: CI release workflow (`.github/workflows/release.yml`)

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: the deb metadata from Task 1 (the build reads `tauri.conf.json`).
- Produces: a public GitHub Release with `Laralux_<version>_amd64.deb` attached when a `v*` tag is pushed.

- [ ] **Step 1: Create the workflow**

```yaml
name: release

on:
  push:
    tags:
      - 'v*'
  workflow_dispatch: {}

jobs:
  release:
    permissions:
      contents: write
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4

      - name: Install Linux build dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev \
            libgtk-3-dev \
            libayatana-appindicator3-dev \
            librsvg2-dev \
            libssl-dev \
            pkg-config \
            patchelf \
            build-essential

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: lts/*

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Rust cache
        uses: swatinem/rust-cache@v2
        with:
          workspaces: './src-tauri -> target'

      - name: Install frontend dependencies
        run: npm ci

      - name: Build and release
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: 'Laralux ${{ github.ref_name }}'
          releaseBody: |
            ## Install (Debian/Ubuntu, x86_64)

            Download `Laralux_<version>_amd64.deb` below, then:

            ```sh
            sudo apt install ./Laralux_<version>_amd64.deb
            ```

            apt resolves the runtime dependencies automatically (no repository to add).
          releaseDraft: false
          prerelease: false
          args: ''
```

- [ ] **Step 2: Validate the workflow YAML**

Run (try a parser that is present; at least one of these will run):
`ruby -ryaml -e "YAML.load_file('.github/workflows/release.yml'); puts 'yaml ok'" 2>/dev/null || python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('yaml ok')" 2>/dev/null || echo "NO_YAML_PARSER_AVAILABLE — verify by review"`
Expected: prints `yaml ok` (or, if no parser is installed, `NO_YAML_PARSER_AVAILABLE` — in that case confirm validity by careful review of indentation and the `on`/`jobs` structure).

Note: the workflow only runs on GitHub (a real run needs a pushed `v*` tag or a manual `workflow_dispatch` from the Actions tab) — that end-to-end run is the user's manual smoke, documented in the report. This task's gate is YAML validity + structural correctness.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci(release): build deb and publish GitHub Release on v* tags"
```

---

### Task 3: Debian source package (`debian/`)

**Files:**
- Create: `debian/control`
- Create: `debian/rules` (executable, `chmod +x`)
- Create: `debian/changelog`
- Create: `debian/copyright`
- Create: `debian/source/format`
- Create: `debian/laralux.desktop`
- Create: `debian/.gitignore`

**Interfaces:**
- Consumes: the `LICENSE` (Task 1) for the copyright text; the workspace binary `target/release/laralux-desktop` (built by `debian/rules`).
- Produces: a buildable (network-permitted) Debian source package for `dpkg-buildpackage`, the submission foundation referenced by Task 4's drafts.

- [ ] **Step 1: `debian/control`**

```
Source: laralux
Section: web
Priority: optional
Maintainer: thotam <thanhtamtotaa@gmail.com>
Build-Depends: debhelper-compat (= 13),
 cargo,
 rustc,
 nodejs,
 npm,
 libwebkit2gtk-4.1-dev,
 libgtk-3-dev,
 libayatana-appindicator3-dev,
 librsvg2-dev,
 libssl-dev,
 pkg-config
Standards-Version: 4.7.0
Homepage: https://github.com/thotam/laralux
Rules-Requires-Root: no

Package: laralux
Architecture: any
Depends: ${shlibs:Depends},
 ${misc:Depends},
 libayatana-appindicator3-1
Description: native Linux local web-development environment manager
 Laralux is a native Linux clone of Laragon: a system-tray and GUI manager for
 a local web-development stack (nginx, PHP-FPM, MariaDB, PostgreSQL, MongoDB,
 Redis, Mailpit) with pretty *.dev HTTPS, automatic mkcert SSL, and
 multi-version tool switching.
 .
 The managed tool binaries are downloaded as static builds into ~/laralux at
 the user's request at runtime; they are not shipped in this package.
```

- [ ] **Step 2: `debian/rules` (make executable)**

```make
#!/usr/bin/make -f
export DH_VERBOSE = 1
# Keep cargo's downloads inside the build tree (network build — see roadmap.md).
export CARGO_HOME = $(CURDIR)/debian/cargo_home

%:
	dh $@

override_dh_auto_build:
	npm ci
	npm run build
	cargo build --release -p laralux-desktop

override_dh_auto_install:
	install -Dm755 target/release/laralux-desktop debian/laralux/usr/bin/laralux
	install -Dm644 debian/laralux.desktop debian/laralux/usr/share/applications/laralux.desktop
	install -Dm644 src-tauri/icons/icon.png debian/laralux/usr/share/icons/hicolor/512x512/apps/laralux.png

override_dh_auto_test:
	cargo test -p laralux-core

override_dh_auto_clean:
	rm -rf target debian/cargo_home node_modules dist
```

After creating it: `chmod +x debian/rules`.

- [ ] **Step 3: `debian/changelog`**

```
laralux (0.1.0-1) UNRELEASED; urgency=medium

  * Initial Debian packaging.

 -- thotam <thanhtamtotaa@gmail.com>  Sun, 29 Jun 2026 12:00:00 +0700
```

(If `dpkg-dev` is installed, you may regenerate the trailer date with `date -R`; the format above is a valid RFC 2822 date.)

- [ ] **Step 4: `debian/copyright` (DEP-5)**

```
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: laralux
Upstream-Contact: thotam <thanhtamtotaa@gmail.com>
Source: https://github.com/thotam/laralux

Files: *
Copyright: 2026 thotam
License: MIT

License: MIT
 Permission is hereby granted, free of charge, to any person obtaining a copy
 of this software and associated documentation files (the "Software"), to deal
 in the Software without restriction, including without limitation the rights
 to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 copies of the Software, and to permit persons to whom the Software is
 furnished to do so, subject to the following conditions:
 .
 The above copyright notice and this permission notice shall be included in all
 copies or substantial portions of the Software.
 .
 THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 SOFTWARE.
```

- [ ] **Step 5: `debian/source/format`**

```
3.0 (quilt)
```

- [ ] **Step 6: `debian/laralux.desktop`**

```
[Desktop Entry]
Name=Laralux
Comment=Local web-development environment manager
Exec=laralux
Icon=laralux
Terminal=false
Type=Application
Categories=Development;WebDevelopment;
```

- [ ] **Step 7: `debian/.gitignore`** (keep build artifacts out of git)

```
/laralux/
/cargo_home/
/files
/*.substvars
/*.debhelper
/debhelper-build-stamp
```

- [ ] **Step 8: Validate the packaging files**

Run: `test -x debian/rules && echo "rules executable"`
Expected: prints `rules executable`.

Run (changelog parse, if `dpkg-dev` is installed): `dpkg-parsechangelog -l debian/changelog 2>/dev/null | grep -E "^(Source|Version): " || echo "dpkg-dev not installed — verify changelog by review"`
Expected: prints `Source: laralux` and `Version: 0.1.0-1` (or the dpkg-dev-absent note).

Run (control parse sanity): `grep -E "^(Source|Package): laralux$" debian/control | sort -u`
Expected: prints both `Source: laralux` and `Package: laralux`.

Note: a full `dpkg-buildpackage -us -uc` (and `lintian`) needs the Debian packaging toolchain (debhelper, dpkg-dev, lintian) plus the build-deps; that end-to-end build is the user's manual smoke (documented in the report and `roadmap.md`), not this task's gate.

- [ ] **Step 9: Commit**

```bash
git add debian/
git commit -m "build(debian): debhelper source package skeleton (network build)"
```

---

### Task 4: ITP / RFS / roadmap drafts (`docs/debian/`)

**Files:**
- Create: `docs/debian/ITP.md`
- Create: `docs/debian/RFS.md`
- Create: `docs/debian/roadmap.md`

**Interfaces:**
- Consumes: the package facts from Tasks 1 & 3 (name `laralux`, MIT, maintainer, homepage, Description).
- Produces: ready-to-file submission text + the explicit roadmap of externally-gated remaining work.

- [ ] **Step 1: `docs/debian/ITP.md`**

```markdown
# ITP: laralux — native Linux local web-development environment manager

File this as a **wnpp** Intent To Package bug before uploading to mentors.

**How to file:** install `reportbug`, then `reportbug wnpp` → choose **ITP**.
Or email `submit@bugs.debian.org` with the body below (pseudo-package `wnpp`,
severity `wishlist`), subject `ITP: laralux -- native Linux local web-development environment manager`.

---

Package: wnpp
Severity: wishlist

* Package name    : laralux
  Version         : 0.1.0
  Upstream Author : thotam <thanhtamtotaa@gmail.com>
* URL             : https://github.com/thotam/laralux
* License         : MIT
  Programming Lang: Rust, TypeScript
  Description      : native Linux local web-development environment manager

Laralux is a native Linux clone of Laragon: a system-tray and GUI manager for a
local web-development stack (nginx, PHP-FPM, MariaDB, PostgreSQL, MongoDB,
Redis, Mailpit) with pretty *.dev HTTPS, automatic mkcert SSL, and multi-version
tool switching.

Disclosure: the managed tool binaries (nginx, mariadb, php, …) are downloaded as
upstream static builds into ~/laralux at the user's request at runtime; they are
not shipped in the package. Packaging review should account for this design.

I will maintain this package. Sponsorship by a Debian Developer is requested
(RFS to follow on mentors.debian.net).
```

- [ ] **Step 2: `docs/debian/RFS.md`**

```markdown
# RFS: laralux

Request For Sponsorship for mentors.debian.net (post after uploading the source
package and filing the ITP).

---

Dear mentors,

I am looking for a sponsor for my package **laralux**.

* Package name : laralux
* Version      : 0.1.0-1
* Upstream URL : https://github.com/thotam/laralux
* License      : MIT
* Section      : web

It builds a single binary package: `laralux` — a native Linux local
web-development environment manager (a Laragon clone) built with Tauri 2
(Rust + a TypeScript/WebKitGTK frontend).

Known review topics I would value guidance on:

1. **Dependency vendoring.** The current `debian/rules` builds with network
   access (cargo + npm fetch). I need help converting to a policy-compliant
   offline build — vendoring the Rust crates (dh-cargo/debcargo) and providing
   the frontend assets in an acceptable way.
2. **Runtime downloads.** At the user's request the app downloads upstream
   static tool binaries into ~/laralux at runtime (not shipped in the package).
3. **lintian.** Current status and any overrides needed.

Source and packaging: https://github.com/thotam/laralux (see debian/).

Thank you,
thotam <thanhtamtotaa@gmail.com>
```

- [ ] **Step 3: `docs/debian/roadmap.md`**

```markdown
# Roadmap to the official Debian/Ubuntu archives

Goal: a bare `sudo apt install laralux` that needs nothing added, by getting the
package into the official archives. This is an external, multi-month process
gated by a Debian Developer sponsor. Status of each step:

1. **License & copyright** — DONE. MIT (`LICENSE`, `debian/copyright` DEP-5).
2. **debian/ skeleton** — DONE. debhelper-compat 13, builds a `.deb` locally
   with `dpkg-buildpackage` (network build).
3. **Offline/vendored build** — REMAINING (largest task). Debian buildds build
   offline: vendor the Rust crate tree (dh-cargo / debcargo) and provide the
   npm/frontend assets in a policy-allowed way. A sponsor will guide this.
4. **lintian clean** — REMAINING. Drive `lintian` toward no errors; record any
   justified overrides.
5. **ITP** — file the wnpp bug (`docs/debian/ITP.md`).
6. **mentors + sponsor** — upload the source package, post the RFS
   (`docs/debian/RFS.md`), and find a sponsoring Debian Developer.
7. **Archive flow** — sponsor uploads to Debian unstable → migrates to testing
   → syncs into the **next** Ubuntu release.

**Interim install (works today):** existing Ubuntu users install the CI-built
`.deb` from the public GitHub Release with `sudo apt install ./Laralux_*.deb`.
Bare-name `apt install laralux` only works once the package is in a release the
user's machine already tracks (i.e. after step 7, on a new enough Ubuntu).

**Risks/open questions:** the runtime-download design and the Tauri/WebKitGTK +
npm build may draw review friction; a sponsor may request changes or decline.
```

- [ ] **Step 4: Validate the docs**

Run: `for f in docs/debian/ITP.md docs/debian/RFS.md docs/debian/roadmap.md; do test -f "$f" && echo "ok $f"; done`
Expected: prints `ok` for all three.

Run: `grep -l "laralux" docs/debian/ITP.md docs/debian/RFS.md && grep -q "MIT" docs/debian/ITP.md && grep -qi "runtime" docs/debian/ITP.md && echo "content checks pass"`
Expected: prints the ITP path and `content checks pass` (confirms package name, MIT, and the runtime-download disclosure are present).

- [ ] **Step 5: Commit**

```bash
git add docs/debian/
git commit -m "docs(debian): ITP, RFS, and archive roadmap drafts"
```

---

## Self-Review

- **Spec coverage:** §2 metadata/LICENSE/README → Task 1; §3 CI Release → Task 2; §4 debian/ source package → Task 3; §5 ITP/RFS/roadmap → Task 4. §7 risks and §8 verification realized as each task's validation steps + the documented manual smoke (build/install/lintian on the user's machine). The honest-scope boundary (network build, vendoring remaining) appears in the Global Constraints, Task 3 rules comment, and `roadmap.md`.
- **Placeholder scan:** none — every file's full content is given. The `<version>` tokens in README/release body are literal user-facing placeholders shown to end users (the actual version is substituted by the release at install time), not plan gaps. `dpkg-dev`/YAML-parser-absent branches are honest environment fallbacks, not missing content.
- **Type/identifier consistency:** package name `laralux` is identical across `debian/control` (Source + Package), `debian/changelog`, `debian/copyright` (Upstream-Name), `debian/laralux.desktop` (Exec/Icon `laralux`), `debian/rules` (installs binary as `/usr/bin/laralux`), and the ITP/RFS. The built binary is `target/release/laralux-desktop` (the crate's `[[bin]]` name) installed as `laralux`. `bundle.licenseFile = "../LICENSE"` points at Task 1's `LICENSE`. Homepage `https://github.com/thotam/laralux` is identical in tauri.conf.json, control, copyright, README, ITP, RFS. The Release artifact filename is `Laralux_<version>_amd64.deb` (Tauri uses `productName` "Laralux" for the file) — the README and release body reference that exact capitalization.
- **Verification honesty:** packaging tasks have no unit tests; gates are config/file validity + identifier checks, and the real end-to-end (CI run on a pushed tag; `dpkg-buildpackage` + `lintian` + `apt install` on the user's machine) is the user's manual smoke, called out in each task and the spec §8.
