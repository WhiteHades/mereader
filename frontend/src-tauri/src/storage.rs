use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub const BOOKS_DIR: &str = "books";
pub const STAGING_DIR: &str = "staging";
pub const TRASH_DIR: &str = "trash";
pub const LOGS_DIR: &str = "logs";

pub fn initialize(root: &Path) -> AppResult<PathBuf> {
    fs::create_dir_all(root).map_err(|error| {
        tracing::error!(%error, path = %root.display(), "failed to create app data directory");
        AppError::storage()
    })?;
    let root = fs::canonicalize(root).map_err(|error| {
        tracing::error!(%error, path = %root.display(), "failed to resolve app data directory");
        AppError::storage()
    })?;
    set_private_permissions(&root, true)?;
    for name in [BOOKS_DIR, STAGING_DIR, TRASH_DIR, LOGS_DIR] {
        create_and_validate_directory(&root, name)?;
    }
    harden_existing_tree(&root.join(BOOKS_DIR))?;
    cleanup_directory_best_effort(&root.join(STAGING_DIR));
    Ok(root)
}

pub fn validate_managed_directories(root: &Path) -> AppResult<()> {
    for name in [BOOKS_DIR, STAGING_DIR, TRASH_DIR] {
        validate_directory(root, name)?;
    }
    Ok(())
}

pub fn validate_logs_directory(root: &Path) -> AppResult<()> {
    validate_directory(root, LOGS_DIR)
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
    validate_directory(root, name)?;
    set_private_permissions(&path, true)
}

#[cfg(unix)]
pub fn set_private_permissions(path: &Path, directory: bool) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if directory { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to secure managed path permissions");
        AppError::storage()
    })
}

#[cfg(not(unix))]
pub fn set_private_permissions(_path: &Path, _directory: bool) -> AppResult<()> {
    Ok(())
}

fn harden_existing_tree(path: &Path) -> AppResult<()> {
    for entry in fs::read_dir(path).map_err(|error| {
        tracing::error!(%error, path = %path.display(), "failed to inspect managed permissions");
        AppError::storage()
    })? {
        let entry = entry.map_err(|_| AppError::storage())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| AppError::storage())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        set_private_permissions(&entry.path(), metadata.is_dir())?;
        if metadata.is_dir() {
            harden_existing_tree(&entry.path())?;
        }
    }
    Ok(())
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

pub fn cleanup_directory_best_effort(path: &Path) {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "managed cleanup will be retried later");
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => {
                if let Err(error) = remove_path(&entry.path()) {
                    tracing::warn!(kind = ?error.kind, path = %entry.path().display(), "managed cleanup entry will be retried later");
                }
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "managed cleanup entry could not be inspected");
            }
        }
    }
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

#[cfg(unix)]
pub fn sync_directory(path: &Path) -> AppResult<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            tracing::error!(%error, path = %path.display(), "failed to synchronize managed directory");
            AppError::storage()
        })
}

#[cfg(not(unix))]
pub fn sync_directory(_path: &Path) -> AppResult<()> {
    Ok(())
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
    fn initialization_cleans_staging_but_preserves_trash_for_recovery() {
        let root = temp_path();
        fs::create_dir_all(root.join(STAGING_DIR).join("stale")).unwrap();
        fs::write(root.join(STAGING_DIR).join("stale").join("file"), b"x").unwrap();
        fs::create_dir_all(root.join(TRASH_DIR).join("stale")).unwrap();

        let canonical = initialize(&root).unwrap();

        assert_eq!(
            fs::read_dir(canonical.join(STAGING_DIR)).unwrap().count(),
            0
        );
        assert_eq!(fs::read_dir(canonical.join(TRASH_DIR)).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_managed_directories() {
        use std::os::unix::fs::symlink;
        use std::os::unix::fs::PermissionsExt;

        let outside = temp_path();
        fs::create_dir_all(&outside).unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).unwrap();
        for name in [BOOKS_DIR, STAGING_DIR, TRASH_DIR, LOGS_DIR] {
            let root = temp_path();
            fs::create_dir_all(&root).unwrap();
            symlink(&outside, root.join(name)).unwrap();

            assert!(initialize(&root).is_err(), "accepted symlinked {name}");

            fs::remove_file(root.join(name)).unwrap();
            fs::remove_dir_all(root).unwrap();
            assert_eq!(
                fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn initialization_tightens_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_path();
        let book = root.join(BOOKS_DIR).join("existing");
        fs::create_dir_all(&book).unwrap();
        let file = book.join("chapter.html");
        fs::write(&file, b"private").unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&book, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();

        let root = initialize(&root).unwrap();

        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&book).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
