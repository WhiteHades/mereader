"""
MeReader Content Service for managing extracted book content
"""
import logging
import os
import json
import re
from typing import Dict, List, Any
from bs4 import BeautifulSoup, Comment
from app.core.config import settings
from app.services.text_extraction_utility import text_extraction_util

logger = logging.getLogger(__name__)

class ContentService:
    """Service for managing and processing extracted book content"""

    def __init__(self):
        self.content_dir = settings.CONTENT_DIR
        os.makedirs(self.content_dir, exist_ok=True)
        logger.info(f"Content service initialised with content directory: {self.content_dir}")

    def process_html_content(self, html_content: str) -> str:
        """
        Process HTML content
        Args:
            html_content: Raw HTML content from EPUB
        Returns:
            Processed HTML content
        """
        try:
            html_content = re.sub(r'<\?xml[^>]+\?>', '', html_content)
            html_content = re.sub(r'[\s\n\r\t]+', ' ', html_content)
            soup = BeautifulSoup(html_content, 'html.parser')

            for tag in soup(['script', 'style']): tag.decompose()

            for comment in soup.find_all(text=lambda text: isinstance(text, Comment)): comment.extract()

            if soup.html:
                html_contents = list(soup.html.children)
                soup.html.replace_with(*html_contents)

            if soup.body:
                body_contents = list(soup.body.children)
                soup.body.replace_with(*body_contents)

            # empty tags whitespace
            for tag in soup.find_all():
                if tag.name not in ['img', 'br'] and not tag.get_text(strip=True) and not tag.find_all(['img']):
                    tag.decompose()

            # links
            for link in soup.find_all('a', href=True):
                href = link['href']
                if href.startswith('#') or '.html' in href or '.xhtml' in href: link['data-internal-link'] = 'true'

            # images
            for img in soup.find_all('img', src=True):
                img['data-epub-src'] = img['src']
                img_src = img['src']
                if '/' in img_src:
                    img['src'] = os.path.basename(img_src)

            # inline styles
            for tag in soup.find_all(style=True): del tag['style']

            style_tag = soup.new_tag('style')
            style_tag.string = """
                body { 
                    font-family: system-ui, -apple-system, sans-serif; 
                    line-height: 1.5; 
                    max-width: 100%;
                    padding: 0 1rem;
                    white-space: normal;
                }
                img { max-width: 100%; height: auto; }
                p { margin: 0.75em 0; white-space: normal; }
                h1, h2, h3, h4, h5, h6 { margin: 1em 0 0.5em 0; white-space: normal; }
                pre, code { white-space: pre-wrap; }
                * { white-space: normal; }
            """

            if not soup.head:
                head_tag = soup.new_tag('head')
                if soup.html: soup.html.insert(0, head_tag)
                else: soup.insert(0, head_tag)

            if not soup.body:
                body_tag = soup.new_tag('body')
                if soup.html:
                    for content in list(soup.html.contents):
                        if content.name != 'head': body_tag.append(content)
                    soup.html.append(body_tag)

            soup.head.append(style_tag)

            for text in soup.find_all(text=True):
                if text.parent.name not in ['pre', 'code']:
                    new_text = re.sub(r'\s+', ' ', text.string.strip())
                    text.replace_with(new_text)

            if soup.body: return ''.join(str(c) for c in soup.body.contents)
            else: return str(soup)

        except Exception as e:
            logger.warning(f"Error processing HTML content: {str(e)}")
            html_content = re.sub(r'[\s\n\r\t]+', ' ', html_content)
            html_content = re.sub(r'<html[^>]*>|</html>|<body[^>]*>|</body>', '', html_content)
            return html_content

    def create_index_file(self, content_dir: str, metadata: Dict[str, Any], chapters: List[Dict[str, Any]]) -> str | None:
        """
        Create an index.html file for the book
        Args:
            content_dir: Directory where content is stored
            metadata: Book metadata
            chapters: List of chapter information
        Returns:
            Path to the created index file
        """
        try:
            index_path = os.path.join(content_dir, "index.html")

            html_content = f"""<!DOCTYPE html>
            <html>
            <head>
                <meta charset="utf-8">
                <title>{metadata.get('title', 'Unknown')}</title>
                <style>
                    body {{
                        font-family: system-ui, -apple-system, sans-serif;
                        line-height: 1.5;
                        max-width: 800px;
                        margin: 0 auto;
                        padding: 1rem;
                    }}
                    .cover {{
                        text-align: center;
                        margin-bottom: 2rem;
                    }}
                    .cover img {{
                        max-width: 300px;
                        height: auto;
                        box-shadow: 0 4px 8px rgba(0,0,0,0.1);
                    }}
                    .metadata {{
                        margin-bottom: 2rem;
                    }}
                    .toc {{
                        margin-bottom: 2rem;
                    }}
                    .toc ol {{
                        list-style-type: decimal;
                        padding-left: 1.5rem;
                    }}
                    .toc li {{
                        padding: 0.25rem 0;
                    }}
                </style>
            </head>
            <body>
                <div class="cover">
                    {f'<img src="../../../{metadata["cover_path"]}" alt="Cover">' if metadata.get('cover_path') else ''}
                </div>
                <div class="metadata">
                    <h1>{metadata.get('title', 'Unknown')}</h1>
                    <p>Author: {metadata.get('author', 'Unknown')}</p>
                    {f'<p>Published: {metadata.get("published_year", "")}</p>' if metadata.get('published_year') else ''}
                    {f'<p>Publisher: {metadata.get("publisher", "")}</p>' if metadata.get('publisher') else ''}
                    {f'<p>{metadata.get("description", "")}</p>' if metadata.get('description') else ''}
                </div>
                <div class="toc">
                    <h2>Table of Contents</h2>
                    <ol>
                        {"".join([f'<li><a href="{os.path.basename(ch["content_path"])}">{ch["title"]}</a></li>' for ch in chapters])}
                    </ol>
                </div>
            </body>
            </html>
            """

            with open(index_path, "w", encoding="utf-8") as f: f.write(html_content)

            return index_path

        except Exception as e:
            logger.error(f"Failed to create index file: {str(e)}")
            return None

    def save_metadata_file(self, content_dir: str, metadata: Dict[str, Any], chapters: List[Dict[str, Any]]) -> str | None:
        """
        Save a JSON metadata file with book and chapter information
        Args:
            content_dir: Directory where content is stored
            metadata: Book metadata
            chapters: List of chapter information
        Returns:
            Path to the created metadata file
        """
        try:
            metadata_path = os.path.join(content_dir, "metadata.json")
            metadata_obj = {
                "book": metadata,
                "chapters": chapters
            }

            with open(metadata_path, "w", encoding="utf-8") as f: json.dump(metadata_obj, f, indent=2)

            return metadata_path

        except Exception as e:
            logger.error(f"Failed to save metadata file: {str(e)}")
            return None

    def get_text_at_location(self, html_content: str, location: int, context_size: int = 500) -> str:
        """
        Extract text around a specific location
        Args:
            html_content: HTML content of the chapter
            location: Location within the chapter (relative to chapter start)
            context_size: Number of characters to include before and after the location
        Returns:
            Text at the specified location with context
        """
        try:
            text = text_extraction_util.extract_text_streamed(html_content, False)
            char_position = min(len(text) - 1, location * settings.LOCATION_CHUNK_SIZE)
            if char_position < 0: return ""

            start = max(0, char_position - context_size)
            end = min(len(text), char_position + context_size)

            return text[start:end]

        except Exception as e:
            logger.error(f"Error getting text at location: {str(e)}")
            return ""

content_service = ContentService()