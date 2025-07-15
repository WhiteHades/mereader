"""
MeReader Reading Progress Pydantic Models
"""
from datetime import datetime
from typing import Optional
from pydantic import BaseModel

class ChapterInfo(BaseModel):
    """Model for chapter information"""
    id: str
    title: str
    order: int
    start_location: int
    end_location: int

class ProgressResponse(BaseModel):
    """Response model for reading progress"""
    book_id: str
    current_location: int
    completion_percentage: float
    current_chapter: Optional[ChapterInfo] = None
    last_read_at: Optional[datetime] = None

    class Config:
        """Pydantic config"""
        from_attributes = True