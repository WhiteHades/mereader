"""
MeReader Application Tests
Testing core functionality of the MeReader application
"""
import os
import uuid
import unittest
import tempfile
import shutil
import pytest
from datetime import datetime, timezone
from unittest.mock import MagicMock, patch
from fastapi.testclient import TestClient
import gc
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

from app.db.models import Base
from app.api.main import app
from app.db.sqlite import get_db
from app.db.models import Book, Chapter, ReadingProgress
from app.services.book_service import book_service
from app.services.location_service import location_service
from app.services.embedding_service import embedding_service
from app.services.content_service import content_service
from app.services.rag_service import rag_service
from app.services.ollama_service import ollama_service
from app.services.text_extraction_utility import text_extraction_util
from app.db.qdrant import qdrant_manager
import app.core.config as config

def setup_test_db():
    """Create a test database"""
    test_dir = tempfile.mkdtemp()
    db_file = os.path.join(test_dir, "test.db")
    db_url = f"sqlite:///{db_file}"

    engine = create_engine(db_url, connect_args={"check_same_thread": False})
    TestingSessionLocal = sessionmaker(autocommit=False, autoflush=False, bind=engine)

    Base.metadata.create_all(bind=engine)

    def override_get_db():
        try:
            db = TestingSessionLocal()
            yield db
        finally: db.close()

    app.dependency_overrides[get_db] = override_get_db

    return {
        'test_dir': test_dir,
        'db_file': db_file,
        'engine': engine,
        'SessionLocal': TestingSessionLocal,
    }

@pytest.mark.asyncio
class TestMeReader(unittest.TestCase):
    """MeReader application tests"""

    @classmethod
    def setUpClass(cls):
        """Set up test environment once for all tests"""
        cls.db_setup = setup_test_db()
        cls.client = TestClient(app)

        cls.test_dir = cls.db_setup['test_dir']
        cls.test_uploads_dir = os.path.join(cls.test_dir, "uploads")
        cls.test_content_dir = os.path.join(cls.test_dir, "contents")
        cls.test_cover_dir = os.path.join(cls.test_dir, "covers")
        cls.test_qdrant_dir = os.path.join(cls.test_dir, "qdrant")
        cls.test_bm25_dir = os.path.join(cls.test_dir, "bm25_cache")

        os.makedirs(cls.test_uploads_dir, exist_ok=True)
        os.makedirs(cls.test_content_dir, exist_ok=True)
        os.makedirs(cls.test_cover_dir, exist_ok=True)
        os.makedirs(cls.test_qdrant_dir, exist_ok=True)
        os.makedirs(cls.test_bm25_dir, exist_ok=True)

        cls.mock_epub_file()
        cls.test_settings()

    @classmethod
    def tearDownClass(cls):
        """Delete all artifacts after tests"""
        gc.collect()

        if hasattr(cls.db_setup, 'engine'): cls.db_setup['engine'].dispose()

        try:
            if os.path.exists(cls.test_dir) and 'temp' in cls.test_dir.lower():
                for _ in range(3):
                    try:
                        shutil.rmtree(cls.test_dir, ignore_errors=True)
                        break
                    except PermissionError:
                        import time
                        time.sleep(0.5)
                        gc.collect()
        except Exception as e: print(f"Warning: Could not clean up test directory: {e}")

    @classmethod
    def mock_epub_file(cls):
        """Mock EPUB file"""
        cls.mock_epub_path = os.path.join(cls.test_uploads_dir, "test_book.epub")
        mock_epub_content = b'PK\x03\x04\x14\x00\x00\x00\x08\x00\x00\x00!\x00'

        with open(cls.mock_epub_path, 'wb') as f: f.write(mock_epub_content)

    @classmethod
    def test_settings(cls):
        """Settings for testing"""
        cls.original_settings = config.settings

        # Create test settings
        test_settings = MagicMock()
        test_settings.UPLOAD_DIR = cls.test_uploads_dir
        test_settings.CONTENT_DIR = cls.test_content_dir
        test_settings.COVER_DIR = cls.test_cover_dir
        test_settings.QDRANT_LOCATION = cls.test_qdrant_dir
        test_settings.BM25_INDEX_CACHE_DIR = cls.test_bm25_dir
        test_settings.OLLAMA_BASE_URL = "http://localhost:11434"
        test_settings.OLLAMA_LLM_MODEL = "llama3.2:latest"
        test_settings.OLLAMA_EMBEDDING_MODEL = "nomic-embed-text:latest"
        test_settings.CHUNK_SIZE = 400
        test_settings.CHUNK_OVERLAP = 100
        test_settings.LOCATION_CHUNK_SIZE = 1000
        test_settings.CONTENT_THRESHOLD = 0.45
        test_settings.SUMMARY_THRESHOLD = 0.55
        test_settings.BM25_RESULTS_LIMIT = 10
        test_settings.BM25_WEIGHT = 0.4

        config.settings = test_settings

    def setUp(self):
        """Set up db before each test"""
        self.db = next(app.dependency_overrides[get_db]())

        self.db.query(ReadingProgress).delete()
        self.db.query(Chapter).delete()
        self.db.query(Book).delete()
        self.db.commit()

    def tearDown(self):
        """Clean up after each test"""
        self.db.close()

    def test_api_books_list_empty(self):
        """Test listing books when library is empty"""
        response = self.client.get("/api/books/")
        self.assertEqual(response.status_code, 200)
        data = response.json()
        self.assertEqual(len(data["books"]), 0)
        self.assertEqual(data["total"], 0)

    @patch.object(book_service, 'parse_book')
    def test_api_book_upload(self, mock_parse_book):
        """Test book upload API"""
        book_id = str(uuid.uuid4())
        mock_parse_book.return_value = {
            'id': book_id,
            'metadata': {
                'title': 'Test Book',
                'author': 'Test Author',
                'cover_path': os.path.join(self.test_cover_dir, f"{book_id}_cover.jpg"),
            },
            'chapters': [
                {
                    'id': 'ch1',
                    'title': 'Chapter 1',
                    'order': 1,
                    'content_path': os.path.join(self.test_content_dir, f"{book_id}/chapter_1.html"),
                    'start_location': 1,
                    'end_location': 10,
                    'char_count': 1000
                }
            ],
            'content_dir': os.path.join(self.test_content_dir, book_id),
            'total_locations': 10
        }

        # Create test directory
        os.makedirs(os.path.join(self.test_content_dir, book_id), exist_ok=True)

        # Create empty test cover file
        cover_path = os.path.join(self.test_cover_dir, f"{book_id}_cover.jpg")
        with open(cover_path, "w") as f:
            f.write("")

        # Create test chapter file
        chapter_path = os.path.join(self.test_content_dir, f"{book_id}/chapter_1.html")
        with open(chapter_path, "w") as f:
            f.write("<html><body><p>Test content</p></body></html>")

        # Create test file for upload
        with open(self.mock_epub_path, "rb") as f:
            mock_file_content = f.read()

        # Mock background tasks
        with patch('app.api.routes.books.embedding_service.embed_book_content') as mock_embed:
            # Use a completed future for mocking
            from asyncio import Future
            future = Future()
            future.set_result(10)
            mock_embed.return_value = future

            # Post file through API
            response = self.client.post(
                "/api/books/upload",
                files={"file": ("test_book.epub", mock_file_content, "application/epub+zip")}
            )

            self.assertEqual(response.status_code, 201)
            data = response.json()
            self.assertEqual(data["title"], "Test Book")
            self.assertEqual(data["author"], "Test Author")

            # Verify database entry was created
            book = self.db.query(Book).first()
            self.assertIsNotNone(book)
            self.assertEqual(book.title, "Test Book")

            # Verify chapter was created
            chapter = self.db.query(Chapter).first()
            self.assertIsNotNone(chapter)
            self.assertEqual(chapter.title, "Chapter 1")

            # Verify reading progress was created
            progress = self.db.query(ReadingProgress).first()
            self.assertIsNotNone(progress)
            self.assertEqual(progress.current_location, 1)
            self.assertEqual(progress.completion_percentage, 0.0)

    def test_location_service(self):
        """Test location service functionality"""
        # calculate_locations
        html_content = "<html><body>" + "<p>Test content</p>" * 100 + "</body></html>"
        locations = location_service.calculate_locations(html_content)
        self.assertGreater(locations, 0)

        # get_percentage_from_location
        percentage = location_service.get_percentage_from_location(5, 10)
        self.assertEqual(percentage, 50.0)

        # get_location_from_percentage
        location = location_service.get_location_from_percentage(50.0, 10)
        self.assertEqual(location, 5)

        # calculate_location_boundary
        boundary = location_service.calculate_location_boundary(5, 10)
        self.assertEqual(boundary, 5)

    @patch('app.services.content_service.BeautifulSoup')
    def test_content_service(self, mock_bs):
        """Test content service functionality"""
        mock_soup = MagicMock()
        mock_bs.return_value = mock_soup

        mock_soup.find_all.return_value = []
        mock_soup.get_text.return_value = "Test content"
        mock_soup.body = MagicMock()
        mock_soup.body.contents = ["Test content"]

        html_content = "<html><head><title>Test</title></head><body><p>Test content</p></body></html>"

        with patch('app.services.content_service.BeautifulSoup', return_value=mock_soup):
            processed_html = content_service.process_html_content(html_content)
            self.assertIn("Test content", processed_html)

    @patch('app.services.text_extraction_utility.BeautifulSoup')
    def test_text_extraction(self, mock_bs):
        """Test text extraction functionality"""
        mock_soup = MagicMock()
        mock_bs.return_value = mock_soup
        mock_soup.get_text.return_value = "Test content"

        test_html_path = os.path.join(self.test_dir, "test.html")
        with open(test_html_path, "w") as f: f.write("<html><body><p>Test content</p></body></html>")

        with patch('app.services.text_extraction_utility.BeautifulSoup', return_value=mock_soup):
            text = text_extraction_util.extract_text_streamed(test_html_path)
            self.assertEqual(text, "Test content")

    @pytest.mark.asyncio
    async def test_embedding_service(self):
        """Test embedding service functionality"""
        with patch.object(ollama_service, 'generate_embedding', return_value=[0.1] * 768):
            # Test embed_single_text
            embedding = await embedding_service.embed_single_text("Test content")
            self.assertEqual(len(embedding), 768)

    @pytest.mark.asyncio
    async def test_rag_service(self):
        """Test RAG service functionality"""
        with patch('os.path.exists', return_value=True):
            book_id = str(uuid.uuid4())
            book = Book(
                id=book_id,
                title="Test Book",
                author="Test Author",
                file_path=self.mock_epub_path,
                content_path=os.path.join(self.test_content_dir, f"test_book_{book_id}"),
                total_locations=100
            )
            self.db.add(book)

            chapter_id = str(uuid.uuid4())
            chapter = Chapter(
                id=chapter_id,
                book_id=book.id,
                title="Chapter 1",
                order=1,
                content_path=os.path.join(self.test_content_dir, f"test_book_{book_id}/chapter_1.html"),
                start_location=1,
                end_location=50
            )
            self.db.add(chapter)

            reading_progress = ReadingProgress(
                book_id=book.id,
                current_location=25,
                completion_percentage=25.0,
                last_read_at=datetime.now(timezone.utc)
            )
            self.db.add(reading_progress)

            self.db.commit()

            with patch.object(embedding_service, 'embed_single_text', return_value=[0.1] * 768):
                with patch.object(qdrant_manager, 'search_vectors', return_value=[
                    {
                        "id": "1",
                        "score": 0.9,
                        "text": "This is a test passage from the book.",
                        "chapter_title": "Chapter 1",
                        "chapter_id": chapter.id,
                        "location": 20,
                        "search_method": "vector"
                    }
                ]):
                    with patch.object(ollama_service, 'generate_completion',
                                    return_value="This is a test response from the AI."):

                        result = await rag_service.process_query(
                            book_id=book.id,
                            query="What happens in this book?",
                            reading_progress=reading_progress,
                            db=self.db
                        )

                        # Verify result
                        self.assertEqual(result["response"], "This is a test response from the AI.")
                        self.assertEqual(result["book_title"], "Test Book")
                        self.assertEqual(len(result["context_used"]), 1)

    @patch('app.api.routes.progress.datetime')
    async def test_api_progress(self, mock_datetime):
        """Test reading progress API"""
        mock_now = datetime.now(timezone.utc)
        mock_datetime.now.return_value = mock_now
        mock_datetime.utcnow.return_value = mock_now.replace(tzinfo=None)

        book = Book(
            id=str(uuid.uuid4()),
            title="Test Book",
            author="Test Author",
            file_path=self.mock_epub_path,
            content_path=os.path.join(self.test_content_dir, "test_book"),
            total_locations=100
        )
        self.db.add(book)
        self.db.commit()

        response = self.client.get(f"/api/progress/{book.id}")
        self.assertEqual(response.status_code, 200)
        data = response.json()
        self.assertEqual(data["current_location"], 1)
        self.assertEqual(data["completion_percentage"], 0.0)

        update_data = {
            "current_location": 50,
            "completion_percentage": 50.0
        }
        response = self.client.put(
            f"/api/progress/{book.id}",
            json=update_data
        )
        self.assertEqual(response.status_code, 200)
        data = response.json()
        self.assertEqual(data["current_location"], 50)
        self.assertEqual(data["completion_percentage"], 50.0)

        progress = self.db.query(ReadingProgress).filter_by(book_id=book.id).first()
        self.assertEqual(progress.current_location, 50)
        self.assertEqual(progress.completion_percentage, 50.0)

    @pytest.mark.asyncio
    def test_api_query(self):
        """Test AI query API"""
        book = Book(
            id=str(uuid.uuid4()),
            title="Test Book",
            author="Test Author",
            file_path=self.mock_epub_path,
            content_path=os.path.join(self.test_content_dir, "test_book"),
            total_locations=100
        )
        self.db.add(book)

        reading_progress = ReadingProgress(
            book_id=book.id,
            current_location=50,
            completion_percentage=50.0,
            last_read_at=datetime.now(timezone.utc)
        )
        self.db.add(reading_progress)

        self.db.commit()

        with patch.object(ollama_service, 'check_status') as mock_check_status:
            mock_check_status.return_value = True

            with patch.object(rag_service, 'process_query') as mock_process_query:
                mock_process_query.return_value = {
                    "response": "This is a test response from the AI.",
                    "context_used": [
                        {
                            "text": "This is a test passage from the book.",
                            "chapter_title": "Chapter 1",
                            "location": 20,
                            "relevance_score": 0.9
                        }
                    ],
                    "query": "What happens in this book?",
                    "book_id": book.id,
                    "book_title": "Test Book",
                    "location_boundary": 50,
                    "progress_boundary": 50.0
                }

                response = self.client.post(
                    f"/api/query/ask/{book.id}",
                    json={"query": "What happens in this book?"}
                )

                self.assertEqual(response.status_code, 200)
                data = response.json()
                self.assertEqual(data["response"], "This is a test response from the AI.")
                self.assertEqual(len(data["context_used"]), 1)


if __name__ == "__main__":
    pytest.main()