# ITP: laralux — native Linux local web-development environment manager

File this as a **wnpp** Intent To Package bug before uploading to mentors.

**How to file:** install `reportbug`, then `reportbug wnpp` → choose **ITP**.
Or email `submit@bugs.debian.org` with the body below (pseudo-package `wnpp`,
severity `wishlist`), subject `ITP: laralux -- native Linux local web-development environment manager`.

---

Package: wnpp
Severity: wishlist

* Package name    : laralux
  Version         : 0.2.0
  Upstream Author : thotam <thanhtamtotaa@gmail.com>
* URL             : https://github.com/thotam/laralux
* License         : MIT
  Programming Lang: Rust, TypeScript
  Description      : native Linux local web-development environment manager

Laralux is a native Linux local web-development environment manager: a
system-tray and GUI manager for a local web-development stack (nginx, PHP-FPM,
MariaDB, PostgreSQL, MongoDB, Redis, Mailpit) with pretty *.dev HTTPS, automatic
mkcert SSL, and multi-version tool switching.

Disclosure: the managed tool binaries (nginx, mariadb, php, …) are downloaded as
upstream static builds into ~/laralux at the user's request at runtime; they are
not shipped in the package. Packaging review should account for this design.

I will maintain this package. Sponsorship by a Debian Developer is requested
(RFS to follow on mentors.debian.net).
