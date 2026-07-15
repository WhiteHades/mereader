use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub progress: Option<Progress>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookList {
    pub books: Vec<BookSummary>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterSummary {
    pub id: String,
    pub title: String,
    pub order: u32,
    pub start_location: u64,
    pub end_location: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub progress: Option<Progress>,
    pub has_cover: bool,
    pub language: Option<String>,
    pub published_year: Option<i32>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub description: Option<String>,
    pub content_length: u64,
    pub total_locations: u64,
    pub total_chapters: u32,
    pub chapters: Vec<ChapterSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverData {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterContent {
    pub book_id: String,
    pub chapter_id: String,
    pub title: String,
    pub order: u32,
    pub start_location: u64,
    pub end_location: u64,
    pub html: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub book_id: String,
    pub current_location: u64,
    pub current_chapter_id: Option<String>,
    pub completion_percentage: f64,
    pub last_read_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub state: String,
    pub message: Option<String>,
    pub indexed_through_location: Option<u64>,
    pub total_locations: Option<u64>,
    pub available: bool,
    pub generation_model: String,
    pub embedding_model: String,
    pub generation_model_available: bool,
    pub embedding_model_available: bool,
    pub installed_models: Vec<String>,
    pub indexed_books: u64,
    pub text_only_books: u64,
    pub failed_books: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePassage {
    pub citation_id: String,
    pub chapter_id: String,
    pub chapter_title: String,
    pub text: String,
    pub start_location: u64,
    pub end_location: u64,
    pub relevance_score: f64,
    pub retrieval_methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerResponse {
    pub answer: String,
    pub question: String,
    pub book_id: String,
    pub book_title: String,
    pub sources: Vec<SourcePassage>,
    pub progress_boundary: Option<ProgressBoundary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressBoundary {
    pub current_location: u64,
    pub completion_percentage: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AnswerEvent {
    Status { message: String },
    Delta { delta: String },
    Complete { response: AnswerResponse },
    Error { message: String },
}

#[derive(Debug, Deserialize)]
pub struct OllamaTagsResponse {
    #[serde(default)]
    pub models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaModel {
    pub name: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct OllamaEmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaGenerateChunk {
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct IndexedChunk {
    pub id: i64,
    pub text: String,
}
