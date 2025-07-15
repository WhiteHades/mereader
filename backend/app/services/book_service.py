"""
MeReader Book Service - EPUB Parsing and Processing
"""
import logging
import os
import shutil
import uuid
from importlib import metadata
from pathlib import Path
from typing import Dict, List, Any, Tuple, Optional
import re
import ebooklib
from ebooklib import epub
from bs4 import BeautifulSoup
from PIL import Image
from app.core.config import settings
from app.core.exceptions import BookParsingException, FileStorageException
from app.services.location_service import LocationService
from app.services.content_service import ContentService

logger = logging.getLogger(__name__)

class BookService:
    """Service for parsing and processing EPUB files"""

    def __init__(self):
        self.upload_dir = settings.UPLOAD_DIR
        self.content_dir = settings.CONTENT_DIR
        self.cover_dir = settings.COVER_DIR
        self.content_service = ContentService()
        self.location_service = LocationService()

        os.makedirs(self.upload_dir, exist_ok=True)
        os.makedirs(self.content_dir, exist_ok=True)
        os.makedirs(self.cover_dir, exist_ok=True)

        logger.info(f"Book service initialised with upload directory: {self.upload_dir}")

    def _extract_metadata(self, book: epub.EpubBook) -> Dict[str, Any]:
        """
        Extract metadata from EPUB
        Args:
            book: EpubBook object
        Returns:
            Dictionary with metadata
        """
        metadata = {'title': book.get_metadata('DC', 'title')[0][0] if book.get_metadata('DC', 'title') else "Unknown Title",
                    'author': book.get_metadata('DC', 'creator')[0][0] if book.get_metadata('DC', 'creator') else "Unknown Author",
                    'language': book.get_metadata('DC', 'language')[0][0] if book.get_metadata('DC', 'language') else None,
                    'publisher': book.get_metadata('DC', 'publisher')[0][0] if book.get_metadata('DC', 'publisher') else None,
                    'description': book.get_metadata('DC', 'description')[0][0] if book.get_metadata('DC', 'description') else None}
        # isbn
        identifiers = book.get_metadata('DC', 'identifier')
        if identifiers: metadata['isbn'] = identifiers[0][0]

        # publication date
        dates = book.get_metadata('DC', 'date')
        if dates:
            date_str = dates[0][0]
            try:
                if '-' in date_str: metadata['published_year'] = int(date_str.split('-')[0])
                elif '/' in date_str: metadata['published_year'] = int(date_str.split('/')[0])
                else: metadata['published_year'] = int(date_str[:4])
            except (ValueError, IndexError): metadata['published_year'] = None

        return metadata

    def _extract_cover(self, book: epub.EpubBook, book_id: str) -> Optional[str]:
        """
        Extract cover image from EPUB book
        Args:
            book: EpubBook object
            book_id: ID of the book for file naming
        Returns:
            Path to the extracted cover image or None
        """
        try:
            cover_image = None

            # 1: cover item
            for item in book.get_items():
                if item.get_type() == ebooklib.ITEM_COVER:
                    cover_image = item
                    break

            # 2: image with 'cover' in ID
            if not cover_image:
                for item in book.get_items_of_type(ebooklib.ITEM_IMAGE):
                    if 'cover' in item.id.lower():
                        cover_image = item
                        break

            # 3: first image in the book
            if not cover_image:
                for item in book.get_items_of_type(ebooklib.ITEM_IMAGE):
                    cover_image = item
                    break

            if cover_image:
                cover_filename = f"{book_id}_cover.jpg"
                cover_path = os.path.join(self.cover_dir, cover_filename)

                with open(cover_path, 'wb') as f: f.write(cover_image.get_content())
                img = Image.open(cover_path)
                if img.mode == 'RGBA': img = img.convert('RGB')
                img.save(cover_path, 'JPEG')

                return cover_path

            return None

        except Exception as e:
            logger.warning(f"Failed to extract cover image: {str(e)}")
            return None

    def _extract_content(self, book: epub.EpubBook) -> Tuple[List[Dict[str, Any]], Dict[str, str]]:
        """
        Extract chapters and content from EPUB book
        Args:
            book: EpubBook object
        Returns:
            Tuple of (chapters list, content by chapter dict)
        """
        chapters = []
        content_by_chapter = {}
        all_items = {}
        toc_items = set()

        # processing all items in spine
        for spine_index, spine_id in enumerate(book.spine):
            if isinstance(spine_id, tuple): spine_id = spine_id[0]
            if spine_id.startswith('nav'): continue
            item = book.get_item_with_id(spine_id)
            if item and item.get_type() == ebooklib.ITEM_DOCUMENT: all_items[spine_index] = item

        toc_chapters = []

        def process_toc_entries(entries, parent_order=0):
            nonlocal toc_chapters, toc_items
            order = parent_order * 100

            for entry in entries:
                if isinstance(entry, tuple) and len(entry) > 1:
                    # (title, href)
                    title, href = entry[0], entry[1]
                    if href:
                        order += 1
                        item = book.get_item_with_href(href)
                        if item:
                            spine_index = None
                            for i, spine_id in enumerate(book.spine):
                                if isinstance(spine_id, tuple): spine_id = spine_id[0]
                                if spine_id == item.id:
                                    spine_index = i
                                    break

                            if spine_index is not None:
                                toc_items.add(item.id)
                                chapter_id = f"ch{len(toc_chapters) + 1}"
                                chapter = {
                                    'id': chapter_id,
                                    'title': title,
                                    'order': order,
                                    'href': item.file_name,
                                    'spine_index': spine_index
                                }
                                toc_chapters.append(chapter)

                                html_content = item.get_content().decode('utf-8')
                                processed_html = self.content_service.process_html_content(html_content)
                                content_by_chapter[chapter_id] = processed_html

                elif isinstance(entry, list): process_toc_entries(entry, parent_order=order)

                elif hasattr(entry, 'title') and hasattr(entry, 'href'):
                    order += 1
                    href = entry.href
                    item = book.get_item_with_href(href)
                    if item:
                        spine_index = None
                        for i, spine_id in enumerate(book.spine):
                            if isinstance(spine_id, tuple): spine_id = spine_id[0]
                            if spine_id == item.id:
                                spine_index = i
                                break

                        if spine_index is not None:
                            toc_items.add(item.id)
                            chapter_id = f"ch{len(toc_chapters) + 1}"
                            chapter = {
                                'id': chapter_id,
                                'title': entry.title,
                                'order': order,
                                'href': item.file_name,
                                'spine_index': spine_index
                            }
                            toc_chapters.append(chapter)

                            html_content = item.get_content().decode('utf-8')
                            processed_html = self.content_service.process_html_content(html_content)
                            content_by_chapter[chapter_id] = processed_html

        if book.toc: process_toc_entries(book.toc)
        if len(toc_chapters) > 0:
            logger.info(f"Using {len(toc_chapters)} chapters from TOC")
            return toc_chapters, content_by_chapter

        spine_chapters = []
        spine_content = {}
        for spine_index, item in sorted(all_items.items()):
            html_content = item.get_content().decode('utf-8')
            soup = BeautifulSoup(html_content, 'html.parser')

            title = None
            for heading in soup.find_all(['h1', 'h2', 'h3', 'h4'], limit=1):
                title = heading.get_text().strip()
                break

            if not title:
                title_elem = soup.find('title')
                if title_elem: title = title_elem.get_text().strip()

            if not title:
                for elem in soup.find_all(['div', 'section']):
                    elem_id = elem.get('id', '').lower()
                    elem_class = ' '.join(elem.get('class', [])).lower()
                    if any(word in elem_id or word in elem_class for word in ['title', 'heading', 'chapter']):
                        title = elem.get_text().strip()
                        break
            if not title or title == "":
                content_text = soup.get_text().strip()
                patterns = [
                    r'^(Prologue|Epilogue|Afterword|Foreword|Introduction|Preface|Appendix|Notes)[\s\:\.\n]',
                    r'^(Chapter|Section)\s+([IVXLCDM]+|[0-9]+)[\s\:\.\n]',  # Roman and Arabic numerals
                ]

                for pattern in patterns:
                    match = re.search(pattern, content_text, re.IGNORECASE)
                    if match:
                        title_para = re.search(r'^.*?[\.\!\?](?=\s|$)', content_text)
                        if title_para: title = title_para.group(0).strip()
                        else: title = match.group(0).strip()
                        break

            if not title or title == "":
                match = re.search(r'(chapter[_\-\s]?(\d+)|epilogue|prologue)', item.file_name.lower())
                if match:
                    title_type = match.group(1)
                    if 'chapter' in title_type and match.group(2):
                        chap_num = match.group(2)
                        chapter_pattern = re.search(r'Chapter\s+' + chap_num + r'[:\.\s]+(.*?)[\.\!\?](?=\s|$)', soup.get_text(), re.IGNORECASE)
                        if chapter_pattern and chapter_pattern.group(1).strip(): title = f"Chapter {chap_num}: {chapter_pattern.group(1).strip()}"
                        else: title = f"Chapter {chap_num}"
                    else: title = title_type.capitalize()
                else: title = f"Section {spine_index + 1}"

            # chapter uuid
            chapter_id = f"sp{spine_index + 1}"
            chapter = {
                'id': chapter_id,
                'title': title,
                'order': spine_index + 1,
                'href': item.file_name,
                'spine_index': spine_index
            }
            spine_chapters.append(chapter)

            processed_html = self.content_service.process_html_content(html_content)
            spine_content[chapter_id] = processed_html

        if len(spine_chapters) > 0:
            logger.info(f"Using {len(spine_chapters)} chapters from spine")
            return spine_chapters, spine_content
        else:
            logger.warning("No chapters found, creating a single chapter with all content")
            all_content = ""
            for spine_index, item in sorted(all_items.items()):
                html_content = item.get_content().decode('utf-8')
                all_content += self.content_service.process_html_content(html_content)

            single_chapter = {
                'id': 'ch1',
                'title': metadata.get('title', 'Full Content'),
                'order': 1,
                'href': None,
                'spine_index': 0
            }
            return [single_chapter], {'ch1': all_content}

    def _extract_book_images(self, book: epub.EpubBook, book_content_dir: str) -> Dict[str, str]:
        """
        Extract images from EPUB book and save them in the book content directory
        Args:
            book: EpubBook object
            book_content_dir: Directory where book content is stored
        Returns:
            Dictionary mapping from image IDs/paths to filenames
        """
        image_mapping = {}
        os.makedirs(book_content_dir, exist_ok=True)

        for item in book.get_items_of_type(ebooklib.ITEM_IMAGE):
            try:
                img_filename = os.path.basename(item.file_name)
                img_filename = re.sub(r'[^\w\.\-]', '_', img_filename)
                img_path = os.path.join(book_content_dir, img_filename)

                with open(img_path, 'wb') as f: f.write(item.get_content())

                image_mapping[item.file_name] = img_filename
                if item.id:
                    image_mapping[item.id] = img_filename
                    image_mapping[f"#{item.id}"] = img_filename
                logger.info(f"Extracted image: {img_filename} to {book_content_dir}")

            except Exception as e: logger.warning(f"Failed to extract image {item.file_name}: {str(e)}")

        return image_mapping

    def save_uploaded_file(self, file_content: bytes, filename: str) -> str:
        """
        Save uploaded EPUB file to disk
        Args:
            file_content: EPUB file content as bytes
            filename: Original filename
        Returns:
            Path to the saved file
        """
        try:
            if not filename.lower().endswith('.epub'): filename = f"{Path(filename).stem}.epub"
            safe_filename = filename.replace(' ', '_')
            file_path = os.path.join(self.upload_dir, safe_filename)

            counter = 1
            while os.path.exists(file_path):
                name_parts = Path(safe_filename).stem.rsplit('_', 1)
                if len(name_parts) > 1 and name_parts[-1].isdigit():
                    base_name = name_parts[0]
                    counter = int(name_parts[-1]) + 1
                else: base_name = Path(safe_filename).stem

                new_filename = f"{base_name}_{counter}.epub"
                file_path = os.path.join(self.upload_dir, new_filename)
                counter += 1

            with open(file_path, "wb") as f: f.write(file_content)
            logger.info(f"EPUB file saved to {file_path}")

            return file_path

        except Exception as e:
            logger.error(f"Failed to save uploaded file: {str(e)}")
            raise FileStorageException(f"Failed to save uploaded file: {str(e)}")

    def parse_book(self, file_path: str) -> Dict[str, Any]:
        """
        Parse EPUB file and extract content, metadata, and structure.
        This method extracts all necessary information from the EPUB file
        and processes the content into a format easily consumable by the frontend
        Args:
            file_path: Path to the EPUB file
        Returns:
            Dictionary with parsed book data
        """
        try:
            logger.info(f"Parsing EPUB file: {file_path}")
            book = epub.read_epub(file_path)

            metadata = self._extract_metadata(book)
            logger.info(f"Extracted metadata for book: {metadata.get('title', 'Unknown')}")

            # book uuid
            book_id = str(uuid.uuid4())
            book_content_dir = os.path.join(self.content_dir, book_id)
            os.makedirs(book_content_dir, exist_ok=True)
            image_mapping = self._extract_book_images(book, book_content_dir)
            logger.info(f"Extracted {len(image_mapping)} images for book {book_id}")

            cover_path = self._extract_cover(book, book_id)
            if cover_path:
                metadata['cover_path'] = cover_path
                logger.info(f"Extracted cover image to {cover_path}")

            chapters, content_by_chapter = self._extract_content(book)
            logger.info(f"Extracted {len(chapters)} chapters and content")

            # processing chapters and locations
            processed_chapters = []
            total_locations = 0

            for chapter in chapters:
                chapter_content = content_by_chapter.get(chapter['id'], "")
                chapter_filename = f"chapter_{chapter['order']}.html"
                chapter_path = os.path.join(book_content_dir, chapter_filename)

                with open(chapter_path, "w", encoding="utf-8") as f: f.write(chapter_content)

                # calculating location info
                start_location = total_locations + 1
                char_count = len(chapter_content)
                locations_in_chapter = self.location_service.calculate_locations(chapter_content)
                end_location = start_location + locations_in_chapter - 1
                total_locations += locations_in_chapter

                # updating chapter with location and path info
                processed_chapter = {
                    **chapter,
                    'content_path': chapter_path,
                    'start_location': start_location,
                    'end_location': end_location,
                    'char_count': char_count
                }
                processed_chapters.append(processed_chapter)

            index_path = self.content_service.create_index_file(book_content_dir, metadata, processed_chapters)
            metadata_path = self.content_service.save_metadata_file(book_content_dir, metadata, processed_chapters)

            result = {
                'id': book_id,
                'metadata': metadata,
                'chapters': processed_chapters,
                'content_dir': book_content_dir,
                'index_path': index_path,
                'metadata_path': metadata_path,
                'total_locations': total_locations
            }
            logger.info(f"Successfully parsed book: {metadata.get('title', 'Unknown')}")

            return result

        except Exception as e:
            logger.error(f"Failed to parse EPUB file: {str(e)}")
            if 'book_content_dir' in locals() and os.path.exists(book_content_dir): shutil.rmtree(book_content_dir)
            raise BookParsingException(f"Failed to parse EPUB file: {str(e)}")

book_service = BookService()