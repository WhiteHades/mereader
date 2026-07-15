use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};

use rusqlite::{params, types::Type, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};
use crate::limits::{
    MAX_CHUNKS_PER_BOOK, MAX_NORMALIZED_TEXT_CHARS, MAX_SPINE_CHAPTERS, MAX_VECTOR_DIMENSION,
};
use crate::models::SourcePassage;
use crate::ollama::vector_from_blob;

const RRF_K: f64 = 60.0;
const CANDIDATE_LIMIT: usize = 20;
const RESULT_LIMIT: usize = 8;
const MIN_COSINE_SIMILARITY: f64 = 0.20;
const MAX_CHUNK_TEXT_BYTES: usize = 1_600;

#[derive(Debug, Clone)]
struct Candidate {
    id: i64,
    chapter_id: String,
    chapter_title: String,
    text: String,
    start_location: u64,
    end_location: u64,
}

#[derive(Debug)]
struct ScoredId {
    id: i64,
    score: f64,
}

impl PartialEq for ScoredId {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.score.to_bits() == other.score.to_bits()
    }
}

impl Eq for ScoredId {}

impl PartialOrd for ScoredId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoredId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.id.cmp(&self.id))
    }
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f64> {
    if left.is_empty()
        || left.len() != right.len()
        || left.iter().chain(right).any(|value| !value.is_finite())
    {
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
    let similarity = dot / (left_norm.sqrt() * right_norm.sqrt());
    similarity.is_finite().then_some(similarity)
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

fn bounded_string(
    row: &rusqlite::Row<'_>,
    index: usize,
    maximum: usize,
) -> rusqlite::Result<String> {
    let value = row.get_ref(index)?.as_str()?;
    if value.len() > maximum {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "persisted text exceeds its limit",
            )),
        ));
    }
    Ok(value.to_owned())
}

fn candidate_by_id(
    connection: &Connection,
    book_id: &str,
    boundary: i64,
    id: i64,
) -> AppResult<Option<Candidate>> {
    connection
        .query_row(
            r#"
            SELECT c.id, c.chapter_id, ch.title, c.text, c.start_location, c.end_location
            FROM chunks c
            JOIN chapters ch ON ch.id = c.chapter_id
            WHERE c.id = ?1 AND c.book_id = ?2 AND c.end_location <= ?3
            "#,
            params![id, book_id, boundary],
            |row| {
                let start = u64::try_from(row.get::<_, i64>(4)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(4, Type::Integer, Box::new(error))
                })?;
                let end = u64::try_from(row.get::<_, i64>(5)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(5, Type::Integer, Box::new(error))
                })?;
                let text = bounded_string(row, 3, MAX_CHUNK_TEXT_BYTES)?;
                if text.chars().count() > 400 || end <= start {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        3,
                        Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "persisted chunk is invalid",
                        )),
                    ));
                }
                Ok(Candidate {
                    id: row.get(0)?,
                    chapter_id: bounded_string(row, 1, 64)?,
                    chapter_title: bounded_string(row, 2, 800)?,
                    text,
                    start_location: start,
                    end_location: end,
                })
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to fetch selected retrieval candidate");
            AppError::database()
        })
}

fn vector_ranking(
    connection: &Connection,
    book_id: &str,
    boundary: i64,
    embedding_model: &str,
    query_embedding: &[f32],
    chunk_count: usize,
) -> AppResult<Vec<i64>> {
    if query_embedding.is_empty()
        || query_embedding.len() > MAX_VECTOR_DIMENSION
        || query_embedding.iter().any(|value| !value.is_finite())
    {
        return Ok(Vec::new());
    }
    let index = connection
        .query_row(
            r#"
            SELECT vector_dimension, vector_count
            FROM index_state
            WHERE book_id = ?1 AND status = 'ready' AND embedding_model = ?2
            "#,
            params![book_id, embedding_model],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read vector index state");
            AppError::database()
        })?;
    let Some((Some(dimension), vector_count)) = index else {
        return Ok(Vec::new());
    };
    let dimension = usize::try_from(dimension).map_err(|_| AppError::database())?;
    let vector_count = usize::try_from(vector_count).map_err(|_| AppError::database())?;
    if dimension != query_embedding.len()
        || dimension > MAX_VECTOR_DIMENSION
        || vector_count > chunk_count
        || vector_count > MAX_CHUNKS_PER_BOOK
    {
        tracing::error!(book_id, "persisted vector index metadata is invalid");
        return Err(AppError::database());
    }

    let mut statement = connection
        .prepare(
            r#"
            SELECT id, embedding
            FROM chunks
            WHERE book_id = ?1 AND end_location <= ?2 AND embedding IS NOT NULL
            "#,
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to prepare vector retrieval");
            AppError::database()
        })?;
    let rows = statement
        .query_map(params![book_id, boundary], |row| {
            let id = row.get::<_, i64>(0)?;
            let blob = row.get_ref(1)?.as_blob()?;
            let score = vector_from_blob(blob, dimension)
                .and_then(|vector| cosine_similarity(query_embedding, &vector))
                .filter(|score| *score >= MIN_COSINE_SIMILARITY);
            Ok(score.map(|score| ScoredId { id, score }))
        })
        .map_err(|error| {
            tracing::error!(%error, "failed to execute vector retrieval");
            AppError::database()
        })?;
    let mut top = BinaryHeap::<Reverse<ScoredId>>::with_capacity(CANDIDATE_LIMIT + 1);
    for row in rows {
        let Some(scored) = row.map_err(|error| {
            tracing::error!(%error, "failed to read vector candidate");
            AppError::database()
        })?
        else {
            continue;
        };
        if top.len() < CANDIDATE_LIMIT {
            top.push(Reverse(scored));
        } else if top
            .peek()
            .is_some_and(|minimum| scored.cmp(&minimum.0) == Ordering::Greater)
        {
            top.pop();
            top.push(Reverse(scored));
        }
    }
    let mut ranked = top.into_iter().map(|entry| entry.0).collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(ranked.into_iter().map(|entry| entry.id).collect())
}

pub fn retrieve(
    connection: &Connection,
    book_id: &str,
    question: &str,
    location_boundary: u64,
    query_embedding: Option<&[f32]>,
    embedding_model: &str,
) -> AppResult<(String, Vec<SourcePassage>)> {
    let (book_title, total_locations) = connection
        .query_row(
            "SELECT title, total_locations FROM books WHERE id = ?1",
            [book_id],
            |row| {
                Ok((
                    bounded_string(row, 0, 4_000)?,
                    u64::try_from(row.get::<_, i64>(1)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(1, Type::Integer, Box::new(error))
                    })?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            tracing::error!(%error, "failed to read retrieval book");
            AppError::database()
        })?
        .ok_or_else(|| AppError::not_found("Book not found."))?;
    if total_locations > (MAX_NORMALIZED_TEXT_CHARS + MAX_SPINE_CHAPTERS) as u64
        || location_boundary > total_locations
    {
        return Err(AppError::database());
    }
    let boundary = i64::try_from(location_boundary)
        .map_err(|_| AppError::invalid("Reading location is too large."))?;
    let chunk_count = connection
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE book_id = ?1",
            [book_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            tracing::error!(%error, "failed to count retrieval chunks");
            AppError::database()
        })?;
    let chunk_count = usize::try_from(chunk_count).map_err(|_| AppError::database())?;
    if chunk_count > MAX_CHUNKS_PER_BOOK {
        tracing::error!(
            book_id,
            chunk_count,
            "book chunk count exceeds retrieval limit"
        );
        return Err(AppError::database());
    }

    let mut rankings = Vec::<(&str, Vec<i64>)>::new();
    if let Some(query) = fts_query(question) {
        let mut statement = connection
            .prepare(
                r#"
                SELECT c.id
                FROM chunks_fts f
                JOIN chunks c ON c.id = f.rowid
                WHERE chunks_fts MATCH ?1 AND c.book_id = ?2 AND c.end_location <= ?3
                ORDER BY bm25(chunks_fts), c.end_location DESC, c.id
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
                |row| row.get::<_, i64>(0),
            )
            .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
            .map_err(|error| {
                tracing::error!(%error, "failed to execute FTS retrieval");
                AppError::database()
            })?;
        rankings.push(("fts", results));
    }
    if let Some(query_embedding) = query_embedding {
        let ranking = vector_ranking(
            connection,
            book_id,
            boundary,
            embedding_model,
            query_embedding,
            chunk_count,
        )?;
        if !ranking.is_empty() {
            rankings.push(("vector", ranking));
        }
    }

    let ranking_ids = rankings
        .iter()
        .map(|(_, ranking)| ranking.clone())
        .collect::<Vec<_>>();
    let scores = reciprocal_rank_fusion(&ranking_ids);
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.truncate(RESULT_LIMIT);

    let mut sources = Vec::with_capacity(ranked.len());
    for (id, score) in ranked {
        let Some(candidate) = candidate_by_id(connection, book_id, boundary, id)? else {
            continue;
        };
        let retrieval_methods = rankings
            .iter()
            .filter(|(_, ranking)| ranking.contains(&candidate.id))
            .map(|(method, _)| (*method).to_owned())
            .collect();
        sources.push(SourcePassage {
            citation_id: format!("S{}", sources.len() + 1),
            chapter_id: candidate.chapter_id,
            chapter_title: candidate.chapter_title,
            text: candidate.text,
            start_location: candidate.start_location,
            end_location: candidate.end_location,
            relevance_score: score,
            retrieval_methods,
        });
    }
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
        assert_eq!(cosine_similarity(&[f32::NAN], &[1.0]), None);
    }

    #[test]
    fn rrf_rewards_candidates_present_in_both_rankings() {
        let scores = reciprocal_rank_fusion(&[vec![1, 2, 3], vec![3, 1, 4]]);
        assert!(scores[&1] > scores[&2]);
        assert!(scores[&3] > scores[&4]);
    }

    #[test]
    fn retrieval_excludes_chunks_past_progress_boundary_and_assigns_citations() {
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

        let (_, sources) =
            retrieve(&connection, "book", "boundary keyword", 100, None, "model").unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].citation_id, "S1");
        assert_eq!(sources[0].end_location, 100);
        assert!(!sources[0].text.contains("hidden"));
    }

    #[test]
    fn vector_retrieval_requires_ready_matching_index_and_similarity_floor() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();
        migrations().to_latest(&mut connection).unwrap();
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO books(id, source_hash, title, author, source_rel_path, content_length, total_locations, total_chapters, created_at, updated_at) VALUES ('book', 'hash', 'Title', 'Author', 'source', 1, 10, 1, ?1, ?1)",
            [&now],
        ).unwrap();
        connection.execute(
            "INSERT INTO chapters(id, book_id, title, chapter_order, content_rel_path, start_location, end_location, char_count) VALUES ('chapter', 'book', 'Chapter', 0, 'chapter', 0, 10, 10)",
            [],
        ).unwrap();
        connection.execute(
            "INSERT INTO chunks(book_id, chapter_id, chunk_order, text, start_location, end_location, embedding) VALUES ('book', 'chapter', 0, 'relevant text', 0, 10, ?1)",
            [crate::ollama::vector_blob(&[1.0, 0.0])],
        ).unwrap();
        connection.execute(
            "INSERT INTO index_state(book_id, status, embedding_model, vector_dimension, vector_count, updated_at) VALUES ('book', 'ready', 'model', 2, 1, ?1)",
            [&now],
        ).unwrap();

        let (_, sources) = retrieve(
            &connection,
            "book",
            "unmatched",
            10,
            Some(&[1.0, 0.0]),
            "model",
        )
        .unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].retrieval_methods, ["vector"]);

        let (_, below_floor) = retrieve(
            &connection,
            "book",
            "unmatched",
            10,
            Some(&[0.0, 1.0]),
            "model",
        )
        .unwrap();
        assert!(below_floor.is_empty());
    }
}
