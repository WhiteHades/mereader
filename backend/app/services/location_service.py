"""
MeReader Location Service - Location tracking for book reading
"""
import logging
import math
from typing import Optional, Dict, List, Any
from app.core.config import settings
from app.services.text_extraction_utility import text_extraction_util

logger = logging.getLogger(__name__)

class LocationService:
    """
    Service for tracking reading positions using location numbers
    """

    def __init__(self):
        self.location_chunk_size = settings.LOCATION_CHUNK_SIZE
        logger.info(f"Location service initialized with chunk size: {self.location_chunk_size}")

    def calculate_locations(self, content: str) -> int:
        """
        Calculate the number of location units in content
        Args:
            content: HTML content to calculate locations for
        Returns:
            Number of locations in the content
        """
        try:
            text = text_extraction_util.extract_text_streamed(content, False)
            char_count = len(text)
            locations = max(1, math.ceil(char_count / self.location_chunk_size))

            return locations

        except Exception as e:
            logger.error(f"Error calculating locations: {str(e)}")
            return 1

    def calculate_location_boundary(self, current_location: int, total_locations: int) -> int:
        """
        Calculate a safe location boundary for query filtering
        Args:
            current_location: Current reading location
            total_locations: Total locations in the book
        Returns:
            Location boundary to use for query filtering
        """
        if current_location <= 0: return 1

        if total_locations and current_location > total_locations:
            return total_locations

        return current_location

    def get_text_at_location(self, content: str, location: int, context_size: int = 100) -> str:
        """
        Get text at a specific location number
        Args:
            content: HTML content of the chapter
            location: Location number to get text at
            context_size: Amount of text to return around the location
        Returns:
            Text at the specified location
        """
        try:
            text = text_extraction_util.extract_text_streamed(content)
            # character position from location
            char_position = min(len(text) - 1, (location - 1) * self.location_chunk_size)
            if char_position < 0: return ""

            start = max(0, char_position - context_size)
            end = min(len(text), char_position + context_size)

            return text[start:end]

        except Exception as e:
            logger.error(f"Error getting text at location: {str(e)}")
            return ""

    def get_percentage_from_location(self, location: int, total_locations: int) -> float:
        """
        Calculate reading progress percentage from location
        Args:
            location: Current location
            total_locations: Total locations in the book
        Returns:
            Reading progress percentage (0-100)
        """
        if total_locations <= 0: return 0.0
        percentage = min(100.0, max(0.0, (location / total_locations) * 100.0))

        return percentage

    def get_location_from_percentage(self, percentage: float, total_locations: int) -> int:
        """
        Calculate location from reading progress percentage
        Args:
            percentage: Reading progress percentage (0-100)
            total_locations: Total locations in the book
        Returns:
            Location corresponding to the percentage
        """
        if percentage <= 0 or total_locations <= 0: return 1
        if percentage >= 100: return total_locations

        location = max(1, min(total_locations, round((percentage / 100.0) * total_locations)))

        return location

    def get_chapter_from_location(self, location: int, chapters: List[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
        """
        Find the chapter containing a specific location
        Args:
            location: Location to find chapter for
            chapters: List of chapter information with location boundaries

        Returns:
            Chapter containing the location or None if not found
        """
        try:
            for chapter in chapters:
                start_location = chapter.get('start_location', 0)
                end_location = chapter.get('end_location', 0)

                if start_location <= location <= end_location: return chapter

            prev_chapter = None
            for chapter in sorted(chapters, key=lambda x: x.get('start_location', 0)):
                if chapter.get('start_location', 0) > location: break
                prev_chapter = chapter
            return prev_chapter

        except Exception as e:
            logger.error(f"Error finding chapter for location: {str(e)}")
            return None

location_service = LocationService()