# MeReader

MeReader is a private desktop EPUB reader with progress-aware local AI. The app is built with Tauri 2, Rust, Svelte 5, and SQLite. Books, reading history, search indexes, and model requests stay on the local machine.

## Features

- Import EPUB files through the native file picker.
- Read sanitized book content with table-of-contents navigation, themes, type controls, and restored reading position.
- Search and sort a local library with lazy-loaded covers.
- Ask questions about the current book with streamed answers and source passages.
- Restrict retrieval to text at or before the current reading position to reduce spoilers.
- Use SQLite FTS5 when semantic embeddings are unavailable.
- Delete a book and its generated data through a native confirmation flow.

MeReader does not run an application web server. The Svelte interface calls the embedded Rust core through Tauri IPC. Ollama is the only optional external process, and it is contacted directly on loopback.

## Requirements

- Node.js `^20.19.0` or `>=22.12.0`
- A current stable Rust toolchain
- The [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your operating system
- Optional: [Ollama](https://ollama.com/) for local AI answers and semantic retrieval

The default Ollama models are:

```bash
ollama pull llama3.2:latest
ollama pull nomic-embed-text:latest
```

The reader remains usable without Ollama. AI answers require the generation model. If that model is installed but the embedding model is not, questions fall back to keyword-only grounding.

## Development

Install dependencies and launch the native desktop app:

```bash
cd frontend
npm ci
npm run tauri dev
```

Use `npm run tauri dev`, not the standalone Vite server, when testing application behavior. Native file dialogs, persistent storage, EPUB processing, and AI streaming are Rust commands exposed through Tauri.

## Verification

Run the frontend checks from `frontend/`:

```bash
npm test
npm run test:coverage
npm run check
npm run build
```

Run the Rust checks from `frontend/src-tauri/`:

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo check
```

## Packaging

Create the platform bundle supported by the current host:

```bash
cd frontend
npm run tauri build
```

For the verified Linux debug Debian package:

```bash
npm run tauri -- build --debug --bundles deb
```

Generated installers are written below `frontend/src-tauri/target/`.

## Repository Layout

```text
frontend/
  src/                 Svelte interface, IPC wrappers, and frontend tests
  src-tauri/src/       Rust application core and unit tests
  src-tauri/           Tauri configuration, capabilities, and icons
MeReader_ICCCI2026.pdf
MeReader_Response_to_Reviewers.pdf
doc.md                 Architecture and implementation notes
```

The repository intentionally keeps the exported paper PDFs but not TeX, BibTeX, or generated paper source trees.

## Data and Privacy

MeReader stores its SQLite database, imported books, sanitized chapters, and generated assets in the operating system application-data directory selected by Tauri. Import staging and deletion trash are cleaned during startup.

No book content is sent to a cloud service by the application. When AI is enabled, relevant passages are sent only to the local Ollama service at `http://127.0.0.1:11434`.

See [`doc.md`](doc.md) for the command surface, data model, import pipeline, retrieval design, and trust boundaries.
