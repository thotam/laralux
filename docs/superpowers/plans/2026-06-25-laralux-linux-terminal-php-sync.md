# Terminal PHP/composer Sync (Phase 2, Version slice 1c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the shell `php` and `composer` use Laralux's active PHP version — install the CLI binary alongside php-fpm, expose the active version as `~/laralux/bin/php`, ship a `composer` wrapper that runs under it, and (opt-in) prepend `~/laralux/bin` to the user's shell PATH.

**Architecture:** Extend `php_static` to fetch the `cli` SAPI too; add `php_cli` (active `php` symlink + `ensure_active_php_cli` + composer.phar/wrapper) and `shell_env` (managed PATH block in rc files). A `Config.shell_integration` flag + Tauri toggle drive enable/disable. `set_php_version` re-points the symlink on switch. Also unify laraluxctl's nginx resolution with the orchestrator's resolver.

**Tech Stack:** Rust (laralux-core, zero Tauri deps), Tauri 2, vanilla JS frontend.

## Global Constraints

- `core` keeps **zero Tauri deps**.
- Commit messages MUST NOT contain a `Co-Authored-By` trailer.
- TDD: failing test first, watch it fail, implement, watch it pass, commit.
- Static source = `https://dl.static-php.dev/static-php-cli/bulk/php-<X.Y.Z>-<sapi>-linux-<arch>.tar.gz`, `sapi ∈ {fpm, cli}`; cli tarball contains a single `php`, fpm a single `php-fpm`.
- Active CLI is `~/laralux/bin/php` → symlink to `php<minor>`; binaries mode `0o755`; everything under user-owned `~/laralux` (no privilege).
- Shell PATH block markers `# >>> laralux >>>` / `# <<< laralux <<<`, body `export PATH="$HOME/laralux/bin:$PATH"` (literal `$HOME`). Modify existing `~/.bashrc`/`~/.zshrc`; if neither exists create the one matching `$SHELL` (zsh→`.zshrc` else `.bashrc`).
- composer wrapper `~/laralux/bin/composer` = `#!/bin/sh\nexec "$(dirname "$0")/php" "$(dirname "$0")/composer.phar" "$@"\n`, mode 0755; `composer.phar` from `https://getcomposer.org/composer.phar`.
- `Downloader::fetch(&self, url: &str, dest: &Path) -> Result<(), SetupError>`; `CommandRunner::run(&self, &str, &[String], Option<&Path>) -> Result<(), ScaffoldError>`.
- Run core tests with `cargo test -p laralux-core`; build `cargo build -p laralux-desktop && cargo build -p laraluxctl`. If `cargo`/`node` aren't on PATH use `$HOME/.cargo/bin/cargo` / `$HOME/.nvm/versions/node/v24.16.0/bin/node`.

---

### Task 1: `php_static` — fetch the cli SAPI alongside fpm

**Files:**
- Modify: `core/src/php_static.rs`

**Interfaces:**
- Produces:
  - `latest_patch_url(version, arch, sapi, listing_json) -> Option<String>` (now SAPI-parameterized).
  - `install_php_static(paths, version, downloader, runner)` installs **both** `php-fpm<v>` and `php<v>`.
  - `pub fn install_php_cli(paths, version, downloader, runner) -> Result<(), PhpStaticError>` (cli only).

- [ ] **Step 1: Update the existing tests for the new signature + dual install**

In `core/src/php_static.rs` tests: change the two `latest_patch_url(...)` calls to pass the SAPI, and make the install test expect two tarball fetches + both binaries. Replace the existing `latest_patch_url_picks_highest_patch_for_arch`, `latest_patch_url_none_for_missing_version_or_arch`, the `TarRunner`, and `install_php_static_downloads_extracts_and_places_binary` with:

```rust
    #[test]
    fn latest_patch_url_picks_highest_patch_for_arch_and_sapi() {
        assert_eq!(
            latest_patch_url("8.4", "x86_64", "fpm", SAMPLE).unwrap(),
            format!("{STATIC_PHP_BASE}/php-8.4.22-fpm-linux-x86_64.tar.gz")
        );
        assert_eq!(
            latest_patch_url("8.4", "x86_64", "cli", SAMPLE).unwrap(),
            format!("{STATIC_PHP_BASE}/php-8.4.22-cli-linux-x86_64.tar.gz")
        );
    }

    #[test]
    fn latest_patch_url_none_for_missing_version_or_arch() {
        assert!(latest_patch_url("7.4", "x86_64", "fpm", SAMPLE).is_none());
        assert!(latest_patch_url("8.4", "riscv64", "fpm", SAMPLE).is_none());
    }
```

Update `SAMPLE` to include a cli entry:

```rust
    const SAMPLE: &str = r#"[
      {"name":"license/","is_dir":true},
      {"name":"php-8.3.31-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.9-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.22-fpm-linux-x86_64.tar.gz"},
      {"name":"php-8.4.22-cli-linux-x86_64.tar.gz"},
      {"name":"php-8.4.30-fpm-linux-aarch64.tar.gz"}
    ]"#;
```

Replace the `TarRunner` so it creates the member named in the tar args, and the install test so it asserts both binaries:

```rust
    struct TarRunner {
        calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }
    impl CommandRunner for TarRunner {
        fn run(&self, program: &str, args: &[String], _cwd: Option<&Path>) -> Result<(), ScaffoldError> {
            self.calls.lock().unwrap().push((program.to_string(), args.to_vec()));
            // args: ["-xzf", <tarball>, "-C", <dir>, <member>]
            let dir = &args[3];
            let member = &args[4];
            std::fs::write(Path::new(dir).join(member), b"bin").unwrap();
            Ok(())
        }
    }

    #[test]
    fn install_php_static_installs_fpm_and_cli() {
        let root = std::env::temp_dir().join(format!("lara-spi-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths = LaraluxPaths::new(root.clone());
        paths.ensure_dirs().unwrap();
        let arch = arch_tag().expect("supported test arch");
        let json = format!(
            "[{{\"name\":\"php-8.4.22-fpm-linux-{arch}.tar.gz\"}},{{\"name\":\"php-8.4.22-cli-linux-{arch}.tar.gz\"}}]"
        );
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let dl = StubDownloader { index_json: json, fetched: fetched.clone() };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runner = TarRunner { calls: calls.clone() };

        install_php_static(&paths, "8.4", &dl, &runner).unwrap();

        let f = fetched.lock().unwrap();
        assert!(f[0].ends_with("?format=json"), "index fetched first");
        assert!(f.iter().any(|u| u.ends_with(&format!("php-8.4.22-fpm-linux-{arch}.tar.gz"))));
        assert!(f.iter().any(|u| u.ends_with(&format!("php-8.4.22-cli-linux-{arch}.tar.gz"))));
        assert!(paths.bin().join("php-fpm8.4").is_file(), "fpm placed");
        assert!(paths.bin().join("php8.4").is_file(), "cli placed");
        assert_eq!(calls.lock().unwrap().len(), 2, "tar run for both SAPIs");
        std::fs::remove_dir_all(&root).ok();
    }
```

(Keep `install_php_static_unavailable_version_errors` and `arch_tag_maps_known`. The `StubDownloader` is unchanged: it writes `index_json` for the `?format=json` URL and dummy bytes otherwise — it already records every fetched URL.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core php_static`
Expected: FAIL to compile — `latest_patch_url` arity changed; install test expects `php8.4`.

- [ ] **Step 3: Refactor the implementation**

In `core/src/php_static.rs`, change `latest_patch_url`'s signature and the `suffix`:

```rust
pub fn latest_patch_url(version: &str, arch: &str, sapi: &str, listing_json: &str) -> Option<String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(listing_json).ok()?;
    let prefix = format!("php-{version}.");
    let suffix = format!("-{sapi}-linux-{arch}.tar.gz");
    let mut best: Option<(u32, String)> = None;
    for e in &entries {
        let name = match e.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => continue,
        };
        if let (true, true) = (name.starts_with(&prefix), name.ends_with(&suffix)) {
            let mid = &name[prefix.len()..name.len() - suffix.len()];
            if let Ok(patch) = mid.parse::<u32>() {
                if best.as_ref().map_or(true, |(b, _)| patch > *b) {
                    best = Some((patch, name.to_string()));
                }
            }
        }
    }
    best.map(|(_, name)| format!("{STATIC_PHP_BASE}/{name}"))
}
```

Replace `install_php_static` with the index-once + per-SAPI helper form:

```rust
/// Fetch the `bulk` directory index JSON once.
fn fetch_index(paths: &LaraluxPaths, downloader: &dyn Downloader) -> Result<String, PhpStaticError> {
    std::fs::create_dir_all(paths.tmp())?;
    let index = paths.tmp().join("static-php-index.json");
    downloader
        .fetch(&format!("{STATIC_PHP_BASE}/?format=json"), &index)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    Ok(std::fs::read_to_string(&index)?)
}

/// Download one SAPI tarball, extract its single `member` binary, and install
/// it as `~/laralux/bin/<dest_name>` (mode 0755).
fn download_static_php(
    paths: &LaraluxPaths,
    version: &str,
    arch: &str,
    sapi: &str,
    member: &str,
    dest_name: &str,
    listing_json: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let url = latest_patch_url(version, arch, sapi, listing_json)
        .ok_or_else(|| PhpStaticError::Unavailable(version.to_string()))?;
    let tarball = paths.tmp().join(format!("php-{version}-{sapi}.tar.gz"));
    downloader
        .fetch(&url, &tarball)
        .map_err(|e| PhpStaticError::Download(e.to_string()))?;
    runner
        .run(
            "tar",
            &[
                "-xzf".to_string(),
                tarball.display().to_string(),
                "-C".to_string(),
                paths.tmp().display().to_string(),
                member.to_string(),
            ],
            None,
        )
        .map_err(|e| PhpStaticError::Extract(e.to_string()))?;
    let extracted = paths.tmp().join(member);
    let dest = paths.bin().join(dest_name);
    std::fs::rename(&extracted, &dest).or_else(|_| {
        std::fs::copy(&extracted, &dest).map(|_| ()).and_then(|_| std::fs::remove_file(&extracted))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Install both the php-fpm and php (cli) static binaries for `version`.
pub fn install_php_static(
    paths: &LaraluxPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.bin())?;
    let json = fetch_index(paths, downloader)?;
    download_static_php(paths, version, arch, "fpm", "php-fpm", &format!("php-fpm{version}"), &json, downloader, runner)?;
    download_static_php(paths, version, arch, "cli", "php", &format!("php{version}"), &json, downloader, runner)?;
    Ok(())
}

/// Install only the php (cli) static binary as `~/laralux/bin/php<version>`.
pub fn install_php_cli(
    paths: &LaraluxPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    let arch = arch_tag().ok_or_else(|| PhpStaticError::Arch(std::env::consts::ARCH.to_string()))?;
    std::fs::create_dir_all(paths.bin())?;
    let json = fetch_index(paths, downloader)?;
    download_static_php(paths, version, arch, "cli", "php", &format!("php{version}"), &json, downloader, runner)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core php_static`
Expected: PASS — dual-install + SAPI URL tests green.

- [ ] **Step 5: Commit**

```bash
git add core/src/php_static.rs
git commit -m "feat(core): install the php cli SAPI alongside php-fpm (static)"
```

---

### Task 2: `php_cli` module — active symlink + composer

**Files:**
- Create: `core/src/php_cli.rs`
- Modify: `core/src/lib.rs` (declare + re-export)

**Interfaces:**
- Consumes: `php_static::install_php_cli`, `Downloader`, `CommandRunner`, `LaraluxPaths`, `PhpStaticError`.
- Produces:
  - `set_active_php(paths, version) -> std::io::Result<()>`
  - `ensure_active_php_cli(paths, version, downloader, runner) -> Result<(), PhpStaticError>`
  - `install_composer(paths, downloader) -> std::io::Result<()>`
  - `const COMPOSER_URL: &str`

- [ ] **Step 1: Write the failing tests**

Create `core/src/php_cli.rs` with imports + tests first:

```rust
use crate::paths::LaraluxPaths;
use crate::php_static::{install_php_cli, PhpStaticError};
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::Path;

pub const COMPOSER_URL: &str = "https://getcomposer.org/composer.phar";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::FakeDownloader;

    fn root() -> LaraluxPaths {
        let p = std::env::temp_dir().join(format!("lara-phpcli-{}-{}", std::process::id(), line!()));
        let paths = LaraluxPaths::new(p);
        paths.ensure_dirs().unwrap();
        paths
    }

    #[test]
    fn set_active_php_points_php_to_versioned_binary() {
        let paths = root();
        std::fs::write(paths.bin().join("php8.4"), b"x").unwrap();
        std::fs::write(paths.bin().join("php8.3"), b"x").unwrap();

        set_active_php(&paths, "8.4").unwrap();
        let link = paths.bin().join("php");
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("php8.4"));

        // re-point
        set_active_php(&paths, "8.3").unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("php8.3"));
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn ensure_active_php_cli_symlinks_without_download_when_present() {
        let paths = root();
        std::fs::write(paths.bin().join("php8.4"), b"x").unwrap();
        let dl = FakeDownloader::new(); // would write "fake"; must NOT be called
        let runner = crate::scaffold::FakeCommandRunner::new();
        ensure_active_php_cli(&paths, "8.4", &dl, &runner).unwrap();
        assert_eq!(std::fs::read_link(paths.bin().join("php")).unwrap(), Path::new("php8.4"));
        assert!(dl.requested().lock().unwrap().is_empty(), "no download when cli present");
        std::fs::remove_dir_all(paths.root()).ok();
    }

    #[test]
    fn install_composer_writes_phar_and_wrapper() {
        let paths = root();
        let dl = FakeDownloader::new();
        install_composer(&paths, &dl).unwrap();
        assert!(paths.bin().join("composer.phar").is_file());
        let wrapper = std::fs::read_to_string(paths.bin().join("composer")).unwrap();
        assert!(wrapper.contains("exec"));
        assert!(wrapper.contains("composer.phar"));
        assert!(wrapper.contains("/php\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths.bin().join("composer")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        std::fs::remove_dir_all(paths.root()).ok();
    }
}
```

(`FakeDownloader::requested()` returns the recorded URL list — it already exists in `setup.rs`; if the accessor name differs, use the existing one.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core php_cli`
Expected: FAIL to compile — `set_active_php`/`ensure_active_php_cli`/`install_composer` not defined.

- [ ] **Step 3: Implement the module**

Add above the `#[cfg(test)]` block in `core/src/php_cli.rs`:

```rust
/// Point `~/laralux/bin/php` at `php<version>` (replace any existing symlink/file).
pub fn set_active_php(paths: &LaraluxPaths, version: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.bin())?;
    let link = paths.bin().join("php");
    let _ = std::fs::remove_file(&link); // remove stale symlink/file if present
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(format!("php{version}"), &link)?;
    }
    Ok(())
}

/// Ensure the active version's cli binary exists (download if missing), then
/// point `~/laralux/bin/php` at it.
pub fn ensure_active_php_cli(
    paths: &LaraluxPaths,
    version: &str,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<(), PhpStaticError> {
    if !paths.bin().join(format!("php{version}")).exists() {
        install_php_cli(paths, version, downloader, runner)?;
    }
    set_active_php(paths, version)?;
    Ok(())
}

/// Download composer.phar and write a `composer` wrapper that runs it under the
/// active `~/laralux/bin/php`.
pub fn install_composer(paths: &LaraluxPaths, downloader: &dyn Downloader) -> std::io::Result<()> {
    std::fs::create_dir_all(paths.bin())?;
    let phar = paths.bin().join("composer.phar");
    downloader
        .fetch(COMPOSER_URL, &phar)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let wrapper = paths.bin().join("composer");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\nexec \"$(dirname \"$0\")/php\" \"$(dirname \"$0\")/composer.phar\" \"$@\"\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
```

- [ ] **Step 4: Declare module + re-exports**

In `core/src/lib.rs`, add `pub mod php_cli;` (near `pub mod php_static;`) and:

```rust
pub use php_cli::{ensure_active_php_cli, install_composer, set_active_php};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p laralux-core php_cli`
Expected: PASS — symlink, ensure-without-download, and composer wrapper tests green.

- [ ] **Step 6: Commit**

```bash
git add core/src/php_cli.rs core/src/lib.rs
git commit -m "feat(core): active php symlink + composer wrapper (php_cli)"
```

---

### Task 3: `shell_env` module — managed PATH block

**Files:**
- Create: `core/src/shell_env.rs`
- Modify: `core/src/lib.rs` (declare + re-export)

**Interfaces:**
- Produces: `SHELL_BLOCK`, `apply_shell_block(&str)->String`, `remove_shell_block(&str)->String`, `rc_filename_for_shell(&str)->&'static str`, `enable_shell_path(&Path,&str)->io::Result<()>`, `disable_shell_path(&Path)->io::Result<()>`.

- [ ] **Step 1: Write the failing tests**

Create `core/src/shell_env.rs` with imports + tests first:

```rust
use std::path::Path;

pub const SHELL_BEGIN: &str = "# >>> laralux >>>";
pub const SHELL_END: &str = "# <<< laralux >>>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_filename_matches_shell() {
        assert_eq!(rc_filename_for_shell("/usr/bin/zsh"), ".zshrc");
        assert_eq!(rc_filename_for_shell("/bin/bash"), ".bashrc");
        assert_eq!(rc_filename_for_shell(""), ".bashrc");
    }

    #[test]
    fn apply_is_idempotent_and_remove_restores() {
        let base = "export EDITOR=vim\n";
        let once = apply_shell_block(base);
        assert!(once.contains("export PATH=\"$HOME/laralux/bin:$PATH\""));
        assert!(once.contains(SHELL_BEGIN) && once.contains(SHELL_END));
        assert!(once.contains("export EDITOR=vim"));
        let twice = apply_shell_block(&once);
        assert_eq!(once, twice, "re-apply is idempotent");
        let removed = remove_shell_block(&once);
        assert!(!removed.contains("laralux"));
        assert!(removed.contains("export EDITOR=vim"));
    }

    #[test]
    fn enable_creates_rc_matching_shell_when_none() {
        let home = std::env::temp_dir().join(format!("lara-rc-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&home).unwrap();
        enable_shell_path(&home, "/bin/bash").unwrap();
        let bashrc = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(bashrc.contains("$HOME/laralux/bin"));
        assert!(!home.join(".zshrc").exists(), "no zshrc for a bash user");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn enable_updates_existing_then_disable_removes() {
        let home = std::env::temp_dir().join(format!("lara-rc2-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".bashrc"), "export A=1\n").unwrap();
        enable_shell_path(&home, "/bin/bash").unwrap();
        assert!(std::fs::read_to_string(home.join(".bashrc")).unwrap().contains("laralux"));
        disable_shell_path(&home).unwrap();
        let after = std::fs::read_to_string(home.join(".bashrc")).unwrap();
        assert!(!after.contains("laralux"));
        assert!(after.contains("export A=1"));
        std::fs::remove_dir_all(&home).ok();
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p laralux-core shell_env`
Expected: FAIL to compile — functions not defined.

- [ ] **Step 3: Implement the module**

Add above the `#[cfg(test)]` block in `core/src/shell_env.rs` (note: fix `SHELL_END` to the correct closing marker):

```rust
pub const SHELL_BLOCK: &str =
    "# >>> laralux >>>\nexport PATH=\"$HOME/laralux/bin:$PATH\"\n# <<< laralux <<<\n";

/// `.zshrc` for a zsh login shell, else `.bashrc`.
pub fn rc_filename_for_shell(shell: &str) -> &'static str {
    if shell.trim_end_matches('/').ends_with("zsh") {
        ".zshrc"
    } else {
        ".bashrc"
    }
}

/// Strip the managed block (markers + their contents) from `contents`.
pub fn remove_shell_block(contents: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in contents.lines() {
        let t = line.trim();
        if t == SHELL_BEGIN {
            skipping = true;
            continue;
        }
        if t == SHELL_END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Return `contents` with the managed block appended exactly once (idempotent).
pub fn apply_shell_block(contents: &str) -> String {
    let mut base = remove_shell_block(contents);
    if !base.is_empty() && !base.ends_with('\n') {
        base.push('\n');
    }
    base.push_str(SHELL_BLOCK);
    base
}

/// Add the managed PATH block to existing rc files; if none exist, create the
/// one matching `$SHELL`.
pub fn enable_shell_path(home: &Path, shell: &str) -> std::io::Result<()> {
    let mut wrote_any = false;
    for rc in [".bashrc", ".zshrc"] {
        let p = home.join(rc);
        if p.exists() {
            let cur = std::fs::read_to_string(&p)?;
            let upd = apply_shell_block(&cur);
            if upd != cur {
                std::fs::write(&p, upd)?;
            }
            wrote_any = true;
        }
    }
    if !wrote_any {
        let p = home.join(rc_filename_for_shell(shell));
        std::fs::write(&p, apply_shell_block(""))?;
    }
    Ok(())
}

/// Remove the managed PATH block from any existing rc files.
pub fn disable_shell_path(home: &Path) -> std::io::Result<()> {
    for rc in [".bashrc", ".zshrc"] {
        let p = home.join(rc);
        if p.exists() {
            let cur = std::fs::read_to_string(&p)?;
            let upd = remove_shell_block(&cur);
            if upd != cur {
                std::fs::write(&p, upd)?;
            }
        }
    }
    Ok(())
}
```

Also correct the `SHELL_END` constant at the top of the file to the proper closing marker:

```rust
pub const SHELL_END: &str = "# <<< laralux <<<";
```

- [ ] **Step 4: Declare module + re-exports**

In `core/src/lib.rs`, add `pub mod shell_env;` and:

```rust
pub use shell_env::{disable_shell_path, enable_shell_path};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p laralux-core shell_env`
Expected: PASS — all four tests green.

- [ ] **Step 6: Commit**

```bash
git add core/src/shell_env.rs core/src/lib.rs
git commit -m "feat(core): managed shell PATH block for ~/laralux/bin (shell_env)"
```

---

### Task 4: `Config.shell_integration`

**Files:**
- Modify: `core/src/config.rs`

**Interfaces:**
- Produces: `Config.shell_integration: bool` (default false).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `core/src/config.rs`:

```rust
    #[test]
    fn shell_integration_defaults_false_and_roundtrips() {
        assert!(!Config::default().shell_integration);
        let tmp = std::env::temp_dir().join(format!("lara-cfg-si-{}.toml", std::process::id()));
        let mut c = Config::default();
        c.shell_integration = true;
        c.save(&tmp).unwrap();
        assert!(Config::load(&tmp).unwrap().shell_integration);
        std::fs::remove_file(&tmp).ok();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p laralux-core config`
Expected: FAIL to compile — no field `shell_integration`.

- [ ] **Step 3: Add the field**

In `core/src/config.rs`, add to `struct Config`:

```rust
    #[serde(default)]
    pub shell_integration: bool,
```

And add `shell_integration: false` to the `Default for Config` impl's returned struct.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p laralux-core config`
Expected: PASS — the new test plus existing config tests (existing `save_then_load_roundtrips` still passes since the field defaults).

- [ ] **Step 5: Commit**

```bash
git add core/src/config.rs
git commit -m "feat(core): Config.shell_integration flag"
```

---

### Task 5: IPC + Setup + laraluxctl wiring

**Files:**
- Modify: `src-tauri/src/commands.rs` (set_php_version, two new commands)
- Modify: `src-tauri/src/main.rs` (register commands)
- Modify: `core/src/setup.rs` (set active php symlink after install)
- Modify: `laraluxctl/src/main.rs` (unify nginx resolution)

**Interfaces:**
- Consumes: `ensure_active_php_cli`, `install_composer`, `enable_shell_path`, `disable_shell_path`, `set_active_php`, `bin::resolve_bin`.
- Produces: `terminal_integration_status()`, `set_terminal_integration(app, enabled)`; `set_php_version` re-points the symlink; setup creates the active symlink; laraluxctl resolves nginx via `resolve_bin`.

- [ ] **Step 1: Imports in commands.rs**

In `src-tauri/src/commands.rs`, add to the `use laralux_core::{...}` block: `ensure_active_php_cli`, `install_composer`, `enable_shell_path`, `disable_shell_path`. (`CurlDownloader`, `RealCommandRunner`, `Config` are already imported.)

- [ ] **Step 2: `set_php_version` re-points the active symlink**

In `set_php_version`, inside the `spawn_blocking`, after `orch.replace_php_version(&version)...` and before building the snapshot return, drop the orchestrator lock scope and add the cli sync. Concretely, after the orchestrator block, add:

```rust
        // Point the CLI `php` (and composer) at the new active version; download
        // the cli binary if a pre-static-cli version only had php-fpm.
        let _ = ensure_active_php_cli(&state.paths, &version, &CurlDownloader, &RealCommandRunner);
```

(Best-effort: a download failure must not fail the switch — the fpm swap already succeeded.)

- [ ] **Step 3: Add the two integration commands**

Append to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub fn terminal_integration_status(state: tauri::State<AppState>) -> Result<bool, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.shell_integration)
}

#[tauri::command]
pub async fn set_terminal_integration(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        let state = app.state::<AppState>();
        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let home = std::path::PathBuf::from(home);

        if enabled {
            ensure_active_php_cli(&state.paths, &config.php_version, &CurlDownloader, &RealCommandRunner)
                .map_err(|e| e.to_string())?;
            install_composer(&state.paths, &CurlDownloader).map_err(|e| e.to_string())?;
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            enable_shell_path(&home, &shell).map_err(|e| e.to_string())?;
        } else {
            disable_shell_path(&home).map_err(|e| e.to_string())?;
        }

        config.shell_integration = enabled;
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
        Ok(enabled)
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 4: Register in main.rs**

In `src-tauri/src/main.rs` `generate_handler!`, add after `commands::set_php_version,`:

```rust
            commands::terminal_integration_status,
            commands::set_terminal_integration,
```

- [ ] **Step 5: Setup creates the active symlink**

In `core/src/setup.rs`, in the `run_setup` PHP-static block, after persisting `cfg.php_version = ver` (on the `Some(ver)` arm), add the symlink so `~/laralux/bin/php` exists for the default version:

```rust
                    let _ = crate::php_cli::set_active_php(paths, &ver);
```

(Place it right after the `cfg.save(...)` line within that arm; best-effort.)

- [ ] **Step 6: Unify laraluxctl nginx resolution**

In `laraluxctl/src/main.rs`, in the `setup-perms` arm, replace:

```rust
            let nginx_bin = which("nginx").unwrap_or_else(|| "/usr/sbin/nginx".into());
```

with:

```rust
            let nginx_bin = laralux_core::bin::resolve_bin("nginx", &[paths.bin()])
                .unwrap_or_else(|| std::path::PathBuf::from("/usr/sbin/nginx"));
```

Then delete the now-unused private `fn which(bin: &str) -> Option<std::path::PathBuf>` (near the bottom of the file). If `which` has no other callers (it doesn't), removing it clears the dead-code warning.

- [ ] **Step 7: Build everything**

Run: `cargo test -p laralux-core` then `cargo build -p laralux-desktop && cargo build -p laraluxctl`
Expected: PASS — all core tests; both binaries compile (no unused imports / dead `which`).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/main.rs core/src/setup.rs laraluxctl/src/main.rs
git commit -m "feat(desktop): terminal integration commands; symlink on switch/setup; unify laraluxctl nginx resolve"
```

---

### Task 6: Frontend — Settings toggle

**Files:**
- Modify: `dist/app.js`

**Interfaces:**
- Consumes: `terminal_integration_status()`, `set_terminal_integration({enabled})`.
- Produces: UI only.

- [ ] **Step 1: Add state**

In `dist/app.js` `state`, add after `phpBusy: false,`:

```js
    terminalIntegration: false,
    termBusy: false,
```

- [ ] **Step 2: Add load + toggle helpers**

Near `loadPhpVersions`, add:

```js
  async function loadTerminalIntegration() {
    try {
      const on = await invoke("terminal_integration_status");
      state.terminalIntegration = !!on;
      render();
    } catch (_) { /* settings-only; stay quiet */ }
  }

  async function toggleTerminalIntegration() {
    if (state.termBusy) return;
    const next = !state.terminalIntegration;
    state.termBusy = true; render();
    try {
      const on = await invoke("set_terminal_integration", { enabled: next });
      state.terminalIntegration = !!on;
      toast({
        type: "success",
        title: on ? "Terminal integration on" : "Terminal integration off",
        msg: on ? "Open a new terminal — php & composer now use Laralux's active version" : "Removed ~/laralux/bin from your shell PATH",
      });
    } catch (e) {
      toast({ type: "error", title: "Couldn't change terminal integration", msg: String(e) });
    } finally {
      state.termBusy = false; render();
    }
  }
```

- [ ] **Step 3: Load on entering Settings**

In `setView`, where it already does `if (v === "settings") loadPhpVersions();`, also load the toggle:

```js
    if (v === "settings") { loadPhpVersions(); loadTerminalIntegration(); }
```

- [ ] **Step 4: Render the toggle row in `settingsView`**

In `settingsView`, add a row to the settings card (e.g. right after the "Sites directory" row, before the closing `"</div>"` of that card):

```js
      '<div class="set-row"><div class="grow"><div class="t">Terminal integration</div>' +
      '<div class="h">Use Laralux’s active PHP + composer in your shell (php, composer)</div></div>' +
      '<button class="btn-sm" data-action="toggle-terminal"' + (state.termBusy ? " disabled" : "") + '>' +
      (state.terminalIntegration ? "On" : "Off") + "</button></div>" +
```

- [ ] **Step 5: Wire the click handler**

In the delegated click handler, add (next to the other settings actions like `toggle-dark`):

```js
    else if (a === "toggle-terminal") toggleTerminalIntegration();
```

- [ ] **Step 6: Syntax-check the JS**

Run: `node --check dist/app.js`
Expected: PASS — exit 0, no output.

- [ ] **Step 7: Manual verification (live)**

Run: `cargo run -p laralux-desktop`. In **Settings**, the **Terminal integration** row shows **Off**. Click it → it downloads composer.phar + ensures the active cli + writes the rc block, shows **On** and a toast. Open a **new terminal**: `php -v` and `composer --version` now report Laralux's active version (e.g. 8.4). Switch the active version in the PHP card, open a new terminal → `php`/`composer` follow. Toggle **Off** → the rc block is removed (binaries stay). (Network/rc edits can't run in unit tests — human-verified.)

- [ ] **Step 8: Commit**

```bash
git add dist/app.js
git commit -m "feat(desktop): Settings toggle for terminal PHP/composer integration"
```

---

## Self-Review

**1. Spec coverage:**
- §3.1 install cli alongside fpm (`latest_patch_url` sapi, dual install, `install_php_cli`) → Task 1. ✓
- §3.2 `set_active_php` / `ensure_active_php_cli` / `install_composer` + COMPOSER_URL → Task 2. ✓
- §3.3 `shell_env` (SHELL_BLOCK literal `$HOME`, apply/remove, rc_filename_for_shell, enable/disable, create-matching-`$SHELL`) → Task 3. ✓
- §3.4 `Config.shell_integration` → Task 4. ✓
- §3.5 IPC status/set + set_php_version symlink + setup symlink + main register → Task 5. ✓
- §3.6 frontend toggle + load on settings → Task 6. ✓
- §3.7 laraluxctl nginx resolver unify → Task 5 Step 6. ✓
- §6 testing: php_static dual install + sapi URL (Task 1); symlink/ensure/composer (Task 2); shell_env apply/remove/rc/create (Task 3); config roundtrip (Task 4); live (Task 6). ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows full code. The one manual step (Task 6 Step 7) is an explicit human-verified gate (network downloads, rc edits, live `php`/`composer` cannot run in unit tests).

**3. Type consistency:**
- `latest_patch_url(version, arch, sapi, json)` (Task 1) — all call sites (download_static_php, tests) pass 4 args. ✓
- `install_php_static`/`install_php_cli`/`ensure_active_php_cli`/`install_composer`/`set_active_php` signatures (Tasks 1–2) match the IPC + setup callers (Task 5). ✓
- `Downloader::fetch -> Result<(), SetupError>` mapped to `PhpStaticError::Download`/`io::Error` consistently. ✓
- `enable_shell_path(&Path, &str)` / `disable_shell_path(&Path)` (Task 3) called with `home` + `$SHELL` in Task 5. ✓
- `Config.shell_integration` (Task 4) read/written by Task 5; `terminal_integration_status`/`set_terminal_integration` IPC names match the JS invokes (Task 6). ✓
- Sequencing keeps each task compiling: core modules added (1–4) → desktop/setup/laraluxctl wired (5) → frontend (6). ✓

**Note:** `set_active_php` removes any existing `~/laralux/bin/php` before re-symlinking (handles switching). `ensure_active_php_cli` only downloads when `php<version>` is absent, so switching among already-installed versions is instant. The composer wrapper resolves `php` as a sibling of `$0`, so it always runs under the active symlink regardless of PATH ordering.
