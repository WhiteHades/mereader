use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};
use crate::models::SourcePassage;
use crate::ollama::vector_from_blob;

const RRF_K: f64 = 60.0;
const CANDIDATE_LIMIT: usize = 20;
const RESULT_LIMIT: usize = 8;

#[derive(Debug, Clone)]
struct Candidate {
    id: i64,
    chapter_id: String,
    chapter_title: String,
    text: String,
    start_location: u64,
    end_location: u64,
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f64> {
    if left.is_empty() || left.len() != right.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left, right) in left.iter().zip(right) {
        let left = f64::from(*left);
        let right = f64::from(*right);
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }
    Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
}

pub fn reciprocal_rank_fusion(rankings: &[Vec<i64>]) -> HashMap<i64, f64> {
    let mut scores = HashMap::new();
    for ranking in rankings {
        for (index, id) in ranking.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) += 1.0 / (RRF_K + index as f64 + 1.0);
        }
    }
    scores
}

fn fts_query(question: &str) -> Option<String> {
    let mut seen = HashSet::new();
    let terms = question
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() > 1)
        .map(str::to_lowercase)
        .filter(|term| seen.insert(term.clone()))
        .take(12)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Candidate> {
    let start: i64 = row.get(4)?;
    let end: i64 = row.get(5)?;
    Ok(Candidate {
        id: row.get(0)?,
        chapter_id: row.get(1)?,
        chapter_title: row.get(2)?,
        text: row.get(3)?,
        start_location: u64::try_from(start).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        end_location: u64::try_from(end).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
    })
}

pub fn retrieve(
    connection: &Connection,
    book_id: &str,
    question: &str,
    location_boundary: u64,
    query_embedding: Option<&[f32]>,
) -> AppResult<(String, Vec<SourcePassage>)> {
    let book_title = connection
        .query_row("SELECT title FROM books WHERE id = ?1", [book_id], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read retrieval book");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Book not found."))?;
    let boundary = i64::try_from(location_boundary)
        .map_err(|_| AppError::invalid("Reading location is too large."))?;

    let mut candidates = HashMap::<i64, Candidate>::new();
    let mut rankings = Vec::<(&str, Vec<i64>)>::new();
    if let Some(query) = fts_query(question) {
        let mut statement = connection
            .prepare(
                r#"
                SELECT c.id, c.chapter_id, ch.title, c.text,
                       c.start_location, c.end_location
                FROM chunks_fts f
                JOIN chunks c ON c.id = f.rowid
                JOIN chapters ch ON ch.id = c.chapter_id
                WHERE chunks_fts MATCH ?1 AND c.book_id = ?2 AND c.end_location <= ?3
                ORDER BY bm25(chunks_fts), c.end_location DESC
                LIMIT ?4
                "#,
            )
            .map_err(|error| {
                tracing::error!(%error, "failed to prepare FTS retrieval");
                AppError::database()
            })?;
        let results = statement
            .query_map(
                params![query, book_id, boundary, CANDIDATE_LIMIT as i64],
                candidate_from_row,
            )
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(|error| {
                tracing::error!(%error, "failed to execute FTS retrieval");
                AppError::database()
            })?;
        rankings.push((
            "fts",
            results.iter().map(|candidate| candidate.id).collect(),
        ));
        candidates.extend(
            results
                .into_iter()
                .map(|candidate| (candidate.id, candidate)),
        );
    }

    if let Some(query_embedding) = query_embedding {
        let mut statement = connection
            .prepare(
                r#"
                SELECT c.id, c.chapter_id, ch.title, c.text,
                       c.start_location, c.end_location, c.embedding
                FROM chunks c
                JOIN chapters ch ON ch.id = c.chapter_id
                WHERE c.book_id = ?1 AND c.end_location <= ?2 AND c.embedding IS NOT NULL
                "#,
            )
            .map_err(|error| {
                tracing::error!(%error, "failed to prepare vector retrieval");
                AppError::database()
            })?;
        let rows = statement
            .query_map(params![book_id, boundary], |row| {
                let candidate = candidate_from_row(row)?;
                let blob: Vec<u8> = row.get(6)?;
                Ok((candidate, blob))
            })
            .map_err(|error| {
                tracing::error!(%error, "failed to execute vector retrieval");
                AppError::database()
            })?;
        let mut vector_results = Vec::new();
        for row in rows {
            let (candidate, blob) = row.map_err(|error| {
                tracing::error!(%error, "failed to read vector candidate");
                AppError::database()
            })?;
            let Some(vector) = vector_from_blob(&blob) else {
                continue;
            };
            let Some(score) = cosine_similarity(query_embedding, &vector) else {
                continue;
            };
            vector_results.push((candidate, score));
        }
        vector_results.sort_by(|left, right| right.1.total_cmp(&left.1));
        vector_results.truncate(CANDIDATE_LIMIT);
        rankings.push((
            "vector",
            vector_results
                .iter()
                .map(|(candidate, _)| candidate.id)
                .collect(),
        ));
        candidates.extend(
            vector_results
                .into_iter()
                .map(|(candidate, _)| (candidate.id, candidate)),
        );
    }

    let ranking_ids = rankings
        .iter()
        .map(|(_, ranking)| ranking.clone())
        .collect::<Vec<_>>();
    let scores = reciprocal_rank_fusion(&ranking_ids);
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    ranked.truncate(RESULT_LIMIT);
    let sources = ranked
        .into_iter()
        .filter_map(|(id, score)| {
            let candidate = candidates.remove(&id)?;
            let retrieval_methods = rankings
                .iter()
                .filter(|(_, ranking)| ranking.contains(&id))
                .map(|(method, _)| (*method).to_owned())
                .collect();
            Some(SourcePassage {
                chapter_id: candidate.chapter_id,
                chapter_title: candidate.chapter_title,
                text: candidate.text,
                start_location: candidate.start_location,
                end_location: candidate.end_location,
                relevance_score: score,
                retrieval_methods,
            })
        })
        .collect();
    Ok((book_title, sources))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rusqlite::Connection;

    use crate::db::{configure_connection, migrations};

    use super::*;

    #[test]
    fn cosine_handles_normal_and_invalid_vectors() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 0.0]), None);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), None);
    }

    #[test]
    fn rrf_rewards_candidates_present_in_both_rankings() {
        let scores = reciprocal_rank_fusion(&[vec![1, 2, 3], vec![3, 1, 4]]);
        assert!(scores[&1] > scores[&2]);
        assert!(scores[&3] > scores[&4]);
    }

    #[test]
    fn retrieval_excludes_chunks_past_progress_boundary() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();
        migrations().to_latest(&mut connection).unwrap();
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                r#"
                INSERT INTO books(
                    id, source_hash, title, author, source_rel_path, content_length,
                    total_locations, total_chapters, created_at, updated_at
                ) VALUES ('book', 'hash', 'Title', 'Author', 'books/book/source.epub', 1, 200, 1, ?1, ?1)
                "#,
                [&now],
            )
            .unwrap();
        connection
            .execute(
                r#"
                INSERT INTO chapters(
                    id, book_id, title, chapter_order, content_rel_path,
                    start_location, end_location, char_count
                ) VALUES ('chapter', 'book', 'Chapter', 0, 'books/book/chapter.html', 0, 200, 200)
                "#,
                [],
            )
            .unwrap();
        for (text, start, end) in [
            ("boundary visible keyword", 0_i64, 100_i64),
            ("boundary hidden keyword", 1_i64, 101_i64),
        ] {
            connection
                .execute(
                    r#"
                    INSERT INTO chunks(book_id, chapter_id, chunk_order, text, start_location, end_location)
                    VALUES ('book', 'chapter', ?1, ?2, ?3, ?4)
                    "#,
                    params![start, text, start, end],
                )
                .unwrap();
            let id = connection.last_insert_rowid();
            connection
                .execute(
                    "INSERT INTO chunks_fts(rowid, text) VALUES (?1, ?2)",
                    params![id, text],
                )
                .unwrap();
        }

        let (_, sources) = retrieve(&connection, "book", "boundary keyword", 100, None).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].end_location, 100);
        assert!(!sources[0].text.contains("hidden"));
    }
}
