use chrono::Utc;
use reqwest::StatusCode;
use rusqlite::params;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::models::{
    IndexedChunk, OllamaEmbedResponse, OllamaGenerateChunk, OllamaModel, OllamaTagsResponse,
};
use crate::state::{AppState, OLLAMA_BASE_URL};

const EMBEDDING_BATCH_SIZE: usize = 32;

pub async fn installed_models(state: &AppState) -> AppResult<Vec<OllamaModel>> {
    let response = state
        .http
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
    response
        .json::<OllamaTagsResponse>()
        .await
        .map(|response| response.models)
        .map_err(|error| {
            tracing::error!(%error, "invalid Ollama model listing response");
            AppError::ai_request("Ollama returned an invalid status response.")
        })
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

pub async fn embed_texts(state: &AppState, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let response = state
        .http
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
    let response = response
        .json::<OllamaEmbedResponse>()
        .await
        .map_err(|error| {
            tracing::error!(%error, "invalid Ollama embedding response");
            AppError::ai_request("Ollama returned invalid embeddings.")
        })?;
    if response.embeddings.len() != texts.len()
        || response
            .embeddings
            .iter()
            .any(|embedding| embedding.is_empty())
    {
        return Err(AppError::ai_request(
            "Ollama returned an incomplete embedding response.",
        ));
    }
    Ok(response.embeddings)
}

fn vector_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(vector));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub fn vector_from_blob(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.is_empty() || !blob.len().is_multiple_of(size_of::<f32>()) {
        return None;
    }
    Some(
        blob.chunks_exact(size_of::<f32>())
            .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("four-byte chunk")))
            .collect(),
    )
}

fn set_index_state(
    state: &AppState,
    book_id: &str,
    status: &str,
    vector_count: usize,
    last_error: Option<&str>,
) -> AppResult<()> {
    let connection = state.lock_db()?;
    connection
        .execute(
            r#"
            UPDATE index_state
            SET status = ?2, embedding_model = ?3, vector_count = ?4,
                last_error = ?5, updated_at = ?6
            WHERE book_id = ?1
            "#,
            params![
                book_id,
                status,
                state.embedding_model,
                vector_count as i64,
                last_error,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to update index state");
            AppError::database()
        })?;
    Ok(())
}

pub fn mark_index_failed(state: &AppState, book_id: &str, message: &str) -> AppResult<()> {
    set_index_state(state, book_id, "failed", 0, Some(message))
}

pub async fn embed_imported_book(
    state: &AppState,
    book_id: &str,
    chunks: &[IndexedChunk],
) -> AppResult<bool> {
    let models = match installed_models(state).await {
        Ok(models) => models,
        Err(_) => {
            set_index_state(
                state,
                book_id,
                "text_ready",
                0,
                Some("Ollama is unavailable; keyword search remains ready."),
            )?;
            return Ok(false);
        }
    };
    if !model_is_installed(&models, &state.embedding_model) {
        set_index_state(
            state,
            book_id,
            "text_ready",
            0,
            Some("The embedding model is not installed; keyword search remains ready."),
        )?;
        return Ok(false);
    }

    set_index_state(state, book_id, "embedding", 0, None)?;
    let mut vector_count = 0_usize;
    for batch in chunks.chunks(EMBEDDING_BATCH_SIZE) {
        let texts = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let embeddings = match embed_texts(state, &texts).await {
            Ok(embeddings) => embeddings,
            Err(error) => {
                set_index_state(
                    state,
                    book_id,
                    "text_ready",
                    vector_count,
                    Some("Embedding stopped; keyword search remains ready."),
                )?;
                tracing::warn!(kind = ?error.kind, "book embedding stopped");
                return Ok(false);
            }
        };
        let mut connection = state.lock_db()?;
        let transaction = connection.transaction().map_err(|error| {
            tracing::error!(%error, "failed to begin embedding update");
            AppError::database()
        })?;
        for (chunk, embedding) in batch.iter().zip(embeddings) {
            transaction
                .execute(
                    "UPDATE chunks SET embedding = ?2 WHERE id = ?1 AND book_id = ?3",
                    params![chunk.id, vector_blob(&embedding), book_id],
                )
                .map_err(|error| {
                    tracing::error!(%error, "failed to store chunk embedding");
                    AppError::database()
                })?;
            vector_count += 1;
        }
        transaction.commit().map_err(|error| {
            tracing::error!(%error, "failed to commit chunk embeddings");
            AppError::database()
        })?;
    }
    set_index_state(state, book_id, "ready", vector_count, None)?;
    Ok(true)
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
        .http
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
    let mut answer = String::new();
    while let Some(bytes) = response.chunk().await.map_err(|error| {
        tracing::error!(%error, "failed to read Ollama stream");
        AppError::ai_request("The AI response stream was interrupted.")
    })? {
        pending.extend_from_slice(&bytes);
        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let line = pending.drain(..=newline).collect::<Vec<_>>();
            consume_generation_line(&line, &mut answer, &mut on_delta)?;
        }
    }
    if !pending.is_empty() {
        consume_generation_line(&pending, &mut answer, &mut on_delta)?;
    }
    if answer.is_empty() {
        return Err(AppError::ai_request("The AI returned an empty response."));
    }
    Ok(answer)
}

fn consume_generation_line<F>(line: &[u8], answer: &mut String, on_delta: &mut F) -> AppResult<()>
where
    F: FnMut(String) -> AppResult<()>,
{
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
        answer.push_str(&chunk.response);
        on_delta(chunk.response)?;
    }
    let _ = chunk.done;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_blob_round_trip_is_compact() {
        let vector = vec![0.25, -1.5, 3.0];
        let blob = vector_blob(&vector);
        assert_eq!(blob.len(), vector.len() * 4);
        assert_eq!(vector_from_blob(&blob).unwrap(), vector);
        assert!(vector_from_blob(&blob[..blob.len() - 1]).is_none());
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
}
