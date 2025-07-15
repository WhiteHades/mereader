"""
MeReader AI Query API Routes
"""
import logging
from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session
from app.db.sqlite import get_db
from app.db.models import Book, ReadingProgress
from app.services.rag_service import rag_service
from app.core.exceptions import AIQueryException, BookNotFoundException
from app.services.ollama_service import ollama_service
from app.models.query import (QueryRequest, QueryResponse)

router = APIRouter()
logger = logging.getLogger(__name__)

async def validate_ollama_service():
    """
    Check if Ollama service is running
    Raises:
        HTTPException: If Ollama service is not available
    """
    is_running = await ollama_service.check_status()

    if not is_running:
        logger.error("Ollama service is not running")
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Ollama is currently unavailable."
        )

@router.post("/ask/{book_id}", response_model=QueryResponse)
async def ask_question(
        book_id: str,
        query_request: QueryRequest,
        db: Session = Depends(get_db),
        _: None = Depends(validate_ollama_service)
):
    """Ask any question to the AI"""
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book: raise BookNotFoundException(book_id)

        progress = db.query(ReadingProgress).filter(ReadingProgress.book_id == book_id).first()
        if not progress:
            raise AIQueryException("No reading progress found for this book. Please start reading first.")

        # processing query
        result = await rag_service.process_query(
            book_id=book_id,
            query=query_request.query,
            reading_progress=progress,
            db=db
        )

        return {
            "response": result.get("response", ""),
            "query": query_request.query,
            "book_id": book_id,
            "book_title": result.get("book_title", book.title),
            "context_used": result.get("context_used", []),
            "location_boundary": result.get("location_boundary", progress.current_location),
            "progress_boundary": result.get("progress_boundary", progress.completion_percentage)
        }

    except BookNotFoundException as e:
        logger.error(f"Book not found: {str(e)}")
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=str(e))

    except AIQueryException as e:
        logger.error(f"AI query error: {str(e)}")
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail=str(e))

    except Exception as e:
        logger.error(f"Failed to process AI query: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to process AI query: {str(e)}"
        )