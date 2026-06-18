#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::{build_state, AppState};
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(build_state())
        .invoke_handler(tauri::generate_handler![
            commands::stack_status,
            commands::stack_start_all,
            commands::stack_stop_all,
            commands::service_start,
            commands::service_stop,
            commands::list_sites,
        ])
        .on_window_event(|window, event| {
            // Stop the stack cleanly when the last window is destroyed.
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    if let Ok(mut orch) = state.orch.lock() {
                        orch.stop_all();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Laragon Linux");
}
