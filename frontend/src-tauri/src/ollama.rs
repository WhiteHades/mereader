use chrono::Utc;
use reqwest::{Response, StatusCode};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::error::{AppError, AppErrorKind, AppResult};
use crate::limits::{
    MAX_CHUNKS_PER_BOOK, MAX_LIBRARY_BOOKS, MAX_NORMALIZED_TEXT_CHARS, MAX_VECTOR_DIMENSION,
};
use crate::models::{
    IndexedChunk, OllamaEmbedResponse, OllamaGenerateChunk, OllamaModel, OllamaTagsResponse,
};
use crate::state::{AppState, OLLAMA_BASE_URL};

const EMBEDDING_BATCH_SIZE: usize = 32;
const MAX_MODEL_LIST_BYTES: usize = 1024 * 1024;
const MAX_EMBEDDING_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_STREAM_LINE_BYTES: usize = 1024 * 1024;
const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;
const MAX_ANSWER_CHARS: usize = 20_000;

async fn bounded_body(
    mut response: Response,
    maximum: usize,
    message: &'static str,
) -> AppResult<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(bytes) = response.chunk().await.map_err(|error| {
        tracing::error!(%error, "failed to read bounded Ollama response");
        AppError::ai_request(message)
    })? {
        if body
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > maximum)
        {
            return Err(AppError::ai_request(message));
        }
        body.extend_from_slice(&bytes);
    }
    Ok(body)
}

pub async fn installed_models(state: &AppState) -> AppResult<Vec<OllamaModel>> {
    let response = state
        .status_http
        .get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .send()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Ollama model listing unavailable");
            AppError::ai_unavailable("Ollama is not available.")
        })?;
    if response.status() != StatusCode::OK {
        tracing::warn!(status = %response.status(), "Ollama model listing failed");
        return Err(AppError::ai_unavailable("Ollama is not available."));
    }
    let body = bounded_body(
        response,
        MAX_MODEL_LIST_BYTES,
        "Ollama returned an invalid status response.",
    )
    .await?;
    let response: OllamaTagsResponse = serde_json::from_slice(&body).map_err(|error| {
        tracing::error!(%error, "invalid Ollama model listing response");
        AppError::ai_request("Ollama returned an invalid status response.")
    })?;
    if response.models.len() > 1_000
        || response
            .models
            .iter()
            .any(|model| model.name.len() > 512 || model.model.len() > 512)
    {
        return Err(AppError::ai_request(
            "Ollama returned an invalid status response.",
        ));
    }
    Ok(response.models)
}

pub fn model_is_installed(models: &[OllamaModel], configured: &str) -> bool {
    models.iter().any(|model| {
        model.name == configured
            || model.model == configured
            || (!configured.contains(':')
                && (model.name.starts_with(&format!("{configured}:"))
                    || model.model.starts_with(&format!("{configured}:"))))
    })
}

fn validate_embedding_batch(
    embeddings: &[Vec<f32>],
    expected_count: usize,
    expected_dimension: Option<usize>,
) -> AppResult<usize> {
    if embeddings.len() != expected_count || embeddings.is_empty() {
        return Err(AppError::ai_request(
            "Ollama returned an incomplete embedding response.",
        ));
    }
    let dimension = embeddings[0].len();
    if dimension == 0
        || dimension > MAX_VECTOR_DIMENSION
        || expected_dimension.is_some_and(|expected| expected != dimension)
        || embeddings.iter().any(|embedding| {
            embedding.len() != dimension || embedding.iter().any(|v| !v.is_finite())
        })
    {
        return Err(AppError::ai_request("Ollama returned invalid embeddings."));
    }
    Ok(dimension)
}

pub async fn embed_texts(
    state: &AppState,
    texts: &[String],
    expected_dimension: Option<usize>,
) -> AppResult<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    if texts.len() > EMBEDDING_BATCH_SIZE
        || texts.iter().map(|text| text.chars().count()).sum::<usize>()
            > EMBEDDING_BATCH_SIZE * 2_000
    {
        return Err(AppError::invalid(
            "Embedding input exceeds the supported limit.",
        ));
    }
    let response = state
        .embedding_http
        .post(format!("{OLLAMA_BASE_URL}/api/embed"))
        .json(&json!({
            "model": state.embedding_model,
            "input": texts,
            "truncate": true
        }))
        .send()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Ollama embedding request unavailable");
            AppError::ai_unavailable("The configured embedding model is unavailable.")
        })?;
    if response.status() != StatusCode::OK {
        tracing::warn!(status = %response.status(), "Ollama embedding request failed");
        return Err(AppError::ai_unavailable(
            "The configured embedding model is unavailable.",
        ));
    }
    let body = bounded_body(
        response,
        MAX_EMBEDDING_BODY_BYTES,
        "Ollama returned invalid embeddings.",
    )
    .await?;
    let response: OllamaEmbedResponse = serde_json::from_slice(&body).map_err(|error| {
        tracing::error!(%error, "invalid Ollama embedding response");
        AppError::ai_request("Ollama returned invalid embeddings.")
    })?;
    validate_embedding_batch(&response.embeddings, texts.len(), expected_dimension)?;
    Ok(response.embeddings)
}

pub(crate) fn vector_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(vector));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub fn vector_from_blob(blob: &[u8], expected_dimension: usize) -> Option<Vec<f32>> {
    if expected_dimension == 0
        || expected_dimension > MAX_VECTOR_DIMENSION
        || blob.len() != expected_dimension.checked_mul(size_of::<f32>())?
    {
        return None;
    }
    let mut vector = Vec::with_capacity(expected_dimension);
    for bytes in blob.chunks_exact(size_of::<f32>()) {
        let value = f32::from_le_bytes(bytes.try_into().ok()?);
        if !value.is_finite() {
            return None;
        }
        vector.push(value);
    }
    Some(vector)
}

struct IndexingWork {
    generation: i64,
    chunks: Vec<IndexedChunk>,
}

fn start_indexing(state: &AppState, book_id: &str) -> AppResult<IndexingWork> {
    let mut connection = state.lock_db()?;
    let chunk_count = connection
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id = ?1",
            [book_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to count chunks before embedding");
            AppError::database()
        })?;
    let chunk_count = usize::try_from(chunk_count).map_err(|_| AppError::database())?;
    if chunk_count > MAX_CHUNKS_PER_BOOK {
        tracing::error!(
            book_id,
            chunk_count,
            "book chunk count exceeds embedding limit"
        );
        return Err(AppError::database());
    }

    let chunks = {
        let mut statement = connection
            .prepare("SELECT id, text FROM chunks WHERE book_id = ?1 ORDER BY id")
            .map_err(|error| {
                tracing::error!(%error, "failed to prepare chunks for embedding");
                AppError::database()
            })?;
        let rows = statement
            .query_map([book_id], |row| {
                let text = row.get_ref(1)?.as_str()?;
                if text.len() > 1_600 {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "persisted chunk exceeds its byte limit",
                        )),
                    ));
                }
                Ok(IndexedChunk {
                    id: row.get(0)?,
                    text: text.to_owned(),
                })
            })
            .map_err(|error| {
                tracing::error!(%error, "failed to read chunks for embedding");
                AppError::database()
            })?;
        let mut chunks = Vec::with_capacity(chunk_count);
        let mut text_chars = 0_usize;
        for row in rows {
            let chunk = row.map_err(|error| {
                tracing::error!(%error, "failed to decode chunk for embedding");
                AppError::database()
            })?;
            let chars = chunk.text.chars().count();
            if chars > 400 {
                return Err(AppError::database());
            }
            text_chars = text_chars
                .checked_add(chars)
                .ok_or_else(AppError::database)?;
            if text_chars > MAX_NORMALIZED_TEXT_CHARS + MAX_CHUNKS_PER_BOOK * 100 {
                return Err(AppError::database());
            }
            chunks.push(chunk);
        }
        chunks
    };
    if chunks.len() != chunk_count {
        return Err(AppError::database());
    }

    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin embedding reset");
        AppError::database()
    })?;
    let changed = transaction
        .execute(
            r#"
            UPDATE index_state
            SET status = 'embedding', embedding_model = ?2, vector_dimension = NULL,
                vector_count = 0, last_error = NULL, generation = generation + 1,
                updated_at = ?3
            WHERE book_id = ?1 AND EXISTS(SELECT 1 FROM books WHERE id = ?1)
            "#,
            params![book_id, state.embedding_model, Utc::now().to_rfc3339()],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to stage embedding state");
            AppError::database()
        })?;
    if changed != 1 {
        return Err(AppError::not_found("Book not found."));
    }
    transaction
        .execute(
            "UPDATE chunks SET embedding = NULL WHERE book_id = ?1",
            [book_id],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to clear prior embeddings");
            AppError::database()
        })?;
    let generation = transaction
        .query_row(
            "SELECT generation FROM index_state WHERE book_id = ?1",
            [book_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to read embedding generation");
            AppError::database()
        })?;
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to commit embedding reset");
        AppError::database()
    })?;
    Ok(IndexingWork { generation, chunks })
}

fn reset_to_text_ready(state: &AppState, book_id: &str, message: &str) -> AppResult<()> {
    let mut connection = state.lock_db()?;
    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin text-only index reset");
        AppError::database()
    })?;
    let changed = transaction
        .execute(
            r#"
            UPDATE index_state
            SET status = 'text_ready', embedding_model = NULL, vector_dimension = NULL,
                vector_count = 0, last_error = ?2, generation = generation + 1,
                updated_at = ?3
            WHERE book_id = ?1 AND EXISTS(SELECT 1 FROM books WHERE id = ?1)
            "#,
            params![book_id, message, Utc::now().to_rfc3339()],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to reset text-only index state");
            AppError::database()
        })?;
    if changed != 1 {
        return Err(AppError::not_found("Book not found."));
    }
    transaction
        .execute(
            "UPDATE chunks SET embedding = NULL WHERE book_id = ?1",
            [book_id],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to clear text-only index embeddings");
            AppError::database()
        })?;
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to commit text-only index reset");
        AppError::database()
    })?;
    Ok(())
}

fn fail_generation(
    state: &AppState,
    book_id: &str,
    generation: i64,
    message: &str,
) -> AppResult<bool> {
    let mut connection = state.lock_db()?;
    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin failed embedding cleanup");
        AppError::database()
    })?;
    let changed = transaction
        .execute(
            r#"
            UPDATE index_state
            SET status = 'text_ready', embedding_model = NULL, vector_dimension = NULL,
                vector_count = 0, last_error = ?3, updated_at = ?4
            WHERE book_id = ?1 AND generation = ?2 AND status = 'embedding'
            "#,
            params![book_id, generation, message, Utc::now().to_rfc3339()],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to reset failed embedding state");
            AppError::database()
        })?;
    if changed == 1 {
        transaction
            .execute(
                "UPDATE chunks SET embedding = NULL WHERE book_id = ?1",
                [book_id],
            )
            .map_err(|error| {
                tracing::error!(%error, "failed to clear failed embeddings");
                AppError::database()
            })?;
    }
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to commit failed embedding cleanup");
        AppError::database()
    })?;
    Ok(changed == 1)
}

fn store_embedding_batch(
    state: &AppState,
    book_id: &str,
    generation: i64,
    chunks: &[IndexedChunk],
    embeddings: &[Vec<f32>],
) -> AppResult<()> {
    let mut connection = state.lock_db()?;
    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin embedding update");
        AppError::database()
    })?;
    let active = transaction
        .query_row(
            "SELECT generation = ?2 AND status = 'embedding' FROM index_state WHERE book_id = ?1",
            params![book_id, generation],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to validate embedding generation");
            AppError::database()
        })?
        .unwrap_or(false);
    if !active {
        return Err(AppError::not_found(
            "The book index changed during embedding.",
        ));
    }
    for (chunk, embedding) in chunks.iter().zip(embeddings) {
        let changed = transaction
            .execute(
                "UPDATE chunks SET embedding = ?2 WHERE id = ?1 AND book_id = ?3",
                params![chunk.id, vector_blob(embedding), book_id],
            )
            .map_err(|error| {
                tracing::error!(%error, "failed to store chunk embedding");
                AppError::database()
            })?;
        if changed != 1 {
            return Err(AppError::not_found("The book changed during embedding."));
        }
    }
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to commit chunk embeddings");
        AppError::database()
    })?;
    Ok(())
}

fn publish_ready(
    state: &AppState,
    book_id: &str,
    generation: i64,
    vector_count: usize,
    vector_dimension: usize,
) -> AppResult<()> {
    let mut connection = state.lock_db()?;
    let transaction = connection.transaction().map_err(|error| {
        tracing::error!(%error, "failed to begin embedding publication");
        AppError::database()
    })?;
    let stored_count = transaction
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id = ?1 AND embedding IS NOT NULL",
            [book_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to verify stored embeddings");
            AppError::database()
        })?;
    if usize::try_from(stored_count).ok() != Some(vector_count) {
        return Err(AppError::database());
    }
    let changed = transaction
        .execute(
            r#"
            UPDATE index_state
            SET status = 'ready', embedding_model = ?3, vector_dimension = ?4,
                vector_count = ?5, last_error = NULL, updated_at = ?6
            WHERE book_id = ?1 AND generation = ?2 AND status = 'embedding'
              AND EXISTS(SELECT 1 FROM books WHERE id = ?1)
            "#,
            params![
                book_id,
                generation,
                state.embedding_model,
                vector_dimension as i64,
                vector_count as i64,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to publish embedding state");
            AppError::database()
        })?;
    if changed != 1 {
        return Err(AppError::not_found(
            "The book index changed during embedding.",
        ));
    }
    transaction.commit().map_err(|error| {
        tracing::error!(%error, "failed to commit embedding publication");
        AppError::database()
    })?;
    Ok(())
}

async fn index_book_with_models(
    state: &AppState,
    book_id: &str,
    models: &[OllamaModel],
) -> AppResult<bool> {
    if !model_is_installed(models, &state.embedding_model) {
        reset_to_text_ready(
            state,
            book_id,
            "The embedding model is not installed; keyword search remains ready.",
        )?;
        return Ok(false);
    }

    let work = start_indexing(state, book_id)?;
    if work.chunks.is_empty() {
        fail_generation(
            state,
            book_id,
            work.generation,
            "This book has no text chunks; keyword search remains ready.",
        )?;
        return Ok(false);
    }

    let mut vector_count = 0_usize;
    let mut vector_dimension = None;
    for batch in work.chunks.chunks(EMBEDDING_BATCH_SIZE) {
        let texts = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let embeddings = match embed_texts(state, &texts, vector_dimension).await {
            Ok(embeddings) => embeddings,
            Err(error) => {
                fail_generation(
                    state,
                    book_id,
                    work.generation,
                    "Embedding stopped; keyword search remains ready.",
                )?;
                tracing::warn!(kind = ?error.kind, "book embedding stopped");
                return Ok(false);
            }
        };
        let dimension = match validate_embedding_batch(&embeddings, batch.len(), vector_dimension) {
            Ok(dimension) => dimension,
            Err(error) => {
                fail_generation(
                    state,
                    book_id,
                    work.generation,
                    "Embedding stopped; keyword search remains ready.",
                )?;
                return Err(error);
            }
        };
        vector_dimension = Some(dimension);
        if let Err(error) =
            store_embedding_batch(state, book_id, work.generation, batch, &embeddings)
        {
            let _ = fail_generation(
                state,
                book_id,
                work.generation,
                "Embedding stopped; keyword search remains ready.",
            );
            return Err(error);
        }
        vector_count += batch.len();
    }
    if let Err(error) = publish_ready(
        state,
        book_id,
        work.generation,
        vector_count,
        vector_dimension.ok_or_else(|| AppError::ai_request("Ollama returned no embeddings."))?,
    ) {
        fail_generation(
            state,
            book_id,
            work.generation,
            "Embedding stopped; keyword search remains ready.",
        )?;
        return Err(error);
    }
    Ok(true)
}

pub async fn index_book(state: &AppState, book_id: &str) -> AppResult<bool> {
    let models = match installed_models(state).await {
        Ok(models) => models,
        Err(_) => {
            reset_to_text_ready(
                state,
                book_id,
                "Ollama is unavailable; keyword search remains ready.",
            )?;
            return Ok(false);
        }
    };
    index_book_with_models(state, book_id, &models).await
}

fn pending_index_books(state: &AppState) -> AppResult<Vec<String>> {
    let connection = state.lock_db()?;
    let count = connection
        .query_row("SELECT COUNT(*) FROM books", [], |row| row.get::<_, i64>(0))
        .map_err(|error| {
            tracing::error!(%error, "failed to count books for startup indexing");
            AppError::database()
        })?;
    if usize::try_from(count).map_or(true, |count| count > MAX_LIBRARY_BOOKS) {
        return Err(AppError::database());
    }
    let mut statement = connection
        .prepare(
            "SELECT book_id FROM index_state WHERE status IN ('text_ready', 'embedding') ORDER BY updated_at",
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to prepare startup index retry");
            AppError::database()
        })?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .and_then(Iterator::collect)
        .map_err(|error| {
            tracing::error!(%error, "failed to read startup index retry list");
            AppError::database()
        })
}

pub async fn retry_pending_indexes(state: AppState) -> AppResult<()> {
    let models = match installed_models(&state).await {
        Ok(models) => models,
        Err(_) => return Ok(()),
    };
    for book_id in pending_index_books(&state)? {
        let permit = state
            .ai_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::new(AppErrorKind::Internal, "AI work is unavailable."))?;
        if let Err(error) = index_book_with_models(&state, &book_id, &models).await {
            tracing::warn!(book_id, kind = ?error.kind, "startup index retry failed");
        }
        drop(permit);
    }
    Ok(())
}

pub async fn generate_stream<F>(
    state: &AppState,
    prompt: &str,
    system: &str,
    mut on_delta: F,
) -> AppResult<String>
where
    F: FnMut(String) -> AppResult<()>,
{
    let mut response = state
        .generation_http
        .post(format!("{OLLAMA_BASE_URL}/api/generate"))
        .json(&json!({
            "model": state.generation_model,
            "prompt": prompt,
            "system": system,
            "stream": true,
            "options": { "temperature": 0.3 }
        }))
        .send()
        .await
        .map_err(|error| {
            tracing::debug!(%error, "Ollama generation unavailable");
            AppError::ai_unavailable("The configured generation model is unavailable.")
        })?;
    if response.status() != StatusCode::OK {
        tracing::warn!(status = %response.status(), "Ollama generation failed");
        return Err(AppError::ai_unavailable(
            "The configured generation model is unavailable.",
        ));
    }

    let mut pending = Vec::<u8>::new();
    let mut stream = GenerationStream::default();
    while let Some(bytes) = response.chunk().await.map_err(|error| {
        tracing::error!(%error, "failed to read Ollama stream");
        AppError::ai_request("The AI response stream was interrupted.")
    })? {
        stream.add_bytes(bytes.len())?;
        if stream.done && !bytes.is_empty() {
            return Err(AppError::ai_request(
                "Ollama returned data after completing the response.",
            ));
        }
        pending.extend_from_slice(&bytes);
        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            if newline + 1 > MAX_STREAM_LINE_BYTES {
                return Err(AppError::ai_request(
                    "Ollama returned an oversized response line.",
                ));
            }
            let line = pending.drain(..=newline).collect::<Vec<_>>();
            stream.consume_line(&line, &mut on_delta)?;
            if stream.done && !pending.is_empty() {
                return Err(AppError::ai_request(
                    "Ollama returned data after completing the response.",
                ));
            }
        }
        if pending.len() > MAX_STREAM_LINE_BYTES {
            return Err(AppError::ai_request(
                "Ollama returned an oversized response line.",
            ));
        }
    }
    if !pending.is_empty() {
        stream.consume_line(&pending, &mut on_delta)?;
    }
    stream.finish()
}

#[derive(Default)]
struct GenerationStream {
    answer: String,
    answer_chars: usize,
    total_bytes: usize,
    done: bool,
}

impl GenerationStream {
    fn add_bytes(&mut self, count: usize) -> AppResult<()> {
        self.total_bytes = self
            .total_bytes
            .checked_add(count)
            .ok_or_else(|| AppError::ai_request("The AI response is too large."))?;
        if self.total_bytes > MAX_STREAM_BYTES {
            return Err(AppError::ai_request("The AI response is too large."));
        }
        Ok(())
    }

    fn consume_line<F>(&mut self, line: &[u8], on_delta: &mut F) -> AppResult<()>
    where
        F: FnMut(String) -> AppResult<()>,
    {
        if self.done {
            return Err(AppError::ai_request(
                "Ollama returned data after completing the response.",
            ));
        }
        if line.len() > MAX_STREAM_LINE_BYTES {
            return Err(AppError::ai_request(
                "Ollama returned an oversized response line.",
            ));
        }
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            return Ok(());
        }
        let chunk: OllamaGenerateChunk = serde_json::from_slice(line).map_err(|error| {
            tracing::error!(%error, "invalid Ollama stream chunk");
            AppError::ai_request("Ollama returned an invalid response stream.")
        })?;
        if !chunk.response.is_empty() {
            let delta_chars = chunk.response.chars().count();
            self.answer_chars = self
                .answer_chars
                .checked_add(delta_chars)
                .ok_or_else(|| AppError::ai_request("The AI answer is too long."))?;
            if self.answer_chars > MAX_ANSWER_CHARS {
                return Err(AppError::ai_request("The AI answer is too long."));
            }
            self.answer.push_str(&chunk.response);
            on_delta(chunk.response)?;
        }
        self.done = chunk.done;
        Ok(())
    }

    fn finish(self) -> AppResult<String> {
        if !self.done {
            return Err(AppError::ai_request(
                "The AI response ended before completion.",
            ));
        }
        if self.answer.is_empty() {
            return Err(AppError::ai_request("The AI returned an empty response."));
        }
        Ok(self.answer)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rusqlite::{params, Connection};

    use crate::db;

    use super::*;

    #[test]
    fn vector_blob_requires_exact_finite_bounded_dimension() {
        let vector = vec![0.25, -1.5, 3.0];
        let blob = vector_blob(&vector);
        assert_eq!(blob.len(), vector.len() * 4);
        assert_eq!(vector_from_blob(&blob, 3).unwrap(), vector);
        assert!(vector_from_blob(&blob, 2).is_none());
        assert!(vector_from_blob(&blob, MAX_VECTOR_DIMENSION + 1).is_none());

        let invalid = vector_blob(&[f32::NAN]);
        assert!(vector_from_blob(&invalid, 1).is_none());
    }

    #[test]
    fn embedding_validation_rejects_count_dimension_and_non_finite_values() {
        assert_eq!(
            validate_embedding_batch(&[vec![1.0, 2.0]], 1, None).unwrap(),
            2
        );
        assert!(validate_embedding_batch(&[vec![1.0]], 2, None).is_err());
        assert!(validate_embedding_batch(&[vec![1.0], vec![1.0, 2.0]], 2, None).is_err());
        assert!(validate_embedding_batch(&[vec![f32::INFINITY]], 1, None).is_err());
        assert!(validate_embedding_batch(&[vec![1.0]], 1, Some(2)).is_err());
    }

    #[test]
    fn model_matching_handles_latest_tags() {
        let models = vec![OllamaModel {
            name: "model:latest".to_owned(),
            model: "model:latest".to_owned(),
        }];
        assert!(model_is_installed(&models, "model"));
        assert!(model_is_installed(&models, "model:latest"));
        assert!(!model_is_installed(&models, "other"));
    }

    #[test]
    fn generation_stream_requires_done_and_rejects_data_after_done() {
        let mut stream = GenerationStream::default();
        let mut deltas = Vec::new();
        stream
            .consume_line(
                b"{\"response\":\"answer\",\"done\":false}\n",
                &mut |delta| {
                    deltas.push(delta);
                    Ok(())
                },
            )
            .unwrap();
        assert!(GenerationStream {
            done: false,
            ..Default::default()
        }
        .finish()
        .is_err());
        stream
            .consume_line(b"{\"response\":\"\",\"done\":true}\n", &mut |_| Ok(()))
            .unwrap();
        assert!(stream
            .consume_line(b"{\"response\":\"extra\",\"done\":true}\n", &mut |_| Ok(()))
            .is_err());
        assert_eq!(deltas, ["answer"]);
        assert_eq!(stream.finish().unwrap(), "answer");
    }

    #[test]
    fn failed_embedding_generation_clears_all_vectors_atomically() {
        let mut connection = Connection::open_in_memory().unwrap();
        db::configure_connection(&connection).unwrap();
        db::migrations().to_latest(&mut connection).unwrap();
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO books(id, source_hash, title, author, source_rel_path, content_length, total_locations, total_chapters, created_at, updated_at) VALUES ('book', 'hash', 'Title', 'Author', 'source', 1, 10, 1, ?1, ?1)",
            [&now],
        ).unwrap();
        connection.execute(
            "INSERT INTO chapters(id, book_id, title, chapter_order, content_rel_path, start_location, end_location, char_count) VALUES ('chapter', 'book', 'Chapter', 0, 'chapter', 0, 10, 10)",
            [],
        ).unwrap();
        for order in 0..2 {
            connection.execute(
                "INSERT INTO chunks(book_id, chapter_id, chunk_order, text, start_location, end_location, embedding) VALUES ('book', 'chapter', ?1, 'text', ?1, ?2, ?3)",
                params![order, order + 1, vector_blob(&[1.0, 0.0])],
            ).unwrap();
        }
        connection.execute(
            "INSERT INTO index_state(book_id, status, embedding_model, vector_dimension, vector_count, generation, updated_at) VALUES ('book', 'embedding', 'model', 2, 2, 5, ?1)",
            [&now],
        ).unwrap();
        let state = AppState::new(std::path::PathBuf::new(), connection).unwrap();

        assert!(!fail_generation(&state, "book", 4, "stale").unwrap());
        assert!(fail_generation(&state, "book", 5, "failed").unwrap());

        let connection = state.lock_db().unwrap();
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE book_id = 'book' AND embedding IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM index_state WHERE book_id = 'book'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(status, "text_ready");
    }
}
