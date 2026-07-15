use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub const BOOKS_DIR: &str = "books";
pub const STAGING_DIR: &str = "staging";
pub const TRASH_DIR: &str = "trash";

pub fn initialize(root: &Path) -> AppResult<PathBuf> {
    fs::create_dir_all(root).map_err(|error| {
        tracing::error!(%error, path = %root.display(), "failed to create app data directory");
        AppError::storage()
    })?;
    let root = fs::canonicalize(root).map_err(|error| {
        tracing::error!(%error, path = %root.display(), "failed to resolve app data directory");
        AppError::storage()
    })?;
    for name in [BOOKS_DIR, STAGING_DIR, TRASH_DIR] {
        create_and_validate_directory(&root, name)?;
    }
    cleanup_directory(&root.join(STAGING_DIR))?;
    cleanup_directory(&root.join(TRASH_DIR))?;
    Ok(root)
}

pub fn validate_managed_directories(root: &Path) -> AppResult<()> {
    for name in [BOOKS_DIR, STAGING_DIR, TRASH_DIR] {
        validate_directory(root, name)?;
    }
    Ok(())
}

fn create_and_validate_directory(root: &Path, name: &str) -> AppResult<()> {
    let path = root.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            tracing::error!(%error, path = %path.display(), "failed to create managed directory");
            return Err(AppError::storage());
        }
    }
    validate_directory(root, name)
}

fn validate_directory(root: &Path, name: &str) -> AppResult<()> {
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to inspect managed directory");
        AppError::storage()
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        tracing::error!(path = %path.display(), "managed path is not a real directory");
        return Err(AppError::storage());
    }
    let canonical = fs::canonicalize(&path).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to resolve managed directory");
        AppError::storage()
    })?;
    if canonical != path {
        tracing::error!(path = %path.display(), resolved = %canonical.display(), "managed directory escaped app data root");
        return Err(AppError::storage());
    }
    Ok(())
}

pub fn cleanup_directory(path: &Path) -> AppResult<()> {
    let entries = fs::read_dir(path).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to read cleanup directory");
        AppError::storage()
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            tracing::error!(%error, path = %path.display(), "failed to read cleanup entry");
            AppError::storage()
        })?;
        remove_path(&entry.path())?;
    }
    Ok(())
}

pub fn remove_path(path: &Path) -> AppResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            tracing::error!(%error, path = %path.display(), "failed to inspect cleanup path");
            return Err(AppError::storage());
        }
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to remove managed path");
        AppError::storage()
    })
}

pub fn validated_uuid(value: &str, message: &str) -> AppResult<String> {
    Uuid::parse_str(value)
        .map(|value| value.to_string())
        .map_err(|_| AppError::invalid(message))
}

pub fn book_path(root: &Path, book_id: &str) -> AppResult<PathBuf> {
    let book_id = validated_uuid(book_id, "The book identifier is invalid.")?;
    Ok(root.join(BOOKS_DIR).join(book_id))
}

pub fn validated_book_path(root: &Path, book_id: &str) -> AppResult<PathBuf> {
    validate_managed_directories(root)?;
    let path = book_path(root, book_id)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| AppError::not_found("Book files not found."))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::storage());
    }
    Ok(path)
}

pub fn chapter_path(root: &Path, book_id: &str, chapter_id: &str) -> AppResult<PathBuf> {
    let book_dir = validated_book_path(root, book_id)?;
    let chapter_id = validated_uuid(chapter_id, "The chapter identifier is invalid.")?;
    Ok(book_dir.join(format!("chapter-{chapter_id}.html")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!("mereader-storage-{}", Uuid::new_v4()))
    }

    #[test]
    fn initialization_cleans_staging_and_trash() {
        let root = temp_path();
        fs::create_dir_all(root.join(STAGING_DIR).join("stale")).unwrap();
        fs::write(root.join(STAGING_DIR).join("stale").join("file"), b"x").unwrap();
        fs::create_dir_all(root.join(TRASH_DIR).join("stale")).unwrap();

        let canonical = initialize(&root).unwrap();

        assert_eq!(
            fs::read_dir(canonical.join(STAGING_DIR)).unwrap().count(),
            0
        );
        assert_eq!(fs::read_dir(canonical.join(TRASH_DIR)).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_managed_directories() {
        use std::os::unix::fs::symlink;

        let outside = temp_path();
        fs::create_dir_all(&outside).unwrap();
        for name in [BOOKS_DIR, STAGING_DIR, TRASH_DIR] {
            let root = temp_path();
            fs::create_dir_all(&root).unwrap();
            symlink(&outside, root.join(name)).unwrap();

            assert!(initialize(&root).is_err(), "accepted symlinked {name}");

            fs::remove_file(root.join(name)).unwrap();
            fs::remove_dir_all(root).unwrap();
        }
        fs::remove_dir_all(outside).unwrap();
    }
}
