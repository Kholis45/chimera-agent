pub mod commands;

use commands::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::init_scheduler,
            commands::add_task,
            commands::run_scheduler,
            commands::get_scheduler_status,
            commands::list_tasks,
            commands::configure_llm,
            commands::llm_complete,
            commands::llm_stream,
            commands::shell_execute,
            commands::file_operation,
            commands::configure_sandbox,
            commands::health_check,
        ])
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                let window = app.get_webview_window("main").unwrap();
                window.open_devtools();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}