#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::{build_state, AppState};
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

/// Overall stack state shown in the tray. Priority:
/// crashed > starting > running (any up) > stopped (all down).
#[derive(PartialEq, Clone, Copy)]
enum TrayState { Stopped, Starting, Running, Crashed }

/// `starting_flag` is the in-progress Start-All guard (`AppState.starting`); a
/// synchronous start barely passes through `Starting` per service, so the guard
/// is what makes the "starting" icon actually visible during a start.
fn tray_state(snap: &[laralux_core::ServiceStatus], starting_flag: bool) -> TrayState {
    use laralux_core::ServiceState;
    if snap.iter().any(|s| s.state == ServiceState::Crashed) {
        TrayState::Crashed
    } else if starting_flag || snap.iter().any(|s| s.state == ServiceState::Starting) {
        TrayState::Starting
    } else if snap.iter().any(|s| s.state == ServiceState::Running) {
        TrayState::Running
    } else {
        TrayState::Stopped
    }
}

fn tray_state_bytes(s: TrayState) -> &'static [u8] {
    match s {
        TrayState::Stopped => include_bytes!("../icons/tray-stopped.png"),
        TrayState::Starting => include_bytes!("../icons/tray-starting.png"),
        TrayState::Running => include_bytes!("../icons/tray-running.png"),
        TrayState::Crashed => include_bytes!("../icons/tray-crashed.png"),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(build_state())
        .invoke_handler(tauri::generate_handler![
            commands::stack_status,
            commands::stack_start_all,
            commands::stack_stop_all,
            commands::service_start,
            commands::service_stop,
            commands::list_sites,
            commands::setup_status,
            commands::run_setup_cmd,
            commands::create_site,
            commands::link_site,
            commands::unlink_site,
            commands::add_proxy,
            commands::update_proxy,
            commands::tool_versions,
            commands::install_tool_version,
            commands::set_tool_version,
            commands::tool_symlinks,
            commands::set_tool_symlink,
            commands::php_ini_settings,
            commands::set_php_ini_settings,
            commands::open_terminal,
            commands::open_folder,
            commands::set_site_domains,
            commands::open_db_client,
            commands::hide_site,
            commands::delete_site_folder,
        ])
        .setup(|app| {
            let start = MenuItemBuilder::with_id("start_all", "Start All").build(app)?;
            let stop = MenuItemBuilder::with_id("stop_all", "Stop All").build(app)?;
            let dashboard = MenuItemBuilder::with_id("dashboard", "Dashboard").build(app)?;
            let db_client = MenuItemBuilder::with_id("db_client", "DB client (DbGate)").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&start, &stop, &dashboard, &db_client, &quit])
                .build()?;

            // Tray shows only one of Start All / Stop All; start on the stopped
            // stack so only Start All is visible. The monitor toggles them.
            // (Tauri 2.11 MenuItem has no set_visible; we remove/insert instead.)
            let _ = menu.remove(&stop);

            // Start on the "stopped" icon; the monitor below swaps it to reflect
            // the live stack state (running / starting / crashed).
            let tray = TrayIconBuilder::new()
                .icon(Image::from_bytes(include_bytes!("../icons/tray-stopped.png"))?)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "start_all" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if state.starting.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                return;
                            }
                            struct ResetGuard<'a>(&'a std::sync::atomic::AtomicBool);
                            impl Drop for ResetGuard<'_> {
                                fn drop(&mut self) {
                                    self.0.store(false, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                            let _reset = ResetGuard(&state.starting);
                            // Same full startup as the UI command (sync hosts/cert/DNS +
                            // setcap + start_all); each privileged step self-skips when
                            // unchanged, so this prompts for a password only when needed.
                            let _ = commands::run_full_start(&state);
                        }
                    }
                    "stop_all" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(mut orch) = state.orch.lock() {
                                orch.stop_all();
                            }
                        }
                    }
                    "dashboard" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "db_client" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if laralux_core::dbgate::is_installed(&state.paths) {
                                let _ = laralux_core::open_dbgate(&state.paths);
                            } else if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                    }
                    "quit" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            if let Ok(mut orch) = state.orch.lock() {
                                orch.stop_all();
                            }
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Realtime service status: poll liveness server-side every ~1s and
            // push `services-changed` ONLY when the snapshot actually changes, so
            // crashes surface within ~1s and the UI never re-renders while idle.
            {
                let handle = app.handle().clone();
                let tray = tray.clone();
                let start_item = start.clone();
                let stop_item = stop.clone();
                let menu_handle = menu.clone();
                std::thread::spawn(move || {
                    let mut last: Option<Vec<laralux_core::ServiceStatus>> = None;
                    let mut last_tray: Option<TrayState> = None;
                    // Seed to the actual initial menu state: startup removed `stop`,
                    // so the menu shows only Start All (= not-all-running). This makes
                    // the first stopped-stack tick a no-op (no duplicate Start All).
                    let mut last_all_running: Option<bool> = Some(false);
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        let Some(state) = handle.try_state::<AppState>() else { continue };
                        let starting_flag = state.starting.load(std::sync::atomic::Ordering::SeqCst);
                        let snap = match state.orch.lock() {
                            Ok(mut orch) => { orch.refresh(); orch.snapshot() }
                            Err(_) => continue,
                        };
                        if last.as_ref() != Some(&snap) {
                            let _ = handle.emit("services-changed", &snap);
                            last = Some(snap.clone());
                        }
                        // Update the tray whenever the VISUAL state changes (incl. the
                        // `starting` guard flipping), not only when the snapshot does.
                        let ts = tray_state(&snap, starting_flag);
                        if last_tray != Some(ts) {
                            if let Ok(img) = Image::from_bytes(tray_state_bytes(ts)) {
                                let _ = tray.set_icon(Some(img));
                            }
                            last_tray = Some(ts);
                        }
                        // Tray shows only one of Start All / Stop All: all up → Stop All.
                        // Tauri 2.11 MenuItem has no set_visible; we remove/insert instead.
                        let all_running = !snap.is_empty()
                            && snap.iter().all(|s| s.state == laralux_core::ServiceState::Running);
                        if last_all_running != Some(all_running) {
                            // The menu is a GTK object on Linux: mutating it from this
                            // background thread corrupts it (the tray menu then renders
                            // blank when next opened). Marshal the swap onto the main
                            // thread. Position 0 is where Start/Stop sit in the original
                            // MenuBuilder order; keep this in sync if the menu is reordered.
                            let menu_m = menu_handle.clone();
                            let start_m = start_item.clone();
                            let stop_m = stop_item.clone();
                            let _ = handle.run_on_main_thread(move || {
                                if all_running {
                                    let _ = menu_m.remove(&start_m);
                                    let _ = menu_m.insert(&stop_m, 0);
                                } else {
                                    let _ = menu_m.remove(&stop_m);
                                    let _ = menu_m.insert(&start_m, 0);
                                }
                            });
                            last_all_running = Some(all_running);
                        }
                    }
                });
            }

            // Realtime site list: watch ~/laralux/www (non-recursive: immediate
            // subdirs are sites) and sites.toml; push `sites-changed` (debounced)
            // so external folder/registry edits appear without polling.
            {
                let handle = app.handle().clone();
                let paths = handle.state::<AppState>().paths.clone();
                let www = paths.www();
                let _ = std::fs::create_dir_all(&www);
                let last = std::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));
                if let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if res.is_ok() {
                        let mut l = last.lock().unwrap();
                        if l.elapsed() >= std::time::Duration::from_millis(300) {
                            *l = std::time::Instant::now();
                            let _ = handle.emit("sites-changed", ());
                        }
                    }
                }) {
                    use notify::Watcher;
                    let _ = watcher.watch(&www, notify::RecursiveMode::NonRecursive);
                    let _ = watcher.watch(&paths.sites_file(), notify::RecursiveMode::NonRecursive);
                    // Keep the watcher alive for the app lifetime (Send, not Sync).
                    std::thread::spawn(move || { let _keep = watcher; loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } });
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Laralux")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    if let Ok(mut orch) = state.orch.lock() {
                        orch.stop_all();
                    }
                }
            }
        });
}
