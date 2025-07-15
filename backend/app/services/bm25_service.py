"""
MeReader BM25 Search Service - Provides keyword-based text search
"""
import logging
import os
import json
import time
import nltk
from typing import List, Dict, Any
from nltk.tokenize import word_tokenize

from app.core.config import settings
from app.services.embedding_service import embedding_service

logger = logging.getLogger(__name__)


class BM25Service:
    """Service for keyword-based search using BM25 algorithm"""

    def __init__(self):
        logger.info("BM25 Service")
        try: nltk.data.find('tokenizers/punkt')
        except LookupError: nltk.download('punkt', quiet=True)

    async def search(self, query: str, book_id: str, location_boundary: int, limit: int = 10) -> List[Dict[str, Any]]:
        """
        Search for relevant text chunks using BM25
        Args:
            query: Search query
            book_id: ID of the book to search
            location_boundary: Maximum location to include (reading progress)
            limit: Maximum number of results to return
        Returns:
            List of search results with scores
        """
        try:
            start_time = time.time()

            index_data = embedding_service.load_bm25_index(book_id)
            if not index_data:
                logger.warning(f"No BM25 index found for book {book_id}")
                return []

            metadata_path = os.path.join(settings.BM25_INDEX_CACHE_DIR, f"{book_id}_metadata.json")
            if not os.path.exists(metadata_path):
                logger.warning(f"No metadata found for BM25 index for book {book_id}")
                return []

            with open(metadata_path, 'r') as f:
                metadata_list = json.load(f)
            bm25_index = index_data['index']
            chunks = index_data['chunks']

            tokenized_query = word_tokenize(query.lower())
            scores = bm25_index.get_scores(tokenized_query)

            results = []
            for i, score in enumerate(scores):
                if i >= len(metadata_list): continue

                metadata = metadata_list[i]
                location = metadata.get('location', 0)

                if location > location_boundary: continue

                if score > 0:
                    results.append({
                        "score": float(score),
                        "text": chunks[i],
                        **metadata
                    })

            results = sorted(results, key=lambda x: x["score"], reverse=True)[:limit]

            if results:
                max_score = max(r["score"] for r in results)
                for r in results:
                    r["score"] = r["score"] / max_score * 0.95

            duration = time.time() - start_time
            logger.info(f"BM25 search completed in {duration:.2f}s, found {len(results)} results")

            return results

        except Exception as e:
            logger.error(f"BM25 search failed: {str(e)}")
            return []


bm25_service = BM25Service()