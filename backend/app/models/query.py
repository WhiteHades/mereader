"""
MeReader AI Query Pydantic Models
"""
from typing import List
from pydantic import BaseModel, Field

class QueryRequest(BaseModel):
    """Request model for AI queries"""
    query: str = Field(..., description="The question to ask about the book")

class ContextPassage(BaseModel):
    """Model for context passages used in AI responses"""
    text: str
    chapter_title: str
    location: int
    relevance_score: float

class QueryResponse(BaseModel):
    """Response model for AI queries"""
    response: str
    query: str
    book_id: str
    book_title: str
    context_used: List[ContextPassage]
    location_boundary: int
    progress_boundary: float

class ChatMessage(BaseModel):
    """Model for chat messages"""
    role: str = Field(..., description="Message role (system, user, assistant)")
    content: str = Field(..., description="Message content")

class ChatQueryRequest(BaseModel):
    """Request model for chat queries"""
    messages: List[ChatMessage] = Field(..., description="Chat conversation history")

class ChatQueryResponse(BaseModel):
    """Response model for chat queries"""
    response: str
    book_id: str
    book_title: str
    context_used: List[ContextPassage]
    location_boundary: int
    progress_boundary: float
    messages: List[ChatMessage]