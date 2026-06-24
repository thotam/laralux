#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::{build_state, AppState};
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
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
                            laragon_core::ensure_nginx_bind_cap(
                                &state.paths,
                                &laragon_core::PkexecPrivileged,
                            );
                            if let Ok(mut orch) = state.orch.lock() {
                                let _ = orch.start_all();
                            }
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
