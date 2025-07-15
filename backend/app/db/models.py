"""
MeReader SQLAlchemy Database Models
"""
import uuid
from datetime import datetime, timezone
from sqlalchemy import Column, Integer, String, Float, DateTime, ForeignKey, Text, JSON
#from sqlalchemy.ext.declarative import declarative_base
from sqlalchemy.orm import relationship, declarative_base

Base = declarative_base()

class Book(Base):
    """Book model representing a book in the library"""
    __tablename__ = "books"

    id = Column(String, primary_key=True, default=lambda: str(uuid.uuid4()))
    title = Column(String, nullable=False)
    author = Column(String, nullable=False)
    file_path = Column(String, nullable=False)
    content_path = Column(String, nullable=True)
    cover_path = Column(String, nullable=True)
    language = Column(String, nullable=True)
    published_year = Column(Integer, nullable=True)
    publisher = Column(String, nullable=True)
    isbn = Column(String, nullable=True)
    description = Column(Text, nullable=True)
    content_length = Column(Integer, nullable=True)
    total_locations = Column(Integer, nullable=True)
    total_chapters = Column(Integer, nullable=True)
    book_metadata = Column(JSON, nullable=True)
    created_at = Column(DateTime, default=lambda: datetime.now(timezone.utc))
    updated_at = Column(DateTime, default=lambda: datetime.now(timezone.utc), onupdate=lambda: datetime.now(timezone.utc))

    reading_progress = relationship("ReadingProgress", back_populates="book", uselist=False, cascade="all, delete-orphan")
    chapters = relationship("Chapter", back_populates="book", cascade="all, delete-orphan")

class Chapter(Base):
    """Chapter model representing a chapter in a book"""
    __tablename__ = "chapters"

    id = Column(String, primary_key=True, default=lambda: str(uuid.uuid4()))
    book_id = Column(String, ForeignKey("books.id", ondelete="CASCADE"), nullable=False)
    title = Column(String, nullable=False)
    order = Column(Integer, nullable=False)
    content_path = Column(String, nullable=True)
    start_location = Column(Integer, nullable=True)
    end_location = Column(Integer, nullable=True)

    book = relationship("Book", back_populates="chapters")

class ReadingProgress(Base):
    """Reading progress model tracking user reading position"""
    __tablename__ = "reading_progress"

    id = Column(String, primary_key=True, default=lambda: str(uuid.uuid4()))
    book_id = Column(String, ForeignKey("books.id", ondelete="CASCADE"), unique=True, nullable=False)
    current_location = Column(Integer, default=0)
    current_chapter_id = Column(String, ForeignKey("chapters.id", ondelete="SET NULL"), nullable=True)
    completion_percentage = Column(Float, default=0.0)
    last_read_at = Column(DateTime, default=lambda: datetime.now(timezone.utc), onupdate=lambda: datetime.now(timezone.utc))

    book = relationship("Book", back_populates="reading_progress")
    current_chapter = relationship("Chapter")

class Settings(Base):
    """User settings model"""
    __tablename__ = "settings"

    id = Column(String, primary_key=True, default=lambda: str(uuid.uuid4()))
    theme = Column(String, default="dark")
    font_family = Column(String, default="Default")
    font_size = Column(Float, default=16.0)
    line_spacing = Column(Float, default=1.5)
    margin_size = Column(Float, default=16.0)
    text_alignment = Column(String, default="left")
    updated_at = Column(DateTime, default=lambda: datetime.now(timezone.utc), onupdate=lambda: datetime.now(timezone.utc))