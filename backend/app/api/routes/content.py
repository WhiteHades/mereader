"""
MeReader Content API Routes for serving processed book content
"""
import logging
import os
from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session
from fastapi.responses import FileResponse
from app.db.sqlite import get_db
from app.db.models import Book, Chapter
from app.services.location_service import location_service
from app.models.content import ChapterContentResponse

router = APIRouter()
logger = logging.getLogger(__name__)

@router.get("/image/{book_id}/{image_name}")
async def get_book_image(book_id: str, image_name: str, db: Session = Depends(get_db)):
    """
    Get an image from a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=f"Book with ID {book_id} not found")

        # Get content directory
        content_dir = os.path.dirname(book.content_path) if os.path.isfile(book.content_path) else book.content_path

        # Log directory and image being searched for
        logger.info(f"Looking for image: {image_name} in directory: {content_dir}")

        # List all files in the directory to help with debugging
        all_files = os.listdir(content_dir)
        logger.info(f"Files in directory: {all_files}")

        # Try different permutations of the filename
        possible_image_names = [
            image_name,
            image_name.lower(),
            image_name.upper(),
            image_name.replace('_', '-'),
            image_name.replace('-', '_')
        ]

        image_path = None
        for name in possible_image_names:
            test_path = os.path.join(content_dir, name)
            if os.path.exists(test_path):
                image_path = test_path
                logger.info(f"Found image at: {image_path}")
                break

        if not image_path:
            raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=f"Image {image_name} not found")

        # Determine content type
        content_type = "image/jpeg"  # Default
        if image_name.lower().endswith(".png"):
            content_type = "image/png"
        elif image_name.lower().endswith(".gif"):
            content_type = "image/gif"
        elif image_name.lower().endswith(".svg"):
            content_type = "image/svg+xml"

        return FileResponse(path=image_path, media_type=content_type, filename=image_name)

    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Failed to get book image: {str(e)}")
        raise HTTPException(status_code=status.HTTP_500_INTERNAL_SERVER_ERROR, detail=f"Failed to get book image: {str(e)}")

@router.get("/index/{book_id}")
async def get_book_index(book_id: str, db: Session = Depends(get_db)):
    """
    Get the index file for a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        if not book.content_path or not os.path.exists(book.content_path):
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Book content not found"
            )

        index_path = os.path.join(book.content_path, "index.html")
        if not os.path.exists(index_path):
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Book index file not found"
            )

        return FileResponse(
            path=index_path,
            media_type="text/html",
            filename="index.html"
        )

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to get book index: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get book index: {str(e)}"
        )

@router.get("/chapter/{book_id}/{chapter_id}", response_model=ChapterContentResponse)
async def get_chapter_content(book_id: str, chapter_id: str, db: Session = Depends(get_db)):
    """
    Get the content of a specific chapter
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        chapter = db.query(Chapter).filter(
            Chapter.book_id == book_id,
            Chapter.id == chapter_id
        ).first()

        if not chapter:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Chapter with ID {chapter_id} not found for book {book_id}"
            )

        if not chapter.content_path or not os.path.exists(chapter.content_path):
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Chapter content file not found"
            )

        try:
            with open(chapter.content_path, 'r', encoding='utf-8') as f:
                content = f.read()

            return ChapterContentResponse(
                book_id=book_id,
                chapter_id=str(chapter.id),
                title=str(chapter.title),
                order=int(chapter.order),
                start_location=int(chapter.start_location),
                end_location=int(chapter.end_location),
                content=content
            )

        except Exception as e:
            logger.error(f"Error reading chapter file: {str(e)}")
            raise HTTPException(
                status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
                detail=f"Error reading chapter file: {str(e)}"
            )

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to get chapter content: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get chapter content: {str(e)}"
        )

@router.get("/chapter-by-location/{book_id}/{location}")
async def get_chapter_by_location(book_id: str, location: int, db: Session = Depends(get_db)):
    """
    Get the chapter that contains a specific location
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        # chapter containing the location
        chapter = db.query(Chapter).filter(
            Chapter.book_id == book_id,
            Chapter.start_location <= location,
            Chapter.end_location >= location
        ).first()

        if not chapter:
            # if not found, get the closest chapter
            chapter = db.query(Chapter).filter(
                Chapter.book_id == book_id,
                Chapter.start_location <= location
            ).order_by(Chapter.start_location.desc()).first()

            if not chapter:
                # if still not found, get the first chapter
                chapter = db.query(Chapter).filter(Chapter.book_id == book_id).order_by(Chapter.order).first()

        if not chapter:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"No chapters found for book {book_id}"
            )

        return {
            "id": chapter.id,
            "title": chapter.title,
            "order": chapter.order,
            "start_location": chapter.start_location,
            "end_location": chapter.end_location,
            "location_in_chapter": location - chapter.start_location,
            "total_locations_in_chapter": chapter.end_location - chapter.start_location + 1
        }

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to get chapter by location: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get chapter by location: {str(e)}"
        )

@router.get("/book-content/{book_id}")
async def get_full_book_content(book_id: str, db: Session = Depends(get_db)):
    """
    Get all content for a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        if not book.content_path or not os.path.exists(book.content_path):
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Book content not found"
            )

        chapters = db.query(Chapter).filter(Chapter.book_id == book_id).order_by(Chapter.order).all()

        # extracting content from chapter htmls
        content_by_chapter = {}
        for chapter in chapters:
            if chapter.content_path and os.path.exists(chapter.content_path):
                try:
                    with open(chapter.content_path, 'r', encoding='utf-8') as f: content_by_chapter[chapter.id] = f.read()
                except Exception as e:
                    logger.warning(f"Error reading chapter {chapter.id}: {str(e)}")
                    content_by_chapter[chapter.id] = f"<p>Error loading chapter: {str(e)}</p>"
            else:
                content_by_chapter[chapter.id] = "<p>Chapter content not found</p>"

        return {
            "book_id": book_id,
            "title": book.title,
            "chapters": [
                {
                    "id": chapter.id,
                    "title": chapter.title,
                    "order": chapter.order,
                    "start_location": chapter.start_location,
                    "end_location": chapter.end_location,
                    "content": content_by_chapter.get(chapter.id, "")
                }
                for chapter in chapters
            ]
        }

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to get full book content: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get full book content: {str(e)}"
        )

@router.get("/text-at-location/{book_id}/{location}")
async def get_text_at_location(book_id: str, location: int, context_size: int = 500, db: Session = Depends(get_db)):
    """
    Get text at a specific location with context
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        # chapter containing the location
        chapter = db.query(Chapter).filter(
            Chapter.book_id == book_id,
            Chapter.start_location <= location,
            Chapter.end_location >= location
        ).first()

        if not chapter:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"No chapter found containing location {location}"
            )

        if not chapter.content_path or not os.path.exists(chapter.content_path):
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Chapter content file not found"
            )

        # chapter content
        try:
            with open(chapter.content_path, 'r', encoding='utf-8') as f: chapter_content = f.read()

            relative_location = location - chapter.start_location + 1
            text_at_location = location_service.get_text_at_location(
                chapter_content,
                relative_location,
                context_size
            )

            return {
                "book_id": book_id,
                "chapter_id": chapter.id,
                "chapter_title": chapter.title,
                "location": location,
                "text": text_at_location
            }

        except Exception as e:
            logger.error(f"Error reading chapter file: {str(e)}")
            raise HTTPException(
                status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
                detail=f"Error reading chapter file: {str(e)}"
            )

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to get text at location: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get text at location: {str(e)}"
        )