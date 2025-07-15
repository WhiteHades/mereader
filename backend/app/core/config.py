"""
MeReader Application Configuration
"""
import os
from typing import Dict, Any
from pydantic import validator
from pydantic_settings import BaseSettings

class Settings(BaseSettings):
    """Application settings"""
    APP_NAME: str = "MeReader"
    LOG_LEVEL: str = "INFO"

    BM25_INDEX_CACHE_DIR: str = "data/bm25_cache"
    BM25_RESULTS_LIMIT: int = 10
    BM25_WEIGHT: float = 0.4

    # db paths
    SQLITE_DB_FILE: str = "data/mereader.db"
    QDRANT_LOCATION: str = "data/qdrant"
    QDRANT_COLLECTION_NAME: str = "mereader_books"
    QDRANT_VECTOR_SIZE: int = 768

    # storage paths
    UPLOAD_DIR: str = "data/uploads"
    CONTENT_DIR: str = "data/contents"
    COVER_DIR: str = "data/covers"

    # Ollama settings
    OLLAMA_BASE_URL: str = "http://localhost:11434"
    # OLLAMA_LLM_MODEL: str = "gemma3:4b"
    OLLAMA_LLM_MODEL: str = "llama3.2:3b"
    # OLLAMA_LLM_MODEL: str = "llama3.2:1b"
    # OLLAMA_LLM_MODEL: str = "qwen:4b"
    OLLAMA_EMBEDDING_MODEL: str = "nomic-embed-text:latest"

    INITIAL_CONTENT_LIMIT: int = 100
    INITIAL_SUMMARY_LIMIT: int = 10
    CONTENT_THRESHOLD: float = 0.45
    SUMMARY_THRESHOLD: float = 0.55
    RERANK_TRIGGER_LIMIT: int = 10
    FINAL_CONTEXT_LIMIT: int = 25
    MAX_RESULTS_TO_RERANK: int = 40
    MAX_CONTEXT_TOKENS: int = 5000

    CHUNK_SIZE: int = 400
    CHUNK_OVERLAP: int = 100

    # chars per location unit
    LOCATION_CHUNK_SIZE: int = 1000

    class Config:
        """Pydantic config"""
        env_file = ".env"
        env_file_encoding = "utf-8"

    @validator("UPLOAD_DIR", "CONTENT_DIR", "COVER_DIR", "QDRANT_LOCATION")
    def create_directories(cls, directory_path):
        """Ensure directories exist"""
        os.makedirs(directory_path, exist_ok=True)
        return directory_path

    def get_qdrant_config(self) -> Dict[str, Any]:
        """Return Qdrant configuration dictionary"""
        return {
            "location": self.QDRANT_LOCATION,
            "collection_name": self.QDRANT_COLLECTION_NAME,
            "vector_size": self.QDRANT_VECTOR_SIZE,
        }

    def get_ollama_config(self) -> Dict[str, Any]:
        """Return Ollama configuration dictionary"""
        return {
            "base_url": self.OLLAMA_BASE_URL,
            "llm_model": self.OLLAMA_LLM_MODEL,
            "embedding_model": self.OLLAMA_EMBEDDING_MODEL,
        }

settings = Settings()

#  all dirs exist
os.makedirs(settings.UPLOAD_DIR, exist_ok=True)
os.makedirs(settings.CONTENT_DIR, exist_ok=True)
os.makedirs(settings.COVER_DIR, exist_ok=True)
os.makedirs(settings.QDRANT_LOCATION, exist_ok=True)
os.makedirs(settings.BM25_INDEX_CACHE_DIR, exist_ok=True)