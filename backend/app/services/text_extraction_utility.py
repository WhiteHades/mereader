"""
MeReader Text Extraction Utility
"""
import logging
import re
import os
from typing import List, Generator, Tuple
import gc
from bs4 import BeautifulSoup, SoupStrainer

logger = logging.getLogger(__name__)

class TextExtractionUtil:
    """Utility for text extraction and processing"""

    def extract_text_streamed(self, html_path_or_content: str, is_file_path: bool = True) -> str:
        """
        Text extraction from html
        Args:
            html_path_or_content: Either a path to an HTML or HTML content as string
            is_file_path: Boolean indicating if the input is a path (True) or an HTML (False)
        Returns:
            Plain text
        """
        try:
            content = html_path_or_content

            if is_file_path:
                if not os.path.exists(html_path_or_content):
                    logger.warning(f"File not found: {html_path_or_content}")
                    return ""
                with open(html_path_or_content, 'r', encoding='utf-8') as f:
                    content = f.read()

            # - XML/HTML
            content = re.sub(r'<\?xml[^>]+\?>', '', content)
            content = re.sub(r'<!DOCTYPE[^>]+>', '', content)

            soup = BeautifulSoup(content, 'html.parser')

            for tag_name in ['script', 'style', 'meta', 'link', 'head']:
                for tag in soup.find_all(tag_name): tag.decompose()

            text = soup.get_text()

            # cleaning whitespace
            lines = (line.strip() for line in text.splitlines())
            chunks = (phrase.strip() for line in lines for phrase in line.split("  "))
            text = '\n'.join(chunk for chunk in chunks if chunk)

            # removing any remaining XML/HTML-like artifacts
            text = re.sub(r'</?[a-z]+[^>]*>', '', text)

            return text

        except Exception as e:
            logger.error(f"Error extracting text: {str(e)}")
            return ""

    def chunk_text_streamed(self, file_path: str, chunk_size: int = 650, chunk_overlap: int = 50, min_chunk_size: int = 100) -> Generator[str, None, None]:
        """
        Chunk generation
        """
        try:
            if file_path.endswith('.html'):
                text = self.extract_text_streamed(file_path)
                pos = 0
                buffer = ""

                while pos < len(text):
                    end_buffer = min(pos + chunk_size * 3, len(text))
                    buffer = text[pos:end_buffer]

                    chunk_end = min(chunk_size, len(buffer))
                    paragraph_break = buffer.rfind('\n\n', chunk_size // 2, chunk_size + 50)

                    if paragraph_break != -1 and paragraph_break > chunk_size // 2: chunk_end = paragraph_break + 2
                    else:
                        sentence_break = buffer.rfind('. ', chunk_size // 2, chunk_size + 30)
                        if sentence_break != -1 and sentence_break > chunk_size // 2: chunk_end = sentence_break + 2

                    chunk = buffer[:chunk_end].strip()

                    if chunk and len(chunk) >= min_chunk_size: yield chunk

                    advance = max(chunk_end - (chunk_overlap // 2), chunk_size // 2)
                    pos += advance

                    if pos % (chunk_size * 20) == 0: gc.collect()

            else:
                # plain text files
                with open(file_path, 'r', encoding='utf-8') as file:
                    buffer = ""

                    for line in file:
                        buffer += line
                        while len(buffer) >= chunk_size * 1.5:
                            # finding optimum breakpoint
                            chunk_end = chunk_size
                            paragraph_break = buffer.rfind('\n\n', chunk_size // 2, chunk_size + 50)

                            if paragraph_break != -1 and paragraph_break > chunk_size // 2: chunk_end = paragraph_break + 2
                            else:
                                sentence_break = buffer.rfind('. ', chunk_size // 2, chunk_size + 30)
                                if sentence_break != -1 and sentence_break > chunk_size // 2: chunk_end = sentence_break + 2

                            chunk = buffer[:chunk_end].strip()
                            if chunk and len(chunk) >= min_chunk_size: yield chunk

                            buffer = buffer[chunk_end - (chunk_overlap // 2):]

                    if len(buffer) >= min_chunk_size: yield buffer.strip()

        except Exception as e:
            logger.error(f"Error in chunk_text_streamed: {str(e)}")
            if 'buffer' in locals() and buffer and len(buffer) >= min_chunk_size: yield buffer.strip()

    def batch_chunks(self, generator, batch_size: int = 10) -> Generator[List[str], None, None]:
        """
        Batching of chunks from a generator
        """
        batch = []
        try:
            for chunk in generator:
                batch.append(chunk)

                if len(batch) >= batch_size:
                    yield batch
                    batch.clear()

            if batch: yield batch
        except Exception as e:
            logger.error(f"Error in batch_chunks: {str(e)}")
            if batch: yield batch

    def extract_chapter_info(self, file_path: str) -> Tuple[str, int]:
        """
        Extract chapter title and approximate location from file path
        Args:
            file_path: Path to chapter file
        Returns:
            Tuple of (title, page_estimate)
        """
        try:
            filename = os.path.basename(file_path)
            match = re.search(r'chapter_(\d+)', filename)
            if match:
                chapter_num = int(match.group(1))
                parse_only = SoupStrainer(['title', 'h1', 'h2', 'h3'])

                with open(file_path, 'r', encoding='utf-8') as f: content = f.read(10 * 1024)  #  10kb

                soup = BeautifulSoup(content, 'html.parser', parse_only=parse_only)
                title = None

                for tag in ['h1', 'h2', 'h3']:
                    element = soup.find(tag)
                    if element:
                        title = element.get_text().strip()
                        break

                if not title:
                    title_elem = soup.find('title')
                    if title_elem: title = title_elem.get_text().strip()

                if not title: title = f"Chapter {chapter_num}"

                # assuming 250 words/page
                page_estimate = chapter_num * 10  # 10 pages per chapter

                return title, page_estimate

            # default values
            return "Unknown Chapter", 1

        except Exception as e:
            logger.error(f"Error extracting chapter info: {str(e)}")
            return "Unknown Chapter", 1

text_extraction_util = TextExtractionUtil()