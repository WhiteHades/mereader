# MeReader: Intelligent Offline-First eBook Reader

## Project Overview

MeReader is an innovative eBook reader that integrates a local AI assistant to enhance the reading experience. It addresses key limitations in traditional eBook readers by providing intelligent, context-aware assistance while maintaining complete privacy and preventing spoilers.

### The Problem

- Traditional eBook readers offer only basic search and annotation features
- General-purpose AI chatbots break reading immersion and risk revealing spoilers
- Most AI solutions require internet connectivity, raising privacy concerns
- Difficulty retaining complex narratives and character details in long books

### The Solution

An offline-first desktop eBook reader with an integrated local AI assistant that acts as an intelligent reading companion, designed to enhance rather than replace the reading experience.

## Core Features & Innovation

### Progress-Aware RAG

- AI knowledge is strictly limited to content the user has already read
- Prevents spoilers by filtering out unread portions
- Provides contextually relevant answers based on reading progress

### Complete Privacy & Offline Operation

- All components run entirely on the user's local machine
- No data sent to cloud services
- Uses local LLM via Ollama for complete independence

### Hybrid Semantic Search

- Combines vector-based semantic search with traditional keyword search (BM25)
- Query expansion for improved results
- Multi-search strategy including summaries

### Assistive Design

- User-initiated AI interaction - no proactive interruptions
- Maintains reading flow and immersion
- On-demand assistance for plot clarification and character details

## System Architecture

### High-Level Architecture

**Frontend:** Tauri + Vue.js desktop application
**Backend:** Python FastAPI REST API
**Local LLM:** Ollama serving llama3.2 and nomic-embed-text models
**Storage:** SQLite (metadata) + Qdrant (vectors) + File system (content)

### Component Architecture

| Component         | Technology              | Responsibility                                        |
| ----------------- | ----------------------- | ----------------------------------------------------- |
| Frontend          | Tauri, Vue.js, Pinia    | UI rendering, state management, API calls             |
| Backend API       | Python, FastAPI         | API logic, book processing, AI coordination           |
| RAG Service       | Custom Python           | Query processing, search orchestration, LLM prompting |
| Embedding Service | Ollama integration      | Text chunking, embedding generation, summaries        |
| Search Services   | Qdrant, BM25            | Vector search with progress filtering, keyword search |
| Book Processing   | EbookLib, BeautifulSoup | EPUB parsing, HTML cleaning, location calculation     |

### Design Patterns

- Singleton, Factory Method, Repository, Strategy
- Dependency Injection, Service Locator

## Technical Implementation

### Book Processing Workflow

1. User uploads EPUB file
2. Extract metadata, cover image, and chapter content
3. Create database records in SQLite
4. Clean and chunk text content
5. Generate vector embeddings via Ollama
6. Store vectors in Qdrant with location metadata
7. Build BM25 index for keyword search

### AI Query Workflow

1. User submits question from reader interface
2. Retrieve current reading progress location
3. Calculate location boundary to filter unread content
4. Expand query into multiple related sub-queries
5. Execute parallel searches:
   - Vector search in Qdrant (filtered by progress)
   - Keyword search using BM25 index
   - Vector search on pre-generated summaries
6. Combine, deduplicate, and re-rank results
7. Format top passages into context block
8. Send prompt to local LLM via Ollama
9. Return answer with source passages to user

### Progress-Aware Filtering

- Character-based location tracking
- Dynamic boundary computation based on reading progress
- Ensures AI only accesses previously read content

## User Interface

### Library View

- Card-based interface displaying books with covers
- Progress bars showing reading completion
- Book management and upload functionality

### Reader View

- Distraction-free reading interface
- Floating control panel for navigation
- Theme options (Light, Sepia, Dark)
- Integrated AI panel access

### AI Assistant Panel

- Side panel for user questions
- Displays AI responses with source passages
- Shows relevant excerpts used for answer generation
- Maintains context of current reading position

## System Requirements

- **OS:** Windows 10 22H2+, Linux, macOS
- **CPU:** Intel Core i5 / AMD Ryzen 5 or newer
- **RAM:** 8GB minimum (16GB+ recommended)
- **GPU:** Optional for faster AI responses
- **Dependencies:** Python 3.8+, Node.js 16+, Ollama

## Performance & Evaluation (to be done more later)

### Model Comparison

Tested models: qwen3:4b, llama3.2:1b, llama3.2:latest, qwen3:1.7b, gemma3:1b

### Evaluation Metrics

- **Quantitative:** Precision, Recall, F1 Score, BERT Score, Cosine Similarity
- **Qualitative:** Accuracy, Relevance, Coherence, Hallucination control
- **Efficiency:** Query processing time, memory usage

### Key Results

- **Best Model:** llama3.2:latest (composite score: 0.72)
- **Precision:** 7.93/10, **F1 Score:** 7.95/10
- **Hallucination Control:** 7.3/10 (inverted scale)
- **Query Time:** Median 60-70 seconds
- **Storage:** ~3KB per vector embedding

### Validation

- Effective semantic retrieval with high precision/recall
- RAG system successfully grounds responses
- Low hallucination rates prove system reliability
- Feasible performance on consumer hardware

## Testing

### Backend Testing

- pytest and unittest framework
- Mocked external services (Ollama, file system)
- Unit tests for services, integration tests for APIs

### Frontend Testing

- vitest and Vue Test Utils
- Component, state management, and API service testing
- Mocked Tauri and browser APIs
- Full UI interaction testing
