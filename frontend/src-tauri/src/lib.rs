mod commands;
mod content;
mod db;
mod error;
mod limits;
mod models;
mod ollama;
mod retrieval;
mod state;
mod storage;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = storage::initialize(&app.path().app_data_dir()?)?;
            let connection = db::open_database(&root.join("library.sqlite3"))?;
            let state = AppState::new(root, connection)?;
            app.manage(state.clone());
            tauri::async_runtime::spawn(async move {
                if let Err(error) = ollama::retry_pending_indexes(state).await {
                    tracing::warn!(kind = ?error.kind, "startup index retry stopped");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_books,
            commands::import_book,
            commands::get_book,
            commands::get_cover,
            commands::get_chapter,
            commands::get_chapter_asset,
            commands::update_progress,
            commands::delete_book,
            commands::get_ai_status,
            commands::reindex_book,
            commands::ask_book,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MeReader");
}
