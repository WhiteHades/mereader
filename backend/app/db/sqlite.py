"""
MeReader SQLite Database Connection
"""
import logging
import os
from contextvars import ContextVar
from typing import Iterator
from sqlalchemy import create_engine, event
from sqlalchemy.orm import sessionmaker, Session
from sqlalchemy.pool import QueuePool
from app.core.config import settings
from app.core.exceptions import DatabaseException
from app.db.models import Base

logger = logging.getLogger(__name__)

db_dir = os.path.dirname(settings.SQLITE_DB_FILE)
os.makedirs(db_dir, exist_ok=True)

SQLALCHEMY_DATABASE_URL = f"sqlite:///{settings.SQLITE_DB_FILE}"
engine = create_engine(
    SQLALCHEMY_DATABASE_URL,
    connect_args={"check_same_thread": False},
    poolclass=QueuePool,
    pool_size=40,
    max_overflow=40,
    pool_timeout=60,
    pool_recycle=1800,
    pool_pre_ping=True
)

SessionLocal = sessionmaker(autocommit=False, autoflush=False, bind=engine)
session_context: ContextVar[Session] = ContextVar("session_context")

@event.listens_for(engine, "connect")
def set_sqlite_pragma(dbapi_connection, connection_record):
    cursor = dbapi_connection.cursor()
    cursor.execute("PRAGMA journal_mode=WAL")  # write-ahead log for better concurrency
    cursor.execute("PRAGMA synchronous=NORMAL")
    cursor.execute("PRAGMA cache_size=10000")  # 10mb cache
    cursor.execute("PRAGMA temp_store=MEMORY")
    cursor.execute("PRAGMA mmap_size=10000000000")  # 10gb storage for I/O
    cursor.execute("PRAGMA busy_timeout=30000")  # 30sec timeout if busy connection
    cursor.close()

def get_db() -> Iterator[Session]:
    """Get database session dependency."""
    db = SessionLocal()
    token = session_context.set(db)
    try: yield db
    finally:
        try:
            session_context.reset(token)
            db.close()
        except ValueError: pass

def get_current_session() -> Session:
    """Get current active database session from context"""
    try:
        return session_context.get()
    except LookupError:
        raise DatabaseException("No database session found in current context")

def initialise_db():
    """Initialise db connections and create tables if they don't exist"""
    try:
        Base.metadata.create_all(bind=engine)
        logger.info("Database initialised - tables created if they didn't exist")
    except Exception as e:
        raise DatabaseException(f"Failed to initialize database: {str(e)}")

def close_db_connection():
    """Close db connection"""
    try:
        engine.dispose()
        logger.info("Database connection closed")
    except Exception as e:
        logger.error(f"Failed to close database connection: {str(e)}")