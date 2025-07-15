"""
MeReader Reading Progress API Routes
"""
import logging
from datetime import datetime
from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session
from app.db.sqlite import get_db
from app.db.models import Book, ReadingProgress, Chapter
from app.models.progress import ProgressResponse, ChapterInfo

router = APIRouter()
logger = logging.getLogger(__name__)

@router.get("/{book_id}", response_model=ProgressResponse)
async def get_reading_progress(book_id: str, db: Session = Depends(get_db)):
    """
    Get reading progress for a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        progress = db.query(ReadingProgress).filter(ReadingProgress.book_id == book_id).first()
        if not progress:
            # default progress if none
            return {
                "book_id": book_id,
                "current_location": 1,
                "completion_percentage": 0.0,
                "current_chapter": None,
                "last_read_at": None
            }

        # current chapter info if available
        current_chapter = None
        if progress.current_chapter_id:
            chapter = db.query(Chapter).filter(Chapter.id == progress.current_chapter_id).first()
            if chapter:
                current_chapter = ChapterInfo(
                    id=chapter.id,
                    title=chapter.title,
                    order=chapter.order,
                    start_location=chapter.start_location,
                    end_location=chapter.end_location
                )
        else:
            # find chapter based on location
            chapter = db.query(Chapter).filter(
                Chapter.book_id == book_id,
                Chapter.start_location <= progress.current_location,
                Chapter.end_location >= progress.current_location
            ).first()

            if chapter:
                current_chapter = ChapterInfo(
                    id=str(chapter.id),
                    title=str(chapter.title),
                    order=int(chapter.order),
                    start_location=int(chapter.start_location),
                    end_location=int(chapter.end_location)
                )

                progress.current_chapter_id = chapter.id
                db.commit()

        return {
            "book_id": book_id,
            "current_location": progress.current_location,
            "completion_percentage": progress.completion_percentage,
            "current_chapter": current_chapter,
            "last_read_at": progress.last_read_at
        }

    except Exception as e:
        logger.error(f"Failed to get reading progress: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get reading progress: {str(e)}"
        )

@router.put("/{book_id}")
async def update_reading_progress(book_id: str, progress_update: dict, db: Session = Depends(get_db)):
    """
    Update reading progress for a book
    """
    try:
        logger.info(f"Raw progress update data: {progress_update}")

        current_location = progress_update.get("current_location")
        chapter_id = progress_update.get("chapter_id")
        completion_percentage = progress_update.get("completion_percentage")

        logger.info(f"Processing data - location: {current_location}, chapter: {chapter_id}, percentage: {completion_percentage}")

        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            logger.error(f"Book with ID {book_id} not found")
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        progress = db.query(ReadingProgress).filter(ReadingProgress.book_id == book_id).first()
        if not progress:
            logger.info(f"Creating new reading progress for book {book_id}")
            progress = ReadingProgress(book_id=book_id, current_location=1, completion_percentage=0.0)
            db.add(progress)
            db.flush()

        current_chapter = None
        if chapter_id:
            chapter = db.query(Chapter).filter(
                Chapter.id == chapter_id,
                Chapter.book_id == book_id
            ).first()

            if chapter:
                logger.info(f"Found chapter: {chapter.title} (ID: {chapter.id})")
                progress.current_chapter_id = chapter.id

                if current_location is None:
                    current_location = chapter.start_location
                    logger.info(f"Using chapter start location: {current_location}")

                current_chapter = {
                    "id": chapter.id,
                    "title": chapter.title,
                    "order": chapter.order,
                    "start_location": chapter.start_location,
                    "end_location": chapter.end_location
                }

        if current_location is not None:
            try:
                if not isinstance(current_location, int):
                    current_location = int(float(current_location))

                current_location = max(1, current_location)
                if book.total_locations:
                    current_location = min(current_location, book.total_locations)

                logger.info(f"Setting current_location to: {current_location}")
                progress.current_location = current_location

                if not progress.current_chapter_id:
                    chapter = db.query(Chapter).filter(
                        Chapter.book_id == book_id,
                        Chapter.start_location <= current_location,
                        Chapter.end_location >= current_location
                    ).first()

                    if chapter:
                        logger.info(f"Found chapter for location {current_location}: {chapter.title}")
                        progress.current_chapter_id = chapter.id
                        current_chapter = {
                            "id": chapter.id,
                            "title": chapter.title,
                            "order": chapter.order,
                            "start_location": chapter.start_location,
                            "end_location": chapter.end_location
                        }

                if book.total_locations and book.total_locations > 0:
                    calculated_percentage = (current_location / book.total_locations) * 100
                    logger.info(f"Calculated percentage: {calculated_percentage}%")
                    progress.completion_percentage = calculated_percentage

            except Exception as location_error:
                logger.error(f"Error processing location: {str(location_error)}", exc_info=True)

        elif completion_percentage is not None:
            try:
                if not isinstance(completion_percentage, float):
                    completion_percentage = float(completion_percentage)

                normalized_percentage = max(0.0, min(100.0, completion_percentage))
                logger.info(f"Setting completion_percentage to: {normalized_percentage}%")
                progress.completion_percentage = normalized_percentage

                if book.total_locations and book.total_locations > 0 and (
                        progress.current_location is None or progress.current_location <= 1):
                    calculated_location = int((normalized_percentage / 100.0) * book.total_locations)
                    calculated_location = max(1, min(book.total_locations, calculated_location))
                    logger.info(f"Calculated location from percentage: {calculated_location}")
                    progress.current_location = calculated_location

            except Exception as percentage_error:
                logger.error(f"Error processing percentage: {str(percentage_error)}", exc_info=True)

        progress.last_read_at = datetime.utcnow()

        logger.info(f"Pre-commit values: location={progress.current_location}, percentage={progress.completion_percentage}")

        try:
            db.commit()
            logger.info("Successfully committed changes to database")
        except Exception as commit_error:
            logger.error(f"Database commit error: {str(commit_error)}", exc_info=True)
            db.rollback()
            raise

        if progress.current_chapter_id and not current_chapter:
            try:
                chapter = db.query(Chapter).filter(Chapter.id == progress.current_chapter_id).first()
                if chapter:
                    current_chapter = {
                        "id": chapter.id,
                        "title": chapter.title,
                        "order": chapter.order,
                        "start_location": chapter.start_location,
                        "end_location": chapter.end_location
                    }
            except Exception as chapter_error:
                logger.error(f"Error getting chapter info: {str(chapter_error)}")

        return {
            "book_id": book_id,
            "current_location": progress.current_location,
            "completion_percentage": progress.completion_percentage,
            "current_chapter": current_chapter,
            "last_read_at": progress.last_read_at
        }

    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Failed to update reading progress: {str(e)}", exc_info=True)
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to update reading progress: {str(e)}"
        )

@router.post("/{book_id}/reset", response_model=ProgressResponse)
async def reset_reading_progress(book_id: str, db: Session = Depends(get_db)):
    """
    Reset reading progress for a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        progress = db.query(ReadingProgress).filter(ReadingProgress.book_id == book_id).first()
        if not progress:
            progress = ReadingProgress(book_id=book_id)
            db.add(progress)

        # reset progress values
        progress.current_location = 1
        progress.current_chapter_id = None
        progress.completion_percentage = 0.0
        progress.last_read_at = datetime.utcnow()

        db.commit()
        db.refresh(progress)

        return {
            "book_id": book_id,
            "current_location": progress.current_location,
            "completion_percentage": progress.completion_percentage,
            "current_chapter": None,
            "last_read_at": progress.last_read_at
        }

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to reset reading progress: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to reset reading progress: {str(e)}"
        )