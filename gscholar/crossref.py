"""
Module for querying Crossref API with concurrent requests.
"""

import logging
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Any

import requests

logger = logging.getLogger(__name__)

CROSSREF_API_URL = "https://api.crossref.org/works"
MAILTO = "c76d@c.com"  # Polite pool email

class CrossrefClient:
    """Client for Crossref API with concurrent requests and rate limiting."""

    def __init__(self, max_workers: int = 3):
        self.max_workers = max_workers
        self._session = requests.Session()
        self._session.headers.update({
            "User-Agent": f"gscholar-cli/1.0 (mailto:{MAILTO})"
        })

    def lookup_by_title(self, title: str) -> dict[str, Any] | None:
        """Lookup single article by title with exponential backoff."""
        if not title:
            return None

        title = title.strip()
        
        params = {
            "query.title": title,
            "rows": 1,
            "select": "DOI,title,author,container-title,published,abstract",
            "mailto": MAILTO
        }
        
        max_retries = 3
        backoff = 0.5
        
        for attempt in range(max_retries):
            try:
                response = self._session.get(CROSSREF_API_URL, params=params, timeout=15)
                
                # Check rate limit headers
                rate_limit = response.headers.get("X-Rate-Limit-Limit")
                rate_interval = response.headers.get("X-Rate-Limit-Interval")
                if rate_limit:
                    logger.debug(f"Rate limit: {rate_limit}/{rate_interval}")
                
                if response.status_code == 429:
                    wait_time = backoff * (2 ** attempt)
                    logger.warning(f"429 Rate limited, waiting {wait_time}s...")
                    time.sleep(wait_time)
                    continue
                
                if response.status_code == 200:
                    data = response.json()
                    items = data.get("message", {}).get("items", [])
                    if items:
                        return self._parse_item(items[0])
                
                return None
                
            except Exception as e:
                logger.error(f"Crossref error for '{title[:30]}...': {e}")
                if attempt < max_retries - 1:
                    time.sleep(backoff * (2 ** attempt))
                    continue
                return None
        
        return None

    def _parse_item(self, item: dict) -> dict[str, Any]:
        """Parse Crossref response item."""
        # Authors
        authors_raw = item.get("author", [])
        authors = ", ".join([
            f"{a.get('given', '')} {a.get('family', '')}".strip()
            for a in authors_raw
        ]) if authors_raw else ""
        
        # Date
        published = item.get("published", {})
        date_parts = published.get("date-parts", [[]])[0]
        date_str = "-".join(str(p) for p in date_parts) if date_parts else ""
        
        # Journal
        container = item.get("container-title", [])
        journal = container[0] if container else ""
        
        # Abstract (clean HTML tags)
        abstract = item.get("abstract", "")
        if abstract:
            import re
            abstract = re.sub(r'<[^>]+>', '', abstract)
        
        return {
            "doi": item.get("DOI", ""),
            "journal": journal,
            "authors": authors,
            "date": date_str,
            "abstract": abstract,
            "crossref_title": item.get("title", [""])[0] if item.get("title") else ""
        }

    def lookup_batch(self, titles: list[str]) -> list[dict[str, Any] | None]:
        """Lookup multiple titles concurrently."""
        results = [None] * len(titles)
        
        with ThreadPoolExecutor(max_workers=self.max_workers) as executor:
            future_to_idx = {
                executor.submit(self.lookup_by_title, title): idx
                for idx, title in enumerate(titles)
            }
            
            for future in as_completed(future_to_idx):
                idx = future_to_idx[future]
                try:
                    results[idx] = future.result()
                except Exception as e:
                    logger.error(f"Batch lookup error at index {idx}: {e}")
                    results[idx] = None
        
        return results
