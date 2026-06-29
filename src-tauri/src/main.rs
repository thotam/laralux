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
        // MUST be the first plugin: enforce a single instance. A second launch
        // (from the launcher/dock while already running) hands its argv to the
        // running instance and exits, instead of spawning a duplicate window
        // that would fight over the same ports/state. We just resurface the
        // existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
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
            commands::service_flags,
            commands::set_service_enabled,
            commands::site_procs,
            commands::start_site_proc,
            commands::stop_site_proc,
            commands::start_site_procs,
            commands::stop_site_procs,
            commands::set_site_autostart,
            commands::site_proc_log_path,
            commands::site_proc_counts,
            commands::launch_config,
            commands::set_launch_option,
        ])
        .setup(|app| {
            // One menu item whose LABEL toggles between Start All / Stop All so the
            // tray shows a single action. We change its text (set_text), never the
            // menu structure: the AppIndicator tray serializes its menu over DBus,
            // and adding/removing items desyncs it and renders the menu blank.
            // Start on the stopped-stack label.
            let toggle = MenuItemBuilder::with_id("stack_toggle", "Start All").build(app)?;
            let dashboard = MenuItemBuilder::with_id("dashboard", "Dashboard").build(app)?;
            let db_client = MenuItemBuilder::with_id("db_client", "DB client (DbGate)").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&toggle, &dashboard, &db_client, &quit])
                .build()?;

            // Start on the "stopped" icon; the monitor below swaps it to reflect
            // the live stack state (running / starting / crashed).
            let tray = TrayIconBuilder::new()
                .icon(Image::from_bytes(include_bytes!("../icons/tray-stopped.png"))?)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "stack_toggle" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            // Decide by current state: all up → Stop All, else Start All.
                            let all_running = match state.orch.lock() {
                                Ok(mut o) => {
                                    o.refresh();
                                    let s = o.snapshot();
                                    !s.is_empty()
                                        && s.iter().all(|x| x.state == laralux_core::ServiceState::Running)
                                }
                                Err(_) => return,
                            };
                            if all_running {
                                if let Ok(mut orch) = state.orch.lock() {
                                    orch.stop_all();
                                }
                            } else {
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
                                // Same full startup as the UI command (sync hosts/cert/DNS
                                // + setcap + start_all); each privileged step self-skips
                                // when unchanged, so it prompts for a password only when
                                // needed.
                                let _ = commands::run_full_start(&state);
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
                            if let Ok(mut sp) = state.site_procs.lock() {
                                sp.stop_all();
                            }
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Apply launch-behavior config: show the window unless "start
            // minimized", and optionally auto-start the stack on launch.
            let launch = {
                let st = app.state::<AppState>();
                laralux_core::Config::load(&st.paths.config_file())
                    .unwrap_or_default()
                    .launch
            };
            if !launch.start_minimized {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            if launch.autostart_services {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let Some(state) = handle.try_state::<AppState>() else { return };
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
                    let _ = commands::run_full_start(&state);
                });
            }

            // Realtime service status: poll liveness server-side every ~1s and
            // push `services-changed` ONLY when the snapshot actually changes, so
            // crashes surface within ~1s and the UI never re-renders while idle.
            {
                let handle = app.handle().clone();
                let tray = tray.clone();
                let toggle_item = toggle.clone();
                std::thread::spawn(move || {
                    let mut last: Option<Vec<laralux_core::ServiceStatus>> = None;
                    let mut last_tray: Option<TrayState> = None;
                    // Seed to the initial toggle label ("Start All" = not-all-running),
                    // so the first stopped-stack tick is a no-op.
                    let mut last_all_running: Option<bool> = Some(false);
                    let mut last_procs: Vec<(String, String, laralux_core::ServiceState)> = Vec::new();
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
                        // Toggle label: all services up → "Stop All", else "Start All".
                        let all_running = !snap.is_empty()
                            && snap.iter().all(|s| s.state == laralux_core::ServiceState::Running);
                        if last_all_running != Some(all_running) {
                            // Update the single toggle item's LABEL only — no structural
                            // menu mutation (that blanks the AppIndicator menu). On the
                            // main thread because it touches the GTK/menu object.
                            let toggle_m = toggle_item.clone();
                            let _ = handle.run_on_main_thread(move || {
                                let _ = toggle_m.set_text(if all_running { "Stop All" } else { "Start All" });
                            });
                            last_all_running = Some(all_running);
                        }
                        {
                            if let Ok(mut sp) = state.site_procs.lock() {
                                sp.refresh();
                                let pairs = sp.state_pairs();
                                if pairs != last_procs {
                                    let _ = handle.emit("site-procs-changed", ());
                                    last_procs = pairs;
                                }
                            };
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
                    if let Ok(mut sp) = state.site_procs.lock() {
                        sp.stop_all();
                    }
                }
            }
        });
}
