"""
MeReader Book Content Pydantic Models
"""
from typing import List
from pydantic import BaseModel

class ChapterContent(BaseModel):
    """Model for chapter content"""
    id: str
    title: str
    order: int
    start_location: int
    end_location: int
    content: str

class BookContentResponse(BaseModel):
    """Response model for book content"""
    book_id: str
    title: str
    chapters: List[ChapterContent]

class ChapterContentResponse(BaseModel):
    """Response model for chapter content"""
    book_id: str
    chapter_id: str
    title: str
    order: int
    start_location: int
    end_location: int
    content: str

class LocationTextResponse(BaseModel):
    """Response model for text at location"""
    book_id: str
    chapter_id: str
    chapter_title: str
    location: int
    text: str

class ChapterLocationResponse(BaseModel):
    """Response model for chapter by location"""
    id: str
    title: str
    order: int
    start_location: int
    end_location: int
    location_in_chapter: int
    total_locations_in_chapter: int