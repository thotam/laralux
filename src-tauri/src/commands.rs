use laragon_core::{
    build_services, create_site as core_create_site, detect_components, ensure_coredns,
    ensure_nginx_bind_cap, list_all_sites, resolved_dropin, run_setup, sync_sites, Config,
    CreateReport, LaragonPaths, MkcertIssuer, Orchestrator, PkexecPrivileged, Privileged,
    ProxyRoute, RealCommandRunner, RealSpawner, ServiceKind, ServiceState, ServiceStatus, Site,
    SiteRegistry, SiteTemplate, ensure_active_php_cli, install_composer, enable_shell_path,
    disable_shell_path,
};
use laragon_core::{ComponentStatus, CurlDownloader, SetupReport};
use laragon_core::service::php_fpm::PhpFpmService;
use laragon_core::{
    install_php_static, list_php_fpm_versions, php_versions as core_php_versions,
    PhpVersionInfo,
};
use std::sync::Mutex;
use tauri::Manager;

/// Shared, app-lifetime state. The orchestrator owns the running child
/// processes, so it must live as long as the app and be stopped on exit.
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub paths: LaragonPaths,
    pub tld: String,
}

/// Build the managed state from the on-disk config.
pub fn build_state() -> AppState {
    let paths = LaragonPaths::new(LaragonPaths::default_root());
    let config = Config::load(&paths.config_file()).unwrap_or_default();
    let _ = paths.ensure_dirs();
    let orch = Orchestrator::new(paths.clone(), build_services(&config, &paths), Box::new(RealSpawner));
    AppState { orch: Mutex::new(orch), paths, tld: config.tld }
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> String {
    "internal lock poisoned".to_string()
}

#[tauri::command]
pub fn stack_status(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.refresh();
    Ok(orch.snapshot())
}

#[tauri::command]
pub async fn stack_start_all(app: tauri::AppHandle) -> Result<Vec<ServiceStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ServiceStatus>, String> {
        let state = app.state::<AppState>();
        // Sync sites (per-site vhosts + mkcert certs + /etc/hosts) BEFORE starting,
        // so nginx loads the vhosts on start and <name>.<tld> resolves. Best-effort:
        // a sync failure (e.g. the user cancels the pkexec prompt) must not block start.
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::new(state.paths.ssl());
        let privileged = PkexecPrivileged;
        let bases = sync_sites(
            &state.paths,
            &config.tld,
            &php_socket,
            std::path::Path::new("/etc/hosts"),
            &issuer,
            &privileged,
        ).map(|o| o.wildcard_bases).unwrap_or_default();
        let _ = apply_wildcard_dns(&state, &bases);

        // Ensure nginx can bind :80/:443 (re-setcap if a binary upgrade cleared it).
        ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged);

        let mut orch = state.orch.lock().map_err(lock_err)?;
        orch.start_all().map_err(|e| e.to_string())?;
        Ok(orch.snapshot())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn stack_stop_all(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.stop_all();
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn service_start(
    state: tauri::State<AppState>,
    kind: ServiceKind,
) -> Result<Vec<ServiceStatus>, String> {
    if kind == ServiceKind::Nginx {
        ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged);
    }
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.start(kind).map_err(|e| e.to_string())?;
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn service_stop(
    state: tauri::State<AppState>,
    kind: ServiceKind,
) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.stop(kind).map_err(|e| e.to_string())?;
    Ok(orch.snapshot())
}

#[tauri::command]
pub fn list_sites(state: tauri::State<AppState>) -> Result<Vec<Site>, String> {
    let (sites, _warnings) = list_all_sites(&state.paths, &state.tld).map_err(|e| e.to_string())?;
    Ok(sites)
}

#[tauri::command]
pub fn setup_status(state: tauri::State<AppState>) -> Result<Vec<ComponentStatus>, String> {
    Ok(detect_components(&state.paths))
}

#[tauri::command]
pub async fn run_setup_cmd(app: tauri::AppHandle) -> Result<SetupReport, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<SetupReport, String> {
        let state = app.state::<AppState>();
        let privileged = PkexecPrivileged;
        let downloader = CurlDownloader;
        Ok(run_setup(&state.paths, &privileged, &downloader, &RealCommandRunner))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn create_site(
    app: tauri::AppHandle,
    name: String,
    template: SiteTemplate,
) -> Result<CreateReport, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<CreateReport, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        // Read whether MariaDB is currently running (brief lock).
        let mariadb_running = {
            let orch = state.orch.lock().map_err(|_| "internal lock poisoned".to_string())?;
            orch.state(ServiceKind::Mariadb) == ServiceState::Running
        };

        // Scaffold (slow; no orchestrator lock held).
        let report = core_create_site(
            &state.paths,
            &name,
            &config.tld,
            template,
            mariadb_running,
            &RealCommandRunner,
            &CurlDownloader,
        )
        .map_err(|e| e.to_string())?;

        // Make it reachable: sync vhost+cert+/etc/hosts, then reload nginx if running.
        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::new(state.paths.ssl());
        let privileged = PkexecPrivileged;
        let _ = sync_sites(
            &state.paths,
            &config.tld,
            &php_socket,
            std::path::Path::new("/etc/hosts"),
            &issuer,
            &privileged,
        );
        {
            let mut orch = state.orch.lock().map_err(|_| "internal lock poisoned".to_string())?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }

        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn link_site(
    app: tauri::AppHandle,
    name: String,
    root: String,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        // Register the folder (validates name, existence, duplicates).
        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry
            .add(&name, std::path::Path::new(&root))
            .map_err(|e| e.to_string())?;
        registry
            .save(&state.paths.sites_file())
            .map_err(|e| e.to_string())?;

        // Make it reachable: sync vhost+cert+/etc/hosts, then reload nginx if running.
        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::new(state.paths.ssl());
        let privileged = PkexecPrivileged;
        let _ = sync_sites(
            &state.paths,
            &config.tld,
            &php_socket,
            std::path::Path::new("/etc/hosts"),
            &issuer,
            &privileged,
        );
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }

        // Return the freshly linked site from the merged list.
        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("linked site `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn unlink_site(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        let removed = registry.remove(&name);
        registry
            .save(&state.paths.sites_file())
            .map_err(|e| e.to_string())?;
        if !removed {
            return Err(format!("site `{name}` is not a linked site"));
        }

        // Remove the now-orphaned vhost so nginx stops serving it.
        let vhost = state
            .paths
            .etc_for("nginx")
            .join("sites")
            .join(format!("{name}.conf"));
        let _ = std::fs::remove_file(&vhost);

        // Re-sync (rewrites /etc/hosts without this host) and reload nginx.
        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::new(state.paths.ssl());
        let privileged = PkexecPrivileged;
        let _ = sync_sites(
            &state.paths,
            &config.tld,
            &php_socket,
            std::path::Path::new("/etc/hosts"),
            &issuer,
            &privileged,
        );
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Re-sync vhosts/certs/hosts and reload nginx if it is running. Best-effort,
/// matching `link_site`/`create_site` (a sync failure must not fail the call).
fn sync_and_reload(state: &AppState, config: &Config) {
    let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
    let issuer = MkcertIssuer::new(state.paths.ssl());
    let privileged = PkexecPrivileged;
    let _ = sync_sites(
        &state.paths,
        &config.tld,
        &php_socket,
        std::path::Path::new("/etc/hosts"),
        &issuer,
        &privileged,
    );
    if let Ok(mut orch) = state.orch.lock() {
        if orch.state(ServiceKind::Nginx) == ServiceState::Running {
            let _ = orch.stop(ServiceKind::Nginx);
            let _ = orch.start(ServiceKind::Nginx);
        }
    }
}

#[tauri::command]
pub async fn add_proxy(
    app: tauri::AppHandle,
    name: String,
    routes: Vec<ProxyRoute>,
    websocket: bool,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.add_proxy(&name, &routes, websocket).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        sync_and_reload(&state, &config);

        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("proxy `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn update_proxy(
    app: tauri::AppHandle,
    name: String,
    routes: Vec<ProxyRoute>,
    websocket: bool,
) -> Result<Site, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Site, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry =
            SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.update_proxy(&name, &routes, websocket).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        sync_and_reload(&state, &config);

        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        sites
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("proxy `{name}` not found after sync"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn php_versions(state: tauri::State<AppState>) -> Result<Vec<PhpVersionInfo>, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(core_php_versions(&state.paths, &config.php_version))
}

#[tauri::command]
pub async fn install_php_version(
    app: tauri::AppHandle,
    version: String,
) -> Result<Vec<PhpVersionInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<PhpVersionInfo>, String> {
        let state = app.state::<AppState>();
        install_php_static(&state.paths, &version, &CurlDownloader, &RealCommandRunner)
            .map_err(|e| e.to_string())?;
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        Ok(core_php_versions(&state.paths, &config.php_version))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_php_version(
    app: tauri::AppHandle,
    version: String,
) -> Result<Vec<ServiceStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ServiceStatus>, String> {
        let state = app.state::<AppState>();
        if !list_php_fpm_versions(&[state.paths.bin()]).contains(&version) {
            return Err(format!("PHP {version} is not installed"));
        }
        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        config.php_version = version.clone();
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;

        let mut orch = state.orch.lock().map_err(lock_err)?;
        orch.replace_php_version(&version).map_err(|e| e.to_string())?;
        let snapshot = orch.snapshot();
        drop(orch);

        // Point the CLI `php` (and composer) at the new active version; download
        // the cli binary if a pre-static-cli version only had php-fpm.
        let _ = ensure_active_php_cli(&state.paths, &version, &CurlDownloader, &RealCommandRunner);

        Ok(snapshot)
    })
    .await
    .map_err(|e| e.to_string())?
}

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

#[tauri::command]
pub fn open_terminal(path: String) -> Result<(), String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    laragon_core::open_terminal(&dir).map_err(|e| e.to_string())
}

/// Best-effort: kill any CoreDNS spawned from our managed bin (e.g. an orphan
/// left by a crashed prior session that still holds 127.0.0.1:5353). Matching on
/// our bin path avoids touching an unrelated system CoreDNS.
fn kill_stale_coredns(state: &AppState) {
    let pat = state.paths.bin().join("coredns");
    let _ = std::process::Command::new("pkill")
        .arg("-f")
        .arg(pat.display().to_string())
        .status();
}

/// Apply DNS state for the current wildcard bases. Returns non-fatal warnings
/// (best-effort: failures never break the explicit-domain path, but are reported
/// so the UI can tell the user wildcard DNS didn't fully apply).
fn apply_wildcard_dns(state: &AppState, bases: &[String]) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    if bases.is_empty() {
        if let Ok(mut orch) = state.orch.lock() {
            let _ = orch.set_coredns(vec![]);
        }
        kill_stale_coredns(state); // ensure no orphan keeps :5353
        if let Err(e) = PkexecPrivileged.remove_resolved_dropin() {
            warnings.push(format!("Could not remove DNS routing drop-in: {e}"));
        }
        return warnings;
    }
    if let Err(e) = ensure_coredns(&state.paths, &CurlDownloader, &RealCommandRunner) {
        warnings.push(format!("Wildcard DNS unavailable (CoreDNS download failed): {e}"));
        return warnings;
    }
    kill_stale_coredns(state); // clear any orphan before (re)starting
    if let Ok(mut orch) = state.orch.lock() {
        if let Err(e) = orch.set_coredns(bases.to_vec()) {
            warnings.push(format!("Could not start CoreDNS: {e}"));
        }
    }
    if let Err(e) = PkexecPrivileged.write_resolved_dropin(&resolved_dropin(bases, 5353)) {
        warnings.push(format!("Could not write DNS routing drop-in: {e}"));
    }
    warnings
}

#[derive(serde::Serialize)]
pub struct SetDomainsResult {
    pub sites: Vec<Site>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn set_site_domains(
    app: tauri::AppHandle,
    name: String,
    domains: Vec<String>,
) -> Result<SetDomainsResult, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<SetDomainsResult, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        let mut registry = SiteRegistry::load(&state.paths.sites_file()).map_err(|e| e.to_string())?;
        registry.set_domains(&name, &domains).map_err(|e| e.to_string())?;
        registry.save(&state.paths.sites_file()).map_err(|e| e.to_string())?;

        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::new(state.paths.ssl());
        let privileged = PkexecPrivileged;
        let outcome = sync_sites(
            &state.paths, &config.tld, &php_socket,
            std::path::Path::new("/etc/hosts"), &issuer, &privileged,
        );
        let bases = outcome.as_ref().map(|o| o.wildcard_bases.clone()).unwrap_or_default();
        let warnings = apply_wildcard_dns(&state, &bases);
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if orch.state(ServiceKind::Nginx) == ServiceState::Running {
                let _ = orch.stop(ServiceKind::Nginx);
                let _ = orch.start(ServiceKind::Nginx);
            }
        }
        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        Ok(SetDomainsResult { sites, warnings })
    })
    .await
    .map_err(|e| e.to_string())?
}
