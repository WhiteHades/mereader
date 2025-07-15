"""
MeReader Book Pydantic Models
"""

from datetime import datetime
from typing import List, Dict, Any, Optional
from pydantic import BaseModel, Field


class ChapterBase(BaseModel):
    """Base model for chapter data"""

    id: str
    title: str
    order: int
    start_location: int
    end_location: int


class BookBase(BaseModel):
    """Base model for book data"""

    id: str
    title: str
    author: str
    cover_path: Optional[str] = None


class BookResponse(BookBase):
    """Response model for book creation"""

    message: str


class BookListItem(BookBase):
    """Book item in list response"""

    completion_percentage: float
    last_read_at: Optional[datetime] = None


class BookListResponse(BaseModel):
    """Response model for listing books"""

    books: List[BookListItem]
    total: int


class BookDetailResponse(BookBase):
    """Response model for detailed book information"""

    content_path: Optional[str] = None
    language: Optional[str] = None
    published_year: Optional[int] = None
    publisher: Optional[str] = None
    isbn: Optional[str] = None
    description: Optional[str] = None
    content_length: Optional[int] = None
    total_locations: Optional[int] = None
    total_chapters: int
    chapters: List[ChapterBase]
    completion_percentage: float
    current_location: int
    last_read_at: Optional[datetime] = None
    metadata: Optional[Dict[str, Any]] = Field(None, alias="book_metadata")

    class Config:
        """Pydantic config"""

        from_attributes = True


class EmbeddingStatusResponse(BaseModel):
    """Response model for embedding status"""

    book_id: str
    has_vectors: bool

