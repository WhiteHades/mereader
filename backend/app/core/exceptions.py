"""
MeReader Custom Exception Classes
"""
from fastapi import status

class MeReaderException(Exception):
    """Base exception for MeReader application"""

    def __init__(
        self,
        detail: str = "An error occurred",
        status_code: int = status.HTTP_500_INTERNAL_SERVER_ERROR
    ):
        self.detail = detail
        self.status_code = status_code
        super().__init__(self.detail)

class BookNotFoundException(MeReaderException):
    """Exception raised when a requested book is not found"""

    def __init__(self, book_id: str):
        super().__init__(
            detail=f"Book with ID {book_id} not found",
            status_code=status.HTTP_404_NOT_FOUND
        )

class BookParsingException(MeReaderException):
    """Exception raised when EPUB parsing fails"""

    def __init__(self, detail: str = "Failed to parse book file"):
        super().__init__(
            detail=detail,
            status_code=status.HTTP_400_BAD_REQUEST
        )

class VectorStoreException(MeReaderException):
    """Exception raised when vector storage operations fail"""

    def __init__(self, detail: str = "Vector store operation failed"):
        super().__init__(
            detail=detail,
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR
        )

class OllamaServiceException(MeReaderException):
    """Exception raised when Ollama service fails"""

    def __init__(self, detail: str = "Ollama service operation failed"):
        super().__init__(
            detail=detail,
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE
        )

class DatabaseException(MeReaderException):
    """Exception raised when database operations fail"""

    def __init__(self, detail: str = "Database operation failed"):
        super().__init__(
            detail=detail,
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR
        )

class AIQueryException(MeReaderException):
    """Exception raised when AI query fails"""

    def __init__(self, detail: str = "Failed to process AI query"):
        super().__init__(
            detail=detail,
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR
        )

class FileStorageException(MeReaderException):
    """Exception raised when file storage operations fail"""

    def __init__(self, detail: str = "File storage operation failed"):
        super().__init__(
            detail=detail,
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR
        )