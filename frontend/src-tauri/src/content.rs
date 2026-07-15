use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ammonia::{Builder, UrlRelative};
use chrono::Utc;
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use rbook::epub::reader::LinearBehavior;
use rbook::Epub;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive};

use crate::error::{AppError, AppErrorKind, AppResult};
use crate::models::{BookSummary, ImportedBook, IndexedChunk};

const MAX_COMPRESSED_BYTES: u64 = 100 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 500 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 100 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_COMPRESSION_RATIO: u64 = 100;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const CHUNK_SIZE: usize = 400;
const CHUNK_OVERLAP: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub order: u32,
    pub text: String,
    pub start_location: u64,
    pub end_location: u64,
}

struct ParsedChapter {
    id: String,
    title: String,
    order: u32,
    text: String,
    start_location: u64,
    end_location: u64,
    rel_path: String,
}

struct ParsedBook {
    id: String,
    source_hash: String,
    title: String,
    author: String,
    source_rel_path: String,
    cover_rel_path: Option<String>,
    cover_mime: Option<String>,
    language: Option<String>,
    published_year: Option<i32>,
    publisher: Option<String>,
    isbn: Option<String>,
    description: Option<String>,
    content_length: u64,
    total_locations: u64,
    chapters: Vec<ParsedChapter>,
}

pub fn validate_epub_path(path: &Path) -> AppResult<PathBuf> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        != Some("epub".to_owned())
    {
        return Err(AppError::invalid("Only .epub files can be imported."));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|_| AppError::invalid("The selected EPUB file could not be opened."))?;
    if !canonical.is_file() {
        return Err(AppError::invalid("The selected path is not a file."));
    }
    Ok(canonical)
}

pub fn preflight_epub(path: &Path) -> AppResult<()> {
    let metadata = fs::metadata(path)
        .map_err(|_| AppError::invalid("The selected EPUB file could not be inspected."))?;
    if metadata.len() == 0 || metadata.len() > MAX_COMPRESSED_BYTES {
        return Err(AppError::invalid_epub(
            "The EPUB compressed size is outside the supported range.",
        ));
    }

    let file = File::open(path).map_err(|_| AppError::storage())?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| AppError::invalid_epub("The selected file is not a valid EPUB archive."))?;
    if archive.is_empty() || archive.len() > MAX_ENTRIES {
        return Err(AppError::invalid_epub(
            "The EPUB contains an unsupported number of entries.",
        ));
    }

    {
        let mimetype = archive
            .by_index(0)
            .map_err(|_| AppError::invalid_epub("The EPUB mimetype entry is missing."))?;
        if mimetype.name() != "mimetype" || mimetype.compression() != CompressionMethod::Stored {
            return Err(AppError::invalid_epub(
                "The EPUB mimetype entry must be first and uncompressed.",
            ));
        }
        let mut value = String::new();
        mimetype
            .take(64)
            .read_to_string(&mut value)
            .map_err(|_| AppError::invalid_epub("The EPUB mimetype entry is invalid."))?;
        if value != "application/epub+zip" {
            return Err(AppError::invalid_epub(
                "The archive does not declare the EPUB mimetype.",
            ));
        }
    }

    let mut expanded_bytes = 0_u64;
    let mut compressed_bytes = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| AppError::invalid_epub("An EPUB archive entry is invalid."))?;
        let name = entry.name();
        if entry.encrypted()
            || entry.is_symlink()
            || entry.enclosed_name().is_none()
            || is_suspicious_archive_name(name)
        {
            return Err(AppError::invalid_epub(
                "The EPUB contains an unsafe or encrypted archive entry.",
            ));
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(AppError::invalid_epub(
                "An EPUB entry is larger than the supported limit.",
            ));
        }
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| AppError::invalid_epub("The EPUB expanded size is invalid."))?;
        compressed_bytes = compressed_bytes
            .checked_add(entry.compressed_size())
            .ok_or_else(|| AppError::invalid_epub("The EPUB compressed size is invalid."))?;
        if expanded_bytes > MAX_EXPANDED_BYTES {
            return Err(AppError::invalid_epub(
                "The EPUB expands beyond the supported size limit.",
            ));
        }
    }
    let maximum_expansion = compressed_bytes.checked_mul(MAX_COMPRESSION_RATIO);
    if expanded_bytes > 0
        && (compressed_bytes == 0 || maximum_expansion.is_none_or(|limit| expanded_bytes > limit))
    {
        return Err(AppError::invalid_epub(
            "The EPUB compression ratio is suspicious.",
        ));
    }
    Ok(())
}

fn is_suspicious_archive_name(name: &str) -> bool {
    name.is_empty()
        || name.len() > 512
        || name.contains('\0')
        || name.contains('\\')
        || name.starts_with('/')
        || name.split('/').count() > 32
        || name
            .split('/')
            .any(|component| component == ".." || component.contains(':'))
}

pub fn chunk_text(text: &str, chapter_start: u64) -> Vec<TextChunk> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    while start < chars.len() {
        let end = (start + CHUNK_SIZE).min(chars.len());
        chunks.push(TextChunk {
            order: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
            text: chars[start..end].iter().collect(),
            start_location: chapter_start + start as u64,
            end_location: chapter_start + end as u64,
        });
        if end == chars.len() {
            break;
        }
        start = end - CHUNK_OVERLAP;
    }
    chunks
}

pub fn extract_plain_text(html: &str) -> String {
    let wrapped = format!("<root>{html}</root>");
    let mut reader = XmlReader::from_str(&wrapped);
    reader.config_mut().check_end_names = false;
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(value)) => {
                if let Ok(decoded) = value.html_content() {
                    match quick_xml::escape::unescape(&decoded) {
                        Ok(unescaped) => text.push_str(&unescaped),
                        Err(_) => text.push_str(&decoded),
                    }
                    text.push(' ');
                }
            }
            Ok(Event::CData(value)) => {
                if let Ok(decoded) = value.decode() {
                    text.push_str(&decoded);
                    text.push(' ');
                }
            }
            Ok(Event::GeneralRef(value)) => {
                if let Ok(name) = value.decode() {
                    let reference = format!("&{name};");
                    if let Ok(unescaped) = quick_xml::escape::unescape(&reference) {
                        text.push_str(&unescaped);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                tracing::warn!(%error, "sanitized HTML text extraction stopped early");
                break;
            }
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitized_html(raw_html: &str, image_names: &HashMap<String, String>) -> String {
    let tags = [
        "a",
        "abbr",
        "article",
        "aside",
        "b",
        "bdi",
        "bdo",
        "blockquote",
        "br",
        "caption",
        "cite",
        "code",
        "col",
        "colgroup",
        "dd",
        "del",
        "details",
        "dfn",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "footer",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hr",
        "i",
        "img",
        "ins",
        "kbd",
        "li",
        "main",
        "mark",
        "nav",
        "ol",
        "p",
        "pre",
        "q",
        "rp",
        "rt",
        "ruby",
        "s",
        "samp",
        "section",
        "small",
        "span",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "time",
        "tr",
        "u",
        "ul",
        "var",
        "wbr",
    ]
    .into_iter()
    .collect();
    let clean_content_tags = [
        "script", "style", "form", "iframe", "frame", "frameset", "object", "embed", "applet",
        "template", "noscript",
    ]
    .into_iter()
    .collect();
    let tag_attributes = HashMap::from([
        ("a", ["href", "title"].into_iter().collect()),
        (
            "img",
            ["src", "alt", "title", "width", "height"]
                .into_iter()
                .collect(),
        ),
        ("ol", ["start"].into_iter().collect()),
        ("li", ["value"].into_iter().collect()),
        (
            "td",
            ["colspan", "rowspan", "headers"].into_iter().collect(),
        ),
        (
            "th",
            ["colspan", "rowspan", "headers", "scope"]
                .into_iter()
                .collect(),
        ),
        ("time", ["datetime"].into_iter().collect()),
    ]);
    let generic_attributes = ["dir", "lang", "title"].into_iter().collect();
    let images = image_names.clone();
    let mut builder = Builder::new();
    builder
        .tags(tags)
        .clean_content_tags(clean_content_tags)
        .tag_attributes(tag_attributes)
        .generic_attributes(generic_attributes)
        .url_schemes(HashSet::new())
        .url_relative(UrlRelative::PassThrough)
        .link_rel(None)
        .attribute_filter(
            move |element, attribute, value| match (element, attribute) {
                ("a", "href") if value.starts_with('#') => Some(Cow::Borrowed(value)),
                ("a", "href") => None,
                ("img", "src") => {
                    let local = value.split(['?', '#']).next().unwrap_or_default();
                    let basename = local.rsplit('/').next().unwrap_or_default();
                    images
                        .get(basename)
                        .map(|name| Cow::Owned(format!("assets/{name}")))
                }
                _ => Some(Cow::Borrowed(value)),
            },
        );
    builder.clean(raw_html).to_string()
}

fn first_heading(html: &str, fallback_order: u32) -> String {
    for tag in ["h1", "h2", "h3", "h4", "h5", "h6"] {
        let open = format!("<{tag}");
        if let Some(start) = html.find(&open) {
            if let Some(open_end) = html[start..].find('>') {
                let content_start = start + open_end + 1;
                let close = format!("</{tag}>");
                if let Some(end) = html[content_start..].find(&close) {
                    let heading = extract_plain_text(&html[content_start..content_start + end]);
                    if !heading.is_empty() {
                        return heading.chars().take(200).collect();
                    }
                }
            }
        }
    }
    format!("Chapter {}", fallback_order + 1)
}

fn image_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

fn has_safe_image_signature(mime: &str, bytes: &[u8]) -> bool {
    match mime {
        "image/jpeg" | "image/jpg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        "image/avif" => bytes.len() >= 12 && &bytes[4..8] == b"ftyp",
        _ => false,
    }
}

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path).map_err(|_| AppError::storage())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| AppError::storage())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn parse_epub(root: &Path, canonical_source: &Path) -> AppResult<ParsedBook> {
    let source_hash = sha256_file(canonical_source)?;
    let id = Uuid::new_v4().to_string();
    let book_rel_dir = format!("books/{id}");
    let book_dir = root.join(&book_rel_dir);
    fs::create_dir_all(book_dir.join("assets")).map_err(|_| AppError::storage())?;
    let source_rel_path = format!("{book_rel_dir}/source.epub");
    let stored_source = root.join(&source_rel_path);
    fs::copy(canonical_source, &stored_source).map_err(|_| AppError::storage())?;

    let parse_result = (|| {
        let epub = Epub::options()
            .strict(false)
            .open(&stored_source)
            .map_err(|error| {
                tracing::error!(%error, "rbook failed to parse EPUB");
                AppError::invalid_epub("The EPUB structure could not be parsed.")
            })?;
        let metadata = epub.metadata();
        let title = metadata
            .title()
            .map(|entry| entry.value().trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Unknown Title".to_owned());
        let author = metadata
            .creators()
            .map(|entry| entry.value().trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let author = if author.is_empty() {
            "Unknown Author".to_owned()
        } else {
            author
        };
        let language = metadata.language().map(|entry| entry.value().to_owned());
        let publisher = metadata
            .publishers()
            .next()
            .map(|entry| entry.value().trim().to_owned())
            .filter(|value| !value.is_empty());
        let description = metadata
            .description()
            .map(|entry| entry.value().trim().to_owned())
            .filter(|value| !value.is_empty());
        let isbn = metadata
            .identifiers()
            .next()
            .map(|entry| entry.value().trim().to_owned())
            .filter(|value| !value.is_empty());
        let published_year = metadata
            .published()
            .map(|value| i32::from(value.date().year()));

        let cover_href = epub
            .manifest()
            .cover_image()
            .map(|entry| entry.href().as_ref().to_owned());
        let mut image_names = HashMap::new();
        let mut duplicate_names = HashSet::new();
        let mut cover_rel_path = None;
        let mut cover_mime = None;
        for image in epub.manifest().images() {
            let mime = image.media_type();
            let Some(extension) = image_extension(mime) else {
                continue;
            };
            let bytes = match image.read_bytes() {
                Ok(bytes) if bytes.len() <= MAX_IMAGE_BYTES => bytes,
                _ => continue,
            };
            if !has_safe_image_signature(mime, &bytes) {
                continue;
            }
            let href = image.href();
            let basename = href.name().decode().into_owned();
            let asset_name = format!("{}.{}", Uuid::new_v4(), extension);
            let asset_rel_path = format!("{book_rel_dir}/assets/{asset_name}");
            let mut file =
                File::create(root.join(&asset_rel_path)).map_err(|_| AppError::storage())?;
            file.write_all(&bytes).map_err(|_| AppError::storage())?;

            if image_names.insert(basename.clone(), asset_name).is_some() {
                duplicate_names.insert(basename);
            }
            if cover_href.as_deref() == Some(href.as_ref()) {
                cover_rel_path = Some(asset_rel_path);
                cover_mime = Some(mime.to_owned());
            }
        }
        for duplicate in duplicate_names {
            image_names.remove(&duplicate);
        }

        let mut chapters = Vec::new();
        let mut next_location = 0_u64;
        let reader = epub
            .reader_builder()
            .linear_behavior(LinearBehavior::LinearOnly)
            .create();
        for (order, result) in reader.enumerate() {
            let content = result.map_err(|error| {
                tracing::error!(%error, "failed to read EPUB spine entry");
                AppError::invalid_epub("An EPUB chapter could not be read.")
            })?;
            let html = sanitized_html(content.content(), &image_names);
            let text = extract_plain_text(&html);
            let char_count = text.chars().count() as u64;
            let end_location = next_location
                .checked_add(char_count)
                .ok_or_else(|| AppError::invalid_epub("The EPUB text is too large."))?;
            let chapter_id = Uuid::new_v4().to_string();
            let rel_path = format!("{book_rel_dir}/chapter-{chapter_id}.html");
            let order = u32::try_from(order)
                .map_err(|_| AppError::invalid_epub("The EPUB has too many chapters."))?;
            fs::write(root.join(&rel_path), html.as_bytes()).map_err(|_| AppError::storage())?;
            chapters.push(ParsedChapter {
                id: chapter_id,
                title: first_heading(&html, order),
                order,
                text,
                start_location: next_location,
                end_location,
                rel_path,
            });
            next_location = end_location;
        }
        if chapters.is_empty() {
            return Err(AppError::invalid_epub(
                "The EPUB does not contain readable chapters.",
            ));
        }

        Ok(ParsedBook {
            id,
            source_hash,
            title,
            author,
            source_rel_path,
            cover_rel_path,
            cover_mime,
            language,
            published_year,
            publisher,
            isbn,
            description,
            content_length: fs::metadata(canonical_source)
                .map_err(|_| AppError::storage())?
                .len(),
            total_locations: next_location,
            chapters,
        })
    })();

    if parse_result.is_err() {
        let _ = fs::remove_dir_all(book_dir);
    }
    parse_result
}

fn persist_book(connection: &mut Connection, parsed: &ParsedBook) -> AppResult<ImportedBook> {
    let now = Utc::now().to_rfc3339();
    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin import transaction");
        AppError::database()
    })?;
    transaction
        .execute(
            r#"
            INSERT INTO books(
                id, source_hash, title, author, source_rel_path, cover_rel_path, cover_mime,
                language, published_year, publisher, isbn, description, content_length,
                total_locations, total_chapters, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
            "#,
            params![
                parsed.id,
                parsed.source_hash,
                parsed.title,
                parsed.author,
                parsed.source_rel_path,
                parsed.cover_rel_path,
                parsed.cover_mime,
                parsed.language,
                parsed.published_year,
                parsed.publisher,
                parsed.isbn,
                parsed.description,
                parsed.content_length as i64,
                parsed.total_locations as i64,
                parsed.chapters.len() as i64,
                now,
            ],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to insert imported book");
            AppError::database()
        })?;

    let mut indexed_chunks = Vec::new();
    for chapter in &parsed.chapters {
        transaction
            .execute(
                r#"
                INSERT INTO chapters(
                    id, book_id, title, chapter_order, content_rel_path,
                    start_location, end_location, char_count
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    chapter.id,
                    parsed.id,
                    chapter.title,
                    chapter.order,
                    chapter.rel_path,
                    chapter.start_location as i64,
                    chapter.end_location as i64,
                    chapter.text.chars().count() as i64,
                ],
            )
            .map_err(|error| {
                tracing::error!(%error, "failed to insert chapter");
                AppError::database()
            })?;

        for chunk in chunk_text(&chapter.text, chapter.start_location) {
            transaction
                .execute(
                    r#"
                    INSERT INTO chunks(
                        book_id, chapter_id, chunk_order, text, start_location, end_location
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    "#,
                    params![
                        parsed.id,
                        chapter.id,
                        chunk.order,
                        chunk.text,
                        chunk.start_location as i64,
                        chunk.end_location as i64,
                    ],
                )
                .map_err(|error| {
                    tracing::error!(%error, "failed to insert text chunk");
                    AppError::database()
                })?;
            let row_id = transaction.last_insert_rowid();
            transaction
                .execute(
                    "INSERT INTO chunks_fts(rowid, text) VALUES (?1, ?2)",
                    params![row_id, chunk.text],
                )
                .map_err(|error| {
                    tracing::error!(%error, "failed to insert FTS chunk");
                    AppError::database()
                })?;
            indexed_chunks.push(IndexedChunk {
                id: row_id,
                text: chunk.text,
            });
        }
    }
    let first_chapter_id = parsed.chapters.first().map(|chapter| chapter.id.clone());
    transaction
        .execute(
            r#"
            INSERT INTO progress(
                book_id, current_location, current_chapter_id, completion_percentage, last_read_at
            ) VALUES (?1, 0, ?2, 0.0, NULL)
            "#,
            params![parsed.id, first_chapter_id],
        )
        .and_then(|_| {
            transaction.execute(
                r#"
                INSERT INTO index_state(book_id, status, vector_count, updated_at)
                VALUES (?1, 'text_ready', 0, ?2)
                "#,
                params![parsed.id, now],
            )
        })
        .map_err(|error| {
            tracing::error!(%error, "failed to initialize imported book state");
            AppError::database()
        })?;
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to commit imported book");
        AppError::database()
    })?;

    Ok(ImportedBook {
        summary: BookSummary {
            id: parsed.id.clone(),
            title: parsed.title.clone(),
            author: Some(parsed.author.clone()),
            progress: Some(crate::models::Progress {
                book_id: parsed.id.clone(),
                current_location: 0,
                current_chapter_id: first_chapter_id,
                completion_percentage: 0.0,
                last_read_at: None,
            }),
        },
        chunks: indexed_chunks,
    })
}

pub fn import_book(
    root: &Path,
    connection: &mut Connection,
    path: &Path,
) -> AppResult<ImportedBook> {
    let canonical = validate_epub_path(path)?;
    preflight_epub(&canonical)?;
    let source_hash = sha256_file(&canonical)?;
    let duplicate = connection
        .query_row(
            "SELECT 1 FROM books WHERE source_hash = ?1",
            [&source_hash],
            |_| Ok(true),
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to check duplicate EPUB");
            AppError::database()
        })?
        .unwrap_or(false);
    if duplicate {
        return Err(AppError::new(
            AppErrorKind::AlreadyExists,
            "This EPUB is already in the library.",
        ));
    }

    let parsed = parse_epub(root, &canonical)?;
    let book_dir = root.join("books").join(&parsed.id);
    match persist_book(connection, &parsed) {
        Ok(imported) => Ok(imported),
        Err(error) => {
            let _ = fs::remove_dir_all(book_dir);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mereader-{}-{name}", Uuid::new_v4()))
    }

    fn write_test_zip(path: &Path, extra_name: Option<&str>) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file(
                "mimetype",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"application/epub+zip").unwrap();
        writer
            .start_file(
                extra_name.unwrap_or("META-INF/container.xml"),
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(b"<container />").unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn validates_path_extension_and_zip_preflight() {
        let epub_path = temp_path("valid.epub");
        write_test_zip(&epub_path, None);
        let canonical = validate_epub_path(&epub_path).unwrap();
        preflight_epub(&canonical).unwrap();

        let wrong_extension = temp_path("invalid.zip");
        fs::copy(&epub_path, &wrong_extension).unwrap();
        assert!(validate_epub_path(&wrong_extension).is_err());

        fs::remove_file(epub_path).unwrap();
        fs::remove_file(wrong_extension).unwrap();
    }

    #[test]
    fn rejects_suspicious_zip_entry_names() {
        let epub_path = temp_path("unsafe.epub");
        write_test_zip(&epub_path, Some("../escape.xhtml"));
        assert!(preflight_epub(&epub_path).is_err());
        fs::remove_file(epub_path).unwrap();
    }

    #[test]
    fn chunk_locations_are_monotonic_with_overlap() {
        let text: String = (0..950)
            .map(|index| char::from(b'a' + (index % 26) as u8))
            .collect();
        let chunks = chunk_text(&text, 1_000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(
            (chunks[0].start_location, chunks[0].end_location),
            (1_000, 1_400)
        );
        assert_eq!(
            (chunks[1].start_location, chunks[1].end_location),
            (1_300, 1_700)
        );
        assert_eq!(
            (chunks[2].start_location, chunks[2].end_location),
            (1_600, 1_950)
        );
        assert!(chunks.windows(2).all(|pair| {
            pair[0].start_location < pair[1].start_location
                && pair[0].end_location < pair[1].end_location
        }));
    }

    #[test]
    fn sanitization_removes_active_and_remote_content() {
        let images = HashMap::from([("safe.png".to_owned(), "generated.png".to_owned())]);
        let html = sanitized_html(
            r#"<p onclick="bad()">Hello<script>bad()</script></p>
               <form><input value="secret"></form>
               <img src="https://example.com/tracker.png">
               <img src="../images/safe.png" onerror="bad()">"#,
            &images,
        );
        assert!(!html.contains("script"));
        assert!(!html.contains("form"));
        assert!(!html.contains("onclick"));
        assert!(!html.contains("onerror"));
        assert!(!html.contains("https://"));
        assert!(html.contains("assets/generated.png"));
        assert_eq!(extract_plain_text(&html), "Hello");
    }

    #[test]
    fn plain_text_offsets_count_unicode_characters() {
        let html = "<p>A &amp; B</p><p>naive cafe</p><p>\u{1f642}</p>";
        let text = extract_plain_text(html);
        assert_eq!(text, "A & B naive cafe \u{1f642}");
        let chunks = chunk_text(&text, 0);
        assert_eq!(chunks[0].end_location, text.chars().count() as u64);
    }
}
