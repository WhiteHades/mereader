use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use rusqlite_migration::{Migrations, M};

use crate::error::{AppError, AppResult};
use crate::limits::{
    MAX_CHUNKS_PER_BOOK, MAX_COMPRESSED_BYTES, MAX_LIBRARY_BOOKS, MAX_NORMALIZED_TEXT_CHARS,
    MAX_SPINE_CHAPTERS,
};
use crate::models::{BookDetail, BookList, BookSummary, ChapterContent, ChapterSummary, Progress};
use crate::state::{DEFAULT_EMBEDDING_MODEL, DEFAULT_GENERATION_MODEL};
use crate::storage;

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            r#"
        CREATE TABLE books (
            id TEXT PRIMARY KEY,
            source_hash TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            author TEXT NOT NULL,
            source_rel_path TEXT NOT NULL,
            cover_rel_path TEXT,
            cover_mime TEXT,
            language TEXT,
            published_year INTEGER,
            publisher TEXT,
            isbn TEXT,
            description TEXT,
            content_length INTEGER NOT NULL,
            total_locations INTEGER NOT NULL,
            total_chapters INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE chapters (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            chapter_order INTEGER NOT NULL,
            content_rel_path TEXT NOT NULL,
            start_location INTEGER NOT NULL CHECK(start_location >= 0),
            end_location INTEGER NOT NULL CHECK(end_location >= start_location),
            char_count INTEGER NOT NULL CHECK(char_count >= 0),
            UNIQUE(book_id, chapter_order)
        );

        CREATE TABLE progress (
            book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
            current_location INTEGER NOT NULL DEFAULT 0 CHECK(current_location >= 0),
            current_chapter_id TEXT REFERENCES chapters(id) ON DELETE SET NULL,
            completion_percentage REAL NOT NULL DEFAULT 0.0,
            last_read_at TEXT
        );

        CREATE TABLE chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            chapter_id TEXT NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
            chunk_order INTEGER NOT NULL,
            text TEXT NOT NULL,
            start_location INTEGER NOT NULL CHECK(start_location >= 0),
            end_location INTEGER NOT NULL CHECK(end_location > start_location),
            embedding BLOB,
            UNIQUE(chapter_id, chunk_order)
        );

        CREATE VIRTUAL TABLE chunks_fts USING fts5(text, tokenize='unicode61');

        CREATE TABLE index_state (
            book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,
            status TEXT NOT NULL,
            embedding_model TEXT,
            vector_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE INDEX chapters_book_order_idx ON chapters(book_id, chapter_order);
        CREATE INDEX chunks_book_boundary_idx ON chunks(book_id, end_location);
        CREATE INDEX chunks_chapter_order_idx ON chunks(chapter_id, chunk_order);
        "#,
        ),
        M::up(
            r#"
            ALTER TABLE index_state ADD COLUMN vector_dimension INTEGER;
            ALTER TABLE index_state ADD COLUMN generation INTEGER NOT NULL DEFAULT 0;
            "#,
        ),
        M::up(
            r#"
            CREATE TABLE pending_deletions (
                book_id TEXT PRIMARY KEY,
                trash_name TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            );
            "#,
        ),
    ])
}

pub fn configure_connection(connection: &Connection) -> AppResult<()> {
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| {
            tracing::error!(%error, "failed to set SQLite busy timeout");
            AppError::database()
        })?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| {
            tracing::error!(%error, "failed to enable SQLite foreign keys");
            AppError::database()
        })?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| {
            tracing::error!(%error, "failed to enable SQLite WAL");
            AppError::database()
        })?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| {
            tracing::error!(%error, "failed to enable durable SQLite synchronization");
            AppError::database()
        })?;
    Ok(())
}

pub fn open_database(path: &Path) -> AppResult<Connection> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            tracing::error!(path = %path.display(), "SQLite path is not a regular file");
            return Err(AppError::storage());
        }
    }
    let mut connection = Connection::open(path).map_err(|error| {
        tracing::error!(%error, "failed to open SQLite database");
        AppError::database()
    })?;
    configure_connection(&connection)?;
    migrations().to_latest(&mut connection).map_err(|error| {
        tracing::error!(%error, "failed to migrate SQLite database");
        AppError::database()
    })?;
    storage::set_private_permissions(path, false)?;
    for suffix in ["-wal", "-shm"] {
        let companion = path.with_file_name(format!(
            "{}{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("library.sqlite3"),
            suffix
        ));
        if companion.exists() {
            storage::set_private_permissions(&companion, false)?;
        }
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO settings(key, value) VALUES ('generation_model', ?1)",
            [DEFAULT_GENERATION_MODEL],
        )
        .and_then(|_| {
            connection.execute(
                "INSERT OR IGNORE INTO settings(key, value) VALUES ('embedding_model', ?1)",
                [DEFAULT_EMBEDDING_MODEL],
            )
        })
        .map_err(|error| {
            tracing::error!(%error, "failed to initialize settings");
            AppError::database()
        })?;
    Ok(connection)
}

fn nonnegative_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn nonnegative_u32(value: i64) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn bounded_count(value: i64, maximum: usize) -> AppResult<usize> {
    let value = usize::try_from(value).map_err(|_| AppError::database())?;
    if value > maximum {
        return Err(AppError::database());
    }
    Ok(value)
}

pub fn book_hash_exists(connection: &Connection, source_hash: &str) -> AppResult<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE source_hash = ?1)",
            [source_hash],
            |row| row.get(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to check duplicate EPUB");
            AppError::database()
        })
}

fn book_summary_from_row(row: &Row<'_>) -> rusqlite::Result<BookSummary> {
    let book_id: String = row.get(0)?;
    let current_location = nonnegative_u64(row.get(6)?)?;
    let progress = Progress {
        book_id: book_id.clone(),
        current_location,
        current_chapter_id: row.get(7)?,
        completion_percentage: row.get(8)?,
        last_read_at: row.get(9)?,
    };
    Ok(BookSummary {
        id: book_id,
        title: row.get(1)?,
        author: row.get(2)?,
        progress: Some(progress),
    })
}

pub fn list_books(connection: &Connection) -> AppResult<BookList> {
    let count = connection
        .query_row("SELECT COUNT(*) FROM books", [], |row| row.get::<_, i64>(0))
        .map_err(|error| {
            tracing::error!(%error, "failed to count books");
            AppError::database()
        })?;
    bounded_count(count, MAX_LIBRARY_BOOKS)?;
    let mut statement = connection
        .prepare(
            r#"
            SELECT b.id, b.title, NULLIF(b.author, ''), b.cover_rel_path, b.total_locations,
                   b.total_chapters, COALESCE(p.current_location, 0), p.current_chapter_id,
                   COALESCE(p.completion_percentage, 0.0), p.last_read_at
            FROM books b
            LEFT JOIN progress p ON p.book_id = b.id
            ORDER BY COALESCE(p.last_read_at, b.created_at) DESC, b.title COLLATE NOCASE
            "#,
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to prepare book list");
            AppError::database()
        })?;
    let books: Vec<BookSummary> = statement
        .query_map([], book_summary_from_row)
        .and_then(Iterator::collect)
        .map_err(|error| {
            tracing::error!(%error, "failed to read book list");
            AppError::database()
        })?;
    let total = books.len();
    Ok(BookList { books, total })
}

pub fn get_book(connection: &Connection, book_id: &str) -> AppResult<BookDetail> {
    let book = connection
        .query_row(
            r#"
            SELECT id, title, author, cover_rel_path, language, published_year, publisher,
                   isbn, description, content_length, total_locations, total_chapters
            FROM books WHERE id = ?1
            "#,
            [book_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?.is_some(),
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i32>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    nonnegative_u64(row.get(9)?)?,
                    nonnegative_u64(row.get(10)?)?,
                    nonnegative_u32(row.get(11)?)?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read book details");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Book not found."))?;

    if book.9 > MAX_COMPRESSED_BYTES
        || book.10 > (MAX_NORMALIZED_TEXT_CHARS + MAX_SPINE_CHAPTERS) as u64
        || usize::try_from(book.11).map_or(true, |count| count > MAX_SPINE_CHAPTERS)
    {
        tracing::error!(book_id, "book metadata exceeds persisted limits");
        return Err(AppError::database());
    }
    let chapter_count = connection
        .query_row(
            "SELECT COUNT(*) FROM chapters WHERE book_id = ?1",
            [book_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to count chapters");
            AppError::database()
        })?;
    let chapter_count = bounded_count(chapter_count, MAX_SPINE_CHAPTERS)?;
    if chapter_count != book.11 as usize {
        tracing::error!(book_id, "chapter count does not match book metadata");
        return Err(AppError::database());
    }

    let mut statement = connection
        .prepare(
            r#"
            SELECT id, title, chapter_order, start_location, end_location
            FROM chapters WHERE book_id = ?1 ORDER BY chapter_order
            "#,
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to prepare chapters");
            AppError::database()
        })?;
    let chapters: Vec<ChapterSummary> = statement
        .query_map([book_id], |row| {
            Ok(ChapterSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                order: nonnegative_u32(row.get(2)?)?,
                start_location: nonnegative_u64(row.get(3)?)?,
                end_location: nonnegative_u64(row.get(4)?)?,
            })
        })
        .and_then(Iterator::collect)
        .map_err(|error| {
            tracing::error!(%error, "failed to read chapters");
            AppError::database()
        })?;
    if chapters.len() != chapter_count {
        tracing::error!(
            book_id,
            "chapter count does not match bounded book metadata"
        );
        return Err(AppError::database());
    }
    let progress = get_progress(connection, book_id)?;

    Ok(BookDetail {
        id: book.0,
        title: book.1,
        author: Some(book.2),
        progress: Some(progress),
        has_cover: book.3,
        language: book.4,
        published_year: book.5,
        publisher: book.6,
        isbn: book.7,
        description: book.8,
        content_length: book.9,
        total_locations: book.10,
        total_chapters: book.11,
        chapters,
    })
}

pub fn get_progress(connection: &Connection, book_id: &str) -> AppResult<Progress> {
    connection
        .query_row(
            r#"
            SELECT book_id, current_location, current_chapter_id,
                   completion_percentage, last_read_at
            FROM progress WHERE book_id = ?1
            "#,
            [book_id],
            |row| {
                Ok(Progress {
                    book_id: row.get(0)?,
                    current_location: nonnegative_u64(row.get(1)?)?,
                    current_chapter_id: row.get(2)?,
                    completion_percentage: row.get(3)?,
                    last_read_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read progress");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Book progress not found."))
}

pub fn get_validated_progress(connection: &Connection, book_id: &str) -> AppResult<Progress> {
    let (progress, total_locations) = connection
        .query_row(
            r#"
            SELECT p.book_id, p.current_location, p.current_chapter_id,
                   p.completion_percentage, p.last_read_at, b.total_locations
            FROM progress p
            JOIN books b ON b.id = p.book_id
            WHERE p.book_id = ?1
            "#,
            [book_id],
            |row| {
                Ok((
                    Progress {
                        book_id: row.get(0)?,
                        current_location: nonnegative_u64(row.get(1)?)?,
                        current_chapter_id: row.get(2)?,
                        completion_percentage: row.get(3)?,
                        last_read_at: row.get(4)?,
                    },
                    nonnegative_u64(row.get(5)?)?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read progress for retrieval");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Book progress not found."))?;

    if total_locations > (MAX_NORMALIZED_TEXT_CHARS + MAX_SPINE_CHAPTERS) as u64
        || progress.current_location > total_locations
        || !progress.completion_percentage.is_finite()
        || !(0.0..=100.0).contains(&progress.completion_percentage)
    {
        tracing::error!(book_id, "stored progress is outside the book bounds");
        return Err(AppError::database());
    }
    let expected_percentage = if total_locations == 0 {
        0.0
    } else {
        progress.current_location as f64 / total_locations as f64 * 100.0
    };
    if (progress.completion_percentage - expected_percentage).abs() > 0.01 {
        tracing::error!(book_id, "stored progress percentage is inconsistent");
        return Err(AppError::database());
    }
    let chapter_id = progress.current_chapter_id.as_deref().ok_or_else(|| {
        tracing::error!(book_id, "stored progress has no chapter");
        AppError::database()
    })?;
    let location = i64::try_from(progress.current_location).map_err(|_| AppError::database())?;
    let total = i64::try_from(total_locations).map_err(|_| AppError::database())?;
    let chapter_valid = connection
        .query_row(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM chapters
                WHERE id = ?1 AND book_id = ?2 AND start_location <= ?3
                  AND start_location >= 0 AND end_location <= ?4
                  AND (end_location > ?3 OR (end_location = ?3 AND ?3 = ?4))
            )
            "#,
            params![chapter_id, book_id, location, total],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to validate progress chapter");
            AppError::database()
        })?;
    if !chapter_valid {
        tracing::error!(
            book_id,
            chapter_id,
            "stored progress chapter is inconsistent"
        );
        return Err(AppError::database());
    }
    Ok(progress)
}

pub fn get_cover_record(connection: &Connection, book_id: &str) -> AppResult<(String, String)> {
    let cover = connection
        .query_row(
            "SELECT cover_rel_path, cover_mime FROM books WHERE id = ?1",
            [book_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read cover record");
            AppError::database()
        })?;
    cover
        .and_then(|(path, mime)| path.zip(mime))
        .ok_or_else(|| AppError::not_found("Cover not found."))
}

pub fn get_chapter_record(
    connection: &Connection,
    book_id: &str,
    chapter_id: &str,
) -> AppResult<(ChapterContent, String)> {
    connection
        .query_row(
            r#"
            SELECT title, chapter_order, start_location, end_location, content_rel_path
            FROM chapters WHERE book_id = ?1 AND id = ?2
            "#,
            params![book_id, chapter_id],
            |row| {
                Ok((
                    ChapterContent {
                        book_id: book_id.to_owned(),
                        chapter_id: chapter_id.to_owned(),
                        title: row.get(0)?,
                        order: nonnegative_u32(row.get(1)?)?,
                        start_location: nonnegative_u64(row.get(2)?)?,
                        end_location: nonnegative_u64(row.get(3)?)?,
                        html: String::new(),
                    },
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read chapter record");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Chapter not found."))
}

pub fn update_progress(
    connection: &Connection,
    book_id: &str,
    current_location: u64,
    requested_chapter_id: Option<&str>,
) -> AppResult<Progress> {
    let total_locations = connection
        .query_row(
            "SELECT total_locations FROM books WHERE id = ?1",
            [book_id],
            |row| nonnegative_u64(row.get(0)?),
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to validate progress book");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Book not found."))?;

    if current_location > total_locations {
        return Err(AppError::invalid("Reading location is outside this book."));
    }
    let location = i64::try_from(current_location)
        .map_err(|_| AppError::invalid("Reading location is too large."))?;

    let chapter_id = if let Some(chapter_id) = requested_chapter_id {
        let valid = connection
            .query_row(
                r#"
                SELECT 1 FROM chapters
                WHERE id = ?1 AND book_id = ?2 AND start_location <= ?3
                  AND (end_location > ?3 OR (end_location = ?3 AND ?3 = ?4))
                "#,
                params![chapter_id, book_id, location, total_locations as i64],
                |_| Ok(true),
            )
            .optional()
            .map_err(|error| {
                tracing::error!(%error, "failed to validate progress chapter");
                AppError::database()
            })?
            .unwrap_or(false);
        if !valid {
            return Err(AppError::invalid(
                "The chapter does not contain the reading location.",
            ));
        }
        Some(chapter_id.to_owned())
    } else {
        connection
            .query_row(
                r#"
                SELECT id FROM chapters
                WHERE book_id = ?1 AND start_location <= ?2
                  AND (end_location > ?2 OR (end_location = ?2 AND ?2 = ?3))
                ORDER BY chapter_order DESC LIMIT 1
                "#,
                params![book_id, location, total_locations as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                tracing::error!(%error, "failed to infer progress chapter");
                AppError::database()
            })?
    };

    let completion_percentage = if total_locations == 0 {
        0.0
    } else {
        current_location as f64 / total_locations as f64 * 100.0
    };
    let last_read_at = Utc::now().to_rfc3339();
    let changed = connection
        .execute(
            r#"
            UPDATE progress
            SET current_location = ?2, current_chapter_id = ?3,
                completion_percentage = ?4, last_read_at = ?5
            WHERE book_id = ?1
            "#,
            params![
                book_id,
                location,
                chapter_id,
                completion_percentage,
                last_read_at
            ],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to update progress");
            AppError::database()
        })?;
    if changed != 1 {
        return Err(AppError::not_found("Book progress not found."));
    }

    get_progress(connection, book_id)
}

fn delete_book_rows_in(connection: &Connection, book_id: &str) -> AppResult<()> {
    let chunk_count = connection
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id = ?1",
            [book_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to count chunks for deletion");
            AppError::database()
        })?;
    bounded_count(chunk_count, MAX_CHUNKS_PER_BOOK)?;
    connection
        .execute(
            "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE book_id = ?1)",
            [book_id],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to delete FTS rows");
            AppError::database()
        })?;
    connection
        .execute("DELETE FROM books WHERE id = ?1", [book_id])
        .map_err(|error| {
            tracing::error!(%error, "failed to delete book row");
            AppError::database()
        })?;
    Ok(())
}

pub fn begin_book_deletion(
    connection: &mut Connection,
    book_id: &str,
    trash_name: &str,
) -> AppResult<()> {
    let transaction = connection.transaction().map_err(|_| AppError::database())?;
    let exists = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM books WHERE id = ?1)",
            [book_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| AppError::database())?;
    if !exists {
        return Err(AppError::not_found("Book not found."));
    }
    transaction
        .execute(
            "INSERT INTO pending_deletions(book_id, trash_name, created_at) VALUES (?1, ?2, ?3)",
            params![book_id, trash_name, Utc::now().to_rfc3339()],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to journal book deletion");
            AppError::database()
        })?;
    transaction.commit().map_err(|_| AppError::database())?;
    Ok(())
}

pub fn cancel_book_deletion(connection: &Connection, book_id: &str) -> AppResult<()> {
    connection
        .execute(
            "DELETE FROM pending_deletions WHERE book_id = ?1",
            [book_id],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to clear book deletion journal");
            AppError::database()
        })?;
    Ok(())
}

pub fn complete_book_deletion(connection: &mut Connection, book_id: &str) -> AppResult<()> {
    let transaction = connection.transaction().map_err(|_| AppError::database())?;
    delete_book_rows_in(&transaction, book_id)?;
    transaction
        .execute(
            "DELETE FROM pending_deletions WHERE book_id = ?1",
            [book_id],
        )
        .map_err(|_| AppError::database())?;
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to complete journaled book deletion");
        AppError::database()
    })?;
    Ok(())
}

pub fn pending_deletions(connection: &Connection) -> AppResult<Vec<(String, String)>> {
    let mut statement = connection
        .prepare("SELECT book_id, trash_name FROM pending_deletions ORDER BY created_at")
        .map_err(|_| AppError::database())?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .and_then(Iterator::collect)
        .map_err(|error| {
            tracing::error!(%error, "failed to read deletion journal");
            AppError::database()
        })
}

pub fn book_ids(connection: &Connection) -> AppResult<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT id FROM books ORDER BY id")
        .map_err(|_| AppError::database())?;
    statement
        .query_map([], |row| row.get(0))
        .and_then(Iterator::collect)
        .map_err(|_| AppError::database())
}

pub fn remove_missing_book(connection: &mut Connection, book_id: &str) -> AppResult<()> {
    let transaction = connection.transaction().map_err(|_| AppError::database())?;
    delete_book_rows_in(&transaction, book_id)?;
    transaction.commit().map_err(|_| AppError::database())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn migrated_connection() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();
        migrations().to_latest(&mut connection).unwrap();
        connection
    }

    #[test]
    fn migrations_create_required_tables_and_fts5() {
        let connection = migrated_connection();
        for table in [
            "books",
            "chapters",
            "progress",
            "chunks",
            "chunks_fts",
            "index_state",
            "settings",
            "pending_deletions",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing {table}");
        }
        let foreign_keys: bool = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(foreign_keys);
        for column in ["vector_dimension", "generation"] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('index_state') WHERE name = ?1)",
                    [column],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing index_state.{column}");
        }
    }

    fn insert_progress_fixture(connection: &Connection) {
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                r#"
                INSERT INTO books(
                    id, source_hash, title, author, source_rel_path, content_length,
                    total_locations, total_chapters, created_at, updated_at
                ) VALUES ('book', 'hash', 'Title', 'Author', 'books/book/source.epub', 1, 200, 2, ?1, ?1)
                "#,
                [&now],
            )
            .unwrap();
        for (id, order, start, end) in [
            ("chapter-1", 0_i64, 0_i64, 100_i64),
            ("chapter-2", 1_i64, 100_i64, 200_i64),
        ] {
            connection
                .execute(
                    r#"
                    INSERT INTO chapters(
                        id, book_id, title, chapter_order, content_rel_path,
                        start_location, end_location, char_count
                    ) VALUES (?1, 'book', ?1, ?2, 'chapter.html', ?3, ?4, 100)
                    "#,
                    params![id, order, start, end],
                )
                .unwrap();
        }
        connection
            .execute(
                r#"
                INSERT INTO progress(book_id, current_location, current_chapter_id, completion_percentage)
                VALUES ('book', 0, 'chapter-1', 0.0)
                "#,
                [],
            )
            .unwrap();
    }

    #[test]
    fn progress_rejects_out_of_range_and_mismatched_chapters() {
        let connection = migrated_connection();
        insert_progress_fixture(&connection);

        assert!(update_progress(&connection, "book", 201, None).is_err());
        assert!(update_progress(&connection, "book", 50, Some("chapter-2")).is_err());
        assert!(update_progress(&connection, "book", 100, Some("chapter-1")).is_err());

        let progress = update_progress(&connection, "book", 100, None).unwrap();
        assert_eq!(progress.current_chapter_id.as_deref(), Some("chapter-2"));
        assert_eq!(progress.completion_percentage, 50.0);

        let requested = update_progress(&connection, "book", 100, Some("chapter-2")).unwrap();
        assert_eq!(requested.current_chapter_id.as_deref(), Some("chapter-2"));

        let completed = update_progress(&connection, "book", 200, Some("chapter-2")).unwrap();
        assert_eq!(completed.completion_percentage, 100.0);
    }

    #[test]
    fn retrieval_progress_fails_closed_on_malformed_rows() {
        let connection = migrated_connection();
        insert_progress_fixture(&connection);
        assert!(get_validated_progress(&connection, "book").is_ok());

        connection
            .execute(
                "UPDATE progress SET current_location = 201, completion_percentage = 100.0 WHERE book_id = 'book'",
                [],
            )
            .unwrap();
        assert!(get_validated_progress(&connection, "book").is_err());

        connection
            .execute(
                "UPDATE progress SET current_location = 50, current_chapter_id = 'chapter-2', completion_percentage = 25.0 WHERE book_id = 'book'",
                [],
            )
            .unwrap();
        assert!(get_validated_progress(&connection, "book").is_err());

        connection
            .execute(
                "UPDATE progress SET current_chapter_id = 'chapter-1', completion_percentage = 90.0 WHERE book_id = 'book'",
                [],
            )
            .unwrap();
        assert!(get_validated_progress(&connection, "book").is_err());
    }

    #[test]
    fn deletion_journal_is_committed_with_book_removal() {
        let mut connection = migrated_connection();
        insert_progress_fixture(&connection);
        begin_book_deletion(
            &mut connection,
            "book",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        )
        .unwrap();
        assert_eq!(pending_deletions(&connection).unwrap().len(), 1);

        complete_book_deletion(&mut connection, "book").unwrap();

        assert!(pending_deletions(&connection).unwrap().is_empty());
        assert!(book_ids(&connection).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn database_open_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("mereader-db-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target.sqlite3");
        Connection::open(&target).unwrap();
        let link = root.join("library.sqlite3");
        symlink(&target, &link).unwrap();

        assert!(open_database(&link).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }
}
