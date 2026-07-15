mod commands;
mod content;
mod db;
mod error;
mod models;
mod ollama;
mod retrieval;
mod state;

use std::fs;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            fs::create_dir_all(root.join("books"))?;
            let connection = db::open_database(&root.join("library.sqlite3"))?;
            app.manage(AppState::new(root, connection)?);
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
            commands::ask_book,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MeReader");
}
