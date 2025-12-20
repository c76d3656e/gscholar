
"""
Module for querying EasyScholar publication rankings.
"""

import logging
import time
import urllib.parse
from typing import Any

import requests

logger = logging.getLogger(__name__)

class RankingClient:
    """Client for EasyScholar API with rate limiting and caching."""

    def __init__(self, secret_key: str):
        self.secret_key = secret_key
        self.base_url = "https://www.easyscholar.cc/open/getPublicationRank"
        self._cache: dict[str, dict[str, Any] | None] = {}
        self._last_request_time = 0.0
        self._min_interval = 0.6  # slightly more than 0.5s to be safe (max 2 req/s)

    def get_rank(self, venue_name: str) -> dict[str, Any] | None:
        """Get ranking info for a venue.

        Returns None if venue not found or error.
        """
        if not venue_name:
            return None

        # Normalize venue name for cache key? 
        # API might care about casing, but let's keep it as is first.
        # Maybe strip whitespace.
        venue_name = venue_name.strip()
        
        if venue_name in self._cache:
            return self._cache[venue_name]

        # Rate limiting
        now = time.time()
        elapsed = now - self._last_request_time
        if elapsed < self._min_interval:
            sleep_time = self._min_interval - elapsed
            time.sleep(sleep_time)

        try:
            # URL encode is handled by requests params usually, but user asked for robust encoding.
            # requests handles encoding of params values automatically.
            params = {
                "secretKey": self.secret_key,
                "publicationName": venue_name
            }
            
            logger.debug(f"Querying EasyScholar for: {venue_name}")
            response = requests.get(self.base_url, params=params, timeout=10)
            self._last_request_time = time.time()

            msg = "Error"
            if response.status_code == 200:
                data = response.json()
                if data.get("code") == 200:
                    result = data.get("data", {})
                    self._cache[venue_name] = result
                    return result
                else:
                    msg = data.get("msg", "Unknown error")
            
            logger.warning(f"EasyScholar API error for '{venue_name}': {msg}")
            # Cache failure as None to avoid repeated failed lookups
            self._cache[venue_name] = None
            return None

        except Exception as e:
            logger.error(f"Request failed for '{venue_name}': {e}")
            return None

    def get_metric(self, rank_data: dict[str, Any], metric_key: str) -> str | float | None:
        """Extract a specific metric from rank data.
        
        Searches in officialRank.select first, then unique logic if needed.
        """
        if not rank_data:
            return None
            
        official = rank_data.get("officialRank")
        if not official:
            return None
            
        select = official.get("select") or {}
        all_ranks = official.get("all") or {}
        
        # Try select first, then all
        val = select.get(metric_key)
        if val is None:
            val = all_ranks.get(metric_key)
            
        return val
