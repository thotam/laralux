use laragon_core::service::php_fpm::PhpFpmService;
use laragon_core::{
    build_services, detect_components, scan_sites, sync_sites, Config, CurlDownloader, LaragonPaths,
    MkcertIssuer, Orchestrator, Privileged, RealCommandRunner, RealSpawner, run_setup, SudoPrivileged,
};

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    let paths = LaragonPaths::new(LaragonPaths::default_root());

    match cmd.as_str() {
        "config-init" => {
            paths.ensure_dirs().expect("create dirs");
            let cfg = Config::default();
            cfg.save(&paths.config_file()).expect("save config");
            println!("Initialized {}", paths.config_file().display());
        }
        "sites" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            let sites = scan_sites(&paths, &cfg.tld).expect("scan sites");
            if sites.is_empty() {
                println!("No sites found in {}", paths.www().display());
            }
            for s in sites {
                println!("{:<20} https://{}", s.name, s.hostname);
            }
        }
        "setup-perms" => {
            let priv_ = SudoPrivileged;
            println!("Installing mkcert local CA (may prompt for sudo)...");
            priv_.install_mkcert_ca().expect("mkcert -install");
            let nginx_bin = which("nginx").unwrap_or_else(|| "/usr/sbin/nginx".into());
            println!("Granting nginx permission to bind low ports via setcap...");
            priv_.setcap_nginx(&nginx_bin).expect("setcap nginx");
            println!("Done.");
        }
        "up" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            paths.ensure_dirs().expect("create dirs");

            // Sync sites (vhosts + certs + /etc/hosts) before starting nginx.
            let php_socket = PhpFpmService::new(cfg.php_version.clone()).socket_path(&paths);
            let issuer = MkcertIssuer::new(paths.ssl());
            let privileged = SudoPrivileged;
            match sync_sites(
                &paths,
                &cfg.tld,
                &php_socket,
                std::path::Path::new("/etc/hosts"),
                &issuer,
                &privileged,
            ) {
                Ok(sites) => println!("Synced {} site(s).", sites.0.len()),
                Err(e) => {
                    eprintln!("site sync failed: {e}");
                    std::process::exit(1);
                }
            }

            let mut orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            match orch.start_all() {
                Ok(()) => println!("Started all services. Press Ctrl-C to stop."),
                Err(e) => {
                    eprintln!("start failed: {e}");
                    orch.stop_all();
                    std::process::exit(1);
                }
            }
            wait_for_ctrl_c();
            println!("Stopping...");
            orch.stop_all();
        }
        "status" => {
            let cfg = Config::load(&paths.config_file()).expect("load config");
            let orch =
                Orchestrator::new(paths.clone(), build_services(&cfg, &paths), Box::new(RealSpawner));
            for kind in orch.start_order() {
                println!("{:?}: {:?}", kind, orch.state(kind));
            }
        }
        "down" => {
            println!("`up` manages the process lifetime; stop it with Ctrl-C.");
        }
        "setup" => {
            paths.ensure_dirs().expect("create dirs");
            println!("Component status:");
            for s in detect_components(&paths) {
                println!("  {:?}: {}", s.component, if s.present { "installed" } else { "missing" });
            }
            println!("Running setup (may prompt for sudo)...");
            let report = run_setup(&paths, &SudoPrivileged, &CurlDownloader, &RealCommandRunner);
            println!(
                "apt: {}\nmailpit fetched: {}\nmkcert CA: {}\nnginx setcap: {}",
                if report.apt_packages.is_empty() { "none".to_string() } else { report.apt_packages.join(" ") },
                report.mailpit_fetched, report.mkcert_ca, report.nginx_setcap
            );
            if let Some(ver) = &report.php_version {
                println!("PHP {ver} installed — restart laragon to use it.");
            }
            for e in &report.errors {
                eprintln!("  error: {e}");
            }
        }
        _ => {
            println!("usage: laragonctl <config-init|up|status|sites|setup-perms|setup>");
        }
    }
}

/// Resolve a binary on PATH (minimal `which`, no external crate).
fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn wait_for_ctrl_c() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SHUTDOWN: AtomicBool = AtomicBool::new(false);

    // Signal handler: only an atomic store — async-signal-safe, cannot deadlock.
    extern "C" fn on_sigint(_sig: i32) {
        SHUTDOWN.store(true, Ordering::SeqCst);
    }
    extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    unsafe {
        signal(2, on_sigint); // SIGINT = 2
    }

    while !SHUTDOWN.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
