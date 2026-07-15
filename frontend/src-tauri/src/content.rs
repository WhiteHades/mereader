use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};

use ammonia::{Builder, UrlRelative};
use chrono::Utc;
use image::{ImageFormat, ImageReader, Limits};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use rbook::epub::reader::LinearBehavior;
use rbook::Epub;
use rusqlite::{params, Connection, ErrorCode};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive};

use crate::error::{AppError, AppErrorKind, AppResult};
use crate::limits::{
    MAX_ARCHIVE_ENTRIES, MAX_ASSETS, MAX_CHUNKS_PER_BOOK, MAX_COMPRESSED_BYTES,
    MAX_COMPRESSION_RATIO, MAX_ENTRY_BYTES, MAX_EXPANDED_BYTES, MAX_GENERATED_BYTES,
    MAX_IMAGE_BYTES, MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS, MAX_LIBRARY_BOOKS,
    MAX_NORMALIZED_TEXT_CHARS, MAX_SPINE_CHAPTERS,
};
use crate::models::BookSummary;
use crate::storage;

const CHUNK_SIZE: usize = 400;
const CHUNK_OVERLAP: usize = 100;

#[derive(Clone, Copy)]
struct ArchiveLimits {
    compressed_bytes: u64,
    expanded_bytes: u64,
    entry_bytes: u64,
    entries: usize,
    compression_ratio: u64,
}

const ARCHIVE_LIMITS: ArchiveLimits = ArchiveLimits {
    compressed_bytes: MAX_COMPRESSED_BYTES,
    expanded_bytes: MAX_EXPANDED_BYTES,
    entry_bytes: MAX_ENTRY_BYTES,
    entries: MAX_ARCHIVE_ENTRIES,
    compression_ratio: MAX_COMPRESSION_RATIO,
};

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

pub struct StagedEpub {
    path: Option<PathBuf>,
    pub source_hash: String,
    content_length: u64,
}

impl Drop for StagedEpub {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = storage::remove_path(&path);
        }
    }
}

pub struct PreparedImport {
    parsed: Option<ParsedBook>,
    staging_dir: Option<PathBuf>,
}

impl Drop for PreparedImport {
    fn drop(&mut self) {
        if let Some(path) = self.staging_dir.take() {
            let _ = storage::remove_path(&path);
        }
    }
}

pub struct PublishedImport {
    parsed: ParsedBook,
    root: PathBuf,
    final_dir: Option<PathBuf>,
}

impl Drop for PublishedImport {
    fn drop(&mut self) {
        if let Some(final_dir) = self.final_dir.take() {
            let rollback_path = self
                .root
                .join(storage::TRASH_DIR)
                .join(Uuid::new_v4().to_string());
            if fs::rename(&final_dir, &rollback_path).is_ok() {
                let _ = storage::remove_path(&rollback_path);
            }
        }
    }
}

#[derive(Default)]
struct BookBudget {
    chapters: usize,
    assets: usize,
    text_chars: usize,
    chunks: usize,
    generated_bytes: u64,
}

impl BookBudget {
    fn add_chapter(&mut self, text_chars: usize, generated_bytes: usize) -> AppResult<()> {
        self.chapters = self
            .chapters
            .checked_add(1)
            .ok_or_else(|| AppError::invalid_epub("The EPUB has too many chapters."))?;
        self.text_chars = self
            .text_chars
            .checked_add(text_chars)
            .ok_or_else(|| AppError::invalid_epub("The EPUB text is too large."))?;
        self.chunks = self
            .chunks
            .checked_add(chunk_count(text_chars))
            .ok_or_else(|| AppError::invalid_epub("The EPUB has too many text chunks."))?;
        self.add_generated(
            u64::try_from(generated_bytes)
                .map_err(|_| AppError::invalid_epub("The generated book data is too large."))?,
        )?;
        if self.chapters > MAX_SPINE_CHAPTERS {
            return Err(AppError::invalid_epub("The EPUB has too many chapters."));
        }
        if self.text_chars > MAX_NORMALIZED_TEXT_CHARS {
            return Err(AppError::invalid_epub(
                "The EPUB contains too much normalized text.",
            ));
        }
        if self.chunks > MAX_CHUNKS_PER_BOOK {
            return Err(AppError::invalid_epub(
                "The EPUB produces too many text chunks.",
            ));
        }
        Ok(())
    }

    fn set_assets(&mut self, assets: usize) -> AppResult<()> {
        self.assets = assets;
        if self.assets > MAX_ASSETS {
            return Err(AppError::invalid_epub("The EPUB has too many assets."));
        }
        Ok(())
    }

    fn add_generated(&mut self, bytes: u64) -> AppResult<()> {
        self.generated_bytes = self
            .generated_bytes
            .checked_add(bytes)
            .ok_or_else(|| AppError::invalid_epub("The generated book data is too large."))?;
        if self.generated_bytes > MAX_GENERATED_BYTES {
            return Err(AppError::invalid_epub(
                "The generated book data exceeds the supported limit.",
            ));
        }
        Ok(())
    }
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
    Ok(path.to_path_buf())
}

pub fn preflight_epub(path: &Path) -> AppResult<()> {
    preflight_epub_with_limits(path, ARCHIVE_LIMITS)
}

fn preflight_epub_with_limits(path: &Path, limits: ArchiveLimits) -> AppResult<()> {
    let metadata = fs::metadata(path)
        .map_err(|_| AppError::invalid("The selected EPUB file could not be inspected."))?;
    if metadata.len() == 0 || metadata.len() > limits.compressed_bytes {
        return Err(AppError::invalid_epub(
            "The EPUB compressed size is outside the supported range.",
        ));
    }

    let file = File::open(path).map_err(|_| AppError::storage())?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| AppError::invalid_epub("The selected file is not a valid EPUB archive."))?;
    if archive.is_empty() || archive.len() > limits.entries {
        return Err(AppError::invalid_epub(
            "The EPUB contains an unsupported number of entries.",
        ));
    }

    let mut expanded_bytes = 0_u64;
    let mut compressed_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| AppError::invalid_epub("An EPUB archive entry is invalid."))?;
        let name = entry.name().to_owned();
        if entry.encrypted()
            || entry.is_symlink()
            || entry.enclosed_name().is_none()
            || is_suspicious_archive_name(&name)
        {
            return Err(AppError::invalid_epub(
                "The EPUB contains an unsafe or encrypted archive entry.",
            ));
        }
        if entry.size() > limits.entry_bytes || entry.compressed_size() > limits.compressed_bytes {
            return Err(AppError::invalid_epub(
                "An EPUB entry is larger than the supported limit.",
            ));
        }
        compressed_bytes = compressed_bytes
            .checked_add(entry.compressed_size())
            .ok_or_else(|| AppError::invalid_epub("The EPUB compressed size is invalid."))?;
        if compressed_bytes > limits.compressed_bytes {
            return Err(AppError::invalid_epub(
                "The EPUB compressed entries exceed the supported size limit.",
            ));
        }

        let mut entry_bytes = 0_u64;
        let mut mimetype = Vec::new();
        loop {
            let read = entry.read(&mut buffer).map_err(|error| {
                tracing::warn!(%error, entry = name, "failed to decompress EPUB entry");
                AppError::invalid_epub("An EPUB archive entry could not be decompressed.")
            })?;
            if read == 0 {
                break;
            }
            entry_bytes = entry_bytes
                .checked_add(read as u64)
                .ok_or_else(|| AppError::invalid_epub("The EPUB expanded size is invalid."))?;
            expanded_bytes = expanded_bytes
                .checked_add(read as u64)
                .ok_or_else(|| AppError::invalid_epub("The EPUB expanded size is invalid."))?;
            if entry_bytes > limits.entry_bytes {
                return Err(AppError::invalid_epub(
                    "An EPUB entry expands beyond the supported limit.",
                ));
            }
            if expanded_bytes > limits.expanded_bytes {
                return Err(AppError::invalid_epub(
                    "The EPUB expands beyond the supported size limit.",
                ));
            }
            if index == 0 && mimetype.len() <= 64 {
                mimetype.extend_from_slice(&buffer[..read]);
            }
        }
        if entry_bytes > 0
            && (entry.compressed_size() == 0
                || entry
                    .compressed_size()
                    .checked_mul(limits.compression_ratio)
                    .is_none_or(|maximum| entry_bytes > maximum))
        {
            return Err(AppError::invalid_epub(
                "An EPUB entry has a suspicious compression ratio.",
            ));
        }
        if index == 0 {
            if name != "mimetype" || entry.compression() != CompressionMethod::Stored {
                return Err(AppError::invalid_epub(
                    "The EPUB mimetype entry must be first and uncompressed.",
                ));
            }
            if mimetype != b"application/epub+zip" {
                return Err(AppError::invalid_epub(
                    "The archive does not declare the EPUB mimetype.",
                ));
            }
        }
    }
    let maximum_expansion = compressed_bytes.checked_mul(limits.compression_ratio);
    if expanded_bytes > 0
        && (compressed_bytes == 0 || maximum_expansion.is_none_or(|limit| expanded_bytes > limit))
    {
        return Err(AppError::invalid_epub(
            "The EPUB compression ratio is suspicious.",
        ));
    }
    Ok(())
}

fn copy_to_staging(root: &Path, source: &Path) -> AppResult<StagedEpub> {
    storage::validate_managed_directories(root)?;
    let source = validate_epub_path(source)?;
    let mut input = File::open(&source)
        .map_err(|_| AppError::invalid("The selected EPUB file could not be opened."))?;
    let metadata = input
        .metadata()
        .map_err(|_| AppError::invalid("The selected EPUB file could not be inspected."))?;
    if !metadata.is_file() {
        return Err(AppError::invalid("The selected path is not a file."));
    }

    let staging_path = root
        .join(storage::STAGING_DIR)
        .join(format!("source-{}.epub", Uuid::new_v4()));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging_path)
        .map_err(|_| AppError::storage())?;
    let copy_result = (|| {
        let mut digest = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer).map_err(|_| AppError::storage())?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .ok_or_else(|| AppError::invalid_epub("The EPUB compressed size is invalid."))?;
            if total > MAX_COMPRESSED_BYTES {
                return Err(AppError::invalid_epub(
                    "The EPUB compressed size is outside the supported range.",
                ));
            }
            output
                .write_all(&buffer[..read])
                .map_err(|_| AppError::storage())?;
            digest.update(&buffer[..read]);
        }
        if total == 0 {
            return Err(AppError::invalid_epub(
                "The EPUB compressed size is outside the supported range.",
            ));
        }
        output.flush().map_err(|_| AppError::storage())?;
        output.sync_all().map_err(|_| AppError::storage())?;
        Ok(StagedEpub {
            path: Some(staging_path.clone()),
            source_hash: format!("{:x}", digest.finalize()),
            content_length: total,
        })
    })();
    if copy_result.is_err() {
        drop(output);
        let _ = storage::remove_path(&staging_path);
    }
    copy_result
}

pub fn stage_epub(root: &Path, source: &Path) -> AppResult<StagedEpub> {
    let staged = copy_to_staging(root, source)?;
    preflight_epub(staged.path.as_deref().ok_or_else(AppError::storage)?)?;
    Ok(staged)
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

fn chunk_count(char_count: usize) -> usize {
    if char_count == 0 {
        0
    } else if char_count <= CHUNK_SIZE {
        1
    } else {
        1 + (char_count - CHUNK_SIZE).div_ceil(CHUNK_SIZE - CHUNK_OVERLAP)
    }
}

fn chapter_end_location(start: u64, char_count: usize) -> AppResult<u64> {
    start
        .checked_add(char_count.max(1) as u64)
        .ok_or_else(|| AppError::invalid_epub("The EPUB text is too large."))
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

fn normalized_resource_href(document_href: &str, reference: &str) -> Option<String> {
    let reference = reference.split(['?', '#']).next().unwrap_or_default();
    if reference.is_empty()
        || reference.contains(['\0', '\\'])
        || reference.starts_with("//")
        || reference
            .split('/')
            .next()
            .is_some_and(|component| component.contains(':'))
    {
        return None;
    }

    let mut components = Vec::new();
    if !reference.starts_with('/') {
        let document = document_href.split(['?', '#']).next().unwrap_or_default();
        for component in document
            .rsplit_once('/')
            .map_or("", |(parent, _)| parent)
            .split('/')
        {
            if !component.is_empty() && component != "." {
                components.push(component);
            }
        }
    }
    for component in reference.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component),
        }
    }
    Some(format!("/{}", components.join("/")))
}

fn sanitized_html(
    raw_html: &str,
    document_href: &str,
    image_names: &HashMap<String, String>,
) -> String {
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
    let document_href = document_href.to_owned();
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
                ("img", "src") => normalized_resource_href(&document_href, value)
                    .and_then(|href| images.get(&href))
                    .map(|name| Cow::Owned(format!("assets/{name}"))),
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

fn bounded_metadata(value: &str, maximum: usize, field: &str) -> AppResult<String> {
    let value = value.trim();
    if value.chars().count() > maximum {
        return Err(AppError::invalid_epub(format!(
            "The EPUB {field} metadata is too large."
        )));
    }
    Ok(value.to_owned())
}

fn image_type(mime: &str) -> Option<(&'static str, ImageFormat, &'static str)> {
    match mime {
        "image/jpeg" | "image/jpg" => Some(("jpg", ImageFormat::Jpeg, "image/jpeg")),
        "image/png" => Some(("png", ImageFormat::Png, "image/png")),
        "image/gif" => Some(("gif", ImageFormat::Gif, "image/gif")),
        "image/webp" => Some(("webp", ImageFormat::WebP, "image/webp")),
        _ => None,
    }
}

fn skip_gif_sub_blocks(bytes: &[u8], offset: &mut usize) -> bool {
    loop {
        let Some(&length) = bytes.get(*offset) else {
            return false;
        };
        *offset += 1;
        if length == 0 {
            return true;
        }
        let Some(next) = offset.checked_add(length as usize) else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        *offset = next;
    }
}

fn gif_has_single_frame(bytes: &[u8]) -> bool {
    if bytes.len() < 13 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return false;
    }
    let mut offset = 13_usize;
    let packed = bytes[10];
    if packed & 0x80 != 0 {
        let table_bytes = 3_usize << ((packed & 0x07) + 1);
        offset = match offset.checked_add(table_bytes) {
            Some(offset) if offset <= bytes.len() => offset,
            _ => return false,
        };
    }

    let mut frames = 0_usize;
    while let Some(&block) = bytes.get(offset) {
        offset += 1;
        match block {
            0x21 => {
                if bytes.get(offset).is_none() {
                    return false;
                }
                offset += 1;
                if !skip_gif_sub_blocks(bytes, &mut offset) {
                    return false;
                }
            }
            0x2c => {
                frames += 1;
                if frames > 1 || offset.checked_add(9).is_none_or(|end| end > bytes.len()) {
                    return false;
                }
                let descriptor_packed = bytes[offset + 8];
                offset += 9;
                if descriptor_packed & 0x80 != 0 {
                    let table_bytes = 3_usize << ((descriptor_packed & 0x07) + 1);
                    offset = match offset.checked_add(table_bytes) {
                        Some(offset) if offset <= bytes.len() => offset,
                        _ => return false,
                    };
                }
                if bytes.get(offset).is_none() {
                    return false;
                }
                offset += 1;
                if !skip_gif_sub_blocks(bytes, &mut offset) {
                    return false;
                }
            }
            0x3b => return frames == 1 && offset == bytes.len(),
            _ => return false,
        }
    }
    false
}

fn valid_image(bytes: &[u8], format: ImageFormat) -> bool {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return false;
    }
    if format == ImageFormat::Gif && !gif_has_single_frame(bytes) {
        return false;
    }
    let mut image_limits = Limits::default();
    image_limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    image_limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    image_limits.max_alloc = Some(256 * 1024 * 1024);
    let mut dimensions_reader =
        ImageReader::with_format(BufReader::new(Cursor::new(bytes)), format);
    dimensions_reader.limits(image_limits.clone());
    let Ok((width, height)) = dimensions_reader.into_dimensions() else {
        return false;
    };
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return false;
    }
    let mut decode_reader = ImageReader::with_format(BufReader::new(Cursor::new(bytes)), format);
    decode_reader.limits(image_limits);
    decode_reader.decode().is_ok()
}

fn parse_epub(
    book_dir: &Path,
    id: String,
    source_hash: String,
    content_length: u64,
) -> AppResult<ParsedBook> {
    fs::create_dir(book_dir.join("assets")).map_err(|_| AppError::storage())?;
    let stored_source = book_dir.join("source.epub");
    let book_rel_dir = format!("books/{id}");
    let source_rel_path = format!("{book_rel_dir}/source.epub");
    let mut budget = BookBudget::default();
    budget.add_generated(content_length)?;

    let epub = Epub::options()
        .strict(false)
        .open(&stored_source)
        .map_err(|error| {
            tracing::error!(%error, "rbook failed to parse EPUB");
            AppError::invalid_epub("The EPUB structure could not be parsed.")
        })?;
    budget.set_assets(epub.manifest().len())?;
    let metadata = epub.metadata();
    let title = metadata
        .title()
        .map(|entry| bounded_metadata(entry.value(), 1_000, "title"))
        .transpose()?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Unknown Title".to_owned());
    let mut author = String::new();
    for creator in metadata.creators() {
        let creator = bounded_metadata(creator.value(), 1_000, "creator")?;
        if creator.is_empty() {
            continue;
        }
        let combined_chars = author
            .chars()
            .count()
            .checked_add(creator.chars().count())
            .and_then(|count| count.checked_add(usize::from(!author.is_empty()) * 2))
            .ok_or_else(|| AppError::invalid_epub("The EPUB author metadata is too large."))?;
        if combined_chars > 2_000 {
            return Err(AppError::invalid_epub(
                "The EPUB author metadata is too large.",
            ));
        }
        if !author.is_empty() {
            author.push_str(", ");
        }
        author.push_str(&creator);
    }
    let author = if author.is_empty() {
        "Unknown Author".to_owned()
    } else {
        author
    };
    let language = metadata
        .language()
        .map(|entry| bounded_metadata(entry.value(), 64, "language"))
        .transpose()?;
    let publisher = metadata
        .publishers()
        .next()
        .map(|entry| bounded_metadata(entry.value(), 1_000, "publisher"))
        .transpose()?
        .filter(|value| !value.is_empty());
    let description = metadata
        .description()
        .map(|entry| bounded_metadata(entry.value(), 100_000, "description"))
        .transpose()?
        .filter(|value| !value.is_empty());
    let isbn = metadata
        .identifiers()
        .next()
        .map(|entry| bounded_metadata(entry.value(), 256, "identifier"))
        .transpose()?
        .filter(|value| !value.is_empty());
    let published_year = metadata
        .published()
        .map(|value| i32::from(value.date().year()));

    let cover_href = epub
        .manifest()
        .cover_image()
        .map(|entry| entry.href().path().as_str().to_owned());
    let mut image_names = HashMap::new();
    let mut cover_rel_path = None;
    let mut cover_mime = None;
    for image in epub.manifest().images() {
        let mime = image.media_type();
        let Some((extension, format, canonical_mime)) = image_type(mime) else {
            continue;
        };
        let bytes = match image.read_bytes() {
            Ok(bytes) if valid_image(&bytes, format) => bytes,
            _ => continue,
        };
        budget.add_generated(
            u64::try_from(bytes.len())
                .map_err(|_| AppError::invalid_epub("The generated book data is too large."))?,
        )?;
        let href = image.href().path();
        let encoded_href = href.as_str().to_owned();
        let decoded_href = href.decode().into_owned();
        let asset_name = format!("{}.{}", Uuid::new_v4(), extension);
        let asset_rel_path = format!("{book_rel_dir}/assets/{asset_name}");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(book_dir.join("assets").join(&asset_name))
            .map_err(|_| AppError::storage())?;
        file.write_all(&bytes).map_err(|_| AppError::storage())?;

        image_names.insert(encoded_href.clone(), asset_name.clone());
        image_names.insert(decoded_href, asset_name);
        if cover_href.as_deref() == Some(&encoded_href) {
            cover_rel_path = Some(asset_rel_path);
            cover_mime = Some(canonical_mime.to_owned());
        }
    }

    let mut chapters = Vec::new();
    let mut next_location = 0_u64;
    if epub.spine().len() > MAX_SPINE_CHAPTERS {
        return Err(AppError::invalid_epub("The EPUB has too many chapters."));
    }
    let reader = epub
        .reader_builder()
        .linear_behavior(LinearBehavior::LinearOnly)
        .create();
    if reader.len() > MAX_SPINE_CHAPTERS {
        return Err(AppError::invalid_epub("The EPUB has too many chapters."));
    }
    for (order, result) in reader.enumerate() {
        let content = result.map_err(|error| {
            tracing::error!(%error, "failed to read EPUB spine entry");
            AppError::invalid_epub("An EPUB chapter could not be read.")
        })?;
        let document_href = content.manifest_entry().href().path().as_str().to_owned();
        let html = sanitized_html(content.content(), &document_href, &image_names);
        let text = extract_plain_text(&html);
        let char_count = text.chars().count();
        budget.add_chapter(char_count, html.len())?;
        let end_location = chapter_end_location(next_location, char_count)?;
        let chapter_id = Uuid::new_v4().to_string();
        let rel_path = format!("{book_rel_dir}/chapter-{chapter_id}.html");
        let order = u32::try_from(order)
            .map_err(|_| AppError::invalid_epub("The EPUB has too many chapters."))?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(book_dir.join(format!("chapter-{chapter_id}.html")))
            .map_err(|_| AppError::storage())?;
        file.write_all(html.as_bytes())
            .map_err(|_| AppError::storage())?;
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
        content_length,
        total_locations: next_location,
        chapters,
    })
}

pub fn parse_staged_epub(root: &Path, mut staged: StagedEpub) -> AppResult<PreparedImport> {
    storage::validate_managed_directories(root)?;
    let id = Uuid::new_v4().to_string();
    let staging_dir = root.join(storage::STAGING_DIR).join(format!("book-{id}"));
    fs::create_dir(&staging_dir).map_err(|_| AppError::storage())?;
    let source_path = staging_dir.join("source.epub");
    let staged_path = staged.path.as_deref().ok_or_else(AppError::storage)?;
    if let Err(error) = fs::rename(staged_path, &source_path) {
        tracing::error!(%error, "failed to move staged EPUB into generated book data");
        let _ = storage::remove_path(&staging_dir);
        return Err(AppError::storage());
    }
    staged.path = None;
    let parsed = match parse_epub(
        &staging_dir,
        id,
        staged.source_hash.clone(),
        staged.content_length,
    ) {
        Ok(parsed) => parsed,
        Err(error) => {
            let _ = storage::remove_path(&staging_dir);
            return Err(error);
        }
    };
    Ok(PreparedImport {
        parsed: Some(parsed),
        staging_dir: Some(staging_dir),
    })
}

fn persist_book_rows(connection: &mut Connection, parsed: &ParsedBook) -> AppResult<BookSummary> {
    let now = Utc::now().to_rfc3339();
    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin import transaction");
        AppError::database()
    })?;
    let book_count = transaction
        .query_row("SELECT COUNT(*) FROM books", [], |row| row.get::<_, i64>(0))
        .map_err(|error| {
            tracing::error!(%error, "failed to count books before import");
            AppError::database()
        })?;
    let book_count = usize::try_from(book_count).map_err(|_| AppError::database())?;
    if book_count > MAX_LIBRARY_BOOKS {
        return Err(AppError::database());
    }
    if book_count == MAX_LIBRARY_BOOKS {
        return Err(AppError::invalid("The library has reached its book limit."));
    }
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
            if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
                AppError::new(
                    AppErrorKind::AlreadyExists,
                    "This EPUB is already in the library.",
                )
            } else {
                AppError::database()
            }
        })?;

    let mut indexed_chunk_count = 0_usize;
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
            indexed_chunk_count += 1;
            if indexed_chunk_count > MAX_CHUNKS_PER_BOOK {
                return Err(AppError::invalid_epub(
                    "The EPUB produces too many text chunks.",
                ));
            }
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

    Ok(BookSummary {
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
    })
}

pub fn publish_import(root: &Path, mut prepared: PreparedImport) -> AppResult<PublishedImport> {
    storage::validate_managed_directories(root)?;
    let staging_dir = prepared
        .staging_dir
        .as_deref()
        .ok_or_else(AppError::storage)?;
    let final_dir = storage::book_path(
        root,
        &prepared.parsed.as_ref().ok_or_else(AppError::storage)?.id,
    )?;
    fs::rename(staging_dir, &final_dir).map_err(|error| {
        tracing::error!(%error, "failed to publish generated book files");
        AppError::storage()
    })?;
    prepared.staging_dir = None;
    Ok(PublishedImport {
        parsed: prepared.parsed.take().ok_or_else(AppError::storage)?,
        root: root.to_path_buf(),
        final_dir: Some(final_dir),
    })
}

pub fn persist_published(
    connection: &mut Connection,
    published: &PublishedImport,
) -> AppResult<BookSummary> {
    persist_book_rows(connection, &published.parsed)
}

pub fn commit_published(mut published: PublishedImport) {
    published.final_dir = None;
}

pub fn rollback_published(mut published: PublishedImport) -> AppResult<()> {
    let final_dir = published
        .final_dir
        .as_ref()
        .ok_or_else(AppError::storage)?
        .clone();
    let rollback_path = published
        .root
        .join(storage::TRASH_DIR)
        .join(Uuid::new_v4().to_string());
    fs::rename(&final_dir, &rollback_path).map_err(|error| {
        tracing::error!(%error, "failed to move rolled-back book files out of the active library");
        AppError::storage()
    })?;
    published.final_dir = None;
    if let Err(cleanup_error) = storage::remove_path(&rollback_path) {
        tracing::warn!(kind = ?cleanup_error.kind, "rolled-back book cleanup deferred until startup");
    }
    Ok(())
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

    fn write_test_epub(path: &Path) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file(
                "mimetype",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"application/epub+zip").unwrap();

        let compressed =
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer
            .start_file("META-INF/container.xml", compressed)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
            )
            .unwrap();
        writer.start_file("OEBPS/content.opf", compressed).unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="book-id" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">fixture-book</dc:identifier>
    <dc:title>Fixture Book</dc:title>
    <dc:creator>Test Author</dc:creator>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-07-15T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="navigation" href="navigation.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chapter"/>
  </spine>
</package>"#,
            )
            .unwrap();
        writer
            .start_file("OEBPS/navigation.xhtml", compressed)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <head><title>Contents</title></head>
  <body>
    <nav epub:type="toc" id="toc">
      <h1>Contents</h1>
      <ol><li><a href="chapter.xhtml">Opening</a></li></ol>
    </nav>
  </body>
</html>"#,
            )
            .unwrap();
        writer
            .start_file("OEBPS/chapter.xhtml", compressed)
            .unwrap();
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Opening</title></head>
  <body><h1>Opening</h1><p>A complete EPUB fixture for the Rust importer.</p></body>
</html>"#,
            )
            .unwrap();
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
        let images = HashMap::from([(
            "/OPS/images/safe.png".to_owned(),
            "generated.png".to_owned(),
        )]);
        let html = sanitized_html(
            r#"<p onclick="bad()">Hello<script>bad()</script></p>
               <form><input value="secret"></form>
               <img src="https://example.com/tracker.png">
               <img src="../images/safe.png" onerror="bad()">"#,
            "/OPS/text/chapter.xhtml",
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

    #[test]
    fn image_references_resolve_by_full_manifest_path() {
        let images = HashMap::from([
            ("/OPS/one/shared.png".to_owned(), "first.png".to_owned()),
            ("/OPS/two/shared.png".to_owned(), "second.png".to_owned()),
        ]);

        let first = sanitized_html(
            r#"<img src="../one/shared.png">"#,
            "/OPS/text/chapter.xhtml",
            &images,
        );
        let second = sanitized_html(
            r#"<img src="../two/shared.png">"#,
            "/OPS/text/chapter.xhtml",
            &images,
        );

        assert!(first.contains("assets/first.png"));
        assert!(!first.contains("second.png"));
        assert!(second.contains("assets/second.png"));
        assert!(!second.contains("first.png"));
    }

    #[test]
    fn zero_text_chapters_have_one_addressable_location_and_no_chunks() {
        assert_eq!(chapter_end_location(41, 0).unwrap(), 42);
        assert_eq!(chunk_count(0), 0);
        assert!(chunk_text("", 41).is_empty());
    }

    #[test]
    fn logical_budget_enforces_every_cap() {
        let mut chapters = BookBudget {
            chapters: MAX_SPINE_CHAPTERS,
            ..Default::default()
        };
        assert!(chapters.add_chapter(0, 0).is_err());

        let mut assets = BookBudget::default();
        assert!(assets.set_assets(MAX_ASSETS + 1).is_err());

        let mut text = BookBudget {
            text_chars: MAX_NORMALIZED_TEXT_CHARS,
            ..Default::default()
        };
        assert!(text.add_chapter(1, 0).is_err());

        let mut chunks = BookBudget {
            chunks: MAX_CHUNKS_PER_BOOK,
            ..Default::default()
        };
        assert!(chunks.add_chapter(1, 0).is_err());

        let mut generated = BookBudget {
            generated_bytes: MAX_GENERATED_BYTES,
            ..Default::default()
        };
        assert!(generated.add_generated(1).is_err());
    }

    #[test]
    fn preflight_reads_entry_data_and_detects_crc_corruption() {
        let epub_path = temp_path("corrupt.epub");
        let file = File::create(&epub_path).unwrap();
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
                "META-INF/container.xml",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"unique-container-payload").unwrap();
        writer.finish().unwrap();

        let mut bytes = fs::read(&epub_path).unwrap();
        let payload = b"unique-container-payload";
        let offset = bytes
            .windows(payload.len())
            .position(|window| window == payload)
            .unwrap();
        bytes[offset] ^= 0xff;
        fs::write(&epub_path, bytes).unwrap();

        assert!(preflight_epub(&epub_path).is_err());
        fs::remove_file(epub_path).unwrap();
    }

    #[test]
    fn preflight_enforces_actual_expanded_limits() {
        let epub_path = temp_path("bounded.epub");
        write_test_zip(&epub_path, None);
        let limits = ArchiveLimits {
            compressed_bytes: 1024 * 1024,
            expanded_bytes: 24,
            entry_bytes: 1024,
            entries: 10,
            compression_ratio: 100,
        };

        assert!(preflight_epub_with_limits(&epub_path, limits).is_err());
        fs::remove_file(epub_path).unwrap();
    }

    #[test]
    fn staged_source_is_immutable_and_removed_on_drop() {
        let root = temp_path("staging-root");
        let root = storage::initialize(&root).unwrap();
        let source = root.join("selected.epub");
        write_test_zip(&source, None);
        let original = fs::read(&source).unwrap();

        let staged = stage_epub(&root, &source).unwrap();
        let staged_path = staged.path.clone().unwrap();
        fs::write(&source, b"changed after staging").unwrap();

        assert_eq!(fs::read(&staged_path).unwrap(), original);
        drop(staged);
        assert!(!staged_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_animated_gif_without_decoding_all_frames() {
        let single = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x21\xf9\x04\x01\x00\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00\x3b";
        assert!(gif_has_single_frame(single));
        assert!(valid_image(single, ImageFormat::Gif));

        let trailer = single.len() - 1;
        let image_start = single.iter().position(|byte| *byte == 0x2c).unwrap();
        let mut animated = single[..trailer].to_vec();
        animated.extend_from_slice(&single[image_start..trailer]);
        animated.push(0x3b);
        assert!(!gif_has_single_frame(&animated));
        assert!(!valid_image(&animated, ImageFormat::Gif));
    }

    #[test]
    fn image_validation_rejects_oversized_dimensions() {
        let mut oversized = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff\x21\xf9\x04\x01\x00\x00\x00\x00\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00\x3b".to_vec();
        oversized[6..8].copy_from_slice(&16_385_u16.to_le_bytes());

        assert!(gif_has_single_frame(&oversized));
        assert!(!valid_image(&oversized, ImageFormat::Gif));

        oversized[6..8].copy_from_slice(&10_000_u16.to_le_bytes());
        oversized[8..10].copy_from_slice(&5_000_u16.to_le_bytes());
        assert!(!valid_image(&oversized, ImageFormat::Gif));
    }

    fn prepared_fixture(root: &Path, id: &str, source_hash: &str) -> PreparedImport {
        let staging_dir = root.join(storage::STAGING_DIR).join(format!("book-{id}"));
        fs::create_dir(&staging_dir).unwrap();
        fs::create_dir(staging_dir.join("assets")).unwrap();
        fs::write(staging_dir.join("source.epub"), b"source").unwrap();
        let chapter_id = Uuid::new_v4().to_string();
        fs::write(
            staging_dir.join(format!("chapter-{chapter_id}.html")),
            b"<p>text</p>",
        )
        .unwrap();
        PreparedImport {
            parsed: Some(ParsedBook {
                id: id.to_owned(),
                source_hash: source_hash.to_owned(),
                title: "Title".to_owned(),
                author: "Author".to_owned(),
                source_rel_path: format!("books/{id}/source.epub"),
                cover_rel_path: None,
                cover_mime: None,
                language: None,
                published_year: None,
                publisher: None,
                isbn: None,
                description: None,
                content_length: 6,
                total_locations: 4,
                chapters: vec![ParsedChapter {
                    id: chapter_id.clone(),
                    title: "Chapter".to_owned(),
                    order: 0,
                    text: "text".to_owned(),
                    start_location: 0,
                    end_location: 4,
                    rel_path: format!("books/{id}/chapter-{chapter_id}.html"),
                }],
            }),
            staging_dir: Some(staging_dir),
        }
    }

    fn persist_fixture(
        root: &Path,
        connection: &mut Connection,
        prepared: PreparedImport,
    ) -> AppResult<BookSummary> {
        let published = publish_import(root, prepared)?;
        match persist_published(connection, &published) {
            Ok(imported) => {
                commit_published(published);
                Ok(imported)
            }
            Err(error) => {
                rollback_published(published)?;
                Err(error)
            }
        }
    }

    #[test]
    fn duplicate_detected_at_final_insert_removes_published_files() {
        let root = temp_path("duplicate-root");
        let root = storage::initialize(&root).unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        crate::db::configure_connection(&connection).unwrap();
        crate::db::migrations().to_latest(&mut connection).unwrap();
        let first_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let second_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

        persist_fixture(
            &root,
            &mut connection,
            prepared_fixture(&root, first_id, "same-hash"),
        )
        .unwrap();
        let error = persist_fixture(
            &root,
            &mut connection,
            prepared_fixture(&root, second_id, "same-hash"),
        )
        .unwrap_err();

        assert!(matches!(error.kind, AppErrorKind::AlreadyExists));
        assert!(root.join(storage::BOOKS_DIR).join(first_id).is_dir());
        assert!(!root.join(storage::BOOKS_DIR).join(second_id).exists());
        assert_eq!(
            fs::read_dir(root.join(storage::STAGING_DIR))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn imports_generated_epub_end_to_end() {
        let root = temp_path("fixture-root");
        let root = storage::initialize(&root).unwrap();
        let fixture = root.join("fixture.epub");
        write_test_epub(&fixture);
        let staged = stage_epub(&root, &fixture).unwrap();
        let prepared = parse_staged_epub(&root, staged).unwrap();
        let published = publish_import(&root, prepared).unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        crate::db::configure_connection(&connection).unwrap();
        crate::db::migrations().to_latest(&mut connection).unwrap();

        let summary = persist_published(&mut connection, &published).unwrap();
        commit_published(published);
        let detail = crate::db::get_book(&connection, &summary.id).unwrap();

        assert_eq!(detail.title, "Fixture Book");
        assert_eq!(detail.author.as_deref(), Some("Test Author"));
        assert_eq!(detail.chapters.len(), 1);
        assert!(detail.total_locations > 0);
        let first_chapter = detail.chapters.first().unwrap();
        assert_eq!(first_chapter.title, "Opening");
        let chapter_path = storage::chapter_path(&root, &detail.id, &first_chapter.id).unwrap();
        assert!(fs::read_to_string(chapter_path)
            .unwrap()
            .contains("complete EPUB fixture"));

        fs::remove_dir_all(root).unwrap();
    }
}
