#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::{build_state, AppState};
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

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
            commands::php_versions,
            commands::install_php_version,
            commands::set_php_version,
            commands::terminal_integration_status,
            commands::set_terminal_integration,
            commands::open_terminal,
            commands::set_site_domains,
        ])
        .setup(|app| {
            let start = MenuItemBuilder::with_id("start_all", "Start All").build(app)?;
            let stop = MenuItemBuilder::with_id("stop_all", "Stop All").build(app)?;
            let dashboard = MenuItemBuilder::with_id("dashboard", "Dashboard").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&start, &stop, &dashboard, &quit])
                .build()?;

            let icon = Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            TrayIconBuilder::new()
                .icon(icon)
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
                std::thread::spawn(move || {
                    let mut last: Option<Vec<laragon_core::ServiceStatus>> = None;
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        let Some(state) = handle.try_state::<AppState>() else { continue };
                        let snap = match state.orch.lock() {
                            Ok(mut orch) => { orch.refresh(); orch.snapshot() }
                            Err(_) => continue,
                        };
                        if last.as_ref() != Some(&snap) {
                            let _ = handle.emit("services-changed", &snap);
                            last = Some(snap);
                        }
                    }
                });
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
        .expect("error while building Laragon Linux")
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
