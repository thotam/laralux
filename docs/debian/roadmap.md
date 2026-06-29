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
