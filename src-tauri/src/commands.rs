use laragon_core::{
    build_services, detect_components, run_setup, scan_sites, Config, LaragonPaths, Orchestrator,
    PkexecPrivileged, RealSpawner, ServiceKind, ServiceStatus, Site,
};
use laragon_core::{ComponentStatus, CurlDownloader, SetupReport};
use std::sync::Mutex;

/// Shared, app-lifetime state. The orchestrator owns the running child
/// processes, so it must live as long as the app and be stopped on exit.
pub struct AppState {
    pub orch: Mutex<Orchestrator>,
    pub paths: LaragonPaths,
    pub tld: String,
    pub php_version: String,
}

/// Build the managed state from the on-disk config.
pub fn build_state() -> AppState {
    let paths = LaragonPaths::new(LaragonPaths::default_root());
    let config = Config::load(&paths.config_file()).unwrap_or_default();
    let _ = paths.ensure_dirs();
    let orch = Orchestrator::new(paths.clone(), build_services(&config, &paths), Box::new(RealSpawner));
    AppState { orch: Mutex::new(orch), paths, tld: config.tld, php_version: config.php_version }
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
pub fn stack_start_all(state: tauri::State<AppState>) -> Result<Vec<ServiceStatus>, String> {
    let mut orch = state.orch.lock().map_err(lock_err)?;
    orch.start_all().map_err(|e| e.to_string())?;
    Ok(orch.snapshot())
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
    scan_sites(&state.paths, &state.tld).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn setup_status(state: tauri::State<AppState>) -> Result<Vec<ComponentStatus>, String> {
    Ok(detect_components(&state.paths, &state.php_version))
}

#[tauri::command]
pub fn run_setup_cmd(state: tauri::State<AppState>) -> Result<SetupReport, String> {
    let privileged = PkexecPrivileged;
    let downloader = CurlDownloader;
    Ok(run_setup(&state.paths, &state.php_version, &privileged, &downloader))
}
