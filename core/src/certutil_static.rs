//! Download `certutil` (NSS tools) as a self-contained binary bundle so mkcert
//! can register its CA in the Firefox/Chrome NSS trust stores — without
//! `apt install libnss3-tools`. There is no clean single-binary certutil for
//! Linux (Mozilla ships only source; certutil dynamically links the whole
//! NSS/NSPR set), so we extract four Ubuntu `.deb`s and bundle `certutil` plus
//! every shared object next to it, then run it with `LD_LIBRARY_PATH`.
//!
//! Debian/Ubuntu only (the target platform): extraction uses `dpkg-deb`.

use crate::paths::LaraluxPaths;
use crate::progress::ProgressSink;
use crate::scaffold::CommandRunner;
use crate::setup::Downloader;
use std::path::{Path, PathBuf};

/// NSS upstream version of the pinned bundle (also the install dir name).
pub const CERTUTIL_VERSION: &str = "3.98";

const POOL_BASE: &str = "http://archive.ubuntu.com/ubuntu/pool/main";

/// Pinned, mutually-coherent Ubuntu 24.04 LTS "noble" debs (glibc 2.39, NSS 3.98).
/// `libnss3-tools` provides `certutil`; the rest are its shared-library closure.
/// `zlib1g`/`libc6`/`libstdc++` are Priority:required and resolved from the system.
const DEBS: &[&str] = &[
    "n/nss/libnss3_3.98-1build1_amd64.deb",
    "n/nss/libnss3-tools_3.98-1build1_amd64.deb",
    "n/nspr/libnspr4_4.35-1.1build1_amd64.deb",
    "s/sqlite3/libsqlite3-0_3.45.1-1ubuntu2_amd64.deb",
];

#[derive(Debug, thiserror::Error)]
pub enum CertutilError {
    #[error("unsupported architecture: {0}")]
    Arch(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("extract failed: {0}")]
    Extract(String),
    #[error("layout error: {0}")]
    Layout(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Debian arch tag for the host, or `None` if unsupported.
pub fn certutil_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        _ => None,
    }
}

/// Absolute URL of a pool-relative `.deb` path.
pub fn deb_url(rel: &str) -> String {
    format!("{POOL_BASE}/{rel}")
}

pub fn certutil_dir(paths: &LaraluxPaths) -> PathBuf {
    paths.version_dir("certutil", CERTUTIL_VERSION)
}
pub fn certutil_bin(paths: &LaraluxPaths) -> PathBuf {
    certutil_dir(paths).join("bin").join("certutil")
}
pub fn certutil_lib_dir(paths: &LaraluxPaths) -> PathBuf {
    certutil_dir(paths).join("lib")
}

fn installed(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// Download + extract the certutil bundle into `bin/certutil/<ver>/{bin,lib}`.
/// Idempotent (skips when the certutil binary already exists). Returns its path.
pub fn install_certutil(
    paths: &LaraluxPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    sink: &dyn ProgressSink,
) -> Result<PathBuf, CertutilError> {
    let bin = certutil_bin(paths);
    if installed(&bin) {
        return Ok(bin);
    }
    // Arch gate (the pinned debs are amd64; arm64 maps the same but is unverified).
    let _arch = certutil_arch().ok_or_else(|| CertutilError::Arch(std::env::consts::ARCH.to_string()))?;

    std::fs::create_dir_all(paths.tmp())?;
    let stage = paths.tmp().join("certutil-stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;

    for (i, rel) in DEBS.iter().enumerate() {
        let name = rel.rsplit('/').next().unwrap_or("pkg.deb");
        let deb = paths.tmp().join(name);
        let url = deb_url(rel);
        // The first (libnss3) deb is the largest — show byte progress for it.
        let res = if i == 0 {
            downloader.fetch_with_progress(&url, &deb, sink)
        } else {
            downloader.fetch(&url, &deb)
        };
        res.map_err(|e| CertutilError::Download(format!("{name}: {e}")))?;
        runner
            .run("dpkg-deb", &["-x".into(), deb.display().to_string(), stage.display().to_string()], None)
            .map_err(|e| CertutilError::Extract(format!("{name}: {e}")))?;
    }

    let dir = certutil_dir(paths);
    let bindir = dir.join("bin");
    let libdir = dir.join("lib");
    std::fs::create_dir_all(&bindir)?;
    std::fs::create_dir_all(&libdir)?;

    // certutil binary.
    let src_certutil = stage.join("usr").join("bin").join("certutil");
    if !src_certutil.is_file() {
        return Err(CertutilError::Layout("certutil not found in extracted debs".into()));
    }
    std::fs::copy(&src_certutil, &bin)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
    }

    // Every shared object from the extracted lib dir (NSS + NSPR + sqlite).
    let src_lib = stage.join("usr").join("lib").join("x86_64-linux-gnu");
    let mut copied = 0usize;
    if let Ok(rd) = std::fs::read_dir(&src_lib) {
        for e in rd.flatten() {
            let p = e.path();
            let is_so = p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(".so"))
                .unwrap_or(false);
            if p.is_file() && is_so {
                if let Some(name) = p.file_name() {
                    std::fs::copy(&p, libdir.join(name))?;
                    copied += 1;
                }
            }
        }
    }
    if copied == 0 {
        return Err(CertutilError::Layout("no shared libraries found in extracted debs".into()));
    }
    let _ = std::fs::remove_dir_all(&stage);
    Ok(bin)
}

/// Run `mkcert -install` for the browser NSS stores only, using the bundled
/// certutil (`PATH` prepend + `LD_LIBRARY_PATH`) and `TRUST_STORES=nss` so it
/// touches only Firefox/Chrome — the system store is handled separately.
pub fn mkcert_install_nss(
    mkcert_bin: &Path,
    certutil_bin_dir: &Path,
    certutil_lib_dir: &Path,
) -> Result<(), CertutilError> {
    let prev_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", certutil_bin_dir.display(), prev_path);
    let status = std::process::Command::new(mkcert_bin)
        .arg("-install")
        .env("PATH", new_path)
        .env("LD_LIBRARY_PATH", certutil_lib_dir.display().to_string())
        .env("TRUST_STORES", "nss")
        .status()
        .map_err(|e| CertutilError::Extract(format!("spawn mkcert: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CertutilError::Extract("mkcert -install (nss) failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_maps_x86_64_to_amd64() {
        assert_eq!(
            certutil_arch(),
            match std::env::consts::ARCH {
                "x86_64" => Some("amd64"),
                _ => None,
            }
        );
    }

    #[test]
    fn deb_url_is_pool_main_absolute() {
        assert_eq!(
            deb_url("n/nss/libnss3_3.98-1build1_amd64.deb"),
            "http://archive.ubuntu.com/ubuntu/pool/main/n/nss/libnss3_3.98-1build1_amd64.deb"
        );
    }

    #[test]
    fn bundle_paths_are_under_versioned_layout() {
        let p = LaraluxPaths::new("/tmp/lara".into());
        assert_eq!(certutil_bin(&p), Path::new("/tmp/lara/bin/certutil/3.98/bin/certutil"));
        assert_eq!(certutil_lib_dir(&p), Path::new("/tmp/lara/bin/certutil/3.98/lib"));
    }

    #[test]
    fn pinned_debs_include_tools_and_libs() {
        assert!(DEBS.iter().any(|d| d.contains("libnss3-tools")));
        assert!(DEBS.iter().any(|d| d.contains("libnspr4")));
        assert_eq!(DEBS.len(), 4);
    }
}
