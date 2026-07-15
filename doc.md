# MeReader Architecture

## 1. Scope

MeReader is a desktop EPUB reader with an embedded Rust core and a Svelte interface. It has three primary responsibilities:

1. Import and render EPUB content safely.
2. Persist a local library and exact reading position.
3. Answer questions from text the reader has already reached.

The current application supports EPUB files only. It does not provide accounts, cloud synchronization, a browser-hosted API, or a remote model provider.

## 2. Runtime Architecture

```text
Svelte 5 interface
        |
        | typed Tauri invoke calls and answer events
        v
Embedded Rust core
  |       |         |
  |       |         +-- reqwest --> Ollama on 127.0.0.1:11434
  |       +------------ rusqlite --> local SQLite and FTS5
  +-------------------- filesystem --> EPUBs, chapters, covers, assets
```

There is no application localhost server. The production frontend is bundled into the Tauri binary, and all privileged operations are registered Rust commands. During development, Tauri starts Vite at `http://localhost:5173` only to serve the interface to the native webview.

The Tauri window has no direct filesystem or dialog capability grants. Native file selection and destructive confirmation are performed inside Rust commands.

## 3. Command Surface

The TypeScript wrappers in `frontend/src/lib/commands.ts` are the frontend boundary for these Rust commands:

| Command | Purpose |
| --- | --- |
| `list_books` | Return bounded library summaries and progress. |
| `import_book` | Select, validate, parse, publish, persist, and index an EPUB. |
| `get_book` | Return metadata, ordered chapters, and saved progress. |
| `get_cover` | Return a validated cover image from managed storage. |
| `get_chapter` | Return one sanitized chapter. |
| `get_chapter_asset` | Return one validated local image referenced by a chapter. |
| `update_progress` | Persist a bounded location and chapter identifier. |
| `delete_book` | Confirm deletion, remove database rows, and clean files. |
| `get_ai_status` | Report Ollama, model, and per-book index readiness. |
| `reindex_book` | Rebuild semantic embeddings for one book. |
| `ask_book` | Retrieve passages and stream a local Ollama answer over a channel. |

Errors crossing this boundary contain a stable kind and a user-safe message. Detailed parser, database, filesystem, and model errors remain in Rust logs.

## 4. EPUB Import Pipeline

`import_book` is a staged transaction across the filesystem and SQLite:

1. The native picker accepts an `.epub` file.
2. Rust copies the source into managed staging while hashing the copied bytes.
3. ZIP preflight validates the mimetype, entry count, paths, compressed and expanded sizes, compression ratios, and per-entry limits.
4. The staged copy is parsed for metadata, reading order, chapters, cover art, and supported images.
5. HTML is sanitized, external references are removed, local image references are rewritten, and supported images are decoded to validate their dimensions and pixel counts.
6. Visible text is normalized and divided into overlapping location-aware chunks.
7. Generated files are published to a new UUID book directory.
8. Metadata, chapters, progress, chunks, FTS rows, and index state are inserted in SQLite.
9. A failed database write rolls the published directory back. A successful write commits it.
10. Semantic indexing is attempted after import. Failure leaves FTS data and reading available.

The parser works only from the immutable staged copy. Duplicate detection uses the staged content hash, so changes to the original selected path cannot alter the book after validation.

## 5. Reading and Progress

Each chapter has a stable UUID, a normalized text length, and an absolute half-open location range. The frontend maps rendered visible text back to these locations rather than using scroll pixels as the persistent position.

`ReaderContent.svelte` restores a saved location, reports new visible-text positions, resolves local chapter assets through Rust, and supports chapter and viewport navigation. `Reader.svelte` places updates in a latest-wins save queue. It flushes the pending position before returning to the library and retains a failed update for retry.

Reader appearance settings are local frontend preferences stored under `mereader.reader-settings.v1`. They currently include theme, font size, line height, and reading width.

## 6. Retrieval and Local AI

The default local model configuration is:

- Generation: `llama3.2:latest`
- Embedding: `nomic-embed-text:latest`
- Ollama endpoint: `http://127.0.0.1:11434`

Books are always chunked and inserted into SQLite FTS5. When the embedding model is available, indexing writes a complete, dimension-checked embedding generation atomically. A failed rebuild leaves the book text-only rather than retaining a partial vector index.

For a question, the core:

1. Validates the book, question, current progress, and configured models.
2. Applies the persisted reading location as the retrieval boundary.
3. Runs bounded FTS retrieval.
4. Runs bounded cosine retrieval when a compatible semantic index is ready.
5. Combines candidates with reciprocal-rank fusion and removes duplicates.
6. Builds a prompt from source passages at or before the reading boundary.
7. Streams answer deltas to the frontend through a Tauri channel.
8. Returns the final answer, source metadata, and the boundary used.

If semantic search is unavailable, questions can still use keyword retrieval when the generation model is installed. If Ollama or the generation model is unavailable, reading remains unaffected and the AI panel reports the concrete recovery action.

## 7. Persistence

Tauri resolves the root application-data directory for the current operating system. The Rust core owns this layout:

```text
app-data/
  library.sqlite3
  library.sqlite3-wal
  library.sqlite3-shm
  books/
    <book-uuid>/
      source.epub
      chapter-<chapter-uuid>.html
      assets/
        <asset-uuid>.<supported-extension>
  staging/
  trash/
```

`staging/` and `trash/` are cleaned at startup. Managed directory components must be real directories, not symbolic links.

SQLite uses foreign keys, WAL journaling, a busy timeout, and embedded migrations. Its main tables are:

| Table | Contents |
| --- | --- |
| `books` | Source hash, metadata, managed paths, and aggregate lengths. |
| `chapters` | Reading order, managed chapter path, and location range. |
| `progress` | Current location, chapter, completion, and last-read time. |
| `chunks` | Passage text, boundaries, and optional embedding blobs. |
| `chunks_fts` | Unicode FTS5 index for keyword retrieval. |
| `index_state` | Per-book embedding model, generation, dimensions, status, and errors. |
| `settings` | Persisted core model defaults reserved for application settings. |

Book deletion first moves managed files into `trash/`, then removes database rows. A database failure restores the files. A cleanup failure is deferred to the next startup.

## 8. Trust Boundaries

EPUB files and Ollama responses are untrusted inputs.

EPUB protections include:

- Canonical staged input and content-hash duplicate detection.
- ZIP path traversal, archive-count, size, ratio, and expansion limits.
- Limits on spine chapters, assets, text length, chunks, image dimensions, image pixels, and generated bytes.
- HTML sanitization with no scripts, forms, frames, remote media, or arbitrary local paths.
- Decoding and bounding of supported JPEG, PNG, GIF, and WebP assets before storage.
- UUID validation and canonical parent checks before managed file reads.
- Symbolic-link rejection for managed directories and files.

Ollama protections include:

- Fixed loopback endpoint, no proxy use, no redirects, and connection timeouts.
- Bounded response bodies and streamed line lengths.
- Strict embedding count, dimension, finite-value, and model-generation checks.
- Bounded concurrent imports and AI operations.

The webview content security policy denies frames, objects, forms, arbitrary connections, and remote scripts.

## 9. Frontend Structure

```text
frontend/src/
  App.svelte                 Library and reader application state
  components/Library.svelte Import, search, sort, covers, and deletion
  components/Reader.svelte  Reader shell, panels, settings, and save queue
  components/ReaderContent.svelte
                             Sanitized chapter rendering and text anchors
  components/AiPanel.svelte AI readiness, streaming, citations, and source jumps
  lib/commands.ts            Typed Tauri IPC wrappers
  lib/chapters.ts            Chapter-boundary helpers
  lib/progress.ts            Latest-wins persistence queue
  lib/text-anchors.ts        Visible-text location mapping
  lib/types.ts               Shared frontend contracts
```

The desktop layout keeps navigation, reading controls, and the AI panel separate. At narrow widths, auxiliary panels become mutually exclusive overlays. Keyboard focus is moved into opened panels and returned to their trigger when closed.

## 10. Verification Strategy

Rust unit tests cover database migrations and constraints, import validation and rollback, archive limits, HTML and image sanitization, storage safety, progress boundaries, hybrid retrieval, vector validation, Ollama response bounds, and command behavior. The import suite generates a complete EPUB archive in the test itself so it does not depend on an opaque binary fixture.

Frontend tests cover IPC wrappers, library states, reader navigation, progress persistence, text anchors, AI streaming and recovery, source jumps, and application transitions. `svelte-check`, Vite production builds, strict Clippy, Rust formatting, and Rust compilation are separate required gates.

The verified Linux packaging path is a debug Debian bundle. AppImage creation also depends on the host `linuxdeploy` environment and is not treated as a source-code verification gate.

## 11. Current Constraints

- EPUB is the only supported book format.
- The configured Ollama endpoint and model names are fixed defaults in the current interface.
- Semantic indexing requires the embedding model to be installed before indexing or reindexing.
- Spoiler control is a retrieval boundary, not a guarantee about facts the generation model learned before receiving the book context.
- There is no synchronization, annotation, export, or multi-profile support.
- Native behavior must be tested through Tauri; a standalone browser preview cannot exercise Rust IPC.
