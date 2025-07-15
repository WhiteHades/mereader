"""
MeReader RAG (Retrieval-Augmented Generation) Service
"""
import logging
import re
import time
from typing import List, Dict, Any
from sqlalchemy.orm import Session

from app.core.exceptions import AIQueryException, VectorStoreException, BookNotFoundException
from app.services.ollama_service import ollama_service
from app.services.embedding_service import embedding_service
from app.services.location_service import location_service
from app.services.bm25_service import bm25_service
from app.db.qdrant import qdrant_manager
from app.db.models import Book, ReadingProgress
from app.core.config import settings

logger = logging.getLogger(__name__)


class RAGService:
    """
    Service for implementing Retrieval Augmented Generation for book queries
    This service combines vector search with LLM generation respecting reading position boundaries
    """

    def __init__(self):
        self.system_prompt = (
            f"You are MeReader's AI assistant, functioning as an insightful reading companion for the book."
            f"Your knowledge is limited to the provided text excerpts for each query and must reflect only what a reader would"
            "Answer user queries by performing a 'mini literary analysis' based exclusively on the provided text excerpts."
            "Focus on character motivations, feelings, actions, plot developments, mood, and tone *as presented within those excerpts*.\n\n"
            "Constraints:**\n"
            "1.  Base your entire answer SOLELY on facts, dialogue, descriptions, and character thoughts explicitly"
            "stated or directly described within the provided text excerpts. DO NOT use outside knowledge of the book or world.\n"
            "2.  NEVER reveal or allude to any plot points, character developments, or events occurring beyond"
            "the user's reading progress marker.\n"
            "3.  If the answer cannot be found or fully substantiated within the"
            "provided excerpts, state clearly and directly: 'Based on the text up to this point, the information needed to answer"
            "isn't available.' or a similar concise phrase.\n"
            "4.  DO NOT mention 'excerpts', 'passages', 'context', 'provided text', 'your system', or any"
            "internal mechanisms in your response.\n"
            "5.  When citing the source of information if necessary for clarity, refer only to the chapter"
            "number(s) associated with the provided excerpts. Do not refer to passage numbers.\n"
            "6. Avoid introductory fluff like 'In the provided context...' or"
            "'Based on the passages...'. State findings directly. Avoid broad opening statements summarizing multiple chapters"
            "(e.g., 'Chapters X to Y show...').\n"
            "7. Ensure any mentioned relationships or facts are explicitly present or directly or indirectly "
            "supportable by the provided excerpt(s) from the relevant chapters.\n\n"
            "Analytical Tone:\n"
            "*   First, understand the nuances of the user's question.\n"
            "*   Identify relevant textual details. When analyzing character psyche (intentions, feelings,"
            "motivations), clearly link your analysis back to specific actions, words, or thoughts *found within the excerpts*. \n"
            "*   Combine the extracted information into a coherent, thoughtful response. Discuss mood, tone,"
            "character actions, stated/implied intent, and plot happenings *as revealed by the text you have*. \n"
            "*   Remain factual. Be cautious when interpreting ambiguity within the excerpts, clearly"
            "stating what is known versus what might be implied but isn't confirmed.\n"
            "*   Be Confident (When Applicable): When the excerpts provide clear, unambiguous facts, state them confidently.\n\n"
            "Persona: You are a disciplined, book-bound reading assistant,"
            "providing literary insights limited strictly by the text excerpts corresponding to the user's progress."
        )
        logger.info("RAG Service initialized")

    async def process_query(
            self,
            book_id: str,
            query: str,
            reading_progress: ReadingProgress,
            db: Session
    ) -> Dict[str, Any]:
        """
        Process a user query about a book with RAG
        Args:
            book_id: ID of the book
            query: User query text
            reading_progress: User's reading progress in the book
            db: Database session
        Returns:
            Dictionary with query results including response and supporting passages
        """
        try:
            start_time = time.time()
            logger.info(f"Processing query for book_id '{book_id}': '{query}'")
            book = db.query(Book).filter(Book.id == book_id).first()

            if not book: raise BookNotFoundException(book_id)

            location_boundary = location_service.calculate_location_boundary(
                reading_progress.current_location,
                book.total_locations or 100
            )

            logger.info(f"Location boundary set to: {location_boundary} (User progress: {reading_progress.completion_percentage:.1f}%)")

            if location_boundary <= 0: raise AIQueryException("No reading progress for this book")

            # 1: query embeddings and expanded queries
            expanded_queries = await self._expand_query(query, book.title)
            query_embedding = await embedding_service.embed_single_text(query)

            # 2: retrieve context
            retrieval_start_time = time.time()
            all_results = []

            # bm25
            try:
                bm25_results = await bm25_service.search(
                    query=query,
                    book_id=book_id,
                    location_boundary=location_boundary,
                    limit=settings.BM25_RESULTS_LIMIT
                )
                if bm25_results:
                    logger.info(f"Found {len(bm25_results)} results from BM25 search")
                    for result in bm25_results: result["search_method"] = "bm25"
                    all_results.extend(bm25_results)
            except Exception as e:
                logger.error(f"BM25 search failed: {str(e)}")


            # vector
            search_results = qdrant_manager.search_vectors(
                query_vector=query_embedding,
                book_id=book_id,
                limit=15,
                score_threshold=0.6,
                location_boundary=location_boundary
            )
            for result in search_results: result["search_method"] = "vector"
            all_results.extend(search_results)

            # summary
            summary_results = qdrant_manager.search_vectors(
                query_vector=query_embedding,
                book_id=book_id,
                limit=5,
                score_threshold=0.65,
                location_boundary=location_boundary,
                filter_metadata={"content_type": "summary"}
            )
            for result in summary_results: result["search_method"] = "summary"
            all_results.extend(summary_results)

            for expanded_query in expanded_queries:
                expanded_embedding = await embedding_service.embed_single_text(expanded_query)
                expanded_results = qdrant_manager.search_vectors(
                    query_vector=expanded_embedding,
                    book_id=book_id,
                    limit=5,
                    score_threshold=0.5,
                    location_boundary=location_boundary
                )
                for result in expanded_results: result["search_method"] = "expanded_vector"
                all_results.extend(expanded_results)

            retrieval_time = time.time() - retrieval_start_time
            logger.info(f"Combined retrieval finished in {retrieval_time:.2f}s. Total results: {len(all_results)}")

            # 3: results
            process_start_time = time.time()
            vector_results = [r for r in all_results if r.get("search_method") == "vector"]
            bm25_results = [r for r in all_results if r.get("search_method") == "bm25"]
            expanded_results = [r for r in all_results if r.get("search_method") == "expanded_vector"]
            summary_results = [r for r in all_results if r.get("search_method") == "summary"]

            logger.info(f"Retrieved results by method - Vector: {len(vector_results)}, BM25: {len(bm25_results)}, "
                        f"Expanded: {len(expanded_results)}, Summary: {len(summary_results)}")

            if summary_results:
                max_summary_score = max(r.get("score", 0) for r in summary_results) if summary_results else 1.0
                summary_weight = 0.4
                for r in summary_results:
                    r["original_score"] = r.get("score", 0)
                    r["score"] = (r.get("score", 0) / max_summary_score if max_summary_score > 0 else 0) * summary_weight

            if vector_results and bm25_results:
                max_vector_score = max(r.get("score", 0) for r in vector_results) if vector_results else 1.0
                for r in vector_results:
                    r["original_score"] = r.get("score", 0)
                    r["score"] = (r.get("score", 0) / max_vector_score if max_vector_score > 0 else 0) * (1 - settings.BM25_WEIGHT)

                max_bm25_score = max(r.get("score", 0) for r in bm25_results) if bm25_results else 1.0
                for r in bm25_results:
                    r["original_score"] = r.get("score", 0)
                    r["score"] = (r.get("score", 0) / max_bm25_score if max_bm25_score > 0 else 0) * settings.BM25_WEIGHT

            if expanded_results:
                max_expanded_score = max(r.get("score", 0) for r in expanded_results) if expanded_results else 1.0
                expanded_weight = 0.7
                for r in expanded_results:
                    r["original_score"] = r.get("score", 0)
                    r["score"] = (r.get("score", 0) / max_expanded_score if max_expanded_score > 0 else 0) * expanded_weight

            # deduplicate
            seen_texts = {}
            unique_results = []

            sorted_results = sorted(all_results, key=lambda x: x.get("score", 0), reverse=True)

            for result in sorted_results:
                # text content as deduplication key
                text = result.get("text", "")
                text_key = text[:100]

                if text_key not in seen_texts:
                    seen_texts[text_key] = True
                    unique_results.append(result)

            process_time = time.time() - process_start_time
            logger.info(f"Results processing completed in {process_time:.2f}s. Unique results: {len(unique_results)}")

            if not unique_results:
                return {
                    "response": "I don't have enough information from the book to answer that question based on what you've read so far.",
                    "context_used": [],
                    "query": query,
                    "book_title": book.title,
                    "location_boundary": location_boundary,
                    "progress_boundary": reading_progress.completion_percentage
                }

            # 4: rerank results
            if len(unique_results) > 8 and len(query.split()) > 3:
                reranked_results = await self._rerank_results(query, unique_results, book.title)
                top_results = reranked_results[:25]
            else:
                top_results = unique_results[:10]

            # 5: context from top results
            context = self._prepare_context_from_search_results(top_results)

            # 6: LLM response
            response_start_time = time.time()
            prompt = self._build_rag_prompt(query, context, reading_progress.completion_percentage)

            response = await ollama_service.generate_completion(
                prompt=prompt,
                system_prompt=self.system_prompt,
                temperature=0.7
            )

            response_time = time.time() - response_start_time
            logger.info(f"LLM response generated in {response_time:.2f}s")

            # 7: final format
            context_snippets = self._format_context_snippets(top_results)

            total_time = time.time() - start_time
            logger.info(f"Total query processing time: {total_time:.2f}s")

            return {
                "response": response,
                "context_used": context_snippets,
                "query": query,
                "book_title": book.title,
                "location_boundary": location_boundary,
                "progress_boundary": reading_progress.completion_percentage
            }

        except VectorStoreException as e:
            logger.error(f"Vector store error during query processing: {str(e)}")
            raise AIQueryException(f"Error retrieving context from book: {str(e)}")

        except Exception as e:
            logger.error(f"Failed to process AI query: {str(e)}")
            raise AIQueryException(f"Failed to process AI query: {str(e)}")

    def _prepare_context_from_search_results(self, search_results: List[Dict[str, Any]]) -> str:
        """
        Prepare context string from search results
        Args:
            search_results: List of search result dictionaries from vector store
        Returns:
            Formatted context string for LLM
        """
        context_parts = []
        summary_parts = []
        content_parts = []

        for i, result in enumerate(search_results):
            text = result.get("text", "")
            chapter_title = result.get("chapter_title", "Unknown Chapter")
            search_method = result.get("search_method", "unknown")
            location = result.get("location", "Unknown Location")

            if search_method == "summary":
                part = f"SUMMARY (Chapter: {chapter_title}, Loc: {location}):\n{text}\n"
                summary_parts.append(part)
            else:
                source_marker = ""
                if search_method == "bm25": source_marker = "[KW] "
                elif search_method == "expanded_vector": source_marker = "[EX] "
                part = f"PASSAGE {i + 1} {source_marker}(Chapter: {chapter_title}, Loc: {location}):\n{text}\n"
                content_parts.append(part)

        if summary_parts:
            context_parts.append("--- SECTION SUMMARIES ---")
            context_parts.extend(summary_parts)
            context_parts.append("--- DETAILED PASSAGES ---")

        context_parts.extend(content_parts)

        return "\n".join(context_parts)

    def _build_rag_prompt(self, query: str, context: str, completion_percentage: float) -> str:
        """
        Build RAG prompt for LLM
        Args:
            query: User query
            context: Context from vector search
            completion_percentage: User's reading progress percentage
        Returns:
            Complete prompt for LLM
        """
        return (
            f"BOOK CONTEXT INFORMATION:\n"
            f"{context}\n\n"
            f"The user has read approximately {completion_percentage:.1f}% of the book.\n"
            f"USER QUESTION: {query}\n\n"
            f"ANSWER:"
        )

    def _format_context_snippets(self, search_results: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        """
        Format context snippets for frontend
        Args:
            search_results: List of search result dictionaries
        Returns:
            List of formatted context snippets
        """
        snippets = []

        for result in search_results:
            snippet = {
                "text": result.get("text", ""),
                "chapter_title": result.get("chapter_title", "Unknown Chapter"),
                "location": result.get("location", 0),
                "relevance_score": round(result.get("score", 0.0), 2),
                "search_method": result.get("search_method", "vector"),
                "content_type": result.get("content_type", "content")
            }
            snippets.append(snippet)

        return snippets

    async def _expand_query(self, query: str, book_title: str) -> List[str]:
        """
        Expand original query into related sub-queries to improve retrieval
        Args:
            query: Original user query
            book_title: Title of the book
        Returns:
            List of expanded queries
        """
        try:
            prompt = (
                f"You are helping to expand a search query about the book '{book_title}'. "
                f"Generate 2 alternative versions of the original query to improve semantic search results. "
                f"Focus on extracting key entities, actions, and concepts from the original query. "
                f"For character-based questions, include full character names and relevant attributes or actions. "
                f"Keep your responses concise and directly related to the original query.\n\n"
                f"Original query: {query}\n\n"
                f"Generate 2 alternative queries (numbered list only):"
            )

            response = await ollama_service.generate_completion(
                prompt=prompt,
                temperature=0.3,
                max_tokens=150
            )

            expanded_queries = []
            for line in response.split('\n'):
                if line.strip() and (line.strip().startswith('1.') or line.strip().startswith('2.')):
                    expanded_query = line.strip().split('.', 1)[1].strip()
                    if expanded_query and len(expanded_query) > 5: expanded_queries.append(expanded_query)

            logger.info(f"Expanded original query into: {expanded_queries}")
            return expanded_queries
        except Exception as e:
            logger.warning(f"Query expansion failed: {e}")
            return []

    async def _rerank_results(self, query: str, results: List[Dict[str, Any]], book_title: str) -> List[Dict[str, Any]]:
        """
        Rerank search results using LLM for better relevance
        Args:
            query: User query
            results: Initial search results to rerank
            book_title: Title of the book
        Returns:
            Reranked results list
        """
        try:
            if len(results) < 5: return results
            results_to_rerank = results[:15]

            prompt_parts = [
                f"You are helping rank search results for the query: '{query}' about the book '{book_title}'.\n",
                f"Rate each passage on a scale from 1-10 based on how directly relevant it is to answering the query.\n",
                "10 = directly answers the query; 1 = unrelated to the query.\n\n",
                "Passages to rank:"
            ]

            # passages with indices
            for i, result in enumerate(results_to_rerank):
                text = result.get("text", "")[:200]
                prompt_parts.append(f"\n[{i + 1}] {text}...")

            prompt_parts.append("\nProvide your ratings in this exact format, one per line:")
            prompt_parts.append("FORMAT: [index]: [score]")
            prompt = "\n".join(prompt_parts)

            # rankings from LLM
            response = await ollama_service.generate_completion(
                prompt=prompt,
                temperature=0.1,
                max_tokens=200
            )

            # parse rankings
            rankings = {}
            pattern = r'\[(\d+)\]:\s*(\d+)'
            matches = re.findall(pattern, response)

            for match in matches:
                try:
                    index = int(match[0]) - 1  # converting to 0-based
                    score = int(match[1])
                    if 0 <= index < len(results_to_rerank):
                        rankings[index] = min(10, max(1, score))  # score 1-10
                except (ValueError, IndexError):
                    continue

            # rankings
            if rankings:
                for i, result in enumerate(results_to_rerank):
                    if i in rankings: result["rerank_score"] = rankings[i]
                    else: result["rerank_score"] = 1

                reranked = sorted(
                    results_to_rerank,
                    key=lambda x: (x.get("rerank_score", 0), x.get("score", 0)),
                    reverse=True
                )

                remaining = results[15:]
                return reranked + remaining

            return results

        except Exception as e:
            logger.error(f"Reranking failed: {e}")
            return results


rag_service = RAGService()