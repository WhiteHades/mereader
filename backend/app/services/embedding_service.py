"""
MeReader Embedding Service for generating and managing text embeddings
"""
import asyncio
import logging
import uuid
import time
import os
import gc
import pickle
import json
import nltk
from nltk.tokenize import word_tokenize
from rank_bm25 import BM25Okapi

from typing import List, Dict, Any
from sqlalchemy.orm import Session

from app.db.qdrant import qdrant_manager
from app.core.config import settings
from app.core.exceptions import VectorStoreException
from app.services.ollama_service import ollama_service
from app.db.sqlite import get_current_session, SessionLocal
from app.services.location_service import location_service
from app.services.text_extraction_utility import text_extraction_util
from app.db.models import Book, Chapter

logger = logging.getLogger(__name__)

class EmbeddingService:
    """Service for generating and managing text embeddings"""

    def __init__(self):
        logger.info("Embedding service initialised")
        pass

    async def embed_book_content(self, book_id: str, db_session: Session = None) -> int:
        """
        Process and embed book content from chapter files
        Args:
            book_id: ID of the book
            db_session: database session
        Returns:
            Number of chunks embedded
        """
        try:
            if qdrant_manager.has_vectors_for_book(book_id):
                logger.info(f"Embeddings already exist for book {book_id}, skipping embedding.")
                return 0

            logger.info(f"Starting optimized embedding process for book {book_id}")

            close_session = False
            if db_session is None:
                try: db_session = get_current_session()
                except Exception:
                    db_session = SessionLocal()
                    close_session = True

            try:
                book = db_session.query(Book).filter(Book.id == book_id).first()
                if not book: raise VectorStoreException(f"Book with id {book_id} not found")

                content_dir = book.content_path
                if not content_dir: raise VectorStoreException(f"Content directory not found for book {book_id}")

                chapters = db_session.query(Chapter).filter(Chapter.book_id == book_id).order_by(Chapter.order).all()
                if not chapters: raise VectorStoreException(f"No chapters found for book {book_id}")

                chunk_size = settings.CHUNK_SIZE
                chunk_overlap = settings.CHUNK_OVERLAP
                max_batch_size = 120
                summary_interval = 11

                total_embedded = 0
                start_time = time.time()

                # BM25 indexing
                all_chunks = []
                all_metadata = []

                # location for summaries
                current_location = 1
                location_text_buffer = ""
                last_summary_location = 0

                for chapter in chapters:
                    if not chapter.content_path or not os.path.exists(chapter.content_path):
                        logger.warning(f"Chapter content not found: {chapter.id}")
                        continue

                    try:
                        file_size = os.path.getsize(chapter.content_path)
                        if file_size < 50:
                            logger.info(f"Skipping empty/small chapter: {chapter.title} (size: {file_size} bytes)")
                            continue

                        logger.info(
                            f"Processing chapter: {chapter.title} (id: {chapter.id}, order: {chapter.order}, size: {file_size / 1024:.1f}kb)")

                        chunk_generator = text_extraction_util.chunk_text_streamed(
                            chapter.content_path,
                            chunk_size=int(chunk_size),
                            chunk_overlap=chunk_overlap,
                            min_chunk_size=100
                        )
                        chunks_processed = 0

                        for batch in text_extraction_util.batch_chunks(chunk_generator, max_batch_size):
                            chunks_processed += len(batch)
                            if chunks_processed % 50 == 0:
                                logger.info(f"Processed {chunks_processed} chunks from chapter {chapter.title}")

                            batch_metadata = []
                            start_location = chapter.start_location
                            end_location = chapter.end_location

                            for i, chunk_text in enumerate(batch):
                                segment_size = (end_location - start_location) / (len(batch) + 1)
                                location = int(start_location + segment_size * (i + 1))
                                location = max(start_location, min(end_location, location))

                                location_text_buffer += chunk_text + " "

                                if (location - last_summary_location) >= summary_interval:
                                    await self._create_location_summary(
                                        book_id,
                                        location,
                                        location_text_buffer,
                                        chapter.title,
                                        book.total_locations or 100
                                    )
                                    last_summary_location = location
                                    location_text_buffer = ""

                                metadata = {
                                    'chapter_id': chapter.id,
                                    'chapter_title': chapter.title,
                                    'chapter_order': chapter.order,
                                    'book_id': book_id,
                                    'location': location,
                                    'completion_percentage': location_service.get_percentage_from_location(location, book.total_locations or 100),
                                    'text': chunk_text,
                                    'content_type': 'content'
                                }

                                batch_metadata.append(metadata)
                                all_chunks.append(chunk_text)
                                all_metadata.append(metadata)

                            embeddings = await ollama_service.generate_embeddings_batch(batch)
                            vector_ids = [str(uuid.uuid4()) for _ in range(len(batch))]
                            qdrant_manager.add_text_vectors(embeddings, batch_metadata, vector_ids)

                            total_embedded += len(batch)

                            if total_embedded % 20 == 0:
                                elapsed = time.time() - start_time
                                avg_time_per_chunk = elapsed / total_embedded if total_embedded else 0
                                eta_minutes = int((avg_time_per_chunk * (len(chapters) * 10 - total_embedded)) / 60)

                                logger.info(
                                    f"Embedded {total_embedded} chunks, "
                                    f"current chapter: {chapter.title} | "
                                    f"avg: {avg_time_per_chunk:.2f}s per chunk | "
                                    f"eta: ~{eta_minutes} min"
                                )

                            gc.collect()

                    except Exception as e:
                        logger.error(f"Error processing chapter {chapter.title}: {str(e)}")
                        continue

                # BM25 index
                if all_chunks:
                    logger.info(f"Creating BM25 index for book {book_id} with {len(all_chunks)} chunks")
                    try:
                        self._create_bm25_index(book_id, all_chunks)

                        # metadata mapping for BM25
                        metadata_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_metadata.json")
                        with open(metadata_path, 'w') as f: json.dump(all_metadata, f)

                        logger.info(f"Created BM25 index and metadata for book {book_id}")
                    except Exception as e:
                        logger.error(f"Failed to create BM25 index: {str(e)}")

                logger.info(f"Completed embedding for book {book_id}, total chunks: {total_embedded}")
                return total_embedded

            finally:
                if close_session and db_session: db_session.close()

        except Exception as e:
            logger.error(f"Failed to embed book content: {str(e)}", exc_info=True)
            try:
                qdrant_manager.delete_book_vectors(book_id)
                bm25_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_bm25.pkl")
                metadata_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_metadata.json")
                os.remove(bm25_path)
                os.remove(metadata_path)

            except Exception: pass

            raise VectorStoreException(f"Failed to embed book content: {str(e)}")

    async def delete_book_embeddings(self, book_id: str) -> bool:
        """
        Delete all embeddings for a book
        Args:
            book_id: ID of the book
        Returns:
            True if deletion was successful
        """
        try:
            result = qdrant_manager.delete_book_vectors(book_id)
            bm25_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_bm25.pkl")
            metadata_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_metadata.json")

            os.remove(bm25_path)
            logger.info(f"Deleted BM25 index for book {book_id}")

            os.remove(metadata_path)
            logger.info(f"Deleted BM25 metadata for book {book_id}")

            logger.info(f"Deleted embeddings for book {book_id}")

            return result
        except Exception as e:
            logger.error(f"Failed to delete book embeddings: {str(e)}")
            raise VectorStoreException(f"Failed to delete book embeddings: {str(e)}")

    async def embed_single_text(self, text: str) -> List[float]:
        """
        Generate embedding for a single text
        Args:
            text: Text to embed
        Returns:
            Vector embedding
        """
        try:
            embedding = await ollama_service.generate_embedding(text)
            return embedding
        except Exception as e:
            logger.error(f"Failed to embed single text: {str(e)}")
            raise VectorStoreException(f"Failed to embed text: {str(e)}")

    async def generate_location_summary(self, text: str, location: int) -> str:
        """Summary for a specific location span"""
        try:
            prompt = (
                f"summarize this text segment (location {location}) in 3 sentences highlighting key plot points, "
                f"character developments, and important information. keep it concise:\n\n{text[:2000]}..."
            )
            summary = await ollama_service.generate_completion(
                prompt=prompt,
                temperature=0.3,
                max_tokens=150
            )
            return summary
        except Exception as e:
            logger.error(f"failed to generate location summary: {str(e)}")
            return ""

    async def _create_location_summary(self, book_id: str, location: int, text: str, chapter_title: str,
                                       total_locations: int = 100) -> None:
        """create and store summary"""
        try:
            if not text.strip(): return

            summary_task = self.generate_location_summary(text, location)
            embedding_task = ollama_service.generate_embedding(text[:2000])
            summary, summary_embedding = await asyncio.gather(summary_task, embedding_task)

            if not summary: return

            summary_metadata = {
                'book_id': book_id,
                'location': location,
                'completion_percentage': location_service.get_percentage_from_location(location, total_locations),
                'text': summary,
                'chapter_title': chapter_title,
                'content_type': 'summary'
            }

            summary_id = str(uuid.uuid5(uuid.NAMESPACE_DNS, f"{book_id}_sum_{location}"))
            qdrant_manager.add_text_vectors([summary_embedding], [summary_metadata], [summary_id])
            logger.info(f"created summary for location {location}")

        except Exception as e: logger.error(f"failed to create location summary: {str(e)}")

    def _create_bm25_index(self, book_id: str, text_chunks: List[str]) -> None:
        """Create and save a BM25 index"""
        try:
            try: nltk.data.find('tokenizers/punkt')
            except LookupError: nltk.download('punkt', quiet=True)

            tokenized_chunks = [word_tokenize(chunk.lower()) for chunk in text_chunks]
            bm25_index = BM25Okapi(tokenized_chunks)

            cache_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_bm25.pkl")
            with open(cache_path, 'wb') as f:
                pickle.dump({
                    'index': bm25_index,
                    'chunks': text_chunks,
                    'tokenized_chunks': tokenized_chunks
                }, f)

            logger.info(f"Created and saved BM25 index for book {book_id} with {len(text_chunks)} chunks")
        except Exception as e:
            logger.error(f"Failed to create BM25 index for book {book_id}: {str(e)}")

    def load_bm25_index(self, book_id: str) -> Dict[str, Any]:
        """BM25 index"""
        try:
            cache_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_bm25.pkl")
            if os.path.exists(cache_path):
                with open(cache_path, 'rb') as f:
                    index_data = pickle.load(f)
                logger.info(f"Loaded BM25 index for book {book_id}")
                return index_data
            else:
                logger.warning(f"No BM25 index found for book {book_id}")
                return None
        except Exception as e:
            logger.error(f"Failed to load BM25 index for book {book_id}: {str(e)}")
            return None



embedding_service = EmbeddingService()