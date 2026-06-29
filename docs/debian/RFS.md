# RFS: laralux

Request For Sponsorship for mentors.debian.net (post after uploading the source
package and filing the ITP).

---

Dear mentors,

I am looking for a sponsor for my package **laralux**.

* Package name : laralux
* Version      : 0.3.0-1
* Upstream URL : https://github.com/thotam/laralux
* License      : MIT
* Section      : web

It builds a single binary package: `laralux` — a native Linux local
web-development environment manager built with Tauri 2 (Rust + a
TypeScript/WebKitGTK frontend).

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
