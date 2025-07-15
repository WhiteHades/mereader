"""
MeReader Book Management API Routes
"""
import gc
import logging
import os
from fastapi import APIRouter, Depends, HTTPException, UploadFile, File, status, BackgroundTasks
from sqlalchemy.orm import Session
from fastapi.responses import FileResponse
from app.db.sqlite import get_db, SessionLocal
from app.db.models import Book, Chapter, ReadingProgress
from app.services.embedding_service import embedding_service
from app.services.book_service import book_service
from app.core.exceptions import BookParsingException, FileStorageException
from app.models.book import BookListResponse, BookListItem

router = APIRouter()
logger = logging.getLogger(__name__)

async def embed_book_content_task(book_id: str):
    """
    Background task for embedding book content
    Args:
        book_id: ID of the book to process
    """
    try:
        gc.collect()
        db_session = SessionLocal()
        try:
            book = db_session.query(Book).filter(Book.id == book_id).first()
            if not book:
                logger.error(f"Book not found for embedding: {book_id}")
                return

            await embedding_service.embed_book_content(book_id=book_id, db_session=db_session)

        except Exception as e: logger.error(f"Error during embedding task: {str(e)}", exc_info=True)
        finally:
            db_session.close()
            gc.collect()

    except Exception as e:
        logger.error(f"Background embedding task failed for book {book_id}: {str(e)}")

@router.get("/")
async def list_books(skip: int = 0, limit: int = 100, db: Session = Depends(get_db)):
    """
    List all books in the library
    """
    try:
        books = db.query(Book).offset(skip).limit(limit).all()
        book_items  = []

        for book in books:
            progress = db.query(ReadingProgress).filter(ReadingProgress.book_id == book.id).first()

            book_item = BookListItem(
                id=str(book.id),
                title=str(book.title),
                author=str(book.author),
                cover_path=str(book.cover_path),
                completion_percentage=progress.completion_percentage if progress else 0.0,
                last_read_at=progress.last_read_at if progress else None
            )
            book_items.append(book_item)

        return BookListResponse(books=book_items, total=len(book_items))

    except Exception as e:
        logger.error(f"Failed to list books: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to list books: {str(e)}"
        )

@router.get("/{book_id}")
async def get_book(book_id: str, db: Session = Depends(get_db)):
    """
    Get information about a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )

        chapters = db.query(Chapter).filter(Chapter.book_id == book_id).order_by(Chapter.order).all()
        progress = db.query(ReadingProgress).filter(ReadingProgress.book_id == book_id).first()

        response = {
            "id": book.id,
            "title": book.title,
            "author": book.author,
            "cover_path": book.cover_path,
            "content_path": book.content_path,
            "language": book.language,
            "published_year": book.published_year,
            "publisher": book.publisher,
            "isbn": book.isbn,
            "description": book.description,
            "content_length": book.content_length,
            "total_locations": book.total_locations,
            "total_chapters": book.total_chapters,
            "chapters": [
                {
                    "id": chapter.id,
                    "title": chapter.title,
                    "order": chapter.order,
                    "start_location": chapter.start_location,
                    "end_location": chapter.end_location
                }
                for chapter in chapters
            ],
            "completion_percentage": progress.completion_percentage if progress else 0.0,
            "current_location": progress.current_location if progress else 0,
            "last_read_at": progress.last_read_at if progress else None
        }

        return response

    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Failed to get book details: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to get book details: {str(e)}"
        )

@router.post("/upload", status_code=status.HTTP_201_CREATED)
async def upload_book(background_tasks: BackgroundTasks, file: UploadFile = File(...), db: Session = Depends(get_db)):
    """
    Upload and process a new book
    """
    try:
        logger.info(f"Uploaded filename: {file.filename}")
        logger.info(f"Client-reported MIME: {file.content_type}")
        if not file.filename.lower().endswith('.epub') and file.content_type != 'application/epub+zip':
            raise HTTPException(
                status_code=status.HTTP_400_BAD_REQUEST,
                detail="Only EPUB files are supported"
            )

        logger.info(f"Uploading book: {file.filename}")

        # reading content in chunks to save memory
        chunk_size = 1024 * 1024  # 1mb chunks
        chunks = []
        while True:
            chunk = await file.read(chunk_size)
            if not chunk: break
            chunks.append(chunk)

        file_content = b''.join(chunks)
        del chunks
        gc.collect()

        file_path = book_service.save_uploaded_file(file_content, file.filename)
        del file_content
        gc.collect()

        book_data = book_service.parse_book(file_path)
        metadata = book_data['metadata']
        chapters = book_data['chapters']
        content_dir = book_data['content_dir']
        total_locations = book_data['total_locations']

        # book record
        book = Book(
            id=book_data['id'],
            title=metadata.get('title', 'Unknown Title'),
            author=metadata.get('author', 'Unknown Author'),
            file_path=file_path,
            content_path=content_dir,
            cover_path=metadata.get('cover_path'),
            language=metadata.get('language'),
            published_year=metadata.get('published_year'),
            publisher=metadata.get('publisher'),
            isbn=metadata.get('isbn'),
            description=metadata.get('description'),
            content_length=book_data.get('content_length', 0),
            total_locations=total_locations,
            total_chapters=len(chapters),
            book_metadata=metadata
        )

        db.add(book)
        db.flush()

        for chapter_data in chapters:
            chapter = Chapter(
                book_id=book.id,
                title=chapter_data.get('title', 'Untitled Chapter'),
                order=chapter_data.get('order', 0),
                content_path=chapter_data.get('content_path'),
                start_location=chapter_data.get('start_location', 0),
                end_location=chapter_data.get('end_location', 0)
            )
            db.add(chapter)

        reading_progress = ReadingProgress(
            book_id=book.id,
            current_location=1,
            completion_percentage=0.0
        )

        db.add(reading_progress)
        db.commit()
        db.refresh(book)

        background_tasks.add_task(embed_book_content_task, book_id=book.id)

        return {
            "id": book.id,
            "title": book.title,
            "author": book.author,
            "cover_path": book.cover_path,
            "message": "Book uploaded successfully. AI indexing started in the background."
        }

    except BookParsingException as e:
        logger.error(f"Book parsing error: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail=str(e)
        )
    except FileStorageException as e:
        logger.error(f"File storage error: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=str(e)
        )
    except Exception as e:
        logger.error(f"Unexpected error during book upload: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"An unexpected error occurred: {str(e)}"
        )

@router.delete("/{book_id}", status_code=status.HTTP_204_NO_CONTENT)
async def delete_book(book_id: str, db: Session = Depends(get_db)):
    """
    Delete a book from the library
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book:
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail=f"Book with ID {book_id} not found"
            )
        try:
            if book.file_path and os.path.exists(book.file_path): os.remove(book.file_path)

            if book.cover_path and os.path.exists(book.cover_path): os.remove(book.cover_path)

            if book.content_path and os.path.exists(book.content_path):
                import shutil
                shutil.rmtree(book.content_path)

        except Exception as e: logger.warning(f"Error deleting book files: {str(e)}")

        try: await embedding_service.delete_book_embeddings(book_id)
        except Exception as e: logger.warning(f"Error deleting book embeddings: {str(e)}")

        db.delete(book)
        db.commit()

        return None

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to delete book: {str(e)}")
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail=f"Failed to delete book: {str(e)}"
        )

@router.get("/cover/{book_id}")
async def get_book_cover(book_id: str, db: Session = Depends(get_db)):
    """
    Get cover image of a book
    """
    try:
        book = db.query(Book).filter(Book.id == book_id).first()
        if not book: raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail=f"Book with ID {book_id} not found")

        if not book.cover_path or not os.path.exists(book.cover_path):
            raise HTTPException(
                status_code=status.HTTP_404_NOT_FOUND,
                detail="Cover image not found for this book"
            )

        return FileResponse(path=book.cover_path, media_type="image/jpeg", filename=os.path.basename(book.cover_path))

    except HTTPException: raise
    except Exception as e:
        logger.error(f"Failed to get book cover: {str(e)}")
        raise HTTPException( status_code=status.HTTP_500_INTERNAL_SERVER_ERROR, detail=f"Failed to get book cover: {str(e)}")