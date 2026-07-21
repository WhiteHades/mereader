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

use std::collections::HashSet;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use uuid::Uuid;

use state::AppState;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct BoundedLogFile {
    file: fs::File,
    size: u64,
    maximum: u64,
}

impl Write for BoundedLogFile {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self
            .size
            .checked_add(buffer.len() as u64)
            .is_none_or(|size| size > self.maximum)
        {
            self.file.set_len(0)?;
            self.file.seek(SeekFrom::Start(0))?;
            self.size = 0;
        }
        let written = self.file.write(buffer)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn initialize_logging(root: &Path) -> error::AppResult<()> {
    storage::validate_logs_directory(root)?;
    let logs = root.join(storage::LOGS_DIR);
    let path = logs.join("mereader.log");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(error::AppError::storage());
        }
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| error::AppError::storage())?;
    storage::set_private_permissions(&path, false)?;
    let mut size = file
        .metadata()
        .map_err(|_| error::AppError::storage())?
        .len();
    if size > MAX_LOG_BYTES {
        file.set_len(0).map_err(|_| error::AppError::storage())?;
        size = 0;
    }
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_writer(Mutex::new(BoundedLogFile {
            file,
            size,
            maximum: MAX_LOG_BYTES,
        }))
        .try_init()
        .map_err(|_| {
            error::AppError::new(
                error::AppErrorKind::Internal,
                "Application logging could not be initialized.",
            )
        })?;
    Ok(())
}

fn move_to_trash(root: &Path, path: &Path) -> error::AppResult<()> {
    let trash = root
        .join(storage::TRASH_DIR)
        .join(Uuid::new_v4().to_string());
    fs::rename(path, trash).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to quarantine inconsistent book data");
        error::AppError::storage()
    })
}

fn reconcile_library(root: &Path, connection: &mut rusqlite::Connection) -> error::AppResult<()> {
    for (book_id, trash_name) in db::pending_deletions(connection)? {
        let book_id = storage::validated_uuid(&book_id, "Invalid deletion journal book ID.")?;
        let trash_name = storage::validated_uuid(&trash_name, "Invalid deletion journal path.")?;
        let active = storage::book_path(root, &book_id)?;
        let trash = root.join(storage::TRASH_DIR).join(trash_name);
        if !trash.exists() && active.exists() {
            fs::rename(&active, &trash).map_err(|error| {
                tracing::error!(%error, book_id, "failed to resume journaled book deletion");
                error::AppError::storage()
            })?;
        }
        db::complete_book_deletion(connection, &book_id)?;
    }

    let persisted = db::book_ids(connection)?
        .into_iter()
        .collect::<HashSet<_>>();
    for entry in
        fs::read_dir(root.join(storage::BOOKS_DIR)).map_err(|_| error::AppError::storage())?
    {
        let entry = entry.map_err(|_| error::AppError::storage())?;
        let name = entry.file_name();
        let name = name.to_str().unwrap_or_default();
        let canonical_id = Uuid::parse_str(name).ok().map(|id| id.to_string());
        if canonical_id
            .as_ref()
            .is_none_or(|id| !persisted.contains(id))
        {
            move_to_trash(root, &entry.path())?;
        }
    }
    for book_id in persisted {
        let path = storage::book_path(root, &book_id)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::error!(book_id, "removing database rows for missing book files");
                db::remove_missing_book(connection, &book_id)?;
            }
            Ok(_) => {
                tracing::error!(book_id, "book path is not a real directory");
                return Err(error::AppError::storage());
            }
            Err(error) => {
                tracing::error!(%error, book_id, "book path could not be inspected during recovery");
                return Err(error::AppError::storage());
            }
        }
    }
    storage::cleanup_directory_best_effort(&root.join(storage::TRASH_DIR));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let result = (|| -> error::AppResult<()> {
                let app_data = app
                    .path()
                    .app_data_dir()
                    .map_err(|_| error::AppError::storage())?;
                let root = storage::initialize(&app_data)?;
                initialize_logging(&root)?;
                let mut connection = db::open_database(&root.join("library.sqlite3"))?;
                reconcile_library(&root, &mut connection)?;
                let state = AppState::new(root, connection)?;
                app.manage(state.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = ollama::retry_pending_indexes(state).await {
                        tracing::warn!(kind = ?error.kind, "startup index retry stopped");
                    }
                });
                Ok(())
            })();
            if let Err(error) = result {
                app.dialog()
                    .message(format!(
                        "MeReader could not open its local library. {}",
                        error.message
                    ))
                    .title("MeReader startup failed")
                    .kind(MessageDialogKind::Error)
                    .buttons(MessageDialogButtons::Ok)
                    .blocking_show();
                return Err(error.into());
            }
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
            commands::cancel_ai_request,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| eprintln!("MeReader runtime failed: {error}"));
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rusqlite::params;

    use super::*;

    fn insert_book(connection: &rusqlite::Connection, book_id: &str) {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "INSERT INTO books(id, source_hash, title, author, source_rel_path, content_length, total_locations, total_chapters, created_at, updated_at) VALUES (?1, ?2, 'Title', 'Author', ?3, 1, 1, 0, ?4, ?4)",
                params![book_id, format!("hash-{book_id}"), format!("books/{book_id}/source.epub"), now],
            )
            .unwrap();
    }

    #[test]
    fn startup_reconciles_journaled_deletions_and_orphans() {
        let root = std::env::temp_dir().join(format!("mereader-recovery-{}", Uuid::new_v4()));
        let root = storage::initialize(&root).unwrap();
        let mut connection = db::open_database(&root.join("library.sqlite3")).unwrap();
        let deleted_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let orphan_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        let missing_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        fs::create_dir(root.join(storage::BOOKS_DIR).join(deleted_id)).unwrap();
        fs::create_dir(root.join(storage::BOOKS_DIR).join(orphan_id)).unwrap();
        insert_book(&connection, deleted_id);
        insert_book(&connection, missing_id);
        db::begin_book_deletion(
            &mut connection,
            deleted_id,
            "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
        )
        .unwrap();

        reconcile_library(&root, &mut connection).unwrap();

        assert!(db::book_ids(&connection).unwrap().is_empty());
        assert!(db::pending_deletions(&connection).unwrap().is_empty());
        assert_eq!(
            fs::read_dir(root.join(storage::BOOKS_DIR)).unwrap().count(),
            0
        );
        assert_eq!(
            fs::read_dir(root.join(storage::TRASH_DIR)).unwrap().count(),
            0
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn log_writer_stays_within_its_size_limit() {
        let root = std::env::temp_dir().join(format!("mereader-log-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("log");
        let file = fs::File::create(&path).unwrap();
        let mut writer = BoundedLogFile {
            file,
            size: 0,
            maximum: 8,
        };

        writer.write_all(b"123456").unwrap();
        writer.write_all(b"abcdef").unwrap();
        writer.flush().unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"abcdef");
        fs::remove_dir_all(root).unwrap();
    }
}
