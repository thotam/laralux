use laralux_core::{
    build_services, create_site as core_create_site, detect_components, ensure_coredns,
    ensure_nginx_bind_cap, list_all_sites, read_procfile, resolved_dropin, run_setup, sync_sites,
    Config, CreateReport, LaraluxPaths, MkcertIssuer, Orchestrator, PkexecPrivileged, Privileged,
    ProxyRoute, ProcStatus, RealCommandRunner, RealSpawner, ServiceKind, ServiceState,
    ServiceStatus, Site, SiteProcs, SiteRegistry, SiteTemplate,
};
use laralux_core::{ComponentStatus, CurlDownloader, SetupReport};
use laralux_core::service::php_fpm::PhpFpmService;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use tauri::Manager;

struct TauriProgress(tauri::AppHandle);
impl laralux_core::ProgressSink for TauriProgress {
    fn emit(&self, ev: laralux_core::ProgressEvent) {
        let _ = self.0.emit("download-progress", ev);
    }
}

/// Shared, app-lifetime state. The orchestrator owns the running child
/// processes, so it must live as long as the app and be stopped on exit.
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub site_procs: Mutex<SiteProcs>,
    pub paths: LaraluxPaths,
    pub tld: String,
    pub starting: AtomicBool,
}

/// Build the managed state from the on-disk config.
pub fn build_state() -> AppState {
    let paths = LaraluxPaths::new(LaraluxPaths::default_root());
    let config = Config::load(&paths.config_file()).unwrap_or_default();
    let _ = paths.ensure_dirs();
    // Reconcile bin/<tool>/current symlinks from config (the source of truth)
    // at startup, so the active versions match config regardless of prior installs.
    let _ = laralux_core::apply_versions(&paths, &config);
    let orch = Orchestrator::new(paths.clone(), build_services(&config, &paths), Box::new(RealSpawner));
    let site_procs = SiteProcs::new(paths.clone(), Box::new(RealSpawner));
    AppState { orch: Mutex::new(orch), site_procs: Mutex::new(site_procs), paths, tld: config.tld, starting: AtomicBool::new(false) }
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

/// The full Start-All sequence shared by the UI command and the tray menu:
/// regenerate vhosts + certs + /etc/hosts, (re)start wildcard DNS, ensure nginx
/// can bind, then start every service. Each privileged step self-skips when
/// nothing changed (hosts unchanged, drop-in already correct, cap already set),
/// so a plain restart needs no password — sudo is requested only when a real
/// change (new site/domain/wildcard, or a cleared capability) requires it.
pub fn run_full_start(state: &AppState) -> Vec<String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
    let issuer = MkcertIssuer::resolved(&state.paths);
    let privileged = PkexecPrivileged;
    let bases = sync_sites(
        &state.paths,
        &config.tld,
        &php_socket,
        std::path::Path::new("/etc/hosts"),
        &issuer,
        &privileged,
    )
    .map(|o| o.wildcard_bases)
    .unwrap_or_default();
    let warnings = apply_wildcard_dns(state, &bases);
    // Ensure nginx can bind :80/:443 (re-setcap only if a binary upgrade cleared it).
    ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged);
    if let Ok(mut orch) = state.orch.lock() {
        let _ = orch.start_all();
    }
    // Start each autostart site's Procfile processes once the stack is up.
    let cfg = Config::load(&state.paths.config_file()).unwrap_or_default();
    if !cfg.proc_autostart.is_empty() {
        if let Ok((sites, _warnings)) = list_all_sites(&state.paths, &state.tld) {
            if let Ok(mut sp) = state.site_procs.lock() {
                for s in &sites {
                    if cfg.proc_autostart.contains(&s.name) {
                        sp.start_site(&s.name, &s.root);
                    }
                }
            }
        }
    }
    warnings
}

#[tauri::command]
pub async fn stack_start_all(app: tauri::AppHandle) -> Result<Vec<ServiceStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ServiceStatus>, String> {
        let state = app.state::<AppState>();
        // Re-entrancy guard: if a start is already in progress (e.g. the tray
        // fired Start All while this one is mid-flight), return the current
        // snapshot instead of spawning a second, port-conflicting stack.
        if state.starting.swap(true, Ordering::SeqCst) {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            orch.refresh();
            return Ok(orch.snapshot());
        }
        struct ResetGuard<'a>(&'a AtomicBool);
        impl Drop for ResetGuard<'_> {
            fn drop(&mut self) { self.0.store(false, Ordering::SeqCst); }
        }
        let _reset = ResetGuard(&state.starting);

        let _warnings = run_full_start(&state);
        let mut orch = state.orch.lock().map_err(lock_err)?;
        orch.refresh();
        Ok(orch.snapshot())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn stack_stop_all(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.stop_all();
    if let Ok(mut sp) = state.site_procs.lock() {
        sp.stop_all();
    }
    Ok(orch.snapshot())
}

/// Current per-service enable flags (drives the Settings "Services" toggles).
#[tauri::command]
pub fn service_flags(state: tauri::State<AppState>) -> Result<laralux_core::ServicesConfig, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.services)
}

/// Enable/disable a service: persist the flag, then reconcile the orchestrator so
/// the change takes effect immediately (a disabled service is stopped).
#[tauri::command]
pub fn set_service_enabled(
    state: tauri::State<AppState>,
    kind: ServiceKind,
    enabled: bool,
) -> Result<Vec<ServiceStatus>, String> {
    let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
    match kind {
        ServiceKind::Nginx => config.services.nginx = enabled,
        ServiceKind::PhpFpm => config.services.php = enabled,
        ServiceKind::Mariadb => config.services.mariadb = enabled,
        ServiceKind::Postgres => config.services.postgres = enabled,
        ServiceKind::Mongodb => config.services.mongodb = enabled,
        ServiceKind::Redis => config.services.redis = enabled,
        ServiceKind::Mailpit => config.services.mailpit = enabled,
        ServiceKind::Coredns => return Err("coredns is managed automatically".into()),
    }
    config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
    let new_services = build_services(&config, &state.paths);
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.reconcile(new_services);
    orch.refresh();
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
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<SetupReport, String> {
        let state = app.state::<AppState>();
        let privileged = PkexecPrivileged;
        let downloader = CurlDownloader;
        let progress = TauriProgress(app_for_progress);
        Ok(run_setup(&state.paths, &privileged, &downloader, &RealCommandRunner, &progress))
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
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<CreateReport, String> {
        let state = app.state::<AppState>();
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();

        // Read whether MariaDB is currently running (brief lock).
        let mariadb_running = {
            let orch = state.orch.lock().map_err(|_| "internal lock poisoned".to_string())?;
            orch.state(ServiceKind::Mariadb) == ServiceState::Running
        };

        // Scaffold (slow; no orchestrator lock held).
        let progress = TauriProgress(app_for_progress);
        let mut report = core_create_site(
            &state.paths,
            &name,
            &config.tld,
            template,
            mariadb_running,
            &RealCommandRunner,
            &CurlDownloader,
            &progress,
        )
        .map_err(|e| e.to_string())?;

        // Make it reachable: sync vhost+cert+/etc/hosts, then reload nginx if running.
        // Surface a sync failure as a warning instead of swallowing it — otherwise
        // the site exists on disk but never resolves (no /etc/hosts entry) silently.
        let php_socket = PhpFpmService::new(config.php_version.clone()).socket_path(&state.paths);
        let issuer = MkcertIssuer::resolved(&state.paths);
        let privileged = PkexecPrivileged;
        if let Err(e) = sync_sites(
            &state.paths,
            &config.tld,
            &php_socket,
            std::path::Path::new("/etc/hosts"),
            &issuer,
            &privileged,
        ) {
            report.warnings.push(format!(
                "Site created, but updating /etc/hosts & vhosts did not complete ({e}). \
                 Click Start All (and approve the password prompt) to finish."
            ));
        }
        {
            let mut orch = state.orch.lock().map_err(|_| "internal lock poisoned".to_string())?;
            // Apply new vhosts via SIGHUP reload (no rebind, no downtime).
            let _ = orch.reload(ServiceKind::Nginx);
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
        let issuer = MkcertIssuer::resolved(&state.paths);
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
            // Apply new vhosts via SIGHUP reload (no rebind, no downtime).
            let _ = orch.reload(ServiceKind::Nginx);
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
        let issuer = MkcertIssuer::resolved(&state.paths);
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
            // Apply new vhosts via SIGHUP reload (no rebind, no downtime).
            let _ = orch.reload(ServiceKind::Nginx);
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
    let issuer = MkcertIssuer::resolved(&state.paths);
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
        // Apply new vhosts via SIGHUP reload (no rebind, no downtime).
        let _ = orch.reload(ServiceKind::Nginx);
    }
}

/// Hide a scanned (www-folder) site: rename it to `.<name>` so it drops out of
/// the list/hosts/nginx, keeping the files. Then drop its vhost and re-sync.
#[tauri::command]
pub async fn hide_site(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        laralux_core::hide_scanned_site(&state.paths, &name).map_err(|e| e.to_string())?;
        let vhost = state.paths.etc_for("nginx").join("sites").join(format!("{name}.conf"));
        let _ = std::fs::remove_file(&vhost);
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        sync_and_reload(&state, &config);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Permanently delete a scanned (www-folder) site's folder, drop its vhost and
/// re-sync (so /etc/hosts and nginx stop referencing it).
#[tauri::command]
pub async fn delete_site_folder(app: tauri::AppHandle, name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        laralux_core::delete_scanned_site(&state.paths, &name).map_err(|e| e.to_string())?;
        let vhost = state.paths.etc_for("nginx").join("sites").join(format!("{name}.conf"));
        let _ = std::fs::remove_file(&vhost);
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        sync_and_reload(&state, &config);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
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
pub fn tool_versions(
    state: tauri::State<AppState>,
    tool: String,
) -> Result<Vec<laralux_core::tools::ToolVersion>, String> {
    let t = laralux_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
    Ok(laralux_core::tools::available_versions(t, &state.paths))
}

#[tauri::command]
pub async fn install_tool_version(
    app: tauri::AppHandle,
    tool: String,
    version: String,
) -> Result<Vec<laralux_core::tools::ToolVersion>, String> {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<laralux_core::tools::ToolVersion>, String> {
        let state = app.state::<AppState>();
        let t = laralux_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
        let progress = TauriProgress(app_for_progress);
        laralux_core::tools::install_version(t, &state.paths, &version, &CurlDownloader, &RealCommandRunner, &progress)
            .map_err(|e| e.to_string())?;
        // Keep `current` symlinks reconciled to config after an install.
        let config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let _ = laralux_core::apply_versions(&state.paths, &config);
        Ok(laralux_core::tools::available_versions(t, &state.paths))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_tool_version(
    app: tauri::AppHandle,
    tool: String,
    version: String,
) -> Result<Vec<ServiceStatus>, String> {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ServiceStatus>, String> {
        let state = app.state::<AppState>();
        let t = laralux_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
        let info = laralux_core::tools::info(t);

        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let full = laralux_core::resolve_installed_version(&state.paths, info.key, &version)
            .unwrap_or_else(|| version.clone());
        config.versions.insert(info.key.to_string(), full.clone());
        if t == laralux_core::tools::ManagedTool::Php {
            config.php_version = full.clone();
        }
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;

        let snapshot = if t == laralux_core::tools::ManagedTool::Nginx {
            // nginx can't use the generic replace_version: the new binary file needs
            // cap_net_bind_service re-applied (setcap) AFTER `current` is repointed and
            // BEFORE start, or it can't bind :80/:443. So: stop -> set_current -> setcap -> start.
            let was_running = {
                let mut orch = state.orch.lock().map_err(lock_err)?;
                let running = orch.state(ServiceKind::Nginx) == ServiceState::Running;
                if running { let _ = orch.stop(ServiceKind::Nginx); }
                running
            };
            laralux_core::set_current(&state.paths, "nginx", &full).map_err(|e| e.to_string())?;
            ensure_nginx_bind_cap(&state.paths, &PkexecPrivileged);
            let mut orch = state.orch.lock().map_err(lock_err)?;
            if was_running {
                orch.start(ServiceKind::Nginx).map_err(|e| e.to_string())?;
            }
            orch.snapshot()
        } else {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            match info.service_kind {
                Some(kind) => { orch.replace_version(kind, info.key, &version).map_err(|e| e.to_string())?; }
                None => { laralux_core::set_current(&state.paths, info.key, &full).map_err(|e| e.to_string())?; }
            }
            orch.snapshot()
        };

        if t == laralux_core::tools::ManagedTool::Php {
            let progress = TauriProgress(app_for_progress);
            let _ = laralux_core::ensure_active_php_cli(&state.paths, &version, &CurlDownloader, &RealCommandRunner, &progress);
        }
        Ok(snapshot)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn tool_symlinks(state: tauri::State<AppState>) -> Result<Vec<String>, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.symlinks.into_iter().collect())
}

#[tauri::command]
pub async fn set_tool_symlink(
    app: tauri::AppHandle,
    tool: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
        let state = app.state::<AppState>();
        let t = laralux_core::tools::from_key(&tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
        if enabled {
            laralux_core::link_tool(&state.paths, t, &PkexecPrivileged).map_err(|e| e.to_string())?;
        } else {
            laralux_core::unlink_tool(t, &PkexecPrivileged).map_err(|e| e.to_string())?;
        }
        let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
        let k = laralux_core::tools::key(t).to_string();
        if enabled { config.symlinks.insert(k); } else { config.symlinks.remove(&k); }
        config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
        Ok(config.symlinks.into_iter().collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn php_ini_settings(
    state: tauri::State<AppState>,
) -> Result<laralux_core::php_ini::PhpIniSettings, String> {
    let config = Config::load(&state.paths.config_file()).unwrap_or_default();
    Ok(config.php_ini)
}

#[tauri::command]
pub async fn set_php_ini_settings(
    app: tauri::AppHandle,
    settings: laralux_core::php_ini::PhpIniSettings,
) -> Result<laralux_core::php_ini::PhpIniSettings, String> {
    tauri::async_runtime::spawn_blocking(
        move || -> Result<laralux_core::php_ini::PhpIniSettings, String> {
            let state = app.state::<AppState>();
            laralux_core::php_ini::validate(&settings).map_err(|e| e.to_string())?;
            let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
            config.php_ini = settings.clone();
            config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
            {
                let mut orch = state.orch.lock().map_err(lock_err)?;
                orch.apply_php_ini(&settings).map_err(|e| e.to_string())?;
            }
            // Only escalate when the CLI symlink isn't already in place — keeps Apply
            // sudo-free after the first time (web always applies via php-fpm's -c).
            let target = laralux_core::php_ini::php_ini_path(&state.paths);
            let already_linked = std::fs::read_link(laralux_core::php_ini::SYSTEM_PHP_INI)
                .map(|p| p == target)
                .unwrap_or(false);
            if !already_linked {
                let _ = laralux_core::privileged::PkexecPrivileged.ensure_php_ini_link(&target);
            }
            Ok(settings)
        },
    )
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_terminal(path: String) -> Result<(), String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    laralux_core::open_terminal(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("not a directory: {path}"));
    }
    laralux_core::open_folder(&dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_db_client(app: tauri::AppHandle) -> Result<(), String> {
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        if !laralux_core::dbgate::is_installed(&state.paths) {
            let progress = TauriProgress(app_for_progress);
            laralux_core::dbgate::ensure_dbgate(
                &state.paths,
                &CurlDownloader,
                &RealCommandRunner,
                &progress,
            )
            .map_err(|e| e.to_string())?;
        }
        laralux_core::open_dbgate(&state.paths).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

const RESOLVED_DROPIN_PATH: &str = "/etc/systemd/resolved.conf.d/laralux.conf";

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

/// Apply DNS state for the current wildcard bases. Returns non-fatal warnings.
/// CoreDNS (a process) is (re)started every call, but the privileged
/// systemd-resolved drop-in is only written/removed when it actually changes,
/// so a plain restart with unchanged wildcard config needs no password.
fn apply_wildcard_dns(state: &AppState, bases: &[String]) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    if bases.is_empty() {
        if let Ok(mut orch) = state.orch.lock() {
            let _ = orch.set_coredns(vec![]);
        }
        kill_stale_coredns(state);
        // Only prompt to remove the drop-in if it is actually present.
        if std::path::Path::new(RESOLVED_DROPIN_PATH).exists() {
            if let Err(e) = PkexecPrivileged.remove_resolved_dropin() {
                warnings.push(format!("Could not remove DNS routing drop-in: {e}"));
            }
        }
        return warnings;
    }
    if let Err(e) = ensure_coredns(&state.paths, &CurlDownloader, &RealCommandRunner, &laralux_core::NullProgress) {
        warnings.push(format!("Wildcard DNS unavailable (CoreDNS download failed): {e}"));
        return warnings;
    }
    kill_stale_coredns(state);
    if let Ok(mut orch) = state.orch.lock() {
        if let Err(e) = orch.set_coredns(bases.to_vec()) {
            warnings.push(format!("Could not start CoreDNS: {e}"));
        }
    }
    // Only prompt to write the drop-in when its content actually changed
    // (trailing-newline-insensitive), so a plain restart needs no password.
    let desired = resolved_dropin(bases, 5353);
    let current = std::fs::read_to_string(RESOLVED_DROPIN_PATH).ok();
    if current.as_deref().map(str::trim_end) != Some(desired.trim_end()) {
        if let Err(e) = PkexecPrivileged.write_resolved_dropin(&desired) {
            warnings.push(format!("Could not write DNS routing drop-in: {e}"));
        }
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
        let issuer = MkcertIssuer::resolved(&state.paths);
        let privileged = PkexecPrivileged;
        let outcome = sync_sites(
            &state.paths, &config.tld, &php_socket,
            std::path::Path::new("/etc/hosts"), &issuer, &privileged,
        );
        let bases = outcome.as_ref().map(|o| o.wildcard_bases.clone()).unwrap_or_default();
        let warnings = apply_wildcard_dns(&state, &bases);
        {
            let mut orch = state.orch.lock().map_err(lock_err)?;
            // Apply new vhosts via SIGHUP reload (no rebind, no downtime).
            let _ = orch.reload(ServiceKind::Nginx);
        }
        let (sites, _w) = list_all_sites(&state.paths, &config.tld).map_err(|e| e.to_string())?;
        Ok(SetDomainsResult { sites, warnings })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// View-model for the Processes modal: the site's declared procs (merged with
/// live state) plus its autostart flag.
#[derive(serde::Serialize)]
pub struct SiteProcsView {
    pub procs: Vec<ProcStatus>,
    pub autostart: bool,
}

/// Build the view: refresh liveness, merge the parsed Procfile entries with
/// current state, and read the autostart flag. Caller holds the SiteProcs lock.
fn site_procs_view(sp: &mut SiteProcs, paths: &LaraluxPaths, name: &str, root: &str) -> SiteProcsView {
    sp.refresh();
    let entries = read_procfile(Path::new(root)).unwrap_or_default();
    let procs = entries
        .into_iter()
        .map(|e| ProcStatus {
            site: name.to_string(),
            name: e.name.clone(),
            command: e.command,
            state: sp.state_of(name, &e.name),
            pid: sp.pid_of(name, &e.name),
        })
        .collect();
    let autostart = Config::load(&paths.config_file())
        .unwrap_or_default()
        .proc_autostart
        .contains(name);
    SiteProcsView { procs, autostart }
}

#[tauri::command]
pub fn site_procs(state: tauri::State<AppState>, name: String, root: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn start_site_proc(state: tauri::State<AppState>, name: String, root: String, proc: String) -> Result<SiteProcsView, String> {
    let cmd = read_procfile(Path::new(&root))
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.name == proc)
        .map(|e| e.command);
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    if let Some(c) = cmd {
        let _ = sp.start(&name, Path::new(&root), &proc, &c);
    }
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn stop_site_proc(state: tauri::State<AppState>, name: String, root: String, proc: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    sp.stop(&name, &proc);
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn start_site_procs(state: tauri::State<AppState>, name: String, root: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    sp.start_site(&name, Path::new(&root));
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn stop_site_procs(state: tauri::State<AppState>, name: String, root: String) -> Result<SiteProcsView, String> {
    let mut sp = state.site_procs.lock().map_err(lock_err)?;
    sp.stop_site(&name);
    Ok(site_procs_view(&mut sp, &state.paths, &name, &root))
}

#[tauri::command]
pub fn set_site_autostart(state: tauri::State<AppState>, name: String, enabled: bool) -> Result<bool, String> {
    let mut config = Config::load(&state.paths.config_file()).unwrap_or_default();
    if enabled {
        config.proc_autostart.insert(name);
    } else {
        config.proc_autostart.remove(&name);
    }
    config.save(&state.paths.config_file()).map_err(|e| e.to_string())?;
    Ok(enabled)
}

#[tauri::command]
pub fn site_proc_log_path(state: tauri::State<AppState>, name: String, proc: String) -> Result<String, String> {
    Ok(state.paths.log().join(format!("proc-{name}-{proc}.log")).display().to_string())
}

#[tauri::command]
pub fn site_proc_counts(state: tauri::State<AppState>) -> Result<std::collections::BTreeMap<String, usize>, String> {
    let (sites, _warnings) = list_all_sites(&state.paths, &state.tld).map_err(|e| e.to_string())?;
    let mut out = std::collections::BTreeMap::new();
    for s in sites {
        if let Some(entries) = read_procfile(&s.root) {
            if !entries.is_empty() {
                out.insert(s.name, entries.len());
            }
        }
    }
    Ok(out)
}
