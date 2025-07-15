"""
MeReader FastAPI Application Main
"""
import logging
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from app.db.sqlite import get_db, initialise_db
from app.api.routes import books, progress, query, content
from app.core.config import settings

logging.basicConfig(
    level=getattr(logging, settings.LOG_LEVEL),
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

initialise_db()

app = FastAPI(
    title="MeReader API",
    description="MeReader API",
    version="0.1.0",
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(books.router, prefix="/api/books", tags=["Books"])
app.include_router(content.router, prefix="/api/content", tags=["Content"])
app.include_router(progress.router, prefix="/api/progress", tags=["Reading Progress"])
app.include_router(query.router, prefix="/api/query", tags=["AI Queries"])