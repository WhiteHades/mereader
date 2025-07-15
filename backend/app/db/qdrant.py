"""
MeReader Qdrant Vector Store Integration
"""
import logging
from typing import List, Dict, Any, Optional
from qdrant_client import QdrantClient
from qdrant_client.http.models import Distance, VectorParams, PointStruct
from qdrant_client.models import Filter, FieldCondition, MatchValue, Range
from app.core.config import settings
from app.core.exceptions import VectorStoreException

logger = logging.getLogger(__name__)

class QdrantManager:
    """Manager for Qdrant vector database ops"""
    def __init__(self):
        try:
            logger.info(f"Starting Qdrant with location: {settings.QDRANT_LOCATION}")
            self.client = QdrantClient(path=settings.QDRANT_LOCATION)

            collections = self.client.get_collections().collections
            collection_names = [collection.name for collection in collections]

            if settings.QDRANT_COLLECTION_NAME not in collection_names:
                logger.info(f"Creating collection: {settings.QDRANT_COLLECTION_NAME}")
                self.client.create_collection(
                    collection_name=settings.QDRANT_COLLECTION_NAME,
                    vectors_config=VectorParams(
                        size=settings.QDRANT_VECTOR_SIZE,
                        distance=Distance.COSINE
                    )
                )
                logger.info(f"Created collection: {settings.QDRANT_COLLECTION_NAME}")
        except Exception as e:
            raise VectorStoreException(f"Failed to start Qdrant: {str(e)}")

    def has_vectors_for_book(self, book_id: str) -> bool:
        """
        Check if vectors exist for a book
        Args:
            book_id: Book ID to check
        Returns:
            True if vectors exist, else False
        """
        try:
            search_filter = Filter(
                must=[FieldCondition(key="book_id", match=MatchValue(value=book_id))])
            result = self.client.scroll(
                collection_name=settings.QDRANT_COLLECTION_NAME,
                scroll_filter=search_filter,
                limit=1
            )
            return bool(result[0])
        except Exception as e:
            logger.error(f"Failed to check existing vectors for book {book_id}: {str(e)}")
            return False

    def add_text_vectors(
            self,
            vectors: List[List[float]],
            metadata: List[Dict[str, Any]],
            ids: Optional[List[str]] = None
    ) -> List[str]:
        """
        Add text vectors to Qdrant
        Args:
            vectors: List of vector embeddings
            metadata: List of metadata dictionaries for each vector
            ids: list of IDs for the vectors
        Returns:
            List of IDs for the added vectors
        """
        try:
            # if no ids, Qdrant will generate them
            points = []
            for i, (vector, meta) in enumerate(zip(vectors, metadata)):
                point_id = ids[i] if ids and i < len(ids) else None
                points.append(
                    PointStruct(
                        id=point_id,
                        vector=vector,
                        payload=meta
                    )
                )

            result = self.client.upsert(
                collection_name=settings.QDRANT_COLLECTION_NAME,
                points=points
            )

            return [str(point.id) for point in points]

        except Exception as e:
            logger.error(f"Failed to add vectors to Qdrant: {str(e)}")
            raise VectorStoreException(f"Failed to add vectors to Qdrant: {str(e)}")

    def search_vectors(
            self,
            query_vector: List[float],
            book_id: str,
            limit: int = 15,
            score_threshold: float = 0.6,
            location_boundary: Optional[int] = None,
            filter_metadata: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """
        Search for similar vectors with metadata filtering
        """
        try:
            # base filter
            filter_conditions = [FieldCondition(key="book_id", match=MatchValue(value=book_id))]

            if location_boundary is not None:
                filter_conditions.append(
                    FieldCondition(
                        key="location",
                        range=Range(lte=location_boundary)
                    )
                )

            if filter_metadata:
                for key, value in filter_metadata.items():
                    filter_conditions.append(FieldCondition(key=key, match=MatchValue(value=value)))

            query_filter = Filter(must=filter_conditions)

            search_result = self.client.search(
                collection_name=settings.QDRANT_COLLECTION_NAME,
                query_vector=query_vector,
                query_filter=query_filter,
                limit=limit,
                score_threshold=score_threshold
            )

            results = []
            for scored_point in search_result:
                result = {
                    "id": str(scored_point.id),
                    "score": scored_point.score,
                    **scored_point.payload
                }
                results.append(result)

            return results

        except Exception as e:
            logger.error(f"failed to search vectors in qdrant: {str(e)}")
            raise VectorStoreException(f"failed to search vectors in qdrant: {str(e)}")

    def delete_book_vectors(self, book_id: str) -> bool:
        """
        Delete vectors embeddings of a book
        Args:
            book_id: ID of the book to delete vectors for
        Returns:
            True if deletion was successful
        """
        try:
            filter_condition = Filter(
                must=[FieldCondition(key="book_id", match=MatchValue(value=book_id))
                ]
            )

            self.client.delete(
                collection_name=settings.QDRANT_COLLECTION_NAME,
                points_selector=filter_condition
            )

            logger.info(f"Deleted vectors for book {book_id}")
            return True

        except Exception as e:
            logger.error(f"Failed to delete vectors for book {book_id}: {str(e)}")
            raise VectorStoreException(f"Failed to delete vectors for book {book_id}: {str(e)}")

qdrant_manager = QdrantManager()